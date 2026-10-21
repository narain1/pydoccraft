//! Pinned-native model/writer proof and owned cell-deletion regression tests.
use super::*;
use deckcraft_model::text::Paragraph as NativeParagraph;

fn fixture(merged: u8) -> (EditablePptx, u32) {
    pyo3::prepare_freethreaded_python();
    let mut owner = EditablePptx::new(576.0, 360.0).unwrap();
    owner.add_slide("blank").unwrap();
    owner.add_slide("blank").unwrap();
    let id = owner
        .add_table(
            0,
            vec![
                vec!["First".into(), "Other".into()],
                vec!["Lower".into(), "Neighbor".into()],
            ],
            24.0,
            24.0,
            520.0,
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
    let cell = cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0).unwrap();
    cell.text.paragraphs = ["First office", "Middle e\u{301} 😀", "Last سلام"]
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let mut para = NativeParagraph::new(text);
            para.props.align = Some([Align::Left, Align::Center, Align::Right][i]);
            para.level = i as u8;
            para.def_props.size = Some(20.0 + i as f64);
            para.end_props.bold = Some(i == 1);
            para.runs[0].props.bold = Some(i != 1);
            para
        })
        .collect();
    // Normalize through the native writer once so the subsequent comparison is
    // about deletion, not initial native default materialization.
    owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
    owner.refresh_budgets().unwrap();
    (owner, id)
}

fn pixels(data: Vec<u8>) -> (u32, u32, Vec<u8>) {
    let mut r = png::Decoder::new(Cursor::new(data)).read_info().unwrap();
    let mut bytes = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut bytes).unwrap();
    bytes.truncate(info.buffer_size());
    (info.width, info.height, bytes)
}

#[test]
fn pptx_cell_delete_raw_preserves_successor_properties_and_native_outputs() {
    for merged in 0..4 {
        for index in 0..3 {
            let (mut owner, id) = fixture(merged);
            let before = owner.document().unwrap().clone();
            let source = table_cell(owner.table(0, id).unwrap(), 0, 0)
                .unwrap()
                .text
                .clone();
            let mut expected = source.clone();
            expected.paragraphs.remove(index);
            cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                .unwrap()
                .text
                .paragraphs
                .remove(index);
            assert!(owner.document().unwrap().validate().is_empty());
            let bytes = owner.to_bytes().unwrap();
            let reopened = deckcraft_pptx::import(&bytes).unwrap();
            let ShapeKind::Table(table) = &reopened.slides[0].shape(ShapeId(id)).unwrap().kind
            else {
                panic!("table");
            };
            assert_eq!(table.cell(0, 0).unwrap().text, expected);
            assert_eq!(reopened.slides[1], before.slides[1]);
            let ShapeKind::Table(old_table) = &before.slides[0].shape(ShapeId(id)).unwrap().kind
            else {
                panic!("table");
            };
            assert_eq!(table.cell(0, 1), old_table.cell(0, 1));
            let raw_image =
                deckcraft_render::render_slide(owner.document().unwrap(), 0, &Default::default());
            let read_image = deckcraft_render::render_slide(&reopened, 0, &Default::default());
            assert_eq!(raw_image.pixels, read_image.pixels);
            let pdf = crate::pptx::to_pdf(&bytes).unwrap();
            for dpi in [72.0, 144.0, 200.0] {
                let source_image = pixels(crate::pptx::render(&bytes, 0, dpi).unwrap());
                let exported = pixels(crate::pdf::render(&pdf, 0, dpi, None).unwrap());
                assert_eq!((source_image.0, source_image.1), (exported.0, exported.1));
                // Native PDF rasterizes slides at 200 DPI. At other output
                // DPIs resampling differs from a fresh native raster.
                assert_eq!(source_image.2.len(), exported.2.len());
                if dpi == 200.0 {
                    assert!(source_image
                        .2
                        .iter()
                        .zip(&exported.2)
                        .all(|(a, b)| a.abs_diff(*b) <= 1));
                }
                assert!(exported.2.chunks_exact(4).filter(|p| p[0] < 128).count() > 20);
                assert!(
                    source_image
                        .2
                        .chunks_exact(4)
                        .filter(|p| p[0] < 128)
                        .count()
                        > 20
                );
            }
        }
    }
}

