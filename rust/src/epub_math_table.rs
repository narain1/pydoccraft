//! Place native equation cells on a bounded MathML table grid.
use std::ops::Range;

use roxmltree::Node;
use wordcraft_doc::{
    math::{self, ColJc, Math},
    para::InlineObject,
    props::{Align, Borders, CharProps, HeightRule, TextColor},
    Document, Paragraph, Table as NativeTable,
};
use wordcraft_fonts::math::{resolve_math, C};
use wordcraft_layout::math::{layout_equation, MathLayout};

use super::{
    invalid, limited, size_points, unsupported, Presentation, Projection, MATHML, MAX_MATRIX_SIZE,
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
    math: Math,
    format: CharProps,
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
    root.descendants().any(|node| {
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
            let nodes = math::merge_runs(projection.elements(cell, depth + 2, style)?);
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
                math: Math {
                    nodes,
                    ..Default::default()
                },
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
    pub cells: Vec<(f32, f32, MathLayout)>,
    widths: Vec<f32>,
    heights: Vec<f32>,
    row_positions: Vec<f32>,
}

impl Table {
    pub(in crate::epub) fn layout(
        &self,
        doc: &Document,
        format: &CharProps,
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
            let measure = if cell.math.nodes.is_empty() {
                MathLayout::default()
            } else {
                layout_equation(
                    &cell.math,
                    "",
                    &resolved,
                    false,
                    MAX_DIMENSION,
                    &Default::default(),
                    &mut 0,
                )
            };
            let offset = match cell.vertical {
                Vertical::Baseline => Some(0.0),
                Vertical::Axis => Some(resolved.draw_size() * axis_ratio - table_axis),
                _ => None,
            };
            if let Some(offset) = offset {
                let row = cell.rows.start;
                ascent[row] = ascent[row].max(measure.ascent - offset);
                if cell.rows.len() == 1 {
                    descent[row] = descent[row].max(measure.descent + offset);
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
                measures[index].0.width,
            );
        }
        for index in ordered(|cell| cell.rows.len()) {
            let cell = &self.cells[index];
            let (measure, offset) = &measures[index];
            let top = offset.map_or(0.0, |offset| {
                ascent[cell.rows.start] + offset - measure.ascent
            });
            grow(
                &mut heights,
                &self.row_gaps,
                &cell.rows,
                top + measure.ascent + measure.descent,
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
                let horizontal_room =
                    extent(&widths, &self.column_gaps, &cell.columns) - measure.width;
                let vertical_room =
                    extent(&heights, &self.row_gaps, &cell.rows) - measure.ascent - measure.descent;
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
                            ascent[cell.rows.start] + offset.unwrap_or_default() - measure.ascent
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
        let layout = self.layout(doc, format)?;
        let widths = margins_for_tracks(&layout.widths, &self.column_gaps);
        let heights = margins_for_tracks(&layout.heights, &self.row_gaps);
        let width = widths.iter().sum();
        let page = &doc.last_section;
        let available_height = page.page_h - page.margin_top - page.margin_bottom;
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
        for (cell, (_, top, _)) in self.cells.into_iter().zip(layout.cells) {
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
            target.props.width = Some(native.grid[cell.columns].iter().sum());
            target.props.no_wrap = true;
            target.props.margins = Some([
                (top - layout.row_positions[row]).max(0.0)
                    + side_gap(&self.row_gaps, row.checked_sub(1)),
                side_gap(&self.column_gaps, col.checked_sub(1)),
                side_gap(&self.row_gaps, Some(last_row)),
                side_gap(&self.column_gaps, Some(last_col)),
            ]);
            let mut paragraph = Paragraph::new();
            paragraph.props.space_before = Some(0.0);
            paragraph.props.space_after = Some(0.0);
            paragraph.props.align = Some(match cell.horizontal {
                ColJc::Left => Align::Left,
                ColJc::Center => Align::Center,
                ColJc::Right => Align::Right,
            });
            if !cell.math.nodes.is_empty() {
                let format = CharProps {
                    color: cell.format.color.or(format.color),
                    ..cell.format
                };
                let equation = InlineObject::Equation {
                    linear: math::to_linear(&cell.math.nodes),
                    math: cell.math,
                    display: false,
                };
                paragraph
                    .insert_object(0, equation, &format)
                    .map_err(super::super::output::conversion)?;
            }
            target.blocks = vec![wordcraft_doc::para_block(paragraph)];
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
