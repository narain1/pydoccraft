//! Pinned public-native mirror-margin feasibility gate; no adapter mutation.
use super::*;
use std::io::Cursor;
use std::sync::Arc;
use wordcraft_doc::para::{Anchor, Float, Wrap};
use wordcraft_doc::section::{LineNumbering, SectionProps};
use wordcraft_doc::{para_block, Pos};
use wordcraft_layout::Placed;

fn para(text: &str) -> Arc<Block> {
    para_block(Paragraph::with_text(text, Default::default()))
}
fn layout(d: &Document) -> DocLayout {
    wordcraft_layout::layout(d, &mut LayoutCache::new(), &LayoutOptions::default())
}
fn setup(d: &mut Document) {
    let s = &mut d.last_section;
    s.page_w = 300.0;
    s.page_h = 300.0;
    s.margin_left = 72.0;
    s.margin_right = 24.0;
    s.margin_top = 48.0;
    s.margin_bottom = 36.0;
    s.gutter = 8.0;
    s.header = 12.0;
    s.footer = 12.0;
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.02, "{a} != {b}");
}
fn left(d: &Document, p: &wordcraft_layout::Page, s: &SectionProps) -> f32 {
    if d.settings.mirror_margins && p.number.is_multiple_of(2) {
        s.margin_right
    } else {
        s.margin_left + s.gutter
    }
}
fn bytes(d: &Document) -> Vec<u8> {
    wordcraft_docx::write(d).unwrap()
}
fn roundtrip(d: &Document) -> Document {
    read(&bytes(d), None).unwrap()
}

