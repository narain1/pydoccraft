//! Place native equation cells on a bounded MathML table grid.
use std::{ops::Range, sync::Arc};

use roxmltree::Node;
use wordcraft_doc::{
    math::{self, ColJc, Math},
    para::InlineObject,
    props::{Align, Borders, CharProps, HeightRule, TextColor},
    Block, Document, Paragraph, Table as NativeTable,
};
use wordcraft_fonts::math::{resolve_math, C};
use wordcraft_layout::math::MathLayout;

use super::{
    compose, invalid, limited, size_points, unsupported, Presentation, Projection, MATHML,
    MAX_MATRIX_SIZE,
};
use crate::EngineError;

const MAX_ROWS: usize = 128;
const MAX_DIMENSION: f32 = 20_000.0;

pub(in crate::epub) struct Table {
    cells: Vec<Cell>,
    row_gaps: Vec<f32>,
    column_gaps: Vec<f32>,
    em: f32,
}

struct Cell {
    rows: Range<usize>,
    columns: Range<usize>,
    horizontal: ColJc,
    vertical: Vertical,
    content: CellContent,
    format: CharProps,
}

enum CellContent {
    Equation(Math),
    Table(Table),
}

#[derive(Clone, Copy)]
enum Vertical {
    Top,
    Bottom,
    Center,
    Baseline,
    Axis,
}

pub(in crate::epub) fn has_spans(root: Node<'_, '_>) -> bool {
    // A spanning cell can contain a plain equation. Its own span belongs to the parent grid.
    root.descendants().skip(1).any(|node| {
        node.has_tag_name((MATHML, "mtd"))
            && ["rowspan", "columnspan"].into_iter().any(|key| {
                node.attribute(key)
                    .is_some_and(|value| value.trim().parse::<usize>() != Ok(1))
            })
            && !node.ancestors().any(|ancestor| {
                ancestor.tag_name().namespace() == Some(MATHML)
                    && matches!(ancestor.tag_name().name(), "annotation" | "annotation-xml")
            })
    })
}

pub(super) fn project_group<'xml>(
    projection: &mut Projection<'_, 'xml>,
    mut group: Node<'xml, '_>,
    mut depth: usize,
    mut style: Presentation<'xml>,
) -> Result<Option<Table>, EngineError> {
    if !has_spans(group) {
        return Ok(None);
    }
    let mut wrappers = Vec::new();
    loop {
        let mut children = group.children().filter(Node::is_element);
        let Some(child) = children.next() else {
            return Err(invalid("empty MathML spanning equation"));
        };
        if (!group.has_tag_name((MATHML, "semantics")) && children.next().is_some())
            || child.tag_name().namespace() != Some(MATHML)
        {
            return Ok(None);
        }
        wrappers.push(child);
        match child.tag_name().name() {
            "mtable" => break,
            "mrow" | "mstyle" | "semantics" => group = child,
            _ => return Ok(None),
        }
    }
    let table = *wrappers
        .last()
        .ok_or_else(|| invalid("missing MathML table"))?;
    for wrapper in wrappers {
        depth += 1;
        projection.charge(depth)?;
        style = projection.style(wrapper, style)?;
    }
    project(projection, table, depth, style).map(Some)
}

