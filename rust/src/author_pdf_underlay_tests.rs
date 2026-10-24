// Included only in author_pdf::tests; fixtures use the same public native engines.
fn underlay_solid(rgb: [u8; 3]) -> Vec<u8> {
    crate::rendering::encode_png(1, 1, &[rgb[0], rgb[1], rgb[2], 255], 72.0).unwrap()
}

fn underlay_pixel(bytes: &[u8], dpi: f64, x: usize, y: usize) -> Vec<u8> {
    let png = crate::pdf::render(bytes, 0, dpi, None).unwrap();
    let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(frame.color_type, png::ColorType::Rgba);
    assert_eq!(
        frame.buffer_size(),
        frame.width as usize * frame.height as usize * 4
    );
    let factor = dpi / 72.0;
    let at =
        ((y as f64 * factor) as usize * frame.width as usize + (x as f64 * factor) as usize) * 4;
    pixels[at..at + 4].to_vec()
}

#[test]
fn typed_underlay_order_identity_output_copy_and_opaque_hiding() {
    pyo3::prepare_freethreaded_python();
    let mut document = EditablePdf::new(160.0, 160.0).unwrap();
    let blue = underlay_solid([0, 0, 255]);
    let red = underlay_solid([255, 0, 0]);
    document
        .add_image(
            0, &blue, 60.0, 60.0, 40.0, 40.0, None, 0, false, false, false,
        )
        .unwrap();
    document
        .add_text(0, "Original", 5.0, 5.0, 10.0, (0, 0, 0), false)
        .unwrap();
    document
        .add_link(
            0,
            (5.0, 5.0, 70.0, 15.0),
            Some("https://example.com/kept"),
            None,
        )
        .unwrap();
    let original = document.to_bytes().unwrap();
    let original_objects: Vec<_> = pdfcraft_edit::list_added(document.document().unwrap())
        .iter()
        .map(|v| v.obj)
        .collect();
    let counts = (document.items, document.input_bytes);
    document
        .add_image(0, &red, 40.0, 40.0, 80.0, 80.0, None, 0, false, false, true)
        .unwrap();
    document
        .add_text_box(
            0,
            "Under",
            (5.0, 140.0, 145.0, 18.0),
            10.0,
            (0, 0, 0),
            "center",
            None,
            true,
        )
        .unwrap();
    let items = pdfcraft_edit::list_added(document.document().unwrap());
    assert_eq!(items.len(), 4);
    assert!(matches!(&items[0].content, Content::Text(v) if v.text == "Under"));
    assert!(matches!(&items[1].content, Content::Image(_)));
    assert_eq!(items[2].obj, original_objects[0]);
    assert_eq!(items[3].obj, original_objects[1]);
    assert_eq!(
        (document.items, document.input_bytes),
        (counts.0 + 2, counts.1 + red.len() + 5)
    );
    let mut copied = document.copy().unwrap();
    for bytes in [
        pdfcraft_cos::write_incremental(document.document().unwrap(), &SaveOptions::default())
            .unwrap(),
        pdfcraft_cos::write_full(document.document().unwrap(), &SaveOptions::default()).unwrap(),
    ] {
        let reopened = EditablePdf::from_bytes(&bytes, None).unwrap();
        assert_eq!(
            pdfcraft_edit::list_added(reopened.document().unwrap()).len(),
            4
        );
        let payload = crate::pdf::extract(&bytes, None).unwrap();
        let text = payload.pages[0]
            .blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(text.matches("Original").count(), 1);
        assert_eq!(text.matches("Under").count(), 1);
        assert_eq!(payload.pages[0].links[0].uri, "https://example.com/kept");
        assert_eq!(payload.pages[0].images.len(), 2);
        for dpi in [72.0, 144.0] {
            assert_eq!(underlay_pixel(&bytes, dpi, 50, 50), [255, 0, 0, 255]);
            assert_eq!(underlay_pixel(&bytes, dpi, 80, 80), [0, 0, 255, 255]);
            assert_eq!(underlay_pixel(&bytes, dpi, 20, 80), [255; 4]);
        }
    }
    copied
        .add_image(
            0, &red, 40.0, 40.0, 80.0, 80.0, None, 0, false, false, false,
        )
        .unwrap();
    assert_eq!(
        underlay_pixel(&copied.to_bytes().unwrap(), 72.0, 80, 80),
        [255, 0, 0, 255]
    );
    assert_eq!(
        underlay_pixel(&document.to_bytes().unwrap(), 72.0, 80, 80),
        [0, 0, 255, 255]
    );
    assert_eq!(underlay_pixel(&original, 72.0, 50, 50), [255; 4]);
    // PdfCraft places behind-content after a page background mark, so an underlay shows over
    // a background (upstream #3591) and only opaque page content hides it.
    let mut backed = EditablePdf::new(160.0, 160.0).unwrap();
    backed.set_background(0, (0, 255, 0)).unwrap();
    backed
        .add_image(0, &red, 40.0, 40.0, 80.0, 80.0, None, 0, false, false, true)
        .unwrap();
    let backed_bytes = backed.to_bytes().unwrap();
    assert_eq!(underlay_pixel(&backed_bytes, 72.0, 80, 80), [255, 0, 0, 255]);
    assert_eq!(underlay_pixel(&backed_bytes, 72.0, 10, 10), [0, 255, 0, 255]);
    let mut hidden = EditablePdf::new(160.0, 160.0).unwrap();
    let white = underlay_solid([255, 255, 255]);
    hidden
        .add_image(
            0, &white, 0.0, 0.0, 160.0, 160.0, None, 0, false, false, false,
        )
        .unwrap();
    let blank = pdf2_raster(&hidden);
    hidden
        .add_text(0, "Hidden", 5.0, 5.0, 10.0, (0, 0, 0), true)
        .unwrap();
    hidden
        .add_image(0, &red, 40.0, 40.0, 80.0, 80.0, None, 0, false, false, true)
        .unwrap();
    assert_eq!(pdf2_raster(&hidden), blank);
    assert!(crate::pdf::extract(&hidden.to_bytes().unwrap(), None)
        .unwrap()
        .pages[0]
        .blocks
        .iter()
        .any(|b| b.text.contains("Hidden")));
}

