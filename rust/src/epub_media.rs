//! Collect source-order XHTML media references without reading the encoded streams.
use roxmltree::Node;
use serde_json::{json, Value};

use super::{content::Context, external, invalid, percent_decode, resolve, unsupported, XHTML};
use crate::EngineError;

pub(super) enum AudioCodec {
    Mpeg,
    Aac,
    Opus,
    Ogg,
    Vorbis,
    Webm,
    Wave,
}

pub(super) fn audio_codec(media: &str) -> Option<AudioCodec> {
    match media {
        "audio/mpeg" | "audio/mp3" => Some(AudioCodec::Mpeg),
        "audio/mp4" | "audio/x-m4a" | "video/mp4" => Some(AudioCodec::Aac),
        "audio/opus" | "audio/ogg; codecs=opus" => Some(AudioCodec::Opus),
        "audio/ogg; codecs=vorbis" => Some(AudioCodec::Vorbis),
        "audio/ogg" | "application/ogg" | "video/ogg" => Some(AudioCodec::Ogg),
        "audio/webm" | "video/webm" => Some(AudioCodec::Webm),
        "audio/wav" | "audio/wave" | "audio/x-wav" | "audio/vnd.wave" => Some(AudioCodec::Wave),
        _ => None,
    }
}

fn codec_hint(hint: &str) -> Option<&str> {
    let (_, parameters) = hint.split_once(';')?;
    parameters.split(';').find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("codecs")
            .then(|| value.trim().trim_matches('"'))
    })
}

fn supported_audio_hint(hint: &str) -> bool {
    let media = hint.split(';').next().unwrap_or_default();
    let Some(codec) = audio_codec(media.trim()) else {
        return false;
    };
    let codecs = codec_hint(hint);
    codecs.is_none_or(|name| match codec {
        AudioCodec::Opus => name.eq_ignore_ascii_case("opus"),
        AudioCodec::Vorbis => name.eq_ignore_ascii_case("vorbis"),
        AudioCodec::Ogg | AudioCodec::Webm => {
            name.eq_ignore_ascii_case("opus") || name.eq_ignore_ascii_case("vorbis")
        }
        AudioCodec::Mpeg => name.eq_ignore_ascii_case("mp3"),
        AudioCodec::Aac => matches!(name.to_ascii_lowercase().as_str(), "mp4a.40.2" | "mp4a.67"),
        AudioCodec::Wave => name == "1" || name.eq_ignore_ascii_case("pcm"),
    })
}

fn html_tag<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    (node.is_element() && node.tag_name().namespace().is_none_or(|ns| ns == XHTML))
        .then(|| node.tag_name().name())
}

pub(super) fn poster<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    if html_tag(node) == Some("video") {
        node.attribute("poster").filter(|src| !src.is_empty())
    } else {
        None
    }
}

pub(super) fn label<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.attribute("aria-label")
        .or_else(|| node.attribute("title"))
}

fn local<'a>(src: &str, cx: &mut Context<'a, '_>) -> Result<(String, &'a str), EngineError> {
    if src.trim().is_empty() {
        return Err(invalid("empty EPUB embedded media reference"));
    }
    if external(src) {
        return Err(unsupported("remote embedded media poster or track"));
    }
    if src.contains(['?', '#']) {
        return Err(unsupported("embedded media URLs with a query or fragment"));
    }
    let path = resolve(cx.path, src)?;
    let media = cx.manifest.get(&path).copied().ok_or_else(|| {
        invalid(format!(
            "embedded EPUB media resource is absent from the manifest: {path}"
        ))
    })?;
    if cx.book.archive.index_for_name(&path).is_none() {
        return Err(invalid(format!(
            "missing EPUB embedded media resource: {path}"
        )));
    }
    cx.budget.object()?;
    cx.budget
        .bytes(path.len().saturating_add(media.len()).saturating_mul(3))?;
    Ok((path, media))
}

