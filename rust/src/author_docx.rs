//! Native-owned DOCX creation. Python never reconstructs this model from extraction.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    sync::Arc,
};

use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;
use wordcraft_doc::para::{Float, InlineObject};
use wordcraft_doc::props::{Border, BorderStyle, Borders, HeightRule, LineSpacing, VAlign, VMerge};
use wordcraft_doc::{
    Align, Block, CharProps, Document, Paragraph, PartKind, Path, Pos, Rgb, StoryRef, StyleKind,
    Table, TextColor,
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
            ("at_least", Some(value)) | ("exact", Some(value)) if value.is_finite() && value > 0.0 && value <= 1584.0 => {
                if rule == "exact" { HeightRule::Exact } else { HeightRule::AtLeast }
            }
            _ => return Err(PyValueError::new_err("row height requires (None, auto) or a finite height in (0, 1584] points with at_least/exact")),
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
        candidate.set_page_break_before(id, true)?;
        *self = candidate;
        Ok(id)
    }

    pub fn paragraph_spacing(&self, id: usize) -> PyResult<(f64, f64, f64)> {
        let paragraph = self.paragraph(id)?;
        let props = self.document()?.styles.resolve_para(&paragraph.props);
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
        assert!(document.page_break_before(break_paragraph).unwrap());
        assert_eq!(document.paragraph_text(break_paragraph).unwrap(), "");
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
}
