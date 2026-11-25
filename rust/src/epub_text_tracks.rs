//! Read bounded, standalone text tracks from local EPUB resources.
use std::{collections::BTreeSet, sync::Arc};

use super::{charge, invalid, limited, media::integer, unsupported, validate_archive_path, Book};
use crate::EngineError;

#[path = "epub_ttml.rs"]
mod ttml;
#[path = "epub_webvtt_css.rs"]
mod webvtt_css;

const MAX_TRACK_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MICROS: u64 = 1_000_000;
pub(super) const MAX_TIME: u64 = (1 << 53) - 1;
const MAX_STYLE_LABEL: usize = 1024;
const MAX_STYLE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CUE_RUNS: usize = 10_000;
const MAX_TRACK_RUNS: usize = 50_000;
const MAX_TIMED_CUE_NODES: usize = 50_000;
const MAX_TIMED_CUE_BOUNDARIES: usize = 4096;
const MAX_TIMED_WEBVTT_WORK: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

impl FontStyle {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Italic => "italic",
            Self::Oblique => "oblique",
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct TextStyle {
    pub bold: bool,
    pub font_style: FontStyle,
    pub underline: bool,
    pub line_through: bool,
    pub overline: bool,
    pub color: Option<[u8; 4]>,
    pub background_color: Option<[u8; 4]>,
    pub classes: Vec<String>,
    pub voice: Option<String>,
    pub language: Option<String>,
}

impl TextStyle {
    fn bytes(&self) -> usize {
        self.classes.iter().map(String::len).sum::<usize>()
            + self.voice.as_ref().map_or(0, String::len)
            + self.language.as_ref().map_or(0, String::len)
    }
}

#[derive(PartialEq, Eq)]
pub(crate) struct TextRun {
    pub text: String,
    pub style: Arc<TextStyle>,
    pub timestamp: Option<u64>,
}

#[derive(Default)]
struct StyledText {
    text: String,
    runs: Vec<TextRun>,
    style_bytes: usize,
}

impl StyledText {
    fn append(
        &mut self,
        text: &str,
        style: &Arc<TextStyle>,
        timestamp: Option<u64>,
    ) -> Result<(), EngineError> {
        let offset = self.text.len();
        self.text.push_str(text);
        self.record(offset, style, timestamp)
    }

