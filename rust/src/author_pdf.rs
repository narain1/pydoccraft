use std::{io::Cursor, sync::Arc};

use pdfcraft_content::{serialize_ops, Op};
use pdfcraft_cos::{Document, SaveOptions};
use pdfcraft_edit::{AddedImage, AddedText, Content};
use pyo3::exceptions::{PyIndexError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde_json::{json, Value};

use crate::EngineError;

const MAX_PAGES: usize = 10_000;
const MAX_ITEMS: usize = 10_000;
const MAX_TEXT_BYTES: usize = 1_000_000;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_DOCUMENT_INPUT: usize = 128 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
const MAX_FIELDS: usize = 1_000;
const MAX_FIELD_TEXT: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
enum BackgroundState {
    Known(Option<(u8, u8, u8)>),
    Unknown,
}

/// Editable native PDF model. Python owns the public authoring facade.
#[pyclass(module = "doc_ex._core")]
pub struct EditablePdf {
    document: Option<Document>,
    sizes: Vec<(f64, f64)>,
    geometry: Vec<pdfcraft_render::PageInfo>,
    backgrounds: Vec<BackgroundState>,
    items: usize,
    input_bytes: usize,
    // Only models created in this process can acquire fields. Import never trusts AcroForms.
    owns_fields: bool,
}

#[pymethods]
impl EditablePdf {
    #[new]
    #[pyo3(signature = (width=612.0, height=792.0))]
    pub fn new(width: f64, height: f64) -> PyResult<Self> {
        validate_page_size(width, height)?;
        let document = pdfcraft_create::blank(width, height, 1).map_err(creation_error)?;
        Ok(Self {
            document: Some(document),
            sizes: vec![(width, height)],
            geometry: vec![page_geometry(width, height)],
            backgrounds: vec![BackgroundState::Known(None)],
            items: 0,
            input_bytes: 0,
            owns_fields: true,
        })
    }

    #[staticmethod]
    #[pyo3(signature = (data, password=None))]
    pub fn from_bytes(data: &[u8], password: Option<&str>) -> PyResult<Self> {
        if data.len() > MAX_DOCUMENT_INPUT {
            return Err(EngineError::ResourceLimit("PDF input exceeds 128 MiB".into()).into());
        }
        let bytes = Arc::new(data.to_vec());
        let info =
            pdfcraft_render::inspect(bytes.clone(), password).map_err(|error| match error {
                pdfcraft_render::OpenError::NeedsPassword
                | pdfcraft_render::OpenError::WrongPassword => EngineError::Encrypted,
                pdfcraft_render::OpenError::Unsupported(message) => {
                    EngineError::Unsupported(message)
                }
                pdfcraft_render::OpenError::Invalid(message) => EngineError::Malformed(message),
            })?;
        if info.pages.is_empty() {
            return Err(EngineError::Malformed("PDF contains no pages".into()).into());
        }
        if info.pages.len() > MAX_PAGES {
            return Err(EngineError::ResourceLimit("PDF exceeds 10000 pages".into()).into());
        }
        if info.outline_more {
            return Err(EngineError::ResourceLimit(
                "PDF outline exceeds the bounded inspection count or depth; editing is unavailable"
                    .into(),
            )
            .into());
        }
        if info.encrypted {
            return Err(
                EngineError::Unsupported("editing encrypted PDFs is not available".into()).into(),
            );
        }
        if !info.fields.is_empty() {
            return Err(EngineError::Unsupported(
                "editing imported PDFs with form or signature fields is not available".into(),
            )
            .into());
        }
        let sizes = info
            .pages
            .iter()
            .map(|page| (f64::from(page.width), f64::from(page.height)))
            .collect::<Vec<_>>();
        for &(width, height) in &sizes {
            validate_page_size(width, height)?;
        }
        let document =
            Document::open_with_password(bytes, password).map_err(|error| match error {
                pdfcraft_cos::CosError::NeedsPassword | pdfcraft_cos::CosError::WrongPassword => {
                    EngineError::Encrypted
                }
                other => EngineError::Malformed(format!("invalid PDF object graph: {other}")),
            })?;
        // The full-save workflow cannot preserve digital signatures/certification. Refuse
        // even an unsigned signature field or a certification entry instead of invalidating it.
        if document.security().is_some()
            || document.root().is_some_and(|root| {
                document.get(root).as_dict().is_some_and(|catalog| {
                    catalog.get(b"Perms").is_some() || catalog.get(b"AcroForm").is_some()
                })
            })
        {
            return Err(EngineError::Unsupported(
                "editing PDFs with encryption, forms, or certification is not available".into(),
            )
            .into());
        }
        let items = pdfcraft_edit::list_added(&document)
            .len()
            .saturating_add(info.annotations.len())
            .saturating_add(info.links.len());
        if items > MAX_ITEMS {
            return Err(
                EngineError::ResourceLimit("PDF authoring item budget exceeded".into()).into(),
            );
        }
        Ok(Self {
            document: Some(document),
            backgrounds: vec![BackgroundState::Unknown; sizes.len()],
            geometry: info.pages,
            sizes,
            items,
            input_bytes: data.len(),
            owns_fields: false,
        })
    }

    pub fn copy(&self) -> PyResult<Self> {
        Ok(Self {
            document: Some(self.document()?.clone()),
            sizes: self.sizes.clone(),
            geometry: self.geometry.clone(),
            backgrounds: self.backgrounds.clone(),
            items: self.items,
            input_bytes: self.input_bytes,
            owns_fields: self.owns_fields,
        })
    }

    pub fn close(&mut self) {
        self.document = None;
        self.sizes.clear();
        self.geometry.clear();
        self.backgrounds.clear();
    }

    #[pyo3(name = "to_bytes", signature = (object_streams=true))]
    pub fn to_bytes_with_options(&self, object_streams: bool) -> PyResult<Vec<u8>> {
        self.serialize_with_limit(object_streams, MAX_OUTPUT_BYTES)
    }

    pub fn page_count(&self) -> PyResult<usize> {
        self.document()?;
        Ok(self.sizes.len())
    }

    pub fn page_size(&self, page: isize) -> PyResult<(f64, f64)> {
        Ok(self.sizes[self.page_index(page)?])
    }

    /// Effective boxes expressed as x, y, width, height in the current displayed view.
    pub fn page_boxes(&self, page: isize) -> PyResult<String> {
        let page = self.page_index(page)?;
        let boxes = pdfcraft_organize::page_boxes(self.document()?).map_err(organize_error)?;
        let values: serde_json::Map<String, Value> = ["media", "crop", "bleed", "trim", "art"]
            .into_iter()
            .zip(boxes[page])
            .map(|(name, rect)| {
                let r = view_rect(&self.geometry[page], rect.map(|value| value as f32));
                (name.into(), json!([r[0], r[1], r[2] - r[0], r[3] - r[1]]))
            })
            .collect();
        crate::limits::bounded_json(&values, 4096).map_err(Into::into)
    }

    #[pyo3(signature = (page, kind, rect=None))]
    pub fn set_page_box(
        &mut self,
        page: isize,
        kind: &str,
        rect: Option<(f64, f64, f64, f64)>,
    ) -> PyResult<()> {
        use pdfcraft_organize::{BoxSpec, PageBox};
        let page = self.page_index(page)?;
        let which = match kind {
            "media" => PageBox::Media,
            "crop" => PageBox::Crop,
            "bleed" => PageBox::Bleed,
            "trim" => PageBox::Trim,
            "art" => PageBox::Art,
            _ => return Err(PyValueError::new_err("unknown PDF page box")),
        };
        let before = pdfcraft_organize::page_boxes(self.document()?).map_err(organize_error)?;
        let spec = if let Some((x, y, width, height)) = rect {
            if ![x, y, width, height, x + width, y + height]
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.0)
                || width <= 0.0
                || height <= 0.0
            {
                return Err(PyValueError::new_err(
                    "page box must have finite coordinates within 1000000 points and positive dimensions",
                ));
            }
            let r = self.geometry[page]
                .view_rect_to_user([x as f32, y as f32, (x + width) as f32, (y + height) as f32])
                .map(f64::from);
            if r[2] - r[0] < 1.0 || r[3] - r[1] < 1.0 {
                return Err(PyValueError::new_err(
                    "page box must span at least one PDF user-space unit per side",
                ));
            }
            let media = before[page][0];
            if which != PageBox::Media
                && (r[0] < media[0] - 0.001
                    || r[1] < media[1] - 0.001
                    || r[2] > media[2] + 0.001
                    || r[3] > media[3] + 0.001)
            {
                return Err(PyValueError::new_err(
                    "page box must lie within the media box",
                ));
            }
            if which == PageBox::Media
                && before[page][1..].iter().any(|child| {
                    child[2].min(r[2]) - child[0].max(r[0]) < 1.0
                        || child[3].min(r[3]) - child[1].max(r[1]) < 1.0
                })
            {
                return Err(PyValueError::new_err(
                    "media box would empty an existing effective page box",
                ));
            }
            BoxSpec::Rect(r)
        } else {
            if which == PageBox::Media {
                return Err(PyValueError::new_err("media box cannot be reset"));
            }
            BoxSpec::Remove
        };
        let mut edited = self.document()?.clone();
        pdfcraft_organize::set_page_box(&mut edited, &[page], which, spec)
            .map_err(organize_error)?;
        let output =
            pdfcraft_cos::write_full(&edited, &SaveOptions::default()).map_err(|error| {
                EngineError::Conversion(format!("PDF geometry serialization failed: {error}"))
            })?;
        if output.len() > MAX_OUTPUT_BYTES {
            return Err(EngineError::ResourceLimit("PDF output exceeds 256 MiB".into()).into());
        }
        let info = pdfcraft_render::inspect(Arc::new(output), None).map_err(|error| {
            EngineError::Malformed(format!("invalid updated PDF geometry: {error}"))
        })?;
        for geometry in &info.pages {
            validate_page_size(f64::from(geometry.width), f64::from(geometry.height))?;
        }
        self.validate_owned_edit(&edited)?;
        self.sizes = info
            .pages
            .iter()
            .map(|g| (f64::from(g.width), f64::from(g.height)))
            .collect();
        self.geometry = info.pages;
        self.backgrounds[page] = BackgroundState::Unknown;
        self.document = Some(edited);
        Ok(())
    }

    #[pyo3(signature = (width=612.0, height=792.0))]
    pub fn add_page(&mut self, width: f64, height: f64) -> PyResult<usize> {
        self.document()?;
        validate_page_size(width, height)?;
        if self.sizes.len() >= MAX_PAGES {
            return Err(EngineError::ResourceLimit("PDF exceeds 10000 pages".into()).into());
        }
        let index = self.sizes.len();
        let mut edited = self.document()?.clone();
        pdfcraft_organize::insert_blank_page(&mut edited, index, width, height).map_err(
            |error| EngineError::Conversion(format!("PDF page insertion failed: {error}")),
        )?;
        self.validate_owned_edit(&edited)?;
        self.document = Some(edited);
        self.sizes.push((width, height));
        self.geometry.push(page_geometry(width, height));
        self.backgrounds.push(BackgroundState::Known(None));
        Ok(index)
    }

    pub fn delete_page(&mut self, page: isize) -> PyResult<()> {
        let page = self.page_index(page)?;
        if self.sizes.len() == 1 {
            return Err(PyValueError::new_err("cannot delete the final PDF page"));
        }
        let mut edited = self.document()?.clone();
        pdfcraft_organize::delete_pages(&mut edited, &[page]).map_err(organize_error)?;
        self.validate_owned_edit(&edited)?;
        self.document = Some(edited);
        self.sizes.remove(page);
        self.geometry.remove(page);
        self.backgrounds.remove(page);
        Ok(())
    }

    pub fn rotate_page(&mut self, page: isize, degrees: i64) -> PyResult<()> {
        let page = self.page_index(page)?;
        if degrees % 90 != 0 {
            return Err(PyValueError::new_err(
                "rotation must be a multiple of 90 degrees",
            ));
        }
        let mut edited = self.document()?.clone();
        pdfcraft_organize::rotate_pages(&mut edited, &[page], degrees).map_err(organize_error)?;
        self.validate_owned_edit(&edited)?;
        self.document = Some(edited);
        if degrees.rem_euclid(180) != 0 {
            let (width, height) = self.sizes[page];
            self.sizes[page] = (height, width);
            self.geometry[page].width = height as f32;
            self.geometry[page].height = width as f32;
        }
        self.geometry[page].rotation = (i64::from(self.geometry[page].rotation)
            + degrees.rem_euclid(360))
        .rem_euclid(360) as u16;
        Ok(())
    }

    /// Destination is the page's final zero-based position, with ordinary negative indexing.
    pub fn move_page(&mut self, page: isize, destination: isize) -> PyResult<()> {
        let page = self.page_index(page)?;
        let destination = self.page_index(destination)?;
        let mut edited = self.document()?.clone();
        pdfcraft_organize::move_pages(&mut edited, &[page], destination).map_err(organize_error)?;
        self.validate_owned_edit(&edited)?;
        self.document = Some(edited);
        let size = self.sizes.remove(page);
        self.sizes.insert(destination, size);
        let geometry = self.geometry.remove(page);
        self.geometry.insert(destination, geometry);
        let background = self.backgrounds.remove(page);
        self.backgrounds.insert(destination, background);
        Ok(())
    }

    #[pyo3(signature = (data, password, indices, destination, outline_title=None))]
    pub fn import_pages(
        &mut self,
        data: &[u8],
        password: Option<&str>,
        indices: Option<Vec<isize>>,
        destination: isize,
        outline_title: Option<&str>,
    ) -> PyResult<usize> {
        self.document()?;
        if let Some(title) = outline_title {
            validate_bookmark_title(title)?;
        }
        let destination = if destination < 0 {
            destination + self.sizes.len() as isize
        } else {
            destination
        };
        if destination < 0 || destination as usize > self.sizes.len() {
            return Err(PyIndexError::new_err("page insertion index out of range"));
        }
        let source = Self::from_bytes(data, password)?;
        let outlines = if outline_title.is_some() {
            let source_entries = checked_import_outline(source.document()?, 31, true)?;
            let destination_entries = checked_import_outline(self.document()?, 32, false)?;
            let expected = destination_entries.len()
                + source_entries.len()
                + usize::from(!source_entries.is_empty());
            if expected > MAX_ITEMS {
                return Err(EngineError::ResourceLimit(
                    "imported outline would exceed 10000 bookmarks".into(),
                )
                .into());
            }
            Some((source_entries, destination_entries, expected))
        } else {
            None
        };
        let selected = match indices {
            Some(indices) => source.selection(&indices)?,
            None => (0..source.sizes.len()).collect(),
        };
        if selected.is_empty() {
            return Ok(0);
        }
        if selected.len() > MAX_PAGES - self.sizes.len() {
            return Err(EngineError::ResourceLimit("PDF exceeds 10000 pages".into()).into());
        }
        // Conservative upper bound: every selected page could reference all source resources.
        let input_cost = data
            .len()
            .checked_mul(selected.len())
            .and_then(|cost| cost.checked_add(outline_title.map_or(0, str::len)))
            .ok_or_else(|| EngineError::ResourceLimit("PDF import size overflow".into()))?;
        self.validate_input(input_cost)?;
        let item_cost = source
            .items
            .checked_mul(selected.len())
            .ok_or_else(|| EngineError::ResourceLimit("PDF import item overflow".into()))?;
        if item_cost > MAX_ITEMS.saturating_sub(self.items) {
            return Err(
                EngineError::ResourceLimit("PDF authoring item budget exceeded".into()).into(),
            );
        }
        let mut edited = self.document()?.clone();
        let imported = if let Some(title) = outline_title {
            pdfcraft_organize::insert_file_pages(
                &mut edited,
                source.document()?,
                title,
                &selected,
                destination as usize,
                pdfcraft_organize::FieldConflicts::Rename,
            )
            .map_err(organize_error)?
            .pages
        } else {
            pdfcraft_organize::import_pages(
                &mut edited,
                source.document()?,
                &selected,
                destination as usize,
            )
            .map_err(organize_error)?
        };
        if imported.len() != selected.len() {
            return Err(EngineError::Conversion(
                "PDF import returned an unexpected page count".into(),
            )
            .into());
        }
        if let Some((source_entries, destination_entries, expected)) = outlines {
            let actual = checked_import_outline(&edited, 32, false)?;
            if actual.len() != expected {
                return Err(EngineError::Unsupported(
                    "outline import could not preserve every source bookmark; document unchanged"
                        .into(),
                )
                .into());
            }
            let existing = destination_entries
                .iter()
                .map(|entry| entry.obj)
                .collect::<std::collections::HashSet<_>>();
            let added = actual
                .iter()
                .filter(|entry| !existing.contains(&entry.obj))
                .collect::<Vec<_>>();
            let preserved = if source_entries.is_empty() {
                added.is_empty()
            } else {
                added.len() == source_entries.len() + 1
                    && added[0].path.len() == 1
                    && added[0].title == outline_title.unwrap_or_default().trim()
                    && added[0].page == Some(destination as usize)
                    && added[1..]
                        .iter()
                        .zip(&source_entries)
                        .all(|(copy, original)| {
                            let page = original
                                .page
                                .and_then(|page| {
                                    selected.iter().position(|selected| *selected == page)
                                })
                                .map(|offset| destination as usize + offset);
                            copy.title == original.title
                                && copy.path.first() == added[0].path.first()
                                && copy.path.get(1..) == Some(original.path.as_slice())
                                && copy.page == page
                                && copy.open == original.open
                                && copy.view == page.and(original.view.clone())
                        })
            };
            if !preserved {
                return Err(EngineError::Unsupported(
                    "native import could not preserve the selected outline destinations; document unchanged".into(),
                ).into());
            }
            // The listing itself caps depth. Inspection explicitly reports omitted nodes.
            let output =
                pdfcraft_cos::write_full(&edited, &SaveOptions::default()).map_err(|error| {
                    EngineError::Conversion(format!("PDF outline serialization failed: {error}"))
                })?;
            if output.len() > MAX_OUTPUT_BYTES {
                return Err(EngineError::ResourceLimit("PDF output exceeds 256 MiB".into()).into());
            }
            let info = pdfcraft_render::inspect(Arc::new(output), None).map_err(|error| {
                EngineError::Malformed(format!("invalid imported PDF outline: {error}"))
            })?;
            if info.outline_more {
                return Err(EngineError::ResourceLimit(
                    "imported outline exceeds inspection limits".into(),
                )
                .into());
            }
        }
        self.validate_owned_edit(&edited)?;
        self.document = Some(edited);
        self.sizes.splice(
            destination as usize..destination as usize,
            selected.iter().map(|index| source.sizes[*index]),
        );
        self.backgrounds.splice(
            destination as usize..destination as usize,
            selected.iter().map(|index| source.backgrounds[*index]),
        );
        self.geometry.splice(
            destination as usize..destination as usize,
            selected.iter().map(|index| source.geometry[*index].clone()),
        );
        self.input_bytes += input_cost;
        self.items += item_cost;
        Ok(selected.len())
    }

    pub fn subset(&self, indices: Vec<isize>) -> PyResult<Self> {
        if !self.owned_fields()?.is_empty() {
            return Err(EngineError::Unsupported("PDF subsets with active fields are not available; flatten or delete the fields first".into()).into());
        }
        let selected = self.selection(&indices)?;
        if selected.is_empty() {
            return Err(PyValueError::new_err(
                "a PDF subset must contain at least one page",
            ));
        }
        let input_bytes = self
            .input_bytes
            .checked_mul(selected.len())
            .ok_or_else(|| EngineError::ResourceLimit("PDF subset size overflow".into()))?;
        if input_bytes > MAX_DOCUMENT_INPUT {
            return Err(
                EngineError::ResourceLimit("PDF subset input budget exceeded".into()).into(),
            );
        }
        let items = self
            .items
            .checked_mul(selected.len())
            .ok_or_else(|| EngineError::ResourceLimit("PDF subset item overflow".into()))?;
        if items > MAX_ITEMS {
            return Err(
                EngineError::ResourceLimit("PDF subset item budget exceeded".into()).into(),
            );
        }
        let document = pdfcraft_organize::extract_pages(self.document()?, &selected)
            .map_err(organize_error)?;
        Ok(Self {
            document: Some(document),
            sizes: selected.iter().map(|index| self.sizes[*index]).collect(),
            geometry: selected
                .iter()
                .map(|index| self.geometry[*index].clone())
                .collect(),
            backgrounds: selected
                .iter()
                .map(|index| self.backgrounds[*index])
                .collect(),
            items,
            input_bytes,
            owns_fields: self.owns_fields,
        })
    }

    #[pyo3(signature = (page=None))]
    pub fn fields(&self, page: Option<isize>) -> PyResult<String> {
        let page = page.map(|p| self.page_index(p)).transpose()?;
        let values = self
            .owned_fields()?
            .iter()
            .filter(|field| page.is_none_or(|p| field.widgets[0].page == Some(p)))
            .map(|field| self.field_payload(field))
            .collect::<PyResult<Vec<_>>>()?;
        crate::limits::bounded_json(&values, 32 * 1024 * 1024).map_err(Into::into)
    }

    pub fn field_info(&self, name: &str) -> PyResult<String> {
        let field = self.field_named(name)?;
        crate::limits::bounded_json(&self.field_payload(&field)?, 2 * 1024 * 1024)
            .map_err(Into::into)
    }

    #[pyo3(signature = (page, name, kind, rect, options=Vec::new()))]
    pub fn add_field(
        &mut self,
        page: isize,
        name: &str,
        kind: &str,
        rect: (f64, f64, f64, f64),
        options: Vec<String>,
    ) -> PyResult<String> {
        self.document()?;
        if !self.owns_fields {
            return Err(EngineError::Unsupported(
                "form creation requires a newly created PDF model".into(),
            )
            .into());
        }
        let page = self.page_index(page)?;
        validate_field_name(name)?;
        let existing = self.owned_fields()?;
        if existing.len() >= MAX_FIELDS {
            return Err(EngineError::ResourceLimit("PDF exceeds 1000 fields".into()).into());
        }
        if existing.iter().any(|f| f.name == name) {
            return Err(PyValueError::new_err("field name already exists"));
        }
        if options.len() > 256 {
            return Err(
                EngineError::ResourceLimit("choice field exceeds 256 options".into()).into(),
            );
        }
        let mut distinct = std::collections::HashSet::new();
        for option in &options {
            validate_field_text(option)?;
            if option.is_empty() || !distinct.insert(option) {
                return Err(PyValueError::new_err(
                    "choice options must be nonempty and distinct",
                ));
            }
        }
        let new = match kind {
            "text" if options.is_empty() => pdfcraft_forms::NewField::Text { multiline: false },
            "checkbox" if options.is_empty() => pdfcraft_forms::NewField::CheckBox,
            "choice" if !options.is_empty() => pdfcraft_forms::NewField::Combo { options: options.clone(), editable: false },
            _ => return Err(PyValueError::new_err("field kind must be text, checkbox, or choice; only choice fields take nonempty options")),
        };
        let cost = name.len() + options.iter().map(String::len).sum::<usize>();
        self.validate_input(cost)?;
        let (x, y, width, height) = rect;
        let view = self.annotation_rect(page, (x, y, x + width, y + height))?;
        let user_rect = self.geometry[page].view_rect_to_user(view).map(f64::from);
        let unit = self.page_user_unit(page);
        let font_size = 12.0 / unit;
        // Fixed-size appearances must have room for their border/padding and all value text.
        // Use a deliberately wider inset than the upstream default one-unit border + padding.
        if width < 12.0 * unit || height < 24.0 * unit.max(1.0) {
            return Err(PyValueError::new_err(
                "form field box is too small for its fixed appearance",
            ));
        }
        validate_field_fit("", width / unit, height / unit, font_size)?;
        for option in &options {
            validate_field_fit(option, width / unit, height / unit, font_size)?;
        }
        let mut edited = self.document()?.clone();
        let created = pdfcraft_forms::add_field(&mut edited, page, user_rect, &new, Some(name))
            .map_err(form_error)?;
        if created != name {
            return Err(EngineError::Conversion(
                "form writer changed the requested field name".into(),
            )
            .into());
        }
        pdfcraft_forms::set_props(
            &mut edited,
            name,
            &pdfcraft_forms::FieldProps {
                font_size: Some(font_size),
                rect: Some((0, user_rect)),
                rotation: Some((0, i64::from(self.geometry[page].rotation))),
                ..Default::default()
            },
        )
        .map_err(form_error)?;
        checked_owned_fields(&edited)?;
        self.document = Some(edited);
        self.input_bytes += cost;
        self.items += 1;
        Ok(name.into())
    }

    pub fn set_field_value(&mut self, name: &str, value_json: &str) -> PyResult<()> {
        use pdfcraft_forms::{FieldKind, FieldValue};
        let field = self.field_named(name)?;
        if value_json.len() > 32 * 1024 {
            return Err(
                EngineError::ResourceLimit("field value JSON exceeds 32 KiB".into()).into(),
            );
        }
        let value: Value = serde_json::from_str(value_json)
            .map_err(|_| PyValueError::new_err("invalid field value JSON"))?;
        self.validate_input(value_json.len())?;
        let native = match (field.kind, value) {
            (FieldKind::Text, Value::String(text)) => {
                validate_field_text(&text)?;
                validate_value_fit(&field, &text)?;
                FieldValue::Text(text)
            }
            (FieldKind::CheckBox, Value::Bool(value)) => FieldValue::Check(value),
            (FieldKind::Combo, Value::Null) => FieldValue::Choice(vec![]),
            (FieldKind::Combo, Value::String(text)) => {
                validate_field_text(&text)?;
                if !field.options.iter().any(|(export, _)| export == &text) {
                    return Err(PyValueError::new_err("value is not a choice option"));
                }
                validate_value_fit(&field, &text)?;
                FieldValue::Choice(vec![text])
            }
            _ => return Err(PyValueError::new_err("field value must be a string for text, bool for checkbox, or option string/null for choice")),
        };
        let mut edited = self.document()?.clone();
        pdfcraft_forms::set_value(&mut edited, name, &native).map_err(form_error)?;
        checked_owned_fields(&edited)?;
        self.document = Some(edited);
        self.input_bytes += value_json.len();
        Ok(())
    }

    pub fn set_field_box(&mut self, name: &str, rect: (f64, f64, f64, f64)) -> PyResult<()> {
        let mut expected = self.field_named(name)?;
        let page = expected.widgets[0]
            .page
            .ok_or_else(|| EngineError::Unsupported("owned field has no page".into()))?;
        let (x, y, width, height) = rect;
        let view = self.annotation_rect(page, (x, y, x + width, y + height))?;
        let unit = self.page_user_unit(page);
        if width < 12.0 * unit || height < 24.0 * unit.max(1.0) {
            return Err(PyValueError::new_err(
                "form field box is too small for its fixed appearance",
            ));
        }
        let user_rect = self.geometry[page].view_rect_to_user(view).map(f64::from);
        expected.widgets[0].rect = user_rect;
        validate_value_fit(&expected, "")?;
        for value in expected
            .value
            .iter()
            .chain(&expected.default)
            .chain(expected.options.iter().map(|(_, display)| display))
        {
            validate_value_fit(&expected, value)?;
        }
        // Resource estimate only: the bounded native writer generates the appearance.
        // Reserve before staging; charge its actual raw bytes after generation, without decoding.
        self.validate_input(owned_field_appearance_peak(&expected))?;
        let mut edited = self.document()?.clone();
        pdfcraft_forms::set_props(
            &mut edited,
            name,
            &pdfcraft_forms::FieldProps {
                rect: Some((0, user_rect)),
                ..Default::default()
            },
        )
        .map_err(form_error)?;
        let fields = checked_owned_fields(&edited)?;
        let actual = fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| EngineError::Conversion("resized field is missing".into()))?;
        if actual != &expected {
            return Err(EngineError::Conversion(
                "field geometry update changed unrelated field properties".into(),
            )
            .into());
        }
        let cost = owned_field_appearance_bytes(&edited, actual)?.saturating_add(32);
        self.validate_input(cost)?;
        self.document = Some(edited);
        self.input_bytes += cost;
        self.items += 1;
        Ok(())
    }

    pub fn set_field_tooltip(&mut self, name: &str, tooltip: &str) -> PyResult<()> {
        let mut expected = self.field_named(name)?;
        validate_annotation_text(tooltip, 4096, "field tooltip")?;
        // The native metadata setter also regenerates the owned appearance.
        let metadata_cost = tooltip.len().saturating_mul(2).saturating_add(32);
        self.validate_input(owned_field_appearance_peak(&expected).saturating_add(metadata_cost))?;
        expected.tooltip = (!tooltip.is_empty()).then(|| tooltip.to_owned());
        let mut edited = self.document()?.clone();
        pdfcraft_forms::set_props(
            &mut edited,
            name,
            &pdfcraft_forms::FieldProps {
                tooltip: Some(tooltip.to_owned()),
                ..Default::default()
            },
        )
        .map_err(form_error)?;
        let fields = checked_owned_fields(&edited)?;
        let actual = fields
            .iter()
            .find(|field| field.name == name)
            .ok_or_else(|| {
                EngineError::Conversion("field is missing after its tooltip update".into())
            })?;
        if actual != &expected {
            return Err(EngineError::Conversion(
                "field tooltip update changed unrelated field properties".into(),
            )
            .into());
        }
        let cost = owned_field_appearance_bytes(&edited, actual)?.saturating_add(metadata_cost);
        self.validate_input(cost)?;
        self.document = Some(edited);
        self.input_bytes += cost;
        self.items += 1;
        Ok(())
    }

    pub fn delete_field(&mut self, name: &str) -> PyResult<()> {
        self.field_named(name)?;
        let mut edited = self.document()?.clone();
        pdfcraft_forms::delete_field(&mut edited, name).map_err(form_error)?;
        if checked_owned_fields(&edited)?
            .iter()
            .any(|f| f.name == name)
        {
            return Err(
                EngineError::Conversion("deleted form field remains reachable".into()).into(),
            );
        }
        self.document = Some(edited);
        Ok(())
    }

    pub fn flatten_fields(&mut self) -> PyResult<usize> {
        let fields = self.owned_fields()?;
        if fields.is_empty() {
            return Ok(0);
        }
        for field in &fields {
            for value in &field.value {
                validate_value_fit(field, value)?;
            }
        }
        let pages = (0..self.sizes.len()).collect::<Vec<_>>();
        let mut edited = self.document()?.clone();
        let flattened =
            pdfcraft_edit::flatten(&mut edited, &pages, false, true).map_err(edit_error)?;
        if flattened != fields.len()
            || !checked_owned_fields(&edited)?.is_empty()
            || has_widgets(&edited)?
        {
            return Err(EngineError::Unsupported(
                "not every field appearance could be flattened; document unchanged".into(),
            )
            .into());
        }
        self.document = Some(edited);
        Ok(flattened)
    }

    #[pyo3(signature = (page, rect, uri=None, target_page=None))]
    pub fn add_link(
        &mut self,
        page: isize,
        rect: (f64, f64, f64, f64),
        uri: Option<&str>,
        target_page: Option<isize>,
    ) -> PyResult<usize> {
        let page = self.page_index(page)?;
        let rect = self.annotation_rect(page, rect)?;
        let action = self.link_action(uri, target_page)?;
        let cost = uri.map_or(0, str::len);
        self.validate_input(cost)?;
        let mut edited = self.document()?.clone();
        let index = pdfcraft_annot::links::add(
            &mut edited,
            page,
            self.geometry[page].view_rect_to_user(rect).map(f64::from),
            &action,
            &pdfcraft_annot::links::LinkStyle::default(),
        )
        .map_err(annotation_error)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += cost;
        Ok(index)
    }

    #[pyo3(signature = (page, kind, rect, text="", author="", color=(255, 210, 0)))]
    #[allow(clippy::too_many_arguments)]
    pub fn add_annotation(
        &mut self,
        page: isize,
        kind: &str,
        rect: (f64, f64, f64, f64),
        text: &str,
        author: &str,
        color: (u8, u8, u8),
    ) -> PyResult<usize> {
        let page = self.page_index(page)?;
        let rect = self.annotation_rect(page, rect)?;
        validate_annotation_text(text, 65_536, "annotation text")?;
        validate_annotation_text(author, 4096, "annotation author")?;
        self.validate_input(text.len() + author.len())?;
        let shape = match kind {
            "note" => {
                let user = self.geometry[page].view_rect_to_user(rect).map(f64::from);
                if (user[2] - user[0] - pdfcraft_annot::NOTE_SIZE).abs() > 0.001
                    || (user[3] - user[1] - pdfcraft_annot::NOTE_SIZE).abs() > 0.001
                {
                    return Err(PyValueError::new_err(
                        "sticky note boxes must be 20 by 20 PDF user units",
                    ));
                }
                pdfcraft_annot::Shape::Note {
                    at: [user[0], user[3]],
                    icon: pdfcraft_annot::NoteIcon::Note,
                }
            }
            "rectangle" | "ellipse" => {
                let rect = self.geometry[page].view_rect_to_user(rect).map(f64::from);
                validate_geometric_rect(rect)?;
                if kind == "rectangle" {
                    pdfcraft_annot::Shape::Rectangle { rect }
                } else {
                    pdfcraft_annot::Shape::Oval { rect }
                }
            }
            "highlight" | "underline" | "strikeout" | "squiggly" => pdfcraft_annot::Shape::TextMarkup {
                kind: match kind {
                    "underline" => pdfcraft_annot::Markup::Underline,
                    "strikeout" => pdfcraft_annot::Markup::StrikeOut,
                    "squiggly" => pdfcraft_annot::Markup::Squiggly,
                    _ => pdfcraft_annot::Markup::Highlight,
                },
                quads: vec![self.geometry[page].view_rect_to_quad(rect)],
            },
            _ => {
                return Err(EngineError::Unsupported(format!(
                    "annotation kind {kind}; supported kinds are note, highlight, underline, strikeout, squiggly, rectangle and ellipse"
                ))
                .into())
            }
        };
        let appearance_peak = match &shape {
            pdfcraft_annot::Shape::Rectangle { .. } | pdfcraft_annot::Shape::Oval { .. } => {
                GEOMETRIC_APPEARANCE_WORK
            }
            pdfcraft_annot::Shape::TextMarkup { kind, quads } => {
                quads.iter().try_fold(4096usize, |total, quad| {
                    markup_quad_peak(quad, *kind == pdfcraft_annot::Markup::Squiggly)
                        .map(|size| total.saturating_add(size))
                })?
            }
            _ => 0,
        };
        self.validate_input(text.len() + author.len() + appearance_peak)?;
        let mut style = pdfcraft_annot::Style::default_for(&shape);
        style.color = rgb(color);
        let annotation = pdfcraft_annot::NewAnnotation {
            page,
            shape,
            style,
            contents: text.into(),
            author: author.into(),
        };
        let names = pdfcraft_annot::summaries(self.document()?)
            .into_iter()
            .filter_map(|annotation| annotation.name)
            .collect::<std::collections::HashSet<_>>();
        let mut suffix = self.items;
        while names.contains(&format!("pydoccraft-annotation-{suffix}")) {
            suffix += 1;
        }
        let meta = pdfcraft_annot::Meta {
            date: None,
            id: format!("pydoccraft-annotation-{suffix}"),
        };
        let mut edited = self.document()?.clone();
        let index = pdfcraft_annot::add_annotation(&mut edited, &annotation, &meta)
            .map_err(annotation_error)?;
        let appearance_bytes = if appearance_peak != 0 {
            annotation_appearance_bytes(&edited, page, index)?
        } else {
            0
        };
        if matches!(kind, "rectangle" | "ellipse") && appearance_bytes > GEOMETRIC_APPEARANCE_WORK {
            return Err(EngineError::ResourceLimit(
                "native geometric annotation appearance exceeded its bound".into(),
            )
            .into());
        }
        let cost = text.len() + author.len() + appearance_bytes;
        self.validate_input(cost)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += cost;
        Ok(index)
    }

    pub fn annotations(&self, page: isize) -> PyResult<String> {
        let page = self.page_index(page)?;
        let geometry = &self.geometry[page];
        let mut values: Vec<(usize, Value)> = Vec::new();
        for annotation in pdfcraft_annot::summaries(self.document()?)
            .into_iter()
            .filter(|annotation| annotation.page == page)
        {
            let kind = match annotation.subtype.as_str() {
                "Text" => "note".to_string(),
                "Highlight" => "highlight".to_string(),
                "Square" => "rectangle".to_string(),
                "Circle" => "ellipse".to_string(),
                other => other.to_ascii_lowercase(),
            };
            values.push((annotation.index, json!({
                "index": annotation.index, "kind": kind, "text": annotation.contents.unwrap_or_default(),
                "author": annotation.author.unwrap_or_default(), "color": annotation.color.map(|color| color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)),
                "rect": view_rect(geometry, annotation.rect), "uri": null, "target_page": null, "locked": annotation.locked,
            })));
        }
        for link in pdfcraft_annot::links::list(self.document()?)
            .into_iter()
            .filter(|link| link.page == page)
        {
            let (uri, target) = match link.action {
                pdfcraft_annot::links::LinkAction::Uri(uri) => (Some(uri), None),
                pdfcraft_annot::links::LinkAction::Page(target) => (None, Some(target)),
                pdfcraft_annot::links::LinkAction::Other(_) => (None, None),
            };
            values.push((link.index, json!({
                "index": link.index, "kind": "link", "text": "", "author": "",
                "color": link.style.color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8),
                "rect": view_rect(geometry, link.rect.map(|value| value as f32)), "uri": uri, "target_page": target,
            })));
        }
        if values.len() > MAX_ITEMS {
            return Err(
                EngineError::ResourceLimit("PDF annotation count exceeds 10000".into()).into(),
            );
        }
        values.sort_by_key(|(index, _)| *index);
        let values = values
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        let payload = serde_json::to_string(&values).map_err(|error| {
            EngineError::Malformed(format!("annotation serialization failed: {error}"))
        })?;
        if payload.len() > 16 * 1024 * 1024 {
            return Err(
                EngineError::ResourceLimit("PDF annotation payload exceeds 16 MiB".into()).into(),
            );
        }
        Ok(payload)
    }

    #[pyo3(signature = (page, index, text=None, rect=None, color=None, uri=None, target_page=None, author=None))]
    #[allow(clippy::too_many_arguments)]
    pub fn update_annotation(
        &mut self,
        page: isize,
        index: usize,
        text: Option<&str>,
        rect: Option<(f64, f64, f64, f64)>,
        color: Option<(u8, u8, u8)>,
        uri: Option<&str>,
        target_page: Option<isize>,
        author: Option<&str>,
    ) -> PyResult<()> {
        let page = self.page_index(page)?;
        // props() builds an appearance internally. Bound accepted geometry and
        // imported style dictionaries before calling that native helper.
        let appearance_peak = with_annotation_dict(self.document()?, page, index, |dict| {
            let geometric = geometric_dict_peak(self.document()?, dict)?;
            Ok(geometric.max(markup_dict_peak(dict)?))
        })?;
        self.validate_input(appearance_peak)?;
        let props = pdfcraft_annot::props(self.document()?, page, index)
            .ok_or_else(|| PyIndexError::new_err("annotation index out of range"))?;
        let user_rect = rect
            .map(|rect| {
                self.annotation_rect(page, rect)
                    .map(|rect| self.geometry[page].view_rect_to_user(rect).map(f64::from))
            })
            .transpose()?;
        let action = if uri.is_some() || target_page.is_some() {
            Some(self.link_action(uri, target_page)?)
        } else {
            None
        };
        if let Some(text) = text {
            validate_annotation_text(text, 65_536, "annotation text")?;
        }
        if let Some(author) = author {
            validate_annotation_text(author, 4096, "annotation author")?;
        }
        let cost = text.map_or(0, str::len) + author.map_or(0, str::len) + uri.map_or(0, str::len);
        self.validate_input(cost.saturating_add(appearance_peak))?;
        let mut edited = self.document()?.clone();
        let mut resized_appearance_bytes = 0usize;
        if props.subtype == "Link" {
            if text.is_some() || author.is_some() {
                return Err(EngineError::Unsupported(
                    "link annotations do not have editable comment text or author".into(),
                )
                .into());
            }
            let mut style = pdfcraft_annot::links::list(self.document()?)
                .into_iter()
                .find(|link| link.page == page && link.index == index)
                .ok_or_else(|| PyIndexError::new_err("link annotation index out of range"))?
                .style;
            if let Some(color) = color {
                style.color = rgb(color);
            }
            pdfcraft_annot::links::set(
                &mut edited,
                page,
                index,
                user_rect,
                action.as_ref(),
                color.map(|_| &style),
            )
            .map_err(annotation_error)?;
        } else if matches!(
            props.subtype.as_str(),
            "Text" | "Highlight" | "Underline" | "StrikeOut" | "Squiggly" | "Square" | "Circle"
        ) {
            if action.is_some() {
                return Err(EngineError::Unsupported(
                    "comment annotations do not have link targets".into(),
                )
                .into());
            }
            let meta = pdfcraft_annot::Meta::default();
            if let Some(rect) = user_rect {
                if matches!(props.subtype.as_str(), "Square" | "Circle") {
                    validate_geometric_rect(rect)?;
                    pdfcraft_annot::set_rect(&mut edited, page, index, rect, &meta)
                        .map_err(annotation_error)?;
                    // A later recolor allocates another native AP object. Charge
                    // this intermediate stream even when it is replaced.
                    resized_appearance_bytes = annotation_appearance_bytes(&edited, page, index)?;
                } else {
                    let original = pdfcraft_annot::summaries(self.document()?)
                        .into_iter()
                        .find(|annotation| annotation.page == page && annotation.index == index)
                        .ok_or_else(|| PyIndexError::new_err("annotation index out of range"))?
                        .rect
                        .map(f64::from);
                    if ((rect[2] - rect[0]) - (original[2] - original[0])).abs() > 0.001
                        || ((rect[3] - rect[1]) - (original[3] - original[1])).abs() > 0.001
                    {
                        return Err(EngineError::Unsupported(
                        "sticky notes and text markup can be moved, but resizing is not available"
                            .into(),
                    )
                    .into());
                    }
                    pdfcraft_annot::move_annotation(
                        &mut edited,
                        page,
                        index,
                        rect[0] - original[0],
                        rect[1] - original[1],
                        &meta,
                    )
                    .map_err(annotation_error)?;
                }
            }
            if let Some(text) = text {
                pdfcraft_annot::set_contents(&mut edited, page, index, text, &meta)
                    .map_err(annotation_error)?;
            }
            if let Some(author) = author {
                pdfcraft_annot::set_info(&mut edited, page, index, Some(author), None, None, &meta)
                    .map_err(annotation_error)?;
            }
            if let Some(color) = color {
                pdfcraft_annot::set_style(
                    &mut edited,
                    page,
                    index,
                    Some(rgb(color)),
                    None,
                    None,
                    None,
                    &meta,
                )
                .map_err(annotation_error)?;
            }
        } else {
            return Err(EngineError::Unsupported(format!(
                "editing {} annotations is not available",
                props.subtype
            ))
            .into());
        }
        // Color changes and geometric resizing regenerate the appearance.
        let geometric = matches!(props.subtype.as_str(), "Square" | "Circle");
        let appearance_bytes = if appearance_peak != 0 && color.is_some() {
            annotation_appearance_bytes(&edited, page, index)?
        } else {
            0
        };
        let appearance_bytes = appearance_bytes.saturating_add(resized_appearance_bytes);
        if geometric && appearance_bytes > GEOMETRIC_APPEARANCE_WORK {
            return Err(EngineError::ResourceLimit(
                "native geometric annotation appearance exceeded its bound".into(),
            )
            .into());
        }
        let cost = cost.saturating_add(appearance_bytes);
        self.validate_input(cost)?;
        self.document = Some(edited);
        self.input_bytes += cost;
        self.items += 1;
        Ok(())
    }

    pub fn set_annotation_locked(
        &mut self,
        page: isize,
        index: usize,
        locked: bool,
    ) -> PyResult<()> {
        use pdfcraft_cos::Object;
        let page = self.page_index(page)?;
        let (reference, peak) = checked_lock_target(self.document()?, page, index)?;
        self.validate_input(peak)?;
        let original = self.document()?.get(reference);
        let mut expected = original
            .as_dict()
            .ok_or_else(|| {
                EngineError::Conversion("validated annotation dictionary is missing".into())
            })?
            .clone();
        let flags = expected.get(b"F").and_then(Object::as_int).ok_or_else(|| {
            EngineError::Conversion("validated annotation flags are missing".into())
        })?;
        let wanted = if locked { flags | 128 } else { flags & !128 };
        if flags == wanted {
            return Ok(());
        }
        expected.set(b"F".to_vec(), Object::Int(wanted));
        let mut edited = self.document()?.clone();
        pdfcraft_annot::set_locked(&mut edited, page, index, locked).map_err(annotation_error)?;
        // The bounded canonical dictionary contains no deep direct graphs. The
        // native operation must preserve every entry except the Locked bit,
        // including the AP reference; it never regenerates that appearance.
        if edited.get(reference).as_dict() != Some(&expected) {
            return Err(EngineError::Conversion(
                "native annotation lock changed unrelated content".into(),
            )
            .into());
        }
        self.document = Some(edited);
        self.input_bytes += 16;
        self.items += 1;
        Ok(())
    }

    pub fn delete_annotation(&mut self, page: isize, index: usize) -> PyResult<()> {
        let page = self.page_index(page)?;
        let supported = with_annotation_dict(self.document()?, page, index, |dict| {
            Ok(dict
                .name(b"Subtype")
                .and_then(|name| std::str::from_utf8(name).ok())
                .is_some_and(|subtype| {
                    pdfcraft_annot::is_comment_subtype(subtype) || subtype == "Link"
                }))
        })?;
        if !supported {
            return Err(EngineError::Unsupported(
                "deleting this annotation subtype is not available".into(),
            )
            .into());
        }
        let mut edited = self.document()?.clone();
        pdfcraft_annot::delete_annotation(&mut edited, page, index).map_err(annotation_error)?;
        self.document = Some(edited);
        Ok(())
    }

    /// Known backgrounds are only those managed in this editable document's lifetime.
    pub fn background(&self, page: isize) -> PyResult<Option<(u8, u8, u8)>> {
        match self.backgrounds[self.page_index(page)?] {
            BackgroundState::Known(color) => Ok(color),
            BackgroundState::Unknown => Err(EngineError::Unsupported(
                "the background is unknown after import or geometry changes; set a background to manage it".into(),
            ).into()),
        }
    }

    pub fn set_background(&mut self, page: isize, color: (u8, u8, u8)) -> PyResult<()> {
        let page = self.page_index(page)?;
        self.validate_input(0)?;
        let mut edited = self.document()?.clone();
        let background = pdfcraft_edit::Background {
            color: [
                f64::from(color.0) / 255.0,
                f64::from(color.1) / 255.0,
                f64::from(color.2) / 255.0,
            ],
            ..pdfcraft_edit::Background::default()
        };
        pdfcraft_edit::add_background(&mut edited, &[page], &background, true)
            .map_err(edit_error)?;
        preserve_drawing_resources(self.document()?, &edited, page)?;
        self.document = Some(edited);
        self.backgrounds[page] = BackgroundState::Known(Some(color));
        self.items += 1;
        Ok(())
    }

    pub fn metadata(&self) -> PyResult<String> {
        let document = self.document()?;
        let values = metadata_keys()
            .into_iter()
            .filter_map(|(key, pdf_key)| {
                pdfcraft_organize::info(document, pdf_key).map(|value| (key, value))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        serde_json::to_string(&values).map_err(|error| {
            EngineError::Malformed(format!("metadata serialization failed: {error}")).into()
        })
    }

    pub fn update_metadata(&mut self, updates: &str) -> PyResult<()> {
        self.document()?;
        if updates.len() > 262_144 {
            return Err(
                EngineError::ResourceLimit("metadata update exceeds 256 KiB".into()).into(),
            );
        }
        let values: serde_json::Map<String, Value> = serde_json::from_str(updates)
            .map_err(|_| PyValueError::new_err("metadata updates must be an object"))?;
        let mut changes = Vec::new();
        for (key, value) in &values {
            let pdf_key = metadata_keys()
                .into_iter()
                .find(|(name, _)| *name == key)
                .map(|(_, name)| name)
                .ok_or_else(|| PyValueError::new_err(format!("unknown metadata key: {key}")))?;
            let text = match value {
                Value::Null => "",
                Value::String(value) => value,
                _ => {
                    return Err(PyValueError::new_err(
                        "metadata values must be strings or null",
                    ))
                }
            };
            validate_annotation_text(text, 65_536, "metadata value")?;
            changes.push((pdf_key, text));
        }
        if changes.is_empty() {
            return Ok(());
        }
        self.validate_input(updates.len())?;
        let mut edited = self.document()?.clone();
        for (key, text) in changes {
            pdfcraft_organize::set_info(&mut edited, key, text).map_err(organize_error)?;
        }
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += updates.len();
        Ok(())
    }

    pub fn bookmarks(&self) -> PyResult<String> {
        let entries = self.bookmark_entries()?.into_iter().map(|entry| json!({"path":entry.path,"title":entry.title,"page":entry.page,"open":entry.open})).collect::<Vec<_>>();
        let payload = serde_json::to_string(&entries).map_err(|error| {
            EngineError::Malformed(format!("bookmark serialization failed: {error}"))
        })?;
        if payload.len() > 16 * 1024 * 1024 {
            return Err(
                EngineError::ResourceLimit("bookmark payload exceeds 16 MiB".into()).into(),
            );
        }
        Ok(payload)
    }

    #[pyo3(signature = (title, page, parent, index=None))]
    pub fn add_bookmark(
        &mut self,
        title: &str,
        page: isize,
        parent: Vec<usize>,
        index: Option<usize>,
    ) -> PyResult<Vec<usize>> {
        let page = self.page_index(page)?;
        validate_bookmark_title(title)?;
        self.validate_input(title.len())?;
        let entries = self.bookmark_entries()?;
        validate_bookmark_path(&entries, &parent, true)?;
        if parent.len() >= 32 {
            return Err(EngineError::ResourceLimit("bookmark depth exceeds 32".into()).into());
        }
        if entries.len() >= MAX_ITEMS {
            return Err(EngineError::ResourceLimit("bookmark count exceeds 10000".into()).into());
        }
        let count = bookmark_children(&entries, &parent);
        let index = index.unwrap_or(count);
        if index > count {
            return Err(PyIndexError::new_err(
                "bookmark insertion index out of range",
            ));
        }
        let mut edited = self.document()?.clone();
        let path = pdfcraft_organize::add_bookmark(&mut edited, &parent, index, title, page)
            .map_err(outline_error)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += title.len();
        Ok(path)
    }

    #[pyo3(signature = (path, title=None, page=None, open=None))]
    pub fn update_bookmark(
        &mut self,
        path: Vec<usize>,
        title: Option<&str>,
        page: Option<isize>,
        open: Option<bool>,
    ) -> PyResult<()> {
        let entries = self.bookmark_entries()?;
        validate_bookmark_path(&entries, &path, false)?;
        if let Some(title) = title {
            validate_bookmark_title(title)?;
        }
        let page = page.map(|page| self.page_index(page)).transpose()?;
        self.validate_input(title.map_or(0, str::len))?;
        let mut edited = self.document()?.clone();
        if let Some(title) = title {
            pdfcraft_organize::rename_bookmark(&mut edited, &path, title).map_err(outline_error)?;
        }
        if let Some(page) = page {
            pdfcraft_organize::set_bookmark_page(&mut edited, &path, page)
                .map_err(outline_error)?;
        }
        if let Some(open) = open {
            pdfcraft_organize::set_bookmark_open(&mut edited, &path, open)
                .map_err(outline_error)?;
        }
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += title.map_or(0, str::len);
        Ok(())
    }

    pub fn delete_bookmark(&mut self, path: Vec<usize>) -> PyResult<()> {
        validate_bookmark_path(&self.bookmark_entries()?, &path, false)?;
        let mut edited = self.document()?.clone();
        pdfcraft_organize::delete_bookmark(&mut edited, &path).map_err(outline_error)?;
        self.document = Some(edited);
        Ok(())
    }

    pub fn move_bookmark(
        &mut self,
        path: Vec<usize>,
        parent: Vec<usize>,
        index: usize,
    ) -> PyResult<Vec<usize>> {
        let entries = self.bookmark_entries()?;
        validate_bookmark_path(&entries, &path, false)?;
        validate_bookmark_path(&entries, &parent, true)?;
        if parent.starts_with(&path) {
            return Err(PyValueError::new_err(
                "bookmark cannot be moved inside itself",
            ));
        }
        let subtree_depth = entries
            .iter()
            .filter(|entry| entry.path.starts_with(&path))
            .map(|entry| entry.path.len() - path.len())
            .max()
            .unwrap_or(0);
        if parent.len() + 1 + subtree_depth > 32 {
            return Err(EngineError::ResourceLimit("bookmark depth exceeds 32".into()).into());
        }
        let siblings = bookmark_children(&entries, &parent);
        let same_parent = path[..path.len() - 1] == parent;
        let count = siblings.saturating_sub(usize::from(same_parent));
        if index > count {
            return Err(PyIndexError::new_err(
                "bookmark destination index out of range",
            ));
        }
        let mut edited = self.document()?.clone();
        let moved = pdfcraft_organize::move_bookmark(&mut edited, &path, &parent, index)
            .map_err(outline_error)?;
        self.document = Some(edited);
        Ok(moved)
    }

    /// Draw bounded typed path commands; the backend serializes and inserts page content.
    #[pyo3(signature = (page, commands, stroke=None, fill=None, width=1.0, close=false))]
    #[allow(clippy::too_many_arguments)]
    pub fn draw_path(
        &mut self,
        page: isize,
        commands: &str,
        stroke: Option<(u8, u8, u8)>,
        fill: Option<(u8, u8, u8)>,
        width: f64,
        close: bool,
    ) -> PyResult<()> {
        use pdfcraft_cos::Object;
        let page = self.page_index(page)?;
        if commands.len() > 262_144 {
            return Err(EngineError::ResourceLimit("path commands exceed 256 KiB".into()).into());
        }
        self.validate_input(commands.len())?;
        if stroke.is_none() && fill.is_none() {
            return Err(PyValueError::new_err("path requires a stroke or fill"));
        }
        if !width.is_finite() || !(0.01..=500.0).contains(&width) {
            return Err(PyValueError::new_err(
                "stroke width must be finite and between 0.01 and 500 points",
            ));
        }
        let commands: Vec<Vec<Value>> = serde_json::from_str(commands).map_err(|_| {
            PyValueError::new_err("commands must be arrays beginning with move, line or curve")
        })?;
        if commands.is_empty() || commands.len() > 10_000 {
            return Err(PyValueError::new_err(
                "path must contain 1 to 10000 commands",
            ));
        }
        let mut ops = vec![
            Op::new("q", vec![]),
            Op::new(
                "cm",
                view_matrix(&self.geometry[page]).map(Object::Real).to_vec(),
            ),
        ];
        let half = if stroke.is_some() { width / 2.0 } else { 0.0 };
        if let Some(color) = stroke {
            ops.push(Op::new("RG", rgb(color).map(Object::Real).to_vec()));
            ops.push(Op::new("w", vec![Object::Real(width)]));
            // Round joins/caps stay inside the checked control-point hull plus half-width.
            ops.push(Op::new("J", vec![Object::Int(1)]));
            ops.push(Op::new("j", vec![Object::Int(1)]));
        }
        if let Some(color) = fill {
            ops.push(Op::new("rg", rgb(color).map(Object::Real).to_vec()));
        }
        let mut has_segment = false;
        let mut started = false;
        for command in &commands {
            let kind = command
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| PyValueError::new_err("path command must begin with a name"))?;
            let (operator, count) = match kind {
                "move" => ("m", 2),
                "line" => ("l", 2),
                "curve" => ("c", 6),
                _ => {
                    return Err(PyValueError::new_err(
                        "path commands must be move, line or curve",
                    ))
                }
            };
            if command.len() != count + 1 || (!started && kind != "move") {
                return Err(PyValueError::new_err(
                    "path must start with move; move/line need two coordinates and curve needs six",
                ));
            }
            let values = command[1..]
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| {
                            PyValueError::new_err("path coordinates must be finite numbers")
                        })
                })
                .collect::<PyResult<Vec<_>>>()?;
            let (page_width, page_height) = self.sizes[page];
            for point in values.chunks_exact(2) {
                if point[0] < half
                    || point[1] < half
                    || point[0] > page_width - half
                    || point[1] > page_height - half
                {
                    return Err(PyValueError::new_err(
                        "path and stroke must fit inside the page",
                    ));
                }
            }
            has_segment |= kind != "move";
            started = true;
            ops.push(Op::new(
                operator,
                values.into_iter().map(Object::Real).collect(),
            ));
        }
        if !has_segment {
            return Err(PyValueError::new_err("path needs a line or curve segment"));
        }
        if close {
            ops.push(Op::new("h", vec![]));
        }
        ops.push(Op::new(
            match (stroke.is_some(), fill.is_some()) {
                (true, true) => "B",
                (true, false) => "S",
                _ => "f",
            },
            vec![],
        ));
        ops.push(Op::new("Q", vec![]));
        let content = serialize_ops(&ops);
        let cost = content.len();
        self.validate_input(cost)?;
        let mut edited = self.document()?.clone();
        pdfcraft_edit::stamp(&mut edited, page, "pydoccraft-path", content).map_err(edit_error)?;
        preserve_drawing_resources(self.document()?, &edited, page)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += cost;
        Ok(())
    }

    /// Explicit rows with a caller-supplied embeddable font; no system-font lookup or fallback.
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (page, text, x, y, size, font_bytes, color=(0, 0, 0)))]
    pub fn add_text_with_font(
        &mut self,
        page: isize,
        text: &str,
        x: f64,
        y: f64,
        size: f64,
        font_bytes: &[u8],
        color: (u8, u8, u8),
    ) -> PyResult<()> {
        self.add_text_font_layout(
            page,
            text,
            x,
            y,
            size,
            font_bytes,
            color,
            None,
            pdfcraft_edit::Align::Left,
        )
    }

    /// x/y are the top-left of a text box, in points. Text wraps to the page's right edge.
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (page, text, x, y, size=12.0, color=(0, 0, 0)))]
    pub fn add_text(
        &mut self,
        page: isize,
        text: &str,
        x: f64,
        y: f64,
        size: f64,
        color: (u8, u8, u8),
    ) -> PyResult<()> {
        self.add_text_layout(
            page,
            text,
            x,
            y,
            size,
            color,
            None,
            pdfcraft_edit::Align::Left,
        )
    }

    /// A bounded native text area; caller fonts retain explicit-line layout only.
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (page, text, rect, size=12.0, color=(0,0,0), align="left", font_bytes=None))]
    pub fn add_text_box(
        &mut self,
        page: isize,
        text: &str,
        rect: (f64, f64, f64, f64),
        size: f64,
        color: (u8, u8, u8),
        align: &str,
        font_bytes: Option<&[u8]>,
    ) -> PyResult<()> {
        let alignment = match align {
            "left" => pdfcraft_edit::Align::Left,
            "center" => pdfcraft_edit::Align::Center,
            "right" => pdfcraft_edit::Align::Right,
            "justify" => pdfcraft_edit::Align::Justify,
            _ => {
                return Err(PyValueError::new_err(
                    "alignment must be left, center, right or justify",
                ))
            }
        };
        match font_bytes {
            Some(font) => {
                if alignment == pdfcraft_edit::Align::Justify {
                    return Err(EngineError::Unsupported("justification with supplied fonts is unavailable; use explicit left, center or right aligned lines".into()).into());
                }
                self.add_text_font_layout(
                    page,
                    text,
                    rect.0,
                    rect.1,
                    size,
                    font,
                    color,
                    Some((rect.2, rect.3)),
                    alignment,
                )
            }
            None => self.add_text_layout(
                page,
                text,
                rect.0,
                rect.1,
                size,
                color,
                Some((rect.2, rect.3)),
                alignment,
            ),
        }
    }

    /// Place PNG/JPEG bytes at a top-left position without resampling source pixels.
    #[allow(clippy::too_many_arguments)] // The narrow native boundary receives flattened geometry.
    #[pyo3(signature = (page,data,x,y,width,height,crop=None,rotation=0,flip_h=false,flip_v=false))]
    pub fn add_image(
        &mut self,
        page: isize,
        data: &[u8],
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        crop: Option<(f64, f64, f64, f64)>,
        rotation: i64,
        flip_h: bool,
        flip_v: bool,
    ) -> PyResult<()> {
        let (left, top, right, bottom) = crop.unwrap_or((0.0, 0.0, 0.0, 0.0));
        let crop = [left, bottom, right, top];
        if crop
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=0.45).contains(value))
        {
            return Err(PyValueError::new_err(
                "image crop fractions must be finite and between 0 and 0.45",
            ));
        }
        if !matches!(rotation, 0 | 90 | 180 | 270) {
            return Err(PyValueError::new_err(
                "image rotation must be 0, 90, 180 or 270 degrees counterclockwise",
            ));
        }
        let page = self.page_index(page)?;
        self.validate_input(data.len())?;
        let (page_width, page_height) = self.sizes[page];
        validate_position(x, y, page_width, page_height)?;
        if !width.is_finite()
            || !height.is_finite()
            || width < 1.0
            || height < 1.0
            || x + width > page_width
            || y + height > page_height
        {
            return Err(PyValueError::new_err(
                "image must fit inside the page with sides at least 1 point",
            ));
        }
        if data.len() > MAX_IMAGE_BYTES {
            return Err(EngineError::ResourceLimit("image input exceeds 64 MiB".into()).into());
        }
        let name = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            validate_png(data)?;
            "image.png"
        } else if data.starts_with(&[0xff, 0xd8]) {
            "image.jpg"
        } else {
            return Err(EngineError::Unsupported(
                "PDF image insertion accepts PNG or JPEG bytes".into(),
            )
            .into());
        };
        let mut edited = self.document()?.clone();
        let (image, _) =
            pdfcraft_create::image_xobject(&mut edited, name, data).map_err(creation_error)?;
        // PdfCraft embeds JPEG bytes without decoding. Check its parsed image metadata before
        // committing, so a huge JPEG frame never reaches a later render through this API.
        let image_object = edited.get(image);
        let dimensions = image_object
            .as_dict()
            .and_then(|image| Some((image.int(b"Width")?, image.int(b"Height")?)))
            .ok_or_else(|| EngineError::Malformed("image has no usable pixel dimensions".into()))?;
        crate::rendering::validate_size(dimensions.0 as f64, dimensions.1 as f64, 16_000)?;
        let rect = [x, page_height - y - height, x + width, page_height - y];
        pdfcraft_edit::add_content(
            &mut edited,
            page,
            &Content::Image(AddedImage {
                crop,
                rotation: (rotation / 90) as u8,
                flip_h,
                flip_v,
                ..AddedImage::new(rect, image)
            }),
        )
        .map_err(edit_error)?;
        preserve_drawing_resources(self.document()?, &edited, page)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += data.len();
        Ok(())
    }
}

