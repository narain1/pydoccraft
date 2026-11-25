//! Horizontal flow-relative box properties, resolved before native layout.
use std::collections::HashMap;

use roxmltree::Node;
use taffy::CalcHandle;

use super::{calc, html_direction, points, Properties};
use crate::EngineError;

#[derive(Clone, Copy, Default)]
pub(in crate::epub) struct Flow {
    pub rtl: Option<bool>,
    vertical: bool,
}

impl Flow {
    pub(super) fn resolve(
        node: Node<'_, '_>,
        direction: Option<&str>,
        writing_mode: Option<&str>,
        mut inherited: Self,
    ) -> Self {
        match direction.filter(|value| !keyword(value, "revert")) {
            Some(direction) if keyword(direction, "rtl") => inherited.rtl = Some(true),
            Some(direction) if keyword(direction, "ltr") || keyword(direction, "initial") => {
                inherited.rtl = Some(false);
            }
            Some(direction) if keyword(direction, "inherit") || keyword(direction, "unset") => {
                inherited.rtl = Some(inherited.rtl.unwrap_or(false));
            }
            None => {
                if let Some(hint) = html_direction::hint(node) {
                    inherited.rtl = Some(hint.rtl(node));
                }
            }
            _ => {}
        }
        if let Some(mode) = writing_mode {
            if ["vertical-rl", "vertical-lr", "sideways-rl", "sideways-lr"]
                .into_iter()
                .any(|keyword_value| keyword(mode, keyword_value))
            {
                inherited.vertical = true;
            } else if keyword(mode, "horizontal-tb") || keyword(mode, "initial") {
                inherited.vertical = false;
            }
        }
        inherited
    }

    pub(super) fn is_vertical(self) -> bool {
        self.vertical
    }
}

fn keyword(value: &str, name: &str) -> bool {
    value.eq_ignore_ascii_case(name)
}

// Box longhand, physical LTR and RTL aliases, and its initial value.
pub(super) struct Pair(
    pub &'static str,
    &'static str,
    &'static str,
    pub &'static str,
);

impl Pair {
    pub(super) fn is_length(&self) -> bool {
        !self.0.starts_with("overflow-")
            && !self.0.ends_with("-color")
            && !self.0.ends_with("-style")
    }

    fn allows_negative(&self) -> bool {
        self.0.starts_with("inset-") || self.0.starts_with("margin-")
    }

    pub(super) fn accepts_percentage(&self) -> bool {
        !self.0.starts_with("border-")
    }

    pub(super) fn physical(&self, flow: Flow) -> &'static str {
        if flow.rtl == Some(true) {
            self.2
        } else {
            self.1
        }
    }
}

static PAIRS: &[Pair] = &[
    Pair("block-size", "height", "height", "auto"),
    Pair(
        "border-block-end-color",
        "border-bottom-color",
        "border-bottom-color",
        "currentColor",
    ),
    Pair(
        "border-block-end-style",
        "border-bottom-style",
        "border-bottom-style",
        "none",
    ),
    Pair(
        "border-block-end-width",
        "border-bottom-width",
        "border-bottom-width",
        "medium",
    ),
    Pair(
        "border-block-start-color",
        "border-top-color",
        "border-top-color",
        "currentColor",
    ),
    Pair(
        "border-block-start-style",
        "border-top-style",
        "border-top-style",
        "none",
    ),
    Pair(
        "border-block-start-width",
        "border-top-width",
        "border-top-width",
        "medium",
    ),
    Pair(
        "border-inline-end-color",
        "border-right-color",
        "border-left-color",
        "currentColor",
    ),
    Pair(
        "border-inline-end-style",
        "border-right-style",
        "border-left-style",
        "none",
    ),
    Pair(
        "border-inline-end-width",
        "border-right-width",
        "border-left-width",
        "medium",
    ),
    Pair(
        "border-inline-start-color",
        "border-left-color",
        "border-right-color",
        "currentColor",
    ),
    Pair(
        "border-inline-start-style",
        "border-left-style",
        "border-right-style",
        "none",
    ),
    Pair(
        "border-inline-start-width",
        "border-left-width",
        "border-right-width",
        "medium",
    ),
    Pair("column-gap", "column-gap", "column-gap", "normal"),
    Pair("inline-size", "width", "width", "auto"),
    Pair("inset-block-end", "bottom", "bottom", "auto"),
    Pair("inset-block-start", "top", "top", "auto"),
    Pair("inset-inline-end", "right", "left", "auto"),
    Pair("inset-inline-start", "left", "right", "auto"),
    Pair("margin-block-end", "margin-bottom", "margin-bottom", "0"),
    Pair("margin-block-start", "margin-top", "margin-top", "0"),
    Pair("margin-inline-end", "margin-right", "margin-left", "0"),
    Pair("margin-inline-start", "margin-left", "margin-right", "0"),
    Pair("max-block-size", "max-height", "max-height", "none"),
    Pair("max-inline-size", "max-width", "max-width", "none"),
    Pair("min-block-size", "min-height", "min-height", "auto"),
    Pair("min-inline-size", "min-width", "min-width", "auto"),
    Pair("overflow-block", "overflow-y", "overflow-y", "visible"),
    Pair("overflow-inline", "overflow-x", "overflow-x", "visible"),
    Pair("padding-block-end", "padding-bottom", "padding-bottom", "0"),
    Pair("padding-block-start", "padding-top", "padding-top", "0"),
    Pair("padding-inline-end", "padding-right", "padding-left", "0"),
    Pair("padding-inline-start", "padding-left", "padding-right", "0"),
    Pair("row-gap", "row-gap", "row-gap", "normal"),
];

