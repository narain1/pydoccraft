//! Spec121 native spacing evidence and bounded owned-cell adapter acceptance.
use super::*;
use deckcraft_model::text::{Bullet, LevelStyle, ParaProps, Paragraph as NativeParagraph, Spacing};

fn fixture(merged: u8) -> (EditablePptx, u32) {
    pyo3::prepare_freethreaded_python();
    let mut owner = EditablePptx::new(576.0, 576.0).unwrap();
    owner.add_slide("blank").unwrap();
    owner.set_slide_notes(0, "Retained notes").unwrap();
    let mut image = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut image, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[200, 160, 0])
            .unwrap();
    }
    owner
        .add_image(0, &image, 500.0, 530.0, 24.0, 24.0, "Retained image")
        .unwrap();
    let id = owner
        .add_table(
            0,
            vec![
                vec!["First".into(), "Neighbor".into()],
                vec!["Lower".into(), "Outside".into()],
            ],
            24.0,
            24.0,
            528.0,
            480.0,
            18.0,
        )
        .unwrap();
    match merged {
        0 => (),
        1 => owner.merge_cells(0, id, 0, 0, 0, 1).unwrap(),
        2 => owner.merge_cells(0, id, 0, 0, 1, 0).unwrap(),
        3 => owner.merge_cells(0, id, 0, 0, 1, 1).unwrap(),
        _ => unreachable!(),
    }
    let url = make_link(
        owner.document().unwrap(),
        Some("https://example.com/retained"),
        None,
        "Retained link",
    )
    .unwrap();
    let ShapeKind::Table(table) = &mut shape_mut(owner.presentation.as_mut().unwrap(), 0, id)
        .unwrap()
        .kind
    else {
        panic!("table")
    };
    table.style = "none".into();
    let cell = table.cell_mut(0, 0).unwrap();
    cell.margins = Some([0.0; 4]);
    cell.text.paragraphs = ["First office", "Middle café", "Last tail"]
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let mut p = NativeParagraph::new(text);
            p.props.bullet = Some(Bullet::None);
            p.props.align = Some(Align::Left);
            p.def_props.size = Some(18.0);
            p.end_props.italic = Some(i == 1);
            p.runs[0].props.fill = Some(solid([(224, 0, 0), (0, 160, 0), (0, 0, 224)][i]));
            p.runs[0].props.link = if i == 0 { url.clone() } else { None };
            p
        })
        .collect();
    let mut inherited = LevelStyle::default();
    inherited.para.margin_left = Some(12.0);
    inherited.para.margin_right = Some(6.0);
    inherited.para.indent = Some(3.0);
    inherited.para.space_before = Some(Spacing::Pts(4.0));
    inherited.para.space_after = Some(Spacing::Pts(5.0));
    inherited.para.line_spacing = Some(Spacing::Pct(1.25));
    cell.text.list_style.set(0, inherited);
    owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
    owner.refresh_budgets().unwrap();
    (owner, id)
}

fn props(owner: &mut EditablePptx, id: u32, index: usize) -> &mut ParaProps {
    &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
        .unwrap()
        .text
        .paragraphs[index]
        .props
}

fn six(p: &ParaProps) -> [Option<f64>; 6] {
    fn points(s: Option<Spacing>) -> Option<f64> {
        s.map(|s| match s {
            Spacing::Pts(v) => v,
            _ => panic!("point spacing"),
        })
    }
    let multiplier = p.line_spacing.map(|s| match s {
        Spacing::Pct(v) => v,
        _ => panic!("multiplier"),
    });
    [
        p.margin_left,
        p.margin_right,
        p.indent,
        points(p.space_before),
        points(p.space_after),
        multiplier,
    ]
}

fn set_six(p: &mut ParaProps, values: Option<[f64; 6]>) {
    p.margin_left = values.map(|v| v[0]);
    p.margin_right = values.map(|v| v[1]);
    p.indent = values.map(|v| v[2]);
    p.space_before = values.map(|v| Spacing::Pts(v[3]));
    p.space_after = values.map(|v| Spacing::Pts(v[4]));
    p.line_spacing = values.map(|v| Spacing::Pct(v[5]));
}

fn raw_pixels(owner: &EditablePptx, scale: f64) -> Vec<u8> {
    deckcraft_render::try_render_slide(
        owner.document().unwrap(),
        0,
        &deckcraft_render::RenderOpts {
            scale,
            ..Default::default()
        },
    )
    .unwrap()
    .pixels
}

