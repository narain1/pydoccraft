//! Supply measured MathML boxes to the pinned equation shaper.
use std::collections::{HashMap, HashSet};

use roxmltree::Node;
use wordcraft_doc::{
    math::{Arg, MNode, MRun, Math},
    math_linear::map_args,
    props::{CharProps, Rgb},
    resolve::ResolvedChar,
    Document,
};
use wordcraft_layout::math::{layout_equation, MItem, MathLayout};

use super::{css, invalid, limited, table, unsupported, zero_length, Presentation, MATHML};
use crate::EngineError;

const BOX_PREFIX: &str = "\0epub-math-box-";
const GUIDE_FONT: &str = "Liberation Serif";
const GUIDE_SIZE: f32 = 16.0;
const MAX_GEOMETRY: f32 = 20_000.0;
const LIMIT_BOXES: usize = 512;
const MAX_SAVED_DRAW_UNITS: usize = 250_000;

#[derive(Default)]
pub(in crate::epub) struct Extras {
    nodes: Vec<Extra>,
    occupied_colors: HashSet<[u8; 3]>,
}

pub(super) enum Extra {
    Table(table::Table, Box<CharProps>),
    Padded(Box<Padding>),
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
        "mpadded" => ["width", "height", "depth", "lspace", "voffset"]
            .iter()
            .any(|&key| node.attribute(key).is_some_and(|value| !zero_length(value))),
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
        let top = guide_metric("H", &base).ascent / GUIDE_SIZE;
        let bottom = guide_metric("g", &base).descent / GUIDE_SIZE;
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
            let mut layout = match extra {
                Extra::Table(table, local) => {
                    let style = CharProps {
                        color: local.color.or(format.color),
                        ..*local
                    };
                    table.layout(doc, &style, &measurements)?.into_math()
                }
                Extra::Padded(padding) => padding.layout(doc, format, &measurements)?,
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
            let placeholder = placeholder(&layout, color, top, bottom)?;
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
                let base = length.basis.map_or(1.0, |i| original[i]);
                let delta = length.value * base;
                result[index] = if length.relative {
                    result[index] + delta
                } else {
                    delta
                };
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
        font: Some(GUIDE_FONT.into()),
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
                return Err(limited(
                    "MathML composed box exceeds the native vertical extent",
                ));
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
        nodes
            .into_iter()
            .map(|node| match node {
                MNode::Run(ref run) => {
                    if let Some(index) = run
                        .font
                        .as_deref()
                        .and_then(|font| font.strip_prefix(BOX_PREFIX))
                        .and_then(|index| index.parse::<usize>().ok())
                    {
                        if let Some(measure) = self.boxes.get(index) {
                            return measure.placeholder.clone();
                        }
                    }
                    node
                }
                _ => map_args(node, &|arg| self.substitute(arg)),
            })
            .collect()
    }
}
