//! Supply measured MathML boxes to the pinned equation shaper.
use std::{
    cell::{Cell, OnceCell},
    collections::{HashMap, HashSet},
};

use roxmltree::Node;
use wordcraft_doc::{
    math::{Arg, ColJc, MNode, MRun, Math},
    math_linear::map_args,
    props::{CharProps, Rgb},
    resolve::ResolvedChar,
    Document,
};
use wordcraft_fonts::math::{resolve_math, C};
use wordcraft_layout::math::{layout_equation, MItem, MathLayout};

use super::{css, invalid, limited, table, unsupported, zero_length, Presentation, MATHML};
use crate::{docx::math_spacing, EngineError};

const BOX_PREFIX: &str = "\0epub-math-box-";
const GUIDE_SIZE: f32 = 16.0;
pub(super) const MAX_GEOMETRY: f32 = 20_000.0;
const LIMIT_BOXES: usize = 512;
const MAX_SAVED_DRAW_UNITS: usize = 250_000;
// The pinned WordCraft equation shaper stops at this many native nodes.
const MAX_EDITABLE_NODES: usize = 200_000;

#[derive(Default)]
pub(in crate::epub) struct Extras {
    nodes: Vec<Extra>,
    occupied_colors: HashSet<[u8; 3]>,
}

#[derive(Default)]
pub(in crate::epub) struct Editable {
    boxes: Vec<Cell<Option<MNode>>>,
}

pub(super) struct EditableLayout<'a> {
    document: &'a Document,
    carrier: ResolvedChar,
    guide_ratios: OnceCell<[f32; 2]>,
    matrix_spacing: OnceCell<[f32; 2]>,
}

pub(super) enum Extra {
    Table(table::Table, Box<CharProps>),
    Padded(Box<Padding>),
    Fraction(Box<super::fraction::Fraction>),
    Space([f32; 3]),
}

pub(in crate::epub) struct Padding {
    body: Arg,
    format: CharProps,
    lengths: [Option<Length>; 5],
}

#[derive(Clone, Copy)]
struct Length {
    value: f32,
    basis: Option<usize>,
    relative: bool,
}

impl Length {
    fn resolve(self, dimension: usize, original: [f32; 3]) -> f32 {
        let value = self.value * self.basis.map_or(1.0, |basis| original[basis]);
        if self.relative {
            original[dimension] + value
        } else {
            value
        }
    }
}

struct Measured {
    placeholder: MNode,
    layout: MathLayout,
}

#[derive(Default)]
pub(in crate::epub) struct Measurements {
    boxes: Vec<Measured>,
    markers: HashMap<[u8; 3], usize>,
    draw_units: usize,
}

pub(in crate::epub) fn selected(node: Node<'_, '_>) -> bool {
    match node.tag_name().name() {
        "mspace" => ["height", "depth"]
            .iter()
            .any(|&key| node.attribute(key).is_some_and(|value| !zero_length(value))),
        "mfrac" => {
            node.attribute("bevelled").map(str::trim) != Some("true")
                && super::fraction::Style::read(node, 1.0, [100.0; 2])
                    .native()
                    .is_none()
        }
        "mpadded" => ["width", "height", "depth", "lspace", "voffset"]
            .iter()
            .any(|&key| node.attribute(key).is_some_and(|value| !zero_length(value))),
        "mtable" => table::has_authored_geometry(node),
        _ => false,
    }
}

pub(in crate::epub) fn needs_layout(root: Node<'_, '_>) -> bool {
    root.descendants().any(|node| {
        node.tag_name().namespace() == Some(MATHML)
            && selected(node)
            && !node
                .ancestors()
                .any(|anc| matches!(anc.tag_name().name(), "annotation" | "annotation-xml"))
    })
}

impl Extras {
    pub(in crate::epub) fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub(super) fn remember(&mut self, color: Rgb) {
        self.occupied_colors.insert([color.0, color.1, color.2]);
    }

