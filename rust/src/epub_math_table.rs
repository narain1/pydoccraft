//! Place native equation cells on a bounded MathML table grid.
use std::{ops::Range, sync::Arc};

use roxmltree::Node;
use wordcraft_doc::{
    math::{self, Arg, ColJc, MNode, Math},
    para::InlineObject,
    props::{Align, Borders, CharProps, HeightRule, Rgb, TextColor},
    resolve::ResolvedChar,
    Block, Document, Paragraph, Table as NativeTable,
};
use wordcraft_fonts::math::{resolve_math, C};
use wordcraft_layout::math::{MItem, MathLayout};

use super::{
    compose, invalid, limited, shares_line_width, size_points, unsupported, Presentation,
    Projection, MATHML, MAX_MATRIX_SIZE,
};
use crate::EngineError;

#[path = "epub_math_table_alignment.rs"]
pub(super) mod alignment;
#[path = "epub_math_table_rules.rs"]
mod rules;
use alignment::Anchor;
use rules::{Occupancy, Rules};

const MAX_ROWS: usize = 128;
const MAX_DIMENSION: f32 = 20_000.0;
const WIDTH_EPSILON: f32 = 0.0001;
const TABLE_WIDTH_EPSILON: f32 = 0.01;

pub(in crate::epub) struct Table {
    cells: Vec<Cell>,
    row_gaps: Vec<f32>,
    column_gaps: Vec<f32>,
    column_widths: Vec<ColumnWidth>,
    width: Option<f32>,
    equal_rows: bool,
    equal_columns: bool,
    labels: Option<Labels>,
    rules: Rules,
    rule_color: Option<Rgb>,
    alignment: Vertical,
    alignment_row: Option<usize>,
    em: f32,
    enclosing_em: f32,
}

struct Cell {
    rows: Range<usize>,
    // Row labels occupy their own track outside the table's data columns.
    columns: Range<usize>,
    horizontal: ColJc,
    vertical: Vertical,
    anchor: Option<Anchor>,
    content: CellContent,
    format: CharProps,
}

struct CellPlacement<'xml, 'input> {
    node: Node<'xml, 'input>,
    rows: Range<usize>,
    columns: Range<usize>,
    horizontal: ColJc,
    vertical: Vertical,
    group: Option<alignment::Group>,
    style: Presentation<'xml>,
}

enum CellContent {
    Equation(Math),
    Table(Table),
}