fn source(
    src: &str,
    hint: Option<&str>,
    kind: &str,
    cx: &mut Context<'_, '_>,
) -> Result<Value, EngineError> {
    cx.budget.bytes(src.len().saturating_mul(3))?;
    if external(src) {
        cx.budget.object()?;
        cx.budget
            .bytes(hint.map_or(0, str::len).saturating_mul(3))?;
        return Ok(json!({"uri": src, "path": null, "media_type": hint, "playable_audio": false}));
    }
    let (uri, fragment) = src
        .split_once('#')
        .map_or((src, None), |(uri, fragment)| (uri, Some(fragment)));
    let MediaFragment {
        start: fragment_start_us,
        end: fragment_end_us,
        spatial,
        tracks,
        chapter_id,
    } = fragment
        .map(|fragment| media_fragment(fragment, kind))
        .transpose()?
        .unwrap_or_default();
    let (path, media) = local(uri, cx)?;
    if media
        .split_once('/')
        .is_none_or(|(prefix, _)| prefix != kind)
        && media != "application/ogg"
        && !(kind == "audio" && media == "video/ogg")
    {
        return Err(invalid("embedded EPUB media type differs from its element"));
    }
    if !tracks.is_empty()
        && !matches!(
            media,
            "audio/ogg" | "video/ogg" | "application/ogg" | "video/webm" | "audio/webm"
        )
    {
        return Err(unsupported(
            "named media fragments require local WebM or Ogg",
        ));
    }
    if chapter_id.is_some()
        && !matches!(
            media,
            "audio/webm" | "video/webm" | "audio/mp4" | "audio/x-m4a" | "video/mp4"
        )
    {
        return Err(unsupported(
            "media fragment IDs require a local WebM or MP4 source",
        ));
    }
    let ogg_video = kind == "video" && matches!(media, "video/ogg" | "application/ogg");
    let webm_video = kind == "video" && media == "video/webm";
    let advertises = |codec: &str| {
        hint.and_then(codec_hint).is_none_or(|list| {
            list.split(',').any(|name| {
                name.trim()
                    .split('.')
                    .next()
                    .is_some_and(|id| id.eq_ignore_ascii_case(codec))
            })
        })
    };
    let playable_audio =
        (kind == "audio" && audio_codec(media).is_some() && hint.is_none_or(supported_audio_hint))
            || (kind == "video" && media == "video/mp4")
            || ((ogg_video || webm_video) && (advertises("opus") || advertises("vorbis")));
    let playable_video = kind == "video" && media == "video/mp4"
        || (ogg_video && advertises("theora"))
        || (webm_video
            && ["av01", "av1", "vp8", "vp9", "vp09"]
                .into_iter()
                .any(advertises));
    let mut source = json!({
        "uri": src, "path": path, "media_type": media, "playable_audio": playable_audio,
        "fragment_start_us": fragment_start_us, "fragment_end_us": fragment_end_us,
        "playable_video": playable_video,
    });
    if let Some(SpatialFragment {
        percent,
        coordinates: [x, y, width, height],
    }) = spatial
    {
        let unit = if percent { "percent" } else { "pixel" };
        source["fragment_xywh"] = json!([unit, x, y, width, height]);
    }
    if !tracks.is_empty() {
        source["fragment_tracks"] = json!(tracks);
    }
    if let Some(chapter_id) = chapter_id {
        source["fragment_id"] = json!(chapter_id);
    }
    Ok(source)
}

pub(super) const MAX_TIME: u128 = (1u128 << 53) - 1;

#[derive(Default)]
struct MediaFragment {
    start: u64,
    end: Option<u64>,
    spatial: Option<SpatialFragment>,
    tracks: Vec<String>,
    chapter_id: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) struct SpatialFragment {
    pub(super) percent: bool,
    pub(super) coordinates: [u64; 4],
}

