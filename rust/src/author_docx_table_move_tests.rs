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

fn fixture() -> (EditableDocx, usize, usize) {
    let mut owner = EditableDocx::new();
    owner.add_paragraph("Keep").unwrap();
    let first = table(&mut owner, "body", "First");
    owner.add_paragraph("Between").unwrap();
    let last = table(&mut owner, "body", "Last");
    (owner, first, last)
}

#[test]
fn table_move_preserves_all_descendant_handles_grids_media_and_tail_identity() {
    pyo3::prepare_freethreaded_python();
    for scope in ["body", "header", "footer"] {
        for shape in ["direct", "nested", "horizontal", "vertical"] {
            let mut owner = EditableDocx::new();
            owner.add_paragraph("Body").unwrap();
            let outer = if shape == "direct" {
                None
            } else {
                let rows = vec![
                    vec!["Outer".into(), "Right".into()],
                    vec!["Below".into(), "Corner".into()],
                ];
                let outer = if scope == "body" {
                    owner.add_table(rows).unwrap()
                } else {
                    let part = owner.ensure_story(scope, "default", 0).unwrap();
                    owner.story_add_table(part, rows).unwrap()
                };
                if shape == "horizontal" {
                    owner.merge_cells(outer, 0, 0, 0, 1).unwrap();
                }
                if shape == "vertical" {
                    owner.merge_cells(outer, 0, 0, 1, 0).unwrap();
                }
                Some(outer)
            };
            let mut ids = Vec::new();
            for label in ["First", "Middle", "Last"] {
                let id = if let Some(outer) = outer {
                    owner
                        .cell_add_table(outer, 0, 0, vec![vec![label.into()]])
                        .unwrap()
                } else {
                    table(&mut owner, scope, label)
                };
                let p = owner.cell_paragraph_ids(id, 0, 0).unwrap()[0];
                owner.add_paragraph_field(p, "page_count").unwrap();
                let deep = owner
                    .cell_add_table(id, 0, 0, vec![vec!["Deep".into()]])
                    .unwrap();
                owner.cell_paragraph_ids(deep, 0, 0).unwrap();
                owner
                    .cell_add_image(deep, 0, 0, &png(), Some(8.0), Some(8.0), "Image")
                    .unwrap();
                owner.table_generations.insert(id, 7);
                ids.push(id);
                if let Some(outer) = outer {
                    owner.cell_add_paragraph(outer, 0, 0, "Between").unwrap();
                } else if scope == "body" {
                    owner.add_paragraph("Between").unwrap();
                } else {
                    let part = owner.ensure_story(scope, "default", 0).unwrap();
                    owner.story_add_paragraph(part, "Between").unwrap();
                }
            }
            let original = owner.copy().unwrap();
            let (story, source) = owner.location(ids[0]).unwrap();
            let target = owner.location(ids[2]).unwrap().1;
            let tail = owner
                .document()
                .unwrap()
                .container(story, &source)
                .unwrap()
                .last()
                .unwrap()
                .clone();
            let blocks: BTreeMap<_, _> = owner
                .locations
                .iter()
                .filter(|(_, (s, p))| {
                    !(*s == story && p.0.len() < source.0.len() && source.0.starts_with(&p.0))
                })
                .map(|(id, (story, path))| {
                    (
                        *id,
                        owner
                            .document()
                            .unwrap()
                            .block(*story, path)
                            .unwrap()
                            .clone(),
                    )
                })
                .collect();
            assert!(owner.move_table(ids[0], ids[2], owner.revision).unwrap());
            assert_eq!(owner.location(ids[0]).unwrap().1, target);
            assert!(Arc::ptr_eq(
                &tail,
                owner
                    .document()
                    .unwrap()
                    .container(story, &source)
                    .unwrap()
                    .last()
                    .unwrap()
            ));
            for (id, block) in blocks {
                let (story, path) = owner.location(id).unwrap();
                assert_eq!(
                    &block,
                    owner.document().unwrap().block(story, &path).unwrap()
                );
            }
            assert_eq!(owner.table_generations, original.table_generations);
            assert_eq!(owner.images, original.images);
            assert_eq!(owner.next_block_id, original.next_block_id);
            assert_eq!(
                (owner.text_bytes, owner.cell_count, owner.media_bytes),
                (
                    original.text_bytes,
                    original.cell_count,
                    original.media_bytes
                )
            );
            assert_eq!(
                owner.document().unwrap().media,
                original.document().unwrap().media
            );
            assert_eq!(owner.revision, original.revision + 1);
            assert!(owner.move_table(ids[0], ids[1], owner.revision).unwrap());
            // Final table ordinals are restored; intervening paragraphs retain their
            // relative order, not a promise of reversing every block position.
            let ordered = if let Some(outer) = outer {
                owner.cell_table_ids(outer, 0, 0).unwrap()
            } else if scope == "body" {
                owner.table_ids().unwrap()
            } else {
                let part = owner.ensure_story(scope, "default", 0).unwrap();
                owner.story_table_ids(part).unwrap()
            };
            assert_eq!(ordered, ids);
            assert_eq!(owner.table_generations, original.table_generations);
            owner.to_bytes().unwrap();
        }
    }
}