// Only boxed caller-font text needs outline containment. Point-positioned text retains
// its existing behavior. All parsing, composite transforms and curve geometry remain native.
const MAX_INK_GLYPHS: usize = 4096;
const MAX_INK_VISITS: usize = 100_000;
const MAX_INK_POINTS: usize = 8192;
const MAX_INK_OPS: usize = 100_000;
const MAX_INK_SCRATCH: usize = 1024 * 1024;

struct InkPen {
    path: kurbo::BezPath,
    remaining: usize,
    failed: bool,
}

impl InkPen {
    fn push(&mut self, element: kurbo::PathEl) {
        if self.remaining == 0 || !element.is_finite() {
            self.failed = true;
            return;
        }
        self.remaining -= 1;
        self.path.push(element);
    }
}

impl skrifa::outline::OutlinePen for InkPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.push(kurbo::PathEl::MoveTo((f64::from(x), f64::from(y)).into()));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(kurbo::PathEl::LineTo((f64::from(x), f64::from(y)).into()));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.push(kurbo::PathEl::QuadTo(
            (f64::from(cx), f64::from(cy)).into(),
            (f64::from(x), f64::from(y)).into(),
        ));
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        self.push(kurbo::PathEl::CurveTo(
            (f64::from(ax), f64::from(ay)).into(),
            (f64::from(bx), f64::from(by)).into(),
            (f64::from(x), f64::from(y)).into(),
        ));
    }
    fn close(&mut self) {
        self.push(kurbo::PathEl::ClosePath);
    }
}

