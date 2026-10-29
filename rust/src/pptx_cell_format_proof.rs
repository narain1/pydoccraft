//! Spec121 native spacing evidence and bounded owned-cell adapter acceptance.
use super::*;
use deckcraft_model::text::{Bullet, LevelStyle, ParaProps, Paragraph as NativeParagraph, Spacing};

fn fixture(merged: u8) -> (EditablePptx, u32) {
    let mut owner = EditablePptx::new(576.0, 576.0).unwrap();
    owner.add_slide("blank").unwrap();
    owner.set_slide_notes(0, "Retained notes").unwrap();
    let mut image = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut image, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[200, 160, 0])
            .unwrap();
    }
    owner
        .add_image(0, &image, 500.0, 530.0, 24.0, 24.0, "Retained image")
        .unwrap();
    let id = owner
        .add_table(
            0,
            vec![
                vec!["First".into(), "Neighbor".into()],
                vec!["Lower".into(), "Outside".into()],
            ],
            24.0,
            24.0,
            528.0,
            480.0,
            18.0,
        )
        .unwrap();
    match merged {
        0 => (),
        1 => owner.merge_cells(0, id, 0, 0, 0, 1).unwrap(),
        2 => owner.merge_cells(0, id, 0, 0, 1, 0).unwrap(),
        3 => owner.merge_cells(0, id, 0, 0, 1, 1).unwrap(),
        _ => unreachable!(),
    }
    let url = make_link(
        owner.document().unwrap(),
        Some("https://example.com/retained"),
        None,
        "Retained link",
    )
    .unwrap();
    let ShapeKind::Table(table) = &mut shape_mut(owner.presentation.as_mut().unwrap(), 0, id)
        .unwrap()
        .kind
    else {
        panic!("table")
    };
    table.style = "none".into();
    let cell = table.cell_mut(0, 0).unwrap();
    cell.margins = Some([0.0; 4]);
    cell.text.paragraphs = ["First office", "Middle café", "Last tail"]
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let mut p = NativeParagraph::new(text);
            p.props.bullet = Some(Bullet::None);
            p.props.align = Some(Align::Left);
            p.def_props.size = Some(18.0);
            p.end_props.italic = Some(i == 1);
            p.runs[0].props.fill = Some(solid([(224, 0, 0), (0, 160, 0), (0, 0, 224)][i]));
            p.runs[0].props.link = if i == 0 { url.clone() } else { None };
            p
        })
        .collect();
    let mut inherited = LevelStyle::default();
    inherited.para.margin_left = Some(12.0);
    inherited.para.margin_right = Some(6.0);
    inherited.para.indent = Some(3.0);
    inherited.para.space_before = Some(Spacing::Pts(4.0));
    inherited.para.space_after = Some(Spacing::Pts(5.0));
    inherited.para.line_spacing = Some(Spacing::Pct(1.25));
    cell.text.list_style.set(0, inherited);
    owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
    owner.refresh_budgets().unwrap();
    (owner, id)
}

fn props(owner: &mut EditablePptx, id: u32, index: usize) -> &mut ParaProps {
    &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
        .unwrap()
        .text
        .paragraphs[index]
        .props
}

fn raw_pixels(owner: &EditablePptx, scale: f64) -> Vec<u8> {
    crate::pptx::try_render_slide(
        owner.document().unwrap(),
        0,
        &deckcraft_render::RenderOpts {
            scale,
            ..Default::default()
        },
    )
    .unwrap()
    .pixels
}

// Unsupported overlapping-layout diagnostic, retained within the raw model gate.
fn target(owner: &EditablePptx, id: u32, paragraph: usize) -> CellTarget {
    let (row, col, grid, body, _) = owner.cell_paragraph_info(0, id, 0, 0).unwrap();
    (row, col, paragraph, grid, body)
}

fn unchanged(owner: &EditablePptx, before: &EditablePptx) {
    assert_eq!(owner.document().unwrap(), before.document().unwrap());
    assert_eq!(owner.revision, before.revision);
    assert_eq!(owner.table_generations, before.table_generations);
    assert_eq!(owner.cell_generations, before.cell_generations);
    assert_eq!(
        (owner.text_bytes, owner.media_bytes, owner.shapes),
        (before.text_bytes, before.media_bytes, before.shapes)
    );
}

