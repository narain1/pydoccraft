//! Imported native-writer fixture: observable PDF corner regression at the selected pin.
use super::*;
use std::io::Cursor;
use wordcraft_doc::para::ShapeKind;

const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/wordcraft-rectangle-stroke100.docx"
));

fn assert_corners(png: &[u8], dpi: f64) {
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    let side = (440.0 * dpi / 72.0).ceil() as u32;
    assert_eq!((info.width, info.height), (side, side));
    for (point, expected) in [(110.0, true), (95.0, false)] {
        let center = (point * dpi / 72.0).round() as usize;
        for y in center - 1..=center + 1 {
            for x in center - 1..=center + 1 {
                let i = (y * side as usize + x) * 4;
                let blue = i16::from(pixels[i + 2]) - i16::from(pixels[i]) > 16
                    && i16::from(pixels[i + 2]) - i16::from(pixels[i + 1]) > 16
                    && pixels[i + 3] > 16;
                assert_eq!(blue, expected, "{dpi}DPI corner({x},{y})");
            }
        }
    }
}

#[test]
fn imported_rectangle_round_corners_survive_native_docx_and_pdf_paths() {
    let source = read(FIXTURE, None).unwrap();
    let rewritten = wordcraft_docx::write(&source).unwrap();
    let reopened = read(&rewritten, None).unwrap();
    let mut native_before = Vec::new();
    let mut pdf_before = Vec::new();
    for (stage, model, bytes) in [(0, &source, FIXTURE), (1, &reopened, rewritten.as_slice())] {
        assert_eq!(
            (model.last_section.page_w, model.last_section.page_h),
            (440.0, 440.0)
        );
        let p = model.para(StoryRef::Body, &Path::top(0)).unwrap();
        let InlineObject::Shape {
            kind,
            w,
            h,
            stroke_width,
            float,
            ..
        } = &p.objects[0]
        else {
            panic!("fixture lost its shape")
        };
        assert!(matches!(kind, ShapeKind::Rectangle));
        assert_eq!(
            (*w, *h, *stroke_width, float.x, float.y),
            (160.0, 100.0, 100.0, 140.0, 140.0)
        );
        let layout =
            wordcraft_layout::layout(model, &mut LayoutCache::new(), &LayoutOptions::default());
        assert_eq!(layout.pages.len(), 1);
        let pdf = to_pdf(bytes, None).unwrap();
        assert_eq!(crate::pdf::extract(&pdf, None).unwrap().pages.len(), 1);
        for (index, dpi) in [72.0, 144.0, 300.0].into_iter().enumerate() {
            let native = render_with(model, &layout, 0, dpi, None).unwrap();
            let pdf_png = crate::pdf::render(&pdf, 0, dpi, None).unwrap();
            assert_corners(&native, dpi);
            assert_corners(&pdf_png, dpi);
            if stage == 0 {
                native_before.push(native);
                pdf_before.push(pdf_png);
            } else {
                assert_eq!(native, native_before[index]);
                assert_eq!(pdf_png, pdf_before[index]);
            }
        }
    }
}
