use super::*;

fn resolved(d: &Document, story: StoryRef, path: &Path) -> (f32, f32) {
    let p = d.para(story, path).unwrap();
    let c = d
        .styles
        .resolve_char(p.props.style.as_deref(), p.props_of_char(0));
    (c.size, c.size_cs)
}

fn snapshot(d: &EditableDocx) -> (Document, String) {
    (
        d.document().unwrap().clone(),
        format!(
            "{}|{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            d.revision,
            d.text_bytes,
            d.cell_count,
            d.media_bytes,
            d.next_block_id,
            d.locations,
            d.stories,
            d.story_placeholders,
            d.cell_placeholders,
            d.table_generations,
            d.retired_style_ids,
            d.shape_ids
        ),
    )
}

#[test]
fn default_size_reset_preserves_native_inheritance_parts_overrides_and_accounting() {
    pyo3::prepare_freethreaded_python();
    let mut d = EditableDocx::new();
    d.format_defaults(r#"{"font_size":10,"font":"DejaVu Sans"}"#)
        .unwrap();
    let body = d.add_paragraph("BodyKeep").unwrap();
    let header = d.ensure_story("header", "default", 0).unwrap();
    let hp = d.story_add_paragraph(header, "HeaderKeep").unwrap();
    let table = d.add_table(vec![vec!["CellKeep".into()]]).unwrap();
    let cell = d.cell_paragraph_ids(table, 0, 0).unwrap()[0];
    let direct = d.add_paragraph("").unwrap();
    d.add_run_patch(direct, "DirectKeep", r#"{"font_size":18}"#)
        .unwrap();
    let style = d.add_style("FixedSize", "paragraph", None).unwrap();
    d.format_style(&style, r#"{"font_size":21}"#, None).unwrap();
    let styled = d.add_paragraph("StyleKeep").unwrap();
    d.set_paragraph_style(styled, &style).unwrap();
    d.set_range_link(body, 0, 8, d.revision, Some("https://example.test/reset"))
        .unwrap();
    // The ordinary property must not erase a distinct complex-script override.
    d.document.as_mut().unwrap().styles.default_chr.size_cs = Some(17.0);
    let locations = [body, hp, cell, direct, styled].map(|id| d.location(id).unwrap());
    let before = snapshot(&d);
    let mut independent = d.copy().unwrap();
    d.format_defaults(r#"{"font_size":null}"#).unwrap();
    assert_eq!(d.document().unwrap().styles.default_chr.size, None);
    assert_eq!(d.document().unwrap().styles.default_chr.size_cs, Some(17.0));
    assert_eq!(d.revision, independent.revision + 1);
    let mut expected = before.0;
    expected.styles.default_chr.size = None;
    assert_eq!(d.document().unwrap(), &expected);
    assert_eq!(
        (d.text_bytes, d.cell_count, d.media_bytes, d.next_block_id),
        (
            independent.text_bytes,
            independent.cell_count,
            independent.media_bytes,
            independent.next_block_id
        )
    );
    assert_eq!(d.locations, independent.locations);
    assert_eq!(d.stories, independent.stories);
    let bytes = d.to_bytes().unwrap();
    let reopened = wordcraft_docx::read(&bytes).unwrap();
    // WordCraft now writes the resolver's fallback size for an unset default, so the reopened
    // document carries it explicitly and every inheriting run keeps its effective size.
    assert_eq!(reopened.styles.default_chr.size, Some(11.0));
    assert_eq!(reopened.styles.default_chr.size_cs, Some(17.0));
    for (index, (story, path)) in locations.iter().enumerate() {
        let size = [11.0, 11.0, 11.0, 18.0, 21.0][index];
        assert_eq!(resolved(d.document().unwrap(), *story, path), (size, 17.0));
        let reopened_story = if matches!(story, StoryRef::Part(_)) {
            StoryRef::Part(reopened.last_section.headers.default.unwrap())
        } else {
            *story
        };
        assert_eq!(resolved(&reopened, reopened_story, path), (size, 17.0));
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("word/styles.xml").unwrap(), &mut xml).unwrap();
    let defaults = xml
        .split("<w:docDefaults>")
        .nth(1)
        .unwrap()
        .split("</w:docDefaults>")
        .next()
        .unwrap();
    assert!(defaults.contains(r#"<w:sz w:val="22"/>"#));
    assert!(defaults.contains("<w:szCs "));
    let revision = d.revision;
    d.format_defaults(r#"{"font_size":null}"#).unwrap();
    assert_eq!(d.revision, revision + 1);
    assert_eq!(d.document().unwrap(), &expected);
    independent.format_defaults(r#"{"font_size":12}"#).unwrap();
    assert_eq!(
        resolved(
            independent.document().unwrap(),
            locations[0].0,
            &locations[0].1
        )
        .0,
        12.0
    );
    assert_eq!(
        resolved(d.document().unwrap(), locations[0].0, &locations[0].1).0,
        11.0
    );
}

#[test]
fn default_size_reset_invalid_combination_tabs_and_revision_fail_atomically() {
    pyo3::prepare_freethreaded_python();
    let mut d = EditableDocx::new();
    d.set_margins(12.0, 12.0, 12.0, 12.0).unwrap();
    d.set_page_size(160.0, 120.0).unwrap();
    d.format_defaults(r#"{"font_size":10}"#).unwrap();
    d.add_paragraph("Retained").unwrap();
    let table = d.add_table(vec![vec!["Cell".into()]]).unwrap();
    for patch in [
        r#"{"font_size":null,"bold":"wrong"}"#,
        r#"{"font_size":null,"tabs":[{"position":10000}]}"#,
    ] {
        let before = snapshot(&d);
        assert!(d.format_defaults(patch).is_err(), "{patch}");
        assert_eq!(snapshot(&d), before);
    }
    d.revision = u64::MAX;
    let before = snapshot(&d);
    assert!(d.format_defaults(r#"{"font_size":null}"#).is_err());
    assert_eq!(snapshot(&d), before);
    assert_eq!(d.cell_text(table, 0, 0).unwrap(), "Cell");
    d.revision = u64::MAX - 1;
    d.format_defaults(r#"{"font_size":null}"#).unwrap();
    assert_eq!(d.revision, u64::MAX);
    assert_eq!(d.document().unwrap().styles.default_chr.size, None);
    let before = snapshot(&d);
    assert!(d.format_defaults(r#"{"font_size":null}"#).is_err());
    assert_eq!(snapshot(&d), before);
    d.close();
    assert!(d.format_defaults(r#"{"font_size":null}"#).is_err());
}
