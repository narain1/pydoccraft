//! Raw public-native feasibility proof only; no extraction adapter is changed here.
use super::*;
use std::io::Cursor;
use std::sync::Arc;
use wordcraft_doc::props::{HeightRule, TextDirection, VMerge};
use wordcraft_doc::{para_block, Pos};
use wordcraft_layout::Placed;

fn paragraph(text: &str) -> Paragraph {
    let mut p = Paragraph::with_text("seed", Default::default());
    p.text = text.into();
    p.runs[0].len = text.len();
    p
}

fn setup(document: &mut Document) {
    let s = &mut document.last_section;
    s.page_w = 360.0;
    s.page_h = 300.0;
    s.margin_top = 36.0;
    s.margin_bottom = 36.0;
    s.margin_left = 36.0;
    s.margin_right = 36.0;
    s.header = 12.0;
    s.footer = 12.0;
}

fn reopened(document: &Document) -> (Document, DocLayout) {
    let document = read(&wordcraft_docx::write(document).unwrap(), None).unwrap();
    let layout = wordcraft_layout::layout(
        &document,
        &mut LayoutCache::new(),
        &LayoutOptions::default(),
    );
    (document, layout)
}

fn intervals(text: &str) -> Vec<(usize, usize)> {
    let mut intervals = Vec::new();
    let mut start = 0;
    for (off, ch) in text.char_indices() {
        if matches!(ch, '\u{c}' | '\u{e}') {
            intervals.push((start, off));
            start = off + ch.len_utf8();
        }
    }
    intervals.push((start, text.len()));
    assert_eq!(
        intervals.iter().map(|(a, b)| b - a).sum::<usize>() + intervals.len() - 1,
        text.len()
    );
    for pair in intervals.windows(2) {
        assert_eq!(pair[1].0, pair[0].1 + 1);
        assert!(matches!(text.as_bytes()[pair[0].1], 12 | 14));
    }
    intervals
}

// Only native coordinate projection: x_spans performs selection/bidi work, and turn_rect
// performs the transform. No copied caret, cluster-selection, shaping, or bidi algorithm.
fn direct_rects(
    document: &Document,
    layout: &DocLayout,
    path: &Path,
    interval: (usize, usize),
) -> Vec<(usize, [f32; 4])> {
    let p = document.para(StoryRef::Body, path).unwrap();
    assert!(p.text.is_char_boundary(interval.0));
    assert!(p.text.is_char_boundary(interval.1));
    let mut output = Vec::new();
    for (pi, page) in layout.pages.iter().enumerate() {
        for item in &page.items {
            let Placed::Lines {
                story,
                path: q,
                para,
                l0,
                l1,
                x,
                y,
                turn,
            } = item
            else {
                continue;
            };
            if *story != StoryRef::Body || q != path {
                continue;
            }
            let first = &para.lines[*l0];
            assert!(*l0 < *l1 && *l1 <= para.lines.len());
            for li in *l0..*l1 {
                let line = &para.lines[li];
                assert!(line.start <= line.stop && line.stop <= p.text.len());
                assert!(p.text.is_char_boundary(line.start));
                assert!(p.text.is_char_boundary(line.stop));
                assert!(line.c0 <= line.c1 && line.c1 <= para.clusters.len());
                let from = line.start.max(interval.0);
                let to = line.stop.min(interval.1);
                if from > to {
                    continue;
                }
                for (a, b) in para.x_spans(li, from, to) {
                    if b <= a {
                        continue;
                    }
                    // Reuse the native Rect value's concrete type, without adding a crate.
                    let mut r = page.body;
                    r.x = a;
                    r.y = line.top - first.top;
                    r.w = b - a;
                    r.h = line.height;
                    let r = wordcraft_layout::turn_rect(*turn, *x, *y, r);
                    let rect = [r.x, r.y, r.right(), r.bottom()];
                    assert!(rect.iter().all(|v| v.is_finite()));
                    output.push((pi, rect));
                }
            }
        }
    }
    output
}

