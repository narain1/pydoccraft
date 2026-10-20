//! Native-owned DOCX creation. Python never reconstructs this model from extraction.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    sync::Arc,
};

use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use serde_json::{json, Map, Value};
use wordcraft_doc::para::{Float, InlineObject, NoteKind};
use wordcraft_doc::props::{
    Border, BorderStyle, Borders, HeightRule, Highlight, LineSpacing, NumRef, TextDirection,
    Underline, VAlign, VMerge, VertAlign,
};
use wordcraft_doc::section::{NotePos, NoteRestart, NumFormat, SectionProps, SectionStart};
use wordcraft_doc::styles::{Style, StyleSheet, TablePart};
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
const MAX_LINK_BYTES: usize = 16 * 1024 * 1024;
const MAX_STYLES: usize = 1024;
const MAX_FORMAT_PATCH_BYTES: usize = 16 * 1024;
const MAX_SECTIONS: usize = 128;
const MAX_NOTES: usize = 10_000;
const MAX_TABLE_PATCH_BYTES: usize = 128 * 1024;
const MAX_TABLE_METADATA_BYTES: usize = 16 * 1024 * 1024;

#[pyclass]
pub struct EditableDocx {
    document: Option<Document>,
    placeholder: Option<usize>,
    text_bytes: usize,
    cell_count: usize,
    revision: u64,
    images: Vec<usize>,
    media_bytes: usize,
    locations: BTreeMap<usize, (StoryRef, Path)>,
    stories: BTreeMap<u32, Vec<usize>>,
    next_story_block: usize,
    cell_placeholders: BTreeSet<(StoryRef, Path)>,
    table_generations: BTreeMap<usize, u64>,
}

#[pymethods]
impl EditableDocx {
    #[new]
    pub fn new() -> Self {
        Self {
            document: Some(Document::new()),
            placeholder: Some(0),
            text_bytes: 0,
            cell_count: 0,
            revision: 0,
            images: Vec::new(),
            media_bytes: 0,
            locations: BTreeMap::new(),
            stories: BTreeMap::new(),
            next_story_block: MAX_BLOCKS,
            cell_placeholders: BTreeSet::new(),
            table_generations: BTreeMap::new(),
        }
    }

