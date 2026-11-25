//! Place native equation cells on a bounded MathML table grid.
use std::{ops::Range, sync::Arc};

use roxmltree::Node;
use wordcraft_doc::{
    math::{self, Arg, ColJc, MNode, Math},
    para::InlineObject,
    props::{Align, Borders, CharProps, HeightRule, Rgb, TextColor},
    Block, Document, Paragraph, Table as NativeTable,
};
use wordcraft_fonts::math::{resolve_math, C};
use wordcraft_layout::math::{MItem, MathLayout};

use super::{
    compose, invalid, limited, size_points, unsupported, Presentation, Projection, MATHML,
    MAX_MATRIX_SIZE,
};
use crate::EngineError;

#[path = "epub_math_table_rules.rs"]
mod rules;
use rules::{Occupancy, Rules};

const MAX_ROWS: usize = 128;
const MAX_DIMENSION: f32 = 20_000.0;
const WIDTH_EPSILON: f32 = 0.0001;

pub(in crate::epub) struct Table {
    cells: Vec<Cell>,
    row_gaps: Vec<f32>,
    column_gaps: Vec<f32>,
    column_widths: Vec<Option<f32>>,
    equal_rows: bool,
    equal_columns: bool,
    rules: Rules,
    rule_color: Option<Rgb>,
    alignment: Vertical,
    alignment_row: Option<usize>,
    em: f32,
    enclosing_em: f32,
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

impl Vertical {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "center" => Some(Self::Center),
            "baseline" => Some(Self::Baseline),
            "axis" => Some(Self::Axis),
            _ => None,
        }
    }
}

pub(super) fn has_authored_geometry(table: Node<'_, '_>) -> bool {
    let cell_alignment =
        |node: Node<'_, '_>| node.has_attribute("rowalign") || node.has_attribute("columnalign");
    rules::requested(table)
        || [
            "align",
            "rowalign",
            "rowspacing",
            "columnspacing",
            "columnwidth",
        ]
        .into_iter()
        .any(|key| table.has_attribute(key))
        || ["equalrows", "equalcolumns"].into_iter().any(|key| {
            table
                .attribute(key)
                .is_some_and(|value| value.trim() != "false")
        })
        || table.children().filter(Node::is_element).any(|row| {
            cell_alignment(row) || row.children().filter(Node::is_element).any(cell_alignment)
        })
}

pub(in crate::epub) fn has_spans(root: Node<'_, '_>) -> bool {
    // A spanning cell can contain a plain equation. Its own span belongs to the parent grid.
    root.descendants().skip(1).any(|node| {
        node.has_tag_name((MATHML, "mtd"))
            && ["rowspan", "columnspan"].into_iter().any(|key| {
                node.attribute(key)
                    .is_some_and(|value| value.trim().parse::<usize>() != Ok(1))
            })
            && is_presentation(node)
    })
}

pub(in crate::epub) fn has_rule_requests(root: Node<'_, '_>) -> bool {
    root.descendants().any(|node| {
        node.has_tag_name((MATHML, "mtable")) && rules::requested(node) && is_presentation(node)
    })
}

fn is_presentation(node: Node<'_, '_>) -> bool {
    !node.ancestors().any(|ancestor| {
        ancestor.tag_name().namespace() == Some(MATHML)
            && matches!(ancestor.tag_name().name(), "annotation" | "annotation-xml")
    })
}

