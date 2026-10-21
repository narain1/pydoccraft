//! Raw pinned-native comment proof only; no authoring API or runtime change.
use super::*;
use wordcraft_layout::display::{page_display, DisplayOptions, Draw};

fn add_comment(
    doc: &mut Document,
    path: &Path,
    start: usize,
    end: usize,
    id: u32,
    parent: Option<u32>,
    text: &str,
) -> u32 {
    let part = doc
        .try_add_part(
            PartKind::Comment,
            vec![wordcraft_doc::para_block(Paragraph::with_text(
                text,
                CharProps::default(),
            ))],
        )
        .unwrap();
    assert!(doc
        .comments
        .insert(
            id,
            wordcraft_doc::Comment {
                author: "Ada 😀".into(),
                date: "2026-10-20T12:00:00Z".into(),
                parent,
                part,
                ..Default::default()
            },
        )
        .is_none());
    if parent.is_none() {
        for (off, object) in [
            (end, InlineObject::CommentEnd { id }),
            (start, InlineObject::CommentStart { id }),
        ] {
            doc.insert_object(
                &Pos::new(StoryRef::Body, path.clone(), off),
                object,
                &CharProps::default(),
            )
            .unwrap();
        }
    }
    part
}

fn layout(doc: &Document) -> wordcraft_layout::DocLayout {
    wordcraft_layout::layout(doc, &mut Default::default(), &Default::default())
}

fn render(doc: &Document, dpi: f32, markup: bool) -> Vec<u8> {
    let pages = layout(doc);
    let mut options = wordcraft_render::RenderOptions::default();
    options.display.markup = markup;
    options.display.dim_header = false;
    wordcraft_render::render_page(doc, &pages.pages[0], dpi / 72.0, &options).pixels
}

fn marker_offsets(doc: &Document, path: &Path, id: u32) -> (usize, usize) {
    let para = doc.para(StoryRef::Body, path).unwrap();
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for offset in para.object_offsets() {
        match para.object_at(offset) {
            Some(InlineObject::CommentStart { id: found }) if *found == id => starts.push(offset),
            Some(InlineObject::CommentEnd { id: found }) if *found == id => ends.push(offset),
            _ => (),
        }
    }
    assert_eq!(starts.len(), 1);
    assert_eq!(ends.len(), 1);
    (starts[0], ends[0])
}

fn drop_comment(doc: &mut Document, path: &Path, id: u32) -> u32 {
    let info = doc.comments.get(&id).unwrap().clone();
    assert!(!doc
        .comments
        .values()
        .any(|comment| comment.parent == Some(id)));
    if info.parent.is_none() {
        let (start, end) = marker_offsets(doc, path, id);
        for off in [end, start] {
            doc.delete_range(
                &Pos::new(StoryRef::Body, path.clone(), off),
                &Pos::new(StoryRef::Body, path.clone(), off + 3),
            )
            .unwrap();
        }
    }
    doc.comments.remove(&id).unwrap();
    assert_eq!(
        doc.parts.remove(&info.part).unwrap().kind,
        PartKind::Comment
    );
    info.part
}

fn glyphs(doc: &Document) -> Vec<(u32, f32, f32)> {
    let pages = layout(doc);
    page_display(doc, &pages.pages[0], &DisplayOptions::default())
        .into_iter()
        .flat_map(|draw| match draw {
            Draw::Glyphs { glyphs, .. } => glyphs,
            _ => Vec::new(),
        })
        .collect()
}

type ClusterProof = (usize, usize, f32, Vec<(u32, f32, f32)>);

