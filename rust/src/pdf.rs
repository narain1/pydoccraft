use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};

use base64::Engine;
use pdfcraft_render::{
    inspect, LinkTarget, PageRenderer, RenderConfig, RenderRequest, RenderWarning, RequestKind,
};
use serde_json::{json, Map, Value};

use crate::model::{
    BlockPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload, TablePayload,
};
use crate::EngineError;

pub struct PdfSession {
    renderer: Mutex<PageRenderer>,
    page_sizes: Vec<[f32; 2]>,
}

impl PdfSession {
    pub fn open(
        data: &[u8],
        password: Option<&str>,
    ) -> Result<(Self, DocumentPayload), EngineError> {
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
                reject_oversize: true,
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
            check_content_warnings(&rendered.warnings)?;
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
            if let Some(text) = &rendered.text {
                add_text(text, &mut page);
            }
            add_table(&cos, page_info, number, &mut page)?;
            add_links(
                &info.links,
                page_info,
                number,
                rendered.text.as_deref(),
                &mut page,
            )?;
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
        let payload = DocumentPayload {
            format: "pdf".into(),
            metadata,
            pages,
            stories: Vec::new(),
        };
        let page_count = payload.pages.len();
        if renderer.page_count() != page_count {
            return Err(EngineError::Malformed(
                "PDF renderer page count does not match inspection".into(),
            ));
        }
        Ok((
            Self {
                renderer: Mutex::new(renderer),
                page_sizes: info
                    .pages
                    .iter()
                    .map(|page| [page.width, page.height])
                    .collect(),
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
        let mut renderer = self
            .renderer
            .lock()
            .map_err(|_| EngineError::Malformed("PDF render session is unavailable".into()))?;
        render_with(
            &mut renderer,
            self.page_sizes.get(page).copied(),
            page,
            dpi,
            clip,
        )
    }

    pub fn stats(&self) -> Result<pdfcraft_render::RenderStats, EngineError> {
        self.renderer
            .lock()
            .map(|renderer| renderer.stats())
            .map_err(|_| EngineError::Malformed("PDF render session is unavailable".into()))
    }
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    PdfSession::open(data, password).map(|(_, payload)| payload)
}

pub fn render(
    data: &[u8],
    page: usize,
    dpi: f64,
    password: Option<&str>,
) -> Result<Vec<u8>, EngineError> {
    render_impl(data, page, dpi, password, None)
}

pub fn render_region(
    data: &[u8],
    page: usize,
    dpi: f64,
    password: Option<&str>,
    clip: crate::rendering::Clip,
) -> Result<Vec<u8>, EngineError> {
    render_impl(data, page, dpi, password, Some(clip))
}

fn render_impl(
    data: &[u8],
    page: usize,
    dpi: f64,
    password: Option<&str>,
    clip: Option<crate::rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    crate::rendering::validate_dpi(dpi)?;
    let bytes = Arc::new(data.to_vec());
    let info = inspect(bytes.clone(), password).map_err(open_error)?;
    let mut renderer = PageRenderer::new(
        bytes,
        RenderConfig {
            password: password.map(Arc::from),
            reject_oversize: true,
            ..Default::default()
        },
    );
    render_with(
        &mut renderer,
        info.pages.get(page).map(|page| [page.width, page.height]),
        page,
        dpi,
        clip,
    )
}

fn render_with(
    renderer: &mut PageRenderer,
    size: Option<[f32; 2]>,
    page: usize,
    dpi: f64,
    clip: Option<crate::rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    crate::rendering::validate_dpi(dpi)?;
    let size =
        size.ok_or_else(|| EngineError::Unsupported(format!("page {page} does not exist")))?;
    let scale = (dpi / 72.0) as f32;
    let region = clip
        .map(|clip| {
            let region = crate::rendering::RasterRegion::new(
                clip,
                [f64::from(size[0]), f64::from(size[1])],
                f64::from(scale),
                [
                    f64::from(pdfcraft_render::device_pixels(size[0], scale)),
                    f64::from(pdfcraft_render::device_pixels(size[1], scale)),
                ],
                4096,
            )?;
            // Tile offsets become f32 in the native renderer; do not silently round large integers.
            if u64::from(region.x) + u64::from(region.width) > (1 << 24)
                || u64::from(region.y) + u64::from(region.height) > (1 << 24)
            {
                return Err(EngineError::ResourceLimit(
                    "PDF clip coordinates exceed exact native pixel precision".into(),
                ));
            }
            Ok(region)
        })
        .transpose()?;
    let rendered = renderer.render(RenderRequest {
        page,
        kind: RequestKind::Pixels,
        scale,
        tile: region.map(|region| pdfcraft_render::Tile {
            x: region.x,
            y: region.y,
            w: region.width,
            h: region.height,
        }),
        ..Default::default()
    });
    check_content_warnings(&rendered.warnings)?;
    if let Some(e) = rendered.error {
        return Err(EngineError::ResourceLimit(format!(
            "PDF rendering failed: {e}"
        )));
    }
    if let Some(region) = region {
        region.validate_result(rendered.width, rendered.height)?;
    }
    crate::rendering::encode_png(rendered.width, rendered.height, rendered.rgba.as_ref(), dpi)
}

fn check_content_warnings(warnings: &[RenderWarning]) -> Result<(), EngineError> {
    if warnings.contains(&RenderWarning::ContentTruncated) {
        return Err(EngineError::ResourceLimit(
            "PDF decoded-content budget exceeded; partial output was discarded".into(),
        ));
    }
    Ok(())
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
    text: Option<&pdfcraft_render::PageText>,
    page: &mut PagePayload,
) -> Result<(), EngineError> {
    let count = links
        .iter()
        .filter(|link| {
            link.page == number
                && matches!(link.target, LinkTarget::Uri(_) | LinkTarget::Page(_, _))
        })
        .count();
    if text.is_some_and(|text| {
        count.saturating_mul(text.glyphs.iter().fold(text.glyphs.len(), |total, glyph| {
            total.saturating_add(glyph.text.len())
        })) > 10_000_000
    }) {
        return Err(EngineError::ResourceLimit(
            "PDF link label geometry exceeds the work budget".into(),
        ));
    }
    if count == 0 {
        return Ok(());
    }
    let source = text.and_then(|text| LinkText::new(text, &page.blocks));
    let mut candidates: Vec<Option<LinkCandidate>> = Vec::new();
    let mut ambiguous = Vec::new();
    let mut glyph_owner = vec![None; text.map_or(0, |text| text.glyphs.len())];
    for link in links.iter().filter(|link| link.page == number) {
        let (uri, target_page) = match &link.target {
            LinkTarget::Uri(uri) => (uri.clone(), None),
            LinkTarget::Page(target, _) => (format!("#page={}", target + 1), Some(*target)),
            _ => continue,
        };
        {
            let a = info.user_to_view(link.rect[0], link.rect[1]);
            let b = info.user_to_view(link.rect[2], link.rect[3]);
            let rect = [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[0].max(b[0]),
                a[1].max(b[1]),
            ];
            let index = candidates.len();
            ambiguous.push(false);
            let candidate = text.and_then(|text| {
                if !rect.iter().all(|value| value.is_finite())
                    || rect[0] >= rect[2]
                    || rect[1] >= rect[3]
                {
                    return None;
                }
                let selected = text.glyphs_in(rect);
                for &glyph in &selected {
                    if text.glyphs[glyph].text.trim().is_empty() {
                        continue;
                    }
                    if let Some(previous) = glyph_owner[glyph] {
                        ambiguous[previous] = true;
                        ambiguous[index] = true;
                    } else {
                        glyph_owner[glyph] = Some(index);
                    }
                }
                link_label(text, &selected, source.as_ref()?, &page.blocks)
            });
            page.links.push(LinkPayload {
                uri: uri.clone(),
                bbox: Some(rect.map(f64::from)),
                page: Some(number),
                metadata: Map::from_iter([
                    ("annotation_index".into(), json!(link.index)),
                    (
                        "target_kind".into(),
                        json!(if target_page.is_some() { "page" } else { "uri" }),
                    ),
                    ("label_status".into(), json!("unresolved_geometry")),
                ]),
                ..Default::default()
            });
            if let Some(target) = target_page {
                if let Some(payload) = page.links.last_mut() {
                    payload.metadata.insert("target_page".into(), json!(target));
                }
            }
            candidates.push(candidate);
        }
    }
    // Unresolved annotations still participate: never select an arbitrary shared-glyph target.
    for (candidate, ambiguous) in candidates.iter_mut().zip(ambiguous) {
        if ambiguous {
            *candidate = None;
        }
    }
    attach_link_candidates(page, candidates)
}

struct GlyphOffset {
    line: usize,
    byte_start: usize,
    byte_end: usize,
    char_start: usize,
    char_end: usize,
}

struct LinkLine {
    start: usize,
    end: usize,
    block: Option<usize>,
}

/// An offset map into the backend's text, not a second reading-order or layout pass.
struct LinkText {
    value: String,
    glyphs: Vec<GlyphOffset>,
    lines: Vec<LinkLine>,
}

impl LinkText {
    fn new(text: &pdfcraft_render::PageText, blocks: &[BlockPayload]) -> Option<Self> {
        if text.line_of.len() != text.glyphs.len() || text.space_before.len() != text.glyphs.len() {
            return None;
        }
        let mut identities = HashMap::new();
        for (index, block) in blocks.iter().enumerate() {
            if block.kind != "paragraph"
                || block.level.is_some()
                || block.style.is_some()
                || !block.links.is_empty()
                || !block.images.is_empty()
                || block.table.is_some()
                || !block.metadata.is_empty()
            {
                continue;
            }
            let Some(bbox) = block.bbox else { continue };
            identities
                .entry((block.text.as_str(), bbox.map(float_key)))
                .and_modify(|entry| *entry = None)
                .or_insert(Some(index));
        }
        let value = text.plain_text();
        let mut glyphs = Vec::with_capacity(text.glyphs.len());
        let mut lines: Vec<LinkLine> = Vec::new();
        let (mut bytes, mut chars) = (0, 0);
        for (index, glyph) in text.glyphs.iter().enumerate() {
            let new_line = index == 0 || text.line_of[index] != text.line_of[index - 1];
            // These separator positions refer to text_of's exact spaces and newlines.
            if index > 0 && (new_line || text.space_before[index]) {
                bytes += 1;
                chars += 1;
            }
            if new_line {
                lines.push(LinkLine {
                    start: index,
                    end: index,
                    block: None,
                });
            }
            let line = lines.len() - 1;
            let byte_start = bytes;
            let char_start = chars;
            bytes += glyph.text.len();
            chars += glyph.text.chars().count();
            glyphs.push(GlyphOffset {
                line,
                byte_start,
                byte_end: bytes,
                char_start,
                char_end: chars,
            });
            lines[line].end = index + 1;
        }
        if bytes != value.len() {
            return None;
        }
        for line in &mut lines {
            let boxes = text.line_rects(line.start..line.end);
            let [bbox] = boxes.as_slice() else { continue };
            let label = value.get(glyphs[line.start].byte_start..glyphs[line.end - 1].byte_end)?;
            line.block = identities
                .get(&(label, bbox.map(f64::from).map(float_key)))
                .copied()
                .flatten();
        }
        Some(Self {
            value,
            glyphs,
            lines,
        })
    }
}

fn float_key(value: f64) -> u64 {
    if value == 0.0 {
        0
    } else {
        value.to_bits()
    }
}

struct LinkCandidate {
    first_block: usize,
    last_block: usize,
    offset: usize,
    length: usize,
    label: String,
}

fn link_label(
    text: &pdfcraft_render::PageText,
    selected: &[usize],
    source: &LinkText,
    blocks: &[BlockPayload],
) -> Option<LinkCandidate> {
    let first = selected
        .iter()
        .position(|index| !text.glyphs[*index].text.trim().is_empty())?;
    let last = selected
        .iter()
        .rposition(|index| !text.glyphs[*index].text.trim().is_empty())?;
    let selected = &selected[first..=last];
    let start = *selected.first()?;
    let end = selected.last()?.checked_add(1)?;
    if selected.len() != end - start {
        return None;
    }
    let first = &source.glyphs[start];
    let last = &source.glyphs[end - 1];
    let lines = &source.lines[first.line..=last.line];
    let first_block = lines.first()?.block?;
    for (offset, line) in lines.iter().enumerate() {
        if line.block != Some(first_block + offset) {
            return None;
        }
    }
    let last_block = first_block + lines.len() - 1;
    let full_start = &source.glyphs[lines.first()?.start];
    let full_end = &source.glyphs[lines.last()?.end - 1];
    // Exact equality excludes removed table text, omitted native lines and normalization gaps.
    let joined = blocks[first_block..=last_block]
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if joined != source.value.get(full_start.byte_start..full_end.byte_end)? {
        return None;
    }
    Some(LinkCandidate {
        first_block,
        last_block,
        offset: first.char_start - full_start.char_start,
        length: last.char_end - first.char_start,
        label: source
            .value
            .get(first.byte_start..last.byte_end)?
            .to_owned(),
    })
}

fn attach_link_candidates(
    page: &mut PagePayload,
    candidates: Vec<Option<LinkCandidate>>,
) -> Result<(), EngineError> {
    let mut intervals = candidates
        .iter()
        .flatten()
        .filter(|candidate| candidate.first_block != candidate.last_block)
        .map(|candidate| (candidate.first_block, candidate.last_block))
        .collect::<Vec<_>>();
    intervals.sort_unstable();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for (start, end) in intervals {
        if let Some(previous) = groups.last_mut().filter(|previous| start <= previous.1) {
            previous.1 = previous.1.max(end);
        } else {
            groups.push((start, end));
        }
    }
    let mut remap = vec![(0, 0); page.blocks.len()];
    let mut old = std::mem::take(&mut page.blocks).into_iter().enumerate();
    let mut groups = groups.into_iter().peekable();
    while let Some((index, mut block)) = old.next() {
        let new_index = page.blocks.len();
        remap[index] = (new_index, 0);
        if groups.peek().is_some_and(|group| group.0 == index) {
            let (_, end) = groups.next().unwrap_or((index, index));
            let mut characters = block.text.chars().count();
            for (other_index, other) in old.by_ref().take(end - index) {
                remap[other_index] = (new_index, characters + 1);
                characters += 1 + other.text.chars().count();
                block.text.push('\n');
                block.text.push_str(&other.text);
                if let (Some(a), Some(b)) = (&mut block.bbox, other.bbox) {
                    *a = [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ];
                }
            }
        }
        page.blocks.push(block);
    }
    for (index, candidate) in candidates.into_iter().enumerate() {
        let Some(candidate) = candidate else { continue };
        let (block, prefix) = remap[candidate.first_block];
        let start = prefix + candidate.offset;
        let end = start + candidate.length;
        if page.blocks[block]
            .text
            .chars()
            .skip(start)
            .take(candidate.length)
            .collect::<String>()
            != candidate.label
        {
            return Err(EngineError::Malformed(
                "PDF link offset normalization mismatch".into(),
            ));
        }
        let link = &mut page.links[index];
        link.text = candidate.label;
        link.metadata.insert("start".into(), json!(start));
        link.metadata.insert("end".into(), json!(end));
        link.metadata.insert("block".into(), json!(block));
        link.metadata
            .insert("label_status".into(), json!("resolved"));
        page.blocks[block].links.push(link.clone());
    }
    Ok(())
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

#[cfg(test)]
mod warning_tests {
    use super::*;

    #[test]
    fn decoded_content_truncation_is_an_explicit_resource_failure() {
        assert!(check_content_warnings(&[]).is_ok());
        assert!(matches!(
            check_content_warnings(&[RenderWarning::ContentTruncated]),
            Err(EngineError::ResourceLimit(message)) if message.contains("partial output was discarded")
        ));
    }
}

#[cfg(test)]
mod link_tests {
    use super::*;
    use pdfcraft_render::{PageText, TextGlyph};

    fn native_text(lines: &[&[&str]]) -> PageText {
        let mut text = PageText::default();
        for (line, values) in lines.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                let x = column as f32 * 10.0;
                let y = line as f32 * 20.0;
                text.glyphs.push(TextGlyph {
                    text: (*value).into(),
                    rect: [x, y, x + 10.0, y + 10.0],
                    quad: [[x, y + 10.0], [x + 10.0, y + 10.0], [x + 10.0, y], [x, y]],
                    direction: [1.0, 0.0],
                });
                text.line_of.push(line as u32);
                text.space_before.push(false);
            }
        }
        text
    }

    fn page_info() -> pdfcraft_render::PageInfo {
        let document = pdfcraft_create::blank(200.0, 200.0, 1).unwrap();
        let bytes =
            pdfcraft_cos::write_full(&document, &pdfcraft_cos::SaveOptions::default()).unwrap();
        pdfcraft_render::inspect(Arc::new(bytes), None)
            .unwrap()
            .pages
            .remove(0)
    }

    fn annotation(index: usize, view: [f32; 4]) -> pdfcraft_render::Link {
        pdfcraft_render::Link {
            page: 0,
            index,
            rect: [view[0], 200.0 - view[3], view[2], 200.0 - view[1]],
            target: LinkTarget::Uri(format!("https://example.com/{index}")),
        }
    }

    fn plain(page: &PagePayload) -> String {
        page.blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn multiline_offsets_preserve_ligatures_unicode_spaces_and_neighboring_links() {
        let mut text = native_text(&[&["é", "ffi", "A"], &["B", "中"], &["Z"]]);
        text.space_before[2] = true;
        let mut page = PagePayload::default();
        add_text(&text, &mut page);
        let before = plain(&page);
        let source = LinkText::new(&text, &page.blocks).unwrap();
        let candidates = [vec![1, 2, 3, 4], vec![0], vec![5]]
            .map(|selected| link_label(&text, &selected, &source, &page.blocks));
        assert!(candidates.iter().all(Option::is_some));
        page.links = vec![LinkPayload::default(); 3];
        attach_link_candidates(&mut page, candidates.into()).unwrap();
        assert_eq!(plain(&page), before);
        assert_eq!(page.blocks.len(), 2);
        assert_eq!(page.blocks[0].text, "éffi A\nB中");
        assert_eq!(page.links[0].text, "ffi A\nB中");
        assert_eq!(page.links[0].metadata["start"], 1);
        assert_eq!(page.links[0].metadata["end"], 9);
        assert_eq!(page.links[1].text, "é");
        assert_eq!(page.links[1].metadata["start"], 0);
        assert_eq!(page.links[1].metadata["end"], 1);
        assert_eq!(page.links[2].metadata["block"], 1);
        assert_eq!(page.blocks[0].bbox, Some([0.0, 0.0, 30.0, 30.0]));
    }

    #[test]
    fn disjoint_labels_sharing_a_line_merge_once_and_keep_exact_offsets() {
        let text = native_text(&[&["A", "B"], &["C", "D"], &["E", "F"]]);
        let mut page = PagePayload::default();
        add_text(&text, &mut page);
        let before = plain(&page);
        let source = LinkText::new(&text, &page.blocks).unwrap();
        let candidates = [vec![0, 1, 2], vec![3, 4, 5]]
            .map(|selected| link_label(&text, &selected, &source, &page.blocks));
        page.links = vec![LinkPayload::default(); 2];
        attach_link_candidates(&mut page, candidates.into()).unwrap();
        assert_eq!(plain(&page), before);
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.links[0].text, "AB\nC");
        assert_eq!(page.links[0].metadata["start"], 0);
        assert_eq!(page.links[0].metadata["end"], 4);
        assert_eq!(page.links[1].text, "D\nEF");
        assert_eq!(page.links[1].metadata["start"], 4);
        assert_eq!(page.links[1].metadata["end"], 8);
    }

    #[test]
    fn overlapping_and_empty_annotations_do_not_merge_native_lines() {
        let text = native_text(&[&["A", "B"], &["C", "D"]]);
        let mut page = PagePayload::default();
        add_text(&text, &mut page);
        let before = serde_json::to_string(&page.blocks).unwrap();
        add_links(
            &[
                annotation(0, [0.0, 0.0, 20.0, 30.0]),
                annotation(1, [0.0, 0.0, 10.0, 10.0]),
                annotation(2, [100.0, 100.0, 110.0, 110.0]),
            ],
            &page_info(),
            0,
            Some(&text),
            &mut page,
        )
        .unwrap();
        assert_eq!(serde_json::to_string(&page.blocks).unwrap(), before);
        assert_eq!(page.links.len(), 3);
        assert!(page
            .links
            .iter()
            .all(|link| link.text.is_empty() && !link.metadata.contains_key("start")));
    }

    #[test]
    fn noncontiguous_table_and_duplicate_line_candidates_stay_unresolved() {
        let text = native_text(&[&["A", "B"], &["C", "D"]]);
        let mut page = PagePayload::default();
        add_text(&text, &mut page);
        let source = LinkText::new(&text, &page.blocks).unwrap();
        assert!(link_label(&text, &[0, 2], &source, &page.blocks).is_none());
        let original = page.blocks.clone();
        page.blocks[1] = BlockPayload {
            kind: "table".into(),
            table: Some(TablePayload::default()),
            ..Default::default()
        };
        let source = LinkText::new(&text, &page.blocks).unwrap();
        assert!(link_label(&text, &[0, 1, 2, 3], &source, &page.blocks).is_none());
        page.blocks = original;
        page.blocks.push(page.blocks[0].clone());
        let source = LinkText::new(&text, &page.blocks).unwrap();
        assert!(link_label(&text, &[0, 1], &source, &page.blocks).is_none());
    }

    #[test]
    fn annotation_work_limit_fails_before_label_allocation() {
        let mut text = native_text(&[&["A"]]);
        text.glyphs.resize(5001, text.glyphs[0].clone());
        text.line_of.resize(5001, 0);
        text.space_before.resize(5001, false);
        let links = vec![annotation(0, [0.0, 0.0, 10.0, 10.0]); 1000];
        let mut page = PagePayload::default();
        assert!(matches!(
            add_links(&links, &page_info(), 0, Some(&text), &mut page),
            Err(EngineError::ResourceLimit(_))
        ));
        assert!(page.links.is_empty());
        assert!(page.blocks.is_empty());
    }
}

#[cfg(test)]
mod clipping_tests {
    use super::*;

    #[test]
    fn native_region_renders_huge_pages_without_reducing_scale() {
        let document = pdfcraft_create::blank(14400.0, 14400.0, 1).unwrap();
        let bytes =
            pdfcraft_cos::write_full(&document, &pdfcraft_cos::SaveOptions::default()).unwrap();
        assert!(matches!(
            render(&bytes, 0, 288.0, None),
            Err(EngineError::ResourceLimit(_))
        ));
        let png = render_region(&bytes, 0, 288.0, None, (100.0, 100.0, 8.0, 8.0)).unwrap();
        let reader = png::Decoder::new(std::io::Cursor::new(png))
            .read_info()
            .unwrap();
        assert_eq!((reader.info().width, reader.info().height), (32, 32));
        assert!(matches!(
            render_region(&bytes, 0, 288.0, None, (0.0, 0.0, 1024.25, 1.0)),
            Err(EngineError::ResourceLimit(_))
        ));
    }
}
