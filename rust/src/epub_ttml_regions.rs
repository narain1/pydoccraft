//! Bounded TTML out-of-line regions and their root-relative rectangle.
use std::{collections::HashMap, sync::Arc};

use roxmltree::Node;

use super::{
    invalid, limited,
    media::integer,
    ttml_clock::{Clock, Timing},
    ttml_styles::{self, Patch, PARAM, STYLING, TT},
    unsupported, XML,
};
use crate::EngineError;

const MAX_REGIONS: usize = 256;
const MAX_NESTED_STYLES: usize = 32;
const MAX_LABEL: usize = 1024;
const MAX_GEOMETRY: f64 = 1_000_000.0;

#[cfg_attr(not(feature = "python"), allow(dead_code))]
pub(crate) struct Region {
    pub id: Option<String>,
    pub origin_percent: (f64, f64),
    pub extent_percent: (f64, f64),
    pub padding_percent: (f64, f64, f64, f64),
    pub display_align: &'static str,
    pub writing_mode: &'static str,
    pub overflow: &'static str,
    pub background_color: Option<[u8; 4]>,
    pub show_background: &'static str,
    pub z_index: Option<i32>,
}

#[cfg_attr(not(feature = "python"), allow(dead_code))]
pub(super) struct Definition {
    pub region: Arc<Region>,
    pub timing: Option<Timing>,
}

#[derive(Clone, Copy, Default)]
struct RegionPatch<'n> {
    origin: Option<&'n str>,
    extent: Option<&'n str>,
    position: Option<&'n str>,
    padding: Option<&'n str>,
    display_align: Option<&'n str>,
    writing_mode: Option<&'n str>,
    overflow: Option<&'n str>,
    color: Option<&'n str>,
    show_background: Option<&'n str>,
    z_index: Option<&'n str>,
    unsupported: bool,
}

impl<'n> Patch<'n> for RegionPatch<'n> {
    fn merge(self, later: Self) -> Self {
        Self {
            origin: later.origin.or(self.origin),
            extent: later.extent.or(self.extent),
            position: later.position.or(self.position),
            padding: later.padding.or(self.padding),
            display_align: later.display_align.or(self.display_align),
            writing_mode: later.writing_mode.or(self.writing_mode),
            overflow: later.overflow.or(self.overflow),
            color: later.color.or(self.color),
            show_background: later.show_background.or(self.show_background),
            z_index: later.z_index.or(self.z_index),
            unsupported: self.unsupported || later.unsupported,
        }
    }

    fn inline(self, node: Node<'n, '_>) -> Result<Self, EngineError> {
        let get = |name| node.attribute((STYLING, name));
        Ok(self.merge(Self {
            origin: get("origin"),
            extent: get("extent"),
            position: get("position"),
            padding: get("padding"),
            display_align: get("displayAlign"),
            writing_mode: get("writingMode"),
            overflow: get("overflow"),
            color: get("backgroundColor"),
            show_background: get("showBackground"),
            z_index: get("zIndex"),
            unsupported: node.has_attribute("condition")
                || get("display").is_some_and(|value| value != "auto")
                || get("visibility").is_some_and(|value| value != "visible")
                || get("backgroundImage").is_some_and(|value| value != "none")
                || get("border").is_some_and(|value| value != "none")
                || get("opacity").is_some_and(|value| value.parse::<f64>() != Ok(1.0))
                || ["ipd", "bpd", "disparity"]
                    .into_iter()
                    .any(|name| get(name).is_some()),
        }))
    }
}