fn clusters(doc: &Document) -> Vec<ClusterProof> {
    let pages = layout(doc);
    let source = doc.para(StoryRef::Body, &Path::top(0)).unwrap();
    let marker_offsets = source.object_offsets();
    let clean = |offset| offset - 3 * marker_offsets.iter().filter(|at| **at < offset).count();
    let para = pages.pages[0]
        .items
        .iter()
        .find_map(|item| match item {
            wordcraft_layout::Placed::Lines { para, .. } => Some(para),
            _ => None,
        })
        .unwrap();
    para.clusters
        .iter()
        .filter(|c| c.kind != wordcraft_layout::para::ClKind::Marker)
        .map(|c| {
            (
                clean(c.start),
                clean(c.end),
                c.adv,
                para.glyphs[c.g0 as usize..c.g1 as usize]
                    .iter()
                    .map(|g| (g.gid, g.dx, g.dy))
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn upstream_comment_markers_endpoint_shaping_and_interior_arabic_control() {
    for (text, rtl) in [
        ("office", false),
        ("سلام", true),
        ("e\u{301} office 😀 tail", false),
    ] {
        let mut control = Document::from_text(text);
        control.styles.default_chr.size = Some(36.0);
        control
            .para_mut(StoryRef::Body, &Path::top(0))
            .unwrap()
            .props
            .bidi = Some(rtl);
        let before_glyphs = glyphs(&control);
        let before_clusters = clusters(&control);
        let mut full = control.clone();
        add_comment(
            &mut full,
            &Path::top(0),
            0,
            text.len(),
            0,
            None,
            "ReviewOnly",
        );
        assert_eq!(glyphs(&full), before_glyphs, "whole range glyphs: {text}");
        assert_eq!(
            clusters(&full),
            before_clusters,
            "whole range clusters: {text}"
        );
        for dpi in [72.0, 144.0] {
            assert_eq!(
                render(&full, dpi, false),
                render(&control, dpi, false),
                "whole range ink: {text}"
            );
            assert_ne!(
                render(&full, dpi, true),
                render(&control, dpi, true),
                "range shading: {text}"
            );
        }
        // Interior boundaries deliberately split native shaping segments.
        let mut inside = control.clone();
        let offsets: Vec<_> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        add_comment(
            &mut inside,
            &Path::top(0),
            offsets[1],
            offsets[3],
            0,
            None,
            "Interior",
        );
        eprintln!(
            "comment interior {text:?}: glyphs_changed={} clusters_changed={} ink_changed={}",
            glyphs(&inside) != before_glyphs,
            clusters(&inside) != before_clusters,
            render(&inside, 144.0, false) != render(&control, 144.0, false)
        );
        if rtl {
            assert_ne!(
                glyphs(&inside),
                before_glyphs,
                "raw Arabic joining defect remains visible without shading"
            );
            assert_ne!(
                render(&inside, 144.0, false),
                render(&control, 144.0, false)
            );
        }
        let reopened = wordcraft_docx::read(&wordcraft_docx::write(&full).unwrap()).unwrap();
        assert_eq!(glyphs(&reopened), before_glyphs);
        assert_eq!(clusters(&reopened), before_clusters);
    }
}

#[test]
fn upstream_comment_storage_unicode_multirun_cells_replies_and_resolved() {
    for scope in ["body", "ordinary", "nested", "merged"] {
        let mut fixture = EditableDocx::new();
        let text = "Before 😀 e\u{301} office سلام after";
        let id = if scope == "body" {
            fixture.add_paragraph(text).unwrap()
        } else {
            let outer = fixture
                .add_table(vec![vec![text.into(), "OtherKeep".into()]])
                .unwrap();
            let table = if scope == "nested" {
                fixture.set_cell_text(outer, 0, 0, "OuterKeep").unwrap();
                fixture
                    .cell_add_table(outer, 0, 0, vec![vec![text.into()]])
                    .unwrap()
            } else {
                if scope == "merged" {
                    fixture.merge_cells(outer, 0, 0, 0, 1).unwrap();
                }
                outer
            };
            fixture.cell_paragraph_ids(table, 0, 0).unwrap()[0]
        };
        fixture
            .format_range_patch(
                id,
                0,
                6,
                fixture.revision,
                r##"{"bold":true,"color":"#123456"}"##,
            )
            .unwrap();
        let path = fixture.location(id).unwrap().1;
        let mut native = fixture.document().unwrap().clone();
        let original_native = native.clone();
        let original = native.para(StoryRef::Body, &path).unwrap().clone();
        let body = native.plain_text(StoryRef::Body);
        let part = add_comment(
            &mut native,
            &path,
            0,
            original.len(),
            7,
            None,
            "ReviewOnly 😀 e\u{301}",
        );
        let reply = add_comment(&mut native, &path, 0, 0, 8, Some(7), "");
        native.comments.get_mut(&7).unwrap().resolved = true;
        native.comments.get_mut(&8).unwrap().resolved = true;
        native.comments.get_mut(&8).unwrap().author.clear();
        native.comments.get_mut(&8).unwrap().date.clear();
        assert_eq!(native.plain_text(StoryRef::Body), body);
        assert_eq!(native.parts[&part].kind, PartKind::Comment);
        assert_eq!(native.parts[&reply].blocks.len(), 1);
        let marked = native.para(StoryRef::Body, &path).unwrap();
        assert_eq!(marked.plain_text(), original.plain_text());
        for (offset, _) in original.text.char_indices() {
            assert_eq!(
                marked.props_of_char(offset + 3),
                original.props_of_char(offset)
            );
        }
        let mut encoded = wordcraft_docx::write(&native).unwrap();
        let first = wordcraft_docx::read(&encoded).unwrap();
        let original_reopened =
            wordcraft_docx::read(&wordcraft_docx::write(&original_native).unwrap()).unwrap();
        let baseline_reopened_layout = layout(&original_reopened);
        let baseline_layout = layout(&original_native);
        let marked_layout = layout(&native);
        let reopened_layout = layout(&first);
        assert_eq!(
            marked_layout.pages.len(),
            baseline_layout.pages.len(),
            "{scope}"
        );
        assert_eq!(
            reopened_layout.pages.len(),
            baseline_layout.pages.len(),
            "{scope}"
        );
        let baseline_pdf =
            crate::docx::to_pdf(&wordcraft_docx::write(&original_native).unwrap(), None).unwrap();
        let marked_pdf = crate::docx::to_pdf(&encoded, None).unwrap();
        for (index, baseline_page) in baseline_layout.pages.iter().enumerate() {
            for dpi in [72.0, 144.0] {
                let mut options = wordcraft_render::RenderOptions::default();
                options.display.markup = false;
                let baseline = wordcraft_render::render_page(
                    &original_native,
                    baseline_page,
                    dpi / 72.0,
                    &options,
                );
                let marked = wordcraft_render::render_page(
                    &native,
                    &marked_layout.pages[index],
                    dpi / 72.0,
                    &options,
                );
                let reopened = wordcraft_render::render_page(
                    &first,
                    &reopened_layout.pages[index],
                    dpi / 72.0,
                    &options,
                );
                assert_eq!(
                    marked.pixels, baseline.pixels,
                    "{scope} native marked/unmarked ink"
                );
                let baseline_reopened = wordcraft_render::render_page(
                    &original_reopened,
                    &baseline_reopened_layout.pages[index],
                    dpi / 72.0,
                    &options,
                );
                eprintln!("comment {scope} dpi={dpi}: unmarked_roundtrip_ink_changed={} comment_reopened_ink_changed={}",baseline_reopened.pixels!=baseline.pixels,reopened.pixels!=baseline_reopened.pixels);
                assert!(
                    reopened.pixels == baseline_reopened.pixels,
                    "{scope} reopened marked/unmarked ink"
                );
                options.display.markup = true;
                let marked = wordcraft_render::render_page(
                    &native,
                    &marked_layout.pages[index],
                    dpi / 72.0,
                    &options,
                );
                let reopened = wordcraft_render::render_page(
                    &first,
                    &reopened_layout.pages[index],
                    dpi / 72.0,
                    &options,
                );
                let twice = wordcraft_docx::read(&wordcraft_docx::write(&first).unwrap()).unwrap();
                let twice_layout = layout(&twice);
                let twice_render = wordcraft_render::render_page(
                    &twice,
                    &twice_layout.pages[index],
                    dpi / 72.0,
                    &options,
                );
                assert!(
                    reopened.pixels == twice_render.pixels,
                    "{scope} reopened actual shaded preview"
                );
                assert_ne!(
                    marked.pixels, baseline.pixels,
                    "{scope} anchored range is shaded"
                );
                assert_eq!(
                    crate::pdf::render(&marked_pdf, index, f64::from(dpi), None).unwrap(),
                    crate::pdf::render(&baseline_pdf, index, f64::from(dpi), None).unwrap(),
                    "{scope} default PDF"
                );
            }
        }
        let first_durable = first.comments[&7].durable_id.clone();
        assert!(first_durable.is_some());
        for _ in 0..2 {
            let read = wordcraft_docx::read(&encoded).unwrap();
            assert_eq!(read.plain_text(StoryRef::Body), body);
            assert_eq!(read.comments.len(), 2);
            let root = &read.comments[&7];
            assert_eq!(root.author, "Ada 😀");
            assert_eq!(root.date, "2026-10-20T12:00:00Z");
            assert_eq!(root.date_utc.as_deref(), Some("2026-10-20T12:00:00Z"));
            assert_eq!(root.parent, None);
            assert!(root.resolved);
            assert_eq!(root.durable_id, first_durable);
            assert_eq!(
                read.plain_text(StoryRef::Part(root.part)),
                "ReviewOnly 😀 e\u{301}"
            );
            let child = &read.comments[&8];
            assert_eq!(child.parent, Some(7));
            assert!(child.resolved);
            assert_eq!(
                child.author, "Unknown",
                "native annotation_author explicitly substitutes empty authors"
            );
            assert!(child.date.is_empty() && child.date_utc.is_none());
            assert_eq!(read.plain_text(StoryRef::Part(child.part)), "");
            let (a, b) = marker_offsets(&read, &path, 7);
            assert_eq!(
                &read.para(StoryRef::Body, &path).unwrap().text[a + 3..b],
                &original.text
            );
            let extracted = crate::docx::extract(&encoded, None).unwrap();
            let comments: Vec<_> = extracted
                .pages
                .iter()
                .flat_map(|p| &p.blocks)
                .filter(|b| b.kind == "comment")
                .collect();
            assert_eq!(comments.len(), 2);
            assert_eq!(comments[0].text, "ReviewOnly 😀 e\u{301}");
            assert!(comments[1].text.is_empty());
            assert_eq!(comments[1].metadata["parent_id"], 7);
            assert!(!extracted
                .pages
                .iter()
                .flat_map(|p| &p.blocks)
                .any(|b| b.kind != "comment" && b.text.contains("ReviewOnly")));
            encoded = wordcraft_docx::write(&read).unwrap();
        }
    }
}

fn blue_png() -> Vec<u8> {
    let mut data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut data, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0, 0, 255, 255].repeat(4))
            .unwrap();
    }
    data
}

#[test]
fn upstream_comment_update_delete_keep_unrelated_media_sections_and_parts() {
    let mut fixture = EditableDocx::new();
    let id = fixture.add_paragraph("OwnedRangeKeep").unwrap();
    let target = fixture.add_paragraph("TargetKeep").unwrap();
    fixture
        .set_bookmark("OtherTarget", target, 0, 10, fixture.revision)
        .unwrap();
    let jump = fixture.add_paragraph("JumpKeep").unwrap();
    fixture
        .set_range_link(jump, 0, 8, fixture.revision, Some("#OtherTarget"))
        .unwrap();
    fixture
        .add_image(&blue_png(), Some(16.0), Some(16.0), "BlueKeep")
        .unwrap();
    fixture.add_section("next_page").unwrap();
    let note_anchor = fixture.add_paragraph("LaterKeep").unwrap();
    fixture
        .add_note(note_anchor, 9, 9, fixture.revision, "NoteKeep", "footnote")
        .unwrap();
    let path = fixture.location(id).unwrap().1;
    let mut native = fixture.document().unwrap().clone();
    let original = native.clone();
    let original_media = native.media.clone();
    let original_sections = owned_body_sections(&native).unwrap();
    let original_parts = native.parts.clone();
    let original_semantics = semantic_anchors(&native);
    let original_body = native.plain_text(StoryRef::Body);
    let rootpart = add_comment(
        &mut native,
        &path,
        0,
        "OwnedRangeKeep".len(),
        0,
        None,
        "ReviewOnly",
    );
    let replypart = add_comment(&mut native, &path, 0, 0, 1, Some(0), "ReplyOnly");
    native
        .set_story(
            StoryRef::Part(rootpart),
            vec![wordcraft_doc::para_block(Paragraph::with_text(
                "UpdatedOnly",
                CharProps::default(),
            ))],
        )
        .unwrap();
    let info = native.comments.get_mut(&0).unwrap();
    info.author = "Ben".into();
    info.date = "2026-10-21T00:00:00Z".into();
    info.resolved = true;
    assert_eq!(native.media, original_media);
    assert_eq!(owned_body_sections(&native).unwrap(), original_sections);
    assert_eq!(native.plain_text(StoryRef::Body), original_body);
    for (id, part) in &original_parts {
        assert_eq!(&native.parts[id], part);
    }
    let saved = wordcraft_docx::write(&native).unwrap();
    let pdf = crate::docx::to_pdf(&saved, None).unwrap();
    let baseline_pdf =
        crate::docx::to_pdf(&wordcraft_docx::write(&original).unwrap(), None).unwrap();
    let actual = crate::pdf::extract(&pdf, None).unwrap();
    let baseline = crate::pdf::extract(&baseline_pdf, None).unwrap();
    assert_eq!(actual.pages.len(), baseline.pages.len());
    assert!(actual
        .pages
        .iter()
        .flat_map(|page| &page.links)
        .any(|link| link.page == Some(0)));
    for (page, expected) in actual.pages.iter().zip(&baseline.pages) {
        assert_eq!(
            serde_json::to_value(&page.links).unwrap(),
            serde_json::to_value(&expected.links).unwrap()
        );
        assert_eq!(
            crate::pdf::render(&pdf, page.number, 72.0, None).unwrap(),
            crate::pdf::render(&baseline_pdf, page.number, 72.0, None).unwrap()
        );
    }
    let read = wordcraft_docx::read(&saved).unwrap();
    let info = &read.comments[&0];
    assert_eq!(info.author, "Ben");
    assert_eq!(info.date, "2026-10-21T00:00:00Z");
    assert_eq!(info.date_utc.as_deref(), Some("2026-10-21T00:00:00Z"));
    assert!(info.resolved);
    assert_eq!(read.plain_text(StoryRef::Part(info.part)), "UpdatedOnly");
    assert_eq!(drop_comment(&mut native, &path, 1), replypart);
    assert_eq!(drop_comment(&mut native, &path, 0), rootpart);
    assert!(native.comments.is_empty());
    assert_eq!(native.parts, original_parts);
    assert_eq!(native.media, original_media);
    assert_eq!(owned_body_sections(&native).unwrap(), original_sections);
    assert_eq!(semantic_anchors(&native), original_semantics);
    assert_eq!(native.body, original.body);
    for dpi in [72.0, 144.0] {
        assert_eq!(render(&native, dpi, true), render(&original, dpi, true));
    }
    let read = wordcraft_docx::read(&wordcraft_docx::write(&native).unwrap()).unwrap();
    assert!(read.comments.is_empty());
    assert!(!read
        .parts
        .values()
        .any(|part| part.kind == PartKind::Comment));
    let reused = add_comment(
        &mut native,
        &path,
        0,
        "OwnedRangeKeep".len(),
        2,
        None,
        "NewOnly",
    );
    assert_eq!(
        reused, rootpart,
        "native part IDs may be reused after deletion"
    );
    assert_eq!(
        wordcraft_doc::free_id(None, std::iter::empty(), 0),
        Some(0),
        "native comment free_id also reuses an empty map"
    );
    assert!(native.comments.contains_key(&2) && !native.comments.contains_key(&0));
}

fn shades(doc: &Document) -> Vec<(f32, f32, f32, f32)> {
    let pages = layout(doc);
    page_display(doc, &pages.pages[0], &DisplayOptions::default())
        .into_iter()
        .filter_map(|draw| match draw {
            Draw::Fill {
                rect,
                color: Rgb(0xEF, 0xE3, 0xF7),
                ..
            } => Some((rect.x, rect.y, rect.w, rect.h)),
            _ => None,
        })
        .collect()
}

#[test]
fn upstream_comment_adjacent_order_overlap_shading_and_default_pdf_display() {
    multiline_comment_control();
    let control = Document::from_text("abcdefghij");
    for reverse in [false, true] {
        let mut adjacent = control.clone();
        if reverse {
            add_comment(&mut adjacent, &Path::top(0), 4, 8, 1, None, "SecondOnly");
            add_comment(&mut adjacent, &Path::top(0), 0, 4, 0, None, "FirstOnly");
        } else {
            add_comment(&mut adjacent, &Path::top(0), 0, 4, 0, None, "FirstOnly");
            add_comment(&mut adjacent, &Path::top(0), 10, 14, 1, None, "SecondOnly");
        }
        let first = marker_offsets(&adjacent, &Path::top(0), 0);
        let second = marker_offsets(&adjacent, &Path::top(0), 1);
        assert_eq!(first.1 + 3, second.0, "END then START at a shared endpoint");
        assert_eq!(shades(&adjacent).len(), 2);
        let raw = wordcraft_docx::write(&adjacent).unwrap();
        let read = wordcraft_docx::read(&raw).unwrap();
        assert_eq!(shades(&read), shades(&adjacent));
    }
    let mut overlap = control.clone();
    // Final visible source ranges [2,6] and [4,8], with nested marker order S0,S1,E0,E1.
    add_comment(&mut overlap, &Path::top(0), 2, 6, 0, None, "One");
    add_comment(&mut overlap, &Path::top(0), 7, 14, 1, None, "Two");
    let pages = layout(&overlap);
    let first = marker_offsets(&overlap, &Path::top(0), 0).0;
    let start = pages.caret(&Pos::body(0, first)).unwrap().x;
    let fills = shades(&overlap);
    assert!(
        fills.iter().any(|r| r.0 < start - 1.0),
        "native overlap shading incorrectly includes unanchored prefix: {fills:?}"
    );

    let mut full = control.clone();
    add_comment(
        &mut full,
        &Path::top(0),
        0,
        control.para(StoryRef::Body, &Path::top(0)).unwrap().len(),
        0,
        None,
        "ReviewOnly",
    );
    add_comment(&mut full, &Path::top(0), 0, 0, 1, Some(0), "ReplyOnly");
    let before = render(&full, 144.0, true);
    for resolved in [true, false] {
        full.comments.get_mut(&0).unwrap().resolved = resolved;
        full.comments.get_mut(&1).unwrap().resolved = resolved;
        assert_eq!(
            render(&full, 144.0, true),
            before,
            "resolved state does not suppress native range shading"
        );
        let raw = wordcraft_docx::write(&full).unwrap();
        let read = wordcraft_docx::read(&raw).unwrap();
        assert_eq!(render(&read, 144.0, true), before);
        let pdf = crate::docx::to_pdf(&raw, None).unwrap();
        let cleanpdf =
            crate::docx::to_pdf(&wordcraft_docx::write(&control).unwrap(), None).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        let text = extracted
            .pages
            .iter()
            .flat_map(|p| &p.blocks)
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("abcdefghij"));
        assert!(!text.contains("ReviewOnly") && !text.contains("ReplyOnly"));
        for dpi in [72.0, 144.0] {
            assert_eq!(
                crate::pdf::render(&pdf, 0, dpi, None).unwrap(),
                crate::pdf::render(&cleanpdf, 0, dpi, None).unwrap(),
                "default PDF omits markup"
            );
        }
    }
}

