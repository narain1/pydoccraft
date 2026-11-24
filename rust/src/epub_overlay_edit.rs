//! Write EPUB 3 narration and its package associations transactionally.
use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    ops::Range,
};

use roxmltree::Node;

use super::{
    append, apply_edits, argument, check_path, encode_url_part, inner_range, relative_url,
    validate_text, OverlayEdit, Plan, TimingEdit,
};
use crate::{
    author_epub::escape_attribute,
    epub::{
        attribute, charge, editor, invalid, limited, name, overlay, parse_xml, required, resolve,
        unsupported, Book, EPUB, METADATA_BYTES, OPF, XML_BYTES,
    },
    limits::{validate_member_count, MAX_INPUT_SIZE},
    EngineError,
};

const MAX_CLOCK: u64 = (1 << 53) - 1;

pub(super) fn charge_edits(edits: &[OverlayEdit], budget: &mut usize) -> Result<(), EngineError> {
    validate_member_count(edits.len())?;
    let mut count = 0;
    for (path, cues) in edits {
        check_path(path)?;
        charge(
            budget,
            path.len(),
            MAX_INPUT_SIZE,
            "EPUB edits exceed their byte budget",
        )?;
        if let Some(cues) = cues {
            charge(
                &mut count,
                cues.len(),
                10_000,
                "too many media overlay entries",
            )?;
            for (depth, _, fragment, audio, _, _, roles) in cues {
                if *depth > 32 {
                    return Err(limited("EPUB media overlay nesting exceeds 32"));
                }
                for value in [fragment, audio, roles].into_iter().flatten() {
                    charge(
                        budget,
                        value.len(),
                        MAX_INPUT_SIZE,
                        "EPUB edits exceed their byte budget",
                    )?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn edit(
    source: &str,
    plan: &mut Plan<'_>,
    book: &mut Book<'_>,
    changes: &[OverlayEdit],
) -> Result<String, EngineError> {
    let xml = parse_xml(source.as_bytes())?;
    let root = xml.root_element();
    let manifest = required(root, "manifest")?;
    let metadata = required(root, "metadata")?;
    let mut local = HashMap::<String, Vec<Node<'_, '_>>>::new();
    let mut items = HashMap::new();
    for item in manifest.children().filter(|&node| name(node, OPF, "item")) {
        let id = attribute(item, "id")?;
        let href = attribute(item, "href")?;
        if items.insert(id, item).is_some() {
            return Err(invalid("duplicate EPUB manifest ID"));
        }
        if !crate::epub::external(href) {
            local
                .entry(resolve(plan.package, href)?)
                .or_default()
                .push(item);
        }
    }

    let mut targets = HashSet::new();
    let mut detached = HashMap::<&str, HashSet<&str>>::new();
    let mut durations = BTreeMap::new();
    let mut edits = Vec::new();
    for (chapter, cues) in changes {
        let Some([item]) = local.get(chapter).map(Vec::as_slice) else {
            return Err(argument(
                "media overlay chapter must identify one local manifest item",
            ));
        };
        if !matches!(
            item.attribute("media-type"),
            Some("application/xhtml+xml" | "image/svg+xml" | "text/html")
        ) {
            return Err(argument(
                "media overlay target must be an EPUB content document",
            ));
        }
        if !targets.insert(item.id()) {
            return Err(argument("duplicate EPUB media overlay chapter edit"));
        }
        let id = if let Some(cues) = cues {
            let path = plan.new_path("overlay", "smil");
            let (smil, duration) = write_smil(&path, chapter, cues, &local)?;
            let id = plan.add_manifest(&path, "application/smil+xml", "")?;
            durations.insert(id.clone(), duration);
            plan.files.insert(path, Cow::Owned(smil.into_bytes()));
            Some(id)
        } else {
            None
        };
        let replacement = id
            .as_ref()
            .map_or_else(String::new, |id| format!(" media-overlay=\"{id}\""));
        if let Some(old) = item.attribute_node("media-overlay") {
            detached.entry(old.value()).or_default().insert(chapter);
            edits.push((old.range(), replacement));
        } else if id.is_some() {
            let at = editor::before_attributes(source, *item)?;
            edits.push((at..at, replacement));
        }
    }

    let declared = durations_in(metadata)?;
    let mut active = BTreeMap::<&str, Vec<Node<'_, '_>>>::new();
    for item in items.values().filter(|item| !targets.contains(&item.id())) {
        if let Some(id) = item.attribute("media-overlay") {
            active.entry(id).or_default().push(*item);
        }
    }
    #[cfg(feature = "python")]
    let mut audio = AudioDurations::default();
    for (id, owners) in active {
        let item = items
            .get(id)
            .ok_or_else(|| invalid("media overlay is absent from the manifest"))?;
        let path = resolve(plan.package, attribute(*item, "href")?)?;
        let staged = plan.files.contains_key(&path);
        let reported = if staged {
            None
        } else {
            declared
                .get(id)
                .map(|node| overlay::clock(node.text().unwrap_or_default().trim()))
                .transpose()?
        };
        let dropped = detached.get(id);
        if dropped.is_none() {
            if let Some(duration) = reported {
                durations.insert(id.to_owned(), duration);
                continue;
            }
        }
        let bytes = match plan.files.get(&path) {
            Some(bytes) => Cow::Borrowed(bytes.as_ref()),
            None => Cow::Owned(book.read(&path, XML_BYTES)?),
        };
        let source = std::str::from_utf8(&bytes).map_err(invalid)?;
        let mut clip_end = |src: &str| {
            #[cfg(feature = "python")]
            {
                audio.end(&path, src, &local, &plan.files, book)
            }
            #[cfg(not(feature = "python"))]
            {
                let _ = src;
                Err(unsupported(
                    "retained media overlay requires an audio duration",
                ))
            }
        };
        if let Some(dropped) = dropped {
            let mut split = SharedSmil::new(source, &path, dropped);
            let xml = parse_xml(source.as_bytes())?;
            if !split.prune(xml.root_element())? {
                // The remaining associations had no cues of their own.
                for owner in owners {
                    if let Some(attribute) = owner.attribute_node("media-overlay") {
                        edits.push((attribute.range(), String::new()));
                    }
                }
                continue;
            }
            let mut replaced_audio = false;
            for (src, _) in &split.kept.open {
                if plan.files.contains_key(&overlay::local_path(&path, src)?) {
                    replaced_audio = true;
                    break;
                }
            }
            let duration = if let Some(duration) =
                reported.filter(|_| split.edits.is_empty() && !replaced_audio)
            {
                duration
            } else {
                match split.kept.total(&mut clip_end) {
                    Err(EngineError::Unsupported(_))
                        if reported.is_some()
                            && split.removed.open.is_empty()
                            && !replaced_audio =>
                    {
                        reported
                            .and_then(|value| value.checked_sub(split.removed.known))
                            .filter(|&value| value > split.kept.known)
                            .ok_or_else(|| invalid("inconsistent shared media overlay duration"))?
                    }
                    result => result?,
                }
            };
            if !split.edits.is_empty() {
                let revised = apply_edits(source, split.edits, XML_BYTES)?;
                plan.files.insert(path, Cow::Owned(revised.into_bytes()));
            }
            durations.insert(id.to_owned(), duration);
        } else {
            durations.insert(id.to_owned(), smil_duration(source, &mut clip_end)?);
        }
    }
    let at = inner_range(source, manifest)?.end;
    edits.push((at..at, std::mem::take(&mut plan.manifest_additions)));
    duration_edits(source, metadata, &declared, &durations, &mut edits)?;
    apply_edits(source, edits, METADATA_BYTES)
}

fn write_smil(
    path: &str,
    chapter: &str,
    cues: &[TimingEdit],
    local: &HashMap<String, Vec<Node<'_, '_>>>,
) -> Result<(String, u64), EngineError> {
    if cues.is_empty() {
        return Err(argument(
            "an authored EPUB media overlay requires at least one cue",
        ));
    }
    let mut smil = format!(
        "<smil xmlns=\"{}\" xmlns:epub=\"{EPUB}\" version=\"3.0\"><body>",
        overlay::SMIL
    );
    let chapter = relative_url(path, chapter);
    let mut duration = 0;
    let mut open_sequences = 0;
    let mut cue_id = 0;
    let mut sequence_id = 0;
    for (i, (depth, sequence, fragment, audio, begin, end, roles)) in cues.iter().enumerate() {
        if *depth > open_sequences {
            return Err(argument("invalid EPUB media overlay sequence depth"));
        }
        for _ in *depth..open_sequences {
            append(&mut smil, "</seq>", XML_BYTES)?;
        }
        open_sequences = *depth;
        let mut target = chapter.clone();
        if let Some(fragment) = fragment {
            validate_text(fragment, 4_096, "media overlay fragment")?;
            if fragment.is_empty() {
                return Err(argument("media overlay fragment is empty"));
            }
            target.push('#');
            encode_url_part(fragment, &mut target);
        }
        let roles = if let Some(roles) = roles {
            validate_text(roles, 1_024, "media overlay roles")?;
            if roles.split_ascii_whitespace().count() == 0 {
                return Err(argument("media overlay roles are empty"));
            }
            format!(" epub:type=\"{}\"", escape_attribute(roles))
        } else {
            String::new()
        };
        if *sequence {
            if cues.get(i + 1).is_none_or(|next| next.0 != depth + 1) {
                return Err(argument(
                    "an EPUB media overlay sequence requires at least one entry",
                ));
            }
            if audio.is_some() || *begin != 0 || end.is_some() {
                return Err(argument("media overlay intervals belong to cues"));
            }
            sequence_id += 1;
            append(
                &mut smil,
                &format!("<seq id=\"seq-{sequence_id}\" epub:textref=\"{target}\"{roles}>"),
                XML_BYTES,
            )?;
            open_sequences += 1;
            continue;
        }
        cue_id += 1;
        append(
            &mut smil,
            &format!("<par id=\"cue-{cue_id}\"{roles}><text src=\"{target}\"/>"),
            XML_BYTES,
        )?;
        if let Some(audio) = audio {
            check_path(audio)?;
            if !local.get(audio).is_some_and(|items| {
                items.iter().all(|item| {
                    item.attribute("media-type")
                        .is_some_and(overlay::audio_media_type)
                })
            }) {
                return Err(argument(
                    "media overlay audio must be in the local audio manifest",
                ));
            }
            let end = end
                .filter(|end| *end > *begin && *end <= MAX_CLOCK)
                .ok_or_else(|| {
                    argument("authored media overlay audio requires an explicit later clip end")
                })?;
            duration = add_duration(duration, end - begin)?;
            append(
                &mut smil,
                &format!(
                    "<audio src=\"{}\" clipBegin=\"{}\" clipEnd=\"{}\"/>",
                    relative_url(path, audio),
                    time(*begin),
                    time(end)
                ),
                XML_BYTES,
            )?;
        } else if *begin != 0 || end.is_some() {
            return Err(argument(
                "media overlay interval requires an audio resource",
            ));
        }
        append(&mut smil, "</par>", XML_BYTES)?;
    }
    for _ in 0..open_sequences {
        append(&mut smil, "</seq>", XML_BYTES)?;
    }
    append(&mut smil, "</body></smil>", XML_BYTES)?;
    Ok((smil, duration))
}

fn durations_in<'a, 'input>(
    metadata: Node<'a, 'input>,
) -> Result<HashMap<String, Node<'a, 'input>>, EngineError> {
    let mut declared = HashMap::new();
    for node in metadata.children().filter(|&node| {
        name(node, OPF, "meta") && node.attribute("property") == Some("media:duration")
    }) {
        let key = match node.attribute("refines") {
            Some(value) if value.starts_with('#') && value.len() > 1 => {
                crate::epub::percent_decode(&value[1..])?
            }
            None => String::new(),
            _ => {
                return Err(unsupported(
                    "media overlay duration with a nonlocal refinement",
                ))
            }
        };
        if declared.insert(key, node).is_some() {
            return Err(invalid("duplicate media overlay duration"));
        }
    }
    Ok(declared)
}

fn duration_edits(
    source: &str,
    metadata: Node<'_, '_>,
    declared: &HashMap<String, Node<'_, '_>>,
    durations: &BTreeMap<String, u64>,
    edits: &mut Vec<(Range<usize>, String)>,
) -> Result<(), EngineError> {
    let mut additions = String::new();
    let mut total = 0;
    for (id, &duration) in durations {
        total = add_duration(total, duration)?;
        if let Some(node) = declared.get(id.as_str()) {
            if overlay::clock(node.text().unwrap_or_default().trim())? != duration {
                edits.push((inner_range(source, *node)?, time(duration)));
            }
        } else {
            append(
                &mut additions,
                &format!(
                    "<meta xmlns=\"{OPF}\" property=\"media:duration\" refines=\"#{}\">{}</meta>",
                    escape_attribute(id),
                    time(duration)
                ),
                METADATA_BYTES,
            )?;
        }
    }
    if let Some(node) = declared.get("") {
        if durations.is_empty() {
            super::removal::check_metadata_id(*node)?;
            edits.push((node.range(), String::new()));
        } else {
            edits.push((inner_range(source, *node)?, time(total)));
        }
    } else if !durations.is_empty() {
        append(
            &mut additions,
            &format!(
                "<meta xmlns=\"{OPF}\" property=\"media:duration\">{}</meta>",
                time(total)
            ),
            METADATA_BYTES,
        )?;
    }
    if !additions.is_empty() {
        let at = inner_range(source, metadata)?.end;
        edits.push((at..at, additions));
    }
    Ok(())
}

#[derive(Default)]
struct ClipTimes<'xml> {
    known: u64,
    open: Vec<(&'xml str, u64)>,
}

impl<'xml> ClipTimes<'xml> {
    fn add(&mut self, audio: Node<'xml, '_>) -> Result<(), EngineError> {
        let begin = audio
            .attribute("clipBegin")
            .map(overlay::clock)
            .transpose()?
            .unwrap_or_default();
        if let Some(end) = audio.attribute("clipEnd") {
            self.known = add_duration(self.known, clip_length(begin, overlay::clock(end)?)?)?;
        } else {
            self.open.push((attribute(audio, "src")?, begin));
        }
        Ok(())
    }

    fn total(
        &self,
        end: &mut impl FnMut(&str) -> Result<u64, EngineError>,
    ) -> Result<u64, EngineError> {
        let mut total = self.known;
        for &(source, begin) in &self.open {
            total = add_duration(total, clip_length(begin, end(source)?)?)?;
        }
        Ok(total)
    }
}

fn clip_length(begin: u64, end: u64) -> Result<u64, EngineError> {
    end.checked_sub(begin)
        .filter(|length| *length > 0)
        .ok_or_else(|| invalid("media overlay clip end must be after clip begin"))
}

fn smil_duration(
    source: &str,
    end: &mut impl FnMut(&str) -> Result<u64, EngineError>,
) -> Result<u64, EngineError> {
    let xml = parse_xml(source.as_bytes())?;
    let namespace = xml.root_element().tag_name().namespace();
    let mut times = ClipTimes::default();
    for audio in xml.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == namespace
            && node.tag_name().name() == "audio"
    }) {
        times.add(audio)?;
    }
    times.total(end)
}

struct SharedSmil<'a, 'xml> {
    source: &'a str,
    path: &'a str,
    dropped: &'a HashSet<&'a str>,
    edits: Vec<(Range<usize>, String)>,
    kept: ClipTimes<'xml>,
    removed: ClipTimes<'xml>,
}

impl<'a, 'xml> SharedSmil<'a, 'xml> {
    fn new(source: &'a str, path: &'a str, dropped: &'a HashSet<&'a str>) -> Self {
        Self {
            source,
            path,
            dropped,
            edits: Vec::new(),
            kept: ClipTimes::default(),
            removed: ClipTimes::default(),
        }
    }

    fn prune(&mut self, root: Node<'xml, '_>) -> Result<bool, EngineError> {
        Ok(self.timing(overlay::body(root)?)?.is_some())
    }

    fn timing(&mut self, node: Node<'xml, '_>) -> Result<Option<&'xml str>, EngineError> {
        match node.tag_name().name() {
            "par" => {
                let text = overlay::only_child(node, "text")?
                    .ok_or_else(|| invalid("media overlay par requires text"))?;
                let target = attribute(text, "src")?;
                let remove = self
                    .dropped
                    .contains(overlay::local_path(self.path, target)?.as_str());
                if let Some(audio) = overlay::only_child(node, "audio")? {
                    let times = if remove {
                        &mut self.removed
                    } else {
                        &mut self.kept
                    };
                    times.add(audio)?;
                }
                if remove {
                    self.edits.push((node.range(), String::new()));
                    return Ok(None);
                }
                Ok(Some(target))
            }
            "body" | "seq" => {
                let start = self.edits.len();
                let mut first = None;
                for child in node.children().filter(Node::is_element) {
                    if child.tag_name().namespace() != node.tag_name().namespace() {
                        return Err(invalid(
                            "media overlay timing elements require the SMIL namespace",
                        ));
                    }
                    if let Some(target) = self.timing(child)? {
                        first.get_or_insert(target);
                    }
                }
                if let Some(target) = first {
                    if let Some(textref) = node.attribute_node((EPUB, "textref")) {
                        let path = overlay::local_path(self.path, textref.value())?;
                        if self.dropped.contains(path.as_str()) {
                            let range = textref.range();
                            let (key, _) =
                                self.source[range.clone()].split_once('=').ok_or_else(|| {
                                    invalid("invalid media overlay textref attribute")
                                })?;
                            let target = target.split('#').next().unwrap_or_default();
                            self.edits
                                .push((range, format!(r#"{key}="{}""#, escape_attribute(target))));
                        }
                    }
                } else {
                    self.edits.truncate(start);
                    self.edits.push((node.range(), String::new()));
                }
                Ok(first)
            }
            _ => Err(invalid("unexpected media overlay timing element")),
        }
    }
}

#[cfg(feature = "python")]
#[derive(Default)]
struct AudioDurations {
    ends: HashMap<String, u64>,
    bytes: usize,
}

#[cfg(feature = "python")]
impl AudioDurations {
    fn end(
        &mut self,
        base: &str,
        src: &str,
        items: &HashMap<String, Vec<Node<'_, '_>>>,
        files: &super::Files<'_>,
        book: &mut Book<'_>,
    ) -> Result<u64, EngineError> {
        use crate::epub::audio::{AudioSource, MAX_ENCODED_BYTES};

        let path = overlay::local_path(base, src)?;
        if let Some(&end) = self.ends.get(&path) {
            return Ok(end);
        }
        let media = items
            .get(&path)
            .and_then(|items| items.first())
            .and_then(|item| item.attribute("media-type"))
            .ok_or_else(|| invalid("media overlay audio is absent from the manifest"))?;
        let end = AudioSource::presentation_end(media, || {
            let limit = MAX_ENCODED_BYTES.min(MAX_INPUT_SIZE.saturating_sub(self.bytes));
            let bytes = match files.get(&path) {
                Some(bytes) if bytes.len() <= limit => bytes.as_ref().to_vec(),
                Some(_) => {
                    return Err(limited(
                        "media overlay duration audio exceeds its byte budget",
                    ))
                }
                None => book.read(&path, limit)?,
            };
            self.bytes += bytes.len();
            Ok(bytes)
        })?;
        self.ends.insert(path, end);
        Ok(end)
    }
}

fn add_duration(total: u64, elapsed: u64) -> Result<u64, EngineError> {
    total
        .checked_add(elapsed)
        .filter(|value| *value <= MAX_CLOCK)
        .ok_or_else(|| limited("media overlay duration exceeds its time range"))
}

fn time(micros: u64) -> String {
    let seconds = micros / 1_000_000;
    format!(
        "{}:{:02}:{:02}.{:06}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        micros % 1_000_000
    )
}
