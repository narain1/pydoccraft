use std::{collections::HashMap, sync::Mutex};

use base64::Engine;
use deckcraft_model::text::{Action, Hyperlink, RunKind, TextBody};
use deckcraft_model::{Comment, Presentation, Shape, ShapeKind, SlideId};
use serde_json::{json, Map, Value};

use crate::diagnostics::Diagnostics;
use crate::model::{
    BlockPayload, CellPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload,
    TablePayload,
};
use crate::EngineError;

pub struct PptxSession {
    state: Mutex<Option<SessionState>>,
}

struct SessionState {
    presentation: Presentation,
    render_requests: u64,
}

impl PptxSession {
    pub fn open(data: &[u8]) -> Result<(Self, DocumentPayload), EngineError> {
        let presentation = read(data)?;
        let payload = extract_with(&presentation)?;
        Ok((
            Self {
                state: Mutex::new(Some(SessionState {
                    presentation,
                    render_requests: 0,
                })),
            },
            payload,
        ))
    }

    pub fn render(
        &self,
        page: usize,
        dpi: f64,
        clip: Option<crate::rendering::Clip>,
    ) -> Result<Vec<u8>, EngineError> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| EngineError::Malformed("PPTX render session is unavailable".into()))?;
        let state = guard
            .as_mut()
            .ok_or_else(|| EngineError::Closed("PPTX render session is closed".into()))?;
        state.render_requests = state.render_requests.saturating_add(1);
        render_with(&state.presentation, page, dpi, clip)
    }

    pub fn stats(&self) -> Result<(u64, u64, u64), EngineError> {
        let guard = self
            .state
            .lock()
            .map_err(|_| EngineError::Malformed("PPTX render session is unavailable".into()))?;
        let state = guard
            .as_ref()
            .ok_or_else(|| EngineError::Closed("PPTX render session is closed".into()))?;
        Ok((1, 0, state.render_requests))
    }

    pub fn close(&self) -> Result<(), EngineError> {
        // Drop our owned model even after a poisoned render lock. DeckCraft's separate,
        // private thread-local renderer/cache has no public clear hook and can outlive this.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    static PARSE_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub fn read(data: &[u8]) -> Result<Presentation, EngineError> {
    if deckcraft_pptx::is_encrypted(data) {
        return Err(EngineError::Encrypted);
    }
    crate::ooxml::preflight_pptx(data)?;
    #[cfg(test)]
    PARSE_BUILDS.with(|counter| counter.set(counter.get() + 1));
    let presentation = deckcraft_pptx::import(data).map_err(pptx_error)?;
    if presentation.slides.is_empty() {
        return Err(EngineError::Malformed(
            "presentation contains no slides".into(),
        ));
    }
    // Defensively reject ambiguous parsed targets before extraction/render/conversion.
    // This does not detect source entries silently truncated by the pinned reader.
    slide_targets(&presentation)?;
    Ok(presentation)
}

pub fn extract(data: &[u8]) -> Result<DocumentPayload, EngineError> {
    PptxSession::open(data).map(|(_, payload)| payload)
}

fn extract_with(presentation: &Presentation) -> Result<DocumentPayload, EngineError> {
    validate_tables(presentation)?;
    let targets = slide_targets(presentation)?;
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
                    presentation,
                    &targets,
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
            (
                "diagnostics".into(),
                extraction_diagnostics(presentation, &targets),
            ),
        ]),
        pages,
        stories: Vec::new(),
    })
}

type SlideTargets = HashMap<SlideId, usize>;

fn slide_targets(presentation: &Presentation) -> Result<SlideTargets, EngineError> {
    // This protects adapter indexing, not allocations or truncation inside import().
    if presentation.slides.len() > 10_000 {
        return Err(EngineError::ResourceLimit(
            "PPTX exceeds 10000 parsed slides".into(),
        ));
    }
    let mut targets = HashMap::with_capacity(presentation.slides.len());
    for (page, slide) in presentation.slides.iter().enumerate() {
        if targets.insert(slide.id, page).is_some() {
            return Err(EngineError::Malformed(
                "PPTX has duplicate native slide IDs; internal navigation is ambiguous".into(),
            ));
        }
    }
    Ok(targets)
}

