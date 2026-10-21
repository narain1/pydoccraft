//! Spec148 links the reviewed native copy capability; no Python copy API is added here.
use super::*;
use crate::model::PagePayload;

fn labels(page: &PagePayload) -> String {
    page.blocks
        .iter()
        .map(|block| {
            block.table.as_ref().map_or_else(
                || block.text.clone(),
                |table| {
                    table
                        .rows
                        .iter()
                        .map(|row| row.join(" "))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn positive_ink(bytes: &[u8], dpi: f64) {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().unwrap();
    let mut data = vec![0; reader.output_buffer_size().expect("bounded fixture PNG")];
    let info = reader.next_frame(&mut data).unwrap();
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(info.color_type, png::ColorType::Rgba);
    data.truncate(info.buffer_size());
    assert_eq!(data.len(), info.width as usize * info.height as usize * 4);
    assert_eq!(
        (info.width, info.height),
        ((576. * dpi / 72.) as u32, (360. * dpi / 72.) as u32)
    );
    let red = data
        .chunks_exact(4)
        .filter(|p| {
            p[0] > 100
                && u16::from(p[0]) > u16::from(p[1]) + 60
                && u16::from(p[0]) > u16::from(p[2]) + 60
        })
        .count();
    assert!(red > 50, "actual red text ink: {red}");
    let scale = dpi / 72.;
    let mut blue = 0;
    for y in (205. * scale) as usize..(235. * scale) as usize {
        for x in (485. * scale) as usize..(515. * scale) as usize {
            let p = &data[(y * info.width as usize + x) * 4..][..4];
            blue += usize::from(p[2] > 140 && p[0] < 80 && p[1] < 120);
        }
    }
    assert!(blue > 400, "actual shared picture ink: {blue}");
}

#[test]
fn linked_native_slide_copy_preserves_references_and_real_outputs() {
    pyo3::prepare_freethreaded_python();
    let mut owner = EditablePptx::new(576., 360.).unwrap();
    owner.add_slide("blank").unwrap();
    owner.add_slide("blank").unwrap();
    owner.set_slide_notes(0, "Retained speaker notes").unwrap();
    let text = ShapeId(
        owner
            .add_text(0, "Copied label", 24., 24., 250., 40., 22.)
            .unwrap(),
    );
    let table = ShapeId(
        owner
            .add_table(
                0,
                vec![vec!["Table cell".into(), "Other cell".into()]],
                24.,
                100.,
                440.,
                100.,
                20.,
            )
            .unwrap(),
    );
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[20, 80, 220])
            .unwrap();
    }
    owner
        .add_image(0, &png, 480., 200., 40., 40., "Shared picture")
        .unwrap();
    let mut doc = owner.document().unwrap().clone();
    let source = doc.slides[0].id;
    let other = doc.slides[1].id;
    let connector = ShapeId(doc.alloc_id().unwrap());
    let target_link = |target| Hyperlink {
        action: Action::Slide { slide: target },
        tooltip: "Target".into(),
        highlight_click: false,
        sound: None,
        stop_sound: false,
        zoom_return: None,
    };
    let slide = Arc::make_mut(&mut doc.slides[0]);
    let shape = slide.shape_mut(text).unwrap();
    shape.click = Some(target_link(source));
    shape.hover = Some(target_link(other));
    let props = &mut shape.text.as_mut().unwrap().paragraphs[0].runs[0].props;
    props.fill = Some(solid((192, 0, 0)));
    props.link = Some(Hyperlink {
        action: Action::Url {
            url: "https://example.com/copied".into(),
        },
        ..target_link(source)
    });
    slide.shapes.push(Shape {
        id: connector,
        xfrm: Some(Xfrm::new(40., 80., 100., 20.)),
        kind: ShapeKind::Connector {
            start: Some((text, 0)),
            end: Some((table, 1)),
        },
        ..Shape::default()
    });
    assert!(doc.validate().is_empty());
    let before = doc.clone();
    assert_eq!(
        doc.copy_slide(SlideId(u32::MAX)),
        Err(deckcraft_model::CopySlideError::NoSlide)
    );
    assert_eq!(doc, before);
    let copied = doc.copy_slide(source).unwrap();
    assert_ne!(copied, source);
    assert_ne!(copied, other);
    assert_eq!(&doc.slides[..2], before.slides.as_slice());
    assert_eq!(doc.media.len(), 1);
    assert_eq!(doc.media, before.media);
    assert!(Arc::ptr_eq(&doc.media[0].data, &before.media[0].data));
    assert_eq!(doc.masters.len(), before.masters.len());
    for (a, b) in doc.masters.iter().zip(&before.masters) {
        assert!(Arc::ptr_eq(a, b));
    }
    assert_eq!(doc.slides[2].layout, doc.slides[0].layout);
    let old: Vec<_> = doc.slides[0].shapes.iter().map(|shape| shape.id).collect();
    let new: Vec<_> = doc.slides[2].shapes.iter().map(|shape| shape.id).collect();
    assert_eq!(old.len(), new.len());
    assert!(new.iter().all(|id| !old.contains(id)));
    let mapped = |id| new[old.iter().position(|old| *old == id).unwrap()];
    assert!(
        matches!(doc.slides[2].shape(mapped(connector)).unwrap().kind, ShapeKind::Connector { start: Some((s, 0)), end: Some((e, 1)) } if s == mapped(text) && e == mapped(table))
    );
    let bytes = deckcraft_pptx::export(&doc).unwrap();
    let reopened = deckcraft_pptx::import(&bytes).unwrap();
    assert_eq!(reopened.slides.len(), 3);
    assert!(reopened.validate().is_empty());
    assert_eq!(reopened.slides[2].notes.text(), "Retained speaker notes");
    let read_text = reopened.slides[2].shape(mapped(text)).unwrap();
    assert_eq!(
        read_text.click.as_ref().unwrap().action,
        Action::Slide {
            slide: reopened.slides[0].id
        }
    );
    assert_eq!(
        read_text.hover.as_ref().unwrap().action,
        Action::Slide {
            slide: reopened.slides[1].id
        }
    );
    assert!(
        matches!(reopened.slides[2].shape(mapped(connector)).unwrap().kind, ShapeKind::Connector { start: Some((s, 0)), end: Some((e, 1)) } if s == mapped(text) && e == mapped(table))
    );
    let extracted = crate::pptx::extract(&bytes).unwrap();
    assert_eq!(extracted.pages.len(), 3);
    let pdf = crate::pptx::to_pdf(&bytes).unwrap();
    let pdf_model = crate::pdf::extract(&pdf, None).unwrap();
    assert_eq!(pdf_model.pages.len(), 3);
    let before_pdf = crate::pptx::to_pdf(&deckcraft_pptx::export(&before).unwrap()).unwrap();
    let before_model = crate::pdf::extract(&before_pdf, None).unwrap();
    assert_eq!(
        serde_json::to_value(&before_model.pages[0].links).unwrap(),
        serde_json::to_value(&pdf_model.pages[0].links).unwrap()
    );
    // Overlapping shape self-link and run URI deliberately have unresolved PDF labels.
    // Preserve targets, geometry and ambiguity metadata exactly, including on the copy.
    let mut copied_links = pdf_model.pages[2].links.clone();
    for link in &mut copied_links {
        link.page = Some(0);
    }
    assert_eq!(
        serde_json::to_value(&before_model.pages[0].links).unwrap(),
        serde_json::to_value(&copied_links).unwrap()
    );
    assert_eq!(copied_links.len(), 2);
    assert!(copied_links.iter().any(|link| link.uri == "#page=1"));
    assert!(copied_links
        .iter()
        .any(|link| link.uri == "https://example.com/copied"));
    assert!(copied_links.iter().all(|link| link.text.is_empty()
        && link.metadata["label_status"] == "unresolved_geometry"
        && link.bbox.is_some()));
    for index in [0, 2] {
        let page = &extracted.pages[index];
        assert_eq!(page.tables.len(), 1);
        assert_eq!(page.tables[0].rows, vec![vec!["Table cell", "Other cell"]]);
        assert_eq!(page.images.len(), 1);
        assert!(page
            .blocks
            .iter()
            .any(|block| block.kind == "notes" && block.text == "Retained speaker notes"));
        assert!(page
            .links
            .iter()
            .any(|link| link.uri == "https://example.com/copied" && link.text == "Copied label"));
        for page in [page, &pdf_model.pages[index]] {
            let text = labels(page);
            for label in ["Copied label", "Table cell", "Other cell"] {
                assert_eq!(text.matches(label).count(), 1, "{text:?}");
            }
        }
    }
    for dpi in [72., 144.] {
        let a = crate::pptx::render(&bytes, 0, dpi).unwrap();
        let b = crate::pptx::render(&bytes, 2, dpi).unwrap();
        assert_eq!(a, b);
        positive_ink(&a, dpi);
        let a = crate::pdf::render(&pdf, 0, dpi, None).unwrap();
        let b = crate::pdf::render(&pdf, 2, dpi, None).unwrap();
        assert_eq!(a, b);
        positive_ink(&a, dpi);
    }
    // A nonoverlapping URI control still resolves the actual exported PDF label.
    let mut uri_only = before.clone();
    uri_only
        .slide_mut(source)
        .unwrap()
        .shape_mut(text)
        .unwrap()
        .click = None;
    let uri_pdf = crate::pptx::to_pdf(&deckcraft_pptx::export(&uri_only).unwrap()).unwrap();
    let uri_model = crate::pdf::extract(&uri_pdf, None).unwrap();
    assert_eq!(uri_model.pages[0].links.len(), 1);
    let link = &uri_model.pages[0].links[0];
    assert_eq!(link.uri, "https://example.com/copied");
    assert_eq!(link.text, "Copied label");
    assert_eq!(link.metadata["label_status"], "resolved");
    let ShapeKind::Table(t) = &mut doc
        .slide_mut(copied)
        .unwrap()
        .shape_mut(mapped(table))
        .unwrap()
        .kind
    else {
        panic!("table");
    };
    t.cell_mut(0, 0).unwrap().text = TextBody::from_text("Only the copy changes");
    assert_eq!(doc.slides[0], before.slides[0]);
}
