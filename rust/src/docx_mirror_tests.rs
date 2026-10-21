use super::*;

fn snapshot(d: &EditableDocx) -> (Vec<u8>, u64, usize, usize, usize, usize, String) {
    (
        wordcraft_docx::write(d.document().unwrap()).unwrap(),
        d.revision,
        d.text_bytes,
        d.cell_count,
        d.media_bytes,
        d.next_block_id,
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
            d.locations,
            d.stories,
            d.story_placeholders,
            d.body_placeholder,
            d.cell_placeholders,
            d.table_generations
        ),
    )
}
fn owner() -> EditableDocx {
    let mut d = EditableDocx::new();
    d.set_section_props(
        0,
        r#"{"page_size":[300,300],"margins":[48,24,36,72],"gutter":8,"page_number_start":2}"#,
    )
    .unwrap();
    d.add_paragraph("Retained body").unwrap();
    d
}
fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut e = png::Encoder::new(&mut bytes, 2, 2);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255].repeat(4))
            .unwrap();
    }
    bytes
}

#[test]
fn mirrored_margin_owned_native_setting_copy_and_revision_preserve_content() {
    let mut d = owner();
    assert!(!d.mirror_margins().unwrap());
    let p = d.add_paragraph("Counter ").unwrap();
    d.add_paragraph_field(p, "page").unwrap();
    let table = d.add_table(vec![vec!["Cell".into()]]).unwrap();
    let before = snapshot(&d);
    let mut copied = d.copy().unwrap();
    d.set_mirror_margins(true).unwrap();
    assert!(d.mirror_margins().unwrap());
    assert!(!copied.mirror_margins().unwrap());
    assert_eq!(d.revision, before.1 + 1);
    assert_eq!(d.text_bytes, before.2);
    assert_eq!(d.cell_count, before.3);
    assert_eq!(d.media_bytes, before.4);
    assert_eq!(d.next_block_id, before.5);
    assert_eq!(d.cell_text(table, 0, 0).unwrap(), "Cell");
    let saved = d.to_bytes().unwrap();
    let read = wordcraft_docx::read(&saved).unwrap();
    assert!(read.settings.mirror_margins);
    assert_eq!(read.last_section.margin_left, 72.0);
    assert_eq!(read.last_section.gutter, 8.0);
    let previous = d.revision;
    d.set_mirror_margins(true).unwrap();
    assert_eq!(d.revision, previous + 1);
    d.set_mirror_margins(false).unwrap();
    assert_eq!(d.to_bytes().unwrap(), copied.to_bytes().unwrap());
    copied.set_mirror_margins(true).unwrap();
    assert!(!d.mirror_margins().unwrap());
    d.close();
    assert!(d.mirror_margins().is_err());
    assert!(d.set_mirror_margins(false).is_err());
}

#[test]
fn mirrored_margin_owned_geometry_failure_is_atomic_for_line_labels_and_floats() {
    pyo3::prepare_freethreaded_python();
    let mut d = owner();
    d.set_section_props(
        0,
        r#"{"margins":[48,10,36,72],"line_numbers":[1,1,20,"each_page"]}"#,
    )
    .unwrap();
    d.to_bytes().unwrap();
    let before = snapshot(&d);
    let err = d.set_mirror_margins(true).unwrap_err();
    assert!(err.to_string().contains("line-number extent"), "{err}");
    assert_eq!(snapshot(&d), before);
    d.set_section_props(0, r#"{"line_numbers":null,"margins":[48,24,36,72]}"#)
        .unwrap();
    let id = d
        .add_image(&png(), Some(24.0), Some(12.0), "Marker")
        .unwrap();
    d.format_image(
        id,
        r#"{"wrap":"behind","anchor":"margin","position":[-30,90]}"#,
        d.revision,
    )
    .unwrap();
    let before = snapshot(&d);
    let err = d.set_mirror_margins(true).unwrap_err();
    assert!(
        err.to_string().contains("floating picture exceeds"),
        "{err}"
    );
    assert_eq!(snapshot(&d), before);
    d.format_image(id, r#"{"position":[4,90]}"#, d.revision)
        .unwrap();
    d.set_mirror_margins(true).unwrap();
    d.to_bytes().unwrap();
}

#[test]
fn mirrored_margin_owned_commit_guards_revision_and_table_page_context() {
    pyo3::prepare_freethreaded_python();
    let mut d = owner();
    d.next_block_id = usize::MAX;
    d.revision = u64::MAX - 1;
    d.set_mirror_margins(true).unwrap();
    assert_eq!(d.revision, u64::MAX);
    assert_eq!(d.next_block_id, usize::MAX);
    let before = snapshot(&d);
    assert!(d
        .set_mirror_margins(false)
        .unwrap_err()
        .to_string()
        .contains("revision limit"));
    assert_eq!(snapshot(&d), before);
    let mut d = owner();
    let table = d.add_table(vec![vec!["Count ".into()]]).unwrap();
    let p = d.cell_paragraph_ids(table, 0, 0).unwrap()[0];
    d.add_paragraph_field(p, "page_count").unwrap();
    let mut native = d.document().unwrap().clone();
    let InlineObject::Field { instr, .. } =
        &mut d.paragraph_mut(&mut native, p).unwrap().objects[0]
    else {
        panic!("field")
    };
    *instr = "PAGE \\* Arabic".into();
    d.document = Some(native);
    let before = snapshot(&d);
    let err = d.set_mirror_margins(true).unwrap_err();
    assert!(
        err.to_string().contains("page fields in table cells"),
        "{err}"
    );
    assert_eq!(snapshot(&d), before);
}
