//! Spec137 native tab proofs and bounded owned-cell adapter acceptance.
use super::*;
use deckcraft_model::text::{
    Bullet, LevelStyle, Paragraph as NativeParagraph, TabStop as NativeTabStop,
};
use std::collections::HashMap;

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
    // Normalize baseline defaults once; the tab fixture then sets its explicit native appearance.
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

fn page_text(bytes: Vec<u8>) -> String {
    use pdfcraft_cos::Object;
    let pdf = pdfcraft_cos::Document::open(Arc::new(bytes)).unwrap();
    let pages = pdfcraft_model::pages(&pdf);
    assert_eq!(pages.len(), 2);
    let resources = pdf.resolve(pages[0].dict.get(b"Resources").unwrap());
    let resources = resources.as_dict().unwrap();
    let fonts = pdf.resolve(resources.get(b"Font").unwrap());
    let fonts = fonts.as_dict().unwrap();
    assert!(
        !fonts.is_empty() && fonts.len() <= 8,
        "bounded fixture fonts"
    );
    eprintln!("tab PDF fixture emitted {} fonts", fonts.len());
    let mut mappings = HashMap::new();
    for (name, font) in fonts.iter() {
        let font = pdf.resolve(font);
        let font = font.as_dict().unwrap();
        let cmap = pdf.resolve(font.get(b"ToUnicode").unwrap());
        let Object::Stream(cmap) = cmap.as_ref() else {
            panic!("ToUnicode stream")
        };
        let cmap = cmap.decoded_within(64 * 1024).unwrap();
        let cmap = std::str::from_utf8(&cmap).unwrap();
        assert!(cmap.len() < 64 * 1024);
        let mut mapping = HashMap::<u16, String>::new();
        let mut in_char_map = false;
        for line in cmap.lines() {
            if line.ends_with("beginbfchar") {
                in_char_map = true;
            } else if line == "endbfchar" {
                in_char_map = false;
            } else if in_char_map {
                let parts: Vec<_> = line.split_whitespace().collect();
                assert_eq!(parts.len(), 2);
                let cid = u16::from_str_radix(parts[0].trim_matches(['<', '>']), 16).unwrap();
                let destination = parts[1].trim_matches(['<', '>']);
                assert!(!destination.is_empty() && destination.len().is_multiple_of(4));
                let units: Vec<_> = destination
                    .as_bytes()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|chunk| {
                        u16::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16).unwrap()
                    })
                    .collect();
                assert!(mapping.len() < 1024);
                mapping.insert(cid, String::from_utf16(&units).unwrap());
            }
        }
        mappings.insert(name.clone(), mapping);
    }

    let content = pdf.resolve(pages[0].dict.get(b"Contents").unwrap());
    let items = match content.as_ref() {
        Object::Array(items) => items.clone(),
        Object::Stream(_) => vec![content.as_ref().clone()],
        _ => panic!("contents"),
    };
    assert!(!items.is_empty() && items.len() <= 8);
    let mut extracted = String::new();
    let mut current_font: Option<Vec<u8>> = None;
    let append = |bytes: &[u8], mapping: &HashMap<u16, String>, out: &mut String| {
        assert!(bytes.len() < 4096 && bytes.len().is_multiple_of(2));
        for pair in bytes.as_chunks::<2>().0 {
            out.push_str(mapping.get(&u16::from_be_bytes(*pair)).unwrap());
        }
    };
    for item in items {
        let item = pdf.resolve(&item);
        let Object::Stream(stream) = item.as_ref() else {
            panic!("content stream")
        };
        let decoded = stream.decoded_within(1024 * 1024).unwrap();
        let parsed = pdfcraft_content::parse(&decoded);
        assert_eq!(parsed.skipped, 0);
        assert!(parsed.ops.len() < 10000);
        for op in parsed.ops {
            if op.is("Tf") {
                let name = op.name(0).unwrap();
                assert!(mappings.contains_key(name));
                current_font = Some(name.to_vec());
            }
            if op.is("Tj") {
                let Object::String(value) = &op.operands[0] else {
                    panic!("Tj string")
                };
                append(
                    &value.bytes,
                    mappings.get(current_font.as_ref().unwrap()).unwrap(),
                    &mut extracted,
                );
            }
            if op.is("TJ") {
                let Object::Array(items) = &op.operands[0] else {
                    panic!("TJ array")
                };
                for item in items {
                    if let Object::String(value) = item {
                        append(
                            &value.bytes,
                            mappings.get(current_font.as_ref().unwrap()).unwrap(),
                            &mut extracted,
                        );
                    }
                }
            }
            assert!(
                !op.is("'") && !op.is("\""),
                "fixture uses explicit show-text operations"
            );
        }
    }
    assert!(extracted.len() < 8192);
    extracted
}