impl SpatialFragment {
    pub(super) fn new(percent: bool, coordinates: [u64; 4]) -> Result<Self, EngineError> {
        let [x, y, width, height] = coordinates;
        if width == 0 || height == 0 {
            return Err(invalid(
                "embedded media spatial fragment needs a positive size",
            ));
        }
        if percent && (x.saturating_add(width) > 100 || y.saturating_add(height) > 100) {
            return Err(invalid(
                "embedded media percent fragment exceeds 100 percent",
            ));
        }
        Ok(Self {
            percent,
            coordinates,
        })
    }

    fn parse(value: &str) -> Result<Self, EngineError> {
        let (percent, coordinates) = match value.split_once(':') {
            Some(("pixel", coordinates)) => (false, coordinates),
            Some(("percent", coordinates)) => (true, coordinates),
            Some(_) => return Err(invalid("invalid embedded media spatial fragment unit")),
            None => (false, value),
        };
        let malformed =
            || invalid("embedded media spatial fragment requires four unsigned integers");
        let mut parts = coordinates.split(',');
        let mut values = [0; 4];
        for value in &mut values {
            *value = parts.next().and_then(integer).ok_or_else(malformed)?;
        }
        if parts.next().is_some() {
            return Err(malformed());
        }
        Self::new(percent, values)
    }
}

fn media_fragment(fragment: &str, kind: &str) -> Result<MediaFragment, EngineError> {
    let mut timing = None;
    let mut spatial = None;
    let mut tracks = Vec::new();
    let mut chapter_id = None;
    for pair in fragment.split('&') {
        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| unsupported("embedded media fragment requires name-value dimensions"))?;
        match percent_decode(name)?.as_str() {
            "t" if timing.is_none() => {
                if chapter_id.is_some() {
                    return Err(unsupported(
                        "embedded media fragments cannot combine t and id",
                    ));
                }
                timing = Some(temporal_fragment(&percent_decode(value)?)?);
            }
            "id" if chapter_id.is_none() => {
                if timing.is_some() {
                    return Err(unsupported(
                        "embedded media fragments cannot combine t and id",
                    ));
                }
                if value.contains('=') {
                    return Err(invalid("invalid embedded media fragment ID"));
                }
                chapter_id = Some(fragment_name(value, "invalid embedded media fragment ID")?);
            }
            "xywh" if kind != "video" => {
                return Err(unsupported(
                    "embedded media audio supports only a temporal t dimension",
                ))
            }
            "xywh" if spatial.is_none() => {
                spatial = Some(SpatialFragment::parse(&percent_decode(value)?)?)
            }
            "track" => {
                let name = fragment_name(value, "invalid embedded media fragment track name")?;
                if !tracks.contains(&name) {
                    if tracks.len() == 2 {
                        return Err(unsupported(
                            "media fragment requires at most one audio and one video track",
                        ));
                    }
                    tracks.push(name);
                }
            }
            _ => {
                return Err(unsupported(
                    "unsupported or repeated embedded media fragment dimension",
                ))
            }
        }
    }
    let (start, end) = timing.unwrap_or((0, None));
    Ok(MediaFragment {
        start,
        end,
        spatial,
        tracks,
        chapter_id,
    })
}

fn fragment_name(value: &str, message: &str) -> Result<String, EngineError> {
    let name = percent_decode(value)?;
    if name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err(invalid(message));
    }
    Ok(name)
}

fn temporal_fragment(value: &str) -> Result<(u64, Option<u64>), EngineError> {
    let value = value.strip_prefix("npt:").unwrap_or(value);
    let (start, end) = match value.split_once(',') {
        Some((start, end)) => {
            let start = if start.is_empty() { 0 } else { npt(start)? };
            (start, Some(npt(end)?))
        }
        None => (npt(value)?, None),
    };
    if end.is_some_and(|end| end <= start) {
        return Err(invalid("embedded media fragment end must follow its start"));
    }
    Ok((start, end))
}

