//! Spec124 raw-only public native line-break model and actual output gate.
use super::*;
use deckcraft_model::text::{Bullet, LevelStyle, Paragraph as NativeParagraph, RunKind};

fn fixture(merged: u8, empty: bool) -> (EditablePptx, u32) {
    pyo3::prepare_freethreaded_python();
    let mut owner = EditablePptx::new(576.0, 360.0).unwrap();
    owner.add_slide("blank").unwrap();
    owner.add_slide("blank").unwrap();
    owner.set_slide_notes(0, "Retained source notes").unwrap();
    owner
        .set_slide_notes(1, "Retained destination notes")
        .unwrap();
    owner.background(0, (248, 248, 248)).unwrap();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[32, 96, 192])
            .unwrap();
    }
    owner
        .add_image(0, &png, 500.0, 300.0, 40.0, 20.0, "Shared image")
        .unwrap();
    owner
        .add_image(1, &png, 500.0, 300.0, 40.0, 20.0, "Shared image")
        .unwrap();
    let id = owner
        .add_table(
            0,
            vec![
                vec!["Initial".into(), "Neighbor".into()],
                vec!["Lower".into(), "Outside".into()],
            ],
            20.0,
            20.0,
            536.0,
            260.0,
            20.0,
        )
        .unwrap();
    match merged {
        0 => (),
        1 => owner.merge_cells(0, id, 0, 0, 0, 1).unwrap(),
        2 => owner.merge_cells(0, id, 0, 0, 1, 0).unwrap(),
        3 => owner.merge_cells(0, id, 0, 0, 1, 1).unwrap(),
        _ => panic!("profile"),
    }
    let url = make_link(
        owner.document().unwrap(),
        Some("https://example.com/first"),
        None,
        "URL tip",
    )
    .unwrap();
    let internal = make_link(
        owner.document().unwrap(),
        None,
        Some(owner.document().unwrap().slides[1].id.0),
        "Slide tip",
    )
    .unwrap();
    let cell = cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0).unwrap();
    cell.text.paragraphs = ["First office", "Middle café", "Last tail"]
        .into_iter()
        .enumerate()
        .map(|(i, label)| {
            let mut p = NativeParagraph::new(label);
            p.level = i as u8;
            p.props.align = Some([Align::Left, Align::Center, Align::Right][i]);
            p.def_props.size = Some(20.0 + i as f64);
            p.def_props.fill = Some(solid([(224, 0, 0), (0, 160, 0), (0, 0, 224)][i]));
            p.end_props.italic = Some(i == 1);
            p.runs[0].props.bold = Some(true);
            p.runs[0].props.link = [url.clone(), None, internal.clone()][i].clone();
            p
        })
        .collect();
    for i in 0..3 {
        let mut style = LevelStyle::default();
        style.run.italic = Some(i == 2);
        cell.text.list_style.set(i, style);
    }
    if empty {
        let mut blank = NativeParagraph::default();
        blank.end_props.size = Some(19.0);
        blank.end_props.bold = Some(true);
        cell.text.paragraphs.insert(1, blank);
        cell.text.paragraphs[2].runs[0].text = "Middle e\u{301} 😀 سلام".into();
    }
    // Normalize baseline defaults once; break insertion is then the only change.
    owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
    owner.refresh_budgets().unwrap();
    (owner, id)
}

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

fn insert_break(body: &mut TextBody, paragraph: usize) -> (usize, usize) {
    let start = body.paragraphs[paragraph].char_len();
    let mut props = deckcraft_model::edit::props_at(body, (paragraph, start));
    // Appearance inheritance is distinct from navigation inheritance.
    props.link = None;
    props.hover = None;
    let expected_props = props.clone();
    let end = deckcraft_model::edit::insert(body, (paragraph, start), "\u{b}", Some(props));
    assert_eq!(
        body.paragraphs[paragraph].runs.last().unwrap().props,
        expected_props
    );
    assert_eq!(end, (paragraph, start + 1));
    assert_eq!(
        deckcraft_model::edit::text_range(body, (paragraph, start), end),
        "\u{b}"
    );
    (start, end.1)
}

