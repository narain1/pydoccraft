use super::*;

fn owner() -> EditableDocx {
    pyo3::prepare_freethreaded_python();
    let mut d = EditableDocx::new();
    d.set_page_size(300.0, 360.0).unwrap();
    d.set_margins(30.0, 30.0, 30.0, 30.0).unwrap();
    d
}
fn create(d: &mut EditableDocx, kind: &str) -> usize {
    add(
        d,
        kind,
        r#"{"size":[40,20],"fill":[255,0,0],"stroke":[[0,0,255],2]}"#,
        d.revision,
    )
    .unwrap()
}
fn patch(d: &mut EditableDocx, id: usize, input: &str) -> PyResult<()> {
    update(d, id, input, d.revision, false)
}
fn place(d: &mut EditableDocx, id: usize, input: &str) -> PyResult<()> {
    update(d, id, input, d.revision, true)
}
fn snapshot(d: &EditableDocx) -> (Vec<u8>, u64, usize, usize, usize, usize) {
    (
        d.to_bytes().unwrap(),
        d.revision,
        d.next_block_id,
        d.text_bytes,
        d.cell_count,
        d.media_bytes,
    )
}
#[test]
fn closed_shape_native_lifetime_placeholders_sections_copy_and_stale_ids() {
    let mut d = owner();
    let id = create(&mut d, "rectangle");
    assert_eq!(d.document().unwrap().body.len(), 1);
    assert!(d.paragraph_ids().unwrap().is_empty());
    assert!(d.image_ids().unwrap().is_empty());
    assert_eq!(ids(&d).unwrap(), vec![id]);
    assert_eq!(d.text_bytes, 0);
    let mut copy = d.copy().unwrap();
    patch(&mut copy, id, r#"{"fill":[0,255,0]}"#).unwrap();
    assert_ne!(info(&d, id).unwrap(), info(&copy, id).unwrap());
    d.add_section("next_page").unwrap();
    let before = d.document().unwrap().clone();
    let location = d.locations[&id].clone();
    let section = before
        .para(location.0, &location.1)
        .unwrap()
        .section
        .clone();
    let revision = d.revision;
    remove(&mut d, id, revision).unwrap();
    let p = d.document().unwrap().para(location.0, &location.1).unwrap();
    assert!(p.text.is_empty());
    assert!(p.objects.is_empty());
    assert_eq!(p.section, section);
    assert_eq!(d.document().unwrap().body.len(), before.body.len());
    assert!(get(&d, id).is_err());
    assert_eq!(d.paragraph_ids().unwrap(), vec![id]);
    let replacement = create(&mut d, "ellipse");
    assert_ne!(id, replacement);
    assert!(get(&d, id).is_err());
    assert!(get(&copy, id).is_ok());
    d.close();
    assert!(info(&d, replacement).is_err());
    assert!(ids(&d).is_err());
}
#[test]
fn closed_shape_properties_strict_values_noops_and_atomic_revision() {
    let mut d = owner();
    let id = create(&mut d, "rectangle");
    let saved = snapshot(&d);
    for input in [
        r#"{"size":[0,20]}"#,
        r#"{"size":[4001,20]}"#,
        r#"{"size":[241,20]}"#,
        r#"{"size":[40.01,20]}"#,
        r#"{"size":[true,20]}"#,
        r#"{"fill":[0,0,256]}"#,
        r#"{"fill":[true,0,0]}"#,
        r#"{"stroke":[null,1]}"#,
        r#"{"stroke":[[0,0,0],0.2]}"#,
        r#"{"stroke":[[0,0,0],100.05]}"#,
        r#"{"stroke":[[0,0,0],20.05]}"#,
        r#"{"size":[1,1]}"#,
        r#"{"stroke":[[0,0,0],200]}"#,
        r#"{"rotation":1}"#,
        r#"[]"#,
    ] {
        assert!(patch(&mut d, id, input).is_err(), "{input}");
        assert_eq!(snapshot(&d), saved);
    }
    assert!(patch(&mut d, id, &" ".repeat(1025)).is_err());
    for input in ["{}", r#"{"size":[40,20]}"#] {
        patch(&mut d, id, input).unwrap();
        assert_eq!(snapshot(&d), saved);
    }
    let stale = d.revision - 1;
    assert!(update(&mut d, id, "{}", stale, false).is_err());
    for kind in ["line", "textbox", "RECTANGLE", "image"] {
        assert!(add(&mut d, kind, r#"{"size":[40,20]}"#, saved.1).is_err());
        assert_eq!(snapshot(&d), saved);
    }
    patch(&mut d, id, r#"{"fill":null,"stroke":null}"#).unwrap();
    assert_eq!(
        info(&d, id).unwrap(),
        r#"{"fill":null,"kind":"rectangle","size":[40.0,20.0],"stroke":null}"#
    );
    d.revision = u64::MAX - 1;
    patch(&mut d, id, r#"{"fill":[255,0,0]}"#).unwrap();
    assert_eq!(d.revision, u64::MAX);
    let saved = snapshot(&d);
    assert!(patch(&mut d, id, r#"{"fill":[0,255,0]}"#).is_err());
    assert_eq!(snapshot(&d), saved);
    assert!(remove(&mut d, id, u64::MAX).is_err());
    assert_eq!(snapshot(&d), saved);
}
#[test]
fn closed_shape_geometry_full_outline_cardinality_and_every_candidate() {
    let mut d = owner();
    let id = create(&mut d, "ellipse");
    place(
        &mut d,
        id,
        r#"{"wrap":"behind","anchor":"page","position":[80,110]}"#,
    )
    .unwrap();
    let saved = snapshot(&d);
    for input in [
        r#"{"position":[0,110]}"#,
        r#"{"position":[280,110]}"#,
        r#"{"position":[80,350]}"#,
        r#"{"wrap":"front"}"#,
    ] {
        assert!(place(&mut d, id, input).is_err());
        assert_eq!(snapshot(&d), saved);
    }
    assert!(d.set_page_size(100.0, 360.0).is_err());
    assert_eq!(snapshot(&d), saved);
    assert!(d
        .set_section_props(0, r#"{"vertical_alignment":"center"}"#)
        .is_err());
    assert_eq!(snapshot(&d), saved);
    let document = d.document().unwrap();
    let map = inventory(document).unwrap();
    let layout = wordcraft_layout::layout(
        document,
        &mut wordcraft_layout::LayoutCache::new(),
        &wordcraft_layout::LayoutOptions::default(),
    );
    let mut page = layout.pages[0].clone();
    let index = page
        .items
        .iter()
        .position(|p| matches!(p, wordcraft_layout::Placed::Object { .. }))
        .unwrap();
    let valid = page.items[index].clone();
    let mut seen = BTreeSet::new();
    page_bounds(&page, &map, &mut seen).unwrap();
    assert_eq!(seen.len(), 1);
    page.items.push(valid.clone());
    assert!(page_bounds(&page, &map, &mut BTreeSet::new()).is_err());
    page.items.pop();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
        page.items[index] = valid.clone();
        if let wordcraft_layout::Placed::Object { rect, .. } = &mut page.items[index] {
            rect.x = bad;
        }
        assert!(page_bounds(&page, &map, &mut BTreeSet::new()).is_err());
    }
    page.items[index] = valid.clone();
    if let wordcraft_layout::Placed::Object { rect, .. } = &mut page.items[index] {
        rect.w = 4000.0;
    }
    assert!(page_bounds(&page, &map, &mut BTreeSet::new()).is_err());
    page.items[index] = valid;
    page.h = f32::NAN;
    assert!(page_bounds(&page, &map, &mut BTreeSet::new()).is_err());
    // A privately malformed source cannot bypass output validation.
    let (story, path) = d.locations[&id].clone();
    let object = &mut d
        .document
        .as_mut()
        .unwrap()
        .para_mut(story, &path)
        .unwrap()
        .objects[0];
    object.float_mut().unwrap().x = 299.0;
    assert!(d.to_bytes().is_err());
    assert!(d.add_paragraph("must not commit").is_err());
}
#[test]
fn closed_shape_profile_registry_and_reference_guards() {
    let mut d = owner();
    let id = create(&mut d, "rectangle");
    let original = d.document().unwrap().clone();
    for change in 0..8 {
        let mut copy = d.copy().unwrap();
        let (story, path) = copy.locations[&id].clone();
        let object = &mut copy
            .document
            .as_mut()
            .unwrap()
            .para_mut(story, &path)
            .unwrap()
            .objects[0];
        if let InlineObject::Shape {
            kind, float, story, ..
        } = object
        {
            match change {
                0 => *kind = ShapeKind::Line,
                1 => float.rot = 10.0,
                2 => float.hidden = true,
                3 => float.flip_h = true,
                4 => float.h_rel = Anchor::Margin,
                5 => float.wrap = Wrap::InFrontOfText,
                6 => *story = Some(999),
                _ => float.effect[0] = 1.0,
            }
        }
        assert!(copy.to_bytes().is_err());
        assert!(copy.copy().is_err());
    }
    assert_eq!(d.document().unwrap(), &original);
    for direction in [TextDirection::Up, TextDirection::Down] {
        let mut bad = d.copy().unwrap();
        bad.document.as_mut().unwrap().last_section.text_direction = direction;
        let before = wordcraft_docx::write(bad.document().unwrap()).unwrap();
        let revision = bad.revision;
        assert!(bad.to_bytes().is_err());
        assert!(bad.copy().is_err());
        assert!(bad.add_paragraph("Rejected rotated section").is_err());
        assert!(patch(&mut bad, id, r#"{"fill":[0,255,0]}"#).is_err());
        assert_eq!(bad.revision, revision);
        assert_eq!(
            wordcraft_docx::write(bad.document().unwrap()).unwrap(),
            before
        );
        bad.document.as_mut().unwrap().last_section.text_direction = TextDirection::Horizontal;
        assert!(!bad.to_bytes().unwrap().is_empty());
    }

    let mut malformed = d.copy().unwrap();
    malformed
        .locations
        .insert(id, (StoryRef::Body, Path(Vec::new())));
    assert!(info(&malformed, id).is_err());
    assert!(formatting(&malformed, id).is_err());
    assert!(malformed.copy().is_err());
    assert!(malformed.to_bytes().is_err());
    let mut bad = d.copy().unwrap();
    bad.shape_ids.clear();
    let mut copied_orphan = bad.copy().unwrap();
    assert_eq!(copied_orphan.document().unwrap(), bad.document().unwrap());
    let before = copied_orphan.document().unwrap().clone();
    let revision = copied_orphan.revision;
    assert!(copied_orphan.to_bytes().is_err());
    assert!(copied_orphan.add_paragraph("Rejected orphan").is_err());
    assert_eq!(copied_orphan.document().unwrap(), &before);
    assert_eq!(copied_orphan.revision, revision);
    assert!(bad.to_bytes().is_err());
    let mut bad = d.copy().unwrap();
    bad.locations.insert(999, bad.locations[&id].clone());
    assert!(bad.to_bytes().is_err());
    let mut bad = d.copy().unwrap();
    bad.images.push(id);
    assert!(bad.copy().is_err());
    assert!(d.set_paragraph_text(id, "cannot repurpose shape").is_err());
    let table = d.add_table(vec![vec!["Cell".into()]]).unwrap();
    for operation in 0..2 {
        let mut malformed = d.copy().unwrap();
        let prefix = malformed.locations[&table].1 .0.clone();
        malformed.locations.insert(
            id,
            (
                StoryRef::Body,
                Path([prefix.as_slice(), &[0, 0, 0]].concat()),
            ),
        );
        let before = wordcraft_docx::write(malformed.document().unwrap()).unwrap();
        let revision = malformed.revision;
        let result = if operation == 0 {
            malformed.set_cell_text(table, 0, 0, "Rejected")
        } else {
            malformed.remove_table(table)
        };
        assert!(result.is_err());
        assert_eq!(malformed.revision, revision);
        assert!(malformed.shape_ids.contains(&id));
        assert_eq!(
            wordcraft_docx::write(malformed.document().unwrap()).unwrap(),
            before
        );
    }

    // Destructive operations validate source shapes before pruning native content.
    // Public copy can retain a diagnostic snapshot with an empty private registry.
    for in_table in [false, true] {
        let mut malformed = owner();
        let target = if in_table {
            malformed.add_table(vec![vec!["Cell".into()]]).unwrap()
        } else {
            malformed.add_paragraph("Paragraph").unwrap()
        };
        let (story, mut path) = malformed.locations[&target].clone();
        if in_table {
            path.0.extend([0, 0, 0]);
        }
        let shape = original.body[0].as_para().unwrap().objects[0].clone();
        let paragraph = malformed
            .document
            .as_mut()
            .unwrap()
            .para_mut(story, &path)
            .unwrap();
        paragraph
            .insert_object(0, shape, &CharProps::default())
            .unwrap();
        let mut malformed = malformed.copy().unwrap();
        let before = malformed.document().unwrap().clone();
        let revision = malformed.revision;
        let result = if in_table {
            malformed.remove_table(target)
        } else {
            malformed.remove_paragraph(target)
        };
        assert!(result.is_err());
        assert_eq!(malformed.document().unwrap(), &before);
        assert_eq!(malformed.revision, revision);
    }

    // Existing unused-style reference walker must admit precisely this reference-free shape.
    let style = d
        .add_style("Unused alongside shape", "paragraph", None)
        .unwrap();
    d.remove_style(&style, d.revision).unwrap();
    assert!(info(&d, id).is_ok());
}
#[test]
fn closed_shape_resource_work_shape_count_and_allocator_boundaries() {
    let mut d = owner();
    let id = create(&mut d, "rectangle");
    let model = d.document().unwrap();
    inventory_with_limit(model, 5).unwrap();
    assert!(inventory_with_limit(model, 4).is_err());
    let block = model.body[0].clone();
    let mut model = model.clone();
    model.body = vec![block.clone(); MAX_SHAPES];
    assert_eq!(inventory(&model).unwrap().len(), MAX_SHAPES);
    model.body.push(block);
    assert!(inventory(&model).is_err());
    let saved = snapshot(&d);
    d.next_block_id = usize::MAX;
    let rev = d.revision;
    assert!(add(&mut d, "ellipse", r#"{"size":[40,20]}"#, rev).is_err());
    assert_eq!(d.revision, saved.1);
    assert_eq!(d.to_bytes().unwrap(), saved.0);
    d.next_block_id = saved.2;
    d.shape_ids = (0..MAX_SHAPES).collect();
    assert!(add(&mut d, "ellipse", r#"{"size":[40,20]}"#, rev).is_err());
    assert_eq!(d.revision, rev);
    d.shape_ids = BTreeSet::from([id]);
    assert_eq!(snapshot(&d), saved);
}
#[test]
fn closed_shape_locator_composition_movement_and_unrelated_accounting() {
    let mut d = owner();
    let first = d.add_paragraph("First kept").unwrap();
    let a = create(&mut d, "rectangle");
    let t = d.add_table(vec![vec!["Cell kept".into()]]).unwrap();
    let last = d.add_paragraph("Last kept").unwrap();
    let b = create(&mut d, "ellipse");
    let before = (d.text_bytes, d.cell_count, d.media_bytes);
    let shape_a = info(&d, a).unwrap();
    let shape_b = info(&d, b).unwrap();
    d.move_paragraph(last, first, d.revision).unwrap();
    assert_eq!(ids(&d).unwrap(), vec![a, b]);
    d.remove_table(t).unwrap();
    assert_eq!(info(&d, a).unwrap(), shape_a);
    assert_eq!(info(&d, b).unwrap(), shape_b);
    d.remove_paragraph(first).unwrap();
    assert_eq!(ids(&d).unwrap(), vec![a, b]);
    assert_eq!(d.media_bytes, before.2);
    assert!(d.text_bytes < before.0);
    assert!(d.cell_count < before.1);
    let section = d.add_section("next_page").unwrap();
    assert_eq!(section, 1);
    let c = create(&mut d, "rectangle");
    assert_eq!(ids(&d).unwrap(), vec![a, b, c]);
    let data = d.to_bytes().unwrap();
    let reopened = wordcraft_docx::read(&data).unwrap();
    // Reader uses Float::default() for inline drawings: its irrelevant vertical
    // anchor is Column, while the owned factory canonicalizes it to Paragraph.
    // Reopening is read-only; do not apply owned-registry admission to its IDs.
    let objects = reopened
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| &p.objects)
        .collect::<Vec<_>>();
    assert_eq!(objects.len(), 3);
    for (object, kind) in objects.iter().zip([
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Rectangle,
    ]) {
        let InlineObject::Shape {
            kind: actual,
            w,
            h,
            fill,
            stroke,
            stroke_width,
            float,
            ..
        } = object
        else {
            panic!("native shape disappeared on readback")
        };
        assert_eq!(*actual, kind);
        assert_eq!((*w, *h), (40.0, 20.0));
        assert_eq!(*fill, Some(Rgb(255, 0, 0)));
        assert_eq!(*stroke, Some(Rgb(0, 0, 255)));
        assert_eq!(*stroke_width, 2.0);
        assert_eq!(float.wrap, Wrap::Inline);
        assert_eq!(float.v_rel, Anchor::Column);
    }
}