pub(super) fn project<'xml>(
    projection: &mut Projection<'_, 'xml>,
    table: Node<'xml, '_>,
    depth: usize,
    style: Presentation<'xml>,
) -> Result<Table, EngineError> {
    let rows: Vec<_> = table
        .children()
        .filter(Node::is_element)
        .take(MAX_ROWS + 1)
        .collect();
    if rows.len() > MAX_ROWS {
        return Err(limited("MathML table exceeds 128 rows"));
    }
    let mut column_count = 0;
    let mut occupied = [0; MAX_MATRIX_SIZE];
    let mut cells = Vec::new();
    for (row_index, row) in rows.iter().copied().enumerate() {
        if !row.has_tag_name((MATHML, "mtr")) {
            return Err(unsupported("MathML table row type"));
        }
        projection.charge(depth + 1)?;
        let row_style = projection.style(row, style)?;
        let mut column = 0;
        for cell in row.children().filter(Node::is_element) {
            if !cell.has_tag_name((MATHML, "mtd")) {
                return Err(invalid("unexpected MathML table child"));
            }
            while column < MAX_MATRIX_SIZE && occupied[column] > row_index {
                column += 1;
            }
            let right = column + span(cell, "columnspan")?;
            let bottom = row_index + span(cell, "rowspan")?;
            if right > MAX_MATRIX_SIZE {
                return Err(limited("MathML table exceeds 32 columns"));
            }
            if bottom > rows.len() || occupied[column..right].iter().any(|end| *end > row_index) {
                return Err(invalid(
                    "MathML table spans overlap or extend beyond the rows",
                ));
            }
            occupied[column..right].fill(bottom);
            projection.charge(depth + 2)?;
            let style = projection.style(cell, row_style)?;
            let content = if let Some(nested) = project_group(projection, cell, depth + 2, style)? {
                CellContent::Table(nested)
            } else {
                CellContent::Equation(Math {
                    nodes: math::merge_runs(projection.elements(cell, depth + 2, style)?),
                    ..Default::default()
                })
            };
            let horizontal = match cell
                .attribute("columnalign")
                .or_else(|| indexed(row.attribute("columnalign"), column))
                .or_else(|| indexed(table.attribute("columnalign"), column))
                .unwrap_or("center")
            {
                "left" => ColJc::Left,
                "right" => ColJc::Right,
                "center" => ColJc::Center,
                _ => return Err(unsupported("MathML table column alignment")),
            };
            let vertical = match cell
                .attribute("rowalign")
                .or_else(|| row.attribute("rowalign"))
                .or_else(|| indexed(table.attribute("rowalign"), row_index))
                .unwrap_or("baseline")
            {
                "top" => Vertical::Top,
                "bottom" => Vertical::Bottom,
                "center" => Vertical::Center,
                "baseline" => Vertical::Baseline,
                "axis" => Vertical::Axis,
                _ => return Err(unsupported("MathML table row alignment")),
            };
            cells.push(Cell {
                rows: row_index..bottom,
                columns: column..right,
                horizontal,
                vertical,
                content,
                format: CharProps {
                    size: Some(style.font_size()),
                    color: style.ink.map(TextColor::Rgb),
                    ..Default::default()
                },
            });
            column = right;
            column_count = column_count.max(right);
        }
    }
    if column_count == 0 {
        return Err(invalid("MathML table has no cells"));
    }
    // Native export materializes the full grid before merging, including
    // entries implied by sparse rows and spanning cells.
    let implicit_cells = rows.len() * column_count - cells.len();
    *projection.remaining = projection
        .remaining
        .checked_sub(implicit_cells)
        .ok_or_else(|| limited("MathML table cells exceed the layout budget"))?;
    let em = style.font_size();
    Ok(Table {
        cells,
        row_gaps: gaps(table, "rowspacing", rows.len(), em, "1ex")?,
        column_gaps: gaps(table, "columnspacing", column_count, em, "0.8em")?,
        em,
    })
}

fn span(node: Node<'_, '_>, key: &str) -> Result<usize, EngineError> {
    let count = node
        .attribute(key)
        .unwrap_or("1")
        .trim()
        .parse::<usize>()
        .ok()
        .filter(|count| *count >= 1)
        .ok_or_else(|| invalid("invalid MathML table span"))?;
    if count > MAX_ROWS {
        return Err(limited("MathML table span exceeds its size budget"));
    }
    Ok(count)
}

fn indexed(values: Option<&str>, index: usize) -> Option<&str> {
    values?.split_ascii_whitespace().take(index + 1).last()
}

fn gaps(
    table: Node<'_, '_>,
    key: &str,
    tracks: usize,
    em: f32,
    default: &str,
) -> Result<Vec<f32>, EngineError> {
    (0..tracks - 1)
        .map(|index| {
            let value = indexed(table.attribute(key), index).unwrap_or(default);
            value
                .strip_suffix("ex")
                .and_then(|n| n.parse::<f32>().ok())
                .map(|units| units * em * 0.5)
                .or_else(|| size_points(value, em, [0.0; 2]))
                .filter(|size| (0.0..=MAX_DIMENSION).contains(size))
                .ok_or_else(|| unsupported("MathML table spacing"))
        })
        .collect()
}