    pub(super) fn push(&mut self, node: Extra) -> Result<Arg, EngineError> {
        let index = self.nodes.len();
        if index >= LIMIT_BOXES {
            return Err(limited("MathML equation exceeds its composed box limit"));
        }
        self.nodes.push(node);
        Ok(vec![MNode::Run(MRun {
            font: Some(format!("{BOX_PREFIX}{index}")),
            ..MRun::plain("\u{2062}")
        })])
    }

    pub(in crate::epub) fn editable(
        self,
        document: &Document,
        paragraph_style: Option<&str>,
    ) -> Result<Editable, EngineError> {
        let mut editable = Editable::default();
        let layout = EditableLayout {
            document,
            carrier: document
                .styles
                .resolve_char(paragraph_style, &CharProps::default()),
            guide_ratios: OnceCell::new(),
            matrix_spacing: OnceCell::new(),
        };
        for extra in self.nodes {
            let node = match extra {
                Extra::Table(mut table, format) => {
                    table.apply_editable(&editable)?;
                    let measured = table.layout(document, &format, &Measurements::default())?;
                    table.editable_matrix(measured, &layout)?
                }
                Extra::Padded(padding) => padding.editable(&editable, &layout)?,
                Extra::Space([width, ascent, descent]) => {
                    let mut nodes: Arg = editable_gap(width)?.into_iter().collect();
                    layout.add_extents(&mut nodes, [Some(ascent), Some(descent)])?;
                    if nodes.is_empty() {
                        MNode::Run(MRun::plain(""))
                    } else {
                        MNode::Boxed { e: nodes }
                    }
                }
                _ => return Err(unsupported("MathML composed geometry in DOCX export")),
            };
            editable.boxes.push(Cell::new(Some(node)));
        }
        Ok(editable)
    }

    pub(in crate::epub) fn measure(
        self,
        doc: &Document,
        format: &CharProps,
    ) -> Result<Measurements, EngineError> {
        let mut measurements = Measurements::default();
        if self.nodes.is_empty() {
            return Ok(measurements);
        }
        let base = doc.styles.resolve_char(None, format);
        let [top, bottom] = guide_ratios(&base);
        if top <= 0.0 || bottom <= 0.0 {
            return Err(invalid("missing MathML guide metrics"));
        }
        let mut colors = (1u32..=0xff_ffff)
            .map(|number| number.to_be_bytes())
            .map(|[_, r, g, b]| [r, g, b])
            .filter(|color| !self.occupied_colors.contains(color));
        let mut draw_units = 0usize;
        for extra in self.nodes {
            let [r, g, b] = colors
                .next()
                .ok_or_else(|| limited("MathML guide color budget"))?;
            let color = Rgb(r, g, b);
            let vertical_limit = if matches!(extra, Extra::Fraction(_)) {
                "MathML fraction rule or operands exceed the native vertical extent"
            } else {
                "MathML composed box exceeds the native vertical extent"
            };
            let mut layout = match extra {
                Extra::Table(table, local) => {
                    let style = CharProps {
                        color: local.color.or(format.color),
                        ..*local
                    };
                    table.layout(doc, &style, &measurements)?.into_math()
                }
                Extra::Padded(padding) => padding.layout(doc, format, &measurements)?,
                Extra::Fraction(fraction) => fraction.layout(doc, format, &measurements)?,
                Extra::Space([width, ascent, descent]) => MathLayout {
                    width,
                    ascent,
                    descent,
                    ..Default::default()
                },
            };
            layout.slots.clear();
            let units = layout
                .items
                .iter()
                .map(|item| match item {
                    MItem::Glyphs { glyphs, .. } => glyphs.len().max(1),
                    _ => 1,
                })
                .sum();
            super::super::charge(
                &mut draw_units,
                units,
                MAX_SAVED_DRAW_UNITS,
                "MathML composed drawing exceeds its item budget",
            )?;
            let placeholder = placeholder(&layout, color, top, bottom, vertical_limit)?;
            measurements
                .markers
                .insert([r, g, b], measurements.boxes.len());
            measurements.boxes.push(Measured {
                placeholder,
                layout,
            });
        }
        measurements.draw_units = draw_units;
        Ok(measurements)
    }
}

