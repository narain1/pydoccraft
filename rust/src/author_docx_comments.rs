//! Owned whole-paragraph comments through public native parts and paired markers.
use super::*;

pub(super) const MAX_COMMENTS: usize = 1000;
const MAX_REPLIES: usize = 200;
const MAX_AUTHOR_BYTES: usize = 1024;
const MAX_COMMENT_METADATA: usize = 256 * 1024;
const MAX_COMMENT_ID: u32 = 0x7fff_fffd;
const MAX_NATIVE_PARTS: usize = 50_000;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CommentAnchor {
    pub path: Path,
    pub end: usize,
}
#[derive(Clone)]
pub(super) struct CommentEntry {
    pub value: wordcraft_doc::Comment,
    pub anchor: Option<CommentAnchor>,
    pub bytes: usize,
}
#[derive(Default)]
pub(super) struct CommentInventory {
    pub entries: BTreeMap<u32, CommentEntry>,
    pub metadata_bytes: usize,
    #[cfg(test)]
    pub source_work: usize,
}
impl CommentInventory {
    pub fn marker(&self, id: u32) -> bool {
        self.entries
            .get(&id)
            .is_some_and(|entry| entry.anchor.is_some())
    }
    fn entry(&self, id: u32) -> PyResult<&CommentEntry> {
        self.entries
            .get(&id)
            .ok_or_else(|| PyIndexError::new_err("unknown or deleted comment identifier"))
    }
    fn anchor(&self, id: u32) -> PyResult<&CommentAnchor> {
        let entry = self.entry(id)?;
        self.entry(entry.value.parent.unwrap_or(id))?
            .anchor
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("comment root has no anchor"))
    }
}

fn invalid(message: &str) -> PyErr {
    PyValueError::new_err(message.to_owned())
}
fn unsupported(message: &str) -> PyErr {
    EngineError::Unsupported(message.into()).into()
}
fn author(value: &str) -> PyResult<()> {
    if value.len() > MAX_AUTHOR_BYTES {
        return Err(limit("comment author exceeds 1024 UTF-8 bytes"));
    }
    validate_text(value)?;
    if value.is_empty()
        || value.trim() != value
        || value
            .chars()
            .any(|c| matches!(c, '\n' | '\t' | '\u{85}' | '\u{2028}' | '\u{2029}'))
    {
        return Err(invalid("comment author requires a nonempty trimmed single-line value; use Unknown when unspecified"));
    }
    Ok(())
}
fn date(value: &str) -> PyResult<()> {
    if !value.is_empty() {
        validate_property_timestamp(value)?;
    }
    Ok(())
}
fn charge(work: &mut usize, amount: usize, work_limit: usize) -> PyResult<()> {
    *work = work
        .checked_add(amount)
        .filter(|n| *n <= work_limit)
        .ok_or_else(|| limit("comment validation exceeds 2000000 source items"))?;
    Ok(())
}

/// One exact-model proof reused by navigation/media/output validation. Traversal
/// charges blocks/rows/cells/objects before allocation or inspecting their payload.
pub(super) fn inventory(document: &Document) -> PyResult<CommentInventory> {
    inventory_with_work_limit(document, MAX_LAYOUT_ITEMS)
}

