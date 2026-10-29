//! Raw native primitives and exact shared placement pairs; no owned shape adapter.
use super::*;
use std::{io::Cursor, sync::Arc};
use wordcraft_doc::para::{Anchor, Float, ShapeKind, Wrap};
use wordcraft_doc::{CharProps, Rgb};
use wordcraft_layout::{
    display::{page_display, DisplayOptions, Draw},
    Placed,
};

fn shape(kind: ShapeKind, float: Float) -> InlineObject {
    InlineObject::Shape {
        kind,
        w: 40.0,
        h: 20.0,
        fill: Some(wordcraft_doc::fill::Fill::solid(Rgb(255, 0, 0))),
        stroke: Some(Rgb(0, 0, 255)),
        stroke_width: 2.0,
        float,
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    }
}
fn page() -> Document {
    let mut d = Document::from_text("BodyKeep\nTailKeep");
    let s = &mut d.last_section;
    s.page_w = 300.0;
    s.page_h = 360.0;
    s.margin_left = 30.0;
    s.margin_right = 50.0;
    s.margin_top = 30.0;
    s.margin_bottom = 30.0;
    d
}
fn layout(d: &Document) -> DocLayout {
    wordcraft_layout::layout(d, &mut LayoutCache::new(), &LayoutOptions::default())
}
fn png_marker() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[255, 0, 0, 255].repeat(4))
            .unwrap();
    }
    out
}