#[test]
fn typed_underlay_view_transforms_retain_native_image_model_and_existing_paint() {
    use pdfcraft_cos::Object;
    let mut source = pdfcraft_create::blank(300.0, 400.0, 1).unwrap();
    let page = pdfcraft_organize::pages(&source).unwrap()[0].obj;
    source
        .update_dict(page, |d| {
            d.set(
                b"CropBox".to_vec(),
                Object::Array([20.0, 30.0, 220.0, 330.0].map(Object::Real).to_vec()),
            );
            d.set(b"UserUnit".to_vec(), Object::Real(1.25));
        })
        .unwrap();
    let bytes = resource_fixture_bytes(&source);
    let image = pdf2_quadrants();
    let blue = underlay_solid([0, 0, 255]);
    // Each page view crosses these four representative transform profiles, not all combinations.
    let transforms = [
        (0, false, false, None),
        (90, false, true, None),
        (180, true, false, Some((0.1, 0.2, 0.15, 0.05))),
        (270, true, true, Some((0.0, 0.0, 0.45, 0.0))),
    ];
    for rotation in [0, 90, 180, 270] {
        for (turn, h, v, crop) in transforms {
            let mut seed = EditablePdf::from_bytes(&bytes, None).unwrap();
            seed.rotate_page(0, rotation).unwrap();
            seed.add_image(
                0, &blue, 45.0, 100.0, 10.0, 10.0, None, 0, false, false, false,
            )
            .unwrap();
            let mut overlay = seed.copy().unwrap();
            seed.add_image(0, &image, 20.0, 80.0, 80.0, 80.0, crop, turn, h, v, true)
                .unwrap();
            overlay
                .add_image(0, &image, 20.0, 80.0, 80.0, 80.0, crop, turn, h, v, false)
                .unwrap();
            let under = pdfcraft_edit::list_added(seed.document().unwrap());
            let over = pdfcraft_edit::list_added(overlay.document().unwrap());
            assert_eq!(under[0].content, over[1].content);
            let (w, height, pixels) = pdf2_raster(&seed);
            let (ow, oh, control) = pdf2_raster(&overlay);
            assert_eq!((w, height), (ow, oh));
            let mut image_colors = std::collections::BTreeSet::new();
            for y in 0..height {
                for x in 0..w {
                    let at = (y * w + x) * 4;
                    if (45..55).contains(&x) && (100..110).contains(&y) {
                        assert_eq!(&pixels[at..at + 4], &[0, 0, 255, 255]);
                    } else {
                        assert_eq!(&pixels[at..at + 4], &control[at..at + 4]);
                    }
                    if (25..95).contains(&x) && (85..155).contains(&y) {
                        image_colors.insert(pixels[at..at + 4].to_vec());
                    }
                }
            }
            assert!(image_colors.contains(&vec![255, 0, 0, 255]));
            assert!(image_colors.contains(&vec![255, 255, 0, 255]));
            let saved = seed.to_bytes().unwrap();
            assert_eq!(
                pdf2_raster(&EditablePdf::from_bytes(&saved, None).unwrap()),
                (w, height, pixels)
            );
            let payload = crate::pdf::extract(&saved, None).unwrap();
            assert_eq!(payload.pages[0].images.len(), 2);
            assert!(payload.pages[0]
                .images
                .iter()
                .any(|v| v.metadata["width"] == 20 && v.metadata["height"] == 20));
        }
    }
}