pub(in crate::epub) struct Layout {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
    pub cells: Vec<(f32, f32, CellDrawing)>,
    widths: Vec<f32>,
    heights: Vec<f32>,
    row_positions: Vec<f32>,
}

impl Layout {
    pub(in crate::epub) fn into_math(self) -> MathLayout {
        let mut result = MathLayout {
            width: self.width,
            ascent: self.baseline,
            descent: self.height - self.baseline,
            ..Default::default()
        };
        for (x, y, cell) in self.cells {
            let layout = match cell {
                CellDrawing::Equation(equation) => equation,
                CellDrawing::Table(nested) => nested.into_math(),
            };
            let rise = self.baseline - y - layout.ascent;
            result
                .items
                .extend(layout.items.into_iter().map(|mut item| {
                    compose::position(&mut item, x, rise, 1.0);
                    item
                }));
        }
        result
    }
}

pub(in crate::epub) enum CellDrawing {
    Equation(MathLayout),
    Table(Layout),
}

impl CellDrawing {
    fn size(&self) -> (f32, f32, f32) {
        match self {
            Self::Equation(layout) => (layout.width, layout.ascent, layout.descent),
            Self::Table(layout) => (
                layout.width,
                layout.baseline,
                layout.height - layout.baseline,
            ),
        }
    }
}

impl Table {
    pub(in crate::epub) fn apply_editable(
        &mut self,
        editable: &compose::Editable,
    ) -> Result<(), EngineError> {
        for cell in &mut self.cells {
            match &mut cell.content {
                CellContent::Equation(math) => editable.apply(math)?,
                CellContent::Table(table) => table.apply_editable(editable)?,
            }
        }
        Ok(())
    }