struct Labels {
    left: bool,
    gap: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum ColumnWidth {
    Auto,
    Fit,
    Fixed(f32),
}

impl ColumnWidth {
    fn fixed(self) -> Option<f32> {
        match self {
            Self::Fixed(width) => Some(width),
            Self::Auto | Self::Fit => None,
        }
    }
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
        || alignment::table_has_groups(table)
        || [
            "align",
            "rowalign",
            "rowspacing",
            "columnspacing",
            "columnwidth",
        ]
        .into_iter()
        .any(|key| table.has_attribute(key))
        || table
            .attribute("width")
            .is_some_and(|width| width.trim() != "auto")
        || ["equalrows", "equalcolumns"].into_iter().any(|key| {
            table
                .attribute(key)
                .is_some_and(|value| value.trim() != "false")
        })
        || table.children().filter(Node::is_element).any(|row| {
            row.has_tag_name((MATHML, "mlabeledtr"))
                || cell_alignment(row)
                || row.children().filter(Node::is_element).any(cell_alignment)
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

pub(in crate::epub) fn has_relative_widths(root: Node<'_, '_>) -> bool {
    root.descendants().any(|node| {
        node.has_tag_name((MATHML, "mtable"))
            && node
                .attribute("width")
                .map(str::trim)
                .and_then(relative_width)
                .is_some()
            && is_presentation(node)
            && node
                .ancestors()
                .skip(1)
                .take_while(|ancestor| *ancestor != root)
                .all(shares_line_width)
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
    let outer_equation = group.has_tag_name((MATHML, "math"));
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
    let labeled = table
        .children()
        .any(|row| row.has_tag_name((MATHML, "mlabeledtr")));
    // Ruled standalone labels share native rows with the bordered grid.
    // Other labeled grids and alignment groups use editable equation tracks.
    if alignment::table_has_groups(table) || labeled && !(outer_equation && rules::requested(table))
    {
        return Ok(None);
    }
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
    let mut placements = Vec::new();
    let em = style.font_size();
    let mut labels = None;
    for (row_index, row) in rows.iter().copied().enumerate() {
        let labeled = row.has_tag_name((MATHML, "mlabeledtr"));
        if !labeled && !row.has_tag_name((MATHML, "mtr")) {
            return Err(unsupported("MathML table row type"));
        }
        if labeled && !row.children().any(|node| node.is_element()) {
            return Err(invalid("missing MathML table row label"));
        }
        if labeled && labels.is_none() {
            labels = Some(Labels::read(table, em)?);
        }
        projection.charge(depth + 1)?;
        let row_style = projection.style(row, style)?;
        let mut column = 0;
        for (index, cell) in row.children().filter(Node::is_element).enumerate() {
            if !cell.has_tag_name((MATHML, "mtd")) {
                return Err(invalid("unexpected MathML table child"));
            }
            let label = labeled && index == 0;
            while !label && column < MAX_MATRIX_SIZE && occupied[column] > row_index {
                column += 1;
            }
            let horizontal = horizontal_alignment(
                cell.attribute("columnalign")
                    .or_else(|| indexed(row.attribute("columnalign"), column))
                    .or_else(|| {
                        if label {
                            None
                        } else {
                            indexed(table.attribute("columnalign"), column)
                        }
                    })
                    .unwrap_or("center"),
            )?;
            let row_alignment = cell
                .attribute("rowalign")
                .or_else(|| row.attribute("rowalign"))
                .or_else(|| indexed(table.attribute("rowalign"), row_index))
                .unwrap_or("baseline");
            let vertical = Vertical::parse(row_alignment.trim())
                .ok_or_else(|| unsupported("MathML table row alignment"))?;
            projection.charge(depth + 2)?;
            let style = projection.style(cell, row_style)?;
            let rows_spanned = span(cell, "rowspan")?;
            let columns_spanned = span(cell, "columnspan")?;
            let group = alignment::leading_group(cell)
                .map(|marker| {
                    if label || columns_spanned != 1 {
                        return Err(unsupported(
                            "MathML alignment group without one data column",
                        ));
                    }
                    alignment::read(table, row, cell, marker, column)
                })
                .transpose()?;
            if label {
                if rows_spanned != 1 || columns_spanned != 1 {
                    return Err(unsupported("spanning MathML table row label"));
                }
                placements.push(CellPlacement {
                    node: cell,
                    rows: row_index..row_index + 1,
                    columns: 0..0,
                    horizontal,
                    vertical,
                    group,
                    style,
                });
                continue;
            }
            let right = column + columns_spanned;
            let bottom = row_index + rows_spanned;
            if right > MAX_MATRIX_SIZE {
                return Err(limited("MathML table exceeds 32 columns"));
            }
            if bottom > rows.len() || occupied[column..right].iter().any(|end| *end > row_index) {
                return Err(invalid(
                    "MathML table spans overlap or extend beyond the rows",
                ));
            }
            occupied[column..right].fill(bottom);
            placements.push(CellPlacement {
                node: cell,
                rows: row_index..bottom,
                columns: column..right,
                horizontal,
                vertical,
                group,
                style,
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
    let margin_cells = usize::from(labels.is_some()) * 2 * rows.len();
    let implicit_cells = rows.len() * column_count + margin_cells - placements.len();
    *projection.remaining = projection
        .remaining
        .checked_sub(implicit_cells)
        .ok_or_else(|| limited("MathML table cells exceed the layout budget"))?;
    let mut row_gaps = gaps(table, "rowspacing", rows.len(), em, "1ex")?;
    let mut column_gaps = gaps(table, "columnspacing", column_count, em, "0.8em")?;
    let rules = Rules::read(table, em, &mut row_gaps, &mut column_gaps)?;
    let width = table_width(table, em, projection)?;
    let mut grid = Table {
        cells: Vec::with_capacity(placements.len()),
        row_gaps,
        column_gaps,
        column_widths: column_widths(table, column_count, em, width)?,
        width,
        equal_rows: boolean(table, "equalrows")?,
        equal_columns: boolean(table, "equalcolumns")?,
        labels,
        rules,
        rule_color: style.ink,
        alignment,
        alignment_row,
        em,
        enclosing_em,
    };
    let definite_columns = grid.definite_columns()?;
    for cell in placements {
        let content_width = if cell.columns.is_empty() {
            None
        } else {
            definite_columns[cell.columns.clone()]
                .iter()
                .copied()
                .sum::<Option<f32>>()
                .map(|width| width + internal_gaps(&grid.column_gaps, &cell.columns))
        };
        let (content, anchor) =
            projection.with_cell_width(cell.node, content_width, |projection| {
                if cell.group.is_none() {
                    if let Some(table) =
                        project_group(projection, cell.node, depth + 2, cell.style)?
                    {
                        return Ok((CellContent::Table(table), None));
                    }
                }
                let (nodes, caret) = projection
                    .with_alignment_group(cell.group.map(|group| group.scope), |projection| {
                        projection.elements(cell.node, depth + 2, cell.style)
                    })?;
                let mut nodes = math::merge_runs(nodes);
                let anchor = cell
                    .group
                    .map(|group| group.resolve(caret, &mut nodes))
                    .transpose()?;
                Ok((
                    CellContent::Equation(Math {
                        nodes,
                        ..Default::default()
                    }),
                    anchor,
                ))
            })?;
        grid.cells.push(Cell {
            rows: cell.rows,
            columns: cell.columns,
            horizontal: cell.horizontal,
            vertical: cell.vertical,
            anchor,
            content,
            format: CharProps {
                size: Some(cell.style.font_size()),
                color: cell.style.ink.map(TextColor::Rgb),
                ..Default::default()
            },
        });
    }
    Ok(grid)
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

fn horizontal_alignment(value: &str) -> Result<ColJc, EngineError> {
    match value.trim() {
        "left" => Ok(ColJc::Left),
        "right" => Ok(ColJc::Right),
        "center" => Ok(ColJc::Center),
        _ => Err(unsupported("MathML table column alignment")),
    }
}

impl Labels {
    fn read(table: Node<'_, '_>, em: f32) -> Result<Self, EngineError> {
        let left = match table.attribute("side").unwrap_or("right").trim() {
            "left" => true,
            "right" => false,
            "leftoverlap" | "rightoverlap" => {
                return Err(unsupported("overlapping MathML table row labels"));
            }
            _ => return Err(invalid("invalid MathML table label side")),
        };
        let default = 0.8 * em;
        let gap = match table.attribute("minlabelspacing").map(str::trim) {
            None => Some(default),
            Some(value) => match value.strip_suffix('%') {
                Some(percent) => percent
                    .parse::<f32>()
                    .ok()
                    .map(|value| value / 100.0 * default),
                None => value
                    .parse::<f32>()
                    .ok()
                    .map(|factor| factor * default)
                    .or_else(|| length(value, em)),
            },
        }
        .filter(|gap| (0.0..=MAX_DIMENSION).contains(gap))
        .ok_or_else(|| unsupported("MathML table label spacing"))?;
        Ok(Self { left, gap })
    }
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
    table_width: Option<f32>,
) -> Result<Vec<ColumnWidth>, EngineError> {
    (0..columns)
        .map(|column| {
            let value = indexed(table.attribute("columnwidth"), column).unwrap_or("auto");
            if value == "auto" {
                return Ok(ColumnWidth::Auto);
            }
            let definite = || {
                table_width.ok_or_else(|| {
                    unsupported(
                        "MathML fitted or percentage column widths require a definite table width",
                    )
                })
            };
            if value == "fit" {
                definite()?;
                return Ok(ColumnWidth::Fit);
            }
            let width = if let Some(percent) = value.strip_suffix('%') {
                let total = definite()?;
                percent
                    .parse::<f32>()
                    .ok()
                    .filter(|value| (0.0..=100.0).contains(value))
                    .map(|value| total * (value / 100.0))
            } else {
                fixed_length(value, em)
            };
            width
                .map(ColumnWidth::Fixed)
                .ok_or_else(|| unsupported("MathML table column width"))
        })
        .collect()
}

fn fixed_length(value: &str, em: f32) -> Option<f32> {
    if value.ends_with('%') || value.ends_with("vw") || value.ends_with("vh") {
        return None;
    }
    length(value, em)
}

fn relative_width(value: &str) -> Option<f32> {
    if let Some(percent) = value.strip_suffix('%') {
        return percent.parse::<f32>().ok().map(|value| value / 100.0);
    }
    if value == "0" {
        return None;
    }
    value.parse::<f32>().ok()
}

fn table_width<'xml>(
    table: Node<'xml, '_>,
    em: f32,
    projection: &Projection<'_, 'xml>,
) -> Result<Option<f32>, EngineError> {
    let Some(value) = table
        .attribute("width")
        .map(str::trim)
        .filter(|value| *value != "auto")
    else {
        return Ok(None);
    };
    if let Some(fraction) = relative_width(value) {
        let line_width = projection.containing_width(table).ok_or_else(|| {
            unsupported("MathML relative table width requires a definite containing line width")
        })?;
        if !(0.0..=1.0).contains(&fraction) {
            return Err(unsupported(
                "MathML relative table width is outside the containing line",
            ));
        }
        let width = line_width * fraction;
        if width > MAX_DIMENSION {
            return Err(limited("MathML table exceeds its geometry budget"));
        }
        return Ok(Some(width));
    }
    fixed_length(value, em)
        .map(Some)
        .ok_or_else(|| unsupported("MathML table width requires a definite length"))
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
    label_width: f32,
    label_inset: f32,
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

struct MeasuredCell {
    drawing: CellDrawing,
    baseline_offset: Option<f32>,
    anchor: Option<f32>,
}

impl MeasuredCell {
    fn width(&self, column: usize, anchors: &[[f32; 2]]) -> f32 {
        if self.anchor.is_some() {
            anchors[column].iter().sum()
        } else {
            self.drawing.size().0
        }
    }
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
        let automatic = |column: &ColumnWidth| *column == ColumnWidth::Auto;
        if self.width.is_some() {
            // Process spans by increasing right edge. Putting each deficit
            // in the last automatic track minimizes their total width.
            let last = tracks
                .iter()
                .rposition(automatic)
                .ok_or_else(|| unsupported("MathML cell exceeds fixed column widths"))?;
            widths[span.start + last] += excess;
        } else {
            let flexible = tracks.iter().filter(|column| automatic(column)).count();
            if flexible == 0 {
                return Err(unsupported("MathML cell exceeds fixed column widths"));
            }
            let share = excess / flexible as f32;
            for (width, column) in widths[span.clone()].iter_mut().zip(tracks) {
                if automatic(column) {
                    *width += share;
                }
            }
        }
        Ok(())
    }

    fn declared_track_width(&self, label_inset: f32) -> Result<Option<f32>, EngineError> {
        let Some(width) = self.width else {
            return Ok(None);
        };
        let remaining = width
            - self.column_gaps.iter().sum::<f32>()
            - 2.0 * (self.rules.inset()[0] + label_inset);
        if remaining < -TABLE_WIDTH_EPSILON {
            return Err(unsupported(if label_inset > 0.0 {
                "MathML labeled contents exceed the authored table width"
            } else {
                "MathML table width is smaller than its gaps and frame"
            }));
        }
        Ok(Some(remaining.max(0.0)))
    }

    fn definite_columns(&self) -> Result<Vec<Option<f32>>, EngineError> {
        // A labeled grid's free space depends on the measured label. Its
        // authored fixed tracks remain available for nested relative widths.
        let total = if self.labels.is_some() {
            None
        } else {
            self.declared_track_width(0.0)?
        };
        let widths: Vec<_> = self
            .column_widths
            .iter()
            .map(|column| column.fixed())
            .collect();
        // Equal columns have a definite width if the table or at least
        // one fixed track supplies it. Otherwise content sizes decide it.
        if self.equal_columns && (total.is_some() || widths.iter().any(Option::is_some)) {
            return Ok(vec![
                Some(self.equal_column_width(0.0, total)?);
                widths.len()
            ]);
        }
        // Fitted tracks are definite when none of the tracks needs an
        // intrinsic size. A lone automatic track is also definite when
        // all other tracks have fixed widths and it receives the remainder.
        let automatic = self
            .column_widths
            .iter()
            .filter(|&&column| column == ColumnWidth::Auto)
            .count();
        let definite_remainder =
            automatic == 0 || automatic == 1 && !self.column_widths.contains(&ColumnWidth::Fit);
        if let Some(total) = total.filter(|_| definite_remainder) {
            let mut allocated = widths
                .iter()
                .map(|width| width.unwrap_or_default())
                .collect::<Vec<_>>();
            self.fill_width(&mut allocated, total)?;
            return Ok(allocated.into_iter().map(Some).collect());
        }
        Ok(widths)
    }

    fn fill_width(&self, widths: &mut [f32], total: f32) -> Result<(), EngineError> {
        let remaining = total - widths.iter().sum::<f32>();
        if remaining < -TABLE_WIDTH_EPSILON {
            return Err(unsupported(
                "MathML table contents exceed the authored width",
            ));
        }
        let sharing = if self.column_widths.contains(&ColumnWidth::Fit) {
            ColumnWidth::Fit
        } else {
            ColumnWidth::Auto
        };
        let tracks = self
            .column_widths
            .iter()
            .filter(|&&column| column == sharing)
            .count();
        if tracks == 0 {
            if remaining > TABLE_WIDTH_EPSILON {
                return Err(unsupported(
                    "fixed MathML columns cannot fill the authored table width",
                ));
            }
            return Ok(());
        }
        let share = remaining.max(0.0) / tracks as f32;
        for (width, &column) in widths.iter_mut().zip(&self.column_widths) {
            if column == sharing {
                *width += share;
            }
        }
        Ok(())
    }

    fn equal_column_width(&self, minimum: f32, total: Option<f32>) -> Result<f32, EngineError> {
        let mut fixed_widths = self
            .column_widths
            .iter()
            .filter_map(|column| column.fixed());
        let width = if let Some(total) = total {
            total / self.column_widths.len() as f32
        } else {
            fixed_widths.next().unwrap_or(minimum)
        };
        if fixed_widths.any(|fixed| (fixed - width).abs() > WIDTH_EPSILON) {
            return Err(unsupported(
                "MathML equal columns request conflicting fixed widths",
            ));
        }
        if minimum > width + WIDTH_EPSILON {
            return Err(unsupported("MathML cell exceeds equal column widths"));
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
        let mut widths = layout.widths;
        let mut gaps = self.column_gaps;
        if self.labels.is_some() {
            widths.insert(0, layout.label_width);
            widths.push(layout.label_width);
            let gap = layout.label_inset - layout.label_width;
            gaps.insert(0, gap);
            gaps.push(gap);
        }
        let native_columns = matrix_columns(&widths, &gaps, column_gap);
        let split_columns = native_columns.is_none();
        let (origins, advances) = native_columns
            .or_else(|| matrix_columns(&widths, &gaps, 0.0))
            .ok_or_else(|| invalid("invalid MathML matrix column origins"))?;
        let rows = layout.heights.len();
        let columns = widths.len();
        let mut contents = vec![vec![Arg::new(); columns]; rows];
        let first_baseline = editor.axis() + (rows - 1) as f32 * row_pitch / 2.0;
        // Native matrices have no cell spans. Give the logical slots their
        // measured widths independently of the expressions' painted bounds.
        // Each expression can then extend into slots covered by its span.
        for (cell, (left, top, drawing)) in self.cells.into_iter().zip(layout.cells) {
            let row = cell.rows.start;
            let column = match &self.labels {
                Some(labels) if cell.columns.is_empty() => {
                    if labels.left {
                        0
                    } else {
                        columns - 1
                    }
                }
                Some(_) => cell.columns.start + 1,
                None => cell.columns.start,
            };
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
            let advance = (left - origins[column]).max(0.0);
            let mut cell_body = if cell.anchor.is_some() {
                compose::group_leading_space(advance)?
            } else {
                compose::grid_gap(advance)?.into_iter().collect()
            };
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
        let mut matrices = if split_columns {
            // The pinned native matrix always adds an inter-column gap.
            // Single-column matrices use the same row baselines and can
            // sit side by side with the independently measured advances.
            let mut tracks = vec![Vec::with_capacity(rows); columns];
            for row in contents {
                for (track, cell) in tracks.iter_mut().zip(row) {
                    track.push(vec![cell]);
                }
            }
            tracks
                .into_iter()
                .map(|rows| MNode::Matrix {
                    rows,
                    col_jc: vec![ColJc::Left],
                })
                .collect::<Arg>()
        } else {
            vec![MNode::Matrix {
                rows: contents,
                col_jc: vec![ColJc::Left; columns],
            }]
        };
        matrices.push(compose::grid_marker());
        let mut nodes = vec![MNode::Phant {
            show: true,
            zero_wid: false,
            zero_asc: true,
            zero_desc: true,
            e: matrices,
        }];
        editor.add_extents(
            &mut nodes,
            [Some(layout.baseline), Some(layout.height - layout.baseline)],
        )?;
        Ok(MNode::Boxed { e: nodes })
    }

    pub(in crate::epub) fn layout(
        &self,
        doc: &Document,
        format: &CharProps,
        measurements: &compose::Measurements,
    ) -> Result<Layout, EngineError> {
        self.layout_with_rules(
            doc,
            format,
            measurements,
            &mut { rules::MAX_RULE_ITEMS },
            None,
        )
    }

    pub(super) fn layout_editable(
        &self,
        doc: &Document,
        format: &CharProps,
        carrier: &ResolvedChar,
    ) -> Result<Layout, EngineError> {
        self.layout_with_rules(
            doc,
            format,
            &compose::Measurements::default(),
            &mut { rules::MAX_RULE_ITEMS },
            Some(carrier),
        )
    }

    fn layout_with_rules(
        &self,
        doc: &Document,
        format: &CharProps,
        measurements: &compose::Measurements,
        rule_budget: &mut usize,
        editable_carrier: Option<&ResolvedChar>,
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
            .map(|column| column.fixed().unwrap_or_default())
            .collect();
        let mut ascent = vec![0.0f32; row_count];
        let mut descent = vec![0.0f32; row_count];
        let mut row_baselines = vec![false; row_count];
        let mut heights = vec![0.0; row_count];
        let mut measures = Vec::with_capacity(self.cells.len());
        let mut anchors = vec![[0.0f32; 2]; widths.len()];
        let mut marked_columns = [false; MAX_MATRIX_SIZE];
        if editable_carrier.is_some() {
            for cell in &self.cells {
                if matches!(cell.anchor, Some(Anchor::Marked(..))) {
                    marked_columns[cell.columns.start] = true;
                }
            }
        }
        let mut label_width = 0.0f32;
        for cell in &self.cells {
            let chars = CharProps {
                size: cell.format.size,
                color: cell.format.color.or(format.color),
                ..Default::default()
            };
            let resolved = if let Some(carrier) = editable_carrier
                .filter(|_| cell.anchor.is_some() && marked_columns[cell.columns.start])
            {
                // A native operator's spacing follows the Word paragraph's
                // math size even when its runs have authored point sizes.
                let mut resolved = carrier.clone();
                resolved.color = chars.color.unwrap_or(resolved.color);
                resolved
            } else {
                doc.styles.resolve_char(None, &chars)
            };
            let measure = match &cell.content {
                CellContent::Table(table) => CellDrawing::Table(table.layout_with_rules(
                    doc,
                    &chars,
                    measurements,
                    rule_budget,
                    editable_carrier,
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
            let (cell_width, cell_ascent, cell_descent) = measure.size();
            let anchor = if let Some(anchor) = cell.anchor {
                let CellDrawing::Equation(equation) = &measure else {
                    return Err(invalid("MathML alignment group has no equation"));
                };
                let anchor = anchor.measure(equation)?;
                let [left, right] = &mut anchors[cell.columns.start];
                *left = left.max(anchor);
                *right = right.max(cell_width - anchor);
                Some(anchor)
            } else {
                None
            };
            if cell.columns.is_empty() {
                label_width = label_width.max(cell_width);
            }
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
            measures.push(MeasuredCell {
                drawing: measure,
                baseline_offset: offset,
                anchor,
            });
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
        let minimum_label_inset = self
            .labels
            .as_ref()
            .map_or(0.0, |labels| label_width + labels.gap);
        let declared = self.declared_track_width(minimum_label_inset)?;
        // Size automatic data columns intrinsically. Fit columns can use
        // remaining space once both outside tracks reserve the label gap.
        let fill = declared
            .filter(|_| self.labels.is_none() || self.column_widths.contains(&ColumnWidth::Fit));
        if self.equal_columns {
            let common_width = self
                .cells
                .iter()
                .zip(&measures)
                .filter(|(cell, _)| !cell.columns.is_empty())
                .map(|(cell, measure)| {
                    let width = measure.width(cell.columns.start, &anchors);
                    equal_track_size(&self.column_gaps, &cell.columns, width)
                })
                .fold(0.0, f32::max);
            widths.fill(self.equal_column_width(common_width, fill)?);
        } else {
            let order = if declared.is_some() {
                ordered(|cell| cell.columns.end)
            } else {
                ordered(|cell| cell.columns.len())
            };
            // Automatic columns are measured before fitted columns share
            // the remaining width. Check spans using fit against that allocation.
            let mut fitted = Vec::new();
            for index in order {
                let span = &self.cells[index].columns;
                if span.is_empty() {
                    continue;
                }
                if self.column_widths[span.clone()].contains(&ColumnWidth::Fit) {
                    fitted.push(index);
                    continue;
                }
                self.grow_columns(
                    &mut widths,
                    span,
                    measures[index].width(span.start, &anchors),
                )?;
            }
            if let Some(total) = fill {
                self.fill_width(&mut widths, total)?;
            }
            for index in fitted {
                let span = &self.cells[index].columns;
                let room = extent(&widths, &self.column_gaps, span);
                if measures[index].width(span.start, &anchors) > room + TABLE_WIDTH_EPSILON {
                    return Err(unsupported("MathML cell exceeds fitted column widths"));
                }
            }
        }
        let mut common_height = heights.iter().copied().fold(0.0, f32::max);
        for index in ordered(|cell| cell.rows.len()) {
            let cell = &self.cells[index];
            let measure = &measures[index];
            let (_, cell_ascent, cell_descent) = measure.drawing.size();
            let top = measure
                .baseline_offset
                .map_or(0.0, |offset| ascent[cell.rows.start] + offset - cell_ascent);
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
        let grid_width = x[widths.len()] + left_inset * 2.0;
        let label_inset = match self.width {
            Some(width) if self.labels.is_some() => {
                let available = (width - grid_width) / 2.0;
                if available < minimum_label_inset - TABLE_WIDTH_EPSILON {
                    return Err(unsupported(
                        "MathML labeled contents exceed the authored table width",
                    ));
                }
                available.max(minimum_label_inset)
            }
            _ => minimum_label_inset,
        };
        let width = grid_width + 2.0 * label_inset;
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
            .map(|(cell, measure)| {
                let cell_width = measure.width(cell.columns.start, &anchors);
                let (_, cell_ascent, cell_descent) = measure.drawing.size();
                let (left, track_width) = match &self.labels {
                    Some(labels) if cell.columns.is_empty() => (
                        if labels.left {
                            0.0
                        } else {
                            width - label_width
                        },
                        label_width,
                    ),
                    _ => (
                        label_inset + left_inset + x[cell.columns.start],
                        extent(&widths, &self.column_gaps, &cell.columns),
                    ),
                };
                let horizontal_room = track_width - cell_width;
                let anchor_shift = measure
                    .anchor
                    .map_or(0.0, |anchor| anchors[cell.columns.start][0] - anchor);
                let vertical_room =
                    extent(&heights, &self.row_gaps, &cell.rows) - cell_ascent - cell_descent;
                let left = left
                    + anchor_shift
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
                            ascent[cell.rows.start] + measure.baseline_offset.unwrap_or_default()
                                - cell_ascent
                        }
                    };
                (left, top, measure.drawing)
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
        let mut rules = occupancy
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
        if label_inset != 0.0 {
            for rule in &mut rules {
                compose::position(rule, label_inset, 0.0, 1.0);
            }
        }
        Ok(Layout {
            width,
            height,
            baseline,
            cells,
            widths,
            heights,
            row_positions: y.into_iter().map(|start| start + top_inset).collect(),
            label_width,
            label_inset,
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
        let mut widths = margins_for_tracks(&layout.widths, &self.column_gaps, frame_inline);
        let heights = margins_for_tracks(&layout.heights, &self.row_gaps, frame_block);
        let [nrows, ncols] = [heights.len(), widths.len()];
        let data_start = usize::from(self.labels.is_some());
        if self.labels.is_some() {
            // Native edges paint on their center, so each outside track
            // includes half the frame stroke without entering the label gap.
            let half_frame = self.rules.inset()[0] - frame_inline;
            let inset = (layout.label_inset + half_frame).max(0.1);
            widths.insert(0, inset);
            widths.push(inset);
        }
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
        let vertical = self
            .cells
            .iter()
            .zip(&layout.cells)
            .map(|(cell, (_, top, _))| {
                self.native_vertical_margins(cell, *top, &layout, frame_block)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut row_margins = vec![[f32::INFINITY, 0.0]; nrows];
        if self.labels.is_some() {
            for (cell, &[before, _]) in self.cells.iter().zip(&vertical) {
                let row = &mut row_margins[cell.rows.start][0];
                *row = row.min(before);
            }
            for (row, [before, after]) in row_margins.iter_mut().enumerate() {
                if !before.is_finite() {
                    *before = side_gap(&self.row_gaps, row.checked_sub(1));
                }
                *after = if row + 1 == nrows {
                    let [_, band] = self.rules.native_vertical_bands(row, row + 1);
                    (frame_block - band).max(0.0)
                } else {
                    side_gap(&self.row_gaps, Some(row))
                };
            }
        }
        for (index, (row, height)) in native.rows.iter_mut().zip(heights).enumerate() {
            row.props.height_rule = HeightRule::AtLeast;
            row.props.height = Some(if self.labels.is_some() {
                layout.heights[index]
            } else {
                height
            });
            for (cell, width) in row.cells.iter_mut().zip(&native.grid) {
                cell.props.width = Some(*width);
                cell.props.hide_mark = true;
                if self.labels.is_some() {
                    let [before, after] = row_margins[index];
                    cell.props.margins = Some([before, 0.0, after, 0.0]);
                }
            }
        }
        for ((cell, (_, _, drawing)), [mut before, mut after]) in
            self.cells.into_iter().zip(layout.cells).zip(vertical)
        {
            let row = cell.rows.start;
            let last_row = cell.rows.end - 1;
            let label = self.labels.as_ref().filter(|_| cell.columns.is_empty());
            let columns = if let Some(labels) = label {
                let column = if labels.left { 0 } else { data_start + ncols };
                column..column + 1
            } else {
                cell.columns.start + data_start..cell.columns.end + data_start
            };
            if row != last_row || columns.len() != 1 {
                native.merge(row, last_row, columns.start, columns.end - 1);
            }
            let native_col = native
                .cell_at_grid(row, columns.start)
                .ok_or_else(|| invalid("missing MathML table cell"))?;
            let target = &mut native.rows[row].cells[native_col];
            let cell_width: f32 = native.grid[columns].iter().sum();
            target.props.width = Some(cell_width);
            target.props.no_wrap = true;
            let [left, right] = if let Some(labels) = label {
                let gap = (cell_width - layout.label_width).max(0.0);
                if labels.left {
                    [0.0, gap]
                } else {
                    [gap, 0.0]
                }
            } else {
                let start = cell.columns.start;
                let end = cell.columns.end;
                [
                    if start == 0 {
                        frame_inline
                    } else {
                        side_gap(&self.column_gaps, Some(start - 1))
                    },
                    if end == ncols {
                        frame_inline
                    } else {
                        side_gap(&self.column_gaps, Some(end - 1))
                    },
                ]
            };
            // Word uses the row's largest top margin in every cell and
            // adds border clearance individually. Retain each label's and
            // data cell's remaining offset as native paragraph spacing.
            let space_before = if self.labels.is_some() {
                let additional = before - row_margins[row][0];
                [before, after] = row_margins[row];
                additional
            } else {
                0.0
            };
            let margins = [before, left, after, right];
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
                    if space_before > TABLE_WIDTH_EPSILON {
                        return Err(unsupported(
                            "MathML labeled table cannot vertically offset a nested Word table",
                        ));
                    }
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
                    paragraph.props.space_before = Some(space_before);
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
                .set_native_edges(&mut native, &map, layout.rule_color, data_start);
        }
        Ok(native)
    }

    fn native_vertical_margins(
        &self,
        cell: &Cell,
        top: f32,
        layout: &Layout,
        frame_block: f32,
    ) -> Result<[f32; 2], EngineError> {
        let start = cell.rows.start;
        let end = cell.rows.end;
        let mut before = (top - layout.row_positions[start]).max(0.0)
            + side_gap(&self.row_gaps, start.checked_sub(1));
        let mut after = side_gap(&self.row_gaps, Some(end - 1));
        if start == 0 {
            before += frame_block;
        }
        if end == layout.heights.len() {
            after += frame_block;
        }
        if !cell.columns.is_empty() {
            let [top_band, bottom_band] = self.rules.native_vertical_bands(start, end);
            before -= top_band;
            after -= bottom_band;
        }
        if before < -WIDTH_EPSILON || after < -WIDTH_EPSILON {
            return Err(unsupported(
                "MathML table frame or row gap is smaller than native DOCX border clearance",
            ));
        }
        Ok([before.max(0.0), after.max(0.0)])
    }
}

fn matrix_columns(widths: &[f32], gaps: &[f32], native_gap: f32) -> Option<(Vec<f32>, Vec<f32>)> {
    let mut origins = Vec::with_capacity(widths.len());
    let mut advances = Vec::with_capacity(widths.len());
    let mut current = 0.0;
    for (index, desired_end) in starts(widths, gaps).into_iter().skip(1).enumerate() {
        let gap = if index + 1 < widths.len() {
            native_gap
        } else {
            0.0
        };
        let reserve = desired_end - current - gap;
        if reserve < -0.0001 {
            return None;
        }
        // Round cumulative origins down so the cell's correction stays
        // nonnegative. The minimum native math space carries 0.4 points.
        let units = (reserve * 5.0 + 0.0001).floor();
        let advance = if units < 2.0 { 0.0 } else { units / 5.0 };
        origins.push(current);
        advances.push(advance);
        current += advance + gap;
    }
    Some((origins, advances))
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
