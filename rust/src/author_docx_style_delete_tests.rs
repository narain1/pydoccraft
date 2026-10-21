use super::*;
use wordcraft_doc::numbering::{AbstractNum, Level, Num};
use wordcraft_doc::props::PropChange;
use wordcraft_doc::styles::TableStyleParts;

fn definition(id: &str, kind: StyleKind) -> Style {
    Style {
        id: id.into(),
        name: format!("Definition {id}"),
        kind,
        ..Default::default()
    }
}
fn paragraph(text: &str) -> Arc<Block> {
    wordcraft_doc::para_block(Paragraph::with_text(text, CharProps::default()))
}
fn fresh_owner() -> EditableDocx {
    pyo3::prepare_freethreaded_python();
    let mut owner = EditableDocx::new();
    owner.add_paragraph("Reference carrier").unwrap();
    owner.add_style("Drop", "paragraph", None).unwrap();
    owner
}
fn unchanged(owner: &EditableDocx, before: &EditableDocx) {
    assert_eq!(owner.document().unwrap(), before.document().unwrap());
    assert_eq!(owner.retired_style_ids, before.retired_style_ids);
    assert_eq!(owner.revision, before.revision);
    assert_eq!(owner.locations, before.locations);
    assert_eq!(owner.stories, before.stories);
    assert_eq!(owner.cell_placeholders, before.cell_placeholders);
    assert_eq!(owner.table_generations, before.table_generations);
    assert_eq!(
        (
            owner.text_bytes,
            owner.cell_count,
            owner.media_bytes,
            owner.next_block_id
        ),
        (
            before.text_bytes,
            before.cell_count,
            before.media_bytes,
            before.next_block_id
        )
    );
}
fn referenced_case(label: &str, edit: impl FnOnce(&mut Document)) {
    let mut owner = fresh_owner();
    edit(owner.document.as_mut().unwrap());
    let before = owner.copy().unwrap();
    let error = owner.remove_style("Drop", owner.revision).unwrap_err();
    assert!(error.to_string().contains("referenced"), "{label}: {error}");
    unchanged(&owner, &before);
}

