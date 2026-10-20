//! Native-owned DOCX creation. Python never reconstructs this model from extraction.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    sync::Arc,
};

use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use serde_json::{json, Map, Value};
use wordcraft_doc::para::{Float, InlineObject};
use wordcraft_doc::props::{
    Border, BorderStyle, Borders, HeightRule, Highlight, LineSpacing, NumRef, Underline, VAlign,
    VMerge, VertAlign,
};
use wordcraft_doc::styles::{Style, StyleSheet};
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
            self.make_table(rows, self.document()?.last_section.text_width())?;
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
        self.commit(candidate)
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
        self.table(id)?;
        let color = parse_rgb(color)?;
        if !width.is_finite() || !(0.125..=12.0).contains(&width) || (width * 8.0).fract() != 0.0 {
            return Err(PyValueError::new_err(
                "border width must be 0.125..12 points in eighth-point increments",
            ));
        }
        let mut candidate = self.document()?.clone();
        let mut border = Border::single(width as f32);
        border.color = Some(color);
        self.table_mut(&mut candidate, id)?.props.borders = Some(Borders::all(border));
        self.commit(candidate)
    }

    pub fn clear_table_borders(&mut self, id: usize) -> PyResult<()> {
        self.table(id)?;
        let mut candidate = self.document()?.clone();
        let border = Border {
            style: BorderStyle::None,
            width: 0.0,
            color: None,
            space: 0.0,
        };
        self.table_mut(&mut candidate, id)?.props.borders = Some(Borders::all(border));
        self.commit(candidate)
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
        let start = table.grid_col(anchor_row, cell);
        let end = start + table.rows[anchor_row].cells[cell].span();
        let margins = wordcraft_layout::cell_margins(self.document()?, table, anchor_row, cell);
        let width = table.grid[start..end].iter().sum::<f32>() - margins[1] - margins[3];
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
        let before = table.rows.iter().map(|row| row.cells.len()).sum::<usize>();
        let mut candidate = self.document()?.clone();
        let table = self.table_mut(&mut candidate, id)?;
        table.merge(r0, r1, c0, c1);
        let after = table.rows.iter().map(|row| row.cells.len()).sum::<usize>();
        self.commit(candidate)?;
        let (story, path) = self.location(id)?;
        self.invalidate_descendants(story, &path);
        self.cell_count = self.cell_count - before + after;
        Ok(())
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
        let mut candidate = self.document()?.clone();
        let table = self.table_mut(&mut candidate, id)?;
        table.split_cell(row, cell_index, columns);
        if table.rows[row].cells.len() != before + columns - 1 {
            return Err(EngineError::Unsupported(
                "upstream could not split the selected cell exactly".into(),
            )
            .into());
        }
        self.commit(candidate)?;
        let (story, path) = self.location(id)?;
        self.invalidate_descendants(story, &path);
        self.cell_count += columns - 1;
        Ok(())
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
        let (row, cell) = cell_anchor(self.table(id)?, row, column)?;
        let color = color.map(parse_rgb).transpose()?;
        let mut candidate = self.document()?.clone();
        self.table_mut(&mut candidate, id)?.rows[row].cells[cell]
            .props
            .shading = color;
        self.commit(candidate)
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
        let (row, cell) = cell_anchor(self.table(id)?, row, column)?;
        let alignment = match alignment {
            "top" => VAlign::Top,
            "center" => VAlign::Center,
            "bottom" => VAlign::Bottom,
            _ => {
                return Err(PyValueError::new_err(
                    "vertical alignment must be top, center or bottom",
                ))
            }
        };
        let mut candidate = self.document()?.clone();
        self.table_mut(&mut candidate, id)?.rows[row].cells[cell]
            .props
            .valign = Some(alignment);
        self.commit(candidate)
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
                matches!(style.kind, StyleKind::Paragraph | StyleKind::Character)
                    && wanted.is_none_or(|kind| kind == style.kind)
            })
            .map(|style| style.id.clone())
            .collect())
    }

    pub fn style_info(&self, id: &str) -> PyResult<String> {
        let style = supported_style(&self.document()?.styles, id)?;
        Ok(json!({"id":style.id,"name":style.name,"kind":if style.kind == StyleKind::Paragraph {"paragraph"} else {"character"},"based_on":style.based_on,"builtin":style.builtin}).to_string())
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
        self.commit(candidate)
    }

    pub fn format_style(&mut self, id: &str, patch: &str) -> PyResult<()> {
        let document = self.document()?;
        let style = supported_style(&document.styles, id)?;
        let id = style.id.clone();
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

    #[pyo3(signature = (id, resolved=false))]
    pub fn style_formatting(&self, id: &str, resolved: bool) -> PyResult<String> {
        let document = self.document()?;
        let style = supported_style(&document.styles, id)?;
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

    pub fn page_size(&self) -> PyResult<(f64, f64)> {
        let section = &self.document()?.last_section;
        Ok((f64::from(section.page_w), f64::from(section.page_h)))
    }

    pub fn margins(&self) -> PyResult<(f64, f64, f64, f64)> {
        let section = &self.document()?.last_section;
        Ok((
            f64::from(section.margin_top),
            f64::from(section.margin_right),
            f64::from(section.margin_bottom),
            f64::from(section.margin_left),
        ))
    }

    pub fn set_margins(&mut self, top: f64, right: f64, bottom: f64, left: f64) -> PyResult<()> {
        let section = &self.document()?.last_section;
        if [top, right, bottom, left]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
            || top + bottom + 18.0 > f64::from(section.page_h)
            || left + right + f64::from(section.gutter) + 18.0 > f64::from(section.page_w)
        {
            return Err(PyValueError::new_err(
                "margins must be finite, nonnegative and leave 18 points of content on each axis",
            ));
        }
        let mut candidate = self.document()?.clone();
        candidate.last_section.margin_top = top as f32;
        candidate.last_section.margin_right = right as f32;
        candidate.last_section.margin_bottom = bottom as f32;
        candidate.last_section.margin_left = left as f32;
        self.commit(candidate)
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
        let section = &self.document()?.last_section;
        if !width.is_finite()
            || !height.is_finite()
            || width > 1584.0
            || height > 1584.0
            || width < f64::from(section.margin_left + section.margin_right + section.gutter) + 18.0
            || height < f64::from(section.margin_top + section.margin_bottom) + 18.0
        {
            return Err(PyValueError::new_err("page size must be finite, at most 1584 points per side and leave 18 points inside margins"));
        }
        let mut candidate = self.document()?.clone();
        candidate.last_section.page_w = width as f32;
        candidate.last_section.page_h = height as f32;
        candidate.last_section.landscape = width > height;
        self.commit(candidate)?;
        Ok(())
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

    #[pyo3(signature = (kind, variant="default"))]
    pub fn story_id(&self, kind: &str, variant: &str) -> PyResult<Option<u32>> {
        let document = self.document()?;
        let refs = match kind {
            "header" => document.last_section.headers,
            "footer" => document.last_section.footers,
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

    #[pyo3(signature = (kind, variant="default"))]
    pub fn ensure_story(&mut self, kind: &str, variant: &str) -> PyResult<u32> {
        let document = self.document()?;
        let (part_kind, refs) = match kind {
            "header" => (PartKind::Header, document.last_section.headers),
            "footer" => (PartKind::Footer, document.last_section.footers),
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
        if let Some(id) = existing {
            self.story_paragraph_ids(id)?;
            return Ok(id);
        }
        let mut candidate = document.clone();
        let id = candidate
            .try_add_part(part_kind, Vec::new())
            .map_err(model_error)?;
        let refs = if part_kind == PartKind::Header {
            &mut candidate.last_section.headers
        } else {
            &mut candidate.last_section.footers
        };
        match variant {
            "default" => refs.default = Some(id),
            "first" => {
                refs.first = Some(id);
                candidate.last_section.title_page = true;
            }
            "even" => {
                refs.even = Some(id);
                candidate.settings.even_odd_headers = true;
            }
            _ => return Err(PyValueError::new_err("invalid story variant")),
        }
        self.commit(candidate)?;
        self.stories.insert(id, Vec::new());
        Ok(id)
    }

    pub fn story_paragraph_ids(&self, story: u32) -> PyResult<Vec<usize>> {
        self.document()?;
        self.stories.get(&story).cloned().ok_or_else(|| {
            PyIndexError::new_err("story index does not identify an owned header or footer")
        })
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
        let paragraph = Paragraph::with_text(text, CharProps::default());
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
    fn row(&self, id: usize, row: usize) -> PyResult<&wordcraft_doc::Row> {
        self.table(id)?
            .rows
            .get(row)
            .ok_or_else(|| PyIndexError::new_err("row index out of bounds"))
    }

    fn container_width(&self, id: usize) -> PyResult<f32> {
        let (story, path) = self.location(id)?;
        let Some((parent, row, cell)) = path.cell() else {
            return Ok(self.document()?.last_section.text_width());
        };
        let table = self
            .document()?
            .table(story, &parent)
            .ok_or_else(|| PyIndexError::new_err("parent table no longer exists"))?;
        let start = table.grid_col(row, cell);
        let end = start + table.rows[row].cells[cell].span();
        let margins = wordcraft_layout::cell_margins(self.document()?, table, row, cell);
        Ok(table.grid[start..end].iter().sum::<f32>() - margins[1] - margins[3])
    }

    fn cell_container(&self, id: usize, row: usize, column: usize) -> PyResult<(StoryRef, Path)> {
        let (row, cell) = cell_anchor(self.table(id)?, row, column)?;
        let (story, table_path) = self.location(id)?;
        Ok((
            story,
            Path([table_path.0.as_slice(), &[row as u32, cell as u32]].concat()),
        ))
    }

    fn invalidate_descendants(&mut self, story: StoryRef, prefix: &Path) {
        self.locations.retain(|_, (owner, path)| {
            *owner != story || path.0.len() <= prefix.0.len() || !path.0.starts_with(&prefix.0)
        });
        self.images.retain(|id| self.locations.contains_key(id));
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

fn style_kind(kind: &str) -> PyResult<StyleKind> {
    match kind {
        "paragraph" => Ok(StyleKind::Paragraph),
        "character" => Ok(StyleKind::Character),
        _ => Err(PyValueError::new_err(
            "style kind must be paragraph or character",
        )),
    }
}

fn supported_style<'a>(styles: &'a StyleSheet, id: &str) -> PyResult<&'a Style> {
    styles
        .find(id)
        .filter(|style| matches!(style.kind, StyleKind::Paragraph | StyleKind::Character))
        .ok_or_else(|| PyValueError::new_err("unknown paragraph or character style"))
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
        let header = document.ensure_story("header", "default").unwrap();
        let revision = document.revision().unwrap();
        assert_eq!(document.ensure_story("header", "default").unwrap(), header);
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
        assert!(document.ensure_story("header", "invalid").is_err());
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
                let story = document.ensure_story(kind, variant).unwrap();
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
        let story = document.ensure_story("header", "default").unwrap();
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
        document.format_style(&base, r##"{"bold":true,"font_size":18,"color":"#123456","space_after":5,"keep_with_next":true,"background":"#EEFFEE"}"##).unwrap();
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
            serde_json::from_str(&document.style_formatting(&derived, false).unwrap()).unwrap();
        let resolved: Value =
            serde_json::from_str(&document.style_formatting(&derived, true).unwrap()).unwrap();
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
        copy.format_style(&base, r#"{"font_size":22}"#).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&document.style_formatting(&derived, true).unwrap())
                .unwrap()["font_size"],
            18.0
        );
        assert_eq!(
            serde_json::from_str::<Value>(&copy.style_formatting(&derived, true).unwrap()).unwrap()
                ["font_size"],
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
        document.format_style(&character, r##"{"underline":"double_wave","strike":true,"script":"superscript","highlight":"yellow","background":"#102030","color":"auto","font_size":15.5}"##).unwrap();
        let direct: Value =
            serde_json::from_str(&document.style_formatting(&character, false).unwrap()).unwrap();
        let resolved: Value =
            serde_json::from_str(&document.style_formatting(&character, true).unwrap()).unwrap();
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
        assert!(document.add_style("Table style", "table", None).is_err());
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
            assert!(document.format_style(&root, patch).is_err(), "{patch}");
            assert!(
                document
                    .format_range_patch(paragraph, 0, 1, revision, patch)
                    .is_err(),
                "{patch}"
            );
        }
        assert!(document
            .format_style(&root, r#"{"style":"Strong"}"#)
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
            .format_style(&root, &" ".repeat(MAX_FORMAT_PATCH_BYTES + 1))
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
            .format_style(&style, r#"{"space_after":3}"#)
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
                .format_style(&style, &json!({"underline":name}).to_string())
                .unwrap();
            let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
            assert_eq!(
                underline_name(reopened.styles.get(&style).unwrap().chr.underline.unwrap()),
                *name
            );
        }
        for (name, _) in HIGHLIGHTS {
            document
                .format_style(&style, &json!({"highlight":name}).to_string())
                .unwrap();
            let reopened = wordcraft_docx::read(&document.to_bytes().unwrap()).unwrap();
            assert_eq!(
                highlight_name(reopened.styles.get(&style).unwrap().chr.highlight.unwrap()),
                *name
            );
        }
        document
            .format_style(&style, r#"{"strike":"double"}"#)
            .unwrap();
        let inherited = document
            .add_style("Inherited Mark", "character", Some(&style))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&document.style_formatting(&inherited, true).unwrap())
                .unwrap()["strike"],
            "double"
        );
        document
            .format_style(&inherited, r#"{"strike":false}"#)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&document.style_formatting(&inherited, true).unwrap())
                .unwrap()["strike"],
            false
        );
        document
            .format_style(&inherited, r#"{"strike":null}"#)
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
            .format_style(&style, r#"{"strike":"triple"}"#)
            .is_err());
    }
}