impl RegionPatch<'_> {
    fn compute(self, id: Option<String>, viewport: Viewport) -> Result<Region, EngineError> {
        if self.unsupported {
            return Err(unsupported(
                "TTML region condition or unsupported layout property",
            ));
        }
        let extent = match self.extent {
            None | Some("auto") => (100.0, 100.0),
            Some(value) => viewport.pair(value, false)?,
        };
        let origin = if let Some(position) = self.position {
            region_position(position, extent)?
        } else {
            match self.origin {
                None | Some("auto") => (0.0, 0.0),
                Some(value) => viewport.pair(value, true)?,
            }
        };
        let writing_mode = match self.writing_mode {
            None | Some("lr" | "lrtb") => "lrtb",
            Some("rl" | "rltb") => "rltb",
            Some("tb" | "tbrl") => "tbrl",
            Some("tblr") => "tblr",
            _ => return Err(invalid("invalid TTML region writingMode")),
        };
        let padding = self.padding.map_or(Ok((0.0, 0.0, 0.0, 0.0)), |value| {
            viewport.padding(value, extent, writing_mode)
        })?;
        Ok(Region {
            id,
            origin_percent: origin,
            extent_percent: extent,
            padding_percent: padding,
            display_align: match self.display_align {
                None | Some("before") => "before",
                Some("center") => "center",
                Some("after") => "after",
                Some("justify") => "justify",
                _ => return Err(invalid("invalid TTML region displayAlign")),
            },
            writing_mode,
            overflow: match self.overflow {
                None | Some("hidden") => "hidden",
                Some("visible") => "visible",
                _ => return Err(invalid("invalid TTML region overflow")),
            },
            background_color: self.color.map(ttml_styles::color).transpose()?,
            show_background: match self.show_background {
                None | Some("always") => "always",
                Some("whenActive") => "whenActive",
                _ => return Err(invalid("invalid TTML region showBackground")),
            },
            z_index: match self.z_index {
                None | Some("auto") => None,
                Some(value) => Some(
                    value
                        .parse()
                        .map_err(|_| invalid("invalid TTML region zIndex"))?,
                ),
            },
        })
    }
}

#[cfg_attr(not(feature = "python"), allow(dead_code))]
pub(super) struct Regions<'n> {
    pub named: HashMap<&'n str, Definition>,
    pub implicit: Option<Arc<Region>>,
}

pub(super) fn read<'n>(root: Node<'n, '_>) -> Result<Regions<'n>, EngineError> {
    if !root.has_tag_name((TT, "tt")) {
        return Err(invalid("missing TTML tt root"));
    }
    let viewport = Viewport::new(root)?;
    let mut initial = RegionPatch::default();
    let mut definitions = Vec::new();
    for node in root.descendants().filter(Node::is_element) {
        if node.has_tag_name((TT, "initial"))
            && node
                .parent_element()
                .is_some_and(|parent| parent.has_tag_name((TT, "styling")))
        {
            initial = initial.inline(node)?;
        }
        if !node.has_tag_name((TT, "region")) {
            continue;
        }
        let parent = node
            .parent_element()
            .filter(|parent| parent.has_tag_name((TT, "layout")))
            .ok_or_else(|| unsupported("inline TTML regions"))?;
        if !parent.parent_element().is_some_and(|head| {
            head.has_tag_name((TT, "head")) && head.parent_element() == Some(root)
        }) {
            return Err(invalid("TTML region requires a head layout"));
        }
        if parent.has_attribute("condition")
            || ["begin", "end", "dur", "timeContainer"]
                .into_iter()
                .any(|key| parent.has_attribute(key))
            || node
                .attribute("timeContainer")
                .is_some_and(|value| value != "par")
        {
            return Err(unsupported("conditional or nonparallel TTML region layout"));
        }
        if definitions.len() == MAX_REGIONS {
            return Err(limited("TTML exceeds 256 region definitions"));
        }
        definitions.push(node);
    }

    let mut regions = HashMap::new();
    if !definitions.is_empty() {
        let clock = definitions
            .iter()
            .any(|node| {
                ["begin", "end", "dur"]
                    .iter()
                    .any(|&key| node.has_attribute(key))
            })
            .then(|| Clock::new(root))
            .transpose()?;
        let styles: HashMap<&str, RegionPatch<'_>> = ttml_styles::definitions(root)?;
        let lookup = |name: &str| {
            styles
                .get(name)
                .copied()
                .ok_or_else(|| invalid("missing TTML style reference"))
        };
        for node in definitions {
            let id = node
                .attribute((XML, "id"))
                .ok_or_else(|| invalid("missing TTML region ID"))?;
            check_id(id)?;
            if regions.contains_key(id) {
                return Err(invalid("duplicate TTML region ID"));
            }
            let mut style = initial.merge(ttml_styles::references(node, lookup)?);
            let mut nested = 0;
            for child in node.children().filter(Node::is_element) {
                if child.has_tag_name((TT, "metadata")) {
                    continue;
                }
                if !child.has_tag_name((TT, "style")) {
                    return Err(unsupported("animated or unsupported TTML region content"));
                }
                if nested == MAX_NESTED_STYLES {
                    return Err(limited("TTML region exceeds 32 nested styles"));
                }
                nested += 1;
                style = style.merge(ttml_styles::local(child, lookup)?);
            }
            let style = style.inline(node)?.compute(Some(id.to_owned()), viewport)?;
            regions.insert(
                id,
                Definition {
                    region: Arc::new(style),
                    timing: clock
                        .map(|clock| Timing::new(node, 0, None, clock))
                        .transpose()?,
                },
            );
        }
    }
    for node in root.descendants().filter(Node::is_element) {
        if node.tag_name().namespace() != Some(TT) {
            continue;
        }
        if let Some(id) = node.attribute("region") {
            check_id(id)?;
            if !regions.contains_key(id) {
                return Err(invalid("missing TTML region reference"));
            }
        }
    }
    let default = if regions.is_empty() {
        Some(Arc::new(RegionPatch::default().compute(None, viewport)?))
    } else {
        None
    };
    Ok(Regions {
        named: regions,
        implicit: default,
    })
}