#[test]
fn mirrored_margin_raw_model_sections_selection_and_flow() {
    let mut d = Document::from_text("Main one\nMain two\nAppendix one\nAppendix two");
    setup(&mut d);
    assert!(!d.settings.mirror_margins);
    let header = d.add_part(PartKind::Header, vec![para("Shared header")]);
    let footer = d.add_part(PartKind::Footer, vec![para("Shared footer")]);
    d.last_section.headers.default = Some(header);
    d.last_section.footers.default = Some(footer);
    d.last_section.page_num_start = Some(2);
    let first = d.last_section.clone();
    d.para_mut(StoryRef::Body, &Path::top(1))
        .unwrap()
        .props
        .page_break_before = Some(true);
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().section = Some(Box::new(first.clone()));
    d.para_mut(StoryRef::Body, &Path::top(3))
        .unwrap()
        .props
        .page_break_before = Some(true);
    d.last_section.margin_left = 60.0;
    d.last_section.margin_right = 30.0;
    d.last_section.gutter = 6.0;
    for (restart, even_odd) in [(7, false), (20, false), (20, true)] {
        d.settings.even_odd_headers = even_odd;
        d.last_section.page_num_start = Some(restart);
        for independent in [false, true] {
            let other = d.add_part(PartKind::Header, vec![para("Independent header")]);
            d.last_section.headers.default = if independent { Some(other) } else { None };
            for enabled in [false, true, false] {
                d.settings.mirror_margins = enabled;
                for model in [&d, &roundtrip(&d)] {
                    assert_eq!(model.settings.mirror_margins, enabled);
                    assert_eq!(model.last_section.gutter, 6.0);
                    let l = layout(model);
                    let nums: Vec<_> = l.pages.iter().map(|p| (p.section, p.number)).collect();
                    let mut wanted = vec![(0, 2), (0, 3)];
                    if even_odd && restart == 20 {
                        wanted.push((0, 4));
                    }
                    wanted.extend([(1, restart), (1, restart + 1)]);
                    assert_eq!(nums, wanted);
                    for p in &l.pages {
                        let s = if p.section == 0 {
                            &first
                        } else {
                            &model.last_section
                        };
                        let x = left(model, p, s);
                        near(p.body.x, x);
                        near(p.body.w, s.text_width());
                        for item in p.items.iter().chain(&p.header).chain(&p.footer) {
                            if let Placed::Lines {
                                x: actual, story, ..
                            } = item
                            {
                                if *story != wordcraft_layout::DECOR_STORY {
                                    near(*actual, x);
                                }
                            }
                        }
                        let h = p
                            .header
                            .iter()
                            .find_map(|it| match it {
                                Placed::Lines { story, .. } => Some(*story),
                                _ => None,
                            })
                            .unwrap();
                        assert_eq!(
                            h,
                            StoryRef::Part(if p.section == 1 && independent {
                                model.last_section.headers.default.unwrap()
                            } else {
                                model
                                    .para(StoryRef::Body, &Path::top(1))
                                    .unwrap()
                                    .section
                                    .as_ref()
                                    .unwrap()
                                    .headers
                                    .default
                                    .unwrap()
                            })
                        );
                        let StoryRef::Part(part) = h else {
                            panic!("header must be a native part")
                        };
                        let expected = if p.section == 1 && independent {
                            "Independent header"
                        } else {
                            "Shared header"
                        };
                        assert_eq!(
                            model.parts[&part].blocks[0].as_para().unwrap().text,
                            expected
                        );
                    }
                    for index in 0..4 {
                        let path = Path::top(index);
                        let p = model.para(StoryRef::Body, &path).unwrap();
                        let rs = l.selection_rects(
                            model,
                            &Pos::new(StoryRef::Body, path.clone(), 0),
                            &Pos::new(StoryRef::Body, path, p.text.len()),
                            0,
                        );
                        assert_eq!(rs.len(), 1);
                        let (pi, r) = rs[0];
                        near(r.x, l.pages[pi].body.x);
                    }
                    let payload = crate::docx::extract(&bytes(model), None).unwrap();
                    let text = payload
                        .pages
                        .iter()
                        .flat_map(|p| &p.blocks)
                        .map(|b| b.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    for label in ["Main one", "Main two", "Appendix one", "Appendix two"] {
                        assert_eq!(text.matches(label).count(), 1);
                    }
                }
            }
        }
    }
    // Natural reflow, supported single-column labels, then equal columns separately.
    let mut flowing = Document::from_text(&"Flow line\n".repeat(45));
    setup(&mut flowing);
    flowing.settings.mirror_margins = true;
    for columns in [1, 2] {
        flowing.last_section.columns.count = columns;
        flowing.last_section.columns.space = 12.0;
        flowing.last_section.line_numbers = if columns == 1 {
            Some(LineNumbering {
                distance: 12.0,
                ..Default::default()
            })
        } else {
            None
        };
        let l = layout(&flowing);
        assert!(l.pages.len() > 1);
        let step = (flowing.last_section.text_width() + 12.0) / columns as f32;
        let mut labels = 0;
        for p in &l.pages {
            let base = left(&flowing, p, &flowing.last_section);
            near(p.body.x, base);
            for item in &p.items {
                if let Placed::Lines { x, story, .. } = item {
                    if *story == StoryRef::Body {
                        assert!(
                            (*x - base).abs() < 0.02
                                || columns == 2 && (*x - base - step).abs() < 0.02
                        );
                    }
                    if *story == wordcraft_layout::DECOR_STORY {
                        labels += 1;
                        assert!(*x >= 0.0 && *x < base);
                    }
                }
            }
        }
        assert_eq!(labels > 0, columns == 1);
        let mut longer = flowing.clone();
        longer
            .body
            .extend((0..40).map(|i| para(&format!("Inserted flow {i}"))));
        let grown = layout(&longer);
        assert!(grown.pages.len() > l.pages.len());
        for p in &grown.pages {
            near(p.body.x, left(&longer, p, &longer.last_section));
        }
    }
}

fn png(color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 4, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&color.repeat(8))
            .unwrap();
    }
    out
}
fn image(d: &mut Document, name: &str, color: [u8; 4], float: Float) -> Arc<Block> {
    d.media.insert(name.into(), Arc::new(png(color)));
    let mut p = Paragraph::new();
    p.insert_object(
        0,
        InlineObject::Image {
            media: name.into(),
            w: 24.0,
            h: 12.0,
            alt: name.into(),
            float,
            crop: [0.0; 4],
            ole: None,
        },
        &Default::default(),
    )
    .unwrap();
    para_block(p)
}
fn color_box(bytes: &[u8], color: [u8; 4]) -> (u32, u32, u32, u32) {
    let mut r = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut pixels = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let mut bounds = (u32::MAX, u32::MAX, 0, 0);
    let mut n = 0;
    for (i, p) in pixels[..info.buffer_size()].chunks_exact(4).enumerate() {
        if p == color {
            let x = i as u32 % info.width;
            let y = i as u32 / info.width;
            bounds.0 = bounds.0.min(x);
            bounds.1 = bounds.1.min(y);
            bounds.2 = bounds.2.max(x + 1);
            bounds.3 = bounds.3.max(y + 1);
            n += 1;
        }
    }
    assert!(n > 10);
    bounds
}
#[test]
fn mirrored_margin_raw_png_pdf_tables_and_picture_anchors() {
    let mut d = Document::from_text("");
    setup(&mut d);
    d.body.clear();
    let header_image = image(&mut d, "header.png", [255, 0, 0, 255], Float::default());
    let header = d.add_part(PartKind::Header, vec![header_image]);
    let footer = d.add_part(PartKind::Footer, vec![para("Footer marker")]);
    d.last_section.headers.default = Some(header);
    d.last_section.footers.default = Some(footer);
    for page in 0..2 {
        let mut p = Paragraph::with_text(&format!("Body marker {page}"), Default::default());
        p.props.page_break_before = Some(page == 1);
        d.body.push(para_block(p));
        for (name, color, anchor, x, y) in [
            ("margin.png", [0, 0, 255, 255], Anchor::Margin, 4.0, 90.0),
            ("page.png", [0, 255, 0, 255], Anchor::Page, 220.0, 120.0),
            (
                "paragraph.png",
                [128, 0, 128, 255],
                Anchor::Paragraph,
                12.0,
                150.0,
            ),
        ] {
            let object = image(
                &mut d,
                name,
                color,
                Float {
                    wrap: Wrap::BehindText,
                    h_rel: anchor,
                    v_rel: Anchor::Page,
                    x,
                    y,
                    ..Default::default()
                },
            );
            d.body.push(object);
        }
        let mut table = Table::new(1, 1, 100.0);
        table.rows[0].cells[0].blocks = vec![para(&format!("Table marker {page}"))];
        d.body.push(Arc::new(Block::Table(table)));
    }
    for margin in [72.0, 60.0] {
        d.last_section.margin_left = margin;
        for enabled in [false, true, false] {
            d.settings.mirror_margins = enabled;
            for model in [&d, &roundtrip(&d)] {
                let l = layout(model);
                assert_eq!(l.pages.len(), 2);
                let mut plain = model.clone();
                plain.settings.mirror_margins = false;
                let unmirrored = layout(&plain);
                assert_eq!(unmirrored.pages.len(), 2);
                let pdf =
                    wordcraft_pdf::export(model, &wordcraft_pdf::PdfOptions::default()).unwrap();
                let payload = crate::pdf::extract(&pdf, None).unwrap();
                assert_eq!(payload.pages.len(), 2);
                for (pi, p) in l.pages.iter().enumerate() {
                    let base = left(model, p, &model.last_section);
                    near(p.body.x, base);
                    let delta = base - (model.last_section.margin_left + model.last_section.gutter);
                    assert_eq!(p.items.len(), unmirrored.pages[pi].items.len());
                    for (a, b) in p.items.iter().zip(&unmirrored.pages[pi].items) {
                        assert_eq!(std::mem::discriminant(a), std::mem::discriminant(b));
                        match (a, b) {
                            (Placed::Cell { rect: a, .. }, Placed::Cell { rect: b, .. }) => {
                                near(a.x - b.x, delta)
                            }
                            (Placed::Lines { x: a, .. }, Placed::Lines { x: b, .. }) => {
                                near(a - b, delta)
                            }
                            _ => {}
                        }
                    }
                    let text = payload.pages[pi]
                        .blocks
                        .iter()
                        .map(|b| b.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    let body = payload.pages[pi]
                        .blocks
                        .iter()
                        .find(|b| b.text.contains(&format!("Body marker {pi}")))
                        .unwrap();
                    assert!((body.bbox.unwrap()[0] - f64::from(base)).abs() < 1.0);
                    for label in [
                        format!("Body marker {pi}"),
                        format!("Table marker {pi}"),
                        "Footer marker".into(),
                    ] {
                        assert_eq!(text.matches(&label).count(), 1, "{text}");
                    }
                    for item in p.items.iter().chain(&p.header) {
                        if let Placed::Image { rect, media, .. } = item {
                            let data = model.media.get(media).unwrap();
                            let x = if **data == png([255, 0, 0, 255]) {
                                base
                            } else if **data == png([0, 0, 255, 255]) {
                                base + 4.0
                            } else if **data == png([0, 255, 0, 255]) {
                                220.0
                            } else if **data == png([128, 0, 128, 255]) {
                                base + 12.0
                            } else {
                                panic!("unexpected media {media}")
                            };
                            near(rect.x, x);
                            assert!(
                                rect.x >= 0.0
                                    && rect.right() <= p.w
                                    && rect.y >= 0.0
                                    && rect.bottom() <= p.h
                            );
                        }
                    }
                    for dpi in [72.0, 144.0, 300.0] {
                        let source = render_with(model, &l, pi, dpi, None).unwrap();
                        let exported = crate::pdf::render(&pdf, pi, dpi, None).unwrap();
                        let scale = dpi / 72.0;
                        for (color, x, y) in [
                            ([255, 0, 0, 255], base, 12.0),
                            ([0, 0, 255, 255], base + 4.0, 90.0),
                            ([0, 255, 0, 255], 220.0, 120.0),
                            ([128, 0, 128, 255], base + 12.0, 150.0),
                        ] {
                            let a = color_box(&source, color);
                            let b = color_box(&exported, color);
                            assert!(
                                (f64::from(a.0) - f64::from(x) * scale).abs() <= 2.0,
                                "source {a:?} expected{x}"
                            );
                            assert!(
                                (a.0 as i64 - b.0 as i64).abs() <= 1
                                    && (a.2 as i64 - b.2 as i64).abs() <= 1,
                                "{a:?} {b:?}"
                            );
                            assert!((f64::from(a.2 - a.0) - 24.0 * scale).abs() <= 2.0);
                            assert!(
                                (f64::from(a.1) - y * scale).abs() <= 2.0,
                                "source y {a:?} expected{y}"
                            );
                            assert!((f64::from(a.3 - a.1) - 12.0 * scale).abs() <= 2.0);
                            assert!(
                                (a.1 as i64 - b.1 as i64).abs() <= 1
                                    && (a.3 as i64 - b.3 as i64).abs() <= 1,
                                "y {a:?} {b:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