#[test]
fn pptx_cell_format_owned_bounds_snapshot_reset_and_model_preservation() {
    for merged in 0..4 {
        let (mut owner, id) = fixture(merged);
        let cell = target(&owner, id, 1);
        let original = owner.clone();
        let direct: serde_json::Value =
            serde_json::from_str(&owner.cell_paragraph_formatting(0, id, cell).unwrap()).unwrap();
        assert_eq!(direct["line_spacing"], serde_json::Value::Null);
        assert_eq!(direct["space_before"], serde_json::Value::Null);
        for patch in [
            r#"{"indent_left":0,"indent_right":0,"indent_first":0,"space_before":0,"space_after":0,"line_spacing":1,"alignment":"left"}"#,
            r#"{"indent_left":4032,"indent_right":4032,"indent_first":-4032,"space_before":1584,"space_after":1584,"line_spacing":100,"alignment":"right"}"#,
            r#"{"indent_first":4032,"line_spacing":1.23456789,"space_after":5.67891}"#,
        ] {
            let before = owner.clone();
            owner
                .format_cell_paragraph(0, id, cell, patch, owner.revision)
                .unwrap();
            let snapshot: serde_json::Value =
                serde_json::from_str(&owner.cell_paragraph_formatting(0, id, cell).unwrap())
                    .unwrap();
            let supplied: serde_json::Value = serde_json::from_str(patch).unwrap();
            for (key, value) in supplied.as_object().unwrap() {
                assert_eq!(snapshot[key].as_f64(), value.as_f64());
                if key == "alignment" {
                    assert_eq!(snapshot[key], *value);
                }
            }
            let mut expected = before.document().unwrap().clone();
            cell_mut(&mut expected, 0, id, 0, 0)
                .unwrap()
                .text
                .paragraphs[1]
                .props = owner.target_body(0, id, Some(cell)).unwrap().paragraphs[1]
                .props
                .clone();
            assert_eq!(*owner.document().unwrap(), expected);
            assert_eq!(owner.revision, before.revision + 1);
            assert_eq!(owner.table_generations, before.table_generations);
            assert_eq!(owner.cell_generations, before.cell_generations);
            assert_eq!(
                (owner.text_bytes, owner.media_bytes, owner.shapes),
                (before.text_bytes, before.media_bytes, before.shapes)
            );
            let reread = deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap();
            assert!(reread.validate().is_empty());
        }
        owner.format_cell_paragraph(0, id, cell,
            r#"{"indent_left":null,"indent_right":null,"indent_first":null,"space_before":null,"space_after":null,"line_spacing":null,"alignment":"left"}"#,
            owner.revision).unwrap();
        assert_eq!(owner.document().unwrap(), original.document().unwrap());
        assert_eq!(raw_pixels(&owner, 1.0), raw_pixels(&original, 1.0));
        assert_eq!(
            owner.cell_paragraph_formatting(0, id, cell).unwrap(),
            original.cell_paragraph_formatting(0, id, cell).unwrap()
        );
    }
}

