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
        if info.encrypted {
            return Err(
                EngineError::Unsupported("editing encrypted PDFs is not available".into()).into(),
            );
        }
        if info
            .fields
            .iter()
            .any(|field| field.kind == pdfcraft_render::FieldKind::Signature)
        {
            return Err(EngineError::Unsupported(
                "editing PDFs with signature fields is not available".into(),
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
        })
    }

    pub fn close(&mut self) {
        self.document = None;
        self.sizes.clear();
        self.geometry.clear();
        self.backgrounds.clear();
    }

    pub fn to_bytes(&self) -> PyResult<Vec<u8>> {
        let output = pdfcraft_cos::write_full(self.document()?, &SaveOptions::default()).map_err(
            |error| EngineError::Conversion(format!("PDF serialization failed: {error}")),
        )?;
        if output.len() > MAX_OUTPUT_BYTES {
            return Err(EngineError::ResourceLimit("PDF output exceeds 256 MiB".into()).into());
        }
        Ok(output)
    }

    pub fn page_count(&self) -> PyResult<usize> {
        self.document()?;
        Ok(self.sizes.len())
    }

    pub fn page_size(&self, page: isize) -> PyResult<(f64, f64)> {
        Ok(self.sizes[self.page_index(page)?])
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
        self.document = Some(edited);
        let size = self.sizes.remove(page);
        self.sizes.insert(destination, size);
        let geometry = self.geometry.remove(page);
        self.geometry.insert(destination, geometry);
        let background = self.backgrounds.remove(page);
        self.backgrounds.insert(destination, background);
        Ok(())
    }

    #[pyo3(signature = (data, password, indices, destination))]
    pub fn import_pages(
        &mut self,
        data: &[u8],
        password: Option<&str>,
        indices: Option<Vec<isize>>,
        destination: isize,
    ) -> PyResult<usize> {
        self.document()?;
        let destination = if destination < 0 {
            destination + self.sizes.len() as isize
        } else {
            destination
        };
        if destination < 0 || destination as usize > self.sizes.len() {
            return Err(PyIndexError::new_err("page insertion index out of range"));
        }
        let source = Self::from_bytes(data, password)?;
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
        let imported = pdfcraft_organize::import_pages(
            &mut edited,
            source.document()?,
            &selected,
            destination as usize,
        )
        .map_err(organize_error)?;
        if imported.len() != selected.len() {
            return Err(EngineError::Conversion(
                "PDF import returned an unexpected page count".into(),
            )
            .into());
        }
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
        })
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
            "highlight" => pdfcraft_annot::Shape::TextMarkup {
                kind: pdfcraft_annot::Markup::Highlight,
                quads: vec![self.geometry[page].view_rect_to_quad(rect)],
            },
            _ => {
                return Err(EngineError::Unsupported(format!(
                    "annotation kind {kind}; supported kinds are note and highlight"
                ))
                .into())
            }
        };
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
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += text.len() + author.len();
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
                other => other.to_ascii_lowercase(),
            };
            values.push((annotation.index, json!({
                "index": annotation.index, "kind": kind, "text": annotation.contents.unwrap_or_default(),
                "author": annotation.author.unwrap_or_default(), "color": annotation.color.map(|color| color.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)),
                "rect": view_rect(geometry, annotation.rect), "uri": null, "target_page": null,
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
        self.validate_input(cost)?;
        let mut edited = self.document()?.clone();
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
        } else if props.subtype == "Text" || props.subtype == "Highlight" {
            if action.is_some() {
                return Err(EngineError::Unsupported(
                    "comment annotations do not have link targets".into(),
                )
                .into());
            }
            let meta = pdfcraft_annot::Meta::default();
            if let Some(rect) = user_rect {
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
                        "sticky notes and highlights can be moved, but resizing is not available"
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
        self.document = Some(edited);
        self.input_bytes += cost;
        self.items += 1;
        Ok(())
    }

    pub fn delete_annotation(&mut self, page: isize, index: usize) -> PyResult<()> {
        let page = self.page_index(page)?;
        let props = pdfcraft_annot::props(self.document()?, page, index)
            .ok_or_else(|| PyIndexError::new_err("annotation index out of range"))?;
        if !pdfcraft_annot::is_comment_subtype(&props.subtype) && props.subtype != "Link" {
            return Err(EngineError::Unsupported(format!(
                "deleting {} annotations is not available",
                props.subtype
            ))
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
                "the background of an imported PDF page cannot be inferred; set a background to manage it".into(),
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
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += cost;
        Ok(())
    }

    /// Explicit rows with a caller-supplied embeddable font; no system-font lookup or fallback.
    #[allow(clippy::too_many_arguments)]
    pub fn add_text_with_font(
        &mut self,
        page: isize,
        text: &str,
        x: f64,
        y: f64,
        size: f64,
        font_bytes: &[u8],
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
        let (width, height) = self.sizes[page];
        validate_position(x, y, width, height)?;
        pdfcraft_fonts::embedding::validate_document_input(font_bytes).map_err(|message| {
            EngineError::Unsupported(format!("font cannot be embedded: {message}"))
        })?;
        let (_, family) =
            pdfcraft_fonts::embedding::names(font_bytes).map_err(EngineError::Unsupported)?;
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
        let mut ops = vec![Op::new("q", vec![])];
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
                let gx = x + glyph.position[0] * size;
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
                    "rg",
                    vec![Object::Int(0), Object::Int(0), Object::Int(0)],
                ));
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
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += cost;
        Ok(())
    }

    /// x/y are the top-left of a text box, in points. Text wraps to the page's right edge.
    #[pyo3(signature = (page, text, x, y, size=12.0))]
    pub fn add_text(&mut self, page: isize, text: &str, x: f64, y: f64, size: f64) -> PyResult<()> {
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
        let (width, height) = self.sizes[page];
        validate_position(x, y, width, height)?;
        if width - x < 1.0 {
            return Err(PyValueError::new_err(
                "text box must be at least 1 point wide",
            ));
        }
        let item = AddedText {
            rect: [x, 0.0, width, height - y],
            text: text.into(),
            size,
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
        let occupied = pdfcraft_edit::added::text_rect(&item);
        if occupied[1] < 0.0 || !occupied.iter().all(|value| value.is_finite()) {
            return Err(PyValueError::new_err(
                "text does not fit within the page height",
            ));
        }
        for line in pdfcraft_edit::added::lines(&item) {
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
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += text.len();
        Ok(())
    }

    /// Place PNG/JPEG bytes at a top-left position without resampling source pixels.
    #[allow(clippy::too_many_arguments)] // The narrow native boundary receives flattened geometry.
    pub fn add_image(
        &mut self,
        page: isize,
        data: &[u8],
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> PyResult<()> {
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
            &Content::Image(AddedImage::new(rect, image)),
        )
        .map_err(edit_error)?;
        self.document = Some(edited);
        self.items += 1;
        self.input_bytes += data.len();
        Ok(())
    }
}

impl EditablePdf {
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

fn metadata_keys() -> [(&'static str, &'static str); 4] {
    [
        ("title", "Title"),
        ("author", "Author"),
        ("subject", "Subject"),
        ("keywords", "Keywords"),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;

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
            .add_text_with_font(0, text, 20.0, 20.0, 16.0, pdfcraft_fonts::INTER_REGULAR)
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
                .add_text_with_font(0, text, x, y, size, font)
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
                .add_text_with_font(0, "ΩЖ", 20.0, 20.0, 16.0, pdfcraft_fonts::INTER_REGULAR)
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
        document.add_text(0, "Hello PDF", 20.0, 30.0, 12.0).unwrap();
        let png = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
        document.add_image(0, &png, 20.0, 60.0, 10.0, 10.0).unwrap();
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
        assert!(original.add_text(0, "too low", 0.0, 99.0, 12.0).is_err());
        assert!(original.add_text(0, "\u{1f600}", 0.0, 0.0, 12.0).is_err());
        assert!(original.add_text(1, "bad page", 0.0, 0.0, 12.0).is_err());
        assert!(original
            .add_image(0, b"not png", 0.0, 0.0, 10.0, 10.0)
            .is_err());
        assert!(original.add_page(f64::INFINITY, 100.0).is_err());
        assert_eq!(original.to_bytes().unwrap(), before);
        let mut copied = original.copy().unwrap();
        copied.add_text(0, "copy", 10.0, 10.0, 12.0).unwrap();
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
        source.add_text(0, "Kept", 10.0, 10.0, 12.0).unwrap();
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
        document.add_text(0, "First", 20.0, 20.0, 12.0).unwrap();
        document.set_background(0, (200, 210, 220)).unwrap();
        document.add_page(200.0, 300.0).unwrap();
        document.add_text(1, "Second", 20.0, 20.0, 12.0).unwrap();
        document.move_page(0, -1).unwrap();
        assert_eq!(document.page_size(0).unwrap(), (200.0, 300.0));
        assert_eq!(document.background(0).unwrap(), None);
        assert_eq!(document.background(1).unwrap(), Some((200, 210, 220)));
        document.rotate_page(-1, 90).unwrap();
        assert_eq!(document.page_size(1).unwrap(), (400.0, 300.0));
        document
            .add_text(1, "After rotation", 20.0, 70.0, 12.0)
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
        source.add_text(0, "Source A", 10.0, 10.0, 10.0).unwrap();
        let image = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
        source.add_image(0, &image, 10.0, 40.0, 20.0, 20.0).unwrap();
        source.add_page(120.0, 100.0).unwrap();
        source.add_text(1, "Source B", 10.0, 10.0, 10.0).unwrap();
        let source_bytes = source.to_bytes().unwrap();
        let mut destination = EditablePdf::new(100.0, 100.0).unwrap();
        destination
            .add_text(0, "Destination", 10.0, 10.0, 10.0)
            .unwrap();
        assert_eq!(
            destination
                .import_pages(&source_bytes, None, Some(vec![1, 0, 1]), 0)
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
        subset.add_text(0, "Subset", 10.0, 50.0, 10.0).unwrap();
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
            .import_pages(&source_bytes, None, Some(vec![99]), 0)
            .is_err());
        assert!(destination
            .import_pages(&source_bytes, None, None, 99)
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
            .add_image(0, &jpeg, 10.0, 10.0, 20.0, 10.0)
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
            .add_image(0, &huge, 10.0, 10.0, 20.0, 10.0)
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), bytes);
    }
}
