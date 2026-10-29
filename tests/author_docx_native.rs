#![cfg(not(feature = "python"))]

use _core::author_docx::{EditableDocx, ListStart};
use _core::{api, EngineError};

#[test]
fn author_edit_serialize_and_extract_without_python() {
    let mut doc = EditableDocx::new();
    let paragraph = doc.add_paragraph("Native authoring").unwrap();
    doc.set_paragraph_list(
        paragraph,
        Some("numbered"),
        0,
        false,
        Some(ListStart::new(7).unwrap()),
    )
    .unwrap();
    let table = doc
        .add_table(vec![vec!["one".into(), "two".into()]])
        .unwrap();
    doc.set_cell_text(table, 0, 1, "changed").unwrap();
    assert_eq!(doc.cell_text(table, 0, 1).unwrap(), "changed");
    let bytes = doc.to_bytes().unwrap();
    let payload = api::extract(&bytes, "docx", None).unwrap();
    let text = serde_json::to_string(&payload).unwrap();
    assert!(text.contains("Native authoring"));
    assert!(text.contains("changed"));
    assert!(matches!(
        ListStart::new(0),
        Err(EngineError::InvalidArgument(_))
    ));
    assert!(matches!(
        doc.cell_text(table, 2, 0),
        Err(EngineError::PageIndex(_))
    ));
    doc.close();
    assert!(matches!(doc.revision(), Err(EngineError::Closed(_))));
}
