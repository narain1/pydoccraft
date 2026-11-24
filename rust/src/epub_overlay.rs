//! Read EPUB media overlays as inert, ordered SMIL presentation data.
use std::collections::{HashMap, HashSet};

use roxmltree::{Document, Node};
use serde_json::{json, Map, Value};

use super::{
    attribute, charge, external, invalid, parse_xml, percent_decode, resolve, unsupported, Book,
    Item, EPUB, XML_BYTES,
};
use crate::EngineError;

pub(super) const SMIL: &str = "http://www.w3.org/ns/SMIL";
const SMIL_HTTPS: &str = "https://www.w3.org/ns/SMIL";
const MAX_OVERLAY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Overlays {
    pub documents: Map<String, Value>,
    targets: HashMap<String, HashSet<String>>,
}

impl Overlays {
    pub(super) fn check_chapter(
        &mut self,
        path: &str,
        xml: &Document<'_>,
    ) -> Result<(), EngineError> {
        if let Some(mut fragments) = self.targets.remove(path) {
            for node in xml.descendants().filter(Node::is_element) {
                for id in super::element_ids(node) {
                    fragments.remove(id);
                }
                if fragments.is_empty() {
                    return Ok(());
                }
            }
            return Err(invalid("media overlay text points to a missing element ID"));
        }
        Ok(())
    }

    pub(super) fn check_remaining(&mut self, book: &mut Book<'_>) -> Result<(), EngineError> {
        while let Some(path) = self.targets.keys().next().cloned() {
            let bytes = book.read(&path, XML_BYTES)?;
            self.check_chapter(&path, &parse_xml(&bytes)?)?;
        }
        Ok(())
    }
}

pub(super) fn read(
    book: &mut Book<'_>,
    items: &HashMap<&str, Item<'_>>,
    media: &HashMap<String, &str>,
) -> Result<Overlays, EngineError> {
    let mut owners = HashMap::new();
    for entry in items.values() {
        if let Some(id) = entry.media_overlay {
            if !entry.local
                || !matches!(
                    entry.media,
                    "application/xhtml+xml" | "text/html" | "image/svg+xml"
                )
            {
                return Err(invalid(
                    "a media overlay must describe a local content document",
                ));
            }
            if owners
                .insert(entry.resolved.as_str(), id)
                .is_some_and(|prior| prior != id)
            {
                return Err(invalid(
                    "a content document selects multiple media overlays",
                ));
            }
        }
    }
    let mut overlays = Overlays::default();
    let mut budget = 0;
    for id in owners.values().copied() {
        if overlays.documents.contains_key(id) {
            continue;
        }
        let overlay = items
            .get(id)
            .ok_or_else(|| invalid("media overlay is absent from the manifest"))?;
        if !overlay.local || overlay.media != "application/smil+xml" {
            return Err(invalid(
                "media-overlay must reference a local SMIL manifest item",
            ));
        }
        let bytes = book.read(&overlay.resolved, XML_BYTES)?;
        let xml = parse_xml(&bytes)?;
        let root = xml.root_element();
        // EPUB 3.3 uses HTTPS in its definition and HTTP in its SMIL examples.
        let namespace = root
            .tag_name()
            .namespace()
            .filter(|ns| matches!(*ns, SMIL | SMIL_HTTPS));
        let Some(namespace) = namespace else {
            return Err(invalid("media overlay requires the SMIL namespace"));
        };
        if root.tag_name().name() != "smil" || root.attribute("version") != Some("3.0") {
            return Err(invalid("media overlay requires a SMIL 3.0 root"));
        }
        let body = only_child(root, "body")?.ok_or_else(|| invalid("media overlay has no body"))?;
        let mut reader = Reader {
            id,
            namespace,
            base: &overlay.resolved,
            owners: &owners,
            media,
            book,
            budget: &mut budget,
            targets: &mut overlays.targets,
        };
        let value = reader.timing(body)?;
        charge(
            &mut budget,
            id.len() + overlay.resolved.len() + 64,
            MAX_OVERLAY_BYTES,
            "media overlay budget exceeded",
        )?;
        overlays.documents.insert(
            id.to_owned(),
            json!({"path": overlay.resolved, "body": value}),
        );
    }
    Ok(overlays)
}

struct Reader<'a, 'archive> {
    id: &'a str,
    namespace: &'a str,
    base: &'a str,
    owners: &'a HashMap<&'a str, &'a str>,
    media: &'a HashMap<String, &'a str>,
    book: &'a Book<'archive>,
    budget: &'a mut usize,
    targets: &'a mut HashMap<String, HashSet<String>>,
}

