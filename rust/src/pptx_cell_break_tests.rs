//! Spec130 bounded cell insertion and 1:1 public LF projection.
use super::*;
use deckcraft_model::text::{Run, RunKind};

fn fixture() -> (EditablePptx, u32, CellTarget) {
    let mut owner = EditablePptx::new(576.0, 360.0).unwrap();
    owner.add_slide("blank").unwrap();
    owner.set_slide_notes(0, "Retained notes").unwrap();
    let id = owner
        .add_table(
            0,
            vec![vec!["A😀e\u{301}Z".into(), "Neighbor".into()]],
            20.0,
            20.0,
            500.0,
            180.0,
            20.0,
        )
        .unwrap();
    owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
    owner.refresh_budgets().unwrap();
    let (r, c, grid, generation, _) = owner.cell_paragraph_info(0, id, 0, 0).unwrap();
    (owner, id, (r, c, 0, grid, generation))
}

fn unchanged(owner: &EditablePptx, before: &EditablePptx) {
    assert_eq!(owner.document().unwrap(), before.document().unwrap());
    assert_eq!(owner.revision, before.revision);
    assert_eq!(owner.table_generations, before.table_generations);
    assert_eq!(owner.cell_generations, before.cell_generations);
    assert_eq!(
        (owner.text_bytes, owner.media_bytes, owner.shapes),
        (before.text_bytes, before.media_bytes, before.shapes)
    );
}

#[test]
fn cell_break_native_markers_public_lf_selection_and_navigation_preserve_model() {
    let (mut owner, id, target) = fixture();
    let url = make_link(
        owner.document().unwrap(),
        Some("https://example.com/label"),
        None,
        "Tip",
    )
    .unwrap();
    let p = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
        .unwrap()
        .text
        .paragraphs[0];
    p.runs[0].props.link = url.clone();
    p.runs[0].props.hover = url;
    p.runs[0].props.bold = Some(true);
    p.runs[0].props.size = Some(23.0);
    let mut expected = p.runs[0].props.clone();
    expected.link = None;
    expected.hover = None;
    owner.refresh_budgets().unwrap();
    let original = owner.clone();
    for i in 0..3 {
        let (start, end, revision) = owner
            .cell_add_break(0, id, target, "line", owner.revision)
            .unwrap();
        assert_eq!((start, end), (5 + i, 6 + i));
        assert_eq!(
            owner
                .range_text(0, id, start, end, revision, Some(target))
                .unwrap(),
            "\n"
        );
        let p = &owner.target_body(0, id, Some(target)).unwrap().paragraphs[0];
        let r = p.runs.last().unwrap();
        assert_eq!(r.kind, RunKind::Break);
        assert!(r.text.is_empty());
        assert_eq!(r.content(), "\u{b}");
        assert_eq!(r.props, expected);
        assert_eq!(p.text(), format!("A😀e\u{301}Z{}", "\u{b}".repeat(i + 1)));
        assert_eq!(
            owner.shape_text(0, id, Some(target)).unwrap(),
            format!("A😀e\u{301}Z{}", "\n".repeat(i + 1))
        );
        assert_eq!(
            owner.cell_text(0, id, 0, 0).unwrap(),
            owner.table_rows(0, id).unwrap()[0][0]
        );
        assert_eq!(
            owner
                .target_body(0, id, Some(target))
                .unwrap()
                .paragraphs
                .len(),
            1
        );
    }
    assert_eq!(owner.table_generations, original.table_generations);
    assert_eq!(owner.cell_generations, original.cell_generations);
    assert_eq!(owner.cell_text(0, id, 0, 1).unwrap(), "Neighbor");
    assert_eq!(
        owner.document().unwrap().slides[0].notes,
        original.document().unwrap().slides[0].notes
    );
    let read = deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap();
    assert_eq!(
        read.slides[0].shapes[0],
        owner.document().unwrap().slides[0].shapes[0]
    );
    let before = owner.clone();
    for input in ["\n", "\u{b}"] {
        assert!(owner
            .replace_range(0, id, 5, 6, owner.revision, input, Some(target))
            .is_err());
        unchanged(&owner, &before);
        assert!(owner
            .add_run_patch(0, id, input, "{}", Some(target))
            .is_err());
        unchanged(&owner, &before);
    }
    owner
        .replace_range(0, id, 5, 6, owner.revision, " ", Some(target))
        .unwrap();
    assert_eq!(
        owner.shape_text(0, id, Some(target)).unwrap(),
        "A😀e\u{301}Z \n\n"
    );
    owner
        .replace_range(0, id, 6, 8, owner.revision, "", Some(target))
        .unwrap();
    assert_eq!(
        owner.shape_text(0, id, Some(target)).unwrap(),
        "A😀e\u{301}Z "
    );
    let shape = owner
        .add_text(0, "Shape", 20.0, 250.0, 200.0, 30.0, 18.0)
        .unwrap();
    let body = shape_mut(owner.presentation.as_mut().unwrap(), 0, shape)
        .unwrap()
        .text
        .as_mut()
        .unwrap();
    let end = body.char_len();
    deckcraft_model::edit::insert(body, (0, end), "\u{b}", None);
    assert_eq!(owner.shape_text(0, shape, None).unwrap(), "Shape\u{b}");
    assert_eq!(owner.text_range(0, shape, 5, 6, None).unwrap().0, "\u{b}");
}

