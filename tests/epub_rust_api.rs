use _core::{api, author_epub::EditableEpub, EngineError};

#[test]
fn authors_and_reopens_epub_without_python() {
    let mut book = EditableEpub::new();
    book.set_metadata(
        "A & B".into(),
        "urn:test".into(),
        "en".into(),
        vec!["Author".into()],
    )
    .unwrap();
    let chapter = book.add_chapter("Chapter".into()).unwrap();
    book.add_heading(chapter, "Heading".into(), 1).unwrap();
    book.add_paragraph(chapter, "Text <escaped> & more".into())
        .unwrap();
    book.add_link(chapter, "Link".into(), "https://example.com".into())
        .unwrap();
    let bytes = book.to_bytes().unwrap();
    assert_eq!(api::detect(&bytes).unwrap(), "epub");
    let document = api::extract(&bytes, "epub", None).unwrap();
    assert_eq!(document.metadata["title"], "A & B");
    assert_eq!(document.metadata["authors"], serde_json::json!(["Author"]));
    let serialized = serde_json::to_string(&document).unwrap();
    assert!(serialized.contains("Text <escaped> & more"));
}

#[test]
fn copy_close_and_validation_are_independent() {
    let mut book = EditableEpub::default();
    let chapter = book.add_chapter("Chapter".into()).unwrap();
    assert!(matches!(
        book.add_heading(chapter, "bad".into(), 7),
        Err(EngineError::InvalidArgument(_))
    ));
    assert!(matches!(
        book.add_link(chapter, "bad".into(), "javascript:alert(1)".into()),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        book.add_image(
            chapter,
            b"<svg/>".to_vec(),
            "image/svg+xml".into(),
            "".into()
        ),
        Err(EngineError::InvalidArgument(_))
    ));
    let copy = book.copy().unwrap();
    book.close();
    assert!(matches!(book.chapter_count(), Err(EngineError::Closed(_))));
    assert_eq!(copy.chapter_count().unwrap(), 1);
}

#[test]
fn metadata_entities_and_repeated_authors_survive_reopen() {
    let mut book = EditableEpub::new();
    book.set_metadata(
        "A & <B>".into(),
        "urn:a&b".into(),
        "en".into(),
        vec!["A & B".into(), "C <D>".into()],
    )
    .unwrap();
    book.add_chapter("Chapter".into()).unwrap();
    let doc = api::extract(&book.to_bytes().unwrap(), "epub", None).unwrap();
    assert_eq!(doc.metadata["title"], "A & <B>");
    assert_eq!(doc.metadata["identifiers"], serde_json::json!(["urn:a&b"]));
    assert_eq!(
        doc.metadata["authors"],
        serde_json::json!(["A & B", "C <D>"])
    );
}