#[test]
fn pptx_cell_format_raw_bounds_quantization_reset_and_inheritance() {
    let cases = [
        [0.0; 6],
        [4032.0, 4032.0, -4032.0, 1584.0, 1584.0, 100.0],
        [4032.0, 4032.0, 4032.0, 1584.0, 1584.0, 100.0],
        [12.0, 6.0, -6.0, 4.0, 5.0, 1.25],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.00001],
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.000001],
        [1.23456, 2.34567, -3.45678, 4.56789, 5.67891, 1.23456789],
        [-1.0, 4033.0, -4033.0, -1.0, 1585.0, 132.0],
    ];
    let mut checks = 0;
    for merged in 0..4 {
        let (mut original, id) = fixture(merged);
        let mut blank = NativeParagraph::default();
        blank.end_props.size = Some(17.0);
        cell_mut(original.presentation.as_mut().unwrap(), 0, id, 0, 0)
            .unwrap()
            .text
            .paragraphs
            .insert(1, blank);
        original.presentation =
            Some(deckcraft_pptx::import(&original.to_bytes().unwrap()).unwrap());
        let baseline = original.document().unwrap().clone();
        // Endpoints are serialized capabilities, not an assertion that enormous
        // offsets/line pitch fit a finite cell or preserve all visible glyphs.
        for values in [[4032.0, 4032.0, -4032.0, 1584.0, 1584.0, 100.0], [0.0; 6]] {
            let mut diagnostic = original.copy().unwrap();
            set_six(props(&mut diagnostic, id, 2), Some(values));
            let visible = bands(576, &raw_pixels(&diagnostic, 1.0), 1);
            eprintln!("spec121 layout diagnostic merged={merged} values={values:?} green_bands={visible:?}");
        }
        for values in cases {
            let mut changed = original.copy().unwrap();
            set_six(props(&mut changed, id, 2), Some(values));
            let current = changed.document().unwrap().clone();
            let reopened = deckcraft_pptx::import(&changed.to_bytes().unwrap()).unwrap();
            let mut expected = values;
            expected[0] = (values[0].clamp(0.0, 4032.0) * 12700.0).round() / 12700.0;
            expected[1] = (values[1].clamp(0.0, 4032.0) * 12700.0).round() / 12700.0;
            expected[2] = (values[2].clamp(-4032.0, 4032.0) * 12700.0).round() / 12700.0;
            expected[3] = (values[3].clamp(0.0, 1584.0) * 100.0).round() / 100.0;
            expected[4] = (values[4].clamp(0.0, 1584.0) * 100.0).round() / 100.0;
            expected[5] = ((values[5].clamp(0.0, 132.0) * 100000.0).round() / 100000.0).min(100.0);
            let actual = six(&cell_mut(&mut reopened.clone(), 0, id, 0, 0)
                .unwrap()
                .text
                .paragraphs[2]
                .props);
            for (a, e) in actual.into_iter().zip(expected) {
                assert!(
                    (a.unwrap() - e).abs() < 1e-10,
                    "raw {values:?}: expected {expected:?}, actual {actual:?}"
                );
            }
            let mut restored = reopened;
            let mut expected_model = current;
            set_six(
                &mut cell_mut(&mut expected_model, 0, id, 0, 0)
                    .unwrap()
                    .text
                    .paragraphs[2]
                    .props,
                Some(expected),
            );
            assert_eq!(
                restored, expected_model,
                "only specified native scalar quantization/clamps"
            );
            set_six(
                &mut cell_mut(&mut restored, 0, id, 0, 0)
                    .unwrap()
                    .text
                    .paragraphs[2]
                    .props,
                None,
            );
            assert_eq!(
                restored, baseline,
                "reset restores entire model, inherited props, authored blank, links and neighbors"
            );
            assert_eq!(
                original.document().unwrap(),
                &baseline,
                "source copy unchanged"
            );
            checks += 1;
        }
        let inherited = raw_pixels(&original, 1.0);
        let mut explicit_zero = original.copy().unwrap();
        set_six(
            props(&mut explicit_zero, id, 2),
            Some([0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
        );
        assert_ne!(
            inherited,
            raw_pixels(&explicit_zero, 1.0),
            "explicit zero overrides inheritance"
        );
        set_six(props(&mut explicit_zero, id, 2), None);
        assert_eq!(
            inherited,
            raw_pixels(&explicit_zero, 1.0),
            "None restores actual native inherited pixels"
        );
    }
    assert_eq!(checks, 32);
    pptx_cell_format_overlap_diagnostic();
    eprintln!("spec121 raw model: {checks} four-merge-profile scalar/endpoints/quantization cases; native reader clamps132 to100; zero distinct from None and inherited pixel reset");
}

fn decode(data: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(Cursor::new(data)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    pixels.truncate(info.buffer_size());
    assert_eq!(pixels.len(), info.width as usize * info.height as usize * 4);
    (info.width, info.height, pixels)
}

fn bands(width: u32, data: &[u8], color: usize) -> Vec<(usize, usize, usize, usize)> {
    let mut out: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (y, row) in data.chunks_exact(width as usize * 4).enumerate() {
        let xs: Vec<_> = row
            .chunks_exact(4)
            .enumerate()
            .filter_map(|(x, p)| {
                (p[color] > 90 && p[(color + 1) % 3] < 60 && p[(color + 2) % 3] < 60 && p[3] > 240)
                    .then_some(x)
            })
            .collect();
        if let (Some(left), Some(right)) = (xs.first(), xs.last()) {
            match out.last_mut() {
                Some((x0, _, x1, y1)) if *y1 + 1 == y => {
                    *x0 = (*x0).min(*left);
                    *x1 = (*x1).max(*right);
                    *y1 = y;
                }
                _ => out.push((*left, y, *right, y)),
            }
        }
    }
    out
}

#[test]
fn pptx_cell_format_raw_visible_geometry_first_last_and_output() {
    let mut output_cases = 0;
    for merged in 0..4 {
        let (mut owner, id) = fixture(merged);
        // Naturally wrapped text is reachable through the public cell paragraph
        // API; geometry comes from the actual native cell renderer.
        cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
            .unwrap()
            .text
            .paragraphs[1]
            .runs[0]
            .text = "M".repeat(48);
        for i in 0..3 {
            set_six(
                props(&mut owner, id, i),
                Some([0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
            );
        }
        if merged == 0 {
            // Preserve the initial thin-glyph carrier as a diagnostic: strict
            // per-row color thresholds can fragment a glyph at another DPI.
            // Its full200-grid comparison is independent of that measurement.
            for changed in [false, true] {
                let mut thin = owner.copy().unwrap();
                if changed {
                    set_six(
                        props(&mut thin, id, 1),
                        Some([24.0, 12.0, -12.0, 10.0, 9.0, 1.5]),
                    );
                }
                if changed {
                    let mut no_line = thin.copy().unwrap();
                    cell_mut(no_line.presentation.as_mut().unwrap(), 0, id, 0, 0)
                        .unwrap()
                        .text
                        .paragraphs[0]
                        .runs[0]
                        .props
                        .underline = Some("none".into());
                    let bytes = no_line.to_bytes().unwrap();
                    let pdf = crate::pptx::to_pdf(&bytes).unwrap();
                    let a = decode(&crate::pptx::render(&bytes, 0, 144.0).unwrap());
                    let b = decode(&crate::pdf::render(&pdf, 0, 144.0, None).unwrap());
                    let a = bands(a.0, &a.2, 0);
                    let b = bands(b.0, &b.2, 0);
                    assert_eq!(a.len(), 1);
                    assert_eq!(b.len(), 1);
                    for (x, y) in [a[0].0, a[0].1, a[0].2, a[0].3]
                        .into_iter()
                        .zip([b[0].0, b[0].1, b[0].2, b[0].3])
                    {
                        assert!(x.abs_diff(y) <= 2);
                    }
                    eprintln!("spec121 thin underline-off144 control native={a:?} pdf={b:?}");
                }
                let bytes = thin.to_bytes().unwrap();
                let pdf = crate::pptx::to_pdf(&bytes).unwrap();
                for dpi in [72.0, 144.0, 200.0] {
                    let a = decode(&crate::pptx::render(&bytes, 0, dpi).unwrap());
                    let b = decode(&crate::pdf::render(&pdf, 0, dpi, None).unwrap());
                    eprintln!("spec121 preserved thin diagnostic changed={changed} dpi={dpi} native={:?} pdf={:?}", (0..3).map(|c| bands(a.0, &a.2, c)).collect::<Vec<_>>(), (0..3).map(|c| bands(b.0, &b.2, c)).collect::<Vec<_>>());
                    assert_eq!((a.0, a.1), (b.0, b.1));
                    if dpi == 200.0 {
                        assert!(
                            a.2.iter().zip(&b.2).all(|(a, b)| a.abs_diff(*b) <= 1),
                            "thin control full200 grid"
                        );
                    }
                }
            }
        }
        for p in &mut cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
            .unwrap()
            .text
            .paragraphs
        {
            for run in &mut p.runs {
                run.props.bold = Some(true);
                run.props.underline = Some("none".into());
            }
        }
        owner.presentation = Some(deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap());
        let baseline_bytes = owner.to_bytes().unwrap();
        let baseline_pdf = crate::pptx::to_pdf(&baseline_bytes).unwrap();
        for dpi in [72.0, 144.0, 200.0] {
            let a = decode(&crate::pptx::render(&baseline_bytes, 0, dpi).unwrap());
            let b = decode(&crate::pdf::render(&baseline_pdf, 0, dpi, None).unwrap());
            assert_eq!((a.0, a.1, a.2.len()), (b.0, b.1, b.2.len()));
            eprintln!("spec121 unmodified raster diagnostic merged={merged} dpi={dpi} native={:?} pdf={:?}", bands(a.0, &a.2, 1), bands(b.0, &b.2, 1));
            if dpi == 200.0 {
                assert!(a.2.iter().zip(&b.2).all(|(a, b)| a.abs_diff(*b) <= 1));
            }
        }
        let baseline = raw_pixels(&owner, 1.0);
        let base = bands(576, &baseline, 1);
        assert!(base.len() >= 2, "naturally wrapped green lines: {base:?}");
        let base_blue = bands(576, &baseline, 2);
        assert_eq!(base_blue.len(), 1);
        let mut zero = owner.copy().unwrap();
        props(&mut zero, id, 1).line_spacing = Some(Spacing::Pct(0.0));
        let zero_pixels = raw_pixels(&zero, 1.0);
        let zero_green = bands(576, &zero_pixels, 1);
        let zero_blue = bands(576, &zero_pixels, 2);
        assert_ne!(zero_pixels, baseline);
        assert!(
            zero_green.len() < base.len(),
            "zero line advance overlays wrapped lines"
        );
        assert!(
            zero_blue[0].1 < base_blue[0].1,
            "zero line advance moves following paragraph upwards"
        );
        let reread = deckcraft_pptx::import(&zero.to_bytes().unwrap()).unwrap();
        assert_eq!(
            zero_pixels,
            deckcraft_render::try_render_slide(&reread, 0, &Default::default())
                .unwrap()
                .pixels
        );
        if merged == 0 {
            let bytes = zero.to_bytes().unwrap();
            let pdf = crate::pptx::to_pdf(&bytes).unwrap();
            let model = crate::pdf::extract(&pdf, None).unwrap();
            assert!(model.pages[0]
                .links
                .iter()
                .any(|link| link.uri == "https://example.com/retained"));
            let text = model.pages[0]
                .blocks
                .iter()
                .map(|block| match &block.table {
                    Some(table) => table
                        .rows
                        .iter()
                        .map(|row| row.join("\n"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None => block.text.clone(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            let count = text.chars().filter(|c| *c == 'M').count();
            eprintln!("spec121 unsupported bold-zero overlap: native_source_M=48 parsed_pdf_M={count}; local20 and CI31 are observations, not a retained-count contract");
            assert!(
                (1..48).contains(&count),
                "unsupported zero-spacing diagnostic must retain text but lose overlapping repeats: {count}"
            );
            let a = decode(&crate::pptx::render(&bytes, 0, 200.0).unwrap());
            let b = decode(&crate::pdf::render(&pdf, 0, 200.0, None).unwrap());
            assert_eq!((a.0, a.1, a.2.len()), (b.0, b.1, b.2.len()));
            assert!(
                a.2.iter().zip(&b.2).all(|(a, b)| a.abs_diff(*b) <= 1),
                "zero native/PDF same200 grid"
            );
            for value in [0.00001, 0.000001] {
                let mut tiny = zero.copy().unwrap();
                props(&mut tiny, id, 1).line_spacing = Some(Spacing::Pct(value));
                let reopened = deckcraft_pptx::import(&tiny.to_bytes().unwrap()).unwrap();
                let value_read = cell_mut(&mut reopened.clone(), 0, id, 0, 0)
                    .unwrap()
                    .text
                    .paragraphs[1]
                    .props
                    .line_spacing;
                let expected = if value == 0.00001 { value } else { 0.0 };
                assert_eq!(value_read, Some(Spacing::Pct(expected)));
                eprintln!("spec121 sub-resolution control supplied={value} readback={expected}; both are outside the admitted normal-height multiplier profile");
            }
        }
        props(&mut zero, id, 1).line_spacing = None;
        let inherited_green = bands(576, &raw_pixels(&zero, 1.0), 1);
        assert!(inherited_green.len() >= 2);
        assert!(
            inherited_green[1].1 - inherited_green[0].1 > base[1].1 - base[0].1,
            "None resolves inherited1.25 pitch instead of zero"
        );
        let mut negative = owner.copy().unwrap();
        props(&mut negative, id, 1).indent = Some(-4032.0);
        let negative_pixels = raw_pixels(&negative, 1.0);
        assert_eq!(
            negative_pixels, baseline,
            "native pins negative effective first origin tocell edge"
        );
        assert!(bands(576, &negative_pixels, 1)
            .iter()
            .all(|band| band.0 >= 24));
        eprintln!("spec121 semantic controls merged={merged} ordinary={base:?} zero={zero_green:?} zero_next={zero_blue:?} inherited={inherited_green:?}; first_indent=-4032 atleft0 is pixel-identical");
        let mut right = owner.copy().unwrap();
        cell_mut(right.presentation.as_mut().unwrap(), 0, id, 0, 0)
            .unwrap()
            .text
            .paragraphs[1]
            .runs[0]
            .text = "M".repeat(48);
        let no_margin = bands(576, &raw_pixels(&right, 1.0), 1);
        props(&mut right, id, 1).margin_right = Some(if merged == 1 || merged == 3 {
            240.0
        } else {
            120.0
        });
        let narrow = bands(576, &raw_pixels(&right, 1.0), 1);
        assert!(
            narrow.len() > no_margin.len(),
            "native right margin must narrow wrapping width: {no_margin:?}/{narrow:?}"
        );
        let mut before_only = owner.copy().unwrap();
        props(&mut before_only, id, 1).space_before = Some(Spacing::Pts(10.0));
        let before_bands = bands(576, &raw_pixels(&before_only, 1.0), 1);
        assert_eq!(before_bands[0].1, base[0].1 + 10);
        let mut after_only = owner.copy().unwrap();
        props(&mut after_only, id, 1).space_after = Some(Spacing::Pts(9.0));
        assert_eq!(bands(576, &raw_pixels(&after_only, 1.0), 1), base);
        assert_eq!(
            bands(576, &raw_pixels(&after_only, 1.0), 2)[0].1,
            base_blue[0].1 + 9
        );
        // First-before and last-after are ignored by native default body policy.
        props(&mut owner, id, 0).space_before = Some(Spacing::Pts(17.0));
        props(&mut owner, id, 2).space_after = Some(Spacing::Pts(19.0));
        assert_eq!(baseline, raw_pixels(&owner, 1.0));
        cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
            .unwrap()
            .text
            .body
            .space_first_last = Some(true);
        let explicit = bands(576, &raw_pixels(&owner, 1.0), 0);
        let first = bands(576, &baseline, 0);
        assert_eq!(explicit[0].1, first[0].1 + 17);
        cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
            .unwrap()
            .text
            .body
            .space_first_last = Some(false);
        set_six(
            props(&mut owner, id, 1),
            Some([24.0, 12.0, -12.0, 10.0, 9.0, 1.5]),
        );
        let changed_pixels = raw_pixels(&owner, 1.0);
        let changed = bands(576, &changed_pixels, 1);
        assert!(changed.len() >= 2);
        assert_eq!(changed[0].0, base[0].0 + 12, "hanging first-line origin");
        assert_eq!(changed[1].0, base[1].0 + 24, "continuation margin");
        assert!(changed[0].1 >= base[0].1 + 10, "before spacing contributes");
        assert!(
            changed[1].1 - changed[0].1 > base[1].1 - base[0].1,
            "native multiplier increases line pitch"
        );
        let blue = bands(576, &changed_pixels, 2);
        assert!(
            blue[0].1 > base_blue[0].1 + 19,
            "multiplier and before/after move later paragraph"
        );
        let bytes = owner.to_bytes().unwrap();
        let reopened = deckcraft_pptx::import(&bytes).unwrap();
        assert_eq!(&reopened, owner.document().unwrap());
        let pdf = crate::pptx::to_pdf(&bytes).unwrap();
        let extracted_pdf = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted_pdf.pages.len(), 1);
        assert!(extracted_pdf.pages[0]
            .links
            .iter()
            .any(|link| link.uri == "https://example.com/retained"));
        let text = extracted_pdf.pages[0]
            .blocks
            .iter()
            .map(|block| match &block.table {
                Some(table) => table
                    .rows
                    .iter()
                    .map(|row| row.join("\n"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => block.text.clone(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            text.chars().filter(|c| *c == 'M').count(),
            48,
            "actual PDF text preserved once"
        );
        for dpi in [72.0, 144.0, 200.0] {
            let source = deckcraft_render::try_render_slide(
                owner.document().unwrap(),
                0,
                &deckcraft_render::RenderOpts {
                    scale: dpi / 72.0,
                    ..Default::default()
                },
            )
            .unwrap();
            let read = deckcraft_render::try_render_slide(
                &reopened,
                0,
                &deckcraft_render::RenderOpts {
                    scale: dpi / 72.0,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(source.pixels, read.pixels, "exact source/readback pixels");
            let png = decode(&crate::pdf::render(&pdf, 0, dpi, None).unwrap());
            assert_eq!((source.width, source.height), (png.0, png.1));
            assert_eq!(source.pixels.len(), png.2.len());
            for color in 0..3 {
                let a = bands(source.width, &source.pixels, color);
                let b = bands(png.0, &png.2, color);
                assert_eq!(a.len(), if color == 1 { changed.len() } else { 1 });
                assert_eq!(a.len(), b.len());
                for (a, b) in a.iter().zip(&b) {
                    for (x, y) in [a.0, a.1, a.2, a.3].into_iter().zip([b.0, b.1, b.2, b.3]) {
                        assert!(x.abs_diff(y) <= 2, "native/PDF visible band merged={merged} dpi={dpi} color={color}: {a:?}/{b:?}");
                    }
                }
            }
            if dpi == 200.0 {
                assert!(source
                    .pixels
                    .iter()
                    .zip(&png.2)
                    .all(|(a, b)| a.abs_diff(*b) <= 1));
            }
        }
        let extracted = crate::pptx::extract(&bytes).unwrap();
        assert_eq!(extracted.pages[0].links.len(), 1);
        assert!(extracted.pages[0].tables[0].rows[0][0].contains(&"M".repeat(48)));
        output_cases += 1;
    }
    assert_eq!(output_cases, 4);
    eprintln!("spec121 raw output: four merged profiles, actual first/continuation x, before/after y, line pitch, first-last body policy, exact source/readback and72/144/200 native/PDF glyph bands; same-grid200<=1 channel");
}

// Unsupported overlapping-layout diagnostic, retained within the raw model gate.
fn pptx_cell_format_overlap_diagnostic() {
    fn text_evidence(bytes: &[u8]) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        use pdfcraft_cos::Object;
        let doc = pdfcraft_cos::Document::open(std::sync::Arc::new(bytes.to_vec())).unwrap();
        let pages = pdfcraft_model::pages(&doc);
        assert_eq!(pages.len(), 1);
        let mut contents = Vec::new();
        let content = doc.resolve(pages[0].dict.get(b"Contents").unwrap());
        match content.as_ref() {
            Object::Stream(stream) => contents.push(stream.decoded_within(1024 * 1024).unwrap()),
            Object::Array(items) => {
                for item in items {
                    let object = doc.resolve(item);
                    let Object::Stream(stream) = object.as_ref() else {
                        panic!("content stream")
                    };
                    contents.push(stream.decoded_within(1024 * 1024).unwrap());
                }
            }
            _ => panic!("page contents"),
        }
        let mut maps = Vec::new();
        let objects: Vec<_> = doc.scan_objects().collect();
        assert!(objects.len() < 1000, "bounded synthetic fixture");
        for (_, object) in objects {
            if let Object::Stream(stream) = object.as_ref() {
                if stream.dict.get(b"Subtype").and_then(Object::as_name) == Some(b"Form") {
                    contents.push(stream.decoded_within(1024 * 1024).unwrap());
                }
            }
            if let Some(dict) = object.as_dict() {
                if let Some(cmap) = dict.get(b"ToUnicode") {
                    let cmap = doc.resolve(cmap);
                    let Object::Stream(cmap) = cmap.as_ref() else {
                        panic!("ToUnicode")
                    };
                    maps.push(cmap.decoded_within(1024 * 1024).unwrap());
                }
            }
        }
        let mut showing = Vec::new();
        for stream in contents {
            let parsed = pdfcraft_content::parse(&stream);
            assert_eq!(parsed.skipped, 0);
            for op in parsed.ops {
                if ["Tj", "TJ", "'", "\""].iter().any(|name| op.is(name)) {
                    showing.push(pdfcraft_content::serialize_ops(&[op]));
                }
            }
        }
        assert!(!showing.is_empty());
        assert!(!maps.is_empty());
        showing.sort();
        maps.sort();
        (showing, maps)
    }
    fn m_count(model: &crate::model::DocumentPayload) -> usize {
        model.pages[0]
            .blocks
            .iter()
            .map(|b| match &b.table {
                Some(t) => t
                    .rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .map(|v| v.chars().filter(|c| *c == 'M').count())
                    .sum::<usize>(),
                None => b.text.chars().filter(|c| *c == 'M').count(),
            })
            .sum()
    }
    let (mut owner, id) = fixture(0);
    cell_mut(owner.presentation.as_mut().unwrap(), 0, id, 0, 0)
        .unwrap()
        .text
        .paragraphs[1]
        .runs[0]
        .text = "M".repeat(48);
    for i in 0..3 {
        set_six(
            props(&mut owner, id, i),
            Some([0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
        );
    }
    let baseline = crate::pptx::to_pdf(&owner.to_bytes().unwrap()).unwrap();
    let baseline_evidence = text_evidence(&baseline);
    assert_eq!(m_count(&crate::pdf::extract(&baseline, None).unwrap()), 48);
    for value in [0.0, 0.000001, 0.00001, 0.06, 0.5, 0.99, 1.0, 1.5] {
        let mut variant = owner.copy().unwrap();
        props(&mut variant, id, 1).line_spacing = Some(Spacing::Pct(value));
        let bytes = variant.to_bytes().unwrap();
        let read = deckcraft_pptx::import(&bytes).unwrap();
        let placed = deckcraft_render::place_slide(&read, 0);
        let text = placed
            .texts
            .iter()
            .find(|t| t.body.paragraphs.iter().any(|p| p.text() == "M".repeat(48)))
            .unwrap();
        let lines: Vec<_> = text
            .layout
            .lines
            .iter()
            .filter(|l| l.para == 1)
            .map(|l| (l.top, l.baseline, l.bottom, l.extent))
            .collect();
        let glyphs: usize = text
            .layout
            .runs
            .iter()
            .filter(|r| r.para == 1)
            .map(|r| r.glyphs.len())
            .sum();
        assert_eq!(glyphs, 48);
        let pdf = crate::pptx::to_pdf(&bytes).unwrap();
        let evidence = text_evidence(&pdf);
        assert_eq!(
            evidence, baseline_evidence,
            "native text-showing code strings/operators and ToUnicode maps unchanged at{value}"
        );
        let model = crate::pdf::extract(&pdf, None).unwrap();
        let count = m_count(&model);
        eprintln!("spec121 overlap DIAGNOSTIC multiplier={value} native_glyphs={glyphs} parsed_pdf_M={count} text_ops_and_cmaps_identical=true lines={lines:?}");
        if value <= 0.06 {
            // Local16 and CI31 both demonstrate loss. Exact survivor counts are
            // observations for unsupported overlap, not a public text contract.
            assert!(
                (1..48).contains(&count),
                "unsupported overlap must retain text but lose repetitions: {count}"
            );
        } else {
            assert_eq!(
                count, 48,
                "this retained-character control keeps every character"
            );
        }
        let a = decode(&crate::pptx::render(&bytes, 0, 200.0).unwrap());
        let b = decode(&crate::pdf::render(&pdf, 0, 200.0, None).unwrap());
        assert_eq!((a.0, a.1, a.2.len()), (b.0, b.1, b.2.len()));
        assert!(a.2.iter().zip(&b.2).all(|(a, b)| a.abs_diff(*b) <= 1));
    }
}

fn target(owner: &EditablePptx, id: u32, paragraph: usize) -> CellTarget {
    let (row, col, grid, body, _) = owner.cell_paragraph_info(0, id, 0, 0).unwrap();
    (row, col, paragraph, grid, body)
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
fn pptx_cell_format_owned_bounds_snapshot_reset_and_model_preservation() {
    for merged in 0..4 {
        let (mut owner, id) = fixture(merged);
        let cell = target(&owner, id, 1);
        let original = owner.clone();
        let direct: serde_json::Value =
            serde_json::from_str(&owner.cell_paragraph_formatting(0, id, cell).unwrap()).unwrap();
        assert_eq!(direct["line_spacing"], serde_json::Value::Null);
        assert_eq!(direct["space_before"], serde_json::Value::Null);
        for patch in [
            r#"{"indent_left":0,"indent_right":0,"indent_first":0,"space_before":0,"space_after":0,"line_spacing":1,"alignment":"left"}"#,
            r#"{"indent_left":4032,"indent_right":4032,"indent_first":-4032,"space_before":1584,"space_after":1584,"line_spacing":100,"alignment":"right"}"#,
            r#"{"indent_first":4032,"line_spacing":1.23456789,"space_after":5.67891}"#,
        ] {
            let before = owner.clone();
            owner
                .format_cell_paragraph(0, id, cell, patch, owner.revision)
                .unwrap();
            let snapshot: serde_json::Value =
                serde_json::from_str(&owner.cell_paragraph_formatting(0, id, cell).unwrap())
                    .unwrap();
            let supplied: serde_json::Value = serde_json::from_str(patch).unwrap();
            for (key, value) in supplied.as_object().unwrap() {
                assert_eq!(snapshot[key].as_f64(), value.as_f64());
                if key == "alignment" {
                    assert_eq!(snapshot[key], *value);
                }
            }
            let mut expected = before.document().unwrap().clone();
            cell_mut(&mut expected, 0, id, 0, 0)
                .unwrap()
                .text
                .paragraphs[1]
                .props = owner.target_body(0, id, Some(cell)).unwrap().paragraphs[1]
                .props
                .clone();
            assert_eq!(*owner.document().unwrap(), expected);
            assert_eq!(owner.revision, before.revision + 1);
            assert_eq!(owner.table_generations, before.table_generations);
            assert_eq!(owner.cell_generations, before.cell_generations);
            assert_eq!(
                (owner.text_bytes, owner.media_bytes, owner.shapes),
                (before.text_bytes, before.media_bytes, before.shapes)
            );
            let reread = deckcraft_pptx::import(&owner.to_bytes().unwrap()).unwrap();
            assert!(reread.validate().is_empty());
        }
        owner.format_cell_paragraph(0, id, cell,
            r#"{"indent_left":null,"indent_right":null,"indent_first":null,"space_before":null,"space_after":null,"line_spacing":null,"alignment":"left"}"#,
            owner.revision).unwrap();
        assert_eq!(owner.document().unwrap(), original.document().unwrap());
        assert_eq!(raw_pixels(&owner, 1.0), raw_pixels(&original, 1.0));
        assert_eq!(
            owner.cell_paragraph_formatting(0, id, cell).unwrap(),
            original.cell_paragraph_formatting(0, id, cell).unwrap()
        );
    }
}

#[test]
fn pptx_cell_format_owned_invalid_compounds_targets_and_revision_are_atomic() {
    let (mut owner, id) = fixture(0);
    let cell = target(&owner, id, 1);
    for patch in [
        r#"{"alignment":"center","line_spacing":0}"#,
        r#"{"line_spacing":0.9999999999999999}"#,
        r#"{"line_spacing":100.00001}"#,
        r#"{"line_spacing":true}"#,
        r#"{"line_spacing":"1.5"}"#,
        r#"{"line_spacing":1e999}"#,
        r#"{"indent_left":-0.00001}"#,
        r#"{"indent_right":4032.00001}"#,
        r#"{"indent_first":-4032.00001}"#,
        r#"{"indent_first":4032.00001}"#,
        r#"{"space_before":-0.01}"#,
        r#"{"space_after":1584.01}"#,
        r#"{"alignment":"distributed","space_after":8}"#,
        r#"{"space_after":8,"keep_with_next":true}"#,
        r#"{"space_after":[]}"#,
        "[]",
    ] {
        let before = owner.clone();
        assert!(
            owner
                .format_cell_paragraph(0, id, cell, patch, owner.revision)
                .is_err(),
            "{patch}"
        );
        unchanged(&owner, &before);
    }
    for bad in [
        (0, 0, 99, cell.3, cell.4),
        (0, 0, 1, cell.3 + 1, cell.4),
        (0, 0, 1, cell.3, cell.4 + 1),
    ] {
        let before = owner.clone();
        assert!(owner
            .format_cell_paragraph(0, id, bad, "{}", owner.revision)
            .is_err());
        assert!(owner.cell_paragraph_formatting(0, id, bad).is_err());
        unchanged(&owner, &before);
    }
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, cell, "{}", owner.revision - 1)
        .unwrap_err()
        .to_string()
        .contains("stale"));
    unchanged(&owner, &before);
    owner.merge_cells(0, id, 0, 0, 0, 1).unwrap();
    let current = target(&owner, id, 1);
    let covered = (0, 1, 1, current.3, current.4);
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, covered, "{}", owner.revision)
        .unwrap_err()
        .to_string()
        .contains("canonical cell anchor"));
    unchanged(&owner, &before);
    owner.revision = u64::MAX - 1;
    owner.cell_generations.insert((id, 0, 0), u64::MAX);
    owner.table_generations.insert(id, u64::MAX);
    let current = target(&owner, id, 1);
    owner
        .format_cell_paragraph(0, id, current, r#"{"line_spacing":1}"#, owner.revision)
        .unwrap();
    assert_eq!(owner.revision, u64::MAX);
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, current, r#"{"line_spacing":2}"#, owner.revision)
        .unwrap_err()
        .to_string()
        .contains("revision limit"));
    unchanged(&owner, &before);
}

#[test]
fn pptx_cell_format_owned_patch_property_budget_and_native_units() {
    let (mut owner, id) = fixture(0);
    let cell = target(&owner, id, 1);
    let patch = format!(
        "{{\"space_after\":8}}{}",
        " ".repeat(MAX_PATCH - r#"{"space_after":8}"#.len())
    );
    assert_eq!(patch.len(), MAX_PATCH);
    owner
        .format_cell_paragraph(0, id, cell, &patch, owner.revision)
        .unwrap();
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, cell, &(patch + " "), owner.revision)
        .unwrap_err()
        .to_string()
        .contains("16 KiB"));
    unchanged(&owner, &before);
    let cost = content_budget(owner.document().unwrap())
        .unwrap()
        .properties;
    Arc::make_mut(&mut owner.presentation.as_mut().unwrap().slides[0])
        .notes
        .paragraphs[0]
        .def_props
        .font = Some("x".repeat(MAX_PROPERTIES - cost));
    assert_eq!(
        content_budget(owner.document().unwrap())
            .unwrap()
            .properties,
        MAX_PROPERTIES
    );
    owner
        .format_cell_paragraph(0, id, cell, r#"{"line_spacing":1.5}"#, owner.revision)
        .unwrap();
    Arc::make_mut(&mut owner.presentation.as_mut().unwrap().slides[0])
        .notes
        .paragraphs[0]
        .def_props
        .font
        .as_mut()
        .unwrap()
        .push('x');
    let before = owner.clone();
    assert!(owner
        .format_cell_paragraph(0, id, cell, r#"{"line_spacing":2}"#, owner.revision)
        .unwrap_err()
        .to_string()
        .contains("formatting/link properties"));
    unchanged(&owner, &before);
    for (before, after, line) in [
        (Some(Spacing::Pct(1.0)), None, None),
        (None, Some(Spacing::Pct(1.0)), None),
        (None, None, Some(Spacing::Pts(18.0))),
    ] {
        let (mut owner, id) = fixture(0);
        let cell = target(&owner, id, 1);
        let p = props(&mut owner, id, 1);
        p.space_before = before;
        p.space_after = after;
        p.line_spacing = line;
        assert!(owner
            .cell_paragraph_formatting(0, id, cell)
            .unwrap_err()
            .to_string()
            .contains("unsupported native unit"));
    }
}