fn check_id(id: &str) -> Result<(), EngineError> {
    if id.len() > MAX_LABEL {
        return Err(limited("TTML region name exceeds its byte budget"));
    }
    if id.is_empty() || id.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(invalid("invalid TTML region ID or reference"));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Viewport {
    pixels: Option<(f64, f64)>,
    cells: (f64, f64),
}

impl Viewport {
    fn new(root: Node<'_, '_>) -> Result<Self, EngineError> {
        let pixels = match root.attribute((STYLING, "extent")) {
            None | Some("auto" | "contain") => None,
            Some(value) => {
                let (x, y) = length_pair(value)?;
                if x.unit != "px" || y.unit != "px" || x.value <= 0.0 || y.value <= 0.0 {
                    return Err(invalid(
                        "TTML root extent requires two positive pixel lengths",
                    ));
                }
                Some((x.value, y.value))
            }
        };
        let cells = if let Some(value) = root.attribute((PARAM, "cellResolution")) {
            let (x, y) = pair(value)?;
            let parse = |value| {
                integer(value)
                    .filter(|&count| (1..=1_000_000).contains(&count))
                    .map(|count| count as f64)
                    .ok_or_else(|| invalid("invalid TTML cell resolution"))
            };
            (parse(x)?, parse(y)?)
        } else {
            (32.0, 15.0)
        };
        Ok(Self { pixels, cells })
    }

    fn length(self, length: Length<'_>, horizontal: bool, basis: f64) -> Result<f64, EngineError> {
        let axis = |pair: (f64, f64)| if horizontal { pair.0 } else { pair.1 };
        let percent = match length.unit {
            "%" => length.value * basis / 100.0,
            "px" => {
                if length.value == 0.0 {
                    return Ok(0.0);
                }
                let pixels = self
                    .pixels
                    .ok_or_else(|| unsupported("pixel TTML regions require a pixel root extent"))?;
                length.value / axis(pixels) * 100.0
            }
            "c" => length.value / axis(self.cells) * 100.0,
            _ => return Err(unsupported("TTML region length unit")),
        };
        bounded(percent)
    }

    fn pair(self, source: &str, allow_negative: bool) -> Result<(f64, f64), EngineError> {
        let (x, y) = length_pair(source)?;
        if !allow_negative && (x.value < 0.0 || y.value < 0.0) {
            return Err(invalid("negative TTML region extent"));
        }
        Ok((self.length(x, true, 100.0)?, self.length(y, false, 100.0)?))
    }

    fn padding(
        self,
        source: &str,
        extent: (f64, f64),
        writing_mode: &str,
    ) -> Result<(f64, f64, f64, f64), EngineError> {
        let mut parts = source.split_ascii_whitespace();
        let first = parts
            .next()
            .ok_or_else(|| invalid("invalid TTML region padding"))?;
        let second = parts.next().unwrap_or(first);
        let third = parts.next().unwrap_or(first);
        let fourth = parts.next().unwrap_or(second);
        if parts.next().is_some() {
            return Err(invalid("invalid TTML region padding"));
        }
        let [top, right, bottom, left] = match writing_mode {
            "rltb" => [first, fourth, third, second],
            "tbrl" => [fourth, first, second, third],
            "tblr" => [fourth, third, second, first],
            _ => [first, second, third, fourth],
        };
        let inset = |token, horizontal, basis| {
            let length = parse_length(token)?;
            if length.value < 0.0 {
                return Err(invalid("negative TTML region padding"));
            }
            self.length(length, horizontal, basis)
        };
        Ok((
            inset(top, false, extent.1)?,
            inset(right, true, extent.0)?,
            inset(bottom, false, extent.1)?,
            inset(left, true, extent.0)?,
        ))
    }
}

#[derive(Clone, Copy)]
struct Length<'a> {
    value: f64,
    unit: &'a str,
}

