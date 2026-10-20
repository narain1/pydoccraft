use std::{collections::BTreeMap, io::Cursor, sync::Arc};

use base64::Engine;
use pdfcraft_render::{
    inspect, LinkTarget, PageRenderer, RenderConfig, RenderRequest, RequestKind,
};
use serde_json::{json, Map, Value};

use crate::model::{
    BlockPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload, TablePayload,
};
use crate::EngineError;

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    let bytes = Arc::new(data.to_vec());
    let info = inspect(bytes.clone(), password).map_err(open_error)?;
    if info.pages.is_empty() {
        return Err(EngineError::Malformed("PDF contains no pages".into()));
    }
    let cos = pdfcraft_cos::Document::open_with_password(bytes.clone(), password)
        .map_err(|e| EngineError::Malformed(format!("invalid PDF object graph: {e}")))?;
    let mut renderer = PageRenderer::new(
        bytes,
        RenderConfig {
            password: password.map(Arc::from),
            ..Default::default()
        },
    );
    let mut pages = Vec::with_capacity(info.pages.len());
    for (number, page_info) in info.pages.iter().enumerate() {
        let rendered = renderer.render(RenderRequest {
            page: number,
            kind: RequestKind::Text,
            scale: 1.0,
            ..Default::default()
        });
        if let Some(e) = rendered.error {
            return Err(EngineError::Malformed(format!(
                "PDF text extraction failed: {e}"
            )));
        }
        let mut page = PagePayload {
            number,
            metadata: Map::from_iter([
                ("width".into(), json!(page_info.width)),
                ("height".into(), json!(page_info.height)),
                ("rotation".into(), json!(page_info.rotation)),
                ("label".into(), json!(page_info.label)),
            ]),
            ..Default::default()
        };
        if let Some(text) = rendered.text {
            add_text(&text, &mut page);
        }
        add_table(&cos, page_info, number, &mut page)?;
        add_links(&info.links, page_info, number, &mut page);
        add_images(&cos, page_info, number, &mut page)?;
        pages.push(page);
    }
    let mut metadata = Map::new();
    for (key, value) in [
        ("title", info.title),
        ("author", info.author),
        ("subject", info.subject),
        ("keywords", info.keywords),
        ("creator", info.creator),
        ("producer", info.producer),
    ] {
        if let Some(value) = value {
            metadata.insert(key.into(), Value::String(value));
        }
    }
    metadata.insert("pdf_version".into(), json!(info.pdf_version));
    metadata.insert("encrypted".into(), json!(info.encrypted));
    metadata.insert("tagged".into(), json!(info.tagged));
    Ok(DocumentPayload {
        format: "pdf".into(),
        metadata,
        pages,
    })
}

pub fn render(
    data: &[u8],
    page: usize,
    dpi: f64,
    password: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    if !dpi.is_finite() || !(1.0..=1200.0).contains(&dpi) {
        return Err(EngineError::ResourceLimit(
            "DPI must be finite and between 1 and 1200".into(),
        ));
    }
    let bytes = Arc::new(data.to_vec());
    let info = inspect(bytes.clone(), password).map_err(open_error)?;
    if page >= info.pages.len() {
        return Err(EngineError::Unsupported(format!(
            "page {page} does not exist"
        )));
    }
    let mut renderer = PageRenderer::new(
        bytes,
        RenderConfig {
            password: password.map(Arc::from),
            reject_oversize: true,
            ..Default::default()
        },
    );
    let rendered = renderer.render(RenderRequest {
        page,
        kind: RequestKind::Pixels,
        scale: (dpi / 72.0) as f32,
        ..Default::default()
    });
    if let Some(e) = rendered.error {
        return Err(EngineError::ResourceLimit(format!(
            "PDF rendering failed: {e}"
        )));
    }
    encode_png(rendered.width, rendered.height, rendered.rgba.as_ref())
}