#[test]
fn pptx_cell_format_owned_invalid_compounds_targets_and_revision_are_atomic() {
    let (mut owner, id) = fixture(0);
    let cell = target(&owner, id, 1);
    for patch in [
        r#"{"alignment":"center","line_spacing":0}"#,
        r#"{"line_spacing":0.9999999999999999}"#,
        r#"{"line_spacing":100.00001}"#,
        r#"{"line_spacing":true}"#,
        r#"{"line_spacing":"1.5"}"#,
        r#"{"line_spacing":1e999}"#,
        r#"{"indent_left":-0.00001}"#,
        r#"{"indent_right":4032.00001}"#,
        r#"{"indent_first":-4032.00001}"#,
        r#"{"indent_first":4032.00001}"#,
        r#"{"space_before":-0.01}"#,
        r#"{"space_after":1584.01}"#,
        r#"{"alignment":"distributed","space_after":8}"#,
        r#"{"space_after":8,"keep_with_next":true}"#,
        r#"{"space_after":[]}"#,
        "[]",
    ] {
        let before = owner.clone();
        assert!(
            owner
                .format_cell_paragraph(0, id, cell, patch, owner.revision)
                .is_err(),
            "{patch}"
        );
        unchanged(&owner, &before);
    }
    for bad in [
        (0, 0, 99, cell.3, cell.4),
        (0, 0, 1, cell.3 + 1, cell.4),
        (0, 0, 1, cell.3, cell.4 + 1),
    ] {
        let before = owner.clone();
        assert!(owner
            .format_cell_paragraph(0, id, bad, "{}", owner.revision)
            .is_err());
        assert!(owner.cell_paragraph_formatting(0, id, bad).is_err());
        unchanged(&owner, &before);
    }
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, cell, "{}", owner.revision - 1)
        .unwrap_err()
        .to_string()
        .contains("stale"));
    unchanged(&owner, &before);
    owner.merge_cells(0, id, 0, 0, 0, 1).unwrap();
    let current = target(&owner, id, 1);
    let covered = (0, 1, 1, current.3, current.4);
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, covered, "{}", owner.revision)
        .unwrap_err()
        .to_string()
        .contains("canonical cell anchor"));
    unchanged(&owner, &before);
    owner.revision = u64::MAX - 1;
    owner.cell_generations.insert((id, 0, 0), u64::MAX);
    owner.table_generations.insert(id, u64::MAX);
    let current = target(&owner, id, 1);
    owner
        .format_cell_paragraph(0, id, current, r#"{"line_spacing":1}"#, owner.revision)
        .unwrap();
    assert_eq!(owner.revision, u64::MAX);
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, current, r#"{"line_spacing":2}"#, owner.revision)
        .unwrap_err()
        .to_string()
        .contains("revision limit"));
    unchanged(&owner, &before);
}

#[test]
fn pptx_cell_format_owned_patch_property_budget_and_native_units() {
    let (mut owner, id) = fixture(0);
    let cell = target(&owner, id, 1);
    let patch = format!(
        "{{\"space_after\":8}}{}",
        " ".repeat(MAX_PATCH - r#"{"space_after":8}"#.len())
    );
    assert_eq!(patch.len(), MAX_PATCH);
    owner
        .format_cell_paragraph(0, id, cell, &patch, owner.revision)
        .unwrap();
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, cell, &(patch + " "), owner.revision)
        .unwrap_err()
        .to_string()
        .contains("16 KiB"));
    unchanged(&owner, &before);
    let cost = content_budget(owner.document().unwrap())
        .unwrap()
        .properties;
    Arc::make_mut(&mut owner.presentation.as_mut().unwrap().slides[0])
        .notes
        .paragraphs[0]
        .def_props
        .font = Some("x".repeat(MAX_PROPERTIES - cost));
    assert_eq!(
        content_budget(owner.document().unwrap())
            .unwrap()
            .properties,
        MAX_PROPERTIES
    );
    owner
        .format_cell_paragraph(0, id, cell, r#"{"line_spacing":1.5}"#, owner.revision)
        .unwrap();
    Arc::make_mut(&mut owner.presentation.as_mut().unwrap().slides[0])
        .notes
        .paragraphs[0]
        .def_props
        .font
        .as_mut()
        .unwrap()
        .push('x');
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, cell, r#"{"line_spacing":2}"#, owner.revision)
        .unwrap_err()
        .to_string()
        .contains("formatting/link properties"));
    unchanged(&owner, &before);
    for (before, after, line) in [
        (Some(Spacing::Pct(1.0)), None, None),
        (None, Some(Spacing::Pct(1.0)), None),
        (None, None, Some(Spacing::Pts(18.0))),
    ] {
        let (mut owner, id) = fixture(0);
        let cell = target(&owner, id, 1);
        let p = props(&mut owner, id, 1);
        p.space_before = before;
        p.space_after = after;
        p.line_spacing = line;
        assert!(owner
            .cell_paragraph_formatting(0, id, cell)
            .unwrap_err()
            .to_string()
            .contains("unsupported native unit"));
    }
}