pub(super) fn pair(name: &str) -> Option<&'static Pair> {
    PAIRS
        .binary_search_by_key(&name, |pair| pair.0)
        .ok()
        .map(|index| &PAIRS[index])
        .filter(|pair| pair.0 != pair.1)
}

pub(super) fn physical_pair(name: &str, flow: Flow) -> Option<&'static Pair> {
    PAIRS.iter().find(|pair| pair.physical(flow) == name)
}

pub(in crate::epub) type Calculations = HashMap<&'static str, CalcHandle>;

pub(in crate::epub) fn resolve_calculated_geometry(
    values: &mut Properties,
    flow: Flow,
    font: f32,
    viewport: [f32; 2],
    calculations: &mut calc::CalculationTable,
    mut handles: Calculations,
) -> Result<Calculations, EngineError> {
    for (name, value) in values {
        let limit = if matches!(name.as_str(), "width" | "height") {
            fit_content_argument(value)
        } else {
            None
        };
        if limit.is_none() && !calc::is_math(value) {
            continue;
        }
        if let Some(pair) = physical_pair(name, flow).filter(|pair| pair.is_length()) {
            let source = limit.unwrap_or(value);
            let resolved = if limit.is_some() && source.parse::<f32>().is_ok_and(|n| n == 0.0) {
                calc::Resolved::Points(0.0)
            } else {
                calc::resolve(source, font, viewport, pair.allows_negative())?
            };
            let fit = limit.is_some();
            let normalized = match resolved {
                calc::Resolved::Points(points) => format!("{points}pt"),
                calc::Resolved::Percent(percent) => format!("{percent}%"),
                calc::Resolved::Deferred(length) => {
                    handles.insert(pair.physical(flow), calculations.register(length)?);
                    // A percentage-dependent value belongs to the typed handle
                    // table, including when inherited by a different box.
                    if fit { "fit-content" } else { "auto" }.to_owned()
                }
            };
            *value = if fit && normalized != "fit-content" {
                format!("fit-content({normalized})")
            } else {
                normalized
            };
        }
    }
    Ok(handles)
}

pub(super) fn shorthand(name: &str) -> Option<[&'static str; 2]> {
    Some(match name {
        "margin-inline" => ["margin-inline-start", "margin-inline-end"],
        "margin-block" => ["margin-block-start", "margin-block-end"],
        "padding-inline" => ["padding-inline-start", "padding-inline-end"],
        "padding-block" => ["padding-block-start", "padding-block-end"],
        "inset-inline" => ["inset-inline-start", "inset-inline-end"],
        "inset-block" => ["inset-block-start", "inset-block-end"],
        "overflow" => ["overflow-x", "overflow-y"],
        "gap" => ["row-gap", "column-gap"],
        "border-inline-width" => ["border-inline-start-width", "border-inline-end-width"],
        "border-block-width" => ["border-block-start-width", "border-block-end-width"],
        "border-inline-color" => ["border-inline-start-color", "border-inline-end-color"],
        "border-block-color" => ["border-block-start-color", "border-block-end-color"],
        "border-inline-style" => ["border-inline-start-style", "border-inline-end-style"],
        "border-block-style" => ["border-block-start-style", "border-block-end-style"],
        _ => return None,
    })
}

pub(super) fn border_sides(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "border" => &["top", "right", "bottom", "left"],
        "border-inline" => &["inline-start", "inline-end"],
        "border-block" => &["block-start", "block-end"],
        "border-inline-start" => &["inline-start"],
        "border-inline-end" => &["inline-end"],
        "border-block-start" => &["block-start"],
        "border-block-end" => &["block-end"],
        "border-top" => &["top"],
        "border-right" => &["right"],
        "border-bottom" => &["bottom"],
        "border-left" => &["left"],
        _ => return None,
    })
}

pub(in crate::epub) struct Inherited {
    pub(super) flow: Flow,
    geometry: Properties,
    calculations: Calculations,
    margin: f32,
}

