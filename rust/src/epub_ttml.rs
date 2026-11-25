//! Project TTML media-time cues and their supported text styles.
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};

use roxmltree::Node;

use super::{Cue, StyledText, TextStyle, Track, MAX_STYLE_LABEL};
use crate::{
    epub::{
        charge, invalid, limited, parse_xml,
        ttml_clock::{Clock, Instant, Timing},
        ttml_regions::{self, Definition, Region, Regions, Selection},
        ttml_styles::{self, StylePatch, TT},
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
        regions,
    };
    let format = reader.format(
        Format::new(reader.regions.initial),
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
                    || !matches!(
                        parent.name(),
                        "body" | "div" | "p" | "span" | "br" | "region"
                    )
            })
        {
            return Err(unsupported(
                "TTML set requires an inline subtitle content target or a region target",
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

struct StylePoint {
    start: Instant,
    // Preserve explicit resets through an ancestor whose current region
    // already has the same default. A descendant can select another region.
    authored: StylePatch,
    style: Arc<TextStyle>,
}

struct StyleSet {
    timing: Timing,
    patch: StylePatch,
}

impl StyleSet {
    fn new(node: Node<'_, '_>, timing: Timing, clock: Clock) -> Result<Self, EngineError> {
        ttml_styles::validate_set(
            node,
            &["color", "fontWeight", "fontStyle", "textDecoration"],
        )?;
        let patch = StylePatch::default().inline(node)?;
        Ok(Self {
            timing: Timing::new(node, timing.begin, timing.end, clock)?,
            patch,
        })
    }
}

#[derive(Clone)]
struct Format<'a> {
    preserve: bool,
    region: Option<Selection<'a>>,
    styles: Arc<[StylePoint]>,
    animation_end: Option<Instant>,
}

impl Format<'_> {
    fn new(initial: StylePatch) -> Self {
        let mut style = TextStyle::default();
        initial.apply(&mut style);
        Self {
            preserve: false,
            region: None,
            styles: Arc::new([StylePoint {
                start: 0,
                authored: StylePatch::default(),
                style: Arc::new(style),
            }]),
            animation_end: Some(0),
        }
    }

    fn style_at(&self, time: Instant) -> usize {
        self.styles
            .partition_point(|point| point.start <= time)
            .saturating_sub(1)
    }
}

struct Piece<'a> {
    region: Option<Selection<'a>>,
    begin: Instant,
    end: Option<Instant>,
    text: &'a str,
    preserve: bool,
    style: Arc<TextStyle>,
}

struct Paragraph<'a> {
    id: Option<&'a str>,
    first_cue: usize,
    region: Option<Definition>,
    region_origin: Instant,
    implicit_region: Option<Arc<Region>>,
}

struct Reader<'a, 'n> {
    clock: Clock,
    track: &'a mut Track,
    text_work: usize,
    style_work: usize,
    animation_work: usize,
    styles: HashMap<&'n str, StylePatch>,
    regions: Regions<'n>,
}

