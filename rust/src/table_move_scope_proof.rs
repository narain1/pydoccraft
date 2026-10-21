//! Raw pinned-native table movement proof; no table movement adapter is implemented.
use super::*;

fn png_color(color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&color.repeat(4))
            .unwrap();
    }
    bytes
}
fn table_in(owner: &mut EditableDocx, scope: &str, parent: Option<usize>, text: &str) -> usize {
    let rows = vec![vec![text.to_owned()]];
    if let Some(parent) = parent {
        return owner.cell_add_table(parent, 0, 0, rows).unwrap();
    }
    if scope == "body" {
        owner.add_table(rows).unwrap()
    } else {
        let (kind, variant) = scope.split_once('/').unwrap();
        let part = owner.ensure_story(kind, variant, 0).unwrap();
        owner.story_add_table(part, rows).unwrap()
    }
}
fn paragraph_in(owner: &mut EditableDocx, scope: &str, parent: Option<usize>, text: &str) -> usize {
    if let Some(parent) = parent {
        owner.cell_add_paragraph(parent, 0, 0, text).unwrap()
    } else if scope == "body" {
        owner.add_paragraph(text).unwrap()
    } else {
        let (kind, variant) = scope.split_once('/').unwrap();
        let part = owner.ensure_story(kind, variant, 0).unwrap();
        owner.story_add_paragraph(part, text).unwrap()
    }
}
fn outer(owner: &mut EditableDocx, scope: &str, shape: &str) -> Option<usize> {
    if shape == "direct" {
        return None;
    }
    let rows = vec![
        vec!["OuterKeep".into(), "RightKeep".into()],
        vec!["BelowKeep".into(), "LastKeep".into()],
    ];
    let id = if scope == "body" {
        owner.add_table(rows).unwrap()
    } else {
        let (kind, variant) = scope.split_once('/').unwrap();
        let part = owner.ensure_story(kind, variant, 0).unwrap();
        owner.story_add_table(part, rows).unwrap()
    };
    match shape {
        "horizontal" => owner.merge_cells(id, 0, 0, 0, 1).unwrap(),
        "vertical" => owner.merge_cells(id, 0, 0, 1, 0).unwrap(),
        _ => (),
    }
    Some(id)
}
fn grid(table: &Table) -> String {
    format!(
        "{:?}|{:?}|{:?}",
        table.grid,
        table.props,
        table
            .rows
            .iter()
            .map(|row| (
                &row.props,
                row.cells.iter().map(|cell| &cell.props).collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>()
    )
}
fn root_comment(owner: &mut EditableDocx, paragraph: usize) -> u32 {
    let end = owner.paragraph(paragraph).unwrap().text.chars().count();
    owner
        .add_comment(
            Some(paragraph),
            0,
            end,
            owner.revision,
            None,
            "ReviewKeep",
            "Ada",
            "",
        )
        .unwrap()
}

fn rotated(path: &Path, source: &Path, target: &Path) -> Path {
    let mut result = path.clone();
    let depth = source.0.len() - 1;
    if result.0.len() > depth && result.0[..depth] == source.0[..depth] {
        let (value, from, to) = (result.0[depth], source.0[depth], target.0[depth]);
        result.0[depth] = if value == from {
            to
        } else if from < to && from < value && value <= to {
            value - 1
        } else if from > to && to <= value && value < from {
            value + 1
        } else {
            value
        };
    }
    result
}
fn move_native(document: &Document, story: StoryRef, source: &Path, target: &Path) -> Document {
    let mut candidate = document.clone();
    let moved = candidate.remove_block(story, source).unwrap();
    candidate.insert_block(story, target, moved).unwrap();
    candidate
}
fn story_text(document: &Document) -> String {
    std::iter::once(document.plain_text(StoryRef::Body))
        .chain(
            document
                .parts
                .keys()
                .map(|part| document.plain_text(StoryRef::Part(*part))),
        )
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn upstream_table_move_preserves_parent_subtree_identity_tails_and_semantic_diagnostics() {
    pyo3::prepare_freethreaded_python();
    for scope in [
        "body",
        "header/default",
        "header/first",
        "header/even",
        "footer/default",
        "footer/first",
        "footer/even",
    ] {
        for shape in ["direct", "nested", "horizontal", "vertical"] {
            for authored_empty in [false, true] {
                let mut owner = EditableDocx::new();
                owner.set_margins(180.0, 36.0, 180.0, 36.0).unwrap();
                owner.add_paragraph("BodyKeep").unwrap();
                owner.add_page_break().unwrap();
                owner.add_paragraph("SecondPageKeep").unwrap();
                let parent = outer(&mut owner, scope, shape);
                let mut tables = Vec::new();
                for label in ["FirstMove", "MiddleMove", "LastMove"] {
                    let table = table_in(&mut owner, scope, parent, label);
                    owner
                        .cell_add_table(table, 0, 0, vec![vec!["DeepKeep".into()]])
                        .unwrap();
                    owner
                        .cell_add_image(
                            table,
                            0,
                            0,
                            &png_color([0, 0, 128, 255]),
                            Some(8.0),
                            Some(8.0),
                            "ImageKeep",
                        )
                        .unwrap();
                    let paragraph = owner.cell_paragraph_ids(table, 0, 0).unwrap()[0];
                    owner
                        .set_range_link(
                            paragraph,
                            0,
                            label.chars().count(),
                            owner.revision,
                            Some("https://example.com"),
                        )
                        .unwrap();
                    owner.add_paragraph_field(paragraph, "page_count").unwrap();
                    tables.push(table);
                    if label != "LastMove" {
                        paragraph_in(&mut owner, scope, parent, "BetweenKeep");
                    }
                }
                if authored_empty {
                    paragraph_in(&mut owner, scope, parent, "");
                }
                let original = owner.document().unwrap();
                let sections = owned_body_sections(original).unwrap();
                for from in 0..3 {
                    for to in 0..3 {
                        if from == to {
                            continue;
                        }
                        let (story, source) = owner.location(tables[from]).unwrap();
                        let target = owner.location(tables[to]).unwrap().1;
                        let before = original.container(story, &source).unwrap();
                        let candidate = move_native(original, story, &source, &target);
                        let after = candidate.container(story, &source).unwrap();
                        assert_eq!(after.len(), before.len(), "{scope}/{shape}");
                        assert!(
                            Arc::ptr_eq(before.last().unwrap(), after.last().unwrap()),
                            "tail {scope}/{shape}"
                        );
                        for (index, block) in before.iter().enumerate() {
                            let destination =
                                rotated(&source.with_last(index), &source, &target).last();
                            assert_eq!(block.as_ref(), after[destination].as_ref());
                            if index != source.last() {
                                assert!(Arc::ptr_eq(block, &after[destination]));
                            }
                        }
                        for path in original.para_paths(story) {
                            let current = rotated(&path, &source, &target);
                            let old = &original.container(story, &path).unwrap()[path.last()];
                            let new =
                                &candidate.container(story, &current).unwrap()[current.last()];
                            assert!(
                                Arc::ptr_eq(old, new),
                                "paragraph descendant {scope}/{shape}"
                            );
                        }
                        for table in &tables {
                            let before_path = owner.location(*table).unwrap().1;
                            let after_path = rotated(&before_path, &source, &target);
                            assert_eq!(
                                candidate.table(story, &after_path),
                                original.table(story, &before_path)
                            );
                        }
                        if let Some(parent) = parent {
                            let path = owner.location(parent).unwrap().1;
                            assert_eq!(
                                grid(candidate.table(story, &path).unwrap()),
                                grid(original.table(story, &path).unwrap())
                            );
                            if shape == "horizontal" {
                                assert_eq!(
                                    cell_anchor(candidate.table(story, &path).unwrap(), 0, 1)
                                        .unwrap(),
                                    (0, 0)
                                );
                            }
                            if shape == "vertical" {
                                assert_eq!(
                                    cell_anchor(candidate.table(story, &path).unwrap(), 1, 0)
                                        .unwrap(),
                                    (0, 0)
                                );
                            }
                        }
                        assert_eq!(
                            content_usage(candidate.story(story).unwrap()),
                            content_usage(original.story(story).unwrap())
                        );
                        assert_eq!(candidate.media, original.media);
                        assert_eq!(owned_body_sections(&candidate).unwrap(), sections);
                        assert_eq!(page_field_count(&candidate).unwrap(), 3);
                        let bytes = wordcraft_docx::write(&candidate).unwrap();
                        let reopened = wordcraft_docx::read(&bytes).unwrap();
                        for label in ["FirstMove", "MiddleMove", "LastMove"] {
                            assert_eq!(story_text(&reopened).matches(label).count(), 1);
                        }
                        assert_eq!(reopened.media.len(), 1);
                        assert_eq!(
                            original.container(story, &source).unwrap(),
                            before,
                            "copy remains unchanged"
                        );
                    }
                }
            }
        }
    }
    // Displaced sibling comments retain exact fresh native paths. A moved anchored subtree
    // is a separate native diagnostic, never authorization for the bounded adapter profile.
    for nested in [false, true] {
        let mut owner = EditableDocx::new();
        let parent = nested.then(|| owner.add_table(vec![vec!["OuterKeep".into()]]).unwrap());
        let first = table_in(&mut owner, "body", parent, "FirstMove");
        let comment_paragraph = paragraph_in(&mut owner, "body", parent, "Retained 😀 comment");
        let comment = root_comment(&mut owner, comment_paragraph);
        owner
            .add_comment(None, 0, 0, 0, Some(comment), "ReplyKeep", "Ben", "")
            .unwrap();
        let last = table_in(&mut owner, "body", parent, "LastMove");
        let (story, source) = owner.location(first).unwrap();
        let target = owner.location(last).unwrap().1;
        let old_comment_path = owner.location(comment_paragraph).unwrap().1;
        let original = owner.document().unwrap();
        let native = move_native(original, story, &source, &target);
        assert_eq!(native.comments, original.comments);
        assert_eq!(native.parts, original.parts);
        let inventory = comments::inventory(&native).unwrap();
        let anchor = inventory.entries[&comment].anchor.as_ref().unwrap();
        assert_eq!(anchor.path, rotated(&old_comment_path, &source, &target));
        assert_eq!(
            &native.para(story, &anchor.path).unwrap().text[3..anchor.end],
            "Retained 😀 comment"
        );
        let mut mapped = owner.copy().unwrap();
        for (s, path) in mapped.locations.values_mut() {
            if *s == story {
                *path = rotated(path, &source, &target);
            }
        }
        mapped.cell_placeholders = mapped
            .cell_placeholders
            .into_iter()
            .map(|(s, path)| {
                (
                    s,
                    if s == story {
                        rotated(&path, &source, &target)
                    } else {
                        path
                    },
                )
            })
            .collect();
        mapped.document = Some(native);
        comments::registry(&mapped, &inventory).unwrap();
        let (id, start, end, revision) = mapped.comment_range(comment).unwrap();
        assert_eq!(id, comment_paragraph);
        assert_eq!(
            mapped.range_text(id, start, end, revision).unwrap(),
            "Retained 😀 comment"
        );
    }
    let mut owner = EditableDocx::new();
    let first = owner
        .add_table(vec![vec!["ProtectedComment".into()]])
        .unwrap();
    let paragraph = owner.cell_paragraph_ids(first, 0, 0).unwrap()[0];
    root_comment(&mut owner, paragraph);
    let bookmark_p = owner
        .cell_add_paragraph(first, 0, 0, "ProtectedBookmark")
        .unwrap();
    owner
        .set_bookmark("Keep", bookmark_p, 0, 17, owner.revision)
        .unwrap();
    let note_p = owner
        .cell_add_paragraph(first, 0, 0, "ProtectedNote")
        .unwrap();
    owner
        .add_note(note_p, 13, 13, owner.revision, "NoteKeep", "footnote")
        .unwrap();
    let last = owner.add_table(vec![vec!["LastMove".into()]]).unwrap();
    let (story, source) = owner.location(first).unwrap();
    let target = owner.location(last).unwrap().1;
    let original = owner.document().unwrap();
    let native = move_native(original, story, &source, &target);
    assert_eq!(semantic_anchors(&native), semantic_anchors(original));
    assert_eq!(native.comments, original.comments);
    assert_eq!(native.parts, original.parts);
    comments::inventory(&native).unwrap();
    let mut unexpected = original.clone();
    let path = owner.location(note_p).unwrap().1;
    unexpected.para_mut(story, &path).unwrap().section =
        Some(Box::new(original.last_section.clone()));
    let native = move_native(&unexpected, story, &source, &target);
    assert!(native
        .para(story, &rotated(&path, &source, &target))
        .unwrap()
        .section
        .is_some());
}

fn color_top(pixels: &[u8], width: u32, color: [u8; 4]) -> Option<usize> {
    pixels
        .chunks_exact(4)
        .position(|pixel| pixel == color)
        .map(|index| index / width as usize)
}
fn assert_native_order(document: &Document, dpi: f32, expected: [usize; 3]) {
    let layout = wordcraft_layout::layout(document, &mut Default::default(), &Default::default());
    assert_eq!(layout.pages.len(), 1);
    let mut options = wordcraft_render::RenderOptions::default();
    options.display.dim_header = false;
    let image = wordcraft_render::render_page(document, &layout.pages[0], dpi / 72.0, &options);
    assert_color_order(&image.pixels, image.width, expected);
}
fn assert_color_order(pixels: &[u8], width: u32, expected: [usize; 3]) {
    let colors = [[0, 0, 255, 255], [255, 0, 0, 255], [0, 255, 0, 255]];
    let tops: Vec<_> = colors
        .into_iter()
        .map(|color| color_top(pixels, width, color).expect("table fill is visible"))
        .collect();
    assert!(
        tops[expected[0]] < tops[expected[1]] && tops[expected[1]] < tops[expected[2]],
        "fill tops {tops:?}, expected {expected:?}"
    );
}
fn assert_png_order(png: &[u8], expected: [usize; 3]) {
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(frame.color_type, png::ColorType::Rgba);
    assert_color_order(&pixels[..frame.buffer_size()], frame.width, expected);
}
fn pdf_texts(pdf: &[u8]) -> Vec<String> {
    crate::pdf::extract(pdf, None)
        .unwrap()
        .pages
        .iter()
        .map(|page| {
            page.blocks
                .iter()
                .map(|block| {
                    block.table.as_ref().map_or_else(
                        || block.text.clone(),
                        |table| {
                            table
                                .rows
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
#[test]
fn upstream_table_move_png_pdf_order_count_fields_and_page_context_diagnostic() {
    pyo3::prepare_freethreaded_python();
    for scope in ["body", "header/default", "footer/default"] {
        for shape in ["direct", "nested", "horizontal", "vertical"] {
            let mut owner = EditableDocx::new();
            owner.set_margins(180.0, 36.0, 180.0, 36.0).unwrap();
            let jump_target = owner.add_paragraph("TargetKeep").unwrap();
            owner
                .set_bookmark("Keep", jump_target, 0, 10, owner.revision)
                .unwrap();
            let jump = owner.add_paragraph("JumpKeep").unwrap();
            owner
                .set_range_link(jump, 0, 8, owner.revision, Some("#Keep"))
                .unwrap();
            owner
                .format_range_patch(jump, 0, 8, owner.revision, r##"{"color":"#000000"}"##)
                .unwrap();
            let parent = outer(&mut owner, scope, shape);
            let mut tables = Vec::new();
            for (label, color) in [
                ("BlueMove", "#0000FF"),
                ("RedMove", "#FF0000"),
                ("GreenMove", "#00FF00"),
            ] {
                let table = table_in(&mut owner, scope, parent, label);
                owner.set_cell_background(table, 0, 0, Some(color)).unwrap();
                owner
                    .cell_add_image(
                        table,
                        0,
                        0,
                        &png_color([0, 0, 128, 255]),
                        Some(8.0),
                        Some(8.0),
                        "ImageKeep",
                    )
                    .unwrap();
                let p = owner.cell_paragraph_ids(table, 0, 0).unwrap()[0];
                owner
                    .set_range_link(
                        p,
                        0,
                        label.len(),
                        owner.revision,
                        Some("https://example.com"),
                    )
                    .unwrap();
                owner
                    .format_range_patch(
                        p,
                        0,
                        label.len(),
                        owner.revision,
                        r##"{"color":"#000000"}"##,
                    )
                    .unwrap();
                tables.push(table);
                if label != "GreenMove" {
                    paragraph_in(&mut owner, scope, parent, "BetweenKeep");
                }
            }
            let later = owner.add_paragraph("ReviewAnchorKeep").unwrap();
            root_comment(&mut owner, later);
            let original = owner.document().unwrap();
            for (from, to, expected) in [(0, 2, [1, 2, 0]), (2, 0, [2, 0, 1])] {
                let (story, source) = owner.location(tables[from]).unwrap();
                let target = owner.location(tables[to]).unwrap().1;
                let native = move_native(original, story, &source, &target);
                assert_eq!(semantic_anchors(&native), semantic_anchors(original));
                assert_eq!(native.comments, original.comments);
                assert_eq!(native.media, original.media);
                assert_eq!(
                    owned_body_sections(&native).unwrap(),
                    owned_body_sections(original).unwrap()
                );
                let bytes = wordcraft_docx::write(&native).unwrap();
                let reopened = wordcraft_docx::read(&bytes).unwrap();
                assert_eq!(reopened.media.len(), 1);
                let text = story_text(&reopened);
                for label in [
                    "BlueMove",
                    "RedMove",
                    "GreenMove",
                    "TargetKeep",
                    "ReviewKeep",
                ] {
                    assert_eq!(text.matches(label).count(), 1, "{scope}/{shape}");
                }
                let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
                let extracted = crate::pdf::extract(&pdf, None).unwrap();
                assert_eq!(extracted.pages.len(), 1, "{scope}/{shape}");
                let text = pdf_texts(&pdf).join(" ");
                for label in ["BlueMove", "RedMove", "GreenMove", "TargetKeep"] {
                    assert_eq!(text.matches(label).count(), 1, "{scope}/{shape}: {text}");
                }
                assert!(!text.contains("ReviewKeep"));
                assert!(extracted.pages[0]
                    .links
                    .iter()
                    .any(|link| link.page == Some(0)));
                for dpi in [72.0, 144.0] {
                    assert_native_order(original, dpi, [0, 1, 2]);
                    assert_native_order(&native, dpi, expected);
                    assert_native_order(&reopened, dpi, expected);
                    assert_png_order(
                        &crate::pdf::render(&pdf, 0, dpi.into(), None).unwrap(),
                        expected,
                    );
                }
            }
        }
    }
    // Fresh native controls distinguish the pinned table PAGE defect from movement
    // or owner/layout cache reuse. The owned API now rejects these PAGE requests.
    for nested in [false, true] {
        for sync_paragraph in [false, true] {
            for kind in ["page", "page_count", "section_page_count"] {
                let mut fresh = EditableDocx::new();
                fresh.add_paragraph("FirstPage").unwrap();
                fresh.add_page_break().unwrap();
                if sync_paragraph {
                    fresh.add_paragraph("SecondPagePrelude").unwrap();
                }
                let parent = nested.then(|| fresh.add_table(vec![vec!["Parent".into()]]).unwrap());
                let table = table_in(&mut fresh, "body", parent, "FreshValue ");
                let paragraph = fresh.cell_paragraph_ids(table, 0, 0).unwrap()[0];
                let native = field_document(&mut fresh, paragraph, kind);
                let expected = if kind == "page" && !sync_paragraph {
                    1
                } else {
                    2
                };
                assert_field_output(&native, 1, "FreshValue", expected, kind, false);
                eprintln!(
                    "fresh nested={nested} sync={sync_paragraph} kind={kind}: displayed={expected}"
                );
            }
        }
    }
    let mut direct = EditableDocx::new();
    direct.add_paragraph("FirstPage").unwrap();
    direct.add_page_break().unwrap();
    let paragraph = direct.add_paragraph("DirectValue ").unwrap();
    direct.add_paragraph_field(paragraph, "page").unwrap();
    let texts = pdf_texts(&crate::docx::to_pdf(&direct.to_bytes().unwrap(), None).unwrap());
    assert!(texts[1].contains("DirectValue 2"));

    // Supported total/section counts remain correct before and after same-section
    // movement across an explicit page break. PAGE intentionally records the native
    // stale value 1 on physical page 2 (U07/spec107), not successful PAGE evaluation.
    // The original desired-PAGE=2 failure log remains frozen outside this fixture.
    for kind in ["page", "page_count", "section_page_count"] {
        let mut owner = EditableDocx::new();
        let first = owner.add_table(vec![vec!["MovedValue ".into()]]).unwrap();
        let paragraph = owner.cell_paragraph_ids(first, 0, 0).unwrap()[0];
        let mut original = field_document(&mut owner, paragraph, kind);
        owner.add_page_break().unwrap();
        let last = owner.add_table(vec![vec!["LaterPage".into()]]).unwrap();
        // Build the final native fixture from the complete owner, retaining only the
        // diagnostic instruction replacement for PAGE. No owner mutation bypass.
        if kind == "page" {
            let native_field = owner
                .paragraph_mut(&mut original, paragraph)
                .unwrap()
                .objects[0]
                .clone();
            original = owner.document().unwrap().clone();
            owner
                .paragraph_mut(&mut original, paragraph)
                .unwrap()
                .objects[0] = native_field;
        } else {
            original = owner.document().unwrap().clone();
        }
        let (story, source) = owner.location(first).unwrap();
        let target = owner.location(last).unwrap().1;
        let moved = move_native(&original, story, &source, &target);
        let expected = if kind == "page" { 1 } else { 2 };
        for (document, page) in [(&original, 0), (&moved, 1)] {
            assert_field_output(document, page, "MovedValue", expected, kind, true);
        }
        eprintln!(
            "moved field kind={kind}: before={expected}, after={expected}, physical pages=1,2"
        );
    }
}

fn table_page_error(error: PyErr) {
    assert!(
        error.to_string().contains("page fields in table cells"),
        "{error}"
    );
}

fn field_document(owner: &mut EditableDocx, paragraph: usize, kind: &str) -> Document {
    if kind == "page" {
        let revision = owner.revision;
        table_page_error(owner.add_paragraph_field(paragraph, "page").unwrap_err());
        assert_eq!(owner.revision, revision);
    }
    owner
        .add_paragraph_field(paragraph, if kind == "page" { "page_count" } else { kind })
        .unwrap();
    let mut native = owner.document().unwrap().clone();
    if kind == "page" {
        let InlineObject::Field { instr, .. } =
            &mut owner.paragraph_mut(&mut native, paragraph).unwrap().objects[0]
        else {
            panic!("missing native field");
        };
        *instr = "PAGE \\* Arabic".into();
    }
    native
}

fn assert_field_output(
    document: &Document,
    page: usize,
    label: &str,
    expected: usize,
    kind: &str,
    render: bool,
) {
    for roundtrip in [false, true] {
        let bytes = wordcraft_docx::write(document).unwrap();
        let current = if roundtrip {
            wordcraft_docx::read(&bytes).unwrap()
        } else {
            document.clone()
        };
        let bytes = wordcraft_docx::write(&current).unwrap();
        let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
        let texts = pdf_texts(&pdf);
        assert_eq!(texts.len(), 2);
        assert!(
            texts[page]
                .split_whitespace()
                .collect::<String>()
                .contains(&format!("{label}{expected}")),
            "{kind}/{roundtrip}: {texts:?}"
        );
        assert!(!texts[1 - page].contains(label));
        let layout =
            wordcraft_layout::layout(&current, &mut Default::default(), &Default::default());
        assert_eq!(layout.pages[page].number, u32::try_from(page + 1).unwrap());
        let shown: Vec<_> = layout.pages[page]
            .items
            .iter()
            .filter_map(|item| {
                if let wordcraft_layout::Placed::Lines { para, .. } = item {
                    Some(para.shown.iter().map(|(_, text, _)| text.as_str()))
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        assert!(
            shown.contains(&expected.to_string().as_str()),
            "{kind}/{roundtrip}: {shown:?}"
        );
        if kind == "page" {
            table_page_error(page_field_count(&current).unwrap_err());
            if page == 1 && expected == 1 {
                assert!(
                    !shown.contains(&"2"),
                    "retain incorrect native PAGE as diagnosis"
                );
            }
        } else {
            assert_eq!(page_field_count(&current).unwrap(), 1);
        }
        if render {
            for dpi in [72.0, 144.0] {
                // Byte lengths prove completed renders; numeric values are proved
                // separately by PDF text and native shown clusters above.
                assert!(crate::docx::render(&bytes, page, dpi, None).unwrap().len() > 1000);
                assert!(crate::pdf::render(&pdf, page, dpi, None).unwrap().len() > 1000);
            }
        }
    }
}