pub(super) fn project_group<'xml>(
    projection: &mut Projection<'_, 'xml>,
    mut group: Node<'xml, '_>,
    mut depth: usize,
    mut style: Presentation<'xml>,
) -> Result<Option<Table>, EngineError> {
    if !has_spans(group) && !has_rule_requests(group) {
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
    let table = wrappers
        .pop()
        .ok_or_else(|| invalid("missing MathML table"))?;
    for wrapper in wrappers {
        depth += 1;
        projection.charge(depth)?;
        style = projection.style(wrapper, style)?;
    }
    projection.charge(depth + 1)?;
    let table_style = projection.style(table, style)?;
    project(projection, table, depth + 1, table_style, style.font_size()).map(Some)
}

pub(super) fn project<'xml>(
    projection: &mut Projection<'_, 'xml>,
    table: Node<'xml, '_>,
    depth: usize,
    style: Presentation<'xml>,
    enclosing_em: f32,
) -> Result<Table, EngineError> {
    let rows: Vec<_> = table
        .children()
        .filter(Node::is_element)
        .take(MAX_ROWS + 1)
        .collect();
    if rows.len() > MAX_ROWS {
        return Err(limited("MathML table exceeds 128 rows"));
    }
    let (alignment, alignment_row) = table_alignment(table, rows.len())?;
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
                .trim()
            {
                "left" => ColJc::Left,
                "right" => ColJc::Right,
                "center" => ColJc::Center,
                _ => return Err(unsupported("MathML table column alignment")),
            };
            let row_alignment = cell
                .attribute("rowalign")
                .or_else(|| row.attribute("rowalign"))
                .or_else(|| indexed(table.attribute("rowalign"), row_index))
                .unwrap_or("baseline");
            let vertical = Vertical::parse(row_alignment.trim())
                .ok_or_else(|| unsupported("MathML table row alignment"))?;
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
    let mut row_gaps = gaps(table, "rowspacing", rows.len(), em, "1ex")?;
    let mut column_gaps = gaps(table, "columnspacing", column_count, em, "0.8em")?;
    let rules = Rules::read(table, em, &mut row_gaps, &mut column_gaps)?;
    Ok(Table {
        cells,
        row_gaps,
        column_gaps,
        column_widths: column_widths(table, column_count, em)?,
        equal_rows: boolean(table, "equalrows")?,
        equal_columns: boolean(table, "equalcolumns")?,
        rules,
        rule_color: style.ink,
        alignment,
        alignment_row,
        em,
        enclosing_em,
    })
}