struct TextInk<'a> {
    font: skrifa::FontRef<'a>,
    scale: f64,
    cache: std::collections::HashMap<u32, Option<kurbo::Rect>>,
    visits: usize,
    ops: usize,
}

fn ink_limit() -> EngineError {
    EngineError::ResourceLimit(
        "boxed font outline work exceeds its bounded glyph, point, operation or memory budget"
            .into(),
    )
}

impl<'a> TextInk<'a> {
    fn new(bytes: &'a [u8], size: f64) -> Result<Self, EngineError> {
        use skrifa::{
            instance::{LocationRef, Size},
            MetadataProvider,
        };
        let font = skrifa::FontRef::new(bytes)
            .map_err(|e| EngineError::Unsupported(format!("font outlines are unavailable: {e}")))?;
        if font.outline_glyphs().format() != Some(skrifa::outline::OutlineGlyphFormat::Glyf) {
            return Err(EngineError::Unsupported("boxed supplied-font text requires static TrueType outlines; bounded CFF outline evaluation is unavailable".into()));
        }
        let units = font
            .metrics(Size::unscaled(), LocationRef::default())
            .units_per_em;
        if units == 0 {
            return Err(EngineError::Unsupported(
                "font has invalid outline units".into(),
            ));
        }
        Ok(Self {
            font,
            scale: size / f64::from(units),
            cache: Default::default(),
            visits: 0,
            ops: 0,
        })
    }

    fn bounds(&mut self, id: skrifa::GlyphId) -> Result<Option<kurbo::Rect>, EngineError> {
        use kurbo::Shape;
        use skrifa::instance::{LocationRef, Size};
        use skrifa::outline::{DrawSettings, Hinting};
        use skrifa::{raw::TableProvider, MetadataProvider};
        if let Some(bounds) = self.cache.get(&id.to_u32()) {
            return Ok(*bounds);
        }
        if self.cache.len() >= MAX_INK_GLYPHS {
            return Err(ink_limit());
        }
        // Native get() recursively measures composites before applying its point limit.
        // Bound that traversal first using native parsed component iterators (no parser).
        let malformed =
            |e| EngineError::Unsupported(format!("font outline cannot be inspected: {e}"));
        let loca = self.font.loca(None).map_err(malformed)?;
        let glyf = self.font.glyf().map_err(malformed)?;
        let mut pending = vec![(id, 0usize)];
        let mut points = 0usize;
        while let Some((glyph, depth)) = pending.pop() {
            self.visits += 1;
            if self.visits > MAX_INK_VISITS || depth > 16 {
                return Err(ink_limit());
            }
            match loca.get_glyf(glyph, &glyf).map_err(malformed)? {
                Some(skrifa::raw::tables::glyf::Glyph::Simple(simple)) => {
                    points = points.saturating_add(simple.num_points());
                    if points > MAX_INK_POINTS {
                        return Err(ink_limit());
                    }
                }
                Some(skrifa::raw::tables::glyf::Glyph::Composite(composite)) => {
                    for (component, _) in composite.component_glyphs_and_flags() {
                        if pending.len() >= MAX_INK_POINTS
                            || self.visits + pending.len() >= MAX_INK_VISITS
                        {
                            return Err(ink_limit());
                        }
                        pending.push((component.into(), depth + 1));
                    }
                }
                None => {}
            }
        }
        let outlines = self.font.outline_glyphs();
        let glyph = outlines
            .get(id)
            .ok_or_else(|| EngineError::Unsupported("font glyph outline is unavailable".into()))?;
        let scratch_size = glyph.draw_memory_size(Hinting::None);
        if scratch_size > MAX_INK_SCRATCH || self.ops >= MAX_INK_OPS {
            return Err(ink_limit());
        }
        let mut scratch = vec![0; scratch_size];
        let mut pen = InkPen {
            path: kurbo::BezPath::new(),
            remaining: (MAX_INK_OPS - self.ops).min(MAX_INK_POINTS * 2),
            failed: false,
        };
        glyph
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::default())
                    .with_memory(Some(&mut scratch)),
                &mut pen,
            )
            .map_err(|e| {
                EngineError::Unsupported(format!("font glyph outline cannot be drawn: {e}"))
            })?;
        if pen.failed {
            return Err(ink_limit());
        }
        self.ops += pen.path.elements().len();
        let bounds = (!pen.path.elements().is_empty()).then(|| pen.path.bounding_box());
        self.cache.insert(id.to_u32(), bounds);
        Ok(bounds)
    }

    fn check(&mut self, id: skrifa::GlyphId, gx: f64, gy: f64, area: kurbo::Rect) -> PyResult<()> {
        if let Some(bounds) = self.bounds(id)? {
            let ink = kurbo::Rect::new(
                gx + bounds.x0 * self.scale,
                gy - bounds.y1 * self.scale,
                gx + bounds.x1 * self.scale,
                gy - bounds.y0 * self.scale,
            );
            if !ink.is_finite()
                || ink.x0 < area.x0
                || ink.y0 < area.y0
                || ink.x1 > area.x1
                || ink.y1 > area.y1
            {
                return Err(PyValueError::new_err(
                    "shaped glyph ink lies outside the text box",
                ));
            }
        }
        Ok(())
    }
}

impl EditablePdf {
    // Keep default serialization for internal callers and existing native tests. The Python
    // boundary exposes the same writer with one optional output-representation switch.
    pub fn to_bytes(&self) -> PyResult<Vec<u8>> {
        self.to_bytes_with_options(true)
    }

    fn serialize_with_limit(
        &self,
        object_streams: bool,
        max_output_bytes: usize,
    ) -> PyResult<Vec<u8>> {
        self.owned_fields()?;
        let options = SaveOptions {
            object_streams,
            ..SaveOptions::default()
        };
        let output = pdfcraft_cos::write_full(self.document()?, &options).map_err(|error| {
            EngineError::Conversion(format!("PDF serialization failed: {error}"))
        })?;
        // The upstream writer returns a complete Vec. This bounds returned output, not its
        // allocation peak; both native representations use the identical check.
        if output.len() > max_output_bytes {
            return Err(EngineError::ResourceLimit(format!(
                "PDF output exceeds {max_output_bytes} bytes"
            ))
            .into());
        }
        Ok(output)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_text_font_layout(
        &mut self,
        page: isize,
        text: &str,
        x: f64,
        y: f64,
        size: f64,
        font_bytes: &[u8],
        color: (u8, u8, u8),
        bounds: Option<(f64, f64)>,
        align: pdfcraft_edit::Align,
    ) -> PyResult<()> {
        use pdfcraft_cos::{Dict, Object, PdfString};
        let page = self.page_index(page)?;
        if font_bytes.len() > 16 * 1024 * 1024 || text.len() > 262_144 {
            return Err(EngineError::ResourceLimit(
                "font exceeds 16 MiB or text exceeds 256 KiB".into(),
            )
            .into());
        }
        self.validate_input(text.len() + font_bytes.len())?;
        if text.trim().is_empty() || text.chars().any(|ch| ch.is_control() && ch != '\n') {
            return Err(PyValueError::new_err(
                "text must be nonempty; only newline control characters are supported",
            ));
        }
        if !size.is_finite() || !(1.0..=500.0).contains(&size) {
            return Err(PyValueError::new_err(
                "font size must be between 1 and 500 points",
            ));
        }
        let (page_width, page_height) = self.sizes[page];
        let (width, height) = text_area(x, y, page_width, page_height, bounds)?;
        validate_font_reuse_profile(self.document()?, page)?;
        pdfcraft_fonts::embedding::validate_document_input(font_bytes).map_err(|message| {
            EngineError::Unsupported(format!("font cannot be embedded: {message}"))
        })?;
        let (_, family) =
            pdfcraft_fonts::embedding::names(font_bytes).map_err(EngineError::Unsupported)?;
        let mut ink = bounds.map(|_| TextInk::new(font_bytes, size)).transpose()?;
        let mut edited = self.document()?.clone();
        let full_resource = pdfcraft_edit::text::register_text_font(&mut edited, page, font_bytes)
            .map_err(edit_error)?;
        let page_ref = pdfcraft_organize::pages(&edited).map_err(organize_error)?[page].obj;
        let page_dict = edited
            .get(page_ref)
            .as_dict()
            .cloned()
            .ok_or_else(|| EngineError::Malformed("page has no dictionary".into()))?;
        let mut resources = page_dict
            .get(b"Resources")
            .and_then(|value| edited.dict(value))
            .unwrap_or_default();
        let mut fonts = resources
            .get(b"Font")
            .and_then(|value| edited.dict(value))
            .unwrap_or_default();
        let font = fonts
            .get(full_resource.as_bytes())
            .and_then(|value| edited.dict(value))
            .ok_or_else(|| EngineError::Malformed("registered font is missing".into()))?;
        let metrics = pdfcraft_fonts::pdf::Metrics::from_dict(&edited, &font);
        if !metrics.ascent.is_finite()
            || !metrics.descent.is_finite()
            || metrics.ascent <= metrics.descent
        {
            return Err(
                EngineError::Unsupported("font has invalid vertical metrics".into()).into(),
            );
        }
        let baseline = y + metrics.ascent * size;
        let leading = (1.2_f64).max(metrics.ascent - metrics.descent) * size;
        let mut ops = vec![
            Op::new("q", vec![]),
            Op::new("rg", rgb(color).map(Object::Real).to_vec()),
        ];
        let mut glyph_count = 0usize;
        for (row, line) in text.split('\n').enumerate() {
            let row_baseline = baseline + row as f64 * leading;
            if row_baseline - metrics.descent * size > height {
                return Err(PyValueError::new_err(
                    "text rows do not fit within the page height",
                ));
            }
            if line.is_empty() {
                continue;
            }
            let shaped =
                pdfcraft_fonts::source_shaping::shape(font_bytes, line).map_err(|message| {
                    EngineError::Unsupported(format!("font cannot shape text: {message}"))
                })?;
            glyph_count = glyph_count.saturating_add(shaped.glyphs.len());
            if glyph_count > 65_534 {
                return Err(EngineError::ResourceLimit(
                    "text item exceeds 65534 shaped glyphs".into(),
                )
                .into());
            }
            if !shaped.advance.is_finite()
                || shaped.advance < 0.0
                || x + shaped.advance * size > width
            {
                return Err(PyValueError::new_err(
                    "text row does not fit within the page width; wrap it explicitly",
                ));
            }
            let remaining = width - x - shaped.advance * size;
            let row_x = x + match align {
                pdfcraft_edit::Align::Center => remaining / 2.0,
                pdfcraft_edit::Align::Right => remaining,
                _ => 0.0,
            };
            let mappings = shaped
                .glyphs
                .iter()
                .map(|glyph| (glyph.text.clone(), glyph.id))
                .collect::<Vec<_>>();
            let resource = pdfcraft_fonts::embedding::embed_mapping(
                &mut edited,
                &mut fonts,
                font_bytes,
                &family,
                &mappings,
            )
            .map_err(EngineError::Unsupported)?;
            let mut properties = Dict::new();
            properties.set(
                b"ActualText".to_vec(),
                Object::String(PdfString::text(line)),
            );
            ops.push(Op::new(
                "BDC",
                vec![Object::name("Span"), Object::Dict(properties)],
            ));
            for (index, glyph) in shaped.glyphs.iter().enumerate() {
                let gx = row_x + glyph.position[0] * size;
                let gy = row_baseline - glyph.position[1] * size;
                if !gx.is_finite()
                    || !gy.is_finite()
                    || gx < 0.0
                    || gx > width
                    || gy < 0.0
                    || gy > height
                {
                    return Err(PyValueError::new_err(
                        "shaped glyph position lies outside the page",
                    ));
                }
                if let Some(ink) = &mut ink {
                    ink.check(glyph.id, gx, gy, kurbo::Rect::new(x, y, width, height))?;
                }
                let mut matrix = view_matrix(&self.geometry[page]);
                let [ux, uy] = self.geometry[page].view_to_user(gx as f32, gy as f32);
                matrix[2] = -matrix[2];
                matrix[3] = -matrix[3];
                matrix[4] = f64::from(ux);
                matrix[5] = f64::from(uy);
                let code = u16::try_from(index + 1).map_err(|_| {
                    EngineError::ResourceLimit("shaped glyph code exceeds limit".into())
                })?;
                ops.push(Op::new("BT", vec![]));
                ops.push(Op::new(
                    "Tf",
                    vec![Object::name(&resource), Object::Real(size)],
                ));
                ops.push(Op::new("Tm", matrix.map(Object::Real).to_vec()));
                ops.push(Op::new(
                    "Tj",
                    vec![Object::String(PdfString::literal(
                        code.to_be_bytes().to_vec(),
                    ))],
                ));
                ops.push(Op::new("ET", vec![]));
            }
            ops.push(Op::new("EMC", vec![]));
        }
        ops.push(Op::new("Q", vec![]));
        resources.set(b"Font".to_vec(), Object::Dict(fonts));
        edited
            .update_dict(page_ref, |page| {
                page.set(b"Resources".to_vec(), Object::Dict(resources))
            })
            .map_err(|error| {
                EngineError::Conversion(format!("font resource update failed: {error}"))
            })?;
        let content = serialize_ops(&ops);
        let cost = text.len() + font_bytes.len() + content.len();
        self.validate_input(cost)?;
        pdfcraft_edit::stamp(&mut edited, page, "pydoccraft-text", content).map_err(edit_error)?;
        preserve_drawing_resources(self.document()?, &edited, page)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += cost;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_text_layout(
        &mut self,
        page: isize,
        text: &str,
        x: f64,
        y: f64,
        size: f64,
        color: (u8, u8, u8),
        bounds: Option<(f64, f64)>,
        align: pdfcraft_edit::Align,
    ) -> PyResult<()> {
        let page = self.page_index(page)?;
        self.validate_input(text.len())?;
        if text.len() > MAX_TEXT_BYTES {
            return Err(EngineError::ResourceLimit("text item exceeds 1 MB".into()).into());
        }
        if text.trim().is_empty() {
            return Err(PyValueError::new_err("text must not be empty"));
        }
        if !size.is_finite() || !(1.0..=500.0).contains(&size) {
            return Err(PyValueError::new_err(
                "font size must be between 1 and 500 points",
            ));
        }
        let (page_width, page_height) = self.sizes[page];
        let (width, height) = text_area(x, y, page_width, page_height, bounds)?;
        let wrap_work = text_wrap_work(text)?;
        let item = AddedText {
            rect: [x, page_height - height, width, page_height - y],
            align,
            text: text.into(),
            size,
            color: rgb(color),
            ..AddedText::default()
        };
        if let Some(character) = pdfcraft_edit::first_undrawable(&item) {
            return Err(EngineError::Unsupported(format!(
                "PDF writing fonts cannot draw U+{:04X}",
                u32::from(character)
            ))
            .into());
        }
        // Let the upstream text layout determine wrapping and height; reject clipping.
        let lines = pdfcraft_edit::added::lines(&item);
        if wrap_work.saturating_mul(lines.len().max(1)) > MAX_TEXT_WRAP_WORK {
            return Err(EngineError::ResourceLimit(
                "PDF native text wrapping exceeds its work budget".into(),
            )
            .into());
        }
        let occupied = pdfcraft_edit::added::text_rect(&item);
        if occupied[1] < page_height - height || !occupied.iter().all(|value| value.is_finite()) {
            return Err(PyValueError::new_err(
                "text does not fit within the page height",
            ));
        }
        for line in lines {
            let line_item = AddedText {
                text: line,
                ..item.clone()
            };
            if pdfcraft_edit::added::text_width(&line_item) > width - x + 0.001 {
                return Err(PyValueError::new_err(
                    "text contains a word wider than the text box",
                ));
            }
        }
        let mut edited = self.document()?.clone();
        pdfcraft_edit::add_content(&mut edited, page, &Content::Text(item)).map_err(edit_error)?;
        preserve_drawing_resources(self.document()?, &edited, page)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += text.len();
        Ok(())
    }

    fn validate_owned_edit(&self, document: &Document) -> PyResult<()> {
        if self.owns_fields {
            checked_owned_fields(document)?;
        }
        Ok(())
    }

    fn owned_fields(&self) -> PyResult<Vec<pdfcraft_forms::Field>> {
        let document = self.document()?;
        if !self.owns_fields {
            return Ok(vec![]);
        }
        checked_owned_fields(document)
    }

    fn field_named(&self, name: &str) -> PyResult<pdfcraft_forms::Field> {
        validate_field_name(name)?;
        self.owned_fields()?
            .into_iter()
            .find(|field| field.name == name)
            .ok_or_else(|| PyValueError::new_err("form field does not exist"))
    }

    fn page_user_unit(&self, page: usize) -> f64 {
        let g = &self.geometry[page];
        let displayed = if g.rotation.is_multiple_of(180) {
            g.width
        } else {
            g.height
        };
        f64::from(displayed / (g.crop[2] - g.crop[0]))
    }

    fn field_payload(&self, field: &pdfcraft_forms::Field) -> PyResult<Value> {
        use pdfcraft_forms::FieldKind;
        let widget = &field.widgets[0];
        let page = widget
            .page
            .ok_or_else(|| EngineError::Unsupported("form widget has no page".into()))?;
        let geometry = self
            .geometry
            .get(page)
            .ok_or_else(|| EngineError::Malformed("form widget page is out of range".into()))?;
        let rect = view_rect(geometry, widget.rect.map(|v| v as f32));
        let (kind, value) = match field.kind {
            FieldKind::Text => (
                "text",
                json!(field.value.first().map_or("", String::as_str)),
            ),
            FieldKind::CheckBox => ("checkbox", json!(!field.value.is_empty())),
            FieldKind::Combo => ("choice", json!(field.value.first())),
            _ => {
                return Err(EngineError::Unsupported("unsupported owned field kind".into()).into())
            }
        };
        Ok(json!({
            "name": field.name, "kind":kind, "value":value,
            "tooltip": field.tooltip.as_deref().unwrap_or(""),
            "options": field.options.iter().map(|(export, _)| export).collect::<Vec<_>>(),
            "widgets":[{"page":page,"box":[rect[0],rect[1],rect[2]-rect[0],rect[3]-rect[1]]}],
            "font_size":12,
        }))
    }

    fn bookmark_entries(&self) -> PyResult<Vec<pdfcraft_organize::Bookmark>> {
        let entries = pdfcraft_organize::bookmark_page(self.document()?, 0, MAX_ITEMS + 1);
        if entries.bookmarks.len() > MAX_ITEMS || entries.next.is_some() {
            return Err(EngineError::ResourceLimit("bookmark count exceeds 10000".into()).into());
        }
        Ok(entries.bookmarks)
    }

    fn annotation_rect(&self, page: usize, rect: (f64, f64, f64, f64)) -> PyResult<[f32; 4]> {
        let values = [rect.0, rect.1, rect.2, rect.3];
        let (width, height) = self.sizes[page];
        if values.iter().any(|value| !value.is_finite())
            || rect.0 < 0.0
            || rect.1 < 0.0
            || rect.2 > width
            || rect.3 > height
            || rect.2 <= rect.0
            || rect.3 <= rect.1
        {
            return Err(PyValueError::new_err(
                "annotation rectangle must have positive area and lie inside the page",
            ));
        }
        let narrowed = values.map(|value| value as f32);
        if narrowed[2] <= narrowed[0] || narrowed[3] <= narrowed[1] {
            return Err(PyValueError::new_err("annotation rectangle is too small"));
        }
        Ok(narrowed)
    }

    fn link_action(
        &self,
        uri: Option<&str>,
        target_page: Option<isize>,
    ) -> PyResult<pdfcraft_annot::links::LinkAction> {
        match (uri, target_page) {
            (Some(uri), None) => {
                validate_annotation_text(uri, 8192, "link URI")?;
                if uri.is_empty() || uri.chars().any(char::is_control) {
                    return Err(PyValueError::new_err(
                        "link URI must be nonempty and contain no control characters",
                    ));
                }
                Ok(pdfcraft_annot::links::LinkAction::Uri(uri.into()))
            }
            (None, Some(page)) => Ok(pdfcraft_annot::links::LinkAction::Page(
                self.page_index(page)?,
            )),
            _ => Err(PyValueError::new_err(
                "specify exactly one of uri or target_page",
            )),
        }
    }

    fn selection(&self, indices: &[isize]) -> PyResult<Vec<usize>> {
        self.document()?;
        if indices.len() > MAX_PAGES {
            return Err(
                EngineError::ResourceLimit("PDF selection exceeds 10000 pages".into()).into(),
            );
        }
        indices
            .iter()
            .map(|index| self.page_index(*index))
            .collect()
    }

    fn document(&self) -> PyResult<&Document> {
        self.document
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("document is closed"))
    }

    fn page_index(&self, page: isize) -> PyResult<usize> {
        self.document()?;
        let index = if page < 0 {
            page + self.sizes.len() as isize
        } else {
            page
        };
        if index < 0 || index as usize >= self.sizes.len() {
            return Err(PyIndexError::new_err("page index out of range"));
        }
        Ok(index as usize)
    }

    fn validate_input(&self, bytes: usize) -> PyResult<()> {
        if self.items >= MAX_ITEMS || bytes > MAX_DOCUMENT_INPUT.saturating_sub(self.input_bytes) {
            return Err(EngineError::ResourceLimit(
                "PDF authoring item or input budget exceeded".into(),
            )
            .into());
        }
        Ok(())
    }
}

// Pinned appearance::ellipse emits 26 coordinates; n() uses three decimals.
// With coordinates bounded to +/-1e6, colors in 0..1, a solid 2-unit stroke and
// no fill, the uncompressed path is below 512 bytes. This conservative allowance
// covers generated appearance work/storage, NOT zlib or total process memory.
const GEOMETRIC_APPEARANCE_WORK: usize = 16 * 1024;

fn validate_geometric_rect(rect: [f64; 4]) -> PyResult<()> {
    // Native inset paths below one user unit can collapse during formatting or
    // rasterization (a 2.01-unit oval disappears at 72/144/300 DPI). Preserve at
    // least one unit after the fixed two-unit stroke inset, without rebuilding it.
    if rect
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1_000_000.0)
        || rect[2] - rect[0] < 3.0
        || rect[3] - rect[1] < 3.0
    {
        return Err(PyValueError::new_err("rectangle and ellipse dimensions must be at least 3 PDF user units, with coordinates within 1000000 units"));
    }
    Ok(())
}

fn geometric_dict_peak(document: &Document, dict: &pdfcraft_cos::Dict) -> PyResult<usize> {
    use pdfcraft_cos::Object;
    if !matches!(dict.name(b"Subtype"), Some(b"Square" | b"Circle")) {
        return Ok(0);
    }
    let unsupported = || {
        EngineError::Unsupported("editing rectangle/ellipse annotations requires canonical solid, unfilled, opaque 2-user-unit geometry and metadata".into())
    };
    if dict.len() > 20 {
        return Err(EngineError::ResourceLimit(
            "geometric annotation dictionary exceeds 20 entries".into(),
        )
        .into());
    }
    // A narrow accepted profile prevents native dict clones/props()/appearance
    // rebuilds from traversing arbitrary arrays, rich metadata or border graphs.
    for (key, value) in dict.iter() {
        match key.as_slice() {
            b"Type" if value.as_name() == Some(b"Annot") => {}
            b"Subtype" => {}
            b"P" if matches!(value, Object::Ref(_)) => {}
            b"F" if matches!(value.as_int(), Some(4 | 132)) => {}
            b"CA" if value.as_f64() == Some(1.0) => {}
            b"Contents" | b"T" | b"NM" | b"M" | b"CreationDate" | b"Subj" => {
                let bytes = value.as_string().ok_or_else(unsupported)?.bytes.len();
                let maximum = if key.as_slice() == b"Contents" {
                    131_074
                } else {
                    8194
                };
                if bytes > maximum {
                    return Err(EngineError::ResourceLimit(
                        "geometric annotation metadata exceeds its bound".into(),
                    )
                    .into());
                }
            }
            b"Rect" => {
                let values = value.as_array().ok_or_else(unsupported)?;
                if values.len() > 4 {
                    return Err(EngineError::ResourceLimit(
                        "geometric annotation Rect exceeds 4 values".into(),
                    )
                    .into());
                }
                if values.len() != 4 {
                    return Err(unsupported().into());
                }
                let mut rect = [0.0; 4];
                for (dest, source) in rect.iter_mut().zip(values) {
                    *dest = source.as_f64().ok_or_else(unsupported)?;
                }
                if validate_geometric_rect(rect).is_err() {
                    return Err(unsupported().into());
                }
            }
            b"C" => {
                let values = value.as_array().ok_or_else(unsupported)?;
                if values.len() > 3 {
                    return Err(EngineError::ResourceLimit(
                        "geometric annotation color exceeds 3 values".into(),
                    )
                    .into());
                }
                if values.len() != 3
                    || values.iter().any(|v| {
                        !v.as_f64()
                            .is_some_and(|n| n.is_finite() && (0.0..=1.0).contains(&n))
                    })
                {
                    return Err(unsupported().into());
                }
            }
            b"BS" => {
                let border = value.as_dict().ok_or_else(unsupported)?;
                if border.len() > 2 {
                    return Err(unsupported().into());
                }
                if border.len() != 2
                    || border.get(b"W").and_then(Object::as_f64) != Some(2.0)
                    || border.name(b"S") != Some(b"S")
                {
                    return Err(unsupported().into());
                }
            }
            b"AP" => {
                let owned = if let Object::Ref(reference) = value {
                    Some(document.get(*reference))
                } else {
                    None
                };
                let appearances = owned
                    .as_deref()
                    .unwrap_or(value)
                    .as_dict()
                    .ok_or_else(unsupported)?;
                if appearances.len() != 1 {
                    return Err(unsupported().into());
                }
                let normal = appearances
                    .get(b"N")
                    .and_then(Object::as_ref)
                    .ok_or_else(unsupported)?;
                if !matches!(document.get(normal).as_ref(), Object::Stream(_)) {
                    return Err(unsupported().into());
                }
                // Never decode an imported appearance. Explicit geometry/style
                // changes replace its normal appearance with the native standard.
            }
            _ => return Err(unsupported().into()),
        }
    }
    if ![b"Rect".as_slice(), b"C", b"BS"]
        .iter()
        .all(|key| dict.contains(key))
    {
        return Err(unsupported().into());
    }
    Ok(GEOMETRIC_APPEARANCE_WORK)
}

const MAX_MARKUP_QUADS: usize = 1024;

// Read only one annotation. Preserve borrowed direct objects rather than cloning an
// untrusted nested dictionary/array, and never decode an imported appearance stream.
fn with_annotation_dict<T>(
    document: &Document,
    page: usize,
    index: usize,
    read: impl FnOnce(&pdfcraft_cos::Dict) -> PyResult<T>,
) -> PyResult<T> {
    use pdfcraft_cos::Object;
    let missing = || PyIndexError::new_err("annotation index out of range");
    let pages = pdfcraft_organize::pages(document).map_err(organize_error)?;
    let page = document.get(pages.get(page).ok_or_else(missing)?.obj);
    let annots = page
        .as_dict()
        .and_then(|p| p.get(b"Annots"))
        .ok_or_else(missing)?;
    let owned = if let Object::Ref(reference) = annots {
        Some(document.get(*reference))
    } else {
        None
    };
    let annots = owned
        .as_deref()
        .unwrap_or(annots)
        .as_array()
        .ok_or_else(missing)?;
    if annots.len() > MAX_ITEMS {
        return Err(EngineError::ResourceLimit("PDF annotation count exceeds 10000".into()).into());
    }
    let annotation = annots.get(index).ok_or_else(missing)?;
    let owned = if let Object::Ref(reference) = annotation {
        Some(document.get(*reference))
    } else {
        None
    };
    read(
        owned
            .as_deref()
            .unwrap_or(annotation)
            .as_dict()
            .ok_or_else(missing)?,
    )
}

// This is a bounded profile for one flag-only mutation, not an imported
// annotation parser. Native annot_ref clones all Annots entries, and update_dict
// clones the target: reject deep inline siblings and unbounded target values.
fn checked_lock_target(
    document: &Document,
    page: usize,
    index: usize,
) -> PyResult<(pdfcraft_cos::ObjRef, usize)> {
    use pdfcraft_cos::Object;
    let unsupported = || {
        EngineError::Unsupported("annotation locking requires a canonical note, text markup, rectangle or ellipse with reference-only page annotations".into())
    };
    let missing = || PyIndexError::new_err("annotation index out of range");
    let pages = pdfcraft_organize::pages(document).map_err(organize_error)?;
    let page = document.get(pages.get(page).ok_or_else(missing)?.obj);
    let array = page
        .as_dict()
        .and_then(|d| d.get(b"Annots"))
        .ok_or_else(missing)?;
    let resolved = if let Object::Ref(r) = array {
        Some(document.get(*r))
    } else {
        None
    };
    let array = resolved
        .as_deref()
        .unwrap_or(array)
        .as_array()
        .ok_or_else(missing)?;
    if array.len() > MAX_ITEMS {
        return Err(EngineError::ResourceLimit("PDF annotation count exceeds 10000".into()).into());
    }
    if array.iter().any(|entry| !matches!(entry, Object::Ref(_))) {
        return Err(unsupported().into());
    }
    let reference = array
        .get(index)
        .and_then(Object::as_ref)
        .ok_or_else(missing)?;
    let object = document.get(reference);
    let dict = object.as_dict().ok_or_else(missing)?;
    let subtype = dict.name(b"Subtype").ok_or_else(unsupported)?;
    let note = subtype == b"Text";
    let markup = matches!(
        subtype,
        b"Highlight" | b"Underline" | b"StrikeOut" | b"Squiggly"
    );
    let geometric = matches!(subtype, b"Square" | b"Circle");
    if !note && !markup && !geometric {
        return Err(unsupported().into());
    }
    if dict.len() > 20 {
        return Err(EngineError::ResourceLimit(
            "annotation lock dictionary exceeds 20 entries".into(),
        )
        .into());
    }
    let mut peak = 4096usize.saturating_add(array.len().saturating_mul(128));
    for (key, value) in dict.iter() {
        match key.as_slice() {
            b"Type" if value.as_name() == Some(b"Annot") => {}
            b"Subtype" => {}
            b"P" | b"Popup"
                if matches!(value, Object::Ref(_)) && (key.as_slice() == b"P" || note) => {}
            b"F" if matches!(value, Object::Int(28 | 156)) && note => {}
            b"F" if matches!(value, Object::Int(4 | 132)) && !note => {}
            b"CA" if value.as_f64() == Some(1.0) => {}
            b"Contents" | b"T" | b"NM" | b"M" | b"CreationDate" | b"Subj" => {
                let length = value.as_string().ok_or_else(unsupported)?.bytes.len();
                let limit = if key.as_slice() == b"Contents" {
                    131_074
                } else {
                    8194
                };
                if length > limit {
                    return Err(EngineError::ResourceLimit(
                        "annotation lock metadata exceeds its bound".into(),
                    )
                    .into());
                }
                peak = peak.saturating_add(length.saturating_mul(2));
            }
            b"Rect" | b"C" => {
                let values = value.as_array().ok_or_else(unsupported)?;
                let size = if key.as_slice() == b"Rect" { 4 } else { 3 };
                if values.len() > size {
                    return Err(EngineError::ResourceLimit(
                        "annotation lock geometry array exceeds its bound".into(),
                    )
                    .into());
                }
                if values.len() != size
                    || values.iter().any(|v| {
                        !v.as_f64().is_some_and(|n| {
                            n.is_finite()
                                && if size == 4 {
                                    n.abs() <= 1_000_000.0
                                } else {
                                    (0.0..=1.0).contains(&n)
                                }
                        })
                    })
                {
                    return Err(unsupported().into());
                }
                if size == 4
                    && (values[0].as_f64() >= values[2].as_f64()
                        || values[1].as_f64() >= values[3].as_f64())
                {
                    return Err(unsupported().into());
                }
            }
            b"Name" if note => {
                let name = value.as_name().ok_or_else(unsupported)?;
                if name.len() > 64 {
                    return Err(EngineError::ResourceLimit(
                        "annotation note name exceeds 64 bytes".into(),
                    )
                    .into());
                }
            }
            b"Open" if note && matches!(value, Object::Bool(_)) => {}
            b"QuadPoints" if markup => {
                // The shared validator bounds the direct array before scanning.
                markup_dict_peak(dict)?;
                peak = peak.saturating_add(
                    value
                        .as_array()
                        .ok_or_else(unsupported)?
                        .len()
                        .saturating_mul(64),
                );
            }
            b"BS" if geometric => {
                let border = value.as_dict().ok_or_else(unsupported)?;
                if border.len() != 2
                    || border.get(b"W").and_then(Object::as_f64) != Some(2.0)
                    || border.name(b"S") != Some(b"S")
                {
                    return Err(unsupported().into());
                }
            }
            b"AP" => {
                let resolved = if let Object::Ref(r) = value {
                    Some(document.get(*r))
                } else {
                    None
                };
                let appearance = resolved
                    .as_deref()
                    .unwrap_or(value)
                    .as_dict()
                    .ok_or_else(unsupported)?;
                if appearance.len() != 1 {
                    return Err(unsupported().into());
                }
                let normal = appearance
                    .get(b"N")
                    .and_then(Object::as_ref)
                    .ok_or_else(unsupported)?;
                if !matches!(document.get(normal).as_ref(), Object::Stream(_)) {
                    return Err(unsupported().into());
                }
            }
            _ => return Err(unsupported().into()),
        }
    }
    if ![b"Type".as_slice(), b"F", b"Rect", b"C", b"AP"]
        .iter()
        .all(|k| dict.contains(k))
        || (markup && !dict.contains(b"QuadPoints"))
    {
        return Err(unsupported().into());
    }
    if geometric {
        geometric_dict_peak(document, dict)?;
    }
    Ok((reference, peak))
}

