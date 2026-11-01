use _core::{api, EngineError};

#[test]
fn detection_and_conversion_errors_are_available_to_rust() {
    assert_eq!(api::detect(b"%PDF-1.4").unwrap(), "pdf");
    assert!(matches!(
        api::detect(b"not a document"),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        api::convert(b"", ".WORD", "PPTX", None),
        Err(EngineError::Unsupported(message)) if message == "conversion from docx to pptx"
    ));
}

#[test]
fn extraction_and_rendering_are_available_without_python() {
    let data = include_bytes!("data/wordcraft-rectangle-stroke100.docx");
    let document = api::extract(data, ".WORD", None).unwrap();
    assert_eq!(document.format, "docx");
    assert!(!document.pages.is_empty());
    let png = api::render(data, "docx", 0, 72.0, None, None).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(matches!(
        api::render(data, "docx", 0, 72.0, None, Some((0.0, 0.0, -1.0, 1.0))),
        Err(EngineError::InvalidArgument(_))
    ));
    assert!(matches!(
        api::render(data, "epub", 0, 72.0, None, None),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        api::extract(b"bad", "pdf", None),
        Err(EngineError::Malformed(_))
    ));
}
