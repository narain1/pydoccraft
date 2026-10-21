//! Raw pinned-native table removal proof; no table deletion API is implemented.
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
fn shifted(path: &Path, removed: &Path) -> Path {
    let mut result = path.clone();
    let depth = removed.0.len() - 1;
    if result.0.len() > depth
        && result.0[..depth] == removed.0[..depth]
        && result.0[depth] > removed.0[depth]
    {
        result.0[depth] -= 1;
    }
    result
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

#[test]
fn upstream_table_remove_preserves_containers_identity_blanks_and_comment_diagnostics() {
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
                let mut targets = Vec::new();
                for text in ["FirstDrop", "MiddleDrop", "LastDrop"] {
                    let table = table_in(&mut owner, scope, parent, text);
                    owner
                        .cell_add_table(table, 0, 0, vec![vec!["DeepDrop".into()]])
                        .unwrap_or_else(|error| panic!("{scope}/{shape}/{text}: {error}"));
                    owner
                        .cell_add_image(
                            table,
                            0,
                            0,
                            &png_color([0, 0, 255, 255]),
                            Some(8.0),
                            Some(8.0),
                            "DropImage",
                        )
                        .unwrap_or_else(|error| panic!("{scope}/{shape}/{text} image: {error}"));
                    targets.push(table);
                }
                if authored_empty {
                    paragraph_in(&mut owner, scope, parent, "");
                }
                let original = owner.document().unwrap();
                let original_sections = owned_body_sections(original).unwrap();
                for target in &targets {
                    let (story, path) = owner.location(*target).unwrap();
                    let before = original.container(story, &path).unwrap().clone();
                    let removed_arc = before[path.last()].clone();
                    let expected_usage = content_usage(&vec![removed_arc.clone()]);
                    let parent_grid = parent.map(|id| grid(owner.table(id).unwrap()));
                    let original_usage = content_usage(original.story(story).unwrap());
                    let mut native = original.clone();
                    let removed = native.remove_block(story, &path).unwrap();
                    assert_eq!(removed, *removed_arc, "{scope}/{shape}");
                    assert_eq!(content_usage(&vec![Arc::new(removed)]), expected_usage);
                    let after = native.container(story, &path).unwrap();
                    assert_eq!(after.len() + 1, before.len());
                    for (old, current) in before
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| *index != path.last())
                        .map(|(_, block)| block)
                        .zip(after)
                    {
                        assert!(
                            Arc::ptr_eq(old, current),
                            "surviving sibling Arc {scope}/{shape}"
                        );
                    }
                    assert!(
                        Arc::ptr_eq(before.last().unwrap(), after.last().unwrap()),
                        "retain both authored and hidden tail identities"
                    );
                    assert!(after.last().unwrap().as_para().unwrap().is_empty());
                    if let Some(parent) = parent {
                        let parent_path = owner.location(parent).unwrap().1;
                        assert_eq!(
                            grid(native.table(story, &parent_path).unwrap()),
                            parent_grid.unwrap()
                        );
                    }
                    for sibling in &targets {
                        if sibling == target {
                            continue;
                        }
                        let old_path = owner.location(*sibling).unwrap().1;
                        let current = shifted(&old_path, &path);
                        assert_eq!(
                            native.table(story, &current),
                            original.table(story, &old_path)
                        );
                        let before_child =
                            &original.table(story, &old_path).unwrap().rows[0].cells[0].blocks[1];
                        let after_child =
                            &native.table(story, &current).unwrap().rows[0].cells[0].blocks[1];
                        assert!(Arc::ptr_eq(before_child, after_child));
                    }
                    let usage = content_usage(native.story(story).unwrap());
                    assert_eq!(
                        (usage.0 + expected_usage.0, usage.1 + expected_usage.1),
                        original_usage
                    );
                    assert_eq!(owned_body_sections(&native).unwrap(), original_sections);
                    assert_eq!(
                        native.parts.keys().collect::<Vec<_>>(),
                        original.parts.keys().collect::<Vec<_>>()
                    );
                    assert_eq!(
                        native.media, original.media,
                        "raw removal leaves media pruning to serialization/adapter"
                    );
                    assert_eq!(
                        original.container(story, &path).unwrap().len(),
                        before.len(),
                        "copy untouched"
                    );
                }
            }
        }
    }
    // Native fallback controls are diagnostics, not a promise to adopt malformed owned containers.
    let mut owner = EditableDocx::new();
    let table = owner.add_table(vec![vec!["OnlyDrop".into()]]).unwrap();
    let block = owner.document().unwrap().body[owner.location(table).unwrap().1.last()].clone();
    for count in [1, 2] {
        let mut native = owner.document().unwrap().clone();
        native.body = vec![block.clone(); count];
        native
            .remove_block(StoryRef::Body, &Path::top(count - 1))
            .unwrap();
        assert_eq!(native.body.len(), count);
        assert_eq!(**native.body.last().unwrap(), Block::Para(Paragraph::new()));
        if count == 2 {
            assert!(Arc::ptr_eq(&native.body[0], &block));
        }
    }
    // Retained roots/replies are discovered afresh after direct and nested parent-index shifts.
    for nested in [false, true] {
        let mut owner = EditableDocx::new();
        let parent = nested.then(|| owner.add_table(vec![vec!["OuterKeep".into()]]).unwrap());
        let table = table_in(&mut owner, "body", parent, "Drop");
        let paragraph = paragraph_in(&mut owner, "body", parent, "Retained 😀 comment");
        let comment = root_comment(&mut owner, paragraph);
        owner
            .add_comment(None, 0, 0, 0, Some(comment), "ReplyKeep", "Ben", "")
            .unwrap();
        let old_path = owner.location(paragraph).unwrap().1;
        let (story, path) = owner.location(table).unwrap();
        let original = owner.document().unwrap();
        let mut native = original.clone();
        native.remove_block(story, &path).unwrap();
        assert_eq!(native.comments, original.comments);
        assert_eq!(native.parts, original.parts);
        let inventory = comments::inventory(&native).unwrap();
        let anchor = inventory.entries[&comment].anchor.as_ref().unwrap();
        assert_eq!(anchor.path, shifted(&old_path, &path));
        let p = native.para(StoryRef::Body, &anchor.path).unwrap();
        assert_eq!(&p.text[3..anchor.end], "Retained 😀 comment");
        // Broader raw native metadata and interior anchors remain intact too; they are not adopted.
        for off in [anchor.end, 0] {
            native
                .delete_range(
                    &Pos::new(StoryRef::Body, anchor.path.clone(), off),
                    &Pos::new(StoryRef::Body, anchor.path.clone(), off + 3),
                )
                .unwrap();
        }
        for (off, marker) in [
            (8, InlineObject::CommentEnd { id: comment }),
            (1, InlineObject::CommentStart { id: comment }),
        ] {
            native
                .insert_object(
                    &Pos::new(StoryRef::Body, anchor.path.clone(), off),
                    marker,
                    &CharProps::default(),
                )
                .unwrap();
        }
        let raw = native.comments.get_mut(&comment).unwrap();
        raw.author = "Foreign author".into();
        raw.initials = "FA".into();
        raw.durable_id = Some("1234ABCD".into());
        let clone = native.clone();
        let before = clone.comments.clone();
        native
            .insert_block(
                story,
                &path,
                (*owner.document().unwrap().container(story, &path).unwrap()[path.last()]).clone(),
            )
            .unwrap();
        native.remove_block(story, &path).unwrap();
        assert_eq!(native.comments, before);
        assert_eq!(native.parts, clone.parts);
        let reopened = wordcraft_docx::read(&wordcraft_docx::write(&native).unwrap()).unwrap();
        let foreign = reopened
            .comments
            .values()
            .find(|value| value.durable_id.as_deref() == Some("1234ABCD"))
            .unwrap();
        assert_eq!(foreign.author, "Foreign author");
        assert_eq!(foreign.initials, "FA");
        assert_eq!(
            native.plain_text(StoryRef::Body),
            clone.plain_text(StoryRef::Body)
        );
    }
    // Raw removal is not semantic authorization: the comment map/part remains orphaned.
    let mut owner = EditableDocx::new();
    let table = owner.add_table(vec![vec!["Protected".into()]]).unwrap();
    let paragraph = owner.cell_paragraph_ids(table, 0, 0).unwrap()[0];
    let comment = root_comment(&mut owner, paragraph);
    let part = owner.document().unwrap().comments[&comment].part;
    let (story, path) = owner.location(table).unwrap();
    let mut native = owner.document().unwrap().clone();
    native.remove_block(story, &path).unwrap();
    assert!(native.comments.contains_key(&comment) && native.parts.contains_key(&part));
    assert!(comments::inventory(&native).is_err());
    assert!(owner.commit(native).is_err());
    // Unexpected cell section metadata is discarded by raw removal; the adapter must refuse it.
    let mut unexpected = owner.document().unwrap().clone();
    let section = unexpected.last_section.clone();
    let child_path = owner.location(paragraph).unwrap().1;
    unexpected.para_mut(story, &child_path).unwrap().section = Some(Box::new(section));
    let removed = unexpected.remove_block(story, &path).unwrap();
    assert!(removed.as_table().unwrap().rows[0].cells[0].blocks[0]
        .as_para()
        .unwrap()
        .section
        .is_some());

    assert_eq!(
        owner.paragraph_text(paragraph).unwrap(),
        "\u{fffc}Protected\u{fffc}"
    );
}

