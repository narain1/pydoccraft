use super::*;
use deckcraft_model::{style::ColorRef, Fill, Line, Rgba, ShapeId, Xfrm};
use deckcraft_render::{try_render_slide_drawn, RenderOpts, ShapeState};
use std::{cell::Cell, io::Cursor, sync::Arc};

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

#[test]
fn busy_native_renderer_reaches_png_session_and_pdf_errors_then_recovers() {
    let presentation = colored_deck();
    let bytes = deckcraft_pptx::export(&presentation).unwrap();
    let (session, _) = PptxSession::open(&bytes).unwrap();
    let clip = Some((0.0, 20.0, 100.0, 100.0));
    let full_before = render_with(&presentation, 0, 72.0, None).unwrap();
    let clip_before = render_with(&presentation, 0, 72.0, clip).unwrap();
    let outer_opts = RenderOpts {
        scale: 0.25,
        ..Default::default()
    };
    let baseline = try_render_slide_drawn(&presentation, 0, &outer_opts).unwrap();
    assert!(baseline.1.x0 < 0.0);
    assert_eq!(session.stats().unwrap(), (1, 0, 0));
    let callbacks = Cell::new(0_u64);
    let callback = |_| {
        callbacks.set(callbacks.get() + 1);
        for region in [None, clip] {
            let error = render_with(&presentation, 0, 72.0, region).unwrap_err();
            assert!(matches!(&error, EngineError::Rendering(_)));
            assert_eq!(error.to_string(), "rendering_failure:PPTX rendering failed: this thread's slide renderer is already in use");
        }
        let error = session.render(0, 72.0, None).unwrap_err();
        assert!(matches!(&error, EngineError::Rendering(_)));
        assert!(error.to_string().contains("already in use"));
        let error = to_pdf(&bytes).unwrap_err();
        assert!(matches!(&error, EngineError::Conversion(_)));
        assert_eq!(error.to_string(), "conversion_failure:PPTX to PDF conversion failed: slide rendering: this thread's slide renderer is already in use");
        ShapeState::default()
    };
    let actual = try_render_slide_drawn(
        &presentation,
        0,
        &RenderOpts {
            state: Some(&callback),
            scale: 0.25,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(callbacks.get() > 0);
    assert_eq!(
        (actual.0.width, actual.0.height, actual.1),
        (baseline.0.width, baseline.0.height, baseline.1)
    );
    assert_eq!(actual.0.pixels, baseline.0.pixels);
    assert_eq!(session.stats().unwrap(), (1, 0, callbacks.get()));
    assert_eq!(
        render_with(&presentation, 0, 72.0, None).unwrap(),
        full_before
    );
    assert_eq!(
        render_with(&presentation, 0, 72.0, clip).unwrap(),
        clip_before
    );
    let session_png = session.render(0, 72.0, None).unwrap();
    assert_eq!(session_png, render(&bytes, 0, 72.0).unwrap());
    assert_eq!(session.stats().unwrap(), (1, 0, callbacks.get() + 1));
    assert_eq!(
        crate::pdf::extract(&to_pdf(&bytes).unwrap(), None)
            .unwrap()
            .pages
            .len(),
        1
    );
    session.close().unwrap();
    assert!(matches!(
        session.render(0, 72.0, None),
        Err(EngineError::Closed(_))
    ));
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
    let outer = colored_deck();
    let calls = Cell::new(0);
    let callback = |_| {
        calls.set(calls.get() + 1);
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
        assert!(matches!(
            render_with(&huge, 0, 72.0, Some((0.0, 0.0, 1.0, 1.0))),
            Err(EngineError::Rendering(_))
        ));
        ShapeState::default()
    };
    try_render_slide_drawn(
        &outer,
        0,
        &RenderOpts {
            state: Some(&callback),
            scale: 0.25,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(calls.get() > 0);
    let (width, height, pixels) =
        decoded(&render_with(&huge, 0, 72.0, Some((0.0, 0.0, 1.0, 1.0))).unwrap());
    assert_eq!((width, height, pixels.len()), (1, 1, 4));
}