impl Editable {
    pub(in crate::epub) fn apply(&self, math: &mut Math) -> Result<(), EngineError> {
        let nodes = self.substitute(std::mem::take(&mut math.nodes))?;
        let too_deep = Cell::new(false);
        let node_count = Cell::new(0);
        math.nodes = check_editable_bounds(nodes, 0, &too_deep, &node_count);
        if too_deep.get() {
            return Err(limited(
                "editable MathML exceeds the DOCX equation depth limit",
            ));
        }
        if node_count.get() > MAX_EDITABLE_NODES {
            return Err(limited(
                "editable MathML exceeds the DOCX equation node limit",
            ));
        }
        Ok(())
    }

    fn substitute(&self, body: Arg) -> Result<Arg, EngineError> {
        let missing = Cell::new(false);
        let nodes = replace_boxes(body, &|index| {
            let node = self.boxes.get(index).and_then(Cell::take);
            missing.set(missing.get() || node.is_none());
            node
        });
        if missing.get() {
            return Err(invalid("missing editable MathML box"));
        }
        Ok(nodes)
    }
}

fn check_editable_bounds(
    nodes: Arg,
    level: usize,
    too_deep: &Cell<bool>,
    node_count: &Cell<usize>,
) -> Arg {
    if level >= wordcraft_doc::math::MAX_DEPTH / 2 && !nodes.is_empty() {
        // WordCraft advances its layout depth both into a node and into its argument.
        too_deep.set(true);
        return nodes;
    }
    node_count.set(node_count.get().saturating_add(nodes.len()));
    if node_count.get() > MAX_EDITABLE_NODES {
        return nodes;
    }
    nodes
        .into_iter()
        .map(|node| {
            map_args(node, &|arg| {
                check_editable_bounds(arg, level + 1, too_deep, node_count)
            })
        })
        .collect()
}

pub(super) fn padding(
    node: Node<'_, '_>,
    body: Arg,
    style: Presentation<'_>,
    viewport: [f32; 2],
) -> Result<Extra, EngineError> {
    let mut lengths = [None; 5];
    for (index, name) in ["width", "height", "depth", "lspace", "voffset"]
        .into_iter()
        .enumerate()
    {
        if let Some(value) = node.attribute(name) {
            lengths[index] = Some(length(value, index, style.font_size(), viewport)?);
        }
    }
    Ok(Extra::Padded(Box::new(Padding {
        body,
        format: format(style),
        lengths,
    })))
}

pub(super) fn space(
    node: Node<'_, '_>,
    style: Presentation<'_>,
    viewport: [f32; 2],
) -> Result<Extra, EngineError> {
    let mut values = [0.0; 3];
    for (index, key) in ["width", "height", "depth"].into_iter().enumerate() {
        if let Some(value) = node.attribute(key) {
            let parsed = length(value, index, style.font_size(), viewport)?;
            if parsed.basis.is_some() {
                return Err(unsupported("relative MathML mspace dimensions"));
            }
            values[index] = parsed.value.max(0.0);
        }
    }
    Ok(Extra::Space(values))
}

pub(super) fn format(style: Presentation<'_>) -> CharProps {
    CharProps {
        size: Some(style.font_size()),
        color: style.ink.map(wordcraft_doc::props::TextColor::Rgb),
        ..Default::default()
    }
}