pub(super) fn inventory_with_work_limit(
    document: &Document,
    work_limit: usize,
) -> PyResult<CommentInventory> {
    if document.comments.len() > MAX_COMMENTS {
        return Err(limit("document exceeds 1000 comments"));
    }
    if document.parts.len() >= MAX_NATIVE_PARTS {
        return Err(limit("document exceeds the supported native part limit"));
    }
    let mut work = 0usize;
    // Two parts passes (orphan check and source roots), and two comment passes
    // (metadata and final pair checks), precharged before inspecting payloads.
    charge(
        &mut work,
        document.parts.len() * 2 + document.comments.len() * 2,
        work_limit,
    )?;
    let mut result = CommentInventory::default();
    let mut owned_parts = BTreeSet::new();
    let mut replies = BTreeMap::<u32, usize>::new();
    let mut text_bytes = 0usize;
    for (id, value) in &document.comments {
        if *id > MAX_COMMENT_ID
            || value.durable_id.as_deref() != Some(format!("{:08X}", id + 1).as_str())
            || !value.initials.is_empty()
            || value.date_utc.is_some()
        {
            return Err(unsupported(
                "comments require canonical owned identities and metadata",
            ));
        }
        author(&value.author)?;
        date(&value.date)?;
        // Canonical fields above bound this temporary: author <=1024 bytes,
        // date <=20, durable ID exactly8, empty initials and no date_utc.
        result.metadata_bytes = result
            .metadata_bytes
            .checked_add(
                serde_json::to_vec(value)
                    .map_err(|_| invalid("comment metadata cannot be encoded"))?
                    .len(),
            )
            .filter(|n| *n <= MAX_COMMENT_METADATA)
            .ok_or_else(|| limit("comment metadata exceeds 256 KiB"))?;
        if !owned_parts.insert(value.part) {
            return Err(invalid("comment parts must be unique"));
        }
        let part = document
            .parts
            .get(&value.part)
            .filter(|part| part.kind == PartKind::Comment)
            .ok_or_else(|| invalid("comment text part is missing"))?;
        if part.blocks.len() != 1 || part.body != Default::default() {
            return Err(unsupported(
                "owned comment text requires one plain paragraph",
            ));
        }
        let p = part.blocks[0]
            .as_para()
            .ok_or_else(|| unsupported("owned comment text requires one plain paragraph"))?;
        charge(&mut work, part.blocks.len() + p.runs.len(), work_limit)?;
        validate_text(&p.text)?;
        if p.props != ParaProps::default()
            || p.mark != CharProps::default()
            || p.section.is_some()
            || !p.objects.is_empty()
            || p.runs.iter().any(|run| run.props != CharProps::default())
        {
            return Err(unsupported(
                "owned comment text requires plain text without formatting or anchors",
            ));
        }
        if let Some(parent) = value.parent {
            if parent == *id
                || document
                    .comments
                    .get(&parent)
                    .is_none_or(|root| root.parent.is_some())
            {
                return Err(unsupported(
                    "comment replies require an existing direct root",
                ));
            }
            let count = replies.entry(parent).or_default();
            *count += 1;
            if *count > MAX_REPLIES {
                return Err(limit("comment root exceeds 200 direct replies"));
            }
        }
        result.entries.insert(
            *id,
            CommentEntry {
                value: value.clone(),
                anchor: None,
                bytes: p.text.len(),
            },
        );
    }
    if document
        .parts
        .iter()
        .any(|(id, part)| part.kind == PartKind::Comment && !owned_parts.contains(id))
    {
        return Err(invalid("orphan comment text part"));
    }
    let mut pending = Vec::new();
    for (story, blocks) in std::iter::once((StoryRef::Body, &document.body)).chain(
        document
            .parts
            .iter()
            .map(|(id, part)| (StoryRef::Part(*id), &part.blocks)),
    ) {
        charge(&mut work, blocks.len(), work_limit)?;
        for (index, block) in blocks.iter().enumerate() {
            pending.push((story, Path::top(index), block.as_ref(), false));
        }
    }
    let mut ended = BTreeSet::new();
    while let Some((story, path, block, clipped)) = pending.pop() {
        if path.0.len() > 49 {
            return Err(limit("comment validation exceeds supported table nesting"));
        }
        match block {
            Block::Table(table) => {
                charge(&mut work, table.rows.len(), work_limit)?;
                for (ri, row) in table.rows.iter().enumerate() {
                    charge(&mut work, row.cells.len(), work_limit)?;
                    for (ci, cell) in row.cells.iter().enumerate() {
                        charge(&mut work, cell.blocks.len(), work_limit)?;
                        let clipped = clipped
                            || row.props.height_rule == HeightRule::Exact
                            || cell.props.text_direction != TextDirection::Horizontal;
                        for (bi, block) in cell.blocks.iter().enumerate() {
                            pending.push((
                                story,
                                Path(
                                    [path.0.as_slice(), &[ri as u32, ci as u32, bi as u32]]
                                        .concat(),
                                ),
                                block.as_ref(),
                                clipped,
                            ));
                        }
                    }
                }
            }
            Block::Para(p) => {
                charge(&mut work, p.objects.len(), work_limit)?;
                text_bytes = text_bytes
                    .checked_add(p.text.len())
                    .filter(|n| *n <= MAX_DOCUMENT_TEXT_BYTES)
                    .ok_or_else(|| {
                        limit("comment source validation exceeds 16 MiB of document text")
                    })?;
                // Streaming offsets avoid allocating an unchecked object_offsets vector.
                let mut found = 0usize;
                for (offset, _) in p
                    .text
                    .char_indices()
                    .filter(|(_, c)| *c == wordcraft_doc::para::OBJ)
                {
                    let object = p.objects.get(found).ok_or_else(|| {
                        invalid("inline object text does not match native objects")
                    })?;
                    found += 1;
                    let (id, start) = match object {
                        InlineObject::CommentStart { id } => (*id, true),
                        InlineObject::CommentEnd { id } => (*id, false),
                        _ => continue,
                    };
                    let entry = result
                        .entries
                        .get_mut(&id)
                        .ok_or_else(|| invalid("orphan comment anchor"))?;
                    if entry.value.parent.is_some() || story != StoryRef::Body || clipped {
                        return Err(unsupported("comment anchors require horizontal body paragraphs outside exact-height rows; replies have no anchors"));
                    }
                    if p.objects.len() != 2
                        || p.text.len() <= 6
                        || !matches!(p.objects.first(),Some(InlineObject::CommentStart{id:other}) if *other==id)
                        || !matches!(p.objects.last(),Some(InlineObject::CommentEnd{id:other}) if *other==id)
                    {
                        return Err(unsupported("one comment root requires the complete nonempty plain-text paragraph without other anchors or objects"));
                    }
                    if start {
                        if offset != 0 || entry.anchor.is_some() {
                            return Err(unsupported(
                                "comment roots require unique whole-paragraph endpoints",
                            ));
                        }
                        entry.anchor = Some(CommentAnchor {
                            path: path.clone(),
                            end: p.text.len() - 3,
                        });
                    } else if offset != p.text.len() - 3 || !ended.insert(id) {
                        return Err(unsupported(
                            "comment roots require unique whole-paragraph endpoints",
                        ));
                    }
                }
                if found != p.objects.len() {
                    return Err(invalid("inline object text does not match native objects"));
                }
            }
        }
    }
    for (id, entry) in &result.entries {
        if entry.value.parent.is_none() && (entry.anchor.is_none() || !ended.contains(id)) {
            return Err(invalid("comment root is missing its exact marker pair"));
        }
    }
    #[cfg(test)]
    {
        result.source_work = work;
    }
    Ok(result)
}