    pub fn copy(&self) -> PyResult<Self> {
        Ok(Self {
            document: Some(self.document()?.clone()),
            placeholder: self.placeholder,
            text_bytes: self.text_bytes,
            cell_count: self.cell_count,
            revision: self.revision,
            images: self.images.clone(),
            media_bytes: self.media_bytes,
            locations: self.locations.clone(),
            stories: self.stories.clone(),
            next_story_block: self.next_story_block,
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

    pub fn to_bytes(&self) -> PyResult<Vec<u8>> {
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
        let mut candidate = self.document()?.clone();
        let paragraph = Block::Para(Paragraph::with_text(text, CharProps::default()));
        let id = if let Some(id) = self.placeholder {
            candidate.body[id] = Arc::new(paragraph);
            id
        } else {
            if candidate.body.len() >= MAX_BLOCKS {
                return Err(limit("document exceeds 100000 blocks"));
            }
            let id = candidate.body.len();
            candidate
                .insert_block(StoryRef::Body, &Path::top(id), paragraph)
                .map_err(model_error)?;
            id
        };
        self.commit(candidate)?;
        self.locations.insert(id, (StoryRef::Body, Path::top(id)));
        self.placeholder = None;
        self.text_bytes = text_bytes;
        Ok(id)
    }

    pub fn paragraph_ids(&self) -> PyResult<Vec<usize>> {
        Ok(self
            .document()?
            .body
            .iter()
            .enumerate()
            .filter_map(|(id, block)| {
                (block.as_para().is_some()
                    && self.locations.contains_key(&id)
                    && self.placeholder != Some(id)
                    && !self.images.contains(&id))
                .then_some(id)
            })
            .collect())
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
        let (table, added_text, cells) =
            self.make_table(rows, column_width(&self.document()?.last_section))?;
        let text_bytes = self.updated_text_bytes(0, added_text)?;
        let mut candidate = self.document()?.clone();
        if candidate.body.len() + 2 > MAX_BLOCKS {
            return Err(limit("document exceeds 100000 blocks"));
        }
        let id = self.placeholder.unwrap_or(candidate.body.len());
        if self.placeholder.is_some() {
            candidate.body[id] = Arc::new(Block::Table(table));
        } else {
            candidate
                .insert_block(StoryRef::Body, &Path::top(id), Block::Table(table))
                .map_err(model_error)?;
        }
        let placeholder = candidate.body.len();
        candidate
            .body
            .push(wordcraft_doc::para_block(Paragraph::new()));
        self.commit(candidate)?;
        self.locations.insert(id, (StoryRef::Body, Path::top(id)));
        self.placeholder = Some(placeholder);
        self.text_bytes = text_bytes;
        self.cell_count += cells;
        Ok(id)
    }

    pub fn table_ids(&self) -> PyResult<Vec<usize>> {
        Ok(self
            .document()?
            .body
            .iter()
            .enumerate()
            .filter_map(|(id, block)| block.as_table().map(|_| id))
            .collect())
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
        let (paragraph, media) = self.make_image(data, width, height, alt)?;
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
            Some((media, data.to_vec())),
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
        Ok(json!({"id":style.id,"name":style.name,"kind":match style.kind {StyleKind::Paragraph => "paragraph", StyleKind::Character => "character", StyleKind::Table => "table", _ => "unsupported"},"based_on":style.based_on,"builtin":style.builtin}).to_string())
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
        if kind == StyleKind::Table {
            validate_style_tables(self.document()?, &candidate)?;
        }
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
        self.commit(candidate)
    }

    pub fn paragraph_list(&self, id: usize) -> PyResult<Option<(&'static str, u8)>> {
        let paragraph = self.paragraph(id)?;
        let document = self.document()?;
        let Some(numbering) = document.styles.resolve_para(&paragraph.props).numbering else {
            return Ok(None);
        };
        for (kind, name) in [
            (ListKind::Bullet, "bullet"),
            (ListKind::Numbered, "numbered"),
        ] {
            let wanted = wordcraft_doc::numbering::levels_for(kind);
            if document
                .numbering
                .level(numbering.num, 0)
                .is_some_and(|level| {
                    level.format == wanted[0].format && level.text == wanted[0].text
                })
            {
                return Ok(Some((name, numbering.level)));
            }
        }
        Err(PyValueError::new_err(
            "paragraph uses an unsupported list style",
        ))
    }

    #[pyo3(signature = (id, kind=None, level=0, restart=false))]
    pub fn set_paragraph_list(
        &mut self,
        id: usize,
        kind: Option<&str>,
        level: u8,
        restart: bool,
    ) -> PyResult<()> {
        self.paragraph(id)?;
        if level > 8 || (kind.is_none() && (level != 0 || restart)) {
            return Err(PyValueError::new_err(
                "list level must be 0..8; clearing a list requires level=0 and restart=false",
            ));
        }
        let kind = match kind {
            Some("bullet") => Some(ListKind::Bullet),
            Some("numbered") => Some(ListKind::Numbered),
            None => None,
            _ => {
                return Err(PyValueError::new_err(
                    "list kind must be bullet, numbered or None",
                ))
            }
        };
        let mut candidate = self.document()?.clone();
        let num = if let Some(kind) = kind {
            let (story, path) = self.location(id)?;
            let wanted = wordcraft_doc::numbering::levels_for(kind);
            // Preserve the current sequence or continue the closest earlier matching list
            // in this story, including a restart. find_kind always returns the first one.
            let existing = if restart {
                None
            } else {
                candidate
                    .para_paths(story)
                    .into_iter()
                    .take_while(|previous| previous <= &path)
                    .filter_map(|previous| {
                        let paragraph = candidate.para(story, &previous)?;
                        let reference = candidate.styles.resolve_para(&paragraph.props).numbering?;
                        candidate
                            .numbering
                            .level(reference.num, 0)
                            .filter(|level| {
                                level.format == wanted[0].format && level.text == wanted[0].text
                            })
                            .map(|_| reference.num)
                    })
                    .last()
            };
            if let Some(existing) = existing {
                existing
            } else {
                if candidate.numbering.nums.len() >= MAX_LISTS
                    || candidate.numbering.abstracts.len() >= MAX_LISTS
                {
                    return Err(limit("document exceeds 10000 native list definitions"));
                }
                candidate
                    .numbering
                    .try_add_list(kind)
                    .map_err(model_error)?
            }
        } else {
            0
        };
        let paragraph = self.paragraph_mut(&mut candidate, id)?;
        // NumRef 0 explicitly cancels any numbering inherited from the paragraph style.
        paragraph.props.numbering = Some(NumRef { num, level });
        paragraph.touch();
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
        let text = wordcraft_doc::para::PAGE_BREAK.to_string();
        let text_bytes = candidate.updated_text_bytes(0, text.len())?;
        let mut document = candidate.document()?.clone();
        document
            .insert_text(&candidate.position(id, 0)?, &text, &CharProps::default())
            .map_err(model_error)?;
        candidate.commit(document)?;
        candidate.text_bytes = text_bytes;
        *self = candidate;
        Ok(id)
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
            if url.len() > 8192
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
            "page_size": [props.page_w, props.page_h],
            "margins": [props.margin_top, props.margin_right, props.margin_bottom, props.margin_left],
            "distances": [props.header, props.footer],
            "columns": [props.columns.count, props.columns.space, props.columns.separator]
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
        staged.placeholder = Some(placeholder);
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
        let (paragraph, media) = self.make_image(data, width, height, alt)?;
        validate_picture_width(&paragraph, column_width(&self.document()?.last_section))?;
        let mut candidate = self.document()?.clone();
        if candidate.body.len() >= MAX_BLOCKS {
            return Err(limit("document exceeds 100000 blocks"));
        }
        candidate.media.insert(media, Arc::new(data.to_vec()));
        let id = self.placeholder.unwrap_or(candidate.body.len());
        if self.placeholder.is_some() {
            candidate.body[id] = wordcraft_doc::para_block(paragraph);
        } else {
            candidate
                .insert_block(StoryRef::Body, &Path::top(id), Block::Para(paragraph))
                .map_err(model_error)?;
        }
        self.commit(candidate)?;
        self.placeholder = None;
        self.images.push(id);
        self.locations.insert(id, (StoryRef::Body, Path::top(id)));
        self.media_bytes += data.len();
        Ok(id)
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
        Ok(id)
    }

    pub fn story_paragraph_ids(&self, story: u32) -> PyResult<Vec<usize>> {
        self.document()?;
        self.stories
            .get(&story)
            .cloned()
            .ok_or_else(|| PyIndexError::new_err("story index does not identify an owned story"))
    }

    pub fn story_add_paragraph(&mut self, story: u32, text: &str) -> PyResult<usize> {
        self.reserve_block()?;
        let mut ids = self.story_paragraph_ids(story)?;
        validate_text(text)?;
        let text_bytes = self.updated_text_bytes(0, text.len())?;
        let id = self.next_story_block;
        let next = id
            .checked_add(1)
            .ok_or_else(|| limit("document child identifier limit reached"))?;
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
        let index = if ids.is_empty() {
            part.blocks[0] = wordcraft_doc::para_block(paragraph);
            0
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
        self.next_story_block = next;
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
        let id = self.next_story_block;
        self.next_story_block = id
            .checked_add(1)
            .ok_or_else(|| limit("document child identifier limit reached"))?;
        self.locations.insert(id, (story, path));
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
        block: Block,
        added_text: usize,
        added_cells: usize,
        media: Option<(String, Vec<u8>)>,
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
        let identifier = self.next_story_block;
        let next = identifier
            .checked_add(1)
            .ok_or_else(|| limit("document child identifier limit reached"))?;
        let is_table = block.as_table().is_some();
        let is_image = media.is_some();
        let media_bytes = media.as_ref().map_or(0, |(_, data)| data.len());
        let mut candidate = self.document()?.clone();
        if let Some((key, bytes)) = media {
            candidate.media.insert(key, Arc::new(bytes));
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
        self.next_story_block = next;
        self.text_bytes = text_bytes;
        self.cell_count += added_cells;
        self.media_bytes += media_bytes;
        Ok(identifier)
    }

    fn make_image(
        &self,
        data: &[u8],
        width: Option<f64>,
        height: Option<f64>,
        alt: &str,
    ) -> PyResult<(Paragraph, String)> {
        if self.document()?.media.len() >= 1000
            || self.media_bytes.saturating_add(data.len()) > MAX_MEDIA_BYTES
        {
            return Err(limit(
                "document images exceed 1000 images or 64 MiB of encoded media",
            ));
        }
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
        let media = format!("pydoccraft-image-{}.png", self.document()?.media.len());
        let mut paragraph = Paragraph::new();
        paragraph
            .insert_object(
                0,
                InlineObject::Image {
                    media: media.clone(),
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
        Ok((paragraph, media))
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
        validate_table_metadata_budget(&document)?;
        // Native generic range/table edits do not repair semantic references. Existing
        // marker identities and multiplicities must survive every ordinary mutation.
        // New notes may add an identity; removing or duplicating one is unsupported.
        let existing = semantic_anchors(self.document()?);
        if !existing.is_empty() {
            let next = semantic_anchors(&document);
            if existing
                .iter()
                .any(|(key, count)| next.get(key) != Some(count))
            {
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
            .filter(|_| self.placeholder != Some(id) && !self.images.contains(&id))
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
        *side = border_value(value)?;
    }
    *target = (borders != Borders::default()).then_some(borders);
    Ok(())
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
                if cell.props.margins.is_some() && inner <= 0.0 {
                    return Err(PyValueError::new_err(
                        "table layout must leave positive width inside explicit cell margins",
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
    if paragraph
        .object_offsets()
        .into_iter()
        .zip(&paragraph.objects)
        .any(|(offset, object)| {
            start <= offset
                && offset < end
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
            "borders" => apply_border_patch(&mut format.borders, &value, true)?,
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
    if values
        .values()
        .any(|value| value.is_array() || value.is_object())
    {
        return Err(PyValueError::new_err("format patch values must be scalar"));
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
    )
}

#[derive(Default)]
struct ParaPatch {
    values: ParaProps,
    clear: BTreeSet<String>,
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
                _ => unreachable!(),
            }
        }
        Ok(patch)
    }

    fn apply(&self, props: &mut ParaProps) {
        for key in &self.clear {
            match key.as_str() {
                "alignment" => props.align = None,
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
                _ => {}
            }
        }
        props.overlay(&self.values);
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
        ("alignment", json!(props.align.map(alignment_name))),
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
        ("alignment", json!(alignment_name(props.align))),
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
        .any(|value| !value.is_finite() || !(0.05..=1584.0).contains(value))
    {
        return Err(PyValueError::new_err(
            "image dimensions must be finite and between 0.05 and 1584 points",
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
    (section.text_width() - section.columns.space * (section.columns.count - 1) as f32)
        / section.columns.count as f32
}

fn validate_section_geometry(section: &SectionProps) -> PyResult<()> {
    if section.page_w < section.margin_left + section.margin_right + section.gutter + 18.0
        || section.page_h < section.margin_top + section.margin_bottom + 18.0
        || section.header > section.page_h
        || section.footer > section.page_h
        || column_width(section) < 18.0
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

fn content_usage(blocks: &wordcraft_doc::Blocks) -> (usize, usize) {
    let mut pending = vec![blocks];
    let (mut bytes, mut cells) = (0, 0);
    while let Some(blocks) = pending.pop() {
        for block in blocks {
            match block.as_ref() {
                Block::Para(paragraph) => {
                    bytes += paragraph
                        .text
                        .chars()
                        .filter(|value| *value != wordcraft_doc::para::OBJ)
                        .map(char::len_utf8)
                        .sum::<usize>()
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
            .para(StoryRef::Body, &Path::top(paragraph))
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
        let paragraph = model.para(StoryRef::Body, &Path::top(id)).unwrap();
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
            .para(StoryRef::Body, &Path::top(image))
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
        let merged = reopened.table(StoryRef::Body, &Path::top(table)).unwrap();
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
        let outer = reopened.table(StoryRef::Body, &Path::top(table)).unwrap();
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
        assert_eq!(document.document().unwrap().media.len(), 2);
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
        let native = reopened.table(StoryRef::Body, &Path::top(table)).unwrap();
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
            .table(StoryRef::Body, &Path::top(table))
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
            .para(StoryRef::Body, &Path::top(first))
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
    fn native_lists_continue_restart_and_clear_without_rebuilding_text() {
        let mut document = EditableDocx::new();
        let mut ids = Vec::new();
        for (index, label) in ["First", "Second", "Restart", "Continued"]
            .iter()
            .enumerate()
        {
            let id = document.add_paragraph(label).unwrap();
            document
                .set_paragraph_list(id, Some("numbered"), 0, index == 2)
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
            .set_paragraph_list(ids[0], Some("numbered"), 0, false)
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
            .set_paragraph_list(sub, Some("bullet"), 2, false)
            .unwrap();
        assert_eq!(document.paragraph_list(sub).unwrap(), Some(("bullet", 2)));
        let revision = document.revision().unwrap();
        assert!(document
            .set_paragraph_list(sub, Some("bullet"), 9, false)
            .is_err());
        assert!(document
            .set_paragraph_list(sub, Some("unsupported"), 0, false)
            .is_err());
        assert!(document.set_paragraph_list(sub, None, 0, true).is_err());
        assert_eq!(document.revision().unwrap(), revision);
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let mut counters = wordcraft_doc::numbering::Counters::default();
        let labels = ids
            .iter()
            .map(|id| {
                let reference = reopened
                    .para(StoryRef::Body, &Path::top(*id))
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
        document.set_paragraph_list(sub, None, 0, false).unwrap();
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
            .set_paragraph_list(header, Some("numbered"), 0, false)
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
            .set_paragraph_list(id, Some("numbered"), 0, true)
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
        let paragraph = reopened.para(StoryRef::Body, &Path::top(id)).unwrap();
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
            .para_mut(StoryRef::Body, &Path::top(id))
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
                    reopened.para(StoryRef::Body, &Path::top(id)).unwrap().text,
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
            .para(StoryRef::Body, &Path::top(paragraph))
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
            .table(StoryRef::Body, &Path::top(table))
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
        let native = reopened.table(StoryRef::Body, &Path::top(table)).unwrap();
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
        let native = reopened.table(StoryRef::Body, &Path::top(table)).unwrap();
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
        validate_editable_grid(reopened.table(StoryRef::Body, &Path::top(table)).unwrap()).unwrap();
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
        let native = reopened.table(StoryRef::Body, &Path::top(table)).unwrap();
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
                if path == &Path::top(table) {
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
            (native_cell_width(&widths, StoryRef::Body, &Path::top(table), 0, 0).unwrap() - 100.0)
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
        document.cell_format_patch(table,0,0,r##"{"background":"#ABCDEF","margins":[2,3,4,5],"alignment":"center","vertical_alignment":"bottom","text_direction":"down","no_wrap":true,"borders":{"left":{"style":"wave","color":"#123456","width":1},"top":{"style":"none"}}}"##).unwrap();
        let value: Value =
            serde_json::from_str(&document.cell_formatting(table, 0, 0).unwrap()).unwrap();
        assert_eq!(value["background"], "#ABCDEF");
        assert_eq!(value["margins"], json!([2.0, 3.0, 4.0, 5.0]));
        assert_eq!(value["text_direction"], "down");
        assert_eq!(value["borders"]["top"]["style"], "none");
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        let cell = &reopened
            .table(StoryRef::Body, &Path::top(table))
            .unwrap()
            .rows[0]
            .cells[0];
        assert_eq!(cell.props.margins, Some([2.0, 5.0, 4.0, 3.0]));
        assert_eq!(cell.props.text_direction, TextDirection::Down);
        assert!(cell.props.no_wrap);
        assert_eq!(
            cell.props.borders.unwrap().left.unwrap().style,
            BorderStyle::Wave
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
        table: usize,
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
                } if path == &Path::top(table) && *r == row && *c == cell => Some(rect),
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
                table_style_cell_fill(&layout, table, r, c),
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
                if path == &Path::top(table) && *cell < 2 {
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
            table_style_cell_fill(&bands, table, 2, 1),
            Some(parse_rgb("#333333").unwrap())
        );
        assert_eq!(
            table_style_cell_fill(&bands, table, 3, 1),
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
            table_style_cell_fill(&layout, table, 0, 0),
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
            table_style_cell_fill(&layout, table, 2, 0),
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
            table_style_cell_fill(&layout, table, 0, 1),
            Some(parse_rgb("#C0D0E0").unwrap())
        );
        let bytes = document.to_bytes().unwrap();
        let reopened = wordcraft_docx::read(&bytes).unwrap();
        assert_eq!(
            reopened
                .table(StoryRef::Body, &Path::top(table))
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
        assert_eq!(table_style_cell_fill(&layout, table, 0, 1), None);
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
}