#[test]
fn upstream_comment_absent_durable_ids_are_reassigned_after_prior_deletion() {
    let mut native = Document::from_text("First\nSecond");
    add_comment(&mut native, &Path::top(0), 0, 5, 0, None, "FirstOnly");
    add_comment(&mut native, &Path::top(1), 0, 6, 1, None, "SecondOnly");
    let first = wordcraft_docx::read(&wordcraft_docx::write(&native).unwrap()).unwrap();
    let stable = first.comments[&1].durable_id.clone().unwrap();
    assert!(
        native.comments.values().all(|c| c.durable_id.is_none()),
        "writer does not mutate source IDs"
    );
    drop_comment(&mut native, &Path::top(0), 0);
    let second = wordcraft_docx::read(&wordcraft_docx::write(&native).unwrap()).unwrap();
    assert_ne!(
        second.comments[&1].durable_id.as_ref().unwrap(),
        &stable,
        "raw writer assigns absent durable IDs by current order"
    );
    let mut explicit = Document::from_text("First\nSecond");
    add_comment(&mut explicit, &Path::top(0), 0, 5, 0, None, "FirstOnly");
    add_comment(&mut explicit, &Path::top(1), 0, 6, 1, None, "SecondOnly");
    for (id, comment) in &mut explicit.comments {
        comment.durable_id = Some(format!("{:08X}", id.checked_add(1).unwrap()));
    }
    let saved_explicit = wordcraft_docx::read(&wordcraft_docx::write(&explicit).unwrap()).unwrap();
    assert_eq!(
        saved_explicit.comments[&1].durable_id.as_deref(),
        Some("00000002")
    );
    drop_comment(&mut explicit, &Path::top(0), 0);
    let saved_explicit = wordcraft_docx::read(&wordcraft_docx::write(&explicit).unwrap()).unwrap();
    assert_eq!(
        saved_explicit.comments[&1].durable_id.as_deref(),
        Some("00000002")
    );
    let mut retained = first;
    drop_comment(&mut retained, &Path::top(0), 0);
    let third = wordcraft_docx::read(&wordcraft_docx::write(&retained).unwrap()).unwrap();
    assert_eq!(
        third.comments[&1].durable_id.as_ref().unwrap(),
        &stable,
        "explicit native durable ID is preserved"
    );
}