pub(super) fn registry(owner: &EditableDocx, proof: &CommentInventory) -> PyResult<()> {
    if owner.next_comment_id > MAX_COMMENT_ID + 1
        || proof.entries.keys().any(|id| *id >= owner.next_comment_id)
    {
        return Err(invalid("comment identity counter is inconsistent"));
    }
    if owner.locations.len() > MAX_BLOCKS {
        return Err(limit("document exceeds 100000 editable blocks"));
    }
    let parts: BTreeSet<_> = proof
        .entries
        .values()
        .map(|entry| entry.value.part)
        .collect();
    let images: BTreeSet<_> = owner.images.iter().copied().collect();
    let mut seen = BTreeSet::new();
    for (id, (story, path)) in &owner.locations {
        if let StoryRef::Part(part) = story {
            if parts.contains(part)
                && (path != &Path::top(0)
                    || !seen.insert(*part)
                    || images.contains(id)
                    || owner.table_generations.contains_key(id))
            {
                return Err(invalid("comment private paragraph locator is malformed"));
            }
        }
    }
    if seen != parts
        || owner.stories.keys().any(|id| parts.contains(id))
        || owner.story_placeholders.keys().any(|id| parts.contains(id))
        || owner
            .cell_placeholders
            .iter()
            .any(|(story, _)| matches!(story,StoryRef::Part(part) if parts.contains(part)))
    {
        return Err(invalid(
            "comment private paragraph ownership is inconsistent",
        ));
    }
    Ok(())
}
fn checked(owner: &EditableDocx) -> PyResult<CommentInventory> {
    let proof = inventory(owner.document()?)?;
    registry(owner, &proof)?;
    Ok(proof)
}

