use base64::Engine;
use deckcraft_model::text::{Action, Hyperlink, RunKind, TextBody};
use deckcraft_model::{Comment, Presentation, Shape, ShapeKind};
use serde_json::{json, Map, Value};

use crate::model::{
    BlockPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload, TablePayload,
};
use crate::EngineError;

pub fn read(data: &[u8]) -> Result<Presentation, EngineError> {
    if deckcraft_pptx::is_encrypted(data) {
        return Err(EngineError::Encrypted);
    }
    crate::ooxml::preflight(data)?;
    let presentation = deckcraft_pptx::import(data).map_err(pptx_error)?;
    if presentation.slides.is_empty() {
        return Err(EngineError::Malformed(
            "presentation contains no slides".into(),
        ));
    }
    Ok(presentation)
}

pub fn extract(data: &[u8]) -> Result<DocumentPayload, EngineError> {
    let presentation = read(data)?;
    let pages = presentation
        .slides
        .iter()
        .enumerate()
        .map(|(number, slide)| {
            let mut page = PagePayload {
                number,
                metadata: Map::from_iter([
                    ("width".into(), json!(presentation.slide_size.width)),
                    ("height".into(), json!(presentation.slide_size.height)),
                    ("name".into(), Value::String(slide.name.clone())),
                    ("hidden".into(), Value::Bool(slide.hidden)),
                ]),
                ..PagePayload::default()
            };
            for shape in &slide.shapes {
                visit_shape(
                    &presentation,
                    shape,
                    number,
                    Placement::default(),
                    &mut page,
                );
            }
            add_notes(&slide.notes, number, &mut page);
            for (index, comment) in slide.comments.iter().enumerate() {
                add_comment(comment, number, None, index.to_string(), &mut page);
            }
            page
        })
        .collect();
    Ok(DocumentPayload {
        format: "pptx".into(),
        metadata: Map::from_iter([
            ("width".into(), json!(presentation.slide_size.width)),
            ("height".into(), json!(presentation.slide_size.height)),
        ]),
        pages,
    })
}

pub fn render(data: &[u8], page: usize, dpi: f64) -> Result<Vec<u8>, EngineError> {
    if !dpi.is_finite() || !(1.0..=1200.0).contains(&dpi) {
        return Err(EngineError::ResourceLimit(
            "DPI must be finite and between 1 and 1200".into(),
        ));
    }
    let presentation = read(data)?;
    if page >= presentation.slides.len() {
        return Err(EngineError::Unsupported(format!(
            "page {page} does not exist"
        )));
    }
    let image = deckcraft_render::render_slide(
        &presentation,
        page,
        &deckcraft_render::RenderOpts {
            scale: dpi / 72.0,
            ..deckcraft_render::RenderOpts::default()
        },
    );
    if image.width == 0 || image.height == 0 {
        return Err(EngineError::Malformed("PPTX rendering failed".into()));
    }
    if image.width >= deckcraft_render::MAX_SIDE || image.height >= deckcraft_render::MAX_SIDE {
        return Err(EngineError::ResourceLimit(
            "PPTX render exceeds the image dimension limit".into(),
        ));
    }
    let png = image.to_png_at(dpi);
    if png.is_empty() {
        return Err(EngineError::Malformed("PPTX PNG encoding failed".into()));
    }
    Ok(png)
}

pub fn to_pdf(data: &[u8]) -> Result<Vec<u8>, EngineError> {
    let presentation = read(data)?;
    deckcraft_pdf::export(&presentation, &deckcraft_pdf::PdfOptions::default())
        .map_err(|error| EngineError::Conversion(format!("PPTX to PDF conversion failed: {error}")))
}

#[derive(Clone, Copy)]
struct Placement {
    sx: f64,
    sy: f64,
    tx: f64,
    ty: f64,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            sx: 1.0,
            sy: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }
}

impl Placement {
    fn bbox(self, shape: &Shape) -> Option<[f64; 4]> {
        let xfrm = shape.xfrm?;
        Some([
            self.tx + xfrm.x * self.sx,
            self.ty + xfrm.y * self.sy,
            self.tx + (xfrm.x + xfrm.w) * self.sx,
            self.ty + (xfrm.y + xfrm.h) * self.sy,
        ])
    }