fn table_alignment(
    table: Node<'_, '_>,
    rows: usize,
) -> Result<(Vertical, Option<usize>), EngineError> {
    let mut words = table
        .attribute("align")
        .unwrap_or("axis")
        .split_ascii_whitespace();
    let alignment = words
        .next()
        .and_then(Vertical::parse)
        .ok_or_else(|| unsupported("MathML table alignment"))?;
    let row = words
        .next()
        .and_then(|value| value.parse::<isize>().ok())
        .and_then(|index| match index {
            1.. => index.checked_sub(1),
            ..=-1 => (rows as isize).checked_add(index),
            0 => None,
        })
        .and_then(|index| usize::try_from(index).ok())
        .filter(|&index| index < rows);
    if words.next().is_some() {
        return Err(unsupported("MathML table alignment"));
    }
    Ok((alignment, row))
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

fn boolean(node: Node<'_, '_>, key: &str) -> Result<bool, EngineError> {
    match node.attribute(key).map(str::trim) {
        Some("true") => Ok(true),
        None | Some("false") => Ok(false),
        Some(_) => Err(invalid("invalid MathML table boolean attribute")),
    }
}

fn column_widths(
    table: Node<'_, '_>,
    columns: usize,
    em: f32,
) -> Result<Vec<Option<f32>>, EngineError> {
    (0..columns)
        .map(
            |column| match indexed(table.attribute("columnwidth"), column) {
                None | Some("auto") => Ok(None),
                Some(value)
                    if value.ends_with('%')
                        || value.ends_with("vw")
                        || value.ends_with("vh")
                        || value == "fit" =>
                {
                    Err(unsupported("MathML fitted or percentage column widths"))
                }
                Some(value) => length(value, em)
                    .map(Some)
                    .ok_or_else(|| unsupported("MathML table column width")),
            },
        )
        .collect()
}

fn length(value: &str, em: f32) -> Option<f32> {
    value
        .strip_suffix("ex")
        .and_then(|n| n.parse::<f32>().ok())
        .map(|units| units * em * 0.5)
        .or_else(|| size_points(value, em, [0.0; 2]))
        .filter(|size| (0.0..=MAX_DIMENSION).contains(size))
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
            length(value, em).ok_or_else(|| unsupported("MathML table spacing"))
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
    rules: Vec<MItem>,
    occupancy: Option<Occupancy>,
    rule_color: Rgb,
}

impl Layout {
    pub(in crate::epub) fn into_math(self) -> MathLayout {
        let mut result = MathLayout {
            width: self.width,
            ascent: self.baseline,
            descent: self.height - self.baseline,
            items: self.rules,
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
    pub(in crate::epub) fn has_rules(&self) -> bool {
        self.rules.visible()
            || self
                .cells
                .iter()
                .any(|cell| matches!(&cell.content, CellContent::Table(table) if table.has_rules()))
    }

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

    fn grow_columns(
        &self,
        widths: &mut [f32],
        span: &Range<usize>,
        needed: f32,
    ) -> Result<(), EngineError> {
        let excess = needed - extent(widths, &self.column_gaps, span);
        if excess <= WIDTH_EPSILON {
            return Ok(());
        }
        let tracks = &self.column_widths[span.clone()];
        let flexible = tracks.iter().filter(|width| width.is_none()).count();
        if flexible == 0 {
            return Err(unsupported("MathML cell exceeds fixed column widths"));
        }
        let share = excess / flexible as f32;
        for (width, fixed) in widths[span.clone()].iter_mut().zip(tracks) {
            if fixed.is_none() {
                *width += share;
            }
        }
        Ok(())
    }

    fn equal_column_width(&self, minimum: f32) -> Result<f32, EngineError> {
        let mut fixed_widths = self.column_widths.iter().flatten().copied();
        let width = fixed_widths.next().unwrap_or(minimum);
        if fixed_widths.any(|fixed| (fixed - width).abs() > WIDTH_EPSILON) {
            return Err(unsupported(
                "MathML equal columns request conflicting fixed widths",
            ));
        }
        if minimum > width + WIDTH_EPSILON {
            return Err(unsupported("MathML cell exceeds fixed column widths"));
        }
        Ok(width)
    }

    pub(super) fn editable_matrix(
        self,
        layout: Layout,
        editor: &compose::EditableLayout<'_>,
    ) -> Result<MNode, EngineError> {
        if self.rules.visible() {
            return Err(unsupported(
                "MathML table rules inside equations in DOCX export",
            ));
        }
        let [column_gap, row_pitch] = editor.matrix_spacing()?;
        let (origins, advances) = self.matrix_columns(&layout.widths, column_gap)?;
        let rows = layout.heights.len();
        let columns = layout.widths.len();
        let mut contents = vec![vec![Arg::new(); columns]; rows];
        let first_baseline = editor.axis() + (rows - 1) as f32 * row_pitch / 2.0;
        // Native matrices have no cell spans. Give the logical slots their
        // measured widths independently of the expressions' painted bounds.
        // Each expression can then extend into slots covered by its span.
        for (cell, (left, top, drawing)) in self.cells.into_iter().zip(layout.cells) {
            let row = cell.rows.start;
            let column = cell.columns.start;
            let (_, ascent, _) = drawing.size();
            let nodes = match (cell.content, drawing) {
                (CellContent::Equation(math), CellDrawing::Equation(_)) => math.nodes,
                (CellContent::Table(table), CellDrawing::Table(nested)) => {
                    vec![table.editable_matrix(nested, editor)?]
                }
                _ => return Err(invalid("MathML table layout does not match its cells")),
            };
            if nodes.is_empty() {
                continue;
            }
            let matrix_baseline = first_baseline - row as f32 * row_pitch;
            let offset = layout.baseline - top - ascent - matrix_baseline;
            let mut cell_body = Arg::new();
            cell_body.extend(compose::grid_gap((left - origins[column]).max(0.0))?);
            cell_body.extend(nodes);
            contents[row][column].push(MNode::Phant {
                show: true,
                zero_wid: true,
                zero_asc: true,
                zero_desc: true,
                e: vec![editor.shift(cell_body, offset)?],
            });
        }
        for row in &mut contents {
            for (cell, &advance) in row.iter_mut().zip(&advances) {
                cell.extend(compose::grid_gap(advance)?);
                if cell.is_empty() {
                    cell.push(compose::grid_marker());
                }
            }
        }
        let mut nodes = vec![MNode::Phant {
            show: true,
            zero_wid: false,
            zero_asc: true,
            zero_desc: true,
            e: vec![
                MNode::Matrix {
                    rows: contents,
                    col_jc: vec![ColJc::Left; columns],
                },
                compose::grid_marker(),
            ],
        }];
        editor.add_extents(
            &mut nodes,
            [Some(layout.baseline), Some(layout.height - layout.baseline)],
        )?;
        Ok(MNode::Boxed { e: nodes })
    }

    fn matrix_columns(
        &self,
        widths: &[f32],
        native_gap: f32,
    ) -> Result<(Vec<f32>, Vec<f32>), EngineError> {
        let mut origins = Vec::with_capacity(widths.len());
        let mut advances = Vec::with_capacity(widths.len());
        let mut current = 0.0;
        for (index, desired_end) in starts(widths, &self.column_gaps)
            .into_iter()
            .skip(1)
            .enumerate()
        {
            let gap = if index + 1 < widths.len() {
                native_gap
            } else {
                0.0
            };
            let reserve = desired_end - current - gap;
            if reserve < -0.0001 {
                return Err(unsupported(
                    "MathML matrix columns are closer than the native DOCX gap",
                ));
            }
            // Round cumulative origins down so the cell's correction stays
            // nonnegative. The minimum native math space carries 0.4 points.
            let units = (reserve * 5.0 + 0.0001).floor();
            let advance = if units < 2.0 { 0.0 } else { units / 5.0 };
            origins.push(current);
            advances.push(advance);
            current += advance + gap;
        }
        Ok((origins, advances))
    }

    pub(in crate::epub) fn layout(
        &self,
        doc: &Document,
        format: &CharProps,
        measurements: &compose::Measurements,
    ) -> Result<Layout, EngineError> {
        self.layout_with_rules(doc, format, measurements, &mut { rules::MAX_RULE_ITEMS })
    }

    fn layout_with_rules(
        &self,
        doc: &Document,
        format: &CharProps,
        measurements: &compose::Measurements,
        rule_budget: &mut usize,
    ) -> Result<Layout, EngineError> {
        let (face, constants) = resolve_math("Cambria Math");
        let axis_ratio = constants.as_ref().map_or(0.25, |constants| {
            constants.get(C::AxisHeight) / (face.upem as f32).max(1.0)
        });
        let table_axis = self.em * axis_ratio;
        let enclosing_axis = self.enclosing_em * axis_ratio;
        let row_count = self.row_gaps.len() + 1;
        let mut widths: Vec<_> = self
            .column_widths
            .iter()
            .map(|width| width.unwrap_or_default())
            .collect();
        let mut ascent = vec![0.0f32; row_count];
        let mut descent = vec![0.0f32; row_count];
        let mut row_baselines = vec![false; row_count];
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
                CellContent::Table(table) => CellDrawing::Table(table.layout_with_rules(
                    doc,
                    &chars,
                    measurements,
                    rule_budget,
                )?),
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
                row_baselines[row] = true;
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
        if self.equal_columns {
            let common_width = self
                .cells
                .iter()
                .zip(&measures)
                .map(|(cell, (measure, _))| {
                    equal_track_size(&self.column_gaps, &cell.columns, measure.size().0)
                })
                .fold(0.0, f32::max);
            widths.fill(self.equal_column_width(common_width)?);
        } else {
            for index in ordered(|cell| cell.columns.len()) {
                self.grow_columns(
                    &mut widths,
                    &self.cells[index].columns,
                    measures[index].0.size().0,
                )?;
            }
        }
        let mut common_height = heights.iter().copied().fold(0.0, f32::max);
        for index in ordered(|cell| cell.rows.len()) {
            let cell = &self.cells[index];
            let (measure, offset) = &measures[index];
            let (_, cell_ascent, cell_descent) = measure.size();
            let top = offset.map_or(0.0, |offset| ascent[cell.rows.start] + offset - cell_ascent);
            let needed = top + cell_ascent + cell_descent;
            if self.equal_rows {
                common_height =
                    common_height.max(equal_track_size(&self.row_gaps, &cell.rows, needed));
            } else {
                grow(&mut heights, &self.row_gaps, &cell.rows, needed);
            }
        }
        if self.equal_rows {
            heights.fill(common_height);
        }
        let x = starts(&widths, &self.column_gaps);
        let y = starts(&heights, &self.row_gaps);
        let [left_inset, top_inset] = self.rules.inset();
        let width = x[widths.len()] + left_inset * 2.0;
        let height = y[heights.len()] + top_inset * 2.0;
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
                let left = left_inset
                    + x[cell.columns.start]
                    + match cell.horizontal {
                        ColJc::Left => 0.0,
                        ColJc::Center => horizontal_room / 2.0,
                        ColJc::Right => horizontal_room,
                    };
                let top = top_inset
                    + y[cell.rows.start]
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
        let (top, bottom, row_baseline) = self.alignment_row.map_or((0.0, height, None), |row| {
            (
                top_inset + y[row],
                top_inset + y[row] + heights[row],
                row_baselines[row].then_some(top_inset + y[row] + ascent[row]),
            )
        });
        let center = (top + bottom) / 2.0;
        let baseline = match self.alignment {
            Vertical::Top => top,
            Vertical::Bottom => bottom,
            Vertical::Center => center,
            Vertical::Baseline => row_baseline.unwrap_or(center),
            Vertical::Axis => (row_baseline.map_or(center, |baseline| baseline - table_axis)
                + enclosing_axis)
                .clamp(0.0, height),
        };
        let rule_color = match format.color {
            Some(TextColor::Rgb(color)) => self.rule_color.unwrap_or(color),
            _ => self.rule_color.unwrap_or(Rgb(0, 0, 0)),
        };
        let occupancy = self
            .rules
            .visible()
            .then(|| Occupancy::new(&self.cells, heights.len(), widths.len()));
        let rules = occupancy
            .as_ref()
            .map(|map| {
                self.rules.draw(
                    [&x, &y],
                    [&self.column_gaps, &self.row_gaps],
                    map,
                    baseline,
                    rule_color,
                    rule_budget,
                )
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Layout {
            width,
            height,
            baseline,
            cells,
            widths,
            heights,
            row_positions: y.into_iter().map(|start| start + top_inset).collect(),
            rules,
            occupancy,
            rule_color,
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
        let [frame_inline, frame_block] = self.rules.native_margin();
        let widths = margins_for_tracks(&layout.widths, &self.column_gaps, frame_inline);
        let heights = margins_for_tracks(&layout.heights, &self.row_gaps, frame_block);
        let [nrows, ncols] = [heights.len(), widths.len()];
        let width: f32 = widths.iter().sum();
        if width.max(layout.width) > available
            || heights.iter().sum::<f32>().max(layout.height) > available_height
        {
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
            let mut margins = [
                (top - layout.row_positions[row]).max(0.0)
                    + side_gap(&self.row_gaps, row.checked_sub(1)),
                side_gap(&self.column_gaps, col.checked_sub(1)),
                side_gap(&self.row_gaps, Some(last_row)),
                side_gap(&self.column_gaps, Some(last_col)),
            ];
            if row == 0 {
                margins[0] += frame_block;
            }
            if col == 0 {
                margins[1] += frame_inline;
            }
            if last_row + 1 == nrows {
                margins[2] += frame_block;
            }
            if last_col + 1 == ncols {
                margins[3] += frame_inline;
            }
            let [top_band, bottom_band] = self.rules.native_vertical_bands(row, last_row + 1);
            margins[0] -= top_band;
            margins[2] -= bottom_band;
            if margins[0] < -WIDTH_EPSILON || margins[2] < -WIDTH_EPSILON {
                return Err(unsupported(
                    "MathML table frame or row gap is smaller than native DOCX border clearance",
                ));
            }
            margins[0] = margins[0].max(0.0);
            margins[2] = margins[2].max(0.0);
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
        if let Some(map) = layout.occupancy {
            self.rules
                .set_native_edges(&mut native, &map, layout.rule_color);
        }
        Ok(native)
    }
}

fn margins_for_tracks(sizes: &[f32], gaps: &[f32], frame: f32) -> Vec<f32> {
    sizes
        .iter()
        .enumerate()
        .map(|(index, size)| {
            let before = if index == 0 {
                frame
            } else {
                gaps[index - 1] / 2.0
            };
            let after = gaps.get(index).copied().map_or(frame, |gap| gap / 2.0);
            (size + before + after).max(0.1)
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
    sizes[range.clone()].iter().sum::<f32>() + internal_gaps(gaps, range)
}

fn internal_gaps(gaps: &[f32], range: &Range<usize>) -> f32 {
    gaps[range.start..range.end - 1].iter().sum()
}

fn equal_track_size(gaps: &[f32], span: &Range<usize>, needed: f32) -> f32 {
    (needed - internal_gaps(gaps, span)).max(0.0) / span.len() as f32
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
