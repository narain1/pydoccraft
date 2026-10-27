//! Native table PAGE context regression and the owned-scope boundary.
use super::*;

fn pdf_texts(bytes: &[u8]) -> Vec<String> {
    let pdf = crate::docx::to_pdf(bytes, None).unwrap();
    crate::pdf::extract(&pdf, None)
        .unwrap()
        .pages
        .iter()
        .map(|page| {
            page.blocks
                .iter()
                .map(|block| {
                    block.table.as_ref().map_or_else(
                        || block.text.clone(),
                        |table| {
                            table
                                .rows
                                .iter()
                                .flatten()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" ")
                        },
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn table_page_error(error: PyErr) {
    assert!(
        error.to_string().contains("page fields in table cells"),
        "{error}"
    );
}

fn force_page(owner: &EditableDocx, document: &mut Document, paragraph: usize) {
    let InlineObject::Field { instr, .. } =
        &mut owner.paragraph_mut(document, paragraph).unwrap().objects[0]
    else {
        panic!("missing native field")
    };
    *instr = "PAGE \\* Arabic".into();
}

#[test]
fn upstream_table_page_uses_prior_paragraph_context_while_counts_and_direct_page_work() {
    pyo3::prepare_freethreaded_python();
    for nested in [false, true] {
        for synchronized in [false, true] {
            for kind in ["page", "page_count", "section_page_count"] {
                let mut owner = EditableDocx::new();
                owner.add_paragraph("FirstPage").unwrap();
                owner.add_page_break().unwrap();
                if synchronized {
                    owner.add_paragraph("SecondPagePrelude").unwrap();
                }
                let outer = owner
                    .add_table(vec![vec![if nested {
                        "Outer".into()
                    } else {
                        "Value ".into()
                    }]])
                    .unwrap();
                let table = if nested {
                    owner
                        .cell_add_table(outer, 0, 0, vec![vec!["Value ".into()]])
                        .unwrap()
                } else {
                    outer
                };
                let paragraph = owner.cell_paragraph_ids(table, 0, 0).unwrap()[0];
                // PAGE is injected through the public native model only to retain the
                // actual upstream defect. This is not an owned API acceptance fixture.
                owner
                    .add_paragraph_field(
                        paragraph,
                        if kind == "page" { "page_count" } else { kind },
                    )
                    .unwrap();
                let mut native = owner.document().unwrap().clone();
                if kind == "page" {
                    force_page(&owner, &mut native, paragraph);
                }
                let expected = if kind == "page" && !synchronized {
                    1
                } else {
                    2
                };
                for roundtrip in [false, true] {
                    let bytes = wordcraft_docx::write(&native).unwrap();
                    let current = if roundtrip {
                        wordcraft_docx::read(&bytes).unwrap()
                    } else {
                        native.clone()
                    };
                    let bytes = wordcraft_docx::write(&current).unwrap();
                    let texts = pdf_texts(&bytes);
                    assert_eq!(texts.len(), 2);
                    assert!(
                        texts[1]
                            .split_whitespace()
                            .collect::<String>()
                            .contains(&format!("Value{expected}")),
                        "{nested}/{synchronized}/{kind}: {texts:?}"
                    );
                    let layout = wordcraft_layout::layout(
                        &current,
                        &mut Default::default(),
                        &Default::default(),
                    );
                    assert_eq!(layout.pages[1].number, 2);
                    let shown: Vec<_> = layout.pages[1]
                        .items
                        .iter()
                        .filter_map(|item| {
                            if let wordcraft_layout::Placed::Lines { para, .. } = item {
                                Some(para.shown.iter().map(|(_, text, _)| text.as_str()))
                            } else {
                                None
                            }
                        })
                        .flatten()
                        .collect();
                    assert!(shown.contains(&expected.to_string().as_str()));
                    if kind == "page" && !synchronized {
                        assert!(!shown.contains(&"2"), "retain the incorrect native displayed value as a regression diagnostic");
                    }
                    if kind == "page" {
                        table_page_error(page_field_count(&current).unwrap_err());
                    } else {
                        page_field_count(&current).unwrap();
                    }
                }
            }
        }
    }
    let mut owner = EditableDocx::new();
    owner.add_paragraph("FirstPage").unwrap();
    owner.add_page_break().unwrap();
    let direct = owner.add_paragraph("DirectValue ").unwrap();
    owner.add_paragraph_field(direct, "page").unwrap();
    for kind in ["header", "footer"] {
        let part = owner.ensure_story(kind, "default", 0).unwrap();
        let p = owner
            .story_add_paragraph(part, &format!("{kind}Value "))
            .unwrap();
        assert_eq!(owner.location(p).unwrap().1 .0.len(), 1);
        owner.add_paragraph_field(p, "page").unwrap();
    }
    let anchor = owner.add_paragraph("NoteAnchor").unwrap();
    let note = owner
        .add_note(anchor, 0, 10, owner.revision, "NoteValue ", "footnote")
        .unwrap();
    let note_p = owner.story_paragraph_ids(note).unwrap()[0];
    assert_eq!(owner.location(note_p).unwrap().1 .0.len(), 1);
    owner.add_paragraph_field(note_p, "page").unwrap();
    let bytes = owner.to_bytes().unwrap();
    let texts = pdf_texts(&bytes);
    assert_eq!(texts.len(), 2);
    for value in [
        "DirectValue 2",
        "headerValue 2",
        "footerValue 2",
        "NoteValue 2",
    ] {
        assert!(texts[1].contains(value), "{texts:?}");
    }
    assert!(texts[0].contains("headerValue 1") && texts[0].contains("footerValue 1"));
}

fn owned_table(owner: &mut EditableDocx, scope: &str) -> usize {
    let rows = vec![
        vec!["Keep".into(), "Other".into()],
        vec!["Below".into(), "Last".into()],
    ];
    if scope == "body" {
        owner.add_table(rows).unwrap()
    } else {
        let part = owner.ensure_story(scope, "default", 0).unwrap();
        owner.story_add_table(part, rows).unwrap()
    }
}

#[test]
fn table_page_insertion_candidate_output_and_recovery_preserve_owned_state() {
    pyo3::prepare_freethreaded_python();
    for scope in ["body", "header", "footer"] {
        for merge in ["none", "horizontal", "vertical"] {
            for nested in [false, true] {
                let mut owner = EditableDocx::new();
                let outer = owned_table(&mut owner, scope);
                let alias = match merge {
                    "horizontal" => {
                        owner.merge_cells(outer, 0, 0, 0, 1).unwrap();
                        (0, 1)
                    }
                    "vertical" => {
                        owner.merge_cells(outer, 0, 0, 1, 0).unwrap();
                        (1, 0)
                    }
                    _ => (0, 0),
                };
                let (table, row, col) = if nested {
                    (
                        owner
                            .cell_add_table(outer, alias.0, alias.1, vec![vec!["Keep".into()]])
                            .unwrap(),
                        0,
                        0,
                    )
                } else {
                    (outer, alias.0, alias.1)
                };
                let paragraph = owner.cell_paragraph_ids(table, row, col).unwrap()[0];
                assert!(owner.location(paragraph).unwrap().1 .0.len() > 1);
                let bytes = owner.to_bytes().unwrap();
                let before = owner.copy().unwrap();
                assert!(owner
                    .add_paragraph_field(paragraph, "page")
                    .unwrap_err()
                    .to_string()
                    .contains("page fields in table cells"));
                assert_eq!(owner.to_bytes().unwrap(), bytes);
                assert_eq!(
                    (owner.text_bytes, owner.revision, owner.next_block_id),
                    (before.text_bytes, before.revision, before.next_block_id)
                );
                assert_eq!(owner.locations, before.locations);
                assert_eq!(owner.table_generations, before.table_generations);
                assert_eq!(owner.cell_placeholders, before.cell_placeholders);
                let (start, end, revision) =
                    owner.add_paragraph_field(paragraph, "page_count").unwrap();
                let valid = owner.to_bytes().unwrap();
                let mut invalid = owner.document().unwrap().clone();
                force_page(&owner, &mut invalid, paragraph);
                table_page_error(owner.commit(invalid.clone()).unwrap_err());
                assert_eq!(owner.to_bytes().unwrap(), valid);
                assert_eq!(owner.revision, revision);
                // Defend serialization/editing of older or privately constructed owned
                // states, while still permitting removal of the unsupported object.
                owner.document = Some(invalid);
                let invalid_bytes = wordcraft_docx::write(owner.document().unwrap()).unwrap();
                table_page_error(owner.to_bytes().unwrap_err());
                table_page_error(
                    owner
                        .format_range_patch(paragraph, 0, 4, revision, r#"{"bold":true}"#)
                        .unwrap_err(),
                );
                assert_eq!(
                    wordcraft_docx::write(owner.document().unwrap()).unwrap(),
                    invalid_bytes
                );
                assert_eq!(owner.revision, revision);
                owner
                    .replace_range(paragraph, start, end, revision, "")
                    .unwrap();
                owner.to_bytes().unwrap();
                owner.add_paragraph_field(paragraph, "page_count").unwrap();
                owner
                    .add_paragraph_field(paragraph, "section_page_count")
                    .unwrap();
                assert_eq!(page_field_count(owner.document().unwrap()).unwrap(), 2);
                owner.to_bytes().unwrap();
            }
        }
    }
}
