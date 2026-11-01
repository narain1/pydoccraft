use _core::author_pptx::{EditablePptx, PptxAuthoringError};
use _core::{api, EngineError};

#[test]
fn authors_and_reopens_a_presentation_without_python() {
    let mut deck = EditablePptx::new(720.0, 540.0).unwrap();
    let slide = deck.add_slide("blank").unwrap();
    let shape = deck
        .add_text(slide, "Hello Rust", 20.0, 30.0, 300.0, 80.0, 18.0)
        .unwrap();
    assert_eq!(deck.shape_text(slide, shape, None).unwrap(), "Hello Rust");
    let bytes = deck.to_bytes().unwrap();
    let extracted = api::extract(&bytes, "pptx", None).unwrap();
    assert_eq!(extracted.pages.len(), 1);
    let png = api::render(&bytes, "pptx", 0, 72.0, None, None).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn errors_and_failed_mutations_are_independent_of_python() {
    let mut deck = EditablePptx::new(720.0, 540.0).unwrap();
    let slide = deck.add_slide("blank").unwrap();
    let revision = deck.revision().unwrap();
    assert!(matches!(
        deck.add_text(slide, "bad", 0.0, 0.0, -1.0, 20.0, 18.0),
        Err(PptxAuthoringError::InvalidArgument(_))
    ));
    assert_eq!(deck.revision().unwrap(), revision);
    assert!(matches!(
        deck.slide_id(99),
        Err(PptxAuthoringError::Index(_))
    ));
    deck.close();
    assert!(matches!(
        deck.slide_count(),
        Err(PptxAuthoringError::Engine(EngineError::Closed(_)))
    ));
}