fn add_text(text: &pdfcraft_render::PageText, page: &mut PagePayload) {
    let mut lines: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (i, line) in text.line_of.iter().copied().enumerate() {
        lines.entry(line).or_default().push(i);
    }
    for indexes in lines.values() {
        let Some((&start, &end)) = indexes.first().zip(indexes.last()) else {
            continue;
        };
        let value = text.text_of(start..end + 1);
        if value.trim().is_empty() {
            continue;
        }
        let mut bbox: Option<[f64; 4]> = None;
        for glyph in indexes.iter().filter_map(|i| text.glyphs.get(*i)) {
            bbox = Some(match bbox {
                None => glyph.rect.map(f64::from),
                Some([x0, y0, x1, y1]) => [
                    x0.min(glyph.rect[0].into()),
                    y0.min(glyph.rect[1].into()),
                    x1.max(glyph.rect[2].into()),
                    y1.max(glyph.rect[3].into()),
                ],
            });
        }
        page.blocks.push(BlockPayload {
            kind: "paragraph".into(),
            text: value,
            bbox,
            ..Default::default()
        });
    }
}

fn add_table(
    cos: &pdfcraft_cos::Document,
    info: &pdfcraft_render::PageInfo,
    number: usize,
    page: &mut PagePayload,
) -> Result<(), EngineError> {
    let mut blocks = pdfcraft_edit::reading_blocks(cos, number)
        .map_err(|e| EngineError::Malformed(format!("PDF table extraction failed: {e}")))?;
    blocks.retain(|block| !block.text.trim().is_empty());
    blocks.sort_by(|left, right| {
        right.rect[1]
            .total_cmp(&left.rect[1])
            .then(left.rect[0].total_cmp(&right.rect[0]))
    });
    type PositionedText = (f64, String, [f64; 4]);

    let mut rows: Vec<(f64, Vec<PositionedText>)> = Vec::new();
    for block in blocks {
        if let Some((_, row)) = rows
            .iter_mut()
            .find(|(y, _)| (*y - block.rect[1]).abs() <= 3.0)
        {
            row.push((block.rect[0], block.text, block.rect));
        } else {
            rows.push((block.rect[1], vec![(block.rect[0], block.text, block.rect)]));
        }
    }
    let mut rows: Vec<Vec<PositionedText>> = rows
        .into_iter()
        .map(|(_, mut row)| {
            row.sort_by(|a, b| a.0.total_cmp(&b.0));
            row
        })
        .filter(|row| row.len() >= 2)
        .collect();
    if rows.len() < 2 || rows.iter().any(|row| row.len() != rows[0].len()) {
        return Ok(());
    }
    let width = rows[0].len();
    if (0..width).any(|column| {
        rows.iter()
            .any(|row| (row[column].0 - rows[0][column].0).abs() > 6.0)
    }) {
        return Ok(());
    }

    let user_bbox = rows.iter().flatten().map(|(_, _, rect)| *rect).fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |[x0, y0, x1, y1], rect| {
            [
                x0.min(rect[0]),
                y0.min(rect[1]),
                x1.max(rect[2]),
                y1.max(rect[3]),
            ]
        },
    );
    let a = info.user_to_view(user_bbox[0] as f32, user_bbox[1] as f32);
    let b = info.user_to_view(user_bbox[2] as f32, user_bbox[3] as f32);
    let table_bbox = [
        f64::from(a[0].min(b[0])),
        f64::from(a[1].min(b[1])),
        f64::from(a[0].max(b[0])),
        f64::from(a[1].max(b[1])),
    ];
    let table = TablePayload {
        rows: rows
            .drain(..)
            .map(|row| row.into_iter().map(|(_, text, _)| text).collect())
            .collect(),
        bbox: Some(table_bbox),
        page: Some(number),
        ..Default::default()
    };

    let mut insertion = None;
    let mut retained = Vec::with_capacity(page.blocks.len());
    for block in page.blocks.drain(..) {
        let inside_table = block.bbox.is_some_and(|bbox| {
            let center_x = (bbox[0] + bbox[2]) / 2.0;
            let center_y = (bbox[1] + bbox[3]) / 2.0;
            center_x >= table_bbox[0]
                && center_x <= table_bbox[2]
                && center_y >= table_bbox[1]
                && center_y <= table_bbox[3]
        });
        if inside_table {
            insertion.get_or_insert(retained.len());
        } else {
            retained.push(block);
        }
    }
    let insertion = insertion.unwrap_or(retained.len());
    retained.insert(
        insertion,
        BlockPayload {
            kind: "table".into(),
            bbox: Some(table_bbox),
            table: Some(table.clone()),
            ..Default::default()
        },
    );
    page.blocks = retained;
    page.tables.push(table);
    Ok(())
}