fn color_count(png: &[u8], color: [u8; 4]) -> usize {
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(frame.color_type, png::ColorType::Rgba);
    pixels[..frame.buffer_size()]
        .chunks_exact(4)
        .filter(|pixel| *pixel == color)
        .count()
}
fn native_colors(document: &Document, dpi: f32) -> (usize, usize) {
    let layout = wordcraft_layout::layout(document, &mut Default::default(), &Default::default());
    let mut blue = 0;
    let mut red = 0;
    for page in &layout.pages {
        let mut options = wordcraft_render::RenderOptions::default();
        options.display.dim_header = false;
        let image = wordcraft_render::render_page(document, page, dpi / 72.0, &options);
        blue += image
            .pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel == [0, 0, 255, 255])
            .count();
        red += image
            .pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel == [255, 0, 0, 255])
            .count();
    }
    (blue, red)
}
#[test]
fn upstream_table_remove_docx_png_pdf_keep_media_sections_links_and_review() {
    pyo3::prepare_freethreaded_python();
    for scope in ["body", "header/default", "footer/default"] {
        for shape in ["direct", "nested", "horizontal", "vertical"] {
            let mut owner = EditableDocx::new();
            owner.set_margins(180.0, 36.0, 180.0, 36.0).unwrap();
            let target = owner.add_paragraph("TargetKeep").unwrap();
            owner
                .set_bookmark("Keep", target, 0, 10, owner.revision)
                .unwrap();
            let jump = owner.add_paragraph("JumpKeep").unwrap();
            owner
                .set_range_link(jump, 0, 8, owner.revision, Some("#Keep"))
                .unwrap();
            owner
                .format_range_patch(jump, 0, 8, owner.revision, r##"{"color":"#000000"}"##)
                .unwrap();
            let parent = outer(&mut owner, scope, shape);
            let table = table_in(&mut owner, scope, parent, "TableDrop");
            owner
                .set_cell_background(table, 0, 0, Some("#0000FF"))
                .unwrap();
            let p = owner.cell_paragraph_ids(table, 0, 0).unwrap()[0];
            owner
                .set_range_link(p, 0, 9, owner.revision, Some("https://example.com"))
                .unwrap();
            owner.add_paragraph_field(p, "page_count").unwrap();
            owner
                .cell_add_table(table, 0, 0, vec![vec!["DeepDrop".into()]])
                .unwrap();
            owner
                .cell_add_image(
                    table,
                    0,
                    0,
                    &png_color([0, 0, 255, 255]),
                    Some(10.0),
                    Some(10.0),
                    "UniqueDrop",
                )
                .unwrap();
            owner
                .cell_add_image(
                    table,
                    0,
                    0,
                    &png_color([255, 0, 0, 255]),
                    Some(10.0),
                    Some(10.0),
                    "SharedDrop",
                )
                .unwrap();
            let kept = table_in(&mut owner, scope, parent, "TableKeep");
            owner
                .cell_add_image(
                    kept,
                    0,
                    0,
                    &png_color([255, 0, 0, 255]),
                    Some(12.0),
                    Some(12.0),
                    "SharedKeep",
                )
                .unwrap();
            if scope == "body" {
                owner.add_section("next_page").unwrap();
            }
            let later = owner.add_paragraph("LaterKeep").unwrap();
            owner
                .add_note(later, 9, 9, owner.revision, "NoteKeep", "footnote")
                .unwrap();
            let comment_p = owner.add_paragraph("ReviewAnchorKeep").unwrap();
            root_comment(&mut owner, comment_p);
            let (story, path) = owner.location(table).unwrap();
            let original = owner.document().unwrap();
            let sections = owned_body_sections(original).unwrap();
            let semantics = semantic_anchors(original);
            let mut native = original.clone();
            native.remove_block(story, &path).unwrap();
            assert_eq!(owned_body_sections(&native).unwrap(), sections);
            assert_eq!(semantic_anchors(&native), semantics);
            assert_eq!(native.comments, original.comments);
            assert_eq!(
                native
                    .parts
                    .iter()
                    .filter(|(_, part)| matches!(part.kind, PartKind::Comment | PartKind::Footnote))
                    .collect::<Vec<_>>(),
                original
                    .parts
                    .iter()
                    .filter(|(_, part)| matches!(part.kind, PartKind::Comment | PartKind::Footnote))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                native.media, original.media,
                "raw native removal does not reclaim owner memory"
            );
            let bytes = wordcraft_docx::write(&native).unwrap();
            let reopened = wordcraft_docx::read(&bytes).unwrap();
            assert_eq!(
                reopened.media.len(),
                1,
                "writer drops only unique unreferenced blue asset"
            );
            let text = std::iter::once(reopened.plain_text(StoryRef::Body))
                .chain(
                    reopened
                        .parts
                        .keys()
                        .map(|part| reopened.plain_text(StoryRef::Part(*part))),
                )
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                !text.contains("TableDrop") && !text.contains("DeepDrop"),
                "{scope}/{shape}: {text}"
            );
            for keep in [
                "TargetKeep",
                "JumpKeep",
                "TableKeep",
                "LaterKeep",
                "NoteKeep",
                "ReviewKeep",
            ] {
                assert!(text.contains(keep), "{scope}/{shape}: {keep}");
            }
            let pdf = crate::docx::to_pdf(&bytes, None).unwrap();
            let extracted = crate::pdf::extract(&pdf, None).unwrap();
            let pdf_text = extracted
                .pages
                .iter()
                .flat_map(|page| &page.blocks)
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                !pdf_text.contains("TableDrop")
                    && !pdf_text.contains("DeepDrop")
                    && !pdf_text.contains("ReviewKeep")
            );
            for keep in [
                "TargetKeep",
                "JumpKeep",
                "TableKeep",
                "LaterKeep",
                "NoteKeep",
            ] {
                assert!(pdf_text.contains(keep), "{scope}/{shape}: {pdf_text}");
            }
            assert!(extracted
                .pages
                .iter()
                .flat_map(|page| &page.links)
                .any(|link| link.page == Some(0)));
            for dpi in [72.0, 144.0] {
                assert!(
                    native_colors(original, dpi).0 > 100,
                    "removed blue visible before deletion {scope}/{shape}"
                );
                for document in [&native, &reopened] {
                    let (blue, red) = native_colors(document, dpi);
                    assert_eq!(blue, 0, "{scope}/{shape}: native blue removed");
                    assert!(red > 100, "{scope}/{shape}: native red kept");
                }
                let mut blue = 0;
                let mut red = 0;
                for index in 0..extracted.pages.len() {
                    let png = crate::pdf::render(&pdf, index, dpi.into(), None).unwrap();
                    blue += color_count(&png, [0, 0, 255, 255]);
                    red += color_count(&png, [255, 0, 0, 255]);
                }
                assert_eq!(blue, 0, "{scope}/{shape}: PDF blue removed");
                assert!(red > 100, "{scope}/{shape}: PDF red kept");
            }
        }
    }
}