    fn group(self, shape: &Shape, child: deckcraft_model::Xfrm) -> Self {
        let Some(group) = shape.xfrm else { return self };
        if child.w == 0.0 || child.h == 0.0 {
            return self;
        }
        let sx = self.sx * group.w / child.w;
        let sy = self.sy * group.h / child.h;
        Self {
            sx,
            sy,
            tx: self.tx + group.x * self.sx - child.x * sx,
            ty: self.ty + group.y * self.sy - child.y * sy,
        }
    }
}

fn visit_shape(
    presentation: &Presentation,
    shape: &Shape,
    page_number: usize,
    placement: Placement,
    page: &mut PagePayload,
) {
    let bbox = placement.bbox(shape);
    let mut block_indexes = Vec::new();
    if let Some(text) = &shape.text {
        for paragraph in &text.paragraphs {
            let (text, links) = paragraph_payload(paragraph, page_number, bbox);
            if text.is_empty() {
                continue;
            }
            let mut block = BlockPayload::paragraph(text);
            block.bbox = bbox;
            block.level = (paragraph.level > 0).then_some(paragraph.level);
            block.links = links;
            page.links.extend(block.links.clone());
            block_indexes.push(page.blocks.len());
            page.blocks.push(block);
        }
    }
    if let Some(link) = shape_link(shape.click.as_ref(), shape, page_number, bbox) {
        page.links.push(link.clone());
        if let Some(index) = block_indexes.first().copied() {
            page.blocks[index].links.push(link);
        }
    }
    match &shape.kind {
        ShapeKind::Group { children, child } => {
            let placement = placement.group(shape, *child);
            for child_shape in children {
                visit_shape(presentation, child_shape, page_number, placement, page);
            }
        }
        ShapeKind::Table(table) => {
            let rows = table
                .rows
                .iter()
                .map(|row| row.cells.iter().map(|cell| body_text(&cell.text)).collect())
                .collect();
            let merges = table
                .rows
                .iter()
                .enumerate()
                .flat_map(|(row, value)| {
                    value
                        .cells
                        .iter()
                        .enumerate()
                        .filter(move |(_, cell)| {
                            cell.grid_span > 1 || cell.row_span > 1 || cell.is_covered()
                        })
                        .map(move |(column, cell)| {
                            json!({
                                "row": row,
                                "column": column,
                                "grid_span": cell.grid_span,
                                "row_span": cell.row_span,
                                "covered": cell.is_covered(),
                                "horizontal_merge": cell.h_merge,
                                "vertical_merge": cell.v_merge,
                            })
                        })
                })
                .collect::<Vec<_>>();
            let mut metadata = Map::new();
            metadata.insert("merges".into(), Value::Array(merges));
            metadata.insert("column_widths".into(), json!(table.cols));
            let payload = TablePayload {
                rows,
                bbox,
                page: Some(page_number),
                metadata,
            };
            page.tables.push(payload.clone());
            page.blocks.push(BlockPayload {
                kind: "table".into(),
                bbox,
                table: Some(payload),
                ..BlockPayload::default()
            });
        }
        ShapeKind::Picture { fill } => {
            if let Some(media) = presentation.media.iter().find(|item| item.id == fill.media) {
                let mut metadata = Map::new();
                if let Some((width, height)) = image_dimensions(media.data.as_slice()) {
                    metadata.insert("width".into(), json!(width));
                    metadata.insert("height".into(), json!(height));
                }
                if let Some([x0, y0, x1, y1]) = bbox {
                    metadata.insert("placement_width".into(), json!(x1 - x0));
                    metadata.insert("placement_height".into(), json!(y1 - y0));
                }
                let name = if media.name.starts_with("images/") {
                    media.name.clone()
                } else {
                    format!("images/{}", media.name)
                };
                page.images.push(ImagePayload {
                    name,
                    data: base64::engine::general_purpose::STANDARD.encode(media.data.as_slice()),
                    media_type: media.content_type.clone(),
                    alt: if !shape.descr.is_empty() {
                        shape.descr.clone()
                    } else if !shape.title.is_empty() {
                        shape.title.clone()
                    } else {
                        media.name.clone()
                    },
                    bbox,
                    page: Some(page_number),
                    metadata,
                });
            }
        }
        _ => {}
    }
}

fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        let width = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
        let height = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
        return Some((width, height));
    }
    deckcraft_render::decode_image(data).map(|image| (image.width(), image.height()))
}

fn paragraph_payload(
    paragraph: &deckcraft_model::text::Paragraph,
    page: usize,
    bbox: Option<[f64; 4]>,
) -> (String, Vec<LinkPayload>) {
    let mut text = String::new();
    let mut links = Vec::new();
    let mut offset = 0_usize;
    for run in &paragraph.runs {
        let value = match run.kind {
            RunKind::Break => "\n".to_string(),
            _ => run.text.clone(),
        };
        let length = value.chars().count();
        if let Some(uri) = run.props.link.as_ref().and_then(url) {
            links.push(LinkPayload {
                uri: uri.into(),
                text: value.clone(),
                bbox,
                page: Some(page),
                metadata: Map::from_iter([
                    ("start".into(), json!(offset)),
                    ("end".into(), json!(offset + length)),
                ]),
            });
        }
        text.push_str(&value);
        offset += length;
    }
    (text, links)
}

fn shape_link(
    hyperlink: Option<&Hyperlink>,
    shape: &Shape,
    page: usize,
    bbox: Option<[f64; 4]>,
) -> Option<LinkPayload> {
    Some(LinkPayload {
        uri: url(hyperlink?)?.into(),
        text: shape.text.as_ref().map(body_text).unwrap_or_default(),
        bbox,
        page: Some(page),
        metadata: Map::from_iter([("shape".into(), Value::Bool(true))]),
    })
}

fn url(hyperlink: &Hyperlink) -> Option<&str> {
    match &hyperlink.action {
        Action::Url { url } => Some(url),
        _ => None,
    }
}

fn body_text(body: &TextBody) -> String {
    body.text().replace('\u{b}', "\n")
}

fn add_notes(notes: &TextBody, page_number: usize, page: &mut PagePayload) {
    let text = body_text(notes);
    if !text.trim().is_empty() {
        page.blocks.push(BlockPayload {
            kind: "notes".into(),
            text,
            metadata: Map::from_iter([("page".into(), json!(page_number))]),
            ..BlockPayload::default()
        });
    }
}

fn add_comment(
    comment: &Comment,
    page_number: usize,
    parent: Option<&str>,
    id: String,
    page: &mut PagePayload,
) {
    let mut metadata = Map::from_iter([
        ("id".into(), Value::String(id.clone())),
        ("author".into(), Value::String(comment.author.clone())),
        ("initials".into(), Value::String(comment.initials.clone())),
        ("date".into(), Value::String(comment.date.clone())),
        ("x".into(), json!(comment.x)),
        ("y".into(), json!(comment.y)),
        ("resolved".into(), Value::Bool(comment.resolved)),
        ("page".into(), json!(page_number)),
    ]);
    if let Some(parent) = parent {
        metadata.insert("parent_id".into(), Value::String(parent.into()));
    }
    if let Some(shape) = comment.shape {
        metadata.insert("shape_id".into(), json!(shape));
    }
    page.blocks.push(BlockPayload {
        kind: "comment".into(),
        text: comment.text.clone(),
        metadata,
        ..BlockPayload::default()
    });
    for (index, reply) in comment.replies.iter().enumerate() {
        add_comment(reply, page_number, Some(&id), format!("{id}.{index}"), page);
    }
}

fn pptx_error(error: deckcraft_pptx::PptxError) -> EngineError {
    match error {
        deckcraft_pptx::PptxError::NotPptx(message) => EngineError::Malformed(message),
        deckcraft_pptx::PptxError::Unsupported(message) => EngineError::Unsupported(message),
        deckcraft_pptx::PptxError::Write(message) => EngineError::Conversion(message),
        deckcraft_pptx::PptxError::Xml(message) => EngineError::Malformed(message),
    }
}