#[test]
fn pptx_cell_delete_raw_range_semantics_and_final_empty_writer_behavior() {
    let (mut owner, id) = fixture(0);
    let source = table_cell(owner.table(0, id).unwrap(), 0, 0)
        .unwrap()
        .text
        .clone();
    let baseline_bytes = owner.to_bytes().unwrap();
    let baseline_pdf = crate::pptx::to_pdf(&baseline_bytes).unwrap();
    for dpi in [72.0, 144.0] {
        let a = pixels(crate::pptx::render(&baseline_bytes, 0, dpi).unwrap());
        let b = pixels(crate::pdf::render(&baseline_pdf, 0, dpi, None).unwrap());
        assert_eq!((a.0, a.1), (b.0, b.1));
        // Unedited fixture isolates the cross-DPI difference from deletion.
        let maximum =
            a.2.iter()
                .zip(&b.2)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
        eprintln!("unedited PDF raster resampling at {dpi} DPI: max channel difference {maximum}");
        assert!(maximum > 1);
    }
    let mut range_deleted = source.clone();
    deckcraft_model::edit::delete(&mut range_deleted, (0, 0), (1, 0));
    assert_eq!(
        range_deleted.paragraphs[0].text(),
        source.paragraphs[1].text()
    );
    assert_eq!(
        range_deleted.paragraphs[0].props,
        source.paragraphs[0].props
    );
    assert_ne!(
        range_deleted.paragraphs[0].props,
        source.paragraphs[1].props
    );
    // Deleting an authored empty paragraph is still a real collection edit.
    let cell = cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0).unwrap();
    cell.text.paragraphs = vec![NativeParagraph::default()];
    let mut expected_cell = cell.clone();
    cell.text.paragraphs.remove(0);
    assert!(owner.document().unwrap().validate().is_empty());
    let bytes = owner.to_bytes().unwrap();
    let reopened = deckcraft_pptx::import(&bytes).unwrap();
    let ShapeKind::Table(table) = &reopened.slides[0].shape(ShapeId(id)).unwrap().kind else {
        panic!("table");
    };
    assert_eq!(
        table.cell(0, 0).unwrap().text.paragraphs,
        vec![NativeParagraph::default()]
    );
    // Compare empty native body to the writer's mandatory empty paragraph.
    let raw = deckcraft_render::render_slide(owner.document().unwrap(), 0, &Default::default());
    let saved = deckcraft_render::render_slide(&reopened, 0, &Default::default());
    assert_eq!(raw.pixels, saved.pixels);
    owner.refresh_budgets().unwrap();
    let mut copied = owner.copy().unwrap();
    let appended = owner
        .cell_add_paragraph(0, id, 0, 0, "After empty")
        .unwrap();
    assert_eq!(appended.2, 0);
    let cell = table_cell(owner.table(0, id).unwrap(), 0, 0).unwrap();
    assert_eq!(cell.text.text(), "After empty");
    assert_eq!(cell.text.paragraphs.len(), 1);
    expected_cell.text = cell.text.clone();
    assert_eq!(cell, &expected_cell);
    assert_eq!(copied.cell_paragraph_info(0, id, 0, 0).unwrap().4, 0);
    copied.cell_add_paragraph(0, id, 0, 0, "Copy only").unwrap();
    assert_eq!(owner.cell_text(0, id, 0, 0).unwrap(), "After empty");
    assert_eq!(copied.cell_text(0, id, 0, 0).unwrap(), "Copy only");
}

fn target(owner: &EditablePptx, id: u32, paragraph: usize) -> CellTarget {
    let (r, c, grid, generation, _) = owner.cell_paragraph_info(0, id, 0, 0).unwrap();
    (r, c, paragraph, grid, generation)
}

#[test]
fn pptx_cell_delete_owned_canonical_generations_budgets_and_copy() {
    for merged in 0..4 {
        let (mut owner, id) = fixture(merged);
        let copied = owner.copy().unwrap();
        let old = target(&owner, id, 0);
        let selected = target(&owner, id, 1);
        let original = table_cell(owner.table(0, id).unwrap(), 0, 0)
            .unwrap()
            .text
            .clone();
        let mut expected = original.clone();
        expected.paragraphs.remove(1);
        let budget = content_budget(owner.document().unwrap()).unwrap();
        let before_bytes = owner.text_bytes;
        owner.remove_cell_paragraph(0, id, selected).unwrap();
        let after = content_budget(owner.document().unwrap()).unwrap();
        assert_eq!(after.paragraphs, budget.paragraphs - 1);
        assert_eq!(
            owner.text_bytes,
            before_bytes - (original.text().len() - expected.text().len())
        );
        assert_eq!(
            table_cell(owner.table(0, id).unwrap(), 0, 0).unwrap().text,
            expected
        );
        assert!(owner.shape_text(0, id, Some(old)).is_err());
        assert!(owner.remove_cell_paragraph(0, id, selected).is_err());
        assert_eq!(copied.shape_text(0, id, Some(old)).unwrap(), "First office");
        let info = owner.cell_paragraph_info(0, id, 0, 0).unwrap();
        assert_eq!(info.2, old.3);
        assert_eq!(info.3, old.4 + 1);
        let alias = match merged {
            0 => (0, 0),
            1 => (0, 1),
            2 => (1, 0),
            _ => (1, 1),
        };
        assert_eq!(
            owner.cell_paragraph_info(0, id, alias.0, alias.1).unwrap(),
            info
        );
        while owner.cell_paragraph_info(0, id, 0, 0).unwrap().4 > 0 {
            let first = target(&owner, id, 0);
            owner.remove_cell_paragraph(0, id, first).unwrap();
        }
        assert_eq!(owner.cell_text(0, id, 0, 0).unwrap(), "");
        let appended = owner
            .cell_add_paragraph(0, id, alias.0, alias.1, "Fresh")
            .unwrap();
        assert_eq!(appended.2, 0);
        assert_eq!(owner.shape_text(0, id, Some(appended)).unwrap(), "Fresh");
        assert!(owner.shape_text(0, id, Some(old)).is_err());
    }
}