fn length(raw: &str, key: usize, em: f32, viewport: [f32; 2]) -> Result<Length, EngineError> {
    let raw = raw.trim();
    let relative = key < 3 && raw.starts_with(['+', '-']);
    let basis = ["width", "height", "depth"]
        .iter()
        .enumerate()
        .find_map(|(index, name)| {
            raw.strip_suffix(name)
                .map(|literal| (index, literal.trim_end()))
        });
    let (value, basis) = if let Some((basis, value)) = basis {
        (factor(value)?, Some(basis))
    } else if raw.ends_with('%') {
        (
            factor(raw)?,
            Some(match key {
                3 => 0,
                4 => 1,
                _ => key,
            }),
        )
    } else if zero_length(raw) {
        (0.0, None)
    } else if let Ok(value) = raw.parse::<f32>() {
        (
            value,
            Some(match key {
                3 => 0,
                4 => 1,
                _ => key,
            }),
        )
    } else {
        let value = css::points(raw, em, viewport)
            .or_else(|| {
                raw.strip_suffix("ex")
                    .and_then(|n| n.parse::<f32>().ok())
                    .map(|n| n * em / 2.0)
            })
            .ok_or_else(|| unsupported("MathML padding or space length"))?;
        (value, None)
    };
    if !value.is_finite() || value.abs() > MAX_GEOMETRY {
        return Err(limited(
            "MathML padding or space exceeds its geometry budget",
        ));
    }
    Ok(Length {
        value,
        basis,
        relative,
    })
}

fn factor(value: &str) -> Result<f32, EngineError> {
    let (number, divisor) = value.strip_suffix('%').map_or((value, 1.0), |s| (s, 100.0));
    Ok(number.trim().parse::<f32>().map_err(invalid)? / divisor)
}

impl Padding {
    fn editable(
        self,
        editable: &Editable,
        layout: &EditableLayout<'_>,
    ) -> Result<MNode, EngineError> {
        let [width, height, depth, left, shift] = self.lengths;
        let unsupported_padding = || unsupported("MathML composed geometry in DOCX export");
        // Keep native width when adding a fixed horizontal gap. Other dimensions
        // may use any measured extent of the original operand as their basis.
        let additive_width = width.filter(|l| l.basis.is_none() && l.relative && l.value >= 0.0);
        let body = editable.substitute(self.body)?;
        let need_shift = shift.is_some_and(|length| length.value != 0.0);
        let need_overhang_width = (additive_width.is_some() || zeroed(width, 0) == Some(false))
            && left.is_some_and(|length| {
                length.basis.is_some() || length.value > additive_width.map_or(0.0, |l| l.value)
            });
        let needs_original = need_shift
            || need_overhang_width
            || [width, height, depth, left]
                .into_iter()
                .enumerate()
                .any(|(index, length)| {
                    length.is_some_and(|l| l.value != 0.0 && (l.relative || l.basis.is_some()))
                        && zeroed(length, index).is_none()
                        && !(index == 0 && additive_width.is_some())
                });
        let original = if needs_original && !body.is_empty() {
            let resolved = layout.document.styles.resolve_char(None, &self.format);
            let math = Math {
                nodes: body.clone(),
                ..Default::default()
            };
            let box_size = layout_equation(
                &math,
                "",
                &resolved,
                false,
                MAX_GEOMETRY,
                &Default::default(),
                &mut 0,
            );
            [box_size.width, box_size.ascent, box_size.descent]
        } else {
            [0.0; 3]
        };
        let (mut absolute_width, mut advance) = match (width, additive_width) {
            (_, Some(additive)) => (false, additive.value),
            (length, _) if zeroed(length, 0) == Some(false) => (false, 0.0),
            (Some(length), _) => (true, length.resolve(0, original).max(0.0)),
            (None, _) => (false, 0.0),
        };
        let left = left.map_or(0.0, |length| length.resolve(3, original));
        if left < 0.0 {
            return Err(unsupported_padding());
        }
        if !absolute_width && left > advance {
            // Reserve the original advance separately when the leading offset
            // overhangs the width added to the operand.
            absolute_width = true;
            advance += original[0];
        }
        let shift = shift.map_or(0.0, |length| length.resolve(4, original));
        if ![left, advance, shift.abs()]
            .into_iter()
            .all(|value| value <= MAX_GEOMETRY)
        {
            return Err(limited("MathML padding exceeds its geometry budget"));
        }
        let lengths = [height, depth];
        let mut vertical = std::array::from_fn(|index| match zeroed(lengths[index], index + 1) {
            Some(false) => None,
            Some(true) => Some(0.0),
            None => lengths[index].map(|length| length.resolve(index + 1, original).max(0.0)),
        });
        if body.is_empty() && vertical.iter().all(|v| v.unwrap_or_default() == 0.0) {
            // An empty visible phantom exposes WordCraft's argument placeholder.
            return Ok(editable_gap(advance)?.unwrap_or_else(|| MNode::Run(MRun::plain(""))));
        }
        let mut contents = Vec::new();
        let leading = if body.is_empty() { 0.0 } else { left };
        if !body.is_empty() {
            contents.extend(editable_gap(leading)?);
            contents.extend(body);
            if shift != 0.0 {
                contents = vec![layout.shift(contents, shift)?];
                for (extent, original) in vertical.iter_mut().zip(&original[1..]) {
                    *extent = extent.or(Some(*original));
                }
            }
            if absolute_width || vertical.iter().any(Option::is_some) {
                contents = vec![MNode::Phant {
                    show: true,
                    zero_wid: absolute_width,
                    zero_asc: vertical[0].is_some(),
                    zero_desc: vertical[1].is_some(),
                    e: contents,
                }];
            }
        }
        contents.extend(editable_gap(if absolute_width {
            advance
        } else {
            advance - leading
        })?);
        layout.add_extents(&mut contents, vertical)?;
        Ok(MNode::Boxed { e: contents })
    }