pub(super) fn ids(owner: &EditableDocx) -> PyResult<Vec<u32>> {
    Ok(checked(owner)?.entries.keys().copied().collect())
}
pub(super) fn info(owner: &EditableDocx, id: u32) -> PyResult<String> {
    let proof = checked(owner)?;
    let entry = proof.entry(id)?;
    let p = owner
        .document()?
        .para(StoryRef::Part(entry.value.part), &Path::top(0))
        .ok_or_else(|| invalid("comment paragraph is missing"))?;
    Ok(json!({"text":p.text,"author":entry.value.author,"date":entry.value.date,"resolved":entry.value.resolved,"parent":entry.value.parent}).to_string())
}
pub(super) fn range(owner: &EditableDocx, id: u32) -> PyResult<(usize, usize, usize, u64)> {
    let proof = checked(owner)?;
    let anchor = proof.anchor(id)?;
    let paragraph = owner
        .document()?
        .para(StoryRef::Body, &anchor.path)
        .ok_or_else(|| invalid("comment paragraph is missing"))?;
    let identifier = owner
        .locations
        .iter()
        .find_map(|(id, (story, path))| {
            (*story == StoryRef::Body && path == &anchor.path).then_some(*id)
        })
        .ok_or_else(|| invalid("comment paragraph has no owned locator"))?;
    Ok((
        identifier,
        1,
        paragraph.text[..anchor.end].chars().count(),
        owner.revision,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn add(
    owner: &mut EditableDocx,
    paragraph: Option<usize>,
    start: usize,
    end: usize,
    revision: u64,
    parent: Option<u32>,
    text: &str,
    author_text: &str,
    date_text: &str,
) -> PyResult<u32> {
    let before = checked(owner)?;
    validate_text(text)?;
    author(author_text)?;
    date(date_text)?;
    owner.reserve_block()?;
    if before.entries.len() >= MAX_COMMENTS {
        return Err(limit("document exceeds 1000 comments"));
    }
    let id = owner.next_comment_id;
    if id > MAX_COMMENT_ID {
        return Err(limit("owned comment identifier limit reached"));
    }
    let anchor = if let Some(parent) = parent {
        if paragraph.is_some() || before.entry(parent)?.value.parent.is_some() {
            return Err(unsupported("replies require an existing root comment"));
        }
        if before
            .entries
            .values()
            .filter(|entry| entry.value.parent == Some(parent))
            .count()
            >= MAX_REPLIES
        {
            return Err(limit("comment root exceeds 200 direct replies"));
        }
        None
    } else {
        owner.check_revision(revision)?;
        let paragraph =
            paragraph.ok_or_else(|| invalid("root comment requires a paragraph range"))?;
        let (story, path) = owner.location(paragraph)?;
        let p = owner.paragraph(paragraph)?;
        let (from, to) = range_offsets(&p.text, start, end)?;
        if story != StoryRef::Body
            || !p.objects.is_empty()
            || from != 0
            || to != p.text.len()
            || from == to
        {
            return Err(unsupported("comments require the whole nonempty body paragraph, without existing anchors or objects"));
        }
        Some((path, to))
    };
    let added = text
        .len()
        .checked_add(if anchor.is_some() { 6 } else { 0 })
        .ok_or_else(|| limit("comment text accounting overflow"))?;
    let text_bytes = owner.updated_text_bytes(0, added)?;
    let mut staged = owner.copy()?;
    let mut candidate = staged.document()?.clone();
    if candidate.parts.len() + 1 >= MAX_NATIVE_PARTS {
        return Err(limit("document exceeds the supported native part limit"));
    }
    let part = candidate
        .try_add_part(
            PartKind::Comment,
            vec![wordcraft_doc::para_block(Paragraph::with_text(
                text,
                CharProps::default(),
            ))],
        )
        .map_err(model_error)?;
    candidate.comments.insert(
        id,
        wordcraft_doc::Comment {
            author: author_text.into(),
            date: date_text.into(),
            parent,
            part,
            durable_id: Some(format!("{:08X}", id + 1)),
            ..Default::default()
        },
    );
    if let Some((path, to)) = anchor {
        candidate
            .insert_object(
                &Pos::new(StoryRef::Body, path.clone(), to),
                InlineObject::CommentEnd { id },
                &CharProps::default(),
            )
            .map_err(model_error)?;
        candidate
            .insert_object(
                &Pos::new(StoryRef::Body, path, 0),
                InlineObject::CommentStart { id },
                &CharProps::default(),
            )
            .map_err(model_error)?;
    }
    if staged
        .locations
        .values()
        .any(|(story, _)| *story == StoryRef::Part(part))
        || staged.stories.contains_key(&part)
        || staged.story_placeholders.contains_key(&part)
        || staged
            .cell_placeholders
            .iter()
            .any(|(story, _)| *story == StoryRef::Part(part))
    {
        return Err(invalid(
            "new native comment part collides with stale owned state",
        ));
    }
    staged.register_location(StoryRef::Part(part), Path::top(0), false)?;
    staged.next_comment_id = id + 1;
    staged.commit(candidate)?;
    staged.text_bytes = text_bytes;
    *owner = staged;
    Ok(id)
}

pub(super) fn update(owner: &mut EditableDocx, id: u32, patch: &str) -> PyResult<()> {
    let before = checked(owner)?;
    let entry = before.entry(id)?;
    if patch.len() > MAX_TEXT_BYTES * 6 + MAX_PROPERTY_BYTES {
        return Err(limit("comment patch exceeds its encoded text limit"));
    }
    let values: Map<String, Value> =
        serde_json::from_str(patch).map_err(|_| invalid("comment update requires an object"))?;
    if values.is_empty() {
        return Ok(());
    }
    let mut candidate = owner.document()?.clone();
    let mut text_bytes = owner.text_bytes;
    for (key, value) in values {
        match key.as_str() {
            "text" => {
                let text = value
                    .as_str()
                    .ok_or_else(|| invalid("comment text must be a string"))?;
                validate_text(text)?;
                text_bytes = owner.updated_text_bytes(entry.bytes, text.len())?;
                candidate
                    .set_story(
                        StoryRef::Part(entry.value.part),
                        vec![wordcraft_doc::para_block(Paragraph::with_text(
                            text,
                            CharProps::default(),
                        ))],
                    )
                    .map_err(model_error)?;
            }
            "author" => {
                let value = value
                    .as_str()
                    .ok_or_else(|| invalid("comment author must be a string"))?;
                author(value)?;
                candidate.comments.get_mut(&id).unwrap().author = value.into();
            }
            "date" => {
                let value = value
                    .as_str()
                    .ok_or_else(|| invalid("comment date must be a string"))?;
                date(value)?;
                candidate.comments.get_mut(&id).unwrap().date = value.into();
            }
            "resolved" => {
                candidate.comments.get_mut(&id).unwrap().resolved = value
                    .as_bool()
                    .ok_or_else(|| invalid("comment resolved must be a bool"))?;
            }
            _ => return Err(invalid("unsupported comment property")),
        }
    }
    owner.commit(candidate)?;
    owner.text_bytes = text_bytes;
    Ok(())
}

pub(super) struct CommentRemoval {
    pub id: u32,
    entry: CommentEntry,
}
impl CommentRemoval {
    pub fn marker(&self, key: &(u8, String)) -> bool {
        self.entry.anchor.is_some() && matches!(key.0, 4 | 5) && key.1 == self.id.to_string()
    }
    pub fn validate(&self, before: &Document, after: &Document) -> PyResult<()> {
        if before.comments.get(&self.id) != Some(&self.entry.value)
            || after.comments.contains_key(&self.id)
            || after.parts.contains_key(&self.entry.value.part)
            || before.parts.len() != after.parts.len() + 1
            || before.comments.len() != after.comments.len() + 1
            || before
                .comments
                .iter()
                .any(|(id, value)| *id != self.id && after.comments.get(id) != Some(value))
            || before.parts.iter().any(|(id, value)| {
                *id != self.entry.value.part && after.parts.get(id) != Some(value)
            })
        {
            return Err(invalid(
                "comment deletion must remove exactly its validated entry and part",
            ));
        }
        let old = semantic_anchors(before);
        let new = semantic_anchors(after);
        for kind in [4, 5] {
            let key = (kind, self.id.to_string());
            if old.get(&key)
                != (if self.entry.anchor.is_some() {
                    Some(&1)
                } else {
                    None
                })
                || new.contains_key(&key)
            {
                return Err(invalid(
                    "comment deletion must remove exactly its own validated marker pair",
                ));
            }
        }
        Ok(())
    }
}
pub(super) fn remove(owner: &mut EditableDocx, id: u32) -> PyResult<()> {
    let before = checked(owner)?;
    let entry = before.entry(id)?.clone();
    if before
        .entries
        .values()
        .any(|other| other.value.parent == Some(id))
    {
        return Err(unsupported(
            "delete direct replies before deleting their root comment",
        ));
    }
    let text_bytes =
        owner.updated_text_bytes(entry.bytes + if entry.anchor.is_some() { 6 } else { 0 }, 0)?;
    let mut staged = owner.copy()?;
    let mut candidate = staged.document()?.clone();
    if let Some(anchor) = &entry.anchor {
        for offset in [anchor.end, 0] {
            candidate
                .delete_range(
                    &Pos::new(StoryRef::Body, anchor.path.clone(), offset),
                    &Pos::new(StoryRef::Body, anchor.path.clone(), offset + 3),
                )
                .map_err(model_error)?;
        }
    }
    candidate.comments.remove(&id);
    candidate.parts.remove(&entry.value.part);
    staged
        .locations
        .retain(|_, (story, _)| *story != StoryRef::Part(entry.value.part));
    staged.commit_validated(candidate, None, None, Some(&CommentRemoval { id, entry }))?;
    staged.text_bytes = text_bytes;
    *owner = staged;
    Ok(())
}