#[test]
#[cfg(feature = "python")]
fn closed_shape_native_insert_delete_retains_anchor_and_neighbors() {
    pyo3::prepare_freethreaded_python();
    let mut owner = crate::author_docx::EditableDocx::new();
    let text = owner.add_paragraph("BodyKeep").unwrap();
    owner
        .set_range_link(
            text,
            0,
            8,
            owner.revision().unwrap(),
            Some("https://example.invalid/keep"),
        )
        .unwrap();
    owner.add_table(vec![vec!["TableKeep".into()]]).unwrap();
    owner
        .add_image(&png_marker(), Some(12.0), Some(12.0), "ImageKeep")
        .unwrap();
    let note = owner.add_paragraph("NoteAnchor").unwrap();
    owner
        .add_note(
            note,
            10,
            10,
            owner.revision().unwrap(),
            "NoteKeep",
            "footnote",
        )
        .unwrap();
    let comment = owner.add_paragraph("CommentAnchor").unwrap();
    owner
        .add_comment(
            Some(comment),
            0,
            13,
            owner.revision().unwrap(),
            None,
            "CommentKeep",
            "Ada",
            "",
        )
        .unwrap();
    let source = owner.to_bytes().unwrap();
    let rich = wordcraft_docx::read(&source).unwrap();
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse] {
        for scenario in ["placeholder", "middle", "section_tail"] {
            let mut d = if scenario == "placeholder" {
                Document::new()
            } else {
                rich.clone()
            };
            let original = d.clone();
            let index = if scenario == "placeholder" {
                0
            } else if scenario == "middle" {
                1
            } else {
                d.body.len()
            };
            if scenario != "placeholder" {
                let mut p = Paragraph::new();
                if scenario == "section_tail" {
                    p.section = Some(Box::new(d.last_section.clone()));
                }
                d.insert_block(StoryRef::Body, &Path::top(index), Block::Para(p))
                    .unwrap();
            }
            let expected_blank = d.clone();
            let count = d.body.len();
            d.para_mut(StoryRef::Body, &Path::top(index))
                .unwrap()
                .insert_object(0, shape(kind, Float::default()), &CharProps::default())
                .unwrap();
            let with_shape = d.clone();
            let p = d.para(StoryRef::Body, &Path::top(index)).unwrap();
            assert_eq!(p.text, "\u{fffc}");
            assert_eq!(p.objects.len(), 1);
            assert_eq!(p.runs.iter().map(|r| r.len).sum::<usize>(), 3);
            assert_eq!(d.body.len(), count);
            for (before, after) in expected_blank
                .body
                .iter()
                .zip(&d.body)
                .enumerate()
                .filter_map(|(i, pair)| (i != index).then_some(pair))
            {
                assert!(Arc::ptr_eq(before, after));
            }
            let reopened = wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
            assert!(
                matches!(&reopened.para(StoryRef::Body,&Path::top(index)).unwrap().objects[0],InlineObject::Shape{kind:actual,..} if *actual==kind)
            );
            d.para_mut(StoryRef::Body, &Path::top(index))
                .unwrap()
                .delete(0, 3)
                .unwrap();
            assert_eq!(
                d, expected_blank,
                "native delete changes only the object in {scenario}"
            );
            assert_eq!(d.body.len(), count, "no tail padding after marker deletion");
            assert!(d
                .para(StoryRef::Body, &Path::top(index))
                .unwrap()
                .objects
                .is_empty());
            assert_eq!(d.parts, original.parts);
            assert_eq!(d.comments, original.comments);
            assert_eq!(d.media, original.media);
            assert_eq!(d.styles, original.styles);
            assert_eq!(
                d.para(StoryRef::Body, &Path::top(index))
                    .unwrap()
                    .section
                    .is_some(),
                scenario == "section_tail"
            );
            let after = wordcraft_docx::write(&d).unwrap();
            assert_eq!(
                wordcraft_docx::read(&after).unwrap(),
                wordcraft_docx::read(&wordcraft_docx::write(&expected_blank).unwrap()).unwrap()
            );
            if scenario != "placeholder" {
                let payload = crate::docx::extract(&after, None).unwrap();
                let json = serde_json::to_string(&payload).unwrap();
                for label in [
                    "BodyKeep",
                    "TableKeep",
                    "NoteKeep",
                    "CommentKeep",
                    "ImageKeep",
                ] {
                    assert!(json.contains(label), "{label}");
                }
                assert!(payload
                    .pages
                    .iter()
                    .flat_map(|p| &p.links)
                    .any(|l| l.uri == "https://example.invalid/keep"));
            }
            // Copy remains independent; a replacement is a new native object in the empty anchor.
            assert_eq!(
                with_shape
                    .para(StoryRef::Body, &Path::top(index))
                    .unwrap()
                    .objects
                    .len(),
                1
            );
            d.para_mut(StoryRef::Body, &Path::top(index))
                .unwrap()
                .insert_object(0, shape(kind, Float::default()), &CharProps::default())
                .unwrap();
            assert_eq!(d, with_shape);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}
fn frames(l: &DocLayout, path: &Path) -> Vec<(usize, Frame)> {
    l.pages
        .iter()
        .enumerate()
        .flat_map(|(page, p)| {
            p.items.iter().filter_map(move |item| match item {
                Placed::Object {
                    story: StoryRef::Body,
                    path: actual,
                    off: 0,
                    rect,
                    ..
                } if actual == path => Some((
                    page,
                    Frame {
                        x: rect.x,
                        y: rect.y,
                        w: rect.w,
                        h: rect.h,
                    },
                )),
                _ => None,
            })
        })
        .collect()
}
fn contained(l: &DocLayout, path: &Path, stroke: f32) -> bool {
    let found = frames(l, path);
    if found.len() != 1 {
        return false;
    }
    let (page, r) = found[0];
    let p = &l.pages[page];
    let half = stroke / 2.0;
    [p.w, p.h, r.x, r.y, r.w, r.h, r.x + r.w, r.y + r.h, half]
        .iter()
        .all(|x| x.is_finite())
        && p.w > 0.0
        && p.h > 0.0
        && r.w > 0.0
        && r.h > 0.0
        && r.x - half >= 0.0
        && r.y - half >= 0.0
        && r.x + r.w + half <= p.w
        && r.y + r.h + half <= p.h
}
fn mask(bytes: &[u8], red: bool) -> Vec<(u32, u32)> {
    let mut r = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut pixels = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    pixels[..info.buffer_size()]
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(i, p)| {
            let yes = if red {
                p[0] > 140 && p[1] < 100 && p[2] < 100
            } else {
                p[2] > 140 && p[0] < 100 && p[1] < 100
            };
            yes.then_some((i as u32 % info.width, i as u32 / info.width))
        })
        .collect()
}
fn bounds(points: &[(u32, u32)]) -> [f64; 4] {
    assert!(!points.is_empty());
    [
        points.iter().map(|p| p.0).min().unwrap() as f64,
        points.iter().map(|p| p.1).min().unwrap() as f64,
        (points.iter().map(|p| p.0).max().unwrap() + 1) as f64,
        (points.iter().map(|p| p.1).max().unwrap() + 1) as f64,
    ]
}
fn inserted(kind: ShapeKind, float: Float, profile: u8) -> Document {
    let mut d = page();
    if profile > 0 {
        d.settings.mirror_margins = true;
        d.last_section.page_num_start = Some(2);
    }
    if profile == 1 {
        d.last_section.columns.count = 2;
        d.last_section.columns.space = 12.0;
    }
    if profile == 2 {
        d.last_section.line_numbers = Some(wordcraft_doc::section::LineNumbering {
            distance: 6.0,
            ..Default::default()
        });
    }
    let mut p = Paragraph::new();
    p.insert_object(0, shape(kind, float), &CharProps::default())
        .unwrap();
    d.insert_block(StoryRef::Body, &Path::top(1), Block::Para(p))
        .unwrap();
    d
}