fn npt(value: &str) -> Result<u64, EngineError> {
    let malformed = || invalid("invalid embedded media normal play time");
    let mut fields = value.rsplit(':');
    let seconds = fields.next().ok_or_else(malformed)?;
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    if !fraction.bytes().all(|digit| digit.is_ascii_digit()) {
        return Err(malformed());
    }
    let mut micro = 0u128;
    for (i, digit) in fraction
        .bytes()
        .chain(std::iter::repeat(b'0'))
        .take(7)
        .enumerate()
    {
        if i < 6 {
            micro = micro * 10 + u128::from(digit - b'0');
        } else if digit >= b'5' {
            micro += 1;
        }
    }
    let s = integer(whole).ok_or_else(malformed)?;
    let m = if let Some(minutes) = fields.next() {
        if s >= 60 || whole.len() != 2 || minutes.len() != 2 {
            return Err(malformed());
        }
        integer(minutes)
            .filter(|&minutes| minutes < 60)
            .ok_or_else(malformed)?
    } else {
        0
    };
    let h = fields
        .next()
        .map(|h| integer(h).ok_or_else(malformed))
        .transpose()?
        .unwrap_or(0);
    if fields.next().is_some() {
        return Err(malformed());
    }
    let time = (u128::from(h) * 3600 + u128::from(m) * 60 + u128::from(s)) * 1_000_000 + micro;
    if time > MAX_TIME {
        return Err(malformed());
    }
    Ok(time as u64)
}

pub(super) fn integer(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub(super) fn inventory(
    body: Node<'_, '_>,
    cx: &mut Context<'_, '_>,
) -> Result<Vec<Value>, EngineError> {
    let mut entries = Vec::new();
    for node in body.descendants() {
        let Some(kind @ ("audio" | "video")) = html_tag(node) else {
            continue;
        };
        if node
            .ancestors()
            .skip(1)
            .any(|ancestor| matches!(html_tag(ancestor), Some("template" | "audio" | "video")))
        {
            continue;
        }
        cx.budget.object()?;
        let id = node.attribute("id");
        let label = label(node);
        cx.budget.bytes(
            id.map_or(0, str::len)
                .saturating_add(label.map_or(0, str::len))
                .saturating_mul(3),
        )?;
        let mut sources = Vec::new();
        let mut tracks = Vec::new();
        let direct = node.attribute("src");
        if let Some(src) = direct {
            sources.push(source(src, None, kind, cx)?);
        }
        for child in node.children() {
            match html_tag(child) {
                Some("source") if direct.is_none() => {
                    let src = super::attribute(child, "src")?;
                    sources.push(source(src, child.attribute("type"), kind, cx)?);
                }
                Some("track") => {
                    let (path, media_type) = local(super::attribute(child, "src")?, cx)?;
                    if !matches!(media_type, "text/vtt" | "application/ttml+xml") {
                        return Err(unsupported(
                            "embedded EPUB text tracks require WebVTT or TTML",
                        ));
                    }
                    let kind = child.attribute("kind").unwrap_or("subtitles");
                    let label = child.attribute("label");
                    let language = child.attribute("srclang");
                    cx.budget.bytes(
                        (kind.len() + label.map_or(0, str::len) + language.map_or(0, str::len))
                            .saturating_mul(3),
                    )?;
                    tracks.push(json!({
                        "path": path, "media_type": media_type, "kind": kind,
                        "label": label, "language": language, "default": child.has_attribute("default"),
                    }));
                }
                _ => {}
            }
        }
        let poster_path = poster(node)
            .map(|src| {
                let (path, media) = local(src, cx)?;
                if !media.starts_with("image/") {
                    return Err(invalid("EPUB video poster must be an image"));
                }
                Ok(path)
            })
            .transpose()?;
        entries.push(json!({
            "kind": kind, "id": id, "label": label, "sources": sources,
            "tracks": tracks, "poster_path": poster_path,
        }));
    }
    Ok(entries)
}