    fn layout(
        mut self,
        doc: &Document,
        inherited: &CharProps,
        measured: &Measurements,
    ) -> Result<MathLayout, EngineError> {
        if self.body.is_empty() {
            return self.bounds(MathLayout::default());
        }
        self.format.color = self.format.color.or(inherited.color);
        let resolved = doc.styles.resolve_char(None, &self.format);
        let math = Math {
            nodes: std::mem::take(&mut self.body),
            ..Default::default()
        };
        let body = measured.layout(&math, "", &resolved, false, MAX_GEOMETRY)?;
        self.bounds(body)
    }

    fn bounds(&self, mut layout: MathLayout) -> Result<MathLayout, EngineError> {
        let original = [layout.width, layout.ascent, layout.descent];
        let mut result = [original[0], original[1], original[2], 0.0, 0.0];
        for (index, length) in self.lengths.iter().enumerate() {
            if let Some(length) = length {
                result[index] = length.resolve(index, original);
            }
            if index < 3 {
                result[index] = result[index].max(0.0);
            }
            if !result[index].is_finite() || result[index].abs() > MAX_GEOMETRY {
                return Err(limited("MathML padding exceeds its geometry budget"));
            }
        }
        let [width, ascent, descent, x, y] = result;
        layout.width = width;
        layout.ascent = ascent;
        layout.descent = descent;
        for item in &mut layout.items {
            position(item, x, y, 1.0);
        }
        Ok(layout)
    }
}

