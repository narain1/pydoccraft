//! A render panic must surface as an explicit error, never as a blank slide.
//!
//! DeckCraft catches render panics and draws a blank slide, counting them in a process-wide
//! counter. The adapter turns that counter into an error; these tests cause real panics through
//! a panicking shape-state callback.
use super::*;
use deckcraft_model::{style::ColorRef, Fill, Line, Rgba, ShapeId, Xfrm};
use deckcraft_render::{RenderOpts, ShapeState};
use std::{io::Cursor, sync::Arc};

fn colored_deck() -> Presentation {
    let mut presentation = Presentation::default();
    Arc::make_mut(&mut presentation.slides[0]).shapes = vec![Shape {
        id: ShapeId(114),
        xfrm: Some(Xfrm::new(-20.0, 30.0, 100.0, 80.0)),
        fill: Some(Fill::solid(ColorRef::rgb(Rgba::rgb(230, 30, 60)))),
        line: Some(Line::none()),
        ..Default::default()
    }];
    presentation
}
fn decoded(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(frame.color_type, png::ColorType::Rgba);
    pixels.truncate(frame.buffer_size());
    (frame.width, frame.height, pixels)
}
fn panicking(_: ShapeId) -> ShapeState {
    panic!("deliberate test panic in a shape-state callback")
}

#[test]
fn render_panic_is_an_error_not_a_blank_slide_and_the_next_render_recovers() {
    let presentation = colored_deck();
    let before = render_with(&presentation, 0, 72.0, None).unwrap();
    let options = RenderOpts {
        scale: 0.25,
        state: Some(&panicking),
        ..Default::default()
    };
    let result = try_render_slide(&presentation, 0, &options);
    assert!(matches!(result, Err(GuardedError::Panicked)), "{result:?}");
    // The slide is not blank: the unguarded call would have returned a white image here.
    assert!(before.windows(4).any(|pixel| pixel != [255, 255, 255, 255]));
    assert_eq!(render_with(&presentation, 0, 72.0, None).unwrap(), before);
}

#[test]
fn invalid_slide_index_is_reported_without_touching_the_panic_counter() {
    let presentation = colored_deck();
    let before = deckcraft_render::panics_caught();
    let error = try_render_slide(&presentation, 9, &RenderOpts::default()).unwrap_err();
    assert!(matches!(error, GuardedError::InvalidSlide));
    assert_eq!(deckcraft_render::panics_caught(), before);
}

#[test]
fn blank_pptx_and_preallocation_checks_remain_distinct_from_renderer_failure() {
    let mut blank = Presentation::default();
    Arc::make_mut(&mut blank.slides[0]).shapes.clear();
    let bytes = deckcraft_pptx::export(&blank).unwrap();
    for dpi in [72.0, 144.0] {
        let (width, height, pixels) = decoded(&render(&bytes, 0, dpi).unwrap());
        assert_eq!(
            (width, height),
            (
                (blank.slide_size.width * dpi / 72.0) as u32,
                (blank.slide_size.height * dpi / 72.0) as u32
            )
        );
        assert_eq!(pixels.len(), width as usize * height as usize * 4);
        // A genuinely empty slide is a valid white image and must not be reported as a failure.
        assert!(pixels.iter().all(|component| *component == 255));
    }
    assert_eq!(
        crate::pdf::extract(&to_pdf(&bytes).unwrap(), None)
            .unwrap()
            .pages
            .len(),
        1
    );
    let mut huge = blank.clone();
    huge.slide_size.width = 20_000.0;
    assert!(matches!(
        render_with(&blank, 0, f64::NAN, None),
        Err(EngineError::ResourceLimit(_))
    ));
    assert!(matches!(
        render_with(&blank, usize::MAX, 72.0, None),
        Err(EngineError::Unsupported(_))
    ));
    assert!(matches!(
        render_with(&huge, 0, 72.0, None),
        Err(EngineError::ResourceLimit(_))
    ));
    assert!(matches!(
        render_with(&blank, 0, 72.0, Some((0.0, 0.0, 0.0, 10.0))),
        Err(EngineError::InvalidArgument(_))
    ));
    // A checked small region reaches the native call instead of allocating the huge slide.
    let (width, height, pixels) =
        decoded(&render_with(&huge, 0, 72.0, Some((0.0, 0.0, 1.0, 1.0))).unwrap());
    assert_eq!((width, height, pixels.len()), (1, 1, 4));
}