fn markup_quad_peak(quad: &[f64; 8], squiggly: bool) -> PyResult<usize> {
    // Use the same absolute user-space limit as editable page boxes. Finite
    // inputs alone do not make native interpolation finite: subtracting opposite
    // near-f64::MAX corners can overflow even when the measured left/bottom
    // edges remain finite. This also keeps downstream f32 geometry representable.
    if quad.iter().any(|n| !n.is_finite() || n.abs() > 1_000_000.0) {
        return Err(EngineError::Unsupported(
            "markup quad coordinates must be finite and within 1000000 PDF user units".into(),
        )
        .into());
    }
    let height = (quad[0] - quad[4]).hypot(quad[1] - quad[5]);
    let length = (quad[6] - quad[4]).hypot(quad[7] - quad[5]);
    if !height.is_finite() || !length.is_finite() {
        return Err(
            EngineError::Unsupported("markup quad exceeds finite native geometry".into()).into(),
        );
    }
    // Resource estimate only; PdfCraft creates the geometry. Its cap coarsens the
    // entire bottom edge, since each vertex uses i/steps (it never truncates it).
    let vertices = if squiggly {
        ((length / (height / 4.0).max(1.5)).ceil() as usize).clamp(1, 10_000) + 1
    } else {
        4
    };
    // Two maximal finite f64 decimal coordinates plus native operator overhead.
    Ok(4096 + vertices * 1024)
}

fn markup_dict_peak(dict: &pdfcraft_cos::Dict) -> PyResult<usize> {
    let subtype = dict.name(b"Subtype").unwrap_or_default();
    if !matches!(
        subtype,
        b"Highlight" | b"Underline" | b"StrikeOut" | b"Squiggly"
    ) {
        return Ok(0);
    }
    let quads = dict
        .get(b"QuadPoints")
        .and_then(|v| v.as_array())
        .ok_or_else(|| EngineError::Unsupported("markup requires direct QuadPoints".into()))?;
    if quads.len() > MAX_MARKUP_QUADS * 8 {
        return Err(EngineError::ResourceLimit("markup exceeds 1024 quadrilaterals".into()).into());
    }
    if quads.is_empty() || quads.len() % 8 != 0 {
        return Err(EngineError::Unsupported("markup has incomplete QuadPoints".into()).into());
    }
    let mut total = 4096usize;
    for values in quads.chunks_exact(8) {
        let mut quad = [0.0; 8];
        for (dest, value) in quad.iter_mut().zip(values) {
            *dest = value.as_f64().ok_or_else(|| {
                EngineError::Unsupported("markup quad contains a nonnumeric coordinate".into())
            })?;
        }
        total = total.saturating_add(markup_quad_peak(&quad, subtype == b"Squiggly")?);
        if total > MAX_DOCUMENT_INPUT {
            return Err(EngineError::ResourceLimit(
                "markup appearance work exceeds 128 MiB".into(),
            )
            .into());
        }
    }
    Ok(total)
}

fn annotation_appearance_bytes(document: &Document, page: usize, index: usize) -> PyResult<usize> {
    use pdfcraft_cos::Object;
    with_annotation_dict(document, page, index, |dict| {
        let missing = || EngineError::Conversion("native annotation appearance is missing".into());
        let ap = dict.get(b"AP").ok_or_else(missing)?;
        let owned = if let Object::Ref(reference) = ap {
            Some(document.get(*reference))
        } else {
            None
        };
        let normal = owned
            .as_deref()
            .unwrap_or(ap)
            .as_dict()
            .and_then(|d| d.get(b"N"))
            .ok_or_else(missing)?;
        let owned = if let Object::Ref(reference) = normal {
            Some(document.get(*reference))
        } else {
            None
        };
        match owned.as_deref().unwrap_or(normal) {
            Object::Stream(stream) => Ok(stream.raw.len()),
            _ => Err(missing().into()),
        }
    })
}

fn metadata_keys() -> [(&'static str, &'static str); 4] {
    [
        ("title", "Title"),
        ("author", "Author"),
        ("subject", "Subject"),
        ("keywords", "Keywords"),
    ]
}

fn checked_import_outline(
    document: &Document,
    maximum_depth: usize,
    copying: bool,
) -> PyResult<Vec<pdfcraft_organize::Bookmark>> {
    let listed = pdfcraft_organize::bookmark_page(document, 0, MAX_ITEMS + 1);
    if listed.next.is_some() || listed.bookmarks.len() > MAX_ITEMS {
        return Err(EngineError::ResourceLimit("bookmark count exceeds 10000".into()).into());
    }
    let mut bytes = 0usize;
    for entry in &listed.bookmarks {
        if entry.path.len() > maximum_depth {
            return Err(EngineError::ResourceLimit(
                "outline import would exceed 32 bookmark levels including the container".into(),
            )
            .into());
        }
        validate_annotation_text(&entry.title, 4096, "bookmark title")?;
        // Account conservatively for escaped JSON titles and normalized path metadata before
        // allowing an outline the public 16 MiB snapshot could not represent.
        bytes += serde_json::to_string(&entry.title)
            .map_err(|error| EngineError::Malformed(format!("invalid bookmark title: {error}")))?
            .len()
            + 128
            + entry.path.len() * 8;
        if bytes > 16 * 1024 * 1024 {
            return Err(EngineError::ResourceLimit("outline titles exceed 16 MiB".into()).into());
        }
        if copying {
            // The native copier preserves titles, colour, flags, expansion and internal
            // destinations. Refuse other semantics that it would otherwise omit.
            let object = document.get(entry.obj);
            let item = object
                .as_dict()
                .ok_or_else(|| EngineError::Malformed("bookmark is not a dictionary".into()))?;
            for (key, _) in item.iter() {
                if !matches!(
                    key.as_slice(),
                    b"Type"
                        | b"Title"
                        | b"C"
                        | b"F"
                        | b"Count"
                        | b"Parent"
                        | b"First"
                        | b"Last"
                        | b"Next"
                        | b"Prev"
                        | b"Dest"
                        | b"A"
                ) {
                    return Err(EngineError::Unsupported(
                        "outline contains properties the native importer cannot preserve".into(),
                    )
                    .into());
                }
            }
            if let Some(action) = item.get(b"A") {
                let action = document.resolve(action);
                let action = action
                    .as_dict()
                    .ok_or_else(|| EngineError::Unsupported("unsupported outline action".into()))?;
                if action.name(b"S") != Some(b"GoTo")
                    || action
                        .iter()
                        .any(|(key, _)| !matches!(key.as_slice(), b"Type" | b"S" | b"D"))
                    || item.contains(b"Dest")
                {
                    return Err(EngineError::Unsupported(
                        "outline import supports only single internal GoTo actions".into(),
                    )
                    .into());
                }
            }
            if (item.contains(b"Dest") || item.contains(b"A")) && entry.page.is_none() {
                return Err(EngineError::Unsupported(
                    "source bookmark destination cannot be resolved to a source page".into(),
                )
                .into());
            }
        }
    }
    Ok(listed.bookmarks)
}

fn validate_field_name(name: &str) -> PyResult<()> {
    if name.len() > 256 {
        return Err(EngineError::ResourceLimit("field name exceeds 256 bytes".into()).into());
    }
    if name.is_empty()
        || name.trim() != name
        || name.contains('.')
        || name.chars().any(char::is_control)
    {
        return Err(PyValueError::new_err(
            "field name must be nonempty without periods, outer whitespace, or controls",
        ));
    }
    Ok(())
}

fn validate_field_text(text: &str) -> PyResult<()> {
    if text.len() > 4096 {
        return Err(
            EngineError::ResourceLimit("field value or option exceeds 4096 bytes".into()).into(),
        );
    }
    if text.chars().any(char::is_control) {
        return Err(PyValueError::new_err(
            "field values and options must be single-line text without control characters",
        ));
    }
    if let Some(character) = pdfcraft_fonts::first_non_win_ansi(text) {
        return Err(EngineError::Unsupported(format!(
            "form Helvetica font cannot draw U+{:04X}",
            u32::from(character)
        ))
        .into());
    }
    Ok(())
}

fn validate_field_fit(text: &str, width: f64, height: f64, size: f64) -> PyResult<()> {
    // Native appearances use a one-unit border and two-unit padding. Four units of inset
    // conservatively retain full glyphs instead of accepting text hidden by the widget clip.
    if !size.is_finite()
        || size <= 0.0
        || size > 300.0
        || height < size * 1.2 + 8.0
        || pdfcraft_fonts::helvetica_width(text, size) > width - 8.0
    {
        return Err(PyValueError::new_err(
            "field text does not fit its fixed 12-point appearance",
        ));
    }
    Ok(())
}

fn validate_value_fit(field: &pdfcraft_forms::Field, text: &str) -> PyResult<()> {
    if field.kind == pdfcraft_forms::FieldKind::CheckBox {
        return Ok(());
    }
    let widget = &field.widgets[0];
    let (width, height) = (
        widget.rect[2] - widget.rect[0],
        widget.rect[3] - widget.rect[1],
    );
    let (width, height) = if widget.rotation % 180 == 0 {
        (width, height)
    } else {
        (height, width)
    };
    validate_field_fit(
        text,
        width,
        height,
        pdfcraft_forms::appearance::parse_da(&field.da).size,
    )
}

// Conservative peak reservation for the existing bounded, fixed-size owned appearance writer.
fn owned_field_appearance_peak(field: &pdfcraft_forms::Field) -> usize {
    let text_bytes = field
        .value
        .iter()
        .chain(&field.default)
        .map(String::len)
        .sum::<usize>()
        + field
            .options
            .iter()
            .map(|(export, display)| export.len() + display.len())
            .sum::<usize>();
    64 * 1024 + 16 * text_bytes
}

// Only newly generated, owned single-widget appearances reach this helper. Never
// traverse or decode arbitrary imported AP graphs; imports with forms stay unsupported.
fn owned_field_appearance_bytes(
    document: &Document,
    field: &pdfcraft_forms::Field,
) -> PyResult<usize> {
    use pdfcraft_cos::Object;
    let missing =
        || EngineError::Conversion("native field appearance is missing or unsupported".into());
    let widget = document.get(field.widgets[0].obj);
    let ap = document.resolve(
        widget
            .as_dict()
            .and_then(|d| d.get(b"AP"))
            .ok_or_else(missing)?,
    );
    let normal = document.resolve(ap.as_dict().and_then(|d| d.get(b"N")).ok_or_else(missing)?);
    match normal.as_ref() {
        Object::Stream(stream) => Ok(stream.raw.len()),
        Object::Dict(states)
            if field.kind == pdfcraft_forms::FieldKind::CheckBox && states.len() <= 2 =>
        {
            let mut bytes = 0usize;
            for (_, value) in states.iter() {
                let appearance = document.resolve(value);
                let Object::Stream(stream) = appearance.as_ref() else {
                    return Err(missing().into());
                };
                bytes = bytes.saturating_add(stream.raw.len());
            }
            Ok(bytes)
        }
        _ => Err(missing().into()),
    }
}

fn checked_owned_fields(document: &Document) -> PyResult<Vec<pdfcraft_forms::Field>> {
    use pdfcraft_forms::FieldKind;
    // This reader only sees our bounded, shallow, script-free trees. Imported forms never
    // enter this path; upstream has no fallible caller-bounded reader for arbitrary trees yet.
    let read = pdfcraft_forms::fields_with_adopted(document);
    if read.items.len() > MAX_FIELDS {
        return Err(EngineError::ResourceLimit("PDF exceeds 1000 fields".into()).into());
    }
    if read.adopted_page_fields != 0 {
        return Err(
            EngineError::Unsupported("owned form contains unregistered widgets".into()).into(),
        );
    }
    let mut names = std::collections::HashSet::new();
    let mut bytes = 0usize;
    for field in &read.items {
        validate_field_name(&field.name)?;
        if !names.insert(&field.name)
            || field.widgets.len() != 1
            || field.widgets[0].page.is_none()
            || field.widgets[0].hidden
            || field.value.len() > 1
            || field.options.len() > 256
            || field.actions != pdfcraft_forms::af::Actions::default()
            || field.button.is_some()
        {
            return Err(EngineError::Unsupported(
                "owned form no longer matches the supported single-widget profile".into(),
            )
            .into());
        }
        match field.kind {
            FieldKind::Text if field.flags == 0 && field.options.is_empty() => {}
            FieldKind::CheckBox if field.flags == 0 && field.options.is_empty() => {}
            FieldKind::Combo
                if field.flags == pdfcraft_forms::flags::COMBO && !field.options.is_empty() => {}
            _ => {
                return Err(EngineError::Unsupported(
                    "owned form contains an unsupported field kind or setting".into(),
                )
                .into())
            }
        }
        bytes = bytes.saturating_add(field.name.len());
        if let Some(tooltip) = &field.tooltip {
            validate_annotation_text(tooltip, 4096, "field tooltip")?;
            bytes = bytes.saturating_add(tooltip.len());
        }
        let mut options = std::collections::HashSet::new();
        for (export, display) in &field.options {
            validate_field_text(display)?;
            if export != display || export.is_empty() || !options.insert(export) {
                return Err(EngineError::Unsupported(
                    "owned choice field has unsupported options".into(),
                )
                .into());
            }
            validate_value_fit(field, display)?;
            bytes = bytes.saturating_add(export.len());
        }
        for value in &field.value {
            validate_field_text(value)?;
            validate_value_fit(field, value)?;
            bytes = bytes.saturating_add(value.len());
        }
        if bytes > MAX_FIELD_TEXT {
            return Err(
                EngineError::ResourceLimit("aggregate form text exceeds 4 MiB".into()).into(),
            );
        }
    }
    if pdfcraft_forms::need_appearances(document)
        || !pdfcraft_forms::undrawn_fields(document).is_empty()
    {
        return Err(EngineError::Unsupported(
            "form appearance is incomplete; partial output was discarded".into(),
        )
        .into());
    }
    Ok(read.items)
}

