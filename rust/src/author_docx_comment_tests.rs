use super::comments::*;
use super::*;

fn root(owner: &mut EditableDocx, paragraph: usize, text: &str) -> u32 {
    let end = owner.paragraph(paragraph).unwrap().text.chars().count();
    owner
        .add_comment(
            Some(paragraph),
            0,
            end,
            owner.revision,
            None,
            text,
            "Ada",
            "2026-10-20T12:00:00Z",
        )
        .unwrap()
}
fn reply(owner: &mut EditableDocx, id: u32, text: &str) -> PyResult<u32> {
    owner.add_comment(None, 0, 0, 0, Some(id), text, "Ben", "")
}
fn fixture() -> (EditableDocx, usize, u32) {
    let mut owner = EditableDocx::new();
    let paragraph = owner.add_paragraph("Whole 😀 e\u{301} office سلام").unwrap();
    let id = root(&mut owner, paragraph, "Review");
    (owner, paragraph, id)
}
fn unchanged(owner: &EditableDocx, before: &EditableDocx) {
    assert_eq!(
        wordcraft_docx::write(owner.document().unwrap()).unwrap(),
        wordcraft_docx::write(before.document().unwrap()).unwrap()
    );
    assert_eq!(owner.revision, before.revision);
    assert_eq!(owner.text_bytes, before.text_bytes);
    assert_eq!(owner.locations, before.locations);
    assert_eq!(owner.stories, before.stories);
    assert_eq!(owner.next_block_id, before.next_block_id);
    assert_eq!(owner.next_comment_id, before.next_comment_id);
}