impl EditableLayout<'_> {
    pub(super) fn axis(&self) -> f32 {
        let (face, constants) = resolve_math("Cambria Math");
        let axis_ratio = constants.as_ref().map_or(0.25, |table| {
            table.get(C::AxisHeight) / face.upem.max(1.0) as f32
        });
        self.carrier.draw_size().clamp(1.0, 1638.0) * axis_ratio
    }

    pub(super) fn matrix_spacing(&self) -> Result<[f32; 2], EngineError> {
        let [column, row] = *self.matrix_spacing.get_or_init(|| {
            // A matrix with zero-sized cells reveals the native gaps before
            // we place the MathML cells into those logical slots.
            let colors = [Rgb(1, 2, 6), Rgb(1, 2, 7), Rgb(1, 2, 8)];
            let marker = |color| {
                vec![phantom(
                    guide("H", GUIDE_SIZE, Some(color)),
                    true,
                    true,
                    true,
                    true,
                )]
            };
            let math = Math {
                nodes: vec![MNode::Matrix {
                    col_jc: vec![ColJc::Left; 2],
                    rows: vec![
                        vec![marker(colors[0]), marker(colors[1])],
                        vec![marker(colors[2]), vec![MNode::Run(MRun::plain(""))]],
                    ],
                }],
                ..Default::default()
            };
            let measured = layout_equation(
                &math,
                "",
                &self.carrier,
                false,
                MAX_GEOMETRY,
                &Default::default(),
                &mut 0,
            );
            let mut positions = [[f32::NAN; 2]; 3];
            for item in measured.items {
                if let MItem::Glyphs { color, glyphs, .. } = item {
                    if let (Some(index), Some(&(_, x, y))) =
                        (colors.iter().position(|ink| *ink == color), glyphs.first())
                    {
                        positions[index] = [x, y];
                    }
                }
            }
            let [origin, next_column, next_row] = positions;
            [next_column[0] - origin[0], origin[1] - next_row[1]]
        });
        if ![column, row]
            .into_iter()
            .all(|value| value.is_finite() && value > 0.0)
        {
            return Err(unsupported("native DOCX math matrix spacing"));
        }
        Ok([column, row])
    }

    pub(super) fn shift(&self, contents: Arg, shift: f32) -> Result<MNode, EngineError> {
        // A one-row equation array centers its row on the math axis. Zero the
        // operand extents and give the row an independent ascent or descent so
        // its baseline lands at the requested offset.
        let correction = 2.0 * (self.axis() - shift);
        let mut row = vec![
            MNode::Phant {
                show: true,
                zero_wid: false,
                zero_asc: true,
                zero_desc: true,
                e: contents,
            },
            phantom(
                guide("H", 1.0, Some(math_spacing::SHIFT_GUIDE_COLOR)),
                false,
                true,
                true,
                true,
            ),
        ];
        let index = usize::from(correction < 0.0);
        let minimum = self.ratios()[index];
        if correction.abs() >= minimum / 2.0 {
            // The smallest positive guide is one point. A subpoint guide
            // rounds to the nearer of that one-point guide and no guide.
            let mut extents = [None; 2];
            extents[index] = Some(correction.abs().max(minimum));
            self.add_extents(&mut row, extents)?;
        }
        Ok(MNode::EqArr { rows: vec![row] })
    }

    fn ratios(&self) -> [f32; 2] {
        *self
            .guide_ratios
            .get_or_init(|| guide_ratios(&self.carrier))
    }

    pub(super) fn add_extents(
        &self,
        nodes: &mut Arg,
        extents: [Option<f32>; 2],
    ) -> Result<(), EngineError> {
        for (index, extent) in extents.into_iter().enumerate() {
            let Some(extent) = extent.filter(|extent| *extent != 0.0) else {
                continue;
            };
            let ratios = self.ratios();
            let size = (extent / ratios[index] * 2.0).round() * 0.5;
            if !(1.0..=1638.0).contains(&size) {
                return Err(unsupported(
                    "MathML vertical extent outside the DOCX spacing range",
                ));
            }
            let above = index == 0;
            nodes.push(phantom(
                guide(
                    if above { "H" } else { "g" },
                    size,
                    Some(math_spacing::GUIDE_COLOR),
                ),
                false,
                true,
                !above,
                above,
            ));
        }
        Ok(())
    }
}

pub(super) fn grid_marker() -> MNode {
    phantom(
        guide("H", 1.0, Some(math_spacing::GRID_GUIDE_COLOR)),
        false,
        true,
        true,
        true,
    )
}

