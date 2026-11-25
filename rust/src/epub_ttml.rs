//! Project TTML media-time cues and their supported text styles.
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

use roxmltree::Node;

use super::{
    integer, Cue, FontStyle, StyledText, TextStyle, Track, MAX_STYLE_LABEL, MAX_TIME, MICROS,
};
use crate::{
    epub::{charge, invalid, limited, parse_xml, unsupported, XML},
    EngineError,
};

const TT: &str = "http://www.w3.org/ns/ttml";
const STYLING: &str = "http://www.w3.org/ns/ttml#styling";
const PARAM: &str = "http://www.w3.org/ns/ttml#parameter";
// Nine decimal places plus the millisecond conversion fit into this scale.
const DECIMALS: u128 = 1_000_000_000_000;
const MAX_DEPTH: usize = 32;
const MAX_PIECES: usize = 10_000;
const MAX_TEXT_WORK: usize = 8 * 1024 * 1024;
const MAX_STYLES: usize = 512;
const MAX_STYLE_REFS: usize = 32;
const MAX_SETS_PER_ELEMENT: usize = 32;
const MAX_ANIMATION_WORK: usize = 8 * 1024 * 1024;

type Instant = u128;

pub(super) fn parse(bytes: &[u8], track: &mut Track) -> Result<(), EngineError> {
    let xml = parse_xml(bytes)?;
    let root = xml.root_element();
    if !root.has_tag_name((TT, "tt")) {
        return Err(invalid("missing TTML tt root"));
    }
    let clock = Clock::new(root)?;
    validate_animation(root)?;
    if root.descendants().any(|node| {
        node.has_tag_name((TT, "region"))
            && ["begin", "end", "dur"]
                .iter()
                .any(|&key| node.has_attribute(key))
    }) {
        return Err(unsupported("timed TTML regions"));
    }
    let mut bodies = root
        .children()
        .filter(|node| node.has_tag_name((TT, "body")));
    let Some(body) = bodies.next() else {
        return Ok(());
    };
    if bodies.next().is_some() {
        return Err(invalid("multiple TTML bodies"));
    }
    let mut reader = Reader {
        clock,
        track,
        text_work: 0,
        style_work: 0,
        animation_work: 0,
        styles: style_definitions(root)?,
    };
    let format = reader.format(
        Format::default(),
        root,
        Timing {
            begin: 0,
            end: None,
            explicit_end: false,
        },
    )?;
    reader.block(body, 0, None, format, 0)?;
    Ok(())
}