const LABELS: [&str; 3] = ["AA\t123.45", "BB\t78.90", "CC\t987"];

fn stop(pos: f64, align: &str) -> NativeTabStop {
    NativeTabStop {
        pos,
        align: align.into(),
    }
}

fn tab_fixture(merged: u8) -> (EditablePptx, u32) {
    let (mut owner, id) = fixture(merged, false);
    let ShapeKind::Table(table) = &mut shape_mut(owner.presentation.as_mut().unwrap(), 0, id)
        .unwrap()
        .kind
    else {
        panic!("table")
    };
    table.style = "none".into();
    let cell = table.cell_mut(0, 0).unwrap();
    cell.margins = Some([0.0; 4]);
    cell.fill = Some(solid((255, 255, 255)));
    for (i, p) in cell.text.paragraphs.iter_mut().enumerate() {
        p.props.bullet = Some(Bullet::None);
        p.props.align = Some(Align::Left);
        p.props.margin_left = Some(0.0);
        p.props.margin_right = Some(0.0);
        p.props.indent = Some(0.0);
        p.props.line_spacing = Some(Spacing::Pct(1.0));
        p.props.space_before = Some(Spacing::Pts(0.0));
        p.props.space_after = Some(Spacing::Pts(0.0));
        p.runs[0].text = LABELS[i].into();
        p.runs[0].props.size = Some(18.0);
        p.runs[0].props.fill = Some(solid([(224, 0, 0), (0, 160, 0), (0, 0, 224)][i]));
        p.runs[0].props.underline = Some("none".into());
    }
    owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
    owner.refresh_budgets().unwrap();
    (owner, id)
}

fn body(owner: &EditablePptx, id: u32) -> &TextBody {
    &owner.table(0, id).unwrap().cell(0, 0).unwrap().text
}

fn body_mut(owner: &mut EditablePptx, id: u32) -> &mut TextBody {
    &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
        .unwrap()
        .text
}

fn tabs(owner: &mut EditablePptx, id: u32, value: Option<Vec<NativeTabStop>>) {
    for p in &mut body_mut(owner, id).paragraphs {
        p.props.tabs = value.clone();
    }
}

fn inherited(owner: &mut EditablePptx, id: u32, value: Vec<NativeTabStop>) {
    for level in 0..3 {
        let mut style = body(owner, id)
            .list_style
            .level(level)
            .cloned()
            .unwrap_or_default();
        style.para.tabs = Some(value.clone());
        body_mut(owner, id).list_style.set(level, style);
    }
}

fn roundtrip(owner: &EditablePptx) -> EditablePptx {
    // Raw writer-limit controls deliberately bypass the owned adapter guard.
    // Original executed helper/source remain archived in spec137 evidence.
    let source = deckcraft_pptx::export(owner.document().unwrap()).unwrap();
    let mut out = owner.clone();
    out.presentation = Some(deckcraft_pptx::import(&source).unwrap());
    out
}

fn lines(owner: &EditablePptx, first: &str) -> Vec<Vec<(f64, f64)>> {
    let placed = deckcraft_render::place_slide(owner.document().unwrap(), 0);
    let text = placed
        .texts
        .iter()
        .find(|t| t.body.paragraphs.first().is_some_and(|p| p.text() == first))
        .unwrap();
    assert!(text.layout.runs.iter().any(|r| !r.glyphs.is_empty()));
    for run in &text.layout.runs {
        assert!(run.glyphs.iter().all(|g| g.0 != 0));
        for (_, c) in &run.cells {
            if !c.is_whitespace() {
                assert!(run.face.covers(*c));
            }
        }
    }
    text.layout
        .lines
        .iter()
        .map(|l| {
            assert!(!l.rtl);
            assert!(l.edges.iter().all(|(a, b)| a.is_finite() && b.is_finite()));
            l.edges.clone()
        })
        .collect()
}