fn extraction_diagnostics(presentation: &Presentation, targets: &SlideTargets) -> Value {
    let mut diagnostics = Diagnostics::default();
    for (slide, value) in presentation.slides.iter().enumerate() {
        for shape in &value.shapes {
            diagnose_shape(presentation, targets, shape, slide, &mut diagnostics);
        }
        diagnose_text(
            &value.notes,
            &json!({"slide": slide, "story": "notes"}),
            true,
            targets,
            &mut diagnostics,
        );
    }
    diagnostics.into_json()
}

fn diagnose_shape(
    presentation: &Presentation,
    targets: &SlideTargets,
    shape: &Shape,
    slide: usize,
    diagnostics: &mut Diagnostics,
) {
    let source = json!({"slide": slide, "shape_id": shape.id.0});
    let notice = match &shape.kind {
        ShapeKind::Chart(_) => Some(("chart_data_omitted", "Semantic extraction omits chart data and chart drawing content.")),
        ShapeKind::Media(_) => Some(("media_content_omitted", "Semantic extraction omits audio/video data and poster content.")),
        ShapeKind::Ink { .. } => Some(("ink_geometry_omitted", "Semantic extraction omits ink stroke geometry.")),
        ShapeKind::Model3d { .. } => Some(("model3d_content_omitted", "Semantic extraction omits 3-D model data and its fallback poster.")),
        ShapeKind::Opaque { .. } => Some(("opaque_content_omitted", "Semantic extraction omits an opaque object's structure and preview; any ordinary shape text remains available.")),
        _ => None,
    };
    if let Some((code, message)) = notice {
        diagnostics.record(code, "warning", message, source.clone());
    }
    if shape.diagram.is_some() {
        diagnostics.record("diagram_structure_omitted", "warning", "Semantic extraction omits diagram structure; ordinary child shape text remains available.", source.clone());
    }
    if shape.xfrm.is_some_and(|xfrm| {
        xfrm.rot != 0.0
            || (matches!(shape.kind, ShapeKind::Group { .. }) && (xfrm.flip_h || xfrm.flip_v))
    }) {
        diagnostics.record("transformed_bbox_approximation", "warning", "Extracted bounding boxes do not account for this shape's rotation or group reflection.", source.clone());
    }
    if let Some(link) = &shape.click {
        diagnose_link(link, targets, source.clone(), diagnostics);
    }
    if let Some(text) = &shape.text {
        diagnose_text(text, &source, false, targets, diagnostics);
    }
    match &shape.kind {
        ShapeKind::Group { children, .. } => {
            for child in children { diagnose_shape(presentation, targets, child, slide, diagnostics); }
        }
        ShapeKind::Table(table) => {
            for (row, value) in table.rows.iter().enumerate() {
                for (column, cell) in value.cells.iter().enumerate() {
                    let mut cell_source = source.clone();
                    cell_source["row"] = json!(row);
                    cell_source["column"] = json!(column);
                    diagnose_text(&cell.text, &cell_source, false, targets, diagnostics);
                }
            }
        }
        ShapeKind::Picture { fill } if presentation.media(fill.media).is_none() => diagnostics.record("missing_image_data", "warning", "An image reference has no media bytes in the parsed model and is omitted from extraction.", source),
        _ => {}
    }
}

fn diagnose_text(
    body: &TextBody,
    source: &Value,
    auxiliary: bool,
    targets: &SlideTargets,
    diagnostics: &mut Diagnostics,
) {
    for (paragraph, value) in body.paragraphs.iter().enumerate() {
        for (run, value) in value.runs.iter().enumerate() {
            let mut source = source.clone();
            source["paragraph"] = json!(paragraph);
            source["run"] = json!(run);
            match &value.kind {
                RunKind::Math { .. } => diagnostics.record("equation_text_only", "info", "An equation contributes its stored text; structured mathematics is not extracted.", source.clone()),
                RunKind::Field { .. } => diagnostics.record("field_cached_result", "info", "A field contributes stored result text; extraction does not refresh the field.", source.clone()),
                _ => {}
            }
            if let Some(link) = &value.props.link {
                if auxiliary {
                    diagnostics.record("auxiliary_link_omitted", "warning", "A notes hyperlink contributes its text label but is not included in extracted links.", source);
                } else {
                    diagnose_link(link, targets, source, diagnostics);
                }
            }
        }
    }
}