fn validate_animation(root: Node<'_, '_>) -> Result<(), EngineError> {
    for node in root.descendants().filter(Node::is_element) {
        let name = node.tag_name();
        if name.namespace() != Some(TT) {
            continue;
        }
        if node.has_attribute("animate") || matches!(name.name(), "animate" | "animation") {
            return Err(unsupported(
                "TTML animation references or continuous animation",
            ));
        }
        if name.name() == "set"
            && node.parent_element().is_none_or(|parent| {
                let parent = parent.tag_name();
                parent.namespace() != Some(TT)
                    || !matches!(parent.name(), "body" | "div" | "p" | "span" | "br")
            })
        {
            return Err(unsupported(
                "TTML set requires an inline subtitle content target",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Clock {
    units: u128,
    frame_rate: u64,
    frame_num: u128,
    frame_den: u128,
    subframes: u128,
    tick_num: u128,
    tick_den: u128,
}

impl Clock {
    fn new(root: Node<'_, '_>) -> Result<Self, EngineError> {
        if root
            .attribute((PARAM, "timeBase"))
            .is_some_and(|base| base != "media")
        {
            return Err(unsupported("TTML requires the media time base"));
        }
        let rate = |name: &str, fallback: u64| {
            root.attribute((PARAM, name))
                .map_or(Ok(fallback), positive_rate)
        };
        let frame_rate = rate("frameRate", 30)?;
        let subframes = u128::from(rate("subFrameRate", 1)?);
        let (num, den) = if let Some(value) = root.attribute((PARAM, "frameRateMultiplier")) {
            let mut parts = value.split_ascii_whitespace();
            let (Some(num), Some(den), None) = (parts.next(), parts.next(), parts.next()) else {
                return Err(invalid("invalid TTML frame rate multiplier"));
            };
            (positive_rate(num)?, positive_rate(den)?)
        } else {
            (1, 1)
        };
        let frame_num = u128::from(frame_rate) * u128::from(num);
        let frame_den = u128::from(den);
        let frame_ticks = frame_num * subframes;
        let (tick_num, tick_den) = if let Some(value) = root.attribute((PARAM, "tickRate")) {
            (u128::from(positive_rate(value)?), 1)
        } else if root.has_attribute((PARAM, "frameRate")) {
            (frame_ticks, frame_den)
        } else {
            (1, 1)
        };
        let units = DECIMALS * (frame_ticks / gcd(frame_ticks, tick_num)) * tick_num;
        if units > 10u128.pow(24) {
            return Err(limited(
                "TTML timing parameters exceed their precision budget",
            ));
        }
        Ok(Self {
            units,
            frame_rate,
            frame_num,
            frame_den,
            subframes,
            tick_num,
            tick_den,
        })
    }

    fn parse(self, value: &str) -> Result<Instant, EngineError> {
        let value = value.trim();
        let malformed = || invalid("invalid TTML time expression");
        if value.contains(':') {
            let mut fields = value.split(':');
            let h = fields
                .next()
                .filter(|s| s.len() >= 2)
                .and_then(integer)
                .ok_or_else(malformed)?;
            let m = fields
                .next()
                .filter(|s| s.len() == 2)
                .and_then(integer)
                .filter(|&n| n < 60)
                .ok_or_else(malformed)?;
            let s = fields.next().ok_or_else(malformed)?;
            if s.split('.').next().is_none_or(|s| s.len() != 2) {
                return Err(malformed());
            }
            let (num, den) = decimal(s).ok_or_else(malformed)?;
            if num >= 60 * den {
                return Err(malformed());
            }
            let seconds = (u128::from(h) * 3600 + u128::from(m) * 60) * den + num;
            let time = self.seconds(seconds, den)?;
            let Some(frame) = fields.next() else {
                return Ok(time);
            };
            if s.contains('.') || fields.next().is_some() {
                return Err(malformed());
            }
            let (frame, subframe) = if let Some((frame, subframe)) = frame.split_once('.') {
                (frame, u128::from(integer(subframe).ok_or_else(malformed)?))
            } else {
                (frame, 0)
            };
            let frame = integer(frame)
                .filter(|&n| frame.len() >= 2 && n < self.frame_rate)
                .ok_or_else(malformed)?;
            if subframe >= self.subframes {
                return Err(malformed());
            }
            let offset = self.seconds(
                (u128::from(frame) * self.subframes + subframe) * self.frame_den,
                self.frame_num * self.subframes,
            )?;
            return self.add(time, offset);
        }
        let (number, multiplier, divisor) = [
            ("ms", 1, 1000),
            ("h", 3600, 1),
            ("m", 60, 1),
            ("s", 1, 1),
            ("f", self.frame_den, self.frame_num),
            ("t", self.tick_den, self.tick_num),
        ]
        .into_iter()
        .find_map(|(unit, mul, div)| value.strip_suffix(unit).map(|number| (number, mul, div)))
        .ok_or_else(malformed)?;
        let (num, den) = decimal(number).ok_or_else(malformed)?;
        self.seconds(
            num.checked_mul(multiplier).ok_or_else(malformed)?,
            den * divisor,
        )
    }

    fn seconds(self, num: u128, den: u128) -> Result<Instant, EngineError> {
        // The document-wide clock is a multiple of every supported time unit.
        let time = (self.units / den)
            .checked_mul(num)
            .ok_or_else(|| invalid("TTML time is out of range"))?;
        self.add(time, 0)
    }

    fn add(self, a: Instant, b: Instant) -> Result<Instant, EngineError> {
        let time = a
            .checked_add(b)
            .ok_or_else(|| invalid("TTML time is out of range"))?;
        if time > u128::from(MAX_TIME) * (self.units / u128::from(MICROS)) {
            return Err(invalid("TTML time is out of range"));
        }
        Ok(time)
    }

    fn micros(self, time: Instant) -> u64 {
        let units = self.units / u128::from(MICROS);
        ((time + units / 2) / units) as u64
    }
}

fn positive_rate(value: &str) -> Result<u64, EngineError> {
    integer(value.trim())
        .filter(|rate| (1..=1_000_000).contains(rate))
        .ok_or_else(|| invalid("invalid or excessive TTML timing rate"))
}

fn decimal(value: &str) -> Option<(u128, u128)> {
    let (whole, fraction) = match value.split_once('.') {
        Some((whole, fraction))
            if !fraction.is_empty() && fraction.bytes().all(|ch| ch.is_ascii_digit()) =>
        {
            (whole, fraction.trim_end_matches('0'))
        }
        Some(_) => return None,
        None => (value, ""),
    };
    if fraction.len() > 9 {
        return None;
    }
    let scale = 10u128.pow(fraction.len() as u32);
    let whole = u128::from(integer(whole)?);
    let sub = if fraction.is_empty() {
        0
    } else {
        u128::from(integer(fraction)?)
    };
    Some((whole * scale + sub, scale))
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[derive(Clone, Copy)]
struct Timing {
    begin: Instant,
    end: Option<Instant>,
    explicit_end: bool,
}

impl Timing {
    fn new(
        node: Node<'_, '_>,
        base: Instant,
        cap: Option<Instant>,
        clock: Clock,
    ) -> Result<Self, EngineError> {
        let begin = clock.add(
            base,
            node.attribute("begin")
                .map(|value| clock.parse(value))
                .transpose()?
                .unwrap_or(0),
        )?;
        let end = node
            .attribute("end")
            .map(|end| clock.parse(end).and_then(|end| clock.add(base, end)))
            .transpose()?;
        let duration = node
            .attribute("dur")
            .map(|dur| clock.parse(dur).and_then(|dur| clock.add(begin, dur)))
            .transpose()?;
        Ok(Self {
            begin,
            end: cap.into_iter().chain(end).chain(duration).min(),
            explicit_end: end.is_some() || duration.is_some(),
        })
    }

    fn empty(self) -> bool {
        self.end.is_some_and(|end| end <= self.begin)
    }

    fn finish(self, implied: Option<Instant>) -> Option<Instant> {
        if self.explicit_end {
            self.end
        } else {
            implied
        }
    }
}

struct Children {
    start: Instant,
    end: Option<Instant>,
    sequential: bool,
}

impl Children {
    fn new(node: Node<'_, '_>, start: Instant) -> Result<Self, EngineError> {
        let sequential = match node.attribute("timeContainer") {
            None | Some("par") => false,
            Some("seq") => true,
            _ => return Err(invalid("invalid TTML time container")),
        };
        Ok(Self {
            start,
            end: Some(start),
            sequential,
        })
    }

    fn base(&self) -> Option<Instant> {
        if self.sequential {
            self.end
        } else {
            Some(self.start)
        }
    }

    fn include(&mut self, end: Option<Instant>) {
        self.end = if self.sequential {
            end
        } else {
            self.end.zip(end).map(|(a, b)| a.max(b))
        };
    }
}

// A declaration is separate from inherited style so explicit resets survive
// merging style references and inline properties before application.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct StylePatch {
    bold: Option<bool>,
    font_style: Option<FontStyle>,
    decoration: Option<[Option<bool>; 3]>,
    color: Option<[u8; 4]>,
}

impl StylePatch {
    fn merge(self, later: Self) -> Self {
        Self {
            bold: later.bold.or(self.bold),
            font_style: later.font_style.or(self.font_style),
            decoration: later.decoration.or(self.decoration),
            color: later.color.or(self.color),
        }
    }

    fn conflicts(self, other: Self) -> bool {
        (self.bold.is_some() && other.bold.is_some())
            || (self.font_style.is_some() && other.font_style.is_some())
            || (self.decoration.is_some() && other.decoration.is_some())
            || (self.color.is_some() && other.color.is_some())
    }

    fn inline(self, node: Node<'_, '_>) -> Result<Self, EngineError> {
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
            color: opt("color").map(ttml_color).transpose()?,
        };
        Ok(self.merge(later))
    }

    fn apply(self, style: &mut TextStyle) {
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

fn ttml_color(value: &str) -> Result<[u8; 4], EngineError> {
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

fn style_definitions<'n>(root: Node<'n, '_>) -> Result<HashMap<&'n str, StylePatch>, EngineError> {
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

fn define_style<'n>(
    name: &'n str,
    definitions: &HashMap<&'n str, Node<'n, '_>>,
    resolved: &mut HashMap<&'n str, StylePatch>,
    active: &mut HashSet<&'n str>,
) -> Result<StylePatch, EngineError> {
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
    let patch = referenced_styles(node, |parent| {
        define_style(parent, definitions, resolved, active)
    })?;
    active.remove(name);
    resolved.insert(name, patch);
    Ok(patch)
}

fn referenced_styles<'n>(
    node: Node<'n, '_>,
    mut lookup: impl FnMut(&'n str) -> Result<StylePatch, EngineError>,
) -> Result<StylePatch, EngineError> {
    let mut patch = StylePatch::default();
    if let Some(references) = node.attribute("style") {
        for (index, name) in references.split_ascii_whitespace().enumerate() {
            if index == MAX_STYLE_REFS {
                return Err(limited("TTML style reference list exceeds 32 entries"));
            }
            patch = patch.merge(lookup(name)?);
        }
    }
    patch.inline(node)
}

struct StylePoint {
    start: Instant,
    style: Arc<TextStyle>,
}

struct StyleSet {
    timing: Timing,
    patch: StylePatch,
}

impl StyleSet {
    fn new(node: Node<'_, '_>, timing: Timing, clock: Clock) -> Result<Self, EngineError> {
        for attribute in node.attributes() {
            let supported = match (attribute.namespace(), attribute.name()) {
                (None, "begin" | "dur" | "end") | (Some(XML), _) => true,
                (None, "fill") => attribute.value() == "remove",
                (None, "repeatCount") => attribute.value() == "1",
                (Some(STYLING), "color" | "fontWeight" | "fontStyle" | "textDecoration") => true,
                _ => false,
            };
            if !supported {
                return Err(unsupported(
                    "unsupported TTML set attribute or style property",
                ));
            }
        }
        if node.children().any(|child| {
            (child.is_element() && !child.has_tag_name((TT, "metadata")))
                || (child.is_text()
                    && child
                        .text()
                        .is_some_and(|text| !text.trim_ascii().is_empty()))
        }) {
            return Err(invalid("TTML set cannot contain subtitle content"));
        }
        let patch = StylePatch::default().inline(node)?;
        if patch == StylePatch::default() {
            return Err(invalid("TTML set has no style property"));
        }
        Ok(Self {
            timing: Timing::new(node, timing.begin, timing.end, clock)?,
            patch,
        })
    }
}

#[derive(Clone)]
struct Format<'a> {
    preserve: bool,
    region: Option<&'a str>,
    styles: Arc<[StylePoint]>,
    animation_end: Option<Instant>,
}

impl Default for Format<'_> {
    fn default() -> Self {
        Self {
            preserve: false,
            region: None,
            styles: Arc::new([StylePoint {
                start: 0,
                style: Arc::default(),
            }]),
            animation_end: Some(0),
        }
    }
}

impl Format<'_> {
    fn style_at(&self, time: Instant) -> usize {
        self.styles
            .partition_point(|point| point.start <= time)
            .saturating_sub(1)
    }
}

struct Piece<'a> {
    begin: Instant,
    end: Option<Instant>,
    text: &'a str,
    preserve: bool,
    style: Arc<TextStyle>,
}

struct Paragraph<'a> {
    id: Option<&'a str>,
    region: Option<&'a str>,
    first_cue: usize,
}

struct Reader<'a, 'n> {
    clock: Clock,
    track: &'a mut Track,
    text_work: usize,
    style_work: usize,
    animation_work: usize,
    styles: HashMap<&'n str, StylePatch>,
}

impl Reader<'_, '_> {
    fn format<'n>(
        &mut self,
        mut format: Format<'n>,
        node: Node<'n, '_>,
        timing: Timing,
    ) -> Result<Format<'n>, EngineError> {
        match node.attribute((XML, "space")) {
            Some("preserve") => format.preserve = true,
            Some("default") => format.preserve = false,
            None => {}
            _ => return Err(invalid("invalid TTML xml:space value")),
        }
        format.region = node.attribute("region").or(format.region);
        let patch = referenced_styles(node, |name| {
            self.styles
                .get(name)
                .copied()
                .ok_or_else(|| invalid("missing TTML style reference"))
        })?;
        let language = node.attribute((XML, "lang"));
        if language.is_some_and(|language| language.len() > MAX_STYLE_LABEL) {
            return Err(limited("TTML language exceeds its byte budget"));
        }
        let mut sets = Vec::new();
        format.animation_end = Some(timing.begin);
        for (index, child) in node
            .children()
            .filter(|child| child.has_tag_name((TT, "set")))
            .enumerate()
        {
            if node.attribute("timeContainer") == Some("seq") {
                return Err(unsupported(
                    "inline TTML set in a sequential time container",
                ));
            }
            if index == MAX_SETS_PER_ELEMENT {
                return Err(limited("TTML content exceeds 32 inline set elements"));
            }
            let set = StyleSet::new(child, timing, self.clock)?;
            format.animation_end = format
                .animation_end
                .zip(set.timing.end)
                .map(|(a, b)| a.max(b));
            if !set.timing.empty() {
                sets.push(set);
            }
        }
        if sets.is_empty() && language.is_none() && patch == StylePatch::default() {
            return Ok(format);
        }
        format.styles = self.style_points(&format, timing, patch, language, &sets)?;
        Ok(format)
    }

    fn style_points(
        &mut self,
        inherited: &Format<'_>,
        timing: Timing,
        patch: StylePatch,
        language: Option<&str>,
        sets: &[StyleSet],
    ) -> Result<Arc<[StylePoint]>, EngineError> {
        let first = inherited.style_at(timing.begin);
        let mut points = vec![timing.begin];
        points.extend(
            inherited.styles[first + 1..]
                .iter()
                .map(|point| point.start)
                .take_while(|&start| timing.end.is_none_or(|end| start < end)),
        );
        for set in sets {
            points.push(set.timing.begin);
            if let Some(end) = set.timing.end.filter(|&end| timing.end != Some(end)) {
                points.push(end);
            }
        }
        points.sort_unstable();
        points.dedup();
        let mut output = Vec::<StylePoint>::with_capacity(points.len());
        for start in points {
            let style = &inherited.styles[inherited.style_at(start)].style;
            if !sets.is_empty() || inherited.styles.len() > 1 {
                charge(
                    &mut self.animation_work,
                    1 + sets.len() + style.bytes() + language.map_or(0, str::len),
                    MAX_ANIMATION_WORK,
                    "TTML animation exceeds its style work budget",
                )?;
            }
            let mut active = StylePatch::default();
            for set in sets.iter().filter(|set| {
                set.timing.begin <= start && set.timing.end.is_none_or(|end| start < end)
            }) {
                if active.conflicts(set.patch) {
                    return Err(unsupported(
                        "overlapping TTML set elements target the same property",
                    ));
                }
                active = active.merge(set.patch);
            }
            let mut style = (**style).clone();
            patch.merge(active).apply(&mut style);
            if let Some(language) = language {
                style.language = (!language.is_empty()).then(|| language.to_owned());
            }
            if output.last().is_none_or(|point| *point.style != style) {
                output.push(StylePoint {
                    start,
                    style: Arc::new(style),
                });
            }
        }
        Ok(output.into())
    }

    fn block<'n>(
        &mut self,
        node: Node<'n, '_>,
        base: Instant,
        cap: Option<Instant>,
        inherited: Format<'n>,
        depth: usize,
    ) -> Result<Option<Instant>, EngineError> {
        check_depth(depth)?;
        let timing = Timing::new(node, base, cap, self.clock)?;
        if timing.empty() {
            return Ok(timing.end);
        }
        let format = self.format(inherited, node, timing)?;
        if node.has_tag_name((TT, "p")) {
            let mut pieces = Vec::new();
            let end = self.inline_children(node, timing, &format, &mut pieces, depth)?;
            self.emit(node, format.region, &pieces)?;
            return Ok(timing.finish(end));
        }
        let mut children = Children::new(node, timing.begin)?;
        children.include(format.animation_end);
        for child in node.children().filter(Node::is_element) {
            let Some(base) = children.base() else { break };
            let end = match child.tag_name() {
                name if name.namespace() != Some(TT) || name.name() == "set" => continue,
                name if matches!(name.name(), "div" | "p") => {
                    self.block(child, base, timing.end, format.clone(), depth + 1)?
                }
                _ => return Err(unsupported("unsupported TTML block content")),
            };
            children.include(end);
        }
        Ok(timing.finish(children.end))
    }

    fn inline_children<'n>(
        &mut self,
        node: Node<'n, '_>,
        timing: Timing,
        format: &Format<'n>,
        pieces: &mut Vec<Piece<'n>>,
        depth: usize,
    ) -> Result<Option<Instant>, EngineError> {
        check_depth(depth)?;
        let mut children = Children::new(node, timing.begin)?;
        children.include(format.animation_end);
        for child in node.children() {
            let Some(base) = children.base() else { break };
            let end = if child.has_tag_name((TT, "set")) {
                continue;
            } else if let Some(text) = child.text().filter(|_| child.is_text()) {
                if children.sequential {
                    continue;
                }
                push_piece(
                    pieces,
                    timing.begin,
                    timing.end,
                    text,
                    format.preserve,
                    format,
                )?;
                timing.end
            } else if child.has_tag_name((TT, "span")) || child.has_tag_name((TT, "br")) {
                let span = Timing::new(child, base, timing.end, self.clock)?;
                if span.empty() {
                    span.end
                } else {
                    let style = self.format(format.clone(), child, span)?;
                    if style.region != format.region {
                        return Err(unsupported("TTML region changes within a paragraph"));
                    }
                    let line_break = child.has_tag_name((TT, "br"));
                    let text_only = line_break || !child.children().any(|node| node.is_element());
                    if children.sequential && text_only && !span.explicit_end {
                        Some(span.begin)
                    } else if line_break {
                        push_piece(pieces, span.begin, span.end, "\n", true, &style)?;
                        span.end
                    } else {
                        let end = self.inline_children(child, span, &style, pieces, depth + 1)?;
                        span.finish(end)
                    }
                }
            } else if child.is_element() && child.tag_name().namespace() == Some(TT) {
                return Err(unsupported("unsupported timed TTML inline content"));
            } else {
                continue;
            };
            children.include(end);
        }
        Ok(children.end)
    }

    fn emit(
        &mut self,
        node: Node<'_, '_>,
        region: Option<&str>,
        pieces: &[Piece<'_>],
    ) -> Result<(), EngineError> {
        let paragraph = Paragraph {
            id: node.attribute((XML, "id")),
            region,
            first_cue: self.track.cues.len(),
        };
        let mut events = Vec::with_capacity(pieces.len() * 2);
        for (index, piece) in pieces.iter().enumerate() {
            events.push((piece.begin, true, index));
            if let Some(end) = piece.end {
                events.push((end, false, index));
            }
        }
        events.sort_unstable();
        let mut active = BTreeSet::new();
        let mut previous = 0;
        for (time, starts, index) in events {
            if time > previous {
                self.emit_interval(pieces, &active, previous, Some(time), &paragraph)?;
                previous = time;
            }
            if starts {
                active.insert(index);
            } else {
                active.remove(&index);
            }
        }
        self.emit_interval(pieces, &active, previous, None, &paragraph)
    }

    fn emit_interval(
        &mut self,
        pieces: &[Piece<'_>],
        active: &BTreeSet<usize>,
        start: Instant,
        end: Option<Instant>,
        paragraph: &Paragraph<'_>,
    ) -> Result<(), EngineError> {
        let start = self.clock.micros(start);
        let end = end.map(|end| self.clock.micros(end));
        if active.is_empty() || end.is_some_and(|end| end == start) {
            return Ok(());
        }
        let mut styled = StyledText::default();
        let mut pending = false;
        for &index in active {
            let piece = &pieces[index];
            charge(
                &mut self.text_work,
                piece.text.len(),
                MAX_TEXT_WORK,
                "TTML timed text exceeds its presentation work budget",
            )?;
            charge(
                &mut self.style_work,
                piece.style.bytes(),
                MAX_TEXT_WORK,
                "TTML styling exceeds its presentation work budget",
            )?;
            let offset = styled.text.len();
            append_text(&mut styled.text, &mut pending, piece);
            styled.record(offset, &piece.style, None)?;
        }
        let StyledText { text, runs, .. } = styled;
        if text.is_empty() {
            return Ok(());
        }
        if self.track.cues.len() > paragraph.first_cue {
            if let Some(previous) = self.track.cues.last_mut() {
                if previous.end == Some(start) && previous.text == text && previous.runs == runs {
                    previous.end = end;
                    return Ok(());
                }
            }
        }
        self.track.push(Cue {
            start,
            end,
            text,
            id: paragraph.id.map(str::to_owned),
            settings: None,
            region: paragraph.region.map(str::to_owned),
            markup: None,
            runs,
            background_color: None,
        })
    }
}

fn check_depth(depth: usize) -> Result<(), EngineError> {
    if depth > MAX_DEPTH {
        return Err(limited("TTML timing nesting exceeds 32"));
    }
    Ok(())
}

fn push_piece<'n>(
    pieces: &mut Vec<Piece<'n>>,
    begin: Instant,
    end: Option<Instant>,
    text: &'n str,
    preserve: bool,
    format: &Format<'_>,
) -> Result<(), EngineError> {
    let first = format.style_at(begin);
    for (index, point) in format.styles[first..].iter().enumerate() {
        let begin = begin.max(point.start);
        if end.is_some_and(|end| begin >= end) {
            break;
        }
        if pieces.len() == MAX_PIECES {
            return Err(limited("TTML paragraph exceeds 10000 timed text fragments"));
        }
        let next = format
            .styles
            .get(first + index + 1)
            .map(|point| point.start);
        pieces.push(Piece {
            begin,
            end: end.into_iter().chain(next).min(),
            text,
            preserve,
            style: point.style.clone(),
        });
    }
    Ok(())
}

fn append_text(output: &mut String, pending: &mut bool, piece: &Piece<'_>) {
    let space = |ch| matches!(ch, ' ' | '\t' | '\r' | '\n');
    if piece.preserve {
        if *pending && !piece.text.starts_with(space) {
            output.push(' ');
        }
        output.push_str(piece.text);
        *pending = false;
        return;
    }
    for ch in piece.text.chars() {
        if space(ch) {
            *pending = !output.is_empty() && !output.ends_with(space);
        } else {
            if *pending {
                output.push(' ');
            }
            output.push(ch);
            *pending = false;
        }
    }
}