#[test]
fn cell_break_actual_text_run_property_limits_and_revision_exhaustion_are_atomic() {
    for kind in ["text", "runs", "properties"] {
        let (mut owner, id, target) = fixture();
        let initial = content_budget(owner.document().unwrap()).unwrap();
        if kind == "text" {
            let p = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                .unwrap()
                .text
                .paragraphs[0];
            let old = p.runs[0].content().len();
            p.runs[0].text = "x".repeat(MAX_TEXT - initial.text + old - 1);
        } else if kind == "runs" {
            let p = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                .unwrap()
                .text
                .paragraphs[0];
            let count = MAX_RUNS - initial.runs + p.runs.len() - 1;
            p.runs = vec![
                Run {
                    text: String::new(),
                    props: RunProps::default(),
                    kind: RunKind::Break
                };
                count
            ];
        } else {
            let p = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                .unwrap()
                .text
                .paragraphs[0];
            p.runs[0].props.font = Some("X".into());
            let used = content_budget(owner.document().unwrap())
                .unwrap()
                .properties;
            Arc::make_mut(&mut owner.presentation.as_mut().unwrap().slides[0])
                .notes
                .paragraphs[0]
                .end_props
                .font = Some("x".repeat(MAX_PROPERTIES - used - 1));
        }
        owner.refresh_budgets().unwrap();
        let before = content_budget(owner.document().unwrap()).unwrap();
        let saved_generation = owner.cell_generations.clone();
        owner
            .cell_add_break(0, id, target, "line", owner.revision)
            .unwrap();
        let after = content_budget(owner.document().unwrap()).unwrap();
        match kind {
            "text" => assert_eq!(after.text, MAX_TEXT),
            "runs" => assert_eq!(after.runs, MAX_RUNS),
            _ => assert_eq!(after.properties, MAX_PROPERTIES),
        }
        assert_eq!(after.paragraphs, before.paragraphs);
        assert_eq!(after.cells, before.cells);
        assert_eq!(owner.cell_generations, saved_generation);
        let checkpoint = owner.clone();
        assert!(owner
            .cell_add_break(0, id, target, "line", owner.revision)
            .unwrap_err()
            .to_string()
            .contains("limit"));
        unchanged(&owner, &checkpoint);
    }
    let (mut owner, id, target) = fixture();
    owner.revision = u64::MAX - 1;
    owner
        .cell_add_break(0, id, target, "line", owner.revision)
        .unwrap();
    assert_eq!(owner.revision, u64::MAX);
    let before = owner.clone();
    assert!(owner
        .cell_add_break(0, id, target, "line", owner.revision)
        .is_err());
    unchanged(&owner, &before);
}

#[test]
fn cell_break_empty_alias_lifetime_and_original_scope_validation() {
    let (mut owner, id, target) = fixture();
    let before = owner.clone();
    for kind in ["page", "column", "unknown"] {
        assert!(owner
            .cell_add_break(0, id, target, kind, owner.revision)
            .is_err());
        unchanged(&owner, &before);
    }
    assert!(owner
        .cell_add_break(0, id, target, "line", owner.revision - 1)
        .is_err());
    unchanged(&owner, &before);
    let mut copy = owner.copy().unwrap();
    copy.replace_range(0, id, 0, 5, copy.revision, "", Some(target))
        .unwrap();
    for i in 0..2 {
        let (a, b, _) = copy
            .cell_add_break(0, id, target, "line", copy.revision)
            .unwrap();
        assert_eq!((a, b), (i, i + 1));
    }
    assert_eq!(copy.shape_text(0, id, Some(target)).unwrap(), "\n\n");
    unchanged(&owner, &before);
    copy.replace_range(0, id, 0, 2, copy.revision, "", Some(target))
        .unwrap();
    assert_eq!(
        copy.target_body(0, id, Some(target))
            .unwrap()
            .paragraphs
            .len(),
        1
    );
    assert!(copy.target_body(0, id, Some(target)).unwrap().paragraphs[0]
        .runs
        .is_empty());
    owner.merge_cells(0, id, 0, 0, 0, 1).unwrap();
    assert!(owner
        .cell_add_break(0, id, target, "line", owner.revision)
        .is_err());
    let (r, c, g, b, _) = owner.cell_paragraph_info(0, id, 0, 1).unwrap();
    let merged = (r, c, 0, g, b);
    owner
        .cell_add_break(0, id, merged, "line", owner.revision)
        .unwrap();
    assert!(owner.cell_text(0, id, 0, 1).unwrap().contains('\n'));
    let state = owner.clone();
    assert!(owner
        .cell_add_break(0, id, (0, 1, 0, g, b), "line", owner.revision)
        .is_err());
    unchanged(&owner, &state);
    owner.split_cell(0, id, 0, 0, None).unwrap();
    let (r, c, g, b, n) = owner.cell_paragraph_info(0, id, 0, 1).unwrap();
    assert_eq!(n, 0);
    let state = owner.clone();
    assert!(owner
        .cell_add_break(0, id, (r, c, 0, g, b), "line", owner.revision)
        .is_err());
    unchanged(&owner, &state);
    // A valid target does not allow malformed unrelated native identity through staging.
    let (mut owner, id, target) = fixture();
    let duplicate = owner.document().unwrap().slides[0].clone();
    owner.presentation.as_mut().unwrap().slides.push(duplicate);
    let state = owner.clone();
    assert!(owner
        .cell_add_break(0, id, target, "line", owner.revision)
        .is_err());
    unchanged(&owner, &state);
    owner.close();
    assert!(owner
        .cell_add_break(0, id, target, "line", owner.revision)
        .is_err());
}
