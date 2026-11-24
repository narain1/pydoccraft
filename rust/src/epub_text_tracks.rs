//! Read bounded, standalone text tracks from local EPUB resources.
use super::{charge, invalid, limited, media::integer, unsupported, validate_archive_path, Book};
use crate::EngineError;

#[path = "epub_ttml.rs"]
mod ttml;

const MAX_TRACK_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MICROS: u64 = 1_000_000;
pub(super) const MAX_TIME: u64 = (1 << 53) - 1;

pub(crate) struct Cue {
    pub start: u64,
    pub end: Option<u64>,
    pub text: String,
    pub id: Option<String>,
    pub settings: Option<String>,
    pub region: Option<String>,
    pub markup: Option<String>,
}

#[derive(Default)]
struct Track {
    cues: Vec<Cue>,
    bytes: usize,
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
        let text = plain(&markup);
        track.push(Cue {
            start,
            end: Some(end),
            text,
            id,
            region,
            settings: (!settings.is_empty()).then_some(settings),
            markup: Some(markup),
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

fn plain(mut markup: &str) -> String {
    let mut output = String::with_capacity(markup.len());
    let mut annotation = false;
    while let Some(index) = markup.find(['<', '&']) {
        if !annotation {
            output.push_str(&markup[..index]);
        }
        let (symbol, rest) = markup[index..].split_at(1);
        if symbol == "<" {
            if let Some(end) = rest
                .find(['<', '>'])
                .filter(|&end| rest.as_bytes()[end] == b'>')
            {
                let (tag, tail) = (&rest[..end], &rest[end + 1..]);
                // Ruby pronunciation remains available in the authored cue markup.
                if tag == "/rt" || tag == "/ruby" {
                    annotation = false;
                } else if tag == "rt" || tag.starts_with("rt.") {
                    annotation = true;
                }
                markup = tail;
                continue;
            }
        } else if let Some((entity, tail)) = rest
            .get(..rest.floor_char_boundary(rest.len().min(16)))
            .and_then(|prefix| prefix.split_once(';'))
            .map(|(entity, _)| (entity, &rest[entity.len() + 1..]))
        {
            let ch = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "lrm" => Some('\u{200e}'),
                "rlm" => Some('\u{200f}'),
                "nbsp" => Some('\u{00a0}'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                other => {
                    let code = other.strip_prefix(['#']).and_then(|number| {
                        if let Some(hex) = number.strip_prefix(['x', 'X']) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            number.parse::<u32>().ok()
                        }
                    });
                    code.map(|code| {
                        char::from_u32(code)
                            .filter(|&ch| ch != '\0')
                            .unwrap_or('\u{fffd}')
                    })
                }
            };
            if let Some(ch) = ch {
                if !annotation {
                    output.push(ch);
                }
                markup = tail;
                continue;
            }
        }
        if !annotation {
            output.push_str(symbol);
        }
        markup = rest;
    }
    if !annotation {
        output.push_str(markup);
    }
    output
}
