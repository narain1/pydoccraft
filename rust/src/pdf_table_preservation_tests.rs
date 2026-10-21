//! Original deterministic PDF fixtures for conservative table admission.
use super::*;

fn fixture(extra: Option<(f64, f64)>, rotation: u16) -> Vec<u8> {
    let mut content = String::new();
    for (text, x, y) in [
        ("LEFTTOP", 50., 340.),
        ("RIGHTTOP", 300., 340.),
        ("LEFTBOTTOM", 50., 180.),
        ("RIGHTBOTTOM", 300., 180.),
    ] {
        content.push_str(&format!("BT /F1 12 Tf 1 0 0 1 {x} {y} Tm ({text}) Tj ET\n"));
    }
    if let Some((x, y)) = extra {
        content.push_str(&format!(
            "BT /F1 12 Tf 1 0 0 1 {x} {y} Tm (EMERALD) Tj ET\n"
        ));
    }
    // Handwritten serialization is test data only; all processing uses selected native APIs.
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 480 420] /CropBox [10 20 470 400] /Rotate {rotation} /UserUnit 1.25 /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.into_iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

fn original(
    bytes: &[u8],
) -> (
    pdfcraft_cos::Document,
    pdfcraft_render::PageInfo,
    PagePayload,
) {
    let data = Arc::new(bytes.to_vec());
    let cos = pdfcraft_cos::Document::open(data.clone()).unwrap();
    let info = inspect(data.clone(), None).unwrap().pages.remove(0);
    let mut renderer = PageRenderer::new(data, RenderConfig::default());
    let rendered = renderer.render(RenderRequest {
        page: 0,
        kind: RequestKind::Text,
        scale: 1.,
        ..Default::default()
    });
    assert!(rendered.error.is_none());
    let mut page = PagePayload::default();
    add_text(rendered.text.as_ref().unwrap(), &mut page);
    (cos, info, page)
}
fn labels(page: &PagePayload) -> String {
    page.blocks
        .iter()
        .map(|b| {
            b.table.as_ref().map_or_else(
                || b.text.clone(),
                |t| {
                    t.rows
                        .iter()
                        .map(|r| r.join(" "))
                        .collect::<Vec<_>>()
                        .join("\n")
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn table_admission_preserves_interior_partial_and_extended_source_blocks() {
    for rotation in [0, 90, 180, 270] {
        for point in [(50., 260.), (20., 260.)] {
            let bytes = fixture(Some(point), rotation);
            let (cos, info, mut page) = original(&bytes);
            assert_eq!(labels(&page).matches("EMERALD").count(), 1);
            let before = serde_json::to_value(&page).unwrap();
            add_table(&cos, &info, 0, &mut page).unwrap();
            assert_eq!(
                serde_json::to_value(&page).unwrap(),
                before,
                "refused candidate must retain original blocks: {point:?}/{rotation}"
            );
            let extracted = extract(&bytes, None).unwrap();
            assert!(extracted.pages[0].tables.is_empty());
            for label in [
                "LEFTTOP",
                "RIGHTTOP",
                "LEFTBOTTOM",
                "RIGHTBOTTOM",
                "EMERALD",
            ] {
                assert_eq!(labels(&extracted.pages[0]).matches(label).count(), 1);
            }
        }
    }
    // A source block that would be suppressed can extend outside the candidate table.
    // Its full bounds must participate; this synthetic state isolates the admission rule.
    let bytes = fixture(Some((30., 100.)), 0);
    let (cos, info, mut page) = original(&bytes);
    for kind in ["paragraph", "image", "heading"] {
        for bbox in [
            [25., 50., 450., 400.],
            [450., 50., 25., 400.],
            [200., 50., 200., 400.],
        ] {
            page.blocks = vec![BlockPayload {
                kind: kind.into(),
                text: "Retain whole native block".into(),
                bbox: Some(bbox),
                ..Default::default()
            }];
            let before = serde_json::to_value(&page).unwrap();
            add_table(&cos, &info, 0, &mut page).unwrap();
            assert_eq!(serde_json::to_value(&page).unwrap(), before);
        }
    }
}

#[test]
fn table_admission_keeps_genuine_tables_and_disjoint_text_without_duplication() {
    for rotation in [0, 90, 180, 270] {
        for extra in [None, Some((50., 100.))] {
            let bytes = fixture(extra, rotation);
            let extracted = extract(&bytes, None).unwrap();
            let page = &extracted.pages[0];
            assert_eq!(
                page.tables.len(),
                1,
                "genuine table control {rotation}/{extra:?}"
            );
            assert_eq!(
                page.tables[0].rows,
                vec![
                    vec!["LEFTTOP", "RIGHTTOP"],
                    vec!["LEFTBOTTOM", "RIGHTBOTTOM"]
                ]
            );
            assert_eq!(page.blocks.iter().filter(|b| b.kind == "table").count(), 1);
            let text = labels(page);
            for label in ["LEFTTOP", "RIGHTTOP", "LEFTBOTTOM", "RIGHTBOTTOM"] {
                assert_eq!(text.matches(label).count(), 1);
            }
            assert_eq!(
                text.matches("EMERALD").count(),
                usize::from(extra.is_some())
            );
            let (w, h) = if rotation % 180 == 0 {
                (575, 475)
            } else {
                (475, 575)
            };
            let png = render(&bytes, 0, 72., None).unwrap();
            let decoder = png::Decoder::new(std::io::Cursor::new(png));
            let reader = decoder.read_info().unwrap();
            assert_eq!((reader.info().width, reader.info().height), (w, h));
        }
    }
}
