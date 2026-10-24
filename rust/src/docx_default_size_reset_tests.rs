use super::*;
fn snapshot(d: &EditableDocx) -> (Document, u64) {
    (d.document().unwrap().clone(), d.revision)
}
#[test]
fn clearing_the_default_font_size_is_rejected_without_changing_the_document() {
    pyo3::prepare_freethreaded_python();
    // WordCraft's reader substitutes 10 pt for an omitted default size while its model keeps the
    // size inherited, so a cleared size would change every inheriting run after a save and
    // reopen. The clear is refused instead.
    let mut d = EditableDocx::new();
    d.format_defaults(r#"{"font_size":10,"font":"DejaVu Sans"}"#)
        .unwrap();
    d.add_paragraph("BodyKeep").unwrap();
    let before = snapshot(&d);
    for patch in [r#"{"font_size":null}"#, r#"{"font_size":null,"bold":true}"#] {
        let error = d.format_defaults(patch).unwrap_err();
        assert!(
            error.to_string().contains("clearing the default font size"),
            "{patch}: {error}"
        );
        assert_eq!(snapshot(&d), before, "{patch}");
    }
    // Setting another size, or touching other defaults, still works.
    d.format_defaults(r#"{"font_size":12}"#).unwrap();
    d.format_defaults(r#"{"bold":true}"#).unwrap();
    assert_eq!(d.document().unwrap().styles.default_chr.size, Some(12.0));
}