#[test]
fn typed_underlay_collision_limits_and_unsupported_font_are_atomic() {
    pyo3::prepare_freethreaded_python();
    let image = underlay_solid([255, 0, 0]);
    for inherited in [false, true] {
        let source = resource_fixture(
            collision_font_resources(b"PCFHelvetica"),
            b"BT /PCFHelvetica 20 Tf 10 150 Td (MMMM) Tj ET",
            inherited,
        );
        let mut document = EditablePdf::from_bytes(&resource_fixture_bytes(&source), None).unwrap();
        let before = document.to_bytes().unwrap();
        let counters = (document.items, document.input_bytes);
        for boxed in [false, true] {
            let result = if boxed {
                document.add_text_box(
                    0,
                    "Fail",
                    (10.0, 100.0, 100.0, 40.0),
                    12.0,
                    (0, 0, 0),
                    "left",
                    None,
                    true,
                )
            } else {
                document.add_text(0, "Fail", 10.0, 100.0, 12.0, (0, 0, 0), true)
            };
            assert!(result.unwrap_err().to_string().contains("resource"));
            assert_eq!(document.to_bytes().unwrap(), before);
            assert_eq!((document.items, document.input_bytes), counters);
        }
        document
            .add_image(
                0, &image, 10.0, 100.0, 10.0, 10.0, None, 0, false, false, true,
            )
            .unwrap();
        assert_eq!(
            original_resource_pixels(document.document().unwrap()),
            original_resource_pixels(&source)
        );
    }
    // A valid existing image binding that happens to use the next generated name
    // must not be overwritten. Prediction is test-only, using the public native factory.
    for inherited in [false, true] {
        use pdfcraft_cos::Object;
        let mut document = EditablePdf::new(160.0, 160.0).unwrap();
        let blue = underlay_solid([0, 0, 255]);
        document
            .add_image(
                0, &blue, 60.0, 60.0, 30.0, 30.0, None, 0, false, false, false,
            )
            .unwrap();
        let old = match &pdfcraft_edit::list_added(document.document().unwrap())[0].content {
            Content::Image(value) => value.image,
            _ => panic!("expected image"),
        };
        let mut probe = document.document().unwrap().clone();
        let (next, _) = pdfcraft_create::image_xobject(&mut probe, "image.png", &image).unwrap();
        let native = document.document.as_mut().unwrap();
        let page = pdfcraft_organize::pages(native).unwrap()[0].obj;
        let page_dict = native.get(page).as_dict().unwrap().clone();
        let mut resources = native.dict(page_dict.get(b"Resources").unwrap()).unwrap();
        let mut images = native.dict(resources.get(b"XObject").unwrap()).unwrap();
        images.set(format!("PCImg{}", next.num).into_bytes(), Object::Ref(old));
        resources.set(b"XObject".to_vec(), Object::Dict(images));
        let owner = if inherited {
            page_dict.reference(b"Parent").unwrap()
        } else {
            page
        };
        if inherited {
            native
                .update_dict(page, |d| {
                    d.remove(b"Resources");
                })
                .unwrap();
        }
        native
            .update_dict(owner, |d| {
                d.set(b"Resources".to_vec(), Object::Dict(resources));
            })
            .unwrap();
        let before = document.to_bytes().unwrap();
        let counts = (document.items, document.input_bytes);
        let error = document
            .add_image(
                0, &image, 10.0, 10.0, 20.0, 20.0, None, 0, false, false, true,
            )
            .unwrap_err();
        assert!(error.to_string().contains("XObject resource"));
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!((document.items, document.input_bytes), counts);
        assert_eq!(underlay_pixel(&before, 72.0, 70, 70), [0, 0, 255, 255]);
    }
    let mut document = EditablePdf::new(160.0, 160.0).unwrap();
    let before = document.to_bytes().unwrap();
    assert!(document
        .add_text_box(
            0,
            "Fail",
            (5.0, 5.0, 100.0, 20.0),
            10.0,
            (0, 0, 0),
            "left",
            Some(b"invalid unread font"),
            true
        )
        .unwrap_err()
        .to_string()
        .contains("unsupported:PDF underlays with supplied fonts"));
    assert_eq!(document.to_bytes().unwrap(), before);
    assert_eq!((document.items, document.input_bytes), (0, 0));
    for (items, input) in [(MAX_ITEMS, 0), (0, MAX_DOCUMENT_INPUT)] {
        document.items = items;
        document.input_bytes = input;
        assert!(document
            .add_text(0, "A", 5.0, 5.0, 10.0, (0, 0, 0), true)
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert!(document
            .add_image(0, &image, 10.0, 100.0, 10.0, 10.0, None, 0, false, false, true)
            .is_err());
        assert_eq!(document.to_bytes().unwrap(), before);
        assert_eq!((document.items, document.input_bytes), (items, input));
    }
    document.items = MAX_ITEMS - 1;
    document.input_bytes = MAX_DOCUMENT_INPUT - 1;
    document
        .add_text(0, "A", 5.0, 5.0, 10.0, (0, 0, 0), true)
        .unwrap();
    assert_eq!(
        (document.items, document.input_bytes),
        (MAX_ITEMS, MAX_DOCUMENT_INPUT)
    );
    let accepted = document.to_bytes().unwrap();
    assert!(document
        .add_text(0, "B", 5.0, 30.0, 10.0, (0, 0, 0), true)
        .is_err());
    assert_eq!(document.to_bytes().unwrap(), accepted);
}