pub(super) fn grid_gap(width: f32) -> Result<Option<MNode>, EngineError> {
    if width < 0.2 {
        return Ok(None);
    }
    let gap = editable_gap(width.max(0.4))?.ok_or_else(|| invalid("empty MathML matrix gap"))?;
    Ok(Some(MNode::Phant {
        show: false,
        zero_wid: false,
        zero_asc: true,
        zero_desc: true,
        e: vec![gap, grid_marker()],
    }))
}

fn zeroed(length: Option<Length>, dimension: usize) -> Option<bool> {
    match length {
        None => Some(false),
        Some(Length {
            value: 0.0,
            relative,
            ..
        }) => Some(!relative),
        Some(Length {
            value: 1.0,
            basis: Some(basis),
            relative: false,
        }) if basis == dimension => Some(false),
        _ => None,
    }
}

fn editable_gap(width: f32) -> Result<Option<MNode>, EngineError> {
    if width == 0.0 {
        return Ok(None);
    }
    // WordCraft advances a math space by 0.4 em and writes run sizes in half-points.
    // Use one run per distinct size so long gaps retain the same rounding precision.
    let units = (width * 5.0).round() as usize;
    if units < 2 {
        return Err(unsupported(
            "MathML padding is below the DOCX spacing resolution",
        ));
    }
    let count = units.div_ceil(3276);
    let size = (units / count) as f32 / 2.0;
    let larger = units % count;
    let run = |count: usize, size| {
        MNode::Run(MRun {
            size: Some(size),
            ..MRun::plain(" ".repeat(count))
        })
    };
    let mut nodes = vec![run(count - larger, size)];
    if larger > 0 {
        nodes.push(run(larger, size + 0.5));
    }
    if nodes.len() == 1 {
        return Ok(nodes.pop());
    }
    Ok(Some(MNode::Boxed { e: nodes }))
}

pub(in crate::epub) fn position(item: &mut MItem, dx: f32, dy: f32, scale: f32) {
    let x_at = |x: &mut f32| *x = *x * scale + dx;
    let y_at = |y: &mut f32| *y = *y * scale + dy;
    match item {
        MItem::Glyphs { size, glyphs, .. } => {
            *size *= scale;
            for (_, x, y) in glyphs {
                x_at(x);
                y_at(y);
            }
        }
        MItem::Rect { x, y, w, h, .. } => {
            x_at(x);
            y_at(y);
            *w *= scale;
            *h *= scale;
        }
        MItem::Line {
            x0,
            x1,
            y0,
            y1,
            width,
            ..
        } => {
            x_at(x0);
            x_at(x1);
            y_at(y0);
            y_at(y1);
            *width *= scale;
        }
        MItem::ScreenOnly(item) => position(item, dx, dy, scale),
    }
}

fn guide(text: &str, size: f32, color: Option<Rgb>) -> MNode {
    MNode::Run(MRun {
        nor: true,
        font: Some(math_spacing::GUIDE_FONT.into()),
        size: Some(size),
        color,
        ..MRun::plain(text)
    })
}

fn guide_metric(text: &str, resolved: &ResolvedChar) -> MathLayout {
    layout_equation(
        &Math {
            nodes: vec![guide(text, GUIDE_SIZE, None)],
            ..Default::default()
        },
        "",
        resolved,
        false,
        MAX_GEOMETRY,
        &Default::default(),
        &mut 0,
    )
}

fn guide_ratios(resolved: &ResolvedChar) -> [f32; 2] {
    [
        guide_metric("H", resolved).ascent / GUIDE_SIZE,
        guide_metric("g", resolved).descent / GUIDE_SIZE,
    ]
}

fn phantom(node: MNode, show: bool, zero_wid: bool, zero_asc: bool, zero_desc: bool) -> MNode {
    MNode::Phant {
        show,
        zero_wid,
        zero_asc,
        zero_desc,
        e: vec![node],
    }
}

