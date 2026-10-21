use super::*;

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255].repeat(4))
            .unwrap();
    }
    bytes
}
fn table(owner: &mut EditableDocx, scope: &str, label: &str) -> usize {
    if scope == "body" {
        owner.add_table(vec![vec![label.into()]]).unwrap()
    } else {
        let part = owner.ensure_story(scope, "default", 0).unwrap();
        owner
            .story_add_table(part, vec![vec![label.into()]])
            .unwrap()
    }
}
fn unchanged(actual: &EditableDocx, before: &EditableDocx) {
    assert_eq!(
        wordcraft_docx::write(actual.document().unwrap()).unwrap(),
        wordcraft_docx::write(before.document().unwrap()).unwrap()
    );
    assert_eq!(actual.locations, before.locations);
    assert_eq!(actual.images, before.images);
    assert_eq!(actual.stories, before.stories);
    assert_eq!(actual.story_placeholders, before.story_placeholders);
    assert_eq!(actual.body_placeholder, before.body_placeholder);
    assert_eq!(actual.cell_placeholders, before.cell_placeholders);
    assert_eq!(actual.table_generations, before.table_generations);
    assert_eq!(
        (
            actual.text_bytes,
            actual.cell_count,
            actual.media_bytes,
            actual.next_block_id,
            actual.next_comment_id,
            actual.revision
        ),
        (
            before.text_bytes,
            before.cell_count,
            before.media_bytes,
            before.next_block_id,
            before.next_comment_id,
            before.revision
        )
    );
}
fn fixture() -> (EditableDocx, usize) {
    let mut owner = EditableDocx::new();
    owner.add_paragraph("Keep").unwrap();
    let id = table(&mut owner, "body", "Drop");
    (owner, id)
}
#[test]
fn table_delete_retires_only_target_subtree_and_preserves_shifted_handles() {
    pyo3::prepare_freethreaded_python();
    for scope in ["body", "header", "footer"] {
        for nested in [false, true] {
            let mut owner = EditableDocx::new();
            owner.set_margins(180.0, 36.0, 180.0, 36.0).unwrap();
            owner.add_paragraph("BodyKeep").unwrap();
            let parent = nested.then(|| table(&mut owner, scope, "OuterKeep"));
            let target = if let Some(parent) = parent {
                owner
                    .cell_add_table(parent, 0, 0, vec![vec!["Drop".into()]])
                    .unwrap()
            } else {
                table(&mut owner, scope, "Drop")
            };
            let removed_p = owner.cell_paragraph_ids(target, 0, 0).unwrap()[0];
            let removed_nested = owner
                .cell_add_table(target, 0, 0, vec![vec!["DeepDrop".into()]])
                .unwrap();
            let removed_deep = owner.cell_paragraph_ids(removed_nested, 0, 0).unwrap()[0];
            let image = owner
                .cell_add_image(
                    removed_nested,
                    0,
                    0,
                    &png(),
                    Some(12.0),
                    Some(12.0),
                    "DropImage",
                )
                .unwrap();
            owner.table_generations.insert(removed_nested, 7);
            let kept = if let Some(parent) = parent {
                owner
                    .cell_add_table(parent, 0, 0, vec![vec!["Keep".into()]])
                    .unwrap()
            } else {
                table(&mut owner, scope, "Keep")
            };
            let kept_p = owner.cell_paragraph_ids(kept, 0, 0).unwrap()[0];
            let kept_nested = owner
                .cell_add_table(kept, 0, 0, vec![vec!["DeepKeep".into()]])
                .unwrap();
            let kept_deep = owner.cell_paragraph_ids(kept_nested, 0, 0).unwrap()[0];
            let kept_image = owner
                .cell_add_image(
                    kept_nested,
                    0,
                    0,
                    &png(),
                    Some(12.0),
                    Some(12.0),
                    "SharedKeep",
                )
                .unwrap();
            owner.table_generations.insert(kept, 9);
            let before = owner.copy().unwrap();
            let usage = table_usage(owner.table(target).unwrap());
            let media = owner.media_bytes;
            let alloc = owner.next_block_id;
            owner.remove_table(target).unwrap();
            for removed in [target, removed_p, removed_nested, removed_deep, image] {
                assert!(owner.location(removed).is_err());
            }
            assert_eq!(owner.paragraph_text(kept_p).unwrap(), "Keep");
            assert_eq!(owner.paragraph_text(kept_deep).unwrap(), "DeepKeep");
            assert_eq!(owner.image_data(kept_image).unwrap(), png());
            assert_eq!(owner.table_generation(kept).unwrap(), 9);
            if let Some(parent) = parent {
                assert_eq!(owner.table_generation(parent).unwrap(), 0);
            }
            assert_eq!(owner.next_block_id, alloc);
            assert_eq!(
                (owner.text_bytes + usage.0, owner.cell_count + usage.1),
                (before.text_bytes, before.cell_count)
            );
            assert_eq!(
                owner.media_bytes, media,
                "shared picture remains referenced"
            );
            assert!(before.table(target).is_ok());
            owner.to_bytes().unwrap();
        }
    }
}
#[test]
fn table_delete_retains_hidden_and_authored_blanks_then_reuses_capacity() {
    pyo3::prepare_freethreaded_python();
    for scope in ["body", "header", "footer"] {
        for nested in [false, true] {
            for authored in [false, true] {
                let mut owner = EditableDocx::new();
                owner.add_paragraph("BodyKeep").unwrap();
                let parent = nested.then(|| table(&mut owner, scope, "OuterKeep"));
                let target = if let Some(parent) = parent {
                    owner
                        .cell_add_table(parent, 0, 0, vec![vec!["Drop".into()]])
                        .unwrap()
                } else {
                    table(&mut owner, scope, "Drop")
                };
                let (story, path) = owner.location(target).unwrap();
                let empty = authored.then(|| {
                    if let Some(parent) = parent {
                        owner.cell_add_paragraph(parent, 0, 0, "").unwrap()
                    } else if let StoryRef::Part(part) = story {
                        owner.story_add_paragraph(part, "").unwrap()
                    } else {
                        owner.add_paragraph("").unwrap()
                    }
                });
                let native_tail = owner
                    .document()
                    .unwrap()
                    .container(story, &path)
                    .unwrap()
                    .last()
                    .unwrap()
                    .clone();
                owner.remove_table(target).unwrap();
                let remaining = owner.document().unwrap().container(story, &path).unwrap();
                assert!(Arc::ptr_eq(&native_tail, remaining.last().unwrap()));
                if let Some(empty) = empty {
                    assert_eq!(owner.paragraph_text(empty).unwrap(), "");
                }
                let before_len = remaining.len();
                let new = if let Some(parent) = parent {
                    owner.cell_add_paragraph(parent, 0, 0, "New").unwrap()
                } else if let StoryRef::Part(part) = story {
                    owner.story_add_paragraph(part, "New").unwrap()
                } else {
                    owner.add_paragraph("New").unwrap()
                };
                let new_path = owner.location(new).unwrap().1;
                assert_eq!(
                    owner
                        .document()
                        .unwrap()
                        .container(story, &new_path)
                        .unwrap()
                        .len(),
                    before_len + usize::from(authored)
                );
                assert!(new > target);
                assert_eq!(owner.paragraph_text(new).unwrap(), "New");
            }
        }
    }
}
#[test]
fn table_delete_rejects_anchors_sections_and_aliases_then_preserves_later_comments() {
    pyo3::prepare_freethreaded_python();
    for kind in ["bookmark", "note", "comment", "section", "shared"] {
        let (mut owner, mut target) = fixture();
        if kind == "shared" {
            target = table(&mut owner, "header", "Drop");
            let part = owner.location(target).unwrap().0;
            let StoryRef::Part(part) = part else {
                unreachable!()
            };
            owner
                .document
                .as_mut()
                .unwrap()
                .last_section
                .footers
                .default = Some(part);
        } else {
            let p = owner.cell_paragraph_ids(target, 0, 0).unwrap()[0];
            match kind {
                "bookmark" => owner
                    .set_bookmark("Protected", p, 0, 4, owner.revision)
                    .unwrap(),
                "note" => {
                    owner
                        .add_note(p, 0, 0, owner.revision, "Detail", "footnote")
                        .unwrap();
                }
                "comment" => {
                    owner
                        .add_comment(Some(p), 0, 4, owner.revision, None, "Review", "Ada", "")
                        .unwrap();
                }
                "section" => {
                    let section = owner.document().unwrap().last_section.clone();
                    let (story, path) = owner.location(p).unwrap();
                    owner
                        .document
                        .as_mut()
                        .unwrap()
                        .para_mut(story, &path)
                        .unwrap()
                        .section = Some(Box::new(section));
                }
                _ => unreachable!(),
            }
        }
        let before = owner.copy().unwrap();
        assert!(owner.remove_table(target).is_err(), "{kind}");
        unchanged(&owner, &before);
    }
    let (mut owner, target) = fixture();
    let p = owner.add_paragraph("Later 😀").unwrap();
    let root = owner
        .add_comment(Some(p), 0, 7, owner.revision, None, "Keep", "Ada", "")
        .unwrap();
    let reply = owner
        .add_comment(None, 0, 0, 0, Some(root), "Reply", "Ben", "")
        .unwrap();
    owner.remove_table(target).unwrap();
    let (id, a, b, revision) = owner.comment_range(reply).unwrap();
    assert_eq!(id, p);
    assert_eq!(owner.range_text(p, a, b, revision).unwrap(), "Later 😀");
    let target = table(&mut owner, "body", "Commented");
    let p = owner.cell_paragraph_ids(target, 0, 0).unwrap()[0];
    let root = owner
        .add_comment(Some(p), 0, 9, owner.revision, None, "Remove", "Ada", "")
        .unwrap();
    owner.remove_comment(root).unwrap();
    owner.remove_table(target).unwrap();
    assert_eq!(owner.comment_ids().unwrap().len(), 2);
}
#[test]
fn table_delete_malformed_registry_is_not_silently_cleaned() {
    pyo3::prepare_freethreaded_python();
    for case in 0..14 {
        let (mut owner, target) = fixture();
        let (story, path) = owner.location(target).unwrap();
        let paragraph = owner.paragraph_ids().unwrap()[0];
        match case {
            0 => {
                owner.images.push(usize::MAX);
            }
            1 => {
                owner.table_generations.insert(usize::MAX, 1);
            }
            2 => {
                owner.table_generations.insert(paragraph, 1);
            }
            3 => {
                owner.images.push(paragraph);
            }
            4 => {
                owner.locations.insert(usize::MAX, (story, path.clone()));
            }
            5 => {
                owner
                    .locations
                    .insert(usize::MAX, (story, Path(vec![path.0[0], 99, 0, 0])));
            }
            6 => {
                owner.cell_placeholders.insert((story, path.clone()));
            }
            7 => {
                owner.body_placeholder = Some(path.last());
            }
            8 => {
                owner
                    .cell_placeholders
                    .insert((StoryRef::Part(u32::MAX), Path(vec![0, 0, 0, 0])));
            }
            9 => {
                owner.locations.get_mut(&target).unwrap().1 = Path(vec![]);
            }
            10 => {
                let header = owner.ensure_story("header", "default", 0).unwrap();
                let table = owner
                    .story_add_table(header, vec![vec!["Other".into()]])
                    .unwrap();
                owner.images.push(table); // unrelated story association cannot be silently pruned
            }
            11 => {
                let image = owner
                    .add_image(&png(), Some(12.0), Some(12.0), "Unrelated")
                    .unwrap();
                owner.images.push(image);
            }
            12 => {
                let image = owner
                    .add_image(&png(), Some(12.0), Some(12.0), "Unrelated")
                    .unwrap();
                owner.locations.remove(&image);
            }
            13 => {
                let header = owner.ensure_story("header", "default", 0).unwrap();
                let p = owner.story_add_paragraph(header, "Other").unwrap();
                owner.table_generations.insert(p, 3);
            }
            _ => unreachable!(),
        }
        let before = owner.copy().unwrap();
        assert!(owner.remove_table(target).is_err(), "case{case}");
        unchanged(&owner, &before);
    }
    for scope in ["header", "footer"] {
        for nested in [false, true] {
            let mut owner = EditableDocx::new();
            owner.add_paragraph("Keep").unwrap();
            let outer = table(&mut owner, scope, "Outer");
            let target = if nested {
                owner
                    .cell_add_table(outer, 0, 0, vec![vec!["Drop".into()]])
                    .unwrap()
            } else {
                outer
            };
            let (story, path) = owner.location(target).unwrap();
            owner
                .document
                .as_mut()
                .unwrap()
                .container_mut(story, &path)
                .unwrap()
                .pop();
            // Retire only the removed generated-tail bookkeeping in this malformed control.
            if nested {
                owner
                    .cell_placeholders
                    .retain(|(s, p)| *s != story || p.parent() != path.parent());
            } else if let StoryRef::Part(part) = story {
                owner.story_placeholders.remove(&part);
            }
            let before = owner.copy().unwrap();
            assert!(owner.remove_table(target).is_err());
            unchanged(&owner, &before);
        }
    }
}
#[test]
fn table_delete_accounting_revision_and_allocator_boundaries_are_atomic() {
    pyo3::prepare_freethreaded_python();
    let (mut owner, target) = fixture();
    let usage = table_usage(owner.table(target).unwrap());
    owner.text_bytes = MAX_DOCUMENT_TEXT_BYTES;
    owner.cell_count = MAX_CELLS;
    owner.next_block_id = usize::MAX;
    owner.revision = u64::MAX - 1;
    owner.remove_table(target).unwrap();
    assert_eq!(owner.text_bytes, MAX_DOCUMENT_TEXT_BYTES - usage.0);
    assert_eq!(owner.cell_count, MAX_CELLS - usage.1);
    assert_eq!(
        owner.next_block_id,
        usize::MAX,
        "deletion consumes no fresh handle"
    );
    assert_eq!(owner.revision, u64::MAX);
    for underflow in [true, false] {
        let (mut owner, target) = fixture();
        if underflow {
            owner.cell_count = 0;
        } else {
            owner.text_bytes = 0;
        }
        let before = owner.copy().unwrap();
        assert!(owner.remove_table(target).is_err());
        unchanged(&owner, &before);
    }
    let (mut owner, target) = fixture();
    owner.revision = u64::MAX;
    let before = owner.copy().unwrap();
    assert!(owner.remove_table(target).is_err());
    unchanged(&owner, &before);
    let (mut owner, target) = fixture();
    owner.cell_count = MAX_CELLS;
    let before = owner.copy().unwrap();
    assert!(owner.add_table(vec![vec![String::new()]]).is_err());
    unchanged(&owner, &before);
    owner.remove_table(target).unwrap();
    owner.add_table(vec![vec![String::new()]]).unwrap();
    assert_eq!(owner.cell_count, MAX_CELLS);
}
#[test]
fn table_delete_source_preflight_and_unique_media_reclamation() {
    pyo3::prepare_freethreaded_python();
    let (mut owner, target) = fixture();
    owner
        .cell_add_image(target, 0, 0, &png(), Some(12.0), Some(12.0), "Unique")
        .unwrap();
    assert!(owner.media_bytes > 0);
    owner.remove_table(target).unwrap();
    assert_eq!(owner.media_bytes, 0);
    assert!(owner.document().unwrap().media.is_empty());
    let (mut owner, target) = fixture();
    let p = owner.paragraph_ids().unwrap()[0];
    let (story, path) = owner.location(p).unwrap();
    owner
        .document
        .as_mut()
        .unwrap()
        .para_mut(story, &path)
        .unwrap()
        .text = "\u{fffc}".into();
    let body = owner.document().unwrap().body.clone();
    let revision = owner.revision;
    assert!(owner.remove_table(target).is_err());
    assert_eq!(owner.document().unwrap().body, body);
    assert_eq!(owner.revision, revision);
    let (mut owner, target) = fixture();
    let work = comments::inventory(owner.document().unwrap())
        .unwrap()
        .source_work;
    assert!(comments::inventory_with_work_limit(owner.document().unwrap(), work).is_ok());
    assert!(comments::inventory_with_work_limit(owner.document().unwrap(), work - 1).is_err());
    owner.remove_table(target).unwrap();
}