pub fn render(data: &[u8], page: usize, dpi: f64) -> Result<Vec<u8>, EngineError> {
    render_impl(data, page, dpi, None)
}

pub fn render_region(
    data: &[u8],
    page: usize,
    dpi: f64,
    clip: crate::rendering::Clip,
) -> Result<Vec<u8>, EngineError> {
    render_impl(data, page, dpi, Some(clip))
}

fn render_impl(
    data: &[u8],
    page: usize,
    dpi: f64,
    clip: Option<crate::rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    crate::rendering::validate_dpi(dpi)?;
    let presentation = read(data)?;
    render_with(&presentation, page, dpi, clip)
}

fn render_with(
    presentation: &Presentation,
    page: usize,
    dpi: f64,
    clip: Option<crate::rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    crate::rendering::validate_dpi(dpi)?;
    if page >= presentation.slides.len() {
        return Err(EngineError::Unsupported(format!(
            "page {page} does not exist"
        )));
    }
    let scale = dpi / 72.0;
    let size = [
        presentation.slide_size.width,
        presentation.slide_size.height,
    ];
    let region = clip
        .map(|clip| {
            crate::rendering::RasterRegion::new(
                clip,
                size,
                scale,
                [
                    (size[0] * scale).round().max(1.0),
                    (size[1] * scale).round().max(1.0),
                ],
                deckcraft_render::MAX_SIDE,
            )
        })
        .transpose()?;
    if region.is_none() {
        // A clipped request bounds its output instead of allocating the full slide.
        crate::rendering::validate_size(
            size[0] * dpi / 72.0,
            size[1] * dpi / 72.0,
            deckcraft_render::MAX_SIDE,
        )?;
    }
    let image = deckcraft_render::render_slide(
        presentation,
        page,
        &deckcraft_render::RenderOpts {
            scale,
            offset: region.map_or((0.0, 0.0), |region| {
                (-f64::from(region.x), -f64::from(region.y))
            }),
            size: region.map(|region| (region.width, region.height)),
            ..deckcraft_render::RenderOpts::default()
        },
    );
    if let Some(region) = region {
        region.validate_result(image.width, image.height)?;
    }
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
    targets: &SlideTargets,
    shape: &Shape,
    page_number: usize,
    placement: Placement,
    page: &mut PagePayload,
) {
    let bbox = placement.bbox(shape);
    let mut block_indexes = Vec::new();
    if let Some(text) = &shape.text {
        for paragraph in &text.paragraphs {
            let (text, links) = paragraph_payload(paragraph, page_number, bbox, targets);
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
    if let Some(link) = shape_link(shape.click.as_ref(), shape, page_number, bbox, targets) {
        page.links.push(link.clone());
        if let Some(index) = block_indexes.first().copied() {
            page.blocks[index].links.push(link);
        }
    }
    match &shape.kind {
        ShapeKind::Group { children, child } => {
            let placement = placement.group(shape, *child);
            for child_shape in children {
                visit_shape(
                    presentation,
                    targets,
                    child_shape,
                    page_number,
                    placement,
                    page,
                );
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
                cells: table_cells(table, shape.id.0, page_number, targets, page),
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
    targets: &SlideTargets,
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
                            let (text, mut links) =
                                paragraph_payload(paragraph, page_number, None, targets);
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
    targets: &SlideTargets,
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
        if let Some((uri, target)) = run
            .props
            .link
            .as_ref()
            .and_then(|link| link_destination(link, targets))
        {
            let mut metadata = Map::from_iter([
                ("start".into(), json!(offset)),
                ("end".into(), json!(offset + length)),
            ]);
            if let Some(target) = target {
                metadata.insert("target_page".into(), json!(target));
            }
            links.push(LinkPayload {
                uri,
                text: value.clone(),
                bbox,
                page: Some(page),
                metadata,
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
    targets: &SlideTargets,
) -> Option<LinkPayload> {
    let (uri, target) = link_destination(hyperlink?, targets)?;
    let mut metadata = Map::from_iter([("shape".into(), Value::Bool(true))]);
    if let Some(target) = target {
        metadata.insert("target_page".into(), json!(target));
    }
    Some(LinkPayload {
        uri,
        text: shape.text.as_ref().map(body_text).unwrap_or_default(),
        bbox,
        page: Some(page),
        metadata,
    })
}

fn link_destination(
    hyperlink: &Hyperlink,
    targets: &SlideTargets,
) -> Option<(String, Option<usize>)> {
    match &hyperlink.action {
        Action::Url { url } => Some((url.clone(), None)),
        Action::Slide { slide } => targets
            .get(slide)
            .map(|target| (format!("#page={}", target + 1), Some(*target))),
        _ => None,
    }
}

fn diagnose_link(
    link: &Hyperlink,
    targets: &SlideTargets,
    source: Value,
    diagnostics: &mut Diagnostics,
) {
    match &link.action {
        Action::Url { .. } => {},
        Action::Slide { slide } if targets.contains_key(slide) => {},
        Action::Slide { .. } => diagnostics.record(
            "unresolved_slide_link_omitted", "warning",
            "An internal slide hyperlink has no target in the parsed presentation and is omitted.", source),
        _ => diagnostics.record(
            "non_url_link_omitted", "warning",
            "A hyperlink action other than a URL or resolved slide target is not included in extracted links.", source),
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

    fn session_fixture() -> Vec<u8> {
        let mut table = Table::new(1, 1, 120.0, 30.0);
        table.rows[0].cells[0].text = TextBody::from_text("Retained table");
        let mut presentation = presentation(table);
        presentation.slide_size = Size::new(240.0, 180.0);
        let layout =
            deckcraft_model::defaults::layout_of_kind(&presentation, LayoutType::Blank).unwrap();
        let mut slide = deckcraft_model::defaults::new_slide(&mut presentation, layout).unwrap();
        slide.hidden = true;
        slide.shapes.push(Shape {
            id: ShapeId(presentation.alloc_id().unwrap()),
            xfrm: Some(Xfrm::new(20.0, 30.0, 180.0, 60.0)),
            text: Some(TextBody::from_text("Second slide")),
            ..Shape::default()
        });
        presentation.slides.push(Arc::new(slide));
        deckcraft_pptx::export(&presentation).unwrap()
    }

    #[test]
    fn render_session_reuses_one_import_and_matches_full_and_region_rasters() {
        let source = session_fixture();
        PARSE_BUILDS.with(|counter| counter.set(0));
        let (session, payload) = PptxSession::open(&source).unwrap();
        assert_eq!(payload.pages.len(), 2);
        assert_eq!(payload.pages[1].metadata["hidden"], true);
        assert_eq!(session.stats().unwrap(), (1, 0, 0));
        assert_eq!(PARSE_BUILDS.with(|counter| counter.get()), 1);
        let mut requests = 0;
        for page in [0, 1, 0] {
            for dpi in [72.0, 144.0, 300.0] {
                for clip in [None, Some((5.25, 10.5, 90.25, 55.75))] {
                    let before = PARSE_BUILDS.with(|counter| counter.get());
                    let actual = session.render(page, dpi, clip).unwrap();
                    assert_eq!(PARSE_BUILDS.with(|counter| counter.get()), before);
                    let expected = render_impl(&source, page, dpi, clip).unwrap();
                    assert_eq!(actual, expected, "page {page}, dpi {dpi}, clip {clip:?}");
                    requests += 1;
                }
            }
        }
        assert_eq!(session.stats().unwrap(), (1, 0, requests));
        assert_eq!(
            PARSE_BUILDS.with(|counter| counter.get()),
            1 + requests as usize
        );
        drop(source);
        session.render(1, 72.0, None).unwrap();
        session.close().unwrap();
        assert_eq!(payload.pages[0].tables[0].rows[0][0], "Retained table");
    }

    #[test]
    fn render_session_preserves_errors_limits_and_oversized_full_small_region_behavior() {
        assert!(matches!(
            PptxSession::open(b"not an archive"),
            Err(EngineError::Malformed(_))
        ));
        let mut presentation = presentation(Table::new(1, 1, 100.0, 30.0));
        presentation.slide_size = Size::new(2000.0, 2000.0);
        let source = deckcraft_pptx::export(&presentation).unwrap();
        let (session, _) = PptxSession::open(&source).unwrap();
        assert!(matches!(
            session.render(0, 1200.0, None),
            Err(EngineError::ResourceLimit(_))
        ));
        let clip = Some((0.0, 0.0, 10.0, 10.0));
        assert_eq!(
            session.render(0, 1200.0, clip).unwrap(),
            render_impl(&source, 0, 1200.0, clip).unwrap()
        );
        for (page, dpi, clip) in [
            (1, 72.0, None),
            (0, f64::NAN, None),
            (0, 0.0, None),
            (0, 72.0, Some((0.0, 0.0, -1.0, 10.0))),
            (0, 72.0, Some((3000.0, 3000.0, 10.0, 10.0))),
        ] {
            assert_eq!(
                session.render(page, dpi, clip).unwrap_err().to_string(),
                render_impl(&source, page, dpi, clip)
                    .unwrap_err()
                    .to_string()
            );
        }
        assert_eq!(session.stats().unwrap(), (1, 0, 7));
        session.close().unwrap();
        session.close().unwrap();
        assert!(matches!(
            session.render(0, 72.0, None),
            Err(EngineError::Closed(_))
        ));
        assert!(matches!(session.stats(), Err(EngineError::Closed(_))));
    }

    #[test]
    fn render_session_close_releases_model_and_recovers_poisoned_lock() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PptxSession>();
        let (session, payload) = PptxSession::open(&session_fixture()).unwrap();
        let slide = {
            let state = session.state.lock().unwrap();
            Arc::downgrade(&state.as_ref().unwrap().presentation.slides[0])
        };
        session.render(0, 72.0, None).unwrap();
        assert!(slide.upgrade().is_some());
        session.close().unwrap();
        assert!(slide.upgrade().is_none());
        assert_eq!(payload.pages[0].tables[0].rows[0][0], "Retained table");

        let (session, _) = PptxSession::open(&session_fixture()).unwrap();
        let slide = {
            let state = session.state.lock().unwrap();
            Arc::downgrade(&state.as_ref().unwrap().presentation.slides[0])
        };
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = session.state.lock().unwrap();
            panic!("test intentional session lock poison");
        }));
        assert!(poisoned.is_err());
        assert!(matches!(
            session.render(0, 72.0, None),
            Err(EngineError::Malformed(_))
        ));
        session.close().unwrap();
        assert!(slide.upgrade().is_none());
        session.close().unwrap();
    }

    #[test]
    fn render_session_close_and_render_are_serialized_across_threads() {
        let (session, _) = PptxSession::open(&session_fixture()).unwrap();
        let session = Arc::new(session);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let rendering = {
            let session = Arc::clone(&session);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                session.render(0, 72.0, None)
            })
        };
        barrier.wait();
        session.close().unwrap();
        match rendering.join().unwrap() {
            Ok(png) => assert!(png.starts_with(b"\x89PNG")),
            Err(EngineError::Closed(_)) => {}
            Err(error) => panic!("unexpected concurrent render result: {error}"),
        }
        assert!(matches!(
            session.render(0, 72.0, None),
            Err(EngineError::Closed(_))
        ));
    }

    #[test]
    fn diagnostics_identify_model_constructs_without_claiming_rendering_failure() {
        let mut document = presentation(Table::new(1, 1, 100.0, 30.0));
        let kinds = [
            ShapeKind::Chart(Box::default()),
            ShapeKind::Media(Default::default()),
            ShapeKind::Ink {
                strokes: Vec::new(),
            },
            ShapeKind::Model3d {
                media: deckcraft_model::MediaId(900),
                poster: None,
            },
            ShapeKind::Opaque {
                xml: "<opaque/>".into(),
                preview: None,
                label: "Object".into(),
                parts: Vec::new(),
            },
        ];
        for kind in kinds {
            let id = ShapeId(document.alloc_id().unwrap());
            Arc::make_mut(&mut document.slides[0]).shapes.push(Shape {
                id,
                kind,
                ..Shape::default()
            });
        }
        let values = extraction_diagnostics(&document, &slide_targets(&document).unwrap());
        let codes: Vec<_> = values
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            codes,
            [
                "chart_data_omitted",
                "media_content_omitted",
                "ink_geometry_omitted",
                "model3d_content_omitted",
                "opaque_content_omitted"
            ]
        );
        assert!(values
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value["count"] == 1
                && value["sources"][0]["slide"] == 0
                && value["operation"] == "extract"));
    }

    #[test]
    fn native_chart_file_emits_semantic_omission_diagnostic() {
        let mut document = presentation(Table::new(1, 1, 100.0, 30.0));
        let id = ShapeId(document.alloc_id().unwrap());
        Arc::make_mut(&mut document.slides[0]).shapes.push(Shape {
            id,
            kind: ShapeKind::Chart(Box::default()),
            xfrm: Some(Xfrm::new(20.0, 80.0, 300.0, 200.0)),
            ..Shape::default()
        });
        let bytes = deckcraft_pptx::export(&document).unwrap();
        let extracted = extract(&bytes).unwrap();
        assert_eq!(
            extracted.metadata["diagnostics"][0]["code"],
            "chart_data_omitted"
        );
        assert_eq!(extracted.metadata["diagnostics"][0]["count"], 1);
    }

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

    fn action_link(action: Action) -> Hyperlink {
        let mut link = linked_run("https://example.com/").props.link.unwrap();
        link.action = action;
        link
    }

    fn action_run(text: &str, action: Action) -> Run {
        Run::with(
            text,
            RunProps {
                link: Some(action_link(action)),
                ..RunProps::default()
            },
        )
    }

    #[test]
    fn internal_navigation_roundtrips_reordered_hidden_slides_runs_tables_and_groups() {
        let mut document = presentation(Table::new(1, 1, 400.0, 30.0));
        let layout =
            deckcraft_model::defaults::layout_of_kind(&document, LayoutType::Blank).unwrap();
        for _ in 0..2 {
            let slide = deckcraft_model::defaults::new_slide(&mut document, layout).unwrap();
            document.slides.push(Arc::new(slide));
        }
        let source = document.slides[0].id;
        let hidden = document.slides[1].id;
        let other = document.slides[2].id;
        Arc::make_mut(&mut document.slides[1]).hidden = true;
        let ShapeKind::Table(table) = &mut Arc::make_mut(&mut document.slides[0]).shapes[0].kind
        else {
            panic!()
        };
        table.rows[0].cells[0].text.paragraphs[0].runs =
            vec![action_run("Cell target", Action::Slide { slide: hidden })];
        let mut text = TextBody::from_text("");
        text.paragraphs[0].runs = vec![
            Run::new("λ "),
            action_run("Same", Action::Slide { slide: hidden }),
            Run::new(" / "),
            linked_run("https://example.com/unchanged"),
            Run::new(" "),
            action_run("Ω", Action::Slide { slide: source }),
        ];
        let id = ShapeId(document.alloc_id().unwrap());
        Arc::make_mut(&mut document.slides[0]).shapes.push(Shape {
            id,
            text: Some(text),
            xfrm: Some(Xfrm::new(20.0, 60.0, 400.0, 50.0)),
            click: Some(action_link(Action::Slide { slide: other })),
            ..Shape::default()
        });
        let mut child_text = TextBody::from_text("");
        child_text.paragraphs[0].runs =
            vec![action_run("Child target", Action::Slide { slide: hidden })];
        let child = Shape {
            id: ShapeId(document.alloc_id().unwrap()),
            text: Some(child_text),
            xfrm: Some(Xfrm::new(5.0, 5.0, 100.0, 30.0)),
            click: Some(action_link(Action::Slide { slide: other })),
            ..Shape::default()
        };
        let group = Shape {
            id: ShapeId(document.alloc_id().unwrap()),
            xfrm: Some(Xfrm::new(30.0, 150.0, 200.0, 100.0)),
            kind: ShapeKind::Group {
                children: vec![child],
                child: Xfrm::new(0.0, 0.0, 200.0, 100.0),
            },
            click: Some(action_link(Action::Slide { slide: hidden })),
            ..Shape::default()
        };
        Arc::make_mut(&mut document.slides[0]).shapes.push(group);
        document.slides.rotate_left(1); // Hidden, Other, Source: stable targets must follow their IDs.
        let bytes = deckcraft_pptx::export(&document).unwrap();
        let payload = extract(&bytes).unwrap();
        assert_eq!(payload.pages.len(), 3);
        assert_eq!(payload.pages[0].metadata["hidden"], true);
        assert_eq!(payload.metadata["diagnostics"], json!([]));
        let page = &payload.pages[2];
        let expected = [
            "#page=1",
            "#page=1",
            "https://example.com/unchanged",
            "#page=3",
            "#page=2",
            "#page=1",
            "#page=1",
            "#page=2",
        ];
        assert_eq!(
            page.links
                .iter()
                .map(|link| link.uri.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        for (index, link) in page.links.iter().enumerate() {
            assert_eq!(link.page, Some(2));
            if index == 2 {
                assert!(!link.metadata.contains_key("target_page"));
            } else {
                assert_eq!(
                    link.metadata["target_page"],
                    link.uri[6..].parse::<usize>().unwrap() - 1
                );
            }
        }
        assert_eq!(page.links[1].text, "Same");
        assert_eq!(page.links[1].metadata["start"], 2);
        assert_eq!(page.links[1].metadata["end"], 6);
        assert_eq!(page.links[2].metadata["start"], 9);
        assert_eq!(page.links[2].metadata["end"], 13);
        assert_eq!(page.links[3].metadata["start"], 14);
        assert_eq!(page.links[3].metadata["end"], 15);
        assert_eq!(
            page.tables[0].cells[0][0].blocks[0].links[0].metadata["target_page"],
            0
        );
        assert_eq!(
            page.blocks
                .iter()
                .map(|block| block.text.as_str())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>(),
            ["λ Same / Same Ω", "Child target"]
        );
        assert_eq!(page.links[5].text, ""); // Group action has no invented text label.
        assert_eq!(page.links[6].bbox, Some([35.0, 155.0, 135.0, 185.0]));
    }

    #[test]
    fn unresolved_and_contextual_actions_remain_explicit_omissions() {
        let mut document = presentation(Table::new(1, 1, 100.0, 30.0));
        let missing = action_link(Action::Slide {
            slide: SlideId(u32::MAX),
        });
        let targets = slide_targets(&document).unwrap();
        assert!(link_destination(&missing, &targets).is_none());
        let mut diagnostics = Diagnostics::default();
        diagnose_link(&missing, &targets, json!({"slide": 0}), &mut diagnostics);
        assert_eq!(
            diagnostics.into_json()[0]["code"],
            "unresolved_slide_link_omitted"
        );
        let actions = [
            Action::NextSlide,
            Action::PreviousSlide,
            Action::FirstSlide,
            Action::LastSlide,
            Action::EndShow,
            Action::LastViewed,
            Action::CustomShow {
                name: "Show".into(),
                returns: true,
            },
            Action::Program {
                path: "program.exe".into(),
            },
            Action::PlayMedia,
        ];
        let mut text = TextBody::from_text("");
        text.paragraphs[0].runs = actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| action_run(&format!("Action{index} "), action))
            .collect();
        let id = ShapeId(document.alloc_id().unwrap());
        Arc::make_mut(&mut document.slides[0]).shapes.push(Shape {
            id,
            text: Some(text),
            ..Shape::default()
        });
        let bytes = deckcraft_pptx::export(&document).unwrap();
        let payload = extract(&bytes).unwrap();
        assert!(payload.pages[0].links.is_empty());
        assert_eq!(
            payload.metadata["diagnostics"][0]["code"],
            "non_url_link_omitted"
        );
        assert_eq!(payload.metadata["diagnostics"][0]["count"], 9);
        assert!(payload.pages[0]
            .blocks
            .last()
            .unwrap()
            .text
            .contains("Action8"));
    }

    #[test]
    fn slide_index_rejects_duplicate_ids_and_bounds_collections() {
        let mut document = presentation(Table::new(1, 1, 100.0, 30.0));
        let mut slide = (*document.slides[0]).clone();
        slide.shapes.clear();
        document.slides.clear();
        for id in 0..10_000 {
            slide.id = SlideId(id);
            document.slides.push(Arc::new(slide.clone()));
        }
        assert_eq!(slide_targets(&document).unwrap().len(), 10_000);
        slide.id = SlideId(10_000);
        document.slides.push(Arc::new(slide.clone()));
        assert!(matches!(
            slide_targets(&document),
            Err(EngineError::ResourceLimit(_))
        ));
        document.slides.pop();
        Arc::make_mut(&mut document.slides[9_999]).id = SlideId(0);
        assert!(matches!(
            slide_targets(&document),
            Err(EngineError::Malformed(_))
        ));
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
