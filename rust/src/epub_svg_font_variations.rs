//! Retain the authored CSS font request through the native SVG resolver.
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::Arc;

use resvg::usvg::FontFamily;
use skrifa::MetadataProvider;

use crate::epub::{
    css::{self, AxisValue},
    fonts::{self, slope},
};

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub(in crate::epub::svg) struct CssFont {
    pub family: Arc<str>,
    weight: u32,
    width: u32,
    slope: slope::Request,
    optical_auto: bool,
    axes: Arc<[([u8; 4], u32)]>,
}

impl Default for CssFont {
    fn default() -> Self {
        Self {
            family: "serif".into(),
            weight: 400.0f32.to_bits(),
            width: 100.0f32.to_bits(),
            slope: slope::Request::Normal,
            optical_auto: true,
            axes: Arc::default(),
        }
    }
}

impl CssFont {
    pub(in crate::epub::svg) fn apply(&mut self, property: &str, value: &str) {
        let keyword = value.to_ascii_lowercase();
        match property {
            "font-family" => match keyword.as_str() {
                "initial" => self.family = "serif".into(),
                "inherit" | "unset" | "revert" | "revert-layer" => {}
                _ => self.family = value.into(),
            },
            "font-weight" => {
                let value = css::relative_font_weight(value, f32::from_bits(self.weight));
                if let Some(value) = value {
                    self.weight = value.to_bits();
                }
            }
            "font-stretch" => {
                if keyword == "initial" {
                    self.width = 100.0f32.to_bits();
                } else if let Some(width) = css::font_width(value) {
                    self.width = width.to_bits();
                }
            }
            "font-style" => {
                use parley::FontStyle;
                if keyword == "initial" {
                    self.slope = slope::Request::Normal;
                } else if let Some(style) = css::font_style(value) {
                    self.slope = match style {
                        FontStyle::Normal => slope::Request::Normal,
                        FontStyle::Italic => slope::Request::Italic,
                        FontStyle::Oblique(angle) => {
                            slope::Request::new(None, Some(angle.unwrap_or(14.0)))
                        }
                    };
                }
            }
            "font-variation-settings" => {
                if let Some(AxisValue::Settings(axes)) = css::parse_axes(value) {
                    self.axes = axes
                        .iter()
                        .map(|&(tag, value)| (tag, value.to_bits()))
                        .collect();
                }
            }
            "font-optical-sizing" => {
                if let Some(auto) = css::optical_sizing(value) {
                    self.optical_auto = auto;
                }
            }
            _ => {}
        }
    }

    pub(super) fn families(&self) -> Vec<FontFamily> {
        // Every non-generic name was replaced with a private, comma-free alias.
        self.family
            .split(',')
            .map(|name| match name {
                "serif" => FontFamily::Serif,
                "sans-serif" => FontFamily::SansSerif,
                "cursive" => FontFamily::Cursive,
                "fantasy" => FontFamily::Fantasy,
                "monospace" => FontFamily::Monospace,
                _ => FontFamily::Named(name.to_owned()),
            })
            .collect()
    }

    pub(super) fn rank(&self, face: &Face) -> (Distance, slope::Rank, Distance) {
        let width = f32::from_bits(self.width);
        let [low, high] = face.width.unwrap_or([100.0; 2]);
        let (side, distance) = fonts::width_rank(width, width.clamp(low, high));
        let slope = self.slope.rank(face.style, face.oblique);
        let weight = f32::from_bits(self.weight);
        let [low, high] = face.weight.unwrap_or([400.0; 2]);
        (
            Distance(u8::from(side), distance),
            slope,
            weight_distance(weight, weight.clamp(low, high)),
        )
    }

    pub(super) fn settings(&self, face: Option<&Face>) -> Vec<([u8; 4], u32)> {
        let mut values = BTreeMap::new();
        if let Some(face) = face {
            if let Some([low, high]) = face.weight {
                values.insert(
                    *b"wght",
                    f32::from_bits(self.weight).clamp(low, high).to_bits(),
                );
            }
            if let Some([low, high]) = face.width {
                values.insert(
                    *b"wdth",
                    f32::from_bits(self.width).clamp(low, high).to_bits(),
                );
            }
            if let Some(range) = face.oblique {
                values.insert(*b"slnt", (-self.slope.angle(range)).to_bits());
            }
            values.extend(face.axes.iter().map(|&(tag, value)| (tag, value.to_bits())));
            values.retain(|tag, _| face.supported.contains(tag));
            if self.optical_auto {
                // usvg supplies optical size from the final positioned text span.
                values.remove(b"opsz");
            }
        }
        values.extend(self.axes.iter().copied());
        values.into_iter().collect()
    }

    pub(super) fn author_settings(&self) -> &[([u8; 4], u32)] {
        &self.axes
    }
}

pub(super) struct Face {
    weight: Option<[f32; 2]>,
    width: Option<[f32; 2]>,
    style: parley::FontStyle,
    oblique: Option<[f32; 2]>,
    axes: std::sync::Arc<[css::Axis]>,
    supported: Vec<[u8; 4]>,
}

pub(super) fn has_descriptors(face: &fonts::Face) -> bool {
    face.weight.is_some() || face.width.is_some() || face.oblique.is_some() || !face.axes.is_empty()
}

impl Face {
    pub(super) fn new(face: &fonts::Face, font: skrifa::FontRef<'_>) -> Self {
        Self {
            weight: face.weight,
            width: face.width,
            style: face.style,
            oblique: face.oblique,
            axes: face.axes.clone(),
            supported: font
                .axes()
                .iter()
                .map(|axis| axis.tag().to_be_bytes())
                .collect(),
        }
    }
}

pub(super) fn rank_cmp(
    a: &(Distance, slope::Rank, Distance),
    b: &(Distance, slope::Rank, Distance),
) -> Ordering {
    a.0.cmp(b.0)
        .then_with(|| a.1.cmp(b.1))
        .then_with(|| a.2.cmp(b.2))
}

#[derive(Clone, Copy)]
pub(super) struct Distance(u8, f32);

impl Distance {
    fn cmp(self, other: Self) -> Ordering {
        self.0
            .cmp(&other.0)
            .then_with(|| self.1.total_cmp(&other.1))
    }
}

fn weight_distance(requested: f32, available: f32) -> Distance {
    if (400.0..=500.0).contains(&requested) {
        if (requested..=500.0).contains(&available) {
            Distance(0, available - requested)
        } else if available < requested {
            Distance(1, requested - available)
        } else {
            Distance(2, available - 500.0)
        }
    } else {
        let preferred = if requested < 400.0 {
            available <= requested
        } else {
            available >= requested
        };
        Distance(u8::from(!preferred), (available - requested).abs())
    }
}