fn multiline_comment_control() {
    for text in [
        "First line\nSecond e\u{301} office سلام 😀 line".to_owned(),
        "office e\u{301} سلام 😀 words ".repeat(60),
    ] {
        let mut control = Document::from_text("placeholder");
        *control.para_mut(StoryRef::Body, &Path::top(0)).unwrap() =
            Paragraph::with_text(&text, CharProps::default());
        control.last_section.page_w = 320.0;
        control.last_section.page_h = 300.0;
        control.last_section.margin_left = 24.0;
        control.last_section.margin_right = 24.0;
        control.last_section.margin_top = 24.0;
        control.last_section.margin_bottom = 24.0;
        let mut marked = control.clone();
        add_comment(
            &mut marked,
            &Path::top(0),
            0,
            text.len(),
            0,
            None,
            "ReviewOnly",
        );
        let before = layout(&control);
        let after = layout(&marked);
        assert_eq!(after.pages.len(), before.pages.len());
        if !text.contains('\n') {
            assert!(after.pages.len() > 1);
        }
        let clean_pdf =
            crate::docx::to_pdf(&wordcraft_docx::write(&control).unwrap(), None).unwrap();
        let saved = wordcraft_docx::write(&marked).unwrap();
        let reopened = wordcraft_docx::read(&saved).unwrap();
        let reopened_layout = layout(&reopened);
        let marked_pdf = crate::docx::to_pdf(&saved, None).unwrap();
        for (index, (page, baseline)) in after.pages.iter().zip(&before.pages).enumerate() {
            let draws = page_display(&marked, page, &DisplayOptions::default());
            let shades = draws
                .iter()
                .filter(|draw| {
                    matches!(
                        draw,
                        Draw::Fill {
                            color: Rgb(0xEF, 0xE3, 0xF7),
                            ..
                        }
                    )
                })
                .count();
            assert!(
                shades > 0,
                "every page of the automatically split paragraph is shaded"
            );
            let glyphs = |doc: &Document, page: &wordcraft_layout::Page| {
                page_display(doc, page, &DisplayOptions::default())
                    .into_iter()
                    .flat_map(|draw| match draw {
                        Draw::Glyphs { glyphs, .. } => glyphs,
                        _ => Vec::new(),
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(glyphs(&marked, page), glyphs(&control, baseline));
            assert_eq!(
                glyphs(&reopened, &reopened_layout.pages[index]),
                glyphs(&control, baseline)
            );
            for dpi in [72.0, 144.0] {
                let mut options = wordcraft_render::RenderOptions::default();
                options.display.markup = false;
                let actual = wordcraft_render::render_page(&marked, page, dpi / 72.0, &options);
                let expected =
                    wordcraft_render::render_page(&control, baseline, dpi / 72.0, &options);
                assert_eq!(
                    actual.pixels, expected.pixels,
                    "multiline/split visible ink"
                );
                assert_eq!(
                    crate::pdf::render(&marked_pdf, index, f64::from(dpi), None).unwrap(),
                    crate::pdf::render(&clean_pdf, index, f64::from(dpi), None).unwrap()
                );
            }
        }
    }
}