#[test]
fn pptx_cell_delete_owned_checked_limits_and_invalid_targets_are_atomic() {
    let (mut owner, id) = fixture(3);
    let initial = target(&owner, id, 0);
    let before = owner.document().unwrap().clone();
    let revision = owner.revision;
    let bytes = owner.text_bytes;
    for bad in [
        (0, 1, 0, initial.3, 0),
        (0, 0, 99, initial.3, 0),
        (0, 0, 0, initial.3 + 1, 0),
        (0, 0, 0, initial.3, 1),
    ] {
        assert!(owner.remove_cell_paragraph(0, id, bad).is_err());
        assert_eq!(owner.document().unwrap(), &before);
        assert_eq!(owner.revision, revision);
        assert_eq!(owner.text_bytes, bytes);
    }
    owner.cell_generations.insert((id, 0, 0), u64::MAX - 1);
    let last_valid = target(&owner, id, 0);
    owner.remove_cell_paragraph(0, id, last_valid).unwrap();
    assert_eq!(owner.cell_generations[&(id, 0, 0)], u64::MAX);
    let before = owner.document().unwrap().clone();
    let revision = owner.revision;
    let bytes = owner.text_bytes;
    assert!(owner
        .remove_cell_paragraph(0, id, target(&owner, id, 0))
        .unwrap_err()
        .to_string()
        .contains("generation limit"));
    assert_eq!(owner.document().unwrap(), &before);
    assert_eq!(owner.revision, revision);
    assert_eq!(owner.text_bytes, bytes);
    owner.cell_generations.insert((id, 0, 0), 1);
    owner.revision = u64::MAX - 1;
    owner
        .remove_cell_paragraph(0, id, target(&owner, id, 0))
        .unwrap();
    assert_eq!(owner.revision, u64::MAX);
    let before = owner.document().unwrap().clone();
    let generation = owner.cell_generations.clone();
    assert!(owner
        .remove_cell_paragraph(0, id, target(&owner, id, 0))
        .unwrap_err()
        .to_string()
        .contains("revision limit"));
    assert_eq!(owner.document().unwrap(), &before);
    assert_eq!(owner.cell_generations, generation);
}

#[test]
fn pptx_cell_delete_owned_unrelated_handles_and_resource_failure() {
    let (mut owner, id) = fixture(0);
    let (r, c, grid, generation, _) = owner.cell_paragraph_info(0, id, 1, 1).unwrap();
    let other = (r, c, 0, grid, generation);
    owner
        .remove_cell_paragraph(0, id, target(&owner, id, 0))
        .unwrap();
    assert_eq!(owner.shape_text(0, id, Some(other)).unwrap(), "Neighbor");
    assert_eq!(owner.cell_paragraph_info(0, id, 1, 1).unwrap().2, grid);
    // Malformed private state remains bounded and cannot partially publish a deletion.
    Arc::make_mut(&mut owner.presentation.as_mut().unwrap().slides[1]).name =
        "x".repeat(MAX_TEXT + 1);
    let before = owner.document().unwrap().clone();
    let generations = owner.cell_generations.clone();
    let revision = owner.revision;
    let bytes = owner.text_bytes;
    assert!(owner
        .remove_cell_paragraph(0, id, target(&owner, id, 0))
        .unwrap_err()
        .to_string()
        .contains("resource_limit"));
    assert_eq!(owner.document().unwrap(), &before);
    assert_eq!(owner.cell_generations, generations);
    assert_eq!(owner.revision, revision);
    assert_eq!(owner.text_bytes, bytes);
}