impl Inherited {
    pub(in crate::epub) fn new(
        flow: Flow,
        values: &Properties,
        font: f32,
        viewport: [f32; 2],
        margin: f32,
        calculations: Calculations,
    ) -> Self {
        let geometry = PAIRS
            .iter()
            .filter_map(|pair| {
                let physical = pair.physical(flow);
                let value = values.get(physical)?;
                let computed = points(value, font, viewport)
                    .map_or_else(|| value.clone(), |length| format!("{length}pt"));
                Some((physical.to_owned(), computed))
            })
            .collect();
        Self {
            flow,
            geometry,
            calculations,
            margin,
        }
    }

    pub(super) fn calculation(&self, physical: &str) -> Option<CalcHandle> {
        self.calculations.get(physical).copied()
    }

    pub(super) fn value(&self, physical: &str, initial: &str) -> String {
        self.geometry.get(physical).cloned().unwrap_or_else(|| {
            if matches!(physical, "margin-top" | "margin-bottom") {
                format!("{}pt", self.margin)
            } else {
                initial.to_owned()
            }
        })
    }
}

#[derive(PartialEq)]
pub(super) enum ValueKind {
    Invalid,
    Supported,
    Unsupported,
}

pub(in crate::epub) fn fit_content_argument(value: &str) -> Option<&str> {
    let (function, argument) = value.split_once('(')?;
    keyword(function, "fit-content").then_some(argument.strip_suffix(')')?.trim())
}

pub(super) fn intrinsic_size_kind(name: &str, value: &str) -> Option<ValueKind> {
    let preferred = matches!(name, "width" | "height" | "inline-size" | "block-size");
    let size = preferred
        || matches!(
            name,
            "min-width"
                | "min-height"
                | "max-width"
                | "max-height"
                | "min-inline-size"
                | "min-block-size"
                | "max-inline-size"
                | "max-block-size"
        );
    if !size {
        return None;
    }
    let kind = if ["min-content", "max-content", "fit-content", "stretch"]
        .iter()
        .any(|name| keyword(value, name))
    {
        ValueKind::Supported
    } else if keyword(value, "contain") {
        ValueKind::Unsupported
    } else if let Some(argument) = fit_content_argument(value) {
        let argument = argument.to_ascii_lowercase();
        if calc::is_math(&argument) {
            if calc::valid(&argument, true) {
                ValueKind::Supported
            } else {
                ValueKind::Invalid
            }
        } else {
            length_value_kind(&argument, false, false)
        }
    } else {
        return None;
    };
    Some(match kind {
        ValueKind::Supported if !preferred => ValueKind::Unsupported,
        _ => kind,
    })
}

pub(super) fn value_kind(name: &str, value: &str) -> ValueKind {
    use ValueKind::{Invalid, Supported};
    let value = value.to_ascii_lowercase();
    let value = value.as_str();
    let classified = |valid| if valid { Supported } else { Invalid };
    if value == "normal" && matches!(name, "row-gap" | "column-gap") {
        return Supported;
    }
    if name.starts_with("overflow-") {
        return classified(matches!(
            value,
            "visible" | "hidden" | "clip" | "scroll" | "auto"
        ));
    }
    if name.ends_with("-color") {
        return classified(value == "currentcolor" || super::color(value).is_some());
    }
    if name.ends_with("-style") {
        return classified(
            super::simple_border_style(value) || super::additional_border_style(value),
        );
    }
    let margin = name.starts_with("margin-");
    let inset = name.starts_with("inset-");
    let border = name.ends_with("-width");
    let size = name.ends_with("-size");
    if value == "auto" && (margin || inset || size && !name.starts_with("max-"))
        || value == "none" && name.starts_with("max-")
        || border && matches!(value, "thin" | "medium" | "thick")
    {
        return Supported;
    }
    if calc::is_math(value) {
        return classified(calc::valid(value, !border));
    }
    intrinsic_size_kind(name, value)
        .unwrap_or_else(|| length_value_kind(value, border, margin || inset))
}

fn length_value_kind(value: &str, border: bool, negative: bool) -> ValueKind {
    use ValueKind::{Invalid, Supported, Unsupported};
    if let Some(number) = value.strip_suffix('%') {
        return if !border && allowed_length(number, negative) {
            Supported
        } else {
            Invalid
        };
    }
    if let Ok(number) = value.parse::<f32>() {
        return if number == 0.0 { Supported } else { Invalid };
    }
    for unit in [
        "rem", "em", "px", "pt", "pc", "in", "cm", "mm", "vw", "vh", "ch", "ex", "cap", "ic", "lh",
        "rlh", "vmin", "vmax", "vi", "vb", "q",
    ] {
        if let Some(number) = value
            .strip_suffix(unit)
            .filter(|number| number.parse::<f32>().is_ok())
        {
            if !allowed_length(number, negative) {
                return Invalid;
            }
            return if points(value, 12.0, [1.0; 2]).is_some() {
                Supported
            } else {
                Unsupported
            };
        }
    }
    Invalid
}

fn allowed_length(number: &str, negative: bool) -> bool {
    number
        .parse::<f32>()
        .is_ok_and(|number| number.is_finite() && (negative || number >= 0.0))
}
