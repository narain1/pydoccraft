//! Public pinned-native section alignment proof; no adapter property yet.
use super::*;
use wordcraft_doc::section::PageVAlign;
use wordcraft_layout::{DocLayout, Placed};

const MODES: [PageVAlign; 4] = [
    PageVAlign::Top,
    PageVAlign::Center,
    PageVAlign::Bottom,
    PageVAlign::Justify,
];
fn para(text: &str, color: Rgb) -> Arc<Block> {
    let mut p = Paragraph::with_text(
        text,
        CharProps {
            color: Some(TextColor::Rgb(color)),
            ..Default::default()
        },
    );
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    wordcraft_doc::para_block(p)
}
fn setup(d: &mut Document) {
    let s = &mut d.last_section;
    s.page_w = 320.0;
    s.page_h = 420.0;
    s.margin_left = 36.0;
    s.margin_right = 36.0;
    s.margin_top = 52.0;
    s.margin_bottom = 52.0;
    s.header = 12.0;
    s.footer = 12.0;
}
fn layout(d: &Document) -> DocLayout {
    wordcraft_layout::layout(d, &mut Default::default(), &Default::default())
}
fn bytes(d: &Document) -> Vec<u8> {
    wordcraft_docx::write(d).unwrap()
}
fn roundtrip(d: &Document) -> Document {
    wordcraft_docx::read(&bytes(d)).unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.1, "{a} != {b}");
}
fn lines(items: &[Placed], story: StoryRef) -> Vec<(Path, f32, f32)> {
    items
        .iter()
        .filter_map(|it| {
            if let Placed::Lines {
                story: s,
                path,
                para,
                l0,
                l1,
                y,
                ..
            } = it
            {
                if *s == story && *l0 < *l1 {
                    let a = &para.lines[*l0];
                    let b = &para.lines[l1 - 1];
                    return Some((path.clone(), *y, *y + b.top - a.top + b.height));
                }
            }
            None
        })
        .collect()
}
fn image_png(color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 4, 4);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        e.write_header()
            .unwrap()
            .write_image_data(&color.repeat(16))
            .unwrap();
    }
    out
}
fn image(d: &mut Document, name: &str, float: Float) -> Arc<Block> {
    d.media
        .insert(name.into(), Arc::new(image_png([128, 0, 128, 255])));
    let mut p = Paragraph::new();
    p.props.space_before = Some(0.0);
    p.props.space_after = Some(0.0);
    p.insert_object(
        0,
        InlineObject::Image {
            media: name.into(),
            w: 16.0,
            h: 16.0,
            alt: name.into(),
            float,
            crop: [0.0; 4],
            ole: None,
        },
        &Default::default(),
    )
    .unwrap();
    wordcraft_doc::para_block(p)
}
fn color_box(bytes: &[u8], color: [u8; 4]) -> (u32, u32, u32, u32) {
    let mut r = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut data = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut data).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    let mut b = (u32::MAX, u32::MAX, 0, 0);
    let mut n = 0;
    for (i, p) in data[..info.buffer_size()].chunks_exact(4).enumerate() {
        // Text antialiasing depends on fractional vertical placement. Detect ink
        // by chroma, not the presence of an exact fully covered color pixel.
        let (red, green, blue) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
        let ink = match color {
            [255, 0, 0, 255] => red > 120 && red > green + 60 && red > blue + 60,
            [0, 128, 0, 255] => green > 60 && green > red + 30 && green > blue + 30,
            [0, 0, 255, 255] => blue > 120 && blue > red + 60 && blue > green + 60,
            [128, 0, 128, 255] => {
                red > 80 && blue > 80 && (red - blue).abs() < 40 && red.min(blue) > green + 40
            }
            _ => p == color,
        };
        if ink {
            let x = i as u32 % info.width;
            let y = i as u32 / info.width;
            b.0 = b.0.min(x);
            b.1 = b.1.min(y);
            b.2 = b.2.max(x + 1);
            b.3 = b.3.max(y + 1);
            n += 1;
        }
    }
    assert!(n > 5, "missing color {color:?}");
    b
}
fn pdf_texts(pdf: &[u8]) -> Vec<String> {
    crate::pdf::extract(pdf, None)
        .unwrap()
        .pages
        .iter()
        .map(|p| {
            p.blocks
                .iter()
                .map(|b| {
                    b.table.as_ref().map_or_else(
                        || b.text.clone(),
                        |t| {
                            t.rows
                                .iter()
                                .flatten()
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(" ")
                        },
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}
fn output(d: &Document, colors: &[[u8; 4]]) {
    let l = layout(d);
    let pdf = wordcraft_pdf::export(d, &Default::default()).unwrap();
    assert_eq!(pdf_texts(&pdf).len(), l.pages.len());
    for page in 0..l.pages.len() {
        for dpi in [72.0, 144.0] {
            eprintln!(
                "visible output {:?} page{page} dpi{dpi}",
                d.last_section.valign
            );
            let mut options = wordcraft_render::RenderOptions::default();
            options.display.dim_header = false;
            let a = wordcraft_render::render_page(d, &l.pages[page], dpi as f32 / 72.0, &options)
                .to_png();
            let b = crate::pdf::render(&pdf, page, dpi, None).unwrap();
            for color in colors {
                let a = color_box(&a, *color);
                let b = color_box(&b, *color);
                for (x, y) in [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)] {
                    assert!(
                        (i64::from(x) - i64::from(y)).abs() <= 2,
                        "native/PDF {a:?} {b:?}"
                    );
                }
            }
        }
    }
}
fn assert_body_inside(l: &DocLayout) {
    for (index, page) in l.pages.iter().enumerate() {
        for (_, top, bottom) in lines(&page.items, StoryRef::Body) {
            assert!(
                top >= page.body.y - 0.1 && bottom <= page.body.y + page.body.h + 0.1,
                "page{index}: body={:?} line={top}..{bottom}",
                page.body
            );
        }
        for item in &page.items {
            if let Placed::Cell {
                rect,
                story: StoryRef::Body,
                ..
            } = item
            {
                assert!(
                    rect.y >= page.body.y - 0.1 && rect.bottom() <= page.body.bottom() + 0.1,
                    "cell {rect:?} vs {:?}",
                    page.body
                );
            }
        }
    }
}

#[test]
fn upstream_section_valign_model_simple_geometry_and_visible_output() {
    for count in [0, 1, 3] {
        let mut d = Document::new();
        setup(&mut d);
        d.body = (0..count)
            .map(|i| {
                para(
                    ["Alpha", "Beta", "Gamma"][i],
                    [Rgb(255, 0, 0), Rgb(0, 128, 0), Rgb(0, 0, 255)][i],
                )
            })
            .collect();
        if count == 0 {
            d.body.push(para("", Rgb(0, 0, 0)));
        }
        let top = layout(&d);
        let b = lines(&top.pages[0].items, StoryRef::Body);
        let used = b.last().unwrap().2 - top.pages[0].body.y;
        let room = top.pages[0].body.h - used;
        for mode in MODES {
            d.last_section.valign = mode;
            for current in [&d, &roundtrip(&d)] {
                assert_eq!(current.last_section.valign, mode);
                let l = layout(current);
                assert_eq!(l.pages.len(), 1);
                let p = lines(&l.pages[0].items, StoryRef::Body);
                assert_eq!(p.len(), b.len());
                match mode {
                    PageVAlign::Top => near(p[0].1, b[0].1),
                    PageVAlign::Center => near(p[0].1, b[0].1 + room / 2.0),
                    PageVAlign::Bottom => near(p[0].1, b[0].1 + room),
                    PageVAlign::Justify => {
                        near(p[0].1, b[0].1);
                        if count > 1 {
                            near(p.last().unwrap().2, l.pages[0].body.bottom());
                            near(p[1].1 - p[0].2, p[2].1 - p[1].2);
                        }
                    }
                }
                assert_body_inside(&l);
                if count > 0 {
                    output(current, &[[255, 0, 0, 255]]);
                    let pdf = wordcraft_pdf::export(current, &Default::default()).unwrap();
                    for label in ["Alpha", "Beta", "Gamma"].iter().take(count) {
                        assert_eq!(pdf_texts(&pdf).join(" ").matches(label).count(), 1);
                    }
                }
            }
        }
    }
    // Native whitespace and section ownership are preserved, not adjusted locally.
    let mut d = Document::new();
    setup(&mut d);
    d.body = vec![
        para("First", Rgb(0, 0, 0)),
        para("", Rgb(0, 0, 0)),
        para("Last", Rgb(0, 0, 0)),
        para("Next section", Rgb(0, 0, 0)),
    ];
    let mut first = d.last_section.clone();
    first.valign = PageVAlign::Justify;
    let p = d.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    p.props.space_after = Some(36.0);
    p.section = Some(Box::new(first));
    d.last_section.valign = PageVAlign::Bottom;
    d.last_section.page_num_start = Some(7);
    for current in [&d, &roundtrip(&d)] {
        let l = layout(current);
        assert_eq!(
            l.pages
                .iter()
                .map(|p| (p.section, p.number))
                .collect::<Vec<_>>(),
            vec![(0, 1), (1, 7)]
        );
        let b = lines(&l.pages[0].items, StoryRef::Body);
        near(l.pages[0].body.bottom() - b.last().unwrap().2, 36.0);
        let b = lines(&l.pages[1].items, StoryRef::Body);
        near(l.pages[1].body.bottom(), b.last().unwrap().2);
    }
}

#[test]
fn upstream_section_valign_composition_fixed_stories_notes_tables_and_pagination() {
    pyo3::prepare_freethreaded_python();
    for with_note in [false, true] {
        let mut owner = EditableDocx::new();
        owner.set_page_size(320.0, 420.0).unwrap();
        owner.set_margins(52.0, 36.0, 52.0, 36.0).unwrap();
        let p = owner.add_paragraph("Body anchor").unwrap();
        owner
            .set_range_link(p, 0, 11, owner.revision, Some("https://example.com/body"))
            .unwrap();
        if with_note {
            owner
                .add_note(p, 11, 11, owner.revision, "Fixed bottom note", "footnote")
                .unwrap();
        }
        let field = owner.add_paragraph("Page value ").unwrap();
        owner.add_paragraph_field(field, "page").unwrap();
        let table = owner.add_table(vec![vec!["Table value".into()]]).unwrap();
        let cell = owner.cell_paragraph_ids(table, 0, 0).unwrap()[0];
        owner.add_paragraph_field(cell, "page_count").unwrap();
        owner
            .cell_add_table(table, 0, 0, vec![vec!["Nested value".into()]])
            .unwrap();
        let page_path = owner.location(field).unwrap().1;
        let count_path = owner.location(cell).unwrap().1;
        let mut d = owner.document().unwrap().clone();
        setup(&mut d);
        d.last_section.page_num_start = Some(7);
        let header = d.add_part(
            PartKind::Header,
            vec![para(
                "Header fixed\nSecond header line\nThird header line\nFourth header line",
                Rgb(0, 128, 0),
            )],
        );
        let footer = d.add_part(
            PartKind::Footer,
            vec![para("Footer fixed\nSecond footer line", Rgb(0, 0, 255))],
        );
        d.last_section.headers.default = Some(header);
        d.last_section.footers.default = Some(footer);
        let art = image(&mut d, "inline.png", Float::default());
        d.body.push(art);
        d.body.push(para("End body", Rgb(255, 0, 0)));
        let baseline = layout(&d);
        assert_eq!(baseline.pages.len(), 1);
        for mode in MODES {
            d.last_section.valign = mode;
            let reopened = roundtrip(&d);
            if !with_note {
                let original = layout(&d);
                let parsed = layout(&reopened);
                assert_eq!(original.pages.len(), parsed.pages.len());
                for (a, b) in original.pages.iter().zip(&parsed.pages) {
                    assert_eq!(
                        lines(&a.items, StoryRef::Body),
                        lines(&b.items, StoryRef::Body)
                    );
                    assert_eq!(
                        lines(&a.header, StoryRef::Part(a.header_story.unwrap())),
                        lines(&b.header, StoryRef::Part(b.header_story.unwrap()))
                    );
                    assert_eq!(
                        lines(&a.footer, StoryRef::Part(a.footer_story.unwrap())),
                        lines(&b.footer, StoryRef::Part(b.footer_story.unwrap()))
                    );
                    let boxes = |items: &[Placed]| {
                        items
                            .iter()
                            .filter_map(|item| match item {
                                Placed::Cell { rect, .. } | Placed::Image { rect, .. } => {
                                    Some((rect.x, rect.y, rect.w, rect.h))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                    };
                    assert!(!boxes(&a.items).is_empty());
                    let (a_boxes, b_boxes) = (boxes(&a.items), boxes(&b.items));
                    assert_eq!(a_boxes.len(), b_boxes.len());
                    for (a, b) in a_boxes.iter().zip(&b_boxes) {
                        for (x, y) in [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)] {
                            // Native DOCX width serialization rounds 237.20001 to237.2.
                            assert!((x - y).abs() < 0.0001, "{a:?} vs {b:?}");
                        }
                    }
                }
            }
            for current in [&d, &reopened] {
                let top = layout(&{
                    let mut c = current.clone();
                    c.last_section.valign = PageVAlign::Top;
                    c
                });
                let l = layout(current);
                assert_eq!(l.pages.len(), 1);
                assert_body_inside(&l);
                let h = l.pages[0].header_story.unwrap();
                assert!(!lines(&l.pages[0].header, StoryRef::Part(h)).is_empty());
                assert_eq!(
                    lines(&l.pages[0].header, StoryRef::Part(h)),
                    lines(&top.pages[0].header, StoryRef::Part(h))
                );
                let f = l.pages[0].footer_story.unwrap();
                assert!(!lines(&l.pages[0].footer, StoryRef::Part(f)).is_empty());
                assert_eq!(
                    lines(&l.pages[0].footer, StoryRef::Part(f)),
                    lines(&top.pages[0].footer, StoryRef::Part(f))
                );
                let note_stories: Vec<_> = current
                    .parts
                    .iter()
                    .filter(|(_, p)| p.kind == PartKind::Footnote)
                    .map(|(id, _)| StoryRef::Part(*id))
                    .collect();
                assert_eq!(!note_stories.is_empty(), with_note);
                for story in note_stories {
                    eprintln!(
                        "page-bottom note {mode:?}: {:?}",
                        lines(&l.pages[0].items, story)
                    );
                    assert!(!lines(&l.pages[0].items, story).is_empty());
                    assert_eq!(
                        lines(&l.pages[0].items, story),
                        lines(&top.pages[0].items, story)
                    );
                }
                output(
                    current,
                    &[
                        [255, 0, 0, 255],
                        [128, 0, 128, 255],
                        [0, 128, 0, 255],
                        [0, 0, 255, 255],
                    ],
                );
                let pdf = wordcraft_pdf::export(current, &Default::default()).unwrap();
                assert!(crate::pdf::extract(&pdf, None).unwrap().pages[0]
                    .links
                    .iter()
                    .any(|link| link.uri == "https://example.com/body"));
                let text = pdf_texts(&pdf).join(" ");
                let compact = text.split_whitespace().collect::<String>();
                assert!(compact.contains("Pagevalue7"), "{text}");
                assert!(compact.contains("Tablevalue1"), "{text}");
                for (path, expected) in [(&page_path, "7"), (&count_path, "1")] {
                    let shown: Vec<_> = l.pages[0]
                        .items
                        .iter()
                        .filter_map(|item| {
                            if let Placed::Lines {
                                story: StoryRef::Body,
                                path: position,
                                para,
                                ..
                            } = item
                            {
                                if position == path {
                                    return Some(
                                        para.shown.iter().map(|(_, text, _)| text.as_str()),
                                    );
                                }
                            }
                            None
                        })
                        .flatten()
                        .collect();
                    assert!(shown.contains(&expected), "{mode:?}/{path:?}: {shown:?}");
                }
                for label in [
                    "Body anchor",
                    "Table value",
                    "Nested value",
                    "Header fixed",
                    "Footer fixed",
                    "End body",
                ] {
                    assert_eq!(text.matches(label).count(), 1, "{mode:?}: {text}");
                }
                assert_eq!(
                    text.matches("Fixed bottom note").count(),
                    usize::from(with_note)
                );
                eprintln!(
                "single-column composition note={with_note} {mode:?}: body {:?}, body lines {:?}",
                l.pages[0].body,
                lines(&l.pages[0].items, StoryRef::Body)
            );
            }
        }
    }
    // First-page stories use their own height and remain fixed as body alignment changes.
    let mut variants = Document::new();
    setup(&mut variants);
    variants.body = vec![
        para("First body", Rgb(255, 0, 0)),
        para("Later body", Rgb(255, 0, 0)),
    ];
    variants
        .para_mut(StoryRef::Body, &Path::top(1))
        .unwrap()
        .props
        .page_break_before = Some(true);
    let first = variants.add_part(
        PartKind::Header,
        vec![para("First header\nline\nline\nline", Rgb(0, 128, 0))],
    );
    let default = variants.add_part(
        PartKind::Header,
        vec![para("Default header", Rgb(0, 128, 0))],
    );
    variants.last_section.title_page = true;
    variants.last_section.headers.first = Some(first);
    variants.last_section.headers.default = Some(default);
    for mode in MODES {
        variants.last_section.valign = mode;
        for current in [&variants, &roundtrip(&variants)] {
            let top = layout(&{
                let mut c = current.clone();
                c.last_section.valign = PageVAlign::Top;
                c
            });
            let l = layout(current);
            assert_eq!(l.pages.len(), 2);
            assert_body_inside(&l);
            for (p, t) in l.pages.iter().zip(&top.pages) {
                let story = StoryRef::Part(p.header_story.unwrap());
                assert_eq!(lines(&p.header, story), lines(&t.header, story));
            }
            output(current, &[[255, 0, 0, 255], [0, 128, 0, 255]]);
        }
    }
    // Natural pagination and table continuation use native flow; alignment must not
    // duplicate or move line/cell boxes beyond the physical body region.
    let mut flow = Document::new();
    setup(&mut flow);
    flow.body = (0..45)
        .map(|i| para(&format!("Flow-{i:02}-end"), Rgb(0, 0, 0)))
        .collect();
    let mut t = Table::new(16, 1, 200.0);
    for (i, row) in t.rows.iter_mut().enumerate() {
        row.cells[0].blocks = vec![para(&format!("Row-{i:02}-end"), Rgb(0, 0, 0))];
    }
    t.rows[0].props.header = true;
    flow.body.push(Arc::new(Block::Table(t)));
    flow.body.push(para("Tail", Rgb(255, 0, 0)));
    let count = layout(&flow).pages.len();
    assert!(count > 2);
    for mode in MODES {
        flow.last_section.valign = mode;
        for current in [&flow, &roundtrip(&flow)] {
            let l = layout(current);
            assert_eq!(l.pages.len(), count);
            assert_body_inside(&l);
            let pdf = wordcraft_pdf::export(current, &Default::default()).unwrap();
            let text = pdf_texts(&pdf).join(" ");
            for i in 0..45 {
                assert_eq!(text.matches(&format!("Flow-{i:02}-end")).count(), 1);
            }
            for i in 1..16 {
                assert_eq!(text.matches(&format!("Row-{i:02}-end")).count(), 1);
            }
            for page in 0..l.pages.len() {
                for dpi in [72.0, 144.0] {
                    assert!(
                        crate::docx::render(&bytes(current), page, dpi, None)
                            .unwrap()
                            .len()
                            > 1000
                    );
                    assert!(crate::pdf::render(&pdf, page, dpi, None).unwrap().len() > 1000);
                }
            }
        }
    }
}

#[test]
fn upstream_section_valign_columns_continuous_and_page_anchor_diagnostics() {
    pyo3::prepare_freethreaded_python();
    // Collect desired-geometry failures before asserting, retaining all independent
    // source-derived controls in the first diagnostic execution.
    let mut failures = Vec::new();
    let mut columns = Document::new();
    setup(&mut columns);
    columns.last_section.columns.count = 2;
    columns.last_section.columns.space = 12.0;
    columns.body = vec![para(
        &format!(
            "Tall column\n{}{}Short column",
            "line\n".repeat(12),
            wordcraft_doc::para::COLUMN_BREAK
        ),
        Rgb(255, 0, 0),
    )];
    for mode in MODES {
        columns.last_section.valign = mode;
        for current in [&columns, &roundtrip(&columns)] {
            let l = layout(current);
            let boxes = lines(&l.pages[0].items, StoryRef::Body);
            eprintln!("columns {mode:?}: {boxes:?}");
            if boxes.iter().any(|(_, a, b)| {
                *a < l.pages[0].body.y - 0.1 || *b > l.pages[0].body.bottom() + 0.1
            }) {
                failures.push(format!("uneven columns {mode:?} leave body"));
            }
        }
    }
    let mut continuous = Document::new();
    setup(&mut continuous);
    continuous.body = vec![
        para("First section", Rgb(255, 0, 0)),
        para("Second section", Rgb(0, 0, 255)),
    ];
    let mut first = continuous.last_section.clone();
    first.valign = PageVAlign::Bottom;
    continuous
        .para_mut(StoryRef::Body, &Path::top(0))
        .unwrap()
        .section = Some(Box::new(first));
    continuous.last_section.start = SectionStart::Continuous;
    for current in [&continuous, &roundtrip(&continuous)] {
        let l = layout(current);
        eprintln!(
            "continuous bottom/top: {:?}",
            lines(&l.pages[0].items, StoryRef::Body)
        );
        assert_body_inside(&l);
        output(current, &[[255, 0, 0, 255], [0, 0, 255, 255]]);
    }
    for wrap in [Wrap::BehindText, Wrap::Square] {
        let mut d = Document::new();
        setup(&mut d);
        let art = image(
            &mut d,
            "page.png",
            Float {
                wrap,
                h_rel: Anchor::Page,
                v_rel: Anchor::Page,
                x: 250.0,
                y: 90.0,
                ..Default::default()
            },
        );
        d.body = vec![art, para("Body", Rgb(255, 0, 0))];
        for mode in MODES {
            d.last_section.valign = mode;
            for current in [&d, &roundtrip(&d)] {
                let l = layout(current);
                let rect = l.pages[0]
                    .items
                    .iter()
                    .find_map(|it| {
                        if let Placed::Image { rect, .. } = it {
                            Some(rect)
                        } else {
                            None
                        }
                    })
                    .unwrap();
                eprintln!("page anchor {wrap:?}/{mode:?}: {rect:?}");
                if (rect.y - 90.0).abs() > 0.1 {
                    failures.push(format!("page anchor {wrap:?}/{mode:?} moves to {}", rect.y));
                }
                output(current, &[[128, 0, 128, 255]]);
            }
        }
    }
    let mut even = Document::new();
    setup(&mut even);
    even.last_section.page_num_start = Some(2);
    even.settings.even_odd_headers = true;
    even.body = (0..16)
        .map(|i| para(&format!("Body {i}"), Rgb(255, 0, 0)))
        .collect();
    let default = even.add_part(
        PartKind::Header,
        vec![para("Default header", Rgb(0, 128, 0))],
    );
    let tall = even.add_part(
        PartKind::Header,
        vec![para(
            "Even header\nline\nline\nline\nline\nline",
            Rgb(0, 128, 0),
        )],
    );
    even.last_section.headers.default = Some(default);
    even.last_section.headers.even = Some(tall);
    for mode in MODES {
        even.last_section.valign = mode;
        for current in [&even, &roundtrip(&even)] {
            let l = layout(current);
            assert_eq!(l.pages[0].number, 2);
            let header = lines(
                &l.pages[0].header,
                StoryRef::Part(l.pages[0].header_story.unwrap()),
            );
            let body = lines(&l.pages[0].items, StoryRef::Body);
            eprintln!(
                "unequal even header {mode:?}: header={header:?} bodyfirst={:?}",
                body.first()
            );
            if body[0].1 < header.last().unwrap().2 - 0.1 {
                failures.push(format!("unequal even header overlaps {mode:?}"));
            }
        }
    }
    for (kind, placement) in [("footnote", "beneath_text"), ("endnote", "document_end")] {
        let mut owner = EditableDocx::new();
        owner.set_page_size(320.0, 420.0).unwrap();
        owner.set_margins(52.0, 36.0, 52.0, 36.0).unwrap();
        let p = owner.add_paragraph("Note anchor").unwrap();
        owner
            .add_note(p, 11, 11, owner.revision, "Note content", kind)
            .unwrap();
        owner
            .set_section_note_options(0, kind, &format!(r#"{{"placement":"{placement}"}}"#))
            .unwrap();
        let mut d = owner.document().unwrap().clone();
        for mode in MODES {
            d.last_section.valign = mode;
            for current in [&d, &roundtrip(&d)] {
                let l = layout(current);
                assert_body_inside(&l);
                for (id, part) in &current.parts {
                    if matches!(part.kind, PartKind::Footnote | PartKind::Endnote) {
                        eprintln!(
                            "{kind}/{placement}/{mode:?}: {:?}",
                            lines(&l.pages[0].items, StoryRef::Part(*id))
                        );
                    }
                }
                let pdf = wordcraft_pdf::export(current, &Default::default()).unwrap();
                assert_eq!(pdf_texts(&pdf).join(" ").matches("Note content").count(), 1);
                output(current, &[]);
            }
        }
    }
    // These are preserved native diagnostics, not accepted adapter profiles.
    eprintln!("native desired-geometry failures: {failures:#?}");
    assert_eq!(failures.len(), 18);
    assert_eq!(failures.iter().filter(|s| s.contains("column")).count(), 4);
    assert_eq!(failures.iter().filter(|s| s.contains("anchor")).count(), 8);
    assert_eq!(
        failures
            .iter()
            .filter(|s| s.contains("even header"))
            .count(),
        6
    );
}