    fn record(
        &mut self,
        offset: usize,
        style: &Arc<TextStyle>,
        timestamp: Option<u64>,
    ) -> Result<(), EngineError> {
        let text = &self.text[offset..];
        if text.is_empty() {
            return Ok(());
        }
        let previous = self.runs.last_mut().filter(|run| {
            (Arc::ptr_eq(&run.style, style) || run.style == *style) && run.timestamp == timestamp
        });
        let style_bytes = if previous.is_some() { 0 } else { style.bytes() };
        charge(
            &mut self.style_bytes,
            text.len() + style_bytes,
            MAX_STYLE_BYTES,
            "EPUB cue styling exceeds its byte budget",
        )?;
        if let Some(previous) = previous {
            previous.text.push_str(text);
        } else {
            if self.runs.len() == MAX_CUE_RUNS {
                return Err(limited("EPUB cue exceeds its styled run count budget"));
            }
            self.runs.push(TextRun {
                text: text.to_owned(),
                style: style.clone(),
                timestamp,
            });
        }
        Ok(())
    }
}

pub(crate) struct Cue {
    pub start: u64,
    pub end: Option<u64>,
    pub text: String,
    pub id: Option<String>,
    pub settings: Option<String>,
    pub region: Option<String>,
    pub markup: Option<String>,
    pub runs: Vec<TextRun>,
    pub background_color: Option<[u8; 4]>,
}

#[derive(Default)]
struct Track {
    cues: Vec<Cue>,
    bytes: usize,
    style_bytes: usize,
    runs: usize,
}

impl Track {
    fn push(&mut self, cue: Cue) -> Result<(), EngineError> {
        if self.cues.len() == 10_000 {
            return Err(limited("EPUB text track exceeds 10000 cues"));
        }
        let bytes = [
            Some(cue.text.as_str()),
            cue.id.as_deref(),
            cue.settings.as_deref(),
            cue.region.as_deref(),
            cue.markup.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::len)
        .sum();
        charge(
            &mut self.bytes,
            bytes,
            8 * 1024 * 1024,
            "EPUB text track exceeds its cue byte budget",
        )?;
        charge(
            &mut self.runs,
            cue.runs.len(),
            MAX_TRACK_RUNS,
            "EPUB text track exceeds its styled run count budget",
        )?;
        let style_bytes = cue
            .runs
            .iter()
            .map(|run| run.text.len() + run.style.bytes())
            .sum();
        charge(
            &mut self.style_bytes,
            style_bytes,
            MAX_STYLE_BYTES,
            "EPUB text track styling exceeds its byte budget",
        )?;
        self.cues.push(cue);
        Ok(())
    }
}

pub(crate) fn read(
    epub: &[u8],
    path: &str,
    media: &str,
    start: u64,
    end: Option<u64>,
    language: Option<&str>,
) -> Result<Vec<Cue>, EngineError> {
    validate_archive_path(path)?;
    let bytes = Book::open(epub)?.read(path, MAX_TRACK_BYTES)?;
    let mut track = Track::default();
    match media {
        "text/vtt" => webvtt(
            &bytes,
            &mut track,
            language.filter(|value| !value.is_empty()),
        )?,
        "application/ttml+xml" => ttml::parse(&bytes, &mut track)?,
        _ => {
            return Err(unsupported(
                "embedded EPUB text tracks require WebVTT or TTML",
            ))
        }
    }
    track.cues.retain_mut(|cue| {
        cue.start = cue.start.max(start);
        cue.end = cue.end.into_iter().chain(end).min();
        cue.end.is_none_or(|end| cue.start < end)
    });
    // Keep WebVTT style intervals in authored cue order for the video sweep.
    if media != "text/vtt" {
        track.cues.sort_by_key(|cue| cue.start);
    }
    Ok(track.cues)
}

fn webvtt(bytes: &[u8], track: &mut Track, language: Option<&str>) -> Result<(), EngineError> {
    let source = std::str::from_utf8(bytes)
        .map_err(invalid)?
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if source.contains('\0') {
        return Err(invalid("NUL in EPUB WebVTT track"));
    }
    let mut lines = source.split('\n');
    let title = lines.next().unwrap_or_default();
    if title.contains("-->")
        || !title
            .strip_prefix("WEBVTT")
            .is_some_and(|tail| tail.is_empty() || tail.starts_with([' ', '\t']))
    {
        return Err(invalid("missing EPUB WebVTT signature"));
    }
    for line in lines.by_ref().take_while(|line| !line.is_empty()) {
        if line.starts_with("X-TIMESTAMP-MAP") {
            return Err(unsupported(
                "WebVTT timestamp maps require a segmented media timeline",
            ));
        }
        if line.contains("-->") {
            return Err(invalid("missing blank line after EPUB WebVTT header"));
        }
    }
    let mut previous = 0;
    let mut cascade = webvtt_css::Cascade::default();
    let mut timed_work = 0;
    while let Some(first) = lines.by_ref().find(|line| !line.is_empty()) {
        if first.trim_end_matches([' ', '\t']) == "STYLE" {
            let style_lines = lines.by_ref().take_while(|line| !line.is_empty());
            if track.cues.is_empty() {
                let mut sheet = String::new();
                for line in style_lines {
                    if line.contains("-->") {
                        return Err(invalid("WebVTT STYLE cannot contain a timing delimiter"));
                    }
                    if sheet.len() + line.len() + 1 > webvtt_css::MAX_STYLESHEET_BYTES {
                        return Err(limited("WebVTT stylesheets exceed 64 KiB"));
                    }
                    sheet.push_str(line);
                    sheet.push('\n');
                }
                cascade.sheet(&sheet)?;
            } else {
                style_lines.for_each(drop);
            }
            continue;
        }
        let side_block = first
            .strip_prefix("NOTE")
            .is_some_and(|tail| tail.is_empty() || tail.starts_with([' ', '\t']))
            || first == "REGION";
        if side_block {
            lines
                .by_ref()
                .take_while(|line| !line.is_empty())
                .for_each(drop);
            continue;
        }

        let (id, timing) = if first.contains("-->") {
            (None, first)
        } else {
            let timing = lines
                .next()
                .ok_or_else(|| invalid("missing EPUB WebVTT cue timing"))?;
            (Some(first.to_owned()), timing)
        };
        let mut parts = timing.split_ascii_whitespace();
        let (Some(start), Some("-->"), Some(end)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(invalid("invalid EPUB WebVTT cue timing"));
        };
        let start = vtt_timestamp(start)?;
        let end = vtt_timestamp(end)?;
        if start < previous || end <= start {
            return Err(invalid("out-of-order or empty EPUB WebVTT cue"));
        }
        previous = start;
        let settings = parts.collect::<Vec<_>>().join(" ");
        let region = settings
            .split_ascii_whitespace()
            .find_map(|value| value.strip_prefix("region:"));
        let mut markup = String::new();
        for line in lines.by_ref().take_while(|line| !line.is_empty()) {
            if !markup.is_empty() {
                markup.push('\n');
            }
            markup.push_str(line);
        }
        WebvttCue {
            start,
            end,
            id: id.as_deref(),
            region,
            settings: (!settings.is_empty()).then_some(settings.as_str()),
            markup: &markup,
            language,
        }
        .emit(track, &mut cascade, &mut timed_work)?;
    }
    Ok(())
}

fn vtt_timestamp(source: &str) -> Result<u64, EngineError> {
    fn read(source: &str) -> Option<u128> {
        let (whole, millis) = source.split_once('.')?;
        if millis.len() != 3 {
            return None;
        }
        let mut fields = whole.rsplit(':');
        let s = fields.next().filter(|s| s.len() == 2).and_then(integer)?;
        let m = fields.next().filter(|m| m.len() == 2).and_then(integer)?;
        let h = match fields.next() {
            Some(h) if h.len() >= 2 => integer(h)?,
            Some(_) => return None,
            None => 0,
        };
        if s >= 60 || m >= 60 || fields.next().is_some() {
            return None;
        }
        Some(
            (u128::from(h) * 3600 + u128::from(m) * 60 + u128::from(s)) * u128::from(MICROS)
                + u128::from(integer(millis)?) * 1000,
        )
    }
    read(source)
        .filter(|&time| time <= u128::from(MAX_TIME))
        .map(|time| time as u64)
        .ok_or_else(|| invalid("invalid EPUB WebVTT timestamp"))
}

struct WebvttCue<'a> {
    start: u64,
    end: u64,
    id: Option<&'a str>,
    region: Option<&'a str>,
    settings: Option<&'a str>,
    markup: &'a str,
    language: Option<&'a str>,
}

