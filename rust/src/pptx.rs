use base64::Engine;
use deckcraft_model::text::{Action, Hyperlink, RunKind, TextBody};
use deckcraft_model::{Comment, Presentation, Shape, ShapeKind};
use serde_json::{json, Map, Value};

use crate::model::{
    BlockPayload, CellPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload,
    TablePayload,
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
    validate_tables(&presentation)?;
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
    crate::rendering::validate_dpi(dpi)?;
    let presentation = read(data)?;
    if page >= presentation.slides.len() {
        return Err(EngineError::Unsupported(format!(
            "page {page} does not exist"
        )));
    }
    // Reject oversized requests before the upstream rasterizer allocates or clamps them.
    crate::rendering::validate_size(
        presentation.slide_size.width * dpi / 72.0,
        presentation.slide_size.height * dpi / 72.0,
        deckcraft_render::MAX_SIDE,
    )?;
    let image = deckcraft_render::render_slide(
        &presentation,
        page,
        &deckcraft_render::RenderOpts {
            scale: dpi / 72.0,
            ..deckcraft_render::RenderOpts::default()
        },
    );
    crate::rendering::validate_rgba(image.width, image.height, &image.pixels)?;
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
                cells: table_cells(table, shape.id.0, page_number, page),
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

fn validate_tables(presentation: &Presentation) -> Result<(), EngineError> {
    let mut total_cells = 0_usize;
    let mut pending: Vec<_> = presentation
        .slides
        .iter()
        .flat_map(|slide| slide.shapes.iter().map(|shape| (shape, 0_usize)))
        .collect();
    while let Some((shape, depth)) = pending.pop() {
        match &shape.kind {
            ShapeKind::Group { children, .. } => {
                if depth >= 64 {
                    return Err(EngineError::ResourceLimit(
                        "PPTX group nesting exceeds 64 levels".into(),
                    ));
                }
                pending.extend(children.iter().map(|child| (child, depth + 1)));
            }
            ShapeKind::Table(table) => {
                validate_table(table, &mut total_cells)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_table(table: &deckcraft_model::Table, total: &mut usize) -> Result<(), EngineError> {
    *total = table
        .rows
        .iter()
        .fold(*total, |count, row| count.saturating_add(row.cells.len()));
    if table.rows.len() > 1000
        || table.cols.len() > 1000
        || *total > 100_000
        || table.rows.iter().any(|row| {
            row.cells.len() > 1000
                || row.cells.iter().enumerate().any(|(column, cell)| {
                    column.saturating_add(cell.grid_span.max(1) as usize) > 1000
                        || cell.row_span > 1000
                })
        })
    {
        return Err(EngineError::ResourceLimit(
            "PPTX table exceeds 1000 rows/columns or presentation exceeds 100000 table cells"
                .into(),
        ));
    }
    Ok(())
}

fn table_cells(
    table: &deckcraft_model::Table,
    shape: u32,
    page_number: usize,
    page: &mut PagePayload,
) -> Vec<Vec<CellPayload>> {
    table
        .rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            row.cells
                .iter()
                .enumerate()
                .map(|(column, cell)| {
                    let blocks = cell
                        .text
                        .paragraphs
                        .iter()
                        .enumerate()
                        .map(|(paragraph_index, paragraph)| {
                            let (text, mut links) = paragraph_payload(paragraph, page_number, None);
                            let path = json!([shape, row_index, column, paragraph_index]);
                            for link in &mut links {
                                link.metadata.insert("source_path".into(), path.clone());
                                link.metadata.insert("shape_id".into(), json!(shape));
                            }
                            page.links.extend(links.clone());
                            BlockPayload {
                                kind: "paragraph".into(),
                                text,
                                links,
                                metadata: Map::from_iter([
                                    ("source_path".into(), path),
                                    ("runs".into(), paragraph_runs(paragraph)),
                                ]),
                                ..BlockPayload::default()
                            }
                        })
                        .collect();
                    CellPayload {
                        blocks,
                        col_span: cell.grid_span.max(1) as usize,
                        row_span: cell.row_span.max(1) as usize,
                        covered: cell.is_covered(),
                        metadata: Map::from_iter([
                            ("grid_column".into(), json!(column)),
                            ("horizontal_merge".into(), json!(cell.h_merge)),
                            ("vertical_merge".into(), json!(cell.v_merge)),
                        ]),
                    }
                })
                .collect()
        })
        .collect()
}

fn paragraph_runs(paragraph: &deckcraft_model::text::Paragraph) -> Value {
    let mut offset = 0_usize;
    Value::Array(paragraph.runs.iter().map(|run| {
        let text = if run.kind == RunKind::Break { "\n" } else { &run.text };
        let end = offset + text.chars().count();
        let value = json!({
            "text": text, "start": offset, "end": end,
            "kind": match run.kind { RunKind::Text => "text", RunKind::Break => "break", RunKind::Field { .. } => "field", RunKind::Math { .. } => "math" },
            "bold": run.props.bold, "italic": run.props.italic,
            "font": run.props.font, "font_size": run.props.size,
        });
        offset = end;
        value
    }).collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_model::text::{Run, RunProps};
    use deckcraft_model::{LayoutType, ShapeId, Size, Table, Theme, Xfrm};
    use std::sync::Arc;

    fn presentation(table: Table) -> Presentation {
        let mut document = deckcraft_model::defaults::blank_presentation(
            Size::new(720.0, 540.0),
            Theme::default(),
            false,
        );
        let layout =
            deckcraft_model::defaults::layout_of_kind(&document, LayoutType::Blank).unwrap();
        let mut slide = deckcraft_model::defaults::new_slide(&mut document, layout).unwrap();
        slide.shapes.push(Shape {
            id: ShapeId(document.alloc_id().unwrap()),
            xfrm: Some(Xfrm::new(10.0, 10.0, 400.0, 200.0)),
            kind: ShapeKind::Table(table),
            ..Shape::default()
        });
        document.slides.push(Arc::new(slide));
        document
    }

    fn linked_run(uri: &str) -> Run {
        Run::with(
            "Same",
            RunProps {
                bold: Some(true),
                link: Some(Hyperlink {
                    action: Action::Url { url: uri.into() },
                    tooltip: String::new(),
                    highlight_click: false,
                    sound: None,
                    stop_sound: false,
                    zoom_return: None,
                }),
                ..RunProps::default()
            },
        )
    }

    #[test]
    fn extracted_cells_keep_runs_merges_and_repeated_link_offsets() {
        let mut table = Table::new(3, 2, 400.0, 60.0);
        table.rows[0].cells[0].text.paragraphs[0].runs = vec![
            Run::new("λ "),
            linked_run("https://example.com/one"),
            Run::new(" / "),
            linked_run("https://example.com/two"),
        ];
        assert!(table.merge(1, 0, 2, 1));
        let source = deckcraft_pptx::export(&presentation(table)).unwrap();
        let payload = extract(&source).unwrap();
        let page = &payload.pages[0];
        let table = &page.tables[0];
        assert_eq!(table.rows[0][0], "λ Same / Same");
        let paragraph = &table.cells[0][0].blocks[0];
        assert_eq!(paragraph.text, "λ Same / Same");
        assert_eq!(paragraph.links.len(), 2);
        assert_eq!(paragraph.links[0].metadata["start"], 2);
        assert_eq!(paragraph.links[0].metadata["end"], 6);
        assert_eq!(paragraph.links[1].metadata["start"], 9);
        assert_eq!(paragraph.links[1].metadata["end"], 13);
        assert_eq!(paragraph.metadata["runs"][1]["bold"], true);
        assert_eq!(page.links.len(), 2);
        assert_eq!(table.cells[1][0].col_span, 2);
        assert_eq!(table.cells[1][0].row_span, 2);
        assert!(table.cells[2][1].covered);
    }

    #[test]
    fn rejects_unbounded_table_spans_before_payload_creation() {
        let mut table = Table::new(1, 1, 100.0, 20.0);
        table.rows[0].cells[0].grid_span = u32::MAX;
        assert!(matches!(
            validate_tables(&presentation(table)),
            Err(EngineError::ResourceLimit(_))
        ));
    }
}
