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
        Err(EngineError::Unsupported(_))
    ));
    let copy = book.copy().unwrap();
    book.close();
    assert!(matches!(book.chapter_count(), Err(EngineError::Closed(_))));
    assert_eq!(copy.chapter_count().unwrap(), 1);
}
