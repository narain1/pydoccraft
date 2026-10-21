//! Imported native-writer fixture: logical break offsets in the real PDF text layer.
use super::*;
use std::io::Cursor;

const FIXTURE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/deckcraft-table-line-breaks.pptx"
));

fn pixels(data: Vec<u8>) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(Cursor::new(data)).read_info().unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    bytes.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(bytes.len(), info.width as usize * info.height as usize * 4);
    (info.width, info.height, bytes)
}

fn picture_ink(image: &(u32, u32, Vec<u8>), dpi: f64) {
    let scale = dpi / 72.0;
    // Strictly inside the known 500,300..540,320 point shared image.
    let mut count = 0;
    for y in (305.0 * scale) as usize..(315.0 * scale) as usize {
        for x in (505.0 * scale) as usize..(535.0 * scale) as usize {
            let i = (y * image.0 as usize + x) * 4;
            if image.2[i..i + 4]
                .iter()
                .zip([32_u8, 96, 192, 255])
                .all(|(a, b)| a.abs_diff(b) <= 1)
            {
                count += 1;
            }
        }
    }
    assert!(
        count > 200,
        "shared picture must paint inside its fixed ROI"
    );
}

fn colored_ink(image: &(u32, u32, Vec<u8>)) -> ([u64; 3], [u64; 3]) {
    let mut sums = [0_u64; 3];
    let mut counts = [0_u64; 3];
    for (i, pixel) in image.2.chunks_exact(4).enumerate() {
        // Exclude the fixed shared picture near the bottom of the slide.
        if i / image.0 as usize >= (image.1 as usize * 4 / 5) {
            continue;
        }
        for c in 0..3 {
            if pixel[c] > 80
                && pixel[c] > pixel[(c + 1) % 3].saturating_add(60)
                && pixel[c] > pixel[(c + 2) % 3].saturating_add(60)
            {
                sums[c] += (i / image.0 as usize) as u64;
                counts[c] += 1;
            }
        }
    }
    (sums, counts)
}

fn color_order(image: &(u32, u32, Vec<u8>), order: &[usize]) {
    let (sums, counts) = colored_ink(image);
    assert!(
        counts.iter().all(|count| *count > 10),
        "all three colored paragraphs must have visible ink: {counts:?}"
    );
    let means = std::array::from_fn::<_, 3, _>(|i| sums[i] as f64 / counts[i] as f64);
    for pair in order.windows(2) {
        assert!(
            means[pair[0]] + 3.0 < means[pair[1]],
            "actual ink order: {means:?}, expected {order:?}"
        );
    }
}

#[test]
fn imported_table_breaks_preserve_searchable_pdf_text_links_and_pixels() {
    let source = read(FIXTURE).unwrap();
    let rewritten = deckcraft_pptx::export(&source).unwrap();
    let readback = read(&rewritten).unwrap();
    assert_eq!(source, readback);
    let mut source_images = Vec::new();
    let mut pdf_images = Vec::new();
    for (stage, bytes) in [FIXTURE, rewritten.as_slice()].into_iter().enumerate() {
        let normalized = extract(bytes).unwrap();
        assert_eq!(normalized.pages.len(), 2);
        let cell = &normalized.pages[0].tables[0].cells[0][0];
        assert_eq!(cell.blocks.len(), 1);
        let block = &cell.blocks[0];
        assert_eq!(block.text, "ALPHA\nBETA\nGAMMA");
        let mut offset = 0;
        let mut breaks = 0;
        for run in block.metadata["runs"].as_array().unwrap() {
            let text = run["text"].as_str().unwrap();
            assert_eq!(run["start"].as_u64().unwrap() as usize, offset);
            offset += text.chars().count();
            assert_eq!(run["end"].as_u64().unwrap() as usize, offset);
            if run["kind"] == "break" {
                assert_eq!(text, "\n");
                breaks += 1;
            }
        }
        assert_eq!((breaks, offset), (2, 16));
        assert_eq!(block.links.len(), 2);
        for (link, label, start) in [
            (&block.links[0], "ALPHA", 0),
            (&block.links[1], "GAMMA", 11),
        ] {
            assert_eq!(link.text, label);
            assert_eq!(link.metadata["start"], start);
            assert_eq!(link.metadata["end"], start + 5);
        }
        assert_eq!(normalized.pages[0].images.len(), 1);
        let pdf = to_pdf(bytes).unwrap();
        let output = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(output.pages.len(), 2);
        let page = &output.pages[0];
        let text = page
            .blocks
            .iter()
            .map(|block| match &block.table {
                Some(table) => table
                    .rows
                    .iter()
                    .map(|row| row.join("\n"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => block.text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut previous = None;
        for label in ["ALPHA", "BETA", "GAMMA"] {
            assert_eq!(text.matches(label).count(), 1, "PDF labels once: {text}");
            let position = text.find(label).unwrap();
            if let Some(before) = previous {
                assert!(before < position);
            }
            previous = Some(position);
        }
        assert_eq!(page.links.len(), 2);
        assert!(page
            .links
            .iter()
            .any(|link| link.uri == "https://example.com/line-break"));
        assert!(page
            .links
            .iter()
            .any(|link| link.page == Some(1) || link.uri == "#page=2"));
        for (i, dpi) in [72.0, 144.0, 200.0].into_iter().enumerate() {
            let native = pixels(render(bytes, 0, dpi).unwrap());
            let target = pixels(crate::pdf::render(&pdf, 0, dpi, None).unwrap());
            assert_eq!(
                (native.0, native.1),
                (
                    (576.0 * dpi / 72.0).ceil() as u32,
                    (360.0 * dpi / 72.0).ceil() as u32
                )
            );
            assert_eq!((native.0, native.1), (target.0, target.1));
            for image in [&native, &target] {
                color_order(image, &[0, 1, 2]);
                picture_ink(image, dpi);
            }
            if dpi == 200.0 {
                assert!(
                    native
                        .2
                        .iter()
                        .zip(&target.2)
                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                    "same200-grid channel bound"
                );
            }
            if stage == 0 {
                source_images.push(native);
                pdf_images.push(target);
            } else {
                assert_eq!(native, source_images[i]);
                assert_eq!(target, pdf_images[i]);
            }
        }
    }
}