fn close(a: f64, b: f64) {
    assert!(
        (a - b).abs() < 0.0001,
        "native glyph boundary {a} != expected tab anchor {b}"
    );
}

#[test]
fn pptx_cell_tabs_raw_model_reset_inheritance_quantization_and_limits() {
    let mut cases = 0;
    for merged in 0..4 {
        for alignment in ["l", "ctr", "r", "dec"] {
            let (mut original, id) = tab_fixture(merged);
            inherited(&mut original, id, vec![stop(120.0, "l")]);
            // Canonicalize inherited native defaults before the only changed property.
            original = roundtrip(&original);
            for direct in [None, Some(vec![]), Some(vec![stop(140.0, alignment)])] {
                let mut owner = original.copy().unwrap();
                tabs(&mut owner, id, direct.clone());
                let before = owner.document().unwrap().clone();
                let read = roundtrip(&owner);
                assert_eq!(read.document().unwrap(), &before);
                assert_eq!(body(&read, id).paragraphs[0].props.tabs, direct);
                let mut restored = read.copy().unwrap();
                tabs(
                    &mut restored,
                    id,
                    body(&original, id).paragraphs[0].props.tabs.clone(),
                );
                assert_eq!(restored.document().unwrap(), original.document().unwrap());
                cases += 1;
            }
        }
    }
    let (mut owner, id) = tab_fixture(0);
    let points = [
        -2_147_483_647.0,
        -1.0,
        0.0,
        1.0 / 12700.0,
        1.23456,
        2_147_483_647.0,
    ];
    tabs(
        &mut owner,
        id,
        Some(points.iter().map(|p| stop(*p, "l")).collect()),
    );
    let read = roundtrip(&owner);
    let got = body(&read, id).paragraphs[0].props.tabs.as_ref().unwrap();
    assert_eq!(got.len(), points.len());
    for (source, actual) in points.iter().zip(got) {
        assert_eq!(actual.pos, (source * 12700.0).round() / 12700.0);
    }
    for count in [0, 1, 32, 33, 64] {
        tabs(
            &mut owner,
            id,
            Some((0..count).map(|i| stop(f64::from(i), "l")).collect()),
        );
        let read = roundtrip(&owner);
        let actual = body(&read, id).paragraphs[0].props.tabs.as_ref().unwrap();
        assert_eq!(actual.len(), count.min(32) as usize);
        eprintln!(
            "spec137 native writer count source={count} readback={} (above32 excluded)",
            actual.len()
        );
    }
    for supplied in [
        vec![stop(150.0, "l"), stop(100.0, "r")],
        vec![stop(100.0, "l"), stop(100.0, "r")],
        vec![stop(1e-8, "l"), stop(2e-8, "r")],
        vec![stop(-2_147_483_648.0, "l"), stop(2_147_483_648.0, "l")],
    ] {
        tabs(&mut owner, id, Some(supplied.clone()));
        let read = roundtrip(&owner);
        let actual = body(&read, id).paragraphs[0].props.tabs.clone();
        eprintln!(
            "spec137 unordered/duplicate/clamp DIAGNOSTIC source={supplied:?} readback={actual:?}"
        );
        assert_eq!(actual.as_ref().unwrap().len(), supplied.len());
        for (expected, actual) in supplied.iter().zip(actual.as_ref().unwrap()) {
            assert_eq!(
                actual.pos,
                ((expected.pos * 12700.0).round() / 12700.0)
                    .clamp(-2_147_483_647.0, 2_147_483_647.0)
            );
            assert_eq!(actual.align, expected.align);
        }
    }
    // Existing structural Break and an authored empty paragraph retain reset semantics.
    let (mut carriers, id) = tab_fixture(0);
    body_mut(&mut carriers, id).paragraphs[0].runs.clear();
    let end = body(&carriers, id).paragraphs[1].char_len();
    assert_eq!(
        deckcraft_model::edit::insert(body_mut(&mut carriers, id), (1, end), "\u{b}", None),
        (1, end + 1)
    );
    inherited(&mut carriers, id, vec![stop(120.0, "l")]);
    carriers = roundtrip(&carriers);
    let baseline = carriers.document().unwrap().clone();
    for direct in [None, Some(vec![]), Some(vec![stop(140.0, "l")]), None] {
        tabs(&mut carriers, id, direct.clone());
        let read = roundtrip(&carriers);
        assert_eq!(read.document().unwrap(), carriers.document().unwrap());
        assert_eq!(body(&read, id).paragraphs.len(), 3);
        assert!(body(&read, id).paragraphs[0].runs.is_empty());
        assert_eq!(
            body(&read, id).paragraphs[1].runs.last().unwrap().kind,
            deckcraft_model::text::RunKind::Break
        );
        assert_eq!(
            body(&read, id).paragraphs[1].text().chars().last(),
            Some('\u{b}')
        );
        for p in &body(&read, id).paragraphs {
            assert_eq!(p.props.tabs, direct);
        }
    }
    assert_eq!(carriers.document().unwrap(), &baseline);

    // Ordinary HT insertion is already public-reachable; Unicode positions remain characters.
    for initial in ["", "A😀e\u{301}Z", "\t", "A\t", "A\t\tZ"] {
        for at in [0, initial.chars().count()] {
            let mut native = TextBody::default();
            native.paragraphs.push(NativeParagraph::new(initial));
            native.paragraphs[0].end_props.bold = Some(true);
            let before = native.clone();
            let caret = deckcraft_model::edit::insert(&mut native, (0, at), "\t", None);
            assert_eq!(caret, (0, at + 1));
            assert_eq!(native.paragraphs.len(), 1);
            assert_eq!(native.paragraphs[0].text().chars().nth(at), Some('\t'));
            deckcraft_model::edit::delete(&mut native, (0, at), caret);
            assert_eq!(native.paragraphs[0].text(), before.paragraphs[0].text());
            assert_eq!(native.paragraphs[0].props, before.paragraphs[0].props);
            assert_eq!(
                native.paragraphs[0].end_props,
                before.paragraphs[0].end_props
            );
        }
    }
    eprintln!(
        "spec137 raw model {cases} inherited/direct/reset profiles; bounded count/quantization/HT controls"
    );
}

