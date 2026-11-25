//! Resolve local TTML style descriptions for timed text and region geometry.
use std::collections::{HashMap, HashSet};

use roxmltree::Node;

use super::{invalid, limited, media::integer, XML};
use crate::EngineError;

#[cfg(feature = "python")]
use super::text_tracks::TextStyle;

pub(super) const TT: &str = "http://www.w3.org/ns/ttml";
pub(super) const STYLING: &str = "http://www.w3.org/ns/ttml#styling";
pub(super) const PARAM: &str = "http://www.w3.org/ns/ttml#parameter";

const MAX_DEPTH: usize = 32;
const MAX_STYLES: usize = 512;
const MAX_STYLE_REFS: usize = 32;
const MAX_STYLE_LABEL: usize = 1024;

pub(super) trait Patch<'n>: Copy + Default {
    fn merge(self, later: Self) -> Self;
    fn inline(self, node: Node<'n, '_>) -> Result<Self, EngineError>;
}

pub(super) fn definitions<'n, P: Patch<'n>>(
    root: Node<'n, '_>,
) -> Result<HashMap<&'n str, P>, EngineError> {
    let mut definitions = HashMap::new();
    let styling = root
        .children()
        .filter(|node| node.has_tag_name((TT, "head")))
        .flat_map(|head| {
            head.children()
                .filter(|node| node.has_tag_name((TT, "styling")))
        });
    for style in styling
        .flat_map(|node| node.descendants())
        .filter(|node| node.has_tag_name((TT, "style")))
    {
        let Some(name) = style.attribute((XML, "id")) else {
            continue;
        };
        if definitions.len() == MAX_STYLES || name.len() > MAX_STYLE_LABEL {
            return Err(limited("TTML style definitions exceed their budget"));
        }
        if definitions.insert(name, style).is_some() {
            return Err(invalid("duplicate TTML style ID"));
        }
    }
    let mut resolved = HashMap::new();
    let mut active = HashSet::new();
    for &name in definitions.keys() {
        define_style(name, &definitions, &mut resolved, &mut active)?;
    }
    Ok(resolved)
}

fn define_style<'n, P: Patch<'n>>(
    name: &'n str,
    definitions: &HashMap<&'n str, Node<'n, '_>>,
    resolved: &mut HashMap<&'n str, P>,
    active: &mut HashSet<&'n str>,
) -> Result<P, EngineError> {
    if let Some(&style) = resolved.get(name) {
        return Ok(style);
    }
    if active.len() == MAX_DEPTH {
        return Err(limited("TTML style reference nesting exceeds 32"));
    }
    if !active.insert(name) {
        return Err(invalid("cyclic TTML style reference"));
    }
    let node = *definitions
        .get(name)
        .ok_or_else(|| invalid("missing TTML style reference"))?;
    let patch = local(node, |parent| {
        define_style(parent, definitions, resolved, active)
    })?;
    active.remove(name);
    resolved.insert(name, patch);
    Ok(patch)
}

pub(super) fn references<'n, P: Patch<'n>>(
    node: Node<'n, '_>,
    mut lookup: impl FnMut(&'n str) -> Result<P, EngineError>,
) -> Result<P, EngineError> {
    let mut patch = P::default();
    if let Some(references) = node.attribute("style") {
        for (index, name) in references.split_ascii_whitespace().enumerate() {
            if index == MAX_STYLE_REFS {
                return Err(limited("TTML style reference list exceeds 32 entries"));
            }
            patch = patch.merge(lookup(name)?);
        }
    }
    Ok(patch)
}

pub(super) fn local<'n, P: Patch<'n>>(
    node: Node<'n, '_>,
    lookup: impl FnMut(&'n str) -> Result<P, EngineError>,
) -> Result<P, EngineError> {
    references(node, lookup)?.inline(node)
}

pub(super) fn color(value: &str) -> Result<[u8; 4], EngineError> {
    let malformed = || invalid("invalid TTML color");
    if let Some((function, values)) = value.split_once('(') {
        let mut values = values.strip_suffix(')').ok_or_else(malformed)?.split(',');
        let count = match function {
            "rgb" => 3,
            "rgba" => 4,
            _ => return Err(malformed()),
        };
        let mut rgba = [255; 4];
        for channel in &mut rgba[..count] {
            *channel = values
                .next()
                .map(str::trim_ascii)
                .and_then(integer)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(malformed)?;
        }
        if values.next().is_some() {
            return Err(malformed());
        }
        return Ok(rgba);
    }
    if let Some(hex) = value.strip_prefix('#') {
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|digit| digit.is_ascii_hexdigit()) {
            return Err(malformed());
        }
    } else if !value.bytes().all(|letter| letter.is_ascii_alphabetic()) {
        return Err(malformed());
    }
    let ([r, g, b], alpha) = crate::epub::css::color(value).ok_or_else(malformed)?;
    Ok([r, g, b, (alpha * 255.0).round() as u8])
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[cfg(feature = "python")]
impl FontStyle {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Italic => "italic",
            Self::Oblique => "oblique",
        }
    }
}