#[test]
fn pptx_cell_break_raw_unicode_carets_model_readback_and_deletion() {
    let mut cases = 0;
    for merged in 0..4 {
        for text in ["", "A😀e\u{301}Z"] {
            for count in 1..=3 {
                let (mut owner, id) = fixture(merged, false);
                let body = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                    .unwrap()
                    .text;
                body.paragraphs.truncate(1);
                body.paragraphs[0].runs[0].text = text.into();
                if text.is_empty() {
                    body.paragraphs[0].runs.clear();
                }
                // Normalize the authored empty/nonempty baseline before insertion.
                owner.presentation =
                    Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
                let original = owner.document().unwrap().clone();
                let body = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                    .unwrap()
                    .text;
                let original_body = body.clone();
                let source_len = text.chars().count();
                assert_eq!(body.paragraphs.len(), 1);
                for n in 0..count {
                    assert_eq!(insert_break(body, 0), (source_len + n, source_len + n + 1));
                    assert_eq!(body.paragraphs.len(), 1);
                    let break_run = body.paragraphs[0].runs.last().unwrap();
                    assert_eq!(break_run.kind, RunKind::Break);
                    assert!(break_run.text.is_empty());
                    assert!(break_run.props.link.is_none() && break_run.props.hover.is_none());
                }
                assert_eq!(
                    body.paragraphs[0].text(),
                    format!("{text}{}", "\u{b}".repeat(count))
                );
                assert_eq!(body.paragraphs[0].props, original_body.paragraphs[0].props);
                assert_eq!(
                    body.paragraphs[0].def_props,
                    original_body.paragraphs[0].def_props
                );
                assert_eq!(
                    body.paragraphs[0].end_props,
                    original_body.paragraphs[0].end_props
                );
                let tail = "😀e\u{301}";
                let start = source_len + count;
                let caret = deckcraft_model::edit::insert(body, (0, start), tail, None);
                assert_eq!(caret, (0, start + 3));
                assert_eq!(
                    deckcraft_model::edit::text_range(body, (0, start), caret),
                    tail
                );
                assert_eq!(body.paragraphs.len(), 1);
                let changed = owner.document().unwrap().clone();
                let read = deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap();
                assert_eq!(
                    read, changed,
                    "break kind, properties, links and full presentation readback"
                );
                let mut outside = changed.clone();
                cell_mut(&mut outside, 0, id, 0, 0).unwrap().text = original_body.clone();
                assert_eq!(outside, original, "unrelated model state exact");
                let body = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                    .unwrap()
                    .text;
                deckcraft_model::edit::delete(body, (0, start), caret);
                for n in (0..count).rev() {
                    let position = source_len + n;
                    assert_eq!(
                        deckcraft_model::edit::text_range(body, (0, position), (0, position + 1)),
                        "\u{b}"
                    );
                    assert_eq!(
                        deckcraft_model::edit::delete(body, (0, position), (0, position + 1)),
                        (0, position)
                    );
                    assert_eq!(body.paragraphs.len(), 1);
                }
                assert_eq!(
                    body, &original_body,
                    "exact restoration after deleting generated ranges"
                );
                assert_eq!(owner.document().unwrap(), &original);
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 24);
    eprintln!("spec124 raw model: {cases} Unicode/empty/consecutive merged-cell cases; exact insertion/deletion carets and full readback");
}

fn line_boxes(document: &Presentation, label: &str) -> Vec<(usize, usize, f64, f64, f64)> {
    let placed = deckcraft_render::place_slide(document, 0);
    let text = placed
        .texts
        .iter()
        .find(|t| {
            t.body
                .paragraphs
                .iter()
                .any(|p| p.text().starts_with(label))
        })
        .unwrap();
    assert_eq!(text.body.paragraphs.len(), 1);
    text.layout
        .lines
        .iter()
        .map(|l| {
            assert_eq!(l.para, 0);
            assert!(l.top.is_finite() && l.baseline.is_finite() && l.bottom.is_finite());
            (l.start, l.end, l.top, l.baseline, l.bottom)
        })
        .collect()
}

#[test]
fn pptx_cell_break_raw_lines_blank_rows_normalization_links_and_output() {
    let labels = ["ALPHA", "BETA", "GAMMA"];
    let mut cases = 0;
    for merged in 0..4 {
        for extra_blank in [false, true] {
            let (mut owner, id) = fixture(merged, false);
            let ShapeKind::Table(table) =
                &mut shape_mut(owner.presentation.as_mut().unwrap(), 0, id)
                    .unwrap()
                    .kind
            else {
                panic!("table")
            };
            table.style = "none".into();
            let cell = table.cell_mut(0, 0).unwrap();
            cell.margins = Some([0.0; 4]);
            let runs: Vec<_> = cell
                .text
                .paragraphs
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let mut run = p.runs[0].clone();
                    run.text = labels[i].into();
                    run.props.size = Some(20.0);
                    run.props.fill = p.def_props.fill.clone();
                    run.props.bold = Some(true);
                    run.props.underline = Some("none".into());
                    run
                })
                .collect();
            cell.text.paragraphs.truncate(1);
            let p = &mut cell.text.paragraphs[0];
            p.props.bullet = Some(Bullet::None);
            p.props.align = Some(Align::Left);
            p.props.margin_left = Some(0.0);
            p.props.margin_right = Some(0.0);
            p.props.indent = Some(0.0);
            p.props.line_spacing = Some(deckcraft_model::text::Spacing::Pct(1.0));
            p.runs = runs.clone();
            owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
            let control = owner.document().unwrap().clone();
            let control_lines = line_boxes(&control, labels[0]);
            assert_eq!(
                control_lines.len(),
                1,
                "unbroken label control fits a single native line"
            );
            let body = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                .unwrap()
                .text;
            body.paragraphs[0].runs.truncate(1);
            for (i, run) in runs.iter().enumerate().skip(1) {
                insert_break(body, 0);
                if extra_blank && i == 1 {
                    insert_break(body, 0);
                }
                let start = body.paragraphs[0].char_len();
                let end = deckcraft_model::edit::insert(
                    body,
                    (0, start),
                    &run.text,
                    Some(run.props.clone()),
                );
                assert_eq!(end, (0, start + run.char_len()));
            }
            if extra_blank {
                insert_break(body, 0);
            }
            assert_eq!(body.paragraphs.len(), 1);
            let expected = if extra_blank {
                "ALPHA\u{b}\u{b}BETA\u{b}GAMMA\u{b}"
            } else {
                "ALPHA\u{b}BETA\u{b}GAMMA"
            };
            assert_eq!(body.paragraphs[0].text(), expected);
            let bytes = owner.to_bytes().unwrap();
            let readback = deckcraft_pptx::import(&bytes).unwrap();
            assert_eq!(readback, *owner.document().unwrap());
            let boxes = line_boxes(owner.document().unwrap(), labels[0]);
            assert_eq!(boxes, line_boxes(&readback, labels[0]));
            assert_eq!(boxes.len(), if extra_blank { 5 } else { 3 });
            for pair in boxes.windows(2) {
                assert!(
                    pair[0].3 < pair[1].3,
                    "native distinct line baselines: {boxes:?}"
                );
            }
            if extra_blank {
                assert_eq!(
                    boxes[1].0, boxes[1].1,
                    "consecutive break creates an empty line"
                );
                assert_eq!(
                    boxes[4].0, boxes[4].1,
                    "trailing break creates an empty final line"
                );
            }
            let normalized = crate::pptx::extract(&bytes).unwrap();
            let block = &normalized.pages[0].tables[0].cells[0][0].blocks[0];
            assert_eq!(block.text, expected.replace('\u{b}', "\n"));
            assert_eq!(normalized.pages[0].tables[0].cells[0][0].blocks.len(), 1);
            let mut offset = 0;
            let mut breaks = 0;
            for run in block.metadata["runs"].as_array().unwrap() {
                let value = run["text"].as_str().unwrap();
                assert_eq!(run["start"].as_u64().unwrap() as usize, offset);
                offset += value.chars().count();
                assert_eq!(run["end"].as_u64().unwrap() as usize, offset);
                if run["kind"] == "break" {
                    assert_eq!(value, "\n");
                    breaks += 1;
                }
            }
            assert_eq!(breaks, if extra_blank { 4 } else { 2 });
            assert_eq!(block.links.len(), 2);
            for (link, label) in block.links.iter().zip([labels[0], labels[2]]) {
                assert_eq!(link.text, label);
                let start = block.text.find(label).unwrap();
                assert_eq!(link.metadata["start"], serde_json::json!(start));
                assert_eq!(link.metadata["end"], serde_json::json!(start + label.len()));
            }
            let pdf = crate::pptx::to_pdf(&bytes).unwrap();
            let extracted = crate::pdf::extract(&pdf, None).unwrap();
            assert_eq!(extracted.pages.len(), 2);
            assert_eq!(extracted.pages[0].links.len(), 2);
            assert!(extracted.pages[0]
                .links
                .iter()
                .any(|l| l.uri == "https://example.com/first"));
            assert!(extracted.pages[0]
                .links
                .iter()
                .any(|l| l.page == Some(1) || l.uri == "#page=2"));
            let text = extracted.pages[0]
                .blocks
                .iter()
                .map(|b| match &b.table {
                    Some(t) => t
                        .rows
                        .iter()
                        .map(|r| r.join("\n"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None => b.text.clone(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let mut last = None;
            for label in labels {
                assert_eq!(text.matches(label).count(), 1, "PDF labels once: {text}");
                let position = text.find(label).unwrap();
                if let Some(previous) = last {
                    assert!(position > previous);
                }
                last = Some(position);
            }
            for dpi in [72.0, 144.0, 200.0] {
                let opts = deckcraft_render::RenderOpts {
                    scale: dpi / 72.0,
                    ..Default::default()
                };
                let original_image =
                    crate::pptx::try_render_slide(owner.document().unwrap(), 0, &opts).unwrap();
                let read_image = crate::pptx::try_render_slide(&readback, 0, &opts).unwrap();
                assert_eq!(
                    (original_image.width, original_image.height),
                    (read_image.width, read_image.height)
                );
                assert_eq!(original_image.pixels, read_image.pixels);
                let source = pixels(crate::pptx::render(&bytes, 0, dpi).unwrap());
                let target = pixels(crate::pdf::render(&pdf, 0, dpi, None).unwrap());
                assert_eq!((source.0, source.1), (target.0, target.1));
                color_order(&source, &[0, 1, 2]);
                color_order(&target, &[0, 1, 2]);
                picture_ink(&source, dpi);
                picture_ink(&target, dpi);
                if dpi == 200.0 {
                    assert!(
                        source
                            .2
                            .iter()
                            .zip(&target.2)
                            .all(|(a, b)| a.abs_diff(*b) <= 1),
                        "same200-grid channel bound"
                    );
                }
            }
            eprintln!("spec124 raw output merged={merged} blank={extra_blank} lines={boxes:?}; normalizedLF/link offsets, three DPI PNG/PDF and full source/readback pass");
            cases += 1;
        }
    }
    assert_eq!(cases, 8);
}
