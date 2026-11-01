//! Spec124 raw-only public native line-break model and actual output gate.
use super::*;
use deckcraft_model::text::{LevelStyle, Paragraph as NativeParagraph, RunKind};

fn fixture(merged: u8, empty: bool) -> (EditablePptx, u32) {
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
    // Normalize baseline defaults once; break insertion is then the only change.
    owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
    owner.refresh_budgets().unwrap();
    (owner, id)
}

fn insert_break(body: &mut TextBody, paragraph: usize) -> (usize, usize) {
    let start = body.paragraphs[paragraph].char_len();
    let mut props = deckcraft_model::edit::props_at(body, (paragraph, start));
    // Appearance inheritance is distinct from navigation inheritance.
    props.link = None;
    props.hover = None;
    let expected_props = props.clone();
    let end = deckcraft_model::edit::insert(body, (paragraph, start), "\u{b}", Some(props));
    assert_eq!(
        body.paragraphs[paragraph].runs.last().unwrap().props,
        expected_props
    );
    assert_eq!(end, (paragraph, start + 1));
    assert_eq!(
        deckcraft_model::edit::text_range(body, (paragraph, start), end),
        "\u{b}"
    );
    (start, end.1)
}

#[test]
fn pptx_cell_break_raw_unicode_carets_model_readback_and_deletion() {
    let mut cases = 0;
    for merged in 0..4 {
        for text in ["", "A😀e\u{301}Z"] {
            for count in 1..=3 {
                let (mut owner, id) = fixture(merged, false);
                let body = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                    .unwrap()
                    .text;
                body.paragraphs.truncate(1);
                body.paragraphs[0].runs[0].text = text.into();
                if text.is_empty() {
                    body.paragraphs[0].runs.clear();
                }
                // Normalize the authored empty/nonempty baseline before insertion.
                owner.presentation =
                    Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
                let original = owner.document().unwrap().clone();
                let body = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                    .unwrap()
                    .text;
                let original_body = body.clone();
                let source_len = text.chars().count();
                assert_eq!(body.paragraphs.len(), 1);
                for n in 0..count {
                    assert_eq!(insert_break(body, 0), (source_len + n, source_len + n + 1));
                    assert_eq!(body.paragraphs.len(), 1);
                    let break_run = body.paragraphs[0].runs.last().unwrap();
                    assert_eq!(break_run.kind, RunKind::Break);
                    assert!(break_run.text.is_empty());
                    assert!(break_run.props.link.is_none() && break_run.props.hover.is_none());
                }
                assert_eq!(
                    body.paragraphs[0].text(),
                    format!("{text}{}", "\u{b}".repeat(count))
                );
                assert_eq!(body.paragraphs[0].props, original_body.paragraphs[0].props);
                assert_eq!(
                    body.paragraphs[0].def_props,
                    original_body.paragraphs[0].def_props
                );
                assert_eq!(
                    body.paragraphs[0].end_props,
                    original_body.paragraphs[0].end_props
                );
                let tail = "😀e\u{301}";
                let start = source_len + count;
                let caret = deckcraft_model::edit::insert(body, (0, start), tail, None);
                assert_eq!(caret, (0, start + 3));
                assert_eq!(
                    deckcraft_model::edit::text_range(body, (0, start), caret),
                    tail
                );
                assert_eq!(body.paragraphs.len(), 1);
                let changed = owner.document().unwrap().clone();
                let read = deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap();
                assert_eq!(
                    read, changed,
                    "break kind, properties, links and full presentation readback"
                );
                let mut outside = changed.clone();
                cell_mut(&mut outside, 0, id, 0, 0).unwrap().text = original_body.clone();
                assert_eq!(outside, original, "unrelated model state exact");
                let body = &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
                    .unwrap()
                    .text;
                deckcraft_model::edit::delete(body, (0, start), caret);
                for n in (0..count).rev() {
                    let position = source_len + n;
                    assert_eq!(
                        deckcraft_model::edit::text_range(body, (0, position), (0, position + 1)),
                        "\u{b}"
                    );
                    assert_eq!(
                        deckcraft_model::edit::delete(body, (0, position), (0, position + 1)),
                        (0, position)
                    );
                    assert_eq!(body.paragraphs.len(), 1);
                }
                assert_eq!(
                    body, &original_body,
                    "exact restoration after deleting generated ranges"
                );
                assert_eq!(owner.document().unwrap(), &original);
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 24);
    eprintln!("spec124 raw model: {cases} Unicode/empty/consecutive merged-cell cases; exact insertion/deletion carets and full readback");
}
