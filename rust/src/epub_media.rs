//! Collect source-order XHTML media references without reading the encoded streams.
use roxmltree::Node;
use serde_json::{json, Value};

use super::{content::Context, external, invalid, resolve, unsupported, XHTML};
use crate::EngineError;

pub(super) enum AudioCodec {
    Mpeg,
    Aac,
    Opus,
    Wave,
}

pub(super) fn audio_codec(media: &str) -> Option<AudioCodec> {
    match media {
        "audio/mpeg" | "audio/mp3" => Some(AudioCodec::Mpeg),
        "audio/mp4" | "audio/x-m4a" => Some(AudioCodec::Aac),
        "audio/ogg" | "audio/opus" | "audio/ogg; codecs=opus" | "application/ogg" | "video/ogg" => {
            Some(AudioCodec::Opus)
        }
        "audio/wav" | "audio/wave" | "audio/x-wav" | "audio/vnd.wave" => Some(AudioCodec::Wave),
        _ => None,
    }
}

fn supported_audio_hint(hint: &str) -> bool {
    let (media, parameters) = hint.split_once(';').unwrap_or((hint, ""));
    let Some(codec) = audio_codec(media.trim()) else {
        return false;
    };
    let codecs = parameters.split(';').find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("codecs")
            .then(|| value.trim().trim_matches('"'))
    });
    codecs.is_none_or(|name| match codec {
        AudioCodec::Opus => name.eq_ignore_ascii_case("opus"),
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
    let (path, media) = local(src, cx)?;
    if media
        .split_once('/')
        .is_none_or(|(prefix, _)| prefix != kind)
        && media != "application/ogg"
        && !(kind == "audio" && media == "video/ogg")
    {
        return Err(invalid("embedded EPUB media type differs from its element"));
    }
    let playable_audio =
        kind == "audio" && audio_codec(media).is_some() && hint.is_none_or(supported_audio_hint);
    Ok(json!({"uri": src, "path": path, "media_type": media, "playable_audio": playable_audio}))
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