fn output(owner: &EditablePptx, id: u32, labels: &[&str; 3], label: &str, normalized_order: bool) {
    let before = owner.document().unwrap().clone();
    let source = owner.to_bytes().unwrap();
    let read = roundtrip(owner);
    assert_eq!(read.document().unwrap(), &before);
    assert_eq!(lines(owner, labels[0]), lines(&read, labels[0]));
    let rewritten = read.to_bytes().unwrap();
    let pdf = crate::pptx::to_pdf(&source).unwrap();
    let emitted = page_text(pdf.clone());
    let model = crate::pdf::extract(&pdf, None).unwrap();
    let normalized = model.pages[0]
        .blocks
        .iter()
        .map(|b| {
            b.table.as_ref().map_or_else(
                || b.text.clone(),
                |t| {
                    t.rows
                        .iter()
                        .map(|r| r.join(" "))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    eprintln!("spec137 output {label} PDF_CMap={emitted:?} normalized={normalized:?}");
    let mut shown_cursor = 0;
    let mut normalized_cursor = 0;
    for label in labels {
        for chunk in label.split_whitespace() {
            assert_eq!(
                emitted.matches(chunk).count(),
                1,
                "actual shown Unicode chunk {chunk:?}"
            );
            assert_eq!(
                normalized.matches(chunk).count(),
                1,
                "actual normalized chunk {chunk:?}"
            );
            let shown = emitted.find(chunk).unwrap();
            let normal = normalized.find(chunk).unwrap();
            assert!(shown >= shown_cursor, "native emitted source label order");
            // Wide gaps use the native PDF reader's column order, also observed
            // with two existing default HTs. Only narrow/default controls assert
            // normalized paragraph order; every profile retains exact token counts.
            if normalized_order {
                assert!(
                    normal >= normalized_cursor,
                    "narrow/default normalized order"
                );
            }
            shown_cursor = shown + chunk.len();
            normalized_cursor = normal + chunk.len();
        }
    }
    assert!(model.pages[0]
        .links
        .iter()
        .any(|l| l.uri == "https://example.com/first"));
    assert!(model.pages[0]
        .links
        .iter()
        .any(|l| l.page == Some(1) || l.uri == "#page=2"));
    let extracted = crate::pptx::extract(&source).unwrap();
    let blocks = &extracted.pages[0].tables[0].cells[0][0].blocks;
    assert_eq!(
        blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>(),
        labels
    );
    for i in [0, 2] {
        assert_eq!(blocks[i].links.len(), 1);
        assert_eq!(blocks[i].links[0].text, labels[i]);
        assert_eq!(blocks[i].links[0].metadata["start"].as_u64(), Some(0));
        assert_eq!(
            blocks[i].links[0].metadata["end"].as_u64(),
            Some(labels[i].chars().count() as u64)
        );
    }
    for dpi in [72.0, 144.0, 200.0] {
        let a = pixels(crate::pptx::render(&source, 0, dpi).unwrap());
        let b = pixels(crate::pptx::render(&rewritten, 0, dpi).unwrap());
        assert_eq!(a, b);
        let exported = pixels(crate::pdf::render(&pdf, 0, dpi, None).unwrap());
        assert_eq!(
            (a.0, a.1, a.2.len()),
            (exported.0, exported.1, exported.2.len())
        );
        color_order(&a, &[0, 1, 2]);
        color_order(&exported, &[0, 1, 2]);
        picture_ink(&a, dpi);
        picture_ink(&exported, dpi);
        if dpi == 200.0 {
            assert!(a
                .2
                .iter()
                .zip(&exported.2)
                .all(|(a, b)| a.abs_diff(*b) <= 1));
        }
    }
    assert_eq!(body(owner, id).paragraphs.len(), 3);
    assert_eq!(owner.document().unwrap(), &before);
}

#[test]
fn pptx_cell_tabs_raw_visible_anchors_reset_and_default_fallback() {
    let mut cases = 1;
    let (default, default_id) = tab_fixture(0);
    output(
        &default,
        default_id,
        &LABELS,
        "existing default72 baseline",
        true,
    );
    for merged in [0, 3] {
        for align in ["l", "ctr", "r", "dec"] {
            let (mut owner, id) = tab_fixture(merged);
            tabs(&mut owner, id, Some(vec![stop(140.0, align)]));
            let edges = lines(&owner, LABELS[0]);
            assert_eq!(edges.len(), 3);
            for (i, line) in edges.iter().enumerate() {
                assert_eq!(line.len(), LABELS[i].chars().count());
                match align {
                    "l" => close(line[3].0, 140.0),
                    "r" => close(line.last().unwrap().1, 140.0),
                    "ctr" => close((line[3].0 + line.last().unwrap().1) / 2.0, 140.0),
                    "dec" => match LABELS[i].chars().position(|c| c == '.') {
                        Some(n) => close(line[n].0, 140.0),
                        None => close(line.last().unwrap().1, 140.0),
                    },
                    _ => unreachable!(),
                }
            }
            output(
                &owner,
                id,
                &LABELS,
                &format!("merged={merged} align={align}"),
                false,
            );
            cases += 1;
        }
    }
    let (mut owner, id) = tab_fixture(0);
    inherited(&mut owner, id, vec![stop(120.0, "l")]);
    owner = roundtrip(&owner);
    for (direct, expected) in [
        (None, 120.0),
        (Some(vec![stop(150.0, "l")]), 150.0),
        (Some(vec![]), 72.0),
        (None, 120.0),
    ] {
        tabs(&mut owner, id, direct);
        for line in lines(&owner, LABELS[0]) {
            close(line[3].0, expected);
        }
        output(
            &owner,
            id,
            &LABELS,
            &format!("inherited reset stop={expected}"),
            expected == 72.0,
        );
        cases += 1;
    }
    let labels = ["AA\tX\tDDD", "BB\tY\tEEE", "CC\tZ\tFFF"];
    for (p, text) in body_mut(&mut owner, id).paragraphs.iter_mut().zip(labels) {
        p.runs[0].text = text.into();
    }
    tabs(&mut owner, id, Some(vec![stop(90.0, "l")]));
    for line in lines(&owner, labels[0]) {
        close(line[3].0, 90.0);
        close(line[5].0, 144.0);
    }
    output(
        &owner,
        id,
        &labels,
        "past last custom stop uses native default72 pitch",
        false,
    );
    cases += 1;
    let (mut wrapped, id) = tab_fixture(3);
    let labels = [
        "AA\tSCARLET CHERRY",
        "BB\tFOREST EMERALD",
        "CC\tAZURE COBALT",
    ];
    for (p, text) in body_mut(&mut wrapped, id).paragraphs.iter_mut().zip(labels) {
        p.runs[0].text = text.into();
    }
    tabs(&mut wrapped, id, Some(vec![stop(400.0, "l")]));
    assert!(
        lines(&wrapped, labels[0]).len() > 3,
        "actual native natural wrapping"
    );
    output(
        &wrapped,
        id,
        &labels,
        "natural wrapping after a custom stop",
        false,
    );
    cases += 1;
    eprintln!("spec137 raw output {cases} fitting profiles at72/144/200 with independent anchors");
}

fn target(owner: &EditablePptx, id: u32, paragraph: usize) -> CellTarget {
    let (r, c, grid, generation, _) = owner.cell_paragraph_info(0, id, 0, 0).unwrap();
    (r, c, paragraph, grid, generation)
}
fn apply(owner: &mut EditablePptx, id: u32, patch: &str) -> PyResult<()> {
    let cell = target(owner, id, 0);
    owner.format_cell_paragraph(0, id, cell, patch, owner.revision)
}
fn unchanged(owner: &EditablePptx, before: &EditablePptx) {
    assert_eq!(owner.document().unwrap(), before.document().unwrap());
    assert_eq!(owner.revision, before.revision);
    assert_eq!(owner.text_bytes, before.text_bytes);
    assert_eq!(owner.table_generations, before.table_generations);
    assert_eq!(owner.cell_generations, before.cell_generations);
}

#[test]
fn pptx_cell_tabs_owned_domain_snapshot_reset_and_output() {
    let (mut owner, id) = tab_fixture(0);
    let cell = target(&owner, id, 0);
    for value in [
        -MAX_TAB_POSITION,
        0.0,
        1.0 / 12700.0,
        1.23456,
        MAX_TAB_POSITION,
    ] {
        apply(
            &mut owner,
            id,
            &format!(r#"{{"tabs":[{{"position":{value},"alignment":"decimal"}}]}}"#),
        )
        .unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_str(&owner.cell_paragraph_formatting(0, id, cell).unwrap()).unwrap();
        assert_eq!(snapshot["tabs"][0]["position"], value);
        assert_eq!(snapshot["tabs"][0]["alignment"], "decimal");
        assert_eq!(snapshot["tabs"][0]["leader"], "none");
        let read = roundtrip(&owner);
        assert_eq!(
            body(&read, id).paragraphs[0].props.tabs.as_ref().unwrap()[0].pos,
            (value * 12700.0).round() / 12700.0
        );
    }
    for value in ["[]", "null"] {
        apply(&mut owner, id, &format!(r#"{{"tabs":{value}}}"#)).unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_str(&owner.cell_paragraph_formatting(0, id, cell).unwrap()).unwrap();
        assert_eq!(
            snapshot["tabs"],
            serde_json::from_str::<serde_json::Value>(value).unwrap()
        );
        assert_eq!(
            roundtrip(&owner).document().unwrap(),
            owner.document().unwrap()
        );
    }
    for patch in [
        r#"{"tabs":[{"position":2147483648}]}"#,
        r#"{"tabs":[{"position":-2147483648}]}"#,
        r#"{"tabs":[{"position":true}]}"#,
        r#"{"tabs":[{"position":"72"}]}"#,
        r#"{"tabs":[{"position":72,"leader":"dot"}]}"#,
        r#"{"tabs":[{"position":72,"alignment":"bar"}]}"#,
        r#"{"tabs":[{"position":2},{"position":1}]}"#,
        r#"{"tabs":[{"position":1e-8},{"position":2e-8}]}"#,
        r#"{"tabs":[{"position":72,"extra":0}]}"#,
        r#"{"tabs":{}}"#,
    ] {
        let before = owner.clone();
        assert!(apply(&mut owner, id, patch).is_err(), "{patch}");
        unchanged(&owner, &before);
    }
    for count in [32, 33] {
        let patch=serde_json::json!({"space_after":4,"tabs":(0..count).map(|i|serde_json::json!({"position":i})).collect::<Vec<_>>()}).to_string();
        let before = owner.clone();
        let result = apply(&mut owner, id, &patch);
        if count == 32 {
            result.unwrap();
        } else {
            assert!(result.unwrap_err().to_string().contains("32"));
            unchanged(&owner, &before);
        }
    }
    for align in ["left", "center", "right", "decimal"] {
        let (mut owner, id) = tab_fixture(3);
        for index in 0..3 {
            let cell = target(&owner, id, index);
            owner
                .format_cell_paragraph(
                    0,
                    id,
                    cell,
                    &format!(r#"{{"tabs":[{{"position":140,"alignment":"{align}"}}]}}"#),
                    owner.revision,
                )
                .unwrap();
        }
        output(
            &owner,
            id,
            &["AA\t123.45", "BB\t78.90", "CC\t987"],
            align,
            false,
        );
    }
}

#[test]
fn pptx_cell_tabs_owned_inherited_inventory_copy_output_and_work_limits() {
    let (original, id) = tab_fixture(0);
    let invalid = Some((0..33).map(|i| stop(f64::from(i), "l")).collect::<Vec<_>>());
    for place in 0..10 {
        let mut owner = original.clone();
        let mut level = LevelStyle::default();
        level.para.tabs = invalid.clone();
        let doc = owner.presentation.as_mut().unwrap();
        match place {
            0 => doc.default_text_style.set(0, level),
            1 => Arc::make_mut(&mut doc.masters[0]).title_style.set(0, level),
            2 => Arc::make_mut(&mut doc.masters[0]).body_style.set(0, level),
            3 => Arc::make_mut(&mut doc.masters[0]).other_style.set(0, level),
            4 | 5 => {
                let mut master = doc.masters[0].as_ref().clone();
                master.other_style.set(0, level);
                if place == 4 {
                    doc.notes_master = Some(Arc::new(master));
                } else {
                    doc.handout_master = Some(Arc::new(master));
                }
            }
            6 => Arc::make_mut(&mut doc.slides[0])
                .notes
                .list_style
                .set(0, level),
            7 => body_mut(&mut owner, id).list_style.set(0, level),
            8 => {
                let mut shape = doc.slides[0]
                    .shapes
                    .iter()
                    .find(|s| matches!(s.kind, ShapeKind::Table(_)))
                    .unwrap()
                    .clone();
                shape.text = Some(TextBody::from_text("Aux"));
                shape.text.as_mut().unwrap().list_style.set(0, level);
                Arc::make_mut(&mut doc.masters[0]).shapes.push(shape);
            }
            9 => {
                let mut shape = doc.slides[0]
                    .shapes
                    .iter()
                    .find(|s| matches!(s.kind, ShapeKind::Table(_)))
                    .unwrap()
                    .clone();
                shape.text = Some(TextBody::from_text("Aux"));
                shape.text.as_mut().unwrap().list_style.set(0, level);
                Arc::make_mut(&mut doc.masters[0]).layouts[0]
                    .shapes
                    .push(shape);
            }
            _ => unreachable!(),
        }
        let before = owner.clone();
        assert!(owner.copy().err().unwrap().to_string().contains("32"));
        assert!(owner.to_bytes().unwrap_err().to_string().contains("32"));
        assert!(apply(&mut owner, id, r#"{"space_after":4}"#)
            .unwrap_err()
            .to_string()
            .contains("32"));
        unchanged(&owner, &before);
    }
    let doc = original.document().unwrap();
    let exact = (0..10000).find(|n| tab_inventory(doc, *n).is_ok()).unwrap();
    assert!(exact > 0);
    assert!(tab_inventory(doc, exact).is_ok());
    assert!(tab_inventory(doc, exact - 1)
        .unwrap_err()
        .to_string()
        .contains("work limit"));
    let mut owner = original.clone();
    body_mut(&mut owner, id).list_style.levels.resize(10, None);
    assert!(owner.copy().err().unwrap().to_string().contains("9 levels"));
}

#[test]
fn pptx_cell_tabs_owned_property_budget_and_split_projection() {
    let (mut owner, id) = tab_fixture(0);
    apply(&mut owner, id, r#"{"tabs":[{"position":72}]}"#).unwrap();
    let tab_cost = tabs_cost(&body(&owner, id).paragraphs[0].props.tabs).unwrap();
    assert_eq!(tab_cost, std::mem::size_of::<TabStop>() + 1);
    let cost = content_budget(owner.document().unwrap())
        .unwrap()
        .properties;
    Arc::make_mut(&mut owner.presentation.as_mut().unwrap().slides[0])
        .notes
        .paragraphs[0]
        .def_props
        .font = Some("x".repeat(MAX_PROPERTIES - cost));
    assert_eq!(
        content_budget(owner.document().unwrap())
            .unwrap()
            .properties,
        MAX_PROPERTIES
    );
    apply(&mut owner, id, r#"{"space_after":4}"#).unwrap();
    let before = owner.clone();
    assert!(apply(
        &mut owner,
        id,
        r#"{"tabs":[{"position":72},{"position":144}]}"#
    )
    .is_err());
    unchanged(&owner, &before);
    assert!(owner.set_cell_text(0, id, 0, 0, "A\nB").is_err());
    unchanged(&owner, &before);
    assert!(edited_body(
        owner.document().unwrap(),
        body(&owner, id),
        0,
        0,
        "\n",
        None,
        None
    )
    .is_err());
    unchanged(&owner, &before);
    // Empty canonical cell retains its inherited list style during insertion.
    let (mut empty, id) = tab_fixture(0);
    body_mut(&mut empty, id).paragraphs.clear();
    let mut level = LevelStyle::default();
    level.para.tabs = Some(vec![stop(72.0, "l")]);
    body_mut(&mut empty, id).list_style.set(0, level);
    let cost = content_budget(empty.document().unwrap())
        .unwrap()
        .properties;
    Arc::make_mut(&mut empty.presentation.as_mut().unwrap().slides[0])
        .notes
        .paragraphs[0]
        .def_props
        .font = Some("x".repeat(MAX_PROPERTIES - cost));
    empty.refresh_budgets().unwrap();
    empty.cell_add_paragraph(0, id, 0, 0, "New").unwrap();
    assert_eq!(
        content_budget(empty.document().unwrap())
            .unwrap()
            .properties,
        MAX_PROPERTIES
    );
    Arc::make_mut(&mut empty.presentation.as_mut().unwrap().slides[0])
        .notes
        .paragraphs[0]
        .def_props
        .font
        .as_mut()
        .unwrap()
        .push('x');
    let before = empty.clone();
    assert!(empty.cell_add_paragraph(0, id, 0, 0, "More").is_err());
    unchanged(&empty, &before);
}

#[test]
fn pptx_cell_tabs_owned_revision_aliases_and_projection_costs() {
    let (mut owner, id) = tab_fixture(3);
    let cell = target(&owner, id, 0);
    owner.revision = u64::MAX - 1;
    apply(&mut owner, id, r#"{"tabs":[{"position":72}]}"#).unwrap();
    assert_eq!(owner.revision, u64::MAX);
    let before = owner.clone();
    assert!(apply(&mut owner, id, r#"{"tabs":[]}"#).is_err());
    unchanged(&owner, &before);
    assert!(owner
        .format_cell_paragraph(
            0,
            id,
            (1, 1, 0, cell.3, cell.4),
            r#"{"tabs":[]}"#,
            owner.revision
        )
        .is_err());
    unchanged(&owner, &before);
    assert!(owner
        .format_cell_paragraph(0, id, cell, r#"{"tabs":[]}"#, owner.revision - 1)
        .is_err());
    unchanged(&owner, &before);
    let (mut owner, id) = tab_fixture(0);
    apply(&mut owner, id, r#"{"tabs":[{"position":72}]}"#).unwrap();
    let before = body_budget(body(&owner, id)).unwrap();
    let edited = edited_body(
        owner.document().unwrap(),
        body(&owner, id),
        0,
        0,
        "\n\n",
        None,
        None,
    )
    .unwrap();
    let after = body_budget(&edited).unwrap();
    assert_eq!(after.paragraphs, before.paragraphs + 2);
    assert_eq!(
        edited.paragraphs[0].props.tabs,
        edited.paragraphs[1].props.tabs
    );
    let per = tabs_cost(&body(&owner, id).paragraphs[0].props.tabs).unwrap();
    assert!(after.properties >= before.properties + 2 * per);
}