// WordCraft's native phantom can zero any extent independently. Compose
// these guides in the native tree so every parent sees the measured box.
fn placeholder(
    layout: &MathLayout,
    color: Rgb,
    ascent_ratio: f32,
    descent_ratio: f32,
    vertical_limit: &str,
) -> Result<MNode, EngineError> {
    let mut nodes = vec![phantom(
        guide("H", GUIDE_SIZE, Some(color)),
        true,
        true,
        true,
        true,
    )];
    if layout.width > 0.0 {
        let runs = (layout.width / (1638.0 * 0.4)).ceil() as usize;
        if runs > 32 {
            return Err(limited("MathML composed box is too wide"));
        }
        let size = (layout.width / runs as f32 / 0.4).clamp(1.0, 1638.0);
        nodes.push(MNode::Run(MRun {
            size: Some(size),
            ..MRun::plain(" ".repeat(runs))
        }));
    }
    for (dimension, ratio, above, glyph) in [
        (layout.ascent, ascent_ratio, true, "H"),
        (layout.descent, descent_ratio, false, "g"),
    ] {
        if dimension > 0.0 {
            let size = dimension / ratio;
            if !size.is_finite() || size > 1638.0 {
                return Err(limited(vertical_limit));
            }
            nodes.push(phantom(
                guide(glyph, size.max(1.0), None),
                false,
                true,
                !above,
                above,
            ));
        }
    }
    Ok(MNode::Boxed { e: nodes })
}

impl Measurements {
    pub(in crate::epub) fn layout(
        &self,
        math: &Math,
        linear: &str,
        resolved: &ResolvedChar,
        display: bool,
        width: f32,
    ) -> Result<MathLayout, EngineError> {
        if self.boxes.is_empty() {
            return Ok(layout_equation(
                math,
                linear,
                resolved,
                display,
                width,
                &Default::default(),
                &mut 0,
            ));
        }
        let native = self.native(math.clone());
        let mut result = layout_equation(
            &native,
            linear,
            resolved,
            display,
            width,
            &Default::default(),
            &mut 0,
        );
        self.resolve(&mut result)?;
        Ok(result)
    }

    pub(in crate::epub) fn native(&self, mut math: Math) -> Math {
        math.nodes = self.substitute(math.nodes);
        math
    }

    pub(in crate::epub) fn draw_units(&self) -> usize {
        self.draw_units
    }

    pub(in crate::epub) fn resolve(&self, result: &mut MathLayout) -> Result<(), EngineError> {
        let old = std::mem::take(&mut result.items);
        for item in old {
            if let MItem::Glyphs {
                color,
                size,
                glyphs,
                text,
                ..
            } = &item
            {
                if text == "H" {
                    if let Some(&index) = self.markers.get(&[color.0, color.1, color.2]) {
                        let &[(_, x, y)] = glyphs.as_slice() else {
                            return Err(invalid("MathML composed box lost its guide position"));
                        };
                        let scale = *size / GUIDE_SIZE;
                        for mut child in self.boxes[index].layout.items.iter().cloned() {
                            position(&mut child, x, y, scale);
                            result.items.push(child);
                        }
                        continue;
                    }
                }
            }
            result.items.push(item);
        }
        Ok(())
    }

    fn substitute(&self, nodes: Arg) -> Arg {
        replace_boxes(nodes, &|index| {
            self.boxes
                .get(index)
                .map(|measure| measure.placeholder.clone())
        })
    }
}

fn replace_boxes(nodes: Arg, replacement: &dyn Fn(usize) -> Option<MNode>) -> Arg {
    nodes
        .into_iter()
        .map(|node| match node {
            MNode::Run(ref run) => run
                .font
                .as_deref()
                .and_then(|font| font.strip_prefix(BOX_PREFIX))
                .and_then(|index| index.parse::<usize>().ok())
                .and_then(replacement)
                .unwrap_or(node),
            _ => map_args(node, &|arg| replace_boxes(arg, replacement)),
        })
        .collect()
}