fn add_links(
    links: &[pdfcraft_render::Link],
    info: &pdfcraft_render::PageInfo,
    number: usize,
    page: &mut PagePayload,
) {
    for link in links.iter().filter(|link| link.page == number) {
        if let LinkTarget::Uri(uri) = &link.target {
            let a = info.user_to_view(link.rect[0], link.rect[1]);
            let b = info.user_to_view(link.rect[2], link.rect[3]);
            page.links.push(LinkPayload {
                uri: uri.clone(),
                bbox: Some([
                    a[0].min(b[0]).into(),
                    a[1].min(b[1]).into(),
                    a[0].max(b[0]).into(),
                    a[1].max(b[1]).into(),
                ]),
                page: Some(number),
                ..Default::default()
            });
        }
    }
}

fn add_images(
    cos: &pdfcraft_cos::Document,
    info: &pdfcraft_render::PageInfo,
    number: usize,
    page: &mut PagePayload,
) -> Result<(), EngineError> {
    let images = pdfcraft_edit::reading_images(cos, number)
        .map_err(|e| EngineError::Malformed(format!("PDF image extraction failed: {e}")))?;
    for image in images.into_iter().filter(|image| !image.is_form) {
        let Some(object) = image.object else { continue };
        let Ok((extension, bytes)) = pdfcraft_create::image_file(cos, object) else {
            continue;
        };
        let a = info.user_to_view(image.rect[0] as f32, image.rect[1] as f32);
        let b = info.user_to_view(image.rect[2] as f32, image.rect[3] as f32);
        page.images.push(ImagePayload {
            name: format!("images/{}.{extension}", image.name.trim_start_matches('/')),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            media_type: if extension == "jpg" {
                "image/jpeg".into()
            } else {
                "image/png".into()
            },
            bbox: Some([
                a[0].min(b[0]).into(),
                a[1].min(b[1]).into(),
                a[0].max(b[0]).into(),
                a[1].max(b[1]).into(),
            ]),
            page: Some(number),
            metadata: Map::from_iter([
                ("width".into(), json!(image.width)),
                ("height".into(), json!(image.height)),
            ]),
            ..Default::default()
        });
    }
    Ok(())
}

fn encode_png(width: u32, height: u32, premultiplied: &[u8]) -> Result<Vec<u8>, EngineError> {
    let mut rgba = premultiplied.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for c in &mut pixel[..3] {
                *c = ((u16::from(*c) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut output), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| EngineError::Malformed(format!("PNG encoding failed: {e}")))?;
        writer
            .write_image_data(&rgba)
            .map_err(|e| EngineError::Malformed(format!("PNG encoding failed: {e}")))?;
    }
    Ok(output)
}

fn open_error(error: pdfcraft_render::OpenError) -> EngineError {
    match error {
        pdfcraft_render::OpenError::NeedsPassword | pdfcraft_render::OpenError::WrongPassword => {
            EngineError::Encrypted
        }
        pdfcraft_render::OpenError::Unsupported(message) => EngineError::Unsupported(message),
        pdfcraft_render::OpenError::Invalid(message) => {
            EngineError::Malformed(format!("invalid PDF: {message}"))
        }
    }
}