// A declaration is separate from inherited style so explicit resets survive
// merging style references and inline properties before application.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct StylePatch {
    bold: Option<bool>,
    font_style: Option<FontStyle>,
    decoration: Option<[Option<bool>; 3]>,
    color: Option<[u8; 4]>,
}

impl StylePatch {
    pub(super) fn merge(self, later: Self) -> Self {
        Self {
            bold: later.bold.or(self.bold),
            font_style: later.font_style.or(self.font_style),
            decoration: later.decoration.or(self.decoration),
            color: later.color.or(self.color),
        }
    }

    // The three text-decoration flags inherit independently across elements.
    #[cfg(feature = "python")]
    pub(super) fn inherit(self, local: Self) -> Self {
        let mut style = self.merge(local);
        if let (Some(parent), Some(child)) = (self.decoration, local.decoration) {
            style.decoration = Some(std::array::from_fn(|index| child[index].or(parent[index])));
        }
        style
    }

    #[cfg(feature = "python")]
    pub(super) fn conflicts(self, other: Self) -> bool {
        (self.bold.is_some() && other.bold.is_some())
            || (self.font_style.is_some() && other.font_style.is_some())
            || (self.decoration.is_some() && other.decoration.is_some())
            || (self.color.is_some() && other.color.is_some())
    }

    pub(super) fn inline(self, node: Node<'_, '_>) -> Result<Self, EngineError> {
        let opt = |name| node.attribute((STYLING, name));
        let later = Self {
            bold: opt("fontWeight")
                .map(|value| match value {
                    "normal" => Ok(false),
                    "bold" => Ok(true),
                    _ => Err(invalid("invalid TTML fontWeight")),
                })
                .transpose()?,
            font_style: opt("fontStyle")
                .map(|value| match value {
                    "normal" => Ok(FontStyle::Normal),
                    "italic" => Ok(FontStyle::Italic),
                    "oblique" => Ok(FontStyle::Oblique),
                    _ => Err(invalid("invalid TTML fontStyle")),
                })
                .transpose()?,
            decoration: opt("textDecoration").map(decoration).transpose()?,
            color: opt("color").map(color).transpose()?,
        };
        Ok(self.merge(later))
    }

    #[cfg(feature = "python")]
    pub(super) fn apply(self, style: &mut TextStyle) {
        style.bold = self.bold.unwrap_or(style.bold);
        style.font_style = self.font_style.unwrap_or(style.font_style);
        style.color = self.color.or(style.color);
        if let Some([underline, line_through, overline]) = self.decoration {
            style.underline = underline.unwrap_or(style.underline);
            style.line_through = line_through.unwrap_or(style.line_through);
            style.overline = overline.unwrap_or(style.overline);
        }
    }
}

impl<'n> Patch<'n> for StylePatch {
    fn merge(self, later: Self) -> Self {
        self.merge(later)
    }

    fn inline(self, node: Node<'n, '_>) -> Result<Self, EngineError> {
        self.inline(node)
    }
}

fn decoration(value: &str) -> Result<[Option<bool>; 3], EngineError> {
    if value == "none" {
        return Ok([Some(false); 3]);
    }
    let mut flags = [None; 3];
    let malformed = || invalid("invalid TTML textDecoration");
    for word in value.split_ascii_whitespace() {
        let (slot, enabled) = match word {
            "underline" => (0, true),
            "noUnderline" => (0, false),
            "lineThrough" => (1, true),
            "noLineThrough" => (1, false),
            "overline" => (2, true),
            "noOverline" => (2, false),
            _ => return Err(malformed()),
        };
        if flags[slot].replace(enabled).is_some() {
            return Err(malformed());
        }
    }
    if flags == [None; 3] {
        return Err(malformed());
    }
    Ok(flags)
}