impl<'n> Reader<'_, 'n> {
    fn format(
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
        let selected = self.regions.select(node, timing.begin, format.region);
        let region_changed = selected != format.region;
        format.region = selected;
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
        if !region_changed
            && sets.is_empty()
            && language.is_none()
            && patch == StylePatch::default()
        {
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
        let region = self
            .regions
            .get(inherited.region)
            .map_or(self.regions.initial, |definition| definition.style);
        for start in points {
            let parent = &inherited.styles[inherited.style_at(start)];
            if !sets.is_empty() || inherited.styles.len() > 1 {
                charge(
                    &mut self.animation_work,
                    1 + sets.len() + parent.style.bytes() + language.map_or(0, str::len),
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
            let authored = parent.authored.inherit(patch.merge(active));
            let mut style = TextStyle {
                language: match language {
                    Some(language) => (!language.is_empty()).then(|| language.to_owned()),
                    None => parent.style.language.clone(),
                },
                ..TextStyle::default()
            };
            region.inherit(authored).apply(&mut style);
            if output
                .last()
                .is_none_or(|point| point.authored != authored || *point.style != style)
            {
                output.push(StylePoint {
                    start,
                    authored,
                    style: Arc::new(style),
                });
            }
        }
        Ok(output.into())
    }

    fn block(
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
            self.emit_paragraph(node, format.region, timing, &pieces)?;
            return Ok(timing.finish(end));
        }
        let mut children = Children::new(node, timing.begin)?;
        children.include(format.animation_end);
        for child in node.children().filter(Node::is_element) {
            let Some(base) = children.base() else { break };
            let end = match child.tag_name() {
                name if name.namespace() != Some(TT) || matches!(name.name(), "set" | "region") => {
                    continue;
                }
                name if matches!(name.name(), "div" | "p") => {
                    self.block(child, base, timing.end, format.clone(), depth + 1)?
                }
                _ => return Err(unsupported("unsupported TTML block content")),
            };
            children.include(end);
        }
        Ok(timing.finish(children.end))
    }

    fn inline_children(
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
            let end = if child.has_tag_name((TT, "set")) || child.has_tag_name((TT, "region")) {
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
                    let line_break = child.has_tag_name((TT, "br"));
                    if style.region != format.region && (format.region.is_some() || line_break) {
                        return Err(unsupported(
                            "TTML mixed regions require region assignments on spans of unassigned content",
                        ));
                    }
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

    fn emit_paragraph(
        &mut self,
        node: Node<'_, '_>,
        region: Option<Selection<'n>>,
        timing: Timing,
        pieces: &[Piece<'n>],
    ) -> Result<(), EngineError> {
        if pieces.iter().all(|piece| piece.region == region) {
            return self.emit(node, region, timing, pieces);
        }
        if node
            .ancestors()
            .chain(node.descendants())
            .any(|element| element.has_attribute("condition"))
        {
            return Err(unsupported("conditional TTML mixed-region content"));
        }
        let mut groups = Vec::new();
        let mut seen = HashSet::new();
        for group in pieces.chunk_by(|left, right| left.region == right.region) {
            let Some(region) = group[0].region else {
                if group
                    .iter()
                    .any(|piece| piece.preserve || !piece.text.trim_ascii().is_empty())
                {
                    return Err(unsupported(
                        "TTML mixed-region paragraph contains text or preserved whitespace without a region",
                    ));
                }
                continue;
            };
            if !seen.insert(region) {
                return Err(unsupported(
                    "TTML mixed-region paragraph returns to an earlier region",
                ));
            }
            groups.push((region, group));
        }
        self.track.mixed_ttml_regions |= groups.len() > 1;
        for (region, group) in groups {
            self.emit(node, Some(region), timing, group)?;
        }
        Ok(())
    }

    fn emit(
        &mut self,
        node: Node<'_, '_>,
        region: Option<Selection<'n>>,
        timing: Timing,
        pieces: &[Piece<'_>],
    ) -> Result<(), EngineError> {
        if pieces.is_empty() {
            return Ok(());
        }
        let paragraph = Paragraph {
            id: node.attribute((XML, "id")),
            first_cue: self.track.cues.len(),
            region: self.regions.get(region).cloned(),
            region_origin: region.map_or(0, Selection::origin),
            implicit_region: self.regions.implicit.clone(),
        };
        let mut events = Vec::with_capacity(pieces.len() * 2);
        let mut piece_end = Some(timing.begin);
        for (index, piece) in pieces.iter().enumerate() {
            piece_end = piece_end
                .zip(piece.end)
                .map(|(left, right)| left.max(right));
            events.push((piece.begin, Some((true, index))));
            if let Some(end) = piece.end {
                events.push((end, Some((false, index))));
            }
        }
        if let Some(region) = &paragraph.region {
            let offset = paragraph.region_origin;
            let begin = timing.begin.saturating_sub(offset);
            let end = piece_end.map(|end| end.saturating_sub(offset));
            let first = region.layouts.partition_point(|point| point.start <= begin);
            for point in region.layouts[first..]
                .iter()
                .take_while(|point| end.is_none_or(|end| point.start < end))
            {
                events.push((self.clock.add(offset, point.start)?, None));
            }
        }
        events.sort_unstable();
        let mut active = BTreeSet::new();
        let mut previous = 0;
        for (time, piece) in events {
            if time > previous {
                self.emit_interval(pieces, &active, previous, Some(time), &paragraph)?;
                previous = time;
            }
            match piece {
                Some((true, index)) => {
                    active.insert(index);
                }
                Some((false, index)) => {
                    active.remove(&index);
                }
                None => {}
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
        let timing = paragraph.region.as_ref().and_then(|region| region.timing);
        let (start, end) = timing.map_or((start, end), |timing| {
            (
                start.max(timing.begin),
                end.into_iter().chain(timing.end).min(),
            )
        });
        let start_us = self.clock.micros(start);
        let end = end.map(|end| self.clock.micros(end));
        if active.is_empty() || end.is_some_and(|end| end <= start_us) {
            return Ok(());
        }
        let region_layout = paragraph.region.as_ref().map_or_else(
            || paragraph.implicit_region.clone(),
            |region| {
                Some(
                    region
                        .layout_at(start.saturating_sub(paragraph.region_origin))
                        .clone(),
                )
            },
        );
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
                if previous.end == Some(start_us)
                    && previous.text == text
                    && previous.runs == runs
                    && previous.ttml_region_layout == region_layout
                {
                    previous.end = end;
                    return Ok(());
                }
            }
        }
        self.track.push(Cue {
            start: start_us,
            end,
            text,
            id: paragraph.id.map(str::to_owned),
            settings: None,
            region: region_layout.as_ref().and_then(|region| region.id.clone()),
            markup: None,
            runs,
            background_color: None,
            region_layout: None,
            ttml_region_layout: region_layout,
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
    format: &Format<'n>,
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
            region: format.region,
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