fn has_widgets(document: &Document) -> PyResult<bool> {
    for page in pdfcraft_organize::pages(document).map_err(organize_error)? {
        let object = document.get(page.obj);
        if let Some(annotations) = object.as_dict().and_then(|d| d.get(b"Annots")) {
            let annotations = document.resolve(annotations);
            if let Some(annotations) = annotations.as_array() {
                if annotations.iter().any(|annotation| {
                    document
                        .resolve(annotation)
                        .as_dict()
                        .is_some_and(|d| d.name(b"Subtype") == Some(b"Widget"))
                }) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

fn form_error(error: pdfcraft_forms::FormError) -> EngineError {
    EngineError::Conversion(format!("PDF form operation failed: {error}"))
}

fn validate_bookmark_title(title: &str) -> PyResult<()> {
    validate_annotation_text(title, 4096, "bookmark title")?;
    if title.trim().is_empty() {
        return Err(PyValueError::new_err("bookmark title must not be empty"));
    }
    Ok(())
}

fn validate_bookmark_path(
    entries: &[pdfcraft_organize::Bookmark],
    path: &[usize],
    root: bool,
) -> PyResult<()> {
    if path.len() > 32 {
        return Err(EngineError::ResourceLimit("bookmark depth exceeds 32".into()).into());
    }
    if root && path.is_empty() {
        return Ok(());
    }
    if !entries.iter().any(|entry| entry.path == path) {
        return Err(PyIndexError::new_err("bookmark path does not exist"));
    }
    Ok(())
}

fn bookmark_children(entries: &[pdfcraft_organize::Bookmark], parent: &[usize]) -> usize {
    entries
        .iter()
        .filter(|entry| entry.path.len() == parent.len() + 1 && entry.path.starts_with(parent))
        .count()
}

fn outline_error(error: pdfcraft_organize::OutlineError) -> EngineError {
    EngineError::Conversion(format!("PDF bookmark operation failed: {error}"))
}

fn page_geometry(width: f64, height: f64) -> pdfcraft_render::PageInfo {
    pdfcraft_render::PageInfo {
        width: width as f32,
        height: height as f32,
        label: String::new(),
        crop: [0.0, 0.0, width as f32, height as f32],
        rotation: 0,
    }
}

fn view_matrix(page: &pdfcraft_render::PageInfo) -> [f64; 6] {
    let origin = page.view_to_user(0.0, 0.0);
    let x = page.view_to_user(1.0, 0.0);
    let y = page.view_to_user(0.0, 1.0);
    [
        x[0] - origin[0],
        x[1] - origin[1],
        y[0] - origin[0],
        y[1] - origin[1],
        origin[0],
        origin[1],
    ]
    .map(f64::from)
}

fn view_rect(page: &pdfcraft_render::PageInfo, rect: [f32; 4]) -> [f64; 4] {
    let a = page.user_to_view(rect[0], rect[1]);
    let b = page.user_to_view(rect[2], rect[3]);
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[0].max(b[0]),
        a[1].max(b[1]),
    ]
    .map(f64::from)
}

fn rgb(color: (u8, u8, u8)) -> [f64; 3] {
    [color.0, color.1, color.2].map(|channel| f64::from(channel) / 255.0)
}

fn validate_annotation_text(text: &str, limit: usize, label: &str) -> PyResult<()> {
    if text.len() > limit {
        return Err(EngineError::ResourceLimit(format!("{label} exceeds {limit} bytes")).into());
    }
    if text.contains('\0') {
        return Err(PyValueError::new_err(format!(
            "{label} must not contain NUL"
        )));
    }
    Ok(())
}

fn annotation_error(error: pdfcraft_annot::AnnotError) -> EngineError {
    match error {
        pdfcraft_annot::AnnotError::Unsupported(message)
        | pdfcraft_annot::AnnotError::Undrawable(message) => EngineError::Unsupported(message),
        other => EngineError::Conversion(format!("PDF annotation operation failed: {other}")),
    }
}

fn validate_page_size(width: f64, height: f64) -> PyResult<()> {
    if !width.is_finite()
        || !height.is_finite()
        || !(3.0..=14_400.0).contains(&width)
        || !(3.0..=14_400.0).contains(&height)
    {
        return Err(PyValueError::new_err(
            "page sides must be finite and between 3 and 14400 points",
        ));
    }
    Ok(())
}

fn validate_position(x: f64, y: f64, width: f64, height: f64) -> PyResult<()> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= width || y >= height {
        return Err(PyValueError::new_err(
            "position must be finite and inside the page",
        ));
    }
    Ok(())
}

fn validate_png(data: &[u8]) -> PyResult<()> {
    if data.len() > MAX_IMAGE_BYTES {
        return Err(EngineError::ResourceLimit("image input exceeds 64 MiB".into()).into());
    }
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(EngineError::Unsupported(
            "PDF image insertion currently accepts PNG bytes".into(),
        )
        .into());
    }
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_limits(png::Limits {
        bytes: MAX_IMAGE_BYTES,
    });
    let reader = decoder
        .read_info()
        .map_err(|error| EngineError::Malformed(format!("invalid PNG image: {error}")))?;
    let info = reader.info();
    crate::rendering::validate_size(f64::from(info.width), f64::from(info.height), 16_000)?;
    if reader
        .output_buffer_size()
        .is_none_or(|size| size > 256_000_000)
    {
        return Err(EngineError::ResourceLimit("decoded image exceeds 256 MB".into()).into());
    }
    Ok(())
}

fn creation_error(error: pdfcraft_create::CreateError) -> EngineError {
    EngineError::Conversion(format!("PDF creation failed: {error}"))
}

fn edit_error(error: pdfcraft_edit::EditError) -> EngineError {
    EngineError::Conversion(format!("PDF content insertion failed: {error}"))
}

fn organize_error(error: pdfcraft_organize::OrganizeError) -> EngineError {
    EngineError::Conversion(format!("PDF page organization failed: {error}"))
}

// PdfCraft searches only the first 256 PCFull-prefixed bindings for an existing
// font program, before its bounded global-object fallback. Keep one slot available
// for register_text_font so every subsequent row can reuse its program regardless
// of document size. This is a conservative native capability limit, not a lookup
// or font deduplication implementation in the adapter.
fn validate_font_reuse_profile(document: &Document, page: usize) -> PyResult<()> {
    let pages = pdfcraft_model::pages(document);
    let page = pages
        .get(page)
        .ok_or_else(|| EngineError::Malformed("font page is missing".into()))?;
    let Some(resources) = page.dict.get(b"Resources") else {
        return Ok(());
    };
    let resources = document.resolve(resources);
    let resources = resources.as_dict().ok_or_else(|| {
        EngineError::Unsupported("supplied-font drawing requires valid page resources".into())
    })?;
    let Some(fonts) = resources.get(b"Font") else {
        return Ok(());
    };
    let fonts = document.resolve(fonts);
    let fonts = fonts.as_dict().ok_or_else(|| {
        EngineError::Unsupported("supplied-font drawing requires valid font resources".into())
    })?;
    if fonts
        .iter()
        .filter(|(name, _)| name.starts_with(b"PCFull"))
        .take(256)
        .count()
        >= 256
    {
        return Err(EngineError::Unsupported("supplied-font drawing cannot guarantee bounded font-program reuse with 256 or more PCFull resources".into()).into());
    }
    Ok(())
}

const MAX_TEXT_WRAP_WORK: usize = 16_000_000;

fn text_wrap_work(text: &str) -> PyResult<usize> {
    let mut work = 0usize;
    let mut paragraphs = 0usize;
    for paragraph in text.split('\n') {
        paragraphs += 1;
        let length = paragraph.len().max(1);
        work = work.saturating_add(length.saturating_mul(length));
        // Each explicit paragraph produces at least one line. Reject excessive
        // line counts before native wrapping allocates its line vector.
        if work.saturating_mul(paragraphs) > MAX_TEXT_WRAP_WORK {
            return Err(EngineError::ResourceLimit(
                "PDF native text wrapping exceeds its work budget".into(),
            )
            .into());
        }
    }
    Ok(work)
}

fn text_area(
    x: f64,
    y: f64,
    page_width: f64,
    page_height: f64,
    bounds: Option<(f64, f64)>,
) -> PyResult<(f64, f64)> {
    validate_position(x, y, page_width, page_height)?;
    let (width, height) = bounds.unwrap_or((page_width - x, page_height - y));
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height <= 0.0
        || x + width > page_width
        || y + height > page_height
    {
        return Err(PyValueError::new_err(
            "text box must fit within the page, have positive height and be at least 1 point wide",
        ));
    }
    Ok((x + width, y + height))
}

// Existing indirect resources retain their shared Arc in a staged Document clone.
// Native replacements are only accepted when their small scalar dictionaries match;
// never invoke Object's recursive equality on an untrusted graph or decode a stream.
fn drawing_binding_unchanged(
    before: &Document,
    after: &Document,
    old: &pdfcraft_cos::Object,
    new: &pdfcraft_cos::Object,
) -> bool {
    use pdfcraft_cos::Object;
    let old = before.resolve(old);
    let new = after.resolve(new);
    if Arc::ptr_eq(&old, &new) {
        return true;
    }
    fn scalar_equal(old: &Object, new: &Object) -> bool {
        match (old, new) {
            (Object::Null, Object::Null) => true,
            (Object::Bool(a), Object::Bool(b)) => a == b,
            (Object::Int(a), Object::Int(b)) => a == b,
            (Object::Real(a), Object::Real(b)) => a == b,
            (Object::Int(a), Object::Real(b)) | (Object::Real(b), Object::Int(a)) => {
                a.unsigned_abs() <= (1_u64 << 53)
                    && b.is_finite()
                    && b.fract() == 0.0
                    && (*a as f64) == *b
            }
            (Object::Name(a), Object::Name(b)) => a.len() <= 256 && a == b,
            (Object::String(a), Object::String(b)) => a.bytes.len() <= 256 && a == b,
            _ => false,
        }
    }
    match (&*old, &*new) {
        (Object::Dict(a), Object::Dict(b)) => {
            let count = a.iter().take(17).count();
            count <= 16
                && b.iter().take(17).count() == count
                && a.iter().all(|(key, value)| {
                    key.len() <= 256 && b.get(key).is_some_and(|other| scalar_equal(value, other))
                })
        }
        (a, b) => scalar_equal(a, b),
    }
}

// The pinned drawing writers use several fixed resource names. Refuse a staged
// operation that would change an existing binding: rewriting names/operators here
// would duplicate the upstream resource allocator. Native page traversal supplies
// effective inherited resources, so parent dictionaries receive the same protection.
fn preserve_drawing_resources(before: &Document, after: &Document, page: usize) -> PyResult<()> {
    fn resources(document: &Document, page: usize) -> PyResult<pdfcraft_cos::Dict> {
        let pages = pdfcraft_model::pages(document);
        let page = pages.get(page).ok_or_else(|| {
            EngineError::Malformed("drawing page disappeared during resource validation".into())
        })?;
        match page.dict.get(b"Resources") {
            None => Ok(pdfcraft_cos::Dict::new()),
            Some(value) => document.dict(value).ok_or_else(|| {
                EngineError::Unsupported("drawing cannot preserve malformed page resources".into())
                    .into()
            }),
        }
    }
    let original = resources(before, page)?;
    let staged = resources(after, page)?;
    let mut checked = 0usize;
    for category in [b"Font".as_slice(), b"XObject", b"ExtGState"] {
        let Some(value) = original.get(category) else {
            continue;
        };
        let original_entries = before.dict(value).ok_or_else(|| {
            EngineError::Unsupported("drawing cannot preserve malformed resource bindings".into())
        })?;
        let staged_entries = staged.get(category).and_then(|value| after.dict(value));
        for (name, old) in original_entries.iter() {
            checked += 1;
            if checked > MAX_ITEMS {
                return Err(EngineError::ResourceLimit(
                    "drawing resource bindings exceed 10000".into(),
                )
                .into());
            }
            let preserved = staged_entries
                .as_ref()
                .and_then(|entries| entries.get(name))
                .is_some_and(|new| drawing_binding_unchanged(before, after, old, new));
            if !preserved {
                return Err(EngineError::Unsupported(format!(
                    "drawing cannot verify preservation of an existing PDF {} resource; collision-safe allocation is unavailable",
                    String::from_utf8_lossy(category),
                )).into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_stream_output_modes_preserve_native_content_and_representation() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(300.0, 400.0).unwrap();
        document
            .add_text(0, "Stable output", 20.0, 30.0, 12.0, (0, 0, 0))
            .unwrap();
        let image = crate::rendering::encode_png(2, 2, &[255, 0, 0, 255].repeat(4), 72.0).unwrap();
        document
            .add_image(0, &image, 20.0, 70.0, 30.0, 30.0, None, 0, false, false)
            .unwrap();
        document
            .add_link(
                0,
                (20.0, 20.0, 150.0, 45.0),
                Some("https://example.com"),
                None,
            )
            .unwrap();
        document.add_bookmark("First", 0, vec![], None).unwrap();
        document
            .update_metadata(r#"{"title":"Output modes","author":"Fixture"}"#)
            .unwrap();
        let baseline = document.to_bytes().unwrap();
        let baseline_png = crate::pdf::render(&baseline, 0, 72.0, None).unwrap();
        let before = (
            document.items,
            document.input_bytes,
            document.metadata().unwrap(),
            document.bookmarks().unwrap(),
        );
        for object_streams in [true, false] {
            let bytes = document.to_bytes_with_options(object_streams).unwrap();
            assert_eq!(
                bytes,
                document.to_bytes_with_options(object_streams).unwrap()
            );
            if object_streams {
                assert_eq!(bytes, baseline);
            }
            let startxref = bytes
                .windows(10)
                .rposition(|v| v == b"startxref\n")
                .unwrap()
                + 10;
            let offset = std::str::from_utf8(&bytes[startxref..])
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .parse::<usize>()
                .unwrap();
            assert_eq!(bytes[offset..].starts_with(b"xref\n"), !object_streams);
            let native = Document::open(Arc::new(bytes.clone())).unwrap();
            assert!(native.repair_log().is_empty());
            let packed = native
                .object_numbers()
                .into_iter()
                .filter(|&num| {
                    native
                        .try_get(num)
                        .unwrap()
                        .as_dict()
                        .is_some_and(|d| d.name(b"Type") == Some(b"ObjStm"))
                })
                .count();
            assert_eq!(packed > 0, object_streams);
            let info = pdfcraft_render::inspect(Arc::new(bytes.clone()), None).unwrap();
            assert_eq!(info.title.as_deref(), Some("Output modes"));
            assert_eq!(info.links.len(), 1);
            assert_eq!(info.outline.len(), 1);
            let extracted = crate::pdf::extract(&bytes, None).unwrap();
            assert!(extracted.pages[0]
                .blocks
                .iter()
                .any(|b| b.text.contains("Stable output")));
            assert_eq!(extracted.pages[0].images.len(), 1);
            assert_eq!(
                crate::pdf::render(&bytes, 0, 72.0, None).unwrap(),
                baseline_png
            );
        }
        assert_eq!(
            (
                document.items,
                document.input_bytes,
                document.metadata().unwrap(),
                document.bookmarks().unwrap()
            ),
            before
        );
        assert_eq!(document.to_bytes().unwrap(), baseline);
    }

    #[test]
    fn object_stream_output_cap_and_closed_checks_apply_to_both_modes() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(100.0, 100.0).unwrap();
        for object_streams in [true, false] {
            let bytes = document.to_bytes_with_options(object_streams).unwrap();
            assert_eq!(
                document
                    .serialize_with_limit(object_streams, bytes.len())
                    .unwrap(),
                bytes
            );
            let error = document
                .serialize_with_limit(object_streams, bytes.len() - 1)
                .unwrap_err();
            assert!(error
                .to_string()
                .contains("resource_limit:PDF output exceeds"));
            assert_eq!(
                document.to_bytes_with_options(object_streams).unwrap(),
                bytes
            );
        }
        document.close();
        for object_streams in [true, false] {
            assert!(document.to_bytes_with_options(object_streams).is_err());
        }
    }

    #[test]
    fn object_stream_output_modes_preserve_owned_fields_and_copies() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(300.0, 300.0).unwrap();
        document
            .add_field(0, "name", "text", (20.0, 20.0, 180.0, 30.0), vec![])
            .unwrap();
        document.set_field_value("name", r#""Alice""#).unwrap();
        document
            .add_field(0, "approved", "checkbox", (20.0, 80.0, 24.0, 24.0), vec![])
            .unwrap();
        document.set_field_value("approved", "true").unwrap();
        let before = document.fields(None).unwrap();
        let png = crate::pdf::render(&document.to_bytes().unwrap(), 0, 72.0, None).unwrap();
        let copied = document.copy().unwrap();
        for object_streams in [false, true] {
            let bytes = document.to_bytes_with_options(object_streams).unwrap();
            assert_eq!(bytes, copied.to_bytes_with_options(object_streams).unwrap());
            let native = Document::open(Arc::new(bytes.clone())).unwrap();
            assert!(native.repair_log().is_empty());
            let fields = checked_owned_fields(&native).unwrap();
            assert!(fields
                .iter()
                .any(|f| f.name == "name" && f.value == ["Alice"]));
            assert!(fields
                .iter()
                .any(|f| f.name == "approved" && f.value == [f.export_of(0).unwrap()]));
            assert_eq!(crate::pdf::render(&bytes, 0, 72.0, None).unwrap(), png);
        }
        assert_eq!(document.fields(None).unwrap(), before);
    }

    fn font_reuse_source(prefixes: usize, padding: usize, inherited: bool) -> Document {
        use pdfcraft_cos::{Dict, Object};
        let mut source = pdfcraft_create::blank(200.0, 200.0, 1).unwrap();
        let standard = collision_font_resources(b"unused");
        let standard = standard
            .get(b"Font")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"unused")
            .unwrap()
            .clone();
        let standard = source.add(standard);
        let mut fonts = Dict::new();
        for index in 0..prefixes {
            fonts.set(format!("PCFull{index}").into_bytes(), Object::Ref(standard));
        }
        let mut resources = Dict::new();
        resources.set(b"Font".to_vec(), Object::Dict(fonts));
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        let owner = if inherited {
            source
                .get(page)
                .as_dict()
                .unwrap()
                .reference(b"Parent")
                .unwrap()
        } else {
            page
        };
        source
            .update_dict(page, |dict| {
                dict.remove(b"Resources");
            })
            .unwrap();
        source
            .update_dict(owner, |dict| {
                dict.set(b"Resources".to_vec(), Object::Dict(resources));
            })
            .unwrap();
        let mut retained = Vec::new();
        for index in 0..padding {
            let mut dictionary = Dict::new();
            dictionary.set(b"AuditId".to_vec(), Object::Int(index as i64));
            retained.push(Object::Ref(source.add(Object::Dict(dictionary))));
        }
        // Reachable inert extension objects survive the native full-save garbage collection.
        source
            .update_dict(source.root().unwrap(), |dict| {
                dict.set(b"AuditPadding".to_vec(), Object::Array(retained));
            })
            .unwrap();
        source
    }

    fn full_font_program_count(document: &Document) -> usize {
        document.scan_objects().filter(|(_,object)| matches!(&**object,pdfcraft_cos::Object::Stream(stream) if stream.dict.get(b"PCFullFont").is_some())).count()
    }

    #[test]
    fn supplied_font_reuse_native_search_limits_are_reproducible() {
        let font = include_bytes!("../../tests/fixtures/fonts/Inter-Regular.ttf");
        for (prefixes, padding, expected) in [(255, 8200, 1), (256, 0, 1), (256, 8200, 3)] {
            let source = font_reuse_source(prefixes, padding, false);
            let mut document = EditablePdf::from_bytes(&resource_fixture_bytes(&source), None)
                .unwrap()
                .document
                .unwrap();
            pdfcraft_edit::text::register_text_font(&mut document, 0, font).unwrap();
            let page = pdfcraft_model::pages(&document).remove(0);
            let resources = document.dict(page.dict.get(b"Resources").unwrap()).unwrap();
            let mut fonts = document.dict(resources.get(b"Font").unwrap()).unwrap();
            let (_, family) = pdfcraft_fonts::embedding::names(font).unwrap();
            for line in ["row one", "row two"] {
                let shaped = pdfcraft_fonts::source_shaping::shape(font, line).unwrap();
                let mappings = shaped
                    .glyphs
                    .iter()
                    .map(|glyph| (glyph.text.clone(), glyph.id))
                    .collect::<Vec<_>>();
                pdfcraft_fonts::embedding::embed_mapping(
                    &mut document,
                    &mut fonts,
                    font,
                    &family,
                    &mappings,
                )
                .unwrap();
            }
            assert_eq!(
                full_font_program_count(&document),
                expected,
                "prefixes={prefixes}, padding={padding}"
            );
        }
    }

    #[test]
    fn supplied_font_reuse_guard_is_atomic_for_inherited_and_direct_resources() {
        pyo3::prepare_freethreaded_python();
        let font = include_bytes!("../../tests/fixtures/fonts/Inter-Regular.ttf");
        for inherited in [false, true] {
            for boxed in [false, true] {
                for prefixes in [255, 256] {
                    let source = font_reuse_source(prefixes, 8200, inherited);
                    let mut document =
                        EditablePdf::from_bytes(&resource_fixture_bytes(&source), None).unwrap();
                    let before = document.to_bytes().unwrap();
                    let counters = (document.items, document.input_bytes);
                    let result = if boxed {
                        document.add_text_box(
                            0,
                            "row one\nrow two",
                            (10.0, 10.0, 180.0, 60.0),
                            12.0,
                            (0, 0, 0),
                            "center",
                            Some(font),
                        )
                    } else {
                        document.add_text_with_font(
                            0,
                            "row one\nrow two",
                            10.0,
                            10.0,
                            12.0,
                            font,
                            (0, 0, 0),
                        )
                    };
                    if prefixes == 256 {
                        assert!(result.unwrap_err().to_string().contains("unsupported:supplied-font drawing cannot guarantee bounded font-program reuse"));
                        assert_eq!(document.to_bytes().unwrap(), before);
                        assert_eq!((document.items, document.input_bytes), counters);
                        assert_eq!(full_font_program_count(document.document().unwrap()), 0);
                    } else {
                        result.unwrap();
                        assert_eq!(full_font_program_count(document.document().unwrap()), 1);
                    }
                }
            }
        }
        let mut ordinary = EditablePdf::new(200.0, 200.0).unwrap();
        ordinary
            .add_text_with_font(0, "one\ntwo", 10.0, 10.0, 12.0, font, (0, 0, 0))
            .unwrap();
        ordinary
            .add_text_box(
                0,
                "three\nfour",
                (10.0, 60.0, 180.0, 60.0),
                12.0,
                (0, 0, 0),
                "right",
                Some(font),
            )
            .unwrap();
        assert_eq!(full_font_program_count(ordinary.document().unwrap()), 1);
    }

    fn pdf2_raster(document: &EditablePdf) -> (usize, usize, Vec<u8>) {
        let raster = crate::pdf::render(&document.to_bytes().unwrap(), 0, 72.0, None).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(raster)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        pixels.truncate(info.buffer_size());
        (info.width as usize, info.height as usize, pixels)
    }

    #[test]
    fn pdf2_builtin_boxes_wrap_align_and_reject_overflow_atomically() {
        let mut document = EditablePdf::new(220.0, 220.0).unwrap();
        for (row, align) in ["left", "center", "right"].iter().enumerate() {
            document
                .add_text_box(
                    0,
                    "Align",
                    (20.0, 20.0 + row as f64 * 40.0, 160.0, 30.0),
                    12.0,
                    (0, 0, 0),
                    align,
                    None,
                )
                .unwrap();
        }
        document
            .add_text_box(
                0,
                "one two three four five six",
                (20.0, 150.0, 70.0, 48.0),
                10.0,
                (0, 0, 0),
                "justify",
                None,
            )
            .unwrap();
        let (width, _, pixels) = pdf2_raster(&document);
        let mut minima = Vec::new();
        for row in 0..3 {
            let mut xs = Vec::new();
            for y in 20 + row * 40..50 + row * 40 {
                for x in 0..width {
                    let pixel = &pixels[(y * width + x) * 4..][..4];
                    if pixel[0] < 128 {
                        xs.push(x);
                    }
                }
            }
            assert!(!xs.is_empty());
            minima.push(*xs.iter().min().unwrap());
            assert!(*xs.iter().max().unwrap() <= 180);
        }
        assert!(minima[0] >= 20 && minima[0] < 25);
        assert!(minima[0] < minima[1] && minima[1] < minima[2]);
        let extracted = crate::pdf::extract(&document.to_bytes().unwrap(), None).unwrap();
        let text = extracted.pages[0]
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.matches("Align").count(), 3);
        assert!(text.contains("one two") && text.contains("five six"));
        let before = document.to_bytes().unwrap();
        for (text, rect) in [
            ("one\ntwo", (20.0, 20.0, 100.0, 23.99)),
            ("unbreakableword", (20.0, 20.0, 10.0, 100.0)),
            ("x", (200.0, 20.0, 30.0, 20.0)),
            ("x", (20.0, 20.0, f64::NAN, 20.0)),
            ("x", (20.0, 20.0, 20.0, 0.0)),
        ] {
            assert!(document
                .add_text_box(0, text, rect, 10.0, (0, 0, 0), "left", None)
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        document
            .add_text_box(
                0,
                "one\ntwo",
                (110.0, 150.0, 100.0, 24.0),
                10.0,
                (0, 0, 0),
                "left",
                None,
            )
            .unwrap();
    }

    #[test]
    fn boxed_font_ink_overflow_is_atomic_and_point_text_is_unchanged() {
        pyo3::prepare_freethreaded_python();
        let font = include_bytes!("../../tests/fixtures/fonts/Inter-Regular.ttf");
        let text = "\u{301}".repeat(6) + "A";
        let mut document = EditablePdf::new(300.0, 300.0).unwrap();
        let before = document.to_bytes().unwrap();
        let error = document
            .add_text_box(
                0,
                &text,
                (100.0, 100.0, 100.0, 100.0),
                30.0,
                (0, 0, 0),
                "left",
                Some(font),
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("glyph ink lies outside"),
            "{error}"
        );
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!((document.items, document.input_bytes), (0, 0));
        document
            .add_text_with_font(0, &text, 100.0, 100.0, 30.0, font, (0, 0, 0))
            .unwrap();
        let (width, _, pixels) = pdf2_raster(&document);
        assert!((0..100).any(|y| (0..width).any(|x| pixels[(y * width + x) * 4] < 200)));
        let mut boxed = EditablePdf::new(300.0, 300.0).unwrap();
        boxed
            .add_text_box(
                0,
                "A Café office",
                (100.0, 100.0, 150.0, 100.0),
                20.0,
                (0, 0, 0),
                "center",
                Some(font),
            )
            .unwrap();
        let (width, height, pixels) = pdf2_raster(&boxed);
        let ink = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .filter(|&(x, y)| pixels[(y * width + x) * 4] < 200)
            .collect::<Vec<_>>();
        assert!(!ink.is_empty());
        assert!(ink
            .iter()
            .all(|&(x, y)| (100..250).contains(&x) && (100..200).contains(&y)));
    }

    #[test]
    fn boxed_font_outline_cache_and_work_limits_are_bounded() {
        use skrifa::MetadataProvider;
        pyo3::prepare_freethreaded_python();
        let font = include_bytes!("../../tests/fixtures/fonts/Inter-Regular.ttf");
        let mut ink = TextInk::new(font, 30.0).unwrap();
        let id = ink.font.charmap().map('A').unwrap();
        let bounds = ink.bounds(id).unwrap().unwrap();
        let work = (ink.visits, ink.ops);
        assert!(bounds.width() > 0.0 && bounds.height() > 0.0);
        assert_eq!(ink.bounds(id).unwrap(), Some(bounds));
        assert_eq!((ink.visits, ink.ops), work);
        let space = ink.font.charmap().map(' ').unwrap();
        assert_eq!(ink.bounds(space).unwrap(), None);
        let mut exhausted = TextInk::new(font, 30.0).unwrap();
        exhausted.visits = MAX_INK_VISITS;
        assert!(matches!(
            exhausted.bounds(id),
            Err(EngineError::ResourceLimit(_))
        ));
        let mut exhausted = TextInk::new(font, 30.0).unwrap();
        exhausted.ops = MAX_INK_OPS;
        assert!(matches!(
            exhausted.bounds(id),
            Err(EngineError::ResourceLimit(_))
        ));
        let mut pen = InkPen {
            path: kurbo::BezPath::new(),
            remaining: 1,
            failed: false,
        };
        pen.push(kurbo::PathEl::MoveTo((0.0, 0.0).into()));
        pen.push(kurbo::PathEl::LineTo((1.0, 1.0).into()));
        assert!(pen.failed);
        assert_eq!(pen.path.elements().len(), 1);
    }

    #[test]
    fn pdf2_boxed_font_keeps_unicode_explicit_lines_and_native_alignment() {
        pyo3::prepare_freethreaded_python();
        let font = include_bytes!("../../tests/fixtures/fonts/Inter-Regular.ttf");
        let mut document = EditablePdf::new(220.0, 220.0).unwrap();
        for (row, align) in ["left", "center", "right"].iter().enumerate() {
            document
                .add_text_box(
                    0,
                    "Café office\nfi",
                    (20.0, 10.0 + row as f64 * 65.0, 180.0, 60.0),
                    12.0,
                    (0, 0, 180),
                    align,
                    Some(font),
                )
                .unwrap();
        }
        let extracted = crate::pdf::extract(&document.to_bytes().unwrap(), None).unwrap();
        let text = extracted.pages[0]
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.matches("Café office").count(), 3);
        let (width, _, pixels) = pdf2_raster(&document);
        let mut lefts = Vec::new();
        for row in 0..3 {
            let xs = (10 + row * 65..40 + row * 65)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .filter_map(|(x, y)| {
                    let p = &pixels[(y * width + x) * 4..][..4];
                    (i16::from(p[2]) > i16::from(p[0]) + 20).then_some(x)
                })
                .collect::<Vec<_>>();
            lefts.push(*xs.iter().min().unwrap());
        }
        assert!(lefts[0] < lefts[1] && lefts[1] < lefts[2]);
        let before = document.to_bytes().unwrap();
        assert!(document
            .add_text_box(
                0,
                "text",
                (20.0, 20.0, 100.0, 40.0),
                12.0,
                (0, 0, 0),
                "justify",
                Some(font)
            )
            .unwrap_err()
            .to_string()
            .contains("unsupported:"));
        assert!(document
            .add_text_box(
                0,
                "this explicit row is too wide",
                (20.0, 20.0, 20.0, 100.0),
                12.0,
                (0, 0, 0),
                "left",
                Some(font)
            )
            .is_err());
        assert!(document
            .add_text_box(
                0,
                "row\nrow",
                (20.0, 20.0, 100.0, 10.0),
                12.0,
                (0, 0, 0),
                "left",
                Some(font)
            )
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
    }

    #[test]
    fn pdf2_wrapping_work_budget_applies_before_native_legacy_and_boxed_layout() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(220.0, 220.0).unwrap();
        let before = document.to_bytes().unwrap();
        let text = "x".repeat(4001);
        assert!(document
            .add_text(0, &text, 10.0, 10.0, 12.0, (0, 0, 0))
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert!(document
            .add_text_box(
                0,
                &text,
                (10.0, 10.0, 200.0, 200.0),
                12.0,
                (0, 0, 0),
                "left",
                None
            )
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        // Short individual words also bound repeated full layout within native draw().
        let repeated = "a ".repeat(1000);
        assert!(document
            .add_text_box(
                0,
                &repeated,
                (10.0, 10.0, 200.0, 200.0),
                12.0,
                (0, 0, 0),
                "left",
                None
            )
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert!(document
            .add_text(
                0,
                &format!("x{}", "\n".repeat(4001)),
                10.0,
                10.0,
                12.0,
                (0, 0, 0)
            )
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!((document.items, document.input_bytes), (0, 0));
        let source = resource_fixture(collision_font_resources(b"PCFHelvetica"), b"", true);
        let mut imported = EditablePdf::from_bytes(&resource_fixture_bytes(&source), None).unwrap();
        let before = imported.to_bytes().unwrap();
        assert!(imported
            .add_text_box(
                0,
                "new",
                (10.0, 10.0, 100.0, 40.0),
                12.0,
                (0, 0, 0),
                "center",
                None
            )
            .unwrap_err()
            .to_string()
            .contains("unsupported:drawing cannot verify preservation"));
        assert_eq!(imported.to_bytes().unwrap(), before);
    }

    fn pdf2_quadrants() -> Vec<u8> {
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let mut rgba = Vec::new();
        for y in 0..20 {
            for x in 0..20 {
                rgba.extend_from_slice(&colors[(y / 10) * 2 + x / 10]);
            }
        }
        crate::rendering::encode_png(20, 20, &rgba, 72.0).unwrap()
    }

    #[test]
    fn pdf2_image_transform_pixels_follow_crop_flip_and_counterclockwise_rotation() {
        let image = pdf2_quadrants();
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let cases = [
            (0, false, false, None, [0, 1, 2, 3]),
            (90, false, false, None, [1, 3, 0, 2]),
            (180, false, false, None, [3, 2, 1, 0]),
            (270, false, false, None, [2, 0, 3, 1]),
            (0, true, false, None, [1, 0, 3, 2]),
            (0, false, true, None, [2, 3, 0, 1]),
            (0, false, false, Some((0.45, 0.45, 0.0, 0.0)), [3, 3, 3, 3]),
            (0, false, false, Some((0.0, 0.0, 0.45, 0.45)), [0, 0, 0, 0]),
            (90, false, true, Some((0.45, 0.0, 0.0, 0.0)), [3, 1, 3, 1]),
        ];
        let mut original_media = None;
        for (rotation, flip_h, flip_v, crop, expected) in cases {
            let mut document = EditablePdf::new(140.0, 140.0).unwrap();
            document
                .add_image(
                    0, &image, 30.0, 30.0, 80.0, 80.0, crop, rotation, flip_h, flip_v,
                )
                .unwrap();
            let (width, _, pixels) = pdf2_raster(&document);
            for ((x, y), index) in [(50, 50), (90, 50), (50, 90), (90, 90)]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(
                    &pixels[(y * width + x) * 4..][..4],
                    &colors[index],
                    "rotation {rotation}, flips {flip_h}/{flip_v}, crop {crop:?}"
                );
            }
            assert_eq!(&pixels[(20 * width + 20) * 4..][..4], &[255, 255, 255, 255]);
            let native =
                pdfcraft_edit::images::reading_images(document.document().unwrap(), 0).unwrap();
            assert_eq!(native.len(), 1);
            let object = document.document().unwrap().get(native[0].object.unwrap());
            let data = match &*object {
                pdfcraft_cos::Object::Stream(stream) => stream.raw.to_vec(),
                _ => panic!("image resource"),
            };
            match &original_media {
                None => original_media = Some(data),
                Some(original) => assert_eq!(&data, original),
            }
        }
    }

    #[test]
    fn pdf2_image_transform_validation_is_atomic() {
        let image = pdf2_quadrants();
        let mut document = EditablePdf::new(140.0, 140.0).unwrap();
        let before = document.to_bytes().unwrap();
        for crop in [
            (0.46, 0.0, 0.0, 0.0),
            (0.0, -0.1, 0.0, 0.0),
            (f64::NAN, 0.0, 0.0, 0.0),
            (0.0, 0.0, 0.0, f64::INFINITY),
        ] {
            assert!(document
                .add_image(
                    0,
                    &image,
                    30.0,
                    30.0,
                    80.0,
                    80.0,
                    Some(crop),
                    0,
                    false,
                    false
                )
                .is_err());
        }
        for rotation in [-90, 1, 45, 360] {
            assert!(document
                .add_image(0, &image, 30.0, 30.0, 80.0, 80.0, None, rotation, false, false)
                .is_err());
        }
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!((document.items, document.input_bytes), (0, 0));
    }

    #[test]
    fn pdf2_boxes_and_transformed_images_follow_rotated_cropped_user_unit_view() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |dict| {
                dict.set(
                    b"CropBox".to_vec(),
                    Object::Array([20.0, 30.0, 220.0, 330.0].map(Object::Real).to_vec()),
                );
                dict.set(b"UserUnit".to_vec(), Object::Real(1.25));
            })
            .unwrap();
        let bytes = resource_fixture_bytes(&source);
        let image = pdf2_quadrants();
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::from_bytes(&bytes, None).unwrap();
            document.rotate_page(0, rotation).unwrap();
            document
                .add_text_box(
                    0,
                    "Text box",
                    (20.0, 20.0, 120.0, 30.0),
                    12.0,
                    (0, 0, 0),
                    "right",
                    None,
                )
                .unwrap();
            document
                .add_image(0, &image, 20.0, 80.0, 80.0, 80.0, None, 90, false, false)
                .unwrap();
            let (width, _, pixels) = pdf2_raster(&document);
            assert_eq!(&pixels[(100 * width + 40) * 4..][..4], &[0, 255, 0, 255]);
            assert_eq!(&pixels[(140 * width + 80) * 4..][..4], &[0, 0, 255, 255]);
            let extracted = crate::pdf::extract(&document.to_bytes().unwrap(), None).unwrap();
            assert!(extracted.pages[0]
                .blocks
                .iter()
                .any(|block| block.text.contains("Text box")));
        }
    }

    fn resource_fixture(
        resources: pdfcraft_cos::Dict,
        content: &[u8],
        inherited: bool,
    ) -> Document {
        use pdfcraft_cos::{Dict, Object, Stream};
        let mut document = pdfcraft_create::blank(200.0, 200.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&document).unwrap()[0].obj;
        let stream = document.add(Object::Stream(Stream::flate(Dict::new(), content)));
        document
            .update_dict(page, |dict| {
                dict.set(b"Contents".to_vec(), Object::Ref(stream));
            })
            .unwrap();
        let owner = if inherited {
            document
                .get(page)
                .as_dict()
                .unwrap()
                .reference(b"Parent")
                .unwrap()
        } else {
            page
        };
        document
            .update_dict(page, |dict| {
                dict.remove(b"Resources");
            })
            .unwrap();
        document
            .update_dict(owner, |dict| {
                dict.set(b"Resources".to_vec(), Object::Dict(resources));
            })
            .unwrap();
        document
    }

    fn collision_font_resources(name: &[u8]) -> pdfcraft_cos::Dict {
        use pdfcraft_cos::{Dict, Object};
        let mut font = Dict::new();
        font.set(b"Type".to_vec(), Object::name("Font"));
        font.set(b"Subtype".to_vec(), Object::name("Type1"));
        font.set(b"BaseFont".to_vec(), Object::name("Courier"));
        font.set(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
        let mut fonts = Dict::new();
        fonts.set(name.to_vec(), Object::Dict(font));
        let mut resources = Dict::new();
        resources.set(b"Font".to_vec(), Object::Dict(fonts));
        resources
    }

    fn resource_fixture_bytes(document: &Document) -> Vec<u8> {
        pdfcraft_cos::write_full(document, &SaveOptions::default()).unwrap()
    }

    fn original_resource_pixels(document: &Document) -> Vec<u8> {
        crate::pdf::render_region(
            &resource_fixture_bytes(document),
            0,
            72.0,
            None,
            (0.0, 0.0, 200.0, 70.0),
        )
        .unwrap()
    }

    #[test]
    fn drawing_resources_reject_fixed_font_collisions_inherited_and_direct() {
        pyo3::prepare_freethreaded_python();
        for inherited in [false, true] {
            let source = resource_fixture(
                collision_font_resources(b"PCHelv"),
                b"BT /PCHelv 20 Tf 1 0 0 1 10 150 Tm (MMMM WWWW) Tj ET",
                inherited,
            );
            let before = original_resource_pixels(&source);
            let mut native = source.clone();
            // An otherwise empty native stamp changes the preexisting text's font.
            pdfcraft_edit::stamp(&mut native, 0, "repro", b"q Q\n".to_vec()).unwrap();
            assert_ne!(original_resource_pixels(&native), before);
            let source_bytes = resource_fixture_bytes(&source);
            let mut editable = EditablePdf::from_bytes(&source_bytes, None).unwrap();
            let saved = editable.to_bytes().unwrap();
            let counts = (editable.items, editable.input_bytes);
            let path = r#"[["move",10,100],["line",30,100]]"#;
            let error = editable
                .draw_path(0, path, Some((0, 0, 0)), None, 1.0, false)
                .unwrap_err();
            assert!(error
                .to_string()
                .contains("unsupported:drawing cannot verify preservation"));
            assert_eq!(editable.to_bytes().unwrap(), saved);
            assert!(editable.set_background(0, (255, 255, 255)).is_err());
            assert_eq!(editable.to_bytes().unwrap(), saved);
            assert!(editable
                .add_text_with_font(
                    0,
                    "new",
                    10.0,
                    100.0,
                    12.0,
                    include_bytes!("../../tests/fixtures/fonts/Inter-Regular.ttf"),
                    (0, 0, 0)
                )
                .is_err());
            assert_eq!(editable.to_bytes().unwrap(), saved);
            assert_eq!((editable.items, editable.input_bytes), counts);
            // Methods whose native writer does not touch this binding remain available.
            editable
                .add_text(0, "new", 10.0, 100.0, 12.0, (0, 0, 0))
                .unwrap();
            let image = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
            editable
                .add_image(0, &image, 10.0, 130.0, 20.0, 20.0, None, 0, false, false)
                .unwrap();
            assert_eq!(
                original_resource_pixels(editable.document().unwrap()),
                before
            );
        }
    }

    #[test]
    fn drawing_resources_reject_added_text_font_collision() {
        pyo3::prepare_freethreaded_python();
        let source = resource_fixture(
            collision_font_resources(b"PCFHelvetica"),
            b"BT /PCFHelvetica 20 Tf 1 0 0 1 10 150 Tm (MMMM WWWW) Tj ET",
            false,
        );
        let item = AddedText {
            rect: [10.0, 0.0, 190.0, 100.0],
            text: "new".into(),
            ..AddedText::default()
        };
        let mut native = source.clone();
        pdfcraft_edit::add_content(&mut native, 0, &Content::Text(item)).unwrap();
        assert_ne!(
            original_resource_pixels(&native),
            original_resource_pixels(&source)
        );
        let mut editable = EditablePdf::from_bytes(&resource_fixture_bytes(&source), None).unwrap();
        let before = editable.to_bytes().unwrap();
        assert!(editable
            .add_text(0, "new", 10.0, 100.0, 12.0, (0, 0, 0))
            .unwrap_err()
            .to_string()
            .contains("unsupported:drawing cannot verify preservation"));
        assert_eq!(editable.to_bytes().unwrap(), before);
    }

    #[test]
    fn drawing_resources_reject_background_opacity_collision() {
        pyo3::prepare_freethreaded_python();
        use pdfcraft_cos::{Dict, Object};
        let mut state = Dict::new();
        state.set(b"Type".to_vec(), Object::name("ExtGState"));
        state.set(b"ca".to_vec(), Object::Real(0.5));
        state.set(b"CA".to_vec(), Object::Real(0.5));
        let mut states = Dict::new();
        states.set(b"PCGS100".to_vec(), Object::Dict(state));
        let mut resources = Dict::new();
        resources.set(b"ExtGState".to_vec(), Object::Dict(states));
        let source = resource_fixture(
            resources,
            b"q /PCGS100 gs 1 0 0 rg 10 140 40 40 re f Q",
            true,
        );
        let mut native = source.clone();
        let background = pdfcraft_edit::Background {
            color: [1.0; 3],
            ..Default::default()
        };
        pdfcraft_edit::add_background(&mut native, &[0], &background, true).unwrap();
        assert_ne!(
            original_resource_pixels(&native),
            original_resource_pixels(&source)
        );
        let mut editable = EditablePdf::from_bytes(&resource_fixture_bytes(&source), None).unwrap();
        let before = editable.to_bytes().unwrap();
        assert!(editable
            .set_background(0, (255, 255, 255))
            .unwrap_err()
            .to_string()
            .contains("unsupported:drawing cannot verify preservation"));
        assert_eq!(editable.to_bytes().unwrap(), before);
        assert!(editable.background(0).is_err());
    }

    #[test]
    fn drawing_resources_reject_image_name_collision() {
        pyo3::prepare_freethreaded_python();
        use pdfcraft_cos::{Dict, Object, Stream};
        let red = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
        let blue = crate::rendering::encode_png(1, 1, &[0, 0, 255, 255], 72.0).unwrap();
        let mut source = pdfcraft_create::blank(200.0, 200.0, 1).unwrap();
        let (old_image, _) = pdfcraft_create::image_xobject(&mut source, "old.png", &red).unwrap();
        let stream = source.add(Object::Stream(Stream::flate(Dict::new(), b"")));
        let mut probe = source.clone();
        let (future_image, _) =
            pdfcraft_create::image_xobject(&mut probe, "new.png", &blue).unwrap();
        let mut images = Dict::new();
        let mut content = String::new();
        // Cover subsequent object numbers too: saving/reopening can allocate an xref object.
        for offset in 0..16 {
            let name = format!("PCImg{}", future_image.num + offset);
            images.set(name.as_bytes().to_vec(), Object::Ref(old_image));
            content.push_str(&format!(
                "q 8 0 0 40 {} 140 cm /{name} Do Q\n",
                10 + offset * 10
            ));
        }
        let mut resources = Dict::new();
        resources.set(b"XObject".to_vec(), Object::Dict(images));
        source.set(
            stream,
            Object::Stream(Stream::flate(Dict::new(), content.as_bytes())),
        );
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |dict| {
                dict.set(b"Resources".to_vec(), Object::Dict(resources));
                dict.set(b"Contents".to_vec(), Object::Ref(stream));
            })
            .unwrap();
        let mut editable = EditablePdf::from_bytes(&resource_fixture_bytes(&source), None).unwrap();
        let before = editable.to_bytes().unwrap();
        let original_pixels = original_resource_pixels(editable.document().unwrap());
        let mut native = editable.document().unwrap().clone();
        let (new_image, _) = pdfcraft_create::image_xobject(&mut native, "new.png", &blue).unwrap();
        assert!((future_image.num..future_image.num + 16).contains(&new_image.num));
        pdfcraft_edit::add_content(
            &mut native,
            0,
            &Content::Image(AddedImage::new([10.0, 40.0, 30.0, 60.0], new_image)),
        )
        .unwrap();
        assert_ne!(original_resource_pixels(&native), original_pixels);
        assert!(editable
            .add_image(0, &blue, 10.0, 140.0, 20.0, 20.0, None, 0, false, false)
            .unwrap_err()
            .to_string()
            .contains("unsupported:drawing cannot verify preservation"));
        assert_eq!(editable.to_bytes().unwrap(), before);
        assert_eq!(
            original_resource_pixels(editable.document().unwrap()),
            original_pixels
        );
    }

    #[test]
    fn drawing_resources_allow_matching_bindings_and_bound_checks() {
        pyo3::prepare_freethreaded_python();
        use pdfcraft_cos::{Dict, Object};
        // A reference number alone must not prove preservation when its object changes.
        let mut source = pdfcraft_create::blank(200.0, 200.0, 1).unwrap();
        let reference = source.add(Object::Int(1));
        let mut changed = source.clone();
        changed.set(reference, Object::Int(2));
        assert!(!drawing_binding_unchanged(
            &source,
            &changed,
            &Object::Ref(reference),
            &Object::Ref(reference)
        ));
        assert!(!drawing_binding_unchanged(
            &source,
            &source,
            &Object::Int((1_i64 << 53) + 1),
            &Object::Real((1_u64 << 53) as f64),
        ));
        let complex = Object::Array(vec![Object::Int(1)]);
        assert!(!drawing_binding_unchanged(
            &source, &source, &complex, &complex
        ));
        let mut editable = EditablePdf::new(200.0, 200.0).unwrap();
        editable.set_background(0, (255, 255, 255)).unwrap();
        editable.set_background(0, (240, 240, 240)).unwrap();
        editable
            .add_text(0, "one", 10.0, 10.0, 12.0, (0, 0, 0))
            .unwrap();
        editable
            .add_text(0, "two", 10.0, 40.0, 12.0, (0, 0, 0))
            .unwrap();
        editable
            .draw_path(
                0,
                r#"[["move",10,100],["line",30,100]]"#,
                Some((0, 0, 0)),
                None,
                1.0,
                false,
            )
            .unwrap();
        let bytes = editable.to_bytes().unwrap();
        let mut imported = EditablePdf::from_bytes(&bytes, None).unwrap();
        imported
            .add_text(0, "three", 10.0, 140.0, 12.0, (0, 0, 0))
            .unwrap();
        imported.set_background(0, (250, 250, 250)).unwrap();
        let mut fonts = Dict::new();
        for i in 0..=MAX_ITEMS {
            fonts.set(format!("F{i}").into_bytes(), Object::Dict(Dict::new()));
        }
        let mut resources = Dict::new();
        resources.set(b"Font".to_vec(), Object::Dict(fonts));
        let source = resource_fixture(resources, b"", false);
        let mut editable = EditablePdf::from_bytes(&resource_fixture_bytes(&source), None).unwrap();
        let before = editable.to_bytes().unwrap();
        assert!(editable
            .add_text(0, "new", 10.0, 100.0, 12.0, (0, 0, 0))
            .unwrap_err()
            .to_string()
            .contains("resource_limit:drawing resource bindings"));
        assert_eq!(editable.to_bytes().unwrap(), before);
    }

    #[test]
    fn outline_import_maps_nested_omitted_and_duplicate_targets_with_links() {
        let mut source = EditablePdf::new(200.0, 240.0).unwrap();
        source.add_page(200.0, 240.0).unwrap();
        source.add_page(200.0, 240.0).unwrap();
        source.add_bookmark("A", 0, vec![], None).unwrap();
        source.add_bookmark("B nested", 1, vec![0], None).unwrap();
        source.add_bookmark("Omitted", 2, vec![], None).unwrap();
        source
            .add_link(
                0,
                (10.0, 10.0, 100.0, 25.0),
                Some("https://example.com"),
                None,
            )
            .unwrap();
        source
            .add_link(0, (10.0, 40.0, 100.0, 55.0), None, Some(1))
            .unwrap();
        source
            .add_link(0, (10.0, 70.0, 100.0, 85.0), None, Some(2))
            .unwrap();
        let bytes = source.to_bytes().unwrap();
        let mut destination = EditablePdf::new(200.0, 240.0).unwrap();
        destination
            .add_bookmark("Existing", 0, vec![], None)
            .unwrap();
        destination
            .add_field(0, "kept", "text", (10.0, 10.0, 150.0, 30.0), vec![])
            .unwrap();
        assert_eq!(
            destination
                .import_pages(&bytes, None, Some(vec![1, 0, 1]), 0, Some("Imported"))
                .unwrap(),
            3
        );
        let entries = destination.bookmark_entries().unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| (e.title.as_str(), e.page))
                .collect::<Vec<_>>(),
            vec![
                ("Imported", Some(0)),
                ("A", Some(1)),
                ("B nested", Some(0)),
                ("Omitted", None),
                ("Existing", Some(3)),
            ]
        );
        assert_eq!(entries[2].path, vec![0, 0, 0]);
        let links: Vec<Value> = serde_json::from_str(&destination.annotations(1).unwrap()).unwrap();
        assert!(links.iter().any(|v| v["uri"] == "https://example.com"));
        assert!(links.iter().any(|v| v["target_page"] == 0));
        assert_eq!(links.len(), 3);
        assert_eq!(
            links
                .iter()
                .filter(|v| v["uri"].is_null() && v["target_page"].is_null())
                .count(),
            1,
            "native import retains the omitted-page annotation without its destination"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&destination.field_info("kept").unwrap()).unwrap()
                ["widgets"][0]["page"],
            3
        );
        destination.move_page(0, 3).unwrap();
        destination.rotate_page(0, 90).unwrap();
        let entries = destination.bookmark_entries().unwrap();
        assert_eq!(entries[1].page, Some(0));
        assert_eq!(entries[2].page, Some(3));
        destination.delete_page(3).unwrap();
        let entries = destination.bookmark_entries().unwrap();
        assert_eq!(
            entries[2].page, None,
            "deleted first copy is not retargeted to a duplicate"
        );
        let output = destination.to_bytes().unwrap();
        let inspection = pdfcraft_render::inspect(Arc::new(output.clone()), None).unwrap();
        assert!(!inspection.outline_more);
        assert!(inspection.links.iter().any(|link| matches!(&link.target, pdfcraft_render::LinkTarget::Uri(uri) if uri == "https://example.com")));
        assert!(inspection.links.iter().all(|link| !matches!(link.target, pdfcraft_render::LinkTarget::Page(page, _) if page >= inspection.pages.len())));
        assert!(crate::pdf::render(&output, 0, 72.0, None).is_ok());
    }

    #[test]
    fn outline_import_is_opt_in_and_empty_sources_add_no_container() {
        let blank = EditablePdf::new(100.0, 100.0).unwrap().to_bytes().unwrap();
        let mut destination = EditablePdf::new(100.0, 100.0).unwrap();
        destination
            .import_pages(&blank, None, None, 1, Some("No outline"))
            .unwrap();
        assert_eq!(destination.bookmarks().unwrap(), "[]");
        let mut source = EditablePdf::new(100.0, 100.0).unwrap();
        source
            .add_bookmark("Not imported", 0, vec![], None)
            .unwrap();
        destination
            .import_pages(&source.to_bytes().unwrap(), None, None, 2, None)
            .unwrap();
        assert_eq!(destination.bookmarks().unwrap(), "[]");
        for title in ["", "  ", "bad\0title", &"x".repeat(4097)] {
            let before = destination.to_bytes().unwrap();
            assert!(destination
                .import_pages(&blank, None, None, 1, Some(title))
                .is_err());
            assert_eq!(destination.to_bytes().unwrap(), before);
        }
        let longest = "x".repeat(4096);
        destination
            .import_pages(&source.to_bytes().unwrap(), None, None, 0, Some(&longest))
            .unwrap();
        assert_eq!(destination.bookmark_entries().unwrap()[0].title, longest);
    }

    #[test]
    fn outline_import_budget_and_unpreserved_actions_fail_without_mutation() {
        use pdfcraft_cos::{Dict, Object, PdfString};
        let mut destination = EditablePdf::new(100.0, 100.0).unwrap();
        let before = destination.to_bytes().unwrap();
        let mut deep = EditablePdf::new(100.0, 100.0).unwrap();
        let mut parent = vec![];
        for _ in 0..32 {
            parent = deep.add_bookmark("Deep", 0, parent, None).unwrap();
        }
        assert!(destination
            .import_pages(&deep.to_bytes().unwrap(), None, None, 0, Some("Parent"))
            .is_err());
        assert_eq!(destination.to_bytes().unwrap(), before);
        let mut many = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        let entries = (0..9999)
            .map(|index| pdfcraft_organize::OutlineEntry {
                level: 1,
                title: format!("Entry {index}"),
                page: 0,
                element: None,
            })
            .collect::<Vec<_>>();
        pdfcraft_organize::add_bookmark_tree(&mut many, "Many", &entries).unwrap();
        let bytes = pdfcraft_cos::write_full(&many, &SaveOptions::default()).unwrap();
        assert!(destination
            .import_pages(&bytes, None, None, 0, Some("Parent"))
            .is_err());
        assert_eq!(destination.to_bytes().unwrap(), before);
        let mut external = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        pdfcraft_organize::add_bookmark(&mut external, &[], 0, "External", 0).unwrap();
        let bookmark = pdfcraft_organize::bookmarks(&external)[0].obj;
        external
            .update_dict(bookmark, |item| {
                item.remove(b"Dest");
                let mut action = Dict::new();
                action.set(b"S".to_vec(), Object::name("URI"));
                action.set(
                    b"URI".to_vec(),
                    Object::String(PdfString::text("https://example.com")),
                );
                item.set(b"A".to_vec(), Object::Dict(action));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&external, &SaveOptions::default()).unwrap();
        assert!(destination
            .import_pages(&bytes, None, None, 0, Some("Parent"))
            .is_err());
        assert_eq!(destination.to_bytes().unwrap(), before);
        // Public outline inspection follows eight destination wrappers; native import
        // follows four. A resolvable target must fail atomically rather than silently vanish.
        let mut indirect = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        pdfcraft_organize::add_bookmark(&mut indirect, &[], 0, "Indirect", 0).unwrap();
        let bookmark = pdfcraft_organize::bookmarks(&indirect)[0].obj;
        let mut target = indirect
            .get(bookmark)
            .as_dict()
            .unwrap()
            .get(b"Dest")
            .unwrap()
            .clone();
        for _ in 0..6 {
            let mut wrapper = Dict::new();
            wrapper.set(b"D".to_vec(), target);
            target = Object::Dict(wrapper);
        }
        indirect
            .update_dict(bookmark, |item| item.set(b"Dest".to_vec(), target))
            .unwrap();
        assert_eq!(pdfcraft_organize::bookmarks(&indirect)[0].page, Some(0));
        let bytes = pdfcraft_cos::write_full(&indirect, &SaveOptions::default()).unwrap();
        assert!(destination
            .import_pages(&bytes, None, None, 0, Some("Parent"))
            .is_err());
        assert_eq!(destination.to_bytes().unwrap(), before);
        assert_eq!(destination.page_count().unwrap(), 1);
        assert_eq!(destination.input_bytes, 0);
        assert_eq!(destination.items, 0);
    }

    fn tooltip_appearance_data(
        document: &Document,
        field: &pdfcraft_forms::Field,
    ) -> Vec<(Vec<u8>, Vec<u8>)> {
        use pdfcraft_cos::Object;
        let widget = document.get(field.widgets[0].obj);
        let ap = document.resolve(widget.as_dict().unwrap().get(b"AP").unwrap());
        let normal = document.resolve(ap.as_dict().unwrap().get(b"N").unwrap());
        match normal.as_ref() {
            Object::Stream(stream) => vec![(vec![], stream.raw.to_vec())],
            Object::Dict(states) => states
                .iter()
                .map(|(name, value)| {
                    let value = document.resolve(value);
                    let Object::Stream(stream) = value.as_ref() else {
                        panic!("owned state appearance")
                    };
                    (name.clone(), stream.raw.to_vec())
                })
                .collect(),
            _ => panic!("owned appearance"),
        }
    }

    #[test]
    fn field_tooltip_native_roundtrip_preserves_every_owned_kind_and_appearance() {
        pyo3::prepare_freethreaded_python();
        for (kind, options, value) in [
            ("text", vec![], r#""Customer""#),
            ("checkbox", vec![], "true"),
            (
                "choice",
                vec!["Canada".into(), "France".into()],
                r#""France""#,
            ),
        ] {
            let mut document = EditablePdf::new(300.0, 300.0).unwrap();
            document
                .add_field(0, "field", kind, (20.0, 20.0, 180.0, 30.0), options)
                .unwrap();
            document.set_field_value("field", value).unwrap();
            let original = document.field_named("field").unwrap();
            let appearance = tooltip_appearance_data(document.document().unwrap(), &original);
            let original_png =
                crate::pdf::render(&document.to_bytes().unwrap(), 0, 144.0, None).unwrap();
            let tooltip = "  Customer — 顧客\nHelp Ω  ";
            let before_input = document.input_bytes;
            document.set_field_tooltip("field", tooltip).unwrap();
            let actual = document.field_named("field").unwrap();
            let mut expected = original.clone();
            expected.tooltip = Some(tooltip.into());
            assert_eq!(actual, expected);
            assert_eq!(
                tooltip_appearance_data(document.document().unwrap(), &actual),
                appearance
            );
            assert_eq!(
                document.input_bytes - before_input,
                owned_field_appearance_bytes(document.document().unwrap(), &actual).unwrap()
                    + tooltip.len() * 2
                    + 32
            );
            let saved = document.to_bytes().unwrap();
            let parsed = Document::open(Arc::new(saved.clone())).unwrap();
            let read = pdfcraft_forms::fields(&parsed);
            assert_eq!(read.len(), 1);
            assert_eq!(read[0].tooltip.as_deref(), Some(tooltip));
            assert_eq!(read[0].value, original.value);
            assert_eq!(read[0].flags, original.flags);
            assert_eq!(read[0].options, original.options);
            assert_eq!(
                crate::pdf::render(&saved, 0, 144.0, None).unwrap(),
                original_png
            );
            let mut copied = document.copy().unwrap();
            copied.set_field_tooltip("field", "Copy").unwrap();
            assert_eq!(
                document.field_named("field").unwrap().tooltip.as_deref(),
                Some(tooltip)
            );
            copied
                .set_field_box("field", (40.0, 100.0, 200.0, 40.0))
                .unwrap();
            copied.set_field_value("field", value).unwrap();
            assert_eq!(
                copied.field_named("field").unwrap().tooltip.as_deref(),
                Some("Copy")
            );
            document.set_field_tooltip("field", "").unwrap();
            assert_eq!(document.field_named("field").unwrap(), original);
            let field_object = document.document().unwrap().get(original.obj);
            assert!(!field_object.as_dict().unwrap().contains(b"TU"));
            assert_eq!(
                crate::pdf::render(&document.to_bytes().unwrap(), 0, 144.0, None).unwrap(),
                original_png
            );
            document.flatten_fields().unwrap();
            assert_eq!(
                crate::pdf::render(&document.to_bytes().unwrap(), 0, 144.0, None).unwrap(),
                original_png
            );
        }
    }

    #[test]
    fn field_tooltip_limits_and_mutation_budgets_are_atomic() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(300.0, 300.0).unwrap();
        document
            .add_field(0, "field", "text", (20.0, 20.0, 180.0, 30.0), vec![])
            .unwrap();
        let limit = "é".repeat(2048);
        document.set_field_tooltip("field", &limit).unwrap();
        assert_eq!(
            document.field_named("field").unwrap().tooltip.as_deref(),
            Some(limit.as_str())
        );
        let before = document.to_bytes().unwrap();
        let items = document.items;
        let input = document.input_bytes;
        for bad in [format!("{limit}x"), "bad\0tooltip".into()] {
            assert!(document.set_field_tooltip("field", &bad).is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
            assert_eq!((document.items, document.input_bytes), (items, input));
        }
        document.input_bytes = MAX_DOCUMENT_INPUT - 1;
        assert!(document
            .set_field_tooltip("field", "Change")
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!(document.input_bytes, MAX_DOCUMENT_INPUT - 1);
        document.input_bytes = input;
        document.items = MAX_ITEMS;
        assert!(document
            .set_field_tooltip("field", "Change")
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!(document.items, MAX_ITEMS);
    }

    #[test]
    fn field_tooltip_aggregate_live_text_exact_limit_and_plus_one() {
        use pdfcraft_cos::{Object, PdfString};
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(300.0, 300.0).unwrap();
        document
            .add_field(0, "seed", "checkbox", (20.0, 20.0, 30.0, 30.0), vec![])
            .unwrap();
        let seed = document.field_named("seed").unwrap();
        // Synthetic bounded owned graph: clone the native single-widget dictionary,
        // share its existing native appearances, and give every field a distinct name.
        // No imported form reaches the runtime's owned-only traversal.
        let native = document.document.as_mut().unwrap();
        let template = native.get(seed.obj).as_dict().unwrap().clone();
        let page = pdfcraft_organize::pages(native).unwrap()[0].obj;
        let root = native.root().unwrap();
        let acroform = native
            .get(root)
            .as_dict()
            .unwrap()
            .reference(b"AcroForm")
            .unwrap();
        let mut remaining = MAX_FIELD_TEXT - MAX_FIELDS * 120;
        let mut fields = Vec::new();
        let mut last_name = String::new();
        for index in 0..MAX_FIELDS {
            let mut field = template.clone();
            let name = format!("{index:04}{}", "n".repeat(116));
            field.set(b"T".to_vec(), PdfString::text(&name));
            let length = remaining.min(4096);
            if length != 0 {
                field.set(b"TU".to_vec(), PdfString::text(&"x".repeat(length)));
            }
            remaining -= length;
            fields.push(Object::Ref(native.add(Object::Dict(field))));
            last_name = name;
        }
        assert_eq!(remaining, 0);
        native
            .update_dict(acroform, |d| {
                d.set(b"Fields".to_vec(), Object::Array(fields.clone()))
            })
            .unwrap();
        native
            .update_dict(page, |d| d.set(b"Annots".to_vec(), Object::Array(fields)))
            .unwrap();
        assert_eq!(checked_owned_fields(native).unwrap().len(), MAX_FIELDS);
        let before = document.to_bytes().unwrap();
        let (items, input) = (document.items, document.input_bytes);
        let error = document.set_field_tooltip(&last_name, "x").unwrap_err();
        assert!(error
            .to_string()
            .contains("aggregate form text exceeds 4 MiB"));
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!((document.items, document.input_bytes), (items, input));
        document
            .set_field_tooltip(&format!("0000{}", "n".repeat(116)), "")
            .unwrap();
        document.set_field_tooltip(&last_name, "x").unwrap();
        assert_eq!(
            document.field_named(&last_name).unwrap().tooltip.as_deref(),
            Some("x")
        );
    }

    #[test]
    fn owned_fields_create_fill_and_flatten_with_complete_appearances() {
        let mut document = EditablePdf::new(300.0, 300.0).unwrap();
        document
            .add_field(0, "customer", "text", (10.0, 10.0, 200.0, 30.0), vec![])
            .unwrap();
        document
            .add_field(0, "approved", "checkbox", (10.0, 60.0, 30.0, 30.0), vec![])
            .unwrap();
        document
            .add_field(
                0,
                "country",
                "choice",
                (10.0, 110.0, 200.0, 30.0),
                vec!["Canada".into(), "France".into()],
            )
            .unwrap();
        let initial: Value =
            serde_json::from_str(&document.field_info("country").unwrap()).unwrap();
        assert_eq!(initial["value"], Value::Null);
        let initial_box: Vec<f64> =
            serde_json::from_value(initial["widgets"][0]["box"].clone()).unwrap();
        for (actual, expected) in initial_box.iter().zip([10.0, 110.0, 200.0, 30.0]) {
            assert!((actual - expected).abs() < 0.001);
        }
        document
            .set_field_value("customer", r#""Alice café""#)
            .unwrap();
        document.set_field_value("approved", "true").unwrap();
        document.set_field_value("country", r#""France""#).unwrap();
        let checked: Value =
            serde_json::from_str(&document.field_info("approved").unwrap()).unwrap();
        assert_eq!(checked["value"], true);
        let before = document.to_bytes().unwrap();
        let inspected = pdfcraft_render::inspect(Arc::new(before.clone()), None).unwrap();
        assert_eq!(inspected.fields.len(), 3);
        assert!(!inspected.has_javascript);
        assert!(EditablePdf::from_bytes(&before, None).is_err());
        let before_png = crate::pdf::render(&before, 0, 144.0, None).unwrap();
        assert_eq!(document.flatten_fields().unwrap(), 3);
        assert_eq!(document.fields(None).unwrap(), "[]");
        let after = document.to_bytes().unwrap();
        let after_png = crate::pdf::render(&after, 0, 144.0, None).unwrap();
        assert_eq!(before_png, after_png);
        assert!(pdfcraft_render::inspect(Arc::new(after.clone()), None)
            .unwrap()
            .fields
            .is_empty());
        let extracted = crate::pdf::extract(&after, None).unwrap();
        let text = extracted.pages[0]
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("Alice café") && text.contains("France"));
        assert_eq!(document.flatten_fields().unwrap(), 0);
    }

    #[test]
    fn field_validation_and_failed_changes_are_atomic() {
        let mut document = EditablePdf::new(300.0, 300.0).unwrap();
        document
            .add_field(0, "name", "text", (10.0, 10.0, 100.0, 30.0), vec![])
            .unwrap();
        document
            .add_field(
                0,
                "choice",
                "choice",
                (10.0, 50.0, 100.0, 30.0),
                vec!["A".into(), "B".into()],
            )
            .unwrap();
        document
            .add_field(0, "check", "checkbox", (10.0, 90.0, 30.0, 30.0), vec![])
            .unwrap();
        let before = document.to_bytes().unwrap();
        for (name, value) in [
            ("name", "true"),
            ("name", r#""Ω""#),
            ("name", r#""line\nnext""#),
            ("name", r#""This text is much too long for its appearance""#),
            ("choice", r#""Unknown""#),
            ("check", r#""yes""#),
            ("missing", r#""value""#),
        ] {
            assert!(
                document.set_field_value(name, value).is_err(),
                "{name}: {value}"
            );
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        for (name, kind, options) in [
            ("name", "text", vec![]),
            ("bad.name", "text", vec![]),
            (" padded ", "text", vec![]),
            ("radio", "radio", vec![]),
            ("empty", "choice", vec![]),
            ("duplicate", "choice", vec!["A".into(), "A".into()]),
            ("unicode", "choice", vec!["Ω".into()]),
        ] {
            assert!(document
                .add_field(0, name, kind, (150.0, 10.0, 100.0, 30.0), options)
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        assert!(document
            .add_field(0, "tiny", "text", (0.0, 0.0, 20.0, 5.0), vec![])
            .is_err());
        assert!(document.subset(vec![0]).is_err());
        document.set_field_value("choice", r#""A""#).unwrap();
        document.set_field_value("choice", "null").unwrap();
        document.set_field_value("check", "true").unwrap();
        document.set_field_value("check", "false").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&document.field_info("choice").unwrap()).unwrap()
                ["value"],
            Value::Null
        );
        assert_eq!(
            serde_json::from_str::<Value>(&document.field_info("check").unwrap()).unwrap()["value"],
            false
        );
        let blank = EditablePdf::new(100.0, 100.0).unwrap().to_bytes().unwrap();
        let mut imported = EditablePdf::from_bytes(&blank, None).unwrap();
        assert!(imported
            .add_field(0, "external", "text", (0.0, 0.0, 80.0, 30.0), vec![])
            .is_err());
    }

    #[test]
    fn owned_fields_follow_copy_page_moves_and_deletion() {
        let mut document = EditablePdf::new(200.0, 200.0).unwrap();
        document.add_page(200.0, 200.0).unwrap();
        document
            .add_field(0, "first", "text", (10.0, 10.0, 150.0, 30.0), vec![])
            .unwrap();
        document.set_field_value("first", r#""Original""#).unwrap();
        document
            .add_field(1, "second", "text", (10.0, 10.0, 150.0, 30.0), vec![])
            .unwrap();
        let mut copied = document.copy().unwrap();
        copied.set_field_value("first", r#""Changed""#).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&document.field_info("first").unwrap()).unwrap()["value"],
            "Original"
        );
        document.move_page(0, 1).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&document.field_info("first").unwrap()).unwrap()
                ["widgets"][0]["page"],
            1
        );
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&document.fields(Some(0)).unwrap()).unwrap()[0]
                ["name"],
            "second"
        );
        document.rotate_page(1, 90).unwrap();
        assert!(crate::pdf::render(&document.to_bytes().unwrap(), 1, 72.0, None).is_ok());
        document.delete_page(0).unwrap();
        assert!(document.field_info("second").is_err());
        assert_eq!(document.owned_fields().unwrap().len(), 1);
        document.delete_field("first").unwrap();
        assert!(document.fields(None).unwrap() == "[]");
        document
            .add_field(0, "first", "checkbox", (10.0, 10.0, 30.0, 30.0), vec![])
            .unwrap();
        assert_eq!(document.flatten_fields().unwrap(), 1);
    }

    #[test]
    fn field_geometry_preserves_identity_and_rejects_fit_and_budget_atomically() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(300.0, 400.0).unwrap();
        for (name, kind, options, value) in [
            ("text", "text", vec![], r#""Customer""#),
            ("check", "checkbox", vec![], "true"),
            (
                "choice",
                "choice",
                vec!["Canada".into(), "France".into()],
                r#""France""#,
            ),
        ] {
            document
                .add_field(0, name, kind, (20.0, 20.0, 180.0, 30.0), options)
                .unwrap();
            document.set_field_value(name, value).unwrap();
            let original = document.field_named(name).unwrap();
            let input_before = document.input_bytes;
            let items_before = document.items;
            document
                .set_field_box(name, (50.0, 100.0, 200.0, 40.0))
                .unwrap();
            let changed = document.field_named(name).unwrap();
            assert_eq!(
                document.input_bytes - input_before,
                owned_field_appearance_bytes(document.document().unwrap(), &changed).unwrap() + 32
            );
            assert_eq!(document.items, items_before + 1);
            assert_eq!(changed.obj, original.obj);
            assert_eq!(changed.widgets[0].obj, original.widgets[0].obj);
            assert_eq!(changed.value, original.value);
            assert_eq!(changed.options, original.options);
        }
        let original = document.to_bytes().unwrap();
        let items = document.items;
        let input_bytes = document.input_bytes;
        for rect in [
            (0.0, 0.0, 12.0, 30.0),
            (0.0, 0.0, 200.0, 4.0),
            (-1.0, 0.0, 200.0, 30.0),
            (0.0, 0.0, f64::NAN, 30.0),
        ] {
            assert!(document.set_field_box("text", rect).is_err());
            assert_eq!(document.to_bytes().unwrap(), original);
        }
        document.input_bytes = MAX_DOCUMENT_INPUT - 100;
        assert!(document
            .set_field_box("text", (20.0, 200.0, 200.0, 40.0))
            .unwrap_err()
            .to_string()
            .contains("resource_limit:PDF authoring item or input budget exceeded"));
        document.input_bytes = input_bytes;
        document.items = MAX_ITEMS;
        assert!(document
            .set_field_box("text", (20.0, 200.0, 200.0, 40.0))
            .is_err());
        document.items = items;
        assert_eq!(document.to_bytes().unwrap(), original);
    }

    #[test]
    fn field_geometry_after_page_rotation_keeps_widget_orientation() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditablePdf::new(300.0, 400.0).unwrap();
        document
            .add_field(0, "rotated", "text", (20.0, 20.0, 180.0, 30.0), vec![])
            .unwrap();
        document
            .set_field_value("rotated", r#""Customer""#)
            .unwrap();
        let original = document.field_named("rotated").unwrap();
        document.rotate_page(0, 90).unwrap();
        let before = document.to_bytes().unwrap();
        assert!(document
            .set_field_box("rotated", (100.0, 60.0, 180.0, 40.0))
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
        document
            .set_field_box("rotated", (100.0, 60.0, 40.0, 180.0))
            .unwrap();
        let current = document.field_named("rotated").unwrap();
        assert_eq!(current.widgets[0].rotation, original.widgets[0].rotation);
        assert_eq!(current.widgets[0].obj, original.widgets[0].obj);
        assert_eq!(current.value, original.value);
    }

    #[test]
    fn moved_field_geometry_uses_rotated_cropped_userunit_view() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |d| {
                d.set(
                    b"MediaBox".to_vec(),
                    Object::Array([10, 20, 310, 420].map(Object::Int).to_vec()),
                );
                d.set(
                    b"CropBox".to_vec(),
                    Object::Array([30, 40, 290, 400].map(Object::Int).to_vec()),
                );
                d.set(b"UserUnit".to_vec(), Object::Real(1.25));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::new(300.0, 400.0).unwrap();
            document.import_pages(&bytes, None, None, 1, None).unwrap();
            document.rotate_page(1, rotation).unwrap();
            document
                .add_field(1, "moved", "text", (20.0, 30.0, 180.0, 30.0), vec![])
                .unwrap();
            document.set_field_value("moved", r#""Moved""#).unwrap();
            document
                .set_field_box("moved", (80.0, 120.0, 200.0, 40.0))
                .unwrap();
            let info: Value = serde_json::from_str(&document.field_info("moved").unwrap()).unwrap();
            let actual: Vec<f64> =
                serde_json::from_value(info["widgets"][0]["box"].clone()).unwrap();
            for (a, b) in actual.iter().zip([80.0, 120.0, 200.0, 40.0]) {
                assert!((a - b).abs() < 0.001, "rotation {rotation}: {actual:?}");
            }
            document.flatten_fields().unwrap();
            let output = document.to_bytes().unwrap();
            let extracted = crate::pdf::extract(&output, None).unwrap();
            let text = extracted.pages[1]
                .blocks
                .iter()
                .find(|b| b.text.contains("Moved"))
                .unwrap();
            let bbox = text.bbox.unwrap();
            assert!(
                bbox[0] >= 80.0 && bbox[1] >= 120.0 && bbox[2] <= 280.0 && bbox[3] <= 160.0,
                "rotation {rotation}: {bbox:?}"
            );
            crate::pdf::render(&output, 1, 72.0, None).unwrap();
        }
    }

    #[test]
    fn fields_created_on_rotated_imported_pages_use_view_boxes_and_fixed_point_text() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(240.0, 320.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |d| {
                d.set(b"UserUnit".to_vec(), Object::Real(1.25));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::new(300.0, 400.0).unwrap();
            document.import_pages(&bytes, None, None, 1, None).unwrap();
            document.rotate_page(1, rotation).unwrap();
            document
                .add_field(1, "rotated", "text", (20.0, 30.0, 180.0, 30.0), vec![])
                .unwrap();
            document.set_field_value("rotated", r#""Upright""#).unwrap();
            let info: Value =
                serde_json::from_str(&document.field_info("rotated").unwrap()).unwrap();
            let actual: Vec<f64> =
                serde_json::from_value(info["widgets"][0]["box"].clone()).unwrap();
            for (a, b) in actual.iter().zip([20.0, 30.0, 180.0, 30.0]) {
                assert!((a - b).abs() < 0.001, "rotation {rotation}: {actual:?}");
            }
            let field = document.field_named("rotated").unwrap();
            assert!((pdfcraft_forms::appearance::parse_da(&field.da).size - 9.6).abs() < 0.001);
            let before = crate::pdf::render(&document.to_bytes().unwrap(), 1, 144.0, None).unwrap();
            assert_eq!(document.flatten_fields().unwrap(), 1);
            let output = document.to_bytes().unwrap();
            let after = crate::pdf::render(&output, 1, 144.0, None).unwrap();
            let extracted = crate::pdf::extract(&output, None).unwrap();
            let text = extracted.pages[1]
                .blocks
                .iter()
                .find(|block| block.text.contains("Upright"))
                .unwrap();
            let bbox = text.bbox.unwrap();
            assert!(bbox[0] >= 20.0 && bbox[1] >= 30.0 && bbox[2] <= 200.0 && bbox[3] <= 60.0);
            let decode = |bytes: Vec<u8>| {
                let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
                let mut output = vec![0; reader.output_buffer_size().unwrap()];
                reader.next_frame(&mut output).unwrap();
                output
            };
            let (before, after) = (decode(before), decode(after));
            let changed = before
                .chunks_exact(4)
                .zip(after.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            let largest = before
                .iter()
                .zip(&after)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                before
                    .chunks_exact(4)
                    .filter(|rgba| rgba[0] < 30 && rgba[1] < 30 && rgba[2] < 30)
                    .count()
                    > 20
            );
            // Observed one-pixel 1/255 variation at 180 degrees/UserUnit 1.25; likely the
            // native appearance/flatten placement rounding. Geometry and text stay exact.
            assert!(
                changed <= 4 && largest <= 1,
                "rotation {rotation}: {changed} changed pixels, maximum difference {largest}"
            );
        }
    }

    #[test]
    fn page_boxes_follow_view_geometry_and_reset_for_every_rotation() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |page| {
                page.set(
                    b"MediaBox".to_vec(),
                    Object::Array([10.0, 20.0, 310.0, 420.0].map(Object::Real).to_vec()),
                );
                page.set(
                    b"CropBox".to_vec(),
                    Object::Array([30.0, 40.0, 230.0, 340.0].map(Object::Real).to_vec()),
                );
                page.set(b"UserUnit".to_vec(), Object::Real(1.25));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::from_bytes(&bytes, None).unwrap();
            document.rotate_page(0, rotation).unwrap();
            let old_size = document.page_size(0).unwrap();
            let boxes: Value = serde_json::from_str(&document.page_boxes(0).unwrap()).unwrap();
            assert_eq!(boxes["crop"], json!([0.0, 0.0, old_size.0, old_size.1]));
            assert!(boxes["media"][0].as_f64().unwrap() < 0.0);
            assert!(boxes["media"][1].as_f64().unwrap() < 0.0);
            document
                .set_page_box(0, "trim", Some((10.0, 15.0, 100.0, 120.0)))
                .unwrap();
            let boxes: Value = serde_json::from_str(&document.page_boxes(0).unwrap()).unwrap();
            let actual: Vec<f64> = serde_json::from_value(boxes["trim"].clone()).unwrap();
            for (a, b) in actual.iter().zip([10.0, 15.0, 100.0, 120.0]) {
                assert!((a - b).abs() < 0.001, "rotation {rotation}: {actual:?}");
            }
            document.set_page_box(0, "trim", None).unwrap();
            document
                .set_page_box(0, "crop", Some((25.0, 25.0, 150.0, 200.0)))
                .unwrap();
            let size = document.page_size(0).unwrap();
            assert!((size.0 - 150.0).abs() < 0.001 && (size.1 - 200.0).abs() < 0.001);
            let output = document.to_bytes().unwrap();
            let reopened = EditablePdf::from_bytes(&output, None).unwrap();
            assert_eq!(reopened.page_size(0).unwrap(), size);
            let png = crate::pdf::render(&output, 0, 144.0, None).unwrap();
            let reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
            assert_eq!((reader.info().width, reader.info().height), (300, 400));
            document.set_page_box(0, "crop", None).unwrap();
            let expected = if rotation % 180 == 0 {
                (375.0, 500.0)
            } else {
                (500.0, 375.0)
            };
            assert_eq!(document.page_size(0).unwrap(), expected);
        }
    }

    #[test]
    fn page_box_failures_are_atomic_and_media_expansion_clears_background_claim() {
        let mut document = EditablePdf::new(200.0, 300.0).unwrap();
        document
            .set_page_box(0, "crop", Some((25.0, 25.0, 150.0, 250.0)))
            .unwrap();
        document
            .set_page_box(0, "trim", Some((10.0, 10.0, 30.0, 30.0)))
            .unwrap();
        let before = document.to_bytes().unwrap();
        for (kind, rect) in [
            ("media", None),
            ("crop", Some((-100.0, 0.0, 20.0, 20.0))),
            ("crop", Some((0.0, 0.0, -20.0, 20.0))),
            ("crop", Some((f64::NAN, 0.0, 20.0, 20.0))),
            ("other", None),
            ("media", Some((100.0, 100.0, 20.0, 20.0))),
        ] {
            assert!(document.set_page_box(0, kind, rect).is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        document.set_background(0, (255, 0, 0)).unwrap();
        document
            .set_page_box(0, "media", Some((-50.0, -50.0, 250.0, 350.0)))
            .unwrap();
        assert!(document.background(0).is_err());
    }

    #[test]
    fn resetting_inherited_crop_pins_media_default_without_changing_siblings() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(200.0, 300.0, 2).unwrap();
        let pages = pdfcraft_organize::pages(&source).unwrap();
        let parent = source
            .get(pages[0].obj)
            .as_dict()
            .unwrap()
            .reference(b"Parent")
            .unwrap();
        source
            .update_dict(parent, |d| {
                d.set(
                    b"CropBox".to_vec(),
                    Object::Array([25.0, 25.0, 175.0, 275.0].map(Object::Real).to_vec()),
                );
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        let mut document = EditablePdf::from_bytes(&bytes, None).unwrap();
        assert_eq!(document.page_size(0).unwrap(), (150.0, 250.0));
        document.set_page_box(0, "crop", None).unwrap();
        assert_eq!(document.page_size(0).unwrap(), (200.0, 300.0));
        assert_eq!(document.page_size(1).unwrap(), (150.0, 250.0));
    }

    #[test]
    fn builtin_and_supplied_font_colors_render_without_affecting_extraction() {
        let mut document = EditablePdf::new(240.0, 140.0).unwrap();
        document
            .add_text(0, "Red", 10.0, 10.0, 24.0, (255, 0, 0))
            .unwrap();
        document
            .add_text_with_font(
                0,
                "Blue Ω",
                10.0,
                60.0,
                24.0,
                pdfcraft_fonts::INTER_REGULAR,
                (0, 0, 255),
            )
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let extracted = crate::pdf::extract(&bytes, None).unwrap();
        assert!(extracted.pages[0]
            .blocks
            .iter()
            .any(|b| b.text.contains("Red")));
        assert!(extracted.pages[0]
            .blocks
            .iter()
            .any(|b| b.text.contains("Blue Ω")));
        let png = crate::pdf::render(&bytes, 0, 144.0, None).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert!(pixels
            .chunks_exact(4)
            .any(|rgba| rgba[0] > 200 && rgba[1] < 30 && rgba[2] < 30));
        assert!(pixels
            .chunks_exact(4)
            .any(|rgba| rgba[0] < 30 && rgba[1] < 30 && rgba[2] > 200));
    }

    #[test]
    fn imported_omitted_outline_is_rejected_but_complete_outline_remains_editable() {
        use pdfcraft_cos::{Dict, Object, PdfString};
        let mut source = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        pdfcraft_organize::add_bookmark(&mut source, &[], 0, "Normal", 0).unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        assert!(EditablePdf::from_bytes(&bytes, None).is_ok());
        // Deliberately hostile depth fixture: ordinary editing APIs cap the depth already.
        let root = source.root().unwrap();
        let outline = source
            .get(root)
            .as_dict()
            .unwrap()
            .reference(b"Outlines")
            .unwrap();
        let mut parent = outline;
        for index in 0..40 {
            let mut node = Dict::new();
            node.set(
                b"Title".to_vec(),
                Object::String(PdfString::text(&format!("Level {index}"))),
            );
            node.set(b"Parent".to_vec(), Object::Ref(parent));
            let child = source.add(Object::Dict(node));
            source
                .update_dict(parent, |d| {
                    d.set(b"First".to_vec(), Object::Ref(child));
                    d.set(b"Last".to_vec(), Object::Ref(child));
                })
                .unwrap();
            parent = child;
        }
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        assert!(
            pdfcraft_render::inspect(Arc::new(bytes.clone()), None)
                .unwrap()
                .outline_more
        );
        assert!(EditablePdf::from_bytes(&bytes, None).is_err());
    }

    #[test]
    fn metadata_updates_roundtrip_and_invalid_batch_is_atomic() {
        let mut document = EditablePdf::new(200.0, 300.0).unwrap();
        document.update_metadata(r#"{"title":" A title Ω ","author":"Author","subject":"Subject","keywords":"one two"}"#).unwrap();
        let metadata: Value = serde_json::from_str(&document.metadata().unwrap()).unwrap();
        assert_eq!(
            metadata,
            json!({"title":"A title Ω","author":"Author","subject":"Subject","keywords":"one two"})
        );
        let before = document.to_bytes().unwrap();
        for updates in [
            r#"{"title":"Changed","unknown":"bad"}"#,
            r#"{"author":42}"#,
            r#"{"subject":"bad\u0000text"}"#,
            "[]",
        ] {
            assert!(document.update_metadata(updates).is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        document
            .update_metadata(r#"{"title":null,"author":""}"#)
            .unwrap();
        let reopened = EditablePdf::from_bytes(&document.to_bytes().unwrap(), None).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reopened.metadata().unwrap()).unwrap(),
            json!({"subject":"Subject","keywords":"one two"})
        );
    }

    #[test]
    fn hierarchical_bookmarks_follow_pages_and_support_strict_atomic_crud() {
        let mut document = EditablePdf::new(200.0, 300.0).unwrap();
        document.add_page(200.0, 300.0).unwrap();
        assert_eq!(
            document.add_bookmark("First", 0, vec![], None).unwrap(),
            vec![0]
        );
        assert_eq!(
            document.add_bookmark("Child Ω", 1, vec![0], None).unwrap(),
            vec![0, 0]
        );
        assert_eq!(
            document.add_bookmark("Second", 1, vec![], None).unwrap(),
            vec![1]
        );
        document
            .update_bookmark(vec![0], Some("Renamed"), Some(1), Some(false))
            .unwrap();
        let listed: Vec<Value> = serde_json::from_str(&document.bookmarks().unwrap()).unwrap();
        assert_eq!(
            listed[0],
            json!({"path":[0],"title":"Renamed","page":1,"open":false})
        );
        assert_eq!(
            document.move_bookmark(vec![0, 0], vec![1], 0).unwrap(),
            vec![1, 0]
        );
        document.move_page(1, 0).unwrap();
        let reopened = EditablePdf::from_bytes(&document.to_bytes().unwrap(), None).unwrap();
        let listed: Vec<Value> = serde_json::from_str(&reopened.bookmarks().unwrap()).unwrap();
        assert!(listed.iter().all(|entry| entry["page"] == 0));
        assert_eq!(listed[2]["title"], "Child Ω");
        let before = document.to_bytes().unwrap();
        assert!(document
            .add_bookmark("Invalid", 0, vec![], Some(9))
            .is_err());
        assert!(document.add_bookmark("Invalid", 8, vec![], None).is_err());
        assert!(document.add_bookmark("Invalid", 0, vec![9], None).is_err());
        assert!(document.move_bookmark(vec![1], vec![1, 0], 0).is_err());
        assert!(document.move_bookmark(vec![0], vec![], 9).is_err());
        assert!(document
            .update_bookmark(vec![0], Some("Must not commit"), Some(9), None)
            .is_err());
        assert!(document.delete_bookmark(vec![]).is_err());
        assert_eq!(before, document.to_bytes().unwrap());
        document.delete_bookmark(vec![1]).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<Value>>(&document.bookmarks().unwrap())
                .unwrap()
                .len(),
            1
        );
        document.delete_bookmark(vec![0]).unwrap();
        assert_eq!(document.bookmarks().unwrap(), "[]");
    }

    #[test]
    fn vector_paths_render_in_display_coordinates_and_remain_vector_content() {
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::new(150.0, 200.0).unwrap();
            document.rotate_page(0, rotation).unwrap();
            document
                .draw_path(
                    0,
                    r#"[["move",20,30],["line",60,30],["line",60,70],["line",20,70]]"#,
                    None,
                    Some((0, 255, 0)),
                    1.0,
                    true,
                )
                .unwrap();
            document
                .draw_path(
                    0,
                    r#"[["move",70,30],["curve",80,10,100,10,110,30]]"#,
                    Some((255, 0, 0)),
                    None,
                    3.0,
                    false,
                )
                .unwrap();
            let bytes = document.to_bytes().unwrap();
            let extracted = crate::pdf::extract(&bytes, None).unwrap();
            assert!(extracted.pages[0].images.is_empty());
            let raster = crate::pdf::render(&bytes, 0, 72.0, None).unwrap();
            let mut reader = png::Decoder::new(Cursor::new(raster)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            let offset = (50 * info.width as usize + 40) * 4;
            assert_eq!(
                &pixels[offset..offset + 4],
                &[0, 255, 0, 255],
                "rotation {rotation}"
            );
            assert!(pixels.chunks_exact(4).any(|rgba| rgba == [255, 0, 0, 255]));
        }
    }

    #[test]
    fn vector_path_validation_rejects_invalid_input_without_mutation() {
        let mut document = EditablePdf::new(100.0, 100.0).unwrap();
        let before = document.to_bytes().unwrap();
        for commands in [
            "[]",
            r#"[["line",10,10]]"#,
            r#"[["move",10,10]]"#,
            r#"[["move",10,10],["curve",20,20]]"#,
            r#"[["move",10,10],["line",100,20]]"#,
            r#"[["move",10,10],["execute",20,20]]"#,
        ] {
            assert!(document
                .draw_path(0, commands, Some((0, 0, 0)), None, 1.0, false)
                .is_err());
        }
        assert!(document
            .draw_path(
                0,
                r#"[["move",10,10],["line",20,20]]"#,
                None,
                None,
                1.0,
                false
            )
            .is_err());
        assert_eq!(before, document.to_bytes().unwrap());
    }

    #[test]
    fn supplied_open_font_embeds_unicode_shaping_and_explicit_rows() {
        let mut document = EditablePdf::new(300.0, 250.0).unwrap();
        let text = "Aé ΩЖ €\nCombining: e\u{301} office";
        document
            .add_text_with_font(
                0,
                text,
                20.0,
                20.0,
                16.0,
                pdfcraft_fonts::INTER_REGULAR,
                (0, 0, 0),
            )
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let extracted = crate::pdf::extract(&bytes, None).unwrap();
        let result = extracted.pages[0]
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(result.contains("Aé ΩЖ €"), "{result}");
        assert!(result.contains("Combining: e\u{301} office"), "{result}");
        let fonts =
            pdfcraft_edit::text::text_editor_fonts(document.document().unwrap(), 0).unwrap();
        assert!(fonts
            .iter()
            .any(|font| font.bytes == pdfcraft_fonts::INTER_REGULAR));
        let raster = crate::pdf::render(&bytes, 0, 144.0, None).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(raster)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert!(pixels.chunks_exact(4).any(|rgba| rgba[..3] == [0, 0, 0]));
    }

    #[test]
    fn font_failures_and_overflow_are_atomic_without_fallback() {
        let mut document = EditablePdf::new(100.0, 100.0).unwrap();
        let before = document.to_bytes().unwrap();
        for (text, x, y, size, font) in [
            ("Broken", 10.0, 10.0, 12.0, b"broken".as_slice()),
            (
                "\u{10ffff}",
                10.0,
                10.0,
                12.0,
                pdfcraft_fonts::INTER_REGULAR,
            ),
            (
                "Too long for this page",
                80.0,
                10.0,
                12.0,
                pdfcraft_fonts::INTER_REGULAR,
            ),
            ("Too low", 10.0, 99.0, 12.0, pdfcraft_fonts::INTER_REGULAR),
            (
                "Invalid\0text",
                10.0,
                10.0,
                12.0,
                pdfcraft_fonts::INTER_REGULAR,
            ),
        ] {
            assert!(document
                .add_text_with_font(0, text, x, y, size, font, (0, 0, 0))
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
        }
    }

    #[test]
    fn font_and_vector_content_follow_crop_rotation_and_user_unit() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |page| {
                page.set(
                    b"CropBox".to_vec(),
                    Object::Array([20.0, 30.0, 220.0, 330.0].map(Object::Real).to_vec()),
                );
                page.set(b"UserUnit".to_vec(), Object::Real(1.25));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::from_bytes(&bytes, None).unwrap();
            document.rotate_page(0, rotation).unwrap();
            document
                .add_text_with_font(
                    0,
                    "ΩЖ",
                    20.0,
                    20.0,
                    16.0,
                    pdfcraft_fonts::INTER_REGULAR,
                    (0, 0, 0),
                )
                .unwrap();
            document
                .draw_path(
                    0,
                    r#"[["move",20,60],["line",60,60],["line",60,100],["line",20,100]]"#,
                    None,
                    Some((0, 255, 0)),
                    1.0,
                    true,
                )
                .unwrap();
            let output = document.to_bytes().unwrap();
            let extracted = crate::pdf::extract(&output, None).unwrap();
            let block = extracted.pages[0]
                .blocks
                .iter()
                .find(|block| block.text.contains("ΩЖ"))
                .unwrap();
            let bbox = block.bbox.unwrap();
            assert!(
                (bbox[0] - 20.0).abs() < 1.0,
                "rotation {rotation}: {bbox:?}"
            );
            assert!(
                bbox[1] >= 19.0 && bbox[3] < 50.0,
                "rotation {rotation}: {bbox:?}"
            );
            let raster = crate::pdf::render(&output, 0, 72.0, None).unwrap();
            let mut reader = png::Decoder::new(Cursor::new(raster)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            let offset = (80 * info.width as usize + 40) * 4;
            assert_eq!(
                &pixels[offset..offset + 4],
                &[0, 255, 0, 255],
                "rotation {rotation}"
            );
        }
    }

    #[test]
    fn annotation_lock_preserves_dictionary_appearance_and_native_edit_rules() {
        pyo3::prepare_freethreaded_python();
        use pdfcraft_cos::Object;
        let raw_appearance = |document: &Document, index| {
            with_annotation_dict(document, 0, index, |dict| {
                let normal = dict
                    .get(b"AP")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .reference(b"N")
                    .unwrap();
                let object = document.get(normal);
                let Object::Stream(stream) = object.as_ref() else {
                    panic!("owned appearance stream")
                };
                Ok(stream.raw.clone())
            })
            .unwrap()
        };
        for kind in [
            "note",
            "highlight",
            "underline",
            "strikeout",
            "squiggly",
            "rectangle",
            "ellipse",
        ] {
            let mut document = EditablePdf::new(240.0, 240.0).unwrap();
            let index = document
                .add_annotation(
                    0,
                    kind,
                    if kind == "note" {
                        (20.0, 30.0, 40.0, 50.0)
                    } else {
                        (20.0, 30.0, 120.0, 50.0)
                    },
                    "Original",
                    "Author",
                    (255, 0, 0),
                )
                .unwrap();
            let (reference, _) =
                checked_lock_target(document.document().unwrap(), 0, index).unwrap();
            let original = document
                .document()
                .unwrap()
                .get(reference)
                .as_dict()
                .unwrap()
                .clone();
            let appearance =
                annotation_appearance_bytes(document.document().unwrap(), 0, index).unwrap();
            let original_ap = raw_appearance(document.document().unwrap(), index);
            let bytes = document.to_bytes().unwrap();
            let png = crate::pdf::render(&bytes, 0, 144.0, None).unwrap();
            document.set_annotation_locked(0, index, true).unwrap();
            let mut expected = original.clone();
            expected.set(
                b"F".to_vec(),
                Object::Int(if kind == "note" { 156 } else { 132 }),
            );
            assert_eq!(
                document.document().unwrap().get(reference).as_dict(),
                Some(&expected),
                "{kind}"
            );
            assert_eq!(
                annotation_appearance_bytes(document.document().unwrap(), 0, index).unwrap(),
                appearance
            );
            assert_eq!(
                raw_appearance(document.document().unwrap(), index),
                original_ap
            );
            let locked = document.to_bytes().unwrap();
            assert_eq!(
                crate::pdf::render(&locked, 0, 144.0, None).unwrap(),
                png,
                "{kind}"
            );
            let mut reopened = EditablePdf::from_bytes(&locked, None).unwrap();
            assert!(pdfcraft_annot::summaries(reopened.document().unwrap())[0].locked);
            reopened.set_annotation_locked(0, index, false).unwrap();
            assert!(!pdfcraft_annot::summaries(reopened.document().unwrap())[0].locked);
            for (rect, color, author) in [
                (
                    Some(if kind == "note" {
                        (30.0, 40.0, 50.0, 60.0)
                    } else {
                        (30.0, 40.0, 130.0, 60.0)
                    }),
                    None,
                    None,
                ),
                (None, Some((0, 0, 255)), None),
                (None, None, Some("Changed")),
            ] {
                assert!(document
                    .update_annotation(
                        0,
                        index,
                        Some("Must roll back"),
                        rect,
                        color,
                        None,
                        None,
                        author
                    )
                    .unwrap_err()
                    .to_string()
                    .contains("locked"));
                assert_eq!(document.to_bytes().unwrap(), locked);
            }
            assert!(document
                .delete_annotation(0, index)
                .unwrap_err()
                .to_string()
                .contains("locked"));
            assert_eq!(document.to_bytes().unwrap(), locked);
            document
                .update_annotation(
                    0,
                    index,
                    Some("Editable while locked"),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            assert!(pdfcraft_annot::summaries(document.document().unwrap())[0].locked);
            assert_eq!(
                crate::pdf::render(&document.to_bytes().unwrap(), 0, 144.0, None).unwrap(),
                png
            );
            document.set_annotation_locked(0, index, false).unwrap();
            document
                .update_annotation(
                    0,
                    index,
                    None,
                    None,
                    Some((0, 0, 255)),
                    None,
                    None,
                    Some("Changed"),
                )
                .unwrap();
            document.delete_annotation(0, index).unwrap();
            assert!(pdfcraft_annot::summaries(document.document().unwrap()).is_empty());
            let page = document
                .document()
                .unwrap()
                .get(pdfcraft_organize::pages(document.document().unwrap()).unwrap()[0].obj);
            assert!(page.as_dict().unwrap().get(b"Annots").is_none()); // note popup removed too
        }
    }

    #[test]
    fn annotation_lock_rejects_foreign_profiles_and_budget_failures_atomically() {
        pyo3::prepare_freethreaded_python();
        use pdfcraft_cos::{Dict, Object, PdfString};
        let mut document = EditablePdf::new(240.0, 240.0).unwrap();
        let index = document
            .add_annotation(
                0,
                "highlight",
                (20.0, 30.0, 120.0, 50.0),
                "Original",
                "Author",
                (255, 0, 0),
            )
            .unwrap();
        let (reference, _) = checked_lock_target(document.document().unwrap(), 0, index).unwrap();
        for (key, value) in [
            (b"F".to_vec(), Object::Real(4.0)),
            (b"F".to_vec(), Object::Int(0)),
            (b"IRT".to_vec(), Object::Ref(reference)),
            (
                b"Foreign".to_vec(),
                Object::Array(vec![Object::Array(vec![Object::Int(1)])]),
            ),
            (b"AP".to_vec(), Object::Dict(Dict::new())),
            (b"C".to_vec(), Object::Array(vec![Object::Real(1.0); 4])),
            (b"Rect".to_vec(), Object::Array(vec![Object::Real(1.0); 4])),
            (
                b"Contents".to_vec(),
                Object::String(PdfString::literal(vec![b'x'; 131_075])),
            ),
            (
                b"QuadPoints".to_vec(),
                Object::Array(vec![Object::Real(0.0); 8193]),
            ),
        ] {
            let mut bad = document.copy().unwrap();
            bad.document
                .as_mut()
                .unwrap()
                .update_dict(reference, |d| d.set(key, value))
                .unwrap();
            let before = bad.to_bytes().unwrap();
            let (items, input) = (bad.items, bad.input_bytes);
            assert!(bad.set_annotation_locked(0, index, true).is_err());
            assert_eq!(bad.to_bytes().unwrap(), before);
            assert_eq!((bad.items, bad.input_bytes), (items, input));
        }
        for (items, input) in [
            (MAX_ITEMS, document.input_bytes),
            (document.items, MAX_DOCUMENT_INPUT - 1),
        ] {
            let mut bad = document.copy().unwrap();
            bad.items = items;
            bad.input_bytes = input;
            let before = bad.to_bytes().unwrap();
            assert!(bad
                .set_annotation_locked(0, index, true)
                .unwrap_err()
                .to_string()
                .contains("resource_limit"));
            assert_eq!(bad.to_bytes().unwrap(), before);
            assert_eq!((bad.items, bad.input_bytes), (items, input));
        }
        let mut link = EditablePdf::new(240.0, 240.0).unwrap();
        let index = link
            .add_link(
                0,
                (20.0, 20.0, 80.0, 40.0),
                Some("https://example.com"),
                None,
            )
            .unwrap();
        let before = link.to_bytes().unwrap();
        assert!(link
            .set_annotation_locked(0, index, true)
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
        assert_eq!(link.to_bytes().unwrap(), before);
    }

    #[test]
    fn annotation_lock_bounds_references_metadata_and_quad_arrays_before_copy() {
        pyo3::prepare_freethreaded_python();
        use pdfcraft_cos::{Dict, Object, PdfString};
        let mut document = EditablePdf::new(240.0, 240.0).unwrap();
        let index = document
            .add_annotation(
                0,
                "highlight",
                (20.0, 30.0, 120.0, 50.0),
                "Original",
                "",
                (255, 0, 0),
            )
            .unwrap();
        let (reference, _) = checked_lock_target(document.document().unwrap(), 0, index).unwrap();
        let page = pdfcraft_organize::pages(document.document().unwrap()).unwrap()[0].obj;
        let native = document.document.as_mut().unwrap();
        native
            .update_dict(reference, |d| {
                d.set(
                    b"Contents".to_vec(),
                    Object::String(PdfString::literal(vec![b'x'; 131_074])),
                );
                d.set(
                    b"T".to_vec(),
                    Object::String(PdfString::literal(vec![b'a'; 8194])),
                );
                let quad = [20.0, 210.0, 120.0, 210.0, 20.0, 190.0, 120.0, 190.0];
                d.set(
                    b"QuadPoints".to_vec(),
                    Object::Array((0..1024).flat_map(|_| quad.map(Object::Real)).collect()),
                );
            })
            .unwrap();
        native
            .update_dict(page, |d| {
                d.set(
                    b"Annots".to_vec(),
                    Object::Array(vec![Object::Ref(reference); MAX_ITEMS]),
                )
            })
            .unwrap();
        assert!(checked_lock_target(document.document().unwrap(), 0, index).is_ok());
        document.set_annotation_locked(0, index, true).unwrap();
        let mut too_many = document.copy().unwrap();
        too_many
            .document
            .as_mut()
            .unwrap()
            .update_dict(page, |d| {
                d.set(
                    b"Annots".to_vec(),
                    Object::Array(vec![Object::Ref(reference); MAX_ITEMS + 1]),
                )
            })
            .unwrap();
        assert!(checked_lock_target(too_many.document().unwrap(), 0, index)
            .unwrap_err()
            .to_string()
            .contains("resource_limit"));
        let mut inline = document.copy().unwrap();
        inline
            .document
            .as_mut()
            .unwrap()
            .update_dict(page, |d| {
                d.set(
                    b"Annots".to_vec(),
                    Object::Array(vec![Object::Ref(reference), Object::Dict(Dict::new())]),
                )
            })
            .unwrap();
        let before = inline.to_bytes().unwrap();
        assert!(inline
            .set_annotation_locked(0, index, false)
            .unwrap_err()
            .to_string()
            .contains("unsupported"));
        assert_eq!(inline.to_bytes().unwrap(), before);
        let mut oversized = document.copy().unwrap();
        oversized
            .document
            .as_mut()
            .unwrap()
            .update_dict(reference, |d| {
                for i in 0..21 {
                    d.set(format!("Extra{i}").into_bytes(), Object::Int(0));
                }
            })
            .unwrap();
        assert!(checked_lock_target(oversized.document().unwrap(), 0, index)
            .unwrap_err()
            .to_string()
            .contains("resource_limit"));
    }

    #[test]
    fn annotations_roundtrip_update_and_delete_through_public_backend() {
        let mut document = EditablePdf::new(300.0, 400.0).unwrap();
        document.add_page(300.0, 400.0).unwrap();
        let link = document
            .add_link(
                0,
                (10.0, 10.0, 80.0, 25.0),
                Some("https://example.com/café"),
                None,
            )
            .unwrap();
        let note = document
            .add_annotation(
                0,
                "note",
                (20.0, 40.0, 40.0, 60.0),
                "Unicode café ✓",
                "Author",
                (255, 210, 0),
            )
            .unwrap();
        let highlight = document
            .add_annotation(
                0,
                "highlight",
                (50.0, 80.0, 150.0, 100.0),
                "Highlight",
                "",
                (255, 255, 0),
            )
            .unwrap();
        document
            .update_annotation(
                0,
                note,
                Some("Edited ✓"),
                Some((30.0, 50.0, 50.0, 70.0)),
                Some((0, 255, 0)),
                None,
                None,
                Some("Reviewer"),
            )
            .unwrap();
        document
            .update_annotation(
                0,
                link,
                None,
                Some((10.0, 15.0, 90.0, 30.0)),
                Some((255, 0, 0)),
                None,
                Some(1),
                None,
            )
            .unwrap();
        document
            .update_annotation(
                0,
                highlight,
                None,
                Some((60.0, 90.0, 160.0, 110.0)),
                Some((0, 100, 255)),
                None,
                None,
                None,
            )
            .unwrap();
        let values: Vec<Value> = serde_json::from_str(&document.annotations(0).unwrap()).unwrap();
        assert_eq!(values.len(), 3);
        assert_eq!(values[0]["kind"], "link");
        assert_eq!(values[0]["target_page"], 1);
        assert_eq!(values[0]["uri"], Value::Null);
        assert_eq!(values[1]["text"], "Edited ✓");
        assert_eq!(values[1]["author"], "Reviewer");
        assert_eq!(values[1]["color"], json!([0, 255, 0]));
        assert_eq!(values[2]["kind"], "highlight");
        let reopened = EditablePdf::from_bytes(&document.to_bytes().unwrap(), None).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reopened.annotations(0).unwrap()).unwrap(),
            json!(values)
        );
        let extracted = crate::pdf::extract(&document.to_bytes().unwrap(), None).unwrap();
        assert_eq!(extracted.pages[0].links[0].uri, "#page=2");
        document.delete_annotation(0, note).unwrap();
        let after: Vec<Value> = serde_json::from_str(&document.annotations(0).unwrap()).unwrap();
        assert_eq!(after.len(), 2);
        assert!(after.iter().all(|value| value["kind"] != "note"));
        let remaining = after
            .iter()
            .find(|value| value["kind"] == "highlight")
            .unwrap()["index"]
            .as_u64()
            .unwrap() as usize;
        document.delete_annotation(0, remaining).unwrap();
        document.delete_annotation(0, link).unwrap();
        assert_eq!(document.annotations(0).unwrap(), "[]");
    }

    #[test]
    fn geometric_annotations_resize_roundtrip_and_render_in_all_page_views() {
        use pdfcraft_cos::Object;
        pyo3::prepare_freethreaded_python();
        let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |dict| {
                dict.set(
                    b"CropBox".to_vec(),
                    Object::Array([20.0, 30.0, 220.0, 330.0].map(Object::Real).to_vec()),
                );
                dict.set(b"UserUnit".to_vec(), Object::Real(2.0));
            })
            .unwrap();
        let bytes = resource_fixture_bytes(&source);
        for rotation in [0, 90, 180, 270] {
            for kind in ["rectangle", "ellipse"] {
                let mut document = EditablePdf::from_bytes(&bytes, None).unwrap();
                document.rotate_page(0, rotation).unwrap();
                document
                    .add_text(0, "Preserved body", 20.0, 190.0, 12.0, (0, 0, 0))
                    .unwrap();
                let base = pdf2_raster(&document);
                let index = document
                    .add_annotation(
                        0,
                        kind,
                        (20.0, 20.0, 140.0, 80.0),
                        "Comment café",
                        "Author",
                        (255, 0, 0),
                    )
                    .unwrap();
                let raw_size =
                    annotation_appearance_bytes(document.document().unwrap(), 0, index).unwrap();
                assert!(raw_size > 0 && raw_size < 512);
                assert_eq!(
                    with_annotation_dict(document.document().unwrap(), 0, index, |dict| {
                        geometric_dict_peak(document.document().unwrap(), dict)
                    })
                    .unwrap(),
                    GEOMETRIC_APPEARANCE_WORK
                );
                let mut resized_only = document.copy().unwrap();
                resized_only
                    .update_annotation(
                        0,
                        index,
                        None,
                        Some((40.0, 100.0, 200.0, 160.0)),
                        None,
                        None,
                        None,
                        None,
                    )
                    .unwrap();
                let resized_cost = resized_only.input_bytes - document.input_bytes;
                let input_before = document.input_bytes;
                document
                    .update_annotation(
                        0,
                        index,
                        Some("Revised ✓"),
                        Some((40.0, 100.0, 200.0, 160.0)),
                        Some((0, 0, 255)),
                        None,
                        None,
                        Some("Reviewer"),
                    )
                    .unwrap();
                assert_eq!(
                    document.input_bytes,
                    input_before
                        + resized_cost
                        + annotation_appearance_bytes(document.document().unwrap(), 0, index)
                            .unwrap()
                        + "Revised ✓".len()
                        + "Reviewer".len()
                );
                let bytes = document.to_bytes().unwrap();
                let mut reopened = EditablePdf::from_bytes(&bytes, None).unwrap();
                let info: Value = serde_json::from_str(&reopened.annotations(0).unwrap()).unwrap();
                assert_eq!(info[0]["kind"], kind);
                assert_eq!(info[0]["text"], "Revised ✓");
                assert_eq!(info[0]["author"], "Reviewer");
                for (actual, expected) in info[0]["rect"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip([40.0, 100.0, 200.0, 160.0])
                {
                    assert!((actual.as_f64().unwrap() - expected).abs() < 0.001);
                }
                let (width, height, pixels) = pdf2_raster(&reopened);
                let ink = (0..height)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let p = &pixels[(y * width + x) * 4..][..4];
                        p[2] > p[0].saturating_add(30)
                    })
                    .collect::<Vec<_>>();
                assert!(ink.len() > 200, "{kind}, rotation {rotation}");
                assert!(ink
                    .iter()
                    .all(|&(x, y)| (40..200).contains(&x) && (100..160).contains(&y)));
                let center = &pixels[(130 * width + 120) * 4..][..4];
                assert_eq!(center, &[255, 255, 255, 255]);
                for y in 180..210 {
                    assert_eq!(
                        &pixels[y * width * 4..(y + 1) * width * 4],
                        &base.2[y * width * 4..(y + 1) * width * 4]
                    );
                }
                let text = crate::pdf::extract(&bytes, None).unwrap().pages[0]
                    .blocks
                    .iter()
                    .map(|b| b.text.clone())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("Preserved body"));
                reopened.delete_annotation(0, index).unwrap();
                assert_eq!(pdf2_raster(&reopened), base);
            }
        }
    }

    #[test]
    fn geometric_native_tiny_oval_collapse_and_supported_minimum() {
        pyo3::prepare_freethreaded_python();
        let mut source = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        let shape = pdfcraft_annot::Shape::Oval {
            rect: [20.0, 77.99, 22.01, 80.0],
        };
        let mut style = pdfcraft_annot::Style::default_for(&shape);
        style.color = [1.0, 0.0, 0.0];
        pdfcraft_annot::add_annotation(
            &mut source,
            &pdfcraft_annot::NewAnnotation {
                page: 0,
                shape,
                style,
                contents: String::new(),
                author: String::new(),
            },
            &pdfcraft_annot::Meta::default(),
        )
        .unwrap();
        let bytes = resource_fixture_bytes(&source);
        let mut imported = EditablePdf::from_bytes(&bytes, None).unwrap();
        let (_, _, pixels) = pdf2_raster(&imported);
        assert!(
            !pixels
                .chunks_exact(4)
                .any(|p| p[0] > p[1].saturating_add(30)),
            "native tiny-oval regression changed"
        );
        let before = imported.to_bytes().unwrap();
        assert!(imported
            .update_annotation(
                0,
                0,
                Some("Do not commit"),
                None,
                Some((0, 0, 255)),
                None,
                None,
                None
            )
            .is_err());
        assert_eq!(imported.to_bytes().unwrap(), before);
        for kind in ["rectangle", "ellipse"] {
            let mut document = EditablePdf::new(100.0, 100.0).unwrap();
            assert!(document
                .add_annotation(0, kind, (20.0, 20.0, 22.01, 22.01), "", "", (255, 0, 0))
                .is_err());
            document
                .add_annotation(0, kind, (20.0, 20.0, 23.0, 23.0), "", "", (255, 0, 0))
                .unwrap();
            let (_, _, pixels) = pdf2_raster(&document);
            assert!(pixels
                .chunks_exact(4)
                .any(|p| p[0] > p[1].saturating_add(30)));
        }
    }

    #[test]
    fn geometric_annotation_profiles_and_budgets_reject_before_rebuild() {
        use pdfcraft_cos::{Dict, Object};
        pyo3::prepare_freethreaded_python();
        for kind in ["rectangle", "ellipse"] {
            let mut document = EditablePdf::new(220.0, 220.0).unwrap();
            let before = document.to_bytes().unwrap();
            document.input_bytes = MAX_DOCUMENT_INPUT - GEOMETRIC_APPEARANCE_WORK + 1;
            assert!(document
                .add_annotation(0, kind, (20.0, 20.0, 140.0, 80.0), "", "", (255, 0, 0))
                .unwrap_err()
                .to_string()
                .contains("resource_limit:"));
            assert_eq!(document.to_bytes().unwrap(), before);
            document.input_bytes = 0;
            document.items = MAX_ITEMS;
            assert!(document
                .add_annotation(0, kind, (20.0, 20.0, 140.0, 80.0), "", "", (255, 0, 0))
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
            document.items = 0;
            let index = document
                .add_annotation(0, kind, (20.0, 20.0, 140.0, 80.0), "", "", (255, 0, 0))
                .unwrap();
            let initial =
                annotation_appearance_bytes(document.document().unwrap(), 0, index).unwrap();
            assert_eq!(document.input_bytes, initial);
            document
                .update_annotation(
                    0,
                    index,
                    None,
                    Some((30.0, 30.0, 150.0, 100.0)),
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            assert_eq!(
                document.input_bytes,
                initial
                    + annotation_appearance_bytes(document.document().unwrap(), 0, index).unwrap()
            );
            let before = document.to_bytes().unwrap();
            for rect in [
                (20.0, 20.0, 22.0, 80.0),
                (20.0, 20.0, 140.0, 21.0),
                (20.0, 20.0, f64::INFINITY, 80.0),
            ] {
                assert!(document
                    .update_annotation(
                        0,
                        index,
                        Some("Do not commit"),
                        Some(rect),
                        None,
                        None,
                        None,
                        None
                    )
                    .is_err());
                assert_eq!(document.to_bytes().unwrap(), before);
            }
            let canonical = with_annotation_dict(document.document().unwrap(), 0, index, |dict| {
                Ok(dict.clone())
            })
            .unwrap();
            for (key, value) in [
                (b"C".to_vec(), Object::Array(vec![Object::Int(0); 100_000])),
                (
                    b"Rect".to_vec(),
                    Object::Array(vec![Object::Int(0); 100_000]),
                ),
                (b"RD".to_vec(), Object::Array(vec![Object::Int(0); 4])),
                (b"IC".to_vec(), Object::Array(vec![Object::Int(1); 3])),
                (b"CA".to_vec(), Object::Real(0.5)),
                (b"Measure".to_vec(), Object::Dict(Dict::new())),
                (b"PCPicture".to_vec(), Object::Bool(true)),
                (b"AP".to_vec(), Object::Dict(Dict::new())),
                (
                    b"Border".to_vec(),
                    Object::Array(vec![Object::Int(0); 100_000]),
                ),
            ] {
                let mut bad = canonical.clone();
                bad.set(key.clone(), value.clone());
                assert!(
                    geometric_dict_peak(document.document().unwrap(), &bad).is_err(),
                    "{key:?}"
                );
                let mut staged = document.copy().unwrap();
                let native = staged.document.as_mut().unwrap();
                let page = native.get(pdfcraft_organize::pages(native).unwrap()[0].obj);
                let reference = page
                    .as_dict()
                    .unwrap()
                    .get(b"Annots")
                    .unwrap()
                    .as_array()
                    .unwrap()[index]
                    .as_ref()
                    .unwrap();
                native
                    .update_dict(reference, |dict| {
                        dict.set(key, value);
                    })
                    .unwrap();
                let before = staged.to_bytes().unwrap();
                assert!(staged
                    .update_annotation(
                        0,
                        index,
                        Some("Do not commit"),
                        None,
                        Some((0, 0, 255)),
                        None,
                        None,
                        None
                    )
                    .is_err());
                assert_eq!(staged.to_bytes().unwrap(), before);
                staged.delete_annotation(0, index).unwrap();
            }
            let mut bad = canonical.clone();
            let mut border = Dict::new();
            border.set(b"W".to_vec(), Object::Real(2.0));
            border.set(b"S".to_vec(), Object::name("D"));
            border.set(b"D".to_vec(), Object::Array(vec![Object::Int(1); 100_000]));
            bad.set(b"BS".to_vec(), Object::Dict(border));
            assert!(geometric_dict_peak(document.document().unwrap(), &bad).is_err());
            document.input_bytes = MAX_DOCUMENT_INPUT - 1;
            let before = document.to_bytes().unwrap();
            assert!(document
                .update_annotation(
                    0,
                    index,
                    Some("Do not commit"),
                    None,
                    None,
                    None,
                    None,
                    None
                )
                .unwrap_err()
                .to_string()
                .contains("resource_limit:"));
            assert_eq!(document.to_bytes().unwrap(), before);
            document.delete_annotation(0, index).unwrap();
        }
    }

    #[test]
    fn markup_annotations_roundtrip_move_recolor_and_delete_without_changing_text() {
        pyo3::prepare_freethreaded_python();
        for kind in ["underline", "strikeout", "squiggly"] {
            let mut document = EditablePdf::new(220.0, 220.0).unwrap();
            document
                .add_text(0, "Preserved body", 20.0, 160.0, 12.0, (0, 0, 0))
                .unwrap();
            let plain = crate::pdf::extract(&document.to_bytes().unwrap(), None)
                .unwrap()
                .pages[0]
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let index = document
                .add_annotation(
                    0,
                    kind,
                    (20.0, 20.0, 140.0, 40.0),
                    "Comment café",
                    "Author",
                    (255, 0, 0),
                )
                .unwrap();
            document
                .update_annotation(
                    0,
                    index,
                    Some("Revised ✓"),
                    Some((30.0, 50.0, 150.0, 70.0)),
                    Some((0, 0, 255)),
                    None,
                    None,
                    Some("Reviewer"),
                )
                .unwrap();
            let snapshot: Value = serde_json::from_str(&document.annotations(0).unwrap()).unwrap();
            assert_eq!(snapshot[0]["kind"], kind);
            assert_eq!(snapshot[0]["text"], "Revised ✓");
            assert_eq!(snapshot[0]["author"], "Reviewer");
            assert_eq!(snapshot[0]["color"], json!([0, 0, 255]));
            assert_eq!(snapshot[0]["rect"], json!([30.0, 50.0, 150.0, 70.0]));
            let bytes = document.to_bytes().unwrap();
            let mut reopened = EditablePdf::from_bytes(&bytes, None).unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&reopened.annotations(0).unwrap()).unwrap(),
                snapshot
            );
            let (width, height, pixels) = pdf2_raster(&reopened);
            let blue = (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let p = &pixels[(y * width + x) * 4..][..4];
                    p[2] > p[0].saturating_add(30)
                })
                .collect::<Vec<_>>();
            assert!(!blue.is_empty(), "{kind} appearance missing");
            assert!(
                blue.iter()
                    .all(|&(x, y)| (30..150).contains(&x) && (50..70).contains(&y)),
                "{kind} move bounds {blue:?}"
            );
            assert_eq!(
                crate::pdf::extract(&bytes, None).unwrap().pages[0]
                    .blocks
                    .iter()
                    .map(|b| b.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                plain
            );
            reopened.delete_annotation(0, index).unwrap();
            assert_eq!(reopened.annotations(0).unwrap(), "[]");
            let (_, _, clean) = pdf2_raster(&reopened);
            assert!(!clean
                .chunks_exact(4)
                .any(|p| p[2] > p[0].saturating_add(30)));
        }
    }

    #[test]
    fn markup_appearances_follow_quad_orientation_in_all_rotated_cropped_views() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |dict| {
                dict.set(
                    b"CropBox".to_vec(),
                    Object::Array([20.0, 30.0, 220.0, 330.0].map(Object::Real).to_vec()),
                );
                dict.set(b"UserUnit".to_vec(), Object::Real(1.25));
            })
            .unwrap();
        let bytes = resource_fixture_bytes(&source);
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::from_bytes(&bytes, None).unwrap();
            document.rotate_page(0, rotation).unwrap();
            for (row, kind) in ["underline", "strikeout", "squiggly"]
                .into_iter()
                .enumerate()
            {
                let y = 20.0 + row as f64 * 50.0;
                document
                    .add_annotation(0, kind, (20.0, y, 140.0, y + 20.0), "", "", (255, 0, 0))
                    .unwrap();
            }
            let reopened = EditablePdf::from_bytes(&document.to_bytes().unwrap(), None).unwrap();
            let metadata: Value = serde_json::from_str(&reopened.annotations(0).unwrap()).unwrap();
            let (width, _, pixels) = pdf2_raster(&reopened);
            for row in 0..3 {
                let y = 20 + row * 50;
                let expected = [20.0, y as f64, 140.0, (y + 20) as f64];
                for (actual, want) in metadata[row]["rect"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .zip(expected)
                {
                    assert!((actual.as_f64().unwrap() - want).abs() < 0.001);
                }
                let ink = (y..y + 20)
                    .flat_map(|py| (20..140).map(move |x| (x, py)))
                    .filter(|&(x, py)| {
                        let p = &pixels[(py * width + x) * 4..][..4];
                        p[0] > p[1].saturating_add(30)
                    })
                    .collect::<Vec<_>>();
                assert!(
                    ink.len() > 60,
                    "rotation {rotation}, kind {} has no substantial appearance",
                    metadata[row]["kind"]
                );
                let top = ink.iter().map(|&(_, py)| py).min().unwrap();
                let bottom = ink.iter().map(|&(_, py)| py).max().unwrap();
                match row {
                    0 => assert!(
                        top >= y + 17 && bottom < y + 20,
                        "underline rotation {rotation}: {top}..{bottom}"
                    ),
                    1 => assert!(
                        top >= y + 10 && bottom <= y + 13,
                        "strikeout rotation {rotation}: {top}..{bottom}"
                    ),
                    _ => assert!(
                        top >= y + 16 && bottom < y + 20 && bottom > top,
                        "squiggly rotation {rotation}: {top}..{bottom}"
                    ),
                }
            }
        }
    }

    #[test]
    fn markup_appearance_budget_preflights_count_bytes_and_accounts_native_output() {
        use pdfcraft_cos::{Dict, Object};
        pyo3::prepare_freethreaded_python();
        let mut dict = Dict::new();
        for subtype in ["Highlight", "Underline", "StrikeOut", "Squiggly"] {
            dict.set(b"Subtype".to_vec(), Object::name(subtype));
            dict.set(
                b"QuadPoints".to_vec(),
                Object::Array(vec![Object::Int(0); (MAX_MARKUP_QUADS + 1) * 8]),
            );
            assert!(markup_dict_peak(&dict)
                .unwrap_err()
                .to_string()
                .contains("resource_limit:"));
        }
        dict.set(
            b"QuadPoints".to_vec(),
            Object::Array(vec![Object::Real(f64::NAN); 8]),
        );
        assert!(markup_dict_peak(&dict).is_err());
        // Opposing right corners overflow native interpolation without changing
        // the finite left height or bottom length used for the work estimate.
        let extreme = [-1e308, 40.0, 1e308, 40.0, -1e308, 20.0, -1e308, 20.0];
        assert!(markup_quad_peak(&extreme, false).is_err());
        assert!(markup_quad_peak(&[1_000_000.0; 8], false).is_ok());
        assert!(markup_quad_peak(&[1_000_001.0; 8], false).is_err());
        let long = [0.0, 1.0, 20000.0, 1.0, 0.0, 0.0, 20000.0, 0.0];
        assert_eq!(markup_quad_peak(&long, true).unwrap(), 4096 + 10001 * 1024);
        dict.set(
            b"QuadPoints".to_vec(),
            Object::Array((0..14).flat_map(|_| long.map(Object::Real)).collect()),
        );
        assert!(markup_dict_peak(&dict)
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        // The count guard must run on the mutation path before native props(), which
        // would otherwise build an appearance just to report editability.
        let mut malformed = EditablePdf::new(200.0, 200.0).unwrap();
        let index = malformed
            .add_annotation(
                0,
                "highlight",
                (20.0, 20.0, 140.0, 40.0),
                "",
                "",
                (255, 0, 0),
            )
            .unwrap();
        let native = malformed.document.as_mut().unwrap();
        let page = native.get(pdfcraft_organize::pages(native).unwrap()[0].obj);
        let annotation = match &page
            .as_dict()
            .unwrap()
            .get(b"Annots")
            .unwrap()
            .as_array()
            .unwrap()[index]
        {
            Object::Ref(reference) => *reference,
            _ => panic!("native annotation must be indirect"),
        };
        native
            .update_dict(annotation, |dict| {
                dict.set(
                    b"QuadPoints".to_vec(),
                    Object::Array(vec![Object::Int(0); (MAX_MARKUP_QUADS + 1) * 8]),
                )
            })
            .unwrap();
        let before = malformed.to_bytes().unwrap();
        assert!(malformed
            .update_annotation(
                0,
                index,
                Some("Do not commit"),
                None,
                None,
                None,
                None,
                None
            )
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(malformed.to_bytes().unwrap(), before);
        malformed.delete_annotation(0, index).unwrap();
        assert_eq!(malformed.annotations(0).unwrap(), "[]");
        for kind in ["highlight", "underline", "strikeout", "squiggly"] {
            let mut document = EditablePdf::new(200.0, 200.0).unwrap();
            document.input_bytes = MAX_DOCUMENT_INPUT - 1;
            let before = document.to_bytes().unwrap();
            assert!(document
                .add_annotation(0, kind, (20.0, 20.0, 140.0, 40.0), "", "", (255, 0, 0))
                .unwrap_err()
                .to_string()
                .contains("resource_limit:"));
            assert_eq!(document.to_bytes().unwrap(), before);
            assert_eq!(document.items, 0);
            document.input_bytes = 0;
            let index = document
                .add_annotation(0, kind, (20.0, 20.0, 140.0, 40.0), "", "", (255, 0, 0))
                .unwrap();
            let charged =
                annotation_appearance_bytes(document.document().unwrap(), 0, index).unwrap();
            assert!(charged > 0);
            assert_eq!(document.input_bytes, charged);
            document
                .update_annotation(0, index, None, None, Some((0, 0, 255)), None, None, None)
                .unwrap();
            assert_eq!(
                document.input_bytes,
                charged
                    + annotation_appearance_bytes(document.document().unwrap(), 0, index).unwrap()
            );
            document.input_bytes = MAX_DOCUMENT_INPUT - 1;
            let before = document.to_bytes().unwrap();
            assert!(document
                .update_annotation(
                    0,
                    index,
                    Some("Do not commit"),
                    None,
                    None,
                    None,
                    None,
                    None
                )
                .unwrap_err()
                .to_string()
                .contains("resource_limit:"));
            assert_eq!(document.to_bytes().unwrap(), before);
            // Removing a markup does not need to rebuild an appearance or spend its budget.
            document.delete_annotation(0, index).unwrap();
            assert_eq!(document.annotations(0).unwrap(), "[]");
        }
    }

    #[test]
    fn markup_invalid_changes_are_atomic() {
        pyo3::prepare_freethreaded_python();
        for kind in ["underline", "strikeout", "squiggly"] {
            let mut document = EditablePdf::new(200.0, 200.0).unwrap();
            let index = document
                .add_annotation(
                    0,
                    kind,
                    (20.0, 20.0, 140.0, 40.0),
                    "Before",
                    "",
                    (255, 0, 0),
                )
                .unwrap();
            let before = document.to_bytes().unwrap();
            let counts = (document.items, document.input_bytes);
            assert!(document
                .update_annotation(
                    0,
                    index,
                    Some("Do not commit"),
                    Some((20.0, 20.0, 150.0, 40.0)),
                    None,
                    None,
                    None,
                    None
                )
                .is_err());
            assert!(document
                .update_annotation(
                    0,
                    index,
                    Some("Do not commit"),
                    None,
                    None,
                    Some("https://example.com"),
                    None,
                    None
                )
                .is_err());
            assert!(document
                .add_annotation(0, kind, (20.0, 20.0, 19.0, 40.0), "", "", (0, 0, 0))
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
            assert_eq!((document.items, document.input_bytes), counts);
        }
    }

    #[test]
    fn annotation_validation_is_atomic_and_rejects_unsupported_edits() {
        let mut document = EditablePdf::new(100.0, 100.0).unwrap();
        let highlight = document
            .add_annotation(
                0,
                "highlight",
                (10.0, 10.0, 40.0, 20.0),
                "",
                "",
                (255, 255, 0),
            )
            .unwrap();
        let before = document.to_bytes().unwrap();
        for uri in ["", "https://example.com/\0bad", "https://example.com/\n"] {
            assert!(document
                .add_link(0, (10.0, 30.0, 40.0, 40.0), Some(uri), None)
                .is_err());
        }
        assert!(document
            .add_link(0, (0.0, 0.0, 10.0, 10.0), None, Some(9))
            .is_err());
        assert!(document
            .add_link(
                0,
                (0.0, 0.0, 10.0, 10.0),
                Some("https://example.com"),
                Some(0)
            )
            .is_err());
        assert!(document
            .add_annotation(0, "redaction", (10.0, 10.0, 30.0, 30.0), "", "", (0, 0, 0))
            .is_err());
        assert!(document
            .add_annotation(0, "note", (10.0, 10.0, 40.0, 30.0), "", "", (0, 0, 0))
            .is_err());
        assert!(document
            .update_annotation(
                0,
                highlight,
                Some("Must not commit"),
                Some((10.0, 10.0, 50.0, 20.0)),
                None,
                None,
                None,
                None
            )
            .is_err());
        assert!(document
            .update_annotation(
                0,
                highlight,
                None,
                None,
                None,
                Some("https://example.com"),
                None,
                None
            )
            .is_err());
        assert!(document.delete_annotation(0, 999).is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
    }

    #[test]
    fn annotation_rectangles_follow_rotated_cropped_user_unit_geometry() {
        use pdfcraft_cos::Object;
        let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
        let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
        source
            .update_dict(page, |page| {
                page.set(
                    b"CropBox".to_vec(),
                    Object::Array([20.0, 30.0, 220.0, 330.0].map(Object::Real).to_vec()),
                );
                page.set(b"UserUnit".to_vec(), Object::Real(1.25));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        for rotation in [0, 90, 180, 270] {
            let mut document = EditablePdf::from_bytes(&bytes, None).unwrap();
            document.rotate_page(0, rotation).unwrap();
            let expected = [40.0, 50.0, 100.0, 70.0];
            document
                .add_link(
                    0,
                    (40.0, 50.0, 100.0, 70.0),
                    Some("https://example.com"),
                    None,
                )
                .unwrap();
            document
                .add_annotation(
                    0,
                    "highlight",
                    (40.0, 50.0, 100.0, 70.0),
                    "",
                    "",
                    (255, 255, 0),
                )
                .unwrap();
            let reopened = EditablePdf::from_bytes(&document.to_bytes().unwrap(), None).unwrap();
            let values: Vec<Value> =
                serde_json::from_str(&reopened.annotations(0).unwrap()).unwrap();
            for value in values {
                for (actual, expected) in value["rect"].as_array().unwrap().iter().zip(expected) {
                    assert!(
                        (actual.as_f64().unwrap() - expected).abs() < 0.001,
                        "rotation {rotation}: {value}"
                    );
                }
            }
        }
    }

    #[test]
    fn blank_text_image_and_page_roundtrip() {
        let mut document = EditablePdf::new(300.0, 400.0).unwrap();
        document
            .add_text(0, "Hello PDF", 20.0, 30.0, 12.0, (0, 0, 0))
            .unwrap();
        let png = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
        document
            .add_image(0, &png, 20.0, 60.0, 10.0, 10.0, None, 0, false, false)
            .unwrap();
        assert_eq!(document.add_page(400.0, 300.0).unwrap(), 1);
        assert_eq!(document.page_size(-1).unwrap(), (400.0, 300.0));
        let bytes = document.to_bytes().unwrap();
        let extracted = crate::pdf::extract(&bytes, None).unwrap();
        assert_eq!(extracted.pages.len(), 2);
        assert!(extracted.pages[0]
            .blocks
            .iter()
            .any(|block| block.text.contains("Hello PDF")));
        assert_eq!(extracted.pages[0].images.len(), 1);
        assert!(extracted.pages[1].blocks.is_empty());
    }

    #[test]
    fn invalid_operations_are_atomic_and_copies_are_independent() {
        let mut original = EditablePdf::new(100.0, 100.0).unwrap();
        let before = original.to_bytes().unwrap();
        assert!(original
            .add_text(0, "too low", 0.0, 99.0, 12.0, (0, 0, 0))
            .is_err());
        assert!(original
            .add_text(0, "\u{1f600}", 0.0, 0.0, 12.0, (0, 0, 0))
            .is_err());
        assert!(original
            .add_text(1, "bad page", 0.0, 0.0, 12.0, (0, 0, 0))
            .is_err());
        assert!(original
            .add_image(0, b"not png", 0.0, 0.0, 10.0, 10.0, None, 0, false, false)
            .is_err());
        assert!(original.add_page(f64::INFINITY, 100.0).is_err());
        assert_eq!(original.to_bytes().unwrap(), before);
        let mut copied = original.copy().unwrap();
        copied
            .add_text(0, "copy", 10.0, 10.0, 12.0, (0, 0, 0))
            .unwrap();
        assert_eq!(original.to_bytes().unwrap(), before);
        assert_ne!(copied.to_bytes().unwrap(), before);
        copied.close();
        copied.close();
        assert!(copied.page_count().is_err());
        assert!(copied.copy().is_err());
        assert!(copied.to_bytes().is_err());
    }

    #[test]
    fn reopens_native_model_and_replaces_background_without_losing_text() {
        let mut source = EditablePdf::new(100.0, 100.0).unwrap();
        source
            .add_text(0, "Kept", 10.0, 10.0, 12.0, (0, 0, 0))
            .unwrap();
        let mut edited = EditablePdf::from_bytes(&source.to_bytes().unwrap(), None).unwrap();
        edited.set_background(0, (255, 0, 0)).unwrap();
        edited.set_background(0, (0, 255, 0)).unwrap();
        let bytes = edited.to_bytes().unwrap();
        let output = crate::pdf::extract(&bytes, None).unwrap();
        assert!(output.pages[0]
            .blocks
            .iter()
            .any(|block| block.text.contains("Kept")));
        let raster = crate::pdf::render(&bytes, 0, 72.0, None).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(raster)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(&pixels[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn organizes_pages_preserving_content_and_known_backgrounds() {
        let mut document = EditablePdf::new(300.0, 400.0).unwrap();
        document
            .add_text(0, "First", 20.0, 20.0, 12.0, (0, 0, 0))
            .unwrap();
        document.set_background(0, (200, 210, 220)).unwrap();
        document.add_page(200.0, 300.0).unwrap();
        document
            .add_text(1, "Second", 20.0, 20.0, 12.0, (0, 0, 0))
            .unwrap();
        document.move_page(0, -1).unwrap();
        assert_eq!(document.page_size(0).unwrap(), (200.0, 300.0));
        assert_eq!(document.background(0).unwrap(), None);
        assert_eq!(document.background(1).unwrap(), Some((200, 210, 220)));
        document.rotate_page(-1, 90).unwrap();
        assert_eq!(document.page_size(1).unwrap(), (400.0, 300.0));
        document
            .add_text(1, "After rotation", 20.0, 70.0, 12.0, (0, 0, 0))
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let extracted = crate::pdf::extract(&bytes, None).unwrap();
        assert!(extracted.pages[0]
            .blocks
            .iter()
            .any(|block| block.text.contains("Second")));
        assert!(extracted.pages[1]
            .blocks
            .iter()
            .any(|block| block.text.contains("First")));
        assert!(extracted.pages[1]
            .blocks
            .iter()
            .any(|block| block.text.contains("After rotation")));
        document.delete_page(0).unwrap();
        assert_eq!(document.page_count().unwrap(), 1);
        assert_eq!(document.background(0).unwrap(), Some((200, 210, 220)));
        let before = document.to_bytes().unwrap();
        assert!(document.delete_page(0).is_err());
        assert!(document.move_page(0, 1).is_err());
        assert!(document.rotate_page(0, 45).is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
    }

    #[test]
    fn refuses_encrypted_signature_and_certification_inputs() {
        use pdfcraft_cos::{Algorithm, Dict, NewEncryption, Object, PdfString};

        let mut source = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        let mut signature = Dict::new();
        signature.set(b"FT".to_vec(), Object::name("Sig"));
        signature.set(b"T".to_vec(), PdfString::text("Signature"));
        let signature = source.add(Object::Dict(signature));
        let mut form = Dict::new();
        form.set(
            b"Fields".to_vec(),
            Object::Array(vec![Object::Ref(signature)]),
        );
        source
            .update_dict(source.root().unwrap(), |catalog| {
                catalog.set(b"AcroForm".to_vec(), Object::Dict(form));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&source, &SaveOptions::default()).unwrap();
        let info = pdfcraft_render::inspect(Arc::new(bytes.clone()), None).unwrap();
        assert!(info
            .fields
            .iter()
            .any(|field| field.kind == pdfcraft_render::FieldKind::Signature));
        assert!(EditablePdf::from_bytes(&bytes, None).is_err());

        let mut certified = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        certified
            .update_dict(certified.root().unwrap(), |catalog| {
                catalog.set(b"Perms".to_vec(), Object::Dict(Dict::new()));
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&certified, &SaveOptions::default()).unwrap();
        assert!(EditablePdf::from_bytes(&bytes, None).is_err());

        let mut encrypted = pdfcraft_create::blank(100.0, 100.0, 1).unwrap();
        encrypted
            .set_encryption(&NewEncryption {
                algorithm: Algorithm::Aes256,
                user_password: "reader",
                owner_password: "owner",
                permissions: -4,
                encrypt_metadata: true,
                seed: [42; 32], // Deterministic encryption fixture, never used for production keys.
            })
            .unwrap();
        let bytes = pdfcraft_cos::write_full(&encrypted, &SaveOptions::default()).unwrap();
        assert!(EditablePdf::from_bytes(&bytes, None).is_err());
        assert!(EditablePdf::from_bytes(&bytes, Some("reader")).is_err());
        assert!(EditablePdf::from_bytes(&bytes, Some("owner")).is_err());
    }

    #[test]
    fn imports_and_extracts_pages_in_requested_order_with_independent_resources() {
        let mut source = EditablePdf::new(100.0, 120.0).unwrap();
        source
            .add_text(0, "Source A", 10.0, 10.0, 10.0, (0, 0, 0))
            .unwrap();
        let image = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
        source
            .add_image(0, &image, 10.0, 40.0, 20.0, 20.0, None, 0, false, false)
            .unwrap();
        source.add_page(120.0, 100.0).unwrap();
        source
            .add_text(1, "Source B", 10.0, 10.0, 10.0, (0, 0, 0))
            .unwrap();
        let source_bytes = source.to_bytes().unwrap();
        let mut destination = EditablePdf::new(100.0, 100.0).unwrap();
        destination
            .add_text(0, "Destination", 10.0, 10.0, 10.0, (0, 0, 0))
            .unwrap();
        assert_eq!(
            destination
                .import_pages(&source_bytes, None, Some(vec![1, 0, 1]), 0, None)
                .unwrap(),
            3
        );
        let output = crate::pdf::extract(&destination.to_bytes().unwrap(), None).unwrap();
        assert_eq!(output.pages.len(), 4);
        for (index, text) in ["Source B", "Source A", "Source B", "Destination"]
            .iter()
            .enumerate()
        {
            assert!(output.pages[index]
                .blocks
                .iter()
                .any(|block| block.text.contains(text)));
        }
        assert_eq!(output.pages[1].images.len(), 1);
        let mut subset = destination.subset(vec![-1, 1]).unwrap();
        subset
            .add_text(0, "Subset", 10.0, 50.0, 10.0, (0, 0, 0))
            .unwrap();
        let extracted = crate::pdf::extract(&subset.to_bytes().unwrap(), None).unwrap();
        assert_eq!(extracted.pages.len(), 2);
        assert!(extracted.pages[0]
            .blocks
            .iter()
            .any(|block| block.text.contains("Subset")));
        assert_eq!(extracted.pages[1].images.len(), 1);
        assert_eq!(source.to_bytes().unwrap(), source_bytes);
        let before = destination.to_bytes().unwrap();
        assert!(destination
            .import_pages(&source_bytes, None, Some(vec![99]), 0, None)
            .is_err());
        assert!(destination
            .import_pages(&source_bytes, None, None, 99, None)
            .is_err());
        assert!(destination.subset(vec![]).is_err());
        assert!(destination.subset(vec![99]).is_err());
        assert_eq!(destination.to_bytes().unwrap(), before);
    }

    #[test]
    fn jpeg_embedding_uses_upstream_metadata_and_rejects_huge_frames_atomically() {
        use base64::Engine;
        // Generated 2x1 solid RGB test image; no external asset or encoder needed at test time.
        let jpeg = base64::engine::general_purpose::STANDARD.decode(
            "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAIDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDiaKKK8Q/XD//Z"
        ).unwrap();
        let mut document = EditablePdf::new(100.0, 100.0).unwrap();
        document
            .add_image(0, &jpeg, 10.0, 10.0, 20.0, 10.0, None, 0, false, false)
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let extracted = crate::pdf::extract(&bytes, None).unwrap();
        assert_eq!(extracted.pages[0].images[0].media_type, "image/jpeg");
        assert_eq!(extracted.pages[0].images[0].metadata["width"], 2);
        assert!(crate::pdf::render(&bytes, 0, 72.0, None).is_ok());
        let mut huge = jpeg;
        let frame = huge
            .windows(2)
            .position(|bytes| bytes == [0xff, 0xc0])
            .unwrap();
        huge[frame + 7..frame + 9].copy_from_slice(&20_000_u16.to_be_bytes());
        assert!(document
            .add_image(0, &huge, 10.0, 10.0, 20.0, 10.0, None, 0, false, false)
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), bytes);
    }
}
