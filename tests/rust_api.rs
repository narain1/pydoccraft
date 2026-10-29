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
