//! Native-owned DOCX creation. Python never reconstructs this model from extraction.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    sync::Arc,
};

use pyo3::exceptions::{PyIndexError, PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyInt};
use serde_json::{json, Map, Value};
use wordcraft_doc::para::{Float, InlineObject, NoteKind};
use wordcraft_doc::props::{
    Border, BorderStyle, Borders, HeightRule, Highlight, LineSpacing, NumRef, TabAlign, TabLeader,
    TabStop, TextDirection, Underline, VAlign, VMerge, VertAlign, TAB_POS_EPS,
};
use wordcraft_doc::section::{
    LineNumberRestart, LineNumbering, NotePos, NoteRestart, NumFormat, SectionProps, SectionStart,
};
use wordcraft_doc::styles::{PartFormat, PartSet, Style, StyleSheet, TablePart};
use wordcraft_doc::{
    Align, Block, CharProps, Document, ListKind, ParaProps, Paragraph, PartKind, Path, Pos, Rgb,
    StoryRef, StyleKind, Table, TextColor,
};

use crate::{DocExNativeError, EngineError};

const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_DOCUMENT_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_BLOCKS: usize = 100_000;
const MAX_CELLS: usize = 100_000;
const MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_MEDIA_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_LISTS: usize = 10_000;
const MAX_PAGE_FIELDS: usize = 10_000;
const MAX_LINE_NUMBER_WORK: usize = 100_000;
const MAX_OUTLINE_WORK: usize = 100_000;
const MAX_LIST_MATCH_WORK: usize = 2_000_000;

/// Private boundary value: Python booleans must not become numbering starts.
#[derive(Clone, Copy)]
pub struct ListStart(u32);

impl<'py> FromPyObject<'py> for ListStart {
    fn extract_bound(value: &Bound<'py, PyAny>) -> PyResult<Self> {
        if value.is_instance_of::<PyBool>() || !value.is_instance_of::<PyInt>() {
            return Err(PyTypeError::new_err(
                "list start must be an integer, not a boolean",
            ));
        }
        let start = value.extract::<u32>()?;
        if !(1..=32767).contains(&start) {
            return Err(PyValueError::new_err(
                "list start must be between 1 and 32767",
            ));
        }
        Ok(Self(start))
    }
}
const MAX_LINK_BYTES: usize = 16 * 1024 * 1024;
const MAX_STYLES: usize = 1024;
const MAX_FORMAT_PATCH_BYTES: usize = 16 * 1024;
const MAX_SECTIONS: usize = 128;
const MAX_NOTES: usize = 10_000;
const MAX_NOTE_CONVERSION_WORK: usize = 2_000_000;
const MAX_BOOKMARKS: usize = 1000;
const MAX_BOOKMARK_WORK: usize = 2_000_000;
const MAX_TABLE_PATCH_BYTES: usize = 128 * 1024;
const MAX_TABLE_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_TAB_STOPS: usize = 64;
const MAX_LAYOUT_PAGES: usize = 10_000;
const MAX_LAYOUT_ITEMS: usize = 2_000_000;
const MAX_PROPERTY_BYTES: usize = 256 * 1024;
const MAX_PROPERTY_VALUE_BYTES: usize = 64 * 1024;
const MAX_CUSTOM_PROPERTIES: usize = 1024;

#[pyclass]
pub struct EditableDocx {
    document: Option<Document>,
    // Native body position, never an editable handle ID.
    body_placeholder: Option<usize>,
    text_bytes: usize,
    cell_count: usize,
    revision: u64,
    images: Vec<usize>,
    media_bytes: usize,
    locations: BTreeMap<usize, (StoryRef, Path)>,
    stories: BTreeMap<u32, Vec<usize>>,
    story_placeholders: BTreeMap<u32, usize>,
    next_block_id: usize,
    cell_placeholders: BTreeSet<(StoryRef, Path)>,
    table_generations: BTreeMap<usize, u64>,
}

#[pymethods]
impl EditableDocx {
    #[new]
    pub fn new() -> Self {
        Self {
            document: Some(Document::new()),
            body_placeholder: Some(0),
            text_bytes: 0,
            cell_count: 0,
            revision: 0,
            images: Vec::new(),
            media_bytes: 0,
            locations: BTreeMap::new(),
            stories: BTreeMap::new(),
            story_placeholders: BTreeMap::new(),
            next_block_id: MAX_BLOCKS,
            cell_placeholders: BTreeSet::new(),
            table_generations: BTreeMap::new(),
        }
    }

    pub fn copy(&self) -> PyResult<Self> {
        Ok(Self {
            document: Some(self.document()?.clone()),
            body_placeholder: self.body_placeholder,
            text_bytes: self.text_bytes,
            cell_count: self.cell_count,
            revision: self.revision,
            images: self.images.clone(),
            media_bytes: self.media_bytes,
            locations: self.locations.clone(),
            stories: self.stories.clone(),
            story_placeholders: self.story_placeholders.clone(),
            next_block_id: self.next_block_id,
            cell_placeholders: self.cell_placeholders.clone(),
            table_generations: self.table_generations.clone(),
        })
    }

    pub fn close(&mut self) {
        self.document = None;
    }

    pub fn revision(&self) -> PyResult<u64> {
        self.document()?;
        Ok(self.revision)
    }

    pub fn metadata(&self) -> PyResult<String> {
        Ok(document_metadata(self.document()?)?.to_string())
    }

    pub fn update_metadata(&mut self, updates: &str) -> PyResult<()> {
        self.document()?;
        let values = property_patch(updates)?;
        if values.is_empty() {
            return Ok(());
        }
        let mut candidate = self.document()?.clone();
        for (key, value) in values {
            if key == "custom" {
                patch_custom_metadata(&mut candidate, value)?;
                continue;
            }
            if key == "revision" {
                candidate.core.revision = if value.is_null() {
                    0
                } else {
                    value
                        .as_u64()
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or_else(|| {
                            PyValueError::new_err(
                                "revision requires an unsigned 32-bit integer or null",
                            )
                        })?
                };
                continue;
            }
            let target = match key.as_str() {
                "title" => &mut candidate.core.title,
                "author" => &mut candidate.core.creator,
                "subject" => &mut candidate.core.subject,
                "keywords" => &mut candidate.core.keywords,
                "description" => &mut candidate.core.description,
                "category" => &mut candidate.core.category,
                "last_modified_by" => &mut candidate.core.last_modified_by,
                "created" => &mut candidate.core.created,
                "modified" => &mut candidate.core.modified,
                _ => {
                    return Err(PyValueError::new_err(format!(
                        "unknown metadata key: {key}"
                    )))
                }
            };
            let text = match &value {
                Value::Null => "",
                Value::String(text) => text.as_str(),
                _ => {
                    return Err(PyValueError::new_err(
                        "metadata values require strings or null, except revision",
                    ))
                }
            };
            validate_property_text(text)?;
            if text.trim() != text {
                return Err(PyValueError::new_err("DOCX metadata cannot preserve leading or trailing whitespace at this native revision"));
            }
            if matches!(key.as_str(), "created" | "modified") && !text.is_empty() {
                validate_property_timestamp(text)?;
            }
            *target = text.to_owned();
        }
        if core_metadata(&candidate).to_string().len() > MAX_PROPERTY_BYTES {
            return Err(limit("metadata exceeds 256 KiB of serialized values"));
        }
        self.commit(candidate)
    }

    pub fn to_bytes(&self) -> PyResult<Vec<u8>> {
        validate_line_number_work(self.document()?)?;
        validate_unsplittable_page_bounds(self.document()?)?;
        validate_unequal_column_tables(self.document()?)?;
        let output = wordcraft_docx::write(self.document()?).map_err(|error| {
            EngineError::Conversion(format!("DOCX serialization failed: {error}"))
        })?;
        if output.len() > MAX_OUTPUT_BYTES {
            return Err(limit("DOCX output exceeds 128 MiB"));
        }
        crate::ooxml::preflight(&output)?;
        Ok(output)
    }

    pub fn add_paragraph(&mut self, text: &str) -> PyResult<usize> {
        self.reserve_block()?;
        validate_text(text)?;
        let text_bytes = self.updated_text_bytes(0, text.len())?;
        let (id, next) = self.next_identifier()?;
        let mut candidate = self.document()?.clone();
        let paragraph = Block::Para(Paragraph::with_text(text, CharProps::default()));
        let index = if let Some(index) = self.body_placeholder {
            *candidate
                .body
                .get_mut(index)
                .ok_or_else(|| PyValueError::new_err("body placeholder no longer exists"))? =
                Arc::new(paragraph);
            index
        } else {
            if candidate.body.len() >= MAX_BLOCKS {
                return Err(limit("document exceeds 100000 blocks"));
            }
            let index = candidate.body.len();
            candidate
                .insert_block(StoryRef::Body, &Path::top(index), paragraph)
                .map_err(model_error)?;
            index
        };
        self.commit(candidate)?;
        self.locations
            .insert(id, (StoryRef::Body, Path::top(index)));
        self.next_block_id = next;
        self.body_placeholder = None;
        self.text_bytes = text_bytes;
        Ok(id)
    }

    pub fn paragraph_ids(&self) -> PyResult<Vec<usize>> {
        self.body_child_ids(false)
    }

    pub fn paragraph_text(&self, id: usize) -> PyResult<String> {
        Ok(self.paragraph(id)?.text.clone())
    }

    pub fn set_paragraph_text(&mut self, id: usize, text: &str) -> PyResult<()> {
        let existing = self.paragraph(id)?;
        reject_semantic_range(existing, 0, existing.text.len())?;
        validate_text(text)?;
        let text_bytes = self.updated_text_bytes(existing.text.len(), text.len())?;
        // Replacing all text intentionally resets run formatting; retain paragraph properties.
        let mut replacement = Paragraph::with_text(text, existing.mark.clone());
        replacement.props = existing.props.clone();
        replacement.section = existing.section.clone();
        let mut candidate = self.document()?.clone();
        *self.paragraph_mut(&mut candidate, id)? = replacement;
        self.commit(candidate)?;
        self.text_bytes = text_bytes;
        Ok(())
    }

    pub fn add_table(&mut self, rows: Vec<Vec<String>>) -> PyResult<usize> {
        self.reserve_block()?;
        let section = &self.document()?.last_section;
        let width = column_width(section);
        let (mut table, added_text, cells) = self.make_table(rows, width)?;
        if !section.columns.widths.is_empty() {
            // Native autofit can enlarge a new table to its starting column and
            // retain that width when it flows into a narrower one.
            table.props.width = Some(width);
            table.props.fixed = true;
        }
        let text_bytes = self.updated_text_bytes(0, added_text)?;
        let (id, next) = self.next_identifier()?;
        let mut candidate = self.document()?.clone();
        if candidate.body.len() + 2 > MAX_BLOCKS {
            return Err(limit("document exceeds 100000 blocks"));
        }
        let index = self.body_placeholder.unwrap_or(candidate.body.len());
        if self.body_placeholder.is_some() {
            *candidate
                .body
                .get_mut(index)
                .ok_or_else(|| PyValueError::new_err("body placeholder no longer exists"))? =
                Arc::new(Block::Table(table));
        } else {
            candidate
                .insert_block(StoryRef::Body, &Path::top(index), Block::Table(table))
                .map_err(model_error)?;
        }
        let placeholder = candidate.body.len();
        candidate
            .body
            .push(wordcraft_doc::para_block(Paragraph::new()));
        self.commit(candidate)?;
        self.locations
            .insert(id, (StoryRef::Body, Path::top(index)));
        self.next_block_id = next;
        self.body_placeholder = Some(placeholder);
        self.text_bytes = text_bytes;
        self.cell_count += cells;
        Ok(id)
    }

    pub fn table_ids(&self) -> PyResult<Vec<usize>> {
        self.body_child_ids(true)
    }

    pub fn table_rows(&self, id: usize) -> PyResult<Vec<Vec<String>>> {
        let table = self.table(id)?;
        let mut rows = vec![vec![String::new(); table.cols()]; table.rows.len()];
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut column = 0;
            for cell in &row.cells {
                if cell.props.vmerge != VMerge::Continue {
                    rows[row_index][column] = cell_plain_text(cell);
                }
                column += cell.span();
            }
        }
        Ok(rows)
    }

    pub fn table_generation(&self, id: usize) -> PyResult<u64> {
        self.table(id)?;
        Ok(*self.table_generations.get(&id).unwrap_or(&0))
    }

    pub fn insert_row(&mut self, id: usize, index: usize) -> PyResult<()> {
        let table = self.table(id)?;
        if index > table.rows.len() {
            return Err(PyIndexError::new_err(
                "table insertion row index out of range",
            ));
        }
        if table.rows.len() >= wordcraft_doc::table::MAX_ROWS {
            return Err(limit("table exceeds 32767 rows"));
        }
        let like = index.saturating_sub(1);
        if self.cell_count.saturating_add(table.rows[like].cells.len()) > MAX_CELLS {
            return Err(limit("document exceeds 100000 table cells"));
        }
        self.mutate_table_structure(id, |table| {
            table.insert_row(index, like);
            Ok(())
        })
    }

    pub fn delete_row(&mut self, id: usize, index: usize) -> PyResult<()> {
        let table = self.table(id)?;
        if index >= table.rows.len() {
            return Err(PyIndexError::new_err("table row index out of range"));
        }
        if table.rows.len() == 1 {
            return Err(PyValueError::new_err("cannot delete the final table row"));
        }
        self.mutate_table_structure(id, |table| {
            table.delete_row(index);
            Ok(())
        })
    }

    pub fn insert_column(&mut self, id: usize, index: usize) -> PyResult<()> {
        let table = self.table(id)?;
        if index > table.grid.len() {
            return Err(PyIndexError::new_err(
                "table insertion column index out of range",
            ));
        }
        if table.grid.len() >= wordcraft_doc::table::MAX_COLS {
            return Err(limit("table exceeds 63 columns"));
        }
        let added = table
            .rows
            .iter()
            .filter(|row| {
                let mut start = 0;
                for cell in &row.cells {
                    let end = start + cell.span();
                    if start < index && index < end {
                        return false;
                    }
                    start = end;
                }
                true
            })
            .count();
        if self.cell_count.saturating_add(added) > MAX_CELLS {
            return Err(limit("document exceeds 100000 table cells"));
        }
        self.mutate_table_structure(id, |table| {
            table.insert_col(index);
            Ok(())
        })
    }

    pub fn delete_column(&mut self, id: usize, index: usize) -> PyResult<()> {
        let table = self.table(id)?;
        if index >= table.grid.len() {
            return Err(PyIndexError::new_err("table column index out of range"));
        }
        if table.grid.len() == 1 {
            return Err(PyValueError::new_err(
                "cannot delete the final table column",
            ));
        }
        self.mutate_table_structure(id, |table| {
            table.delete_col(index);
            Ok(())
        })
    }

    pub fn column_widths(&self, id: usize) -> PyResult<Vec<f64>> {
        Ok(self
            .table(id)?
            .grid
            .iter()
            .map(|width| f64::from(*width))
            .collect())
    }

    pub fn set_column_widths(&mut self, id: usize, widths: Vec<f64>) -> PyResult<()> {
        let table = self.table(id)?;
        if widths.len() != table.cols()
            || widths
                .iter()
                .any(|width| !width.is_finite() || *width < 6.0)
        {
            return Err(PyValueError::new_err(
                "column widths require one finite value of at least 6 points per grid column",
            ));
        }
        let total = widths.iter().sum::<f64>();
        if total > f64::from(self.container_width(id)?) + 0.001 {
            return Err(PyValueError::new_err(
                "table columns exceed the available page or cell width",
            ));
        }
        // A parent resize must not silently squeeze an explicitly sized nested table or image.
        for (row_index, row) in table.rows.iter().enumerate() {
            for (cell_index, cell) in row.cells.iter().enumerate() {
                let start = table.grid_col(row_index, cell_index);
                let width = widths[start..start + cell.span()].iter().sum::<f64>();
                let margins =
                    wordcraft_layout::cell_margins(self.document()?, table, row_index, cell_index);
                let available = width - f64::from(margins[1] + margins[3]);
                if cell.props.margins.is_some() && available <= 0.0 {
                    return Err(PyValueError::new_err(
                        "column resize must leave positive width inside explicit cell margins",
                    ));
                }
                for block in &cell.blocks {
                    let exceeds = match block.as_ref() {
                        Block::Table(nested) => f64::from(nested.total_width()) > available + 0.001,
                        Block::Para(paragraph) => paragraph.objects.iter().any(|object| matches!(object, InlineObject::Image { w, .. } if f64::from(*w) > available + 0.001)),
                    };
                    if exceeds {
                        return Err(PyValueError::new_err("column resize would overflow an existing nested table or image; resize the child first"));
                    }
                }
            }
        }
        let mut candidate = self.document()?.clone();
        let table = self.table_mut(&mut candidate, id)?;
        table.grid = widths.into_iter().map(|width| width as f32).collect();
        table.props.width = Some(total as f32);
        table.props.width_pct = None;
        table.props.fixed = true;
        for row in &mut table.rows {
            let mut column = row.props.grid_before as usize;
            for cell in &mut row.cells {
                let end = column + cell.span();
                cell.props.width = Some(table.grid[column..end].iter().sum());
                cell.props.width_pct = None;
                column = end;
            }
        }
        self.validate_table_layout(&candidate, id)?;
        self.commit(candidate)
    }

    pub fn table_formatting(&self, id: usize) -> PyResult<String> {
        let props = &self.table(id)?.props;
        Ok(json!({
            "alignment": props.align.map(alignment_name), "indent": props.indent,
            "width": props.width, "width_percent": props.width_pct,
            "layout": if props.fixed { "fixed" } else { "autofit" },
            "rtl": props.bidi_visual, "title": props.caption, "description": props.description,
            "borders": borders_snapshot(props.borders, true),
            "style": props.style, "look": table_look_snapshot(&props.look),
        })
        .to_string())
    }

    pub fn table_format_patch(&mut self, id: usize, patch: &str) -> PyResult<()> {
        let values = table_patch(patch)?;
        let assigned_style = values
            .get("style")
            .map(|value| table_style_reference(&self.document()?.styles, value))
            .transpose()?;
        let mut candidate = self.document()?.clone();
        let table = self.table_mut(&mut candidate, id)?;
        if values.get("width").is_some_and(|v| !v.is_null())
            && values.get("width_percent").is_some_and(|v| !v.is_null())
        {
            return Err(PyValueError::new_err(
                "set width or width_percent, not both",
            ));
        }
        for (key, value) in &values {
            match key.as_str() {
                "alignment" => table.props.align = nullable_alignment(value, false)?,
                "indent" => table.props.indent = nullable_point(value, 0.0, 1584.0, "indent")?,
                "width" => table.props.width = nullable_point(value, 6.0, 1584.0, "width")?,
                "width_percent" => {
                    table.props.width_pct = nullable_point(value, 0.02, 100.0, "width_percent")?
                }
                "layout" => {
                    table.props.fixed = match value.as_str() {
                        Some("fixed") => true,
                        Some("autofit") => false,
                        _ => return Err(PyValueError::new_err("layout must be fixed or autofit")),
                    }
                }
                "rtl" => table.props.bidi_visual = strict_bool(value, "rtl")?,
                "title" => table.props.caption = table_alt(value)?,
                "description" => table.props.description = table_alt(value)?,
                "borders" => apply_border_patch(&mut table.props.borders, value, true)?,
                "style" => table.props.style = assigned_style.clone().flatten(),
                "look" => patch_table_look(&mut table.props.look, value)?,
                _ => {
                    return Err(PyValueError::new_err(format!(
                        "unknown table format property: {key}"
                    )))
                }
            }
        }
        // Setting one width preference also clears the old competing preference,
        // independent of JSON map order. Null alone clears only the named preference.
        if values.get("width").is_some_and(|v| !v.is_null()) {
            table.props.width_pct = None;
        }
        if values.get("width_percent").is_some_and(|v| !v.is_null()) {
            table.props.width = None;
        }
        if table
            .props
            .width_pct
            .is_some_and(|v| ((v * 50.0).round() - v * 50.0).abs() > 0.001)
        {
            return Err(PyValueError::new_err(
                "width_percent requires 0.02-percent increments",
            ));
        }
        if values.keys().any(|key| {
            matches!(
                key.as_str(),
                "width" | "width_percent" | "indent" | "layout" | "alignment"
            )
        }) {
            self.validate_table_layout(&candidate, id)?;
        }
        if values.contains_key("style") || values.contains_key("look") {
            validate_style_tables(self.document()?, &candidate)?;
        }
        self.commit(candidate)
    }

    pub fn cell_formatting(&self, id: usize, row: usize, column: usize) -> PyResult<String> {
        let table = self.table(id)?;
        let (row, cell) = cell_anchor(table, row, column)?;
        let props = &table.rows[row].cells[cell].props;
        Ok(json!({
            "background": props.shading.map(rgb_string),
            "margins": props.margins.map(|[top,left,bottom,right]| [top,right,bottom,left]),
            "alignment": self.cell_alignment(id, row, column)?,
            "vertical_alignment": props.valign.map(vertical_alignment_name),
            "text_direction": if props.text_direction_set { Some(text_direction_name(props.text_direction)) } else { None },
            "no_wrap": props.no_wrap,
            "borders": borders_snapshot(props.borders, false),
        }).to_string())
    }

    pub fn cell_format_patch(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        patch: &str,
    ) -> PyResult<()> {
        let values = table_patch(patch)?;
        let (row, cell_index) = cell_anchor(self.table(id)?, row, column)?;
        let allocated_width = if values.contains_key("margins") {
            let (story, path) = self.location(id)?;
            Some(native_cell_width(
                &native_cell_widths(self.document()?)?,
                story,
                &path,
                row,
                cell_index,
            )?)
        } else {
            None
        };
        let mut candidate = self.document()?.clone();
        let cell = &mut self.table_mut(&mut candidate, id)?.rows[row].cells[cell_index];
        for (key, value) in &values {
            match key.as_str() {
                "background" => {
                    cell.props.shading = if value.is_null() {
                        None
                    } else {
                        Some(parse_rgb(value.as_str().ok_or_else(|| {
                            PyValueError::new_err("background requires a color or None")
                        })?)?)
                    }
                }
                "margins" => {
                    cell.props.margins = if value.is_null() {
                        None
                    } else {
                        let [top, right, bottom, left] = section_numbers::<4>(value, "margins")?;
                        Some([top, left, bottom, right])
                    }
                }
                "alignment" => {
                    let alignment = nullable_alignment(value, true)?;
                    for block in &mut cell.blocks {
                        if let Block::Para(paragraph) = Arc::make_mut(block) {
                            paragraph.props.align = alignment;
                            paragraph.touch();
                        }
                    }
                }
                "vertical_alignment" => {
                    cell.props.valign = if value.is_null() {
                        None
                    } else {
                        Some(match value.as_str() {
                            Some("top") => VAlign::Top,
                            Some("center") => VAlign::Center,
                            Some("bottom") => VAlign::Bottom,
                            _ => {
                                return Err(PyValueError::new_err(
                                    "vertical_alignment must be top, center, bottom or None",
                                ))
                            }
                        })
                    }
                }
                "text_direction" => {
                    cell.props.text_direction_set = !value.is_null();
                    cell.props.text_direction = match value.as_str() {
                        None if value.is_null() => TextDirection::Horizontal,
                        Some("horizontal") => TextDirection::Horizontal,
                        Some("down") => TextDirection::Down,
                        Some("up") => TextDirection::Up,
                        _ => {
                            return Err(PyValueError::new_err(
                                "text_direction must be horizontal, down, up or None",
                            ))
                        }
                    };
                }
                "no_wrap" => cell.props.no_wrap = strict_bool(value, "no_wrap")?,
                "borders" => apply_border_patch(&mut cell.props.borders, value, false)?,
                _ => {
                    return Err(PyValueError::new_err(format!(
                        "unknown cell format property: {key}"
                    )))
                }
            }
        }
        if let Some(width) = allocated_width {
            let (story, path) = self.location(id)?;
            let table = candidate
                .table(story, &path)
                .ok_or_else(|| PyIndexError::new_err("table no longer exists"))?;
            // Margins change within the current allocation. An explicit fit operation
            // is required to redistribute columns around previously fixed child content.
            let margins = wordcraft_layout::cell_margins(&candidate, table, row, cell_index);
            let available = width - margins[1] - margins[3];
            if available <= 0.0 {
                return Err(PyValueError::new_err(
                    "cell margins must leave positive content width",
                ));
            }
            blocks_fit_width(&table.rows[row].cells[cell_index].blocks, available)?;
        }
        if values.keys().any(|key| {
            matches!(
                key.as_str(),
                "margins" | "text_direction" | "no_wrap" | "alignment"
            )
        }) {
            self.validate_table_layout(&candidate, id)?;
        }
        self.commit(candidate)
    }

    pub fn fit_table(&mut self, id: usize) -> PyResult<Vec<f64>> {
        let table = self.table(id)?;
        let measured_paragraphs = table
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .flat_map(|cell| &cell.blocks)
            .filter(|block| block.as_para().is_some())
            .count();
        if measured_paragraphs > 4000 {
            return Err(limit(
                "native content fitting supports at most 4000 direct cell paragraphs",
            ));
        }
        let available = self.container_width(id)? - active_table_indent(table);
        let measured = wordcraft_layout::measure_table_columns(self.document()?, table);
        let widths = wordcraft_layout::autofit_widths(&measured, available);
        if widths
            .iter()
            .any(|width| !width.is_finite() || *width < 6.0)
            || widths.iter().sum::<f32>() > available + 0.001
        {
            return Err(PyValueError::new_err(
                "table content minimum widths exceed available room",
            ));
        }
        let mut candidate = self.document()?.clone();
        let table = self.table_mut(&mut candidate, id)?;
        table.grid = widths;
        synchronize_grid_widths(table);
        table.props.width = None;
        table.props.width_pct = None;
        table.props.fixed = false;
        self.validate_table_layout(&candidate, id)?;
        self.commit(candidate)?;
        self.column_widths(id)
    }

    pub fn row_height(&self, id: usize, row: usize) -> PyResult<(Option<f64>, &'static str)> {
        let props = &self.row(id, row)?.props;
        Ok((
            props.height.map(f64::from),
            match props.height_rule {
                HeightRule::Auto => "auto",
                HeightRule::AtLeast => "at_least",
                HeightRule::Exact => "exact",
            },
        ))
    }

    #[pyo3(signature = (id, row, height=None, rule="auto"))]
    pub fn set_row_height(
        &mut self,
        id: usize,
        row: usize,
        height: Option<f64>,
        rule: &str,
    ) -> PyResult<()> {
        self.row(id, row)?;
        let rule = match (rule, height) {
            ("auto", None) => HeightRule::Auto,
            ("at_least", Some(value)) | ("exact", Some(value)) if value.is_finite() && (0.05..=1584.0).contains(&value) => {
                if rule == "exact" { HeightRule::Exact } else { HeightRule::AtLeast }
            }
            _ => return Err(PyValueError::new_err("row height requires (None, auto) or a finite height in [0.05, 1584] points with at_least/exact")),
        };
        let mut candidate = self.document()?.clone();
        let props = &mut self.table_mut(&mut candidate, id)?.rows[row].props;
        props.height = height.map(|height| height as f32);
        props.height_rule = rule;
        self.commit(candidate)
    }

    pub fn row_repeat_header(&self, id: usize, row: usize) -> PyResult<bool> {
        Ok(self.row(id, row)?.props.header)
    }

    pub fn set_row_repeat_header(&mut self, id: usize, row: usize, repeat: bool) -> PyResult<()> {
        self.row(id, row)?;
        let rows = &self.table(id)?.rows;
        if (repeat && rows[..row].iter().any(|row| !row.props.header))
            || (!repeat && rows[row + 1..].iter().any(|row| row.props.header))
        {
            return Err(PyValueError::new_err(
                "repeating header rows must form a contiguous prefix of the table",
            ));
        }
        let mut candidate = self.document()?.clone();
        self.table_mut(&mut candidate, id)?.rows[row].props.header = repeat;
        self.commit(candidate)
    }

    pub fn row_allow_split(&self, id: usize, row: usize) -> PyResult<bool> {
        Ok(!self.row(id, row)?.props.cant_split)
    }

    pub fn set_row_allow_split(&mut self, id: usize, row: usize, allow: bool) -> PyResult<()> {
        self.row(id, row)?;
        let mut candidate = self.document()?.clone();
        self.table_mut(&mut candidate, id)?.rows[row]
            .props
            .cant_split = !allow;
        self.commit(candidate)
    }

    pub fn table_borders(&self, id: usize) -> PyResult<Option<(String, f64)>> {
        let table = self.table(id)?;
        let inherited = table
            .props
            .style
            .as_deref()
            .and_then(|style| self.document().ok()?.styles.table_style(style))
            .and_then(|style| style.parts.borders);
        let mut borders = inherited.unwrap_or_default();
        if let Some(direct) = table.props.borders {
            borders.overlay(&direct);
        }
        if !borders.any_visible() {
            return Ok(None);
        }
        let sides = [
            borders.top,
            borders.left,
            borders.bottom,
            borders.right,
            borders.between,
            borders.inside_v,
        ];
        let border =
            sides[0].ok_or_else(|| PyValueError::new_err("table borders are not uniform"))?;
        if border.style != BorderStyle::Single || sides.iter().any(|side| *side != Some(border)) {
            return Err(PyValueError::new_err(
                "table borders are not uniform solid lines",
            ));
        }
        let Rgb(red, green, blue) = border.color.unwrap_or(Rgb(0, 0, 0));
        Ok(Some((
            format!("#{red:02X}{green:02X}{blue:02X}"),
            f64::from(border.width),
        )))
    }

    #[pyo3(signature = (id, color="#000000", width=0.5))]
    pub fn set_table_borders(&mut self, id: usize, color: &str, width: f64) -> PyResult<()> {
        if !width.is_finite() {
            return Err(PyValueError::new_err("border width must be finite"));
        }
        let border = json!({"style":"single","color":color,"width":width});
        self.table_format_patch(
            id,
            &json!({"borders":{
                "top":border,"right":border,"bottom":border,"left":border,
                "inside_horizontal":border,"inside_vertical":border,
            }})
            .to_string(),
        )
    }

    pub fn clear_table_borders(&mut self, id: usize) -> PyResult<()> {
        let border = json!({"style":"none"});
        self.table_format_patch(
            id,
            &json!({"borders":{
                "top":border,"right":border,"bottom":border,"left":border,
                "inside_horizontal":border,"inside_vertical":border,
            }})
            .to_string(),
        )
    }

    pub fn cell_text(&self, id: usize, row: usize, column: usize) -> PyResult<String> {
        let table = self.table(id)?;
        let (row, cell) = cell_anchor(table, row, column)?;
        Ok(cell_plain_text(&table.rows[row].cells[cell]))
    }

    pub fn set_cell_text(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        text: &str,
    ) -> PyResult<()> {
        let table = self.table(id)?;
        let (row, column) = cell_anchor(table, row, column)?;
        let cell = &table.rows[row].cells[column];
        validate_text(text)?;
        let (old_size, removed_cells) = content_usage(&cell.blocks);
        let (story, table_path) = self.location(id)?;
        let prefix = Path([table_path.0.as_slice(), &[row as u32, column as u32]].concat());
        let text_bytes = self.updated_text_bytes(old_size, text.len())?;
        let mut candidate = self.document()?.clone();
        let table = self.table_mut(&mut candidate, id)?;
        table.rows[row].cells[column].blocks = vec![wordcraft_doc::para_block(
            Paragraph::with_text(text, CharProps::default()),
        )];
        self.commit(candidate)?;
        self.invalidate_descendants(story, &prefix);
        self.text_bytes = text_bytes;
        self.cell_count -= removed_cells;
        Ok(())
    }

    pub fn cell_paragraph_ids(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
    ) -> PyResult<Vec<usize>> {
        self.cell_child_ids(id, row, column, "paragraph")
    }

    pub fn cell_table_ids(&mut self, id: usize, row: usize, column: usize) -> PyResult<Vec<usize>> {
        self.cell_child_ids(id, row, column, "table")
    }

    pub fn cell_image_ids(&mut self, id: usize, row: usize, column: usize) -> PyResult<Vec<usize>> {
        self.cell_child_ids(id, row, column, "image")
    }

    pub fn cell_add_paragraph(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        text: &str,
    ) -> PyResult<usize> {
        validate_text(text)?;
        self.add_cell_block(
            id,
            row,
            column,
            Block::Para(Paragraph::with_text(text, CharProps::default())),
            text.len(),
            0,
            None,
        )
    }

    pub fn cell_add_table(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        rows: Vec<Vec<String>>,
    ) -> PyResult<usize> {
        let table = self.table(id)?;
        let (anchor_row, cell) = cell_anchor(table, row, column)?;
        let margins = wordcraft_layout::cell_margins(self.document()?, table, anchor_row, cell);
        let (story, table_path) = self.location(id)?;
        let widths = native_cell_widths(self.document()?)?;
        let width = native_cell_width(&widths, story, &table_path, anchor_row, cell)?
            - margins[1]
            - margins[3];
        let (_, path) = self.location(id)?;
        if path.depth() >= 15 {
            return Err(limit("DOCX table nesting exceeds 16 levels"));
        }
        let (mut nested, text_bytes, cells) = self.make_table(rows, width)?;
        nested.props.width = Some(width);
        nested.props.fixed = true;
        self.add_cell_block(
            id,
            row,
            column,
            Block::Table(nested),
            text_bytes,
            cells,
            None,
        )
    }

    #[pyo3(signature = (id, row, column, data, width=None, height=None, alt=""))]
    #[allow(clippy::too_many_arguments)]
    pub fn cell_add_image(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        data: &[u8],
        width: Option<f64>,
        height: Option<f64>,
        alt: &str,
    ) -> PyResult<usize> {
        let paragraph = self.make_image(data, width, height, alt)?;
        let table = self.table(id)?;
        let (anchor_row, cell) = cell_anchor(table, row, column)?;
        let margins = wordcraft_layout::cell_margins(self.document()?, table, anchor_row, cell);
        let (story, table_path) = self.location(id)?;
        let widths = native_cell_widths(self.document()?)?;
        validate_picture_width(
            &paragraph,
            native_cell_width(&widths, story, &table_path, anchor_row, cell)?
                - margins[1]
                - margins[3],
        )?;
        self.add_cell_block(
            id,
            row,
            column,
            Block::Para(paragraph),
            0,
            0,
            Some(data.to_vec()),
        )
    }

    pub fn merge_cells(
        &mut self,
        id: usize,
        r0: usize,
        c0: usize,
        r1: usize,
        c1: usize,
    ) -> PyResult<()> {
        let table = self.table(id)?;
        if r0 > r1 || c0 > c1 || r1 >= table.rows.len() || c1 >= table.cols() {
            return Err(PyValueError::new_err(
                "merge requires an ordered rectangle within the table grid",
            ));
        }
        for row in r0..=r1 {
            let first = table
                .cell_at_grid(row, c0)
                .ok_or_else(|| PyIndexError::new_err("merge starts outside a cell"))?;
            let last = table
                .cell_at_grid(row, c1)
                .ok_or_else(|| PyIndexError::new_err("merge ends outside a cell"))?;
            if table.grid_col(row, first) != c0
                || table.grid_col(row, last) + table.rows[row].cells[last].span() != c1 + 1
                || table.rows[row].cells[first..=last]
                    .iter()
                    .any(|cell| cell.props.vmerge != VMerge::None)
            {
                return Err(PyValueError::new_err(
                    "merge cannot cut an existing merged cell or intersect a vertical merge",
                ));
            }
        }
        self.mutate_table_structure(id, |table| {
            table.merge(r0, r1, c0, c1);
            Ok(())
        })
    }

    #[pyo3(signature = (id, row, column, columns=2))]
    pub fn split_cell(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        columns: usize,
    ) -> PyResult<()> {
        let table = self.table(id)?;
        let (row, cell_index) = cell_anchor(table, row, column)?;
        let cell = &table.rows[row].cells[cell_index];
        if !(2..=wordcraft_doc::table::MAX_COLS).contains(&columns) {
            return Err(PyValueError::new_err("split requires 2..63 columns"));
        }
        if cell.props.vmerge != VMerge::None {
            return Err(PyValueError::new_err(
                "horizontal splitting of vertical merges is not supported",
            ));
        }
        let extra_grid = if cell.span() >= columns {
            0
        } else {
            columns - 1
        };
        if table.cols() + extra_grid > wordcraft_doc::table::MAX_COLS
            || self.cell_count + columns - 1 > MAX_CELLS
        {
            return Err(limit(
                "split exceeds the table column or document cell limit",
            ));
        }
        let before = table.rows[row].cells.len();
        self.mutate_table_structure(id, |table| {
            table.split_cell(row, cell_index, columns);
            if table.rows[row].cells.len() != before + columns - 1 {
                return Err(EngineError::Unsupported(
                    "upstream could not split the selected cell exactly".into(),
                )
                .into());
            }
            Ok(())
        })
    }

    pub fn cell_margins(
        &self,
        id: usize,
        row: usize,
        column: usize,
    ) -> PyResult<(f64, f64, f64, f64)> {
        let table = self.table(id)?;
        let (row, cell) = cell_anchor(table, row, column)?;
        let [top, left, bottom, right] =
            wordcraft_layout::cell_margins(self.document()?, table, row, cell);
        Ok((
            f64::from(top),
            f64::from(right),
            f64::from(bottom),
            f64::from(left),
        ))
    }

    #[pyo3(signature = (id, row, column, margins=None))]
    pub fn set_cell_margins(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        margins: Option<(f64, f64, f64, f64)>,
    ) -> PyResult<()> {
        if margins.is_some_and(|(top, right, bottom, left)| {
            [top, right, bottom, left].iter().any(|v| !v.is_finite())
        }) {
            return Err(PyValueError::new_err("cell margins must be finite"));
        }
        self.cell_format_patch(
            id,
            row,
            column,
            &json!({"margins": margins.map(|(top,right,bottom,left)| [top,right,bottom,left])})
                .to_string(),
        )
    }

    pub fn cell_alignment(
        &self,
        id: usize,
        row: usize,
        column: usize,
    ) -> PyResult<Option<&'static str>> {
        let table = self.table(id)?;
        let (row, cell) = cell_anchor(table, row, column)?;
        let mut alignment = None;
        for paragraph in table.rows[row].cells[cell]
            .blocks
            .iter()
            .filter_map(|block| block.as_para())
        {
            let value = paragraph.props.align;
            if alignment.is_some_and(|previous| previous != value) {
                return Err(PyValueError::new_err(
                    "cell paragraphs have mixed direct alignment",
                ));
            }
            alignment = Some(value);
        }
        Ok(alignment.flatten().map(|value| match value {
            Align::Left => "left",
            Align::Right => "right",
            Align::Center => "center",
            Align::Justify => "justify",
            Align::Distribute => "distribute",
        }))
    }

    #[pyo3(signature = (id, row, column, alignment=None))]
    pub fn set_cell_alignment(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        alignment: Option<&str>,
    ) -> PyResult<()> {
        self.cell_format_patch(
            id,
            row,
            column,
            &json!({"alignment": alignment}).to_string(),
        )
    }

    pub fn cell_background(
        &self,
        id: usize,
        row: usize,
        column: usize,
    ) -> PyResult<Option<String>> {
        let table = self.table(id)?;
        let (row, cell) = cell_anchor(table, row, column)?;
        Ok(table.rows[row].cells[cell]
            .props
            .shading
            .map(|color| format!("#{:02X}{:02X}{:02X}", color.0, color.1, color.2)))
    }

    #[pyo3(signature = (id, row, column, color=None))]
    pub fn set_cell_background(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        color: Option<&str>,
    ) -> PyResult<()> {
        self.cell_format_patch(id, row, column, &json!({"background": color}).to_string())
    }

    pub fn cell_vertical_alignment(
        &self,
        id: usize,
        row: usize,
        column: usize,
    ) -> PyResult<&'static str> {
        let table = self.table(id)?;
        let (row, cell) = cell_anchor(table, row, column)?;
        Ok(
            match table.rows[row].cells[cell].props.valign.unwrap_or_default() {
                VAlign::Top => "top",
                VAlign::Center => "center",
                VAlign::Bottom => "bottom",
            },
        )
    }

    pub fn set_cell_vertical_alignment(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        alignment: &str,
    ) -> PyResult<()> {
        self.cell_format_patch(
            id,
            row,
            column,
            &json!({"vertical_alignment": alignment}).to_string(),
        )
    }

    pub fn paragraph_style(&self, id: usize) -> PyResult<String> {
        let paragraph = self.paragraph(id)?;
        let style = paragraph.props.style.as_deref().unwrap_or("Normal");
        Ok(self
            .document()?
            .styles
            .get(style)
            .map_or_else(|| style.to_string(), |value| value.name.clone()))
    }

    #[pyo3(signature = (kind=None))]
    pub fn style_ids(&self, kind: Option<&str>) -> PyResult<Vec<String>> {
        let document = self.document()?;
        let wanted = kind.map(style_kind).transpose()?;
        Ok(document
            .styles
            .styles
            .iter()
            .filter(|style| {
                matches!(
                    style.kind,
                    StyleKind::Paragraph | StyleKind::Character | StyleKind::Table
                ) && wanted.is_none_or(|kind| kind == style.kind)
            })
            .map(|style| style.id.clone())
            .collect())
    }

    pub fn style_info(&self, id: &str) -> PyResult<String> {
        let style = supported_style(&self.document()?.styles, id)?;
        Ok(json!({"id":style.id,"name":style.name,"kind":match style.kind {StyleKind::Paragraph => "paragraph", StyleKind::Character => "character", StyleKind::Table => "table", _ => "unsupported"},"based_on":style.based_on,"next":style.next,"linked":style.linked,"builtin":style.builtin}).to_string())
    }

    pub fn style_relations_patch(&mut self, id: &str, patch: &str) -> PyResult<()> {
        let document = self.document()?;
        let style = supported_style(&document.styles, id)?;
        let (id, kind) = (style.id.clone(), style.kind);
        let values = format_patch(patch)?;
        if values
            .keys()
            .any(|key| !matches!(key.as_str(), "next" | "linked"))
        {
            return Err(PyValueError::new_err(
                "style relations accept only next and linked",
            ));
        }
        let mut candidate = document.clone();
        if let Some(value) = values.get("next") {
            if kind != StyleKind::Paragraph {
                return Err(PyValueError::new_err("next requires a paragraph style"));
            }
            let next = if value.is_null() {
                None
            } else {
                let next = supported_style(&document.styles, text_value(value)?)?;
                if next.kind != StyleKind::Paragraph {
                    return Err(PyValueError::new_err(
                        "next must reference a paragraph style",
                    ));
                }
                Some(next.id.clone())
            };
            candidate
                .styles
                .get_mut(&id)
                .ok_or_else(|| PyValueError::new_err("unknown style"))?
                .next = next;
        }
        if let Some(value) = values.get("linked") {
            if !matches!(kind, StyleKind::Paragraph | StyleKind::Character) {
                return Err(PyValueError::new_err(
                    "linked requires a paragraph or character style",
                ));
            }
            let target = if value.is_null() {
                None
            } else {
                let target = supported_style(&document.styles, text_value(value)?)?;
                if !matches!(
                    (kind, target.kind),
                    (StyleKind::Paragraph, StyleKind::Character)
                        | (StyleKind::Character, StyleKind::Paragraph)
                ) {
                    return Err(PyValueError::new_err(
                        "linked styles must pair paragraph and character styles",
                    ));
                }
                if target
                    .linked
                    .as_deref()
                    .is_some_and(|partner| partner != id)
                {
                    return Err(PyValueError::new_err(
                        "linked target already belongs to another style; clear that link first",
                    ));
                }
                Some(target.id.clone())
            };
            // Remove only the source's reciprocal old relation; never disconnect an
            // unrelated partner. Pinned built-in headings may name an absent
            // HeadingNChar: clearing that dangling source reference is safe.
            if let Some(previous) = &style.linked {
                if let Some(partner) = candidate.styles.get_mut(previous) {
                    if partner.linked.as_deref() == Some(id.as_str()) {
                        partner.linked = None;
                    }
                }
            }
            candidate
                .styles
                .get_mut(&id)
                .ok_or_else(|| PyValueError::new_err("unknown style"))?
                .linked = target.clone();
            if let Some(target) = target {
                candidate
                    .styles
                    .get_mut(&target)
                    .ok_or_else(|| PyValueError::new_err("unknown linked style"))?
                    .linked = Some(id);
            }
        }
        // next is a follow-on paragraph hint, not an inheritance edge: self
        // references and cycles are valid and do not enter style resolution.
        self.commit(candidate)
    }

    pub fn default_formatting(&self) -> PyResult<String> {
        let styles = &self.document()?.styles;
        let mut values = char_snapshot(&styles.default_chr);
        values.remove("style");
        values.extend(para_snapshot(&styles.default_para)?);
        Ok(Value::Object(values).to_string())
    }

    pub fn format_defaults(&mut self, patch: &str) -> PyResult<()> {
        let document = self.document()?;
        let values = format_patch(patch)?;
        if values.get("font_size").is_some_and(Value::is_null) {
            return Err(EngineError::Unsupported(
                "clearing document-default font_size is unavailable: pinned upstream native resolution uses 11pt but DOCX reopening substitutes 10pt; set an explicit font_size".into(),
            ).into());
        }
        let (mut character, mut paragraph) = (Map::new(), Map::new());
        for (key, value) in values {
            if is_paragraph_key(&key) {
                paragraph.insert(key, value);
            } else {
                character.insert(key, value);
            }
        }
        let has_paragraph = !paragraph.is_empty();
        let character = CharPatch::parse(&document.styles, &character, false)?;
        let paragraph = ParaPatch::parse(&paragraph)?;
        let mut candidate = document.clone();
        character.apply(&mut candidate.styles.default_chr);
        paragraph.apply(&mut candidate.styles.default_para);
        if has_paragraph {
            candidate.styles.default_para_present = true;
        }
        validate_document_tabs(&candidate)?;
        validate_style_tables(document, &candidate)?;
        self.commit(candidate)
    }

    #[pyo3(signature = (name, kind="paragraph", based_on=None))]
    pub fn add_style(
        &mut self,
        name: &str,
        kind: &str,
        based_on: Option<&str>,
    ) -> PyResult<String> {
        let document = self.document()?;
        validate_style_name(name)?;
        let kind = style_kind(kind)?;
        if document.styles.styles.len() >= MAX_STYLES {
            return Err(limit("document exceeds 1024 named styles"));
        }
        if document.styles.styles.iter().any(|style| {
            style.name.eq_ignore_ascii_case(name) || style.id.eq_ignore_ascii_case(name)
        }) {
            return Err(PyValueError::new_err(
                "style name is already used by a style name or identifier",
            ));
        }
        let id = document.styles.new_id(name);
        if document
            .styles
            .styles
            .iter()
            .any(|style| style.id == id || style.name.eq_ignore_ascii_case(&id))
        {
            return Err(PyValueError::new_err(
                "generated style identifier conflicts with an existing style",
            ));
        }
        let base = style_base(&document.styles, based_on, kind)?;
        let mut candidate = document.clone();
        candidate.styles.upsert(Style {
            id: id.clone(),
            name: name.into(),
            kind,
            based_on: base,
            ..Default::default()
        });
        validate_style_chains(&candidate.styles)?;
        self.commit(candidate)?;
        Ok(id)
    }

    #[pyo3(signature = (id, base=None))]
    pub fn set_style_base(&mut self, id: &str, base: Option<&str>) -> PyResult<()> {
        let styles = &self.document()?.styles;
        let style = supported_style(styles, id)?;
        let (id, kind) = (style.id.clone(), style.kind);
        let base = style_base(styles, base, kind)?;
        let mut candidate = self.document()?.clone();
        candidate
            .styles
            .get_mut(&id)
            .ok_or_else(|| PyValueError::new_err("unknown style"))?
            .based_on = base;
        validate_style_chains(&candidate.styles)?;
        validate_style_tables(self.document()?, &candidate)?;
        self.commit(candidate)
    }

    #[pyo3(signature = (id, patch, region=None))]
    pub fn format_style(&mut self, id: &str, patch: &str, region: Option<&str>) -> PyResult<()> {
        let document = self.document()?;
        let style = supported_style(&document.styles, id)?;
        let id = style.id.clone();
        if style.kind == StyleKind::Table {
            let part = table_region(region.unwrap_or("whole_table"))?;
            let values = table_patch(patch)?;
            let mut candidate = document.clone();
            patch_table_style(
                candidate
                    .styles
                    .get_mut(&id)
                    .ok_or_else(|| PyValueError::new_err("unknown table style"))?,
                &document.styles,
                part,
                values,
            )?;
            validate_style_tables(document, &candidate)?;
            return self.commit(candidate);
        }
        if region.is_some() {
            return Err(PyValueError::new_err("regions require a table style"));
        }
        let values = format_patch(patch)?;
        let (mut character, mut paragraph) = (Map::new(), Map::new());
        for (key, value) in values {
            if style.kind == StyleKind::Paragraph && is_paragraph_key(&key) {
                paragraph.insert(key, value);
            } else {
                character.insert(key, value);
            }
        }
        let character = CharPatch::parse(&document.styles, &character, false)?;
        let paragraph = ParaPatch::parse(&paragraph)?;
        let mut candidate = document.clone();
        let style = candidate
            .styles
            .get_mut(&id)
            .ok_or_else(|| PyValueError::new_err("unknown style"))?;
        character.apply(&mut style.chr);
        paragraph.apply(&mut style.para);
        if paragraph.values.tabs.is_some() || paragraph.clear.contains("tabs") {
            validate_style_tables(document, &candidate)?;
        }
        self.commit(candidate)
    }

    #[pyo3(signature = (id, resolved=false, region=None))]
    pub fn style_formatting(
        &self,
        id: &str,
        resolved: bool,
        region: Option<&str>,
    ) -> PyResult<String> {
        let document = self.document()?;
        let style = supported_style(&document.styles, id)?;
        if style.kind == StyleKind::Table {
            return table_style_snapshot(&document.styles, style, resolved, region)
                .map(|v| v.to_string());
        }
        if region.is_some() {
            return Err(PyValueError::new_err("regions require a table style"));
        }
        let mut values = if resolved {
            let character = if style.kind == StyleKind::Paragraph {
                document
                    .styles
                    .resolve_char(Some(&style.id), &CharProps::default())
            } else {
                document.styles.resolve_char(
                    None,
                    &CharProps {
                        style: Some(style.id.clone()),
                        ..Default::default()
                    },
                )
            };
            resolved_char_snapshot(&character)
        } else {
            char_snapshot(&style.chr)
        };
        values.remove("style");
        if style.kind == StyleKind::Paragraph {
            let paragraph = if resolved {
                resolved_para_snapshot(&document.styles.resolve_para(&ParaProps {
                    style: Some(style.id.clone()),
                    ..Default::default()
                }))
            } else {
                para_snapshot(&style.para)
            }?;
            values.extend(paragraph);
        }
        Ok(Value::Object(values).to_string())
    }

    pub fn format_paragraph(&mut self, id: usize, patch: &str) -> PyResult<()> {
        self.paragraph(id)?;
        let formatting = ParaPatch::parse(&format_patch(patch)?)?;
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        formatting.apply(&mut paragraph.props);
        paragraph.touch();
        if formatting.values.tabs.is_some() || formatting.clear.contains("tabs") {
            validate_style_tables(self.document()?, &candidate)?;
        }
        self.commit(candidate)
    }

    #[pyo3(signature = (id, resolved=false))]
    pub fn paragraph_formatting(&self, id: usize, resolved: bool) -> PyResult<String> {
        let paragraph = self.paragraph(id)?;
        let values = if resolved {
            let document = self.document()?;
            let (story, path) = self.location(id)?;
            let table = paragraph_table_context(document, story, &path);
            resolved_para_snapshot(
                &document
                    .styles
                    .resolve_para_in(&paragraph.props, table.as_ref()),
            )?
        } else {
            para_snapshot(&paragraph.props)?
        };
        Ok(Value::Object(values).to_string())
    }

    pub fn paragraph_list(&self, id: usize) -> PyResult<Option<(&'static str, u8)>> {
        let paragraph = self.paragraph(id)?;
        let document = self.document()?;
        let Some(numbering) = document.styles.resolve_para(&paragraph.props).numbering else {
            return Ok(None);
        };
        let definitions = list_definitions();
        if let Some(name) = supported_list_kind(document, numbering.num, &definitions, &mut 0)? {
            return Ok(Some((name, numbering.level)));
        }
        Err(PyValueError::new_err(
            "paragraph uses an unsupported list style",
        ))
    }

    #[pyo3(signature = (id, kind=None, level=0, restart=false, start=None))]
    pub fn set_paragraph_list(
        &mut self,
        id: usize,
        kind: Option<&str>,
        level: u8,
        restart: bool,
        start: Option<ListStart>,
    ) -> PyResult<()> {
        self.paragraph(id)?;
        let start = start.map(|value| value.0);
        if start.is_some_and(|value| !(1..=32767).contains(&value)) {
            return Err(PyValueError::new_err(
                "list start must be between 1 and 32767",
            ));
        }
        if start.is_some() && (!matches!(kind, Some("numbered" | "decimal_outline")) || level != 0)
        {
            return Err(PyValueError::new_err(
                "list start requires a top-level numbered or decimal_outline list",
            ));
        }
        if start.is_some() {
            let (story, _) = self.location(id)?;
            if let StoryRef::Part(part) = story {
                if self
                    .document()?
                    .parts
                    .get(&part)
                    .is_some_and(|part| matches!(part.kind, PartKind::Header | PartKind::Footer))
                {
                    return Err(EngineError::Unsupported("explicit list starts in headers/footers are unavailable: native measurement advances their counters".into()).into());
                }
            }
        }
        if level > 8 || (kind.is_none() && (level != 0 || restart)) {
            return Err(PyValueError::new_err(
                "list level must be 0..8; clearing a list requires level=0 and restart=false",
            ));
        }
        let kind = match kind {
            Some("bullet") => Some(ListKind::Bullet),
            Some("numbered") => Some(ListKind::Numbered),
            Some("decimal_outline") => Some(ListKind::Legal),
            None => None,
            _ => {
                return Err(PyValueError::new_err(
                    "list kind must be bullet, numbered, decimal_outline or None",
                ))
            }
        };
        if kind == Some(ListKind::Legal) {
            let (story, path) = self.location(id)?;
            if story != StoryRef::Body || path.0.len() != 1 {
                return Err(EngineError::Unsupported("decimal_outline is available only on top-level body paragraphs; cells and auxiliary stories are not supported".into()).into());
            }
        }
        let mut candidate = self.document()?.clone();
        let num = if let Some(kind) = kind {
            let (story, path) = self.location(id)?;
            let wanted = match kind {
                ListKind::Bullet => "bullet",
                ListKind::Numbered => "numbered",
                ListKind::Legal => "decimal_outline",
                _ => return Err(PyValueError::new_err("unsupported list kind")),
            };
            // Preserve the current sequence or continue the closest earlier matching list
            // in this story, including a restart. find_kind always returns the first one.
            let existing = if restart || start.is_some() {
                None
            } else {
                let mut found = None;
                let mut work = 0usize;
                let mut cache = BTreeMap::new();
                let definitions = list_definitions();
                for (visited, previous) in candidate
                    .para_paths(story)
                    .into_iter()
                    .take_while(|previous| previous <= &path)
                    .enumerate()
                {
                    if visited >= MAX_BLOCKS {
                        return Err(limit("list continuation exceeds 100000 paragraph visits"));
                    }
                    let Some(paragraph) = candidate.para(story, &previous) else {
                        continue;
                    };
                    let Some(reference) = candidate.styles.resolve_para(&paragraph.props).numbering
                    else {
                        continue;
                    };
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        cache.entry(reference.num)
                    {
                        entry.insert(supported_list_kind(
                            &candidate,
                            reference.num,
                            &definitions,
                            &mut work,
                        )?);
                    }
                    if cache[&reference.num] == Some(wanted) {
                        found = Some(reference.num);
                    }
                }
                found
            };
            if let Some(existing) = existing {
                existing
            } else {
                if candidate.numbering.nums.len() >= MAX_LISTS
                    || candidate.numbering.abstracts.len() >= MAX_LISTS
                {
                    return Err(limit("document exceeds 10000 native list definitions"));
                }
                let id = candidate
                    .numbering
                    .try_add_list(kind)
                    .map_err(model_error)?;
                if let Some(start) = start {
                    candidate
                        .numbering
                        .nums
                        .iter_mut()
                        .find(|number| number.id == id)
                        .ok_or_else(|| PyValueError::new_err("new list definition is missing"))?
                        .start_overrides
                        .push((0, start));
                }
                id
            }
        } else {
            0
        };
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        // NumRef 0 explicitly cancels any numbering inherited from the paragraph style.
        paragraph.props.numbering = Some(NumRef { num, level });
        paragraph.touch();
        if kind == Some(ListKind::Legal) {
            validate_unsplittable_page_bounds(&candidate)?;
        }
        self.commit(candidate)
    }

    pub fn set_paragraph_style(&mut self, id: usize, style: &str) -> PyResult<()> {
        self.paragraph(id)?;
        let style = self
            .document()?
            .styles
            .find(style)
            .filter(|value| value.kind == StyleKind::Paragraph)
            .ok_or_else(|| PyValueError::new_err("unknown paragraph style"))?
            .id
            .clone();
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        paragraph.props.style = Some(style);
        paragraph.touch();
        self.commit(candidate)?;
        Ok(())
    }

    pub fn paragraph_alignment(&self, id: usize) -> PyResult<&'static str> {
        let paragraph = self.paragraph(id)?;
        let alignment = self.document()?.styles.resolve_para(&paragraph.props).align;
        Ok(match alignment {
            Align::Left => "left",
            Align::Right => "right",
            Align::Center => "center",
            Align::Justify => "justify",
            Align::Distribute => "distribute",
        })
    }

    pub fn set_paragraph_alignment(&mut self, id: usize, alignment: &str) -> PyResult<()> {
        self.paragraph(id)?;
        let alignment = match alignment {
            "left" => Align::Left,
            "right" => Align::Right,
            "center" => Align::Center,
            "justify" => Align::Justify,
            "distribute" => Align::Distribute,
            _ => {
                return Err(PyValueError::new_err(
                    "alignment must be left, right, center, justify or distribute",
                ))
            }
        };
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        paragraph.props.align = Some(alignment);
        paragraph.touch();
        self.commit(candidate)?;
        Ok(())
    }

    pub fn page_break_before(&self, id: usize) -> PyResult<bool> {
        let paragraph = self.paragraph(id)?;
        Ok(self
            .document()?
            .styles
            .resolve_para(&paragraph.props)
            .page_break_before)
    }

    pub fn set_page_break_before(&mut self, id: usize, enabled: bool) -> PyResult<()> {
        self.paragraph(id)?;
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        paragraph.props.page_break_before = Some(enabled);
        paragraph.touch();
        self.commit(candidate)
    }

    pub fn add_page_break(&mut self) -> PyResult<usize> {
        let mut candidate = self.copy()?;
        let id = candidate.add_paragraph("")?;
        // A paragraph's page_break_before is ignored at the start of a document/page.
        // Use the upstream explicit break character so leading and consecutive breaks
        // retain their blank pages; the DOCX writer emits w:br with type="page".
        candidate.add_paragraph_break(id, "page")?;
        *self = candidate;
        Ok(id)
    }

    pub fn add_paragraph_field(&mut self, id: usize, kind: &str) -> PyResult<(usize, usize, u64)> {
        let paragraph = self.paragraph(id)?;
        let instruction = match kind {
            "page" => "PAGE \\* Arabic",
            "page_count" => "NUMPAGES \\* Arabic",
            "section_page_count" => "SECTIONPAGES \\* Arabic",
            "title" => "TITLE",
            "author" => "AUTHOR",
            _ => {
                return Err(PyValueError::new_err(
                    "field kind must be page, page_count, section_page_count, title or author",
                ))
            }
        };
        let (offset, start) = (paragraph.text.len(), paragraph.text.chars().count());
        let text_bytes = self.updated_text_bytes(0, wordcraft_doc::para::OBJ.len_utf8())?;
        if page_field_count(self.document()?)? >= MAX_PAGE_FIELDS {
            return Err(limit("document exceeds 10000 fields"));
        }
        let mut candidate = self.document()?.clone();
        // Explicit decimal avoids inconsistent section-format inheritance for page
        // fields. TITLE/AUTHOR use native core metadata. Empty cached results let
        // native layout evaluate the display without mutating the stored result.
        self.paragraph_mut(&mut candidate, id)?
            .insert_object(
                offset,
                InlineObject::Field {
                    instr: instruction.to_owned(),
                    result: String::new(),
                    locked: false,
                    dirty: true,
                    code: false,
                    form: String::new(),
                },
                &CharProps::default(),
            )
            .map_err(model_error)?;
        self.commit(candidate)?;
        self.text_bytes = text_bytes;
        Ok((start, start + 1, self.revision))
    }

    #[pyo3(signature = (id, kind="line"))]
    pub fn add_paragraph_break(&mut self, id: usize, kind: &str) -> PyResult<(usize, usize, u64)> {
        let paragraph = self.paragraph(id)?;
        let marker = match kind {
            "line" => wordcraft_doc::para::LINE_BREAK,
            "page" => wordcraft_doc::para::PAGE_BREAK,
            "column" => wordcraft_doc::para::COLUMN_BREAK,
            _ => {
                return Err(PyValueError::new_err(
                    "break kind must be line, page or column",
                ))
            }
        };
        let (story, path) = self.location(id)?;
        if kind != "line" && (story != StoryRef::Body || path.0.len() != 1) {
            return Err(EngineError::Unsupported(
                "page and column breaks require a top-level body paragraph".into(),
            )
            .into());
        }
        let (offset, start) = (paragraph.text.len(), paragraph.text.chars().count());
        let text = marker.to_string();
        let text_bytes = self.updated_text_bytes(0, text.len())?;
        let mut candidate = self.document()?.clone();
        candidate
            .insert_text(&self.position(id, offset)?, &text, &CharProps::default())
            .map_err(model_error)?;
        self.commit(candidate)?;
        self.text_bytes = text_bytes;
        Ok((start, start + 1, self.revision))
    }

    pub fn paragraph_spacing(&self, id: usize) -> PyResult<(f64, f64, f64)> {
        let paragraph = self.paragraph(id)?;
        let (story, path) = self.location(id)?;
        let table_style = path
            .cell()
            .and_then(|(table, _, _)| self.document().ok()?.table(story, &table))
            .and_then(|table| table.props.style.as_deref())
            .and_then(|style| self.document().ok()?.styles.table_style(style));
        let props = self.document()?.styles.resolve_para_in(
            &paragraph.props,
            table_style.as_ref().map(|style| &style.para),
        );
        let LineSpacing::Multiple(multiple) = props.line_spacing else {
            return Err(PyValueError::new_err("paragraph uses a point-based line spacing rule; assign spacing to use a line multiple"));
        };
        Ok((
            f64::from(props.space_before),
            f64::from(props.space_after),
            f64::from(multiple),
        ))
    }

    #[pyo3(signature = (id, before, after, line_spacing=1.0))]
    pub fn set_paragraph_spacing(
        &mut self,
        id: usize,
        before: f64,
        after: f64,
        line_spacing: f64,
    ) -> PyResult<()> {
        self.paragraph(id)?;
        if [before, after]
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1584.0).contains(value))
            || !line_spacing.is_finite()
            || !(0.06..=132.0).contains(&line_spacing)
        {
            return Err(PyValueError::new_err("paragraph spacing must be finite: before/after 0..1584 points and line multiple 0.06..132"));
        }
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        paragraph.props.space_before = Some(before as f32);
        paragraph.props.space_after = Some(after as f32);
        paragraph.props.space_before_auto = Some(false);
        paragraph.props.space_after_auto = Some(false);
        paragraph.props.line_spacing = Some(LineSpacing::Multiple(line_spacing as f32));
        paragraph.touch();
        self.commit(candidate)
    }

    #[pyo3(signature = (id, text, bold=false, italic=false, size=None, font=None, color=None))]
    #[allow(clippy::too_many_arguments)]
    pub fn add_run(
        &mut self,
        id: usize,
        text: &str,
        bold: bool,
        italic: bool,
        size: Option<f64>,
        font: Option<&str>,
        color: Option<&str>,
    ) -> PyResult<(usize, usize, u64)> {
        let paragraph = self.paragraph(id)?;
        validate_text(text)?;
        let text_bytes = self.updated_text_bytes(0, text.len())?;
        let offset = paragraph.text.len();
        let start = paragraph.text.chars().count();
        let props = run_props(Some(bold), Some(italic), size, font, color)?;
        let mut candidate = self.document()?.clone();
        candidate
            .insert_text(&self.position(id, offset)?, text, &props)
            .map_err(model_error)?;
        self.commit(candidate)?;
        self.text_bytes = text_bytes;
        Ok((start, start + text.chars().count(), self.revision))
    }

    pub fn text_range(&self, id: usize, start: usize, end: usize) -> PyResult<(String, u64)> {
        let paragraph = self.paragraph(id)?;
        let (from, to) = range_offsets(&paragraph.text, start, end)?;
        Ok((paragraph.text[from..to].into(), self.revision))
    }

    #[pyo3(signature = (id, text, patch="{}"))]
    pub fn add_run_patch(
        &mut self,
        id: usize,
        text: &str,
        patch: &str,
    ) -> PyResult<(usize, usize, u64)> {
        let paragraph = self.paragraph(id)?;
        validate_text(text)?;
        let formatting = CharPatch::parse(&self.document()?.styles, &format_patch(patch)?, true)?;
        let text_bytes = self.updated_text_bytes(0, text.len())?;
        let (offset, start) = (paragraph.text.len(), paragraph.text.chars().count());
        let mut props = CharProps::default();
        formatting.apply(&mut props);
        let mut candidate = self.document()?.clone();
        candidate
            .insert_text(&self.position(id, offset)?, text, &props)
            .map_err(model_error)?;
        self.commit(candidate)?;
        self.text_bytes = text_bytes;
        Ok((start, start + text.chars().count(), self.revision))
    }

    pub fn format_range_patch(
        &mut self,
        id: usize,
        start: usize,
        end: usize,
        revision: u64,
        patch: &str,
    ) -> PyResult<u64> {
        self.check_revision(revision)?;
        let (from, to) = range_offsets(&self.paragraph(id)?.text, start, end)?;
        if from == to {
            return Err(PyValueError::new_err("cannot format an empty text range"));
        }
        let formatting = CharPatch::parse(&self.document()?.styles, &format_patch(patch)?, true)?;
        let mut candidate = self.document()?.clone();
        candidate
            .format_range(
                &self.position(id, from)?,
                &self.position(id, to)?,
                &|props| formatting.apply(props),
            )
            .map_err(model_error)?;
        self.commit(candidate)?;
        Ok(self.revision)
    }

    pub fn range_text(
        &self,
        id: usize,
        start: usize,
        end: usize,
        revision: u64,
    ) -> PyResult<String> {
        self.check_revision(revision)?;
        self.text_range(id, start, end).map(|(text, _)| text)
    }

    pub fn bookmark_names(&self) -> PyResult<Vec<String>> {
        let document = self.document()?;
        validate_bookmark_links(document)?;
        Ok(document
            .bookmarks()
            .into_iter()
            .map(|(name, _)| name)
            .collect())
    }

    pub fn bookmark_range(&mut self, name: &str) -> PyResult<(usize, usize, usize, u64)> {
        let spans = validate_bookmark_links(self.document()?)?;
        let span = spans
            .get(name)
            .ok_or_else(|| PyKeyError::new_err(name.to_owned()))?;
        let paragraph = self
            .document()?
            .para(StoryRef::Body, &span.path)
            .ok_or_else(|| PyValueError::new_err("bookmark paragraph no longer exists"))?;
        let start = paragraph.text[..span.start + 3].chars().count();
        let end = paragraph.text[..span.end].chars().count();
        let id = self.register_location(StoryRef::Body, span.path.clone(), false)?;
        Ok((id, start, end, self.revision))
    }

    pub fn set_bookmark(
        &mut self,
        name: &str,
        id: usize,
        start: usize,
        end: usize,
        revision: u64,
    ) -> PyResult<()> {
        self.check_revision(revision)?;
        validate_bookmark_name(name)?;
        let document = self.document()?;
        let spans = validate_bookmark_links(document)?;
        let old = spans.get(name);
        if old.is_none() && spans.len() >= MAX_BOOKMARKS {
            return Err(limit("document exceeds 1000 bookmarks"));
        }
        let (story, path) = self.location(id)?;
        if story != StoryRef::Body {
            return Err(EngineError::Unsupported(
                "bookmark targets require a body paragraph, including supported body table cells"
                    .into(),
            )
            .into());
        }
        let paragraph = self.paragraph(id)?;
        let (mut from, mut to) = range_offsets(&paragraph.text, start, end)?;
        reject_semantic_range_except(paragraph, from, to, old.map(|_| name))?;
        if paragraph
            .object_offsets()
            .into_iter()
            .zip(&paragraph.objects)
            .any(|(offset, object)| {
                from <= offset && offset < to && matches!(object, InlineObject::Field { .. })
            })
        {
            return Err(EngineError::Unsupported(
                "bookmark selection cannot include evaluated field anchors".into(),
            )
            .into());
        }
        let text_bytes = self.updated_text_bytes(if old.is_some() { 6 } else { 0 }, 6)?;
        let mut candidate = document.clone();
        if let Some(old) = old {
            for offset in [old.end, old.start] {
                candidate
                    .delete_range(
                        &Pos::new(StoryRef::Body, old.path.clone(), offset),
                        &Pos::new(StoryRef::Body, old.path.clone(), offset + 3),
                    )
                    .map_err(model_error)?;
            }
            if old.path == path {
                from -= [old.start, old.end]
                    .iter()
                    .filter(|offset| **offset < from)
                    .count()
                    * 3;
                to -= [old.start, old.end]
                    .iter()
                    .filter(|offset| **offset < to)
                    .count()
                    * 3;
            }
        }
        candidate
            .insert_object(
                &Pos::new(story, path.clone(), to),
                InlineObject::BookmarkEnd { name: name.into() },
                &CharProps::default(),
            )
            .map_err(model_error)?;
        candidate
            .insert_object(
                &Pos::new(story, path, from),
                InlineObject::BookmarkStart { name: name.into() },
                &CharProps::default(),
            )
            .map_err(model_error)?;
        validate_unsplittable_page_bounds(&candidate)?;
        self.commit(candidate)?;
        self.text_bytes = text_bytes;
        Ok(())
    }

    pub fn delete_bookmark(&mut self, name: &str) -> PyResult<()> {
        let document = self.document()?;
        let spans = validate_bookmark_links(document)?;
        let span = spans
            .get(name)
            .ok_or_else(|| PyKeyError::new_err(name.to_owned()))?;
        let text_bytes = self.updated_text_bytes(6, 0)?;
        let mut candidate = document.clone();
        for offset in [span.end, span.start] {
            candidate
                .delete_range(
                    &Pos::new(StoryRef::Body, span.path.clone(), offset),
                    &Pos::new(StoryRef::Body, span.path.clone(), offset + 3),
                )
                .map_err(model_error)?;
        }
        // The central reference scan rejects deletion while any story still links here.
        self.commit_checked(candidate, Some(name))?;
        self.text_bytes = text_bytes;
        Ok(())
    }

    pub fn range_link(
        &self,
        id: usize,
        start: usize,
        end: usize,
        revision: u64,
    ) -> PyResult<Option<String>> {
        self.check_revision(revision)?;
        let paragraph = self.paragraph(id)?;
        let (from, to) = range_offsets(&paragraph.text, start, end)?;
        if from == to {
            return Err(PyValueError::new_err(
                "an empty text range has no hyperlink",
            ));
        }
        let target = paragraph.props_of_char(from).link.clone();
        if paragraph
            .run_ranges()
            .any(|(range, props)| range.start < to && range.end > from && props.link != target)
        {
            return Err(PyValueError::new_err(
                "text range contains mixed hyperlink targets",
            ));
        }
        Ok(target)
    }

    #[pyo3(signature = (id, start, end, revision, url=None))]
    pub fn set_range_link(
        &mut self,
        id: usize,
        start: usize,
        end: usize,
        revision: u64,
        url: Option<&str>,
    ) -> PyResult<u64> {
        self.check_revision(revision)?;
        let paragraph = self.paragraph(id)?;
        let (from, to) = range_offsets(&paragraph.text, start, end)?;
        if from == to {
            return Err(PyValueError::new_err("cannot link an empty text range"));
        }
        if let Some(url) = url {
            validate_text(url)?;
            if let Some(name) = url.strip_prefix('#') {
                validate_bookmark_name(name)?;
                if !validate_bookmark_links(self.document()?)?.contains_key(name) {
                    return Err(PyValueError::new_err(
                        "internal hyperlink requires an existing body bookmark",
                    ));
                }
            } else if url.len() > 8192
                || url.chars().any(|ch| ch.is_control() || ch.is_whitespace())
                || !wordcraft_doc::link::is_web_or_mail_link(url)
                || url
                    .split_once(':')
                    .is_none_or(|(_, target)| target.is_empty())
            {
                return Err(PyValueError::new_err("hyperlink must be an http, https or mailto URI of at most 8192 bytes without whitespace or controls"));
            }
        }
        // Account for a link target being copied when a formatted run is split, before the
        // upstream edit allocates those copies. Adjacent equal runs may later coalesce.
        let document = self.document()?;
        let mut bytes = hyperlink_bytes(document);
        for (range, props) in paragraph
            .run_ranges()
            .filter(|(range, _)| range.start < to && range.end > from)
        {
            let old = props.link.as_ref().map_or(0, String::len);
            bytes -= old;
            bytes += url.map_or(0, str::len)
                + usize::from(range.start < from) * old
                + usize::from(range.end > to) * old;
            if bytes > MAX_LINK_BYTES {
                return Err(limit("document hyperlink targets exceed 16 MiB"));
            }
        }
        let mut candidate = document.clone();
        candidate
            .format_range(
                &self.position(id, from)?,
                &self.position(id, to)?,
                &|props| {
                    if let Some(url) = url {
                        props.link = Some(url.into());
                        if props.style.is_none() {
                            props.style = Some("Hyperlink".into());
                        }
                    } else {
                        *props = props.unlinked();
                    }
                },
            )
            .map_err(model_error)?;
        if url.is_some_and(|target| target.starts_with('#')) {
            validate_unsplittable_page_bounds(&candidate)?;
        }
        self.commit(candidate)?;
        Ok(self.revision)
    }

    #[pyo3(signature = (id, start, end, revision, bold=None, italic=None, size=None, font=None, color=None))]
    #[allow(clippy::too_many_arguments)]
    pub fn format_range(
        &mut self,
        id: usize,
        start: usize,
        end: usize,
        revision: u64,
        bold: Option<bool>,
        italic: Option<bool>,
        size: Option<f64>,
        font: Option<&str>,
        color: Option<&str>,
    ) -> PyResult<u64> {
        self.check_revision(revision)?;
        let paragraph = self.paragraph(id)?;
        let (from, to) = range_offsets(&paragraph.text, start, end)?;
        if from == to {
            return Err(PyValueError::new_err("cannot format an empty text range"));
        }
        let formatting = run_props(bold, italic, size, font, color)?;
        let mut candidate = self.document()?.clone();
        candidate
            .format_range(
                &self.position(id, from)?,
                &self.position(id, to)?,
                &|props| {
                    if formatting.bold.is_some() {
                        props.bold = formatting.bold;
                    }
                    if formatting.italic.is_some() {
                        props.italic = formatting.italic;
                    }
                    if formatting.size.is_some() {
                        props.size = formatting.size;
                    }
                    if formatting.font.is_some() {
                        props.font.clone_from(&formatting.font);
                    }
                    if formatting.color.is_some() {
                        props.color = formatting.color;
                    }
                },
            )
            .map_err(model_error)?;
        self.commit(candidate)?;
        Ok(self.revision)
    }

    pub fn replace_range(
        &mut self,
        id: usize,
        start: usize,
        end: usize,
        revision: u64,
        text: &str,
    ) -> PyResult<(usize, usize, u64)> {
        self.check_revision(revision)?;
        let paragraph = self.paragraph(id)?;
        let (from, to) = range_offsets(&paragraph.text, start, end)?;
        reject_semantic_range(paragraph, from, to)?;
        validate_text(text)?;
        let text_bytes = self.updated_text_bytes(to - from, text.len())?;
        // Replacement takes the first selected character's properties. Surrounding runs
        // are retained by the upstream range-edit operation rather than reconstructed.
        let formatting = paragraph.props_of_char(from).clone();
        let mut candidate = self.document()?.clone();
        let position = candidate
            .delete_range(&self.position(id, from)?, &self.position(id, to)?)
            .map_err(model_error)?;
        candidate
            .insert_text(&position, text, &formatting)
            .map_err(model_error)?;
        self.commit(candidate)?;
        self.text_bytes = text_bytes;
        Ok((start, start + text.chars().count(), self.revision))
    }

    /// Attach a note after a revision-bound range without replacing its label.
    /// Public offsets count Unicode scalars, including each native OBJ as one scalar.
    #[pyo3(signature = (paragraph, start, end, revision, text, kind="footnote"))]
    #[allow(clippy::too_many_arguments)]
    pub fn add_note(
        &mut self,
        paragraph: usize,
        start: usize,
        end: usize,
        revision: u64,
        text: &str,
        kind: &str,
    ) -> PyResult<u32> {
        self.check_revision(revision)?;
        let (part_kind, note_kind, style, reference_style) = note_kind_values(kind)?;
        let source = self.paragraph(paragraph)?;
        let (_, offset) = range_offsets(&source.text, start, end)?;
        let at = self.position(paragraph, offset)?;
        if at.story != StoryRef::Body {
            return Err(PyValueError::new_err(
                "notes require a body paragraph or body table anchor",
            ));
        }
        validate_text(text)?;
        self.reserve_block()?;
        if self.note_ids(None)?.len() >= MAX_NOTES {
            return Err(limit("document exceeds 10000 notes"));
        }
        let added_bytes = text
            .len()
            .checked_add(wordcraft_doc::para::OBJ.len_utf8())
            .ok_or_else(|| limit("note text size overflow"))?;
        let text_bytes = self.updated_text_bytes(0, added_bytes)?;
        let mut staged = self.copy()?;
        let mut candidate = staged.document()?.clone();
        // The native writer supplies the note's own mark in its first paragraph.
        // Keeping it out of the editable story permits ordinary paragraph replacement;
        // serialized rendering/conversion reopens the native mark as usual.
        let body = Paragraph::with_text(text, CharProps::default()).styled(style);
        let note = candidate
            .try_add_part(part_kind, vec![wordcraft_doc::para_block(body)])
            .map_err(model_error)?;
        candidate
            .insert_object(
                &at,
                InlineObject::NoteRef {
                    kind: note_kind,
                    id: note,
                    custom: String::new(),
                },
                &CharProps {
                    style: Some(reference_style.into()),
                    ..Default::default()
                },
            )
            .map_err(model_error)?;
        staged.commit(candidate)?;
        let child = staged.register_location(StoryRef::Part(note), Path::top(0), false)?;
        staged.stories.insert(note, vec![child]);
        staged.text_bytes = text_bytes;
        *self = staged;
        Ok(note)
    }

    /// Convert the owned note collection through the native semantic operation.
    /// Stable part/path identities survive; an empty conversion has no revision effect.
    pub fn convert_notes(&mut self, kind: &str) -> PyResult<usize> {
        let original = self.document()?;
        let (destination, target, _, _) = note_kind_values(kind)?;
        let changed = original
            .parts
            .values()
            .filter(|part| {
                matches!(part.kind, PartKind::Footnote | PartKind::Endnote)
                    && part.kind != destination
            })
            .count();
        if changed == 0 {
            return Ok(0);
        }
        validate_note_conversion_work(original, target)?;
        let mut candidate = original.clone();
        let actual =
            candidate.convert_notes(target == NoteKind::Endnote, target == NoteKind::Footnote);
        if actual != changed {
            return Err(EngineError::Unsupported(
                "native note conversion changed an unexpected number of notes".into(),
            )
            .into());
        }
        self.commit_with_note_conversion(candidate, None, Some(target))?;
        Ok(changed)
    }

    #[pyo3(signature = (kind=None))]
    pub fn note_ids(&self, kind: Option<&str>) -> PyResult<Vec<u32>> {
        let filter = kind.map(note_kind_values).transpose()?.map(|value| value.0);
        Ok(self
            .document()?
            .parts
            .iter()
            .filter_map(|(id, part)| {
                (matches!(part.kind, PartKind::Footnote | PartKind::Endnote)
                    && filter.is_none_or(|kind| part.kind == kind))
                .then_some(*id)
            })
            .collect())
    }

    pub fn note_info(&self, note: u32) -> PyResult<String> {
        let part = self
            .document()?
            .parts
            .get(&note)
            .filter(|part| matches!(part.kind, PartKind::Footnote | PartKind::Endnote))
            .ok_or_else(|| PyIndexError::new_err("unknown note identifier"))?;
        Ok(json!({"kind": if part.kind == PartKind::Footnote { "footnote" } else { "endnote" }, "story": note}).to_string())
    }

    pub fn section_note_options(&self, section: usize, kind: &str) -> PyResult<String> {
        let (_, note, _, _) = note_kind_values(kind)?;
        let options = self
            .document()?
            .note_options(self.section(section)?, note == NoteKind::Endnote);
        Ok(json!({
            "placement": note_placement_name(options.pos),
            "numbering": note_numbering_name(options.num_fmt)?,
            "start": options.num_start,
            "restart": note_restart_name(options.num_restart),
        })
        .to_string())
    }

    pub fn set_section_note_options(
        &mut self,
        section: usize,
        kind: &str,
        patch: &str,
    ) -> PyResult<()> {
        let (_, note, _, _) = note_kind_values(kind)?;
        let endnote = note == NoteKind::Endnote;
        let mut properties = self.section(section)?.clone();
        if patch.len() > MAX_FORMAT_PATCH_BYTES {
            return Err(limit("note options patch exceeds 16 KiB"));
        }
        let value: Value = serde_json::from_str(patch)
            .map_err(|_| PyValueError::new_err("note options require a JSON object"))?;
        let values = value
            .as_object()
            .ok_or_else(|| PyValueError::new_err("note options require a JSON object"))?;
        let options = if endnote {
            &mut properties.endnote_pr
        } else {
            &mut properties.footnote_pr
        };
        for (key, value) in values {
            match key.as_str() {
                "placement" => {
                    options.pos = if value.is_null() {
                        None
                    } else {
                        let placement = match value.as_str() {
                        Some("page_bottom") if !endnote => NotePos::PageBottom,
                        Some("beneath_text") if !endnote => NotePos::BeneathText,
                        Some("section_end") if endnote => NotePos::SectEnd,
                        Some("document_end") if endnote => NotePos::DocEnd,
                        _ => return Err(PyValueError::new_err("placement must match the note kind: page_bottom/beneath_text or section_end/document_end")),
                    };
                        Some(placement)
                    }
                }
                "numbering" => {
                    options.num_fmt = if value.is_null() {
                        None
                    } else {
                        Some(parse_note_numbering(value.as_str().unwrap_or(""))?)
                    }
                }
                "start" => {
                    options.num_start = if value.is_null() {
                        None
                    } else {
                        Some(
                            value
                                .as_u64()
                                .filter(|n| {
                                    (1..=u64::from(wordcraft_doc::section::MAX_NOTE_START))
                                        .contains(n)
                                })
                                .ok_or_else(|| {
                                    PyValueError::new_err(
                                        "note start must be an integer from 1 to 32767",
                                    )
                                })? as u32,
                        )
                    }
                }
                "restart" => {
                    options.num_restart = if value.is_null() {
                        None
                    } else {
                        Some(match value.as_str() {
                        Some("continuous") => NoteRestart::Continuous,
                        Some("each_section") => NoteRestart::EachSect,
                        Some("each_page") if !endnote => NoteRestart::EachPage,
                        _ => return Err(PyValueError::new_err("restart must be continuous, each_section, or each_page for footnotes")),
                    })
                    }
                }
                _ => return Err(PyValueError::new_err(format!("unknown note option: {key}"))),
            }
        }
        let mut candidate = self.document()?.clone();
        let end = candidate.sections()[section].0;
        *candidate.section_mut(end) = properties;
        self.commit(candidate)
    }

    pub fn section_ids(&self) -> PyResult<Vec<usize>> {
        Ok((0..self.document()?.sections().len()).collect())
    }

    pub fn section_props(&self, section: usize) -> PyResult<String> {
        let props = self.section(section)?;
        Ok(json!({
            "start": section_start_name(props.start),
            "page_number_start": props.page_num_start,
            "line_numbers": props.line_numbers.as_ref().map(|value| json!([value.count_by, value.start, value.distance, line_number_restart_name(value.restart)])),
            "page_size": [props.page_w, props.page_h],
            "margins": [props.margin_top, props.margin_right, props.margin_bottom, props.margin_left],
            "gutter": props.gutter,
            "distances": [props.header, props.footer],
            "columns": [props.columns.count, props.columns.space, props.columns.separator],
            "column_widths": if props.columns.widths.is_empty() { None } else {
                Some(props.columns.widths.iter().map(|(width, _)| *width).collect::<Vec<_>>())
            }
        }).to_string())
    }

    pub fn set_section_props(&mut self, section: usize, patch: &str) -> PyResult<()> {
        let mut props = self.section(section)?.clone();
        if patch.len() > MAX_FORMAT_PATCH_BYTES {
            return Err(limit("section patch exceeds 16 KiB"));
        }
        let value: Value = serde_json::from_str(patch)
            .map_err(|_| PyValueError::new_err("section properties require a JSON object"))?;
        let object = value
            .as_object()
            .ok_or_else(|| PyValueError::new_err("section properties require a JSON object"))?;
        for (key, value) in object {
            match key.as_str() {
                "start" => {
                    props.start = parse_section_start(value.as_str().unwrap_or(""))?;
                    if section == 0 && props.start != SectionStart::NextPage {
                        return Err(PyValueError::new_err(
                            "the first section must start with next_page",
                        ));
                    }
                }
                "line_numbers" => {
                    props.line_numbers = if value.is_null() {
                        None
                    } else {
                        let values = value.as_array().filter(|values| values.len() == 4).ok_or_else(|| PyValueError::new_err("line_numbers requires (count_by, start, distance, restart), or null"))?;
                        let count_by = values[0]
                            .as_u64()
                            .filter(|value| (1..=100).contains(value))
                            .ok_or_else(|| {
                                PyValueError::new_err(
                                    "line-number count_by must be an integer from 1 to 100",
                                )
                            })? as u32;
                        let start = values[1]
                            .as_u64()
                            .filter(|value| (1..=32768).contains(value))
                            .ok_or_else(|| {
                                PyValueError::new_err(
                                    "line-number start must be an integer from 1 to 32768",
                                )
                            })? as u32;
                        let distance = values[2].as_f64().filter(|value| value.is_finite() && (0.0..=1584.0).contains(value))
                            .ok_or_else(|| PyValueError::new_err("line-number distance must be finite and between 0 and 1584 points"))? as f32;
                        let restart = match values[3].as_str() {
                            Some("each_page") => LineNumberRestart::Page,
                            Some("each_section") => LineNumberRestart::Section,
                            Some("continuous") => LineNumberRestart::Continuous,
                            _ => return Err(PyValueError::new_err(
                                "line-number restart must be each_page, each_section or continuous",
                            )),
                        };
                        Some(LineNumbering {
                            count_by,
                            start,
                            distance,
                            restart,
                        })
                    };
                }
                "page_number_start" => {
                    props.page_num_start = if value.is_null() {
                        None
                    } else {
                        Some(value.as_u64().filter(|value| (1..=32767).contains(value))
                            .ok_or_else(|| PyValueError::new_err("page_number_start must be an integer from 1 to 32767, or null"))? as u32)
                    };
                }
                "page_size" => {
                    let [width, height] = section_numbers::<2>(value, key)?;
                    props.page_w = width;
                    props.page_h = height;
                    props.landscape = width > height;
                }
                "margins" => {
                    let [top, right, bottom, left] = section_numbers::<4>(value, key)?;
                    props.margin_top = top;
                    props.margin_right = right;
                    props.margin_bottom = bottom;
                    props.margin_left = left;
                }
                "distances" => {
                    let [header, footer] = section_numbers::<2>(value, key)?;
                    props.header = header;
                    props.footer = footer;
                }
                "gutter" => {
                    props.gutter = value
                        .as_f64()
                        .filter(|number| number.is_finite() && *number >= 0.0 && *number <= 1584.0)
                        .ok_or_else(|| {
                            PyValueError::new_err(
                                "gutter must be finite and between 0 and 1584 points",
                            )
                        })? as f32;
                }
                // Apply explicit widths after columns, independent of JSON key order.
                "column_widths" => {}
                "columns" => {
                    let values = value.as_array().filter(|v| v.len() == 3).ok_or_else(|| {
                        PyValueError::new_err("columns must contain count, gap and separator")
                    })?;
                    let count = values[0]
                        .as_u64()
                        .filter(|n| (1..=45).contains(n))
                        .ok_or_else(|| {
                            PyValueError::new_err("column count must be an integer from 1 to 45")
                        })?;
                    let gap = values[1]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1584.0)
                        .ok_or_else(|| {
                            PyValueError::new_err(
                                "column gap must be finite and between 0 and 1584 points",
                            )
                        })?;
                    let separator = values[2]
                        .as_bool()
                        .ok_or_else(|| PyValueError::new_err("column separator must be boolean"))?;
                    props.columns.count = count as u32;
                    props.columns.space = gap as f32;
                    props.columns.separator = separator;
                    props.columns.widths.clear();
                }
                _ => {
                    return Err(PyValueError::new_err(format!(
                        "unknown section property: {key}"
                    )))
                }
            }
        }
        if let Some(value) = object.get("column_widths") {
            props.columns.widths = if value.is_null() {
                Vec::new()
            } else {
                let widths = value.as_array().filter(|widths| widths.len() == props.columns.count as usize)
                    .ok_or_else(|| PyValueError::new_err("column_widths must contain one width for each configured column, or null"))?;
                widths
                    .iter()
                    .enumerate()
                    .map(|(index, width)| {
                        let width = width
                            .as_f64()
                            .filter(|width| width.is_finite() && *width >= 18.0 && *width <= 1584.0)
                            .ok_or_else(|| {
                                PyValueError::new_err(
                                    "column widths must be finite and between 18 and 1584 points",
                                )
                            })?;
                        let gap = if index + 1 == widths.len() {
                            0.0
                        } else {
                            props.columns.space
                        };
                        Ok((width as f32, gap))
                    })
                    .collect::<PyResult<Vec<_>>>()?
            };
        }
        if props.start == SectionStart::Continuous && props.page_num_start.is_some() {
            return Err(EngineError::Unsupported("page numbering restart is not supported in continuous sections at this native revision".into()).into());
        }
        validate_section_geometry(&props)?;
        let mut candidate = self.document()?.clone();
        let end = candidate.sections()[section].0;
        *candidate.section_mut(end) = props;
        validate_section_content(&candidate, section)?;
        self.commit(candidate)
    }

    #[pyo3(signature = (start="next_page"))]
    pub fn add_section(&mut self, start: &str) -> PyResult<usize> {
        let start = parse_section_start(start)?;
        let section = self.document()?.sections().len();
        if section >= MAX_SECTIONS {
            return Err(limit("document exceeds 128 sections"));
        }
        if self.document()?.body.len() >= MAX_BLOCKS {
            return Err(limit("document exceeds 100000 body blocks"));
        }
        // Stage model and locator maps together; allocation/validation failures publish nothing.
        let mut staged = self.copy()?;
        let mut candidate = self.document()?.clone();
        let previous = candidate.last_section.clone();
        let mut next = previous.clone();
        next.start = start;
        next.page_num_start = None;
        let mut copied = BTreeMap::new();
        for (kind, refs) in [
            (PartKind::Header, &mut next.headers),
            (PartKind::Footer, &mut next.footers),
        ] {
            for slot in [&mut refs.default, &mut refs.first, &mut refs.even] {
                let old = *slot;
                let new = if let Some(id) = old.and_then(|id| copied.get(&id).copied()) {
                    id
                } else {
                    let blocks = old
                        .and_then(|id| candidate.parts.get(&id))
                        .map(|part| part.blocks.clone())
                        .unwrap_or_default();
                    let (bytes, cells) = content_usage(&blocks);
                    staged.text_bytes = staged.updated_text_bytes(0, bytes)?;
                    staged.cell_count = staged.cell_count.saturating_add(cells);
                    if staged.cell_count > MAX_CELLS {
                        return Err(limit("document exceeds 100000 table cells"));
                    }
                    let id = candidate.try_add_part(kind, blocks).map_err(model_error)?;
                    if let Some(old) = old {
                        copied.insert(old, id);
                        let old_part = candidate
                            .parts
                            .get(&old)
                            .ok_or_else(|| PyValueError::new_err("missing section story"))?;
                        let body = old_part.body;
                        if let Some(part) = candidate.parts.get_mut(&id) {
                            part.body = body;
                        }
                        let mut ids = Vec::new();
                        for old_id in self.stories.get(&old).into_iter().flatten() {
                            let (_, path) = self.location(*old_id)?;
                            ids.push(staged.register_location(StoryRef::Part(id), path, false)?);
                        }
                        staged.stories.insert(id, ids);
                        if let Some(index) = self.story_placeholders.get(&old) {
                            staged.story_placeholders.insert(id, *index);
                        }
                        for (old_id, (story, path)) in &self.locations {
                            if *story == StoryRef::Part(old) {
                                staged.register_location(
                                    StoryRef::Part(id),
                                    path.clone(),
                                    self.images.contains(old_id),
                                )?;
                            }
                        }
                        for (story, path) in &self.cell_placeholders {
                            if *story == StoryRef::Part(old) {
                                staged
                                    .cell_placeholders
                                    .insert((StoryRef::Part(id), path.clone()));
                            }
                        }
                    } else {
                        // An omitted reference links to the previous section in OOXML. An owned
                        // empty part is required to keep future edits to that section independent.
                        staged.stories.insert(id, Vec::new());
                        staged.story_placeholders.insert(id, 0);
                    }
                    id
                };
                *slot = Some(new);
            }
        }
        let end = candidate
            .body
            .len()
            .checked_sub(1)
            .ok_or_else(|| PyValueError::new_err("document body is empty"))?;
        let paragraph = candidate
            .para_mut(StoryRef::Body, &Path::top(end))
            .map_err(model_error)?;
        paragraph.section = Some(Box::new(previous));
        candidate.last_section = next;
        let placeholder = candidate.body.len();
        candidate
            .body
            .push(wordcraft_doc::para_block(Paragraph::new()));
        staged.body_placeholder = Some(placeholder);
        staged.commit(candidate)?;
        *self = staged;
        Ok(section)
    }

    pub fn page_size(&self) -> PyResult<(f64, f64)> {
        let section = self.section(0)?;
        Ok((f64::from(section.page_w), f64::from(section.page_h)))
    }

    pub fn margins(&self) -> PyResult<(f64, f64, f64, f64)> {
        let section = self.section(0)?;
        Ok((
            f64::from(section.margin_top),
            f64::from(section.margin_right),
            f64::from(section.margin_bottom),
            f64::from(section.margin_left),
        ))
    }

    pub fn set_margins(&mut self, top: f64, right: f64, bottom: f64, left: f64) -> PyResult<()> {
        if [top, right, bottom, left].iter().any(|n| !n.is_finite()) {
            return Err(PyValueError::new_err("margins must be finite"));
        }
        self.set_section_props(
            0,
            &json!({"margins": [top, right, bottom, left]}).to_string(),
        )
    }

    #[pyo3(signature = (data, width=None, height=None, alt=""))]
    pub fn add_image(
        &mut self,
        data: &[u8],
        width: Option<f64>,
        height: Option<f64>,
        alt: &str,
    ) -> PyResult<usize> {
        self.reserve_block()?;
        let mut paragraph = self.make_image(data, width, height, alt)?;
        validate_picture_width(&paragraph, column_width(&self.document()?.last_section))?;
        let (id, next) = self.next_identifier()?;
        let mut candidate = self.document()?.clone();
        if candidate.body.len() >= MAX_BLOCKS {
            return Err(limit("document exceeds 100000 blocks"));
        }
        attach_image_media(&mut candidate, &mut paragraph, data.to_vec())?;
        let index = self.body_placeholder.unwrap_or(candidate.body.len());
        if self.body_placeholder.is_some() {
            *candidate
                .body
                .get_mut(index)
                .ok_or_else(|| PyValueError::new_err("body placeholder no longer exists"))? =
                wordcraft_doc::para_block(paragraph);
        } else {
            candidate
                .insert_block(StoryRef::Body, &Path::top(index), Block::Para(paragraph))
                .map_err(model_error)?;
        }
        self.commit(candidate)?;
        self.next_block_id = next;
        self.body_placeholder = None;
        self.images.push(id);
        self.locations
            .insert(id, (StoryRef::Body, Path::top(index)));
        Ok(id)
    }

    pub fn image_data(&self, id: usize) -> PyResult<Vec<u8>> {
        let InlineObject::Image { media, .. } = self.image(id)? else {
            return Err(PyIndexError::new_err(
                "image index does not identify a picture",
            ));
        };
        self.document()?
            .media
            .get(media)
            .map(|bytes| bytes.as_ref().clone())
            .ok_or_else(|| PyValueError::new_err("picture references missing native media"))
    }

    pub fn set_image_data(&mut self, id: usize, data: &[u8]) -> PyResult<()> {
        self.image(id)?;
        validate_png(data)?;
        let mut candidate = self.document()?.clone();
        let key = candidate.add_media(data.to_vec(), "png");
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        let Some(InlineObject::Image { media, .. }) = paragraph.objects.first_mut() else {
            return Err(PyIndexError::new_err(
                "image index does not identify a picture",
            ));
        };
        *media = key;
        paragraph.touch();
        self.commit(candidate)
    }

    pub fn remove_image(&mut self, id: usize) -> PyResult<()> {
        self.image(id)?;
        let (story, path) = self.location(id)?;
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        // Authored pictures occupy one native inline-object paragraph. Removing
        // the object keeps its anchor and every unrelated stable block locator.
        if paragraph.text != "\u{fffc}" || paragraph.objects.len() != 1 {
            return Err(EngineError::Unsupported(
                "image removal requires an authored inline-picture paragraph".into(),
            )
            .into());
        }
        paragraph
            .delete(0, '\u{fffc}'.len_utf8())
            .map_err(model_error)?;
        self.commit(candidate)?;
        self.images.retain(|image| *image != id);
        if let StoryRef::Part(story) = story {
            if path.0.len() == 1 {
                if let Some(ids) = self.stories.get_mut(&story) {
                    // The retained empty anchor is now an ordinary direct paragraph.
                    // Keep its stable locator and its position among story paragraphs.
                    if !ids.contains(&id) {
                        let index = ids.partition_point(|other| {
                            self.locations
                                .get(other)
                                .is_some_and(|(_, other_path)| other_path < &path)
                        });
                        ids.insert(index, id);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn image_ids(&mut self) -> PyResult<Vec<usize>> {
        let document = self.document()?;
        let targets = std::iter::once(StoryRef::Body)
            .chain(document.parts.keys().map(|id| StoryRef::Part(*id)))
            .flat_map(|story| {
                document
                    .para_paths(story)
                    .into_iter()
                    .filter_map(move |path| {
                        document
                            .para(story, &path)
                            .filter(|paragraph| {
                                paragraph
                                    .objects
                                    .iter()
                                    .any(|object| matches!(object, InlineObject::Image { .. }))
                            })
                            .map(|_| (story, path))
                    })
            })
            .collect::<Vec<_>>();
        targets
            .into_iter()
            .map(|(story, path)| self.register_location(story, path, true))
            .collect()
    }

    pub fn image_size(&self, id: usize) -> PyResult<(f64, f64)> {
        match self.image(id)? {
            InlineObject::Image { w, h, .. } => Ok((f64::from(*w), f64::from(*h))),
            _ => Err(PyIndexError::new_err(
                "image index does not identify a picture",
            )),
        }
    }

    pub fn image_crop(&self, id: usize) -> PyResult<(f64, f64, f64, f64)> {
        let InlineObject::Image { crop, .. } = self.image(id)? else {
            return Err(PyIndexError::new_err(
                "image index does not identify a picture",
            ));
        };
        Ok((
            f64::from(crop[0]),
            f64::from(crop[1]),
            f64::from(crop[2]),
            f64::from(crop[3]),
        ))
    }

    pub fn set_image_crop(&mut self, id: usize, crop: (f64, f64, f64, f64)) -> PyResult<()> {
        self.image(id)?;
        let values = [crop.0, crop.1, crop.2, crop.3];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=0.45).contains(value))
        {
            return Err(PyValueError::new_err("crop fractions must be finite and between 0 and 0.45 in left/top/right/bottom order"));
        }
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        if let Some(InlineObject::Image { crop, .. }) = paragraph.objects.first_mut() {
            *crop = values.map(|value| value as f32);
        }
        paragraph.touch();
        self.commit(candidate)
    }

    pub fn image_flips(&self, id: usize) -> PyResult<(bool, bool)> {
        let InlineObject::Image { float, .. } = self.image(id)? else {
            return Err(PyIndexError::new_err(
                "image index does not identify a picture",
            ));
        };
        Ok((float.flip_h, float.flip_v))
    }

    #[pyo3(signature = (id, flip_h=None, flip_v=None))]
    pub fn set_image_flips(
        &mut self,
        id: usize,
        flip_h: Option<bool>,
        flip_v: Option<bool>,
    ) -> PyResult<()> {
        self.image(id)?;
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        if let Some(InlineObject::Image { float, .. }) = paragraph.objects.first_mut() {
            if let Some(value) = flip_h {
                float.flip_h = value;
            }
            if let Some(value) = flip_v {
                float.flip_v = value;
            }
        }
        paragraph.touch();
        self.commit(candidate)
    }

    pub fn set_image_size(&mut self, id: usize, width: f64, height: f64) -> PyResult<()> {
        self.image(id)?;
        validate_image_size(width, height)?;
        if width > f64::from(self.container_width(id)?) + 0.001 {
            return Err(PyValueError::new_err(
                "picture width exceeds its section or cell width",
            ));
        }
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        if let Some(InlineObject::Image { w, h, .. }) = paragraph.objects.first_mut() {
            *w = width as f32;
            *h = height as f32;
        }
        paragraph.touch();
        self.commit(candidate)
    }

    pub fn image_alt(&self, id: usize) -> PyResult<String> {
        match self.image(id)? {
            InlineObject::Image { alt, .. } => Ok(alt.clone()),
            _ => Err(PyIndexError::new_err(
                "image index does not identify a picture",
            )),
        }
    }

    pub fn set_image_alt(&mut self, id: usize, alt: &str) -> PyResult<()> {
        self.image(id)?;
        validate_alt(alt)?;
        let mut candidate = self.document()?.clone();
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        if let Some(InlineObject::Image { alt: target, .. }) = paragraph.objects.first_mut() {
            *target = alt.into();
        }
        paragraph.touch();
        self.commit(candidate)
    }

    pub fn set_page_size(&mut self, width: f64, height: f64) -> PyResult<()> {
        if !width.is_finite() || !height.is_finite() {
            return Err(PyValueError::new_err("page size must be finite"));
        }
        self.set_section_props(0, &json!({"page_size": [width, height]}).to_string())
    }

    pub fn background(&self) -> PyResult<Option<String>> {
        Ok(self
            .document()?
            .settings
            .page_color
            .map(|color| format!("#{:02X}{:02X}{:02X}", color.0, color.1, color.2)))
    }

    pub fn background_image(&self) -> PyResult<Option<Vec<u8>>> {
        let document = self.document()?;
        document
            .settings
            .page_picture
            .as_ref()
            .map(|picture| {
                document
                    .media
                    .get(&picture.media)
                    .map(|bytes| bytes.as_ref().clone())
                    .ok_or_else(|| {
                        PyValueError::new_err("background image references missing native media")
                    })
            })
            .transpose()
    }

    #[pyo3(signature = (data=None))]
    pub fn set_background_image(&mut self, data: Option<&[u8]>) -> PyResult<()> {
        self.document()?;
        if let Some(data) = data {
            validate_png(data)?;
        }
        let mut candidate = self.document()?.clone();
        candidate.settings.page_picture = data.map(|bytes| wordcraft_doc::PagePicture {
            media: candidate.add_media(bytes.to_vec(), "png"),
            fill_type: "frame".into(),
            title: String::new(),
        });
        self.commit(candidate)
    }

    #[pyo3(signature = (color=None))]
    pub fn set_background(&mut self, color: Option<&str>) -> PyResult<()> {
        self.document()?;
        let color = color.map(parse_rgb).transpose()?;
        let mut candidate = self.document()?.clone();
        candidate.settings.page_color = color;
        self.commit(candidate)?;
        Ok(())
    }

    #[pyo3(signature = (kind, variant="default", section=0))]
    pub fn story_id(&self, kind: &str, variant: &str, section: usize) -> PyResult<Option<u32>> {
        self.document()?;
        let refs = match kind {
            "header" => self.section(section)?.headers,
            "footer" => self.section(section)?.footers,
            _ => return Err(PyValueError::new_err("story kind must be header or footer")),
        };
        match variant {
            "default" => Ok(refs.default),
            "first" => Ok(refs.first),
            "even" => Ok(refs.even),
            _ => Err(PyValueError::new_err(
                "story variant must be default, first or even",
            )),
        }
    }

    #[pyo3(signature = (kind, variant="default", section=0))]
    pub fn ensure_story(&mut self, kind: &str, variant: &str, section: usize) -> PyResult<u32> {
        let document = self.document()?;
        let (part_kind, refs) = match kind {
            "header" => (PartKind::Header, self.section(section)?.headers),
            "footer" => (PartKind::Footer, self.section(section)?.footers),
            _ => return Err(PyValueError::new_err("story kind must be header or footer")),
        };
        let existing = match variant {
            "default" => refs.default,
            "first" => refs.first,
            "even" => refs.even,
            _ => {
                return Err(PyValueError::new_err(
                    "story variant must be default, first or even",
                ))
            }
        };
        let mut candidate = document.clone();
        let id = if let Some(id) = existing {
            self.story_paragraph_ids(id)?;
            if variant == "default"
                || (variant == "first" && self.section(section)?.title_page)
                || (variant == "even" && document.settings.even_odd_headers)
            {
                return Ok(id);
            }
            id
        } else {
            candidate
                .try_add_part(part_kind, Vec::new())
                .map_err(model_error)?
        };
        let end = candidate.sections()[section].0;
        let properties = candidate.section_mut(end);
        let refs = if part_kind == PartKind::Header {
            &mut properties.headers
        } else {
            &mut properties.footers
        };
        match variant {
            "default" => refs.default = Some(id),
            "first" => {
                refs.first = Some(id);
                candidate.section_mut(end).title_page = true;
            }
            "even" => {
                refs.even = Some(id);
                candidate.settings.even_odd_headers = true;
            }
            _ => return Err(PyValueError::new_err("invalid story variant")),
        }
        self.commit(candidate)?;
        self.stories.entry(id).or_default();
        if existing.is_none() {
            self.story_placeholders.insert(id, 0);
        }
        Ok(id)
    }

    /// Clear only an independently owned header/footer. False preserves revisions/caches.
    #[pyo3(signature = (kind, variant="default", section=0))]
    pub fn clear_story(&mut self, kind: &str, variant: &str, section: usize) -> PyResult<bool> {
        let Some(id) = self.story_id(kind, variant, section)? else {
            return Ok(false);
        };
        self.story_paragraph_ids(id)?;
        let document = self.document()?;
        let part = document
            .parts
            .get(&id)
            .ok_or_else(|| PyValueError::new_err("owned story part is missing"))?;
        let expected = if kind == "header" {
            PartKind::Header
        } else {
            PartKind::Footer
        };
        if part.kind != expected {
            return Err(EngineError::Unsupported(
                "clear requires an owned header/footer part".into(),
            )
            .into());
        }
        let references = document
            .sections()
            .iter()
            .flat_map(|(_, section)| {
                [
                    section.headers.default,
                    section.headers.first,
                    section.headers.even,
                    section.footers.default,
                    section.footers.first,
                    section.footers.even,
                ]
            })
            .filter(|reference| *reference == Some(id))
            .count();
        if references != 1 {
            return Err(EngineError::Unsupported(
                "clearing shared header/footer parts requires explicit linkage ownership support"
                    .into(),
            )
            .into());
        }
        if self.story_placeholders.get(&id) == Some(&0)
            && (part.blocks.is_empty()
                || (part.blocks.len() == 1 && *part.blocks[0] == Block::Para(Paragraph::new())))
        {
            return Ok(false);
        }
        self.clear_owned_story_content(id, Paragraph::new())
    }

    /// Clear an owned note's content, preserving its external reference and native kind.
    pub fn clear_note(&mut self, note: u32) -> PyResult<bool> {
        self.story_paragraph_ids(note)?;
        let part = self
            .document()?
            .parts
            .get(&note)
            .ok_or_else(|| PyIndexError::new_err("owned note part is missing"))?;
        let style = match part.kind {
            PartKind::Footnote => "FootnoteText",
            PartKind::Endnote => "EndnoteText",
            _ => return Err(PyValueError::new_err("story is not an owned note")),
        };
        let replacement = Paragraph::new().styled(style);
        if self.story_placeholders.get(&note) == Some(&0)
            && part.blocks.len() == 1
            && *part.blocks[0] == Block::Para(replacement.clone())
        {
            return Ok(false);
        }
        self.clear_owned_story_content(note, replacement)
    }

    /// Remove one owned direct paragraph while preserving native section boundaries.
    pub fn remove_paragraph(&mut self, id: usize) -> PyResult<()> {
        let paragraph = self.paragraph(id)?;
        let (story, path) = self.location(id)?;
        if path.0.len() != 1 {
            return Err(EngineError::Unsupported(
                "cell paragraph deletion is not supported".into(),
            )
            .into());
        }
        if paragraph.section.is_some() {
            return Err(EngineError::Unsupported(
                "paragraph deletion cannot remove section boundaries".into(),
            )
            .into());
        }
        let document = self.document()?;
        let (blocks, ids, part_kind, placeholder) = match story {
            StoryRef::Body => (
                &document.body,
                self.paragraph_ids()?,
                None,
                self.body_placeholder,
            ),
            StoryRef::Part(part_id) => {
                let part = document
                    .parts
                    .get(&part_id)
                    .ok_or_else(|| PyIndexError::new_err("owned story part is missing"))?;
                if !matches!(
                    part.kind,
                    PartKind::Header | PartKind::Footer | PartKind::Footnote | PartKind::Endnote
                ) {
                    return Err(EngineError::Unsupported(
                        "paragraph deletion requires an owned body, header, footer or note".into(),
                    )
                    .into());
                }
                if matches!(part.kind, PartKind::Header | PartKind::Footer) {
                    let refs = document
                        .sections()
                        .iter()
                        .flat_map(|(_, section)| {
                            [
                                section.headers.default,
                                section.headers.first,
                                section.headers.even,
                                section.footers.default,
                                section.footers.first,
                                section.footers.even,
                            ]
                        })
                        .filter(|target| *target == Some(part_id))
                        .count();
                    if refs != 1 {
                        return Err(EngineError::Unsupported(
                            "paragraph deletion requires independent header/footer ownership"
                                .into(),
                        )
                        .into());
                    }
                }
                (
                    &part.blocks,
                    self.story_paragraph_ids(part_id)?,
                    Some(part.kind),
                    self.story_placeholders.get(&part_id).copied(),
                )
            }
        };
        if !ids.contains(&id) {
            return Err(PyIndexError::new_err(
                "paragraph is not a direct owned story child",
            ));
        }
        let original_sections = if story == StoryRef::Body {
            Some(owned_body_sections(document)?)
        } else {
            None
        };
        if self.locations.len() > MAX_BLOCKS
            || ids.len() > MAX_BLOCKS
            || self.cell_placeholders.len() > MAX_CELLS
            || blocks.len() > MAX_BLOCKS + usize::from(story != StoryRef::Body)
        {
            return Err(limit("paragraph deletion exceeds owned locator limits"));
        }
        let index = path.last();
        if placeholder == Some(index) {
            return Err(PyValueError::new_err(
                "synthetic story paragraphs cannot be deleted by handle",
            ));
        }
        // Only adapter locators move. Native semantic IDs/content remain native-owned.
        for (other, (owner, path)) in &self.locations {
            if *owner != story {
                continue;
            }
            validate_story_locator(path, blocks.len())?;
            if path.0[0] as usize == index && *other != id {
                return Err(PyValueError::new_err(
                    "paragraph has inconsistent descendant locators",
                ));
            }
        }
        for (owner, path) in &self.cell_placeholders {
            if *owner != story {
                continue;
            }
            validate_story_locator(path, blocks.len())?;
            if path.0[0] as usize == index {
                return Err(PyValueError::new_err(
                    "paragraph has an inconsistent cell placeholder",
                ));
            }
        }
        if placeholder.is_some_and(|position| position >= blocks.len()) {
            return Err(PyValueError::new_err(
                "story placeholder is outside its native container",
            ));
        }
        let old_len = blocks.len();
        let mut staged = self.copy()?;
        let mut candidate = document.clone();
        let removed = candidate.remove_block(story, &path).map_err(model_error)?;
        if let Some(expected) = original_sections {
            // Native removal does not append after a section-bearing paragraph.
            // Keep the final section's required physical body paragraph without
            // moving any section property or assigning the blank an editable ID.
            if candidate
                .body
                .last()
                .and_then(|block| block.as_para())
                .is_some_and(|paragraph| paragraph.section.is_some())
            {
                candidate
                    .insert_block(
                        StoryRef::Body,
                        &Path::top(candidate.body.len()),
                        Block::Para(Paragraph::new()),
                    )
                    .map_err(model_error)?;
            }
            if owned_body_sections(&candidate)? != expected {
                return Err(EngineError::Unsupported(
                    "paragraph deletion cannot change section properties or count".into(),
                )
                .into());
            }
        }
        let (removed_text, removed_cells) = content_usage(&vec![Arc::new(removed)]);
        if removed_cells != 0 {
            return Err(PyValueError::new_err(
                "paragraph deletion unexpectedly removed table cells",
            ));
        }
        staged.text_bytes = self.updated_text_bytes(removed_text, 0)?;
        let new_len = match story {
            StoryRef::Body => candidate.body.len(),
            StoryRef::Part(part_id) => candidate.parts[&part_id].blocks.len(),
        };
        let appended = new_len == old_len;
        if appended
            && old_len == 1
            && matches!(part_kind, Some(PartKind::Footnote | PartKind::Endnote))
        {
            let style = if part_kind == Some(PartKind::Footnote) {
                "FootnoteText"
            } else {
                "EndnoteText"
            };
            candidate
                .set_story(
                    story,
                    vec![wordcraft_doc::para_block(Paragraph::new().styled(style))],
                )
                .map_err(model_error)?;
        }
        staged.locations.remove(&id);
        for (owner, path) in staged.locations.values_mut() {
            if *owner == story && path.0[0] as usize > index {
                path.0[0] -= 1;
            }
        }
        staged.cell_placeholders = staged
            .cell_placeholders
            .into_iter()
            .map(|(owner, mut path)| {
                if owner == story && path.0[0] as usize > index {
                    path.0[0] -= 1;
                }
                (owner, path)
            })
            .collect();
        match story {
            StoryRef::Body => {
                if appended {
                    staged.body_placeholder = Some(new_len - 1);
                } else if let Some(position) = staged.body_placeholder.as_mut() {
                    if *position > index {
                        *position -= 1;
                    }
                }
            }
            StoryRef::Part(part_id) => {
                staged
                    .stories
                    .get_mut(&part_id)
                    .ok_or_else(|| PyIndexError::new_err("owned story is missing"))?
                    .retain(|child| *child != id);
                if appended {
                    staged.story_placeholders.insert(part_id, new_len - 1);
                } else if let Some(position) = staged.story_placeholders.get_mut(&part_id) {
                    if *position > index {
                        *position -= 1;
                    }
                }
            }
        }
        staged.images.retain(|image| *image != id);
        staged.table_generations.remove(&id);
        // The staged model still holds the original anchors until normal commit checks finish.
        staged.commit(candidate)?;
        *self = staged;
        Ok(())
    }

    /// Reorder one intact ordinary paragraph through native block operations.
    pub fn move_paragraph(&mut self, id: usize, target: usize, revision: u64) -> PyResult<bool> {
        self.check_revision(revision)?;
        let paragraph = self.paragraph(id)?;
        let destination = self.paragraph(target)?;
        let (story, path) = self.location(id)?;
        let (target_story, target_path) = self.location(target)?;
        if story != target_story || path.0.len() != 1 || target_path.0.len() != 1 {
            return Err(EngineError::Unsupported(
                "paragraph movement requires the same direct owned container".into(),
            )
            .into());
        }
        if paragraph.section.is_some() || destination.section.is_some() {
            return Err(EngineError::Unsupported(
                "paragraph movement cannot select section boundaries".into(),
            )
            .into());
        }
        if paragraph.objects.iter().any(|object| {
            matches!(
                object,
                InlineObject::NoteRef { .. }
                    | InlineObject::BookmarkStart { .. }
                    | InlineObject::BookmarkEnd { .. }
                    | InlineObject::CommentStart { .. }
                    | InlineObject::CommentEnd { .. }
            )
        }) {
            return Err(EngineError::Unsupported(
                "moving paragraphs containing note, bookmark or comment anchors is not supported"
                    .into(),
            )
            .into());
        }
        let document = self.document()?;
        let (blocks, ids, placeholder) = match story {
            StoryRef::Body => (&document.body, self.paragraph_ids()?, self.body_placeholder),
            StoryRef::Part(part_id) => {
                let part = document
                    .parts
                    .get(&part_id)
                    .ok_or_else(|| PyIndexError::new_err("owned story part is missing"))?;
                if !matches!(
                    part.kind,
                    PartKind::Header | PartKind::Footer | PartKind::Footnote | PartKind::Endnote
                ) {
                    return Err(EngineError::Unsupported(
                        "paragraph movement requires an owned body, header, footer or note".into(),
                    )
                    .into());
                }
                if matches!(part.kind, PartKind::Header | PartKind::Footer) {
                    let references = document
                        .sections()
                        .iter()
                        .flat_map(|(_, section)| {
                            [
                                section.headers.default,
                                section.headers.first,
                                section.headers.even,
                                section.footers.default,
                                section.footers.first,
                                section.footers.even,
                            ]
                        })
                        .filter(|reference| *reference == Some(part_id))
                        .count();
                    if references != 1 {
                        return Err(EngineError::Unsupported(
                            "paragraph movement requires independent header/footer ownership"
                                .into(),
                        )
                        .into());
                    }
                }
                (
                    &part.blocks,
                    self.story_paragraph_ids(part_id)?,
                    self.story_placeholders.get(&part_id).copied(),
                )
            }
        };
        if !ids.contains(&id) || !ids.contains(&target) {
            return Err(PyIndexError::new_err(
                "paragraph is not a direct owned story child",
            ));
        }
        if self.locations.len() > MAX_BLOCKS
            || ids.len() > MAX_BLOCKS
            || self.cell_placeholders.len() > MAX_CELLS
            || blocks.len() > MAX_BLOCKS + usize::from(story != StoryRef::Body)
        {
            return Err(limit("paragraph movement exceeds owned locator limits"));
        }
        let source_index = path.last();
        let target_index = target_path.last();
        let sections = if story == StoryRef::Body {
            let sections = owned_body_sections(document)?;
            if document.section_index_of(source_index) != document.section_index_of(target_index) {
                return Err(EngineError::Unsupported(
                    "paragraph movement cannot cross body section boundaries".into(),
                )
                .into());
            }
            Some(sections)
        } else {
            None
        };
        for (other, (owner, location)) in &self.locations {
            if *owner != story {
                continue;
            }
            validate_story_locator(location, blocks.len())?;
            let root = location.0[0] as usize;
            if (root == source_index && *other != id) || (root == target_index && *other != target)
            {
                return Err(PyValueError::new_err(
                    "paragraph has inconsistent descendant locators",
                ));
            }
        }
        for (owner, location) in &self.cell_placeholders {
            if *owner != story {
                continue;
            }
            validate_story_locator(location, blocks.len())?;
            if [source_index, target_index].contains(&(location.0[0] as usize)) {
                return Err(PyValueError::new_err(
                    "paragraph has an inconsistent cell placeholder",
                ));
            }
        }
        if let Some(position) = placeholder {
            if position >= blocks.len()
                || position + 1 != blocks.len()
                || [source_index, target_index].contains(&position)
                || blocks[position].as_para().is_none_or(|paragraph| {
                    !paragraph.text.is_empty() || paragraph.section.is_some()
                })
            {
                return Err(PyValueError::new_err(
                    "story placeholder is invalid for paragraph movement",
                ));
            }
        }
        // Auxiliary direct IDs are stored separately from native paths; validate the
        // complete bounded list before sorting it so corrupt bookkeeping cannot panic.
        let mut previous = None;
        for child in &ids {
            let (owner, location) = self.location(*child)?;
            if owner != story || location.0.len() != 1 {
                return Err(PyValueError::new_err(
                    "direct paragraph registry is inconsistent",
                ));
            }
            validate_story_locator(&location, blocks.len())?;
            let position = location.last();
            if previous.is_some_and(|before| before >= position)
                || blocks[position].as_para().is_none()
            {
                return Err(PyValueError::new_err(
                    "direct paragraph registry is out of order",
                ));
            }
            previous = Some(position);
        }
        if id == target {
            return Ok(false);
        }
        let old_len = blocks.len();
        let mut staged = self.copy()?;
        let mut candidate = document.clone();
        let moved = candidate.remove_block(story, &path).map_err(model_error)?;
        let after_removal = match story {
            StoryRef::Body => candidate.body.len(),
            StoryRef::Part(part) => candidate.parts[&part].blocks.len(),
        };
        let appended = after_removal == old_len;
        if appended && placeholder.is_some() {
            return Err(PyValueError::new_err(
                "paragraph movement cannot duplicate a story placeholder",
            ));
        }
        candidate
            .insert_block(story, &Path::top(target_index), moved)
            .map_err(model_error)?;
        let new_len = match story {
            StoryRef::Body => candidate.body.len(),
            StoryRef::Part(part) => candidate.parts[&part].blocks.len(),
        };
        if new_len > MAX_BLOCKS + usize::from(story != StoryRef::Body) {
            return Err(limit("paragraph movement exceeds native block limits"));
        }
        if let Some(expected) = sections {
            if owned_body_sections(&candidate)? != expected
                || candidate.section_index_of(target_index)
                    != document.section_index_of(source_index)
            {
                return Err(EngineError::Unsupported(
                    "paragraph movement cannot change section ownership or properties".into(),
                )
                .into());
            }
        }
        // Only owner-local root positions change; native block contents and IDs survive intact.
        let moved_index = |position: usize| {
            if position == source_index {
                target_index
            } else if source_index < target_index
                && source_index < position
                && position <= target_index
            {
                position - 1
            } else if source_index > target_index
                && target_index <= position
                && position < source_index
            {
                position + 1
            } else {
                position
            }
        };
        for (owner, location) in staged.locations.values_mut() {
            if *owner == story {
                location.0[0] = moved_index(location.0[0] as usize) as u32;
            }
        }
        staged.cell_placeholders = staged
            .cell_placeholders
            .into_iter()
            .map(|(owner, mut location)| {
                if owner == story {
                    location.0[0] = moved_index(location.0[0] as usize) as u32;
                }
                (owner, location)
            })
            .collect();
        let next_placeholder = if appended {
            Some(new_len - 1)
        } else {
            placeholder.map(moved_index)
        };
        match story {
            StoryRef::Body => staged.body_placeholder = next_placeholder,
            StoryRef::Part(part) => {
                let mut reordered = ids;
                reordered.sort_by_key(|child| staged.locations[child].1 .0[0]);
                staged.stories.insert(part, reordered);
                if let Some(position) = next_placeholder {
                    staged.story_placeholders.insert(part, position);
                } else {
                    staged.story_placeholders.remove(&part);
                }
            }
        }
        staged.commit(candidate)?;
        *self = staged;
        Ok(true)
    }

    pub fn story_paragraph_ids(&self, story: u32) -> PyResult<Vec<usize>> {
        self.document()?;
        self.stories
            .get(&story)
            .cloned()
            .ok_or_else(|| PyIndexError::new_err("story index does not identify an owned story"))
    }

    pub fn story_table_ids(&mut self, story: u32) -> PyResult<Vec<usize>> {
        self.rich_story_child_ids(story, false)
    }

    pub fn story_image_ids(&mut self, story: u32) -> PyResult<Vec<usize>> {
        self.rich_story_child_ids(story, true)
    }

    pub fn story_add_table(&mut self, story: u32, rows: Vec<Vec<String>>) -> PyResult<usize> {
        let width = self.rich_story_width(story)?;
        let (table, text, cells) = self.make_table(rows, width)?;
        self.add_story_block(story, Block::Table(table), text, cells, None)
    }

    #[pyo3(signature = (story, data, width=None, height=None, alt=""))]
    pub fn story_add_image(
        &mut self,
        story: u32,
        data: &[u8],
        width: Option<f64>,
        height: Option<f64>,
        alt: &str,
    ) -> PyResult<usize> {
        let available = self.rich_story_width(story)?;
        let paragraph = self.make_image(data, width, height, alt)?;
        validate_picture_width(&paragraph, available)?;
        self.add_story_block(story, Block::Para(paragraph), 0, 0, Some(data.to_vec()))
    }

    pub fn story_add_paragraph(&mut self, story: u32, text: &str) -> PyResult<usize> {
        self.reserve_block()?;
        let mut ids = self.story_paragraph_ids(story)?;
        validate_text(text)?;
        let text_bytes = self.updated_text_bytes(0, text.len())?;
        let (id, next) = self.next_identifier()?;
        let mut candidate = self.document()?.clone();
        let part = candidate
            .parts
            .get_mut(&story)
            .ok_or_else(|| PyIndexError::new_err("story is missing from the native model"))?;
        let paragraph = match part.kind {
            PartKind::Footnote => {
                Paragraph::with_text(text, CharProps::default()).styled("FootnoteText")
            }
            PartKind::Endnote => {
                Paragraph::with_text(text, CharProps::default()).styled("EndnoteText")
            }
            _ => Paragraph::with_text(text, CharProps::default()),
        };
        let index = if let Some(index) = self.story_placeholders.get(&story).copied() {
            let target = part
                .blocks
                .get_mut(index)
                .ok_or_else(|| PyValueError::new_err("story placeholder no longer exists"))?;
            *target = wordcraft_doc::para_block(paragraph);
            index
        } else {
            let index = part.blocks.len();
            candidate
                .insert_block(
                    StoryRef::Part(story),
                    &Path::top(index),
                    Block::Para(paragraph),
                )
                .map_err(model_error)?;
            index
        };
        ids.push(id);
        self.commit(candidate)?;
        self.locations
            .insert(id, (StoryRef::Part(story), Path::top(index)));
        self.stories.insert(story, ids);
        self.story_placeholders.remove(&story);
        self.next_block_id = next;
        self.text_bytes = text_bytes;
        Ok(id)
    }
}

impl Default for EditableDocx {
    fn default() -> Self {
        Self::new()
    }
}

impl EditableDocx {
    /// Publish bookkeeping only after ordinary semantic/media guards accept the replacement.
    fn clear_owned_story_content(&mut self, id: u32, replacement: Paragraph) -> PyResult<bool> {
        let part = self
            .document()?
            .parts
            .get(&id)
            .ok_or_else(|| PyIndexError::new_err("owned story part is missing"))?;
        let (removed_text, removed_cells) = content_usage(&part.blocks);
        let text_bytes = self.updated_text_bytes(removed_text, 0)?;
        let cell_count = self
            .cell_count
            .checked_sub(removed_cells)
            .ok_or_else(|| limit("document cell accounting underflow"))?;
        let mut candidate = self.document()?.clone();
        candidate
            .set_story(
                StoryRef::Part(id),
                vec![wordcraft_doc::para_block(replacement)],
            )
            .map_err(model_error)?;
        // Semantic/media/field validation completes before publishing any locator changes.
        self.commit(candidate)?;
        self.locations
            .retain(|_, (story, _)| *story != StoryRef::Part(id));
        self.images.retain(|id| self.locations.contains_key(id));
        self.table_generations
            .retain(|id, _| self.locations.contains_key(id));
        self.cell_placeholders
            .retain(|(story, _)| *story != StoryRef::Part(id));
        self.stories.insert(id, Vec::new());
        self.story_placeholders.insert(id, 0);
        self.text_bytes = text_bytes;
        self.cell_count = cell_count;
        Ok(true)
    }

    fn rich_story_width(&self, story: u32) -> PyResult<f32> {
        self.story_paragraph_ids(story)?;
        let document = self.document()?;
        let part = document
            .parts
            .get(&story)
            .ok_or_else(|| PyIndexError::new_err("story is missing from the native model"))?;
        if !matches!(part.kind, PartKind::Header | PartKind::Footer) {
            return Err(EngineError::Unsupported(
                "rich note content requires native note-container geometry support".into(),
            )
            .into());
        }
        document
            .sections()
            .iter()
            .filter_map(|(_, section)| {
                let h = section.headers;
                let f = section.footers;
                [h.default, h.first, h.even, f.default, f.first, f.even]
                    .contains(&Some(story))
                    .then(|| section.text_width())
            })
            .reduce(f32::min)
            .ok_or_else(|| PyIndexError::new_err("story has no owning section"))
    }

    fn rich_story_child_ids(&mut self, story: u32, images: bool) -> PyResult<Vec<usize>> {
        self.rich_story_width(story)?;
        let targets = self.document()?.parts[&story]
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                let image = block.as_para().is_some_and(|paragraph| {
                    paragraph
                        .objects
                        .iter()
                        .any(|object| matches!(object, InlineObject::Image { .. }))
                });
                (if images {
                    image
                } else {
                    block.as_table().is_some()
                })
                .then_some(Path::top(index))
            })
            .collect::<Vec<_>>();
        targets
            .into_iter()
            .map(|path| self.register_location(StoryRef::Part(story), path, images))
            .collect()
    }

    fn add_story_block(
        &mut self,
        story: u32,
        mut block: Block,
        added_text: usize,
        added_cells: usize,
        media: Option<Vec<u8>>,
    ) -> PyResult<usize> {
        self.rich_story_width(story)?;
        self.reserve_block()?;
        let text_bytes = self.updated_text_bytes(0, added_text)?;
        let (identifier, next) = self.next_identifier()?;
        let is_table = block.as_table().is_some();
        let is_image = media.is_some();
        let mut candidate = self.document()?.clone();
        if let Some(bytes) = media {
            let Block::Para(paragraph) = &mut block else {
                return Err(PyValueError::new_err("picture media requires a paragraph"));
            };
            attach_image_media(&mut candidate, paragraph, bytes)?;
        }
        let length = candidate.parts[&story].blocks.len();
        let placeholder = self.story_placeholders.get(&story).copied();
        let index = placeholder.unwrap_or(length);
        let path = Path::top(index);
        if placeholder.is_some() {
            let target = candidate
                .parts
                .get_mut(&story)
                .and_then(|part| part.blocks.get_mut(index))
                .ok_or_else(|| PyValueError::new_err("story placeholder no longer exists"))?;
            *target = Arc::new(block);
        } else {
            candidate
                .insert_block(StoryRef::Part(story), &path, block)
                .map_err(model_error)?;
        }
        if is_table {
            candidate
                .insert_block(
                    StoryRef::Part(story),
                    &Path::top(index + 1),
                    Block::Para(Paragraph::new()),
                )
                .map_err(model_error)?;
        }
        self.commit(candidate)?;
        self.locations
            .insert(identifier, (StoryRef::Part(story), path));
        if is_image {
            self.images.push(identifier);
        }
        if is_table {
            self.story_placeholders.insert(story, index + 1);
        } else {
            self.story_placeholders.remove(&story);
        }
        self.next_block_id = next;
        self.text_bytes = text_bytes;
        self.cell_count += added_cells;
        Ok(identifier)
    }

    fn section(&self, section: usize) -> PyResult<&SectionProps> {
        self.document()?
            .sections()
            .get(section)
            .map(|(_, props)| *props)
            .ok_or_else(|| PyIndexError::new_err("section index out of bounds"))
    }

    fn validate_table_layout(&self, document: &Document, id: usize) -> PyResult<()> {
        let (story, path) = self.location(id)?;
        validate_native_table_layout(document, &native_cell_widths(document)?, story, path)
    }

    fn row(&self, id: usize, row: usize) -> PyResult<&wordcraft_doc::Row> {
        self.table(id)?
            .rows
            .get(row)
            .ok_or_else(|| PyIndexError::new_err("row index out of bounds"))
    }

    fn container_width(&self, id: usize) -> PyResult<f32> {
        let (story, path) = self.location(id)?;
        let Some((parent, row, cell)) = path.cell() else {
            let document = self.document()?;
            return match story {
                StoryRef::Body => Ok(column_width(
                    self.section(document.section_index_of(path.0[0] as usize))?,
                )),
                StoryRef::Part(id) => document
                    .sections()
                    .iter()
                    .find_map(|(_, section)| {
                        let h = section.headers;
                        let f = section.footers;
                        [h.default, h.first, h.even, f.default, f.first, f.even]
                            .contains(&Some(id))
                            .then(|| section.text_width())
                    })
                    .ok_or_else(|| PyIndexError::new_err("story has no owning section")),
            };
        };
        let table = self
            .document()?
            .table(story, &parent)
            .ok_or_else(|| PyIndexError::new_err("parent table no longer exists"))?;
        let margins = wordcraft_layout::cell_margins(self.document()?, table, row, cell);
        let widths = native_cell_widths(self.document()?)?;
        Ok(native_cell_width(&widths, story, &parent, row, cell)? - margins[1] - margins[3])
    }

    fn cell_container(&self, id: usize, row: usize, column: usize) -> PyResult<(StoryRef, Path)> {
        let (row, cell) = cell_anchor(self.table(id)?, row, column)?;
        let (story, table_path) = self.location(id)?;
        Ok((
            story,
            Path([table_path.0.as_slice(), &[row as u32, cell as u32]].concat()),
        ))
    }

    fn mutate_table_structure(
        &mut self,
        id: usize,
        operation: impl FnOnce(&mut Table) -> PyResult<()>,
    ) -> PyResult<()> {
        let original = self.table(id)?;
        let before = table_usage(original);
        let generation = self
            .table_generation(id)?
            .checked_add(1)
            .ok_or_else(|| limit("table generation limit reached"))?;
        let (story, path) = self.location(id)?;
        // Keep original blocks alive through staging. Arc identity follows the native moves,
        // including merged cells, without inventing an OOXML path-remapping algorithm.
        let hidden = self
            .cell_placeholders
            .iter()
            .filter(|(owner, child)| *owner == story && child.0.starts_with(&path.0))
            .filter_map(|(_, child)| {
                self.document()
                    .ok()?
                    .container(story, child)?
                    .get(child.last())
            })
            .map(|block| Arc::as_ptr(block) as usize)
            .collect::<BTreeSet<_>>();
        let mut candidate = self.document()?.clone();
        let table = self.table_mut(&mut candidate, id)?;
        operation(table)?;
        validate_editable_grid(table)?;
        let width_percent = table.props.width_pct;
        synchronize_grid_widths(table);
        if let Some(percent) = width_percent {
            table.props.width = None;
            table.props.width_pct = Some(percent);
        }
        let after = table_usage(table);
        let text_bytes = self.updated_text_bytes(before.0, after.0)?;
        let cells = self
            .cell_count
            .checked_sub(before.1)
            .and_then(|count| count.checked_add(after.1))
            .filter(|count| *count <= MAX_CELLS)
            .ok_or_else(|| limit("document exceeds 100000 table cells"))?;
        if table.total_width() > self.container_width(id)? + 0.001 {
            return Err(PyValueError::new_err("table columns exceed the available page or cell width; resize columns before inserting"));
        }
        validate_grid_children(
            &candidate,
            candidate
                .table(story, &path)
                .ok_or_else(|| PyIndexError::new_err("table no longer exists"))?,
        )?;
        let placeholders = candidate
            .para_paths(story)
            .into_iter()
            .filter(|child| child.0.len() > path.0.len() && child.0.starts_with(&path.0))
            .filter(|child| {
                candidate
                    .container(story, child)
                    .and_then(|blocks| blocks.get(child.last()))
                    .is_some_and(|block| hidden.contains(&(Arc::as_ptr(block) as usize)))
            })
            .collect::<Vec<_>>();
        self.validate_table_layout(&candidate, id)?;
        self.commit(candidate)?;
        self.invalidate_descendants(story, &path);
        self.cell_placeholders
            .extend(placeholders.into_iter().map(|path| (story, path)));
        self.table_generations.insert(id, generation);
        self.text_bytes = text_bytes;
        self.cell_count = cells;
        Ok(())
    }

    fn invalidate_descendants(&mut self, story: StoryRef, prefix: &Path) {
        self.locations.retain(|_, (owner, path)| {
            *owner != story || path.0.len() <= prefix.0.len() || !path.0.starts_with(&prefix.0)
        });
        self.images.retain(|id| self.locations.contains_key(id));
        self.table_generations
            .retain(|id, _| self.locations.contains_key(id));
        self.cell_placeholders
            .retain(|(owner, path)| *owner != story || !path.0.starts_with(&prefix.0));
    }

    fn register_location(&mut self, story: StoryRef, path: Path, image: bool) -> PyResult<usize> {
        if let Some((id, _)) = self
            .locations
            .iter()
            .find(|(_, location)| **location == (story, path.clone()))
        {
            return Ok(*id);
        }
        self.reserve_block()?;
        let (id, next) = self.next_identifier()?;
        self.locations.insert(id, (story, path));
        self.next_block_id = next;
        if image {
            self.images.push(id);
        }
        Ok(id)
    }

    fn cell_child_ids(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        kind: &str,
    ) -> PyResult<Vec<usize>> {
        let (story, prefix) = self.cell_container(id, row, column)?;
        let (anchor_row, cell) = cell_anchor(self.table(id)?, row, column)?;
        let targets = self.table(id)?.rows[anchor_row].cells[cell].blocks.iter().enumerate().filter_map(|(index, block)| {
            let path = Path([prefix.0.as_slice(), &[index as u32]].concat());
            if self.cell_placeholders.contains(&(story, path.clone())) { return None; }
            let image = matches!(block.as_para(), Some(paragraph) if paragraph.objects.iter().any(|object| matches!(object, InlineObject::Image { .. })));
            let matches = match kind { "image" => image, "paragraph" => block.as_para().is_some() && !image, "table" => block.as_table().is_some(), _ => false };
            matches.then_some((path, image))
        }).collect::<Vec<_>>();
        targets
            .into_iter()
            .map(|(path, image)| self.register_location(story, path, image))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn add_cell_block(
        &mut self,
        id: usize,
        row: usize,
        column: usize,
        mut block: Block,
        added_text: usize,
        added_cells: usize,
        media: Option<Vec<u8>>,
    ) -> PyResult<usize> {
        self.reserve_block()?;
        let text_bytes = self.updated_text_bytes(0, added_text)?;
        let (story, prefix) = self.cell_container(id, row, column)?;
        let (row, cell) = cell_anchor(self.table(id)?, row, column)?;
        let length = self.table(id)?.rows[row].cells[cell].blocks.len();
        let previous = Path([prefix.0.as_slice(), &[length.saturating_sub(1) as u32]].concat());
        let consume = self.cell_placeholders.contains(&(story, previous.clone()));
        let index = if consume { length - 1 } else { length };
        let path = Path([prefix.0.as_slice(), &[index as u32]].concat());
        let (identifier, next) = self.next_identifier()?;
        let is_table = block.as_table().is_some();
        let is_image = media.is_some();
        let mut candidate = self.document()?.clone();
        if let Some(bytes) = media {
            let Block::Para(paragraph) = &mut block else {
                return Err(PyValueError::new_err("picture media requires a paragraph"));
            };
            attach_image_media(&mut candidate, paragraph, bytes)?;
        }
        if consume {
            candidate.container_mut(story, &path).map_err(model_error)?[index] = Arc::new(block);
        } else {
            candidate
                .insert_block(story, &path, block)
                .map_err(model_error)?;
        }
        let placeholder = if is_table {
            let path = Path([prefix.0.as_slice(), &[(index + 1) as u32]].concat());
            candidate
                .insert_block(story, &path, Block::Para(Paragraph::new()))
                .map_err(model_error)?;
            Some(path)
        } else {
            None
        };
        self.commit(candidate)?;
        self.cell_placeholders.remove(&(story, previous));
        if let Some(path) = placeholder {
            self.cell_placeholders.insert((story, path));
        }
        self.locations.insert(identifier, (story, path));
        if is_image {
            self.images.push(identifier);
        }
        self.next_block_id = next;
        self.text_bytes = text_bytes;
        self.cell_count += added_cells;
        Ok(identifier)
    }

    fn make_image(
        &self,
        data: &[u8],
        width: Option<f64>,
        height: Option<f64>,
        alt: &str,
    ) -> PyResult<Paragraph> {
        self.document()?;
        validate_alt(alt)?;
        let (pixel_width, pixel_height) = validate_png(data)?;
        let (width, height) = match (width, height) {
            (Some(width), Some(height)) => (width, height),
            (Some(width), None) => (
                width,
                width * f64::from(pixel_height) / f64::from(pixel_width),
            ),
            (None, Some(height)) => (
                height * f64::from(pixel_width) / f64::from(pixel_height),
                height,
            ),
            (None, None) => (f64::from(pixel_width), f64::from(pixel_height)),
        };
        validate_image_size(width, height)?;
        let mut paragraph = Paragraph::new();
        paragraph
            .insert_object(
                0,
                InlineObject::Image {
                    media: String::new(),
                    w: width as f32,
                    h: height as f32,
                    alt: alt.into(),
                    float: Float::default(),
                    crop: [0.0; 4],
                    ole: None,
                },
                &CharProps::default(),
            )
            .map_err(model_error)?;
        Ok(paragraph)
    }

    fn make_table(&self, rows: Vec<Vec<String>>, width: f32) -> PyResult<(Table, usize, usize)> {
        let columns = rows.first().map_or(0, Vec::len);
        if rows.is_empty() || rows.len() > wordcraft_doc::table::MAX_ROWS {
            return Err(PyValueError::new_err("table requires 1..32767 rows"));
        }
        if columns == 0 || columns > wordcraft_doc::table::MAX_COLS {
            return Err(PyValueError::new_err("table requires 1..63 columns"));
        }
        if rows.iter().any(|row| row.len() != columns) {
            return Err(PyValueError::new_err("table rows must have equal lengths"));
        }
        let cells = rows
            .len()
            .checked_mul(columns)
            .ok_or_else(|| limit("table is too large"))?;
        if self.cell_count.saturating_add(cells) > MAX_CELLS {
            return Err(limit("document exceeds 100000 table cells"));
        }
        let mut added_text = 0_usize;
        for text in rows.iter().flatten() {
            validate_text(text)?;
            added_text = added_text
                .checked_add(text.len())
                .ok_or_else(|| limit("table text is too large"))?;
        }
        self.updated_text_bytes(0, added_text)?;
        if width / (columns as f32) < 6.0 {
            return Err(PyValueError::new_err(
                "page width must allow at least 6 points per table column",
            ));
        }
        let mut table = Table::new(rows.len(), columns, width);
        for (target_row, source_row) in table.rows.iter_mut().zip(rows) {
            for (cell, text) in target_row.cells.iter_mut().zip(source_row) {
                cell.blocks = vec![wordcraft_doc::para_block(Paragraph::with_text(
                    &text,
                    CharProps::default(),
                ))];
            }
        }
        Ok((table, added_text, cells))
    }

    // IDs are owner-local identities; native paths remain bounded physical positions.
    // Return the prospective counter without changing it, so failed commits consume no ID.
    fn next_identifier(&self) -> PyResult<(usize, usize)> {
        let id = self.next_block_id;
        let next = id
            .checked_add(1)
            .ok_or_else(|| limit("document child identifier limit reached"))?;
        if self.locations.contains_key(&id) {
            return Err(PyValueError::new_err(
                "document child identifier is already allocated",
            ));
        }
        Ok((id, next))
    }

    fn body_child_ids(&self, tables: bool) -> PyResult<Vec<usize>> {
        let document = self.document()?;
        if self.locations.len() > MAX_BLOCKS || document.body.len() > MAX_BLOCKS {
            return Err(limit("body enumeration exceeds document block limits"));
        }
        // A transient reverse view keeps enumeration in native order without either
        // repeated linear registry scans or a second persistent identity index.
        let mut by_position = BTreeMap::new();
        for (id, (story, path)) in &self.locations {
            if *story == StoryRef::Body && path.0.len() == 1 {
                let index = path.0[0] as usize;
                if index >= document.body.len() || by_position.insert(index, *id).is_some() {
                    return Err(PyValueError::new_err(
                        "invalid or duplicate body child location",
                    ));
                }
            }
        }
        let images: BTreeSet<_> = self.images.iter().copied().collect();
        let mut ids = Vec::new();
        for (index, block) in document.body.iter().enumerate() {
            if tables {
                if block.as_table().is_some() {
                    ids.push(*by_position.get(&index).ok_or_else(|| {
                        PyValueError::new_err("body table has no owned identifier")
                    })?);
                }
            } else if block.as_para().is_some() && self.body_placeholder != Some(index) {
                if let Some(id) = by_position.get(&index).filter(|id| !images.contains(id)) {
                    ids.push(*id);
                }
            }
        }
        Ok(ids)
    }

    fn reserve_block(&self) -> PyResult<()> {
        self.document()?;
        if self.locations.len() >= MAX_BLOCKS {
            return Err(limit(
                "document exceeds 100000 editable blocks across all stories",
            ));
        }
        Ok(())
    }

    fn location(&self, id: usize) -> PyResult<(StoryRef, Path)> {
        self.document()?;
        self.locations
            .get(&id)
            .cloned()
            .ok_or_else(|| PyIndexError::new_err("unknown document child identifier"))
    }

    fn position(&self, id: usize, offset: usize) -> PyResult<Pos> {
        let (story, path) = self.location(id)?;
        Ok(Pos::new(story, path, offset))
    }

    fn paragraph_mut<'a>(
        &self,
        document: &'a mut Document,
        id: usize,
    ) -> PyResult<&'a mut Paragraph> {
        let (story, path) = self.location(id)?;
        document.para_mut(story, &path).map_err(model_error)
    }

    fn table_mut<'a>(&self, document: &'a mut Document, id: usize) -> PyResult<&'a mut Table> {
        let (story, path) = self.location(id)?;
        document.table_mut(story, &path).map_err(model_error)
    }
    fn commit(&mut self, document: Document) -> PyResult<()> {
        self.commit_checked(document, None)
    }

    fn commit_checked(
        &mut self,
        document: Document,
        removed_bookmark: Option<&str>,
    ) -> PyResult<()> {
        self.commit_with_note_conversion(document, removed_bookmark, None)
    }

    fn commit_with_note_conversion(
        &mut self,
        mut document: Document,
        removed_bookmark: Option<&str>,
        converted_notes: Option<NoteKind>,
    ) -> PyResult<()> {
        if let Some(target) = converted_notes {
            if removed_bookmark.is_some() {
                return Err(PyValueError::new_err(
                    "note conversion cannot remove a bookmark",
                ));
            }
            validate_note_conversion_anchors(self.document()?, &document, target)?;
        }
        validate_bookmark_links(&document)?;
        let media_bytes = if document.media.is_empty() && document.settings.page_picture.is_none() {
            0
        } else {
            retain_authored_image_media(&mut document)?
        };
        page_field_count(&document)?;
        validate_line_number_work(&document)?;
        outline_targets(&document)?;
        validate_table_metadata_budget(&document)?;
        validate_document_tabs(&document)?;
        // Native generic range/table edits do not repair semantic references. Existing
        // marker identities and multiplicities must survive every ordinary mutation.
        // New notes may add an identity; removing or duplicating one is unsupported.
        let existing = semantic_anchors(self.document()?);
        if let Some(name) = removed_bookmark {
            let next = semantic_anchors(&document);
            if [2, 3].iter().any(|kind| {
                existing.get(&(*kind, name.to_owned())) != Some(&1)
                    || next.contains_key(&(*kind, name.to_owned()))
            }) {
                return Err(PyValueError::new_err(
                    "bookmark deletion must remove exactly its own marker pair",
                ));
            }
        }
        if converted_notes.is_none() && !existing.is_empty() {
            let next = semantic_anchors(&document);
            if existing.iter().any(|(key, count)| {
                !(removed_bookmark == Some(key.1.as_str()) && matches!(key.0, 2 | 3))
                    && next.get(key) != Some(count)
            }) {
                return Err(EngineError::Unsupported(
                    "edit would remove or duplicate a note, bookmark or comment anchor; semantic deletion is not available".into(),
                ).into());
            }
        }
        if hyperlink_bytes(&document) > MAX_LINK_BYTES {
            return Err(limit("document hyperlink targets exceed 16 MiB"));
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| limit("document revision limit reached"))?;
        self.document = Some(document);
        self.media_bytes = media_bytes;
        self.revision = revision;
        Ok(())
    }

    fn check_revision(&self, revision: u64) -> PyResult<()> {
        self.document()?;
        if self.revision != revision {
            return Err(PyValueError::new_err(
                "text range is stale after a document mutation",
            ));
        }
        Ok(())
    }

    fn document(&self) -> PyResult<&Document> {
        self.document
            .as_ref()
            .ok_or_else(|| DocExNativeError::new_err("closed:document is closed"))
    }

    fn paragraph(&self, id: usize) -> PyResult<&Paragraph> {
        let (story, path) = self.location(id)?;
        self.document()?
            .para(story, &path)
            .filter(|_| {
                !(self.images.contains(&id)
                    || story == StoryRef::Body
                        && path.0.len() == 1
                        && self.body_placeholder == Some(path.0[0] as usize))
            })
            .ok_or_else(|| {
                PyIndexError::new_err("paragraph index does not identify an added paragraph")
            })
    }

    fn image(&self, id: usize) -> PyResult<&InlineObject> {
        let (story, path) = self.location(id)?;
        self.document()?
            .para(story, &path)
            .filter(|_| self.images.contains(&id))
            .and_then(|paragraph| paragraph.objects.first())
            .filter(|object| matches!(object, InlineObject::Image { .. }))
            .ok_or_else(|| PyIndexError::new_err("image index does not identify a picture"))
    }

    fn table(&self, id: usize) -> PyResult<&Table> {
        let (story, path) = self.location(id)?;
        self.document()?
            .table(story, &path)
            .ok_or_else(|| PyIndexError::new_err("table index does not identify a table"))
    }

    fn updated_text_bytes(&self, old: usize, new: usize) -> PyResult<usize> {
        let total = self
            .text_bytes
            .checked_sub(old)
            .and_then(|value| value.checked_add(new))
            .ok_or_else(|| limit("document text size overflow"))?;
        if total > MAX_DOCUMENT_TEXT_BYTES {
            return Err(limit("document text exceeds 16 MiB"));
        }
        Ok(total)
    }
}

fn property_patch(updates: &str) -> PyResult<Map<String, Value>> {
    if updates.len() > MAX_PROPERTY_BYTES {
        return Err(limit("metadata update exceeds 256 KiB"));
    }
    serde_json::from_str(updates)
        .map_err(|_| PyValueError::new_err("metadata updates require a JSON object"))
}

fn validate_property_text(text: &str) -> PyResult<()> {
    if text.len() > MAX_PROPERTY_VALUE_BYTES {
        return Err(limit("metadata value exceeds 64 KiB"));
    }
    validate_text(text)
}

fn validate_property_timestamp(text: &str) -> PyResult<()> {
    let bytes = text.as_bytes();
    let invalid = || {
        PyValueError::new_err(
            "metadata timestamps require a valid UTC YYYY-MM-DDTHH:MM:SSZ value (years 0001..9999)",
        )
    };
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| ![4, 7, 10, 13, 16, 19].contains(&index) && !byte.is_ascii_digit())
    {
        return Err(invalid());
    }
    let number = |start: usize, end: usize| {
        bytes[start..end]
            .iter()
            .fold(0u32, |value, byte| value * 10 + u32::from(*byte - b'0'))
    };
    let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return Err(invalid()),
    };
    if year == 0
        || day == 0
        || day > days
        || number(11, 13) > 23
        || number(14, 16) > 59
        || number(17, 19) > 59
    {
        return Err(invalid());
    }
    Ok(())
}

fn core_metadata(document: &Document) -> Value {
    let core = &document.core;
    let mut values = Map::new();
    for (key, text) in [
        ("title", &core.title),
        ("author", &core.creator),
        ("subject", &core.subject),
        ("keywords", &core.keywords),
        ("description", &core.description),
        ("category", &core.category),
        ("last_modified_by", &core.last_modified_by),
        ("created", &core.created),
        ("modified", &core.modified),
    ] {
        if !text.is_empty() {
            values.insert(key.into(), json!(text));
        }
    }
    if core.revision > 0 {
        values.insert("revision".into(), json!(core.revision));
    }
    Value::Object(values)
}

fn custom_metadata(document: &Document) -> PyResult<Value> {
    let mut values = Map::new();
    for property in &document.custom_props {
        let malformed =
            || PyValueError::new_err("custom metadata is outside the supported scalar profile");
        let value = crate::docx::custom_scalar(property).ok_or_else(malformed)?;
        values.insert(property.name.clone(), value);
    }
    Ok(Value::Object(values))
}

fn document_metadata(document: &Document) -> PyResult<Value> {
    let mut values = core_metadata(document);
    values
        .as_object_mut()
        .unwrap()
        .insert("custom".into(), custom_metadata(document)?);
    Ok(values)
}

fn patch_custom_metadata(candidate: &mut Document, value: Value) -> PyResult<()> {
    if value.is_null() {
        candidate.custom_props.clear();
        return Ok(());
    }
    let values = value
        .as_object()
        .cloned()
        .ok_or_else(|| PyValueError::new_err("custom metadata requires an object or null"))?;
    let mut names = BTreeSet::new();
    for (name, value) in values {
        validate_property_text(&name)?;
        if name.is_empty()
            || name.len() > 255
            || name.trim() != name
            || name.chars().any(char::is_control)
        {
            return Err(PyValueError::new_err("custom property names require 1..255 UTF-8 bytes, no control characters or surrounding whitespace"));
        }
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(PyValueError::new_err(
                "custom property updates contain case-insensitive duplicate names",
            ));
        }
        if value.is_null() {
            candidate.remove_custom_prop(&name);
            continue;
        }
        let (kind, text) = match value {
                Value::String(text) => {
                    validate_property_text(&text)?;
                    ("lpwstr", text)
                }
                Value::Bool(value) => ("bool", value.to_string()),
                Value::Number(value) if value.is_i64() => ("i8", value.to_string()),
                Value::Number(value) if value.is_f64() && value.as_f64().is_some_and(f64::is_finite) => ("r8", value.to_string()),
                _ => return Err(PyValueError::new_err("custom properties require strings, bools, signed 64-bit integers, finite floats or null")),
            };
        if let Some(property) = candidate
            .custom_props
            .iter_mut()
            .find(|property| property.name.eq_ignore_ascii_case(&name))
        {
            property.kind = kind.into();
            property.value = text;
        } else {
            candidate.custom_props.push(wordcraft_doc::CustomProp {
                name,
                kind: kind.into(),
                value: text,
            });
        }
    }
    if candidate.custom_props.len() > MAX_CUSTOM_PROPERTIES {
        return Err(limit("document exceeds 1024 custom properties"));
    }
    if custom_metadata(candidate)?.to_string().len() > MAX_PROPERTY_BYTES {
        return Err(limit(
            "custom properties exceed 256 KiB of serialized values",
        ));
    }
    Ok(())
}

fn page_field_count(document: &Document) -> PyResult<usize> {
    let sections = document.sections();
    // Reuse native section endpoints: section_index_of() rebuilds that inventory
    // for every call, which would make this whole-document check quadratic.
    let section_of = |path: &Path| {
        sections
            .iter()
            .position(|(end, _)| path.0[0] as usize <= *end)
            .unwrap_or(0)
    };
    let continuous: BTreeSet<_> = sections
        .iter()
        .enumerate()
        .filter_map(|(index, (_, props))| {
            (props.start == SectionStart::Continuous).then_some(index)
        })
        .collect();
    // A section-end collector flushes all pending endnotes referenced so far,
    // including notes whose anchor section requested document-end placement.
    // Project only ownership for the safety guard; native layout still computes
    // all page counts, field values and actual note placement.
    let mut collector = sections.len().saturating_sub(1);
    let mut endnote_sections = vec![collector; sections.len()];
    for (index, (_, props)) in sections.iter().enumerate().rev() {
        if document.note_options(props, true).pos == NotePos::SectEnd {
            collector = index;
        }
        endnote_sections[index] = collector;
    }
    let mut continuous_parts = BTreeSet::new();
    for index in &continuous {
        let props = sections[*index].1;
        continuous_parts.extend(
            [
                props.headers.default,
                props.headers.first,
                props.headers.even,
                props.footers.default,
                props.footers.first,
                props.footers.even,
            ]
            .into_iter()
            .flatten(),
        );
    }
    if !continuous.is_empty() {
        for path in document.para_paths(StoryRef::Body) {
            let section = section_of(&path);
            if let Some(paragraph) = document.para(StoryRef::Body, &path) {
                for object in &paragraph.objects {
                    if let InlineObject::NoteRef { id, kind, .. } = object {
                        let target = if *kind == NoteKind::Endnote {
                            endnote_sections[section]
                        } else {
                            section
                        };
                        if continuous.contains(&target) {
                            continuous_parts.insert(*id);
                        }
                    }
                }
            }
        }
    }
    let mut count = 0usize;
    let mut expanded_bytes = 0usize;
    let mut has_metadata_fields = false;
    for story in
        std::iter::once(StoryRef::Body).chain(document.parts.keys().copied().map(StoryRef::Part))
    {
        for path in document.para_paths(story) {
            if let Some(paragraph) = document.para(story, &path) {
                let continuous_scope = match story {
                    StoryRef::Body => continuous.contains(&section_of(&path)),
                    StoryRef::Part(id) => continuous_parts.contains(&id),
                };
                let mut paragraph_bytes = paragraph.text.len();
                for object in &paragraph.objects {
                    if let InlineObject::Field { instr, result, .. } = object {
                        if let Some(value) = metadata_field_value(document, instr, result) {
                            has_metadata_fields = true;
                            if value.chars().any(|ch| {
                                matches!(
                                    ch,
                                    '\n' | '\t' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
                                )
                            }) {
                                return Err(EngineError::Unsupported("title and author fields require single-line metadata without tabs".into()).into());
                            }
                            paragraph_bytes = paragraph_bytes.saturating_add(value.len());
                            if paragraph_bytes > MAX_TEXT_BYTES {
                                return Err(limit(
                                    "metadata field expansion exceeds 1 MiB of paragraph text",
                                ));
                            }
                        }
                        count = count.saturating_add(1);
                        if count > MAX_PAGE_FIELDS {
                            return Err(limit("document exceeds 10000 fields"));
                        }
                        if continuous_scope
                            && wordcraft_layout::fields::field_name(instr) == "SECTIONPAGES"
                        {
                            return Err(EngineError::Unsupported("section_page_count is not supported in continuous sections at this native revision".into()).into());
                        }
                    }
                }
                expanded_bytes = expanded_bytes.saturating_add(paragraph_bytes);
                if has_metadata_fields && expanded_bytes > MAX_DOCUMENT_TEXT_BYTES {
                    return Err(limit(
                        "metadata field expansion exceeds 16 MiB of document text",
                    ));
                }
            }
        }
    }
    Ok(count)
}

fn metadata_field_value<'a>(
    document: &'a Document,
    instruction: &str,
    result: &str,
) -> Option<&'a str> {
    if !result.is_empty() {
        return None;
    }
    match wordcraft_layout::fields::field_name(instruction).as_str() {
        "TITLE" => Some(&document.core.title),
        "AUTHOR" => Some(&document.core.creator),
        _ => None,
    }
}

// Keys and object offsets are native UTF-8 paths/bytes, never Python scalar indexes.
type MetadataFields = BTreeMap<(StoryRef, Path), BTreeMap<usize, usize>>;

fn metadata_field_offsets(document: &Document) -> PyResult<MetadataFields> {
    let mut fields = BTreeMap::new();
    let mut count = 0usize;
    for story in
        std::iter::once(StoryRef::Body).chain(document.parts.keys().copied().map(StoryRef::Part))
    {
        for path in document.para_paths(story) {
            let Some(paragraph) = document.para(story, &path) else {
                continue;
            };
            for (offset, object) in paragraph
                .object_offsets()
                .into_iter()
                .zip(&paragraph.objects)
            {
                if let InlineObject::Field { instr, result, .. } = object {
                    if let Some(value) = metadata_field_value(document, instr, result) {
                        count += 1;
                        if count > MAX_PAGE_FIELDS {
                            return Err(limit("document exceeds 10000 fields"));
                        }
                        fields
                            .entry((story, path.clone()))
                            .or_insert_with(BTreeMap::new)
                            .insert(offset, value.len());
                    }
                }
            }
        }
    }
    Ok(fields)
}

fn attach_image_media(
    document: &mut Document,
    paragraph: &mut Paragraph,
    data: Vec<u8>,
) -> PyResult<()> {
    let Some(InlineObject::Image { media, .. }) = paragraph.objects.first_mut() else {
        return Err(PyValueError::new_err(
            "picture media requires an inline image",
        ));
    };
    *media = document.add_media(data, "png");
    Ok(())
}

/// Reference accounting for this owner's authored image/note/page-field profile. Native
/// paths traverse nested tables; we do not reconstruct an OOXML resource graph.
fn retain_authored_image_media(document: &mut Document) -> PyResult<usize> {
    let mut used = BTreeSet::new();
    // A document-wide background is one resource reference, not an inline
    // placement per rendered page. Shared inline/background bytes count once.
    if let Some(picture) = &document.settings.page_picture {
        if !document.media.contains_key(&picture.media) {
            return Err(PyValueError::new_err(
                "background image references missing native media",
            ));
        }
        used.insert(picture.media.clone());
    }
    let mut references = 0usize;
    for story in
        std::iter::once(StoryRef::Body).chain(document.parts.keys().map(|id| StoryRef::Part(*id)))
    {
        for path in document.para_paths(story) {
            let paragraph = document
                .para(story, &path)
                .ok_or_else(|| PyValueError::new_err("native paragraph path no longer exists"))?;
            for object in &paragraph.objects {
                match object {
                    InlineObject::Image { media, ole: None, .. } => {
                        references += 1;
                        if references > 1000 {
                            return Err(limit("document exceeds 1000 image placements"));
                        }
                        if !document.media.contains_key(media) {
                            return Err(PyValueError::new_err("picture references missing native media"));
                        }
                        used.insert(media.clone());
                    }
                    InlineObject::NoteRef { .. } | InlineObject::BookmarkStart { .. } | InlineObject::BookmarkEnd { .. } => {}
                    // Fields own instruction/result text, not media references. Legacy
                    // form XML is outside our authored page-field profile.
                    InlineObject::Field { form, .. } if form.is_empty() => {}
                    _ => return Err(EngineError::Unsupported("media cleanup requires the authored image/note/page-field profile; other inline objects need upstream reference traversal".into()).into()),
                }
            }
        }
    }
    if used.len() > 1000 {
        return Err(limit("document exceeds 1000 distinct image resources"));
    }
    let bytes = used.iter().try_fold(0usize, |total, key| {
        total
            .checked_add(document.media[key].len())
            .filter(|total| *total <= MAX_MEDIA_BYTES)
            .ok_or_else(|| limit("document images exceed 64 MiB of referenced encoded media"))
    })?;
    document.media.retain(|key, _| used.contains(key));
    Ok(bytes)
}

fn table_patch(patch: &str) -> PyResult<Map<String, Value>> {
    if patch.len() > MAX_TABLE_PATCH_BYTES {
        return Err(limit("table format patch exceeds 128 KiB"));
    }
    let value: Value = serde_json::from_str(patch)
        .map_err(|_| PyValueError::new_err("table formatting requires a JSON object"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| PyValueError::new_err("table formatting requires a JSON object"))
}

fn strict_bool(value: &Value, key: &str) -> PyResult<bool> {
    value
        .as_bool()
        .ok_or_else(|| PyValueError::new_err(format!("{key} requires a bool")))
}

fn nullable_point(value: &Value, minimum: f32, maximum: f32, key: &str) -> PyResult<Option<f32>> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_f64()
        .filter(|v| v.is_finite() && *v >= f64::from(minimum) && *v <= f64::from(maximum))
        .map(|v| Some(v as f32))
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "{key} must be a finite number between {minimum} and {maximum}, or None"
            ))
        })
}

fn nullable_alignment(value: &Value, paragraph: bool) -> PyResult<Option<Align>> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(match value.as_str() {
        Some("left") => Align::Left,
        Some("center") => Align::Center,
        Some("right") => Align::Right,
        Some("justify") if paragraph => Align::Justify,
        Some("distribute") if paragraph => Align::Distribute,
        _ => {
            return Err(PyValueError::new_err(if paragraph {
                "alignment must be left, center, right, justify, distribute or None"
            } else {
                "table alignment must be left, center, right or None"
            }))
        }
    }))
}

fn vertical_alignment_name(value: VAlign) -> &'static str {
    match value {
        VAlign::Top => "top",
        VAlign::Center => "center",
        VAlign::Bottom => "bottom",
    }
}
fn text_direction_name(value: TextDirection) -> &'static str {
    match value {
        TextDirection::Horizontal => "horizontal",
        TextDirection::Down => "down",
        TextDirection::Up => "up",
    }
}
fn table_alt(value: &Value) -> PyResult<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let text = value
        .as_str()
        .ok_or_else(|| PyValueError::new_err("table title/description must be text or None"))?;
    validate_text(text)?;
    if text.chars().count() > wordcraft_doc::para::MAX_ALT_TEXT {
        return Err(PyValueError::new_err(
            "table title/description exceeds 10000 characters",
        ));
    }
    Ok(Some(text.into()))
}

const BORDER_STYLES: &[(&str, BorderStyle)] = &[
    ("none", BorderStyle::None),
    ("single", BorderStyle::Single),
    ("double", BorderStyle::Double),
    ("dotted", BorderStyle::Dotted),
    ("dashed", BorderStyle::Dashed),
    ("thick", BorderStyle::Thick),
    ("triple", BorderStyle::Triple),
    ("dot_dash", BorderStyle::DotDash),
    ("wave", BorderStyle::Wave),
];
fn border_value(value: &Value) -> PyResult<Option<Border>> {
    if value.is_null() {
        return Ok(None);
    }
    let fields = value.as_object().ok_or_else(|| {
        PyValueError::new_err("border must be a style/color/width object or None")
    })?;
    if fields
        .keys()
        .any(|key| !matches!(key.as_str(), "style" | "color" | "width"))
    {
        return Err(PyValueError::new_err(
            "border fields must be style, color or width",
        ));
    }
    let style = match fields.get("style") {
        None => BorderStyle::Single,
        Some(value) => BORDER_STYLES
            .iter()
            .find(|(name, _)| Some(*name) == value.as_str())
            .map(|(_, style)| *style)
            .ok_or_else(|| PyValueError::new_err("unsupported border style"))?,
    };
    let color = match fields.get("color") {
        None | Some(Value::Null) => None,
        Some(value) => Some(parse_rgb(value.as_str().ok_or_else(|| {
            PyValueError::new_err("border color requires #RRGGBB or None")
        })?)?),
    };
    let width = match fields.get("width") {
        None => {
            if style == BorderStyle::None {
                0.0
            } else {
                0.5
            }
        }
        Some(value) => value
            .as_f64()
            .ok_or_else(|| PyValueError::new_err("border width requires a number"))?,
    };
    if style == BorderStyle::None {
        if width != 0.0 {
            return Err(PyValueError::new_err("a hidden border requires zero width"));
        }
    } else if !width.is_finite() || !(0.125..=12.0).contains(&width) || (width * 8.0).fract() != 0.0
    {
        return Err(PyValueError::new_err(
            "border width must be 0.125..12 points in eighth-point increments",
        ));
    }
    if matches!(style, BorderStyle::Triple | BorderStyle::DotDash)
        || (style != BorderStyle::None && width < 0.25)
        || (style == BorderStyle::Thick && width < 1.5)
    {
        return Err(EngineError::Unsupported(
            "native DOCX borders require width at least 0.25pt (1.5pt for thick); triple and dot_dash styles are unavailable".into(),
        ).into());
    }
    Ok(Some(Border {
        style,
        width: width as f32,
        color,
        space: 0.0,
    }))
}
fn apply_border_patch(target: &mut Option<Borders>, value: &Value, inner: bool) -> PyResult<()> {
    if value.is_null() {
        *target = None;
        return Ok(());
    }
    let values = value
        .as_object()
        .ok_or_else(|| PyValueError::new_err("borders requires a side mapping or None"))?;
    let mut borders = target.unwrap_or_default();
    for (key, value) in values {
        let side = match key.as_str() {
            "top" => &mut borders.top,
            "right" => &mut borders.right,
            "bottom" => &mut borders.bottom,
            "left" => &mut borders.left,
            "inside_horizontal" if inner => &mut borders.between,
            "inside_vertical" if inner => &mut borders.inside_v,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "unsupported border side: {key}"
                )))
            }
        };
        *side = border_for_side(value, key)?;
    }
    *target = (borders != Borders::default()).then_some(borders);
    Ok(())
}
fn border_for_side(value: &Value, side: &str) -> PyResult<Option<Border>> {
    let border = border_value(value)?;
    if matches!(side, "left" | "right" | "inside_vertical")
        && border
            .is_some_and(|border| matches!(border.style, BorderStyle::Double | BorderStyle::Wave))
    {
        return Err(EngineError::Unsupported(
            "native DOCX double and wave borders support only horizontal edges".into(),
        )
        .into());
    }
    Ok(border)
}
fn borders_snapshot(borders: Option<Borders>, inner: bool) -> Value {
    let Some(borders) = borders else {
        return Value::Null;
    };
    let mut out = Map::new();
    for (name, border) in [
        ("top", borders.top),
        ("right", borders.right),
        ("bottom", borders.bottom),
        ("left", borders.left),
        ("inside_horizontal", borders.between),
        ("inside_vertical", borders.inside_v),
    ] {
        if !inner && name.starts_with("inside_") {
            continue;
        }
        let value = border.map(|border| json!({
            "style": BORDER_STYLES.iter().find(|(_,style)| *style == border.style).map(|(name,_)| *name).unwrap_or("none"),
            "color": border.color.map(rgb_string), "width": border.width,
        })).unwrap_or(Value::Null);
        out.insert(name.into(), value);
    }
    Value::Object(out)
}
fn active_table_indent(table: &Table) -> f32 {
    if matches!(table.props.align, Some(Align::Center | Align::Right)) {
        0.0
    } else {
        table.props.indent.unwrap_or(0.0)
    }
}
fn validate_table_metadata_budget(document: &Document) -> PyResult<()> {
    let mut bytes = 0_usize;
    let mut pending: Vec<&Block> = document
        .body
        .iter()
        .chain(document.parts.values().flat_map(|part| part.blocks.iter()))
        .map(AsRef::as_ref)
        .collect();
    while let Some(block) = pending.pop() {
        if let Block::Table(table) = block {
            for value in [&table.props.caption, &table.props.description]
                .into_iter()
                .flatten()
            {
                bytes = bytes
                    .checked_add(value.len())
                    .filter(|bytes| *bytes <= MAX_TABLE_METADATA_BYTES)
                    .ok_or_else(|| limit("table titles and descriptions exceed 16 MiB"))?;
            }
            pending.extend(
                table
                    .rows
                    .iter()
                    .flat_map(|row| &row.cells)
                    .flat_map(|cell| &cell.blocks)
                    .map(AsRef::as_ref),
            );
        }
    }
    Ok(())
}

type LayoutCellKey = (StoryRef, Path, usize, usize);

fn rich_story_ids(document: &Document) -> BTreeSet<u32> {
    document
        .parts
        .iter()
        .filter_map(|(id, part)| {
            (matches!(part.kind, PartKind::Header | PartKind::Footer)
                && part.blocks.iter().any(|block| {
                    block.as_table().is_some()
                        || block.as_para().is_some_and(|paragraph| {
                            paragraph
                                .objects
                                .iter()
                                .any(|object| matches!(object, InlineObject::Image { .. }))
                        })
                }))
            .then_some(*id)
        })
        .collect()
}

/// Header/footer boxes do not paginate. Check public placed frames, retaining
/// native exact-height cell clipping when its clipping frame fits the page.
fn validate_rich_story_page_bounds(
    page: &wordcraft_layout::Page,
    rich: &BTreeSet<u32>,
) -> PyResult<()> {
    if rich.is_empty() {
        return Ok(());
    }
    let clipped = page
        .header
        .iter()
        .chain(&page.footer)
        .filter_map(|item| match item {
            wordcraft_layout::Placed::Cell {
                story,
                table,
                row,
                cell,
                clip: true,
                ..
            } => Some((
                *story,
                Path([table.0.as_slice(), &[*row as u32, *cell as u32]].concat()),
            )),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for item in page.header.iter().chain(&page.footer) {
        let (story, path, rect) = match item {
            wordcraft_layout::Placed::Cell {
                story, table, rect, ..
            } => (story, table, rect),
            wordcraft_layout::Placed::Image {
                story, path, rect, ..
            }
            | wordcraft_layout::Placed::Object {
                story, path, rect, ..
            } => (story, path, rect),
            _ => continue,
        };
        if !matches!(story, StoryRef::Part(id) if rich.contains(id)) {
            continue;
        }
        if (3..path.0.len())
            .step_by(3)
            .any(|length| clipped.contains(&(*story, Path(path.0[..length].to_vec()))))
        {
            continue;
        }
        if ![page.w, page.h, rect.x, rect.y, rect.w, rect.h]
            .iter()
            .all(|v| v.is_finite())
            || page.w <= 0.0
            || page.h <= 0.0
            || rect.w < 0.0
            || rect.h < 0.0
            || rect.x < -0.01
            || rect.y < -0.01
            || rect.x + rect.w > page.w + 0.01
            || rect.y + rect.h > page.h + 0.01
        {
            return Err(EngineError::Unsupported("header/footer table or image exceeds the physical page; resize the content before serialization".into()).into());
        }
    }
    Ok(())
}

/// The native paginator places an oversized unsplittable row whole, even beyond
/// the physical page. Reject that known loss before serializing authored data.
/// Vertical merge anchors carry multi-row geometry and are deliberately outside
/// this narrow check; exact-height clipping *inside* a page remains intentional.
fn validate_unsplittable_page_bounds(document: &Document) -> PyResult<()> {
    validate_line_number_work(document)?;
    let outlines = outline_targets(document)?;
    let mut pending = native_table_roots(document);
    let mut guarded = BTreeSet::new();
    let mut cells = 0usize;
    while let Some((story, path)) = pending.pop() {
        let table = document.table(story, &path).ok_or_else(|| {
            PyValueError::new_err("table no longer exists during pagination validation")
        })?;
        let mut header_prefix = true;
        for (row_index, row) in table.rows.iter().enumerate() {
            header_prefix &= row.props.header;
            cells = cells.saturating_add(row.cells.len());
            if cells > MAX_CELLS {
                return Err(limit("pagination validation exceeds 100000 table cells"));
            }
            if (row.props.cant_split
                || header_prefix
                || (row.props.height_rule == HeightRule::Exact
                    && row.props.height.is_some_and(|height| height > 0.0)))
                && row
                    .cells
                    .iter()
                    .all(|cell| cell.props.vmerge == VMerge::None)
            {
                guarded.insert((story, path.clone(), row_index));
            }
            for (cell_index, cell) in row.cells.iter().enumerate() {
                for (block_index, block) in cell.blocks.iter().enumerate() {
                    if matches!(block.as_ref(), Block::Table(_)) {
                        pending.push((
                            story,
                            Path(
                                [
                                    path.0.as_slice(),
                                    &[row_index as u32, cell_index as u32, block_index as u32],
                                ]
                                .concat(),
                            ),
                        ));
                    }
                }
            }
        }
    }
    let rich = rich_story_ids(document);
    let numbered = document
        .sections()
        .iter()
        .any(|(_, section)| section.line_numbers.is_some());
    let bookmarks = validate_bookmark_links(document)?;
    let metadata_fields = metadata_field_offsets(document)?;
    if guarded.is_empty()
        && rich.is_empty()
        && !numbered
        && bookmarks.is_empty()
        && outlines.is_empty()
        && metadata_fields.is_empty()
    {
        return Ok(());
    }
    let layout = wordcraft_layout::layout(
        document,
        &mut wordcraft_layout::LayoutCache::new(),
        &wordcraft_layout::LayoutOptions::default(),
    );
    if layout.pages.len() > MAX_LAYOUT_PAGES {
        return Err(limit("pagination validation exceeds 10000 pages"));
    }
    if numbered {
        validate_line_number_bounds(&layout)?;
    }
    let mut items = 0usize;
    let mut field_work = 0usize;
    let mut field_display_bytes = 0usize;
    for page in &layout.pages {
        items = items
            .saturating_add(page.items.len())
            .saturating_add(page.header.len())
            .saturating_add(page.footer.len());
        if items > MAX_LAYOUT_ITEMS {
            return Err(limit("pagination validation exceeds 2000000 placed items"));
        }
        if !page.h.is_finite() || page.h <= 0.0 {
            return Err(EngineError::Unsupported(
                "native pagination produced invalid page geometry".into(),
            )
            .into());
        }
        validate_rich_story_page_bounds(page, &rich)?;
        if !metadata_fields.is_empty() {
            validate_metadata_field_placement(
                page,
                &metadata_fields,
                &mut field_work,
                &mut field_display_bytes,
            )?;
        }
        for item in page.items.iter().chain(&page.header).chain(&page.footer) {
            if let wordcraft_layout::Placed::Cell {
                story,
                table,
                row,
                rect,
                ..
            } = item
            {
                if guarded.contains(&(*story, table.clone(), *row))
                    && (!rect.y.is_finite()
                        || !rect.h.is_finite()
                        || rect.h < 0.0
                        || rect.y < -0.01
                        || rect.bottom() > page.h + 0.01)
                {
                    return Err(EngineError::Unsupported(
                        "an unsplittable table row exceeds the physical page; allow row splitting, reduce its content or formatting, or enlarge the page".into(),
                    ).into());
                }
            }
        }
    }
    if !bookmarks.is_empty() {
        validate_bookmark_layout(document, &layout, &bookmarks)?;
    }
    if !outlines.is_empty() {
        validate_outline_layout(&layout, &outlines)?;
    }
    Ok(())
}

// Native fields are indivisible text clusters. Check their public layout boxes;
// do not reconstruct shaping, wrap field text, or promise glyph-outline bounds.
fn validate_metadata_field_placement(
    page: &wordcraft_layout::Page,
    fields: &MetadataFields,
    work: &mut usize,
    display_bytes: &mut usize,
) -> PyResult<()> {
    let unsupported = || {
        EngineError::Unsupported(
        "metadata field requires an unclipped horizontal line box within its available width and physical page; shorten the metadata or change formatting/layout".into(),
    )
    };
    let charge = |work: &mut usize, count: usize| -> PyResult<()> {
        *work = work.saturating_add(count);
        if *work > MAX_LAYOUT_ITEMS {
            return Err(limit(
                "metadata field placement exceeds 2000000 item/line/cluster scan units",
            ));
        }
        Ok(())
    };
    // Charge every actual placement, including repeated headers/footers, before
    // looking up field targets or collecting any clipping prefixes.
    charge(
        work,
        page.items
            .len()
            .saturating_add(page.header.len())
            .saturating_add(page.footer.len()),
    )?;
    let clipped: BTreeSet<_> = page
        .items
        .iter()
        .chain(&page.header)
        .chain(&page.footer)
        .filter_map(|item| match item {
            wordcraft_layout::Placed::Cell {
                story,
                table,
                row,
                cell,
                clip: true,
                ..
            } => Some((
                *story,
                Path([table.0.as_slice(), &[*row as u32, *cell as u32]].concat()),
            )),
            _ => None,
        })
        .collect();
    for item in page.items.iter().chain(&page.header).chain(&page.footer) {
        let wordcraft_layout::Placed::Lines {
            story,
            path,
            para,
            l0,
            l1,
            x,
            y,
            turn,
        } = item
        else {
            continue;
        };
        let Some(targets) = fields.get(&(*story, path.clone())) else {
            continue;
        };
        if *turn != TextDirection::Horizontal
            || *l0 >= *l1
            || *l1 > para.lines.len()
            || (3..path.0.len())
                .step_by(3)
                .any(|length| clipped.contains(&(*story, Path(path.0[..length].to_vec()))))
        {
            return Err(unsupported().into());
        }
        charge(work, l1 - l0)?;
        let mut seen = BTreeSet::new();
        for line in &para.lines[*l0..*l1] {
            if line.c0 > line.c1 || line.c1 > para.clusters.len() {
                return Err(unsupported().into());
            }
            charge(work, line.c1 - line.c0)?;
            for index in line.c0..line.c1 {
                let cluster = &para.clusters[index];
                if cluster.start >= cluster.end {
                    if targets.contains_key(&cluster.start) {
                        return Err(unsupported().into());
                    }
                    continue;
                }
                let mut matches = targets.range(cluster.start..cluster.end);
                let Some((offset, bytes)) = matches.next() else {
                    continue;
                };
                if matches.next().is_some()
                    || cluster.start != *offset
                    || cluster.end != offset.saturating_add(wordcraft_doc::para::OBJ.len_utf8())
                    || !seen.insert(*offset)
                {
                    return Err(unsupported().into());
                }
                *display_bytes = display_bytes.saturating_add(*bytes);
                if *display_bytes > MAX_DOCUMENT_TEXT_BYTES {
                    return Err(limit(
                        "rendered metadata field occurrences exceed 16 MiB of display text",
                    ));
                }
                let left = line.cl_left(index).ok_or_else(unsupported)?;
                let right = line.cl_right(index).ok_or_else(unsupported)?;
                let top = *y + line.top - para.lines[*l0].top;
                let bottom = top + line.height;
                if ![
                    left, right, line.left, line.right, top, bottom, *x, *y, page.w, page.h,
                ]
                .iter()
                .all(|value| value.is_finite())
                    || page.w <= 0.0
                    || page.h <= 0.0
                    || right < left
                    || bottom < top
                    || left < line.left - 0.01
                    || right > line.right + 0.01
                    || *x + left < -0.01
                    || *x + right > page.w + 0.01
                    || top < -0.01
                    || bottom > page.h + 0.01
                {
                    return Err(unsupported().into());
                }
            }
        }
    }
    Ok(())
}

fn native_table_roots(document: &Document) -> Vec<(StoryRef, Path)> {
    let mut roots = Vec::new();
    for (index, block) in document.body.iter().enumerate() {
        if matches!(block.as_ref(), Block::Table(_)) {
            roots.push((StoryRef::Body, Path::top(index)));
        }
    }
    for (id, part) in &document.parts {
        for (index, block) in part.blocks.iter().enumerate() {
            if matches!(block.as_ref(), Block::Table(_)) {
                roots.push((StoryRef::Part(*id), Path::top(index)));
            }
        }
    }
    roots
}

fn validate_unequal_column_tables(document: &Document) -> PyResult<()> {
    let mut roots = Vec::new();
    let mut begin = 0;
    for (end, section) in document.sections() {
        if !section.columns.widths.is_empty() {
            let blocks = document
                .body
                .get(begin..=end)
                .ok_or_else(|| PyValueError::new_err("native section has invalid body bounds"))?;
            roots.extend(blocks.iter().enumerate().filter_map(|(index, block)| {
                matches!(block.as_ref(), Block::Table(_)).then_some(Path::top(begin + index))
            }));
        }
        begin = end + 1;
    }
    if roots.is_empty() {
        return Ok(());
    }
    // Recheck actual public layout at serialization, since edits to preceding
    // text can move an autofit table to a different-width column.
    let widths = native_cell_widths(document)?;
    for path in roots {
        validate_native_table_layout(document, &widths, StoryRef::Body, path)?;
    }
    Ok(())
}

fn validate_style_tables(original: &Document, candidate: &Document) -> PyResult<()> {
    let roots = native_table_roots(candidate);
    if roots.is_empty() {
        return Ok(());
    }
    let widths = native_cell_widths(candidate)?;
    let mut previous_widths = None;
    for ((story, path, row, index), width) in &widths {
        let table = candidate
            .table(*story, path)
            .ok_or_else(|| PyValueError::new_err("native layout references a missing table"))?;
        let cell = table
            .rows
            .get(*row)
            .and_then(|r| r.cells.get(*index))
            .ok_or_else(|| PyValueError::new_err("native layout references a missing cell"))?;
        let margins = wordcraft_layout::cell_margins(candidate, table, *row, *index);
        let inner = width - margins[1] - margins[3];
        if inner <= 0.0 {
            return Err(PyValueError::new_err(
                "table style margins must leave positive cell content width",
            ));
        }
        blocks_fit_width(&cell.blocks, inner)?;
        if let Some(old_table) = original.table(*story, path) {
            let old_margins = wordcraft_layout::cell_margins(original, old_table, *row, *index);
            if old_margins != margins {
                if previous_widths.is_none() {
                    previous_widths = Some(native_cell_widths(original)?);
                }
                if let Some(previous_widths) = &previous_widths {
                    let previous = native_cell_width(previous_widths, *story, path, *row, *index)?;
                    let available = previous - margins[1] - margins[3];
                    if available <= 0.0 {
                        return Err(PyValueError::new_err(
                            "table style margins exceed the previous cell allocation",
                        ));
                    }
                    blocks_fit_width(&cell.blocks, available)?;
                }
            }
        }
    }
    for (story, path) in roots {
        validate_native_table_layout(candidate, &widths, story, path)?;
    }
    Ok(())
}

fn validate_native_table_layout(
    document: &Document,
    widths: &BTreeMap<LayoutCellKey, f32>,
    story: StoryRef,
    path: Path,
) -> PyResult<()> {
    let mut pending = vec![path];
    while let Some(path) = pending.pop() {
        let table = document
            .table(story, &path)
            .ok_or_else(|| PyIndexError::new_err("table no longer exists"))?;
        let available =
            native_container_width(document, widths, story, &path)? - active_table_indent(table);
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut total = 0.0;
            for (cell_index, cell) in row.cells.iter().enumerate() {
                let width = native_cell_width(widths, story, &path, row_index, cell_index)?;
                total += width;
                if width + 0.001 < cell.span() as f32 * 6.0 {
                    return Err(PyValueError::new_err(
                        "native table layout must leave at least 6 points per column",
                    ));
                }
                let margins =
                    wordcraft_layout::cell_margins(document, table, row_index, cell_index);
                let inner = width - margins[1] - margins[3];
                if inner <= 0.0 {
                    return Err(PyValueError::new_err(
                        "table layout must leave positive width inside effective cell margins",
                    ));
                }
                for (index, block) in cell.blocks.iter().enumerate() {
                    match block.as_ref() {
                        Block::Para(paragraph) => validate_picture_width(paragraph, inner)?,
                        Block::Table(_) => pending.push(Path(
                            [
                                path.0.as_slice(),
                                &[row_index as u32, cell_index as u32, index as u32],
                            ]
                            .concat(),
                        )),
                    }
                }
            }
            if total > available + 0.01 || available <= 0.0 {
                return Err(PyValueError::new_err(
                    "native table layout exceeds its section or cell width",
                ));
            }
        }
    }
    Ok(())
}

fn native_cell_widths(document: &Document) -> PyResult<BTreeMap<LayoutCellKey, f32>> {
    // Mutations can measure candidate table geometry before commit. Guard the
    // candidate itself before native pagination allocates line-number layouts.
    validate_line_number_work(document)?;
    let layout = wordcraft_layout::layout(
        document,
        &mut wordcraft_layout::LayoutCache::new(),
        &wordcraft_layout::LayoutOptions::default(),
    );
    let mut widths = BTreeMap::new();
    for page in &layout.pages {
        for item in page.items.iter().chain(&page.header).chain(&page.footer) {
            if let wordcraft_layout::Placed::Cell {
                story,
                table,
                row,
                cell,
                rect,
                ..
            } = item
            {
                if !rect.w.is_finite() || rect.w <= 0.0 {
                    return Err(PyValueError::new_err(
                        "native layout produced invalid cell width",
                    ));
                }
                widths
                    .entry((*story, table.clone(), *row, *cell))
                    .and_modify(|width: &mut f32| *width = width.min(rect.w))
                    .or_insert(rect.w);
                if widths.len() > MAX_CELLS {
                    return Err(limit("native layout exceeds 100000 table cells"));
                }
            }
        }
    }
    Ok(widths)
}
fn native_cell_width(
    widths: &BTreeMap<LayoutCellKey, f32>,
    story: StoryRef,
    path: &Path,
    row: usize,
    cell: usize,
) -> PyResult<f32> {
    widths
        .get(&(story, path.clone(), row, cell))
        .copied()
        .ok_or_else(|| {
            EngineError::Unsupported(
                "native layout did not expose the requested table cell geometry".into(),
            )
            .into()
        })
}
fn native_container_width(
    document: &Document,
    widths: &BTreeMap<LayoutCellKey, f32>,
    story: StoryRef,
    path: &Path,
) -> PyResult<f32> {
    if let Some((parent, row, cell)) = path.cell() {
        let table = document
            .table(story, &parent)
            .ok_or_else(|| PyIndexError::new_err("parent table no longer exists"))?;
        let margins = wordcraft_layout::cell_margins(document, table, row, cell);
        return Ok(native_cell_width(widths, story, &parent, row, cell)? - margins[1] - margins[3]);
    }
    match story {
        StoryRef::Body => {
            let sections = document.sections();
            let index = document.section_index_of(path.0[0] as usize);
            Ok(column_width(sections[index].1))
        }
        StoryRef::Part(id) => document
            .sections()
            .iter()
            .find_map(|(_, section)| {
                let h = section.headers;
                let f = section.footers;
                [h.default, h.first, h.even, f.default, f.first, f.even]
                    .contains(&Some(id))
                    .then(|| section.text_width())
            })
            .ok_or_else(|| PyIndexError::new_err("story has no owning section")),
    }
}

fn note_kind_values(kind: &str) -> PyResult<(PartKind, NoteKind, &'static str, &'static str)> {
    match kind {
        "footnote" => Ok((
            PartKind::Footnote,
            NoteKind::Footnote,
            "FootnoteText",
            "FootnoteReference",
        )),
        "endnote" => Ok((
            PartKind::Endnote,
            NoteKind::Endnote,
            "EndnoteText",
            "EndnoteReference",
        )),
        _ => Err(PyValueError::new_err(
            "note kind must be footnote or endnote",
        )),
    }
}

fn note_placement_name(value: NotePos) -> &'static str {
    match value {
        NotePos::PageBottom => "page_bottom",
        NotePos::BeneathText => "beneath_text",
        NotePos::SectEnd => "section_end",
        NotePos::DocEnd => "document_end",
    }
}

fn note_restart_name(value: NoteRestart) -> &'static str {
    match value {
        NoteRestart::Continuous => "continuous",
        NoteRestart::EachSect => "each_section",
        NoteRestart::EachPage => "each_page",
    }
}

const NOTE_NUMBERING: &[(&str, NumFormat)] = &[
    ("decimal", NumFormat::Decimal),
    ("upper_roman", NumFormat::UpperRoman),
    ("lower_roman", NumFormat::LowerRoman),
    ("upper_letter", NumFormat::UpperLetter),
    ("lower_letter", NumFormat::LowerLetter),
    ("chicago", NumFormat::Chicago),
];

fn parse_note_numbering(value: &str) -> PyResult<NumFormat> {
    NOTE_NUMBERING.iter().find(|(name, _)| *name == value).map(|(_, format)| *format)
        .ok_or_else(|| PyValueError::new_err("numbering must be decimal, upper_roman, lower_roman, upper_letter, lower_letter or chicago"))
}

fn note_numbering_name(value: NumFormat) -> PyResult<&'static str> {
    NOTE_NUMBERING
        .iter()
        .find(|(_, format)| *format == value)
        .map(|(name, _)| *name)
        .ok_or_else(|| {
            EngineError::Unsupported(
                "native note numbering is outside the supported authoring subset".into(),
            )
            .into()
        })
}

fn reject_semantic_range(paragraph: &Paragraph, start: usize, end: usize) -> PyResult<()> {
    reject_semantic_range_except(paragraph, start, end, None)
}

// Only relocation of an already validated pair may select its own markers.
fn reject_semantic_range_except(
    paragraph: &Paragraph,
    start: usize,
    end: usize,
    relocated_bookmark: Option<&str>,
) -> PyResult<()> {
    if paragraph
        .object_offsets()
        .into_iter()
        .zip(&paragraph.objects)
        .any(|(offset, object)| {
            let own_pair = matches!(object,
                InlineObject::BookmarkStart { name } | InlineObject::BookmarkEnd { name }
                if relocated_bookmark == Some(name.as_str()));
            start <= offset
                && offset < end
                && !own_pair
                && matches!(
                    object,
                    InlineObject::NoteRef { .. }
                        | InlineObject::BookmarkStart { .. }
                        | InlineObject::BookmarkEnd { .. }
                        | InlineObject::CommentStart { .. }
                        | InlineObject::CommentEnd { .. }
                )
        })
    {
        return Err(EngineError::Unsupported(
            "text replacement cannot remove note, bookmark or comment anchors".into(),
        )
        .into());
    }
    Ok(())
}

#[derive(Clone)]
struct BookmarkSpan {
    path: Path,
    start: usize,
    end: usize,
}

fn validate_bookmark_name(name: &str) -> PyResult<()> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 40
        || !bytes[0].is_ascii_alphabetic()
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return Err(PyValueError::new_err("bookmark name requires 1..40 ASCII letters, digits or underscores, starting with a letter"));
    }
    Ok(())
}

/// Inspect only public native paragraph paths/inline markers. Names are identities,
/// not cached coordinates; table grid edits may move their paths.
fn validate_bookmark_links(document: &Document) -> PyResult<BTreeMap<String, BookmarkSpan>> {
    let mut starts = BTreeMap::new();
    let mut ends = BTreeMap::new();
    let mut targets = BTreeSet::new();
    let mut visited = 0usize;
    let mut unsupported_objects = false;
    for story in
        std::iter::once(StoryRef::Body).chain(document.parts.keys().map(|id| StoryRef::Part(*id)))
    {
        for path in document.para_paths(story) {
            visited += 1;
            if visited > MAX_LAYOUT_ITEMS {
                return Err(limit("bookmark validation exceeds 2000000 paragraph paths"));
            }
            let paragraph = document
                .para(story, &path)
                .ok_or_else(|| PyValueError::new_err("native paragraph path no longer exists"))?;
            let mut marker_offsets = Vec::new();
            for (offset, object) in paragraph
                .object_offsets()
                .into_iter()
                .zip(&paragraph.objects)
            {
                unsupported_objects |= !matches!(
                    object,
                    InlineObject::Image { ole: None, .. }
                        | InlineObject::NoteRef { .. }
                        | InlineObject::BookmarkStart { .. }
                        | InlineObject::BookmarkEnd { .. }
                ) && !matches!(object, InlineObject::Field { form, .. } if form.is_empty());
                let (name, map) = match object {
                    InlineObject::BookmarkStart { name } => (name, &mut starts),
                    InlineObject::BookmarkEnd { name } => (name, &mut ends),
                    _ => continue,
                };
                validate_bookmark_name(name)?;
                marker_offsets.push(offset);
                if story != StoryRef::Body {
                    return Err(EngineError::Unsupported("bookmark targets outside the body are not exported by the pinned native engine".into()).into());
                }
                if map.insert(name.clone(), (path.clone(), offset)).is_some() {
                    return Err(PyValueError::new_err("duplicate bookmark marker identity"));
                }
                if map.len() > MAX_BOOKMARKS {
                    return Err(limit("document exceeds 1000 bookmarks"));
                }
            }
            // Native shaping ends a segment at every bookmark object. Only
            // paragraph endpoints preserve the original text shaping. Discount
            // other bookmark markers, but never image/note/field object bytes.
            let content_bytes = paragraph.text.len() - marker_offsets.len() * 3;
            for (index, offset) in marker_offsets.into_iter().enumerate() {
                let content_offset = offset - index * 3;
                if content_offset != 0 && content_offset != content_bytes {
                    return Err(EngineError::Unsupported("bookmark targets require whole paragraphs or paragraph-endpoint points; interior markers change native text shaping".into()).into());
                }
            }
            for (_, props) in paragraph.run_ranges() {
                if let Some(name) = props
                    .link
                    .as_deref()
                    .and_then(|link| link.strip_prefix('#'))
                {
                    validate_bookmark_name(name)?;
                    targets.insert(name.to_owned());
                    if targets.len() > MAX_BOOKMARKS {
                        return Err(limit("document exceeds 1000 internal link targets"));
                    }
                }
            }
        }
    }
    if unsupported_objects && (!starts.is_empty() || !ends.is_empty() || !targets.is_empty()) {
        return Err(EngineError::Unsupported("navigation requires the authored image/note/page-field/bookmark profile; other objects need upstream export auditing".into()).into());
    }
    let mut spans = BTreeMap::new();
    for (name, (path, start)) in starts {
        let (end_path, end) = ends
            .remove(&name)
            .ok_or_else(|| PyValueError::new_err("bookmark is missing its end marker"))?;
        if path != end_path || start >= end {
            return Err(EngineError::Unsupported(
                "bookmark pairs require an ordered range within one body paragraph".into(),
            )
            .into());
        }
        spans.insert(name, BookmarkSpan { path, start, end });
    }
    if !ends.is_empty() {
        return Err(PyValueError::new_err(
            "bookmark is missing its start marker",
        ));
    }
    if targets.iter().any(|name| !spans.contains_key(name)) {
        return Err(PyValueError::new_err(
            "internal hyperlink refers to a missing bookmark; remove or retarget its links first",
        ));
    }
    Ok(spans)
}

/// One linked Glyphs batch can call native link_rect at most once. Recursive
/// wrappers never duplicate calls. This conservatively bounds the native 10000
/// rectangles/page cap without copying its private coalescing/geometry algorithm.
fn validate_bookmark_draws(
    draws: &[wordcraft_layout::display::Draw],
    target_work: &BTreeMap<String, usize>,
    work: &mut usize,
) -> PyResult<usize> {
    use wordcraft_layout::display::Draw;
    if draws.len() > MAX_LAYOUT_ITEMS {
        return Err(limit("navigation display traversal exceeds 2000000 items"));
    }
    let mut pending = draws
        .iter()
        .map(|draw| (draw, 0usize, false))
        .collect::<Vec<_>>();
    let mut visited = 0usize;
    let mut linked = 0usize;
    while let Some((draw, depth, unsupported)) = pending.pop() {
        visited += 1;
        charge_bookmark_work(work, 1)?;
        if visited.saturating_add(pending.len()) > MAX_LAYOUT_ITEMS || depth > 32 {
            return Err(limit(
                "navigation display traversal exceeds its item/depth budget",
            ));
        }
        match draw {
            Draw::Glyphs {
                link: Some(link),
                glyphs,
                ..
            } if !glyphs.is_empty() => {
                if unsupported {
                    return Err(EngineError::Unsupported("internal navigation cannot preserve links in clipped, turned, rotated or figure display contexts at this native revision".into()).into());
                }
                if let Some(name) = link.strip_prefix('#') {
                    let cost = target_work.get(name).ok_or_else(|| {
                        PyValueError::new_err("linked bookmark has no checked native lookup budget")
                    })?;
                    // Native export resolves a caret again for every final annotation.
                    // Counting each source batch is conservative because coalescing
                    // can reduce the exported annotation count, never increase it.
                    charge_bookmark_work(work, *cost)?;
                }
                linked += 1;
                if linked > 10_000 {
                    return Err(limit("navigation exceeds 10000 linked native glyph batches on one page; native PDF export could omit annotations"));
                }
            }
            Draw::Clip { items, .. } | Draw::Turned { items, .. } | Draw::Rotated { items, .. } => {
                if visited
                    .saturating_add(pending.len())
                    .saturating_add(items.len())
                    > MAX_LAYOUT_ITEMS
                {
                    return Err(limit("navigation display traversal exceeds 2000000 items"));
                }
                pending.extend(items.iter().map(|item| (item, depth + 1, true)))
            }
            Draw::Figure { draws, .. } => {
                if visited
                    .saturating_add(pending.len())
                    .saturating_add(draws.len())
                    > MAX_LAYOUT_ITEMS
                {
                    return Err(limit("navigation display traversal exceeds 2000000 items"));
                }
                pending.extend(draws.iter().map(|item| (item, depth + 1, true)))
            }
            _ => {}
        }
    }
    Ok(linked)
}

fn charge_bookmark_work(work: &mut usize, amount: usize) -> PyResult<()> {
    let next = work
        .checked_add(amount)
        .filter(|next| *next <= MAX_BOOKMARK_WORK)
        .ok_or_else(|| limit("bookmark lookup and geometry scans exceed 2000000 work units"))?;
    *work = next;
    Ok(())
}

// Native body caret() materializes the public index, visits its pieces, then
// scans each candidate's lines and a selected line's clusters. Export also
// searches that index for the destination's paragraph edge. Charge public
// lengths conservatively before calling the native helper, without duplicating
// its line selection, bidi positions or destination calculation.
fn bookmark_lookup_work(
    layout: &wordcraft_layout::DocLayout,
    path: &Path,
    work: &mut usize,
) -> PyResult<usize> {
    let initial = *work;
    charge_bookmark_work(work, 1)?;
    if let Some(pieces) = layout.index.get(&(StoryRef::Body, path.clone())) {
        charge_bookmark_work(work, pieces.len().saturating_mul(3))?;
        for (page, item) in pieces {
            if let Some(wordcraft_layout::Placed::Lines { para, .. }) = layout
                .pages
                .get(*page)
                .and_then(|page| page.items.get(*item))
            {
                charge_bookmark_work(work, para.lines.len())?;
                charge_bookmark_work(work, para.clusters.len())?;
            }
        }
    }
    Ok(*work - initial)
}

// The pinned native vertical-text display drops link metadata, so a draw-only
// count cannot detect it. Repeated placements must also pay for source-run scans.
fn validate_bookmark_source_links(
    document: &Document,
    page: &wordcraft_layout::Page,
    work: &mut usize,
) -> PyResult<()> {
    charge_bookmark_work(
        work,
        page.items
            .len()
            .saturating_add(page.header.len())
            .saturating_add(page.footer.len()),
    )?;
    for item in page.items.iter().chain(&page.header).chain(&page.footer) {
        if let wordcraft_layout::Placed::Lines {
            story, path, turn, ..
        } = item
        {
            if *turn != TextDirection::Horizontal {
                if let Some(paragraph) = document.para(*story, path) {
                    charge_bookmark_work(work, paragraph.runs.len())?;
                    if paragraph.runs.iter().any(|run| run.props.link.is_some()) {
                        return Err(EngineError::Unsupported("internal navigation cannot preserve links in turned table text at this native revision".into()).into());
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_bookmark_layout(
    document: &Document,
    layout: &wordcraft_layout::DocLayout,
    spans: &BTreeMap<String, BookmarkSpan>,
) -> PyResult<()> {
    let mut work = 0usize;
    let mut target_work = BTreeMap::new();
    for (name, span) in spans {
        let cost = bookmark_lookup_work(layout, &span.path, &mut work)?;
        target_work.insert(name.clone(), cost);
        let caret = layout.caret(&Pos::new(StoryRef::Body, span.path.clone(), span.start)).ok_or_else(|| EngineError::Unsupported("bookmark has no native layout caret; paragraph fallback would change its destination".into()))?;
        let page = layout
            .pages
            .get(caret.page)
            .ok_or_else(|| PyValueError::new_err("bookmark destination page no longer exists"))?;
        if ![caret.x, caret.top, page.w, page.h]
            .iter()
            .all(|v| v.is_finite())
            || caret.x < 0.0
            || caret.x > page.w
            || caret.top < 0.0
            || caret.top > page.h
        {
            return Err(EngineError::Unsupported(
                "bookmark destination is outside the physical page".into(),
            )
            .into());
        }
        charge_bookmark_work(&mut work, page.items.len())?;
        for item in &page.items {
            match item {
                wordcraft_layout::Placed::Cell {
                    story: StoryRef::Body,
                    table,
                    row,
                    cell,
                    clip: true,
                    ..
                } => {
                    let prefix = [table.0.as_slice(), &[*row as u32, *cell as u32]].concat();
                    if span.path.0.starts_with(&prefix) {
                        return Err(EngineError::Unsupported(
                            "bookmark targets in clipped table cells are not supported".into(),
                        )
                        .into());
                    }
                }
                wordcraft_layout::Placed::Lines {
                    story: StoryRef::Body,
                    path,
                    turn,
                    ..
                } if path == &span.path && *turn != TextDirection::Horizontal => {
                    return Err(EngineError::Unsupported(
                        "bookmark targets in turned table text are not supported".into(),
                    )
                    .into())
                }
                _ => {}
            }
        }
    }
    let options = wordcraft_layout::display::DisplayOptions {
        marks: false,
        dim_header: false,
        dim_body: false,
        markup: false,
        placeholders: false,
        ..Default::default()
    };
    for page in &layout.pages {
        validate_bookmark_source_links(document, page, &mut work)?;
        validate_bookmark_draws(
            &wordcraft_layout::display::page_display(document, page, &options),
            &target_work,
            &mut work,
        )?;
    }
    Ok(())
}

/// The native conversion scans all reference offsets for each affected run.
/// Charge that product before its otherwise linear all-story operation. Public
/// para_paths is complete for the adapter's existing <=16-level authored tables.
fn validate_note_conversion_work(document: &Document, target: NoteKind) -> PyResult<()> {
    let source = if target == NoteKind::Footnote {
        PartKind::Endnote
    } else {
        PartKind::Footnote
    };
    let converted: BTreeSet<_> = document
        .parts
        .iter()
        .filter_map(|(id, part)| (part.kind == source).then_some(*id))
        .collect();
    let mut work = 0usize;
    for story in
        std::iter::once(StoryRef::Body).chain(document.parts.keys().copied().map(StoryRef::Part))
    {
        for path in document.para_paths(story) {
            let paragraph = document.para(story, &path).ok_or_else(|| {
                PyValueError::new_err("note conversion encountered an invalid paragraph path")
            })?;
            let references = paragraph.objects.iter().filter(|object| {
                matches!(object, InlineObject::NoteRef { id, .. } if converted.contains(id))
            }).count();
            work = work
                .saturating_add(1)
                .saturating_add(paragraph.objects.len());
            if references != 0 || matches!(story, StoryRef::Part(id) if converted.contains(&id)) {
                work = work
                    .saturating_add(paragraph.runs.len())
                    .saturating_add(references.saturating_mul(paragraph.runs.len()));
            }
            if work > MAX_NOTE_CONVERSION_WORK {
                return Err(limit(
                    "note conversion exceeds 2000000 paragraph/object/run comparison units",
                ));
            }
            if references != 0 {
                // Native conversion swaps the style of the entire containing run.
                // Permit that only when it cannot restyle text or unrelated objects.
                let offsets = paragraph.object_offsets();
                if offsets.len() != paragraph.objects.len() {
                    return Err(PyValueError::new_err(
                        "note conversion encountered mismatched object offsets",
                    ));
                }
                let selected: BTreeSet<_> = offsets
                    .into_iter()
                    .zip(&paragraph.objects)
                    .filter_map(|(offset, object)| {
                        matches!(object, InlineObject::NoteRef { id, .. } if converted.contains(id))
                            .then_some(offset)
                    })
                    .collect();
                let mut start = 0usize;
                for run in &paragraph.runs {
                    let end = start
                        .checked_add(run.len)
                        .filter(|end| *end <= paragraph.text.len())
                        .ok_or_else(|| {
                            PyValueError::new_err(
                                "note conversion encountered an invalid formatting run",
                            )
                        })?;
                    if matches!(
                        run.props.style.as_deref(),
                        Some(
                            "FootnoteText"
                                | "EndnoteText"
                                | "FootnoteReference"
                                | "EndnoteReference"
                        )
                    ) {
                        let count = selected.range(start..end).count();
                        if count != 0
                            && count.saturating_mul(wordcraft_doc::para::OBJ.len_utf8()) != run.len
                        {
                            return Err(EngineError::Unsupported("native note conversion would restyle ordinary text or an unrelated marker in a shared standard-style run; use a custom style or separate formatting first".into()).into());
                        }
                    }
                    start = end;
                }
            }
        }
    }
    Ok(())
}

/// Conversion alone may change note kinds. Require every original part identity
/// and the exact full semantic-anchor multiset, with only that known kind change.
fn validate_note_conversion_anchors(
    before: &Document,
    after: &Document,
    target: NoteKind,
) -> PyResult<()> {
    let error = || {
        EngineError::Unsupported(
            "note conversion must preserve owned part IDs and every semantic anchor".into(),
        )
    };
    if before.parts.len() != after.parts.len() {
        return Err(error().into());
    }
    let target_part = if target == NoteKind::Footnote {
        PartKind::Footnote
    } else {
        PartKind::Endnote
    };
    for (id, part) in &before.parts {
        let expected = if matches!(part.kind, PartKind::Footnote | PartKind::Endnote) {
            target_part
        } else {
            part.kind
        };
        if after.parts.get(id).is_none_or(|next| next.kind != expected) {
            return Err(error().into());
        }
    }
    let mut expected = BTreeMap::new();
    for ((kind, name), count) in semantic_anchors(before) {
        let next_kind = if kind <= 1 {
            let id: u32 = name.parse().map_err(|_| error())?;
            let part = before.parts.get(&id).ok_or_else(error)?;
            let source = if kind == 0 {
                PartKind::Footnote
            } else {
                PartKind::Endnote
            };
            if part.kind != source {
                return Err(error().into());
            }
            u8::from(target == NoteKind::Endnote)
        } else {
            kind
        };
        if expected.insert((next_kind, name), count).is_some() {
            return Err(error().into());
        }
    }
    if expected != semantic_anchors(after) {
        return Err(error().into());
    }
    Ok(())
}

fn semantic_anchors(document: &Document) -> BTreeMap<(u8, String), usize> {
    let mut anchors = BTreeMap::new();
    let mut pending: Vec<&Block> = document
        .body
        .iter()
        .chain(document.parts.values().flat_map(|part| part.blocks.iter()))
        .map(AsRef::as_ref)
        .collect();
    while let Some(block) = pending.pop() {
        match block {
            Block::Para(paragraph) => {
                for object in &paragraph.objects {
                    let key = match object {
                        InlineObject::NoteRef { kind, id, .. } => (
                            if *kind == NoteKind::Footnote { 0 } else { 1 },
                            id.to_string(),
                        ),
                        InlineObject::BookmarkStart { name } => (2, name.clone()),
                        InlineObject::BookmarkEnd { name } => (3, name.clone()),
                        InlineObject::CommentStart { id } => (4, id.to_string()),
                        InlineObject::CommentEnd { id } => (5, id.to_string()),
                        _ => continue,
                    };
                    *anchors.entry(key).or_insert(0) += 1;
                }
            }
            Block::Table(table) => pending.extend(
                table
                    .rows
                    .iter()
                    .flat_map(|row| row.cells.iter())
                    .flat_map(|cell| cell.blocks.iter())
                    .map(AsRef::as_ref),
            ),
        }
    }
    anchors
}

const TABLE_REGIONS: &[(&str, TablePart)] = &[
    ("whole_table", TablePart::WholeTable),
    ("odd_columns", TablePart::Band1Vert),
    ("even_columns", TablePart::Band2Vert),
    ("odd_rows", TablePart::Band1Horz),
    ("even_rows", TablePart::Band2Horz),
    ("first_column", TablePart::FirstCol),
    ("last_column", TablePart::LastCol),
    ("first_row", TablePart::FirstRow),
    ("last_row", TablePart::LastRow),
    ("top_right", TablePart::NeCell),
    ("top_left", TablePart::NwCell),
    ("bottom_right", TablePart::SeCell),
    ("bottom_left", TablePart::SwCell),
];

fn table_region(name: &str) -> PyResult<TablePart> {
    TABLE_REGIONS
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, part)| *part)
        .ok_or_else(|| PyValueError::new_err("unknown table style region"))
}

fn table_style_reference(styles: &StyleSheet, value: &Value) -> PyResult<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let name = value.as_str().ok_or_else(|| {
        PyValueError::new_err("table style requires a style name, identifier or None")
    })?;
    let style = styles
        .find(name)
        .filter(|s| s.kind == StyleKind::Table)
        .ok_or_else(|| PyValueError::new_err("unknown table style"))?;
    Ok(Some(style.id.clone()))
}

fn table_look_snapshot(look: &wordcraft_doc::props::TableLook) -> Value {
    json!({"header_row":look.header_row,"total_row":look.total_row,"banded_rows":look.banded_rows,
        "first_column":look.first_column,"last_column":look.last_column,"banded_columns":look.banded_columns})
}

fn patch_table_look(look: &mut wordcraft_doc::props::TableLook, value: &Value) -> PyResult<()> {
    if value.is_null() {
        *look = Default::default();
        return Ok(());
    }
    let values = value
        .as_object()
        .ok_or_else(|| PyValueError::new_err("look requires a flag mapping or None"))?;
    for (key, value) in values {
        let target = match key.as_str() {
            "header_row" => &mut look.header_row,
            "total_row" => &mut look.total_row,
            "banded_rows" => &mut look.banded_rows,
            "first_column" => &mut look.first_column,
            "last_column" => &mut look.last_column,
            "banded_columns" => &mut look.banded_columns,
            _ => return Err(PyValueError::new_err("unknown table look flag")),
        };
        *target = strict_bool(value, key)?;
    }
    Ok(())
}

fn patch_table_style(
    style: &mut Style,
    styles: &StyleSheet,
    part: TablePart,
    values: Map<String, Value>,
) -> PyResult<()> {
    let whole = part == TablePart::WholeTable;
    let mut format = style.table_part(part);
    let (mut character, mut paragraph) = (Map::new(), Map::new());
    let (mut margins, mut band_size) = (None, None);
    for (key, value) in values {
        match key.as_str() {
            "background" => {
                format.fill = if value.is_null() {
                    None
                } else {
                    Some(parse_rgb(value.as_str().ok_or_else(|| {
                        PyValueError::new_err("background requires a color or None")
                    })?)?)
                }
            }
            "alignment" => format.align = nullable_alignment(&value, true)?,
            "vertical_alignment" => {
                format.v_align = match value.as_str() {
                    None if value.is_null() => None,
                    Some("top") => Some(VAlign::Top),
                    Some("center") => Some(VAlign::Center),
                    Some("bottom") => Some(VAlign::Bottom),
                    _ => {
                        return Err(PyValueError::new_err(
                            "vertical_alignment must be top, center, bottom or None",
                        ))
                    }
                }
            }
            "borders" => {
                apply_border_patch(&mut format.borders, &value, true)?;
                // Native region_borders maps these edges into per-cell left/right,
                // but layout_table reads rb.inside_v at interior boundaries. It drops
                // visible borders and explicit nil suppression alike. Clearing a
                // direct side/group remains safe; whole-table borders are unaffected.
                if value.as_object().is_some_and(|sides| {
                    sides.iter().any(|(side, border)| {
                        !border.is_null()
                            && match part {
                                TablePart::FirstRow
                                | TablePart::LastRow
                                | TablePart::Band1Horz
                                | TablePart::Band2Horz => side == "inside_vertical",
                                TablePart::FirstCol | TablePart::NwCell | TablePart::SwCell => {
                                    side == "right"
                                }
                                TablePart::LastCol | TablePart::NeCell | TablePart::SeCell => {
                                    side == "left"
                                }
                                TablePart::Band1Vert | TablePart::Band2Vert => {
                                    matches!(side.as_str(), "left" | "right")
                                }
                                TablePart::WholeTable => false,
                            }
                    })
                }) {
                    return Err(EngineError::Unsupported(
                        "native DOCX conditional table styles cannot preserve this interior vertical border; use direct cell or whole-table borders".into(),
                    ).into());
                }
            }
            "margins" if whole => {
                margins = Some(if value.is_null() {
                    None
                } else {
                    let values = if value.is_number() {
                        json!([value, value, value, value])
                    } else {
                        value
                    };
                    let [top, right, bottom, left] = section_numbers::<4>(&values, "margins")?;
                    if [top, right, bottom, left].iter().any(|v| *v < 0.0) {
                        return Err(PyValueError::new_err(
                            "table style margins must be nonnegative",
                        ));
                    }
                    Some([top, left, bottom, right])
                });
            }
            "row_band_size" if whole => {
                band_size = Some(if value.is_null() {
                    None
                } else {
                    Some(
                        value
                            .as_u64()
                            .filter(|n| (1..=1000).contains(n))
                            .ok_or_else(|| {
                                PyValueError::new_err(
                                    "row_band_size requires an integer from 1 to 1000 or None",
                                )
                            })? as u32,
                    )
                });
            }
            _ if is_paragraph_key(&key) => {
                if !whole {
                    return Err(PyValueError::new_err(
                        "conditional table regions support only paragraph alignment",
                    ));
                }
                paragraph.insert(key, value);
            }
            _ => {
                character.insert(key, value);
            }
        }
    }
    CharPatch::parse(styles, &character, false)?.apply(&mut format.chr);
    ParaPatch::parse(&paragraph)?.apply(&mut style.para);
    style.set_table_part(part, format);
    if margins.is_some() || band_size.is_some() {
        let table = style.table.get_or_insert_with(Default::default);
        if let Some(value) = margins {
            table.cell_margins = value;
        }
        if let Some(value) = band_size {
            table.band_size = value;
        }
        if *table == Default::default() {
            style.table = None;
        }
    }
    Ok(())
}

fn table_part_snapshot(style: &Style, part: TablePart) -> PyResult<Map<String, Value>> {
    let format = style.table_part(part);
    let mut values = char_snapshot(&format.chr);
    values.remove("style");
    if part == TablePart::WholeTable {
        values.extend(para_snapshot(&style.para)?);
        values.insert(
            "margins".into(),
            json!(style
                .table
                .as_ref()
                .and_then(|t| t.cell_margins)
                .map(|[t, l, b, r]| [t, r, b, l])),
        );
        values.insert(
            "row_band_size".into(),
            json!(style.table.as_ref().and_then(|t| t.band_size)),
        );
    }
    values.insert("background".into(), json!(format.fill.map(rgb_string)));
    values.insert("alignment".into(), json!(format.align.map(alignment_name)));
    values.insert(
        "vertical_alignment".into(),
        json!(format.v_align.map(vertical_alignment_name)),
    );
    values.insert("borders".into(), borders_snapshot(format.borders, true));
    Ok(values)
}

fn table_style_snapshot(
    styles: &StyleSheet,
    style: &Style,
    resolved: bool,
    region: Option<&str>,
) -> PyResult<Value> {
    let merged;
    let style = if resolved {
        let native = styles
            .table_style(&style.id)
            .ok_or_else(|| PyValueError::new_err("unknown table style"))?;
        merged = Style {
            chr: native.chr,
            para: native.para,
            table: Some(native.parts),
            ..style.clone()
        };
        &merged
    } else {
        style
    };
    let mut values = table_part_snapshot(style, table_region(region.unwrap_or("whole_table"))?)?;
    if region.is_none() {
        let regions = TABLE_REGIONS
            .iter()
            .filter(|(_, part)| *part != TablePart::WholeTable)
            .map(|(name, part)| {
                table_part_snapshot(style, *part).map(|v| (name.to_string(), Value::Object(v)))
            })
            .collect::<PyResult<Map<String, Value>>>()?;
        values.insert("regions".into(), Value::Object(regions));
    }
    Ok(Value::Object(values))
}

fn style_kind(kind: &str) -> PyResult<StyleKind> {
    match kind {
        "paragraph" => Ok(StyleKind::Paragraph),
        "character" => Ok(StyleKind::Character),
        "table" => Ok(StyleKind::Table),
        _ => Err(PyValueError::new_err(
            "style kind must be paragraph, character or table",
        )),
    }
}

fn supported_style<'a>(styles: &'a StyleSheet, id: &str) -> PyResult<&'a Style> {
    styles
        .find(id)
        .filter(|style| {
            matches!(
                style.kind,
                StyleKind::Paragraph | StyleKind::Character | StyleKind::Table
            )
        })
        .ok_or_else(|| PyValueError::new_err("unknown paragraph, character or table style"))
}

fn style_base(
    styles: &StyleSheet,
    base: Option<&str>,
    kind: StyleKind,
) -> PyResult<Option<String>> {
    base.map(|base| {
        let style = supported_style(styles, base)?;
        if style.kind != kind {
            return Err(PyValueError::new_err(
                "a style's base must have the same kind",
            ));
        }
        Ok(style.id.clone())
    })
    .transpose()
}

fn validate_style_name(name: &str) -> PyResult<()> {
    validate_text(name)?;
    if name.trim().is_empty()
        || name.len() > 255
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(PyValueError::new_err(
            "style name requires 1..255 bytes, no control characters or surrounding whitespace",
        ));
    }
    Ok(())
}

fn validate_style_chains(styles: &StyleSheet) -> PyResult<()> {
    let by_id = styles
        .styles
        .iter()
        .map(|style| (style.id.as_str(), style))
        .collect::<BTreeMap<_, _>>();
    for style in &styles.styles {
        let mut seen = BTreeSet::new();
        let mut next = Some(style.id.as_str());
        while let Some(id) = next {
            if !seen.insert(id) {
                return Err(PyValueError::new_err(
                    "style inheritance cannot contain a cycle",
                ));
            }
            if seen.len() > 32 {
                return Err(PyValueError::new_err(
                    "style inheritance cannot exceed 32 levels",
                ));
            }
            next = by_id
                .get(id)
                .ok_or_else(|| {
                    PyValueError::new_err("style inheritance references a missing style")
                })?
                .based_on
                .as_deref();
        }
    }
    Ok(())
}

const TAB_ALIGNMENTS: &[(&str, TabAlign)] = &[
    ("left", TabAlign::Left),
    ("center", TabAlign::Center),
    ("right", TabAlign::Right),
    ("decimal", TabAlign::Decimal),
    ("clear", TabAlign::Clear),
];
const TAB_LEADERS: &[(&str, TabLeader)] = &[
    ("none", TabLeader::None),
    ("dot", TabLeader::Dot),
    ("hyphen", TabLeader::Hyphen),
    ("underscore", TabLeader::Underscore),
    ("middle_dot", TabLeader::MiddleDot),
];

fn parse_tabs(value: &Value) -> PyResult<Vec<TabStop>> {
    let values = value
        .as_array()
        .ok_or_else(|| PyValueError::new_err("tabs requires a list or None"))?;
    if values.len() > MAX_TAB_STOPS {
        return Err(PyValueError::new_err("tabs cannot exceed 64 direct stops"));
    }
    let mut tabs: Vec<TabStop> = Vec::with_capacity(values.len());
    for value in values {
        let entry = value
            .as_object()
            .ok_or_else(|| PyValueError::new_err("each tab stop requires a property mapping"))?;
        if entry
            .keys()
            .any(|key| !matches!(key.as_str(), "position" | "alignment" | "leader"))
        {
            return Err(PyValueError::new_err("unknown tab stop property"));
        }
        let position = entry
            .get("position")
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite() && (-1584.0..=1584.0).contains(v))
            .ok_or_else(|| {
                PyValueError::new_err(
                    "tab position must be finite and between -1584 and 1584 points",
                )
            })?;
        let twips = position * 20.0;
        if (twips - twips.round()).abs() > 0.000001 {
            return Err(PyValueError::new_err(
                "tab position requires 0.05-point increments",
            ));
        }
        let alignment = entry
            .get("alignment")
            .map(|v| v.as_str())
            .unwrap_or(Some("left"));
        let align=TAB_ALIGNMENTS.iter().find(|(name,value)|Some(*name)==alignment && *value != TabAlign::Decimal).map(|(_,value)|*value)
            .ok_or_else(||PyValueError::new_err("tab alignment must be left, center, right or clear; decimal and bar tabs are not correctly rendered upstream"))?;
        let leader = entry
            .get("leader")
            .map(|v| v.as_str())
            .unwrap_or(Some("none"));
        let leader = TAB_LEADERS
            .iter()
            .find(|(name, _)| Some(*name) == leader)
            .map(|(_, value)| *value)
            .ok_or_else(|| {
                PyValueError::new_err(
                    "tab leader must be none, dot, hyphen, underscore or middle_dot",
                )
            })?;
        if align == TabAlign::Clear && leader != TabLeader::None {
            return Err(PyValueError::new_err(
                "clearing an inherited tab requires leader none",
            ));
        }
        let pos = (twips.round() / 20.0) as f32;
        if tabs.iter().any(|tab| (tab.pos - pos).abs() < TAB_POS_EPS) {
            return Err(PyValueError::new_err(
                "tab positions must differ by at least 0.5 points",
            ));
        }
        tabs.push(TabStop { pos, align, leader });
    }
    tabs.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    Ok(tabs)
}

fn tabs_snapshot(tabs: &[TabStop]) -> Value {
    Value::Array(tabs.iter().map(|tab|json!({"position":tab.pos,
        "alignment":TAB_ALIGNMENTS.iter().find(|(_,value)|*value==tab.align).map_or("bar",|(name,_)|*name),
        "leader":TAB_LEADERS.iter().find(|(_,value)|*value==tab.leader).map_or("none",|(name,_)|*name),
    })).collect())
}

fn paragraph_table_context(document: &Document, story: StoryRef, path: &Path) -> Option<ParaProps> {
    let (path, row, cell) = path.cell()?;
    let table = document.table(story, &path)?;
    let style = document.styles.table_style(table.props.style.as_deref()?)?;
    let current = table.rows.get(row)?.cells.get(cell)?;
    let regions = PartSet::of_cell(
        &table.props.look,
        style.parts.band_size.unwrap_or(1),
        (row, table.rows.len()),
        (table.grid_col(row, cell), current.span(), table.cols()),
    );
    let mut merged = PartFormat::default();
    for part in regions.iter() {
        merged.overlay(&style.part(part));
    }
    let mut paragraph = style.para;
    paragraph.align = merged.align.or(paragraph.align);
    Some(paragraph)
}

fn check_tab_count(tabs: Option<&[TabStop]>) -> PyResult<()> {
    if tabs.is_some_and(|tabs| tabs.len() > MAX_TAB_STOPS) {
        return Err(PyValueError::new_err(
            "direct and effective tab stops cannot exceed 64",
        ));
    }
    Ok(())
}

fn validate_document_tabs(document: &Document) -> PyResult<()> {
    let styles = &document.styles;
    check_tab_count(styles.default_para.tabs.as_deref())?;
    let mut inherited = styles.default_para.tabs.is_some();
    for style in &styles.styles {
        check_tab_count(style.para.tabs.as_deref())?;
        inherited |= style.para.tabs.is_some();
        if style.kind == StyleKind::Paragraph {
            let resolved = styles.resolve_para(&ParaProps {
                style: Some(style.id.clone()),
                ..Default::default()
            });
            check_tab_count(Some(&resolved.tabs))?;
        } else if style.kind == StyleKind::Table {
            if let Some(resolved) = styles.table_style(&style.id) {
                check_tab_count(resolved.para.tabs.as_deref())?;
            }
        }
    }
    for story in
        std::iter::once(StoryRef::Body).chain(document.parts.keys().map(|id| StoryRef::Part(*id)))
    {
        for path in document.para_paths(story) {
            let paragraph = document
                .para(story, &path)
                .ok_or_else(|| PyValueError::new_err("paragraph no longer exists"))?;
            check_tab_count(paragraph.props.tabs.as_deref())?;
            if inherited || paragraph.props.tabs.is_some() {
                let table = paragraph_table_context(document, story, &path);
                let resolved = styles.resolve_para_in(&paragraph.props, table.as_ref());
                check_tab_count(Some(&resolved.tabs))?;
            }
        }
    }
    Ok(())
}

fn format_patch(input: &str) -> PyResult<Map<String, Value>> {
    if input.len() > MAX_FORMAT_PATCH_BYTES {
        return Err(limit("format patch exceeds 16 KiB"));
    }
    let value: Value = serde_json::from_str(input).map_err(|_| {
        PyValueError::new_err("format patch must be a JSON object of finite scalar values")
    })?;
    let Value::Object(values) = value else {
        return Err(PyValueError::new_err("format patch must be a JSON object"));
    };
    if values.iter().any(|(key, value)| {
        (value.is_object() && key != "borders") || (value.is_array() && key != "tabs")
    }) {
        return Err(PyValueError::new_err(
            "format patch values must be scalar, except the tabs list and borders mapping",
        ));
    }
    Ok(values)
}

fn bool_value(value: &Value) -> PyResult<Option<bool>> {
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_bool()
            .map(Some)
            .ok_or_else(|| PyValueError::new_err("format flag must be a bool or None"))
    }
}

fn text_value(value: &Value) -> PyResult<&str> {
    value
        .as_str()
        .ok_or_else(|| PyValueError::new_err("format value must be a string or None"))
}

fn point_value(value: &Value, low: f64, high: f64) -> PyResult<f32> {
    let number = value
        .as_f64()
        .filter(|number| number.is_finite() && (low..=high).contains(number))
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "format number must be finite and within {low}..{high}"
            ))
        })?;
    Ok(number as f32)
}

const UNDERLINES: &[(&str, Underline)] = &[
    ("none", Underline::None),
    ("single", Underline::Single),
    ("words", Underline::Words),
    ("double", Underline::Double),
    ("thick", Underline::Thick),
    ("dotted", Underline::Dotted),
    ("dash", Underline::Dash),
    ("dot_dash", Underline::DotDash),
    ("dot_dot_dash", Underline::DotDotDash),
    ("wave", Underline::Wave),
    ("double_wave", Underline::DoubleWave),
];
const HIGHLIGHTS: &[(&str, Highlight)] = &[
    ("none", Highlight::None),
    ("yellow", Highlight::Yellow),
    ("bright_green", Highlight::BrightGreen),
    ("turquoise", Highlight::Turquoise),
    ("pink", Highlight::Pink),
    ("blue", Highlight::Blue),
    ("red", Highlight::Red),
    ("dark_blue", Highlight::DarkBlue),
    ("teal", Highlight::Teal),
    ("green", Highlight::Green),
    ("violet", Highlight::Violet),
    ("dark_red", Highlight::DarkRed),
    ("dark_yellow", Highlight::DarkYellow),
    ("gray50", Highlight::Gray50),
    ("gray25", Highlight::Gray25),
    ("black", Highlight::Black),
    ("white", Highlight::White),
];

// DOCX stores character spacing in integer twips. Accept only that precision,
// with a few f64 ULPs for ordinary arithmetic such as 0.1 + 0.2; retain the
// canonical value before conversion to the native f32 model.
fn letter_spacing(value: &Value) -> PyResult<f32> {
    let points = value
        .as_f64()
        .ok_or_else(|| PyValueError::new_err("letter_spacing must be a number or None"))?;
    if !points.is_finite() || !(-100.0..=100.0).contains(&points) {
        return Err(PyValueError::new_err(
            "letter_spacing must be finite and between -100 and 100 points",
        ));
    }
    let canonical = (points * 20.0).round() / 20.0;
    if (canonical - points).abs() > 4.0 * f64::EPSILON * points.abs().max(1.0) {
        return Err(PyValueError::new_err(
            "letter_spacing requires 0.05-point (one-twip) increments",
        ));
    }
    Ok(canonical as f32)
}

fn spacing_points(value: f32) -> f64 {
    f64::from((value * 20.0).round()) / 20.0
}

#[derive(Default)]
struct CharPatch {
    values: CharProps,
    clear: BTreeSet<String>,
}

impl CharPatch {
    fn parse(
        styles: &StyleSheet,
        values: &Map<String, Value>,
        allow_style: bool,
    ) -> PyResult<Self> {
        let mut patch = Self::default();
        for (key, value) in values {
            if !matches!(
                key.as_str(),
                "bold"
                    | "italic"
                    | "font"
                    | "font_size"
                    | "language"
                    | "letter_spacing"
                    | "color"
                    | "underline"
                    | "strike"
                    | "script"
                    | "highlight"
                    | "background"
                    | "style"
            ) || (key == "style" && !allow_style)
            {
                return Err(PyValueError::new_err(format!(
                    "unsupported character format key: {key}"
                )));
            }
            if value.is_null() {
                patch.clear.insert(key.clone());
                continue;
            }
            match key.as_str() {
                "bold" => patch.values.bold = bool_value(value)?,
                "italic" => patch.values.italic = bool_value(value)?,
                "letter_spacing" => patch.values.spacing = Some(letter_spacing(value)?),
                "language" => {
                    let tag = text_value(value)?;
                    if tag.is_empty()
                        || tag.len() > 63
                        || !tag.split('-').all(|part| {
                            !part.is_empty()
                                && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
                        })
                    {
                        return Err(PyValueError::new_err("language must be a 1..63-byte ASCII alphanumeric tag with single hyphen separators"));
                    }
                    patch.values.lang = Some(tag.to_owned());
                }
                "strike" => {
                    let (single, double) = match (value.as_bool(), value.as_str()) {
                        (Some(enabled), _) => (enabled, false),
                        (_, Some("none")) => (false, false),
                        (_, Some("single")) => (true, false),
                        (_, Some("double")) => (false, true),
                        _ => {
                            return Err(PyValueError::new_err(
                                "strike must be bool, none, single, double or None",
                            ))
                        }
                    };
                    patch.values.strike = Some(single);
                    patch.values.double_strike = Some(double);
                }
                "font" => {
                    patch.values.font =
                        run_props(None, None, None, Some(text_value(value)?), None)?.font
                }
                "font_size" => {
                    patch.values.size =
                        run_props(
                            None,
                            None,
                            Some(value.as_f64().ok_or_else(|| {
                                PyValueError::new_err("font_size must be numeric")
                            })?),
                            None,
                            None,
                        )?
                        .size
                }
                "color" => {
                    patch.values.color = Some(if text_value(value)? == "auto" {
                        TextColor::Auto
                    } else {
                        TextColor::Rgb(parse_rgb(text_value(value)?)?)
                    })
                }
                "background" => patch.values.shading = Some(parse_rgb(text_value(value)?)?),
                "underline" => {
                    patch.values.underline = Some(if let Some(enabled) = value.as_bool() {
                        if enabled {
                            Underline::Single
                        } else {
                            Underline::None
                        }
                    } else {
                        UNDERLINES
                            .iter()
                            .find(|(name, _)| Some(*name) == value.as_str())
                            .map(|(_, value)| *value)
                            .ok_or_else(|| PyValueError::new_err("unknown underline style"))?
                    })
                }
                "highlight" => {
                    patch.values.highlight = Some(
                        HIGHLIGHTS
                            .iter()
                            .find(|(name, _)| Some(*name) == value.as_str())
                            .map(|(_, value)| *value)
                            .ok_or_else(|| PyValueError::new_err("unknown highlight color"))?,
                    )
                }
                "script" => {
                    patch.values.vert_align = Some(match text_value(value)? {
                        "baseline" => VertAlign::Baseline,
                        "superscript" => VertAlign::Superscript,
                        "subscript" => VertAlign::Subscript,
                        _ => {
                            return Err(PyValueError::new_err(
                                "script must be baseline, superscript or subscript",
                            ))
                        }
                    })
                }
                "style" => {
                    let style = supported_style(styles, text_value(value)?)?;
                    if style.kind != StyleKind::Character {
                        return Err(PyValueError::new_err("run style must be a character style"));
                    }
                    patch.values.style = Some(style.id.clone());
                }
                _ => unreachable!(),
            }
        }
        Ok(patch)
    }

    fn apply(&self, props: &mut CharProps) {
        for key in &self.clear {
            match key.as_str() {
                "bold" => props.bold = None,
                "italic" => props.italic = None,
                "font" => props.font = None,
                "language" => props.lang = None,
                "letter_spacing" => props.spacing = None,
                "font_size" => props.size = None,
                "color" => props.color = None,
                "underline" => props.underline = None,
                "strike" => {
                    props.strike = None;
                    props.double_strike = None;
                }
                "script" => props.vert_align = None,
                "highlight" => props.highlight = None,
                "background" => props.shading = None,
                "style" => props.style = None,
                _ => {}
            }
        }
        props.overlay(&self.values);
    }
}

fn is_paragraph_key(key: &str) -> bool {
    matches!(
        key,
        "alignment"
            | "direction"
            | "indent_left"
            | "indent_right"
            | "indent_first"
            | "space_before"
            | "space_after"
            | "line_spacing"
            | "keep_with_next"
            | "keep_together"
            | "widow_control"
            | "page_break_before"
            | "background"
            | "tabs"
            | "borders"
    )
}

#[derive(Default)]
struct ParaPatch {
    values: ParaProps,
    clear: BTreeSet<String>,
    border_sides: BTreeMap<String, Option<Border>>,
}

impl ParaPatch {
    fn parse(values: &Map<String, Value>) -> PyResult<Self> {
        let mut patch = Self::default();
        for (key, value) in values {
            if !is_paragraph_key(key) {
                return Err(PyValueError::new_err(format!(
                    "unsupported paragraph format key: {key}"
                )));
            }
            if value.is_null() {
                patch.clear.insert(key.clone());
                continue;
            }
            match key.as_str() {
                "direction" => {
                    patch.values.bidi = Some(match text_value(value)? {
                        "rtl" => true,
                        "ltr" => false,
                        _ => {
                            return Err(PyValueError::new_err("direction must be rtl, ltr or None"))
                        }
                    });
                }
                "alignment" => {
                    patch.values.align = Some(match text_value(value)? {
                        "left" => Align::Left,
                        "right" => Align::Right,
                        "center" => Align::Center,
                        "justify" => Align::Justify,
                        "distribute" => Align::Distribute,
                        _ => return Err(PyValueError::new_err("unknown paragraph alignment")),
                    })
                }
                "indent_left" => {
                    patch.values.indent_left = Some(point_value(value, -1584.0, 1584.0)?);
                    patch.values.indent_left_chars = Some(0.0);
                }
                "indent_right" => {
                    patch.values.indent_right = Some(point_value(value, -1584.0, 1584.0)?);
                    patch.values.indent_right_chars = Some(0.0);
                }
                "indent_first" => {
                    patch.values.indent_first = Some(point_value(value, -1584.0, 1584.0)?);
                    patch.values.indent_first_chars = Some(0.0);
                }
                "space_before" => {
                    patch.values.space_before = Some(point_value(value, 0.0, 1584.0)?);
                    patch.values.space_before_auto = Some(false);
                }
                "space_after" => {
                    patch.values.space_after = Some(point_value(value, 0.0, 1584.0)?);
                    patch.values.space_after_auto = Some(false);
                }
                "line_spacing" => {
                    patch.values.line_spacing =
                        Some(LineSpacing::Multiple(point_value(value, 0.06, 132.0)?))
                }
                "keep_with_next" => patch.values.keep_next = bool_value(value)?,
                "keep_together" => patch.values.keep_lines = bool_value(value)?,
                "widow_control" => patch.values.widow_control = bool_value(value)?,
                "page_break_before" => patch.values.page_break_before = bool_value(value)?,
                "background" => patch.values.shading = Some(parse_rgb(text_value(value)?)?),
                "borders" => {
                    let sides = value.as_object().ok_or_else(|| {
                        PyValueError::new_err("borders requires a side mapping or None")
                    })?;
                    for (side, value) in sides {
                        if !matches!(side.as_str(), "top" | "right" | "bottom" | "left") {
                            return Err(PyValueError::new_err(
                                "paragraph borders support only top, right, bottom and left",
                            ));
                        }
                        let border = border_for_side(value, side)?;
                        patch.border_sides.insert(side.clone(), border);
                    }
                }
                "tabs" => {
                    let tabs = parse_tabs(value)?;
                    if tabs.is_empty() {
                        patch.clear.insert(key.clone());
                    } else {
                        patch.values.tabs = Some(tabs);
                    }
                }
                _ => unreachable!(),
            }
        }
        Ok(patch)
    }

    fn apply(&self, props: &mut ParaProps) {
        for key in &self.clear {
            match key.as_str() {
                "alignment" => props.align = None,
                "direction" => props.bidi = None,
                "indent_left" => {
                    props.indent_left = None;
                    props.indent_left_chars = None;
                }
                "indent_right" => {
                    props.indent_right = None;
                    props.indent_right_chars = None;
                }
                "indent_first" => {
                    props.indent_first = None;
                    props.indent_first_chars = None;
                }
                "space_before" => {
                    props.space_before = None;
                    props.space_before_auto = None;
                }
                "space_after" => {
                    props.space_after = None;
                    props.space_after_auto = None;
                }
                "line_spacing" => props.line_spacing = None,
                "keep_with_next" => props.keep_next = None,
                "keep_together" => props.keep_lines = None,
                "widow_control" => props.widow_control = None,
                "page_break_before" => props.page_break_before = None,
                "background" => props.shading = None,
                "tabs" => props.tabs = None,
                "borders" => props.borders = None,
                _ => {}
            }
        }
        props.overlay(&self.values);
        if !self.border_sides.is_empty() {
            let mut borders = props.borders.unwrap_or_default();
            for (side, border) in &self.border_sides {
                match side.as_str() {
                    "top" => borders.top = *border,
                    "right" => borders.right = *border,
                    "bottom" => borders.bottom = *border,
                    "left" => borders.left = *border,
                    _ => unreachable!("validated paragraph border side"),
                }
            }
            props.borders = (borders != Borders::default()).then_some(borders);
        }
    }
}

fn snapshot<const N: usize>(values: [(&str, Value); N]) -> Map<String, Value> {
    values
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect()
}
fn rgb_string(rgb: Rgb) -> String {
    format!("#{}", rgb.hex())
}
fn color_string(color: TextColor) -> String {
    match color {
        TextColor::Auto => "auto".into(),
        TextColor::Rgb(rgb) => rgb_string(rgb),
    }
}
fn underline_name(value: Underline) -> &'static str {
    UNDERLINES
        .iter()
        .find(|(_, underline)| *underline == value)
        .map_or("none", |(name, _)| name)
}
fn highlight_name(value: Highlight) -> &'static str {
    HIGHLIGHTS
        .iter()
        .find(|(_, highlight)| *highlight == value)
        .map_or("none", |(name, _)| name)
}
fn script_name(value: VertAlign) -> &'static str {
    match value {
        VertAlign::Baseline => "baseline",
        VertAlign::Superscript => "superscript",
        VertAlign::Subscript => "subscript",
    }
}
fn alignment_name(value: Align) -> &'static str {
    match value {
        Align::Left => "left",
        Align::Right => "right",
        Align::Center => "center",
        Align::Justify => "justify",
        Align::Distribute => "distribute",
    }
}
fn line_multiple(value: LineSpacing) -> PyResult<Value> {
    match value {
        LineSpacing::Multiple(multiple) => Ok(json!(multiple)),
        _ => Err(PyValueError::new_err(
            "point-based line spacing is not supported by this formatting snapshot",
        )),
    }
}

fn char_snapshot(props: &CharProps) -> Map<String, Value> {
    snapshot([
        ("bold", json!(props.bold)),
        ("italic", json!(props.italic)),
        ("font", json!(props.font)),
        ("language", json!(props.lang)),
        ("letter_spacing", json!(props.spacing.map(spacing_points))),
        ("font_size", json!(props.size)),
        ("color", json!(props.color.map(color_string))),
        ("underline", json!(props.underline.map(underline_name))),
        (
            "strike",
            if props.double_strike == Some(true) {
                json!("double")
            } else {
                json!(props.strike)
            },
        ),
        ("script", json!(props.vert_align.map(script_name))),
        ("highlight", json!(props.highlight.map(highlight_name))),
        ("background", json!(props.shading.map(rgb_string))),
        ("style", json!(props.style)),
    ])
}

fn resolved_char_snapshot(props: &wordcraft_doc::resolve::ResolvedChar) -> Map<String, Value> {
    snapshot([
        ("bold", json!(props.bold)),
        ("italic", json!(props.italic)),
        ("font", json!(props.font)),
        ("language", json!(props.lang)),
        ("letter_spacing", json!(spacing_points(props.spacing))),
        ("font_size", json!(props.size)),
        ("color", json!(color_string(props.color))),
        ("underline", json!(underline_name(props.underline))),
        (
            "strike",
            if props.double_strike {
                json!("double")
            } else {
                json!(props.strike)
            },
        ),
        ("script", json!(script_name(props.vert_align))),
        (
            "highlight",
            json!(HIGHLIGHTS
                .iter()
                .find(|(_, value)| value.rgb() == props.highlight)
                .map_or("none", |(name, _)| *name)),
        ),
        ("background", json!(props.shading.map(rgb_string))),
    ])
}

fn para_snapshot(props: &ParaProps) -> PyResult<Map<String, Value>> {
    Ok(snapshot([
        ("borders", borders_snapshot(props.borders, false)),
        (
            "tabs",
            props
                .tabs
                .as_deref()
                .map(tabs_snapshot)
                .unwrap_or(Value::Null),
        ),
        ("alignment", json!(props.align.map(alignment_name))),
        (
            "direction",
            json!(props.bidi.map(|rtl| if rtl { "rtl" } else { "ltr" })),
        ),
        ("indent_left", json!(props.indent_left)),
        ("indent_right", json!(props.indent_right)),
        ("indent_first", json!(props.indent_first)),
        ("space_before", json!(props.space_before)),
        ("space_after", json!(props.space_after)),
        (
            "line_spacing",
            props
                .line_spacing
                .map(line_multiple)
                .transpose()?
                .unwrap_or(Value::Null),
        ),
        ("keep_with_next", json!(props.keep_next)),
        ("keep_together", json!(props.keep_lines)),
        ("widow_control", json!(props.widow_control)),
        ("page_break_before", json!(props.page_break_before)),
        ("background", json!(props.shading.map(rgb_string))),
    ]))
}

fn resolved_para_snapshot(
    props: &wordcraft_doc::resolve::ResolvedPara,
) -> PyResult<Map<String, Value>> {
    Ok(snapshot([
        ("borders", borders_snapshot(props.borders, false)),
        ("tabs", tabs_snapshot(&props.tabs)),
        ("alignment", json!(alignment_name(props.align))),
        ("direction", json!(if props.bidi { "rtl" } else { "ltr" })),
        ("indent_left", json!(props.indent_left)),
        ("indent_right", json!(props.indent_right)),
        ("indent_first", json!(props.indent_first)),
        ("space_before", json!(props.space_before)),
        ("space_after", json!(props.space_after)),
        ("line_spacing", line_multiple(props.line_spacing)?),
        ("keep_with_next", json!(props.keep_next)),
        ("keep_together", json!(props.keep_lines)),
        ("widow_control", json!(props.widow_control)),
        ("page_break_before", json!(props.page_break_before)),
        ("background", json!(props.shading.map(rgb_string))),
    ]))
}

fn hyperlink_bytes(document: &Document) -> usize {
    std::iter::once(StoryRef::Body)
        .chain(document.parts.keys().map(|id| StoryRef::Part(*id)))
        .flat_map(|story| {
            document
                .para_paths(story)
                .into_iter()
                .filter_map(move |path| document.para(story, &path))
        })
        .flat_map(|paragraph| paragraph.runs.iter())
        .map(|run| run.props.link.as_ref().map_or(0, String::len))
        .fold(0, usize::saturating_add)
}

fn validate_text(text: &str) -> PyResult<()> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(limit("text value exceeds 1 MiB"));
    }
    if text.chars().any(|character| {
        character == '\u{fffc}'
            || character == '\r'
            || !(character == '\t'
                || character == '\n'
                || ('\u{20}'..='\u{d7ff}').contains(&character)
                || ('\u{e000}'..='\u{fffd}').contains(&character)
                || character >= '\u{10000}')
    }) {
        return Err(PyValueError::new_err(
            "text contains a reserved or unsupported XML character; use LF for line breaks",
        ));
    }
    Ok(())
}

fn limit(message: &str) -> PyErr {
    EngineError::ResourceLimit(message.into()).into()
}

fn model_error(error: wordcraft_doc::DocError) -> PyErr {
    PyValueError::new_err(format!("DOCX model operation failed: {error}"))
}

fn parse_rgb(value: &str) -> PyResult<Rgb> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|value| value.is_ascii_hexdigit())
    {
        return Err(PyValueError::new_err("color must be #RRGGBB"));
    }
    Rgb::parse(value).ok_or_else(|| PyValueError::new_err("color must be #RRGGBB"))
}

fn validate_alt(alt: &str) -> PyResult<()> {
    validate_text(alt)?;
    if alt.chars().count() > wordcraft_doc::para::MAX_ALT_TEXT {
        return Err(PyValueError::new_err(
            "image alternative text exceeds 10000 characters",
        ));
    }
    Ok(())
}

fn validate_image_size(width: f64, height: f64) -> PyResult<()> {
    if [width, height]
        .iter()
        .any(|value| !value.is_finite() || !(1.0..=1584.0).contains(value))
    {
        return Err(PyValueError::new_err(
            "image dimensions must be finite and between 1 and 1584 points",
        ));
    }
    Ok(())
}

fn validate_png(data: &[u8]) -> PyResult<(u32, u32)> {
    if data.len() > MAX_IMAGE_BYTES {
        return Err(limit("PNG input exceeds 16 MiB"));
    }
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(EngineError::Unsupported(
            "DOCX image creation currently requires PNG bytes".into(),
        )
        .into());
    }
    let error = |error| EngineError::Malformed(format!("invalid PNG: {error}"));
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(data),
        png::Limits {
            bytes: MAX_MEDIA_BYTES,
        },
    );
    decoder.ignore_checksums(false);
    let mut reader = decoder.read_info().map_err(error)?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if info.animation_control.is_some() {
        return Err(
            EngineError::Unsupported("animated PNG authoring is not supported".into()).into(),
        );
    }
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(limit(
            "PNG exceeds 8192 pixels per side or 16 million pixels",
        ));
    }
    let buffer_size = reader
        .output_buffer_size()
        .filter(|size| *size <= MAX_MEDIA_BYTES)
        .ok_or_else(|| limit("PNG decoded buffer exceeds 64 MiB"))?;
    let mut pixels = vec![0; buffer_size];
    reader.next_frame(&mut pixels).map_err(error)?;
    reader.finish().map_err(error)?;
    Ok((width, height))
}

fn cell_plain_text(cell: &wordcraft_doc::Cell) -> String {
    cell.blocks
        .iter()
        .map(|block| match block.as_ref() {
            Block::Para(paragraph) => paragraph.plain_text(),
            Block::Table(table) => table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(cell_plain_text)
                        .collect::<Vec<_>>()
                        .join("\t")
                })
                .collect::<Vec<_>>()
                .join("\n"),
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_section_start(start: &str) -> PyResult<SectionStart> {
    match start {
        "next_page" => Ok(SectionStart::NextPage),
        "continuous" => Ok(SectionStart::Continuous),
        "even_page" => Ok(SectionStart::EvenPage),
        "odd_page" => Ok(SectionStart::OddPage),
        _ => Err(PyValueError::new_err(
            "section start must be next_page, continuous, even_page or odd_page",
        )),
    }
}

fn section_start_name(start: SectionStart) -> &'static str {
    match start {
        SectionStart::NextPage => "next_page",
        SectionStart::Continuous => "continuous",
        SectionStart::EvenPage => "even_page",
        SectionStart::OddPage => "odd_page",
        SectionStart::NextColumn => "next_column",
    }
}

fn section_numbers<const N: usize>(value: &Value, key: &str) -> PyResult<[f32; N]> {
    let array = value
        .as_array()
        .filter(|v| v.len() == N)
        .ok_or_else(|| PyValueError::new_err(format!("{key} requires {N} numeric values")))?;
    let mut result = [0.0; N];
    for (target, value) in result.iter_mut().zip(array) {
        let number = value
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1584.0)
            .ok_or_else(|| {
                PyValueError::new_err(format!(
                    "{key} values must be finite and between 0 and 1584 points"
                ))
            })?;
        *target = number as f32;
    }
    Ok(result)
}

fn column_width(section: &SectionProps) -> f32 {
    // Content can flow into any column after later edits. In particular native
    // table layout reuses its initial width when it advances into another column.
    section
        .column_boxes()
        .iter()
        .map(|(_, width)| *width)
        .fold(f32::INFINITY, f32::min)
}

fn line_number_restart_name(value: LineNumberRestart) -> &'static str {
    match value {
        LineNumberRestart::Page => "each_page",
        LineNumberRestart::Section => "each_section",
        LineNumberRestart::Continuous => "continuous",
    }
}

fn list_definitions() -> [(&'static str, Vec<wordcraft_doc::numbering::Level>); 3] {
    [
        (ListKind::Bullet, "bullet"),
        (ListKind::Numbered, "numbered"),
        (ListKind::Legal, "decimal_outline"),
    ]
    .map(|(kind, name)| (name, wordcraft_doc::numbering::levels_for(kind)))
}

// Compare all nine effective levels, never rendered labels or start counters.
// Native start_overrides do not participate in Numbering::level.
fn supported_list_kind(
    document: &Document,
    num: u32,
    definitions: &[(&'static str, Vec<wordcraft_doc::numbering::Level>); 3],
    work: &mut usize,
) -> PyResult<Option<&'static str>> {
    let numbering = &document.numbering;
    // Count the initial num lookup, then upper-bound the two num scans, one
    // abstract scan and override scan performed by each native level lookup.
    *work = work.saturating_add(numbering.nums.len());
    if *work > MAX_LIST_MATCH_WORK {
        return Err(limit(
            "list definition matching exceeds 2000000 lookup units",
        ));
    }
    let Some(number) = numbering.num(num) else {
        return Ok(None);
    };
    let cost = numbering
        .nums
        .len()
        .saturating_mul(2)
        .saturating_add(numbering.abstracts.len())
        .saturating_add(number.level_overrides.len())
        .saturating_add(1)
        .saturating_mul(9);
    *work = work.saturating_add(cost);
    if *work > MAX_LIST_MATCH_WORK {
        return Err(limit(
            "list definition matching exceeds 2000000 lookup units",
        ));
    }
    let levels: Option<Vec<_>> = (0..9).map(|level| numbering.level(num, level)).collect();
    let Some(levels) = levels else {
        return Ok(None);
    };
    for (name, wanted) in definitions {
        if levels
            .iter()
            .zip(wanted)
            .all(|(actual, expected)| *actual == expected)
        {
            return Ok(Some(*name));
        }
    }
    Ok(None)
}

// The authored profile has no custom numbering-definition editor. A canonical
// Legal abstract is a cheap activation test; full effective matching follows.
fn outline_targets(document: &Document) -> PyResult<BTreeMap<usize, usize>> {
    let definitions = list_definitions();
    let wanted = &definitions[2].1;
    let abstracts: BTreeSet<_> = document
        .numbering
        .abstracts
        .iter()
        .filter(|value| &value.levels == wanted)
        .map(|value| value.id)
        .collect();
    if abstracts.is_empty() {
        return Ok(BTreeMap::new());
    }
    if document.numbering.nums.len() > MAX_LISTS || document.numbering.abstracts.len() > MAX_LISTS {
        return Err(limit("document exceeds 10000 native list definitions"));
    }
    let numbers: BTreeSet<_> = document
        .numbering
        .nums
        .iter()
        .filter(|number| abstracts.contains(&number.abstract_id))
        .map(|number| number.id)
        .collect();
    let mut matching = 0;
    let mut matched = BTreeMap::new();
    if numbers.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sections = document.sections();
    let mut begin = 0;
    let mut visited = 0usize;
    let mut work = 0usize;
    let mut targets = BTreeMap::new();
    for (section_index, (end, section)) in sections.iter().enumerate() {
        let blocks = document
            .body
            .get(begin..=*end)
            .ok_or_else(|| PyValueError::new_err("invalid section bounds for decimal_outline"))?;
        for (offset, block) in blocks.iter().enumerate() {
            visited = visited.saturating_add(1);
            if visited > MAX_BLOCKS {
                return Err(limit(
                    "decimal_outline preflight exceeds 100000 body blocks",
                ));
            }
            let Some(paragraph) = block.as_para() else {
                continue;
            };
            let Some(reference) = document.styles.resolve_para(&paragraph.props).numbering else {
                continue;
            };
            if !numbers.contains(&reference.num) {
                continue;
            }
            if let std::collections::btree_map::Entry::Vacant(entry) = matched.entry(reference.num)
            {
                entry.insert(supported_list_kind(
                    document,
                    reference.num,
                    &definitions,
                    &mut matching,
                )?);
            }
            if matched[&reference.num] != Some("decimal_outline") {
                continue;
            }
            if section.columns.count != 1
                || section.start == SectionStart::Continuous
                || sections
                    .get(section_index + 1)
                    .is_some_and(|(_, next)| next.start == SectionStart::Continuous)
            {
                return Err(EngineError::Unsupported("decimal_outline requires a single-column, non-continuous body section that is not followed by a continuous section".into()).into());
            }
            work = work
                .saturating_add(paragraph.text.len().saturating_mul(4))
                .saturating_add(paragraph.objects.len())
                .saturating_add(1);
            if work > MAX_OUTLINE_WORK {
                return Err(limit("decimal_outline text exceeds 100000 layout-work units (four per UTF-8 byte, one per paragraph and inline object)"));
            }
            targets.insert(begin + offset, section_index);
        }
        begin = end + 1;
    }
    Ok(targets)
}

fn validate_outline_layout(
    layout: &wordcraft_layout::DocLayout,
    targets: &BTreeMap<usize, usize>,
) -> PyResult<()> {
    let mut work = 0usize;
    let mut items = 0usize;
    let mut seen = BTreeSet::new();
    for page in &layout.pages {
        items = items.saturating_add(page.items.len());
        if items > MAX_LAYOUT_ITEMS {
            return Err(limit(
                "decimal_outline validation exceeds 2000000 placed items",
            ));
        }
        for item in &page.items {
            let wordcraft_layout::Placed::Lines {
                story: StoryRef::Body,
                path,
                para,
                l0,
                l1,
                x,
                y,
                turn,
            } = item
            else {
                continue;
            };
            if path.0.len() != 1 {
                continue;
            }
            let id = path.0[0] as usize;
            let Some(section) = targets.get(&id) else {
                continue;
            };
            let label_line = para.label_line();
            if label_line < *l0 || label_line >= *l1 {
                continue;
            }
            if *turn != TextDirection::Horizontal
                || page.section != *section
                || *l0 >= *l1
                || *l1 > para.lines.len()
            {
                return Err(EngineError::Unsupported(
                    "decimal_outline requires unambiguous horizontal body placement".into(),
                )
                .into());
            }
            let label = para.label.as_ref().ok_or_else(|| {
                EngineError::Unsupported("native decimal_outline label is missing".into())
            })?;
            let line = para.lines.get(label_line).ok_or_else(|| {
                EngineError::Unsupported("native decimal_outline label line is missing".into())
            })?;
            work = work
                .saturating_add(label.glyphs.len())
                .saturating_add(label.runs.len())
                .saturating_add(line.xs.len());
            if work > MAX_LAYOUT_ITEMS {
                return Err(limit(
                    "decimal_outline geometry exceeds 2000000 glyph/caret scan units",
                ));
            }
            let left = *x + label.x;
            let right = left + label.width;
            let mut text_left = f32::INFINITY;
            let mut text_right = f32::NEG_INFINITY;
            for caret in &line.xs {
                let placed = *x + *caret;
                if !caret.is_finite() || !placed.is_finite() {
                    return Err(EngineError::Unsupported(
                        "native decimal_outline caret geometry is nonfinite".into(),
                    )
                    .into());
                }
                text_left = text_left.min(placed);
                text_right = text_right.max(placed);
            }
            let top = *y + line.top - para.lines[*l0].top;
            let bottom = top + line.height;
            let body = page.body;
            let body_right = body.right();
            let body_bottom = body.bottom();
            let disjoint = text_right - text_left <= 0.01
                || if para.rp.bidi {
                    text_right <= left + 0.01
                } else {
                    right <= text_left + 0.01
                };
            if ![
                left,
                right,
                text_left,
                text_right,
                top,
                bottom,
                body.x,
                body.y,
                body.w,
                body.h,
                body_right,
                body_bottom,
            ]
            .iter()
            .all(|value| value.is_finite())
                || label.width < 0.0
                || line.height <= 0.0
                || body.w <= 0.0
                || body.h <= 0.0
                || left < body.x - 0.01
                || right > body_right + 0.01
                || text_left < body.x - 0.01
                || text_right > body_right + 0.01
                || top < body.y - 0.01
                || bottom > body_bottom + 0.01
                || !disjoint
            {
                return Err(EngineError::Unsupported("native decimal_outline label or first-line text exceeds the body area or overlaps; adjust level, font, indentation or margins".into()).into());
            }
            if !seen.insert(id) {
                return Err(EngineError::Unsupported(
                    "native decimal_outline label is placed more than once".into(),
                )
                .into());
            }
        }
    }
    if seen.len() != targets.len() {
        return Err(EngineError::Unsupported(
            "native decimal_outline placement is missing or ambiguous".into(),
        )
        .into());
    }
    Ok(())
}

/// Bound additional native line-number work before any pagination. This is not a
/// memory estimate or a local line/counter evaluator. Pinned para.rs consumes
/// clusters for ordinary lines, plus one empty final line. Its uppercase mapping
/// can expand a source character; four units per UTF-8 byte conservatively covers
/// that expansion. Fields and note references use shape_atomic (one cluster), and
/// inline images use one object cluster. Beside-float filler lines are not numbered.
/// Tables and auxiliary stories never enter the native top-level numbering path.
fn validate_line_number_work(document: &Document) -> PyResult<()> {
    let mut begin = 0usize;
    let mut work = 0usize;
    let mut seen_numbered = false;
    let mut previous_each_page = false;
    for (end, section) in document.sections() {
        if let Some(numbers) = &section.line_numbers {
            // The pinned paginator only resets a page counter when that page
            // begins inside a numbered Page section. Re-enabling it later on a
            // shared page can carry a counter from an older page. This is a
            // conservative configuration guard, not pagination/counter recovery.
            if section.start == SectionStart::Continuous
                && numbers.restart == LineNumberRestart::Page
                && seen_numbered
                && !previous_each_page
            {
                return Err(EngineError::Unsupported("each_page line numbering cannot resume in a continuous section after an unnumbered or differently restarted section; use each_section, continuous, or a next_page section".into()).into());
            }
            if section.columns.count != 1 {
                return Err(EngineError::Unsupported("line numbering requires a single-column section: native multi-column labels overlap in the left margin".into()).into());
            }
            if !(1..=100).contains(&numbers.count_by)
                || !(1..=32768).contains(&numbers.start)
                || !numbers.distance.is_finite()
                || !(0.0..=1584.0).contains(&numbers.distance)
            {
                return Err(PyValueError::new_err(
                    "line-number values exceed the supported native serialization bounds",
                ));
            }
            let blocks = document.body.get(begin..=end).ok_or_else(|| {
                PyValueError::new_err("invalid section bounds for line numbering")
            })?;
            for paragraph in blocks.iter().filter_map(|block| block.as_para()) {
                // Include suppressed paragraphs conservatively; later style changes
                // cannot invalidate the preflight, and no style/layout scan is needed.
                work = work
                    .saturating_add(paragraph.text.len().saturating_mul(4))
                    .saturating_add(paragraph.objects.len())
                    .saturating_add(1);
                if work > MAX_LINE_NUMBER_WORK {
                    return Err(limit("line-numbered body text exceeds the 100000-unit native label-work budget (four per UTF-8 byte, one per paragraph and inline object)"));
                }
            }
        }
        previous_each_page = section
            .line_numbers
            .as_ref()
            .is_some_and(|numbers| numbers.restart == LineNumberRestart::Page);
        seen_numbered |= section.line_numbers.is_some();
        begin = end.saturating_add(1);
    }
    Ok(())
}

/// The pinned native label path uses DECOR_STORY with an empty source path;
/// note separators use nonempty paragraph paths. Inspect actual placed logical
/// extents rather than guessing label width from the nominal 60pt layout box.
/// This does not claim font-ink clipping or rebuild any numbering/layout logic.
fn validate_line_number_bounds(layout: &wordcraft_layout::DocLayout) -> PyResult<()> {
    let mut items = 0usize;
    let mut labels = 0usize;
    for page in &layout.pages {
        items = items.saturating_add(page.items.len());
        if items > MAX_LAYOUT_ITEMS {
            return Err(limit("line-number validation exceeds 2000000 placed items"));
        }
        for item in &page.items {
            let wordcraft_layout::Placed::Lines {
                story,
                path,
                para,
                l0,
                l1,
                x,
                y,
                turn,
            } = item
            else {
                continue;
            };
            if *story != wordcraft_layout::DECOR_STORY || !path.0.is_empty() {
                continue;
            }
            labels += 1;
            if labels > MAX_LINE_NUMBER_WORK {
                return Err(limit("native line labels exceed the preflight work budget"));
            }
            if *turn != TextDirection::Horizontal || *l0 != 0 || *l1 != 1 || para.lines.len() != 1 {
                return Err(EngineError::Unsupported("native line-number text wrapped or changed orientation; adjust document-default formatting".into()).into());
            }
            let line = &para.lines[0];
            let left = *x + line.xs.iter().copied().fold(f32::INFINITY, f32::min);
            let right = *x + line.xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let bottom = *y + line.height;
            if ![left, right, *y, bottom, page.w, page.h]
                .iter()
                .all(|value| value.is_finite())
                || left < -0.01
                || right > page.w + 0.01
                || *y < -0.01
                || bottom > page.h + 0.01
            {
                return Err(EngineError::Unsupported("native line-number extent exceeds the physical page; reduce its distance or adjust margins/formatting".into()).into());
            }
        }
    }
    Ok(())
}

fn validate_section_geometry(section: &SectionProps) -> PyResult<()> {
    let columns = &section.columns;
    let text_width = section.page_w - section.margin_left - section.margin_right - section.gutter;
    let column_width_valid = if columns.widths.is_empty() {
        text_width - columns.space * columns.count.saturating_sub(1) as f32
            >= 18.0 * columns.count as f32
    } else {
        columns.widths.len() == columns.count as usize
            && columns.widths.iter().all(|(width, gap)| {
                width.is_finite() && *width >= 18.0 && gap.is_finite() && *gap >= 0.0
            })
            && columns
                .widths
                .iter()
                .map(|(width, gap)| width + gap)
                .sum::<f32>()
                <= text_width + 0.001
    };
    if section.page_w < section.margin_left + section.margin_right + section.gutter + 18.0
        || section.page_h < section.margin_top + section.margin_bottom + 18.0
        || section.header > section.page_h
        || section.footer > section.page_h
        || !(1..=45).contains(&columns.count)
        || !column_width_valid
    {
        return Err(PyValueError::new_err("section geometry must leave 18 points per column and of body height; header/footer distances must fit the page"));
    }
    Ok(())
}

fn validate_picture_width(paragraph: &Paragraph, width: f32) -> PyResult<()> {
    if paragraph
        .objects
        .iter()
        .any(|object| matches!(object, InlineObject::Image { w, .. } if *w > width + 0.001))
    {
        return Err(PyValueError::new_err(
            "picture width exceeds its section or cell width",
        ));
    }
    Ok(())
}

fn blocks_fit_width(blocks: &[Arc<Block>], width: f32) -> PyResult<()> {
    for block in blocks {
        let exceeds = match block.as_ref() {
            Block::Table(table) => {
                table.props.width_pct.is_none()
                    && table.props.width.unwrap_or_else(|| table.total_width()) > width + 0.001
            }
            Block::Para(paragraph) => paragraph.objects.iter().any(
                |object| matches!(object, InlineObject::Image { w, .. } if *w > width + 0.001),
            ),
        };
        if exceeds {
            return Err(PyValueError::new_err(
                "operation would overflow an existing table or image; resize the child first",
            ));
        }
    }
    Ok(())
}

fn validate_section_content(document: &Document, section: usize) -> PyResult<()> {
    let sections = document.sections();
    // The pinned layout retains the current page box for continuous sections.
    for pair in sections.windows(2) {
        if pair[1].1.start == SectionStart::Continuous
            && (pair[0].1.page_w != pair[1].1.page_w || pair[0].1.page_h != pair[1].1.page_h)
        {
            return Err(PyValueError::new_err(
                "continuous sections must have the same page size as the preceding section",
            ));
        }
    }
    let (end, props) = sections[section];
    let begin = if section == 0 {
        0
    } else {
        sections[section - 1].0 + 1
    };
    blocks_fit_width(&document.body[begin..=end], column_width(props))?;
    let mut tables = document.body[begin..=end]
        .iter()
        .enumerate()
        .filter(|(_, block)| matches!(block.as_ref(), Block::Table(_)))
        .map(|(index, _)| (StoryRef::Body, Path::top(begin + index)))
        .collect::<Vec<_>>();
    let h = props.headers;
    let f = props.footers;
    for id in [h.default, h.first, h.even, f.default, f.first, f.even]
        .into_iter()
        .flatten()
    {
        let part = document
            .parts
            .get(&id)
            .ok_or_else(|| PyValueError::new_err("section references a missing story"))?;
        blocks_fit_width(&part.blocks, props.text_width())?;
        tables.extend(part.blocks.iter().enumerate().filter_map(|(index, block)| {
            matches!(block.as_ref(), Block::Table(_))
                .then_some((StoryRef::Part(id), Path::top(index)))
        }));
    }
    if !tables.is_empty() {
        let widths = native_cell_widths(document)?;
        for (story, path) in tables {
            validate_native_table_layout(document, &widths, story, path)?;
        }
    }
    Ok(())
}

fn table_usage(table: &Table) -> (usize, usize) {
    let mut bytes = 0;
    let mut cells = 0;
    for row in &table.rows {
        cells += row.cells.len();
        for cell in &row.cells {
            let (nested_bytes, nested_cells) = content_usage(&cell.blocks);
            bytes += nested_bytes;
            cells += nested_cells;
        }
    }
    (bytes, cells)
}

fn validate_editable_grid(table: &Table) -> PyResult<()> {
    let columns = table.grid.len();
    if table.rows.is_empty()
        || table.rows.len() > wordcraft_doc::table::MAX_ROWS
        || columns == 0
        || columns > wordcraft_doc::table::MAX_COLS
        || table
            .grid
            .iter()
            .any(|width| !width.is_finite() || *width < 6.0)
    {
        return Err(PyValueError::new_err(
            "table requires 1..32767 rows and 1..63 columns of at least 6 points",
        ));
    }
    let mut previous_merges = BTreeSet::new();
    let mut body_seen = false;
    for row in &table.rows {
        if row.props.header && body_seen {
            return Err(PyValueError::new_err(
                "table edit would interrupt the repeated-header prefix; clear repeat_header first",
            ));
        }
        body_seen |= !row.props.header;
        if row.props.grid_before != 0 || row.props.grid_after != 0 {
            return Err(PyValueError::new_err(
                "table grid edits require complete rows",
            ));
        }
        let mut column = 0_usize;
        let mut merges = BTreeSet::new();
        for cell in &row.cells {
            let span = cell.props.span as usize;
            let end = column.saturating_add(span);
            if span == 0 || span > wordcraft_doc::table::MAX_COLS || end > columns {
                return Err(PyValueError::new_err(
                    "table edit produced an invalid cell span",
                ));
            }
            if cell.props.vmerge == VMerge::Continue && !previous_merges.contains(&(column, end)) {
                return Err(EngineError::Unsupported(
                    "upstream cannot preserve this vertical merge during the row edit".into(),
                )
                .into());
            }
            if cell.props.vmerge != VMerge::None {
                merges.insert((column, end));
            }
            column = end;
        }
        if column != columns {
            return Err(PyValueError::new_err(
                "table edit produced an incomplete row",
            ));
        }
        previous_merges = merges;
    }
    Ok(())
}

fn synchronize_grid_widths(table: &mut Table) {
    table.props.width = Some(table.grid.iter().sum());
    table.props.width_pct = None;
    for row in &mut table.rows {
        let mut column = 0;
        for cell in &mut row.cells {
            let end = column + cell.span();
            cell.props.width = Some(table.grid[column..end].iter().sum());
            cell.props.width_pct = None;
            column = end;
        }
    }
}

fn validate_grid_children(document: &Document, table: &Table) -> PyResult<()> {
    for (row_index, row) in table.rows.iter().enumerate() {
        let mut column = 0;
        for (cell_index, cell) in row.cells.iter().enumerate() {
            let end = column + cell.span();
            let margins = wordcraft_layout::cell_margins(document, table, row_index, cell_index);
            let available = table.grid[column..end].iter().sum::<f32>() - margins[1] - margins[3];
            if cell.props.margins.is_some() && available <= 0.0 {
                return Err(PyValueError::new_err(
                    "table edit must leave positive width inside explicit cell margins",
                ));
            }
            blocks_fit_width(&cell.blocks, available)?;
            column = end;
        }
    }
    Ok(())
}

// Owned section ranges always have a physical block, including the final blank.
// This validates native structure; it does not relocate or synthesize section properties.
fn owned_body_sections(document: &Document) -> PyResult<Vec<SectionProps>> {
    if document.body.len() > MAX_BLOCKS {
        return Err(limit("paragraph deletion exceeds 100000 body blocks"));
    }
    if document
        .body
        .last()
        .and_then(|block| block.as_para())
        .is_none_or(|paragraph| paragraph.section.is_some())
    {
        return Err(PyValueError::new_err(
            "owned body requires a final non-section paragraph",
        ));
    }
    let sections = document.sections();
    if sections.len() > MAX_SECTIONS {
        return Err(limit("document exceeds 128 sections"));
    }
    let mut previous = None;
    for (end, _) in &sections {
        if *end >= document.body.len() || previous.is_some_and(|before| *end <= before) {
            return Err(PyValueError::new_err(
                "owned body section endpoints are invalid",
            ));
        }
        previous = Some(*end);
    }
    if previous != document.body.len().checked_sub(1) {
        return Err(PyValueError::new_err(
            "owned body final section endpoint is invalid",
        ));
    }
    Ok(sections
        .into_iter()
        .map(|(_, section)| section.clone())
        .collect())
}

fn validate_story_locator(path: &Path, blocks: usize) -> PyResult<()> {
    if path.0.is_empty()
        || path.0.len() > 49
        || path.0.len() % 3 != 1
        || path.0[0] as usize >= blocks
    {
        return Err(PyValueError::new_err(
            "owned story locator is malformed or exceeds 16 table levels",
        ));
    }
    Ok(())
}

fn content_usage(blocks: &wordcraft_doc::Blocks) -> (usize, usize) {
    let mut pending = vec![blocks];
    let (mut bytes, mut cells) = (0, 0);
    while let Some(blocks) = pending.pop() {
        for block in blocks {
            match block.as_ref() {
                Block::Para(paragraph) => {
                    // Image insertion does not charge its object marker. Fields, notes
                    // and bookmark markers do; copies and removals retain those costs.
                    let image_bytes = paragraph
                        .object_offsets()
                        .into_iter()
                        .zip(&paragraph.objects)
                        .filter(|(_, object)| matches!(object, InlineObject::Image { .. }))
                        .count()
                        * wordcraft_doc::para::OBJ.len_utf8();
                    bytes += paragraph.text.len() - image_bytes
                }
                Block::Table(table) => {
                    for row in &table.rows {
                        cells += row.cells.len();
                        for cell in &row.cells {
                            pending.push(&cell.blocks);
                        }
                    }
                }
            }
        }
    }
    (bytes, cells)
}

fn cell_anchor(table: &Table, row: usize, column: usize) -> PyResult<(usize, usize)> {
    let cell_index = table
        .cell_at_grid(row, column)
        .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))?;
    let cell = &table.rows[row].cells[cell_index];
    if cell.props.vmerge != VMerge::Continue {
        return Ok((row, cell_index));
    }
    let start = table.grid_col(row, cell_index);
    for above in (0..row).rev() {
        let index = table
            .cell_at_grid(above, start)
            .ok_or_else(|| PyValueError::new_err("invalid vertical merge"))?;
        let candidate = &table.rows[above].cells[index];
        if table.grid_col(above, index) != start || candidate.span() != cell.span() {
            break;
        }
        match candidate.props.vmerge {
            VMerge::Restart => return Ok((above, index)),
            VMerge::Continue => {}
            VMerge::None => break,
        }
    }
    Err(PyValueError::new_err(
        "vertical merge has no matching anchor",
    ))
}

fn run_props(
    bold: Option<bool>,
    italic: Option<bool>,
    size: Option<f64>,
    font: Option<&str>,
    color: Option<&str>,
) -> PyResult<CharProps> {
    if let Some(size) = size {
        if !size.is_finite() || !(1.0..=400.0).contains(&size) || (size * 2.0).fract() != 0.0 {
            return Err(PyValueError::new_err(
                "font size must be 1..400 points in half-point increments",
            ));
        }
    }
    if let Some(font) = font {
        validate_text(font)?;
        if font.is_empty() || font.len() > 255 || font.chars().any(char::is_control) {
            return Err(PyValueError::new_err(
                "font must be a nonempty name of at most 255 bytes without control characters",
            ));
        }
    }
    Ok(CharProps {
        bold,
        italic,
        size: size.map(|value| value as f32),
        font: font.map(str::to_owned),
        color: color.map(parse_rgb).transpose()?.map(TextColor::Rgb),
        ..CharProps::default()
    })
}

fn range_offsets(text: &str, start: usize, end: usize) -> PyResult<(usize, usize)> {
    if start > end {
        return Err(PyValueError::new_err(
            "text range start must not exceed end",
        ));
    }
    let mut from = None;
    let mut to = None;
    for (character_index, byte_index) in text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .enumerate()
    {
        if character_index == start {
            from = Some(byte_index);
        }
        if character_index == end {
            to = Some(byte_index);
            break;
        }
    }
    from.zip(to)
        .ok_or_else(|| PyIndexError::new_err("text range offset out of bounds"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_automatic_pages_and_unnumbered_column_transitions() {
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 260.0).unwrap();
        document
            .set_section_props(0, r#"{"line_numbers":[1,1,0,"continuous"]}"#)
            .unwrap();
        for index in 0..15 {
            document
                .add_paragraph(&format!("Paragraph{index}"))
                .unwrap();
        }
        let pages = line_label_layout(document.document().unwrap());
        assert!(pages.len() > 1);
        assert_eq!(
            pages
                .iter()
                .flatten()
                .map(|label| label.0.parse::<u32>().unwrap())
                .collect::<Vec<_>>(),
            (1..=15).collect::<Vec<_>>()
        );
        assert!(pages
            .iter()
            .all(|page| page.windows(2).all(|pair| pair[0].2 < pair[1].2)));
        for restart in ["each_section", "continuous"] {
            let mut document = EditableDocx::new();
            document
                .set_section_props(0, r#"{"columns":[2,36,false]}"#)
                .unwrap();
            for index in 0..4 {
                document.add_paragraph(&format!("Before{index}")).unwrap();
            }
            let numbered = document.add_section("continuous").unwrap();
            document
                .set_section_props(
                    numbered,
                    &json!({"columns":[1,36,false],"line_numbers":[1,1,0,restart]}).to_string(),
                )
                .unwrap();
            document.add_paragraph("First numbered").unwrap();
            document.add_paragraph("Second numbered").unwrap();
            let unnumbered = document.add_section("continuous").unwrap();
            document
                .set_section_props(
                    unnumbered,
                    r#"{"line_numbers":null,"columns":[2,36,false]}"#,
                )
                .unwrap();
            for index in 0..4 {
                document.add_paragraph(&format!("Middle{index}")).unwrap();
            }
            let last = document.add_section("continuous").unwrap();
            document
                .set_section_props(
                    last,
                    &json!({"columns":[1,36,false],"line_numbers":[1,1,0,restart]}).to_string(),
                )
                .unwrap();
            document.add_paragraph("Last numbered").unwrap();
            let labels = line_label_layout(document.document().unwrap());
            assert_eq!(
                labels
                    .iter()
                    .flatten()
                    .map(|label| label.0.as_str())
                    .collect::<Vec<_>>(),
                if restart == "continuous" {
                    vec!["1", "2", "3"]
                } else {
                    vec!["1", "2", "1"]
                }
            );
            document.to_bytes().unwrap();
        }
    }

    #[test]
    fn line_numbers_native_page_restart_lost_across_unnumbered_pages() {
        let mut document = EditableDocx::new();
        document.set_page_size(400.0, 350.0).unwrap();
        document
            .set_section_props(0, r#"{"line_numbers":[1,1,0,"each_page"]}"#)
            .unwrap();
        document.add_paragraph("First").unwrap();
        let middle = document.add_section("next_page").unwrap();
        document
            .set_section_props(middle, r#"{"line_numbers":null,"columns":[2,36,false]}"#)
            .unwrap();
        for index in 0..24 {
            document.add_paragraph(&format!("Middle{index}")).unwrap();
        }
        let last = document.add_section("continuous").unwrap();
        document
            .set_section_props(last, r#"{"columns":[1,36,false]}"#)
            .unwrap();
        document.add_paragraph("Last").unwrap();
        let mut raw = document.document().unwrap().clone();
        raw.last_section.line_numbers = Some(LineNumbering::default());
        let labels = line_label_layout(&raw);
        assert!(labels.len() >= 3);
        assert_eq!(labels[0][0].0, "1");
        assert_eq!(
            labels.last().unwrap()[0].0,
            "2",
            "pinned native bug: each-page numbering carries the old page's counter"
        );
        assert!(validate_line_number_work(&raw).is_err());
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .set_section_props(last, r#"{"line_numbers":[1,1,0,"each_page"]}"#)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);
        document
            .set_section_props(last, r#"{"line_numbers":[1,1,0,"each_section"]}"#)
            .unwrap();
        assert_eq!(
            line_label_layout(document.document().unwrap())
                .last()
                .unwrap()[0]
                .0,
            "1"
        );
        document.to_bytes().unwrap();
    }

    fn line_label_layout(document: &Document) -> Vec<Vec<(String, f32, f32)>> {
        let layout = wordcraft_layout::layout(
            document,
            &mut wordcraft_layout::LayoutCache::new(),
            &wordcraft_layout::LayoutOptions::default(),
        );
        layout
            .pages
            .iter()
            .map(|page| {
                page.items
                    .iter()
                    .filter_map(|item| {
                        let wordcraft_layout::Placed::Lines {
                            story,
                            path,
                            para,
                            x,
                            y,
                            ..
                        } = item
                        else {
                            return None;
                        };
                        if *story != wordcraft_layout::DECOR_STORY || !path.0.is_empty() {
                            return None;
                        }
                        let mut text = String::new();
                        for cluster in &para.clusters {
                            let face = &para.styles[cluster.style as usize].face;
                            for glyph in &para.glyphs[cluster.g0 as usize..cluster.g1 as usize] {
                                text.push(
                                    ('0'..='9')
                                        .find(|value| face.glyph_for(*value) == glyph.gid)
                                        .expect("native label contains an ASCII digit"),
                                );
                            }
                        }
                        Some((text, *x, *y))
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn line_numbers_roundtrip_restarts_tables_suppression_and_pdf() {
        let mut document = EditableDocx::new();
        document
            .set_section_props(0, r#"{"line_numbers":[1,1,0,"each_page"]}"#)
            .unwrap();
        document.add_paragraph("First").unwrap();
        let suppressed = document.add_paragraph("Suppressed").unwrap();
        let mut candidate = document.document().unwrap().clone();
        candidate
            .para_mut(StoryRef::Body, &document.location(suppressed).unwrap().1)
            .unwrap()
            .props
            .suppress_line_numbers = Some(true);
        document.commit(candidate).unwrap();
        document.add_table(vec![vec!["Table text".into()]]).unwrap();
        document.add_paragraph("Second").unwrap();
        assert_eq!(
            line_label_layout(document.document().unwrap())[0]
                .iter()
                .map(|x| x.0.as_str())
                .collect::<Vec<_>>(),
            ["1", "2"]
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened.sections()[0].1.line_numbers,
            document.document().unwrap().sections()[0].1.line_numbers
        );
        assert_eq!(
            line_label_layout(&reopened),
            line_label_layout(document.document().unwrap())
        );
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let png = crate::docx::render(&bytes, 0, 72.0, None).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        for restart in ["each_page", "each_section", "continuous"] {
            let mut document = EditableDocx::new();
            document
                .set_section_props(0, &json!({"line_numbers":[1,7,12,restart]}).to_string())
                .unwrap();
            document.add_paragraph("First").unwrap();
            document.add_section("next_page").unwrap();
            document.add_paragraph("Second").unwrap();
            let labels = line_label_layout(document.document().unwrap());
            assert_eq!(labels[0][0].0, "7");
            assert_eq!(
                labels[1][0].0,
                if restart == "continuous" { "8" } else { "7" }
            );
        }
        let mut document = EditableDocx::new();
        document
            .set_section_props(0, r#"{"line_numbers":[2,5,0,"each_page"]}"#)
            .unwrap();
        for text in ["One", "Two", "Three"] {
            document.add_paragraph(text).unwrap();
        }
        assert_eq!(
            line_label_layout(document.document().unwrap())[0]
                .iter()
                .map(|x| x.0.as_str())
                .collect::<Vec<_>>(),
            ["6"]
        );
    }

    #[test]
    fn line_numbers_bounds_work_and_later_edits_fail_without_partial_commit() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Budget").unwrap();
        document
            .set_section_props(0, r#"{"line_numbers":[100,32768,0,"each_section"]}"#)
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for patch in [
            r#"{"line_numbers":[true,1,0,"each_page"]}"#,
            r#"{"line_numbers":[0,1,0,"each_page"]}"#,
            r#"{"line_numbers":[101,1,0,"each_page"]}"#,
            r#"{"line_numbers":[1,0,0,"each_page"]}"#,
            r#"{"line_numbers":[1,32769,0,"each_page"]}"#,
            r#"{"line_numbers":[1,1,-1,"each_page"]}"#,
            r#"{"line_numbers":[1,1,1585,"each_page"]}"#,
            r#"{"line_numbers":[1,1,0,"page"]}"#,
            r#"{"columns":[2,36,false]}"#,
        ] {
            assert!(document.set_section_props(0, patch).is_err(), "{patch}");
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), bytes);
        }
        assert!(document
            .set_paragraph_text(paragraph, &"a".repeat(MAX_LINE_NUMBER_WORK / 4))
            .is_err());
        assert_eq!(document.paragraph_text(paragraph).unwrap(), "Budget");
        assert_eq!(document.revision().unwrap(), revision);
        document
            .set_section_props(0, r#"{"line_numbers":null}"#)
            .unwrap();
        document
            .set_paragraph_text(paragraph, &"a".repeat(MAX_LINE_NUMBER_WORK / 4))
            .unwrap();
        assert!(document
            .set_section_props(0, r#"{"line_numbers":[1,1,0,"each_page"]}"#)
            .is_err());
        assert!(document
            .document()
            .unwrap()
            .last_section
            .line_numbers
            .is_none());
        let mut model = Document::new();
        model.last_section.line_numbers = Some(LineNumbering::default());
        model.body = vec![
            Arc::new(Block::Para(Paragraph::with_text(
                &"a".repeat(24999),
                CharProps::default(),
            ))),
            Arc::new(Block::Para(Paragraph::default())),
            Arc::new(Block::Para(Paragraph::default())),
            Arc::new(Block::Para(Paragraph::default())),
        ];
        validate_line_number_work(&model).unwrap(); // 4*24999 + four paragraph units = 100000.
        model.body.push(Arc::new(Block::Para(Paragraph::default())));
        assert!(validate_line_number_work(&model).is_err());
    }

    #[test]
    fn line_numbers_candidate_table_measurement_preflights_before_pagination() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        document.add_paragraph(&"x".repeat(25_000)).unwrap();
        document.add_table(vec![vec!["cell".into()]]).unwrap();
        let mut raw = document.document().unwrap().clone();
        raw.last_section.line_numbers = Some(LineNumbering::default());
        // Both production layout entry points reject the candidate independently
        // of commit/to_bytes. Section mutation reaches the first via table checks.
        assert!(native_cell_widths(&raw)
            .unwrap_err()
            .to_string()
            .contains("label-work budget"));
        assert!(validate_unsplittable_page_bounds(&raw)
            .unwrap_err()
            .to_string()
            .contains("label-work budget"));
        let before = document.to_bytes().unwrap();
        let error = document
            .set_section_props(0, r#"{"line_numbers":[1,1,0,"each_page"]}"#)
            .unwrap_err();
        assert!(error.to_string().contains("label-work budget"));
        assert!(document
            .document()
            .unwrap()
            .last_section
            .line_numbers
            .is_none());
        assert_eq!(document.to_bytes().unwrap(), before);
    }

    #[test]
    fn line_numbers_native_column_overlap_and_physical_page_guard() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        document.add_paragraph("Left").unwrap();
        let split = document.add_paragraph("Left end").unwrap();
        document.add_paragraph_break(split, "column").unwrap();
        document.add_paragraph("Right").unwrap();
        let mut model = document.document().unwrap().clone();
        model.last_section.columns.count = 2;
        model.last_section.line_numbers = Some(LineNumbering::default());
        let labels = line_label_layout(&model);
        assert!(
            labels[0]
                .iter()
                .enumerate()
                .any(|(i, a)| labels[0][i + 1..]
                    .iter()
                    .any(|b| (a.1 - b.1).abs() < 0.001 && (a.2 - b.2).abs() < 0.001)),
            "native columns must reproduce overlapping line labels"
        );
        assert!(validate_line_number_work(&model).is_err());
        document
            .set_section_props(0, r#"{"line_numbers":[1,1,0,"each_page"]}"#)
            .unwrap();
        document.to_bytes().unwrap(); // Normal 72pt margins are valid.
        document
            .set_section_props(0, r#"{"line_numbers":[1,1,1000,"each_page"]}"#)
            .unwrap();
        assert!(document
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("physical page"));
        document
            .set_section_props(0, r#"{"line_numbers":[1,1,0,"each_page"]}"#)
            .unwrap();
        document.to_bytes().unwrap();
    }

    #[test]
    fn rich_stories_keep_append_order_locators_and_section_copy_independent() {
        let mut document = EditableDocx::new();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let table = document
            .story_add_table(header, vec![vec!["Header cell".into()]])
            .unwrap();
        let png = media_png(3, 2, [255, 0, 0, 255]);
        let image = document
            .story_add_image(header, &png, Some(24.0), Some(16.0), "Header picture")
            .unwrap();
        let paragraph = document
            .story_add_paragraph(header, "After picture")
            .unwrap();
        document.story_add_paragraph(header, "").unwrap();
        document.story_add_paragraph(header, "After empty").unwrap();
        assert_eq!(document.story_table_ids(header).unwrap(), vec![table]);
        assert_eq!(document.story_image_ids(header).unwrap(), vec![image]);
        assert_eq!(document.story_paragraph_ids(header).unwrap().len(), 3);
        assert_eq!(document.paragraph_text(paragraph).unwrap(), "After picture");
        assert_eq!(
            document.table_rows(table).unwrap(),
            vec![vec!["Header cell"]]
        );
        let pending = document.ensure_story("footer", "default", 0).unwrap();
        document
            .story_add_table(pending, vec![vec!["Footer table".into()]])
            .unwrap();
        document.add_paragraph("First body").unwrap();
        document.add_section("next_page").unwrap();
        document.add_paragraph("Second body").unwrap();
        let copied_header = document.story_id("header", "default", 1).unwrap().unwrap();
        let copied_table = document.story_table_ids(copied_header).unwrap()[0];
        document
            .set_cell_text(copied_table, 0, 0, "Second cell")
            .unwrap();
        assert_eq!(
            document.table_rows(table).unwrap(),
            vec![vec!["Header cell"]]
        );
        let copied_footer = document.story_id("footer", "default", 1).unwrap().unwrap();
        document
            .story_add_paragraph(copied_footer, "After copied table")
            .unwrap();
        assert_eq!(document.story_table_ids(copied_footer).unwrap().len(), 1);
        assert!(document.story_paragraph_ids(pending).unwrap().is_empty());
        let mut copy = document.copy().unwrap();
        copy.remove_image(image).unwrap();
        assert_eq!(document.image_data(image).unwrap(), png);
        assert_eq!(copy.story_image_ids(header).unwrap().len(), 0);
        assert_eq!(copy.story_paragraph_ids(header).unwrap()[0], image);
        assert_eq!(copy.paragraph_text(image).unwrap(), "");
        let bytes = document.to_bytes().unwrap();
        let read = wordcraft_docx::read(&bytes).unwrap();
        let part = &read.parts[&read.last_section.headers.default.unwrap()];
        assert!(part.blocks[0].as_table().is_some());
        assert!(part.blocks[1]
            .as_para()
            .unwrap()
            .objects
            .iter()
            .any(|object| matches!(object, InlineObject::Image { .. })));
        assert_eq!(part.blocks[2].as_para().unwrap().text, "After picture");
        assert_eq!(part.blocks[3].as_para().unwrap().text, "");
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        assert!(border_pixels(crate::pdf::render(&pdf, 0, 72.0, None).unwrap(), 0) > 200);
        assert!(border_pixels(crate::docx::render(&bytes, 1, 72.0, None).unwrap(), 0) > 200);
    }

    #[test]
    fn rich_stories_guard_physical_overflow_and_inactive_nested_geometry() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        document.set_page_size(240.0, 320.0).unwrap();
        document.add_paragraph("Body").unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let table = document
            .story_add_table(header, vec![vec!["short".into()]])
            .unwrap();
        document
            .set_cell_text(table, 0, 0, &"Overflow ".repeat(500))
            .unwrap();
        assert!(document
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("physical page"));
        document.set_cell_text(table, 0, 0, "short").unwrap();
        document.to_bytes().unwrap();
        let png = media_png(2, 2, [255, 0, 0, 255]);
        let footer = document.ensure_story("footer", "default", 0).unwrap();
        let image = document
            .story_add_image(footer, &png, Some(40.0), Some(400.0), "Tall")
            .unwrap();
        assert!(document
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("physical page"));
        document.set_image_size(image, 40.0, 20.0).unwrap();
        document
            .set_row_height(table, 0, Some(20.0), "exact")
            .unwrap();
        document
            .cell_add_image(
                table,
                0,
                0,
                &png,
                Some(10.0),
                Some(400.0),
                "Intentionally clipped",
            )
            .unwrap();
        document.to_bytes().unwrap();
        let even = document.ensure_story("header", "even", 0).unwrap();
        let hidden = document
            .story_add_table(even, vec![vec!["Not shown yet".into()]])
            .unwrap();
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .cell_add_image(hidden, 0, 0, &png, Some(10.0), Some(10.0), "hidden")
            .unwrap_err()
            .to_string()
            .contains("geometry"));
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);
        assert!(document
            .story_add_image(header, &png, Some(999.0), Some(10.0), "wide")
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
    }

    #[test]
    fn core_metadata_roundtrips_without_changing_timestamps_or_content() {
        let mut document = EditableDocx::new();
        document
            .add_paragraph("Metadata body remains visible")
            .unwrap();
        assert_eq!(document.metadata().unwrap(), r#"{"custom":{}}"#);
        let values = json!({"title":"Report <&> Ω", "author":"Example Author", "subject":"Subject",
            "keywords":"one;two", "description":"First line\nSecond line", "category":"Review",
            "last_modified_by":"Editor", "created":"2024-02-29T01:02:03Z",
            "modified":"2026-10-20T23:59:59Z", "revision":4294967295u32});
        document.update_metadata(&values.to_string()).unwrap();
        let expected = document.metadata().unwrap();
        let copied = document.copy().unwrap();
        let bytes = document.to_bytes().unwrap();
        assert_eq!(document.to_bytes().unwrap(), bytes);
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(document_metadata(&reopened).unwrap().to_string(), expected);
        assert_eq!(reopened.core.creator, "Example Author");
        assert_eq!(reopened.core.created, "2024-02-29T01:02:03Z");
        crate::docx::render(&bytes, 0, 72.0, None).unwrap();
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        crate::pdf::render(&pdf, 0, 72.0, None).unwrap();
        document
            .update_metadata(r#"{"author":null,"title":"","revision":null,"created":null}"#)
            .unwrap();
        let updated: Value = serde_json::from_str(&document.metadata().unwrap()).unwrap();
        for key in ["title", "author", "revision", "created"] {
            assert!(updated.get(key).is_none());
        }
        assert_eq!(updated["modified"], "2026-10-20T23:59:59Z");
        assert_eq!(copied.metadata().unwrap(), expected);
        document.close();
        assert!(document.metadata().is_err());
        assert!(document.update_metadata("{}").is_err());
    }

    #[test]
    fn custom_metadata_scalar_types_clear_and_case_identity_roundtrip() {
        let mut document = EditableDocx::new();
        let custom = json!({"Text":"  escaped <&> Ω\ntext  ", "Empty":"", "Flag":false,
            "Minimum":i64::MIN, "Maximum":i64::MAX, "Ratio":1.25, "NegativeZero":-0.0});
        document
            .update_metadata(&json!({"custom":custom}).to_string())
            .unwrap();
        let expected: Value = serde_json::from_str(&document.metadata().unwrap()).unwrap();
        assert_eq!(expected["custom"], custom);
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(document_metadata(&reopened).unwrap(), expected);
        assert_eq!(
            reopened
                .custom_props
                .iter()
                .find(|p| p.name == "Flag")
                .unwrap()
                .kind,
            "bool"
        );
        assert_eq!(
            reopened
                .custom_props
                .iter()
                .find(|p| p.name == "Maximum")
                .unwrap()
                .kind,
            "i8"
        );
        assert_eq!(
            reopened
                .custom_props
                .iter()
                .find(|p| p.name == "Ratio")
                .unwrap()
                .kind,
            "r8"
        );
        document
            .update_metadata(r#"{"title":"Retained","custom":{"flag":true,"ratio":null}}"#)
            .unwrap();
        let values: Value = serde_json::from_str(&document.metadata().unwrap()).unwrap();
        assert_eq!(values["custom"]["Flag"], true);
        assert!(values["custom"].get("flag").is_none());
        assert!(values["custom"].get("Ratio").is_none());
        document.update_metadata(r#"{"custom":null}"#).unwrap();
        assert_eq!(
            document.metadata().unwrap(),
            r#"{"custom":{},"title":"Retained"}"#
        );
        assert!(wordcraft_docx::read(&document.to_bytes().unwrap())
            .unwrap()
            .custom_props
            .is_empty());
    }

    #[test]
    fn metadata_invalid_updates_preserve_owner_and_revision() {
        let mut document = EditableDocx::new();
        document
            .update_metadata(r#"{"title":"Original","custom":{"Keep":1}}"#)
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        let mut invalid = vec![
            "[]".into(),
            r#"{"title":"Changed","language":"en"}"#.into(),
            r#"{"title":" Changed"}"#.into(),
            r#"{"title":"Changed "}"#.into(),
            r#"{"title":"\u0000"}"#.into(),
            r#"{"title":true}"#.into(),
            r#"{"revision":true}"#.into(),
            r#"{"revision":-1}"#.into(),
            r#"{"revision":4294967296}"#.into(),
            r#"{"revision":1.0}"#.into(),
            r#"{"custom":{"Key":1,"KEY":2}}"#.into(),
            r#"{"custom":{"Keep":null,"Nested":[]}}"#.into(),
            r#"{"custom":{"Huge":9223372036854775808}}"#.into(),
            r#"{"custom":{"Huge":1e400}}"#.into(),
            r#"{"custom":[]}"#.into(),
            r#"{"custom":{"":1}}"#.into(),
            r#"{"custom":{" bad":1}}"#.into(),
            r#"{"custom":{"bad\nname":1}}"#.into(),
        ];
        for date in [
            "2023-02-29T00:00:00Z",
            "1900-02-29T00:00:00Z",
            "0000-01-01T00:00:00Z",
            "2024-13-01T00:00:00Z",
            "2024-04-31T00:00:00Z",
            "2024-01-00T00:00:00Z",
            "2024-01-01T24:00:00Z",
            "2024-01-01T00:60:00Z",
            "2024-01-01T00:00:60Z",
            "2024-01-01",
            "2024-01-01T00:00:00+01:00",
            "é024-01-01T00:00:00Z",
        ] {
            invalid.push(json!({"title":"Changed", "created":date}).to_string());
        }
        invalid.push(json!({"title":"x".repeat(MAX_PROPERTY_VALUE_BYTES+1)}).to_string());
        invalid.push(json!({"custom":{"x".repeat(256):1}}).to_string());
        for update in invalid {
            assert!(
                document.update_metadata(&update).is_err(),
                "unexpectedly accepted {update}"
            );
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), bytes);
        }
        document.update_metadata("{}").unwrap();
        assert_eq!(document.revision().unwrap(), revision);
    }

    #[test]
    fn metadata_aggregate_and_custom_count_budgets_reclaim_after_clear() {
        let mut document = EditableDocx::new();
        let value = "x".repeat(60_000);
        for name in ["title", "author", "subject", "description"] {
            document
                .update_metadata(&json!({name:&value}).to_string())
                .unwrap();
        }
        let revision = document.revision().unwrap();
        assert!(document
            .update_metadata(&json!({"keywords":&value}).to_string())
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.update_metadata(r#"{"description":null}"#).unwrap();
        document
            .update_metadata(&json!({"keywords":&value}).to_string())
            .unwrap();
        for name in ["First", "Second", "Third", "Fourth"] {
            document
                .update_metadata(&json!({"custom":{name:&value}}).to_string())
                .unwrap();
        }
        assert!(document
            .update_metadata(&json!({"custom":{"Fifth":&value}}).to_string())
            .is_err());
        document.update_metadata(r#"{"custom":null}"#).unwrap();
        let properties: Map<String, Value> = (0..MAX_CUSTOM_PROPERTIES)
            .map(|index| (format!("Property{index}"), json!(index)))
            .collect();
        document
            .update_metadata(&json!({"custom":properties}).to_string())
            .unwrap();
        assert!(document
            .update_metadata(r#"{"custom":{"Extra":1}}"#)
            .is_err());
        document
            .update_metadata(r#"{"custom":{"Property0":null,"Extra":1}}"#)
            .unwrap();
        assert_eq!(
            document.document().unwrap().custom_props.len(),
            MAX_CUSTOM_PROPERTIES
        );
        document.to_bytes().unwrap();
    }

    #[test]
    fn upstream_core_metadata_reader_trims_supplied_strings() {
        let mut document = Document::new();
        document.core.title = "  Title Ω  ".into();
        document.core.created = "2024-02-29T01:02:03Z".into();
        let reopened = wordcraft_docx::read(&wordcraft_docx::write(&document).unwrap()).unwrap();
        assert_eq!(reopened.core.title, "Title Ω");
        assert_eq!(reopened.core.created, document.core.created);
    }

    fn media_png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        crate::rendering::encode_png(
            width,
            height,
            &color.repeat((width * height) as usize),
            72.0,
        )
        .unwrap()
    }

    #[test]
    fn page_background_roundtrip_preserves_color_media_and_copy() {
        let red = media_png(2, 2, [255, 0, 0, 255]);
        let blue = media_png(2, 2, [0, 0, 255, 255]);
        let mut document = EditableDocx::new();
        document.set_page_size(240.0, 320.0).unwrap();
        document.set_background(Some("#00FF00")).unwrap();
        let paragraph = document.add_paragraph("Page ").unwrap();
        document.add_paragraph_field(paragraph, "page").unwrap();
        document.set_background_image(Some(&red)).unwrap();
        let copy = document.copy().unwrap();
        let original = document.to_bytes().unwrap();
        let read = wordcraft_docx::read(&original).unwrap();
        let background = read.settings.page_picture.as_ref().unwrap();
        assert_eq!(background.fill_type, "frame");
        assert!(background.title.is_empty());
        assert_eq!(read.media[&background.media].as_slice(), red);
        assert_eq!(read.settings.page_color, Some(Rgb(0, 255, 0)));
        assert!(border_pixels(crate::docx::render(&original, 0, 72.0, None).unwrap(), 0) > 10_000);
        let pdf = crate::docx::to_pdf(&original, None).unwrap();
        assert!(border_pixels(crate::pdf::render(&pdf, 0, 72.0, None).unwrap(), 0) > 10_000);
        document.set_background_image(Some(&blue)).unwrap();
        assert_eq!(document.background_image().unwrap(), Some(blue));
        assert_eq!(copy.background_image().unwrap(), Some(red));
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        assert!(document.set_background_image(Some(b"invalid PNG")).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);
        document.set_background_image(None).unwrap();
        assert!(document.background_image().unwrap().is_none());
        assert!(document.document().unwrap().media.is_empty());
        assert_eq!(document.background().unwrap().as_deref(), Some("#00FF00"));
        let cleared = document.to_bytes().unwrap();
        assert!(border_pixels(crate::docx::render(&cleared, 0, 72.0, None).unwrap(), 1) > 10_000);
        document.close();
        assert!(document.background_image().is_err());
        assert!(document.set_background_image(None).is_err());
    }

    #[test]
    fn page_background_resource_accounting_is_shared_bounded_and_transactional() {
        let red = media_png(2, 2, [255, 0, 0, 255]);
        let blue = media_png(2, 2, [0, 0, 255, 255]);
        let mut document = EditableDocx::new();
        let image = document.add_image(&red, None, None, "Shared").unwrap();
        document.set_background_image(Some(&red)).unwrap();
        assert_eq!(document.media_bytes, red.len());
        assert_eq!(document.document().unwrap().media.len(), 1);
        document.set_background_image(Some(&blue)).unwrap();
        assert_eq!(document.media_bytes, red.len() + blue.len());
        document.remove_image(image).unwrap();
        assert_eq!(document.media_bytes, blue.len());
        assert_eq!(document.document().unwrap().media.len(), 1);
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        let mut missing = document.document().unwrap().clone();
        missing.media.clear();
        assert!(document.commit(missing).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);

        // Synthetic encoded byte lengths exercise aggregate accounting without
        // decoding or generating huge PNGs; the live owner remains a valid PNG.
        let mut candidate = document.document().unwrap().clone();
        let key = candidate
            .settings
            .page_picture
            .as_ref()
            .unwrap()
            .media
            .clone();
        candidate
            .media
            .insert(key.clone(), Arc::new(vec![0; MAX_MEDIA_BYTES]));
        assert_eq!(
            retain_authored_image_media(&mut candidate).unwrap(),
            MAX_MEDIA_BYTES
        );
        candidate
            .media
            .insert(key, Arc::new(vec![0; MAX_MEDIA_BYTES + 1]));
        assert!(retain_authored_image_media(&mut candidate).is_err());
        candidate.settings.page_picture = None;
        assert_eq!(retain_authored_image_media(&mut candidate).unwrap(), 0);
        assert!(candidate.media.is_empty());
        document.set_background_image(None).unwrap();
        assert_eq!(document.media_bytes, 0);
    }

    #[test]
    fn page_fields_and_images_share_accounting_but_raw_forms_remain_rejected() {
        let png = media_png(2, 2, [0, 0, 255, 255]);
        for image_first in [true, false] {
            let mut document = EditableDocx::new();
            if image_first {
                document.add_image(&png, Some(32.0), None, "Image").unwrap();
            }
            let paragraph = document.add_paragraph("Page ").unwrap();
            document.add_paragraph_field(paragraph, "page").unwrap();
            if !image_first {
                document.add_image(&png, Some(32.0), None, "Image").unwrap();
            }
            assert_eq!(document.media_bytes, png.len());
            let original = document.to_bytes().unwrap();
            let revision = document.revision().unwrap();
            let mut candidate = document.document().unwrap().clone();
            let InlineObject::Field { form, .. } = &mut document
                .paragraph_mut(&mut candidate, paragraph)
                .unwrap()
                .objects[0]
            else {
                panic!("missing native field");
            };
            *form = "<w:ffData/>".into();
            assert!(document.commit(candidate).is_err());
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.media_bytes, png.len());
            assert_eq!(document.to_bytes().unwrap(), original);
            let read = wordcraft_docx::read(&original).unwrap();
            assert_eq!(read.media.len(), 1);
            assert_eq!(page_field_count(&read).unwrap(), 1);
        }
    }

    #[test]
    fn metadata_field_expansion_limits_recheck_later_updates_atomically() {
        pyo3::prepare_freethreaded_python();
        // Synthetic repeated native objects hit exact bounds without thousands
        // of public transactions or allocating their evaluated display strings.
        for (paragraphs, per_paragraph, message) in [(1, 16, "1 MiB"), (32, 8, "16 MiB")] {
            let mut document = EditableDocx::new();
            let value = "x".repeat(MAX_PROPERTY_VALUE_BYTES - wordcraft_doc::para::OBJ.len_utf8());
            document
                .update_metadata(&json!({"title":value}).to_string())
                .unwrap();
            let id = document.add_paragraph("").unwrap();
            document.add_paragraph_field(id, "title").unwrap();
            let mut paragraph = document.paragraph(id).unwrap().clone();
            paragraph.text = wordcraft_doc::para::OBJ.to_string().repeat(per_paragraph);
            paragraph.objects = vec![paragraph.objects[0].clone(); per_paragraph];
            paragraph.touch();
            let mut candidate = document.document().unwrap().clone();
            candidate.body = (0..paragraphs)
                .map(|_| Arc::new(Block::Para(paragraph.clone())))
                .collect();
            document.commit(candidate).unwrap();
            document.text_bytes = paragraphs * per_paragraph * wordcraft_doc::para::OBJ.len_utf8();
            assert_eq!(
                page_field_count(document.document().unwrap()).unwrap(),
                paragraphs * per_paragraph
            );
            let before = wordcraft_docx::write(document.document().unwrap()).unwrap();
            let metadata = document.metadata().unwrap();
            let revision = document.revision().unwrap();
            let error = document
                .update_metadata(&json!({"title":format!("{value}x")}).to_string())
                .unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
            assert_eq!(document.metadata().unwrap(), metadata);
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(
                wordcraft_docx::write(document.document().unwrap()).unwrap(),
                before
            );
            // Lowering the value reclaims evaluated bytes without changing fields.
            document.update_metadata(r#"{"title":""}"#).unwrap();
            document
                .update_metadata(&json!({"title":value}).to_string())
                .unwrap();
            assert_eq!(document.metadata().unwrap(), metadata);
        }
        // The normal commit path counts independent copied header parts too.
        let mut document = EditableDocx::new();
        document.update_metadata(r#"{"title":"Copied"}"#).unwrap();
        let part = document.ensure_story("header", "default", 0).unwrap();
        let paragraph = document.story_add_paragraph(part, "").unwrap();
        document.add_paragraph_field(paragraph, "title").unwrap();
        document.add_section("next_page").unwrap();
        assert_eq!(page_field_count(document.document().unwrap()).unwrap(), 2);
        assert_eq!(
            metadata_field_offsets(document.document().unwrap())
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn metadata_field_placement_charges_repeated_occurrences_and_native_byte_offsets() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        document.update_metadata(r#"{"title":"Abc"}"#).unwrap();
        let id = document.add_paragraph("é😀 ").unwrap();
        let (start, end, _) = document.add_paragraph_field(id, "title").unwrap();
        assert_eq!((start, end), (3, 4));
        let fields = metadata_field_offsets(document.document().unwrap()).unwrap();
        assert_eq!(
            fields[&(StoryRef::Body, Path::top(0))],
            BTreeMap::from([(7, 3)])
        );
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        let page = &layout.pages[0];
        let (mut work, mut bytes) = (0, 0);
        validate_metadata_field_placement(page, &fields, &mut work, &mut bytes).unwrap();
        assert_eq!(bytes, 3);
        let cost = work;
        assert!(cost >= 3);
        validate_metadata_field_placement(page, &fields, &mut work, &mut bytes).unwrap();
        assert_eq!((work, bytes), (cost * 2, 6));
        // Exact-bound then repeated-placement failure; no deduplication discount.
        let mut work = MAX_LAYOUT_ITEMS - cost;
        let mut bytes = MAX_DOCUMENT_TEXT_BYTES - 3;
        validate_metadata_field_placement(page, &fields, &mut work, &mut bytes).unwrap();
        assert_eq!((work, bytes), (MAX_LAYOUT_ITEMS, MAX_DOCUMENT_TEXT_BYTES));
        assert!(
            validate_metadata_field_placement(page, &fields, &mut work, &mut bytes)
                .unwrap_err()
                .to_string()
                .contains("scan units")
        );
        work = 0;
        assert!(
            validate_metadata_field_placement(page, &fields, &mut work, &mut bytes)
                .unwrap_err()
                .to_string()
                .contains("display text")
        );
        document
            .format_paragraph(id, r#"{"direction":"rtl"}"#)
            .unwrap();
        let rtl = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        validate_metadata_field_placement(&rtl.pages[0], &fields, &mut 0, &mut 0).unwrap();
        let mut repeated = EditableDocx::new();
        repeated.update_metadata(r#"{"title":"Abc"}"#).unwrap();
        repeated.add_paragraph("First page").unwrap();
        for kind in ["header", "footer"] {
            let story = repeated.ensure_story(kind, "default", 0).unwrap();
            let paragraph = repeated.story_add_paragraph(story, "").unwrap();
            repeated.add_paragraph_field(paragraph, "title").unwrap();
        }
        for _ in 0..2 {
            repeated.add_page_break().unwrap();
            repeated.add_paragraph("Later page").unwrap();
        }
        let repeated_fields = metadata_field_offsets(repeated.document().unwrap()).unwrap();
        let repeated_layout = wordcraft_layout::layout(
            repeated.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(repeated_layout.pages.len(), 3);
        assert_eq!(repeated_fields.len(), 2);
        let (mut repeated_work, mut repeated_bytes) = (0, 0);
        for placed_page in &repeated_layout.pages {
            let before = repeated_bytes;
            validate_metadata_field_placement(
                placed_page,
                &repeated_fields,
                &mut repeated_work,
                &mut repeated_bytes,
            )
            .unwrap();
            assert_eq!(repeated_bytes - before, 6);
        }
        assert_eq!(repeated_bytes, 18);
        let pdf = crate::docx::to_pdf(&repeated.to_bytes().unwrap(), None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 3);
        for exported_page in &extracted.pages {
            assert_eq!(
                exported_page
                    .blocks
                    .iter()
                    .map(|block| block.text.matches("Abc").count())
                    .sum::<usize>(),
                2
            );
        }
        let mut invalid = page.clone();
        let line = invalid
            .items
            .iter_mut()
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { para, .. } => Some(Arc::make_mut(para)),
                _ => None,
            })
            .unwrap();
        line.lines[0].xs.fill(f32::NAN);
        assert!(validate_metadata_field_placement(&invalid, &fields, &mut 0, &mut 0).is_err());
    }

    #[test]
    fn upstream_metadata_fields_evaluate_all_stories_and_preserve_empty_caches() {
        let mut owned = EditableDocx::new();
        let body = owned.add_paragraph("Body ").unwrap();
        owned.add_table(vec![vec!["Cell ".into()]]).unwrap();
        for kind in ["header", "footer"] {
            let story = owned.ensure_story(kind, "default", 0).unwrap();
            owned
                .story_add_paragraph(story, &format!("{kind} "))
                .unwrap();
        }
        for kind in ["footnote", "endnote"] {
            owned
                .add_note(
                    body,
                    0,
                    4,
                    owned.revision().unwrap(),
                    &format!("{kind} "),
                    kind,
                )
                .unwrap();
        }
        let mut native = owned.document().unwrap().clone();
        let stories: Vec<_> = std::iter::once(StoryRef::Body)
            .chain(native.parts.keys().copied().map(StoryRef::Part))
            .collect();
        let mut count = 0;
        for story in &stories {
            for path in native.para_paths(*story) {
                let paragraph = native.para_mut(*story, &path).unwrap();
                if paragraph.text.is_empty() {
                    continue;
                }
                for instruction in ["TITLE", "AUTHOR"] {
                    paragraph
                        .insert_object(
                            paragraph.text.len(),
                            InlineObject::Field {
                                instr: instruction.into(),
                                result: String::new(),
                                locked: false,
                                dirty: true,
                                code: false,
                                form: String::new(),
                            },
                            &CharProps::default(),
                        )
                        .unwrap();
                    paragraph
                        .insert_text(paragraph.text.len(), " ", &CharProps::default())
                        .unwrap();
                    count += 1;
                }
            }
        }
        assert_eq!(count, 12);
        let mut previous = None;
        for (title, author) in [("ReportOne", "WriterOne"), ("ReportTwo", "WriterTwo")] {
            native.core.title = title.into();
            native.core.creator = author.into();
            let bytes = wordcraft_docx::write(&native).unwrap();
            let read = wordcraft_docx::read(&bytes).unwrap();
            let mut fields = 0;
            for story in std::iter::once(StoryRef::Body)
                .chain(read.parts.keys().copied().map(StoryRef::Part))
            {
                for path in read.para_paths(story) {
                    for object in &read.para(story, &path).unwrap().objects {
                        if let InlineObject::Field {
                            instr,
                            result,
                            dirty,
                            ..
                        } = object
                        {
                            assert!(matches!(instr.as_str(), "TITLE" | "AUTHOR"));
                            assert!(result.is_empty() && *dirty);
                            fields += 1;
                        }
                    }
                }
            }
            assert_eq!(fields, count);
            let png = crate::docx::render(&bytes, 0, 144.0, None).unwrap();
            if let Some(previous) = previous {
                assert_ne!(png, previous);
            }
            previous = Some(png);
            let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
            let extracted = crate::pdf::extract(&pdf, None).unwrap();
            let text = extracted
                .pages
                .iter()
                .flat_map(|page| &page.blocks)
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(text.matches(title).count(), 6, "{text}");
            assert_eq!(text.matches(author).count(), 6, "{text}");
            for label in ["Body", "Cell", "header", "footer", "footnote", "endnote"] {
                assert!(text.contains(label), "{text}");
            }
            let roundtrip = wordcraft_docx::write(&read).unwrap();
            assert_eq!(
                crate::docx::render(&roundtrip, 0, 144.0, None).unwrap(),
                previous.clone().unwrap()
            );
        }
    }

    #[test]
    fn upstream_metadata_fields_keep_cached_results_and_shape_one_cluster() {
        let context = wordcraft_layout::FieldCtx {
            title: "CurrentTitle".into(),
            author: "CurrentAuthor".into(),
            ..Default::default()
        };
        for (instruction, expected) in [("TITLE", "CurrentTitle"), ("AUTHOR", "CurrentAuthor")] {
            assert_eq!(
                wordcraft_layout::fields::field_text(instruction, "", &context),
                (expected.into(), false)
            );
            assert_eq!(
                wordcraft_layout::fields::field_text(instruction, "CachedValue", &context),
                ("CachedValue".into(), false)
            );
        }
        let mut native = Document::from_text("");
        native.core.title = "Long title ".repeat(80);
        let paragraph = native.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
        paragraph
            .insert_object(
                0,
                InlineObject::Field {
                    instr: "TITLE".into(),
                    result: String::new(),
                    locked: false,
                    dirty: true,
                    code: false,
                    form: String::new(),
                },
                &CharProps::default(),
            )
            .unwrap();
        let layout =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        let paragraph = layout
            .pages
            .iter()
            .flat_map(|page| &page.items)
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { para, .. } => Some(para),
                _ => None,
            })
            .unwrap();
        assert_eq!(paragraph.clusters.len(), 1);
        assert!(paragraph.clusters[0].adv > layout.pages[0].w);
    }

    #[test]
    fn page_fields_use_native_objects_and_revision_bound_ranges() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Page ").unwrap();
        document
            .set_section_props(0, r#"{"page_number_start":7}"#)
            .unwrap();
        let (start, end, revision) = document.add_paragraph_field(paragraph, "page").unwrap();
        assert_eq!((start, end), (5, 6));
        assert_eq!(
            document
                .range_text(paragraph, start, end, revision)
                .unwrap(),
            "\u{fffc}"
        );
        let raw = document.to_bytes().unwrap();
        let native = wordcraft_docx::read(&raw).unwrap();
        let p = native.para(StoryRef::Body, &Path::top(0)).unwrap();
        let InlineObject::Field {
            instr,
            result,
            dirty,
            locked,
            ..
        } = &p.objects[0]
        else {
            panic!("missing field")
        };
        assert_eq!(instr, "PAGE \\* Arabic");
        assert!(result.is_empty() && *dirty && !*locked);
        assert_eq!(native.last_section.page_num_start, Some(7));
        let source = crate::docx::extract(&raw, None).unwrap();
        assert_eq!(source.pages[0].blocks[0].text, "Page ");
        let pdf = crate::docx::to_pdf(&raw, None).unwrap();
        assert!(crate::pdf::extract(&pdf, None).unwrap().pages[0]
            .blocks
            .iter()
            .any(|block| block.text.contains("Page 7")));
        let copy = document.copy().unwrap();
        document
            .replace_range(paragraph, start, end, revision, "literal")
            .unwrap();
        assert_eq!(document.paragraph_text(paragraph).unwrap(), "Page literal");
        assert_eq!(page_field_count(document.document().unwrap()).unwrap(), 0);
        assert_eq!(page_field_count(copy.document().unwrap()).unwrap(), 1);
        let section = document.add_section("next_page").unwrap();
        assert!(document.section(section).unwrap().page_num_start.is_none());
    }

    #[test]
    fn page_field_bounds_and_scope_edits_are_transactional() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("").unwrap();
        document.add_paragraph_field(paragraph, "page").unwrap();
        let raw = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for patch in [
            r#"{"page_number_start":true}"#,
            r#"{"page_number_start":0}"#,
            r#"{"page_number_start":32768}"#,
            r#"{"page_number_start":1.5}"#,
        ] {
            assert!(document.set_section_props(0, patch).is_err());
        }
        assert!(document.add_paragraph_field(paragraph, "date").is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), raw);
        // Synthetic native model reaches the bound without 10000 public transactions.
        let mut candidate = document.document().unwrap().clone();
        let p = document.paragraph_mut(&mut candidate, paragraph).unwrap();
        p.text = wordcraft_doc::para::OBJ.to_string().repeat(MAX_PAGE_FIELDS);
        p.objects = vec![p.objects[0].clone(); MAX_PAGE_FIELDS];
        p.runs[0].len = p.text.len();
        p.touch();
        document.commit(candidate).unwrap();
        document.text_bytes = MAX_PAGE_FIELDS * wordcraft_doc::para::OBJ.len_utf8();
        let revision = document.revision().unwrap();
        assert!(document
            .add_paragraph_field(paragraph, "page_count")
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document
            .replace_range(paragraph, 0, 1, revision, "")
            .unwrap();
        document
            .add_paragraph_field(paragraph, "page_count")
            .unwrap();
        assert_eq!(
            page_field_count(document.document().unwrap()).unwrap(),
            MAX_PAGE_FIELDS
        );
        document.text_bytes = MAX_DOCUMENT_TEXT_BYTES;
        assert!(document.add_paragraph_field(paragraph, "page").is_err());
        document.close();
        assert!(document.add_paragraph_field(paragraph, "page").is_err());
    }

    #[test]
    fn upstream_intermediate_section_end_collects_earlier_deferred_endnotes() {
        let mut document = EditableDocx::new();
        document.set_page_size(360.0, 420.0).unwrap();
        document.set_margins(40.0, 40.0, 40.0, 40.0).unwrap();
        let anchor = document.add_paragraph("First anchor").unwrap();
        let note = document
            .add_note(
                anchor,
                6,
                12,
                document.revision().unwrap(),
                "Deferred count=",
                "endnote",
            )
            .unwrap();
        let collector = document.add_section("continuous").unwrap();
        document
            .set_section_note_options(collector, "endnote", r#"{"placement":"section_end"}"#)
            .unwrap();
        document.add_paragraph("Collector begins").unwrap();
        document.add_page_break().unwrap();
        document.add_paragraph("Collector second page").unwrap();
        let last = document.add_section("next_page").unwrap();
        document
            .set_section_note_options(last, "endnote", r#"{"placement":"document_end"}"#)
            .unwrap();
        document.add_paragraph("Final section").unwrap();
        let paragraph = document.story_paragraph_ids(note).unwrap()[0];
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .add_paragraph_field(paragraph, "section_page_count")
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);
        document.add_paragraph_field(paragraph, "page").unwrap();
        // Bypass our authoring guard only in this raw native fidelity reproduction.
        let mut native = document.document().unwrap().clone();
        let p = document.paragraph_mut(&mut native, paragraph).unwrap();
        let InlineObject::Field { instr, .. } = &mut p.objects[0] else {
            panic!("missing field")
        };
        *instr = "SECTIONPAGES \\* Arabic".into();
        p.touch();
        let bytes = wordcraft_docx::write(&native).unwrap();
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let output = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(output.pages.len(), 3);
        let texts: Vec<String> = output
            .pages
            .iter()
            .map(|page| {
                page.blocks
                    .iter()
                    .map(|block| block.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert!(texts[0].contains("Collector begins"));
        assert!(texts[1].contains("Collector second page"));
        // The intermediate collector spans both pages, but native SECTIONPAGES is 1.
        assert!(
            texts[1]
                .split_whitespace()
                .collect::<String>()
                .contains("Deferredcount=1"),
            "{:?}",
            texts
        );
        assert!(!texts[2].contains("Deferred"));
    }

    #[test]
    fn upstream_page_fields_expose_format_and_continuous_section_limits() {
        let mut document = EditableDocx::new();
        let first = document.add_paragraph("First ").unwrap();
        document.add_paragraph_field(first, "page").unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let head = document.story_add_paragraph(header, "Header ").unwrap();
        document.add_paragraph_field(head, "page").unwrap();
        let mut native = document.document().unwrap().clone();
        native.last_section.page_num_start = Some(3);
        native.last_section.page_num_format = NumFormat::LowerRoman;
        for id in [first, head] {
            if let InlineObject::Field { instr, .. } =
                &mut document.paragraph_mut(&mut native, id).unwrap().objects[0]
            {
                *instr = "PAGE".into();
            }
        }
        let raw = wordcraft_docx::write(&native).unwrap();
        let pdf = crate::docx::to_pdf(&raw, None).unwrap();
        let payload = crate::pdf::extract(&pdf, None).unwrap();
        let text = payload.pages[0]
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("First 3") && text.contains("Header iii"),
            "{text}"
        );
        assert_eq!(
            NumFormat::LowerRoman.format(50_001),
            NumFormat::LowerRoman.format(50_000)
        );
        assert_eq!(
            NumFormat::UpperLetter.format(781),
            NumFormat::UpperLetter.format(780)
        );

        let section = document.add_section("continuous").unwrap();
        let second = document.add_paragraph("Second ").unwrap();
        document.add_paragraph_field(second, "page").unwrap();
        assert!(document
            .add_paragraph_field(second, "section_page_count")
            .is_err());
        assert!(document
            .set_section_props(section, r#"{"page_number_start":10}"#)
            .is_err());
        document.add_page_break().unwrap();
        let third = document.add_paragraph("Third ").unwrap();
        document.add_paragraph_field(third, "page").unwrap();
        let mut native = document.document().unwrap().clone();
        native.last_section.page_num_start = Some(10);
        for id in [second, third] {
            if let InlineObject::Field { instr, .. } =
                &mut document.paragraph_mut(&mut native, id).unwrap().objects[0]
            {
                *instr = "SECTIONPAGES \\* Arabic".into();
            }
        }
        let raw = wordcraft_docx::write(&native).unwrap();
        let native = wordcraft_docx::read(&raw).unwrap();
        let layout =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        assert_eq!(layout.pages.len(), 2);
        assert_eq!(layout.pages[1].number, 2); // continuous restart 10 was ignored
        let pdf = crate::docx::to_pdf(&raw, None).unwrap();
        let payload = crate::pdf::extract(&pdf, None).unwrap();
        let text = payload
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // Section two occupies both physical pages; native reports only its owned last page.
        assert!(
            text.contains("Second 1") && text.contains("Third 1"),
            "{text}"
        );
    }

    #[test]
    fn picture_transforms_roundtrip_preserve_media_and_failed_changes_are_atomic() {
        let source = media_png(40, 20, [0, 0, 255, 255]);
        let mut document = EditableDocx::new();
        let id = document
            .add_image(&source, Some(96.0), Some(48.0), "Kept")
            .unwrap();
        document.set_image_crop(id, (0.25, 0.0, 0.0, 0.0)).unwrap();
        document
            .set_image_flips(id, Some(true), Some(true))
            .unwrap();
        let copy = document.copy().unwrap();
        let replacement = media_png(20, 40, [255, 0, 0, 255]);
        document.set_image_data(id, &replacement).unwrap();
        document.set_image_size(id, 80.0, 40.0).unwrap();
        assert_eq!(document.image_crop(id).unwrap(), (0.25, 0.0, 0.0, 0.0));
        assert_eq!(document.image_flips(id).unwrap(), (true, true));
        assert_eq!(copy.image_size(id).unwrap(), (96.0, 48.0));
        assert_eq!(copy.image_data(id).unwrap(), source);
        let raw = document.to_bytes().unwrap();
        let native = wordcraft_docx::read(&raw).unwrap();
        let (story, path) = document.location(id).unwrap();
        let InlineObject::Image {
            crop,
            float,
            w,
            h,
            alt,
            ..
        } = &native.para(story, &path).unwrap().objects[0]
        else {
            panic!("missing image")
        };
        assert_eq!(*crop, [0.25, 0.0, 0.0, 0.0]);
        assert!(float.flip_h && float.flip_v);
        assert_eq!((*w, *h, alt.as_str()), (80.0, 40.0, "Kept"));
        let revision = document.revision().unwrap();
        for crop in [
            (f64::NAN, 0.0, 0.0, 0.0),
            (0.0, f64::INFINITY, 0.0, 0.0),
            (-0.01, 0.0, 0.0, 0.0),
            (0.0, 0.0, 0.451, 0.0),
        ] {
            assert!(document.set_image_crop(id, crop).is_err());
        }
        for (w, h) in [(0.99, 20.0), (20.0, 0.99)] {
            assert!(document.set_image_size(id, w, h).is_err());
            assert!(document
                .add_image(&source, Some(w), Some(h), "Invalid")
                .is_err());
        }
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), raw);
        document.set_image_size(id, 1.0, 1.0).unwrap();
        document
            .set_image_crop(id, (0.45, 0.45, 0.45, 0.45))
            .unwrap();
        document.set_image_flips(id, Some(false), None).unwrap();
        assert_eq!(document.image_flips(id).unwrap(), (false, true));
        document.close();
        assert!(document.image_crop(id).is_err());
        assert!(document.set_image_flips(id, Some(true), None).is_err());
    }

    #[test]
    fn upstream_picture_frames_below_one_point_are_silently_enlarged() {
        let mut document = EditableDocx::new();
        let id = document
            .add_image(
                &media_png(2, 2, [0, 0, 255, 255]),
                Some(1.0),
                Some(1.0),
                "Tiny",
            )
            .unwrap();
        let mut raw = document.document().unwrap().clone();
        let para = document.paragraph_mut(&mut raw, id).unwrap();
        let InlineObject::Image { w, h, .. } = &mut para.objects[0] else {
            panic!("missing image")
        };
        *w = 0.05;
        *h = 0.05;
        para.touch();
        let native = wordcraft_docx::read(&wordcraft_docx::write(&raw).unwrap()).unwrap();
        let layout =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        let frames: Vec<_> = layout
            .pages
            .iter()
            .flat_map(|page| &page.items)
            .filter_map(|item| {
                if let wordcraft_layout::Placed::Object { rect, .. } = item {
                    Some((rect.w, rect.h))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(frames, vec![(1.0, 1.0)]);
    }

    #[test]
    fn upstream_quarter_turn_picture_loses_solid_edge_pixels() {
        // Native Float + writer supplies matching effectExtent/spin_pad. This is not
        // malformed XML: render/lib.rs and pdf/lib.rs both lose part of a solid edge.
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 300.0).unwrap();
        document.set_margins(40.0, 40.0, 40.0, 40.0).unwrap();
        let id = document
            .add_image(
                &media_png(40, 20, [0, 0, 255, 255]),
                Some(96.0),
                Some(48.0),
                "Rotation repro",
            )
            .unwrap();
        let mut native = document.document().unwrap().clone();
        let para = document.paragraph_mut(&mut native, id).unwrap();
        let InlineObject::Image { float, .. } = &mut para.objects[0] else {
            panic!("missing image")
        };
        float.rot = 90.0;
        para.touch();
        let raw = wordcraft_docx::write(&native).unwrap();
        let pdf = crate::docx::to_pdf(&raw, None).unwrap();
        for png in [
            crate::docx::render(&raw, 0, 144.0, None).unwrap(),
            crate::pdf::render(&pdf, 0, 144.0, None).unwrap(),
        ] {
            let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut pixels).unwrap();
            assert_eq!(frame.color_type, png::ColorType::Rgba);
            let span = |y: usize| {
                pixels[y * frame.width as usize * 4..(y + 1) * frame.width as usize * 4]
                    .chunks_exact(4)
                    .filter(|p| p[0] < 20 && p[1] < 20 && p[2] > 240)
                    .count()
            };
            // A 90-degree turn should give a constant-width solid rectangle.
            assert!(
                span(200) >= span(100) + 4,
                "native edge loss no longer reproduced: {} vs {}",
                span(100),
                span(200)
            );
        }
    }

    #[test]
    fn picture_payload_replacement_preserves_geometry_alt_copy_and_visible_output() {
        let red = media_png(16, 8, [255, 0, 0, 255]);
        let blue = media_png(8, 16, [0, 0, 255, 255]);
        let mut document = EditableDocx::new();
        let before = document.add_paragraph("Before picture").unwrap();
        let picture = document
            .add_image(&red, Some(80.0), None, "Kept alt")
            .unwrap();
        let after = document.add_paragraph("After picture").unwrap();
        let copy = document.copy().unwrap();
        document.set_image_data(picture, &blue).unwrap();
        assert_eq!(document.image_data(picture).unwrap(), blue);
        assert_eq!(document.image_size(picture).unwrap(), (80.0, 40.0));
        assert_eq!(document.image_alt(picture).unwrap(), "Kept alt");
        assert_eq!(copy.image_data(picture).unwrap(), red);
        assert_eq!(document.paragraph_text(before).unwrap(), "Before picture");
        assert_eq!(document.paragraph_text(after).unwrap(), "After picture");
        let raw = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&raw).unwrap();
        assert_eq!(reopened.media.len(), 1);
        assert_eq!(reopened.media.values().next().unwrap().as_ref(), &blue);
        let pdf = crate::docx::to_pdf(&raw, None).unwrap();
        assert!(border_pixels(crate::docx::render(&raw, 0, 72.0, None).unwrap(), 2) > 100);
        assert!(border_pixels(crate::pdf::render(&pdf, 0, 72.0, None).unwrap(), 2) > 100);
        let revision = document.revision().unwrap();
        for invalid in [b"not PNG".as_slice(), &blue[..blue.len() - 8]] {
            assert!(document.set_image_data(picture, invalid).is_err());
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), raw);
        }
    }

    #[test]
    fn picture_resources_deduplicate_reclaim_and_never_overwrite_live_keys() {
        let red = media_png(1, 1, [255, 0, 0, 255]);
        let blue = media_png(1, 1, [0, 0, 255, 255]);
        let green = media_png(1, 1, [0, 255, 0, 255]);
        let mut document = EditableDocx::new();
        let first = document.add_image(&red, None, None, "First").unwrap();
        let table = document
            .add_table(vec![vec!["Keep".into()], vec!["Delete".into()]])
            .unwrap();
        let second = document
            .cell_add_image(table, 0, 0, &red, None, None, "Second")
            .unwrap();
        assert_eq!(document.document().unwrap().media.len(), 1);
        assert_eq!(document.media_bytes, red.len());
        document.set_image_data(first, &blue).unwrap();
        assert_eq!(document.document().unwrap().media.len(), 2);
        assert_eq!(document.image_data(second).unwrap(), red);
        document.set_image_data(second, &blue).unwrap();
        assert_eq!(document.document().unwrap().media.len(), 1);
        assert_eq!(document.media_bytes, blue.len());
        document.remove_image(first).unwrap();
        assert!(document.image_data(first).is_err());
        assert_eq!(document.paragraph_text(first).unwrap(), "");
        assert_eq!(document.image_data(second).unwrap(), blue);
        document.set_cell_text(table, 0, 0, "Replacement").unwrap();
        assert!(document.image_data(second).is_err());
        assert_eq!(document.media_bytes, 0);
        assert!(document.document().unwrap().media.is_empty());
        let live = document.add_image(&blue, None, None, "Live").unwrap();
        let dropped = document
            .cell_add_image(table, 1, 0, &red, None, None, "Drop")
            .unwrap();
        document.delete_row(table, 1).unwrap();
        assert!(document.image_data(dropped).is_err());
        let fresh = document.add_image(&green, None, None, "Fresh").unwrap();
        assert_eq!(document.image_data(live).unwrap(), blue);
        assert_eq!(document.image_data(fresh).unwrap(), green);
        assert_eq!(document.media_bytes, blue.len() + green.len());
    }

    #[test]
    fn image_reference_accounting_includes_header_footer_and_note_stories() {
        let png = media_png(1, 1, [255, 0, 0, 255]);
        let mut document = EditableDocx::new();
        let body = document.add_image(&png, None, None, "Shared").unwrap();
        let anchor = document.add_paragraph("Note anchor").unwrap();
        let note = document
            .add_note(
                anchor,
                0,
                4,
                document.revision().unwrap(),
                "Footnote",
                "footnote",
            )
            .unwrap();
        let note_info: Value = serde_json::from_str(&document.note_info(note).unwrap()).unwrap();
        let note_story = note_info["story"].as_u64().unwrap() as u32;
        let mut copies = Vec::new();
        let (story, path) = document.location(body).unwrap();
        let image_para = document
            .document()
            .unwrap()
            .para(story, &path)
            .unwrap()
            .clone();
        for story in [
            document.ensure_story("header", "default", 0).unwrap(),
            document.ensure_story("footer", "default", 0).unwrap(),
            note_story,
        ] {
            let mut candidate = document.document().unwrap().clone();
            let part = candidate.parts.get_mut(&story).unwrap();
            let index = part.blocks.len();
            part.blocks.push(Arc::new(Block::Para(image_para.clone())));
            document.commit(candidate).unwrap();
            copies.push(
                document
                    .register_location(StoryRef::Part(story), Path::top(index), true)
                    .unwrap(),
            );
        }
        document.remove_image(body).unwrap();
        assert_eq!(document.document().unwrap().media.len(), 1);
        for (index, picture) in copies.iter().enumerate() {
            assert_eq!(document.image_data(*picture).unwrap(), png);
            document.remove_image(*picture).unwrap();
            assert_eq!(
                document.document().unwrap().media.len(),
                usize::from(index + 1 < copies.len())
            );
        }
        document.to_bytes().unwrap();
    }

    #[test]
    fn aggregate_media_budget_is_transactional_and_removed_payloads_release_capacity() {
        let red = media_png(1, 1, [255, 0, 0, 255]);
        let blue = media_png(1, 1, [0, 0, 255, 255]);
        let mut document = EditableDocx::new();
        let first = document.add_image(&red, None, None, "First").unwrap();
        let second = document.add_image(&blue, None, None, "Second").unwrap();
        let keys: Vec<_> = [first, second]
            .into_iter()
            .map(|id| match document.image(id).unwrap() {
                InlineObject::Image { media, .. } => media.clone(),
                _ => unreachable!(),
            })
            .collect();
        // Synthetic native media isolates encoded-byte accounting from PNG
        // decoding. Never serialize/render these deliberately non-PNG values.
        let mut candidate = document.document().unwrap().clone();
        for key in &keys {
            candidate
                .media
                .insert(key.clone(), Arc::new(vec![0; MAX_MEDIA_BYTES / 2]));
        }
        document.commit(candidate).unwrap();
        assert_eq!(document.media_bytes, MAX_MEDIA_BYTES);
        let revision = document.revision().unwrap();
        let original = document.document().unwrap().media[&keys[0]].clone();
        let mut oversized = document.document().unwrap().clone();
        oversized
            .media
            .insert(keys[0].clone(), Arc::new(vec![1; MAX_MEDIA_BYTES / 2 + 1]));
        assert!(document.commit(oversized).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.media_bytes, MAX_MEDIA_BYTES);
        assert!(Arc::ptr_eq(
            &document.document().unwrap().media[&keys[0]],
            &original
        ));
        document.remove_image(first).unwrap();
        assert_eq!(document.media_bytes, MAX_MEDIA_BYTES / 2);
        assert!(!document.document().unwrap().media.contains_key(&keys[0]));
        document.set_image_data(second, &blue).unwrap();
        assert_eq!(document.media_bytes, blue.len());
        document
            .add_image(&red, None, None, "Capacity reused")
            .unwrap();
        assert_eq!(document.media_bytes, red.len() + blue.len());
        document.to_bytes().unwrap();
    }

    #[test]
    fn removed_picture_resources_release_limits_and_duplicate_placements_are_bounded() {
        let png = media_png(1, 1, [255, 0, 0, 255]);
        let mut document = EditableDocx::new();
        for _ in 0..1001 {
            let image = document.add_image(&png, None, None, "Transient").unwrap();
            document.remove_image(image).unwrap();
            assert_eq!(document.media_bytes, 0);
        }
        let mut document = EditableDocx::new();
        for _ in 0..1000 {
            document.add_image(&png, None, None, "Shared").unwrap();
        }
        assert_eq!(document.document().unwrap().media.len(), 1);
        assert_eq!(document.media_bytes, png.len());
        let revision = document.revision().unwrap();
        assert!(document.add_image(&png, None, None, "Over limit").is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.image_ids().unwrap().len(), 1000);
    }

    fn border_pixels(png: Vec<u8>, channel: usize) -> usize {
        let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(frame.color_type, png::ColorType::Rgba);
        pixels[..frame.buffer_size()]
            .chunks_exact(4)
            .filter(|pixel| {
                // Count hue dominance, including antialiased edge pixels.
                pixel[channel] > 200
                    && pixel[channel].saturating_sub(pixel[(channel + 1) % 3]) > 40
                    && pixel[channel].saturating_sub(pixel[(channel + 2) % 3]) > 40
            })
            .count()
    }

    #[test]
    fn paragraph_borders_inherit_as_groups_and_reset_direct_sides() {
        let mut document = EditableDocx::new();
        let style = document.add_style("Bordered", "paragraph", None).unwrap();
        document
            .format_defaults(r##"{"borders":{"top":{"color":"#FF0000","width":2}}}"##)
            .unwrap();
        document
            .format_style(
                &style,
                r##"{"borders":{"bottom":{"color":"#00FF00","width":2}}}"##,
                None,
            )
            .unwrap();
        let inherited = document.add_paragraph("Default border").unwrap();
        let paragraph = document.add_paragraph("Style border").unwrap();
        document.set_paragraph_style(paragraph, &style).unwrap();
        let second = document.add_paragraph("Same style border").unwrap();
        document.set_paragraph_style(second, &style).unwrap();
        document
            .format_paragraph(
                paragraph,
                r##"{"borders":{"right":{"color":"#0000FF","width":2}}}"##,
            )
            .unwrap();
        let resolved: Value =
            serde_json::from_str(&document.paragraph_formatting(paragraph, true).unwrap()).unwrap();
        assert!(resolved["borders"]["top"].is_null());
        assert!(resolved["borders"]["bottom"].is_null());
        assert_eq!(resolved["borders"]["right"]["color"], "#0000FF");
        document
            .format_paragraph(
                paragraph,
                r##"{"borders":{"left":{"color":"#FF0000","width":1}}}"##,
            )
            .unwrap();
        document
            .format_paragraph(paragraph, r#"{"borders":{"left":null}}"#)
            .unwrap();
        let direct: Value =
            serde_json::from_str(&document.paragraph_formatting(paragraph, false).unwrap())
                .unwrap();
        assert!(direct["borders"]["left"].is_null());
        assert_eq!(direct["borders"]["right"]["width"], 2.0);
        let output = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&output).unwrap();
        for (id, side, color) in [
            (inherited, "top", "#FF0000"),
            (paragraph, "right", "#0000FF"),
            (second, "bottom", "#00FF00"),
        ] {
            let para = reopened
                .para(StoryRef::Body, &document.location(id).unwrap().1)
                .unwrap();
            let snapshot =
                resolved_para_snapshot(&reopened.styles.resolve_para(&para.props)).unwrap();
            assert_eq!(snapshot["borders"][side]["color"], color);
        }
        let pdf = crate::docx::to_pdf(&output, None).unwrap();
        for channel in 0..3 {
            assert!(
                border_pixels(
                    crate::docx::render(&output, 0, 72.0, None).unwrap(),
                    channel
                ) > 10
            );
            assert!(border_pixels(crate::pdf::render(&pdf, 0, 72.0, None).unwrap(), channel) > 10);
        }
        let mut copy = document.copy().unwrap();
        copy.format_paragraph(paragraph, r#"{"borders":null}"#)
            .unwrap();
        let reset: Value =
            serde_json::from_str(&copy.paragraph_formatting(paragraph, true).unwrap()).unwrap();
        assert_eq!(reset["borders"]["bottom"]["color"], "#00FF00");
        assert!(reset["borders"]["right"].is_null());
        copy.format_style(&style, r#"{"borders":null}"#, None)
            .unwrap();
        let reset: Value =
            serde_json::from_str(&copy.paragraph_formatting(paragraph, true).unwrap()).unwrap();
        assert_eq!(reset["borders"]["top"]["color"], "#FF0000");
        copy.format_defaults(r#"{"borders":null}"#).unwrap();
        let reset: Value =
            serde_json::from_str(&copy.paragraph_formatting(paragraph, true).unwrap()).unwrap();
        assert!(reset["borders"].is_null());
        let output = copy.to_bytes().unwrap();
        assert_eq!(
            border_pixels(crate::docx::render(&output, 0, 72.0, None).unwrap(), 0),
            0
        );
        assert_eq!(
            document.paragraph_formatting(paragraph, false).unwrap(),
            Value::Object(direct.as_object().unwrap().clone()).to_string()
        );
    }

    #[test]
    fn paragraph_border_styles_draw_colored_edges_in_docx_and_pdf() {
        for style in ["single", "double", "dotted", "dashed", "thick", "wave"] {
            let vertical = if matches!(style, "double" | "wave") {
                "single"
            } else {
                style
            };
            let mut document = EditableDocx::new();
            document.set_page_size(300.0, 300.0).unwrap();
            document.set_margins(40.0, 40.0, 40.0, 40.0).unwrap();
            let paragraph = document.add_paragraph("Four colored borders").unwrap();
            document
                .format_paragraph(
                    paragraph,
                    &json!({"borders":{
                        "top":{"style":style,"color":"#FF0000","width":2},
                        "right":{"style":vertical,"color":"#00FF00","width":2},
                        "bottom":{"style":style,"color":"#0000FF","width":2},
                        "left":{"style":vertical,"color":"#FF0000","width":2}
                    }})
                    .to_string(),
                )
                .unwrap();
            let output = document.to_bytes().unwrap();
            let reopened = wordcraft_docx::read(&output).unwrap();
            let props = &reopened
                .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
                .unwrap()
                .props;
            assert_eq!(
                borders_snapshot(props.borders, false)["top"]["style"],
                style
            );
            let pdf = crate::docx::to_pdf(&output, None).unwrap();
            for channel in 0..3 {
                assert!(
                    border_pixels(
                        crate::docx::render(&output, 0, 144.0, None).unwrap(),
                        channel
                    ) > 10,
                    "{style} channel {channel}"
                );
                assert!(
                    border_pixels(crate::pdf::render(&pdf, 0, 144.0, None).unwrap(), channel) > 10,
                    "{style} PDF channel {channel}"
                );
            }
        }
    }

    #[test]
    fn paragraph_borders_render_in_cell_header_and_footer_scopes() {
        for scope in ["cell", "header", "footer"] {
            let mut document = EditableDocx::new();
            document.add_paragraph("Body text").unwrap();
            let paragraph = if scope == "cell" {
                let table = document
                    .add_table(vec![vec!["Cell paragraph".into()]])
                    .unwrap();
                document.cell_paragraph_ids(table, 0, 0).unwrap()[0]
            } else {
                let story = document.ensure_story(scope, "default", 0).unwrap();
                document
                    .story_add_paragraph(story, "Story paragraph")
                    .unwrap()
            };
            document
                .format_paragraph(
                    paragraph,
                    r##"{"borders":{"bottom":{"color":"#FF0000","width":2}}}"##,
                )
                .unwrap();
            let output = document.to_bytes().unwrap();
            let pdf = crate::docx::to_pdf(&output, None).unwrap();
            assert!(
                border_pixels(crate::docx::render(&output, 0, 144.0, None).unwrap(), 0) > 100,
                "{scope}"
            );
            assert!(
                border_pixels(crate::pdf::render(&pdf, 0, 144.0, None).unwrap(), 0) > 100,
                "{scope} PDF"
            );
        }
    }

    #[test]
    fn paragraph_border_patches_are_atomic_and_preserve_table_border_dispatch() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Atomic border").unwrap();
        let style = document
            .add_style("Paragraph border", "paragraph", None)
            .unwrap();
        let table_style = document.add_style("Table border", "table", None).unwrap();
        let table = document.add_table(vec![vec!["Table cell".into()]]).unwrap();
        document
            .format_style(
                &table_style,
                r##"{"borders":{"inside_horizontal":{"color":"#123456","width":1}}}"##,
                None,
            )
            .unwrap();
        let table_snapshot: Value = serde_json::from_str(
            &document
                .style_formatting(&table_style, false, None)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            table_snapshot["borders"]["inside_horizontal"]["color"],
            "#123456"
        );
        assert!(document
            .document()
            .unwrap()
            .styles
            .get(&table_style)
            .unwrap()
            .para
            .borders
            .is_none());
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for borders in [
            json!({"between":{}}),
            json!({"bar":{}}),
            json!({"inside_horizontal":{}}),
            json!({"top":{"style":"triple"}}),
            json!({"left":{"style":"dot_dash"}}),
            json!({"right":{"width":0.125}}),
            json!({"bottom":{"style":"thick","width":1}}),
            json!({"left":{"style":"double","width":2}}),
            json!({"right":{"style":"wave","width":2}}),
            json!({"top":{"space":1}}),
            json!({"top":{"color":"bad"}}),
            json!({"top":{"width":13}}),
        ] {
            let patch = json!({"space_before":10,"borders":borders}).to_string();
            assert!(document.format_paragraph(paragraph, &patch).is_err());
            assert!(document.format_style(&style, &patch, None).is_err());
            assert!(document.format_defaults(&patch).is_err());
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        document
            .format_paragraph(paragraph, r#"{"borders":{"top":{"style":"none"}}}"#)
            .unwrap();
        let direct: Value =
            serde_json::from_str(&document.paragraph_formatting(paragraph, false).unwrap())
                .unwrap();
        assert_eq!(direct["borders"]["top"]["style"], "none");
        assert_eq!(direct["borders"]["top"]["width"], 0.0);
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for border in [
            json!({"style":"triple"}),
            json!({"style":"dot_dash"}),
            json!({"width":0.125}),
            json!({"style":"thick","width":1}),
        ] {
            let patch = json!({"borders":{"bottom":border}}).to_string();
            assert!(document.table_format_patch(table, &patch).is_err());
            assert!(document.cell_format_patch(table, 0, 0, &patch).is_err());
            assert!(document
                .format_style(&table_style, &patch, Some("first_row"))
                .is_err());
            assert!(document.format_style(&table_style, &patch, None).is_err());
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        for side in ["left", "right", "inside_vertical"] {
            for style in ["double", "wave"] {
                let patch = json!({"borders":{side:{"style":style,"width":2}}}).to_string();
                assert!(document.table_format_patch(table, &patch).is_err());
                assert!(document.cell_format_patch(table, 0, 0, &patch).is_err());
                assert!(document
                    .format_style(&table_style, &patch, Some("first_row"))
                    .is_err());
                assert_eq!(document.revision().unwrap(), revision);
                assert_eq!(document.to_bytes().unwrap(), before);
            }
        }
    }

    #[test]
    fn upstream_between_paragraph_border_is_serialized_but_not_drawn() {
        let mut document = Document::new();
        let props = ParaProps {
            borders: Some(Borders {
                between: Some(Border::single(2.0)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut paragraph = Paragraph::with_text("Between borders", CharProps::default());
        paragraph.props = props;
        document.body = vec![
            Arc::new(Block::Para(paragraph.clone())),
            Arc::new(Block::Para(paragraph)),
        ];
        let output = wordcraft_docx::write(&document).unwrap();
        let reopened = wordcraft_docx::read(&output).unwrap();
        assert!(reopened
            .para(StoryRef::Body, &Path::top(0))
            .unwrap()
            .props
            .borders
            .unwrap()
            .between
            .is_some());
        let layout = wordcraft_layout::layout(
            &reopened,
            &mut wordcraft_layout::LayoutCache::new(),
            &wordcraft_layout::LayoutOptions::default(),
        );
        assert!(!layout
            .pages
            .iter()
            .flat_map(|page| &page.items)
            .any(|item| matches!(item, wordcraft_layout::Placed::Rule { .. })));
    }

    #[test]
    fn upstream_border_styles_and_widths_are_approximated_in_native_output() {
        fn raw_border(style: BorderStyle, width: f32) -> Vec<u8> {
            let mut document = Document::new();
            let mut paragraph = Paragraph::with_text("Raw native border", CharProps::default());
            paragraph.props.borders = Some(Borders {
                bottom: Some(Border {
                    style,
                    width,
                    color: Some(Rgb(255, 0, 0)),
                    space: 0.0,
                }),
                ..Default::default()
            });
            document.body = vec![Arc::new(Block::Para(paragraph))];
            wordcraft_docx::write(&document).unwrap()
        }
        for (style, width, substitute, actual_width) in [
            (BorderStyle::Triple, 2.0, BorderStyle::Double, 2.0),
            (BorderStyle::DotDash, 2.0, BorderStyle::Dashed, 2.0),
            (BorderStyle::Single, 0.125, BorderStyle::Single, 0.25),
            (BorderStyle::Thick, 1.0, BorderStyle::Single, 1.5),
        ] {
            let raw = raw_border(style, width);
            let substitute = raw_border(substitute, actual_width);
            assert_eq!(
                crate::docx::render(&raw, 0, 144.0, None).unwrap(),
                crate::docx::render(&substitute, 0, 144.0, None).unwrap()
            );
            let raw_pdf = crate::docx::to_pdf(&raw, None).unwrap();
            let substitute_pdf = crate::docx::to_pdf(&substitute, None).unwrap();
            assert_eq!(
                crate::pdf::render(&raw_pdf, 0, 144.0, None).unwrap(),
                crate::pdf::render(&substitute_pdf, 0, 144.0, None).unwrap()
            );
        }
    }

    #[test]
    fn upstream_vertical_wave_is_empty_and_double_strokes_overlap() {
        for style in [BorderStyle::Wave, BorderStyle::Double] {
            let mut document = Document::new();
            let mut paragraph = Paragraph::with_text("Vertical border", CharProps::default());
            paragraph.props.borders = Some(Borders {
                left: Some(Border {
                    style,
                    width: 2.0,
                    color: Some(Rgb(255, 0, 0)),
                    space: 0.0,
                }),
                ..Default::default()
            });
            document.body = vec![Arc::new(Block::Para(paragraph))];
            let raw = wordcraft_docx::write(&document).unwrap();
            let layout = wordcraft_layout::layout(
                &document,
                &mut wordcraft_layout::LayoutCache::new(),
                &wordcraft_layout::LayoutOptions::default(),
            );
            let (x, y) = layout.pages[0]
                .items
                .iter()
                .find_map(|item| match item {
                    wordcraft_layout::Placed::Rule { x0, y0, x1, y1, .. }
                        if (x0 - x1).abs() < 0.01 =>
                    {
                        Some((*x0, (y0 + y1) / 2.0))
                    }
                    _ => None,
                })
                .unwrap();
            let png = crate::docx::render(&raw, 0, 144.0, None).unwrap();
            if style == BorderStyle::Wave {
                assert_eq!(border_pixels(png, 0), 0);
                continue;
            }
            let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut pixels).unwrap();
            let row = (y * 2.0) as usize;
            let mut runs = 0;
            let mut previous = false;
            for column in ((x - 6.0) * 2.0) as usize..=((x + 6.0) * 2.0) as usize {
                let offset = (row * frame.width as usize + column) * 4;
                let p = &pixels[offset..offset + 4];
                let red = p[0] > 200 && p[1] < 80 && p[2] < 80;
                if red && !previous {
                    runs += 1;
                }
                previous = red;
            }
            assert_eq!(
                runs, 1,
                "native vertical double produces one overlapping stroke, not two separated strokes"
            );
        }
    }

    fn pagination_row(lines: usize) -> (EditableDocx, usize, usize) {
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 300.0).unwrap();
        document.set_margins(30.0, 30.0, 30.0, 30.0).unwrap();
        document
            .format_defaults(
                r#"{"font_size":12,"space_before":0,"space_after":0,"line_spacing":1}"#,
            )
            .unwrap();
        let text = (0..lines)
            .map(|line| format!("CellLine{line:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let table = document.add_table(vec![vec![text]]).unwrap();
        let paragraph = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        document
            .format_paragraph(
                paragraph,
                r#"{"keep_together":false,"widow_control":false}"#,
            )
            .unwrap();
        document.set_row_allow_split(table, 0, false).unwrap();
        (document, table, paragraph)
    }

    #[test]
    fn oversized_unsplittable_row_fails_serialization_and_recovers() {
        let (mut document, table, _) = pagination_row(24);
        let revision = document.revision().unwrap();
        assert!(document.to_bytes().is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.set_row_allow_split(table, 0, true).unwrap();
        let output = document.to_bytes().unwrap();
        assert_eq!(crate::docx::extract(&output, None).unwrap().pages.len(), 2);
    }

    #[test]
    fn upstream_oversized_unsplittable_row_has_off_page_geometry() {
        let (document, _, _) = pagination_row(24);
        let raw = wordcraft_docx::write(document.document().unwrap()).unwrap();
        let reopened = wordcraft_docx::read(&raw).unwrap();
        let layout = wordcraft_layout::layout(
            &reopened,
            &mut wordcraft_layout::LayoutCache::new(),
            &wordcraft_layout::LayoutOptions::default(),
        );
        assert!(layout.pages.iter().any(|page| {
            page.items.iter().any(|item| {
            matches!(item, wordcraft_layout::Placed::Cell { rect, .. } if rect.bottom() > page.h)
        })
        }));
        let pdf = crate::docx::to_pdf(&raw, None).unwrap();
        let payload = crate::pdf::extract(&pdf, None).unwrap();
        let last = payload.pages[0]
            .blocks
            .iter()
            .find(|block| block.text.contains("CellLine23"))
            .unwrap();
        assert!(last.bbox.unwrap()[1] > 300.0);
    }

    #[test]
    fn pagination_guard_catches_later_font_margin_and_text_growth() {
        let (source, table, paragraph) = pagination_row(12);
        source.to_bytes().unwrap();
        let mut font = source.copy().unwrap();
        font.format_defaults(r#"{"font_size":28}"#).unwrap();
        assert!(font.to_bytes().is_err());
        let mut margin = source.copy().unwrap();
        margin.set_margins(150.0, 30.0, 30.0, 30.0).unwrap();
        assert!(margin.to_bytes().is_err());
        let mut text = source.copy().unwrap();
        text.set_paragraph_text(
            paragraph,
            &(0..24)
                .map(|line| format!("Expanded{line}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert!(text.to_bytes().is_err());
        text.set_row_allow_split(table, 0, true).unwrap();
        text.to_bytes().unwrap();
        source.to_bytes().unwrap();
    }

    #[test]
    fn pagination_guard_preserves_exact_height_clipping_and_vertical_merges() {
        let (mut clipped, table, _) = pagination_row(24);
        clipped
            .set_row_height(table, 0, Some(30.0), "exact")
            .unwrap();
        clipped.to_bytes().unwrap();
        let mut merged = EditableDocx::new();
        merged.set_page_size(300.0, 300.0).unwrap();
        merged.set_margins(30.0, 30.0, 30.0, 30.0).unwrap();
        let table = merged
            .add_table(
                (0..20)
                    .map(|row| vec![format!("Left{row}"), format!("Right{row}")])
                    .collect(),
            )
            .unwrap();
        merged.merge_cells(table, 0, 0, 19, 0).unwrap();
        merged.set_cell_text(table, 0, 0, "MergedAnchor").unwrap();
        for row in 0..20 {
            merged.set_row_allow_split(table, row, false).unwrap();
        }
        let raw = merged.to_bytes().unwrap();
        assert!(crate::docx::extract(&raw, None).unwrap().pages.len() > 1);
    }

    #[test]
    fn pagination_guard_uses_absolute_header_and_footer_table_geometry() {
        let (mut document, table, _) = pagination_row(1);
        document
            .set_row_height(table, 0, Some(18.0), "exact")
            .unwrap();
        let native_table = document.table(table).unwrap().clone();
        for kind in ["header", "footer"] {
            let story = document.ensure_story(kind, "default", 0).unwrap();
            // Story table insertion is not yet public authoring API; exercise
            // existing public native parts rather than inventing a facade here.
            document
                .document
                .as_mut()
                .unwrap()
                .parts
                .get_mut(&story)
                .unwrap()
                .blocks = vec![Arc::new(Block::Table(native_table.clone()))];
        }
        let raw = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&raw).unwrap();
        let layout = wordcraft_layout::layout(
            &reopened,
            &mut wordcraft_layout::LayoutCache::new(),
            &wordcraft_layout::LayoutOptions::default(),
        );
        for page in &layout.pages {
            for story in [&page.header, &page.footer] {
                assert!(story.iter().any(|item| matches!(item, wordcraft_layout::Placed::Cell { rect, .. } if rect.y >= 0.0 && rect.bottom() <= page.h)));
            }
        }
        crate::docx::to_pdf(&raw, None).unwrap();
        crate::docx::render(&raw, 0, 72.0, None).unwrap();
    }

    #[test]
    fn style_relations_are_reciprocal_bounded_and_roundtrip() {
        let mut document = EditableDocx::new();
        let first = document
            .add_style("First paragraph", "paragraph", None)
            .unwrap();
        let second = document
            .add_style("Second paragraph", "paragraph", None)
            .unwrap();
        let character = document
            .add_style("Linked character", "character", None)
            .unwrap();
        let replacement = document
            .add_style("Replacement character", "character", None)
            .unwrap();
        let table = document
            .add_style("Unrelated table", "table", None)
            .unwrap();
        document
            .style_relations_patch(
                &first,
                &json!({"next":"Second paragraph","linked":"Linked character"}).to_string(),
            )
            .unwrap();
        document
            .style_relations_patch(&second, &json!({"next":first}).to_string())
            .unwrap();
        let info: Value = serde_json::from_str(&document.style_info(&first).unwrap()).unwrap();
        assert_eq!(info["next"], second);
        assert_eq!(info["linked"], character);
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .get(&character)
                .unwrap()
                .linked
                .as_deref(),
            Some(first.as_str())
        );
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        for (id, patch) in [
            (&second, json!({"linked":character})),
            (&first, json!({"next":first,"linked":second})),
            (&character, json!({"next":first})),
            (&table, json!({"linked":null})),
            (&first, json!({"next":character})),
            (&first, json!({"linked":"missing"})),
            (&first, json!({"linked":false})),
            (&first, json!({"based_on":second})),
        ] {
            assert!(document
                .style_relations_patch(id, &patch.to_string())
                .is_err());
            assert_eq!(document.document().unwrap(), &before);
            assert_eq!(document.revision().unwrap(), revision);
        }
        assert!(document
            .style_relations_patch(&first, &" ".repeat(MAX_FORMAT_PATCH_BYTES + 1))
            .is_err());
        document
            .style_relations_patch(
                &first,
                &json!({"linked":replacement,"next":first}).to_string(),
            )
            .unwrap();
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .get(&character)
                .unwrap()
                .linked,
            None
        );
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(
            reopened.styles.get(&first).unwrap().next.as_deref(),
            Some(first.as_str())
        );
        assert_eq!(
            reopened.styles.get(&first).unwrap().linked.as_deref(),
            Some(replacement.as_str())
        );
        assert_eq!(
            reopened.styles.get(&replacement).unwrap().linked.as_deref(),
            Some(first.as_str())
        );
        let mut copy = document.copy().unwrap();
        copy.style_relations_patch(&replacement, r#"{"linked":null}"#)
            .unwrap();
        assert_eq!(
            copy.document().unwrap().styles.get(&first).unwrap().linked,
            None
        );
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .get(&first)
                .unwrap()
                .linked
                .as_deref(),
            Some(replacement.as_str())
        );
        document
            .style_relations_patch(&first, r#"{"next":null,"linked":null}"#)
            .unwrap();
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .get(&first)
                .unwrap()
                .next,
            None
        );
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .get(&replacement)
                .unwrap()
                .linked,
            None
        );
        document.close();
        assert!(document.style_relations_patch(&first, "{}").is_err());
        assert!(document.default_formatting().is_err());
        assert!(document.format_defaults("{}").is_err());
        assert!(copy.style_info(&first).is_ok());
    }

    #[test]
    fn builtin_dangling_style_links_can_be_cleared_or_repaired() {
        let mut document = EditableDocx::new();
        // The pinned built-in generator references companions it never creates.
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .get("Heading1")
                .unwrap()
                .linked
                .as_deref(),
            Some("Heading1Char")
        );
        assert!(document
            .document()
            .unwrap()
            .styles
            .get("Heading1Char")
            .is_none());
        document
            .style_relations_patch("Heading1", r#"{"linked":null}"#)
            .unwrap();
        assert!(document
            .document()
            .unwrap()
            .styles
            .get("Heading1")
            .unwrap()
            .linked
            .is_none());
        let character = document
            .add_style("Heading companion", "character", None)
            .unwrap();
        document
            .style_relations_patch("Heading2", &json!({"linked":character}).to_string())
            .unwrap();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(
            reopened.styles.get("Heading2").unwrap().linked.as_deref(),
            Some(character.as_str())
        );
        assert_eq!(
            reopened.styles.get(&character).unwrap().linked.as_deref(),
            Some("Heading2")
        );
    }

    #[test]
    fn document_defaults_roundtrip_inherit_and_clear_without_overwriting_direct_values() {
        let mut document = EditableDocx::new();
        let style = document
            .add_style("Inheriting body", "paragraph", Some("Normal"))
            .unwrap();
        let first = document.add_paragraph("Inherited text").unwrap();
        document.set_paragraph_style(first, &style).unwrap();
        let second = document.add_paragraph("").unwrap();
        document
            .add_run_patch(
                second,
                "Direct text",
                r##"{"font_size":16,"color":"#0000FF"}"##,
            )
            .unwrap();
        document
            .format_paragraph(second, r#"{"space_after":3}"#)
            .unwrap();
        document.format_defaults(r##"{"font_size":28,"bold":false,"color":"#FF0000","space_after":11,"line_spacing":1,"background":"#FFFFFF","tabs":[{"position":72}]}"##).unwrap();
        let before = document.default_formatting().unwrap();
        let direct: Value = serde_json::from_str(&before).unwrap();
        assert_eq!(direct["font_size"], 28.0);
        assert_eq!(direct["bold"], false);
        assert_eq!(direct["space_after"], 11.0);
        assert!(!direct.as_object().unwrap().contains_key("style"));
        document.format_defaults(&before).unwrap();
        assert_eq!(document.default_formatting().unwrap(), before);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert!(reopened.styles.default_para_present);
        assert_eq!(reopened.styles.default_chr.size, Some(28.0));
        assert_eq!(reopened.styles.default_chr.bold, Some(false));
        assert_eq!(reopened.styles.default_para.space_after, Some(11.0));
        assert_eq!(
            reopened.styles.default_para.tabs.as_ref().unwrap()[0].pos,
            72.0
        );
        let inherited = reopened
            .para(StoryRef::Body, &document.location(first).unwrap().1)
            .unwrap();
        let resolved = reopened
            .styles
            .resolve_char(inherited.props.style.as_deref(), inherited.props_of_char(0));
        assert_eq!(resolved.size, 28.0);
        assert_eq!(resolved.color, TextColor::Rgb(Rgb(255, 0, 0)));
        let explicit = reopened
            .para(StoryRef::Body, &document.location(second).unwrap().1)
            .unwrap();
        let resolved = reopened
            .styles
            .resolve_char(explicit.props.style.as_deref(), explicit.props_of_char(0));
        assert_eq!(resolved.size, 16.0);
        assert_eq!(resolved.color, TextColor::Rgb(Rgb(0, 0, 255)));
        assert_eq!(
            reopened.styles.resolve_para(&explicit.props).space_after,
            3.0
        );
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.matches("Inherited text").count(), 1);
        assert_eq!(text.matches("Direct text").count(), 1);
        for png in [
            crate::docx::render(&bytes, 0, 72.0, None).unwrap(),
            crate::pdf::render(&pdf, 0, 72.0, None).unwrap(),
        ] {
            let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let frame = reader.next_frame(&mut pixels).unwrap();
            let pixels = &pixels[..frame.buffer_size()];
            assert!(pixels
                .chunks_exact(4)
                .any(|p| p[0] > 200 && p[1] < 80 && p[2] < 80));
            assert!(pixels
                .chunks_exact(4)
                .any(|p| p[2] > 200 && p[0] < 80 && p[1] < 80));
        }
        let mut copy = document.copy().unwrap();
        copy.format_defaults(r#"{"font_size":20,"bold":true}"#)
            .unwrap();
        assert_eq!(document.default_formatting().unwrap(), before);
        let revision = document.revision().unwrap();
        assert!(document
            .format_defaults(r#"{"font_size":null,"bold":true}"#)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.default_formatting().unwrap(), before);
        document
            .format_defaults(r#"{"bold":null,"space_after":null,"background":null,"tabs":null}"#)
            .unwrap();
        let cleared: Value = serde_json::from_str(&document.default_formatting().unwrap()).unwrap();
        for key in ["bold", "space_after", "background", "tabs"] {
            assert!(cleared[key].is_null());
        }
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert!(reopened.styles.default_para_present);
        assert_eq!(reopened.styles.default_chr.size, Some(28.0));
        assert_eq!(reopened.styles.default_para.space_after, None);
        assert_eq!(document.paragraph_text(second).unwrap(), "Direct text");
    }

    #[test]
    fn upstream_omitted_default_font_size_resolves_inconsistently() {
        // Reproducer for the upstream blocker; the public default patch rejects
        // this mutation until native resolution and reader defaults agree.
        let mut document = Document::new();
        document.styles.default_chr.size = None;
        assert_eq!(
            document
                .styles
                .resolve_char(None, &CharProps::default())
                .size,
            11.0
        );
        let bytes = wordcraft_docx::write(&document).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("word/styles.xml").unwrap(), &mut xml)
            .unwrap();
        let defaults = xml
            .split("<w:docDefaults>")
            .nth(1)
            .unwrap()
            .split("</w:docDefaults>")
            .next()
            .unwrap();
        assert!(
            !defaults.contains("<w:sz "),
            "clearing removes the direct font size"
        );
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert!(reopened.styles.default_para_present);
        assert_eq!(reopened.styles.default_chr.size, Some(10.0));
        assert_eq!(
            reopened
                .styles
                .resolve_char(None, &CharProps::default())
                .size,
            10.0
        );
    }

    #[test]
    fn supported_default_clears_preserve_effective_native_readback() {
        let mut original = EditableDocx::new();
        original.format_defaults(r##"{"font":"Aptos","font_size":18,"bold":true,"italic":true,"color":"#123456","underline":"double","strike":"double","script":"superscript","highlight":"yellow","alignment":"center","indent_left":12,"indent_right":6,"indent_first":-2,"space_before":4,"space_after":6,"line_spacing":1.5,"keep_with_next":true,"keep_together":true,"widow_control":false,"page_break_before":true,"background":"#EEEEEE","tabs":[{"position":72}]}"##).unwrap();
        let keys = serde_json::from_str::<Value>(&original.default_formatting().unwrap()).unwrap();
        for key in keys
            .as_object()
            .unwrap()
            .keys()
            .filter(|key| key.as_str() != "font_size")
        {
            let mut document = original.copy().unwrap();
            document
                .format_defaults(&json!({key:Value::Null}).to_string())
                .unwrap();
            let current = document.document().unwrap();
            let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
            let expected_char =
                resolved_char_snapshot(&current.styles.resolve_char(None, &CharProps::default()));
            let actual_char =
                resolved_char_snapshot(&reopened.styles.resolve_char(None, &CharProps::default()));
            assert_eq!(
                actual_char, expected_char,
                "character defaults after clearing {key}"
            );
            let expected_para =
                resolved_para_snapshot(&current.styles.resolve_para(&ParaProps::default()))
                    .unwrap();
            let actual_para =
                resolved_para_snapshot(&reopened.styles.resolve_para(&ParaProps::default()))
                    .unwrap();
            assert_eq!(
                actual_para, expected_para,
                "paragraph defaults after clearing {key}"
            );
        }
    }

    #[test]
    fn document_defaults_validate_patches_tabs_and_table_geometry_atomically() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Kept").unwrap();
        let stops = (0..64)
            .map(|n| json!({"position":n * 2}))
            .collect::<Vec<_>>();
        document
            .format_paragraph(paragraph, &json!({"tabs":stops}).to_string())
            .unwrap();
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        for patch in [
            r#"{"font_size":24,"tabs":[{"position":200}]}"#,
            r#"{"font_size":24,"style":"Normal"}"#,
            r#"{"region":"first_row"}"#,
            r#"{"margins":12}"#,
            r#"{"row_band_size":2}"#,
            r#"{"bold":"false"}"#,
            r#"{"font_size":10000}"#,
            r#"{"background":"bad"}"#,
        ] {
            assert!(document.format_defaults(patch).is_err(), "{patch}");
            assert_eq!(document.document().unwrap(), &before);
            assert_eq!(document.revision().unwrap(), revision);
        }
        assert!(document
            .format_defaults(&" ".repeat(MAX_FORMAT_PATCH_BYTES + 1))
            .is_err());
        document
            .format_paragraph(paragraph, r#"{"tabs":null}"#)
            .unwrap();
        document.set_page_size(300.0, 400.0).unwrap();
        document.set_margins(24.0, 24.0, 24.0, 24.0).unwrap();
        let table = document
            .add_table(vec![vec!["".into(), "NNNNNNNN".into()]])
            .unwrap();
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["Nested".into()]])
            .unwrap();
        let nested_paragraph = document.cell_paragraph_ids(nested, 0, 0).unwrap()[0];
        document.set_paragraph_text(nested_paragraph, "").unwrap();
        document
            .add_run_patch(nested_paragraph, "Nested", r#"{"font_size":12}"#)
            .unwrap();
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        assert!(document.format_defaults(r#"{"font_size":144}"#).is_err());
        assert_eq!(document.document().unwrap(), &before);
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.cell_text(nested, 0, 0).unwrap(), "Nested");
    }

    #[test]
    fn paragraph_and_table_creation_roundtrip_and_copy_independence() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["Name".into(), "Value".into()]])
            .unwrap();
        let paragraph = document.add_paragraph("Unicode λ and 日本語").unwrap();
        assert_eq!(document.table_ids().unwrap(), vec![table]);
        assert_eq!(document.paragraph_ids().unwrap(), vec![paragraph]);
        let mut copy = document.copy().unwrap();
        copy.set_cell_text(table, 0, 1, "Changed").unwrap();
        copy.set_paragraph_text(paragraph, "Independent").unwrap();
        assert_eq!(document.table_rows(table).unwrap()[0][1], "Value");
        assert_eq!(
            document.paragraph_text(paragraph).unwrap(),
            "Unicode λ and 日本語"
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened.plain_text(StoryRef::Body),
            "Name\tValue\nUnicode λ and 日本語"
        );
        document.close();
        assert!(document.to_bytes().is_err());
        assert_eq!(copy.paragraph_text(paragraph).unwrap(), "Independent");
    }

    #[test]
    fn invalid_edits_are_atomic_and_explicit_empty_paragraphs_survive() {
        let mut document = EditableDocx::new();
        let empty = document.add_paragraph("").unwrap();
        let paragraph = document.add_paragraph("Keep").unwrap();
        assert_eq!(document.paragraph_ids().unwrap(), vec![empty, paragraph]);
        assert!(document.set_paragraph_text(paragraph, "bad\0text").is_err());
        assert!(document.add_paragraph("reserved\u{fffc}").is_err());
        assert!(document.add_table(vec![vec!["A".into()], vec![]]).is_err());
        assert!(document
            .set_cell_text(paragraph, 0, 0, "wrong object")
            .is_err());
        assert_eq!(document.paragraph_text(paragraph).unwrap(), "Keep");
        assert!(document.table_ids().unwrap().is_empty());
        let blank = EditableDocx::new().to_bytes().unwrap();
        assert_eq!(
            wordcraft_docx::read(&blank)
                .unwrap()
                .plain_text(StoryRef::Body),
            ""
        );
    }

    #[test]
    fn formatting_is_native_and_roundtrips() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("λ ").unwrap();
        document
            .set_paragraph_style(paragraph, "Heading 1")
            .unwrap();
        document
            .set_paragraph_alignment(paragraph, "center")
            .unwrap();
        assert_eq!(
            document
                .add_run(
                    paragraph,
                    "bold",
                    true,
                    false,
                    Some(18.0),
                    None,
                    Some("#123456")
                )
                .unwrap(),
            (2, 6, 4)
        );
        document.set_page_size(595.0, 842.0).unwrap();
        document.set_background(Some("#FAFAF7")).unwrap();
        let before = document.copy().unwrap();
        assert!(document.set_page_size(f64::NAN, 842.0).is_err());
        assert!(document
            .add_run(paragraph, "invalid", false, false, Some(10.25), None, None)
            .is_err());
        assert_eq!(
            before.paragraph_text(paragraph).unwrap(),
            document.paragraph_text(paragraph).unwrap()
        );
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let value = reopened
            .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
            .unwrap();
        assert_eq!(value.props.style.as_deref(), Some("Heading1"));
        assert_eq!(value.props.align, Some(Align::Center));
        assert_eq!(value.runs.last().unwrap().props.bold, Some(true));
        assert_eq!(reopened.settings.page_color, Some(Rgb(250, 250, 247)));
        assert_eq!(
            (reopened.last_section.page_w, reopened.last_section.page_h),
            (595.0, 842.0)
        );
    }

    #[test]
    fn revision_bound_ranges_use_unicode_offsets_and_preserve_surrounding_runs() {
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("λ ").unwrap();
        document
            .add_run(id, "first", true, false, None, None, None)
            .unwrap();
        let (_, _, revision) = document
            .add_run(id, " second", false, true, None, None, None)
            .unwrap();
        assert_eq!(
            document.text_range(id, 4, 11).unwrap(),
            ("rst sec".into(), revision)
        );
        assert!(document
            .format_range(id, 2, 7, revision, None, None, Some(f64::NAN), None, None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let (start, end, next) = document
            .replace_range(id, 4, 11, revision, "X日本")
            .unwrap();
        assert_eq!((start, end), (4, 7));
        assert_eq!(document.paragraph_text(id).unwrap(), "λ fiX日本ond");
        assert!(document.range_text(id, 4, 11, revision).is_err());
        assert_eq!(document.range_text(id, start, end, next).unwrap(), "X日本");
        let model = document.document().unwrap();
        let paragraph = model
            .para(StoryRef::Body, &document.location(id).unwrap().1)
            .unwrap();
        assert_eq!(
            paragraph
                .props_of_char(paragraph.text.find('X').unwrap())
                .bold,
            Some(true)
        );
        assert_eq!(
            paragraph
                .props_of_char(paragraph.text.find("ond").unwrap())
                .italic,
            Some(true)
        );
        let formatted = document
            .format_range(
                id,
                start,
                end,
                next,
                None,
                Some(true),
                None,
                None,
                Some("#123456"),
            )
            .unwrap();
        assert!(document.range_text(id, start, end, next).is_err());
        assert_eq!(
            document.range_text(id, start, end, formatted).unwrap(),
            "X日本"
        );
        let before = document.paragraph_text(id).unwrap();
        assert!(document
            .replace_range(id, 0, 100, formatted, "bad")
            .is_err());
        assert_eq!(document.paragraph_text(id).unwrap(), before);
        assert_eq!(document.revision().unwrap(), formatted);
        document.set_background(Some("#FFFFFF")).unwrap();
        assert!(document.range_text(id, start, end, formatted).is_err());
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(reopened.plain_text(StoryRef::Body), "λ fiX日本ond");
    }

    #[test]
    fn png_pictures_and_margins_roundtrip_and_reject_corrupt_input() {
        let png =
            crate::rendering::encode_png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255], 72.0).unwrap();
        let mut document = EditableDocx::new();
        document.set_margins(30.0, 40.0, 50.0, 60.0).unwrap();
        let image = document
            .add_image(&png, Some(100.0), None, "λ image")
            .unwrap();
        assert!(document.paragraph_ids().unwrap().is_empty());
        assert_eq!(document.image_size(image).unwrap(), (100.0, 50.0));
        document.set_image_size(image, 80.0, 40.0).unwrap();
        document.set_image_alt(image, "Updated").unwrap();
        let revision = document.revision().unwrap();
        let mut corrupt = png.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        assert!(document.add_image(&corrupt, None, None, "").is_err());
        assert!(document
            .add_image(&png[..png.len() - 8], None, None, "")
            .is_err());
        assert!(document.set_image_size(image, 0.0, 2.0).is_err());
        assert!(document.set_paragraph_text(image, "erase image").is_err());
        assert!(document.set_margins(500.0, 72.0, 500.0, 72.0).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let mut copy = document.copy().unwrap();
        copy.set_image_alt(image, "Independent").unwrap();
        assert_eq!(document.image_alt(image).unwrap(), "Updated");
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(reopened.last_section.margin_left, 60.0);
        assert_eq!(reopened.last_section.margin_bottom, 50.0);
        assert_eq!(reopened.media.values().next().unwrap().as_ref(), &png);
        let picture = &reopened
            .para(StoryRef::Body, &document.location(image).unwrap().1)
            .unwrap()
            .objects[0];
        assert!(
            matches!(picture, InlineObject::Image { w: 80.0, h: 40.0, alt, .. } if alt == "Updated")
        );
    }

    #[test]
    fn merged_cells_alias_anchor_and_split_without_duplicate_content() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![
                vec!["A".into(), "B".into(), "C".into()],
                vec!["D".into(), "E".into(), "F".into()],
            ])
            .unwrap();
        document.merge_cells(table, 0, 0, 1, 1).unwrap();
        assert_eq!(document.cell_text(table, 1, 1).unwrap(), "A\nB\nD\nE");
        assert_eq!(
            document.table_rows(table).unwrap(),
            vec![vec!["A\nB\nD\nE", "", "C"], vec!["", "", "F"]]
        );
        document
            .set_cell_background(table, 1, 1, Some("#123456"))
            .unwrap();
        document
            .set_cell_vertical_alignment(table, 1, 1, "center")
            .unwrap();
        assert_eq!(
            document.cell_background(table, 0, 0).unwrap().as_deref(),
            Some("#123456")
        );
        assert_eq!(
            document.cell_vertical_alignment(table, 0, 0).unwrap(),
            "center"
        );
        let revision = document.revision().unwrap();
        assert!(document.merge_cells(table, 0, 1, 0, 2).is_err());
        assert!(document.split_cell(table, 1, 1, 2).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.set_cell_text(table, 1, 1, "Joined").unwrap();
        assert_eq!(document.cell_text(table, 0, 0).unwrap(), "Joined");
        let horizontal = document
            .add_table(vec![vec!["Left".into(), "Right".into()]])
            .unwrap();
        document.merge_cells(horizontal, 0, 0, 0, 1).unwrap();
        document.split_cell(horizontal, 0, 0, 2).unwrap();
        assert_eq!(
            document.table_rows(horizontal).unwrap(),
            vec![vec!["Left\nRight", ""]]
        );
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let merged = reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap();
        assert_eq!(merged.rows[0].cells[0].span(), 2);
        assert_eq!(merged.rows[0].cells[0].props.shading, Some(Rgb(18, 52, 86)));
        assert_eq!(merged.rows[0].cells[0].props.valign, Some(VAlign::Center));
    }

    #[test]
    fn header_footer_paragraphs_share_safe_formatting_and_copy_ownership() {
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Body").unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let revision = document.revision().unwrap();
        assert_eq!(
            document.ensure_story("header", "default", 0).unwrap(),
            header
        );
        assert_eq!(document.revision().unwrap(), revision);
        let paragraph = document.story_add_paragraph(header, "Header λ").unwrap();
        assert!(paragraph >= MAX_BLOCKS);
        assert_eq!(
            document.location(paragraph).unwrap().0,
            StoryRef::Part(header)
        );
        document
            .set_paragraph_style(paragraph, "Heading 1")
            .unwrap();
        document
            .set_paragraph_alignment(paragraph, "right")
            .unwrap();
        let (start, end, revision) = document
            .add_run(paragraph, " bold", true, false, None, None, None)
            .unwrap();
        document
            .replace_range(paragraph, start, end, revision, " styled")
            .unwrap();
        assert_eq!(
            document.paragraph_text(paragraph).unwrap(),
            "Header λ styled"
        );
        assert_eq!(document.paragraph_text(body).unwrap(), "Body");
        assert_eq!(document.paragraph_ids().unwrap(), vec![body]);
        assert_eq!(
            document.story_paragraph_ids(header).unwrap(),
            vec![paragraph]
        );
        let empty = document.story_add_paragraph(header, "").unwrap();
        assert_eq!(
            document.story_paragraph_ids(header).unwrap(),
            vec![paragraph, empty]
        );
        let mut copy = document.copy().unwrap();
        copy.set_paragraph_text(paragraph, "Independent header")
            .unwrap();
        assert_eq!(
            document.paragraph_text(paragraph).unwrap(),
            "Header λ styled"
        );
        let revision = document.revision().unwrap();
        assert!(document.story_add_paragraph(u32::MAX, "invalid").is_err());
        assert!(document.ensure_story("header", "invalid", 0).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let id = reopened.last_section.headers.default.unwrap();
        assert_eq!(reopened.plain_text(StoryRef::Part(id)), "Header λ styled\n");
        document.close();
        assert!(document.paragraph_text(paragraph).is_err());
        assert_eq!(
            copy.paragraph_text(paragraph).unwrap(),
            "Independent header"
        );
    }

    #[test]
    fn header_footer_variants_render_and_export_on_matching_pages() {
        let mut document = EditableDocx::new();
        for page in 0..3 {
            let paragraph = document
                .add_paragraph(&format!("Body page {page}"))
                .unwrap();
            if page > 0 {
                // Explicit breaks make this a deterministic layout fixture; no viewer needed.
                let mut candidate = document.document().unwrap().clone();
                document
                    .paragraph_mut(&mut candidate, paragraph)
                    .unwrap()
                    .props
                    .page_break_before = Some(true);
                document.commit(candidate).unwrap();
            }
        }
        for variant in ["default", "first", "even"] {
            for kind in ["header", "footer"] {
                let story = document.ensure_story(kind, variant, 0).unwrap();
                document
                    .story_add_paragraph(story, &format!("{variant} {kind}"))
                    .unwrap();
            }
        }
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert!(reopened.last_section.title_page);
        assert!(reopened.settings.even_odd_headers);
        let png = crate::docx::render(&bytes, 0, 72.0, None).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 3);
        for (page, variant) in extracted.pages.iter().zip(["first", "even", "default"]) {
            let text = page
                .blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains(&format!("{variant} header")), "{text}");
            assert!(text.contains(&format!("{variant} footer")), "{text}");
        }
    }

    #[test]
    fn rich_cells_reuse_objects_and_invalidate_moved_or_replaced_children() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["Outer".into(), "Next".into()]])
            .unwrap();
        let original = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        document
            .add_run(original, " bold", true, false, None, None, None)
            .unwrap();
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["Inner".into()]])
            .unwrap();
        let nested_paragraph = document.cell_paragraph_ids(nested, 0, 0).unwrap()[0];
        document
            .set_paragraph_alignment(nested_paragraph, "center")
            .unwrap();
        let png = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
        let image = document
            .cell_add_image(table, 0, 0, &png, Some(20.0), None, "Nested image")
            .unwrap();
        assert_eq!(document.cell_table_ids(table, 0, 0).unwrap(), vec![nested]);
        assert_eq!(document.cell_image_ids(table, 0, 0).unwrap(), vec![image]);
        assert_eq!(
            document.cell_paragraph_ids(table, 0, 0).unwrap(),
            vec![original]
        );
        assert_eq!(
            document.cell_text(table, 0, 0).unwrap(),
            "Outer bold\nInner"
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let outer = reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap();
        assert!(outer.rows[0].cells[0]
            .blocks
            .iter()
            .any(|block| block.as_table().is_some()));
        assert_eq!(reopened.media.len(), 1);
        document.merge_cells(table, 0, 0, 0, 1).unwrap();
        assert!(document.paragraph_text(original).is_err());
        assert!(document.paragraph_text(nested_paragraph).is_err());
        assert!(document.table_rows(nested).is_err());
        assert!(document.image_size(image).is_err());
        let rebound = document.cell_table_ids(table, 0, 1).unwrap()[0];
        assert!(rebound > nested);
        assert_eq!(document.cell_text(rebound, 0, 0).unwrap(), "Inner");
        let rebound_image = document.image_ids().unwrap()[0];
        assert!(rebound_image > image);
        assert_eq!(document.image_alt(rebound_image).unwrap(), "Nested image");
        document.set_cell_text(table, 0, 1, "Replaced").unwrap();
        assert!(document.table_rows(rebound).is_err());
        assert!(document.image_ids().unwrap().is_empty());
        assert_eq!(document.cell_text(table, 0, 0).unwrap(), "Replaced");
        let later = document.add_image(&png, None, None, "Later").unwrap();
        assert_eq!(document.image_alt(later).unwrap(), "Later");
        assert_eq!(document.document().unwrap().media.len(), 1);
    }

    #[test]
    fn table_layout_controls_roundtrip_and_reject_partial_mutations() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![
                vec!["Header".into(), "Value".into()],
                vec!["Body".into(), "Content".into()],
            ])
            .unwrap();
        assert_eq!(
            document.table_borders(table).unwrap(),
            Some(("#000000".into(), 0.5))
        );
        document
            .set_column_widths(table, vec![160.0, 240.0])
            .unwrap();
        document
            .set_row_height(table, 0, Some(24.0), "at_least")
            .unwrap();
        document
            .set_row_height(table, 1, Some(36.0), "exact")
            .unwrap();
        document.set_row_repeat_header(table, 0, true).unwrap();
        document.set_row_allow_split(table, 1, false).unwrap();
        document.set_table_borders(table, "#123456", 1.25).unwrap();
        let revision = document.revision().unwrap();
        for widths in [
            vec![100.0],
            vec![f64::NAN, 100.0],
            vec![5.0, 100.0],
            vec![1000.0, 1000.0],
        ] {
            assert!(document.set_column_widths(table, widths).is_err());
        }
        assert!(document
            .set_row_height(table, 0, Some(24.0), "auto")
            .is_err());
        assert!(document.set_row_height(table, 0, None, "exact").is_err());
        assert!(document
            .set_row_height(table, 0, Some(f64::INFINITY), "exact")
            .is_err());
        assert!(document.set_row_allow_split(table, 2, false).is_err());
        assert!(document.set_table_borders(table, "#123456", 0.6).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.column_widths(table).unwrap(), vec![160.0, 240.0]);
        assert_eq!(
            document.row_height(table, 0).unwrap(),
            (Some(24.0), "at_least")
        );
        assert!(document.row_repeat_header(table, 0).unwrap());
        assert!(!document.row_allow_split(table, 1).unwrap());
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let native = reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap();
        assert_eq!(native.grid, vec![160.0, 240.0]);
        assert_eq!(native.rows[0].cells[1].props.width, Some(240.0));
        assert_eq!(native.rows[1].props.height_rule, HeightRule::Exact);
        assert_eq!(native.rows[1].props.height, Some(36.0));
        assert!(native.rows[0].props.header);
        assert!(native.rows[1].props.cant_split);
        assert_eq!(
            native.props.borders.unwrap().top.unwrap().color,
            Some(Rgb(0x12, 0x34, 0x56))
        );
        document.set_row_repeat_header(table, 1, true).unwrap();
        assert!(document.set_row_repeat_header(table, 0, false).is_err());
        document.set_row_repeat_header(table, 1, false).unwrap();
        document.set_row_repeat_header(table, 0, false).unwrap();
        assert!(document.set_row_repeat_header(table, 1, true).is_err());
        document.clear_table_borders(table).unwrap();
        assert_eq!(document.table_borders(table).unwrap(), None);
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert!(!reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap()
            .props
            .borders
            .unwrap()
            .any_visible());
        document.set_row_height(table, 0, None, "auto").unwrap();
        assert_eq!(document.row_height(table, 0).unwrap(), (None, "auto"));
    }

    #[test]
    fn nested_column_widths_respect_containers_and_preserve_merged_spans() {
        let mut document = EditableDocx::new();
        let outer = document
            .add_table(vec![vec!["A".into(), "B".into()]])
            .unwrap();
        document
            .set_column_widths(outer, vec![200.0, 200.0])
            .unwrap();
        let inner = document
            .cell_add_table(outer, 0, 0, vec![vec!["Nested".into()]])
            .unwrap();
        let revision = document.revision().unwrap();
        assert!(document.set_column_widths(inner, vec![200.0]).is_err());
        assert!(document
            .set_column_widths(outer, vec![100.0, 300.0])
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.set_column_widths(inner, vec![80.0]).unwrap();
        document
            .set_column_widths(outer, vec![100.0, 300.0])
            .unwrap();
        document.merge_cells(outer, 0, 0, 0, 1).unwrap();
        document
            .set_column_widths(outer, vec![150.0, 250.0])
            .unwrap();
        let anchor = &document.table(outer).unwrap().rows[0].cells[0];
        assert_eq!(anchor.span(), 2);
        assert_eq!(anchor.props.width, Some(400.0));
        assert_eq!(document.cell_text(outer, 0, 1).unwrap(), "A\nNested\nB");
    }

    #[test]
    fn paragraph_spacing_and_page_breaks_roundtrip_and_paginate() {
        let mut document = EditableDocx::new();
        let first = document.add_paragraph("First page").unwrap();
        document
            .set_paragraph_spacing(first, 12.0, 6.0, 1.5)
            .unwrap();
        assert_eq!(document.paragraph_spacing(first).unwrap(), (12.0, 6.0, 1.5));
        let revision = document.revision().unwrap();
        assert!(document
            .set_paragraph_spacing(first, -1.0, 0.0, 1.0)
            .is_err());
        assert!(document
            .set_paragraph_spacing(first, 0.0, 0.0, f64::NAN)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let break_paragraph = document.add_page_break().unwrap();
        assert!(!document.page_break_before(break_paragraph).unwrap());
        assert_eq!(
            document.paragraph_text(break_paragraph).unwrap(),
            "\u{000c}"
        );
        document.add_paragraph("Second page").unwrap();
        let third = document.add_paragraph("Third page").unwrap();
        document.set_page_break_before(third, true).unwrap();
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let props = &reopened
            .para(StoryRef::Body, &document.location(first).unwrap().1)
            .unwrap()
            .props;
        assert_eq!(props.space_before, Some(12.0));
        assert_eq!(props.line_spacing, Some(LineSpacing::Multiple(1.5)));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 3);
        for (page, expected) in
            extracted
                .pages
                .iter()
                .zip(["First page", "Second page", "Third page"])
        {
            assert!(page
                .blocks
                .iter()
                .any(|block| block.text.contains(expected)));
        }
    }

    #[test]
    fn list_start_creates_an_independent_native_sequence_with_visible_labels() {
        let mut document = EditableDocx::new();
        let mut ids = Vec::new();
        for text in ["First", "Selected", "Already assigned"] {
            let id = document.add_paragraph(text).unwrap();
            document
                .set_paragraph_list(id, Some("numbered"), 0, false, None)
                .unwrap();
            ids.push(id);
        }
        let original_num = document
            .paragraph(ids[2])
            .unwrap()
            .props
            .numbering
            .unwrap()
            .num;
        let original = document.copy().unwrap();
        document
            .set_paragraph_list(ids[1], Some("numbered"), 0, false, Some(ListStart(7)))
            .unwrap();
        assert_eq!(
            document
                .paragraph(ids[2])
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num,
            original_num
        );
        assert_eq!(
            original
                .paragraph(ids[1])
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num,
            original_num
        );
        let selected_num = document
            .paragraph(ids[1])
            .unwrap()
            .props
            .numbering
            .unwrap()
            .num;
        assert_ne!(selected_num, original_num);
        assert_ne!(
            document
                .document()
                .unwrap()
                .numbering
                .num(selected_num)
                .unwrap()
                .abstract_id,
            document
                .document()
                .unwrap()
                .numbering
                .num(original_num)
                .unwrap()
                .abstract_id
        );
        let later = document.add_paragraph("Future assignment").unwrap();
        document
            .set_paragraph_list(later, Some("numbered"), 0, false, None)
            .unwrap();
        assert_eq!(
            document
                .paragraph(later)
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num,
            original_num
        );
        document
            .set_paragraph_list(later, Some("numbered"), 0, true, Some(ListStart(20)))
            .unwrap();
        let continuation = document.add_paragraph("Continued start").unwrap();
        document
            .set_paragraph_list(continuation, Some("numbered"), 0, false, None)
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened
                .numbering
                .num(selected_num)
                .unwrap()
                .start_overrides,
            [(0, 7)]
        );
        let payload = crate::docx::extract(&bytes, None).unwrap();
        let labels: Vec<_> = payload
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .filter_map(|block| {
                block
                    .metadata
                    .get("numbering")
                    .map(|value| value["label"].as_str().unwrap())
            })
            .collect();
        assert_eq!(labels, ["1.", "7.", "2.", "20.", "21."]);
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let payload = crate::pdf::extract(&pdf, None).unwrap();
        let text = payload
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .map(|block| {
                block.table.as_ref().map_or_else(
                    || block.text.clone(),
                    |table| {
                        table
                            .rows
                            .iter()
                            .map(|row| row.join(" "))
                            .collect::<Vec<_>>()
                            .join("\n")
                    },
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        for expected in ["7.", "20.", "21.", "Selected", "Continued start"] {
            assert!(text.contains(expected), "{text:?}");
        }
        assert_ne!(
            crate::docx::render(&bytes, 0, 72.0, None).unwrap(),
            crate::docx::render(&original.to_bytes().unwrap(), 0, 72.0, None).unwrap()
        );
    }

    #[test]
    fn header_footer_start_rejection_preserves_native_counter_advance_reproducer() {
        for (kind, rendered) in [("header", "12."), ("footer", "13.")] {
            let mut document = EditableDocx::new();
            document.add_paragraph("Body").unwrap();
            let story = document.ensure_story(kind, "default", 0).unwrap();
            let id = document
                .story_add_paragraph(story, "Auxiliary list")
                .unwrap();
            document
                .set_paragraph_list(id, Some("numbered"), 0, false, None)
                .unwrap();
            let bytes = document.to_bytes().unwrap();
            let revision = document.revision().unwrap();
            assert!(document
                .set_paragraph_list(id, Some("numbered"), 0, false, Some(ListStart(11)))
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), bytes);
            assert_eq!(document.revision().unwrap(), revision);

            // The public model/writer preserve11, but native header/footer measurement advances
            // its counter before the rendered placement. Do not compensate in the adapter.
            let mut native = wordcraft_docx::read(&bytes).unwrap();
            let num = native
                .para(StoryRef::Part(story), &Path::top(0))
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num;
            native
                .numbering
                .nums
                .iter_mut()
                .find(|number| number.id == num)
                .unwrap()
                .start_overrides
                .push((0, 11));
            let raw = wordcraft_docx::write(&native).unwrap();
            assert_eq!(
                wordcraft_docx::read(&raw)
                    .unwrap()
                    .numbering
                    .num(num)
                    .unwrap()
                    .start_overrides,
                [(0, 11)]
            );
            let pdf = crate::docx::to_pdf(&raw, None).unwrap();
            let text = crate::pdf::extract(&pdf, None).unwrap().pages[0]
                .blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            // PdfCraft infers the gap after a list marker as whitespace on some platforms.
            // Assert the exact counter and following content independently of that gap.
            let compact = text.split_whitespace().collect::<String>();
            assert!(
                compact.contains(&format!("{rendered}Auxiliarylist")),
                "{text:?}"
            );
        }
    }

    #[test]
    fn list_start_validation_and_definition_budgets_are_atomic() {
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("Stable").unwrap();
        document
            .set_paragraph_list(id, Some("numbered"), 0, false, Some(ListStart(32767)))
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for (kind, level, start) in [
            (None, 0, 1),
            (Some("bullet"), 0, 1),
            (Some("numbered"), 1, 1),
            (Some("numbered"), 0, 0),
            (Some("numbered"), 0, 32768),
        ] {
            assert!(document
                .set_paragraph_list(id, kind, level, false, Some(ListStart(start)))
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), bytes);
            assert_eq!(document.revision().unwrap(), revision);
        }
        for abstracts in [false, true] {
            let mut bounded = document.copy().unwrap();
            let mut candidate = bounded.document().unwrap().clone();
            if abstracts {
                candidate
                    .numbering
                    .abstracts
                    .resize_with(MAX_LISTS, Default::default);
            } else {
                candidate
                    .numbering
                    .nums
                    .resize_with(MAX_LISTS, Default::default);
            }
            bounded.commit(candidate).unwrap();
            let revision = bounded.revision().unwrap();
            assert!(bounded
                .set_paragraph_list(id, Some("numbered"), 0, false, Some(ListStart(7)))
                .is_err());
            assert_eq!(bounded.revision().unwrap(), revision);
            assert_eq!(
                bounded
                    .document()
                    .unwrap()
                    .numbering
                    .num(bounded.paragraph(id).unwrap().props.numbering.unwrap().num)
                    .unwrap()
                    .start_overrides,
                [(0, 32767)]
            );
        }
        document.close();
        assert!(document
            .set_paragraph_list(id, Some("numbered"), 0, false, Some(ListStart(7)))
            .is_err());
    }

    #[test]
    fn native_lists_continue_restart_and_clear_without_rebuilding_text() {
        let mut document = EditableDocx::new();
        let mut ids = Vec::new();
        for (index, label) in ["First", "Second", "Restart", "Continued"]
            .iter()
            .enumerate()
        {
            let id = document.add_paragraph(label).unwrap();
            document
                .set_paragraph_list(id, Some("numbered"), 0, index == 2, None)
                .unwrap();
            ids.push(id);
        }
        let first_num = document
            .paragraph(ids[0])
            .unwrap()
            .props
            .numbering
            .unwrap()
            .num;
        let restarted = document
            .paragraph(ids[2])
            .unwrap()
            .props
            .numbering
            .unwrap()
            .num;
        assert_ne!(first_num, restarted);
        assert_eq!(
            document
                .paragraph(ids[1])
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num,
            first_num
        );
        assert_eq!(
            document
                .paragraph(ids[3])
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num,
            restarted
        );
        document
            .set_paragraph_list(ids[0], Some("numbered"), 0, false, None)
            .unwrap();
        assert_eq!(
            document
                .paragraph(ids[0])
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num,
            first_num
        );
        let sub = document.add_paragraph("Nested bullet").unwrap();
        document
            .add_run(sub, " bold", true, false, None, None, None)
            .unwrap();
        document
            .set_paragraph_list(sub, Some("bullet"), 2, false, None)
            .unwrap();
        assert_eq!(document.paragraph_list(sub).unwrap(), Some(("bullet", 2)));
        let revision = document.revision().unwrap();
        assert!(document
            .set_paragraph_list(sub, Some("bullet"), 9, false, None)
            .is_err());
        assert!(document
            .set_paragraph_list(sub, Some("unsupported"), 0, false, None)
            .is_err());
        assert!(document
            .set_paragraph_list(sub, None, 0, true, None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let mut counters = wordcraft_doc::numbering::Counters::default();
        let labels = ids
            .iter()
            .map(|id| {
                let reference = reopened
                    .para(StoryRef::Body, &document.location(*id).unwrap().1)
                    .unwrap()
                    .props
                    .numbering
                    .unwrap();
                counters
                    .next_label(&reopened.numbering, reference.num, reference.level)
                    .unwrap()
                    .0
            })
            .collect::<Vec<_>>();
        assert_eq!(labels, ["1.", "2.", "1.", "2."]);
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        assert_eq!(crate::pdf::extract(&pdf, None).unwrap().pages.len(), 1);
        document
            .set_paragraph_list(sub, None, 0, false, None)
            .unwrap();
        assert_eq!(document.paragraph_list(sub).unwrap(), None);
        assert_eq!(document.paragraph_text(sub).unwrap(), "Nested bullet bold");
        assert_eq!(
            document
                .paragraph(sub)
                .unwrap()
                .runs
                .last()
                .unwrap()
                .props
                .bold,
            Some(true)
        );
        let story = document.ensure_story("header", "default", 0).unwrap();
        let header = document.story_add_paragraph(story, "Header list").unwrap();
        document
            .set_paragraph_list(header, Some("numbered"), 0, false, None)
            .unwrap();
        assert_ne!(
            document
                .paragraph(header)
                .unwrap()
                .props
                .numbering
                .unwrap()
                .num,
            first_num
        );
    }

    #[test]
    fn list_definition_limit_is_atomic() {
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("Limited").unwrap();
        let mut candidate = document.document().unwrap().clone();
        // Fill the boundary directly: exercising 10,000 public document clones would test
        // allocation cost rather than the rejection path.
        candidate.numbering.nums = (1..=MAX_LISTS as u32)
            .map(|id| wordcraft_doc::numbering::Num {
                id,
                ..Default::default()
            })
            .collect();
        document.commit(candidate).unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .set_paragraph_list(id, Some("numbered"), 0, true, None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.paragraph_list(id).unwrap(), None);
    }

    #[test]
    fn unicode_links_cross_runs_roundtrip_clear_and_reject_stale_or_unsafe_edits() {
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("Before λ").unwrap();
        document
            .add_run(id, "日本", true, false, None, None, None)
            .unwrap();
        document
            .add_run(id, " after", false, true, None, None, None)
            .unwrap();
        let revision = document.revision().unwrap();
        let url = "https://example.com/path?a=1&b=2";
        let current = document
            .set_range_link(id, 7, 10, revision, Some(url))
            .unwrap();
        assert_eq!(
            document.range_link(id, 7, 10, current).unwrap().as_deref(),
            Some(url)
        );
        assert!(document.range_link(id, 6, 10, current).is_err());
        assert!(document.range_link(id, 7, 10, revision).is_err());
        assert!(document.set_range_link(id, 7, 10, revision, None).is_err());
        for bad in [
            "javascript:alert(1)",
            "file:///tmp/private",
            "https://example.com/a\nb",
            "mailto:",
            "//example.com",
            "https://example.com/a b",
        ] {
            assert!(document
                .set_range_link(id, 7, 10, current, Some(bad))
                .is_err());
        }
        assert!(document
            .set_range_link(id, 7, 7, current, Some(url))
            .is_err());
        assert_eq!(document.revision().unwrap(), current);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let paragraph = reopened
            .para(StoryRef::Body, &document.location(id).unwrap().1)
            .unwrap();
        assert_eq!(paragraph.props_of_char("Before λ".len()).bold, Some(true));
        assert_eq!(
            paragraph.props_of_char("Before λ日本".len()).italic,
            Some(true)
        );
        assert_eq!(
            paragraph.props_of_char("Before ".len()).link.as_deref(),
            Some(url)
        );
        assert_eq!(paragraph.mark.link, None);
        let extracted = crate::docx::extract(&bytes, None).unwrap();
        let labels = extracted
            .pages
            .iter()
            .flat_map(|page| &page.links)
            .filter(|link| link.uri == url)
            .map(|link| link.text.as_str())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(labels, "λ日本");
        let current = document.set_range_link(id, 7, 10, current, None).unwrap();
        assert_eq!(document.range_link(id, 7, 10, current).unwrap(), None);
        assert!(
            document
                .paragraph(id)
                .unwrap()
                .runs
                .iter()
                .all(|run| run.props.link.is_none()
                    && run.props.style.as_deref() != Some("Hyperlink"))
        );
        assert_eq!(document.paragraph_text(id).unwrap(), "Before λ日本 after");
        assert_eq!(
            document
                .paragraph(id)
                .unwrap()
                .props_of_char("Before λ".len())
                .bold,
            Some(true)
        );
        assert_eq!(
            document
                .paragraph(id)
                .unwrap()
                .props_of_char("Before λ日本".len())
                .italic,
            Some(true)
        );
    }

    #[test]
    fn repeated_table_headers_appear_on_every_exported_page() {
        let mut document = EditableDocx::new();
        document.set_margins(40.0, 40.0, 40.0, 40.0).unwrap();
        document.set_page_size(400.0, 400.0).unwrap();
        let mut rows = vec![vec!["Repeat heading".into()]];
        rows.extend((0..24).map(|index| vec![format!("UniqueRow{index:02}")]));
        let table = document.add_table(rows).unwrap();
        document.set_row_repeat_header(table, 0, true).unwrap();
        for row in 0..25 {
            document
                .set_row_height(table, row, Some(30.0), "exact")
                .unwrap();
            document.set_row_allow_split(table, row, false).unwrap();
        }
        let revision = document.revision().unwrap();
        assert!(document
            .set_row_height(table, 0, Some(0.01), "exact")
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let pdf = crate::docx::to_pdf(&document.to_bytes().unwrap(), None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert!(extracted.pages.len() >= 3);
        let pages = extracted
            .pages
            .iter()
            .map(|page| {
                page.blocks
                    .iter()
                    .map(|block| block.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>();
        for page in &pages {
            assert!(
                page.contains("Repeat heading"),
                "missing repeated header: {page}"
            );
        }
        let all = pages.join("\n");
        for index in 0..24 {
            assert_eq!(all.matches(&format!("UniqueRow{index:02}")).count(), 1);
        }
    }

    #[test]
    fn formatting_cannot_bypass_hyperlink_resource_limit() {
        let mut document = EditableDocx::new();
        let count = MAX_LINK_BYTES / 8192;
        let id = document.add_paragraph(&"abc".repeat(count)).unwrap();
        let mut candidate = document.document().unwrap().clone();
        let mut url = "https://example.com/".to_string();
        url.push_str(&"a".repeat(8192 - url.len()));
        candidate
            .para_mut(StoryRef::Body, &document.location(id).unwrap().1)
            .unwrap()
            .runs = (0..count)
            .map(|index| wordcraft_doc::para::Run {
                len: 3,
                props: CharProps {
                    bold: Some(index % 2 == 0),
                    link: Some(url.clone()),
                    ..Default::default()
                },
            })
            .collect();
        document.commit(candidate).unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .format_range(id, 1, 2, revision, Some(false), None, None, None, None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.paragraph(id).unwrap().runs.len(), count);
        assert_eq!(
            document.paragraph(id).unwrap().runs[0].props.bold,
            Some(true)
        );
        document
            .set_range_link(id, 0, count * 3, revision, None)
            .unwrap();
        assert_eq!(hyperlink_bytes(document.document().unwrap()), 0);
    }

    #[test]
    fn explicit_page_breaks_preserve_initial_and_consecutive_blank_pages() {
        for (before, breaks, expected_pages) in
            [(false, 1, 2), (false, 2, 3), (true, 1, 2), (true, 2, 3)]
        {
            let mut document = EditableDocx::new();
            if before {
                document.add_paragraph("Before").unwrap();
            }
            let mut break_ids = Vec::new();
            for _ in 0..breaks {
                break_ids.push(document.add_page_break().unwrap());
            }
            document.add_paragraph("After").unwrap();
            let bytes = document.to_bytes().unwrap();
            let reopened = wordcraft_docx::read(&bytes).unwrap();
            for id in break_ids {
                assert_eq!(
                    reopened
                        .para(StoryRef::Body, &document.location(id).unwrap().1)
                        .unwrap()
                        .text,
                    wordcraft_doc::para::PAGE_BREAK.to_string()
                );
            }
            let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
            let extracted = crate::pdf::extract(&pdf, None).unwrap();
            assert_eq!(
                extracted.pages.len(),
                expected_pages,
                "before={before}, breaks={breaks}"
            );
            for (index, page) in extracted.pages.iter().enumerate() {
                let text = page
                    .blocks
                    .iter()
                    .map(|block| block.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                assert_eq!(text.contains("After"), index == expected_pages - 1);
                assert_eq!(text.contains("Before"), before && index == 0);
                if (!before || index > 0) && index < expected_pages - 1 {
                    assert!(
                        text.trim().is_empty(),
                        "expected blank page {index}: {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn named_styles_inherit_reset_and_keep_owned_copy_state() {
        let mut document = EditableDocx::new();
        let base = document
            .add_style("Report & \"Base\"", "paragraph", Some("Normal"))
            .unwrap();
        document.format_style(&base, r##"{"bold":true,"font_size":18,"color":"#123456","space_after":5,"keep_with_next":true,"background":"#EEFFEE"}"##, None).unwrap();
        let derived = document
            .add_style("Report Body", "paragraph", Some(&base))
            .unwrap();
        let info: Value =
            serde_json::from_str(&document.style_info("Report Body").unwrap()).unwrap();
        assert_eq!(info["based_on"], base);
        assert_eq!(info["kind"], "paragraph");
        assert!(document
            .style_ids(Some("paragraph"))
            .unwrap()
            .contains(&derived));
        let direct: Value =
            serde_json::from_str(&document.style_formatting(&derived, false, None).unwrap())
                .unwrap();
        let resolved: Value =
            serde_json::from_str(&document.style_formatting(&derived, true, None).unwrap())
                .unwrap();
        assert!(direct["bold"].is_null());
        assert!(direct["font_size"].is_null());
        assert!(direct.get("size").is_none());
        assert_eq!(resolved["bold"], true);
        assert_eq!(resolved["font_size"], 18.0);
        assert_eq!(resolved["background"], "#EEFFEE");
        assert_eq!(resolved["space_after"], 5.0);
        let paragraph = document.add_paragraph("Styled ").unwrap();
        document.set_paragraph_style(paragraph, &derived).unwrap();
        let (start, end, revision) = document.add_run_patch(paragraph, "λ日本", "{}").unwrap();
        let props = document
            .paragraph(paragraph)
            .unwrap()
            .props_of_char("Styled ".len());
        assert_eq!(props.bold, None, "omitted bold must inherit");
        assert!(
            document
                .document()
                .unwrap()
                .styles
                .resolve_char(Some(&derived), props)
                .bold
        );
        let revision = document
            .format_range_patch(
                paragraph,
                start,
                end,
                revision,
                r#"{"bold":false,"italic":true}"#,
            )
            .unwrap();
        let props = document
            .paragraph(paragraph)
            .unwrap()
            .props_of_char("Styled ".len());
        assert_eq!(props.bold, Some(false));
        assert_eq!(props.italic, Some(true));
        document
            .format_range_patch(paragraph, start, end, revision, r#"{"bold":null}"#)
            .unwrap();
        let props = document
            .paragraph(paragraph)
            .unwrap()
            .props_of_char("Styled ".len());
        assert_eq!(props.bold, None);
        assert_eq!(
            props.italic,
            Some(true),
            "omitted key must remain unchanged"
        );
        assert!(
            document
                .document()
                .unwrap()
                .styles
                .resolve_char(Some(&derived), props)
                .bold
        );
        document
            .format_paragraph(paragraph, r#"{"space_after":0,"keep_with_next":false}"#)
            .unwrap();
        assert_eq!(document.paragraph_spacing(paragraph).unwrap().1, 0.0);
        assert!(
            !document
                .document()
                .unwrap()
                .styles
                .resolve_para(&document.paragraph(paragraph).unwrap().props)
                .keep_next
        );
        document
            .format_paragraph(paragraph, r#"{"space_after":null,"keep_with_next":null}"#)
            .unwrap();
        assert_eq!(document.paragraph_spacing(paragraph).unwrap().1, 5.0);
        assert!(
            document
                .document()
                .unwrap()
                .styles
                .resolve_para(&document.paragraph(paragraph).unwrap().props)
                .keep_next
        );
        let mut copy = document.copy().unwrap();
        copy.format_style(&base, r#"{"font_size":22}"#, None)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(
                &document.style_formatting(&derived, true, None).unwrap()
            )
            .unwrap()["font_size"],
            18.0
        );
        assert_eq!(
            serde_json::from_str::<Value>(&copy.style_formatting(&derived, true, None).unwrap())
                .unwrap()["font_size"],
            22.0
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened.styles.get(&base).unwrap().name,
            "Report & \"Base\""
        );
        assert_eq!(
            reopened.styles.get(&derived).unwrap().based_on.as_deref(),
            Some(base.as_str())
        );
        assert_eq!(
            reopened
                .styles
                .resolve_char(Some(&derived), &CharProps::default())
                .size,
            18.0
        );
        assert!(crate::docx::render(&bytes, 0, 72.0, None)
            .unwrap()
            .starts_with(b"\x89PNG"));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        assert_eq!(crate::pdf::extract(&pdf, None).unwrap().pages.len(), 1);
        document.close();
        assert!(document.style_ids(None).is_err());
        assert!(copy.style_info(&base).is_ok());
    }

    #[test]
    fn character_style_formatting_roundtrips_and_preserves_links() {
        let mut document = EditableDocx::new();
        let character = document
            .add_style("Annotation", "character", Some("Emphasis"))
            .unwrap();
        document.format_style(&character, r##"{"underline":"double_wave","strike":true,"script":"superscript","highlight":"yellow","background":"#102030","color":"auto","font_size":15.5}"##, None).unwrap();
        let direct: Value =
            serde_json::from_str(&document.style_formatting(&character, false, None).unwrap())
                .unwrap();
        let resolved: Value =
            serde_json::from_str(&document.style_formatting(&character, true, None).unwrap())
                .unwrap();
        assert_eq!(direct["underline"], "double_wave");
        assert_eq!(direct["background"], "#102030");
        assert!(direct["italic"].is_null());
        assert_eq!(resolved["italic"], true);
        let paragraph = document.add_paragraph("Before ").unwrap();
        let patch = json!({"style":character}).to_string();
        let (start, end, revision) = document.add_run_patch(paragraph, "word", &patch).unwrap();
        let revision = document
            .set_range_link(paragraph, start, end, revision, Some("https://example.com"))
            .unwrap();
        let revision = document
            .format_range_patch(
                paragraph,
                start,
                end,
                revision,
                r#"{"strike":false,"script":"baseline","underline":false,"highlight":"none"}"#,
            )
            .unwrap();
        assert_eq!(
            document
                .range_link(paragraph, start, end, revision)
                .unwrap()
                .as_deref(),
            Some("https://example.com")
        );
        let props = document
            .paragraph(paragraph)
            .unwrap()
            .props_of_char("Before ".len());
        let run = document
            .document()
            .unwrap()
            .styles
            .resolve_char(None, props);
        assert!(!run.strike);
        assert_eq!(run.underline, Underline::None);
        assert_eq!(run.vert_align, VertAlign::Baseline);
        assert_eq!(run.highlight, None);
        document
            .format_range_patch(
                paragraph,
                start,
                end,
                revision,
                r#"{"strike":null,"script":null,"underline":null,"highlight":null}"#,
            )
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let style = reopened.styles.get(&character).unwrap();
        assert_eq!(style.chr.underline, Some(Underline::DoubleWave));
        assert_eq!(style.chr.highlight, Some(Highlight::Yellow));
        assert_eq!(style.chr.vert_align, Some(VertAlign::Superscript));
        assert_eq!(style.chr.shading, Some(Rgb(0x10, 0x20, 0x30)));
        let paragraph = reopened
            .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
            .unwrap();
        let run = reopened
            .styles
            .resolve_char(None, paragraph.props_of_char("Before ".len()));
        assert_eq!(run.underline, Underline::DoubleWave);
        assert_eq!(run.vert_align, VertAlign::Superscript);
        assert!(run.strike);
        assert!(run.italic);
        assert_eq!(run.highlight, Some(Rgb(255, 255, 0)));
        assert_eq!(run.link.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn style_chains_names_limits_and_invalid_patches_are_atomic() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Unchanged").unwrap();
        let root = document.add_style("Chain00", "paragraph", None).unwrap();
        let mut previous = root.clone();
        for index in 1..32 {
            previous = document
                .add_style(&format!("Chain{index:02}"), "paragraph", Some(&previous))
                .unwrap();
        }
        let revision = document.revision().unwrap();
        assert!(document
            .add_style("Too deep", "paragraph", Some(&previous))
            .is_err());
        assert!(
            document.set_style_base(&root, Some("Normal")).is_err(),
            "must validate descendants, not just changed style"
        );
        assert!(document.set_style_base(&root, Some(&previous)).is_err());
        assert!(document.add_style("chain00", "paragraph", None).is_err());
        assert!(document.add_style("Normal", "paragraph", None).is_err());
        assert!(document.add_style("Bad\nName", "paragraph", None).is_err());
        assert!(document
            .add_style("Wrong base", "character", Some("Normal"))
            .is_err());
        assert!(document
            .add_style("Missing base", "paragraph", Some("Missing"))
            .is_err());
        assert!(document
            .add_style("Numbering style", "numbering", None)
            .is_err());
        for patch in [
            r#"{"size":20}"#,
            r#"{"font_size":true}"#,
            r#"{"font_size":1.1}"#,
            r#"{"underline":"invented"}"#,
            r#"{"script":"raised"}"#,
            r##"{"highlight":"#123456"}"##,
            r#"{"bold":true,"color":"invalid"}"#,
            r#"{"bold":{"nested":true}}"#,
            "[]",
            r#"{"font_size":NaN}"#,
        ] {
            assert!(
                document.format_style(&root, patch, None).is_err(),
                "{patch}"
            );
            assert!(
                document
                    .format_range_patch(paragraph, 0, 1, revision, patch)
                    .is_err(),
                "{patch}"
            );
        }
        assert!(document
            .format_style(&root, r#"{"style":"Strong"}"#, None)
            .is_err());
        assert!(document
            .format_paragraph(paragraph, r#"{"bold":true}"#)
            .is_err());
        assert!(document
            .format_paragraph(paragraph, r#"{"line_spacing":0}"#)
            .is_err());
        assert!(document
            .format_range_patch(paragraph, 0, 1, revision, r#"{"style":"Normal"}"#)
            .is_err());
        assert!(document
            .format_style(&root, &" ".repeat(MAX_FORMAT_PATCH_BYTES + 1), None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.paragraph_text(paragraph).unwrap(), "Unchanged");
        let mut candidate = document.document().unwrap().clone();
        while candidate.styles.styles.len() < MAX_STYLES {
            let index = candidate.styles.styles.len();
            candidate.styles.upsert(Style {
                id: format!("Filler{index}"),
                name: format!("Filler{index}"),
                ..Default::default()
            });
        }
        document.commit(candidate).unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .add_style("One too many", "paragraph", None)
            .is_err());
        assert!(document
            .add_style("One table too many", "table", None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
    }

    #[test]
    fn table_paragraph_spacing_resolves_native_table_style_and_direct_resets() {
        let mut document = EditableDocx::new();
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let paragraph = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        assert_eq!(
            document.paragraph_spacing(paragraph).unwrap(),
            (0.0, 0.0, 1.0)
        );
        document.format_paragraph(paragraph, r##"{"space_before":12,"space_after":6,"line_spacing":1.5,"alignment":"center","indent_left":5,"indent_right":10,"indent_first":-2,"keep_together":true,"widow_control":false,"background":"#ABCDEF"}"##).unwrap();
        assert_eq!(
            document.paragraph_spacing(paragraph).unwrap(),
            (12.0, 6.0, 1.5)
        );
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let props = &reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap()
            .rows[0]
            .cells[0]
            .blocks[0]
            .as_para()
            .unwrap()
            .props;
        assert_eq!(props.shading, Some(Rgb(0xAB, 0xCD, 0xEF)));
        assert_eq!(props.indent_first, Some(-2.0));
        assert_eq!(props.keep_lines, Some(true));
        assert_eq!(props.widow_control, Some(false));
        document
            .format_paragraph(
                paragraph,
                r#"{"space_before":null,"space_after":null,"line_spacing":null}"#,
            )
            .unwrap();
        assert_eq!(
            document.paragraph_spacing(paragraph).unwrap(),
            (0.0, 0.0, 1.0)
        );
        assert_eq!(document.paragraph_alignment(paragraph).unwrap(), "center");
        let style = document
            .add_style("Cell Body", "paragraph", Some("Normal"))
            .unwrap();
        document
            .format_style(&style, r#"{"space_after":3}"#, None)
            .unwrap();
        document.set_paragraph_style(paragraph, &style).unwrap();
        assert_eq!(
            document.paragraph_spacing(paragraph).unwrap(),
            (0.0, 3.0, 1.0)
        );
    }

    #[test]
    fn formatting_enum_values_and_double_strike_roundtrip() {
        let mut document = EditableDocx::new();
        let style = document.add_style("Marked", "character", None).unwrap();
        for (name, _) in UNDERLINES {
            document
                .format_style(&style, &json!({"underline":name}).to_string(), None)
                .unwrap();
            let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
            assert_eq!(
                underline_name(reopened.styles.get(&style).unwrap().chr.underline.unwrap()),
                *name
            );
        }
        for (name, _) in HIGHLIGHTS {
            document
                .format_style(&style, &json!({"highlight":name}).to_string(), None)
                .unwrap();
            let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
            assert_eq!(
                highlight_name(reopened.styles.get(&style).unwrap().chr.highlight.unwrap()),
                *name
            );
        }
        document
            .format_style(&style, r#"{"strike":"double"}"#, None)
            .unwrap();
        let inherited = document
            .add_style("Inherited Mark", "character", Some(&style))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(
                &document.style_formatting(&inherited, true, None).unwrap()
            )
            .unwrap()["strike"],
            "double"
        );
        document
            .format_style(&inherited, r#"{"strike":false}"#, None)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(
                &document.style_formatting(&inherited, true, None).unwrap()
            )
            .unwrap()["strike"],
            false
        );
        document
            .format_style(&inherited, r#"{"strike":null}"#, None)
            .unwrap();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let resolved = reopened.styles.resolve_char(
            None,
            &CharProps {
                style: Some(inherited),
                ..Default::default()
            },
        );
        assert!(resolved.double_strike);
        assert!(!resolved.strike);
        assert!(document
            .format_style(&style, r#"{"strike":"triple"}"#, None)
            .is_err());
    }
    #[test]
    fn sections_keep_ids_geometry_stories_and_native_page_sizes() {
        let mut document = EditableDocx::new();
        document.set_page_size(400.0, 500.0).unwrap();
        let first = document.add_paragraph("First body").unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let header_para = document
            .story_add_paragraph(header, "Original header")
            .unwrap();
        document
            .add_run_patch(header_para, " bold", r#"{"bold":true}"#)
            .unwrap();
        assert_eq!(document.add_section("next_page").unwrap(), 1);
        let second = document.add_paragraph("Second body").unwrap();
        document
            .set_section_props(
                1,
                r#"{"page_size":[600,700],"margins":[40,50,60,70],"distances":[20,25]}"#,
            )
            .unwrap();
        assert_eq!(document.section_ids().unwrap(), vec![0, 1]);
        assert_eq!(document.paragraph_ids().unwrap(), vec![first, second]);
        assert_eq!(document.page_size().unwrap(), (400.0, 500.0));
        let cloned_header = document.story_id("header", "default", 1).unwrap().unwrap();
        assert_ne!(header, cloned_header);
        let cloned_para = document.story_paragraph_ids(cloned_header).unwrap()[0];
        assert_ne!(header_para, cloned_para);
        assert_eq!(
            document.paragraph_text(cloned_para).unwrap(),
            "Original header bold"
        );
        assert_eq!(
            document
                .paragraph(cloned_para)
                .unwrap()
                .runs
                .last()
                .unwrap()
                .props
                .bold,
            Some(true)
        );
        document
            .set_paragraph_text(header_para, "First header edited")
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(reopened.sections().len(), 2);
        assert_eq!(reopened.sections()[0].1.page_w, 400.0);
        assert_eq!(reopened.sections()[1].1.page_w, 600.0);
        assert_eq!(reopened.last_section.header, 20.0);
        assert_eq!(reopened.last_section.footer, 25.0);
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 2);
        for (index, (width, height, body, head)) in [
            (400.0, 500.0, "First body", "First header edited"),
            (600.0, 700.0, "Second body", "Original header bold"),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(extracted.pages[index].metadata["width"], json!(width));
            assert_eq!(extracted.pages[index].metadata["height"], json!(height));
            let text = extracted.pages[index]
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<String>();
            assert!(text.contains(body), "{text}");
            assert!(text.contains(head), "{text}");
            let png = crate::docx::render(&bytes, index, 72.0, None).unwrap();
            assert_eq!(validate_png(&png).unwrap(), (width as u32, height as u32));
        }
        let mut copied = document.copy().unwrap();
        document.close();
        assert!(document.section_props(0).is_err());
        copied
            .set_paragraph_text(cloned_para, "Copy owns header")
            .unwrap();
        assert_eq!(copied.paragraph_text(first).unwrap(), "First body");
        assert_eq!(copied.section_ids().unwrap(), vec![0, 1]);
    }

    #[test]
    fn absent_section_stories_remain_independent_and_variants_can_be_enabled() {
        let mut document = EditableDocx::new();
        document.add_paragraph("First").unwrap();
        document.add_section("next_page").unwrap();
        document.add_paragraph("Second").unwrap();
        let empty = document.story_id("header", "default", 1).unwrap().unwrap();
        assert!(document.story_paragraph_ids(empty).unwrap().is_empty());
        let first = document.ensure_story("header", "default", 0).unwrap();
        document
            .story_add_paragraph(first, "Only first section")
            .unwrap();
        assert_ne!(first, empty);
        let first_variant = document.ensure_story("header", "first", 1).unwrap();
        document
            .story_add_paragraph(first_variant, "Second first page")
            .unwrap();
        assert!(document.section(1).unwrap().title_page);
        assert!(!document.section(0).unwrap().title_page);
        document.ensure_story("footer", "even", 1).unwrap();
        assert!(document.document().unwrap().settings.even_odd_headers);
        let pdf = crate::docx::to_pdf(&document.to_bytes().unwrap(), None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 2);
        let text = extracted.pages[1]
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<String>();
        assert!(text.contains("Second first page"));
        assert!(!text.contains("Only first section"));
        let before = document.revision().unwrap();
        document.story_id("footer", "default", 0).unwrap();
        assert_eq!(document.revision().unwrap(), before);
    }

    #[test]
    fn section_breaks_preserve_empty_sections_and_body_handles() {
        for (start, expected) in [
            ("next_page", 2),
            ("continuous", 1),
            ("even_page", 2),
            ("odd_page", 3),
        ] {
            let mut document = EditableDocx::new();
            let id = document.add_paragraph("First").unwrap();
            document.add_section(start).unwrap();
            let second = document.add_paragraph("Last").unwrap();
            document.set_paragraph_text(id, "First edited").unwrap();
            assert_eq!(document.paragraph_ids().unwrap(), vec![id, second]);
            let pdf = crate::docx::to_pdf(&document.to_bytes().unwrap(), None).unwrap();
            assert_eq!(
                crate::pdf::extract(&pdf, None).unwrap().pages.len(),
                expected,
                "{start}"
            );
        }
        let mut document = EditableDocx::new();
        document.add_section("next_page").unwrap();
        document.add_section("next_page").unwrap();
        assert!(document.paragraph_ids().unwrap().is_empty());
        let id = document.add_paragraph("After two empty sections").unwrap();
        assert_eq!(document.paragraph_ids().unwrap(), vec![id]);
        let pdf = crate::docx::to_pdf(&document.to_bytes().unwrap(), None).unwrap();
        assert_eq!(crate::pdf::extract(&pdf, None).unwrap().pages.len(), 3);
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        document.add_section("next_page").unwrap();
        assert_eq!(document.paragraph_ids().unwrap(), vec![id]);
        assert_eq!(document.table_rows(table).unwrap(), vec![vec!["Cell"]]);
    }

    #[test]
    fn gutter_and_unequal_columns_use_native_positions_and_roundtrip() {
        let mut document = EditableDocx::new();
        document.set_section_props(0, r#"{"page_size":[440,300],"margins":[20,20,20,20],"gutter":20,"columns":[2,20,true],"column_widths":[120,240]}"#).unwrap();
        let paragraph = document.add_paragraph("").unwrap();
        document
            .add_run_patch(paragraph, "LEFT", r##"{"color":"#FF0000","font_size":18}"##)
            .unwrap();
        document.add_paragraph_break(paragraph, "column").unwrap();
        document
            .add_run_patch(
                paragraph,
                "RIGHT",
                r##"{"color":"#00FF00","font_size":18}"##,
            )
            .unwrap();
        document.add_paragraph_break(paragraph, "column").unwrap();
        document
            .add_run_patch(paragraph, "NEXT", r##"{"color":"#0000FF","font_size":18}"##)
            .unwrap();
        let source = &document.paragraph(paragraph).unwrap().text;
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(layout.pages.len(), 2);
        let mut positions = BTreeMap::new();
        for (page_number, page) in layout.pages.iter().enumerate() {
            assert_eq!((page.body.x, page.body.w), (40.0, 380.0));
            for item in &page.items {
                if let wordcraft_layout::Placed::Lines {
                    path,
                    para,
                    l0,
                    l1,
                    x,
                    ..
                } = item
                {
                    if path == &document.location(paragraph).unwrap().1 {
                        for line in &para.lines[*l0..*l1] {
                            let text = &source[line.start..line.stop];
                            for label in ["LEFT", "RIGHT", "NEXT"] {
                                if text.contains(label) {
                                    positions.insert(label, (page_number, *x));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(positions["LEFT"], (0, 40.0));
        assert_eq!(positions["RIGHT"], (0, 180.0));
        assert_eq!(positions["NEXT"], (1, 40.0));
        let copy = document.copy().unwrap();
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(reopened.last_section.gutter, 20.0);
        assert_eq!(
            reopened.last_section.columns.widths,
            vec![(120.0, 20.0), (240.0, 0.0)]
        );
        assert_eq!(
            reopened.last_section.column_boxes(),
            vec![(0.0, 120.0), (140.0, 240.0)]
        );
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        assert_eq!(crate::pdf::extract(&pdf, None).unwrap().pages.len(), 2);
        for png in [
            crate::docx::render(&bytes, 0, 144.0, None).unwrap(),
            crate::pdf::render(&pdf, 0, 144.0, None).unwrap(),
        ] {
            assert!(border_pixels(png.clone(), 0) > 10);
            assert!(border_pixels(png, 1) > 10);
        }
        document
            .set_section_props(0, r#"{"column_widths":null}"#)
            .unwrap();
        assert!(document
            .document()
            .unwrap()
            .last_section
            .columns
            .widths
            .is_empty());
        assert_eq!(
            copy.document().unwrap().last_section.columns.widths.len(),
            2
        );
        document
            .set_section_props(0, r#"{"column_widths":[120,240]}"#)
            .unwrap();
        document
            .set_section_props(0, r#"{"columns":[2,18,false]}"#)
            .unwrap();
        assert!(document
            .document()
            .unwrap()
            .last_section
            .columns
            .widths
            .is_empty());
    }

    #[test]
    fn unequal_column_and_gutter_geometry_rejects_overflow_atomically() {
        let mut document = EditableDocx::new();
        document.set_section_props(0, r#"{"page_size":[440,300],"margins":[20,20,20,20],"columns":[2,20,false],"column_widths":[120,260]}"#).unwrap();
        let table = document.add_table(vec![vec!["Outer".into()]]).unwrap();
        assert_eq!(document.column_widths(table).unwrap(), vec![120.0]);
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["Nested".into()]])
            .unwrap();
        let png = media_png(1, 1, [255, 0, 0, 255]);
        let image = document
            .add_image(&png, Some(100.0), Some(10.0), "Image")
            .unwrap();
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for patch in [
            r#"{"column_widths":[]}"#,
            r#"{"column_widths":[120]}"#,
            r#"{"column_widths":[17,260]}"#,
            r#"{"column_widths":[120,261]}"#,
            r#"{"column_widths":[true,260]}"#,
            r#"{"column_widths":[-1,260]}"#,
            r#"{"gutter":1}"#,
            r#"{"gutter":-1}"#,
            r#"{"gutter":null}"#,
            r#"{"column_widths":[80,260]}"#,
        ] {
            assert!(document.set_section_props(0, patch).is_err(), "{patch}");
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        assert!(document.set_image_size(image, 121.0, 10.0).is_err());
        assert!(document.set_column_widths(table, vec![121.0]).is_err());
        assert!(document.set_column_widths(nested, vec![120.0]).is_err());
        document
            .set_section_props(0, r#"{"gutter":20,"column_widths":[120,240]}"#)
            .unwrap();
        assert_eq!(document.table_rows(nested).unwrap(), vec![vec!["Nested"]]);
    }

    #[test]
    fn unequal_column_table_preflight_detects_relocation_and_can_recover() {
        let mut document = EditableDocx::new();
        document.set_section_props(0, r#"{"page_size":[440,300],"margins":[20,20,20,20],"columns":[2,20,false],"column_widths":[260,120]}"#).unwrap();
        let before = document.add_paragraph("").unwrap();
        document.add_paragraph_break(before, "column").unwrap();
        let table = document
            .add_table(vec![vec!["WideContent".repeat(5)]])
            .unwrap();
        assert!(document.table(table).unwrap().props.fixed);
        assert_eq!(document.table(table).unwrap().props.width, Some(120.0));
        document
            .table_format_patch(table, r#"{"layout":"autofit","width":null}"#)
            .unwrap();
        document.to_bytes().unwrap();
        document
            .set_paragraph_text(before, "Moved before table")
            .unwrap();
        let revision = document.revision().unwrap();
        assert!(
            document.to_bytes().is_err(),
            "relocated table props: {:?}; column boxes: {:?}",
            document.table(table).unwrap().props,
            document.document().unwrap().last_section.column_boxes()
        );
        assert_eq!(document.revision().unwrap(), revision);
        document.set_column_widths(table, vec![120.0]).unwrap();
        let bytes = document.to_bytes().unwrap();
        crate::docx::render(&bytes, 0, 72.0, None).unwrap();
        crate::docx::to_pdf(&bytes, None).unwrap();
    }

    #[test]
    fn upstream_spanning_table_keeps_starting_width_in_unequal_columns() {
        let mut native = Document::new();
        native.last_section.page_w = 440.0;
        native.last_section.page_h = 200.0;
        native.last_section.margin_top = 20.0;
        native.last_section.margin_bottom = 20.0;
        native.last_section.margin_left = 20.0;
        native.last_section.margin_right = 20.0;
        native.last_section.columns.count = 2;
        native.last_section.columns.widths = vec![(260.0, 20.0), (120.0, 0.0)];
        let mut table = Table::new(20, 1, 260.0);
        for (index, row) in table.rows.iter_mut().enumerate() {
            row.cells[0].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text(
                &format!("Row {index}"),
                CharProps::default(),
            ))];
        }
        native.body = vec![Arc::new(Block::Table(table))];
        let layout =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        assert!(layout.pages.iter().flat_map(|page| &page.items).any(|item| {
            matches!(item, wordcraft_layout::Placed::Cell { rect, .. } if rect.x >= 299.0 && rect.w > 120.0 && rect.right() > 440.0)
        }), "raw native spanning table must reproduce the narrower-column overflow");
        let mut document = EditableDocx::new();
        document.set_section_props(0, r#"{"page_size":[440,200],"margins":[20,20,20,20],"columns":[2,20,false],"column_widths":[260,120]}"#).unwrap();
        let table = document
            .add_table(
                (0..20)
                    .map(|index| vec![format!("Entry{index:02}")])
                    .collect(),
            )
            .unwrap();
        assert_eq!(document.column_widths(table).unwrap(), vec![120.0]);
        for row in 0..20 {
            document
                .set_row_height(table, row, Some(20.0), "at_least")
                .unwrap();
        }
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert!(layout.pages.len() >= 2);
        let mut reached_right = false;
        for page in &layout.pages {
            for item in &page.items {
                if let wordcraft_layout::Placed::Cell { rect, .. } = item {
                    reached_right |= rect.x >= 299.0;
                    // Native TableGrid places the cell rect one half of its
                    // 0.5pt border beyond the column origin.
                    assert!(
                        rect.w <= 120.01 && rect.right() <= 420.26,
                        "safe table rect: {rect:?}"
                    );
                }
            }
        }
        assert!(reached_right);
        document.to_bytes().unwrap();
    }

    #[test]
    fn section_geometry_columns_limits_and_invalid_mutations_are_atomic() {
        let mut document = EditableDocx::new();
        document.add_paragraph("First").unwrap();
        document.add_section("continuous").unwrap();
        document
            .set_section_props(1, r#"{"columns":[2,24,true]}"#)
            .unwrap();
        let table = document
            .add_table(vec![vec!["Left".into(), "Right".into()]])
            .unwrap();
        assert_eq!(
            document.column_widths(table).unwrap().iter().sum::<f64>(),
            222.0
        );
        document
            .set_column_widths(table, vec![100.0, 100.0])
            .unwrap();
        let before = document.revision().unwrap();
        let props = document.section_props(1).unwrap();
        for patch in [
            r#"{"columns":[0,24,false]}"#,
            r#"{"columns":[2.5,24,false]}"#,
            r#"{"columns":[45,50,false]}"#,
            r#"{"columns":[2,400,false]}"#,
            r#"{"columns":[2,-1,false]}"#,
            r#"{"columns":[2,24,1]}"#,
            r#"{"margins":[500,500,500,500]}"#,
            r#"{"distances":[900,25]}"#,
            r#"{"page_size":[400,500]}"#,
            r#"{"start":"next_column"}"#,
            r#"{"unknown":5}"#,
        ] {
            assert!(document.set_section_props(1, patch).is_err(), "{patch}");
            assert_eq!(document.section_props(1).unwrap(), props);
            assert_eq!(document.revision().unwrap(), before);
        }
        assert!(document.set_page_size(400.0, 500.0).is_err());
        assert!(document
            .set_column_widths(table, vec![200.0, 200.0])
            .is_err());
        assert!(document.section_props(2).is_err());
        assert!(document.set_section_props(2, "{}").is_err());
        assert!(document.ensure_story("header", "default", 2).is_err());
        assert!(document.add_section("next_column").is_err());
        let png = crate::rendering::encode_png(1, 1, &[255, 0, 0, 255], 72.0).unwrap();
        assert!(document
            .add_image(&png, Some(250.0), Some(10.0), "")
            .is_err());
        let image = document
            .add_image(&png, Some(100.0), Some(10.0), "")
            .unwrap();
        document.add_section("next_page").unwrap();
        document
            .set_section_props(2, r#"{"columns":[1,36,false]}"#)
            .unwrap();
        assert!(document.set_image_size(image, 300.0, 10.0).is_err());
        document.set_image_size(image, 200.0, 10.0).unwrap();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(reopened.sections()[1].1.columns.count, 2);
        assert_eq!(reopened.sections()[1].1.columns.space, 24.0);
        assert!(reopened.sections()[1].1.columns.separator);
        let mut limits = EditableDocx::new();
        for _ in 1..MAX_SECTIONS {
            limits.add_section("next_page").unwrap();
        }
        let before = limits.revision().unwrap();
        assert!(limits.add_section("next_page").is_err());
        assert_eq!(limits.section_ids().unwrap().len(), MAX_SECTIONS);
        assert_eq!(limits.revision().unwrap(), before);
    }
    #[test]
    fn cell_margins_and_alignment_reuse_native_context_and_preserve_nested_content() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["Main".into(), "Other".into()]])
            .unwrap();
        let first = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let second = document
            .cell_add_paragraph(table, 0, 0, "Second paragraph")
            .unwrap();
        let inner = document
            .cell_add_table(table, 0, 0, vec![vec!["Nested".into()]])
            .unwrap();
        let nested = document.cell_paragraph_ids(inner, 0, 0).unwrap()[0];
        document.set_paragraph_alignment(nested, "right").unwrap();
        document.set_paragraph_alignment(second, "center").unwrap();
        assert!(document.cell_alignment(table, 0, 0).is_err());
        document
            .set_cell_alignment(table, 0, 0, Some("center"))
            .unwrap();
        assert_eq!(
            document.cell_alignment(table, 0, 0).unwrap(),
            Some("center")
        );
        assert_eq!(document.paragraph_alignment(first).unwrap(), "center");
        assert_eq!(document.paragraph_alignment(nested).unwrap(), "right");
        document.set_cell_alignment(table, 0, 0, None).unwrap();
        assert_eq!(document.cell_alignment(table, 0, 0).unwrap(), None);
        assert_eq!(document.paragraph(first).unwrap().props.align, None);
        let before = document.revision().unwrap();
        assert!(document
            .set_cell_margins(table, 0, 0, Some((2.0, 30.0, 4.0, 30.0)))
            .is_err());
        assert_eq!(document.revision().unwrap(), before);
        document.set_column_widths(inner, vec![160.0]).unwrap();
        document
            .set_cell_margins(table, 0, 0, Some((2.0, 30.0, 4.0, 30.0)))
            .unwrap();
        assert_eq!(
            document.cell_margins(table, 0, 0).unwrap(),
            (2.0, 30.0, 4.0, 30.0)
        );
        document
            .set_cell_margins(table, 0, 1, Some((3.0, 14.0, 5.0, 22.0)))
            .unwrap();
        document
            .set_cell_alignment(table, 0, 1, Some("right"))
            .unwrap();
        let before = document.revision().unwrap();
        for margins in [
            (0.0, 200.0, 0.0, 200.0),
            (f64::NAN, 0.0, 0.0, 0.0),
            (-1.0, 0.0, 0.0, 0.0),
        ] {
            assert!(document
                .set_cell_margins(table, 0, 1, Some(margins))
                .is_err());
        }
        assert!(document
            .set_cell_alignment(table, 0, 1, Some("invalid"))
            .is_err());
        assert_eq!(document.revision().unwrap(), before);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let native = reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap();
        assert_eq!(
            native.rows[0].cells[1].props.margins,
            Some([3.0, 22.0, 5.0, 14.0])
        );
        assert_eq!(
            native.rows[0].cells[1].blocks[0]
                .as_para()
                .unwrap()
                .props
                .align,
            Some(Align::Right)
        );
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 1);
        let text = extracted.pages[0]
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<String>();
        assert!(text.contains("Nested"));
        document.set_cell_margins(table, 0, 1, None).unwrap();
        assert_eq!(
            document.table(table).unwrap().rows[0].cells[1]
                .props
                .margins,
            None
        );
        document.merge_cells(table, 0, 0, 0, 1).unwrap();
        document
            .set_cell_alignment(table, 0, 1, Some("justify"))
            .unwrap();
        assert_eq!(
            document.cell_alignment(table, 0, 0).unwrap(),
            Some("justify")
        );
        document
            .set_cell_margins(table, 0, 1, Some((1.0, 2.0, 3.0, 4.0)))
            .unwrap();
        assert_eq!(
            document.cell_margins(table, 0, 0).unwrap(),
            (1.0, 2.0, 3.0, 4.0)
        );
    }
    #[test]
    fn grid_crud_preserves_table_identity_and_invalidates_only_its_children() {
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Outside paragraph").unwrap();
        let table = document
            .add_table(vec![
                vec!["Anchor".into(), "Other".into()],
                vec!["Bottom".into(), "Last".into()],
            ])
            .unwrap();
        document
            .set_column_widths(table, vec![100.0, 100.0])
            .unwrap();
        let anchor = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        document
            .add_run_patch(anchor, " bold", r#"{"bold":true}"#)
            .unwrap();
        document
            .set_row_height(table, 0, Some(30.0), "at_least")
            .unwrap();
        document
            .set_cell_background(table, 0, 0, Some("#123456"))
            .unwrap();
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["Nested".into()]])
            .unwrap();
        let outside = document
            .add_table(vec![vec!["Outside cell".into()]])
            .unwrap();
        let outside_child = document.cell_paragraph_ids(outside, 0, 0).unwrap()[0];
        document.insert_row(table, 1).unwrap();
        assert_eq!(document.table_generation(table).unwrap(), 1);
        assert_eq!(document.table_generation(outside).unwrap(), 0);
        assert!(document.paragraph_text(anchor).is_err());
        assert!(document.table_rows(nested).is_err());
        assert_eq!(document.paragraph_text(body).unwrap(), "Outside paragraph");
        assert_eq!(
            document.paragraph_text(outside_child).unwrap(),
            "Outside cell"
        );
        assert_eq!(document.table_rows(table).unwrap()[1], vec!["", ""]);
        assert_eq!(
            document.row_height(table, 1).unwrap(),
            (Some(30.0), "at_least")
        );
        assert_eq!(
            document.cell_background(table, 1, 0).unwrap(),
            Some("#123456".into())
        );
        let children = document.cell_paragraph_ids(table, 0, 0).unwrap();
        assert_eq!(
            children.len(),
            1,
            "synthetic trailing paragraph must remain hidden"
        );
        let new_anchor = children[0];
        assert!(new_anchor > anchor);
        assert_eq!(
            document
                .paragraph(new_anchor)
                .unwrap()
                .runs
                .last()
                .unwrap()
                .props
                .bold,
            Some(true)
        );
        let new_nested = document.cell_table_ids(table, 0, 0).unwrap()[0];
        assert!(new_nested > nested);
        document
            .cell_add_paragraph(table, 0, 0, "After nested")
            .unwrap();
        assert_eq!(document.cell_paragraph_ids(table, 0, 0).unwrap().len(), 2);
        document.insert_column(table, 1).unwrap();
        assert_eq!(
            document.column_widths(table).unwrap(),
            vec![100.0, 100.0, 100.0]
        );
        document.delete_column(table, 1).unwrap();
        document.delete_row(table, 1).unwrap();
        assert_eq!(document.table_generation(table).unwrap(), 4);
        assert_eq!(document.column_widths(table).unwrap(), vec![100.0, 100.0]);
        assert_eq!(document.table_rows(table).unwrap().len(), 2);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let native = reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap();
        assert_eq!(native.props.width, Some(200.0));
        assert_eq!(native.rows[0].cells[0].props.width, Some(100.0));
        assert_eq!(native.rows[1].cells[1].props.width, Some(100.0));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .map(|block| block.text.as_str())
            .collect::<String>();
        assert_eq!(text.matches("Anchor bold").count(), 1);
        assert_eq!(text.matches("Nested").count(), 1);
        assert!(crate::docx::render(&bytes, 0, 72.0, None)
            .unwrap()
            .starts_with(b"\x89PNG"));
        let mut copied = document.copy().unwrap();
        copied.insert_row(table, 2).unwrap();
        assert_eq!(copied.table_generation(table).unwrap(), 5);
        assert_eq!(document.table_generation(table).unwrap(), 4);
        document.close();
        assert!(document.table_generation(table).is_err());
        assert_eq!(
            copied.paragraph_text(outside_child).unwrap(),
            "Outside cell"
        );
    }

    #[test]
    fn grid_edits_preserve_supported_merges_and_reject_orphan_continuations_atomically() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![
                vec!["A".into(), "B".into(), "C".into()],
                vec!["D".into(), "E".into(), "F".into()],
                vec!["G".into(), "H".into(), "I".into()],
            ])
            .unwrap();
        document
            .set_column_widths(table, vec![80.0, 80.0, 80.0])
            .unwrap();
        document.merge_cells(table, 0, 0, 0, 1).unwrap();
        assert_eq!(document.table_generation(table).unwrap(), 1);
        document.insert_column(table, 1).unwrap();
        assert_eq!(document.table(table).unwrap().rows[0].cells[0].span(), 3);
        assert_eq!(document.cell_text(table, 0, 2).unwrap(), "A\nB");
        document.delete_column(table, 1).unwrap();
        assert_eq!(document.table(table).unwrap().rows[0].cells[0].span(), 2);
        document.merge_cells(table, 1, 2, 2, 2).unwrap();
        let before = document.revision().unwrap();
        let generation = document.table_generation(table).unwrap();
        let rows = document.table_rows(table).unwrap();
        assert!(document.insert_row(table, 2).is_err());
        assert!(document.delete_row(table, 1).is_err());
        assert_eq!(document.revision().unwrap(), before);
        assert_eq!(document.table_generation(table).unwrap(), generation);
        assert_eq!(document.table_rows(table).unwrap(), rows);
        document.insert_row(table, 3).unwrap();
        assert_eq!(document.cell_text(table, 3, 2).unwrap(), "F\nI");
        document.delete_row(table, 2).unwrap();
        assert_eq!(document.cell_text(table, 2, 2).unwrap(), "F\nI");
        document.delete_column(table, 2).unwrap();
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        validate_editable_grid(
            reopened
                .table(StoryRef::Body, &document.location(table).unwrap().1)
                .unwrap(),
        )
        .unwrap();
        assert!(!reopened.plain_text(StoryRef::Body).contains('F'));
        let before = document.table_generation(table).unwrap();
        document.split_cell(table, 0, 0, 2).unwrap();
        assert_eq!(document.table_generation(table).unwrap(), before + 1);
    }

    #[test]
    fn grid_limits_headers_widths_and_failed_edits_keep_generations_and_handles() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![
                vec!["A".into(), "B".into()],
                vec!["C".into(), "D".into()],
            ])
            .unwrap();
        let child = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let before = document.revision().unwrap();
        assert!(
            document.insert_column(table, 1).is_err(),
            "default full-width table cannot silently grow past its container"
        );
        assert!(document.insert_row(table, 3).is_err());
        assert!(document.delete_row(table, 2).is_err());
        assert!(document.insert_column(table, 3).is_err());
        assert!(document.delete_column(table, 2).is_err());
        assert_eq!(document.revision().unwrap(), before);
        assert_eq!(document.table_generation(table).unwrap(), 0);
        assert_eq!(document.paragraph_text(child).unwrap(), "A");
        document.set_row_repeat_header(table, 0, true).unwrap();
        document.set_row_repeat_header(table, 1, true).unwrap();
        let before = document.revision().unwrap();
        assert!(document.insert_row(table, 0).is_err());
        assert!(document.insert_row(table, 1).is_err());
        assert_eq!(document.revision().unwrap(), before);
        document.insert_row(table, 2).unwrap();
        assert!(!document.row_repeat_header(table, 2).unwrap());
        document.delete_row(table, 0).unwrap();
        assert!(document.row_repeat_header(table, 0).unwrap());
        document.delete_row(table, 1).unwrap();
        document.delete_column(table, 1).unwrap();
        let before = document.revision().unwrap();
        assert!(document.delete_row(table, 0).is_err());
        assert!(document.delete_column(table, 0).is_err());
        assert_eq!(document.revision().unwrap(), before);
        document.table_generations.insert(table, u64::MAX);
        assert!(document.insert_row(table, 1).is_err());
        assert_eq!(document.revision().unwrap(), before);
        let mut limits = EditableDocx::new();
        let table = limits.add_table(vec![vec![String::new(); 63]]).unwrap();
        assert!(limits.insert_column(table, 0).is_err());
        limits.cell_count = MAX_CELLS;
        assert!(limits.insert_row(table, 0).is_err());
        assert_eq!(limits.table_generation(table).unwrap(), 0);
    }

    #[test]
    fn grid_deletion_releases_text_and_nested_cell_budget_without_reusing_handles() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![
                vec!["Keep".into(), "Keep2".into()],
                vec!["Drop".into(), "Drop2".into()],
            ])
            .unwrap();
        let nested = document
            .cell_add_table(
                table,
                1,
                0,
                vec![vec!["Nested drop".into(), "Second drop".into()]],
            )
            .unwrap();
        let nested_child = document.cell_paragraph_ids(nested, 0, 0).unwrap()[0];
        let png = crate::rendering::encode_png(1, 1, &[0, 0, 0, 255], 72.0).unwrap();
        let picture = document
            .cell_add_image(table, 1, 1, &png, Some(10.0), Some(10.0), "Drop image")
            .unwrap();
        let old_bytes = document.text_bytes;
        let old_cells = document.cell_count;
        document.delete_row(table, 1).unwrap();
        assert_eq!(document.text_bytes, "KeepKeep2".len());
        assert!(document.text_bytes < old_bytes);
        assert_eq!(document.cell_count, 2);
        assert!(document.cell_count < old_cells);
        assert!(document.table_rows(nested).is_err());
        assert!(document.paragraph_text(nested_child).is_err());
        assert!(document.image_size(picture).is_err());
        assert!(document.image_ids().unwrap().is_empty());
        let fresh = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        assert!(fresh > picture);
        assert_eq!(document.paragraph_text(fresh).unwrap(), "Keep");
    }
    #[test]
    fn nested_grid_changes_preserve_parent_handles_and_guard_merged_child_widths() {
        let mut document = EditableDocx::new();
        let outer = document
            .add_table(vec![vec!["Outer".into(), String::new()]])
            .unwrap();
        document
            .set_column_widths(outer, vec![100.0, 100.0])
            .unwrap();
        document.merge_cells(outer, 0, 0, 0, 1).unwrap();
        let outer_paragraph = document.cell_paragraph_ids(outer, 0, 0).unwrap()[0];
        let inner = document
            .cell_add_table(
                outer,
                0,
                0,
                vec![vec!["Inner".into()], vec!["Second".into()]],
            )
            .unwrap();
        let old_inner_paragraph = document.cell_paragraph_ids(inner, 0, 0).unwrap()[0];
        document.insert_row(inner, 1).unwrap();
        assert_eq!(document.table_generation(inner).unwrap(), 1);
        assert_eq!(document.table_generation(outer).unwrap(), 1);
        assert!(document.paragraph_text(old_inner_paragraph).is_err());
        assert_eq!(document.paragraph_text(outer_paragraph).unwrap(), "Outer");
        let before = document.revision().unwrap();
        assert!(document.delete_column(outer, 1).is_err());
        assert!(document.split_cell(outer, 0, 0, 2).is_err());
        assert_eq!(document.revision().unwrap(), before);
        assert_eq!(document.table_generation(outer).unwrap(), 1);
        assert_eq!(
            document.cell_paragraph_ids(outer, 0, 0).unwrap(),
            vec![outer_paragraph]
        );
        document.set_column_widths(inner, vec![80.0]).unwrap();
        document.delete_column(outer, 1).unwrap();
        assert!(document.table_generation(inner).is_err());
        assert!(!document.table_generations.contains_key(&inner));
        let rebound = document.cell_table_ids(outer, 0, 0).unwrap()[0];
        assert_eq!(document.table_generation(rebound).unwrap(), 0);
        assert_eq!(document.cell_paragraph_ids(outer, 0, 0).unwrap().len(), 1);
        document
            .cell_add_paragraph(outer, 0, 0, "Trailing")
            .unwrap();
        assert_eq!(document.cell_paragraph_ids(outer, 0, 0).unwrap().len(), 2);
        let second = document
            .add_table(vec![vec!["Start".into(), String::new()]])
            .unwrap();
        let nested = document
            .cell_add_table(second, 0, 0, vec![vec!["Content".into()]])
            .unwrap();
        document.set_column_widths(nested, vec![80.0]).unwrap();
        document.merge_cells(second, 0, 0, 0, 1).unwrap();
        assert_eq!(document.cell_paragraph_ids(second, 0, 0).unwrap().len(), 1);
        document.split_cell(second, 0, 0, 2).unwrap();
        assert_eq!(document.cell_paragraph_ids(second, 0, 0).unwrap().len(), 1);
    }
    #[test]
    fn grid_edits_reject_explicit_margin_overflow_before_invalidating_handles() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["A".into(), "B".into()]])
            .unwrap();
        document
            .set_column_widths(table, vec![100.0, 100.0])
            .unwrap();
        document.merge_cells(table, 0, 0, 0, 1).unwrap();
        document
            .set_cell_margins(table, 0, 0, Some((0.0, 75.0, 0.0, 75.0)))
            .unwrap();
        let paragraph = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let revision = document.revision().unwrap();
        let generation = document.table_generation(table).unwrap();
        let before = document.document().unwrap().clone();
        assert!(document.delete_column(table, 1).is_err());
        assert!(document.split_cell(table, 0, 0, 2).is_err());
        assert!(document.set_column_widths(table, vec![50.0, 50.0]).is_err());
        assert_eq!(document.document().unwrap(), &before);
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.table_generation(table).unwrap(), generation);
        assert_eq!(document.paragraph_text(paragraph).unwrap(), "A");
        document
            .set_cell_margins(table, 0, 0, Some((0.0, 10.0, 0.0, 10.0)))
            .unwrap();
        document.delete_column(table, 1).unwrap();
        assert_eq!(document.column_widths(table).unwrap(), vec![100.0]);
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert!(reopened.plain_text(StoryRef::Body).contains('A'));
        assert!(reopened.plain_text(StoryRef::Body).contains('B'));
    }

    #[test]
    fn notes_attach_after_unicode_ranges_and_keep_editable_owned_stories() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("α😀 cited tail").unwrap();
        let revision = document.revision().unwrap();
        let first = document
            .add_note(paragraph, 3, 8, revision, "First note", "footnote")
            .unwrap();
        assert_eq!(
            document.paragraph_text(paragraph).unwrap(),
            "α😀 cited\u{fffc} tail"
        );
        assert!(document
            .add_note(paragraph, 0, 1, revision, "stale", "footnote")
            .is_err());
        let last = document.paragraph_text(paragraph).unwrap().chars().count();
        let second = document
            .add_note(
                paragraph,
                last,
                last,
                document.revision().unwrap(),
                "Second note",
                "endnote",
            )
            .unwrap();
        assert_eq!(document.note_ids(None).unwrap(), vec![first, second]);
        assert_eq!(document.note_ids(Some("footnote")).unwrap(), vec![first]);
        assert_eq!(document.note_ids(Some("endnote")).unwrap(), vec![second]);
        let note_info: Value = serde_json::from_str(&document.note_info(first).unwrap()).unwrap();
        assert_eq!(note_info["kind"], "footnote");
        assert_eq!(note_info["story"], first);
        let note_para = document.story_paragraph_ids(first).unwrap()[0];
        assert_eq!(document.paragraph_text(note_para).unwrap(), "First note");
        document
            .set_paragraph_text(note_para, "Edited λ note")
            .unwrap();
        let extra = document.story_add_paragraph(first, "More detail").unwrap();
        assert_eq!(document.paragraph_style(extra).unwrap(), "Footnote Text");
        let mut copy = document.copy().unwrap();
        copy.set_paragraph_text(note_para, "Copy only").unwrap();
        assert_eq!(document.paragraph_text(note_para).unwrap(), "Edited λ note");
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let footnotes = reopened
            .parts
            .iter()
            .filter(|(_, part)| part.kind == PartKind::Footnote)
            .collect::<Vec<_>>();
        assert_eq!(footnotes.len(), 1);
        assert!(reopened
            .plain_text(StoryRef::Part(*footnotes[0].0))
            .contains("Edited λ note"));
        assert!(reopened
            .plain_text(StoryRef::Part(*footnotes[0].0))
            .contains("More detail"));
        assert!(!reopened
            .plain_text(StoryRef::Body)
            .contains("Edited λ note"));
        document.close();
        assert!(document.note_ids(None).is_err());
        assert!(document.note_info(first).is_err());
        assert_eq!(copy.paragraph_text(note_para).unwrap(), "Copy only");
    }

    #[test]
    fn notes_reject_destructive_edits_but_follow_surrounding_text_and_table_moves() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![
                vec!["safe".into(), "row".into()],
                vec!["anchor".into(), "keep".into()],
            ])
            .unwrap();
        let paragraph = document.cell_paragraph_ids(table, 1, 0).unwrap()[0];
        let note = document
            .add_note(
                paragraph,
                0,
                6,
                document.revision().unwrap(),
                "Note text",
                "footnote",
            )
            .unwrap();
        let snapshot = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        let generation = document.table_generation(table).unwrap();
        assert!(document
            .set_paragraph_text(paragraph, "replacement")
            .is_err());
        assert!(document
            .replace_range(paragraph, 6, 7, revision, "")
            .is_err());
        assert!(document.set_cell_text(table, 1, 0, "replacement").is_err());
        assert!(document.delete_row(table, 1).is_err());
        assert!(document.delete_column(table, 0).is_err());
        assert_eq!(document.document().unwrap(), &snapshot);
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.table_generation(table).unwrap(), generation);
        document
            .replace_range(paragraph, 0, 6, revision, "longer anchor")
            .unwrap();
        assert_eq!(
            document.paragraph_text(paragraph).unwrap(),
            "longer anchor\u{fffc}"
        );
        document.delete_row(table, 0).unwrap();
        assert!(document.paragraph_text(paragraph).is_err());
        let moved = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        assert_eq!(
            document.paragraph_text(moved).unwrap(),
            "longer anchor\u{fffc}"
        );
        assert_eq!(document.note_ids(None).unwrap(), vec![note]);
        document.merge_cells(table, 0, 0, 0, 1).unwrap();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert!(reopened
            .plain_text(StoryRef::Body)
            .contains("longer anchor"));
        assert_eq!(
            reopened
                .parts
                .values()
                .filter(|part| part.kind == PartKind::Footnote)
                .count(),
            1
        );
    }

    #[test]
    fn notes_nested_body_tables_reopen_render_and_export_without_body_duplication() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Body anchor").unwrap();
        document
            .add_note(
                paragraph,
                0,
                11,
                document.revision().unwrap(),
                "Footnote detail",
                "footnote",
            )
            .unwrap();
        let outer = document.add_table(vec![vec!["Outer".into()]]).unwrap();
        let inner = document
            .cell_add_table(outer, 0, 0, vec![vec!["Nested anchor".into()]])
            .unwrap();
        let nested = document.cell_paragraph_ids(inner, 0, 0).unwrap()[0];
        document
            .add_note(
                nested,
                0,
                13,
                document.revision().unwrap(),
                "Nested note detail",
                "footnote",
            )
            .unwrap();
        document
            .add_note(
                paragraph,
                0,
                4,
                document.revision().unwrap(),
                "Endnote detail",
                "endnote",
            )
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened
                .parts
                .values()
                .filter(|part| matches!(part.kind, PartKind::Footnote | PartKind::Endnote))
                .count(),
            3
        );
        assert!(!reopened.plain_text(StoryRef::Body).contains("detail"));
        let png = crate::docx::render(&bytes, 0, 72.0, None).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for label in ["Footnote detail", "Nested note detail", "Endnote detail"] {
            assert!(text.contains(label), "{label}: {text}");
            assert_eq!(text.matches(label).count(), 1, "{text}");
        }
        let footnote = extracted.pages[0]
            .blocks
            .iter()
            .find(|block| block.text.contains("Footnote detail"))
            .unwrap();
        assert!(
            footnote.bbox.unwrap()[1] > 600.0,
            "footnote must be at page bottom: {:?}",
            footnote.bbox
        );
        let payload = crate::docx::extract(&bytes, None).unwrap();
        assert_eq!(
            payload
                .stories
                .iter()
                .filter(|story| matches!(story.kind.as_str(), "footnote" | "endnote"))
                .count(),
            3
        );
        assert!(!payload
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .any(|block| block.text.contains("note detail")));
    }

    #[test]
    fn note_options_inherit_roundtrip_and_reject_ineffective_values_atomically() {
        let mut document = EditableDocx::new();
        let defaults: Value =
            serde_json::from_str(&document.section_note_options(0, "endnote").unwrap()).unwrap();
        assert_eq!(
            defaults,
            json!({"placement":"document_end","numbering":"lower_roman","start":1,"restart":"continuous"})
        );
        document.set_section_note_options(0, "footnote", r#"{"placement":"beneath_text","numbering":"upper_roman","start":4,"restart":"each_page"}"#).unwrap();
        document.set_section_note_options(0, "endnote", r#"{"placement":"section_end","numbering":"upper_letter","start":3,"restart":"each_section"}"#).unwrap();
        let section = document.add_section("next_page").unwrap();
        document
            .set_section_note_options(
                section,
                "footnote",
                r#"{"placement":null,"numbering":null,"start":null,"restart":null}"#,
            )
            .unwrap();
        let reset: Value =
            serde_json::from_str(&document.section_note_options(section, "footnote").unwrap())
                .unwrap();
        assert_eq!(
            reset,
            json!({"placement":"page_bottom","numbering":"decimal","start":1,"restart":"continuous"})
        );
        let revision = document.revision().unwrap();
        let snapshot = document.document().unwrap().clone();
        for patch in [
            r#"{"start":0}"#,
            r#"{"start":32768}"#,
            r#"{"start":true}"#,
            r#"{"start":1.5}"#,
            r#"{"numbering":"unknown"}"#,
            r#"{"placement":"document_end"}"#,
            r#"{"unexpected":true}"#,
            "[]",
        ] {
            assert!(
                document
                    .set_section_note_options(0, "footnote", patch)
                    .is_err(),
                "{patch}"
            );
        }
        assert!(document
            .set_section_note_options(0, "endnote", r#"{"restart":"each_page"}"#)
            .is_err());
        assert!(document
            .set_section_note_options(0, "endnote", r#"{"placement":"page_bottom"}"#)
            .is_err());
        assert!(document.section_note_options(99, "footnote").is_err());
        assert!(document.section_note_options(0, "unknown").is_err());
        assert_eq!(document.document().unwrap(), &snapshot);
        assert_eq!(document.revision().unwrap(), revision);
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let sections = reopened.sections();
        let first = reopened.note_options(sections[0].1, false);
        assert_eq!(first.pos, NotePos::BeneathText);
        assert_eq!(first.num_fmt, NumFormat::UpperRoman);
        assert_eq!(first.num_start, 4);
        assert_eq!(first.num_restart, NoteRestart::EachPage);
        assert_eq!(
            reopened.note_options(sections[0].1, true).pos,
            NotePos::SectEnd
        );
        assert_eq!(reopened.note_options(sections[1].1, false).num_start, 1);
    }

    #[test]
    fn notes_validate_stories_bounds_and_failed_mutations_without_consuming_ids() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Source").unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let header_para = document.story_add_paragraph(header, "Header").unwrap();
        let revision = document.revision().unwrap();
        let snapshot = document.document().unwrap().clone();
        assert!(document
            .add_note(header_para, 0, 6, revision, "bad", "footnote")
            .is_err());
        assert!(document
            .add_note(paragraph, 0, 7, revision, "bad", "footnote")
            .is_err());
        assert!(document
            .add_note(paragraph, 0, 1, revision, "bad", "unknown")
            .is_err());
        assert!(document
            .add_note(paragraph, 0, 1, revision, "bad\rtext", "footnote")
            .is_err());
        assert!(document
            .add_note(
                paragraph,
                0,
                1,
                revision,
                &"x".repeat(MAX_TEXT_BYTES + 1),
                "footnote"
            )
            .is_err());
        assert_eq!(document.document().unwrap(), &snapshot);
        assert_eq!(document.revision().unwrap(), revision);
        let first = document
            .add_note(paragraph, 0, 1, revision, "First", "footnote")
            .unwrap();
        let own = document.story_paragraph_ids(first).unwrap()[0];
        assert!(document
            .add_note(own, 0, 1, document.revision().unwrap(), "nested", "endnote")
            .is_err());
        document.text_bytes = MAX_DOCUMENT_TEXT_BYTES;
        let revision = document.revision().unwrap();
        assert!(document
            .add_note(paragraph, 0, 1, revision, "over budget", "footnote")
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.note_ids(None).unwrap(), vec![first]);
    }

    #[test]
    fn note_count_and_anchor_bytes_are_bounded_even_for_empty_notes() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("").unwrap();
        document.text_bytes = MAX_DOCUMENT_TEXT_BYTES - wordcraft_doc::para::OBJ.len_utf8();
        document
            .add_note(
                paragraph,
                0,
                0,
                document.revision().unwrap(),
                "",
                "footnote",
            )
            .unwrap();
        assert_eq!(document.text_bytes, MAX_DOCUMENT_TEXT_BYTES);
        let revision = document.revision().unwrap();
        assert!(document
            .add_note(paragraph, 0, 0, revision, "", "endnote")
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Source").unwrap();
        for _ in 0..MAX_NOTES {
            document
                .document
                .as_mut()
                .unwrap()
                .try_add_part(PartKind::Footnote, Vec::new())
                .unwrap();
        }
        let revision = document.revision().unwrap();
        assert!(document
            .add_note(paragraph, 0, 0, revision, "", "footnote")
            .is_err());
        assert_eq!(document.note_ids(None).unwrap().len(), MAX_NOTES);
        assert_eq!(document.revision().unwrap(), revision);
    }

    #[test]
    fn table_geometry_entry_points_reject_overflow_before_commit() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["a".into(), "b".into()]])
            .unwrap();
        document
            .table_format_patch(table, r#"{"alignment":"center","indent":400}"#)
            .unwrap();
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        for alignment in [r#"{"alignment":"left"}"#, r#"{"alignment":null}"#] {
            assert!(document.table_format_patch(table, alignment).is_err());
            assert_eq!(document.document().unwrap(), &before);
            assert_eq!(document.revision().unwrap(), revision);
        }
        let mut document = EditableDocx::new();
        let table = document.add_table(vec![vec!["a".into()]]).unwrap();
        document
            .table_format_patch(table, r#"{"width_percent":50,"layout":"fixed"}"#)
            .unwrap();
        document
            .set_cell_margins(table, 0, 0, Some((0.0, 100.0, 0.0, 100.0)))
            .unwrap();
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        for patch in [
            r#"{"margins":[72,180,72,180]}"#,
            r#"{"page_size":[400,792]}"#,
            r#"{"columns":[2,24,false]}"#,
        ] {
            assert!(document.set_section_props(0, patch).is_err(), "{patch}");
            assert_eq!(document.document().unwrap(), &before);
            assert_eq!(document.revision().unwrap(), revision);
        }
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["a".into(), "b".into()]])
            .unwrap();
        document
            .table_format_patch(table, r#"{"layout":"fixed"}"#)
            .unwrap();
        document.merge_cells(table, 0, 0, 0, 1).unwrap();
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["nested".into()]])
            .unwrap();
        document
            .table_format_patch(nested, r#"{"width_percent":100,"layout":"fixed"}"#)
            .unwrap();
        document
            .set_cell_margins(nested, 0, 0, Some((0.0, 150.0, 0.0, 150.0)))
            .unwrap();
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        let generation = document.table_generation(table).unwrap();
        assert!(document.split_cell(table, 0, 0, 2).is_err());
        assert_eq!(document.document().unwrap(), &before);
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.table_generation(table).unwrap(), generation);
        assert_eq!(document.cell_text(nested, 0, 0).unwrap(), "nested");
    }

    #[test]
    fn table_formatting_preserves_direct_preferences_metadata_and_native_output() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![
                vec!["LEFT".into(), "RIGHT".into()],
                vec!["one".into(), "two".into()],
            ])
            .unwrap();
        let generation = document.table_generation(table).unwrap();
        document.table_format_patch(table, r##"{"alignment":"center","width":240,"layout":"fixed","rtl":true,"title":"Résumé totals","description":"Accessible description","borders":{"top":{"style":"double","color":"#123456","width":1.5},"inside_vertical":{"style":"dotted","width":0.5}}}"##).unwrap();
        let value: Value =
            serde_json::from_str(&document.table_formatting(table).unwrap()).unwrap();
        assert_eq!(value["width"], 240.0);
        assert!(value["width_percent"].is_null());
        assert_eq!(value["title"], "Résumé totals");
        assert_eq!(value["layout"], "fixed");
        assert_eq!(document.table_generation(table).unwrap(), generation);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let native = reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap();
        assert_eq!(native.props.align, Some(Align::Center));
        assert_eq!(native.props.width, Some(240.0));
        assert!(native.props.bidi_visual);
        assert_eq!(native.props.caption.as_deref(), Some("Résumé totals"));
        assert_eq!(
            native.props.description.as_deref(),
            Some("Accessible description")
        );
        assert_eq!(
            native.props.borders.unwrap().top.unwrap().style,
            BorderStyle::Double
        );
        let layout = wordcraft_layout::layout(
            &reopened,
            &mut wordcraft_layout::LayoutCache::new(),
            &wordcraft_layout::LayoutOptions::default(),
        );
        let mut x = [None, None];
        for item in &layout.pages[0].items {
            if let wordcraft_layout::Placed::Cell {
                table: path,
                row: 0,
                cell,
                rect,
                ..
            } = item
            {
                if path == &document.location(table).unwrap().1 {
                    x[*cell] = Some(rect.x);
                }
            }
        }
        assert!(
            x[0].unwrap() > x[1].unwrap(),
            "RTL keeps logical column zero on the right"
        );
        assert!(crate::docx::render(&bytes, 0, 72.0, None)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .flat_map(|block| {
                std::iter::once(block.text.as_str()).chain(
                    block
                        .table
                        .iter()
                        .flat_map(|table| &table.rows)
                        .flatten()
                        .map(String::as_str),
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text.matches("LEFT").count(), 1, "{text:?}");
        assert_eq!(text.matches("RIGHT").count(), 1, "{text:?}");
        assert!(!text.contains("Accessible description"));
    }

    #[test]
    fn table_formatting_width_precedence_and_invalid_patches_are_atomic() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["a".into(), "b".into()]])
            .unwrap();
        document
            .table_format_patch(
                table,
                r#"{"width_percent":50,"width":null,"layout":"fixed"}"#,
            )
            .unwrap();
        assert_eq!(document.table(table).unwrap().props.width_pct, Some(50.0));
        document
            .table_format_patch(table, r#"{"width_percent":null,"width":200}"#)
            .unwrap();
        assert_eq!(document.table(table).unwrap().props.width, Some(200.0));
        assert_eq!(document.table(table).unwrap().props.width_pct, None);
        document
            .table_format_patch(table, r#"{"width_percent":50}"#)
            .unwrap();
        assert_eq!(document.table(table).unwrap().props.width, None);
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        for patch in [
            r#"{"width":200,"width_percent":50}"#,
            r#"{"width_percent":0.03}"#,
            r#"{"width_percent":101}"#,
            r#"{"width_percent":0.02}"#,
            r#"{"width":1500}"#,
            r#"{"indent":1500}"#,
            r#"{"alignment":"justify"}"#,
            r#"{"layout":"guess"}"#,
            r#"{"rtl":1}"#,
            r##"{"background":"#FFFFFF"}"##,
            r#"{"borders":{"top":{"style":"doubleWave"}}}"#,
            r#"{"borders":{"top":{"width":0.1}}}"#,
        ] {
            assert!(
                document.table_format_patch(table, patch).is_err(),
                "{patch}"
            );
            assert_eq!(document.document().unwrap(), &before, "{patch}");
            assert_eq!(document.revision().unwrap(), revision);
        }
        document
            .table_format_patch(table, r#"{"width":200,"alignment":"left","indent":12}"#)
            .unwrap();
        let widths = native_cell_widths(document.document().unwrap()).unwrap();
        assert!(
            (native_cell_width(
                &widths,
                StoryRef::Body,
                &document.location(table).unwrap().1,
                0,
                0
            )
            .unwrap()
                - 100.0)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn cell_formatting_reuses_direct_properties_borders_and_native_directions() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["Turned text".into(), "Normal text".into()]])
            .unwrap();
        // The old left-wave fixture checked only serialization and file headers.
        // Native raster output omits vertical waves, so reject that loss explicitly.
        let revision = document.revision().unwrap();
        assert!(document
            .cell_format_patch(
                table,
                0,
                0,
                r#"{"borders":{"left":{"style":"wave","width":1}}}"#
            )
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.cell_format_patch(table,0,0,r##"{"background":"#ABCDEF","margins":[2,3,4,5],"alignment":"center","vertical_alignment":"bottom","text_direction":"down","no_wrap":true,"borders":{"left":{"style":"dashed","color":"#123456","width":1},"top":{"style":"none"}}}"##).unwrap();
        let value: Value =
            serde_json::from_str(&document.cell_formatting(table, 0, 0).unwrap()).unwrap();
        assert_eq!(value["background"], "#ABCDEF");
        assert_eq!(value["margins"], json!([2.0, 3.0, 4.0, 5.0]));
        assert_eq!(value["text_direction"], "down");
        assert_eq!(value["borders"]["top"]["style"], "none");
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let cell = &reopened
            .table(StoryRef::Body, &document.location(table).unwrap().1)
            .unwrap()
            .rows[0]
            .cells[0];
        assert_eq!(cell.props.margins, Some([2.0, 5.0, 4.0, 3.0]));
        assert_eq!(cell.props.text_direction, TextDirection::Down);
        assert!(cell.props.no_wrap);
        assert_eq!(
            cell.props.borders.unwrap().left.unwrap().style,
            BorderStyle::Dashed
        );
        assert!(crate::docx::render(&bytes, 0, 72.0, None)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(crate::docx::to_pdf(&bytes, None)
            .unwrap()
            .starts_with(b"%PDF-"));
        document.cell_format_patch(table,0,0,r#"{"background":null,"margins":null,"alignment":null,"vertical_alignment":null,"text_direction":null,"borders":{"left":null}}"#).unwrap();
        let value: Value =
            serde_json::from_str(&document.cell_formatting(table, 0, 0).unwrap()).unwrap();
        for key in [
            "background",
            "margins",
            "alignment",
            "vertical_alignment",
            "text_direction",
        ] {
            assert!(value[key].is_null(), "{key}");
        }
        assert!(value["borders"]["left"].is_null());
        assert_eq!(value["borders"]["top"]["style"], "none");
        let revision = document.revision().unwrap();
        for patch in [
            r#"{"margins":[-1,0,0,0]}"#,
            r#"{"margins":[0,2000,0,0]}"#,
            r#"{"vertical_alignment":"middle"}"#,
            r#"{"text_direction":"rtl"}"#,
            r#"{"no_wrap":null}"#,
            r#"{"borders":{"inside_vertical":{"style":"single"}}}"#,
        ] {
            assert!(
                document.cell_format_patch(table, 0, 0, patch).is_err(),
                "{patch}"
            );
            assert_eq!(document.revision().unwrap(), revision);
        }
    }

    #[test]
    fn table_formatting_checks_effective_parent_width_and_preserves_nested_content() {
        let mut document = EditableDocx::new();
        let table = document.add_table(vec![vec!["Parent".into()]]).unwrap();
        document
            .table_format_patch(table, r#"{"width_percent":50,"layout":"fixed"}"#)
            .unwrap();
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["Nested".into()]])
            .unwrap();
        assert!(document.table(nested).unwrap().total_width() < 234.0);
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        assert!(document
            .table_format_patch(table, r#"{"width_percent":25}"#)
            .is_err());
        assert!(document
            .cell_format_patch(table, 0, 0, r#"{"margins":[0,200,0,200]}"#)
            .is_err());
        assert_eq!(document.document().unwrap(), &before);
        assert_eq!(document.revision().unwrap(), revision);
        let png = crate::rendering::encode_png(1, 1, &[0, 0, 0, 255], 72.0).unwrap();
        assert!(document
            .cell_add_image(table, 0, 0, &png, Some(300.0), Some(10.0), "")
            .is_err());
        let image = document
            .cell_add_image(table, 0, 0, &png, Some(20.0), Some(10.0), "")
            .unwrap();
        assert!(document.set_image_size(image, 300.0, 10.0).is_err());
        document
            .table_format_patch(nested, r#"{"width":80,"layout":"fixed"}"#)
            .unwrap();
        document
            .table_format_patch(table, r#"{"width_percent":25}"#)
            .unwrap();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert!(reopened.plain_text(StoryRef::Body).contains("Nested"));
    }

    #[test]
    fn table_native_fit_measures_content_and_rejects_truncation_and_overflow() {
        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["a".into(), "Longer text".into()]])
            .unwrap();
        document
            .table_format_patch(table, r#"{"width_percent":100,"layout":"fixed"}"#)
            .unwrap();
        let widths = document.fit_table(table).unwrap();
        assert_eq!(widths.len(), 2);
        assert!(widths[1] > widths[0]);
        assert!(widths.iter().sum::<f64>() < 468.0);
        assert_eq!(document.table(table).unwrap().props.width_pct, None);
        assert_eq!(document.table(table).unwrap().props.width, None);
        assert!(!document.table(table).unwrap().props.fixed);
        document
            .set_cell_text(table, 0, 1, &"W".repeat(200))
            .unwrap();
        let revision = document.revision().unwrap();
        assert!(document.fit_table(table).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let mut document = EditableDocx::new();
        let table = document
            .add_table((0..4001).map(|_| vec!["x".into()]).collect())
            .unwrap();
        let revision = document.revision().unwrap();
        assert!(document.fit_table(table).is_err());
        assert_eq!(document.revision().unwrap(), revision);
    }

    #[test]
    fn conditional_vertical_border_guard_is_atomic_and_allows_resets() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let style = document
            .add_style("Checked conditional edges", "table", None)
            .unwrap();
        let table = document
            .add_table(vec![vec!["Kept".into(), "Other".into()]])
            .unwrap();
        document
            .table_format_patch(table, &json!({"style":style}).to_string())
            .unwrap();
        let affected = [
            ("first_row", "inside_vertical"),
            ("last_row", "inside_vertical"),
            ("odd_rows", "inside_vertical"),
            ("even_rows", "inside_vertical"),
            ("first_column", "right"),
            ("last_column", "left"),
            ("top_left", "right"),
            ("bottom_left", "right"),
            ("top_right", "left"),
            ("bottom_right", "left"),
            ("odd_columns", "left"),
            ("odd_columns", "right"),
            ("even_columns", "left"),
            ("even_columns", "right"),
        ];
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for (region, side) in affected {
            for border in [
                json!({"style":"single","color":"#FF0000","width":4}),
                json!({"style":"none"}),
            ] {
                let error = document
                    .format_style(
                        &style,
                        &json!({"background":"#00FF00","borders":{side:border}}).to_string(),
                        Some(region),
                    )
                    .unwrap_err();
                assert!(error.to_string().contains("conditional table styles"));
                assert_eq!(document.revision().unwrap(), revision);
                assert_eq!(document.to_bytes().unwrap(), before);
                assert_eq!(document.table_rows(table).unwrap()[0][0], "Kept");
            }
        }
        for (region, side) in affected {
            document
                .format_style(
                    &style,
                    &json!({"borders":{side:null}}).to_string(),
                    Some(region),
                )
                .unwrap();
            document
                .format_style(&style, r#"{"borders":null}"#, Some(region))
                .unwrap();
        }
        for (region, side) in [
            ("first_row", "left"),
            ("last_row", "right"),
            ("odd_rows", "left"),
            ("even_rows", "right"),
            ("first_column", "left"),
            ("last_column", "right"),
            ("top_left", "left"),
            ("bottom_left", "left"),
            ("top_right", "right"),
            ("bottom_right", "right"),
            ("odd_columns", "top"),
            ("even_columns", "bottom"),
        ] {
            document
                .format_style(
                    &style,
                    &json!({"borders":{side:{"color":"#FF0000","width":4}}}).to_string(),
                    Some(region),
                )
                .unwrap();
        }
        document
            .format_style(
                &style,
                r##"{"borders":{"inside_vertical":{"color":"#FF0000","width":4}}}"##,
                None,
            )
            .unwrap();
        document
            .table_format_patch(
                table,
                r##"{"borders":{"inside_vertical":{"color":"#0000FF","width":2}}}"##,
            )
            .unwrap();
        document
            .cell_format_patch(table, 0, 0, r#"{"borders":{"right":{"style":"none"}}}"#)
            .unwrap();
        document.set_table_borders(table, "#123456", 1.0).unwrap();
        assert_eq!(
            document.table_borders(table).unwrap(),
            Some(("#123456".into(), 1.0))
        );
    }

    #[test]
    fn upstream_conditional_vertical_borders_are_serialized_but_dropped() {
        fn raw(part: TablePart, side: &str) -> (Document, String) {
            let mut owned = EditableDocx::new();
            let id = owned.add_style("Raw native border", "table", None).unwrap();
            let table = owned.add_table(vec![vec![String::new(); 3]; 3]).unwrap();
            owned.table_format_patch(table,&json!({"style":id,"look":{"header_row":true,"first_column":true,"last_column":true,"banded_columns":true,"banded_rows":false}}).to_string()).unwrap();
            owned.set_column_widths(table, vec![80.0; 3]).unwrap();
            for row in 0..3 {
                owned
                    .set_row_height(table, row, Some(30.0), "exact")
                    .unwrap();
            }
            let mut native = owned.document().unwrap().clone();
            let style = native
                .styles
                .styles
                .iter_mut()
                .find(|style| style.id == id)
                .unwrap();
            let mut format = style.table_part(part);
            apply_border_patch(
                &mut format.borders,
                &json!({side:{"color":"#FF0000","width":4}}),
                true,
            )
            .unwrap();
            style.set_table_part(part, format);
            (native, id)
        }
        for (part, side, visible) in [
            (TablePart::WholeTable, "inside_vertical", true),
            (TablePart::FirstRow, "inside_vertical", false),
            (TablePart::FirstCol, "right", false),
            (TablePart::LastCol, "left", false),
            (TablePart::NwCell, "right", false),
            (TablePart::NeCell, "left", false),
            (TablePart::Band1Vert, "right", false),
            (TablePart::FirstCol, "left", true),
            (TablePart::NeCell, "right", true),
            (TablePart::FirstRow, "top", true),
        ] {
            let (native, id) = raw(part, side);
            let bytes = wordcraft_docx::write(&native).unwrap();
            let reopened = wordcraft_docx::read(&bytes).unwrap();
            assert_eq!(
                reopened.styles.find(&id).unwrap().table_part(part).borders,
                native.styles.find(&id).unwrap().table_part(part).borders
            );
            let layout =
                wordcraft_layout::layout(&reopened, &mut Default::default(), &Default::default());
            let red_rules = layout.pages.iter().flat_map(|page| &page.items).filter(|item| {
                matches!(item, wordcraft_layout::Placed::Rule {border,..} if border.color==Some(Rgb(255,0,0)))
            }).count();
            assert_eq!(red_rules > 0, visible, "{part:?}/{side}");
            let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
            for png in [
                crate::docx::render(&bytes, 0, 144.0, None).unwrap(),
                crate::pdf::render(&pdf, 0, 144.0, None).unwrap(),
            ] {
                assert_eq!(border_pixels(png, 0) > 100, visible, "{part:?}/{side}");
            }
        }
        // Explicit nil is also lost: unlike direct-cell nil it cannot suppress the
        // inherited table edge. Readback proves that serialization did not drop it.
        let (mut native, id) = raw(TablePart::WholeTable, "inside_vertical");
        let original =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        let style = native
            .styles
            .styles
            .iter_mut()
            .find(|style| style.id == id)
            .unwrap();
        let mut part = style.table_part(TablePart::FirstRow);
        part.borders = Some(Borders {
            inside_v: Some(Border::default()),
            ..Default::default()
        });
        style.set_table_part(TablePart::FirstRow, part);
        let bytes = wordcraft_docx::write(&native).unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened
                .styles
                .find(&id)
                .unwrap()
                .table_part(TablePart::FirstRow)
                .borders
                .unwrap()
                .inside_v
                .unwrap()
                .style,
            BorderStyle::None
        );
        let ignored =
            wordcraft_layout::layout(&reopened, &mut Default::default(), &Default::default());
        fn red_rules(layout: &wordcraft_layout::DocLayout) -> Vec<(f32, f32, f32, f32)> {
            layout
                .pages
                .iter()
                .flat_map(|page| &page.items)
                .filter_map(|item| match item {
                    wordcraft_layout::Placed::Rule {
                        x0,
                        y0,
                        x1,
                        y1,
                        border,
                    } if border.color == Some(Rgb(255, 0, 0)) => Some((*x0, *y0, *x1, *y1)),
                    _ => None,
                })
                .collect()
        }
        assert_eq!(red_rules(&original), red_rules(&ignored));
        for block in &mut native.body {
            if let Block::Table(table) = Arc::make_mut(block) {
                for cell in &mut table.rows[0].cells {
                    cell.props.borders = Some(Borders {
                        left: Some(Border::default()),
                        right: Some(Border::default()),
                        ..Default::default()
                    });
                }
            }
        }
        let direct =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        assert!(
            red_rules(&direct).len() < red_rules(&original).len(),
            "direct-cell nil must still suppress its edges"
        );
    }

    #[test]
    fn conditional_table_styles_roundtrip_all_regions_and_clear_inheritance() {
        let mut document = EditableDocx::new();
        let base = document
            .add_style("Table résumé", "table", Some("TableGrid"))
            .unwrap();
        document.format_style(&base,r##"{"font_size":11,"space_after":2,"alignment":"center","background":"#F1F2F3","margins":[1,2,3,4],"row_band_size":2,"vertical_alignment":"bottom"}"##,None).unwrap();
        for (index, (region, _)) in TABLE_REGIONS.iter().enumerate().skip(1) {
            document.format_style(&base,&json!({"bold":true,"italic":false,"color":"#123456","alignment":"right","vertical_alignment":"center","background":format!("#{:06X}",0x120000+index),"borders":{"top":{"style":"double","width":1.0}}}).to_string(),Some(region)).unwrap();
        }
        let derived = document
            .add_style("Derived table", "table", Some(&base))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&document.style_info(&derived).unwrap()).unwrap()["kind"],
            "table"
        );
        assert!(document.style_ids(None).unwrap().contains(&derived));
        assert!(document.style_ids(Some("table")).unwrap().contains(&base));
        assert!(!document
            .style_ids(Some("paragraph"))
            .unwrap()
            .contains(&base));
        let direct: Value =
            serde_json::from_str(&document.style_formatting(&derived, false, None).unwrap())
                .unwrap();
        let resolved: Value =
            serde_json::from_str(&document.style_formatting(&derived, true, None).unwrap())
                .unwrap();
        assert!(direct["background"].is_null());
        assert_eq!(direct["regions"].as_object().unwrap().len(), 12);
        assert_eq!(resolved["background"], "#F1F2F3");
        assert_eq!(resolved["space_after"], 2.0);
        assert_eq!(resolved["margins"], json!([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(resolved["regions"]["first_row"]["bold"], true);
        document
            .format_style(
                &derived,
                r#"{"bold":false,"borders":{"top":{"style":"none"}}}"#,
                Some("first_row"),
            )
            .unwrap();
        let inherited = document
            .document()
            .unwrap()
            .styles
            .table_style(&derived)
            .unwrap()
            .part(TablePart::FirstRow);
        assert_eq!(inherited.chr.bold, Some(false));
        assert_eq!(
            inherited.borders.unwrap().top.unwrap().style,
            BorderStyle::None
        );
        document
            .format_style(
                &derived,
                r#"{"bold":null,"borders":{"top":null}}"#,
                Some("first_row"),
            )
            .unwrap();
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .table_style(&derived)
                .unwrap()
                .part(TablePart::FirstRow)
                .chr
                .bold,
            Some(true)
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        for (_, part) in TABLE_REGIONS {
            assert_eq!(
                reopened.styles.get(&base).unwrap().table_part(*part),
                document
                    .document()
                    .unwrap()
                    .styles
                    .get(&base)
                    .unwrap()
                    .table_part(*part)
            );
        }
        assert_eq!(
            reopened
                .styles
                .get(&base)
                .unwrap()
                .table
                .as_ref()
                .unwrap()
                .cell_margins,
            Some([1.0, 4.0, 3.0, 2.0])
        );
        assert_eq!(
            reopened
                .styles
                .get(&base)
                .unwrap()
                .table
                .as_ref()
                .unwrap()
                .band_size,
            Some(2)
        );
        assert!(document
            .copy()
            .unwrap()
            .style_ids(Some("table"))
            .unwrap()
            .contains(&derived));
    }

    fn table_style_cell_fill(
        layout: &wordcraft_layout::DocLayout,
        table: &Path,
        row: usize,
        cell: usize,
    ) -> Option<Rgb> {
        let items = &layout.pages[0].items;
        let rect = items
            .iter()
            .find_map(|item| match item {
                wordcraft_layout::Placed::Cell {
                    table: path,
                    row: r,
                    cell: c,
                    rect,
                    ..
                } if path == table && *r == row && *c == cell => Some(rect),
                _ => None,
            })
            .unwrap();
        items.iter().find_map(|item| match item {
            wordcraft_layout::Placed::Fill { rect: fill, color }
                if (fill.x - rect.x).abs() < 0.01
                    && (fill.y - rect.y).abs() < 0.01
                    && (fill.w - rect.w).abs() < 0.01 =>
            {
                Some(*color)
            }
            _ => None,
        })
    }

    #[test]
    fn conditional_table_styles_native_precedence_look_rtl_and_direct_overrides() {
        let mut document = EditableDocx::new();
        let style = document
            .add_style("Conditional palette", "table", None)
            .unwrap();
        for (region, color) in [
            ("whole_table", "#111111"),
            ("odd_columns", "#222222"),
            ("odd_rows", "#333333"),
            ("even_rows", "#444444"),
            ("first_column", "#555555"),
            ("first_row", "#666666"),
            ("top_left", "#777777"),
        ] {
            document
                .format_style(
                    &style,
                    &json!({"background":color}).to_string(),
                    Some(region),
                )
                .unwrap();
        }
        document
            .format_style(
                &style,
                r#"{"bold":true,"alignment":"right"}"#,
                Some("first_row"),
            )
            .unwrap();
        let table = document
            .add_table(
                (0..4)
                    .map(|r| (0..3).map(|c| format!("Cell{r}{c}")).collect())
                    .collect(),
            )
            .unwrap();
        document
            .table_format_patch(
                table,
                &json!({"style":style,"rtl":true,"look":{"banded_columns":true}}).to_string(),
            )
            .unwrap();
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        for (r, c, color) in [
            (0, 0, "#777777"),
            (0, 1, "#666666"),
            (1, 0, "#555555"),
            (1, 1, "#333333"),
            (2, 1, "#444444"),
        ] {
            assert_eq!(
                table_style_cell_fill(&layout, &document.location(table).unwrap().1, r, c),
                Some(parse_rgb(color).unwrap())
            );
        }
        let mut xs = [None, None];
        for item in &layout.pages[0].items {
            if let wordcraft_layout::Placed::Cell {
                table: path,
                row: 0,
                cell,
                rect,
                ..
            } = item
            {
                if path == &document.location(table).unwrap().1 && *cell < 2 {
                    xs[*cell] = Some(rect.x);
                }
            }
        }
        assert!(xs[0].unwrap() > xs[1].unwrap());
        document
            .format_style(&style, r#"{"row_band_size":2}"#, None)
            .unwrap();
        let bands = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(
            table_style_cell_fill(&bands, &document.location(table).unwrap().1, 2, 1),
            Some(parse_rgb("#333333").unwrap())
        );
        assert_eq!(
            table_style_cell_fill(&bands, &document.location(table).unwrap().1, 3, 1),
            Some(parse_rgb("#444444").unwrap())
        );
        let paragraph = document.cell_paragraph_ids(table, 0, 1).unwrap()[0];
        let (_, path) = document.location(paragraph).unwrap();
        let formatted = layout.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { path: p, para, .. } if p == &path => Some(para),
                _ => None,
            })
            .unwrap();
        assert_eq!(formatted.rp.align, Align::Right);
        assert!(formatted.styles.iter().any(|s| s.rc.bold));
        document
            .set_cell_background(table, 0, 0, Some("#ABCDEF"))
            .unwrap();
        document
            .format_paragraph(paragraph, r#"{"alignment":"left"}"#)
            .unwrap();
        let revision = document.revision().unwrap();
        document
            .format_range_patch(paragraph, 0, 6, revision, r#"{"bold":false}"#)
            .unwrap();
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(
            table_style_cell_fill(&layout, &document.location(table).unwrap().1, 0, 0),
            Some(parse_rgb("#ABCDEF").unwrap())
        );
        let formatted = layout.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { path: p, para, .. } if p == &path => Some(para),
                _ => None,
            })
            .unwrap();
        assert_eq!(formatted.rp.align, Align::Left);
        assert!(formatted.styles.iter().all(|s| !s.rc.bold));
        document
            .table_format_patch(
                table,
                r#"{"look":{"header_row":false,"first_column":false}}"#,
            )
            .unwrap();
        assert!(!document.table(table).unwrap().rows[0].props.header);
        document.set_row_repeat_header(table, 0, true).unwrap();
        assert!(!document.table(table).unwrap().props.look.header_row);
        document
            .table_format_patch(table, r#"{"look":null}"#)
            .unwrap();
        assert_eq!(
            document.table(table).unwrap().props.look,
            Default::default()
        );
        document.merge_cells(table, 2, 0, 2, 1).unwrap();
        document
            .table_format_patch(table, r#"{"look":{"first_column":true}}"#)
            .unwrap();
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(
            table_style_cell_fill(&layout, &document.location(table).unwrap().1, 2, 0),
            Some(parse_rgb("#555555").unwrap())
        );
    }

    #[test]
    fn conditional_table_styles_native_output_and_base_mutations() {
        let mut document = EditableDocx::new();
        let base = document
            .add_style("Base report", "table", Some("TableGrid"))
            .unwrap();
        let child = document
            .add_style("Child report", "table", Some(&base))
            .unwrap();
        let table = document
            .add_table(vec![
                vec!["LabelA".into(), "LabelB".into()],
                vec!["DataA".into(), "DataB".into()],
            ])
            .unwrap();
        document
            .table_format_patch(table, &json!({"style":child}).to_string())
            .unwrap();
        document
            .format_style(
                &base,
                r##"{"background":"#C0D0E0","bold":true}"##,
                Some("first_row"),
            )
            .unwrap();
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(
            table_style_cell_fill(&layout, &document.location(table).unwrap().1, 0, 1),
            Some(parse_rgb("#C0D0E0").unwrap())
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened
                .table(StoryRef::Body, &document.location(table).unwrap().1)
                .unwrap()
                .props
                .style
                .as_deref(),
            Some(child.as_str())
        );
        assert!(crate::docx::render(&bytes, 0, 72.0, None)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|p| &p.blocks)
            .flat_map(|b| {
                std::iter::once(b.text.as_str()).chain(
                    b.table
                        .iter()
                        .flat_map(|t| &t.rows)
                        .flatten()
                        .map(String::as_str),
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        for label in ["LabelA", "LabelB", "DataA", "DataB"] {
            assert_eq!(text.matches(label).count(), 1, "{text:?}");
        }
        document.set_style_base(&child, None).unwrap();
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(
            table_style_cell_fill(&layout, &document.location(table).unwrap().1, 0, 1),
            None
        );
        document
            .table_format_patch(table, r#"{"style":null}"#)
            .unwrap();
        assert!(document.table(table).unwrap().props.style.is_none());
    }

    #[test]
    fn conditional_table_style_geometry_preserves_unregistered_nested_content_atomically() {
        let mut document = EditableDocx::new();
        let style = document.add_style("Narrow cells", "table", None).unwrap();
        let derived = document
            .add_style("Inherited cells", "table", Some(&style))
            .unwrap();
        let outer = document
            .add_table(vec![vec!["".into(), "Outer".into()]])
            .unwrap();
        document
            .table_format_patch(outer, &json!({"style":derived}).to_string())
            .unwrap();
        let inner = document
            .cell_add_table(outer, 0, 0, vec![vec!["Nested".into()]])
            .unwrap();
        let (_, path) = document.location(inner).unwrap();
        // Style validation cannot depend on whether a Python wrapper registered this child.
        document.locations.remove(&inner);
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        assert!(document
            .format_style(&style, r#"{"margins":[0,150,0,150]}"#, None)
            .is_err());
        assert_eq!(document.document().unwrap(), &before);
        assert_eq!(document.revision().unwrap(), revision);
        let impossible = document
            .add_style("Impossible margins", "table", None)
            .unwrap();
        document
            .format_style(&impossible, r#"{"margins":150}"#, None)
            .unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .set_style_base(&derived, Some(&impossible))
            .is_err());
        assert!(document
            .table_format_patch(outer, &json!({"style":impossible}).to_string())
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert!(document
            .document()
            .unwrap()
            .table(StoryRef::Body, &path)
            .is_some());
        document
            .format_style(&style, r#"{"margins":0}"#, None)
            .unwrap();
    }

    #[test]
    fn conditional_table_styles_validate_kind_regions_limits_and_atomic_patches() {
        let mut document = EditableDocx::new();
        let style = document.add_style("Bounds table", "table", None).unwrap();
        let child = document
            .add_style("Bounds child", "table", Some(&style))
            .unwrap();
        let table = document.add_table(vec![vec!["Data".into()]]).unwrap();
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        for (patch, region) in [
            (r#"{"space_after":3}"#, Some("first_row")),
            (r#"{"margins":3}"#, Some("odd_rows")),
            (r#"{"row_band_size":2}"#, Some("first_row")),
            (r#"{"row_band_size":0}"#, None),
            (r#"{"row_band_size":1001}"#, None),
            (r#"{"row_band_size":true}"#, None),
            (r#"{"row_band_size":1.5}"#, None),
            (r#"{"margins":-1}"#, None),
            (r#"{"margins":[0,0,0,1585]}"#, None),
            (r#"{"style":"Normal"}"#, None),
            (r#"{"bold":true}"#, Some("unknown")),
            (r#"{"vertical_alignment":"baseline"}"#, None),
        ] {
            assert!(
                document.format_style(&style, patch, region).is_err(),
                "{patch} {region:?}"
            );
            assert_eq!(document.document().unwrap(), &before);
        }
        assert!(document
            .format_style("Normal", r#"{"bold":true}"#, Some("first_row"))
            .is_err());
        assert!(document
            .style_formatting("Normal", false, Some("whole_table"))
            .is_err());
        assert!(document.set_style_base(&style, Some(&child)).is_err());
        assert!(document.set_style_base(&style, Some("Normal")).is_err());
        assert!(document
            .add_style("Invalid table", "table", Some("Normal"))
            .is_err());
        for patch in [
            r#"{"style":"Normal"}"#,
            r#"{"style":"missing"}"#,
            r#"{"look":{"header_row":null}}"#,
            r#"{"look":{"header_row":1}}"#,
            r#"{"look":{"unknown":true}}"#,
        ] {
            assert!(document.table_format_patch(table, patch).is_err());
        }
        assert!(document
            .format_style(&style, &" ".repeat(MAX_TABLE_PATCH_BYTES + 1), None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
    }

    #[test]
    fn tabs_roundtrip_inherit_clear_and_resolve_conditional_table_context() {
        let mut document = EditableDocx::new();
        let base = document
            .add_style("Tabs base", "paragraph", Some("Normal"))
            .unwrap();
        document
            .format_style(
                &base,
                r#"{"tabs":[{"position":144,"alignment":"right","leader":"dot"},{"position":72}]}"#,
                None,
            )
            .unwrap();
        let derived = document
            .add_style("Tabs child", "paragraph", Some(&base))
            .unwrap();
        document.format_style(&derived,r#"{"tabs":[{"position":72,"alignment":"clear"},{"position":216,"alignment":"center","leader":"hyphen"}]}"#,None).unwrap();
        let paragraph = document.add_paragraph("α\t😀\tValue").unwrap();
        document.set_paragraph_style(paragraph, &derived).unwrap();
        let direct: Value =
            serde_json::from_str(&document.paragraph_formatting(paragraph, false).unwrap())
                .unwrap();
        assert!(direct["tabs"].is_null());
        let effective: Value =
            serde_json::from_str(&document.paragraph_formatting(paragraph, true).unwrap()).unwrap();
        assert_eq!(effective["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(effective["tabs"][0]["position"], 144.0);
        assert_eq!(effective["tabs"][1]["leader"], "hyphen");
        document.format_paragraph(paragraph,r#"{"tabs":[{"position":144,"alignment":"clear"},{"position":-72.05,"leader":"middle_dot"}]}"#).unwrap();
        let before = document.paragraph(paragraph).unwrap().props.tabs.clone();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(
            reopened
                .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
                .unwrap()
                .props
                .tabs,
            before
        );
        assert_eq!(
            reopened
                .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
                .unwrap()
                .text,
            "α\t😀\tValue"
        );
        for reset in [r#"{"tabs":[]}"#, r#"{"tabs":null}"#] {
            document.format_paragraph(paragraph, reset).unwrap();
            assert!(document.paragraph(paragraph).unwrap().props.tabs.is_none());
        }
        let table_style = document.add_style("Tab table", "table", None).unwrap();
        document
            .format_style(
                &table_style,
                r#"{"tabs":[{"position":36}],"space_after":0,"alignment":"left"}"#,
                None,
            )
            .unwrap();
        document
            .format_style(&table_style, r#"{"alignment":"right"}"#, Some("first_row"))
            .unwrap();
        document
            .format_style(&table_style, r#"{"alignment":"center"}"#, Some("top_left"))
            .unwrap();
        let table = document.add_table(vec![vec!["A\tB".into()]]).unwrap();
        document
            .table_format_patch(table, &json!({"style":table_style}).to_string())
            .unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let effective: Value =
            serde_json::from_str(&document.paragraph_formatting(cell, true).unwrap()).unwrap();
        assert_eq!(effective["tabs"][0]["position"], 36.0);
        assert_eq!(effective["alignment"], "center");
        document
            .format_paragraph(cell, r#"{"alignment":"left"}"#)
            .unwrap();
        let effective: Value =
            serde_json::from_str(&document.paragraph_formatting(cell, true).unwrap()).unwrap();
        assert_eq!(effective["alignment"], "left");
        assert!(document
            .format_style(&table_style, r#"{"tabs":[]}"#, Some("first_row"))
            .is_err());
    }

    #[test]
    fn tabs_native_alignment_and_leaders_reach_render_and_pdf() {
        let mut document = EditableDocx::new();
        let mut ids = Vec::new();
        for (alignment, leader, label) in [
            ("left", "dot", "Left"),
            ("center", "hyphen", "Center"),
            ("right", "underscore", "Right"),
            ("left", "middle_dot", "Middle"),
        ] {
            let id = document.add_paragraph(&format!("\t{label}")).unwrap();
            document
                .format_paragraph(
                    id,
                    &json!({"tabs":[{"position":144,"alignment":alignment,"leader":leader}]})
                        .to_string(),
                )
                .unwrap();
            ids.push((id, alignment, leader));
        }
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        for (id, alignment, leader) in ids {
            let para = layout.pages[0]
                .items
                .iter()
                .find_map(|item| match item {
                    wordcraft_layout::Placed::Lines { path, para, .. }
                        if path == &document.location(id).unwrap().1 =>
                    {
                        Some(para)
                    }
                    _ => None,
                })
                .unwrap();
            let line = &para.lines[0];
            let text: Vec<_> = (line.c0..line.c1)
                .filter(|k| para.clusters[*k].kind == wordcraft_layout::para::ClKind::Text)
                .collect();
            let left = line.cl_left(text[0]).unwrap();
            let right = line.cl_right(*text.last().unwrap()).unwrap();
            let actual = match alignment {
                "left" => left,
                "center" => (left + right) / 2.0,
                _ => right,
            };
            assert!((actual - 144.0).abs() < 0.1, "{alignment}: {actual}");
            let wanted = TAB_LEADERS
                .iter()
                .find(|(name, _)| *name == leader)
                .unwrap()
                .1;
            assert!(line.leaders.iter().any(|(_, value)| *value == wanted));
        }
        let bytes = document.to_bytes().unwrap();
        assert!(crate::docx::render(&bytes, 0, 72.0, None)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|p| &p.blocks)
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for label in ["Left", "Center", "Right", "Middle"] {
            assert_eq!(text.matches(label).count(), 1, "{text:?}");
        }
    }

    #[test]
    fn tabs_decimal_upstream_finalization_gap_is_explicitly_rejected() {
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("\t12.34").unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .format_paragraph(id, r#"{"tabs":[{"position":144,"alignment":"decimal"}]}"#)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        // Reproduce the pinned upstream limitation without exposing it as working authoring:
        // resolve_tab finalizes Decimal as Right, moving the decimal point off its stop.
        let mut native = document.document().unwrap().clone();
        native
            .para_mut(StoryRef::Body, &document.location(id).unwrap().1)
            .unwrap()
            .props
            .tabs = Some(vec![TabStop {
            pos: 144.0,
            align: TabAlign::Decimal,
            leader: TabLeader::None,
        }]);
        let layout =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        let para = layout.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { path, para, .. }
                    if path == &document.location(id).unwrap().1 =>
                {
                    Some(para)
                }
                _ => None,
            })
            .unwrap();
        let line = &para.lines[0];
        let dot = para.clusters.iter().position(|c| c.dot).unwrap();
        assert!(
            (line.cl_left(dot).unwrap() - 144.0).abs() > 1.0,
            "review decimal-tab support when upstream fixes this placement"
        );
        assert!((line.end_x() - 144.0).abs() < 0.1);
    }

    #[test]
    fn tabs_bounds_effective_chains_and_invalid_patches_are_atomic() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Tabs").unwrap();
        let revision = document.revision().unwrap();
        let before = document.document().unwrap().clone();
        for tabs in [
            json!({}),
            json!([null]),
            json!([{}]),
            json!([{"position":true}]),
            json!([{"position":1584.05}]),
            json!([{"position":0.01}]),
            json!([{"position":72,"alignment":"bar"}]),
            json!([{"position":72,"alignment":"clear","leader":"dot"}]),
            json!([{"position":72,"leader":"heavy"}]),
            json!([{"position":72,"unknown":1}]),
            json!([{"position":72},{"position":72.45}]),
        ] {
            assert!(document
                .format_paragraph(
                    paragraph,
                    &json!({"tabs":tabs,"alignment":"right"}).to_string()
                )
                .is_err());
            assert_eq!(document.document().unwrap(), &before);
            assert_eq!(document.revision().unwrap(), revision);
        }
        let stops: Vec<_> = (0..65).map(|i| json!({"position":i*2})).collect();
        assert!(document
            .format_paragraph(paragraph, &json!({"tabs":stops}).to_string())
            .is_err());
        let base = document.add_style("Many tabs", "paragraph", None).unwrap();
        document
            .format_style(&base, &json!({"tabs":&stops[..64]}).to_string(), None)
            .unwrap();
        let child = document
            .add_style("One extra tab", "paragraph", Some(&base))
            .unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .format_style(&child, r#"{"tabs":[{"position":200}]}"#, None)
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document
            .format_style(
                &child,
                r#"{"tabs":[{"position":0,"alignment":"clear"},{"position":200}]}"#,
                None,
            )
            .unwrap();
        document.set_paragraph_style(paragraph, &child).unwrap();
        let before = document.document().unwrap().clone();
        assert!(document
            .format_paragraph(paragraph, r#"{"tabs":[{"position":300}]}"#)
            .is_err());
        assert_eq!(document.document().unwrap(), &before);
        let resolved: Value =
            serde_json::from_str(&document.style_formatting(&child, true, None).unwrap()).unwrap();
        assert_eq!(resolved["tabs"].as_array().unwrap().len(), 64);
        assert!(resolved["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["alignment"] != "clear"));
        document.set_paragraph_style(paragraph, "Normal").unwrap();
        document
            .format_paragraph(
                paragraph,
                r#"{"tabs":[{"position":0,"alignment":"clear"},{"position":0.5}]}"#,
            )
            .unwrap();
    }

    #[test]
    fn paragraph_breaks_native_columns_pages_lines_unicode_and_roundtrip() {
        let mut document = EditableDocx::new();
        document
            .set_section_props(0, r#"{"columns":[2,24,false]}"#)
            .unwrap();
        let paragraph = document.add_paragraph("LEFT α😀").unwrap();
        let (start, end, revision) = document.add_paragraph_break(paragraph, "column").unwrap();
        assert_eq!((start, end), (7, 8));
        assert_eq!(
            document
                .range_text(paragraph, start, end, revision)
                .unwrap(),
            "\u{e}"
        );
        document.add_run_patch(paragraph, "RIGHT", "{}").unwrap();
        document.add_paragraph_break(paragraph, "column").unwrap();
        document
            .add_run_patch(paragraph, "NEXT PAGE", "{}")
            .unwrap();
        document.add_paragraph_break(paragraph, "line").unwrap();
        document
            .add_run_patch(paragraph, "NEXT LINE", "{}")
            .unwrap();
        document.add_paragraph_break(paragraph, "page").unwrap();
        document
            .add_run_patch(paragraph, "AFTER PAGE", "{}")
            .unwrap();
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        assert_eq!(layout.pages.len(), 3);
        let source = &document.paragraph(paragraph).unwrap().text;
        let mut placements = BTreeMap::new();
        for (page_number, page) in layout.pages.iter().enumerate() {
            for item in &page.items {
                if let wordcraft_layout::Placed::Lines {
                    path,
                    para,
                    l0,
                    l1,
                    x,
                    y,
                    ..
                } = item
                {
                    if path == &document.location(paragraph).unwrap().1 {
                        for line in &para.lines[*l0..*l1] {
                            let text = source.get(line.start..line.stop).unwrap();
                            for label in ["LEFT", "RIGHT", "NEXT PAGE", "NEXT LINE", "AFTER PAGE"] {
                                if text.contains(label) {
                                    placements.insert(label, (page_number, *x, *y + line.top));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(placements["LEFT"].0, 0);
        assert_eq!(placements["RIGHT"].0, 0);
        assert!(placements["RIGHT"].1 > placements["LEFT"].1 + 200.0);
        assert_eq!(placements["NEXT PAGE"].0, 1);
        assert_eq!(placements["NEXT LINE"].0, 1);
        assert!(placements["NEXT LINE"].2 > placements["NEXT PAGE"].2);
        assert_eq!(placements["AFTER PAGE"].0, 2);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened
                .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
                .unwrap()
                .text,
            *source
        );
        for page in 0..3 {
            assert!(crate::docx::render(&bytes, page, 72.0, None)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n"));
        }
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 3);
        for (page, labels) in [
            (0, vec!["LEFT", "RIGHT"]),
            (1, vec!["NEXT PAGE", "NEXT LINE"]),
            (2, vec!["AFTER PAGE"]),
        ] {
            let text = extracted.pages[page]
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            for label in labels {
                assert_eq!(text.matches(label).count(), 1, "{text:?}");
            }
        }
    }

    #[test]
    fn paragraph_breaks_consecutive_pages_scope_budgets_and_anchors_are_atomic() {
        for kind in ["column", "page"] {
            let mut document = EditableDocx::new();
            let paragraph = document.add_paragraph("").unwrap();
            document.add_paragraph_break(paragraph, kind).unwrap();
            document.add_paragraph_break(paragraph, kind).unwrap();
            document
                .add_run_patch(paragraph, "Third page", "{}")
                .unwrap();
            let layout = wordcraft_layout::layout(
                document.document().unwrap(),
                &mut Default::default(),
                &Default::default(),
            );
            assert_eq!(layout.pages.len(), 3, "{kind}");
        }
        let mut document = EditableDocx::new();
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let revision = document.revision().unwrap();
        assert!(document.add_paragraph_break(cell, "column").is_err());
        assert!(document.add_paragraph_break(cell, "page").is_err());
        assert!(document.add_paragraph_break(cell, "unknown").is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.add_paragraph_break(cell, "line").unwrap();
        let paragraph = document.add_paragraph("Note α😀").unwrap();
        let revision = document.revision().unwrap();
        let note = document
            .add_note(paragraph, 0, 1, revision, "Note body", "footnote")
            .unwrap();
        let anchors = semantic_anchors(document.document().unwrap());
        document.add_paragraph_break(paragraph, "line").unwrap();
        assert_eq!(semantic_anchors(document.document().unwrap()), anchors);
        let note_paragraph = document.story_paragraph_ids(note).unwrap()[0];
        assert!(document
            .add_paragraph_break(note_paragraph, "page")
            .is_err());
        document
            .add_paragraph_break(note_paragraph, "line")
            .unwrap();
        assert!(document.add_run_patch(paragraph, "\u{e}", "{}").is_err());
        assert!(document.add_run_patch(paragraph, "\u{c}", "{}").is_err());
        document.text_bytes = MAX_DOCUMENT_TEXT_BYTES;
        let before = document.document().unwrap().clone();
        let revision = document.revision().unwrap();
        assert!(document.add_paragraph_break(paragraph, "line").is_err());
        assert!(document.add_page_break().is_err());
        assert_eq!(document.document().unwrap(), &before);
        assert_eq!(document.revision().unwrap(), revision);
    }

    #[test]
    fn table_metadata_budgets_and_border_reset_semantics_are_explicit() {
        let mut document = EditableDocx::new();
        let table = document.add_table(vec![vec!["Data".into()]]).unwrap();
        let text = "😀".repeat(10_000);
        document
            .table_format_patch(table, &json!({"title":text,"description":text}).to_string())
            .unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .table_format_patch(table, &json!({"title":"x".repeat(10_001)}).to_string())
            .is_err());
        assert!(document
            .table_format_patch(table, &" ".repeat(MAX_TABLE_PATCH_BYTES + 1))
            .is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.clear_table_borders(table).unwrap();
        assert!(document
            .table(table)
            .unwrap()
            .props
            .borders
            .unwrap()
            .top
            .is_some_and(|border| border.style == BorderStyle::None));
        document
            .table_format_patch(table, r#"{"borders":null,"title":null,"description":null}"#)
            .unwrap();
        assert_eq!(document.table(table).unwrap().props.borders, None);
        assert!(
            document.table_borders(table).unwrap().is_some(),
            "clearing direct borders restores TableGrid inheritance"
        );
        let mut candidate = document.document().unwrap().clone();
        document
            .table_mut(&mut candidate, table)
            .unwrap()
            .props
            .caption = Some("x".repeat(MAX_TABLE_METADATA_BYTES + 1));
        assert!(document.commit(candidate).is_err());
        assert_eq!(document.table(table).unwrap().props.caption, None);
    }
    #[test]
    fn direction_language_inheritance_roundtrip_and_native_bidi_geometry() {
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 300.0).unwrap();
        document.set_margins(30.0, 30.0, 30.0, 30.0).unwrap();
        document
            .format_defaults(r#"{"direction":"rtl","language":"fa-IR"}"#)
            .unwrap();
        assert!(
            document
                .document()
                .unwrap()
                .styles
                .resolve_para(&ParaProps::default())
                .bidi
        );
        assert_eq!(
            document
                .document()
                .unwrap()
                .styles
                .resolve_char(None, &CharProps::default())
                .lang
                .as_deref(),
            Some("fa-IR")
        );
        let style = document
            .add_style("Mixed scripts", "paragraph", Some("Normal"))
            .unwrap();
        document
            .format_style(
                &style,
                r##"{"direction":"rtl","language":"fa-IR","color":"#FF0000","font_size":18}"##,
                None,
            )
            .unwrap();
        let paragraph = document.add_paragraph("سلام Word 250").unwrap();
        document.set_paragraph_style(paragraph, &style).unwrap();
        let direct: Value =
            serde_json::from_str(&document.paragraph_formatting(paragraph, false).unwrap())
                .unwrap();
        let effective: Value =
            serde_json::from_str(&document.paragraph_formatting(paragraph, true).unwrap()).unwrap();
        assert!(direct["direction"].is_null());
        assert_eq!(effective["direction"], "rtl");
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        let para = layout.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { path, para, .. }
                    if path == &document.location(paragraph).unwrap().1 =>
                {
                    Some(para)
                }
                _ => None,
            })
            .unwrap();
        assert!(para.rp.bidi);
        assert!(!para.glyphs.is_empty());
        assert!(
            para.glyphs.iter().all(|glyph| glyph.gid != 0),
            "fixture must use real bundled glyphs, not missing-glyph boxes"
        );
        assert!(para
            .styles
            .iter()
            .all(|style| style.rc.lang.as_deref() == Some("fa-IR")));
        let line = &para.lines[0];
        let cluster_x = |offset| {
            let k = (line.c0..line.c1)
                .find(|&k| para.clusters[k].start == offset)
                .unwrap();
            line.cl_left(k).unwrap()
        };
        assert!(
            cluster_x(0) > cluster_x("سلا".len()),
            "Persian glyph clusters advance right to left"
        );
        assert!(
            cluster_x("سلام ".len()) < cluster_x("سلام Wor".len()),
            "Latin embedded run keeps left to right order"
        );
        assert!(
            cluster_x("سلام Word ".len()) < cluster_x("سلام Word 25".len()),
            "digits keep left to right order"
        );
        assert!(
            (layout
                .caret(&document.position(paragraph, 0).unwrap())
                .unwrap()
                .x
                - 270.0)
                .abs()
                < 1.0
        );
        let output = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&output).unwrap();
        let p = reopened
            .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
            .unwrap();
        assert_eq!(p.text, "سلام Word 250");
        assert!(reopened.styles.resolve_para(&p.props).bidi);
        assert_eq!(
            reopened
                .styles
                .resolve_char(p.props.style.as_deref(), p.props_of_char(0))
                .lang
                .as_deref(),
            Some("fa-IR")
        );
        let pdf = crate::docx::to_pdf(&output, None).unwrap();
        assert!(border_pixels(crate::docx::render(&output, 0, 144.0, None).unwrap(), 0) > 100);
        assert!(border_pixels(crate::pdf::render(&pdf, 0, 144.0, None).unwrap(), 0) > 100);
        document
            .format_paragraph(paragraph, r#"{"direction":"ltr"}"#)
            .unwrap();
        assert_eq!(
            document.paragraph(paragraph).unwrap().props.bidi,
            Some(false)
        );
        document
            .format_paragraph(paragraph, r#"{"direction":null}"#)
            .unwrap();
        assert_eq!(document.paragraph(paragraph).unwrap().props.bidi, None);
        document
            .format_defaults(r#"{"direction":null,"language":null}"#)
            .unwrap();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(reopened.styles.default_chr.lang, None);
        assert_eq!(reopened.styles.default_para.bidi, None);
        assert!(!reopened.styles.resolve_para(&ParaProps::default()).bidi);
    }

    #[test]
    fn direction_language_table_region_context_and_local_precedence() {
        let mut document = EditableDocx::new();
        let style = document.add_style("RTL table text", "table", None).unwrap();
        document
            .format_style(&style, r#"{"direction":"rtl","language":"ar-SA"}"#, None)
            .unwrap();
        document
            .format_style(&style, r#"{"language":"he-IL"}"#, Some("first_row"))
            .unwrap();
        let table = document
            .add_table(vec![
                vec!["First".into(), "Second".into()],
                vec!["Third".into(), "Fourth".into()],
            ])
            .unwrap();
        document
            .table_format_patch(
                table,
                &json!({"style":style,"look":{"header_row":true}}).to_string(),
            )
            .unwrap();
        let first = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let second = document.cell_paragraph_ids(table, 0, 1).unwrap()[0];
        document
            .format_paragraph(first, r#"{"direction":"ltr"}"#)
            .unwrap();
        let (_, _, revision) = document
            .add_run_patch(first, " Local", r#"{"language":"en-GB"}"#)
            .unwrap();
        let layout = wordcraft_layout::layout(
            document.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        let mut xs = [0.0; 2];
        for item in &layout.pages[0].items {
            if let wordcraft_layout::Placed::Cell {
                table: path,
                row: 0,
                cell,
                rect,
                ..
            } = item
            {
                if path == &document.location(table).unwrap().1 && *cell < 2 {
                    xs[*cell] = rect.x;
                }
            }
        }
        assert!(
            xs[0] < xs[1],
            "paragraph direction must not reverse table cell order"
        );
        for (id, rtl) in [(first, false), (second, true)] {
            let (_, path) = document.location(id).unwrap();
            let para = layout.pages[0]
                .items
                .iter()
                .find_map(|item| match item {
                    wordcraft_layout::Placed::Lines { path: p, para, .. } if p == &path => {
                        Some(para)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(para.rp.bidi, rtl);
            assert!(para
                .styles
                .iter()
                .any(|s| s.rc.lang.as_deref() == Some("he-IL")));
            if id == first {
                assert!(para
                    .styles
                    .iter()
                    .any(|s| s.rc.lang.as_deref() == Some("en-GB")));
            }
        }
        document
            .format_range_patch(first, 5, 11, revision, r#"{"language":null}"#)
            .unwrap();
        document
            .format_paragraph(first, r#"{"direction":null}"#)
            .unwrap();
        let snapshot: Value =
            serde_json::from_str(&document.paragraph_formatting(first, true).unwrap()).unwrap();
        assert_eq!(snapshot["direction"], "rtl");
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let reopened_layout =
            wordcraft_layout::layout(&reopened, &mut Default::default(), &Default::default());
        let (_, first_path) = document.location(first).unwrap();
        let cleared = reopened_layout.pages[0]
            .items
            .iter()
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { path, para, .. } if path == &first_path => {
                    Some(para)
                }
                _ => None,
            })
            .unwrap();
        assert!(cleared.rp.bidi);
        assert!(cleared
            .styles
            .iter()
            .all(|style| style.rc.lang.as_deref() == Some("he-IL")));
        let native_style = reopened.styles.get(&style).unwrap();
        assert_eq!(native_style.para.bidi, Some(true));
        assert_eq!(
            native_style
                .table_part(TablePart::FirstRow)
                .chr
                .lang
                .as_deref(),
            Some("he-IL")
        );
        let before = document.to_bytes().unwrap();
        assert!(document
            .format_style(&style, r#"{"direction":"ltr"}"#, Some("first_row"))
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
    }

    #[test]
    fn direction_language_bounds_reset_and_failed_patches_are_atomic() {
        let mut document = EditableDocx::new();
        let paragraph = document.add_paragraph("Preserve 😀 e\u{301}").unwrap();
        let tag = "a".repeat(63);
        let (_, _, revision) = document
            .add_run_patch(paragraph, "tail", &json!({"language":tag}).to_string())
            .unwrap();
        let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(
            reopened
                .para(StoryRef::Body, &document.location(paragraph).unwrap().1)
                .unwrap()
                .runs
                .last()
                .unwrap()
                .props
                .lang
                .as_deref(),
            Some(tag.as_str())
        );
        let before = document.to_bytes().unwrap();
        for value in [
            json!("a".repeat(64)),
            json!(""),
            json!(" en"),
            json!("en_Us"),
            json!("en--US"),
            json!("-en"),
            json!("en-"),
            json!("日本"),
            json!(false),
            json!(123),
        ] {
            assert!(document
                .add_run_patch(
                    paragraph,
                    "changed",
                    &json!({"language":value,"bold":true}).to_string()
                )
                .is_err());
            assert!(document
                .format_defaults(&json!({"language":value}).to_string())
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
            assert_eq!(document.revision().unwrap(), revision);
        }
        for value in [json!("auto"), json!("RTL"), json!(true), json!(1)] {
            assert!(document
                .format_paragraph(
                    paragraph,
                    &json!({"direction":value,"alignment":"right"}).to_string()
                )
                .is_err());
            assert_eq!(document.to_bytes().unwrap(), before);
        }
        let mut copied = document.copy().unwrap();
        copied
            .format_paragraph(paragraph, r#"{"direction":"rtl"}"#)
            .unwrap();
        assert_eq!(document.paragraph(paragraph).unwrap().props.bidi, None);
        document.close();
        assert!(document
            .format_paragraph(paragraph, r#"{"direction":"rtl"}"#)
            .is_err());
        assert!(copied.to_bytes().is_ok());
    }
    fn raw_bookmark(
        document: &mut Document,
        story: StoryRef,
        path: Path,
        name: &str,
        start: usize,
        end: usize,
    ) {
        document
            .insert_object(
                &Pos::new(story, path.clone(), end),
                InlineObject::BookmarkEnd { name: name.into() },
                &CharProps::default(),
            )
            .unwrap();
        document
            .insert_object(
                &Pos::new(story, path, start),
                InlineObject::BookmarkStart { name: name.into() },
                &CharProps::default(),
            )
            .unwrap();
    }

    #[test]
    fn bookmarks_endpoint_profile_preserves_native_shaping() {
        for (text, direction) in [
            ("office", "ltr"),
            ("سلام", "rtl"),
            ("Before Target 😀 after", "ltr"),
        ] {
            let mut document = EditableDocx::new();
            let p = document.add_paragraph(text).unwrap();
            document
                .format_paragraph(p, &format!(r#"{{"direction":"{}"}}"#, direction))
                .unwrap();
            document
                .format_range_patch(
                    p,
                    0,
                    text.chars().count(),
                    document.revision().unwrap(),
                    r#"{"font_size":36}"#,
                )
                .unwrap();
            let before = document.to_bytes().unwrap();
            let png = crate::docx::render(&before, 0, 144.0, None).unwrap();
            let revision = document.revision().unwrap();
            assert!(document.set_bookmark("Unsafe", p, 1, 3, revision).is_err());
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), before);
            if direction == "rtl" {
                let mut raw = document.document().unwrap().clone();
                raw_bookmark(
                    &mut raw,
                    StoryRef::Body,
                    document.location(p).unwrap().1,
                    "Unsafe",
                    2,
                    6,
                );
                let raw_png =
                    crate::docx::render(&wordcraft_docx::write(&raw).unwrap(), 0, 144.0, None)
                        .unwrap();
                assert_ne!(
                    raw_png, png,
                    "pinned native interior markers break Arabic joining"
                );
                assert!(validate_bookmark_links(&raw).is_err());
            }
            for (start, end) in [
                (0, text.chars().count()),
                (0, 0),
                (text.chars().count(), text.chars().count()),
            ] {
                document
                    .set_bookmark("Safe", p, start, end, document.revision().unwrap())
                    .unwrap();
                assert_eq!(
                    crate::docx::render(&document.to_bytes().unwrap(), 0, 144.0, None).unwrap(),
                    png
                );
                document.delete_bookmark("Safe").unwrap();
                assert_eq!(
                    crate::docx::render(&document.to_bytes().unwrap(), 0, 144.0, None).unwrap(),
                    png
                );
            }
        }
    }

    #[test]
    fn bookmarks_owned_ranges_move_delete_copy_and_semantic_protection() {
        let mut document = EditableDocx::new();
        let p = document.add_paragraph("Before Target 😀 after").unwrap();
        let link = document.add_paragraph("JumpLabel").unwrap();
        let before_png = crate::docx::render(&document.to_bytes().unwrap(), 0, 72.0, None).unwrap();
        document
            .set_bookmark("Target", p, 0, 21, document.revision().unwrap())
            .unwrap();
        let (id, start, end, revision) = document.bookmark_range("Target").unwrap();
        assert_eq!(
            document.range_text(id, start, end, revision).unwrap(),
            "Before Target 😀 after"
        );
        assert_eq!(
            before_png,
            crate::docx::render(&document.to_bytes().unwrap(), 0, 72.0, None).unwrap()
        );
        document
            .set_range_link(link, 0, 9, revision, Some("#Target"))
            .unwrap();
        let before = document.to_bytes().unwrap();
        assert!(document.delete_bookmark("Target").is_err());
        assert!(document.set_paragraph_text(p, "lost").is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
        let mut copied = document.copy().unwrap();
        copied
            .set_bookmark("Target", p, 0, 0, copied.revision().unwrap())
            .unwrap();
        let (_, a, b, r) = copied.bookmark_range("Target").unwrap();
        assert_eq!(copied.range_text(p, a, b, r).unwrap(), "");
        assert!(document.range_text(id, start, end, revision).is_err());
        assert_eq!(document.bookmark_names().unwrap(), vec!["Target"]);
        let reopened = wordcraft_docx::read(&copied.to_bytes().unwrap()).unwrap();
        assert_eq!(reopened.bookmarks().len(), 1);
        copied.set_range_link(link, 0, 9, r, None).unwrap();
        copied.delete_bookmark("Target").unwrap();
        assert_eq!(copied.paragraph_text(p).unwrap(), "Before Target 😀 after");
        assert!(copied.bookmark_names().unwrap().is_empty());
        copied
            .set_bookmark("Point", p, 0, 0, copied.revision().unwrap())
            .unwrap();
        let (_, a, b, r) = copied.bookmark_range("Point").unwrap();
        assert_eq!(a, b);
        assert_eq!(copied.range_text(p, a, b, r).unwrap(), "");
        document.close();
        assert!(document.bookmark_range("Target").is_err());
        assert!(copied.to_bytes().is_ok());
    }

    #[test]
    fn bookmarks_nested_target_grid_and_late_paragraph_pdf_destination() {
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 300.0).unwrap();
        document.set_margins(30.0, 30.0, 30.0, 30.0).unwrap();
        let link = document.add_paragraph("JumpLabel").unwrap();
        let p = document.add_paragraph(&"Before\n".repeat(45)).unwrap();
        let (_, to, _) = document
            .add_run_patch(p, "Destination", r#"{"bold":true}"#)
            .unwrap();
        document
            .set_bookmark("Late", p, to, to, document.revision().unwrap())
            .unwrap();
        document
            .set_range_link(link, 0, 9, document.revision().unwrap(), Some("#Late"))
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let native = wordcraft_docx::read(&bytes).unwrap();
        let layout =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        let caret = layout.caret(&native.bookmarks()[0].1).unwrap();
        assert!(caret.page > 0);
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(
            extracted.pages[0].links[0].metadata["target_page"],
            caret.page
        );
        assert_eq!(extracted.pages[0].links[0].text, "JumpLabel");
        let table = document
            .add_table(vec![vec!["Cell target".into()], vec!["Keep".into()]])
            .unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        document
            .set_bookmark("Cell", cell, 0, 11, document.revision().unwrap())
            .unwrap();
        document.insert_row(table, 0).unwrap();
        assert!(document.paragraph_text(cell).is_err());
        let (fresh, a, b, r) = document.bookmark_range("Cell").unwrap();
        assert_ne!(fresh, cell);
        assert_eq!(document.range_text(fresh, a, b, r).unwrap(), "Cell target");
        document
            .set_range_link(link, 0, 9, r, Some("#Cell"))
            .unwrap();
        assert!(document.delete_row(table, 1).is_err());
        for kind in ["header", "footer"] {
            let story = document.ensure_story(kind, "default", 0).unwrap();
            let label = document.story_add_paragraph(story, kind).unwrap();
            document
                .set_range_link(
                    label,
                    0,
                    kind.len(),
                    document.revision().unwrap(),
                    Some("#Cell"),
                )
                .unwrap();
        }
        let native = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
        let placed =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        let options = wordcraft_layout::display::DisplayOptions {
            marks: false,
            dim_header: false,
            dim_body: false,
            markup: false,
            placeholders: false,
            ..Default::default()
        };
        let batches: usize = placed
            .pages
            .iter()
            .map(|page| {
                validate_bookmark_draws(
                    &wordcraft_layout::display::page_display(&native, page, &options),
                    &BTreeMap::from([("Late".into(), 0), ("Cell".into(), 0)]),
                    &mut 0,
                )
                .unwrap()
            })
            .sum();
        let pdf = crate::docx::to_pdf(&document.to_bytes().unwrap(), None).unwrap();
        let annotation_count = pdfcraft_render::inspect(Arc::new(pdf), None)
            .unwrap()
            .links
            .len();
        assert!(annotation_count > 2);
        assert!(
            batches >= annotation_count,
            "native body/cell/repeated story draws upper-bound actual annotation count"
        );
        let output = crate::pdf::extract(
            &crate::docx::to_pdf(&document.to_bytes().unwrap(), None).unwrap(),
            None,
        )
        .unwrap();
        let jump = output.pages[0]
            .links
            .iter()
            .find(|link| link.text == "JumpLabel")
            .unwrap();
        assert!(jump.metadata["target_page"].as_u64().unwrap() > 0);
    }

    #[test]
    fn bookmarks_native_missing_header_and_wrapped_annotation_limits() {
        for header in [false, true] {
            let mut document = EditableDocx::new();
            let p = document.add_paragraph("JumpLabel").unwrap();
            let mut raw = document.document().unwrap().clone();
            raw.para_mut(StoryRef::Body, &document.location(p).unwrap().1)
                .unwrap()
                .runs[0]
                .props
                .link = Some("#Absent".into());
            if header {
                let story = document.ensure_story("header", "default", 0).unwrap();
                document.story_add_paragraph(story, "Destination").unwrap();
                raw = document.document().unwrap().clone();
                raw.para_mut(StoryRef::Body, &document.location(p).unwrap().1)
                    .unwrap()
                    .runs[0]
                    .props
                    .link = Some("#Absent".into());
                raw_bookmark(
                    &mut raw,
                    StoryRef::Part(story),
                    Path::top(0),
                    "Absent",
                    0,
                    0,
                );
            }
            let bytes = wordcraft_docx::write(&raw).unwrap();
            let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
            assert!(
                crate::pdf::extract(&pdf, None).unwrap().pages[0]
                    .links
                    .is_empty(),
                "pinned native drops missing/auxiliary targets"
            );
            assert!(validate_bookmark_links(&raw).is_err());
        }
        // Turned/clip link geometry is audited separately from the conservative
        // display count. Keep a direct-native repro rather than compensating it.
        for turned in [false, true] {
            let mut document = EditableDocx::new();
            let target = document.add_paragraph("Target").unwrap();
            let table = document
                .add_table(vec![vec![if turned {
                    "Vertical link".into()
                } else {
                    "Clipped\nHidden link".into()
                }]])
                .unwrap();
            if turned {
                document
                    .cell_format_patch(table, 0, 0, r#"{"text_direction":"down"}"#)
                    .unwrap();
            } else {
                document
                    .set_row_height(table, 0, Some(20.0), "exact")
                    .unwrap();
            }
            let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
            let mut raw = document.document().unwrap().clone();
            raw_bookmark(
                &mut raw,
                StoryRef::Body,
                document.location(target).unwrap().1,
                "Target",
                0,
                0,
            );
            for run in &mut document.paragraph_mut(&mut raw, cell).unwrap().runs {
                run.props.link = Some("#Target".into());
            }
            let layout =
                wordcraft_layout::layout(&raw, &mut Default::default(), &Default::default());
            let bounds = layout.pages[0]
                .items
                .iter()
                .find_map(|item| match item {
                    wordcraft_layout::Placed::Cell {
                        table: path, rect, ..
                    } if path == &document.location(table).unwrap().1 => Some(*rect),
                    _ => None,
                })
                .unwrap();
            let pdf = crate::docx::to_pdf(&wordcraft_docx::write(&raw).unwrap(), None).unwrap();
            let extracted = crate::pdf::extract(&pdf, None).unwrap();
            if turned {
                assert!(extracted.pages[0]
                    .blocks
                    .iter()
                    .any(|block| block.text.contains("Vertical link")));
                assert!(
                    extracted.pages[0].links.is_empty(),
                    "native vertical text drops hyperlink metadata while preserving visible label"
                );
            } else {
                assert!(!extracted.pages[0].links.is_empty());
                assert!(
                    extracted.pages[0].links.iter().any(|link| link
                        .bbox
                        .is_some_and(|b| b[3] > f64::from(bounds.bottom()) + 0.1)),
                    "native annotation exceeds the cell clip: {:?} {:?}",
                    bounds,
                    extracted.pages[0].links
                );
            }
            assert!(validate_unsplittable_page_bounds(&raw).is_err());
        }
    }

    #[test]
    fn bookmarks_native_link_batch_cap_is_a_conservative_export_bound() {
        use wordcraft_layout::display::{page_display, DisplayOptions, Draw};
        let mut document = EditableDocx::new();
        document.set_page_size(1500.0, 1500.0).unwrap();
        document.set_margins(10.0, 10.0, 10.0, 10.0).unwrap();
        let target = document.add_paragraph("Target").unwrap();
        let mut raw = document.document().unwrap().clone();
        raw_bookmark(
            &mut raw,
            StoryRef::Body,
            document.location(target).unwrap().1,
            "A",
            0,
            0,
        );
        raw_bookmark(
            &mut raw,
            StoryRef::Body,
            document.location(target).unwrap().1,
            "B",
            0,
            0,
        );
        let mut paragraph = Paragraph::new();
        for index in 0..10001 {
            paragraph
                .insert_text(
                    paragraph.text.len(),
                    "x",
                    &CharProps {
                        size: Some(1.0),
                        link: Some(if index % 2 == 0 { "#A" } else { "#B" }.into()),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        raw.body.push(Arc::new(Block::Para(paragraph)));
        let layout = wordcraft_layout::layout(&raw, &mut Default::default(), &Default::default());
        assert_eq!(layout.pages.len(), 1);
        let options = DisplayOptions {
            marks: false,
            dim_header: false,
            dim_body: false,
            markup: false,
            placeholders: false,
            ..Default::default()
        };
        let draws = page_display(&raw, &layout.pages[0], &options);
        let count = draws
            .iter()
            .filter(
                |draw| matches!(draw,Draw::Glyphs {link:Some(_),glyphs,..} if !glyphs.is_empty()),
            )
            .count();
        assert_eq!(count, 10001);
        assert!(validate_bookmark_draws(
            &draws,
            &BTreeMap::from([("A".into(), 0), ("B".into(), 0)]),
            &mut 0
        )
        .is_err());
        let pdf = crate::docx::to_pdf(&wordcraft_docx::write(&raw).unwrap(), None).unwrap();
        let extracted = pdfcraft_render::inspect(Arc::new(pdf), None).unwrap();
        assert_eq!(
            extracted.links.len(),
            10000,
            "direct native export silently omits the final link batch"
        );
        let linked = draws
            .iter()
            .filter(|draw| matches!(draw, Draw::Glyphs { link: Some(_), .. }))
            .take(10000)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            validate_bookmark_draws(
                &linked,
                &BTreeMap::from([("A".into(), 0), ("B".into(), 0)]),
                &mut 0
            )
            .unwrap(),
            10000
        );
        let glyph = linked[0].clone();
        for wrapped in [
            Draw::Clip {
                rect: layout.pages[0].body,
                items: vec![glyph.clone()],
            },
            Draw::Turned {
                x: 0.0,
                y: 0.0,
                turn: TextDirection::Down,
                items: vec![glyph.clone()],
            },
            Draw::Rotated {
                cx: 0.0,
                cy: 0.0,
                spin: Default::default(),
                items: vec![glyph.clone()],
            },
            Draw::Figure {
                alt: String::new(),
                kind: wordcraft_doc::graphic::GraphicKind::Chart,
                draws: vec![glyph],
            },
        ] {
            assert!(
                validate_bookmark_draws(&[wrapped], &BTreeMap::new(), &mut 0).is_err(),
                "unaudited wrapper contexts cannot bypass navigation validation"
            );
        }

        assert!(validate_unsplittable_page_bounds(&raw).is_err());
    }
    #[test]
    fn bookmarks_lookup_work_bounds_native_carets_and_repeated_export_targets() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 300.0).unwrap();
        document.set_margins(30.0, 30.0, 30.0, 30.0).unwrap();
        let text = format!("{}Target", "Before\n".repeat(700));
        let target = document.add_paragraph(&text).unwrap();
        document
            .set_bookmark(
                "Late",
                target,
                text.len(),
                text.len(),
                document.revision().unwrap(),
            )
            .unwrap();
        let label = document.add_paragraph(&"Jump\n".repeat(30)).unwrap();
        let native = document.document().unwrap();
        let layout = wordcraft_layout::layout(native, &mut Default::default(), &Default::default());
        let path = &validate_bookmark_links(native).unwrap()["Late"].path;
        let cost = bookmark_lookup_work(&layout, path, &mut 0).unwrap();
        assert!(
            cost > 50_000,
            "split paragraph carets repeatedly scan the full native line list"
        );
        let mut exact = MAX_BOOKMARK_WORK - cost;
        assert_eq!(
            bookmark_lookup_work(&layout, path, &mut exact).unwrap(),
            cost
        );
        assert_eq!(exact, MAX_BOOKMARK_WORK);
        let mut over = MAX_BOOKMARK_WORK - cost + 1;
        assert!(bookmark_lookup_work(&layout, path, &mut over).is_err());
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        let error = document
            .set_range_link(label, 0, 150, revision, Some("#Late"))
            .unwrap_err();
        assert!(error.to_string().contains("lookup and geometry scans"));
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);
        document
            .set_range_link(label, 0, 4, revision, Some("#Late"))
            .unwrap();
        assert!(document.to_bytes().is_ok());

        let mut plain = EditableDocx::new();
        plain.add_paragraph("Unlinked").unwrap();
        let native = plain.document().unwrap();
        let mut layout =
            wordcraft_layout::layout(native, &mut Default::default(), &Default::default());
        let page = &mut layout.pages[0];
        for item in &mut page.items {
            if let wordcraft_layout::Placed::Lines { turn, .. } = item {
                *turn = TextDirection::Down;
            }
        }
        let mut cost = 0;
        validate_bookmark_source_links(native, page, &mut cost).unwrap();
        assert!(
            cost > page.items.len(),
            "turned placements must charge their source run lists"
        );
        let mut exact = MAX_BOOKMARK_WORK - cost;
        validate_bookmark_source_links(native, page, &mut exact).unwrap();
        assert_eq!(exact, MAX_BOOKMARK_WORK);
        let mut over = MAX_BOOKMARK_WORK - cost + 1;
        assert!(validate_bookmark_source_links(native, page, &mut over).is_err());
    }

    #[test]
    fn bookmarks_pair_budget_malformed_state_and_limited_deletion_permission() {
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("Keep").unwrap();
        let mut raw = document.document().unwrap().clone();
        for index in 0..MAX_BOOKMARKS {
            raw_bookmark(
                &mut raw,
                StoryRef::Body,
                document.location(id).unwrap().1,
                &format!("Mark{index}"),
                0,
                0,
            );
        }
        document.commit(raw).unwrap();
        document.text_bytes = 4 + MAX_BOOKMARKS * 6;
        assert_eq!(document.bookmark_names().unwrap().len(), MAX_BOOKMARKS);
        let before = wordcraft_docx::write(document.document().unwrap()).unwrap();
        let revision = document.revision().unwrap();
        assert!(document
            .set_bookmark("Overflow", id, 0, 0, revision)
            .is_err());
        assert_eq!(
            wordcraft_docx::write(document.document().unwrap()).unwrap(),
            before
        );
        assert_eq!(document.revision().unwrap(), revision);
        for malformed in [0, 1, 2] {
            let mut raw = document.document().unwrap().clone();
            match malformed {
                0 => {
                    let span = validate_bookmark_links(&raw)
                        .unwrap()
                        .remove("Mark0")
                        .unwrap();
                    raw.delete_range(
                        &document.position(id, span.end).unwrap(),
                        &document.position(id, span.end + 3).unwrap(),
                    )
                    .unwrap();
                }
                1 => raw_bookmark(
                    &mut raw,
                    StoryRef::Body,
                    document.location(id).unwrap().1,
                    "Mark0",
                    0,
                    0,
                ),
                _ => {
                    raw.delete_range(
                        &document.position(id, 0).unwrap(),
                        &document.position(id, 3).unwrap(),
                    )
                    .unwrap();
                }
            }
            assert!(document.commit(raw).is_err());
            assert_eq!(document.revision().unwrap(), revision);
        }
        // A private deletion transaction cannot waive unrelated marker loss.
        let mut raw = document.document().unwrap().clone();
        for name in ["Mark0", "Mark1"] {
            let span = validate_bookmark_links(&raw).unwrap().remove(name).unwrap();
            for offset in [span.end, span.start] {
                raw.delete_range(
                    &Pos::new(StoryRef::Body, span.path.clone(), offset),
                    &Pos::new(StoryRef::Body, span.path.clone(), offset + 3),
                )
                .unwrap();
            }
        }
        assert!(document.commit_checked(raw, Some("Mark0")).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        document.delete_bookmark("Mark0").unwrap();
        assert_eq!(document.bookmark_names().unwrap().len(), MAX_BOOKMARKS - 1);
    }
    #[test]
    fn story_clear_noops_authored_empty_and_alias_guards() {
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Body").unwrap();
        let revision = document.revision().unwrap();
        let bytes = document.to_bytes().unwrap();
        assert!(!document.clear_story("header", "default", 0).unwrap());
        assert_eq!(document.to_bytes().unwrap(), bytes);
        assert_eq!(document.revision().unwrap(), revision);
        let story = document.ensure_story("header", "default", 0).unwrap();
        let revision = document.revision().unwrap();
        assert!(!document.clear_story("header", "default", 0).unwrap());
        assert_eq!(document.revision().unwrap(), revision);
        let paragraph = document.story_add_paragraph(story, "").unwrap();
        let old_next = document.next_block_id;
        let revision = document.revision().unwrap();
        assert!(document.clear_story("header", "default", 0).unwrap());
        assert_eq!(document.revision().unwrap(), revision + 1);
        assert!(document.paragraph_text(paragraph).is_err());
        assert_eq!(document.next_block_id, old_next);
        assert_eq!(
            document.story_id("header", "default", 0).unwrap(),
            Some(story)
        );
        let revision = document.revision().unwrap();
        let bytes = document.to_bytes().unwrap();
        assert!(!document.clear_story("header", "default", 0).unwrap());
        assert_eq!(document.to_bytes().unwrap(), bytes);
        assert_eq!(document.revision().unwrap(), revision);
        let fresh = document.story_add_paragraph(story, "Fresh").unwrap();
        assert!(fresh > paragraph);
        assert_eq!(document.document().unwrap().parts[&story].blocks.len(), 1);
        assert_eq!(document.paragraph_text(body).unwrap(), "Body");
        document
            .document
            .as_mut()
            .unwrap()
            .last_section
            .headers
            .first = Some(story);
        let bytes = wordcraft_docx::write(document.document().unwrap()).unwrap();
        let revision = document.revision().unwrap();
        assert!(document.clear_story("header", "default", 0).is_err());
        assert_eq!(
            wordcraft_docx::write(document.document().unwrap()).unwrap(),
            bytes
        );
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.paragraph_text(fresh).unwrap(), "Fresh");
        for (kind, variant, section) in [
            ("note", "default", 0),
            ("header", "unknown", 0),
            ("header", "default", 1),
        ] {
            assert!(document.clear_story(kind, variant, section).is_err());
        }
        document.close();
        assert!(document.clear_story("header", "default", 0).is_err());
        let mut inactive = EditableDocx::new();
        let first = inactive.ensure_story("header", "first", 0).unwrap();
        inactive.story_add_paragraph(first, "Hidden first").unwrap();
        let even = inactive.ensure_story("footer", "even", 0).unwrap();
        inactive.story_add_paragraph(even, "Hidden even").unwrap();
        inactive.document.as_mut().unwrap().last_section.title_page = false;
        inactive
            .document
            .as_mut()
            .unwrap()
            .settings
            .even_odd_headers = false;
        assert!(inactive.clear_story("header", "first", 0).unwrap());
        assert!(inactive.clear_story("footer", "even", 0).unwrap());
        assert!(!inactive.document().unwrap().last_section.title_page);
        assert!(!inactive.document().unwrap().settings.even_odd_headers);
        assert_eq!(
            inactive.story_id("header", "first", 0).unwrap(),
            Some(first)
        );
        assert_eq!(inactive.story_id("footer", "even", 0).unwrap(), Some(even));
    }

    #[test]
    fn story_clear_rich_content_copies_and_exact_resource_reclamation() {
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Body").unwrap();
        let png = media_png(2, 2, [255, 0, 0, 255]);
        let unique = media_png(3, 2, [0, 0, 255, 255]);
        document.set_background_image(Some(&png)).unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let paragraph = document.story_add_paragraph(header, "Page ").unwrap();
        document.add_paragraph_field(paragraph, "page").unwrap();
        let table = document
            .story_add_table(header, vec![vec!["Cell".into()]])
            .unwrap();
        let child = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        document.add_paragraph_field(child, "page_count").unwrap();
        let picture = document
            .story_add_image(header, &png, Some(12.0), None, "shared")
            .unwrap();
        document
            .cell_add_image(table, 0, 0, &unique, Some(12.0), None, "unique")
            .unwrap();
        assert_eq!(document.text_bytes, "BodyPage Cell".len() + 6);
        let section = document.add_section("next_page").unwrap();
        let copied = document
            .story_id("header", "default", section)
            .unwrap()
            .unwrap();
        assert_ne!(copied, header);
        assert_eq!(
            document.text_bytes,
            "Body".len() + 2 * ("Page Cell".len() + 6)
        );
        let copied_table = document.story_table_ids(copied).unwrap()[0];
        let copied_child = document.cell_paragraph_ids(copied_table, 0, 0).unwrap()[0];
        let props_before = document.section_props(0).unwrap();
        let settings_before = document.document().unwrap().settings.clone();
        assert!(document.clear_story("header", "default", 0).unwrap());
        assert_eq!(document.text_bytes, "BodyPage Cell".len() + 6);
        assert_eq!(document.cell_count, 1);
        assert_eq!(document.document().unwrap().media.len(), 2);
        for id in [paragraph, child] {
            assert!(document.paragraph_text(id).is_err());
        }
        assert!(document.image_size(picture).is_err());
        assert!(document.table_rows(table).is_err());
        assert!(document.table_generation(table).is_err());
        assert!(document
            .paragraph_text(copied_child)
            .unwrap()
            .starts_with("Cell"));
        assert_eq!(document.paragraph_text(body).unwrap(), "Body");
        assert_eq!(document.section_props(0).unwrap(), props_before);
        assert_eq!(document.document().unwrap().settings, settings_before);
        assert!(document.clear_story("header", "default", section).unwrap());
        assert_eq!(document.text_bytes, "Body".len());
        assert_eq!(document.cell_count, 0);
        assert_eq!(document.document().unwrap().media.len(), 1);
        document.set_background_image(None).unwrap();
        assert!(document.document().unwrap().media.is_empty());
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let first = reopened.sections()[0].1.headers.default.unwrap();
        let second = reopened.last_section.headers.default.unwrap();
        assert_ne!(first, second);
        assert_eq!(reopened.parts[&first].blocks.len(), 1);
        assert_eq!(reopened.parts[&second].blocks.len(), 1);
    }

    #[test]
    fn story_clear_protected_anchors_rollback_and_charged_field_boundaries() {
        let mut document = EditableDocx::new();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let paragraph = document.story_add_paragraph(header, "").unwrap();
        document.add_paragraph_field(paragraph, "page").unwrap();
        assert_eq!(document.text_bytes, 3);
        document.text_bytes = MAX_DOCUMENT_TEXT_BYTES - 2;
        let revision = document.revision().unwrap();
        let bytes = document.to_bytes().unwrap();
        assert!(
            document.add_section("next_page").is_err(),
            "a copied field consumes three bytes"
        );
        assert_eq!(document.to_bytes().unwrap(), bytes);
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.text_bytes, MAX_DOCUMENT_TEXT_BYTES - 2);
        assert!(document.clear_story("header", "default", 0).unwrap());
        assert_eq!(document.text_bytes, MAX_DOCUMENT_TEXT_BYTES - 5);
        let fresh = document.story_add_paragraph(header, "abcde").unwrap();
        assert_eq!(document.text_bytes, MAX_DOCUMENT_TEXT_BYTES);
        assert!(document.add_run_patch(fresh, "x", "{}").is_err());

        let mut document = EditableDocx::new();
        let table = document
            .add_table(vec![vec!["".into()], vec!["Keep".into()]])
            .unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        document.add_paragraph_field(cell, "page").unwrap();
        assert_eq!(document.text_bytes, 7);
        document.delete_row(table, 0).unwrap();
        assert_eq!(document.text_bytes, 4);
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        document.add_paragraph_field(cell, "page").unwrap();
        document.set_cell_text(table, 0, 0, "New").unwrap();
        assert_eq!(document.text_bytes, 3);

        let header = document.ensure_story("header", "default", 0).unwrap();
        let paragraph = document.story_add_paragraph(header, "Protected").unwrap();
        let path = document.location(paragraph).unwrap().1;
        document
            .document
            .as_mut()
            .unwrap()
            .insert_object(
                &Pos::new(StoryRef::Part(header), path, 0),
                InlineObject::NoteRef {
                    id: 7,
                    kind: NoteKind::Footnote,
                    custom: String::new(),
                },
                &CharProps::default(),
            )
            .unwrap();
        document.text_bytes += 3;
        let before = wordcraft_docx::write(document.document().unwrap()).unwrap();
        let revision = document.revision().unwrap();
        let locations = document.locations.clone();
        let count = document.text_bytes;
        assert!(document.clear_story("header", "default", 0).is_err());
        assert_eq!(
            wordcraft_docx::write(document.document().unwrap()).unwrap(),
            before
        );
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.locations, locations);
        assert_eq!(document.text_bytes, count);
    }
    fn decimal_outline_labels(document: &Document) -> Vec<String> {
        let layout =
            wordcraft_layout::layout(document, &mut Default::default(), &Default::default());
        layout
            .pages
            .iter()
            .flat_map(|page| &page.items)
            .filter_map(|item| {
                let wordcraft_layout::Placed::Lines { para, l0, l1, .. } = item else {
                    return None;
                };
                let line = para.label_line();
                (line >= *l0 && line < *l1)
                    .then(|| para.label.as_ref().map(|label| label.text.clone()))
                    .flatten()
            })
            .collect()
    }

    #[test]
    fn decimal_outline_owned_identity_start_restart_copy_and_rtl() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let mut ids = Vec::new();
        for (text, kind, level, start, restart) in [
            ("Ordinary", "numbered", 0, None, false),
            ("Root", "decimal_outline", 0, Some(ListStart(7)), false),
            ("Child", "decimal_outline", 1, None, false),
            ("Other ordinary", "numbered", 0, None, false),
            ("Next", "decimal_outline", 0, None, false),
            ("Restart", "decimal_outline", 0, None, true),
            ("Restart child", "decimal_outline", 1, None, false),
            ("", "decimal_outline", 2, None, false),
        ] {
            let id = document.add_paragraph(text).unwrap();
            document
                .set_paragraph_list(id, Some(kind), level, restart, start)
                .unwrap();
            assert_eq!(document.paragraph_list(id).unwrap(), Some((kind, level)));
            ids.push(id);
        }
        let expected = ["1.", "7.", "7.1.", "2.", "8.", "1.", "1.1.", "1.1.1."];
        assert_eq!(
            decimal_outline_labels(document.document().unwrap()),
            expected
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(decimal_outline_labels(&reopened), expected);
        assert_eq!(reopened.numbering, document.document().unwrap().numbering);
        let mut copied = document.copy().unwrap();
        copied
            .set_paragraph_list(
                ids[1],
                Some("decimal_outline"),
                0,
                false,
                Some(ListStart(20)),
            )
            .unwrap();
        assert_eq!(decimal_outline_labels(copied.document().unwrap())[1], "20.");
        assert_eq!(document.to_bytes().unwrap(), bytes);
        document
            .format_paragraph(ids[1], r#"{"direction":"rtl"}"#)
            .unwrap();
        assert!(document.to_bytes().is_ok());
        let revision = document.revision().unwrap();
        document
            .set_paragraph_list(ids[1], Some("decimal_outline"), 0, false, None)
            .unwrap();
        assert_eq!(
            decimal_outline_labels(document.document().unwrap())[1],
            "7."
        );
        assert!(document.revision().unwrap() > revision);
        document
            .set_paragraph_list(ids[1], None, 0, false, None)
            .unwrap();
        assert_eq!(document.paragraph_list(ids[1]).unwrap(), None);
    }

    #[test]
    fn decimal_outline_scope_geometry_and_later_output_failures_are_explicit() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Anchor").unwrap();
        let cell = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let cell_para = document.cell_paragraph_ids(cell, 0, 0).unwrap()[0];
        let header = document.ensure_story("header", "default", 0).unwrap();
        let header_para = document.story_add_paragraph(header, "Header").unwrap();
        let note = document
            .add_note(body, 0, 0, document.revision().unwrap(), "Note", "footnote")
            .unwrap();
        let note_para = document.story_paragraph_ids(note).unwrap()[0];
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for id in [cell_para, header_para, note_para] {
            assert!(document
                .set_paragraph_list(id, Some("decimal_outline"), 0, false, None)
                .unwrap_err()
                .to_string()
                .contains("top-level body"));
            assert_eq!(document.to_bytes().unwrap(), before);
            assert_eq!(document.revision().unwrap(), revision);
        }
        document
            .set_paragraph_list(body, Some("decimal_outline"), 0, false, None)
            .unwrap();
        let before = document.to_bytes().unwrap();
        assert!(document.add_section("continuous").is_err());
        assert!(document
            .set_section_props(0, r#"{"columns":[2,20,false]}"#)
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
        document
            .format_paragraph(body, r#"{"indent_left":600}"#)
            .unwrap();
        assert!(document
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("body area"));
        document
            .format_paragraph(body, r#"{"indent_left":null}"#)
            .unwrap();
        assert!(document.to_bytes().is_ok());
        let mut narrow = EditableDocx::new();
        narrow
            .set_section_props(0, r#"{"page_size":[144,300],"margins":[18,18,18,18]}"#)
            .unwrap();
        let id = narrow.add_paragraph("Narrow").unwrap();
        let before = narrow.to_bytes().unwrap();
        let revision = narrow.revision().unwrap();
        assert!(narrow
            .set_paragraph_list(id, Some("decimal_outline"), 8, false, None)
            .is_err());
        assert_eq!(narrow.to_bytes().unwrap(), before);
        assert_eq!(narrow.revision().unwrap(), revision);
    }

    #[test]
    fn decimal_outline_definition_identity_and_work_budgets_are_bounded() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("Identity").unwrap();
        document
            .set_paragraph_list(id, Some("decimal_outline"), 0, false, Some(ListStart(7)))
            .unwrap();
        let native = document.document().unwrap();
        let num = native
            .para(StoryRef::Body, &document.location(id).unwrap().1)
            .unwrap()
            .props
            .numbering
            .unwrap()
            .num;
        assert_eq!(
            supported_list_kind(native, num, &list_definitions(), &mut 0).unwrap(),
            Some("decimal_outline")
        );
        let mut work = MAX_LIST_MATCH_WORK;
        assert!(supported_list_kind(native, num, &list_definitions(), &mut work).is_err());
        let mut overridden = native.clone();
        let mut level = wordcraft_doc::numbering::levels_for(ListKind::Legal)[0].clone();
        level.start = 7;
        overridden.numbering.nums[0]
            .level_overrides
            .push((0, level));
        assert_eq!(
            supported_list_kind(&overridden, num, &list_definitions(), &mut 0).unwrap(),
            None
        );
        let mut unused = native.clone();
        let original = unused.numbering.nums[0].clone();
        unused.numbering.nums = (1..=MAX_LISTS as u32)
            .map(|id| wordcraft_doc::numbering::Num {
                id,
                ..original.clone()
            })
            .collect();
        assert_eq!(
            outline_targets(&unused).unwrap().len(),
            1,
            "unused restart IDs must not incur full matching work"
        );
        let mut altered = native.clone();
        altered.numbering.abstracts[0].levels[8].text = "%9)".into();
        assert_eq!(
            supported_list_kind(&altered, num, &list_definitions(), &mut 0).unwrap(),
            None
        );
        let mut large = EditableDocx::new();
        let id = large
            .add_paragraph(&"x".repeat(MAX_OUTLINE_WORK / 4))
            .unwrap();
        let revision = large.revision().unwrap();
        let before = large.to_bytes().unwrap();
        assert!(large
            .set_paragraph_list(id, Some("decimal_outline"), 0, false, None)
            .unwrap_err()
            .to_string()
            .contains("layout-work"));
        assert_eq!(large.revision().unwrap(), revision);
        assert_eq!(large.to_bytes().unwrap(), before);
        let id = document.add_paragraph("Ordinary still separate").unwrap();
        document
            .set_paragraph_list(id, Some("numbered"), 0, false, None)
            .unwrap();
        assert_eq!(document.paragraph_list(id).unwrap(), Some(("numbered", 0)));
    }
    #[test]
    fn decimal_outline_nonfinite_native_geometry_is_rejected() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let id = document.add_paragraph("Geometry").unwrap();
        document
            .set_paragraph_list(id, Some("decimal_outline"), 0, false, None)
            .unwrap();
        let native = document.document().unwrap();
        let targets = outline_targets(native).unwrap();
        let layout = wordcraft_layout::layout(native, &mut Default::default(), &Default::default());
        validate_outline_layout(&layout, &targets).unwrap();
        for malformed in 0..3 {
            let mut changed = layout.clone();
            let page = &mut changed.pages[0];
            match malformed {
                0 => {
                    let para = page
                        .items
                        .iter_mut()
                        .find_map(|item| {
                            if let wordcraft_layout::Placed::Lines { path, para, .. } = item {
                                (path == &document.location(id).unwrap().1).then_some(para)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    let para = Arc::make_mut(para);
                    let line = para.label_line();
                    para.lines[line].xs.insert(1, f32::NAN);
                }
                1 => {
                    page.body.x = f32::MAX;
                    page.body.w = f32::MAX;
                }
                _ => {
                    page.body.y = f32::MAX;
                    page.body.h = f32::MAX;
                }
            }
            assert!(validate_outline_layout(&changed, &targets).is_err());
        }
    }

    #[test]
    fn raw_note_conversion_preserves_ids_paths_and_standard_custom_styles() {
        pyo3::prepare_freethreaded_python();
        let mut owned = EditableDocx::new();
        let body = owned.add_paragraph("First anchor. Last anchor.").unwrap();
        let foot = owned
            .add_note(
                body,
                0,
                5,
                owned.revision().unwrap(),
                "Foot body",
                "footnote",
            )
            .unwrap();
        let end = owned
            .add_note(
                body,
                15,
                19,
                owned.revision().unwrap(),
                "End body",
                "endnote",
            )
            .unwrap();
        let table = owned.add_table(vec![vec!["Outer".into()]]).unwrap();
        let nested = owned
            .cell_add_table(table, 0, 0, vec![vec!["Nested anchor".into()]])
            .unwrap();
        let nested_para = owned.cell_paragraph_ids(nested, 0, 0).unwrap()[0];
        let nested_note = owned
            .add_note(
                nested_para,
                0,
                6,
                owned.revision().unwrap(),
                "Nested note body",
                "footnote",
            )
            .unwrap();
        let mut raw = owned.document().unwrap().clone();
        let custom = raw.para_mut(StoryRef::Part(foot), &Path::top(0)).unwrap();
        custom.props.style = Some("CustomNote".into());
        custom.runs[0].props.bold = Some(true);
        let ids: Vec<_> = raw.parts.keys().copied().collect();
        let before_text: Vec<_> = ids
            .iter()
            .map(|id| raw.plain_text(StoryRef::Part(*id)))
            .collect();
        let body_text = raw.plain_text(StoryRef::Body);
        let paths = raw.para_paths(StoryRef::Body);
        assert_eq!(raw.convert_notes(true, false), 2);
        assert_eq!(raw.parts.keys().copied().collect::<Vec<_>>(), ids);
        assert_eq!(raw.para_paths(StoryRef::Body), paths);
        assert_eq!(raw.plain_text(StoryRef::Body), body_text);
        assert_eq!(
            ids.iter()
                .map(|id| raw.plain_text(StoryRef::Part(*id)))
                .collect::<Vec<_>>(),
            before_text
        );
        for id in [foot, end, nested_note] {
            assert_eq!(raw.parts[&id].kind, PartKind::Endnote);
        }
        assert_eq!(
            raw.para(StoryRef::Part(foot), &Path::top(0))
                .unwrap()
                .props
                .style
                .as_deref(),
            Some("CustomNote")
        );
        assert_eq!(
            raw.para(StoryRef::Part(foot), &Path::top(0)).unwrap().runs[0]
                .props
                .bold,
            Some(true)
        );
        assert_eq!(
            raw.para(StoryRef::Part(nested_note), &Path::top(0))
                .unwrap()
                .props
                .style
                .as_deref(),
            Some("EndnoteText")
        );
        assert_eq!(raw.convert_notes(true, false), 0);
        assert_eq!(raw.convert_notes(false, true), 3);
        for id in [foot, end, nested_note] {
            assert_eq!(raw.parts[&id].kind, PartKind::Footnote);
        }
        let data = wordcraft_docx::write(&raw).unwrap();
        let reopened = wordcraft_docx::read(&data).unwrap();
        assert_eq!(
            reopened
                .parts
                .values()
                .filter(|p| p.kind == PartKind::Footnote)
                .count(),
            3
        );
        let pdf = crate::docx::to_pdf(&data, None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|p| &p.blocks)
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for text_part in ["Foot body", "End body", "Nested note body"] {
            assert_eq!(text.matches(text_part).count(), 1, "{text}");
        }
    }

    #[test]
    fn raw_note_conversion_placement_counters_and_long_continuation() {
        pyo3::prepare_freethreaded_python();
        let mut owned = EditableDocx::new();
        owned.set_page_size(300.0, 360.0).unwrap();
        owned.set_margins(24.0, 24.0, 24.0, 24.0).unwrap();
        owned
            .set_section_note_options(
                0,
                "footnote",
                r#"{"start":3,"numbering":"decimal","placement":"page_bottom"}"#,
            )
            .unwrap();
        owned
            .set_section_note_options(
                0,
                "endnote",
                r#"{"start":7,"numbering":"decimal","placement":"document_end"}"#,
            )
            .unwrap();
        let body = owned.add_paragraph("Citation").unwrap();
        let note = owned
            .add_note(
                body,
                0,
                8,
                owned.revision().unwrap(),
                "DetailFirst",
                "footnote",
            )
            .unwrap();
        owned.add_page_break().unwrap();
        owned.add_paragraph("FinalBody").unwrap();
        for target in ["footnote", "endnote"] {
            let mut raw = owned.document().unwrap().clone();
            if target == "endnote" {
                assert_eq!(raw.convert_notes(true, false), 1);
            }
            let data = wordcraft_docx::write(&raw).unwrap();
            let reopened = wordcraft_docx::read(&data).unwrap();
            let layout =
                wordcraft_layout::layout(&reopened, &mut Default::default(), &Default::default());
            let native_note = reopened
                .parts
                .iter()
                .find(|(_, p)| matches!(p.kind, PartKind::Footnote | PartKind::Endnote))
                .unwrap()
                .0;
            let places: Vec<_> = layout
                .pages
                .iter()
                .enumerate()
                .flat_map(|(page, p)| {
                    p.items.iter().filter_map(move |item| match item {
                        wordcraft_layout::Placed::Lines {
                            story: StoryRef::Part(id),
                            y,
                            ..
                        } if id == native_note => Some((page, *y)),
                        _ => None,
                    })
                })
                .collect();
            eprintln!("{target} placements {places:?}");
            assert_eq!(places.len(), 1);
            assert_eq!(places[0].0, if target == "footnote" { 0 } else { 1 });
            if target == "footnote" {
                assert!(places[0].1 > 250.0);
            } else {
                assert!(places[0].1 < 100.0);
            }
            let pdf = crate::docx::to_pdf(&data, None).unwrap();
            let extracted = crate::pdf::extract(&pdf, None).unwrap();
            let note_page = if target == "footnote" { 0 } else { 1 };
            let text = extracted.pages[note_page]
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("{target} text {text:?}");
            assert!(text.contains("DetailFirst"));
            assert!(text.contains(if target == "footnote" { "3" } else { "7" }));
            assert!(
                crate::docx::render(&data, note_page, 72.0, None)
                    .unwrap()
                    .len()
                    > 100
            );
        }
        for index in 0..45 {
            owned
                .story_add_paragraph(note, &format!("Continuation{index:02} complete text."))
                .unwrap();
        }
        for end in [false, true] {
            let mut raw = owned.document().unwrap().clone();
            raw.convert_notes(end, !end);
            let data = wordcraft_docx::write(&raw).unwrap();
            let reopened = wordcraft_docx::read(&data).unwrap();
            let layout =
                wordcraft_layout::layout(&reopened, &mut Default::default(), &Default::default());
            let pages: BTreeSet<_> = layout
                .pages
                .iter()
                .enumerate()
                .filter_map(|(page, p)| {
                    p.items
                        .iter()
                        .any(|i| {
                            matches!(
                                i,
                                wordcraft_layout::Placed::Lines {
                                    story: StoryRef::Part(_),
                                    ..
                                }
                            )
                        })
                        .then_some(page)
                })
                .collect();
            eprintln!("end={end} long pages {pages:?}");
            assert!(pages.len() > 1);
            let pdf = crate::docx::to_pdf(&data, None).unwrap();
            let extracted = crate::pdf::extract(&pdf, None).unwrap();
            let text = extracted
                .pages
                .iter()
                .flat_map(|p| &p.blocks)
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            for index in 0..45 {
                assert_eq!(
                    text.matches(&format!("Continuation{index:02}")).count(),
                    1,
                    "{index}: {text}"
                );
            }
        }
    }

    #[test]
    fn note_conversion_owned_noop_handles_accounting_and_exact_work_budget() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        assert_eq!(document.convert_notes("endnote").unwrap(), 0);
        assert_eq!(document.revision().unwrap(), 0);
        let body = document.add_paragraph("Anchor").unwrap();
        let note = document
            .add_note(
                body,
                0,
                6,
                document.revision().unwrap(),
                "Detail",
                "footnote",
            )
            .unwrap();
        let note_para = document.story_paragraph_ids(note).unwrap()[0];
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        let usage = (
            document.text_bytes,
            document.cell_count,
            document.media_bytes,
        );
        let locations = document.locations.clone();
        let copied = document.copy().unwrap();
        assert_eq!(document.convert_notes("footnote").unwrap(), 0);
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);
        for kind in ["", "Footnote", "swap", "endnote\0", &"x".repeat(100)] {
            assert!(document.convert_notes(kind).is_err());
        }
        assert_eq!(document.convert_notes("endnote").unwrap(), 1);
        assert_eq!(document.revision().unwrap(), revision + 1);
        assert_eq!(document.locations, locations);
        assert_eq!(
            (
                document.text_bytes,
                document.cell_count,
                document.media_bytes
            ),
            usage
        );
        assert_eq!(document.paragraph_text(note_para).unwrap(), "Detail");
        assert_eq!(document.paragraph_style(note_para).unwrap(), "Endnote Text");
        assert_eq!(copied.note_ids(Some("footnote")).unwrap(), vec![note]);
        assert_eq!(document.note_ids(Some("endnote")).unwrap(), vec![note]);
        assert!(document.text_range(note_para, 0, 1).is_ok());
        let mut raw = copied.document().unwrap().clone();
        let p = raw
            .para_mut(StoryRef::Body, &copied.location(body).unwrap().1)
            .unwrap();
        p.text = "\u{fffc}".repeat(1997);
        p.objects = vec![
            InlineObject::NoteRef {
                kind: NoteKind::Footnote,
                id: note,
                custom: String::new()
            };
            1997
        ];
        p.runs = (0..1000)
            .map(|i| wordcraft_doc::para::Run {
                len: if i < 997 { 6 } else { 3 },
                props: CharProps::default(),
            })
            .collect();
        // Body:1+1997+1000+(1997*1000); converted note:1+1. Exactly2million.
        validate_note_conversion_work(&raw, NoteKind::Endnote).unwrap();
        raw.body.push(wordcraft_doc::para_block(Paragraph::new()));
        assert!(validate_note_conversion_work(&raw, NoteKind::Endnote).is_err());
        let mut over = copied.copy().unwrap();
        over.document = Some(raw);
        let revision = over.revision().unwrap();
        let original = wordcraft_docx::write(over.document().unwrap()).unwrap();
        assert!(over.convert_notes("endnote").is_err());
        assert_eq!(over.revision().unwrap(), revision);
        assert_eq!(
            wordcraft_docx::write(over.document().unwrap()).unwrap(),
            original
        );
        document.close();
        assert!(document.convert_notes("footnote").is_err());
    }

    #[test]
    fn note_conversion_only_allows_exact_kind_remapping_and_rolls_back_field_scope() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Anchor").unwrap();
        let note = document
            .add_note(
                body,
                0,
                6,
                document.revision().unwrap(),
                "Detail",
                "footnote",
            )
            .unwrap();
        let bookmark = document.add_paragraph("Target").unwrap();
        document
            .set_bookmark("Mark", bookmark, 0, 6, document.revision().unwrap())
            .unwrap();
        let bytes = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        for malformed in 0..6 {
            let mut raw = document.document().unwrap().clone();
            raw.convert_notes(true, false);
            match malformed {
                0 => {
                    raw.parts.remove(&note);
                }
                1 => {
                    raw.insert_object(
                        &document.position(body, 0).unwrap(),
                        InlineObject::NoteRef {
                            kind: NoteKind::Endnote,
                            id: note,
                            custom: String::new(),
                        },
                        &Default::default(),
                    )
                    .unwrap();
                }
                2 => {
                    raw.parts.get_mut(&note).unwrap().kind = PartKind::Footnote;
                }
                3 => {
                    raw.insert_object(
                        &document.position(body, 0).unwrap(),
                        InlineObject::BookmarkStart {
                            name: "Extra".into(),
                        },
                        &Default::default(),
                    )
                    .unwrap();
                }
                4 => {
                    raw.delete_range(
                        &document.position(body, 6).unwrap(),
                        &document.position(body, 9).unwrap(),
                    )
                    .unwrap();
                }
                _ => {
                    raw.insert_object(
                        &document.position(body, 0).unwrap(),
                        InlineObject::CommentStart { id: 42 },
                        &Default::default(),
                    )
                    .unwrap();
                }
            }
            assert!(
                document
                    .commit_with_note_conversion(raw, None, Some(NoteKind::Endnote))
                    .is_err(),
                "case{malformed}"
            );
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), bytes);
        }
        let mut raw = document.document().unwrap().clone();
        raw.convert_notes(true, false);
        assert!(
            document.commit(raw).is_err(),
            "ordinary edits cannot change anchor kind"
        );
        let note_para = document.story_paragraph_ids(note).unwrap()[0];
        document
            .add_paragraph_field(note_para, "section_page_count")
            .unwrap();
        let section = document.add_section("continuous").unwrap();
        document.add_paragraph("Last continuous section").unwrap();
        document
            .set_section_note_options(section, "endnote", r#"{"placement":"section_end"}"#)
            .unwrap();
        let before = document.to_bytes().unwrap();
        let revision = document.revision().unwrap();
        assert!(document.convert_notes("endnote").is_err());
        assert_eq!(document.revision().unwrap(), revision);
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!(document.note_ids(Some("footnote")).unwrap(), vec![note]);
        document
            .set_section_props(section, r#"{"start":"next_page"}"#)
            .unwrap();
        assert_eq!(document.convert_notes("endnote").unwrap(), 1);
    }

    #[test]
    fn upstream_note_conversion_restyles_whole_shared_reference_run() {
        pyo3::prepare_freethreaded_python();
        let mut owned = EditableDocx::new();
        let p = owned.add_paragraph("Ordinary before after").unwrap();
        owned
            .add_note(
                p,
                9,
                15,
                owned.revision().unwrap(),
                "Foot detail",
                "footnote",
            )
            .unwrap();
        owned
            .add_note(
                p,
                17,
                22,
                owned.revision().unwrap(),
                "End detail",
                "endnote",
            )
            .unwrap();
        owned
            .format_style(
                "FootnoteReference",
                r##"{"color":"#FF0000","font_size":16}"##,
                None,
            )
            .unwrap();
        owned
            .format_style(
                "EndnoteReference",
                r##"{"color":"#0000FF","font_size":16}"##,
                None,
            )
            .unwrap();
        let end = owned.paragraph_text(p).unwrap().chars().count();
        owned
            .format_range_patch(
                p,
                0,
                end,
                owned.revision().unwrap(),
                r#"{"style":"FootnoteReference"}"#,
            )
            .unwrap();
        let before = owned.document().unwrap().clone();
        let mut after = before.clone();
        assert_eq!(after.convert_notes(true, false), 1);
        let before_para = before
            .para(StoryRef::Body, &owned.location(p).unwrap().1)
            .unwrap();
        let after_para = after
            .para(StoryRef::Body, &owned.location(p).unwrap().1)
            .unwrap();
        assert_eq!(before_para.text, after_para.text);
        assert_eq!(before_para.runs.len(), 1);
        assert_eq!(after_para.runs.len(), 1);
        assert_eq!(
            before_para.runs[0].props.style.as_deref(),
            Some("FootnoteReference")
        );
        assert_eq!(
            after_para.runs[0].props.style.as_deref(),
            Some("EndnoteReference")
        );
        // Raw native output bypasses the local guard, retaining the upstream defect.
        for pdf in [false, true] {
            for (raw, visible, absent) in [(&before, 0, 2), (&after, 2, 0)] {
                let bytes = wordcraft_docx::write(raw).unwrap();
                let png = if pdf {
                    crate::pdf::render(&crate::docx::to_pdf(&bytes, None).unwrap(), 0, 144.0, None)
                        .unwrap()
                } else {
                    crate::docx::render(&bytes, 0, 144.0, None).unwrap()
                };
                assert!(border_pixels(png.clone(), visible) > 100);
                assert_eq!(border_pixels(png, absent), 0);
            }
        }
        assert!(owned.convert_notes("endnote").is_err());
        assert_eq!(
            owned.document().unwrap().plain_text(StoryRef::Body),
            before.plain_text(StoryRef::Body)
        );
    }
}

#[cfg(test)]
mod letter_spacing_tests {
    use super::*;

    fn first_para(layout: &wordcraft_layout::DocLayout) -> &wordcraft_layout::ParaLayout {
        layout
            .pages
            .iter()
            .flat_map(|page| &page.items)
            .find_map(|item| match item {
                wordcraft_layout::Placed::Lines { para, .. } => Some(para.as_ref()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn letter_spacing_all_twips_native_write_read_and_scalar_limits() {
        let mut native = Document::new();
        native.body.clear();
        for twip in -2000..=2000 {
            let value = f64::from(twip) / 20.0;
            let spacing = letter_spacing(&json!(value)).unwrap();
            assert_eq!(spacing_points(spacing), value);
            native
                .body
                .push(wordcraft_doc::para_block(Paragraph::with_text(
                    "x",
                    CharProps {
                        spacing: Some(spacing),
                        ..Default::default()
                    },
                )));
        }
        let read = wordcraft_docx::read(&wordcraft_docx::write(&native).unwrap()).unwrap();
        for (index, twip) in (-2000..=2000).enumerate() {
            let p = read.para(StoryRef::Body, &Path::top(index)).unwrap();
            assert_eq!(
                spacing_points(p.props_of_char(0).spacing.unwrap()),
                f64::from(twip) / 20.0
            );
        }
        assert_eq!(
            spacing_points(letter_spacing(&json!(0.1_f64 + 0.2)).unwrap()),
            0.3
        );
        assert_eq!(
            spacing_points(letter_spacing(&json!(-0.1_f64 - 0.2)).unwrap()),
            -0.3
        );
        for bad in [
            json!(true),
            json!("1"),
            json!(100.05),
            json!(-100.05),
            json!(0.301),
            json!(-0.001),
        ] {
            assert!(letter_spacing(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn letter_spacing_native_clusters_and_visible_source_pdf_parity() {
        use std::io::Cursor;
        let pixels = |bytes: Vec<u8>| {
            let mut read = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
            let mut data = vec![0; read.output_buffer_size().unwrap()];
            let frame = read.next_frame(&mut data).unwrap();
            assert_eq!(frame.color_type, png::ColorType::Rgba);
            data.truncate(frame.buffer_size());
            (frame.width, frame.height, data)
        };
        for text in ["e\u{301} office", "سلام Word", "A\u{200b}B"] {
            let mut previous = None;
            for spacing in [-100.0_f32, -0.05, 0.0, 2.0, 100.0] {
                let mut native = Document::from_text(text);
                let section = native.section_mut(0);
                section.page_w = 1584.0;
                section.page_h = 400.0;
                section.margin_left = 1000.0;
                section.margin_right = 30.0;
                section.margin_top = 30.0;
                section.margin_bottom = 30.0;
                native.styles.default_chr.spacing = Some(spacing);
                let layout =
                    wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
                let p = first_para(&layout);
                let ranges: Vec<_> = p.clusters.iter().map(|c| (c.start, c.end)).collect();
                if let Some(previous) = &previous {
                    assert_eq!(&ranges, previous);
                }
                previous = Some(ranges);
                assert!(p
                    .lines
                    .iter()
                    .all(|line| line.xs.iter().all(|x| x.is_finite())));
                if text.starts_with('e') {
                    assert!(p
                        .clusters
                        .iter()
                        .any(|c| c.start == 0 && c.end == "e\u{301}".len()));
                }
                let raw = wordcraft_docx::write(&native).unwrap();
                let reopened = wordcraft_docx::read(&raw).unwrap();
                assert_eq!(
                    reopened.para(StoryRef::Body, &Path::top(0)).unwrap().text,
                    text
                );
                let source = pixels(crate::docx::render(&raw, 0, 144.0, None).unwrap());
                let pdf = crate::docx::to_pdf(&raw, None).unwrap();
                let exported = pixels(crate::pdf::render(&pdf, 0, 144.0, None).unwrap());
                assert_eq!((source.0, source.1), (exported.0, exported.1));
                assert_eq!(source.2.len(), exported.2.len());
                assert!(source.2.chunks_exact(4).filter(|px| px[0] < 128).count() > 20);
                assert!(source
                    .2
                    .iter()
                    .zip(&exported.2)
                    .all(|(a, b)| a.abs_diff(*b) <= 1));
            }
        }
    }

    #[test]
    fn letter_spacing_style_default_table_precedence_and_atomic_failure() {
        let mut d = EditableDocx::new();
        d.format_defaults(r#"{"letter_spacing":0.05}"#).unwrap();
        let base = d.add_style("Spaced", "paragraph", Some("Normal")).unwrap();
        d.format_style(&base, r#"{"letter_spacing":2}"#, None)
            .unwrap();
        let child = d
            .add_style("Inherited spacing", "paragraph", Some(&base))
            .unwrap();
        let character = d.add_style("Condensed", "character", None).unwrap();
        d.format_style(&character, r#"{"letter_spacing":-0.3}"#, None)
            .unwrap();
        let id = d.add_paragraph("").unwrap();
        d.set_paragraph_style(id, &child).unwrap();
        let (start, end, revision) = d
            .add_run_patch(
                id,
                "Scope",
                &json!({"style":character,"letter_spacing":0}).to_string(),
            )
            .unwrap();
        let effective = |doc: &EditableDocx| {
            let p = doc.paragraph(id).unwrap();
            doc.document()
                .unwrap()
                .styles
                .resolve_char(p.props.style.as_deref(), p.props_of_char(0))
                .spacing
        };
        assert_eq!(effective(&d), 0.0);
        d.format_range_patch(id, start, end, revision, r#"{"letter_spacing":null}"#)
            .unwrap();
        assert_eq!(spacing_points(effective(&d)), -0.3);
        d.format_style(&character, r#"{"letter_spacing":null}"#, None)
            .unwrap();
        assert_eq!(effective(&d), 2.0);
        d.format_style(&base, r#"{"letter_spacing":null}"#, None)
            .unwrap();
        assert_eq!(spacing_points(effective(&d)), 0.05);
        d.format_defaults(r#"{"letter_spacing":null}"#).unwrap();
        assert_eq!(effective(&d), 0.0);
        let style = d.add_style("Conditional spacing", "table", None).unwrap();
        d.format_style(&style, r#"{"letter_spacing":1}"#, None)
            .unwrap();
        d.format_style(&style, r#"{"letter_spacing":3}"#, Some("first_row"))
            .unwrap();
        let table = d
            .add_table(vec![vec!["Header".into()], vec!["Body".into()]])
            .unwrap();
        d.table_format_patch(
            table,
            &json!({"style":style,"look":{"header_row":true}}).to_string(),
        )
        .unwrap();
        let raw = d.to_bytes().unwrap();
        let native = wordcraft_docx::read(&raw).unwrap();
        let layout =
            wordcraft_layout::layout(&native, &mut Default::default(), &Default::default());
        let mut seen = Vec::new();
        for page in &layout.pages {
            for item in &page.items {
                if let wordcraft_layout::Placed::Lines {
                    story, path, para, ..
                } = item
                {
                    if let Some(p) = native.para(*story, path) {
                        seen.push((p.text.clone(), para.styles[0].rc.spacing));
                    }
                }
            }
        }
        assert!(seen
            .iter()
            .any(|(text, spacing)| text == "Header" && *spacing == 3.0));
        assert!(seen
            .iter()
            .any(|(text, spacing)| text == "Body" && *spacing == 1.0));
        let before = d.document().unwrap().clone();
        let revision = d.revision().unwrap();
        for patch in [
            r#"{"bold":true,"letter_spacing":true}"#,
            r#"{"letter_spacing":100.05}"#,
            r#"{"letter_spacing":0.301}"#,
        ] {
            assert!(d.format_defaults(patch).is_err());
            assert!(d.format_style(&base, patch, None).is_err());
            assert!(d.format_range_patch(id, 0, 5, revision, patch).is_err());
            assert_eq!(d.document().unwrap(), &before);
            assert_eq!(d.revision().unwrap(), revision);
        }
    }
}

#[cfg(test)]
mod note_clear_tests {
    use super::*;

    fn owned(kind: &str, text: &str) -> (EditableDocx, usize, u32, usize) {
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Anchor").unwrap();
        let note = document
            .add_note(body, 0, 6, document.revision().unwrap(), text, kind)
            .unwrap();
        let paragraph = document.story_paragraph_ids(note).unwrap()[0];
        (document, body, note, paragraph)
    }

    #[test]
    fn note_clear_native_styled_mark_roundtrip_and_conversion_noop() {
        pyo3::prepare_freethreaded_python();
        for kind in ["footnote", "endnote"] {
            let (mut document, body, note, old) = owned(kind, "EraseMe");
            let anchors = semantic_anchors(document.document().unwrap());
            let part_body = document.document().unwrap().parts[&note].body;
            let old_next = document.next_block_id;
            assert!(document.clear_note(note).unwrap());
            assert_eq!(semantic_anchors(document.document().unwrap()), anchors);
            assert_eq!(document.document().unwrap().parts[&note].body, part_body);
            assert!(document.paragraph_text(old).is_err());
            assert!(document.story_paragraph_ids(note).unwrap().is_empty());
            assert_eq!(document.next_block_id, old_next);
            assert_eq!(document.paragraph_text(body).unwrap(), "Anchor\u{fffc}");
            assert_eq!(document.text_bytes, 9);
            let style = if kind == "footnote" {
                "FootnoteText"
            } else {
                "EndnoteText"
            };
            let bytes = document.to_bytes().unwrap();
            let reopened = wordcraft_docx::read(&bytes).unwrap();
            let part = reopened
                .parts
                .values()
                .find(|p| matches!(p.kind, PartKind::Footnote | PartKind::Endnote))
                .unwrap();
            let Block::Para(paragraph) = part.blocks[0].as_ref() else {
                panic!("note paragraph")
            };
            assert_eq!(paragraph.props.style.as_deref(), Some(style));
            assert_eq!(
                paragraph.objects.len(),
                1,
                "writer emits one own number mark"
            );
            assert!(matches!(paragraph.objects[0], InlineObject::NoteRef { .. }));
            assert_eq!(paragraph.text, "\u{fffc}");
            let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
            let text = crate::pdf::extract(&pdf, None)
                .unwrap()
                .pages
                .into_iter()
                .flat_map(|p| p.blocks)
                .map(|b| b.text)
                .collect::<Vec<_>>()
                .join(" ");
            assert!(text.contains("Anchor"));
            assert!(!text.contains("EraseMe"));
            assert!(crate::docx::render(&bytes, 0, 72.0, None).unwrap().len() > 100);
            let revision = document.revision().unwrap();
            assert!(!document.clear_note(note).unwrap());
            assert_eq!(document.revision().unwrap(), revision);
            assert_eq!(document.to_bytes().unwrap(), bytes);
            let other = if kind == "footnote" {
                "endnote"
            } else {
                "footnote"
            };
            assert_eq!(document.convert_notes(other).unwrap(), 1);
            let revision = document.revision().unwrap();
            assert!(
                !document.clear_note(note).unwrap(),
                "conversion keeps canonical synthetic paragraph"
            );
            assert_eq!(document.revision().unwrap(), revision);
            let fresh = document.story_add_paragraph(note, "Fresh").unwrap();
            assert!(fresh > old);
            assert_eq!(document.document().unwrap().parts[&note].blocks.len(), 1);
            assert_eq!(
                document.paragraph_style(fresh).unwrap(),
                if other == "footnote" {
                    "Footnote Text"
                } else {
                    "Endnote Text"
                }
            );
            document.close();
            assert!(document.clear_note(note).is_err());
        }
    }

    #[test]
    fn note_clear_accounting_copies_empty_and_semantic_rollback() {
        pyo3::prepare_freethreaded_python();
        let (mut document, body, note, paragraph) = owned("footnote", "");
        let revision = document.revision().unwrap();
        assert!(
            document.clear_note(note).unwrap(),
            "authored empty paragraph is removed"
        );
        assert_eq!(document.revision().unwrap(), revision + 1);
        assert!(document.paragraph_text(paragraph).is_err());
        let paragraph = document.story_add_paragraph(note, "abc").unwrap();
        document.add_paragraph_field(paragraph, "page").unwrap();
        assert_eq!(document.text_bytes, 15); // 6 body + 3 reference + 3 text + 3 field
        let mut copied = document.copy().unwrap();
        copied.text_bytes = MAX_DOCUMENT_TEXT_BYTES;
        assert!(copied.clear_note(note).unwrap());
        assert_eq!(copied.text_bytes, MAX_DOCUMENT_TEXT_BYTES - 6);
        let fresh = copied.story_add_paragraph(note, "123456").unwrap();
        assert_eq!(copied.text_bytes, MAX_DOCUMENT_TEXT_BYTES);
        let revision = copied.revision().unwrap();
        assert!(copied.add_run_patch(fresh, "x", "{}").is_err());
        assert_eq!(copied.revision().unwrap(), revision);
        assert_eq!(document.paragraph_text(paragraph).unwrap(), "abc\u{fffc}");
        assert_eq!(document.paragraph_text(body).unwrap(), "Anchor\u{fffc}");
        for object in [
            InlineObject::NoteRef {
                kind: NoteKind::Footnote,
                id: note,
                custom: String::new(),
            },
            InlineObject::NoteRef {
                kind: NoteKind::Footnote,
                id: 999,
                custom: String::new(),
            },
            InlineObject::BookmarkStart {
                name: "guarded".into(),
            },
            InlineObject::CommentStart { id: 99 },
        ] {
            let mut bad = document.copy().unwrap();
            bad.document
                .as_mut()
                .unwrap()
                .insert_object(
                    &Pos::new(StoryRef::Part(note), Path::top(0), 0),
                    object,
                    &CharProps::default(),
                )
                .unwrap();
            bad.text_bytes += 3;
            let before = wordcraft_docx::write(bad.document().unwrap()).unwrap();
            let revision = bad.revision().unwrap();
            let locations = bad.locations.clone();
            let stories = bad.stories.clone();
            let counts = (bad.text_bytes, bad.cell_count, bad.media_bytes);
            assert!(bad.clear_note(note).is_err());
            assert_eq!(
                wordcraft_docx::write(bad.document().unwrap()).unwrap(),
                before
            );
            assert_eq!(bad.revision().unwrap(), revision);
            assert_eq!(bad.locations, locations);
            assert_eq!(bad.stories, stories);
            assert_eq!((bad.text_bytes, bad.cell_count, bad.media_bytes), counts);
        }
        assert!(document.clear_note(u32::MAX).is_err());
        let header = document.ensure_story("header", "default", 0).unwrap();
        let before = document.to_bytes().unwrap();
        assert!(document.clear_note(header).is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
        let mut missing = document.copy().unwrap();
        missing.document.as_mut().unwrap().parts.remove(&note);
        assert!(missing.clear_note(note).is_err());
    }

    #[test]
    fn note_clear_native_media_and_cells_reclaim_without_reference_loss() {
        pyo3::prepare_freethreaded_python();
        let (mut document, _, note, _) = owned("endnote", "Gone");
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0])
                .unwrap();
        }
        let picture = document
            .add_image(&png, Some(10.0), Some(10.0), "shared")
            .unwrap();
        let (story, path) = document.location(picture).unwrap();
        let image = document
            .document()
            .unwrap()
            .para(story, &path)
            .unwrap()
            .clone();
        let mut candidate = document.document().unwrap().clone();
        candidate
            .insert_block(StoryRef::Part(note), &Path::top(1), Block::Para(image))
            .unwrap();
        let (table, bytes, cells) = document
            .make_table(vec![vec!["nested".into()]], 100.0)
            .unwrap();
        candidate
            .insert_block(StoryRef::Part(note), &Path::top(2), Block::Table(table))
            .unwrap();
        document.commit(candidate).unwrap();
        document.text_bytes += bytes;
        document.cell_count += cells;
        let table = document
            .register_location(StoryRef::Part(note), Path::top(2), false)
            .unwrap();
        let note_picture = document
            .register_location(StoryRef::Part(note), Path::top(1), true)
            .unwrap();
        document.table_generations.insert(table, 0);
        let copy = document.copy().unwrap();
        assert!(document.clear_note(note).unwrap());
        assert_eq!(document.text_bytes, 9);
        assert_eq!(document.cell_count, 0);
        assert!(!document.locations.contains_key(&note_picture));
        assert!(!document.table_generations.contains_key(&table));
        assert_eq!(document.image_data(picture).unwrap(), png);
        assert_eq!(document.media_bytes, png.len());
        document.remove_image(picture).unwrap();
        assert_eq!(document.media_bytes, 0);
        assert!(document.document().unwrap().media.is_empty());
        assert_eq!(copy.media_bytes, png.len());
        assert_eq!(copy.cell_count, 1);
        assert_eq!(copy.note_ids(None).unwrap(), vec![note]);
    }
}

#[cfg(test)]
mod story_paragraph_delete_tests {
    use super::*;

    #[test]
    fn story_paragraph_delete_preserves_shifted_descendant_handles_and_native_tail() {
        pyo3::prepare_freethreaded_python();
        for kind in ["header", "footer"] {
            let mut document = EditableDocx::new();
            document.add_paragraph("Body").unwrap();
            let part = document.ensure_story(kind, "default", 0).unwrap();
            let first = document.story_add_paragraph(part, "Remove").unwrap();
            let table = document
                .story_add_table(part, vec![vec!["Cell".into()]])
                .unwrap();
            let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
            let nested = document
                .cell_add_table(table, 0, 0, vec![vec!["Nested".into()]])
                .unwrap();
            let deep = document.cell_paragraph_ids(nested, 0, 0).unwrap()[0];
            let last = document.story_add_paragraph(part, "Last").unwrap();
            let generation = document.table_generation(table).unwrap();
            let nested_generation = document.table_generation(nested).unwrap();
            let old_path = document.location(deep).unwrap().1;
            let copied = document.copy().unwrap();
            document.remove_paragraph(first).unwrap();
            assert!(document.paragraph_text(first).is_err());
            assert_eq!(document.story_paragraph_ids(part).unwrap(), vec![last]);
            assert_eq!(document.paragraph_text(cell).unwrap(), "Cell");
            assert_eq!(document.paragraph_text(deep).unwrap(), "Nested");
            assert_eq!(document.table_generation(table).unwrap(), generation);
            assert_eq!(
                document.table_generation(nested).unwrap(),
                nested_generation
            );
            let new_path = document.location(deep).unwrap().1;
            assert_eq!(new_path.0[0] + 1, old_path.0[0]);
            assert_eq!(new_path.0[1..], old_path.0[1..]);
            assert_eq!(copied.paragraph_text(first).unwrap(), "Remove");
            document.remove_paragraph(last).unwrap();
            let blocks = &document.document().unwrap().parts[&part].blocks;
            assert_eq!(blocks.len(), 2);
            assert!(matches!(blocks[0].as_ref(), Block::Table(_)));
            assert_eq!(*blocks[1], Block::Para(Paragraph::new()));
            assert_eq!(document.story_placeholders.get(&part), Some(&1));
            assert!(document.story_paragraph_ids(part).unwrap().is_empty());
            let next = document.story_add_paragraph(part, "Fresh").unwrap();
            assert!(next > last);
            assert_eq!(document.document().unwrap().parts[&part].blocks.len(), 2);
            assert_eq!(document.paragraph_text(deep).unwrap(), "Nested");
            let bytes = document.to_bytes().unwrap();
            let reopened = wordcraft_docx::read(&bytes).unwrap();
            let part = reopened
                .parts
                .values()
                .find(|p| {
                    p.kind
                        == if kind == "header" {
                            PartKind::Header
                        } else {
                            PartKind::Footer
                        }
                })
                .unwrap();
            assert!(matches!(part.blocks[0].as_ref(), Block::Table(_)));
            assert!(part.blocks[1].as_para().unwrap().text.contains("Fresh"));
        }
    }

    #[test]
    fn story_paragraph_delete_note_order_empty_placeholder_and_exact_charges() {
        pyo3::prepare_freethreaded_python();
        for kind in ["footnote", "endnote"] {
            let mut document = EditableDocx::new();
            let body = document.add_paragraph("Body").unwrap();
            let note = document
                .add_note(body, 0, 4, document.revision().unwrap(), "First", kind)
                .unwrap();
            let first = document.story_paragraph_ids(note).unwrap()[0];
            let second = document.story_add_paragraph(note, "Second").unwrap();
            document.add_paragraph_field(second, "page").unwrap();
            let anchors = semantic_anchors(document.document().unwrap());
            let before_count = document.text_bytes;
            document.remove_paragraph(first).unwrap();
            assert_eq!(document.text_bytes, before_count - 5);
            assert_eq!(document.paragraph_text(second).unwrap(), "Second\u{fffc}");
            assert_eq!(document.location(second).unwrap().1, Path::top(0));
            document.text_bytes = MAX_DOCUMENT_TEXT_BYTES;
            document.remove_paragraph(second).unwrap();
            assert_eq!(document.text_bytes, MAX_DOCUMENT_TEXT_BYTES - 9);
            assert_eq!(semantic_anchors(document.document().unwrap()), anchors);
            assert!(document.story_paragraph_ids(note).unwrap().is_empty());
            let revision = document.revision().unwrap();
            assert!(!document.clear_note(note).unwrap());
            assert_eq!(document.revision().unwrap(), revision);
            let other = if kind == "footnote" {
                "endnote"
            } else {
                "footnote"
            };
            document.convert_notes(other).unwrap();
            assert!(!document.clear_note(note).unwrap());
            let new = document.story_add_paragraph(note, "123456789").unwrap();
            assert!(new > second);
            assert_eq!(document.text_bytes, MAX_DOCUMENT_TEXT_BYTES);
            assert!(document.add_run_patch(new, "x", "{}").is_err());
            assert_eq!(document.document().unwrap().parts[&note].blocks.len(), 1);
        }
    }

    #[test]
    fn story_paragraph_delete_scope_anchors_bad_locators_and_aliases_are_atomic() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let body = document.add_paragraph("Body").unwrap();
        document.add_section("next_page").unwrap();
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let part = document.ensure_story("header", "default", 0).unwrap();
        let target = document.story_add_paragraph(part, "Protected").unwrap();
        for id in [body, cell, table, usize::MAX] {
            let raw = document.to_bytes().unwrap();
            let revision = document.revision().unwrap();
            assert!(document.remove_paragraph(id).is_err());
            assert_eq!(document.to_bytes().unwrap(), raw);
            assert_eq!(document.revision().unwrap(), revision);
        }
        for failure in 0..5 {
            let mut bad = document.copy().unwrap();
            match failure {
                0 => {
                    bad.document
                        .as_mut()
                        .unwrap()
                        .insert_object(
                            &Pos::new(StoryRef::Part(part), Path::top(0), 0),
                            InlineObject::NoteRef {
                                kind: NoteKind::Footnote,
                                id: 999,
                                custom: String::new(),
                            },
                            &CharProps::default(),
                        )
                        .unwrap();
                    bad.text_bytes += 3;
                }
                1 => bad.document.as_mut().unwrap().last_section.headers.first = Some(part),
                2 => {
                    bad.locations
                        .insert(usize::MAX, (StoryRef::Part(part), Path(vec![0; 52])));
                }
                3 => {
                    bad.cell_placeholders
                        .insert((StoryRef::Part(part), Path(vec![])));
                }
                _ => {
                    bad.document
                        .as_mut()
                        .unwrap()
                        .para_mut(StoryRef::Part(part), &Path::top(0))
                        .unwrap()
                        .section = Some(Box::default())
                }
            }
            let bytes = wordcraft_docx::write(bad.document().unwrap()).unwrap();
            let revision = bad.revision().unwrap();
            let locations = bad.locations.clone();
            let placeholders = bad.cell_placeholders.clone();
            let text = bad.text_bytes;
            assert!(bad.remove_paragraph(target).is_err());
            assert_eq!(
                wordcraft_docx::write(bad.document().unwrap()).unwrap(),
                bytes
            );
            assert_eq!(bad.revision().unwrap(), revision);
            assert_eq!(bad.locations, locations);
            assert_eq!(bad.cell_placeholders, placeholders);
            assert_eq!(bad.text_bytes, text);
        }
        document.close();
        assert!(document.remove_paragraph(target).is_err());
    }
}

#[cfg(test)]
mod body_identity_tests {
    use super::*;

    fn red_png() -> Vec<u8> {
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255].repeat(4))
                .unwrap();
        }
        data
    }

    fn unchanged(actual: &EditableDocx, before: &EditableDocx) {
        assert_eq!(
            wordcraft_docx::write(actual.document().unwrap()).unwrap(),
            wordcraft_docx::write(before.document().unwrap()).unwrap()
        );
        assert_eq!(actual.locations, before.locations);
        assert_eq!(actual.next_block_id, before.next_block_id);
        assert_eq!(actual.body_placeholder, before.body_placeholder);
        assert_eq!(actual.images, before.images);
        assert_eq!(actual.stories, before.stories);
        assert_eq!(actual.story_placeholders, before.story_placeholders);
        assert_eq!(actual.cell_placeholders, before.cell_placeholders);
        assert_eq!(actual.table_generations, before.table_generations);
        assert_eq!(
            (
                actual.revision,
                actual.text_bytes,
                actual.cell_count,
                actual.media_bytes
            ),
            (
                before.revision,
                before.text_bytes,
                before.cell_count,
                before.media_bytes
            )
        );
    }

    #[test]
    fn body_identity_native_order_is_independent_of_allocation_and_registry_key_order() {
        let mut document = EditableDocx::new();
        let first = document.add_paragraph("First").unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let auxiliary = document.story_add_paragraph(header, "Header").unwrap();
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let picture = document
            .add_image(&red_png(), Some(12.0), Some(12.0), "Red")
            .unwrap();
        let last = document.add_paragraph("Last").unwrap();
        assert!(
            first < auxiliary
                && auxiliary < table
                && table < cell
                && cell < picture
                && picture < last
        );
        assert_ne!(first, document.location(first).unwrap().1 .0[0] as usize);
        assert_eq!(document.paragraph_ids().unwrap(), vec![first, last]);
        assert_eq!(document.table_ids().unwrap(), vec![table]);
        assert_eq!(document.image_ids().unwrap(), vec![picture]);
        document
            .set_bookmark("First", first, 0, 5, document.revision)
            .unwrap();
        assert_eq!(document.bookmark_range("First").unwrap().0, first);
        let before = document.to_bytes().unwrap();
        let revision = document.revision;
        let next = document.next_block_id;
        // Test-only permutation of opaque identities: native positions and the model
        // stay untouched. Collection order must not follow BTreeMap's ID ordering.
        let first_location = document.locations.remove(&first).unwrap();
        let last_location = document.locations.remove(&last).unwrap();
        document.locations.insert(last, first_location.clone());
        document.locations.insert(first, last_location.clone());
        assert_eq!(document.paragraph_ids().unwrap(), vec![last, first]);
        assert_eq!(document.paragraph_text(first).unwrap(), "Last");
        assert!(document.paragraph_text(last).unwrap().contains("First"));
        assert_eq!(document.bookmark_range("First").unwrap().0, last);
        assert_eq!(document.location(first).unwrap(), last_location);
        assert_eq!(document.location(last).unwrap(), first_location);
        assert_eq!(document.table_rows(table).unwrap(), vec![vec!["Cell"]]);
        assert_eq!(document.paragraph_text(cell).unwrap(), "Cell");
        assert_eq!(document.paragraph_text(auxiliary).unwrap(), "Header");
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!(
            (document.revision, document.next_block_id),
            (revision, next)
        );
        let copy = document.copy().unwrap();
        document.remove_image(picture).unwrap();
        assert_eq!(
            document.paragraph_ids().unwrap(),
            vec![last, picture, first]
        );
        assert_eq!(document.paragraph_text(picture).unwrap(), "");
        assert!(document.image_data(picture).is_err());
        assert_eq!(copy.image_data(picture).unwrap(), red_png());
        document.set_cell_text(table, 0, 0, "Replacement").unwrap();
        assert!(document.paragraph_text(cell).is_err());
        let fresh = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        assert!(fresh > last);
        assert_eq!(
            document.table_generation(table).unwrap(),
            copy.table_generation(table).unwrap()
        );
        assert_eq!(copy.paragraph_text(cell).unwrap(), "Cell");
        document.insert_row(table, 1).unwrap();
        assert!(document.paragraph_text(fresh).is_err());
        assert_eq!(
            document.table_generation(table).unwrap(),
            copy.table_generation(table).unwrap() + 1
        );
        assert_eq!(document.table_ids().unwrap(), vec![table]);
        document.clear_story("header", "default", 0).unwrap();
        assert!(document.paragraph_text(auxiliary).is_err());
        let fresh_auxiliary = document.story_add_paragraph(header, "Fresh").unwrap();
        assert!(fresh_auxiliary > fresh);
        assert!(document.paragraph_text(last).unwrap().contains("First"));
    }

    #[test]
    fn body_identity_preserves_hidden_section_markers_and_owned_placeholders() {
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 360.0).unwrap();
        document.set_margins(24.0, 24.0, 24.0, 24.0).unwrap();
        let table = document
            .add_table(vec![vec!["First section".into()]])
            .unwrap();
        let first_header = document.ensure_story("header", "default", 0).unwrap();
        let original_header = document.story_add_paragraph(first_header, "Head").unwrap();
        assert_eq!(document.add_section("next_page").unwrap(), 1);
        assert_eq!(document.add_section("next_page").unwrap(), 2);
        document
            .set_section_props(2, r#"{"page_size":[420,480]}"#)
            .unwrap();
        let last = document.add_paragraph("Last section").unwrap();
        assert_eq!(document.paragraph_ids().unwrap(), vec![last]);
        assert_eq!(document.table_ids().unwrap(), vec![table]);
        let model = document.document().unwrap();
        assert_eq!(
            model
                .sections()
                .iter()
                .map(|(end, _)| *end)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        for index in [1, 2] {
            assert!(model
                .para(StoryRef::Body, &Path::top(index))
                .unwrap()
                .section
                .is_some());
            assert!(!document
                .locations
                .values()
                .any(|value| value == &(StoryRef::Body, Path::top(index))));
        }
        assert_eq!(
            document.location(last).unwrap(),
            (StoryRef::Body, Path::top(3))
        );
        let later_header = document.story_id("header", "default", 2).unwrap().unwrap();
        let later_child = document.story_paragraph_ids(later_header).unwrap()[0];
        assert_ne!(later_child, original_header);
        document
            .set_paragraph_text(later_child, "Last head")
            .unwrap();
        assert_eq!(document.paragraph_text(original_header).unwrap(), "Head");
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened
                .sections()
                .iter()
                .map(|(end, _)| *end)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            reopened
                .para(StoryRef::Body, &document.location(last).unwrap().1)
                .unwrap()
                .text,
            "Last section"
        );
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let output = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(output.pages.len(), 3);
        for (index, (width, height)) in [(300, 360), (300, 360), (420, 480)].into_iter().enumerate()
        {
            assert_eq!(
                validate_png(&crate::docx::render(&bytes, index, 72.0, None).unwrap()).unwrap(),
                (width, height)
            );
            assert_eq!(
                output.pages[index].metadata["width"].as_f64(),
                Some(f64::from(width))
            );
            assert_eq!(
                output.pages[index].metadata["height"].as_f64(),
                Some(f64::from(height))
            );
        }
        let copy = document.copy().unwrap();
        document.close();
        assert!(document.paragraph_ids().is_err());
        assert_eq!(copy.paragraph_text(last).unwrap(), "Last section");
        assert_eq!(copy.table_rows(table).unwrap(), vec![vec!["First section"]]);
    }

    #[test]
    fn body_identity_failed_allocations_and_commits_publish_nothing() {
        let png = red_png();
        for failure in ["identifier", "revision", "placeholder", "collision"] {
            for kind in 0..3 {
                let mut document = EditableDocx::new();
                match failure {
                    "identifier" => document.next_block_id = usize::MAX,
                    "revision" => document.revision = u64::MAX,
                    "placeholder" => document.body_placeholder = Some(MAX_BLOCKS),
                    _ => {
                        document
                            .locations
                            .insert(document.next_block_id, (StoryRef::Body, Path::top(0)));
                    }
                }
                let before = document.copy().unwrap();
                let result = match kind {
                    0 => document.add_paragraph("New"),
                    1 => document.add_table(vec![vec!["New".into()]]),
                    _ => document.add_image(&png, Some(12.0), Some(12.0), "New"),
                };
                assert!(result.is_err(), "{failure}/{kind}");
                unchanged(&document, &before);
            }
        }
        let mut document = EditableDocx::new();
        let first = document.add_paragraph("First").unwrap();
        let second = document.add_paragraph("Second").unwrap();
        let (id, next) = document.next_identifier().unwrap();
        assert_eq!(next, id + 1);
        assert_eq!(id, second + 1);
        let before = document.copy().unwrap();
        assert!(document.add_paragraph("\0").is_err());
        assert!(document.add_table(vec![vec!["\0".into()]]).is_err());
        assert!(document
            .add_image(&png, Some(0.5), Some(12.0), "New")
            .is_err());
        unchanged(&document, &before);
        // Invalid reverse views are rejected, rather than exposing guessed positions.
        document
            .locations
            .insert(id, document.location(first).unwrap());
        assert!(document.paragraph_ids().is_err());
        assert!(document.table_ids().is_err());
        document.locations.remove(&id);
        document
            .locations
            .insert(id, (StoryRef::Body, Path::top(MAX_BLOCKS)));
        assert!(document.paragraph_ids().is_err());
        document.locations.remove(&id);
        assert_eq!(document.add_paragraph("Third").unwrap(), id);
    }
}

#[cfg(test)]
mod body_paragraph_delete_tests {
    use super::*;

    fn sections(document: &Document) -> Vec<SectionProps> {
        document
            .sections()
            .iter()
            .map(|(_, section)| (*section).clone())
            .collect()
    }

    #[test]
    fn body_delete_native_tail_requires_a_final_section_paragraph() {
        let mut document = EditableDocx::new();
        document.set_page_size(300.0, 360.0).unwrap();
        document.set_margins(24.0, 24.0, 24.0, 24.0).unwrap();
        document.add_paragraph("First section").unwrap();
        document.add_section("next_page").unwrap();
        document
            .set_section_props(1, r#"{"page_size":[420,480],"page_number_start":7}"#)
            .unwrap();
        let tail = document.add_paragraph("Remove final section text").unwrap();
        let mut raw = document.document().unwrap().clone();
        let wanted = sections(&raw);
        raw.remove_block(StoryRef::Body, &document.location(tail).unwrap().1)
            .unwrap();
        // Native remove_block only appends for empty/table-ended containers. A
        // section-bearing paragraph is already a paragraph, so the final section
        // now shares the prior section's endpoint rather than owning a body block.
        assert_eq!(raw.body.len(), 1);
        assert_eq!(
            raw.sections()
                .iter()
                .map(|(end, _)| *end)
                .collect::<Vec<_>>(),
            vec![0, 0]
        );
        assert_eq!(sections(&raw), wanted);
        raw.insert_block(
            StoryRef::Body,
            &Path::top(raw.body.len()),
            Block::Para(Paragraph::new()),
        )
        .unwrap();
        assert_eq!(raw.body.len(), 2);
        assert_eq!(
            raw.sections()
                .iter()
                .map(|(end, _)| *end)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(sections(&raw), wanted);
        let bytes = wordcraft_docx::write(&raw).unwrap();
        let read = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            read.sections()
                .iter()
                .map(|(end, _)| *end)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(read.body[1].as_para().unwrap().text.is_empty());
        assert!(read.body[1].as_para().unwrap().section.is_none());
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let output = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(output.pages.len(), 2);
        assert_eq!(output.pages[0].metadata["width"].as_f64(), Some(300.0));
        assert_eq!(output.pages[1].metadata["width"].as_f64(), Some(420.0));
        for (page, expected) in [(0, (300, 360)), (1, (420, 480))] {
            assert_eq!(
                validate_png(&crate::docx::render(&bytes, page, 72.0, None).unwrap()).unwrap(),
                expected
            );
        }
    }

    fn unchanged(actual: &EditableDocx, before: &EditableDocx) {
        assert_eq!(
            wordcraft_docx::write(actual.document().unwrap()).unwrap(),
            wordcraft_docx::write(before.document().unwrap()).unwrap()
        );
        assert_eq!(actual.locations, before.locations);
        assert_eq!(actual.body_placeholder, before.body_placeholder);
        assert_eq!(actual.cell_placeholders, before.cell_placeholders);
        assert_eq!(actual.story_placeholders, before.story_placeholders);
        assert_eq!(actual.stories, before.stories);
        assert_eq!(actual.images, before.images);
        assert_eq!(actual.table_generations, before.table_generations);
        assert_eq!(
            (
                actual.text_bytes,
                actual.cell_count,
                actual.media_bytes,
                actual.revision,
                actual.next_block_id
            ),
            (
                before.text_bytes,
                before.cell_count,
                before.media_bytes,
                before.revision,
                before.next_block_id
            )
        );
    }

    #[test]
    fn body_delete_shifts_owned_descendants_and_reuses_native_tail_without_new_ids() {
        let mut document = EditableDocx::new();
        let first = document.add_paragraph("First").unwrap();
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["Deep".into()]])
            .unwrap();
        let deep = document.cell_paragraph_ids(nested, 0, 0).unwrap()[0];
        let tail = document.add_paragraph("Tail").unwrap();
        let header = document.ensure_story("header", "default", 0).unwrap();
        let head = document.story_add_paragraph(header, "Head").unwrap();
        let copy = document.copy().unwrap();
        let path = document.location(deep).unwrap().1;
        let next = document.next_block_id;
        let bytes = document.text_bytes;
        document.remove_paragraph(first).unwrap();
        assert_eq!(document.text_bytes, bytes - 5);
        assert!(document.paragraph_text(first).is_err());
        assert_eq!(document.location(deep).unwrap().1 .0[0] + 1, path.0[0]);
        assert_eq!(document.location(deep).unwrap().1 .0[1..], path.0[1..]);
        assert_eq!(document.paragraph_text(cell).unwrap(), "Cell");
        assert_eq!(document.paragraph_text(deep).unwrap(), "Deep");
        assert_eq!(document.paragraph_text(head).unwrap(), "Head");
        assert_eq!(
            document.table_generation(table).unwrap(),
            copy.table_generation(table).unwrap()
        );
        assert_eq!(
            document.table_generation(nested).unwrap(),
            copy.table_generation(nested).unwrap()
        );
        document.remove_paragraph(tail).unwrap();
        assert_eq!(document.text_bytes, bytes - 9);
        assert_eq!(document.next_block_id, next);
        assert_eq!(document.body_placeholder, Some(1));
        assert!(document.paragraph_ids().unwrap().is_empty());
        assert_eq!(document.document().unwrap().body.len(), 2);
        assert_eq!(
            *document.document().unwrap().body[1],
            Block::Para(Paragraph::new())
        );
        let fresh = document.add_paragraph("").unwrap();
        assert_eq!(fresh, next);
        assert_eq!(document.paragraph_ids().unwrap(), vec![fresh]);
        document.remove_paragraph(fresh).unwrap();
        assert_eq!(document.document().unwrap().body.len(), 2);
        assert_eq!(document.body_placeholder, Some(1));
        assert_eq!(copy.paragraph_text(first).unwrap(), "First");
        assert_eq!(copy.paragraph_text(tail).unwrap(), "Tail");
        // A shifted nested-table trailing placeholder remains reusable by its cell.
        let before = document.cell_placeholders.len();
        document
            .cell_add_paragraph(table, 0, 0, "After nested")
            .unwrap();
        assert_eq!(document.cell_placeholders.len() + 1, before);
        let bytes = document.to_bytes().unwrap();
        let read = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            cell_plain_text(&read.table(StoryRef::Body, &Path::top(0)).unwrap().rows[0].cells[0]),
            "Cell\nDeep\nAfter nested"
        );
        assert!(read.body[1].as_para().unwrap().text.is_empty());
    }

    #[test]
    fn body_delete_preserves_all_section_values_and_boundary_handles() {
        for (start, pages) in [
            ("next_page", 2),
            ("continuous", 1),
            ("odd_page", 3),
            ("even_page", 2),
        ] {
            let mut document = EditableDocx::new();
            document.set_page_size(300.0, 360.0).unwrap();
            document.set_margins(24.0, 24.0, 24.0, 24.0).unwrap();
            let boundary = document.add_paragraph("Boundary").unwrap();
            let head = document.ensure_story("header", "default", 0).unwrap();
            document.story_add_paragraph(head, "Header").unwrap();
            document.add_section(start).unwrap();
            let tail = document.add_paragraph("Delete tail").unwrap();
            let expected = owned_body_sections(document.document().unwrap()).unwrap();
            let before = document.copy().unwrap();
            assert!(document.remove_paragraph(boundary).is_err());
            unchanged(&document, &before);
            document.remove_paragraph(tail).unwrap();
            assert_eq!(
                owned_body_sections(document.document().unwrap()).unwrap(),
                expected
            );
            assert_eq!(document.paragraph_ids().unwrap(), vec![boundary]);
            assert_eq!(document.body_placeholder, Some(1));
            assert_eq!(document.document().unwrap().body.len(), 2);
            assert_eq!(document.next_block_id, before.next_block_id);
            let bytes = document.to_bytes().unwrap();
            let read = wordcraft_docx::read(&bytes).unwrap();
            assert_eq!(read.sections().len(), 2);
            assert_eq!(read.sections()[1].1.start, expected[1].start);
            assert!(read.body[1].as_para().unwrap().text.is_empty());
            let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
            assert_eq!(
                crate::pdf::extract(&pdf, None).unwrap().pages.len(),
                pages,
                "{start}"
            );
            assert_eq!(
                validate_png(&crate::docx::render(&bytes, pages - 1, 72.0, None).unwrap()).unwrap(),
                (300, 360)
            );
            let fresh = document.add_paragraph("Fresh tail").unwrap();
            assert!(fresh > tail);
            assert_eq!(document.document().unwrap().body.len(), 2);
            assert_eq!(
                owned_body_sections(document.document().unwrap()).unwrap(),
                expected
            );
        }
        let mut empty = EditableDocx::new();
        let paragraph = empty.add_paragraph("Only").unwrap();
        let props = sections(empty.document().unwrap());
        empty.remove_paragraph(paragraph).unwrap();
        assert_eq!(sections(empty.document().unwrap()), props);
        assert_eq!(empty.document().unwrap().body.len(), 1);
        assert_eq!(empty.body_placeholder, Some(0));
        assert!(empty.paragraph_ids().unwrap().is_empty());
        let table = empty.add_table(vec![vec!["Recovered".into()]]).unwrap();
        assert_eq!(empty.location(table).unwrap().1, Path::top(0));
        assert_eq!(empty.document().unwrap().body.len(), 2);
    }

    #[test]
    fn body_delete_rejects_protected_anchors_and_malformed_state_atomically() {
        for protected in ["note", "bookmark"] {
            let mut document = EditableDocx::new();
            let preceding = document.add_paragraph("Before").unwrap();
            let target = document.add_paragraph("Target").unwrap();
            if protected == "note" {
                document
                    .add_note(target, 0, 6, document.revision, "Detail", "footnote")
                    .unwrap();
            } else {
                document
                    .set_bookmark("Target", target, 0, 6, document.revision)
                    .unwrap();
            }
            let anchors = semantic_anchors(document.document().unwrap());
            document.remove_paragraph(preceding).unwrap();
            assert_eq!(semantic_anchors(document.document().unwrap()), anchors);
            let before = document.copy().unwrap();
            assert!(document.remove_paragraph(target).is_err());
            unchanged(&document, &before);
        }
        for failure in 0..6 {
            let mut document = EditableDocx::new();
            let target = document.add_paragraph("Text").unwrap();
            match failure {
                0 => document.body_placeholder = Some(20),
                1 => {
                    document
                        .locations
                        .insert(usize::MAX, (StoryRef::Body, Path(vec![0; 52])));
                }
                2 => {
                    document
                        .cell_placeholders
                        .insert((StoryRef::Body, Path(vec![])));
                }
                3 => document.text_bytes = 0,
                4 => document.revision = u64::MAX,
                _ => {
                    document.document.as_mut().unwrap().last_section.page_w = f32::NAN;
                }
            }
            // NaN section equality is intentionally not accepted as preservation.
            let before = document.copy().unwrap();
            assert!(
                document.remove_paragraph(target).is_err(),
                "failure {failure}"
            );
            unchanged(&document, &before);
        }
        let mut document = EditableDocx::new();
        let target = document.add_paragraph("No allocation needed").unwrap();
        document.next_block_id = usize::MAX;
        document.remove_paragraph(target).unwrap();
        assert_eq!(document.next_block_id, usize::MAX);
        assert!(document.add_paragraph("Cannot allocate").is_err());
    }
}

#[cfg(test)]
mod paragraph_move_tests {
    use super::*;

    fn unchanged(actual: &EditableDocx, before: &EditableDocx) {
        assert_eq!(
            wordcraft_docx::write(actual.document().unwrap()).unwrap(),
            wordcraft_docx::write(before.document().unwrap()).unwrap()
        );
        assert_eq!(actual.locations, before.locations);
        assert_eq!(actual.stories, before.stories);
        assert_eq!(actual.cell_placeholders, before.cell_placeholders);
        assert_eq!(actual.story_placeholders, before.story_placeholders);
        assert_eq!(actual.body_placeholder, before.body_placeholder);
        assert_eq!(actual.table_generations, before.table_generations);
        assert_eq!(actual.images, before.images);
        assert_eq!(
            (
                actual.text_bytes,
                actual.cell_count,
                actual.media_bytes,
                actual.revision,
                actual.next_block_id
            ),
            (
                before.text_bytes,
                before.cell_count,
                before.media_bytes,
                before.revision,
                before.next_block_id
            )
        );
    }

    #[test]
    fn paragraph_move_body_retains_descendants_accounting_and_one_native_tail() {
        pyo3::prepare_freethreaded_python();
        let mut document = EditableDocx::new();
        let first = document.add_paragraph("First").unwrap();
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let nested = document
            .cell_add_table(table, 0, 0, vec![vec!["Deep".into()]])
            .unwrap();
        let deep = document.cell_paragraph_ids(nested, 0, 0).unwrap()[0];
        let last = document.add_paragraph("Last").unwrap();
        let before = document.copy().unwrap();
        let old_deep = document.location(deep).unwrap().1;
        document
            .move_paragraph(last, first, document.revision)
            .unwrap();
        assert_eq!(document.paragraph_ids().unwrap(), vec![last, first]);
        assert_eq!(document.body_placeholder, Some(3));
        assert_eq!(document.document().unwrap().body.len(), 4);
        assert_eq!(document.location(table).unwrap().1, Path::top(2));
        assert_eq!(document.location(deep).unwrap().1 .0[0], old_deep.0[0] + 1);
        assert_eq!(document.location(deep).unwrap().1 .0[1..], old_deep.0[1..]);
        assert_eq!(document.paragraph_text(cell).unwrap(), "Cell");
        assert_eq!(document.paragraph_text(deep).unwrap(), "Deep");
        assert_eq!(document.table_generations, before.table_generations);
        assert_eq!(
            (
                document.text_bytes,
                document.cell_count,
                document.media_bytes,
                document.next_block_id
            ),
            (
                before.text_bytes,
                before.cell_count,
                before.media_bytes,
                before.next_block_id
            )
        );
        let fresh = document.add_paragraph("Fresh").unwrap();
        assert_eq!(document.document().unwrap().body.len(), 4);
        assert!(document.body_placeholder.is_none());
        document
            .move_paragraph(first, fresh, document.revision)
            .unwrap();
        assert_eq!(document.paragraph_ids().unwrap(), vec![last, fresh, first]);
        assert_eq!(document.location(table).unwrap().1, Path::top(1));
        assert_eq!(before.paragraph_ids().unwrap(), vec![first, last]);
        let saved = document.to_bytes().unwrap();
        let read = wordcraft_docx::read(&saved).unwrap();
        assert_eq!(read.body.len(), 4);
        assert_eq!(read.body[0].as_para().unwrap().text, "Last");
        assert_eq!(read.body[3].as_para().unwrap().text, "First");
        document.next_block_id = usize::MAX;
        assert!(document
            .move_paragraph(first, last, document.revision)
            .unwrap());
        assert_eq!(document.next_block_id, usize::MAX);
    }

    #[test]
    fn paragraph_move_parts_keep_native_content_order_and_placeholders() {
        pyo3::prepare_freethreaded_python();
        for kind in ["header", "footer", "footnote", "endnote"] {
            let mut document = EditableDocx::new();
            let body = document.add_paragraph("Body").unwrap();
            let part = if matches!(kind, "header" | "footer") {
                document.ensure_story(kind, "default", 0).unwrap()
            } else {
                document
                    .add_note(body, 0, 4, document.revision, "Initial", kind)
                    .unwrap()
            };
            let first = if matches!(kind, "header" | "footer") {
                document.story_add_paragraph(part, "Initial").unwrap()
            } else {
                document.story_paragraph_ids(part).unwrap()[0]
            };
            let table_cell = if matches!(kind, "header" | "footer") {
                let table = document
                    .story_add_table(part, vec![vec!["Cell".into()]])
                    .unwrap();
                Some((table, document.cell_paragraph_ids(table, 0, 0).unwrap()[0]))
            } else {
                None
            };
            let last = document.story_add_paragraph(part, "Last").unwrap();
            let anchors = semantic_anchors(document.document().unwrap());
            let charges = (
                document.text_bytes,
                document.cell_count,
                document.media_bytes,
                document.next_block_id,
            );
            document
                .move_paragraph(last, first, document.revision)
                .unwrap();
            assert_eq!(
                document.story_paragraph_ids(part).unwrap(),
                vec![last, first]
            );
            if let Some((table, cell)) = table_cell {
                assert_eq!(document.story_placeholders[&part], 3);
                assert_eq!(document.paragraph_text(cell).unwrap(), "Cell");
                assert_eq!(document.location(table).unwrap().1, Path::top(2));
            } else {
                assert!(!document.story_placeholders.contains_key(&part));
            }
            assert_eq!(semantic_anchors(document.document().unwrap()), anchors);
            assert_eq!(
                (
                    document.text_bytes,
                    document.cell_count,
                    document.media_bytes,
                    document.next_block_id
                ),
                charges
            );
            let fresh = document.story_add_paragraph(part, "Fresh").unwrap();
            assert_eq!(
                document.document().unwrap().parts[&part].blocks.len(),
                if table_cell.is_some() { 4 } else { 3 }
            );
            document
                .move_paragraph(fresh, last, document.revision)
                .unwrap();
            assert_eq!(
                document.story_paragraph_ids(part).unwrap(),
                vec![fresh, last, first]
            );
            let read = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
            let matching = read
                .parts
                .values()
                .find(|p| p.kind == document.document().unwrap().parts[&part].kind)
                .unwrap();
            assert!(matching.blocks[0].as_para().unwrap().text.contains("Fresh"));
        }
    }

    #[test]
    fn paragraph_move_sections_noop_scope_and_anchors_are_explicit() {
        pyo3::prepare_freethreaded_python();
        for start in ["next_page", "continuous", "odd_page", "even_page"] {
            let mut document = EditableDocx::new();
            let first = document.add_paragraph("First").unwrap();
            let second = document.add_paragraph("Second").unwrap();
            let boundary = document.add_paragraph("Boundary").unwrap();
            document.add_section(start).unwrap();
            let last = document.add_paragraph("Last").unwrap();
            let expected = owned_body_sections(document.document().unwrap()).unwrap();
            document
                .move_paragraph(second, first, document.revision)
                .unwrap();
            assert_eq!(
                owned_body_sections(document.document().unwrap()).unwrap(),
                expected
            );
            for (a, b) in [(first, boundary), (boundary, first), (last, first)] {
                let before = document.copy().unwrap();
                assert!(document.move_paragraph(a, b, document.revision).is_err());
                unchanged(&document, &before);
            }
            let before = document.copy().unwrap();
            assert!(!document
                .move_paragraph(first, first, document.revision)
                .unwrap());
            unchanged(&document, &before);
        }
        let mut document = EditableDocx::new();
        let first = document.add_paragraph("Body").unwrap();
        let second = document.add_paragraph("Other").unwrap();
        document
            .add_note(first, 0, 4, document.revision, "Note", "footnote")
            .unwrap();
        let before = document.copy().unwrap();
        assert!(document
            .move_paragraph(first, second, document.revision)
            .is_err());
        unchanged(&document, &before);
        assert!(document
            .move_paragraph(second, first, document.revision)
            .unwrap());
        let table = document.add_table(vec![vec!["Cell".into()]]).unwrap();
        let cell = document.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        let before = document.copy().unwrap();
        assert!(document
            .move_paragraph(cell, cell, document.revision)
            .is_err());
        unchanged(&document, &before);
    }

    #[test]
    fn paragraph_move_corrupt_bookkeeping_and_failed_commit_roll_back() {
        pyo3::prepare_freethreaded_python();
        for failure in 0..8 {
            let mut document = EditableDocx::new();
            let first = document.add_paragraph("First").unwrap();
            let second = document.add_paragraph("Second").unwrap();
            match failure {
                0 => document.revision = u64::MAX,
                1 => {
                    document
                        .locations
                        .insert(998, (StoryRef::Body, Path(vec![])));
                }
                2 => {
                    document
                        .locations
                        .insert(998, (StoryRef::Body, Path::top(99)));
                }
                3 => {
                    document
                        .locations
                        .insert(998, (StoryRef::Body, Path::top(0)));
                }
                4 => {
                    document
                        .cell_placeholders
                        .insert((StoryRef::Body, Path(vec![0, 0, 0, 0])));
                }
                5 => document.body_placeholder = Some(99),
                6 => document.body_placeholder = Some(0),
                _ => {
                    document
                        .document
                        .as_mut()
                        .unwrap()
                        .insert_object(
                            &Pos::new(StoryRef::Body, Path::top(0), 0),
                            InlineObject::BookmarkStart {
                                name: "broken".into(),
                            },
                            &CharProps::default(),
                        )
                        .unwrap();
                }
            }
            let before = document.copy().unwrap();
            assert!(
                document
                    .move_paragraph(second, first, document.revision)
                    .is_err(),
                "failure {failure}"
            );
            unchanged(&document, &before);
        }
        let mut document = EditableDocx::new();
        document.add_paragraph("Body").unwrap();
        let part = document.ensure_story("header", "default", 0).unwrap();
        let first = document.story_add_paragraph(part, "First").unwrap();
        let second = document.story_add_paragraph(part, "Second").unwrap();
        document.stories.get_mut(&part).unwrap().push(999);
        let before = document.copy().unwrap();
        assert!(document
            .move_paragraph(first, second, document.revision)
            .is_err());
        unchanged(&document, &before);
        let mut document = EditableDocx::new();
        document.add_paragraph("Body").unwrap();
        let part = document.ensure_story("header", "default", 0).unwrap();
        let first = document.story_add_paragraph(part, "First").unwrap();
        let second = document.story_add_paragraph(part, "Second").unwrap();
        document
            .document
            .as_mut()
            .unwrap()
            .last_section
            .headers
            .first = Some(part);
        let before = document.copy().unwrap();
        assert!(document
            .move_paragraph(first, second, document.revision)
            .is_err());
        unchanged(&document, &before);
        for object in [
            InlineObject::BookmarkStart {
                name: "Guard".into(),
            },
            InlineObject::CommentStart { id: 9 },
        ] {
            let mut document = EditableDocx::new();
            let first = document.add_paragraph("First").unwrap();
            let second = document.add_paragraph("Second").unwrap();
            document
                .document
                .as_mut()
                .unwrap()
                .insert_object(
                    &Pos::new(StoryRef::Body, Path::top(0), 0),
                    object,
                    &CharProps::default(),
                )
                .unwrap();
            let before = document.copy().unwrap();
            assert!(document
                .move_paragraph(first, second, document.revision)
                .is_err());
            unchanged(&document, &before);
        }
    }
}

#[cfg(test)]
mod paragraph_move_raw_proof {
    use super::*;

    fn order(document: &Document) -> Vec<String> {
        document
            .body
            .iter()
            .map(|block| match block.as_ref() {
                Block::Para(paragraph) => paragraph.text.clone(),
                Block::Table(_) => "<table>".to_owned(),
            })
            .collect()
    }

    fn para(text: &str) -> Arc<Block> {
        wordcraft_doc::para_block(Paragraph::with_text(text, CharProps::default()))
    }

    #[test]
    fn raw_native_move_order_sentinel_and_readback() {
        for (labels, source, destination, expected) in [
            (vec!["A", "B", "C"], 0, 2, vec!["B", "C", "A"]),
            (vec!["A", "B", "C"], 2, 0, vec!["C", "A", "B"]),
            (
                vec!["A", "<table>", "B", "C"],
                0,
                2,
                vec!["<table>", "B", "A", "C"],
            ),
            (
                vec!["A", "<table>", "B", "C"],
                2,
                0,
                vec!["B", "A", "<table>", "C"],
            ),
            (
                vec!["A", "<table>", "B"],
                2,
                0,
                vec!["B", "A", "<table>", ""],
            ),
            (vec!["A", "<table>", "B"], 0, 2, vec!["<table>", "B", "A"]),
        ] {
            let mut document = EditableDocx::new().document().unwrap().clone();
            document.body = labels
                .iter()
                .map(|text| {
                    if *text == "<table>" {
                        Arc::new(Block::Table(Table::new(1, 1, 200.0)))
                    } else {
                        para(text)
                    }
                })
                .collect();
            let selected = document.body[source].as_ref().clone();
            let removed = document
                .remove_block(StoryRef::Body, &Path::top(source))
                .unwrap();
            assert_eq!(removed, selected);
            document
                .insert_block(StoryRef::Body, &Path::top(destination), removed)
                .unwrap();
            assert_eq!(order(&document), expected);
            assert_eq!(*document.body[destination], selected);
            assert!(document.body.last().unwrap().as_para().is_some());
            assert_eq!(owned_body_sections(&document).unwrap().len(), 1);
            let saved = wordcraft_docx::write(&document).unwrap();
            let read = wordcraft_docx::read(&saved).unwrap();
            assert_eq!(order(&read), expected);
        }
        // The adapter must treat identical IDs as no-op before native mutation.
        let mut singleton = EditableDocx::new().document().unwrap().clone();
        singleton.body = vec![para("Only")];
        let removed = singleton
            .remove_block(StoryRef::Body, &Path::top(0))
            .unwrap();
        singleton
            .insert_block(StoryRef::Body, &Path::top(0), removed)
            .unwrap();
        assert_eq!(order(&singleton), vec!["Only", ""]);
    }

    #[test]
    fn raw_native_move_section_membership_requires_target_boundary_guard() {
        let mut author = EditableDocx::new();
        let first = author.add_paragraph("First").unwrap();
        let boundary = author.add_paragraph("Boundary").unwrap();
        author.add_section("next_page").unwrap();
        author.add_paragraph("Last").unwrap();
        let mut document = author.document().unwrap().clone();
        let source = author.location(first).unwrap().1.last();
        let destination = author.location(boundary).unwrap().1.last();
        let sections = owned_body_sections(&document).unwrap();
        assert_eq!(source, 0);
        assert_eq!(destination, 1);
        assert_eq!(document.section_index_of(source), 0);
        assert_eq!(document.section_index_of(destination), 0);
        assert!(document.body[destination]
            .as_para()
            .unwrap()
            .section
            .is_some());
        let removed = document
            .remove_block(StoryRef::Body, &Path::top(source))
            .unwrap();
        document
            .insert_block(StoryRef::Body, &Path::top(destination), removed)
            .unwrap();
        assert_eq!(order(&document), vec!["Boundary", "First", "Last"]);
        // Merely checking equal section properties/count does not catch the crossing.
        assert_eq!(owned_body_sections(&document).unwrap(), sections);
        assert_eq!(document.section_index_of(destination), 1);
        let read = wordcraft_docx::read(&wordcraft_docx::write(&document).unwrap()).unwrap();
        assert_eq!(read.section_index_of(destination), 1);
    }
}
