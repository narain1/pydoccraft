//! Read bounded, standalone text tracks from local EPUB resources.
use std::sync::Arc;

use super::{charge, invalid, limited, media::integer, unsupported, validate_archive_path, Book};
use crate::EngineError;

#[path = "epub_ttml.rs"]
mod ttml;

const MAX_TRACK_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MICROS: u64 = 1_000_000;
pub(super) const MAX_TIME: u64 = (1 << 53) - 1;
const MAX_STYLE_LABEL: usize = 1024;
const MAX_STYLE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CUE_RUNS: usize = 10_000;
const MAX_TRACK_RUNS: usize = 50_000;

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
) -> Result<Vec<Cue>, EngineError> {
    validate_archive_path(path)?;
    let bytes = Book::open(epub)?.read(path, MAX_TRACK_BYTES)?;
    let mut track = Track::default();
    match media {
        "text/vtt" => webvtt(&bytes, &mut track)?,
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
    track.cues.sort_by_key(|cue| cue.start);
    Ok(track.cues)
}

fn webvtt(bytes: &[u8], track: &mut Track) -> Result<(), EngineError> {
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
    while let Some(first) = lines.by_ref().find(|line| !line.is_empty()) {
        let side_block = first
            .strip_prefix("NOTE")
            .is_some_and(|tail| tail.is_empty() || tail.starts_with([' ', '\t']))
            || matches!(first, "STYLE" | "REGION");
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
            .find_map(|value| value.strip_prefix("region:"))
            .map(str::to_owned);
        let mut markup = String::new();
        for line in lines.by_ref().take_while(|line| !line.is_empty()) {
            if !markup.is_empty() {
                markup.push('\n');
            }
            markup.push_str(line);
        }
        let StyledText { text, runs, .. } = styled_webvtt(&markup, start, end)?;
        track.push(Cue {
            start,
            end: Some(end),
            text,
            id,
            region,
            settings: (!settings.is_empty()).then_some(settings),
            markup: Some(markup),
            runs,
        })?;
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

fn styled_webvtt(mut markup: &str, start: u64, end: u64) -> Result<StyledText, EngineError> {
    let mut out = StyledText::default();
    let mut stack = Vec::<(&str, Arc<TextStyle>)>::new();
    let fallback = Arc::new(TextStyle::default());
    let mut style_work = 0;
    let mut annotation = false;
    let mut timestamp = None;
    while let Some(index) = markup.find(['<', '&']) {
        let style = stack.last().map_or(&fallback, |(_, style)| style);
        if !annotation {
            out.append(&markup[..index], style, timestamp)?;
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
                    if time > timestamp.unwrap_or(start) && time < end {
                        timestamp = Some(time);
                    }
                } else {
                    webvtt_style(tag, &mut stack, &mut style_work)?;
                }
                markup = tail;
                continue;
            }
        } else if let Some((ch, tail)) = character_reference(rest) {
            if !annotation {
                let mut encoded = [0; 4];
                out.append(ch.encode_utf8(&mut encoded), style, timestamp)?;
            }
            markup = tail;
            continue;
        }
        if !annotation {
            out.append(symbol, style, timestamp)?;
        }
        markup = rest;
    }
    if !annotation {
        out.append(
            markup,
            stack.last().map_or(&fallback, |(_, style)| style),
            timestamp,
        )?;
    }
    Ok(out)
}

fn webvtt_style<'a>(
    tag: &'a str,
    stack: &mut Vec<(&'a str, Arc<TextStyle>)>,
    work: &mut usize,
) -> Result<(), EngineError> {
    if let Some(close) = tag.strip_prefix('/') {
        if close == "ruby" && stack.last().is_some_and(|(tag, _)| *tag == "rt") {
            stack.pop();
        }
        if stack.last().is_some_and(|(open, _)| *open == close) {
            stack.pop();
        }
        return Ok(());
    }
    let (name, annotation) = tag.split_once([' ', '\t', '\n']).unwrap_or((tag, ""));
    let mut parts = name.split('.');
    let name = parts.next().unwrap_or_default();
    if !matches!(name, "b" | "i" | "u" | "c" | "v" | "lang" | "ruby" | "rt") {
        return Ok(());
    }
    if name == "rt" {
        if stack.last().is_some_and(|(tag, _)| *tag == "rt") {
            stack.pop();
        }
        if stack.last().is_none_or(|(tag, _)| *tag != "ruby") {
            return Ok(());
        }
    }
    if stack.len() == 32 {
        return Err(limited("WebVTT cue span nesting exceeds 32"));
    }
    let inherited = stack.last().map(|(_, style)| style);
    charge(
        work,
        tag.len() + inherited.map_or(0, |style| style.bytes()),
        MAX_STYLE_BYTES,
        "WebVTT cue styling exceeds its processing budget",
    )?;
    let mut style = inherited.map_or_else(TextStyle::default, |style| (**style).clone());
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
    for class in parts.filter(|class| !class.is_empty()) {
        if style.classes.len() == 32 || class.len() > MAX_STYLE_LABEL {
            return Err(limited("WebVTT cue classes exceed their budget"));
        }
        if let Some(color) = webvtt_color(class) {
            style.color = Some(color);
        }
        if let Some(color) = class.strip_prefix("bg_").and_then(webvtt_color) {
            style.background_color = Some(color);
        }
        style.classes.push(class.to_owned());
    }
    stack.push((name, Arc::new(style)));
    Ok(())
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
