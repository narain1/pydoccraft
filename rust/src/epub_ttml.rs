//! Project TTML media-time cues and their supported text styles.
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use roxmltree::Node;

use super::{Cue, FontStyle, StyledText, TextStyle, Track, MAX_STYLE_LABEL};
use crate::{
    epub::{
        charge, invalid, limited, parse_xml,
        ttml_clock::{Clock, Instant, Timing},
        ttml_regions::{self, Definition, Region},
        ttml_styles::{self, Patch, STYLING, TT},
        unsupported, XML,
    },
    EngineError,
};

const MAX_DEPTH: usize = 32;
const MAX_PIECES: usize = 10_000;
const MAX_TEXT_WORK: usize = 8 * 1024 * 1024;
const MAX_SETS_PER_ELEMENT: usize = 32;
const MAX_ANIMATION_WORK: usize = 8 * 1024 * 1024;

pub(super) fn parse(bytes: &[u8], track: &mut Track) -> Result<(), EngineError> {
    let xml = parse_xml(bytes)?;
    let root = xml.root_element();
    if !root.has_tag_name((TT, "tt")) {
        return Err(invalid("missing TTML tt root"));
    }
    let clock = Clock::new(root)?;
    validate_animation(root)?;
    let regions = ttml_regions::read(root)?;
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
        styles: ttml_styles::definitions(root)?,
        regions: regions.named,
        implicit_region: regions.implicit,
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
            color: opt("color").map(ttml_styles::color).transpose()?,
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
    region_layout: Option<Arc<Region>>,
    region_timing: Option<Timing>,
}

struct Reader<'a, 'n> {
    clock: Clock,
    track: &'a mut Track,
    text_work: usize,
    style_work: usize,
    animation_work: usize,
    styles: HashMap<&'n str, StylePatch>,
    regions: HashMap<&'n str, Definition>,
    implicit_region: Option<Arc<Region>>,
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
        let patch = ttml_styles::local(node, |name| {
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
        let (region_layout, region_timing) =
            region.and_then(|id| self.regions.get(id)).map_or_else(
                || (self.implicit_region.clone(), None),
                |region| (Some(region.region.clone()), region.timing),
            );
        let paragraph = Paragraph {
            id: node.attribute((XML, "id")),
            region,
            first_cue: self.track.cues.len(),
            region_layout,
            region_timing,
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
        let (start, end) = paragraph.region_timing.map_or((start, end), |timing| {
            (
                start.max(timing.begin),
                end.into_iter().chain(timing.end).min(),
            )
        });
        let start = self.clock.micros(start);
        let end = end.map(|end| self.clock.micros(end));
        if active.is_empty() || end.is_some_and(|end| end <= start) {
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
            region_layout: None,
            ttml_region_layout: paragraph.region_layout.clone(),
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