    pub(in crate::epub) fn layout(
        &self,
        doc: &Document,
        format: &CharProps,
        measurements: &compose::Measurements,
    ) -> Result<Layout, EngineError> {
        let (face, constants) = resolve_math("Cambria Math");
        let axis_ratio = constants.as_ref().map_or(0.25, |constants| {
            constants.get(C::AxisHeight) / (face.upem as f32).max(1.0)
        });
        let table_axis = self.em * axis_ratio;
        let row_count = self.row_gaps.len() + 1;
        let mut widths = vec![0.0; self.column_gaps.len() + 1];
        let mut ascent = vec![0.0f32; row_count];
        let mut descent = vec![0.0f32; row_count];
        let mut heights = vec![0.0; row_count];
        let mut measures = Vec::with_capacity(self.cells.len());
        for cell in &self.cells {
            let chars = CharProps {
                size: cell.format.size,
                color: cell.format.color.or(format.color),
                ..Default::default()
            };
            let resolved = doc.styles.resolve_char(None, &chars);
            let measure = match &cell.content {
                CellContent::Table(table) => {
                    CellDrawing::Table(table.layout(doc, &chars, measurements)?)
                }
                CellContent::Equation(math) if math.nodes.is_empty() => {
                    CellDrawing::Equation(MathLayout::default())
                }
                CellContent::Equation(math) => CellDrawing::Equation(measurements.layout(
                    math,
                    "",
                    &resolved,
                    false,
                    MAX_DIMENSION,
                )?),
            };
            let (_, cell_ascent, cell_descent) = measure.size();
            let offset = match cell.vertical {
                Vertical::Baseline => Some(0.0),
                Vertical::Axis => Some(resolved.draw_size() * axis_ratio - table_axis),
                _ => None,
            };
            if let Some(offset) = offset {
                let row = cell.rows.start;
                ascent[row] = ascent[row].max(cell_ascent - offset);
                if cell.rows.len() == 1 {
                    descent[row] = descent[row].max(cell_descent + offset);
                }
            }
            measures.push((measure, offset));
        }
        for row in 0..row_count {
            heights[row] = ascent[row] + descent[row];
        }
        // Resolve single cells first. Wider or taller spans impose a minimum
        // over all of their tracks, including the gaps internal to the span.
        let ordered = |key: fn(&Cell) -> usize| {
            let mut indices: Vec<_> = (0..self.cells.len()).collect();
            indices.sort_by_key(|&index| key(&self.cells[index]));
            indices
        };
        for index in ordered(|cell| cell.columns.len()) {
            grow(
                &mut widths,
                &self.column_gaps,
                &self.cells[index].columns,
                measures[index].0.size().0,
            );
        }
        for index in ordered(|cell| cell.rows.len()) {
            let cell = &self.cells[index];
            let (measure, offset) = &measures[index];
            let (_, cell_ascent, cell_descent) = measure.size();
            let top = offset.map_or(0.0, |offset| ascent[cell.rows.start] + offset - cell_ascent);
            grow(
                &mut heights,
                &self.row_gaps,
                &cell.rows,
                top + cell_ascent + cell_descent,
            );
        }
        let x = starts(&widths, &self.column_gaps);
        let y = starts(&heights, &self.row_gaps);
        let width = x[widths.len()];
        let height = y[heights.len()];
        if ![width, height]
            .into_iter()
            .all(|value| (0.0..=MAX_DIMENSION).contains(&value))
        {
            return Err(limited("MathML table exceeds its geometry budget"));
        }
        let cells = self
            .cells
            .iter()
            .zip(measures)
            .map(|(cell, (measure, offset))| {
                let (cell_width, cell_ascent, cell_descent) = measure.size();
                let horizontal_room =
                    extent(&widths, &self.column_gaps, &cell.columns) - cell_width;
                let vertical_room =
                    extent(&heights, &self.row_gaps, &cell.rows) - cell_ascent - cell_descent;
                let left = x[cell.columns.start]
                    + match cell.horizontal {
                        ColJc::Left => 0.0,
                        ColJc::Center => horizontal_room / 2.0,
                        ColJc::Right => horizontal_room,
                    };
                let top = y[cell.rows.start]
                    + match cell.vertical {
                        Vertical::Top => 0.0,
                        Vertical::Bottom => vertical_room,
                        Vertical::Center => vertical_room / 2.0,
                        Vertical::Baseline | Vertical::Axis => {
                            ascent[cell.rows.start] + offset.unwrap_or_default() - cell_ascent
                        }
                    };
                (left, top, measure)
            })
            .collect();
        Ok(Layout {
            width,
            height,
            baseline: (height / 2.0 + table_axis).min(height),
            cells,
            widths,
            heights,
            row_positions: y,
        })
    }

    pub(in crate::epub) fn export(
        self,
        doc: &Document,
        format: &CharProps,
        available: f32,
        align: Option<Align>,
    ) -> Result<NativeTable, EngineError> {
        let layout = self.layout(doc, format, &compose::Measurements::default())?;
        let page = &doc.last_section;
        let available_height = page.page_h - page.margin_top - page.margin_bottom;
        self.export_layout(layout, format, available, available_height, align)
    }