impl Reader<'_, '_> {
    fn timing(&mut self, node: Node<'_, '_>) -> Result<Value, EngineError> {
        let name = node.tag_name();
        let kind = name.name();
        if name.namespace() != Some(self.namespace) {
            return Err(invalid(
                "media overlay timing elements require the SMIL namespace",
            ));
        }
        let id = node.attribute("id");
        let roles = node.attribute((EPUB, "type"));
        self.charge(256 + id.map_or(0, str::len) + roles.map_or(0, str::len))?;
        let mut result = json!({"kind": kind, "id": id, "epub_type": roles});
        match kind {
            "body" | "seq" => {
                if let Some(textref) = node.attribute((EPUB, "textref")) {
                    result["text"] = self.target(textref)?;
                } else if kind == "seq" {
                    return Err(invalid("media overlay seq requires epub:textref"));
                }
                let mut children = Vec::new();
                for child in node.children().filter(Node::is_element) {
                    if !matches!(child.tag_name().name(), "seq" | "par") {
                        return Err(invalid("unexpected media overlay timing element"));
                    }
                    children.push(self.timing(child)?);
                }
                if children.is_empty() {
                    return Err(invalid("empty media overlay sequence"));
                }
                result["children"] = Value::Array(children);
            }
            "par" => {
                let text = only_child(node, "text")?
                    .ok_or_else(|| invalid("media overlay par requires text"))?;
                let audio = only_child(node, "audio")?;
                if node.children().filter(Node::is_element).count()
                    != 1 + usize::from(audio.is_some())
                {
                    return Err(invalid("unexpected media overlay content inside par"));
                }
                result["text"] = self.target(attribute(text, "src")?)?;
                result["audio"] = audio
                    .map(|node| self.audio(node))
                    .transpose()?
                    .unwrap_or(Value::Null);
            }
            _ => return Err(invalid("unexpected media overlay element")),
        }
        Ok(result)
    }

    fn charge(&mut self, bytes: usize) -> Result<(), EngineError> {
        charge(
            self.budget,
            bytes,
            MAX_OVERLAY_BYTES,
            "media overlay budget exceeded",
        )
    }

    fn target(&mut self, source: &str) -> Result<Value, EngineError> {
        let path = local_path(self.base, source)?;
        if self.owners.get(path.as_str()).copied() != Some(self.id) {
            return Err(invalid(
                "media overlay text points to a content document without this overlay",
            ));
        }
        if self.book.archive.index_for_name(&path).is_none() {
            return Err(invalid("media overlay text is absent from the archive"));
        }
        self.charge(path.len() + source.len())?;
        let fragment = source
            .split_once('#')
            .map(|(_, id)| percent_decode(id))
            .transpose()?;
        let fragment = fragment.filter(|id| !id.is_empty());
        if let Some(fragment) = &fragment {
            self.charge(path.len() + fragment.len() + 64)?;
            self.targets
                .entry(path.clone())
                .or_default()
                .insert(fragment.clone());
        }
        Ok(json!({"path": path, "fragment": fragment}))
    }

    fn audio(&mut self, node: Node<'_, '_>) -> Result<Value, EngineError> {
        let path = local_path(self.base, attribute(node, "src")?)?;
        let media_type = self
            .media
            .get(&path)
            .copied()
            .filter(|value| value.starts_with("audio/"))
            .ok_or_else(|| invalid("media overlay audio is absent from the audio manifest"))?;
        if self.book.archive.index_for_name(&path).is_none() {
            return Err(invalid("media overlay audio is absent from the archive"));
        }
        self.charge(path.len() + media_type.len())?;
        let begin = node
            .attribute("clipBegin")
            .map(clock)
            .transpose()?
            .unwrap_or(0);
        let end = node.attribute("clipEnd").map(clock).transpose()?;
        if end.is_some_and(|end| end <= begin) {
            return Err(invalid("media overlay clipEnd must be after clipBegin"));
        }
        Ok(
            json!({"path": path, "media_type": media_type, "clip_begin_us": begin, "clip_end_us": end}),
        )
    }
}

fn only_child<'a, 'input>(
    node: Node<'a, 'input>,
    tag: &str,
) -> Result<Option<Node<'a, 'input>>, EngineError> {
    let namespace = node.tag_name().namespace();
    let mut children = node.children().filter(|child| {
        child.is_element()
            && child.tag_name().namespace() == namespace
            && child.tag_name().name() == tag
    });
    let first = children.next();
    if children.next().is_some() {
        return Err(invalid(format!("multiple media overlay {tag} elements")));
    }
    Ok(first)
}

fn local_path(base: &str, source: &str) -> Result<String, EngineError> {
    if external(source) {
        return Err(unsupported("remote media overlay resources"));
    }
    resolve(base, source)
}

pub(super) fn clock(source: &str) -> Result<u64, EngineError> {
    let bad = || invalid("invalid media overlay clock value");
    let micros = if source.contains(':') {
        let mut parts = source.rsplit(':');
        let seconds = decimal(parts.next().ok_or_else(bad)?)
            .filter(|num| *num < 60.0)
            .ok_or_else(bad)?;
        let minutes = parts
            .next()
            .and_then(integer)
            .filter(|num| *num < 60)
            .ok_or_else(bad)?;
        let hours = match parts.next() {
            Some(hours) => integer(hours).ok_or_else(bad)?,
            None => 0,
        };
        if parts.next().is_some() {
            return Err(bad());
        }
        (hours as f64 * 3600.0 + minutes as f64 * 60.0 + seconds) * 1_000_000.0
    } else {
        let (number, scale) = [
            ("min", 60_000_000.0),
            ("ms", 1_000.0),
            ("h", 3_600_000_000.0),
            ("s", 1_000_000.0),
        ]
        .into_iter()
        .find_map(|(suffix, scale)| source.strip_suffix(suffix).map(|value| (value, scale)))
        .unwrap_or((source, 1_000_000.0));
        decimal(number).ok_or_else(bad)? * scale
    };
    // Keep clip times exactly representable as integers across JSON clients.
    if !micros.is_finite() || micros > ((1u64 << 53) - 1) as f64 {
        return Err(bad());
    }
    Ok(micros.round() as u64)
}

fn integer(value: &str) -> Option<u64> {
    if value.bytes().all(|digit| digit.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn decimal(value: &str) -> Option<f64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, "0"));
    if whole.is_empty()
        || fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|digit| digit.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}