#[test]
fn owned_comments_crud_ranges_metadata_and_native_roundtrip() {
    let (mut owner, paragraph, id) = fixture();
    let child = reply(&mut owner, id, "").unwrap();
    let (p, a, b, r) = owner.comment_range(id).unwrap();
    assert_eq!(p, paragraph);
    assert_eq!(a, 1);
    assert_eq!(
        owner.range_text(p, a, b, r).unwrap(),
        "Whole 😀 e\u{301} office سلام"
    );
    assert_eq!(
        owner.comment_range(child).unwrap(),
        owner.comment_range(id).unwrap()
    );
    owner
        .update_comment(child, r#"{"text":"Reply\nSecond","resolved":true}"#)
        .unwrap();
    owner
        .update_comment(
            id,
            r#"{"text":"Changed","author":"Other","date":"","resolved":true}"#,
        )
        .unwrap();
    assert_eq!(owner.comment_ids().unwrap(), vec![id, child]);
    let read = wordcraft_docx::read(&owner.to_bytes().unwrap()).unwrap();
    assert_eq!(read.comments[&child].parent, Some(id));
    assert!(read.comments[&child].resolved);
    assert_eq!(read.comments[&id].author, "Other");
    assert!(read.comments[&id].date.is_empty());
    assert_eq!(read.comments[&id].durable_id.as_deref(), Some("00000001"));
    assert_eq!(
        read.plain_text(StoryRef::Part(read.comments[&child].part)),
        "Reply\nSecond"
    );
    let before = owner.copy().unwrap();
    assert!(owner.remove_comment(id).is_err());
    unchanged(&owner, &before);
    owner.remove_comment(child).unwrap();
    owner.remove_comment(id).unwrap();
    assert_eq!(
        owner.paragraph_text(paragraph).unwrap(),
        "Whole 😀 e\u{301} office سلام"
    );
    assert!(owner.comment_ids().unwrap().is_empty());
    assert!(owner.comment_info(child).is_err());
}

#[test]
fn owned_comments_private_locators_reuse_copy_and_counter_exhaustion() {
    let (mut owner, paragraph, id) = fixture();
    let part = owner.document().unwrap().comments[&id].part;
    let private = *owner
        .locations
        .iter()
        .find(|(_, loc)| loc.0 == StoryRef::Part(part))
        .unwrap()
        .0;
    assert_eq!(owner.paragraph_ids().unwrap(), vec![paragraph]);
    assert!(owner.note_ids(None).unwrap().is_empty());
    assert!(owner.story_paragraph_ids(part).is_err());
    assert!(owner.story_table_ids(part).is_err());
    let mut copied = owner.copy().unwrap();
    owner.remove_comment(id).unwrap();
    assert!(!owner.locations.contains_key(&private));
    let header = owner.ensure_story("header", "default", 0).unwrap();
    assert_eq!(header, part);
    let hp = owner.story_add_paragraph(header, "HeaderKeep").unwrap();
    let another = owner.add_paragraph("Note target").unwrap();
    let note = owner
        .add_note(another, 0, 4, owner.revision, "NoteKeep", "footnote")
        .unwrap();
    let new = root(&mut owner, paragraph, "");
    assert!(new > id);
    assert!(owner.comment_info(id).is_err());
    assert_eq!(owner.paragraph_text(hp).unwrap(), "HeaderKeep");
    assert!(owner.note_info(note).is_ok());
    copied
        .update_comment(id, r#"{"text":"Copy only"}"#)
        .unwrap();
    assert_ne!(
        copied.comment_info(id).unwrap(),
        owner.comment_info(new).unwrap()
    );
    let (mut last, p, _) = fixture();
    last.remove_comment(0).unwrap();
    last.next_comment_id = 0x7fff_fffd;
    assert_eq!(root(&mut last, p, ""), 0x7fff_fffd);
    last.remove_comment(0x7fff_fffd).unwrap();
    let before = last.copy().unwrap();
    assert!(last
        .add_comment(Some(p), 0, 22, last.revision, None, "", "Ada", "")
        .is_err());
    unchanged(&last, &before);
}

#[test]
fn owned_comments_endpoint_guards_survive_later_text_and_cell_changes() {
    let (mut owner, p, id) = fixture();
    let before = owner.copy().unwrap();
    assert!(owner.add_run_patch(p, "tail", "{}").is_err());
    unchanged(&owner, &before);
    assert!(owner
        .replace_range(p, 0, 0, owner.revision, "prefix")
        .is_err());
    unchanged(&owner, &before);
    let (_, a, b, r) = owner.comment_range(id).unwrap();
    assert!(owner.replace_range(p, a, b, r, "").is_err());
    unchanged(&owner, &before);
    owner.replace_range(p, a, b, r, "Updated 😀").unwrap();
    assert_eq!(owner.comment_range(id).unwrap().1, 1);
    let (_, a, b, r) = owner.comment_range(id).unwrap();
    assert_eq!(owner.range_text(p, a, b, r).unwrap(), "Updated 😀");
    let mut cells = EditableDocx::new();
    let table = cells.add_table(vec![vec!["Cell".into()]]).unwrap();
    let cell = cells.cell_paragraph_ids(table, 0, 0).unwrap()[0];
    root(&mut cells, cell, "");
    let before = cells.copy().unwrap();
    assert!(cells
        .cell_format_patch(table, 0, 0, r#"{"text_direction":"down"}"#)
        .is_err());
    unchanged(&cells, &before);
    assert!(cells.set_row_height(table, 0, Some(30.0), "exact").is_err());
    unchanged(&cells, &before);
    assert!(cells.remove_paragraph(cell).is_err());
    unchanged(&cells, &before);
}

#[test]
fn owned_comments_text_block_and_source_work_limits_are_atomic() {
    let (mut owner, _, id) = fixture();
    let text = "x".repeat(MAX_TEXT_BYTES);
    let child = reply(&mut owner, id, &text).unwrap();
    let before = owner.copy().unwrap();
    assert!(reply(&mut owner, id, &format!("{text}x")).is_err());
    unchanged(&owner, &before);
    owner.remove_comment(child).unwrap();
    owner.text_bytes = MAX_DOCUMENT_TEXT_BYTES - 1;
    reply(&mut owner, id, "x").unwrap();
    assert_eq!(owner.text_bytes, MAX_DOCUMENT_TEXT_BYTES);
    let before = owner.copy().unwrap();
    assert!(reply(&mut owner, id, "x").is_err());
    unchanged(&owner, &before);
    let (mut blocks, _, rootid) = fixture();
    for id in 0..MAX_BLOCKS - blocks.locations.len() - 1 {
        blocks
            .locations
            .insert(id, (StoryRef::Part(u32::MAX), Path::top(id)));
    }
    reply(&mut blocks, rootid, "").unwrap();
    assert_eq!(blocks.locations.len(), MAX_BLOCKS);
    let before = blocks.copy().unwrap();
    assert!(reply(&mut blocks, rootid, "").is_err());
    unchanged(&blocks, &before);
}

// Native fixture construction, intentionally avoiding O(n²) authoring setup for boundary tests.
fn install(owner: &mut EditableDocx, p: usize, parent: Option<u32>, author: &str) -> u32 {
    let id = owner.next_comment_id;
    let mut doc = owner.document().unwrap().clone();
    let part = doc
        .try_add_part(
            PartKind::Comment,
            vec![wordcraft_doc::para_block(Paragraph::new())],
        )
        .unwrap();
    doc.comments.insert(
        id,
        wordcraft_doc::Comment {
            author: author.into(),
            part,
            parent,
            durable_id: Some(format!("{:08X}", id + 1)),
            ..Default::default()
        },
    );
    if parent.is_none() {
        let path = owner.location(p).unwrap().1;
        let end = doc.para(StoryRef::Body, &path).unwrap().len();
        doc.insert_object(
            &Pos::new(StoryRef::Body, path.clone(), end),
            InlineObject::CommentEnd { id },
            &CharProps::default(),
        )
        .unwrap();
        doc.insert_object(
            &Pos::new(StoryRef::Body, path, 0),
            InlineObject::CommentStart { id },
            &CharProps::default(),
        )
        .unwrap();
        owner.text_bytes += 6;
    }
    owner.document = Some(doc);
    owner.next_comment_id += 1;
    owner
        .register_location(StoryRef::Part(part), Path::top(0), false)
        .unwrap();
    id
}
#[test]
fn owned_comments_count_reply_and_metadata_exact_plus_one_boundaries() {
    let mut owner = EditableDocx::new();
    for _ in 0..5 {
        let p = owner.add_paragraph("Target").unwrap();
        let root = install(&mut owner, p, None, "A");
        for _ in 0..199 {
            install(&mut owner, p, Some(root), "A");
        }
    }
    assert_eq!(owner.comment_ids().unwrap().len(), MAX_COMMENTS);
    let before = owner.copy().unwrap();
    assert!(reply(&mut owner, 0, "").is_err());
    unchanged(&owner, &before);
    let mut capped = EditableDocx::new();
    let p = capped.add_paragraph("Target").unwrap();
    let root = install(&mut capped, p, None, "A");
    for _ in 0..200 {
        install(&mut capped, p, Some(root), "A");
    }
    assert_eq!(capped.comment_ids().unwrap().len(), 201);
    let before = capped.copy().unwrap();
    assert!(reply(&mut capped, root, "").is_err());
    unchanged(&capped, &before);
    let p = capped.add_paragraph("Second").unwrap();
    let other = install(&mut capped, p, None, "A");
    for _ in 0..40 {
        install(&mut capped, p, Some(other), "A");
    }
    let used: usize = capped
        .document()
        .unwrap()
        .comments
        .values()
        .map(|c| serde_json::to_vec(c).unwrap().len())
        .sum();
    let mut remaining = 256 * 1024 - used;
    for value in capped.document.as_mut().unwrap().comments.values_mut() {
        let add = remaining.min(1023);
        value.author.push_str(&"A".repeat(add));
        remaining -= add;
    }
    assert_eq!(remaining, 0);
    assert_eq!(
        inventory(capped.document().unwrap())
            .unwrap()
            .metadata_bytes,
        256 * 1024
    );
    let (id, value) = capped
        .document()
        .unwrap()
        .comments
        .iter()
        .find(|(_, c)| c.author.len() < 1024)
        .unwrap();
    let id = *id;
    let patch = json!({"author":format!("{}B",value.author)}).to_string();
    let before = capped.copy().unwrap();
    assert!(capped.update_comment(id, &patch).is_err());
    unchanged(&capped, &before);
}

#[test]
fn owned_comments_malformed_models_and_private_state_fail_closed() {
    let (owner, paragraph, id) = fixture();
    let part = owner.document().unwrap().comments[&id].part;
    for case in 0..11 {
        let mut bad = owner.copy().unwrap();
        match case {
            0 => {
                bad.document
                    .as_mut()
                    .unwrap()
                    .comments
                    .get_mut(&id)
                    .unwrap()
                    .parent = Some(id)
            }
            1 => {
                bad.document
                    .as_mut()
                    .unwrap()
                    .comments
                    .get_mut(&id)
                    .unwrap()
                    .durable_id = None
            }
            2 => bad
                .document
                .as_mut()
                .unwrap()
                .comments
                .remove(&id)
                .map(|_| ())
                .unwrap(),
            3 => bad
                .locations
                .retain(|_, (story, _)| *story != StoryRef::Part(part)),
            4 => {
                bad.stories.insert(part, Vec::new());
            }
            5 => {
                bad.story_placeholders.insert(part, 0);
            }
            6 => {
                bad.cell_placeholders
                    .insert((StoryRef::Part(part), Path::top(0)));
            }
            7 => {
                bad.locations
                    .insert(0, (StoryRef::Part(part), Path::top(0)));
            }
            8 => bad.next_comment_id = 0,
            9 => {
                bad.document
                    .as_mut()
                    .unwrap()
                    .insert_text(&Pos::body(0, 0), "Prefix", &CharProps::default())
                    .unwrap();
            }
            _ => {
                bad.document
                    .as_mut()
                    .unwrap()
                    .para_mut(StoryRef::Part(part), &Path::top(0))
                    .unwrap()
                    .section = Some(Box::default());
            }
        }
        let revision = bad.revision;
        assert!(bad.to_bytes().is_err(), "case {case}");
        assert!(
            bad.update_comment(id, r#"{"resolved":true}"#).is_err(),
            "case {case}"
        );
        assert_eq!(bad.revision, revision);
    }
    let mut bad = owner.copy().unwrap();
    assert!(bad
        .set_bookmark("Bad", paragraph, 0, 1, bad.revision)
        .is_err());
}

#[test]
fn owned_comments_navigation_media_and_other_semantic_guards_remain_active() {
    let mut owner = EditableDocx::new();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0, 0, 255, 255].repeat(4))
            .unwrap();
    }
    owner
        .add_image(&png, Some(16.0), Some(16.0), "KeepImage")
        .unwrap();
    let media = owner.document().unwrap().media.clone();
    let target = owner.add_paragraph("Target").unwrap();
    owner
        .set_bookmark("Keep", target, 0, 6, owner.revision)
        .unwrap();
    let jump = owner.add_paragraph("Jump").unwrap();
    owner
        .set_range_link(jump, 0, 4, owner.revision, Some("#Keep"))
        .unwrap();
    let p = owner.add_paragraph("Comment").unwrap();
    let id = root(&mut owner, p, "Review");
    owner.update_comment(id, r#"{"text":"Changed"}"#).unwrap();
    assert_eq!(owner.document().unwrap().media, media);
    let mut malformed = owner.copy().unwrap();
    malformed
        .document
        .as_mut()
        .unwrap()
        .comments
        .get_mut(&id)
        .unwrap()
        .durable_id = None;
    let before = malformed.copy().unwrap();
    assert!(malformed.set_background(Some("red")).is_err());
    unchanged(&malformed, &before);
    let bytes = owner.to_bytes().unwrap();
    let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
    assert!(crate::pdf::extract(&pdf, None).unwrap().pages[0]
        .links
        .iter()
        .any(|l| l.page == Some(0)));
    let before = owner.copy().unwrap();
    assert!(owner.delete_bookmark("Keep").is_err());
    unchanged(&owner, &before);
    assert!(owner.remove_paragraph(p).is_err());
    unchanged(&owner, &before);
    owner.remove_comment(id).unwrap();
    assert_eq!(owner.document().unwrap().media, media);
    assert_eq!(owner.bookmark_names().unwrap(), vec!["Keep"]);
    let mut orphan = owner.document().unwrap().clone();
    orphan
        .insert_object(
            &Pos::new(StoryRef::Body, owner.location(p).unwrap().1, 0),
            InlineObject::CommentStart { id: 99 },
            &CharProps::default(),
        )
        .unwrap();
    assert!(validate_bookmark_links(&orphan).is_err());
    assert!(retain_authored_image_media(&mut orphan).is_err());
}

#[test]
fn owned_comments_source_work_and_revision_identifier_limits_preflight() {
    let (mut owner, _, id) = fixture();
    let proof = inventory(owner.document().unwrap()).unwrap();
    assert!(proof.source_work > 0);
    assert!(inventory_with_work_limit(owner.document().unwrap(), proof.source_work).is_ok());
    let before = owner.copy().unwrap();
    assert!(inventory_with_work_limit(owner.document().unwrap(), proof.source_work - 1).is_err());
    unchanged(&owner, &before);
    // Private runs are precharged before their canonical-format validation.
    let mut bad = owner.document().unwrap().clone();
    let part = bad.comments[&id].part;
    bad.para_mut(StoryRef::Part(part), &Path::top(0))
        .unwrap()
        .runs
        .push(wordcraft_doc::para::Run {
            len: 0,
            props: CharProps::default(),
        });
    assert!(inventory_with_work_limit(&bad, proof.source_work).is_err());
    owner.revision = u64::MAX - 1;
    owner.update_comment(id, r#"{"resolved":true}"#).unwrap();
    assert_eq!(owner.revision, u64::MAX);
    let before = owner.copy().unwrap();
    assert!(owner.update_comment(id, r#"{"resolved":false}"#).is_err());
    unchanged(&owner, &before);
    let (mut allocation, _, id) = fixture();
    allocation.next_block_id = usize::MAX - 1;
    reply(&mut allocation, id, "").unwrap();
    assert_eq!(allocation.next_block_id, usize::MAX);
    let before = allocation.copy().unwrap();
    assert!(reply(&mut allocation, id, "").is_err());
    unchanged(&allocation, &before);
    for bad in ["A\u{85}B", "A\u{2028}B", "A\u{2029}B"] {
        let before = allocation.copy().unwrap();
        assert!(allocation
            .update_comment(id, &json!({"author":bad}).to_string())
            .is_err());
        unchanged(&allocation, &before);
    }
}