struct WebvttSpan<'a> {
    tag: &'a str,
    name: &'a str,
    classes: &'a str,
    annotation: &'a str,
}

enum WebvttEvent<'a> {
    Text(&'a str),
    Character(char),
    Timestamp(u64),
    Open(WebvttSpan<'a>),
    Close(usize),
}

struct TimedNode {
    before: Option<u64>,
    after: usize,
}

#[derive(Default)]
struct WebvttTiming {
    nodes: Vec<TimedNode>,
    timestamps: Vec<u64>,
}

impl WebvttTiming {
    fn build(cue: &WebvttCue<'_>) -> Result<Self, EngineError> {
        let mut timing = Self::default();
        let mut open = Vec::new();
        let mut nodes = 0;
        cue.walk(|event| {
            match event {
                WebvttEvent::Timestamp(time) => {
                    charge(
                        &mut nodes,
                        1,
                        MAX_TIMED_CUE_NODES,
                        "WebVTT timed cue exceeds its node budget",
                    )?;
                    timing.timestamps.push(time);
                }
                WebvttEvent::Open(_) => {
                    charge(
                        &mut nodes,
                        1,
                        MAX_TIMED_CUE_NODES,
                        "WebVTT timed cue exceeds its node budget",
                    )?;
                    open.push(timing.nodes.len());
                    timing.nodes.push(TimedNode {
                        before: timing.timestamps.last().copied(),
                        after: usize::MAX,
                    });
                }
                WebvttEvent::Close(count) => {
                    for node in open.drain(open.len() - count..) {
                        // The following timestamp index excludes the entire subtree.
                        timing.nodes[node].after = timing.timestamps.len();
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        Ok(timing)
    }

    fn position(&self, index: usize, time: u64) -> webvtt_css::PlaybackState {
        let Some(node) = self.nodes.get(index) else {
            return webvtt_css::PlaybackState::default();
        };
        webvtt_css::PlaybackState {
            past: self
                .timestamps
                .get(node.after)
                .is_some_and(|&after| after < time),
            future: node.before.is_some_and(|before| before > time),
        }
    }

    fn boundaries(
        &self,
        selectors: webvtt_css::PlaybackState,
        end: u64,
    ) -> Result<BTreeSet<u64>, EngineError> {
        let mut boundaries = BTreeSet::new();
        for node in &self.nodes {
            if selectors.past {
                if let Some(&timestamp) = self.timestamps.get(node.after) {
                    // Match a strict comparison on the public integer microsecond clock.
                    let time = timestamp + 1;
                    if time < end {
                        boundaries.insert(time);
                    }
                }
            }
            if selectors.future {
                if let Some(time) = node.before {
                    boundaries.insert(time);
                }
            }
            if boundaries.len() > MAX_TIMED_CUE_BOUNDARIES {
                return Err(limited(
                    "WebVTT timed cue exceeds its style boundary budget",
                ));
            }
        }
        Ok(boundaries)
    }
}

impl<'a> WebvttCue<'a> {
    fn emit(
        &self,
        track: &mut Track,
        cascade: &mut webvtt_css::Cascade,
        work: &mut usize,
    ) -> Result<(), EngineError> {
        let timed = cascade.timed();
        let has_timed_styles = timed.past || timed.future;
        let timing = if has_timed_styles {
            WebvttTiming::build(self)?
        } else {
            WebvttTiming::default()
        };
        let boundaries = timing.boundaries(timed, self.end)?;
        let first = track.cues.len();
        let mut start = self.start;
        for end in boundaries.into_iter().chain([self.end]) {
            if has_timed_styles {
                charge_timed_projection(work, self.markup.len().max(1))?;
            }
            let (StyledText { text, runs, .. }, background_color) = self.styled(
                &timing,
                start,
                cascade,
                has_timed_styles.then_some(&mut *work),
            )?;
            let previous = track.cues.get_mut(first..).and_then(|cues| cues.last_mut());
            if let Some(previous) =
                previous.filter(|cue| cue.runs == runs && cue.background_color == background_color)
            {
                previous.end = Some(end);
            } else {
                track.push(Cue {
                    start,
                    end: Some(end),
                    text,
                    id: self.id.map(str::to_owned),
                    region: self.region.map(str::to_owned),
                    settings: self.settings.map(str::to_owned),
                    markup: Some(self.markup.to_owned()),
                    runs,
                    background_color,
                })?;
            }
            start = end;
        }
        Ok(())
    }

    fn styled(
        &self,
        timing: &WebvttTiming,
        position: u64,
        cascade: &mut webvtt_css::Cascade,
        mut timed_work: Option<&mut usize>,
    ) -> Result<(StyledText, Option<[u8; 4]>), EngineError> {
        let mut out = StyledText::default();
        let mut styles = Vec::<Arc<TextStyle>>::new();
        let inherited = TextStyle::default();
        let mut fallback = inherited.clone();
        let background_color = cascade
            .paint(webvtt_css::Target {
                id: self.id,
                language: self.language,
                language_attribute: self.language,
                ..Default::default()
            })?
            .apply(&inherited, &mut fallback);
        let fallback = Arc::new(fallback);
        let mut timestamp = None;
        let mut style_work = 0;
        let mut node = 0;
        self.walk(|event| {
            let style = styles.last().unwrap_or(&fallback);
            match event {
                WebvttEvent::Text(text) => out.append(text, style, timestamp)?,
                WebvttEvent::Character(character) => {
                    let mut encoded = [0; 4];
                    out.append(character.encode_utf8(&mut encoded), style, timestamp)?;
                }
                WebvttEvent::Timestamp(time) => timestamp = Some(time),
                WebvttEvent::Open(span) => {
                    let inherited_bytes = style.bytes();
                    charge(
                        &mut style_work,
                        span.tag.len() + inherited_bytes,
                        MAX_STYLE_BYTES,
                        "WebVTT cue styling exceeds its processing budget",
                    )?;
                    if let Some(work) = timed_work.as_deref_mut() {
                        charge_timed_projection(work, inherited_bytes)?;
                    }
                    styles.push(Arc::new(webvtt_span_style(
                        span,
                        style,
                        self.language,
                        timing.position(node, position),
                        cascade,
                    )?));
                    node += 1;
                }
                WebvttEvent::Close(count) => styles.truncate(styles.len() - count),
            }
            Ok(())
        })?;
        Ok((out, background_color))
    }

    fn walk(
        &self,
        mut visit: impl FnMut(WebvttEvent<'a>) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let mut markup = self.markup;
        let mut stack = Vec::new();
        let mut annotation = false;
        let mut timestamp = self.start;
        while let Some(index) = markup.find(['<', '&']) {
            if !annotation {
                visit(WebvttEvent::Text(&markup[..index]))?;
            }
            let (symbol, rest) = markup[index..].split_at(1);
            if symbol == "<" {
                if let Some(end_tag) = rest
                    .find(['<', '>'])
                    .filter(|&end| rest.as_bytes()[end] == b'>')
                {
                    let (tag, tail) = (&rest[..end_tag], &rest[end_tag + 1..]);
                    // Keep ruby pronunciation in authored markup and out of the plain projection.
                    if tag == "/rt" || tag == "/ruby" {
                        annotation = false;
                    } else if tag == "rt" || tag.starts_with("rt.") {
                        annotation = true;
                    }
                    if let Ok(time) = vtt_timestamp(tag) {
                        if time > timestamp && time < self.end {
                            timestamp = time;
                            visit(WebvttEvent::Timestamp(time))?;
                        }
                    } else {
                        webvtt_tag(tag, &mut stack, &mut visit)?;
                    }
                    markup = tail;
                    continue;
                }
            } else if let Some((character, tail)) = character_reference(rest) {
                if !annotation {
                    visit(WebvttEvent::Character(character))?;
                }
                markup = tail;
                continue;
            }
            if !annotation {
                visit(WebvttEvent::Text(symbol))?;
            }
            markup = rest;
        }
        if !annotation {
            visit(WebvttEvent::Text(markup))?;
        }
        Ok(())
    }
}

fn charge_timed_projection(work: &mut usize, amount: usize) -> Result<(), EngineError> {
    charge(
        work,
        amount,
        MAX_TIMED_WEBVTT_WORK,
        "WebVTT timed style projection exceeds its work budget",
    )
}

fn webvtt_tag<'a>(
    tag: &'a str,
    stack: &mut Vec<&'a str>,
    visit: &mut impl FnMut(WebvttEvent<'a>) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    if let Some(close) = tag.strip_prefix('/') {
        let mut count = 0;
        if close == "ruby" && stack.last() == Some(&"rt") {
            stack.pop();
            count += 1;
        }
        if stack.last() == Some(&close) {
            stack.pop();
            count += 1;
        }
        if count > 0 {
            visit(WebvttEvent::Close(count))?;
        }
        return Ok(());
    }
    let (name, annotation) = tag.split_once([' ', '\t', '\n']).unwrap_or((tag, ""));
    let (name, classes) = name.split_once('.').unwrap_or((name, ""));
    if !matches!(name, "b" | "i" | "u" | "c" | "v" | "lang" | "ruby" | "rt") {
        return Ok(());
    }
    if name == "rt" {
        if stack.last() == Some(&"rt") {
            stack.pop();
            visit(WebvttEvent::Close(1))?;
        }
        if stack.last() != Some(&"ruby") {
            return Ok(());
        }
    }
    if stack.len() == 32 {
        return Err(limited("WebVTT cue span nesting exceeds 32"));
    }
    stack.push(name);
    visit(WebvttEvent::Open(WebvttSpan {
        tag,
        name,
        classes,
        annotation,
    }))
}

fn webvtt_span_style(
    span: WebvttSpan<'_>,
    inherited: &TextStyle,
    track_language: Option<&str>,
    playback: webvtt_css::PlaybackState,
    cascade: &mut webvtt_css::Cascade,
) -> Result<TextStyle, EngineError> {
    let WebvttSpan {
        name,
        classes,
        annotation,
        ..
    } = span;
    let mut style = inherited.clone();
    let annotation = annotation.trim_ascii();
    match name {
        "b" => style.bold = true,
        "i" => style.font_style = FontStyle::Italic,
        "u" => style.underline = true,
        "v" | "lang" if !annotation.is_empty() => {
            if annotation.len() > MAX_STYLE_LABEL {
                return Err(limited("WebVTT cue annotation exceeds its byte budget"));
            }
            let value = Some(decoded_annotation(annotation));
            if name == "v" {
                style.voice = value;
            } else {
                style.language = value;
            }
        }
        _ => {}
    }
    let class_start = style.classes.len();
    let mut local_background = None;
    for class in classes.split('.').filter(|class| !class.is_empty()) {
        if style.classes.len() == 32 || class.len() > MAX_STYLE_LABEL {
            return Err(limited("WebVTT cue classes exceed their budget"));
        }
        if let Some(color) = webvtt_color(class) {
            style.color = Some(color);
        }
        if let Some(color) = class.strip_prefix("bg_").and_then(webvtt_color) {
            local_background = Some(color);
        }
        style.classes.push(class.to_owned());
    }
    let paint = cascade.paint(webvtt_css::Target {
        tag: Some(name),
        id: None,
        classes: &style.classes[class_start..],
        voice: (name == "v" && !annotation.is_empty())
            .then_some(())
            .and(style.voice.as_deref()),
        language: style.language.as_deref().or(track_language),
        language_attribute: (name == "lang" && !annotation.is_empty())
            .then_some(())
            .and(style.language.as_deref()),
        playback,
    })?;
    let background = paint.apply(inherited, &mut style).or(local_background);
    style.background_color = webvtt_css::background(background, inherited.background_color);
    Ok(style)
}

fn webvtt_color(class: &str) -> Option<[u8; 4]> {
    Some(match class {
        "white" => [255, 255, 255, 255],
        "lime" => [0, 255, 0, 255],
        "cyan" => [0, 255, 255, 255],
        "red" => [255, 0, 0, 255],
        "yellow" => [255, 255, 0, 255],
        "magenta" => [255, 0, 255, 255],
        "blue" => [0, 0, 255, 255],
        "black" => [0, 0, 0, 255],
        _ => return None,
    })
}

fn decoded_annotation(mut text: &str) -> String {
    let mut output = String::new();
    while let Some(start) = text.find('&') {
        output.push_str(&text[..start]);
        let rest = &text[start + 1..];
        if let Some((character, tail)) = character_reference(rest) {
            output.push(character);
            text = tail;
        } else {
            output.push('&');
            text = rest;
        }
    }
    output.push_str(text);
    output
}

fn character_reference(rest: &str) -> Option<(char, &str)> {
    let (entity, _) = rest
        .get(..rest.floor_char_boundary(rest.len().min(16)))?
        .split_once(';')?;
    let ch = match entity {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "lrm" => '\u{200e}',
        "rlm" => '\u{200f}',
        "nbsp" => '\u{00a0}',
        "quot" => '"',
        "apos" => '\'',
        other => {
            let number = other.strip_prefix('#')?;
            let code = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                number.parse::<u32>().ok()?
            };
            char::from_u32(code)
                .filter(|&ch| ch != '\0')
                .unwrap_or('\u{fffd}')
        }
    };
    Some((ch, &rest[entity.len() + 1..]))
}