fn parse_length(source: &str) -> Result<Length<'_>, EngineError> {
    let split = source
        .find(|character| !matches!(character, '+' | '-' | '.' | '0'..='9'))
        .unwrap_or(source.len());
    let (number, unit) = source.split_at(split);
    let value = number
        .parse::<f64>()
        .map_err(|_| invalid("invalid TTML region length"))?;
    let value = bounded(value)?;
    if unit.is_empty() && value != 0.0 {
        return Err(invalid("missing TTML region length unit"));
    }
    Ok(Length {
        value,
        unit: if unit.is_empty() { "px" } else { unit },
    })
}

fn bounded(value: f64) -> Result<f64, EngineError> {
    if value.abs() <= MAX_GEOMETRY {
        Ok(value)
    } else {
        Err(limited(
            "TTML region geometry exceeds its coordinate budget",
        ))
    }
}

fn pair(source: &str) -> Result<(&str, &str), EngineError> {
    let mut parts = source.split_ascii_whitespace();
    match (parts.next(), parts.next(), parts.next()) {
        (Some(x), Some(y), None) => Ok((x, y)),
        _ => Err(invalid("TTML region geometry requires two values")),
    }
}

fn length_pair(source: &str) -> Result<(Length<'_>, Length<'_>), EngineError> {
    let (x, y) = pair(source)?;
    Ok((parse_length(x)?, parse_length(y)?))
}

fn region_position(source: &str, extent: (f64, f64)) -> Result<(f64, f64), EngineError> {
    let mut words = source.split_ascii_whitespace();
    let first = words
        .next()
        .ok_or_else(|| invalid("empty TTML region position"))?;
    let second = words.next();
    if words.next().is_some() {
        return Err(unsupported("TTML region position offsets"));
    }
    let (x, y) = match (first, second) {
        ("top" | "bottom", Some(x @ ("left" | "right" | "center"))) => (x, first),
        ("top" | "bottom", None) => ("center", first),
        (x, y) => (x, y.unwrap_or("center")),
    };
    let offset = |word, low, high| match word {
        "center" => Ok(50.0),
        current if current == low => Ok(0.0),
        current if current == high => Ok(100.0),
        current if current.ends_with('%') => Ok(parse_length(current)?.value),
        _ => Err(unsupported(
            "TTML region position requires keywords or percentages",
        )),
    };
    Ok((
        bounded(offset(x, "left", "right")? * (100.0 - extent.0) / 100.0)?,
        bounded(offset(y, "top", "bottom")? * (100.0 - extent.1) / 100.0)?,
    ))
}