fn assert_selection_parity(document: &Document, layout: &DocLayout, path: &Path) {
    let p = document.para(StoryRef::Body, path).unwrap();
    for interval in intervals(&p.text) {
        let expected: Vec<_> = layout
            .selection_rects(
                document,
                &Pos::new(StoryRef::Body, path.clone(), interval.0),
                &Pos::new(StoryRef::Body, path.clone(), interval.1),
                0,
            )
            .into_iter()
            .filter(|(_, r)| r.w > 0.0)
            .map(|(page, r)| (page, [r.x, r.y, r.right(), r.bottom()]))
            .collect();
        assert_eq!(direct_rects(document, layout, path, interval), expected);
    }
}

fn blue_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0, 0, 255, 255].repeat(4))
            .unwrap();
    }
    bytes
}

#[test]
fn provenance_raw_utf8_break_object_ranges_and_native_selection_agree() {
    let mut doc = Document::new();
    setup(&mut doc);
    doc.last_section.columns.count = 2;
    doc.last_section.columns.space = 12.0;
    let mut p = paragraph("λ🙂 before\u{c}after page\u{e}after column\u{c}\u{e}tail");
    p.insert_object(
        0,
        InlineObject::Field {
            instr: "PAGE".into(),
            result: "CACHED VALUE".into(),
            locked: false,
            dirty: false,
            code: false,
            form: String::new(),
        },
        &Default::default(),
    )
    .unwrap();
    p.insert_object(
        OBJ.len_utf8(),
        InlineObject::Image {
            media: "image.png".into(),
            w: 10.0,
            h: 10.0,
            alt: "Blue".into(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: None,
        },
        &Default::default(),
    )
    .unwrap();
    doc.media.insert("image.png".into(), blue_png().into());
    let mut rtl = paragraph("עברית English العربية λ🙂");
    rtl.props.bidi = Some(true);
    doc.body = vec![
        para_block(p),
        para_block(rtl),
        para_block(paragraph(&"Automatic wrap Ω🙂 words ".repeat(120))),
    ];
    let (doc, layout) = reopened(&doc);
    assert!(layout.pages.len() >= 3);
    let raw = &doc.para(StoryRef::Body, &Path(vec![0])).unwrap().text;
    assert_eq!(raw.chars().filter(|ch| *ch == OBJ).count(), 2);
    assert_eq!(intervals(raw).len(), 5);
    let adapter = Adapter::new(&doc, &layout).unwrap();
    let segments =
        adapter.paragraph_segments(doc.para(StoryRef::Body, &Path(vec![0])).unwrap(), &[0]);
    assert_ne!(segments[0].text.len(), intervals(raw)[0].1);
    assert!(segments[0].text.contains("CACHED VALUE"));
    for path in doc.para_paths(StoryRef::Body) {
        assert_selection_parity(&doc, &layout, &path);
    }
    let automatic = layout.index.get(&(StoryRef::Body, Path(vec![2]))).unwrap();
    assert!(automatic.first().unwrap().0 < automatic.last().unwrap().0);
    assert!(layout.pages.iter().flat_map(|p| &p.items).any(|item| {
        matches!(item, Placed::Lines { para, .. } if para.lines.iter().any(|line| !line.vis.is_empty()))
    }));
}

#[test]
fn provenance_raw_cells_repetition_turns_and_negative_paint_controls() {
    let mut doc = Document::new();
    setup(&mut doc);
    let mut table = Table::new(9, 3, 288.0);
    for (ri, row) in table.rows.iter_mut().enumerate() {
        row.props.height = Some(65.0);
        row.props.height_rule = HeightRule::AtLeast;
        row.props.cant_split = true;
        for (ci, cell) in row.cells.iter_mut().enumerate() {
            cell.blocks = vec![para_block(paragraph(&format!("Row {ri} cell {ci}")))];
        }
    }
    table.rows[0].props.header = true;
    table.rows[1].cells[0].props.text_direction = TextDirection::Down;
    table.rows[1].cells[1].props.text_direction = TextDirection::Up;
    table.rows[2].cells[0].props.vmerge = VMerge::Restart;
    table.rows[3].cells[0].props.vmerge = VMerge::Continue;
    table.rows[3].cells[0].blocks = vec![para_block(Paragraph::new())];
    let mut nested = Table::new(1, 1, 70.0);
    nested.rows[0].cells[0].blocks = vec![para_block(paragraph("Nested Ω"))];
    table.rows[4].cells[0].blocks =
        vec![Arc::new(Block::Table(nested)), para_block(Paragraph::new())];
    table.rows[5].props.height_rule = HeightRule::Exact;
    table.rows[5].props.height = Some(14.0);
    table.rows[5].cells[0].blocks = vec![para_block(paragraph(&"Clipped words ".repeat(30)))];
    let mut cap = paragraph("Dropped capital text is a paint control.");
    cap.props.drop_cap = Some(3);
    doc.body = vec![Arc::new(Block::Table(table)), para_block(cap)];
    let (doc, layout) = reopened(&doc);
    for path in doc.para_paths(StoryRef::Body) {
        assert_selection_parity(&doc, &layout, &path);
    }
    let repeat = layout
        .index
        .get(&(StoryRef::Body, Path(vec![0, 0, 0, 0])))
        .unwrap();
    assert!(repeat.first().unwrap().0 < repeat.last().unwrap().0);
    assert!(layout
        .index
        .contains_key(&(StoryRef::Body, Path(vec![0, 4, 0, 0, 0, 0, 0]))));
    assert!(layout
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .any(|it| matches!(it, Placed::Lines { turn, .. } if turn.is_turned())));
    assert!(layout
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .any(|it| matches!(it, Placed::Cell { clip: true, .. })));
    assert!(layout
        .pages
        .iter()
        .flat_map(|p| &p.items)
        .any(|it| matches!(it, Placed::Lines { para, .. } if para.drop_cap.is_some())));
    // A selection is a logical extent, not a clipped-ink assertion. Establish this with a
    // real exact-height source row rather than equating matching selection APIs with paint.
    let clipped = Path(vec![0, 5, 0, 0]);
    let p = doc.para(StoryRef::Body, &clipped).unwrap();
    let rects = direct_rects(&doc, &layout, &clipped, (0, p.text.len()));
    assert!(rects
        .iter()
        .any(|(pi, r)| layout.pages[*pi].items.iter().any(|it| {
            matches!(it, Placed::Cell { table, row: 5, cell: 0, rect, clip: true, .. }
            if table == &Path(vec![0]) && r[3] > rect.bottom())
        })));
}

#[test]
fn provenance_raw_story_identity_and_simple_render_pdf_controls() {
    let mut doc = Document::from_text("Visible source text\nSecond page source");
    setup(&mut doc);
    let Block::Para(second) = Arc::make_mut(&mut doc.body[1]) else {
        unreachable!()
    };
    second.props.page_break_before = Some(true);
    let header = doc.add_part(
        PartKind::Header,
        vec![para_block(paragraph("Repeated header"))],
    );
    let footer = doc.add_part(
        PartKind::Footer,
        vec![para_block(paragraph("Repeated footer"))],
    );
    let note = doc.add_part(
        PartKind::Footnote,
        vec![para_block(paragraph("Note source"))],
    );
    let comment = doc.add_part(
        PartKind::Comment,
        vec![para_block(paragraph("Comment source"))],
    );
    let textbox = doc.add_part(PartKind::TextBox, vec![para_block(paragraph("Box source"))]);
    doc.last_section.headers.default = Some(header);
    doc.last_section.footers.default = Some(footer);
    doc.comments.insert(
        7,
        wordcraft_doc::Comment {
            part: comment,
            author: "Proof".into(),
            ..Default::default()
        },
    );
    let Block::Para(first) = Arc::make_mut(&mut doc.body[0]) else {
        unreachable!()
    };
    for object in [
        InlineObject::CommentStart { id: 7 },
        InlineObject::CommentEnd { id: 7 },
    ] {
        first
            .insert_object(first.text.len(), object, &Default::default())
            .unwrap();
    }
    first
        .insert_object(
            first.text.len(),
            InlineObject::NoteRef {
                id: note,
                kind: wordcraft_doc::para::NoteKind::Footnote,
                custom: String::new(),
            },
            &Default::default(),
        )
        .unwrap();
    first
        .insert_object(
            first.text.len(),
            InlineObject::Shape {
                kind: wordcraft_doc::para::ShapeKind::TextBox,
                w: 100.0,
                h: 25.0,
                fill: None,
                stroke: None,
                stroke_width: 0.0,
                float: Default::default(),
                story: Some(textbox),
                freeform: None,
                effects: Default::default(),
                extra: Default::default(),
            },
            &Default::default(),
        )
        .unwrap();
    // Comment parts have no ordinary page layout; preserving their source path must not
    // manufacture a body line when the same [0] exists in both stories.
    assert_eq!(
        doc.para(StoryRef::Part(comment), &Path(vec![0]))
            .unwrap()
            .text,
        "Comment source"
    );
    let (doc, layout) = reopened(&doc);
    assert_eq!(layout.pages.len(), 2);
    for kind in [
        PartKind::Header,
        PartKind::Footer,
        PartKind::Footnote,
        PartKind::TextBox,
        PartKind::Comment,
    ] {
        let (&id, part) = doc
            .parts
            .iter()
            .find(|(_, part)| part.kind == kind)
            .unwrap();
        assert_eq!(part.blocks.len(), 1);
        assert!(doc.para(StoryRef::Part(id), &Path(vec![0])).is_some());
        if matches!(
            kind,
            PartKind::Header | PartKind::Footer | PartKind::Comment
        ) {
            assert!(!layout
                .index
                .contains_key(&(StoryRef::Part(id), Path(vec![0]))));
        }
    }
    for page in &layout.pages {
        assert!(!page.header.is_empty() && !page.footer.is_empty());
        assert!(page
            .header
            .iter()
            .any(|it| matches!(it, Placed::Lines { path, .. } if path == &Path(vec![0]))));
    }
    assert!(layout
        .index
        .keys()
        .any(|(story, path)| *story != StoryRef::Body && path == &Path(vec![0])));
    for path in doc.para_paths(StoryRef::Body) {
        assert_selection_parity(&doc, &layout, &path);
    }
    let body = doc.para(StoryRef::Body, &Path(vec![0])).unwrap();
    let rects = direct_rects(
        &doc,
        &layout,
        &Path(vec![0]),
        (0, "Visible source text".len()),
    );
    assert!(!rects.is_empty());
    assert!(body.text.starts_with("Visible source text"));
    for dpi in [72.0, 144.0] {
        let bytes = render_with(&doc, &layout, 0, dpi, None).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.width, (360.0 * dpi / 72.0) as u32);
        for (_, r) in &rects {
            let s = dpi as f32 / 72.0;
            let mut dark = 0;
            for y in
                (r[1] * s).floor().max(0.0) as u32..(r[3] * s).ceil().min(info.height as f32) as u32
            {
                for x in (r[0] * s).floor().max(0.0) as u32
                    ..(r[2] * s).ceil().min(info.width as f32) as u32
                {
                    let offset = ((y * info.width + x) * 4) as usize;
                    dark += usize::from(pixels[offset..offset + 3].iter().all(|v| *v < 100));
                }
            }
            assert!(dark > 10);
        }
    }
    let pdf = wordcraft_pdf::export(&doc, &wordcraft_pdf::PdfOptions::default()).unwrap();
    let payload = crate::pdf::extract(&pdf, None).unwrap();
    assert_eq!(payload.pages.len(), 2);
    let text: String = payload.pages[0]
        .blocks
        .iter()
        .map(|b| b.text.as_str())
        .collect();
    assert!(text.contains("Visible source text"));
    assert!(payload.pages[0]
        .blocks
        .iter()
        .filter(|b| b.text.contains("Visible source text"))
        .all(|b| b.bbox.is_some_and(|b| rects.iter().any(|(_, r)| {
            b[0] < f64::from(r[2])
                && b[2] > f64::from(r[0])
                && b[1] < f64::from(r[3])
                && b[3] > f64::from(r[1])
        }))));
}
