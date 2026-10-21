//! Spec110 raw native premises only; no style-deletion adapter or public API.
use super::*;
use std::io::Read;
use wordcraft_doc::numbering::{AbstractNum, Level, Num};
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
fn members(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            (file.name().to_owned(), data)
        })
        .collect()
}
fn marker() -> Vec<u8> {
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
fn red_pixels(bytes: &[u8]) -> usize {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut data = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut data).unwrap();
    let stride = match frame.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("unexpected color {other:?}"),
    };
    data[..frame.buffer_size()]
        .chunks_exact(stride)
        .filter(|p| p[0] > 240 && p[1] < 15 && p[2] < 15)
        .count()
}
fn referenced_case(label: &str, edit: impl FnOnce(&mut Document)) {
    let mut d = Document::from_text("Reference carrier");
    d.styles.upsert(definition("Drop", StyleKind::Paragraph));
    edit(&mut d);
    // Test-only serialization proves the deliberately injected ID occurs once outside
    // the target. It is not the proposed production reference detector.
    assert_eq!(
        serde_json::to_string(&d)
            .unwrap()
            .matches("\"Drop\"")
            .count(),
        2,
        "{label}"
    );
    let before = d.clone();
    d.styles.remove("Drop").unwrap();
    assert_eq!(
        serde_json::to_string(&d)
            .unwrap()
            .matches("\"Drop\"")
            .count(),
        1,
        "native leaves dangling {label}"
    );
    d.styles = before.styles.clone();
    assert_eq!(d, before, "native remove does not repair carrier {label}");
}

