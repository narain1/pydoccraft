//! Adapt the native stacked fraction for authored MathML rules and alignment.
use roxmltree::Node;
use wordcraft_doc::{
    math::{Arg, FracKind, MNode, Math},
    CharProps, Document,
};
use wordcraft_layout::math::{MItem, MathLayout};

use super::{
    compose::{self, Measurements, MAX_GEOMETRY},
    css, invalid, limited, Presentation,
};
use crate::EngineError;

#[derive(Clone, Copy)]
enum Rule {
    Default,
    Relative(f32),
    Points(f32),
}

impl Rule {
    fn parse(value: &str, em: f32, viewport: [f32; 2]) -> Self {
        match value.trim() {
            "thin" => Self::Relative(0.5),
            "thick" => Self::Relative(2.0),
            "none" => Self::Relative(0.0),
            value => {
                if let Some(percent) = value.strip_suffix('%') {
                    return percent
                        .trim()
                        .parse::<f32>()
                        .map_or(Self::Default, |percent| Self::Relative(percent / 100.0));
                }
                if let Ok(factor) = value.parse::<f32>() {
                    return Self::Relative(factor);
                }
                let number = value.trim_end_matches(|c: char| c.is_ascii_alphabetic());
                let units = &value[number.len()..];
                let scale = if units == "ex" {
                    Some(em / 2.0)
                } else {
                    css::points(&format!("1{units}"), em, viewport)
                };
                let length = number
                    .parse::<f32>()
                    .ok()
                    .zip(scale)
                    .map(|(number, scale)| number * scale);
                length.map_or(Self::Default, Self::Points)
            }
        }
    }

    fn native(self) -> Option<FracKind> {
        match self {
            Self::Default | Self::Relative(1.0) => Some(FracKind::Bar),
            Self::Points(n) | Self::Relative(n) if n <= 0.0 => Some(FracKind::NoBar),
            _ => None,
        }
    }

    fn thickness(self, ordinary: f32) -> Result<f32, EngineError> {
        let thickness = match self {
            Self::Default => ordinary,
            Self::Relative(factor) => ordinary * factor,
            Self::Points(points) => points,
        };
        if !(0.0..=MAX_GEOMETRY).contains(&thickness) {
            return Err(limited("MathML fraction rule exceeds its geometry budget"));
        }
        Ok(thickness)
    }
}

#[derive(Clone, Copy, Default)]
enum Align {
    Left,
    #[default]
    Center,
    Right,
}

impl Align {
    fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("left") => Self::Left,
            Some("right") => Self::Right,
            _ => Self::Center,
        }
    }

    fn offset(self, centered: f32, edge: f32) -> f32 {
        match self {
            Self::Left => edge - centered,
            Self::Center => 0.0,
            Self::Right => centered - edge,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Style {
    rule: Rule,
    align: [Align; 2],
}

impl Style {
    pub(super) fn read(node: Node<'_, '_>, em: f32, viewport: [f32; 2]) -> Self {
        Self {
            rule: node
                .attribute("linethickness")
                .map_or(Rule::Default, |value| Rule::parse(value, em, viewport)),
            align: [
                Align::parse(node.attribute("numalign")),
                Align::parse(node.attribute("denomalign")),
            ],
        }
    }

    pub(super) fn native(self) -> Option<FracKind> {
        if self
            .align
            .iter()
            .all(|align| matches!(align, Align::Center))
        {
            self.rule.native()
        } else {
            None
        }
    }
}

pub(super) struct Fraction {
    style: Style,
    display: bool,
    format: CharProps,
    num: Arg,
    den: Arg,
}

impl Fraction {
    pub(super) fn new(style: Style, presentation: Presentation<'_>, num: Arg, den: Arg) -> Self {
        Self {
            style,
            display: presentation.display_style,
            format: compose::format(presentation),
            num,
            den,
        }
    }

    pub(super) fn layout(
        mut self,
        doc: &Document,
        inherited: &CharProps,
        measured: &Measurements,
    ) -> Result<MathLayout, EngineError> {
        self.format.color = self.format.color.or(inherited.color);
        let resolved = doc.styles.resolve_char(None, &self.format);
        let kind = self.style.rule.native().unwrap_or(FracKind::Bar);
        // Native invisible phantoms preserve the opposite argument's geometry.
        // Lay each visible operand out in its native fraction style before moving its items.
        let numerator = MNode::Frac {
            kind,
            num: self.num.clone(),
            den: hidden(&self.den),
        };
        let denominator = MNode::Frac {
            kind,
            num: hidden(&self.num),
            den: self.den,
        };
        let draw = |node| {
            measured.layout(
                &Math {
                    nodes: vec![node],
                    ..Default::default()
                },
                "",
                &resolved,
                self.display,
                MAX_GEOMETRY,
            )
        };
        let mut top = draw(numerator)?;
        let mut bottom = draw(denominator)?;
        let left = [operand_x(&top, 0), operand_x(&bottom, 1)];
        let edge = left[0].min(left[1]);

        let mut rule = None;
        let mut shift = 0.0;
        if kind == FracKind::Bar {
            let Some(MItem::Rect { x, y, w, h, color }) = top.items.pop() else {
                return Err(invalid("missing native MathML fraction rule"));
            };
            if !matches!(bottom.items.pop(), Some(MItem::Rect { .. })) {
                return Err(invalid("missing native MathML fraction rule"));
            }
            let thickness = self.style.rule.thickness(h)?;
            let delta = (thickness - h) / 2.0;
            // Keep the math axis fixed and preserve native clearance when the bar grows.
            shift = delta.max(0.0);
            rule = Some(MItem::Rect {
                x,
                y: y - delta,
                w,
                h: thickness,
                color,
            });
        }
        for (index, (layout, dy)) in [(&mut top, shift), (&mut bottom, -shift)]
            .into_iter()
            .enumerate()
        {
            let dx = self.style.align[index].offset(left[index], edge);
            for item in &mut layout.items {
                compose::position(item, dx, dy, 1.0);
            }
        }
        top.ascent += shift;
        top.descent += shift;
        top.items.extend(bottom.items);
        top.items.extend(rule);
        Ok(top)
    }
}

fn hidden(nodes: &Arg) -> Arg {
    if nodes.is_empty() {
        return Vec::new();
    }
    vec![MNode::Phant {
        show: false,
        zero_wid: false,
        zero_asc: false,
        zero_desc: false,
        e: nodes.clone(),
    }]
}

fn operand_x(layout: &MathLayout, index: usize) -> f32 {
    layout
        .slots
        .iter()
        .filter(|slot| slot.path == [(0, index)])
        .map(|slot| slot.x)
        .reduce(f32::min)
        .unwrap_or(layout.width / 2.0)
}