#[test]
fn table_move_protected_source_refuses_even_noop_and_retained_comments_shift() {
    pyo3::prepare_freethreaded_python();
    for kind in ["bookmark", "comment", "note", "section"] {
        let (mut owner, first, last) = fixture();
        let p = owner.cell_paragraph_ids(first, 0, 0).unwrap()[0];
        match kind {
            "bookmark" => owner.set_bookmark("Keep", p, 0, 5, owner.revision).unwrap(),
            "comment" => {
                owner
                    .add_comment(Some(p), 0, 5, owner.revision, None, "Review", "Ada", "")
                    .unwrap();
            }
            "note" => {
                owner
                    .add_note(p, 5, 5, owner.revision, "Note", "footnote")
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
        let before = owner.copy().unwrap();
        for destination in [first, last] {
            assert!(owner
                .move_table(first, destination, owner.revision)
                .unwrap_err()
                .to_string()
                .contains(if kind == "section" {
                    "section"
                } else {
                    "anchors"
                }));
            unchanged(&owner, &before);
        }
    }
    let (mut owner, first, last) = fixture();
    let p = owner.cell_paragraph_ids(last, 0, 0).unwrap()[0];
    let root = owner
        .add_comment(Some(p), 0, 4, owner.revision, None, "Review", "Ada", "")
        .unwrap();
    let reply = owner
        .add_comment(None, 0, 0, 0, Some(root), "Reply", "Ben", "")
        .unwrap();
    owner.move_table(first, last, owner.revision).unwrap();
    let (id, start, end, revision) = owner.comment_range(reply).unwrap();
    assert_eq!(id, p);
    assert_eq!(owner.range_text(id, start, end, revision).unwrap(), "Last");
    assert_eq!(owner.comment_ids().unwrap(), vec![root, reply]);
    owner.to_bytes().unwrap();
}

#[test]
fn table_move_scope_sections_and_shared_auxiliary_ownership_are_explicit() {
    pyo3::prepare_freethreaded_python();
    let (mut owner, first, last) = fixture();
    let nested = owner
        .cell_add_table(last, 0, 0, vec![vec!["Nested".into()]])
        .unwrap();
    let before = owner.copy().unwrap();
    assert!(owner
        .move_table(first, nested, owner.revision)
        .unwrap_err()
        .to_string()
        .contains("same owned container"));
    unchanged(&owner, &before);
    let header = owner.ensure_story("header", "default", 0).unwrap();
    let h = owner
        .story_add_table(header, vec![vec!["Header".into()]])
        .unwrap();
    let before = owner.copy().unwrap();
    assert!(owner.move_table(first, h, owner.revision).is_err());
    unchanged(&owner, &before);
    owner.add_section("next_page").unwrap();
    let second = table(&mut owner, "body", "Second section");
    let before = owner.copy().unwrap();
    assert!(owner
        .move_table(first, second, owner.revision)
        .unwrap_err()
        .to_string()
        .contains("section"));
    unchanged(&owner, &before);
    owner
        .document
        .as_mut()
        .unwrap()
        .last_section
        .footers
        .default = Some(header);
    let before = owner.copy().unwrap();
    assert!(owner
        .move_table(h, h, owner.revision)
        .unwrap_err()
        .to_string()
        .contains("independent"));
    unchanged(&owner, &before);
    let p = owner.paragraph_ids().unwrap()[0];
    assert!(owner.move_table(first, p, owner.revision).is_err());
    unchanged(&owner, &before);
}

#[test]
fn table_move_noop_revision_resource_and_source_work_boundaries_are_atomic() {
    pyo3::prepare_freethreaded_python();
    let (mut owner, first, last) = fixture();
    let work = comments::inventory(owner.document().unwrap())
        .unwrap()
        .source_work;
    assert!(comments::inventory_with_work_limit(owner.document().unwrap(), work).is_ok());
    assert!(comments::inventory_with_work_limit(owner.document().unwrap(), work - 1).is_err());
    let before = owner.copy().unwrap();
    assert!(!owner.move_table(first, first, owner.revision).unwrap());
    unchanged(&owner, &before);
    owner.next_block_id = usize::MAX;
    owner.text_bytes = MAX_DOCUMENT_TEXT_BYTES;
    owner.cell_count = MAX_CELLS;
    owner.revision = u64::MAX - 1;
    owner.move_table(first, last, owner.revision).unwrap();
    assert_eq!(owner.revision, u64::MAX);
    assert_eq!(owner.next_block_id, usize::MAX);
    assert_eq!(
        (owner.text_bytes, owner.cell_count),
        (MAX_DOCUMENT_TEXT_BYTES, MAX_CELLS)
    );
    let before = owner.copy().unwrap();
    assert!(!owner.move_table(first, first, owner.revision).unwrap());
    unchanged(&owner, &before);
    assert!(owner.move_table(first, last, owner.revision).is_err());
    unchanged(&owner, &before);
    assert!(owner.move_table(first, last, owner.revision - 1).is_err());
    unchanged(&owner, &before);
}

#[test]
fn table_move_malformed_associations_tails_and_table_page_never_repair_state() {
    pyo3::prepare_freethreaded_python();
    for case in 0..9 {
        let (mut owner, first, last) = fixture();
        let (story, path) = owner.location(first).unwrap();
        match case {
            0 => owner.images.push(usize::MAX),
            1 => {
                owner.table_generations.insert(usize::MAX, 1);
            }
            2 => {
                owner.locations.insert(usize::MAX, (story, path.clone()));
            }
            3 => {
                owner.cell_placeholders.insert((story, path.clone()));
            }
            4 => {
                owner.body_placeholder = Some(path.last());
            }
            5 => {
                owner.document.as_mut().unwrap().body.pop();
                owner.body_placeholder = None;
            }
            6 => {
                let p = owner.cell_paragraph_ids(first, 0, 0).unwrap()[0];
                owner.add_paragraph_field(p, "page_count").unwrap();
                let (s, p) = owner.location(p).unwrap();
                let InlineObject::Field { instr, .. } = &mut owner
                    .document
                    .as_mut()
                    .unwrap()
                    .para_mut(s, &p)
                    .unwrap()
                    .objects[0]
                else {
                    unreachable!()
                };
                *instr = "PAGE".into();
            }
            7 => {
                let image = owner
                    .add_image(&png(), Some(8.0), Some(8.0), "Keep")
                    .unwrap();
                owner.images.push(image);
            }
            8 => {
                let p = owner.add_paragraph("Other").unwrap();
                owner.table_generations.insert(p, 1);
            }
            _ => unreachable!(),
        }
        let before = owner.copy().unwrap();
        for target in [first, last] {
            assert!(
                owner.move_table(first, target, owner.revision).is_err(),
                "case{case}"
            );
            unchanged(&owner, &before);
        }
    }
}

#[test]
fn table_move_authored_and_required_empty_tails_keep_identity_and_reuse() {
    pyo3::prepare_freethreaded_python();
    for scope in ["body", "header", "footer"] {
        for authored in [false, true] {
            let mut owner = EditableDocx::new();
            owner.add_paragraph("Body").unwrap();
            let outer = table(&mut owner, scope, "Outer");
            let first = owner
                .cell_add_table(outer, 0, 0, vec![vec!["First".into()]])
                .unwrap();
            let last = owner
                .cell_add_table(outer, 0, 0, vec![vec!["Last".into()]])
                .unwrap();
            let blank = authored.then(|| owner.cell_add_paragraph(outer, 0, 0, "").unwrap());
            let (story, path) = owner.location(first).unwrap();
            let before = owner
                .document()
                .unwrap()
                .container(story, &path)
                .unwrap()
                .clone();
            owner.move_table(last, first, owner.revision).unwrap();
            let after = owner.document().unwrap().container(story, &path).unwrap();
            assert_eq!(before.len(), after.len());
            assert!(Arc::ptr_eq(before.last().unwrap(), after.last().unwrap()));
            if let Some(blank) = blank {
                assert_eq!(owner.paragraph_text(blank).unwrap(), "");
            }
            owner.cell_add_paragraph(outer, 0, 0, "Fresh").unwrap();
            assert_eq!(
                owner
                    .document()
                    .unwrap()
                    .container(story, &path)
                    .unwrap()
                    .len(),
                before.len() + usize::from(authored)
            );
        }
    }
}