#[test]
fn unused_style_reference_preflight_all_typed_carriers() {
    for slot in ["paragraph", "mark", "run"] {
        referenced_case(slot, |d| {
            let p = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
            match slot {
                "paragraph" => p.props.style = Some("Drop".into()),
                "mark" => p.mark.style = Some("Drop".into()),
                _ => p.runs[0].props.style = Some("Drop".into()),
            }
        });
    }
    for kind in [
        PartKind::Header,
        PartKind::Footer,
        PartKind::Footnote,
        PartKind::Endnote,
        PartKind::Comment,
        PartKind::TextBox,
    ] {
        referenced_case(&format!("part {kind:?}"), |d| {
            let part = d.add_part(kind, vec![paragraph("Stored part")]);
            d.para_mut(StoryRef::Part(part), &Path::top(0))
                .unwrap()
                .props
                .style = Some("Drop".into());
        });
    }
    for endnote in [false, true] {
        for slot in 0..3 {
            referenced_case(&format!("separator {endnote}/{slot}"), |d| {
                let mut p = Paragraph::with_text("separator", Default::default());
                p.props.style = Some("Drop".into());
                let blocks = Some(vec![wordcraft_doc::para_block(p)]);
                let sep = if endnote {
                    &mut d.endnote_separators
                } else {
                    &mut d.footnote_separators
                };
                match slot {
                    0 => sep.separator = blocks,
                    1 => sep.continuation_separator = blocks,
                    _ => sep.continuation_notice = blocks,
                }
            });
        }
    }
    for slot in ["table", "row exception", "nested paragraph"] {
        referenced_case(slot, |d| {
            let mut table = Table::new(1, 1, 180.0);
            match slot {
                "table" => table.props.style = Some("Drop".into()),
                "row exception" => {
                    table.rows[0].props.exceptions =
                        Some(Box::new(wordcraft_doc::props::TableProps {
                            style: Some("Drop".into()),
                            ..Default::default()
                        }))
                }
                _ => {
                    let mut p = Paragraph::with_text("Nested", Default::default());
                    p.props.style = Some("Drop".into());
                    let mut nested = Table::new(1, 1, 120.0);
                    nested.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(p)];
                    table.rows[0].cells[0]
                        .blocks
                        .insert(0, Arc::new(Block::Table(nested)));
                }
            }
            d.body.insert(0, Arc::new(Block::Table(table)));
        });
    }
    for slot in [
        "default paragraph",
        "default character",
        "style paragraph",
        "style character",
    ] {
        referenced_case(slot, |d| {
            let mut s = definition("Holder", StyleKind::Paragraph);
            match slot {
                "default paragraph" => d.styles.default_para.style = Some("Drop".into()),
                "default character" => d.styles.default_chr.style = Some("Drop".into()),
                "style paragraph" => s.para.style = Some("Drop".into()),
                _ => s.chr.style = Some("Drop".into()),
            }
            d.styles.upsert(s);
        });
    }
    for slot in 0..6 {
        referenced_case(&format!("named conditional {slot}"), |d| {
            let mut t = TableStyleParts::default();
            let fields = [
                &mut t.header_chr,
                &mut t.band_chr,
                &mut t.first_col_chr,
                &mut t.total_chr,
                &mut t.last_col_chr,
                &mut t.col_band_chr,
            ];
            fields.into_iter().nth(slot).unwrap().style = Some("Drop".into());
            let mut s = definition("Holder", StyleKind::Table);
            s.table = Some(t);
            d.styles.upsert(s);
        });
    }
    for part in TablePart::ALL {
        referenced_case(&format!("raw region {part:?}"), |d| {
            let mut t = TableStyleParts::default();
            t.regions.insert(
                part,
                PartFormat {
                    chr: CharProps {
                        style: Some("Drop".into()),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            let mut s = definition("Holder", StyleKind::Table);
            s.table = Some(t);
            d.styles.upsert(s);
        });
    }
    for override_level in [false, true] {
        for character in [false, true] {
            referenced_case(&format!("numbering {override_level}/{character}"), |d| {
                let mut level = Level::default();
                if character {
                    level.chr.style = Some("Drop".into());
                } else {
                    level.style = Some("Drop".into());
                }
                if override_level {
                    d.numbering.nums.push(Num {
                        id: 1,
                        abstract_id: 1,
                        level_overrides: vec![(0, level)],
                        ..Default::default()
                    });
                } else {
                    d.numbering.abstracts.push(AbstractNum {
                        id: 1,
                        levels: vec![level],
                        ..Default::default()
                    });
                }
            });
        }
    }
    for edge in ["based_on", "next", "linked"] {
        referenced_case(edge, |d| {
            let mut style = definition("Holder", StyleKind::Paragraph);
            match edge {
                "based_on" => style.based_on = Some("Drop".into()),
                "next" => style.next = Some("Drop".into()),
                _ => style.linked = Some("Drop".into()),
            }
            d.styles.upsert(style);
        });
    }
}

#[test]
fn unused_style_identity_retirement_copy_and_lifetime_capacity() {
    let mut owner = fresh_owner();
    let before = owner.copy().unwrap();
    owner.remove_style("Drop", owner.revision).unwrap();
    assert_eq!(owner.revision, before.revision + 1);
    assert_eq!(owner.retired_style_ids, BTreeSet::from(["Drop".into()]));
    assert_eq!(
        (
            owner.text_bytes,
            owner.cell_count,
            owner.media_bytes,
            owner.next_block_id
        ),
        (
            before.text_bytes,
            before.cell_count,
            before.media_bytes,
            before.next_block_id
        )
    );
    let fresh = owner.add_style("Drop", "paragraph", None).unwrap();
    assert_eq!(fresh, "Drop1");
    assert_eq!(
        owner.style_info("Drop").unwrap(),
        owner.style_info_exact(&fresh).unwrap()
    );
    assert!(owner.style_info_exact("Drop").is_err());
    assert!(owner.style_formatting("Drop", false, None).is_err());
    assert!(owner.style_formatting("Drop", true, None).is_err());
    assert!(owner.format_style("Drop", "{}", None).is_err());
    assert!(owner.set_style_base("Drop", None).is_err());
    assert!(owner
        .style_relations_patch("Drop", r#"{"next":null}"#)
        .is_err());
    let mut copy = owner.copy().unwrap();
    copy.remove_style(&fresh, copy.revision).unwrap();
    assert_eq!(copy.add_style("Drop", "paragraph", None).unwrap(), "Drop2");
    assert!(owner.style_info_exact(&fresh).is_ok());
    assert_eq!(owner.retired_style_ids.len(), 1);
    copy.close();
    assert!(copy.retired_style_ids.is_empty());

    let mut full = fresh_owner();
    let document = full.document.as_mut().unwrap();
    let original_count = document.styles.styles.len();
    document.styles.styles.extend(
        (original_count..MAX_STYLES)
            .map(|i| definition(&format!("Capacity{i}"), StyleKind::Paragraph)),
    );
    storage(full.document().unwrap(), &full.retired_style_ids).unwrap();
    let before = full.copy().unwrap();
    assert!(full
        .add_style("Beyond", "paragraph", None)
        .unwrap_err()
        .to_string()
        .contains("lifetime"));
    unchanged(&full, &before);
    full.remove_style("Drop", full.revision).unwrap();
    assert_eq!(
        full.document().unwrap().styles.styles.len() + full.retired_style_ids.len(),
        MAX_STYLES
    );
    let before = full.copy().unwrap();
    assert!(full.add_style("Drop", "paragraph", None).is_err());
    unchanged(&full, &before);
    full.retired_style_ids.insert("Excess".into());
    assert!(storage(full.document().unwrap(), &full.retired_style_ids).is_err());
    assert!(full.copy().is_err());
}

#[test]
fn unused_style_opaque_history_and_protected_identity_refusals() {
    let mut owner = fresh_owner();
    for id in owner
        .style_ids(None)
        .unwrap()
        .into_iter()
        .filter(|id| id != "Drop")
    {
        let before = owner.copy().unwrap();
        assert!(owner.remove_style(&id, owner.revision).is_err());
        unchanged(&owner, &before);
    }
    owner
        .document
        .as_mut()
        .unwrap()
        .styles
        .get_mut("Normal")
        .unwrap()
        .builtin = false;
    owner
        .document
        .as_mut()
        .unwrap()
        .styles
        .get_mut("Normal")
        .unwrap()
        .default = false;
    assert!(owner.remove_style("Normal", owner.revision).is_err());
    owner
        .document
        .as_mut()
        .unwrap()
        .styles
        .get_mut("Drop")
        .unwrap()
        .default = true;
    assert!(owner.remove_style("Drop", owner.revision).is_err());
    owner
        .document
        .as_mut()
        .unwrap()
        .styles
        .get_mut("Drop")
        .unwrap()
        .default = false;
    // A valid target-internal next hint disappears with its definition.
    owner
        .style_relations_patch("Drop", r#"{"next":"Drop"}"#)
        .unwrap();
    owner.remove_style("Drop", owner.revision).unwrap();

    for mode in 0..13 {
        let mut owner = fresh_owner();
        let d = owner.document.as_mut().unwrap();
        match mode {
            0 => d.styles.latent_styles = Some("<opaque/>".into()),
            1 => {
                d.passthrough.insert("custom.xml".into(), Arc::new(vec![0]));
            }
            2 => d.settings.link_styles = true,
            3 => {
                d.styles.get_mut("Drop").unwrap().chr.fmt_change =
                    PropChange::boxed(0, CharProps::default())
            }
            4 => d.styles.default_para.fmt_change = PropChange::boxed(0, ParaProps::default()),
            5 => d.para_mut(StoryRef::Body, &Path::top(0)).unwrap().mark.ins = Some(0),
            6 => d.last_section.fmt_change = PropChange::boxed(0, SectionProps::default()),
            7 => d
                .para_mut(StoryRef::Body, &Path::top(0))
                .unwrap()
                .objects
                .push(InlineObject::Opaque {
                    format: "docx".into(),
                    xml: "<w:r/>".into(),
                    text: String::new(),
                }),
            8 => {
                let mut table = Table::new(1, 1, 100.0);
                table.rows[0].cells[0].props.fmt_change = PropChange::boxed(0, Default::default());
                d.body.insert(0, Arc::new(Block::Table(table)));
            }
            9 => {
                let mut table = Table::new(1, 1, 100.0);
                table.rows[0].props.fmt_change = PropChange::boxed(0, Default::default());
                d.body.insert(0, Arc::new(Block::Table(table)));
            }
            10 => {
                let mut table = Table::new(1, 1, 100.0);
                table.props.fmt_change = PropChange::boxed(0, Default::default());
                d.body.insert(0, Arc::new(Block::Table(table)));
            }
            11 => d
                .para_mut(StoryRef::Body, &Path::top(0))
                .unwrap()
                .objects
                .push(InlineObject::ControlEnd),
            _ => d
                .para_mut(StoryRef::Body, &Path::top(0))
                .unwrap()
                .objects
                .push(InlineObject::FieldStart {
                    instr: "STYLEREF Drop".into(),
                    locked: false,
                    dirty: false,
                }),
        }
        let before = owner.copy().unwrap();
        assert!(
            owner.remove_style("Drop", owner.revision).is_err(),
            "mode {mode}"
        );
        unchanged(&owner, &before);
    }
    let mut malformed = fresh_owner();
    let duplicate = malformed
        .document()
        .unwrap()
        .styles
        .get("Drop")
        .unwrap()
        .clone();
    malformed
        .document
        .as_mut()
        .unwrap()
        .styles
        .styles
        .push(duplicate);
    let before = malformed.document().unwrap().clone();
    assert!(malformed.remove_style("Drop", malformed.revision).is_err());
    assert_eq!(malformed.document().unwrap(), &before);
    assert!(malformed.retired_style_ids.is_empty());
}

#[test]
fn unused_style_exact_work_id_and_nesting_limits() {
    let mut owner = fresh_owner();
    let d = owner.document().unwrap();
    let actual = scan(d, "Drop", MAX_WORK).unwrap();
    assert_eq!(scan(d, "Drop", actual).unwrap(), actual);
    assert!(scan(d, "Drop", actual - 1).is_err());
    let id = "X".repeat(MAX_ID_BYTES);
    owner.retired_style_ids.insert(id.clone());
    storage(owner.document().unwrap(), &owner.retired_style_ids).unwrap();
    owner.retired_style_ids.remove(&id);
    owner.retired_style_ids.insert(format!("{id}X"));
    assert!(storage(owner.document().unwrap(), &owner.retired_style_ids).is_err());
    owner.retired_style_ids.clear();
    owner.retired_style_ids.insert("Drop".into());
    assert!(storage(owner.document().unwrap(), &owner.retired_style_ids).is_err());
    owner.retired_style_ids.clear();
    let table = vec![Arc::new(Block::Table(Table::new(1, 1, 100.0)))];
    let mut walk = Walk {
        target: "Drop",
        ignore_refs: false,
        work: 0,
        cap: MAX_WORK,
        blocks: 0,
        cells: MAX_CELLS - 1,
    };
    walk.blocks(&table, 0).unwrap();
    assert_eq!(walk.cells, MAX_CELLS);
    assert!(walk.blocks(&table, 0).is_err());
    let mut d = owner.document().unwrap().clone();
    d.body = vec![paragraph(""); MAX_BLOCKS];
    scan(&d, "Drop", MAX_WORK).unwrap();
    d.body.push(paragraph(""));
    assert!(scan(&d, "Drop", MAX_WORK).is_err());
    let mut blocks = vec![paragraph("Nested")];
    for depth in 1..=17 {
        let mut table = Table::new(1, 1, 100.0);
        table.rows[0].cells[0].blocks = blocks;
        blocks = vec![Arc::new(Block::Table(table))];
        let mut d = owner.document().unwrap().clone();
        d.body = blocks.clone();
        assert_eq!(scan(&d, "Drop", MAX_WORK).is_ok(), depth <= 16);
    }
}

#[test]
fn unused_style_transaction_revision_and_existing_semantic_guards() {
    let mut supported = fresh_owner();
    let paragraph = supported.add_paragraph("Supported fields ").unwrap();
    for kind in [
        "page",
        "page_count",
        "section_page_count",
        "title",
        "author",
    ] {
        supported.add_paragraph_field(paragraph, kind).unwrap();
    }
    supported.remove_style("Drop", supported.revision).unwrap();
    let mut owner = fresh_owner();
    let before = owner.copy().unwrap();
    assert!(owner.remove_style("Drop", owner.revision - 1).is_err());
    unchanged(&owner, &before);
    owner.revision = u64::MAX;
    let before = owner.copy().unwrap();
    assert!(owner.remove_style("Drop", owner.revision).is_err());
    unchanged(&owner, &before);
    owner.revision = u64::MAX - 1;
    owner.remove_style("Drop", owner.revision).unwrap();
    assert_eq!(owner.revision, u64::MAX);

    let mut owner = fresh_owner();
    let table = owner.add_table(vec![vec!["Count".into()]]).unwrap();
    let p = owner.cell_paragraph_ids(table, 0, 0).unwrap()[0];
    owner.add_paragraph_field(p, "page_count").unwrap();
    let (story, path) = owner.location(p).unwrap();
    let para = owner
        .document
        .as_mut()
        .unwrap()
        .para_mut(story, &path)
        .unwrap();
    let InlineObject::Field { instr, .. } = &mut para.objects[0] else {
        panic!("field fixture")
    };
    *instr = "PAGE \\* Arabic".into();
    let before = owner.copy().unwrap();
    let bytes = wordcraft_docx::write(owner.document().unwrap()).unwrap();
    assert!(
        owner.remove_style("Drop", owner.revision).is_err(),
        "ordinary table-PAGE guard remains active"
    );
    unchanged(&owner, &before);
    assert_eq!(
        wordcraft_docx::write(owner.document().unwrap()).unwrap(),
        bytes
    );
}