#[test]
fn closed_shape_shared_anchor_pairs_native_frames_and_full_stroke_bounds() {
    let path = Path::top(1);
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse] {
        for profile in [0, 1, 2] {
            for wrap in [
                Wrap::Inline,
                Wrap::Square,
                Wrap::TopAndBottom,
                Wrap::BehindText,
            ] {
                for (h_rel, v_rel) in [
                    (Anchor::Page, Anchor::Page),
                    (Anchor::Margin, Anchor::Margin),
                    (Anchor::Column, Anchor::Paragraph),
                ] {
                    if wrap == Wrap::Inline && h_rel != Anchor::Column {
                        continue;
                    }
                    let f = if wrap == Wrap::Inline {
                        Float::default()
                    } else {
                        Float {
                            wrap,
                            h_rel,
                            v_rel,
                            x: 10.05,
                            y: 40.05,
                            dist: 2.0,
                            dist_top: 1.0,
                            dist_bottom: 3.0,
                            ..Default::default()
                        }
                    };
                    let d = inserted(kind, f, profile);
                    let reopened =
                        wordcraft_docx::read(&wordcraft_docx::write(&d).unwrap()).unwrap();
                    let original = frames(&layout(&d), &path);
                    assert_eq!(original.len(), 1);
                    for (stage, model) in [&d, &reopened].into_iter().enumerate() {
                        let l = layout(model);
                        assert_eq!(l.pages.len(), 1);
                        let labels=l.pages[0].items.iter().filter(|item|matches!(item,Placed::Lines{story,..} if *story==wordcraft_layout::DECOR_STORY)).count();
                        assert_eq!(labels > 0, profile == 2);
                        assert!(
                            contained(&l, &path, 2.0),
                            "{kind:?}/{wrap:?}/{h_rel:?}/profile{profile}/stage{stage}"
                        );
                        let found = frames(&l, &path);
                        assert_eq!(found.len(), 1);
                        let r = found[0].1;
                        for (a, b) in [r.x, r.y, r.w, r.h].into_iter().zip([
                            original[0].1.x,
                            original[0].1.y,
                            original[0].1.w,
                            original[0].1.h,
                        ]) {
                            assert!((a - b).abs() < 0.03, "readback {a}!={b}");
                        }
                        assert_eq!((r.w, r.h), (40.0, 20.0));
                        if wrap != Wrap::Inline {
                            let origin = l.pages[0]
                                .items
                                .iter()
                                .find_map(|it| match it {
                                    Placed::Object {
                                        story: StoryRef::Body,
                                        path: p,
                                        origin,
                                        ..
                                    } if *p == path => Some(*origin),
                                    _ => None,
                                })
                                .unwrap();
                            let expected_x = match h_rel {
                                Anchor::Page => 10.05,
                                Anchor::Margin => l.pages[0].body.x + 10.05,
                                Anchor::Column => origin.x + 10.05,
                                _ => unreachable!(),
                            };
                            let expected_y = match v_rel {
                                Anchor::Page => 40.05,
                                Anchor::Margin => model.last_section.margin_top + 40.05,
                                Anchor::Paragraph => origin.y + 40.05,
                                _ => unreachable!(),
                            };
                            assert!((r.x-expected_x).abs()<0.03 && (r.y-expected_y).abs()<0.03,"paired anchor {h_rel:?}/{v_rel:?} profile{profile}: {r:?} vs({expected_x},{expected_y})");
                        }
                        let draws: Vec<_> =
                            page_display(model, &l.pages[0], &DisplayOptions::default())
                                .into_iter()
                                .filter_map(|d| match d {
                                    Draw::Shape {
                                        rect, kind: actual, ..
                                    } => {
                                        assert_eq!(actual, kind);
                                        Some(Frame {
                                            x: rect.x,
                                            y: rect.y,
                                            w: rect.w,
                                            h: rect.h,
                                        })
                                    }
                                    _ => None,
                                })
                                .collect();
                        assert_eq!(draws.len(), 1);
                        let drawn = draws[0];
                        for (a, b) in [drawn.x, drawn.y, drawn.w, drawn.h]
                            .into_iter()
                            .zip([r.x, r.y, r.w, r.h])
                        {
                            assert!(
                                (a - b).abs() <= 0.0001,
                                "native frame arithmetic {drawn:?} != {r:?}"
                            );
                        }
                        let pdf = wordcraft_pdf::export(model, &Default::default()).unwrap();
                        let text = crate::pdf::extract(&pdf, None)
                            .unwrap()
                            .pages
                            .iter()
                            .flat_map(|p| &p.blocks)
                            .map(|b| match &b.table {
                                Some(table) => table
                                    .rows
                                    .iter()
                                    .map(|row| row.join("\t"))
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                None => b.text.clone(),
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        for label in ["BodyKeep", "TailKeep"] {
                            assert_eq!(text.matches(label).count(), 1,"{label}: {kind:?}/{wrap:?}/{h_rel:?} profile{profile} stage{stage}: {text:?}");
                        }
                        // The original112 gate retains full72/144/300 positive coverage.
                        // This exact-anchor matrix uses144 plus inline representatives at72/300.
                        let dpis: &[f64] = if wrap == Wrap::Inline && profile == 0 {
                            &[72.0, 144.0, 300.0]
                        } else {
                            &[144.0]
                        };
                        for dpi in dpis {
                            let scale = dpi / 72.0;
                            let mut ink = Vec::new();
                            for png in [
                                render_with(model, &l, 0, *dpi, None).unwrap(),
                                crate::pdf::render(&pdf, 0, *dpi, None).unwrap(),
                            ] {
                                assert!(!mask(&png, true).is_empty());
                                let b = bounds(&mask(&png, false));
                                for (actual, want) in b.into_iter().zip([
                                    f64::from(r.x - 1.0) * scale,
                                    f64::from(r.y - 1.0) * scale,
                                    f64::from(r.x + r.w + 1.0) * scale,
                                    f64::from(r.y + r.h + 1.0) * scale,
                                ]) {
                                    assert!((actual - want).abs() <= 3.0, "ink {actual}!={want}");
                                }
                                ink.push(b);
                            }
                            for (a, b) in ink[0].into_iter().zip(ink[1]) {
                                assert!((a - b).abs() <= 2.0, "backend parity {a}!={b}");
                            }
                        }
                    }
                }
            }
        }
    }
    let f = Float {
        wrap: Wrap::BehindText,
        h_rel: Anchor::Page,
        v_rel: Anchor::Page,
        x: 1.0,
        y: 1.0,
        ..Default::default()
    };
    let d = inserted(ShapeKind::Rectangle, f.clone(), 0);
    let l = layout(&d);
    assert!(contained(&l, &path, 2.0));
    let mut missing = l.clone();
    missing.pages[0]
        .items
        .retain(|item| !matches!(item,Placed::Object{path:p,..} if *p==path));
    assert!(!contained(&missing, &path, 2.0));
    let mut duplicate = l.clone();
    let item = duplicate.pages[0]
        .items
        .iter()
        .find(|item| matches!(item,Placed::Object{path:p,..} if *p==path))
        .unwrap()
        .clone();
    duplicate.pages[0].items.push(item);
    assert!(!contained(&duplicate, &path, 2.0));
    let mut edge = f.clone();
    edge.x = 0.5;
    let edge = layout(&inserted(ShapeKind::Rectangle, edge, 0));
    assert!(contained(&edge, &path, 0.0));
    assert!(
        !contained(&edge, &path, 2.0),
        "fill frame alone misses clipped outline"
    );
    let mut below = f;
    below.y = 350.0;
    assert!(!contained(
        &layout(&inserted(ShapeKind::Rectangle, below, 0)),
        &path,
        2.0
    ));
    for (width, height) in [(400.0, 20.0), (40.0, 400.0), (5000.0, 20.0)] {
        let mut oversized = d.clone();
        let InlineObject::Shape { w, h, .. } =
            &mut oversized.para_mut(StoryRef::Body, &path).unwrap().objects[0]
        else {
            unreachable!()
        };
        *w = width;
        *h = height;
        let l = layout(&oversized);
        assert!(!contained(&l, &path, 2.0));
        let actual = frames(&l, &path);
        assert_eq!(actual.len(), 1);
        if width == 5000.0 {
            assert_eq!(actual[0].1.w, 4000.0);
            assert_ne!(
                actual[0].1.w, width,
                "native clamp must be detectable, never admitted"
            );
        }
    }
}