#[test]
fn unused_style_raw_removal_preservation_and_reference_carriers() {
    pyo3::prepare_freethreaded_python();
    let mut owner = EditableDocx::new();
    owner.set_page_size(360.0, 480.0).unwrap();
    owner.set_margins(48.0, 36.0, 48.0, 36.0).unwrap();
    let used_p = owner
        .add_style("Used Paragraph", "paragraph", Some("Normal"))
        .unwrap();
    let used_c = owner
        .add_style("Used Character", "character", None)
        .unwrap();
    let used_t = owner
        .add_style("Used Table", "table", Some("TableGrid"))
        .unwrap();
    owner
        .format_style(&used_p, r#"{"space_after":4}"#, None)
        .unwrap();
    owner
        .format_style(&used_c, r##"{"color":"#0000FF"}"##, None)
        .unwrap();
    owner
        .format_style(&used_t, r##"{"background":"#E0FFE0"}"##, Some("first_row"))
        .unwrap();
    let body = owner.add_paragraph("BodyKeep").unwrap();
    owner.set_paragraph_style(body, &used_p).unwrap();
    owner
        .format_range_patch(
            body,
            0,
            8,
            owner.revision,
            &format!(r#"{{"style":"{used_c}"}}"#),
        )
        .unwrap();
    let link = owner.add_paragraph("LinkKeep").unwrap();
    owner
        .set_range_link(
            link,
            0,
            8,
            owner.revision,
            Some("https://example.com/style-proof"),
        )
        .unwrap();
    let numbered = owner.add_paragraph("NumberedKeep").unwrap();
    owner
        .set_paragraph_list(numbered, Some("numbered"), 0, false, None)
        .unwrap();
    let table = owner.add_table(vec![vec!["TableKeep".into()]]).unwrap();
    owner
        .table_format_patch(table, &format!(r#"{{"style":"{used_t}"}}"#))
        .unwrap();
    owner
        .cell_add_table(table, 0, 0, vec![vec!["NestedKeep".into()]])
        .unwrap();
    owner
        .cell_add_image(table, 0, 0, &marker(), Some(10.0), Some(10.0), "MarkerKeep")
        .unwrap();
    for kind in ["header", "footer"] {
        let part = owner.ensure_story(kind, "default", 0).unwrap();
        let p = owner
            .story_add_paragraph(part, &format!("{kind}Keep"))
            .unwrap();
        owner.set_paragraph_style(p, &used_p).unwrap();
    }
    let note_anchor = owner.add_paragraph("NoteAnchor").unwrap();
    owner
        .add_note(note_anchor, 10, 10, owner.revision, "NoteKeep", "footnote")
        .unwrap();
    let comment_anchor = owner.add_paragraph("CommentAnchor").unwrap();
    owner
        .add_comment(
            Some(comment_anchor),
            0,
            13,
            owner.revision,
            None,
            "CommentKeep",
            "Ada",
            "",
        )
        .unwrap();
    let drops: Vec<_> = ["paragraph", "character", "table"]
        .into_iter()
        .map(|kind| {
            owner
                .add_style(&format!("Unused {kind}"), kind, None)
                .unwrap()
        })
        .collect();
    let original = owner.document().unwrap().clone();
    let mut removed = original.clone();
    for id in &drops {
        assert_eq!(removed.styles.remove(id).unwrap().id, *id);
    }
    let mut expected = original.clone();
    expected.styles.styles.retain(|s| !drops.contains(&s.id));
    assert_eq!(
        removed, expected,
        "all native fields except target definitions stay identical"
    );
    for (a, b) in original.body.iter().zip(&removed.body) {
        assert!(Arc::ptr_eq(a, b));
    }
    assert_eq!(original.body.len(), removed.body.len());
    let before = wordcraft_docx::write(&original).unwrap();
    let after = wordcraft_docx::write(&removed).unwrap();
    let mut before_members = members(&before);
    let mut after_members = members(&after);
    let old_styles = before_members.remove("word/styles.xml").unwrap();
    let new_styles = after_members.remove("word/styles.xml").unwrap();
    assert_eq!(
        before_members, after_members,
        "all unrelated package members are exact"
    );
    assert_ne!(old_styles, new_styles);
    let old_model = wordcraft_docx::read(&before).unwrap();
    let new_model = wordcraft_docx::read(&after).unwrap();
    let mut expected_read = old_model.clone();
    expected_read
        .styles
        .styles
        .retain(|s| !drops.contains(&s.id));
    assert_eq!(
        new_model, expected_read,
        "readback changes only the target definitions"
    );
    let source_before = crate::docx::extract(&before, None).unwrap();
    let source_after = crate::docx::extract(&after, None).unwrap();
    let source_json = serde_json::to_string(&source_before).unwrap();
    for label in [
        "BodyKeep",
        "LinkKeep",
        "NumberedKeep",
        "TableKeep",
        "NestedKeep",
        "headerKeep",
        "footerKeep",
        "NoteKeep",
        "CommentKeep",
    ] {
        assert!(source_json.contains(label), "source preserves {label}");
    }
    assert!(source_before
        .pages
        .iter()
        .flat_map(|p| &p.links)
        .any(|l| l.uri == "https://example.com/style-proof" && l.text == "LinkKeep"));
    assert!(
        source_json.contains("MarkerKeep"),
        "referenced image is normalized"
    );
    assert_eq!(original.comments.len(), 1);
    let comment_part = original.comments.values().next().unwrap().part;
    assert!(original.parts[&comment_part]
        .blocks
        .iter()
        .any(|b| b.as_para().is_some_and(|p| p.text == "CommentKeep")));
    assert_eq!(
        serde_json::to_value(&source_before).unwrap(),
        serde_json::to_value(&source_after).unwrap()
    );
    let before_pdf =
        wordcraft_pdf::export(&original, &wordcraft_pdf::PdfOptions::default()).unwrap();
    let after_pdf = wordcraft_pdf::export(&removed, &wordcraft_pdf::PdfOptions::default()).unwrap();
    let pdf_before = crate::pdf::extract(&before_pdf, None).unwrap();
    let pdf_after = crate::pdf::extract(&after_pdf, None).unwrap();
    let pdf_text = pdf_before
        .pages
        .iter()
        .flat_map(|p| &p.blocks)
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for label in [
        "BodyKeep",
        "LinkKeep",
        "NumberedKeep",
        "TableKeep",
        "NestedKeep",
        "headerKeep",
        "footerKeep",
        "NoteKeep",
    ] {
        assert!(
            pdf_text.contains(label),
            "PDF actually displays {label}: {pdf_text}"
        );
    }
    assert!(pdf_before
        .pages
        .iter()
        .flat_map(|p| &p.links)
        .any(|l| l.uri == "https://example.com/style-proof"));
    assert_eq!(
        serde_json::to_value(&pdf_before).unwrap(),
        serde_json::to_value(&pdf_after).unwrap()
    );
    assert!(!pdf_before.pages.is_empty());
    for dpi in [72.0, 144.0] {
        let (mut source_red, mut pdf_red) = (0, 0);
        for page in 0..pdf_before.pages.len() {
            let source_png = crate::docx::render(&before, page, dpi, None).unwrap();
            let pdf_png = crate::pdf::render(&before_pdf, page, dpi, None).unwrap();
            source_red += red_pixels(&source_png);
            pdf_red += red_pixels(&pdf_png);
            assert_eq!(
                source_png,
                crate::docx::render(&after, page, dpi, None).unwrap()
            );
            assert_eq!(
                pdf_png,
                crate::pdf::render(&after_pdf, page, dpi, None).unwrap()
            );
        }
        assert!(
            source_red > 25 && pdf_red > 25,
            "actual red image is visible in source and PDF at {dpi} DPI"
        );
    }

    // Every typed carrier below remains dangling after native remove: it cannot
    // be used as an upstream usage oracle. Production must preflight these.
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
        let mut styles = StyleSheet::builtin();
        let mut drop = definition("Drop", StyleKind::Paragraph);
        drop.based_on = Some("Normal".into());
        styles.upsert(drop);
        let mut holder = definition("Holder", StyleKind::Paragraph);
        match edge {
            "based_on" => holder.based_on = Some("Drop".into()),
            "next" => holder.next = Some("Drop".into()),
            _ => holder.linked = Some("Drop".into()),
        }
        styles.upsert(holder);
        styles.remove("Drop").unwrap();
        let h = styles.get("Holder").unwrap();
        assert_eq!(
            h.based_on.as_deref(),
            (edge == "based_on").then_some("Normal")
        );
        assert_eq!(h.next, None);
        assert_eq!(h.linked, None);
    }
    let mut table_reference = Document::from_text("Keep");
    let mut table = Table::new(1, 1, 180.0);
    table.props.style = Some("Drop".into());
    table_reference
        .body
        .insert(0, Arc::new(Block::Table(table)));
    assert_eq!(
        table_reference.restyle("Drop", None),
        0,
        "public restyle omits table references"
    );
    assert_eq!(
        table_reference
            .table(StoryRef::Body, &Path::top(0))
            .unwrap()
            .props
            .style
            .as_deref(),
        Some("Drop")
    );
}

#[test]
fn unused_style_raw_identity_reservations_aliases_and_native_fallback() {
    let mut styles = StyleSheet::builtin();
    let old = styles.new_id("Old Name");
    let mut s = definition(&old, StyleKind::Paragraph);
    s.name = "Old Name".into();
    styles.upsert(s);
    styles.remove(&old).unwrap();
    assert_eq!(
        styles.new_id("Old Name"),
        old,
        "native IDs are reusable without reservations"
    );
    let mut alias = definition("OtherID", StyleKind::Paragraph);
    alias.name = old.clone();
    styles.upsert(alias);
    assert!(styles.get(&old).is_none());
    assert_eq!(
        styles.find(&old).unwrap().id,
        "OtherID",
        "old handle may alias a different display name"
    );
    styles.upsert(definition(&old, StyleKind::Character));
    assert_eq!(
        styles.find(&old).unwrap().id,
        old,
        "a live exact ID takes precedence over an equal display name"
    );
    styles.remove(&old).unwrap();
    let retired = BTreeSet::from([old.clone()]);
    let mut reservation = StyleSheet::empty();
    for id in styles.styles.iter().map(|s| &s.id).chain(&retired) {
        reservation.upsert(definition(id, StyleKind::Paragraph));
    }
    let fresh = reservation.new_id("Old Name");
    assert_ne!(fresh, old);
    assert!(reservation.get(&fresh).is_none());
    assert_eq!(fresh, reservation.clone().new_id("Old Name"));
    assert!(
        styles.get(&fresh).is_none(),
        "reservation sheet is not live document storage"
    );
    for name in ["!!!", "", "é猫", "A B", "AB"] {
        let id = reservation.new_id(name);
        assert!(reservation.get(&id).is_none());
        reservation.upsert(definition(&id, StyleKind::Paragraph));
        assert_ne!(reservation.new_id(name), id);
    }
    let mut exhausted = StyleSheet::empty();
    exhausted
        .styles
        .push(definition("Clash", StyleKind::Paragraph));
    exhausted
        .styles
        .extend((1..10_000).map(|i| definition(&format!("Clash{i}"), StyleKind::Paragraph)));
    let collision = exhausted.new_id("Clash");
    assert_eq!(collision, "Clash");
    assert!(
        exhausted.get(&collision).is_some(),
        "caller must explicitly reject native fallback"
    );
    let mut malformed = StyleSheet::empty();
    malformed.styles = vec![
        definition("Duplicate", StyleKind::Paragraph),
        definition("Duplicate", StyleKind::Character),
    ];
    assert_eq!(malformed.styles.len(), 2);
    assert_eq!(
        malformed.get("Duplicate").unwrap().kind,
        StyleKind::Paragraph
    );
    assert_eq!(
        malformed.remove("Duplicate").unwrap().kind,
        StyleKind::Paragraph
    );
    assert!(
        malformed.get("Duplicate").is_some(),
        "duplicate IDs must fail preflight"
    );
    let builtins = StyleSheet::builtin();
    for id in [
        "Normal",
        "DefaultParagraphFont",
        "TableNormal",
        "FootnoteText",
        "EndnoteText",
    ] {
        let mut unprotected = builtins.clone();
        assert!(unprotected.get(id).is_some());
        unprotected.get_mut(id).unwrap().builtin = false;
        unprotected.get_mut(id).unwrap().default = false;
        assert!(
            unprotected.remove(id).is_some(),
            "remove itself does not protect implicit builtin {id}"
        );
    }
    let mut custom_default = definition("LocalizedDefault", StyleKind::Paragraph);
    custom_default.default = true;
    let mut sheet = StyleSheet::empty();
    sheet.upsert(custom_default);
    assert!(
        sheet.remove("LocalizedDefault").is_some(),
        "default flag also requires adapter protection"
    );
}