    fn export_layout(
        self,
        layout: Layout,
        format: &CharProps,
        available: f32,
        available_height: f32,
        align: Option<Align>,
    ) -> Result<NativeTable, EngineError> {
        let widths = margins_for_tracks(&layout.widths, &self.column_gaps);
        let heights = margins_for_tracks(&layout.heights, &self.row_gaps);
        let width = widths.iter().sum();
        if width > available || heights.iter().sum::<f32>() > available_height {
            return Err(unsupported(
                "MathML spanning table exceeds the semantic export page",
            ));
        }
        let mut native = NativeTable::new(heights.len(), widths.len(), width);
        native.grid = widths;
        native.props.style = None;
        native.props.width = Some(width);
        native.props.fixed = true;
        native.props.borders = Some(Borders::default());
        native.props.cell_margins = Some([0.0; 4]);
        native.props.align = align;
        for (row, height) in native.rows.iter_mut().zip(heights) {
            row.props.height_rule = HeightRule::AtLeast;
            row.props.height = Some(height);
            for (cell, width) in row.cells.iter_mut().zip(&native.grid) {
                cell.props.width = Some(*width);
                cell.props.hide_mark = true;
            }
        }
        for (cell, (_, top, drawing)) in self.cells.into_iter().zip(layout.cells) {
            let row = cell.rows.start;
            let col = cell.columns.start;
            let last_row = cell.rows.end - 1;
            let last_col = cell.columns.end - 1;
            if row != last_row || col != last_col {
                native.merge(row, last_row, col, last_col);
            }
            let native_col = native
                .cell_at_grid(row, col)
                .ok_or_else(|| invalid("missing MathML table cell"))?;
            let target = &mut native.rows[row].cells[native_col];
            let cell_width: f32 = native.grid[cell.columns].iter().sum();
            target.props.width = Some(cell_width);
            target.props.no_wrap = true;
            let margins = [
                (top - layout.row_positions[row]).max(0.0)
                    + side_gap(&self.row_gaps, row.checked_sub(1)),
                side_gap(&self.column_gaps, col.checked_sub(1)),
                side_gap(&self.row_gaps, Some(last_row)),
                side_gap(&self.column_gaps, Some(last_col)),
            ];
            target.props.margins = Some(margins);
            let align = Some(match cell.horizontal {
                ColJc::Left => Align::Left,
                ColJc::Center => Align::Center,
                ColJc::Right => Align::Right,
            });
            let format = CharProps {
                color: cell.format.color.or(format.color),
                ..cell.format
            };
            target.blocks = vec![match (cell.content, drawing) {
                (CellContent::Table(nested), CellDrawing::Table(nested_layout)) => {
                    // Widths reach DOCX in twentieths of a point.
                    let inner_width = cell_width - margins[1] - margins[3] + 0.05;
                    let nested = nested.export_layout(
                        nested_layout,
                        &format,
                        inner_width,
                        available_height,
                        align,
                    )?;
                    Arc::new(Block::Table(nested))
                }
                (CellContent::Equation(math), CellDrawing::Equation(_)) => {
                    let mut paragraph = Paragraph::new();
                    paragraph.props.space_before = Some(0.0);
                    paragraph.props.space_after = Some(0.0);
                    paragraph.props.align = align;
                    if !math.nodes.is_empty() {
                        let equation = InlineObject::Equation {
                            linear: math::to_linear(&math.nodes),
                            math,
                            display: false,
                        };
                        paragraph
                            .insert_object(0, equation, &format)
                            .map_err(super::super::output::conversion)?;
                    }
                    wordcraft_doc::para_block(paragraph)
                }
                _ => return Err(invalid("MathML table layout does not match its cells")),
            }];
        }
        Ok(native)
    }
}

fn margins_for_tracks(sizes: &[f32], gaps: &[f32]) -> Vec<f32> {
    sizes
        .iter()
        .enumerate()
        .map(|(index, size)| {
            (size + side_gap(gaps, index.checked_sub(1)) + side_gap(gaps, Some(index))).max(0.1)
        })
        .collect()
}

fn side_gap(gaps: &[f32], index: Option<usize>) -> f32 {
    index
        .and_then(|index| gaps.get(index))
        .copied()
        .unwrap_or_default()
        / 2.0
}

fn extent(sizes: &[f32], gaps: &[f32], range: &Range<usize>) -> f32 {
    sizes[range.clone()].iter().sum::<f32>() + gaps[range.start..range.end - 1].iter().sum::<f32>()
}

fn grow(sizes: &mut [f32], gaps: &[f32], range: &Range<usize>, needed: f32) {
    let extra = (needed - extent(sizes, gaps, range)).max(0.0) / range.len() as f32;
    for size in &mut sizes[range.clone()] {
        *size += extra;
    }
}

fn starts(sizes: &[f32], gaps: &[f32]) -> Vec<f32> {
    let mut positions = vec![0.0];
    for (index, size) in sizes.iter().enumerate() {
        positions.push(positions[index] + size + gaps.get(index).copied().unwrap_or_default());
    }
    positions
}
