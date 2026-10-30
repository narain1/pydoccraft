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
    renderer: Mutex<Option<PageRenderer>>,
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
                renderer: Mutex::new(Some(renderer)),
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
            renderer
                .as_mut()
                .ok_or_else(|| EngineError::Closed("PDF render session is closed".into()))?,
            self.page_sizes.get(page).copied(),
            page,
            dpi,
            clip,
        )
    }

    pub fn search_for(&self, page: usize, query: &str) -> Result<Vec<[f32; 4]>, EngineError> {
        let mut guard = self
            .renderer
            .lock()
            .map_err(|_| EngineError::Malformed("PDF search session is unavailable".into()))?;
        let renderer = guard
            .as_mut()
            .ok_or_else(|| EngineError::Closed("PDF render session is closed".into()))?;
        if page >= self.page_sizes.len() {
            return Err(EngineError::PageIndex("page index is out of range".into()));
        }
        check_search_query(query)?;
        if query.chars().all(char::is_whitespace) {
            return Ok(Vec::new());
        }
        let rendered = renderer.render(RenderRequest {
            page,
            kind: RequestKind::Text,
            scale: 1.0,
            ..Default::default()
        });
        check_content_warnings(&rendered.warnings)?;
        if let Some(error) = rendered.error {
            return Err(EngineError::Malformed(format!(
                "PDF search text extraction failed: {error}"
            )));
        }
        let text = rendered.text.ok_or_else(|| {
            EngineError::Malformed("PDF search text extraction returned no text".into())
        })?;
        search_text(&text, query)
    }

    pub fn stats(&self) -> Result<pdfcraft_render::RenderStats, EngineError> {
        let renderer = self
            .renderer
            .lock()
            .map_err(|_| EngineError::Malformed("PDF render session is unavailable".into()))?;
        renderer
            .as_ref()
            .map(PageRenderer::stats)
            .ok_or_else(|| EngineError::Closed("PDF render session is closed".into()))
    }

    pub fn close(&self) -> Result<(), EngineError> {
        self.renderer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(())
    }
}

const MAX_SEARCH_QUERY_BYTES: usize = 4096;
const MAX_SEARCH_GLYPHS: usize = 1_000_000;
const MAX_SEARCH_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SEARCH_WORK: usize = 1_000_000;
const MAX_SEARCH_RESULTS: usize = 10_000;

fn search_limit(message: &str) -> EngineError {
    EngineError::ResourceLimit(format!("PDF search {message}"))
}

fn check_search_query(query: &str) -> Result<(), EngineError> {
    if query.len() > MAX_SEARCH_QUERY_BYTES {
        return Err(search_limit("query exceeds 4096 UTF-8 bytes"));
    }
    Ok(())
}

fn check_search_indexes(count: usize, lines: usize, spaces: usize) -> Result<(), EngineError> {
    if count > MAX_SEARCH_GLYPHS {
        return Err(search_limit("glyph count exceeds 1000000"));
    }
    if lines != count || spaces != count {
        return Err(EngineError::Malformed(
            "PDF search glyph indexes are inconsistent".into(),
        ));
    }
    Ok(())
}

fn check_search_text(text: &pdfcraft_render::PageText) -> Result<(), EngineError> {
    let count = text.glyphs.len();
    check_search_indexes(count, text.line_of.len(), text.space_before.len())?;
    // Bound the whole UTF-8 scan before visiting any glyph characters. The original
    // native text is already interpreted; this bounds subsequent search work only.
    let mut bytes = 0usize;
    for glyph in &text.glyphs {
        bytes = bytes
            .checked_add(glyph.text.len())
            .ok_or_else(|| search_limit("source byte count overflow"))?;
        if bytes > MAX_SEARCH_SOURCE_BYTES {
            return Err(search_limit("source text exceeds 4 MiB"));
        }
    }
    // Native KMP is linear in folded characters plus query length, but allocates
    // its entire normalized character buffer before honoring the result limit.
    // Native Turkish-I/sigma substitutions cannot exceed this lowercase bound.
    // Include one possible inserted separator per glyph; UTF-8 folding can grow
    // bytes even when scalar count is unchanged (at most four bytes per scalar).
    let mut work = count.saturating_sub(1);
    for glyph in &text.glyphs {
        for c in glyph.text.chars() {
            work = work
                .checked_add(c.to_lowercase().count())
                .ok_or_else(|| search_limit("normalization work overflow"))?;
            if work > MAX_SEARCH_WORK {
                return Err(search_limit("normalization exceeds 1000000 scalar slots"));
            }
        }
    }
    Ok(())
}

fn valid_search_rectangle(rect: &[f32; 4]) -> bool {
    rect.iter().all(|value| value.is_finite()) && rect[0] <= rect[2] && rect[1] <= rect[3]
}

fn search_rectangles(
    text: &pdfcraft_render::PageText,
    hits: &[std::ops::Range<usize>],
) -> Result<Vec<[f32; 4]>, EngineError> {
    if hits.len() > MAX_SEARCH_RESULTS {
        return Err(search_limit("matches exceed 10000"));
    }
    let mut work = 0usize;
    let mut rectangles = 0usize;
    // Preflight the full result before asking line_rects to allocate any geometry.
    for hit in hits {
        if hit.start >= hit.end || hit.end > text.glyphs.len() {
            return Err(EngineError::Malformed(
                "PDF search returned an invalid glyph range".into(),
            ));
        }
        work = work
            .checked_add(hit.len())
            .ok_or_else(|| search_limit("geometry work overflow"))?;
        if work > MAX_SEARCH_WORK {
            return Err(search_limit("geometry exceeds 1000000 glyph visits"));
        }
        let mut previous = None;
        for i in hit.clone() {
            let line = text
                .line_of
                .get(i)
                .ok_or_else(|| EngineError::Malformed("PDF search line index is missing".into()))?;
            if previous != Some(line) {
                rectangles += 1;
                if rectangles > MAX_SEARCH_RESULTS {
                    return Err(search_limit("rectangles exceed 10000"));
                }
            }
            previous = Some(line);
            if !valid_search_rectangle(&text.glyphs[i].rect) {
                return Err(EngineError::Malformed(
                    "PDF search glyph rectangle is non-finite or reversed".into(),
                ));
            }
        }
    }
    let mut result = Vec::with_capacity(rectangles);
    for hit in hits {
        result.extend(text.line_rects(hit.clone()));
    }
    if result.len() != rectangles || result.iter().any(|rect| !valid_search_rectangle(rect)) {
        return Err(EngineError::Malformed(
            "PDF search rectangle projection is inconsistent".into(),
        ));
    }
    Ok(result)
}

fn search_text(
    text: &pdfcraft_render::PageText,
    query: &str,
) -> Result<Vec<[f32; 4]>, EngineError> {
    check_search_query(query)?;
    if query.chars().all(char::is_whitespace) {
        return Ok(Vec::new());
    }
    check_search_text(text)?;
    let hits = text.find_opts_limited(query, false, false, MAX_SEARCH_RESULTS + 1);
    search_rectangles(text, &hits)
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    PdfSession::open(data, password).map(|(_, payload)| payload)
}

pub(crate) const MAX_SELECTED_PAGES: usize = 10_000;

fn check_selected_count(count: usize) -> Result<(), EngineError> {
    if count > MAX_SELECTED_PAGES {
        return Err(EngineError::ResourceLimit(
            "selected page count exceeds 10000".into(),
        ));
    }
    Ok(())
}

fn selected_page(index: isize, count: usize) -> Result<usize, EngineError> {
    let page = if index < 0 {
        count.checked_sub(index.unsigned_abs())
    } else {
        Some(index as usize)
    };
    page.filter(|page| *page < count)
        .ok_or_else(|| EngineError::PageIndex("page index is out of range".into()))
}

fn check_plain_text_budget(
    text: &pdfcraft_render::PageText,
    remaining: usize,
) -> Result<(), EngineError> {
    // Native plain_text inserts at most one ASCII separator between glyphs.
    // Preflight its upper bound before asking upstream to allocate the output string.
    let mut bytes = text.glyphs.len().saturating_sub(1);
    if bytes > remaining {
        return Err(EngineError::ResourceLimit(
            "selected text exceeds the 64 MiB output limit".into(),
        ));
    }
    for glyph in &text.glyphs {
        bytes = bytes
            .checked_add(glyph.text.len())
            .filter(|bytes| *bytes <= remaining)
            .ok_or_else(|| {
                EngineError::ResourceLimit("selected text exceeds the 64 MiB output limit".into())
            })?;
    }
    Ok(())
}

pub fn extract_text(
    data: &[u8],
    password: Option<&str>,
    pages: Option<&[isize]>,
) -> Result<String, EngineError> {
    let bytes = Arc::new(data.to_vec());
    let mut renderer = PageRenderer::new(
        bytes.clone(),
        RenderConfig {
            password: password.map(Arc::from),
            ..Default::default()
        },
    );
    let page_count = renderer.page_count();
    if page_count == 0 {
        let info = inspect(bytes, password).map_err(open_error)?;
        if info.pages.is_empty() {
            return Err(EngineError::Malformed("PDF contains no pages".into()));
        }
        return Err(EngineError::Malformed(
            "PDF text renderer could not open the inspected document".into(),
        ));
    }
    let selected_count = pages.map_or(page_count, <[isize]>::len);
    check_selected_count(selected_count)?;
    for &index in pages.unwrap_or_default() {
        selected_page(index, page_count)?;
    }
    let mut output = String::new();
    for position in 0..selected_count {
        let page = match pages {
            Some(indices) => selected_page(indices[position], page_count)?,
            None => position,
        };
        let rendered = renderer.render(RenderRequest {
            page,
            kind: RequestKind::Text,
            scale: 1.0,
            ..Default::default()
        });
        check_content_warnings(&rendered.warnings)?;
        if let Some(error) = rendered.error {
            return Err(EngineError::Malformed(format!(
                "PDF text extraction failed: {error}"
            )));
        }
        let text = rendered
            .text
            .ok_or_else(|| EngineError::Malformed("PDF page has no text result".into()))?;
        let separator = usize::from(position > 0);
        let remaining = crate::limits::MAX_PAYLOAD_SIZE
            .checked_sub(output.len())
            .and_then(|remaining| remaining.checked_sub(separator))
            .ok_or_else(|| {
                EngineError::ResourceLimit("selected text exceeds the 64 MiB output limit".into())
            })?;
        check_plain_text_budget(&text, remaining)?;
        let text = text.plain_text();
        if position > 0 {
            output.push('\u{c}');
        }
        output.push_str(text.trim_end());
    }
    Ok(output)
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
    if region.is_none() {
        // Keep local rejection before any native request, independently of the
        // upstream final-allocation guard. Continuous scale limits can round
        // above the pixel budget; retain strict scale rejection as well.
        crate::rendering::validate_size(
            f64::from(pdfcraft_render::device_pixels(size[0], scale)),
            f64::from(pdfcraft_render::device_pixels(size[1], scale)),
            pdfcraft_render::MAX_SIDE as u32,
        )?;
    }
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
    let (mut rows, discarded): (Vec<Vec<PositionedText>>, Vec<Vec<PositionedText>>) = rows
        .into_iter()
        .map(|(_, mut row)| {
            row.sort_by(|a, b| a.0.total_cmp(&b.0));
            row
        })
        .partition(|row| row.len() >= 2);
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
    let valid_rect = |rect: [f64; 4]| {
        rect.iter().all(|value| value.is_finite()) && rect[0] < rect[2] && rect[1] < rect[3]
    };
    if !valid_rect(table_bbox) {
        return Ok(());
    }
    // Use exactly the existing replacement predicate, for every block kind. A rejected
    // candidate must leave the original blocks intact, including bounds outside the table.
    let inside_table = |block: &BlockPayload| {
        block.bbox.is_some_and(|bbox| {
            let center_x = (bbox[0] + bbox[2]) / 2.0;
            let center_y = (bbox[1] + bbox[3]) / 2.0;
            center_x >= table_bbox[0]
                && center_x <= table_bbox[2]
                && center_y >= table_bbox[1]
                && center_y <= table_bbox[3]
        })
    };
    let mut suppressed_bounds = table_bbox;
    for block in page.blocks.iter().filter(|block| inside_table(block)) {
        let Some(rect) = block.bbox else {
            return Ok(());
        };
        if !valid_rect(rect) {
            return Ok(());
        }
        suppressed_bounds = [
            suppressed_bounds[0].min(rect[0]),
            suppressed_bounds[1].min(rect[1]),
            suppressed_bounds[2].max(rect[2]),
            suppressed_bounds[3].max(rect[3]),
        ];
    }
    for (_, _, rect) in discarded.iter().flatten() {
        if !valid_rect(*rect) {
            return Ok(());
        }
        let a = info.user_to_view(rect[0] as f32, rect[1] as f32);
        let b = info.user_to_view(rect[2] as f32, rect[3] as f32);
        let rect = [
            f64::from(a[0].min(b[0])),
            f64::from(a[1].min(b[1])),
            f64::from(a[0].max(b[0])),
            f64::from(a[1].max(b[1])),
        ];
        if !valid_rect(rect)
            || (rect[0] < suppressed_bounds[2]
                && rect[2] > suppressed_bounds[0]
                && rect[1] < suppressed_bounds[3]
                && rect[3] > suppressed_bounds[1])
        {
            return Ok(());
        }
    }
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
        if inside_table(&block) {
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

    #[test]
    fn selective_plain_text_budget_preflights_utf8_glyphs_and_separators() {
        let text = native_text(&[&["α"], &["😀"], &["z"]]);
        assert!(check_plain_text_budget(&text, 9).is_ok());
        assert_eq!(text.plain_text().len(), 9);
        assert!(check_plain_text_budget(&text, 8).is_err());
        let expanded = "α".repeat(2048);
        let text = native_text(&[&[expanded.as_str()]]);
        assert!(check_plain_text_budget(&text, 8).is_err());
        assert!(check_plain_text_budget(&pdfcraft_render::PageText::default(), 0).is_ok());
    }

    #[test]
    fn selective_page_counts_and_negative_indexes_are_bounded() {
        assert!(check_selected_count(MAX_SELECTED_PAGES).is_ok());
        assert!(check_selected_count(MAX_SELECTED_PAGES + 1).is_err());
        assert!(check_selected_count(usize::MAX).is_err());
        assert_eq!(selected_page(-1, 3).unwrap(), 2);
        assert_eq!(selected_page(-3, 3).unwrap(), 0);
        for index in [-4, 3, isize::MIN, isize::MAX] {
            assert!(matches!(
                selected_page(index, 3),
                Err(EngineError::PageIndex(_))
            ));
        }
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

#[cfg(test)]
mod selective_open_tests {
    use super::*;

    #[test]
    fn single_parser_text_preserves_blank_pages_and_password_error_categories() {
        let mut document = pdfcraft_create::blank(200.0, 200.0, 2).unwrap();
        document
            .set_encryption(&pdfcraft_cos::NewEncryption {
                algorithm: pdfcraft_cos::Algorithm::Aes256,
                user_password: "user",
                owner_password: "owner",
                permissions: -4,
                encrypt_metadata: true,
                seed: [7; 32],
            })
            .unwrap();
        let bytes =
            pdfcraft_cos::write_full(&document, &pdfcraft_cos::SaveOptions::default()).unwrap();
        for pages in [None, Some(&[][..])] {
            assert!(matches!(
                extract_text(&bytes, None, pages),
                Err(EngineError::Encrypted)
            ));
            assert!(matches!(
                extract_text(&bytes, Some("wrong"), pages),
                Err(EngineError::Encrypted)
            ));
            assert!(matches!(
                extract_text(b"%PDF-1.7\ninvalid", None, pages),
                Err(EngineError::Malformed(_))
            ));
        }
        assert_eq!(extract_text(&bytes, Some("user"), None).unwrap(), "\u{c}");
        assert_eq!(
            extract_text(&bytes, Some("owner"), Some(&[-1, 0, -1])).unwrap(),
            "\u{c}\u{c}"
        );
        assert_eq!(extract_text(&bytes, Some("user"), Some(&[])).unwrap(), "");
        assert!(matches!(
            extract_text(&bytes, Some("user"), Some(&[2])),
            Err(EngineError::PageIndex(_))
        ));
    }
}

// Original synthetic fixtures verify the pinned native search semantics.
#[cfg(test)]
mod search_tests {
    use super::*;
    use pdfcraft_render::{PageText, TextGlyph};
    use std::io::Cursor;

    fn synthetic(lines: &[&[&str]]) -> PageText {
        let mut page = PageText::default();
        for (line, words) in lines.iter().enumerate() {
            for (column, value) in words.iter().enumerate() {
                let x = column as f32 * 10.0;
                let y = line as f32 * 20.0;
                page.glyphs.push(TextGlyph {
                    text: (*value).into(),
                    rect: [x, y, x + 10.0, y + 10.0],
                    quad: [[x, y + 10.0], [x + 10.0, y + 10.0], [x + 10.0, y], [x, y]],
                    direction: [1.0, 0.0],
                });
                page.line_of.push(line as u32);
                page.space_before.push(false);
            }
        }
        page
    }

    #[test]
    fn search_normalization_and_ligature_ownership() {
        let text = synthetic(&[&["Quick  brown"], &["fox", " STRAẞE İıI Σσς"]]);
        assert_eq!(
            text.find_opts_limited("QUICK\t brown\nfox", false, false, 10),
            vec![0..2]
        );
        assert_eq!(
            text.line_rects(0..2),
            vec![[0.0, 0.0, 10.0, 10.0], [0.0, 20.0, 10.0, 30.0]]
        );
        assert_eq!(
            text.find_opts_limited("straße", false, false, 10),
            vec![2..3]
        );
        assert!(text
            .find_opts_limited("strasse", false, false, 10)
            .is_empty());
        assert_eq!(
            text.find_opts_limited("iii σσσ", false, false, 10),
            vec![2..3]
        );
        assert!(text.find_opts_limited("", false, false, 10).is_empty());
        assert!(text.find_opts_limited("\t\n ", false, false, 10).is_empty());
        let text = synthetic(&[&["o", "ffi", "c", "e"]]);
        assert_eq!(
            text.find_opts_limited("f", false, false, 10),
            vec![1..2, 1..2]
        );
        assert_eq!(text.find_opts_limited("ffi", false, false, 10), vec![1..2]);
        assert_eq!(text.find_opts_limited("of", false, false, 10), vec![0..2]);
        let text = synthetic(&[&["aaaaa"]]);
        assert_eq!(
            text.find_opts_limited("aa", false, false, 10),
            vec![0..1, 0..1]
        );
        assert_eq!(text.find_opts_limited("a", false, false, 3).len(), 3);
        eprintln!("search probe: normalized whitespace/case; no ss/ß expansion; ligature duplicate geometry; nonoverlapping hits verified");
    }

    #[test]
    fn search_case_expansion_accounting_exhaustive_unicode() {
        let mut expansion_scalars = 0;
        let mut largest_native = 0;
        for value in 0..=0x10ffff {
            let Some(c) = char::from_u32(value) else {
                continue;
            };
            let source = c.to_string();
            let native = pdfcraft_render::text::fold_case(&source);
            let lower_bound = c.to_lowercase().count();
            assert!(native.chars().count() <= lower_bound, "U+{value:04X}");
            largest_native = largest_native.max(native.chars().count());
            if lower_bound > 1 {
                expansion_scalars += 1;
            }
            // UTF-8 can grow even when the folded scalar count does not (e.g. U+023A).
            assert!(native.len() <= 4 * lower_bound);
        }
        eprintln!("Unicode lowercase expanding scalars={expansion_scalars}, largest native fold scalar count={largest_native}");
        assert_eq!(pdfcraft_render::text::fold_case("Ⱥ"), "ⱥ");
        assert_eq!(pdfcraft_render::text::fold_case("İ"), "i");
    }

    // Original synthetic PDF fixture; the only handwritten serialization is test data.
    fn fixture(content: &str, rotation: u16, cmap: &str) -> Vec<u8> {
        let cmap = format!("/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Probe def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange {cmap} endcmap CMapName currentdict /CMap defineresource pop end end");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /CropBox [20 30 220 330] /Rotate {rotation} /UserUnit 1.25 /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream",content.len()),
            format!("<< /Length {} >>\nstream\n{cmap}\nendstream",cmap.len()),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![0];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in offsets.into_iter().skip(1) {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }

    fn text(bytes: &[u8]) -> Arc<PageText> {
        let mut renderer = PageRenderer::new(Arc::new(bytes.to_vec()), RenderConfig::default());
        let rendered = renderer.render(RenderRequest {
            page: 0,
            kind: RequestKind::Text,
            scale: 1.0,
            ..Default::default()
        });
        check_content_warnings(&rendered.warnings).unwrap();
        assert!(rendered.error.is_none(), "{:?}", rendered.error);
        rendered.text.unwrap()
    }

    fn raster(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
        let png = render(bytes, 0, 144.0, None).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(frame.color_type, png::ColorType::Rgba);
        pixels.truncate(frame.buffer_size());
        (frame.width as usize, frame.height as usize, pixels)
    }

    #[test]
    fn search_native_pdf_ligature_geometry_and_visible_pixels_all_rotations() {
        for rotation in [0, 90, 180, 270] {
            let bytes = fixture(
                "BT /F1 20 Tf 1 0 0 1 60 260 Tm (oWce) Tj ET",
                rotation,
                "1 beginbfchar <57> <006600660069> endbfchar",
            );
            let control = fixture("BT /F1 20 Tf 1 0 0 1 60 260 Tm (oWce) Tj ET", rotation, "");
            let native = text(&bytes);
            assert_eq!(native.plain_text(), "office");
            assert_eq!(
                native.find_opts_limited("F", false, false, 10),
                vec![1..2, 1..2]
            );
            let rects =
                native.line_rects(native.find_opts_limited("ffi", false, false, 10)[0].clone());
            assert_eq!(rects.len(), 1);
            assert_eq!(rects, text(&control).line_rects(1..2));
            let info = inspect(Arc::new(bytes.clone()), None).unwrap();
            // Native glyph-em rectangle in PDF user units is [71.12,256,90.0,276].
            // Compare public user-to-view projection, including crop origin, rotation and UserUnit.
            let page = &info.pages[0];
            let points = [[71.12, 256.0], [90.0, 256.0], [71.12, 276.0], [90.0, 276.0]];
            let view: Vec<_> = points
                .into_iter()
                .map(|p| page.user_to_view(p[0], p[1]))
                .collect();
            let expected = [
                view.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
                view.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                view.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max),
                view.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max),
            ];
            for (actual, want) in rects[0].into_iter().zip(expected) {
                assert!(
                    (actual - want).abs() < 0.03,
                    "rotation {rotation}: {rects:?} expected {expected:?}"
                );
            }
            let (width, height, pixels) = raster(&bytes);
            assert_eq!(raster(&control), (width, height, pixels.clone()));
            let r = rects[0];
            let mut ink = 0;
            for y in
                (r[1] * 2.0).floor().max(0.0) as usize..((r[3] * 2.0).ceil() as usize).min(height)
            {
                for x in (r[0] * 2.0).floor().max(0.0) as usize
                    ..((r[2] * 2.0).ceil() as usize).min(width)
                {
                    let p = &pixels[(y * width + x) * 4..(y * width + x) * 4 + 4];
                    if p[0] < 100 && p[1] < 100 && p[2] < 100 && p[3] > 0 {
                        ink += 1;
                    }
                }
            }
            assert!(
                ink > 20,
                "rotation {rotation}: {ink} visible pixels in {r:?}"
            );
            eprintln!("rotation={rotation} ffi rectangle={r:?}, visible dark pixels={ink}");
        }
    }

    #[test]
    fn search_clipped_and_invisible_text_is_still_searchable() {
        let clipped = fixture(
            "q 0 0 1 1 re W n BT /F1 20 Tf 1 0 0 1 60 260 Tm (Hidden) Tj ET Q",
            0,
            "",
        );
        let invisible = fixture("BT /F1 20 Tf 3 Tr 1 0 0 1 60 260 Tm (Hidden) Tj ET", 0, "");
        let blank = fixture("", 0, "");
        for bytes in [&clipped, &invisible] {
            assert_eq!(
                text(bytes).find_opts_limited("hidden", false, false, 10),
                vec![0..6]
            );
            assert_eq!(raster(bytes), raster(&blank));
        }
    }
    #[test]
    fn search_real_multiline_native_order_and_empty_page() {
        for rotation in [0, 90, 180, 270] {
            let bytes = fixture(
                "BT /F1 16 Tf 1 0 0 1 60 260 Tm (Quick  brown) Tj 0 -24 Td (fox) Tj ET",
                rotation,
                "",
            );
            let native = text(&bytes);
            assert_eq!(native.plain_text(), "Quick  brown\nfox");
            let hits = native.find_opts_limited("QUICK\tbrown\nfox", false, false, 10);
            assert_eq!(hits.len(), 1);
            let rects = native.line_rects(hits[0].clone());
            assert_eq!(rects.len(), 2, "rotation {rotation}: {rects:?}");
            eprintln!("multiline rotation={rotation}: {rects:?}");
        }
        let blank = text(&fixture("", 0, ""));
        assert!(blank.find_opts_limited("any", false, false, 10).is_empty());
    }
    #[test]
    fn search_preflights_query_source_indexes_and_normalized_work() {
        assert!(check_search_query(&"a".repeat(MAX_SEARCH_QUERY_BYTES)).is_ok());
        assert!(matches!(
            check_search_query(&"a".repeat(MAX_SEARCH_QUERY_BYTES + 1)),
            Err(EngineError::ResourceLimit(_))
        ));
        assert!(check_search_query(&"α".repeat(MAX_SEARCH_QUERY_BYTES / 2)).is_ok());
        assert!(check_search_query(&"α".repeat(MAX_SEARCH_QUERY_BYTES / 2 + 1)).is_err());
        assert!(
            check_search_indexes(MAX_SEARCH_GLYPHS, MAX_SEARCH_GLYPHS, MAX_SEARCH_GLYPHS).is_ok()
        );
        assert!(matches!(
            check_search_indexes(MAX_SEARCH_GLYPHS + 1, 0, 0),
            Err(EngineError::ResourceLimit(_))
        ));
        assert!(matches!(
            check_search_indexes(1, 0, 1),
            Err(EngineError::Malformed(_))
        ));
        let text = synthetic(&[&[&"a".repeat(MAX_SEARCH_WORK)]]);
        assert!(check_search_text(&text).is_ok());
        assert_eq!(
            search_text(&text, &format!("{}b", "a".repeat(4095))).unwrap(),
            Vec::<[f32; 4]>::new()
        );
        let over = synthetic(&[&[&"a".repeat(MAX_SEARCH_WORK + 1)]]);
        assert!(
            matches!(check_search_text(&over),Err(EngineError::ResourceLimit(message)) if message.contains("normalization"))
        );
        // The byte scan must win over normalization failure once the raw cap is crossed.
        for extra in [0, 1] {
            let text = synthetic(&[&[&"a".repeat(MAX_SEARCH_SOURCE_BYTES + extra)]]);
            let Err(EngineError::ResourceLimit(message)) = check_search_text(&text) else {
                panic!("expected budget failure")
            };
            assert!(message.contains(if extra == 0 {
                "normalization"
            } else {
                "source text"
            }));
        }
        let mut malformed = synthetic(&[&["a"]]);
        malformed.line_of.clear();
        assert!(matches!(
            search_text(&malformed, "a"),
            Err(EngineError::Malformed(_))
        ));
        let separators = synthetic(&[&["a"], &["a"]]);
        assert_eq!(search_text(&separators, "a a").unwrap().len(), 2);
    }

    #[test]
    fn search_result_rectangle_and_geometry_work_limits_do_not_truncate() {
        let text = synthetic(&[&[&"a".repeat(MAX_SEARCH_RESULTS)]]);
        assert_eq!(search_text(&text, "a").unwrap().len(), MAX_SEARCH_RESULTS);
        let over = synthetic(&[&[&"a".repeat(MAX_SEARCH_RESULTS + 1)]]);
        assert!(
            matches!(search_text(&over,"a"),Err(EngineError::ResourceLimit(message)) if message.contains("matches"))
        );
        let values = vec!["a"; MAX_SEARCH_RESULTS + 1];
        let mut text = synthetic(&[&values]);
        text.line_of = (0..=MAX_SEARCH_RESULTS as u32).collect();
        assert_eq!(
            search_rectangles(&text, std::slice::from_ref(&(0..MAX_SEARCH_RESULTS)))
                .unwrap()
                .len(),
            MAX_SEARCH_RESULTS
        );
        assert!(
            matches!(search_rectangles(&text,std::slice::from_ref(&(0..MAX_SEARCH_RESULTS+1))),Err(EngineError::ResourceLimit(message)) if message.contains("rectangles"))
        );
        let values = vec!["a"; 1001];
        let text = synthetic(&[&values]);
        let mut hits = vec![0..1000; 1000];
        assert_eq!(search_rectangles(&text, &hits).unwrap().len(), 1000);
        hits[999] = 0..1001;
        assert!(
            matches!(search_rectangles(&text,&hits),Err(EngineError::ResourceLimit(message)) if message.contains("glyph visits"))
        );
        assert!(matches!(
            search_rectangles(&text, std::slice::from_ref(&(0..1002))),
            Err(EngineError::Malformed(_))
        ));
        let mut invalid = synthetic(&[&["a"]]);
        for rect in [
            [f32::INFINITY, 0.0, 10.0, 10.0],
            [10.0, 0.0, 0.0, 10.0],
            [0.0, 10.0, 10.0, 0.0],
        ] {
            invalid.glyphs[0].rect = rect;
            assert!(matches!(
                search_text(&invalid, "a"),
                Err(EngineError::Malformed(_))
            ));
        }
        invalid.glyphs[0].rect = [-30.0, -20.0, -10.0, -10.0];
        assert_eq!(
            search_text(&invalid, "a").unwrap(),
            vec![invalid.glyphs[0].rect]
        );
    }

    #[test]
    fn search_session_preserves_parser_and_avoids_raster_or_blank_interpretation() {
        let bytes = fixture("BT /F1 20 Tf 1 0 0 1 60 260 Tm (Search) Tj ET", 0, "");
        let (session, _) = PdfSession::open(&bytes, None).unwrap();
        let before = session.stats().unwrap();
        assert!(session.search_for(0, "\t\n ").unwrap().is_empty());
        assert!(matches!(
            session.search_for(1, ""),
            Err(EngineError::PageIndex(_))
        ));
        assert!(matches!(
            session.search_for(0, &"x".repeat(4097)),
            Err(EngineError::ResourceLimit(_))
        ));
        assert_eq!(
            session.stats().unwrap().render_requests,
            before.render_requests
        );
        let first = session.search_for(0, "SEARCH").unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(session.search_for(0, "search").unwrap(), first);
        let after = session.stats().unwrap();
        assert_eq!(after.parser_builds, before.parser_builds);
        assert_eq!(after.rasterizations, before.rasterizations);
        assert_eq!(after.render_requests, before.render_requests + 2);
        session.close().unwrap();
        assert!(matches!(
            session.search_for(0, ""),
            Err(EngineError::Closed(_))
        ));
        assert_eq!(first.len(), 1);
    }
}

#[cfg(test)]
mod raster_limit_tests {
    use super::*;

    fn fixture(width: f32, height: f32, unit: f32, rotation: u16) -> Vec<u8> {
        // Original empty-page fixture; offset CropBox is smaller than MediaBox.
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /CropBox [10 20 {} {}] /UserUnit {unit} /Rotate {rotation} /Resources << >> /Contents 4 0 R >>", width + 20.0, height + 40.0, width + 10.0, height + 20.0),
            "<< /Length 0 >>\nstream\n\nendstream".to_owned(),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        out
    }

    #[test]
    fn native_continuous_area_can_round_above_integer_pixel_budget() {
        // Arithmetic only: never allocate a 64-million-pixel acceptance fixture.
        let (w, h) = (8000.5_f32, 7999.5_f32);
        assert_eq!(w * h, 64_000_000.0);
        assert_eq!(pdfcraft_render::effective_scale(w, h, 1.0), 1.0);
        let width = pdfcraft_render::device_pixels(w, 1.0);
        let height = pdfcraft_render::device_pixels(h, 1.0);
        assert_eq!((width, height), (8001, 8000));
        assert_eq!(u64::from(width) * u64::from(height), 64_008_000);
        assert!(matches!(
            crate::rendering::validate_size(
                f64::from(width),
                f64::from(height),
                pdfcraft_render::MAX_SIDE as u32
            ),
            Err(EngineError::ResourceLimit(_))
        ));
        // Exact cap remains accepted by the pure helper without rasterization.
        assert!(
            crate::rendering::validate_size(8000.0, 8000.0, pdfcraft_render::MAX_SIDE as u32)
                .is_ok()
        );
        assert!(
            crate::rendering::validate_size(8192.0, 1.0, pdfcraft_render::MAX_SIDE as u32).is_ok()
        );
    }

    #[test]
    #[cfg(feature = "python")]
    fn linked_native_typed_placement_preserves_identity_order_and_reopened_paint() {
        use crate::author_pdf::{add_content_with_placement, Placement};
        use pdfcraft_cos::{
            write_full, write_incremental, Dict, Document, Object, SaveOptions, Stream,
        };
        use pdfcraft_edit::{
            add_content, delete_content, list_added, update_content, AddedImage, AddedText, Content,
        };
        let mut doc = Document::open(Arc::new(fixture(100.0, 120.0, 1.0, 0))).unwrap();
        let image = |doc: &mut Document, rgb: [u8; 3]| {
            let mut dict = Dict::new();
            dict.set(b"Type".to_vec(), Object::name("XObject"));
            dict.set(b"Subtype".to_vec(), Object::name("Image"));
            dict.set(b"Width".to_vec(), Object::Int(1));
            dict.set(b"Height".to_vec(), Object::Int(1));
            dict.set(b"ColorSpace".to_vec(), Object::name("DeviceRGB"));
            dict.set(b"BitsPerComponent".to_vec(), Object::Int(8));
            doc.add(Object::Stream(Stream::from_raw(dict, rgb.to_vec())))
        };
        let text = |label: &str, rect| {
            Content::Text(AddedText {
                text: label.into(),
                rect,
                bold: true,
                ..AddedText::default()
            })
        };
        let blue = image(&mut doc, [0, 0, 255]);
        let blue_item = Content::Image(AddedImage::new([40.0, 40.0, 80.0, 80.0], blue));
        assert_eq!(add_content(&mut doc, 0, &blue_item).unwrap(), 0);
        assert_eq!(
            add_content(&mut doc, 0, &text("Original", [5.0, 96.0, 95.0, 112.0])).unwrap(),
            1
        );
        let original_refs: Vec<_> = list_added(&doc).iter().map(|a| a.obj).collect();
        pdfcraft_annot::links::add(
            &mut doc,
            0,
            [15.0, 25.0, 45.0, 35.0],
            &pdfcraft_annot::links::LinkAction::Uri("https://example.com/retained".into()),
            &pdfcraft_annot::links::LinkStyle::default(),
        )
        .unwrap();
        let red = image(&mut doc, [255, 0, 0]);
        let red_item = Content::Image(AddedImage::new([20.0, 20.0, 90.0, 90.0], red));
        assert_eq!(
            add_content_with_placement(&mut doc, 0, &red_item, Placement::Underlay).unwrap(),
            0
        );
        let red_ref = list_added(&doc)[0].obj;
        assert_eq!(
            add_content_with_placement(
                &mut doc,
                0,
                &text("New", [5.0, 0.0, 95.0, 16.0]),
                Placement::Underlay
            )
            .unwrap(),
            0
        );
        let text_ref = list_added(&doc)[0].obj;
        let expected = [text_ref, red_ref, original_refs[0], original_refs[1]];
        assert_eq!(
            list_added(&doc).iter().map(|a| a.obj).collect::<Vec<_>>(),
            expected
        );
        update_content(&mut doc, 0, 0, &text("Updated", [5.0, 0.0, 95.0, 16.0])).unwrap();
        update_content(&mut doc, 0, 3, &text("Original", [5.0, 96.0, 95.0, 112.0])).unwrap();
        assert_eq!(
            list_added(&doc).iter().map(|a| a.obj).collect::<Vec<_>>(),
            expected
        );

        let verify = |doc: &Document, overlay: bool, labels: bool| {
            for (incremental, bytes) in [
                (
                    true,
                    write_incremental(doc, &SaveOptions::default()).unwrap(),
                ),
                (false, write_full(doc, &SaveOptions::default()).unwrap()),
            ] {
                let reopened = Document::open(Arc::new(bytes.clone())).unwrap();
                let items = list_added(&reopened);
                assert_eq!(items.len(), if labels { 4 } else { 3 });
                if incremental && labels {
                    assert_eq!(items.iter().map(|a| a.obj).collect::<Vec<_>>(), expected);
                }
                let info = pdfcraft_render::inspect(Arc::new(bytes.clone()), None).unwrap();
                assert_eq!(info.links.len(), 1);
                assert_eq!(
                    info.links[0].target,
                    pdfcraft_render::LinkTarget::Uri("https://example.com/retained".into())
                );
                let mut renderer = PageRenderer::new(Arc::new(bytes), RenderConfig::default());
                let extracted = renderer
                    .filtered_text(0, Default::default())
                    .unwrap()
                    .plain_text();
                assert_eq!(extracted.matches("Original").count(), usize::from(labels));
                assert_eq!(extracted.matches("Updated").count(), usize::from(labels));
                assert!(!extracted.contains("New"));
                for scale in [1.0_f32, 2.0] {
                    let rendered = renderer.render(RenderRequest {
                        scale,
                        ..Default::default()
                    });
                    assert!(rendered.error.is_none(), "{:?}", rendered.error);
                    assert_eq!(
                        (rendered.width, rendered.height),
                        ((100.0 * scale) as u32, (120.0 * scale) as u32)
                    );
                    assert_eq!(
                        rendered.rgba.len(),
                        rendered.width as usize * rendered.height as usize * 4
                    );
                    let pixel = |x: usize, y: usize| {
                        let factor = scale as usize;
                        let at = (((120 - y) * factor) * rendered.width as usize + x * factor) * 4;
                        &rendered.rgba[at..at + 4]
                    };
                    assert_eq!(pixel(30, 30), &[255, 0, 0, 255]);
                    assert_eq!(
                        pixel(60, 60),
                        if overlay {
                            &[255, 0, 0, 255]
                        } else {
                            &[0, 0, 255, 255]
                        }
                    );
                    assert_eq!(pixel(5, 60), &[255, 255, 255, 255]);
                    if labels {
                        for (top, bottom) in [(8, 22), (102, 119)] {
                            let mut ink = 0;
                            for y in top * scale as usize..bottom * scale as usize {
                                for x in 5 * scale as usize..95 * scale as usize {
                                    let at = (y * rendered.width as usize + x) * 4;
                                    ink += usize::from(
                                        rendered.rgba[at..at + 3].iter().all(|v| *v < 80),
                                    );
                                }
                            }
                            assert!(ink > 20, "actual label ink must survive save and reopen");
                        }
                    }
                }
            }
        };
        verify(&doc, false, true);
        delete_content(&mut doc, 0, 3).unwrap();
        delete_content(&mut doc, 0, 0).unwrap();
        assert_eq!(
            list_added(&doc).iter().map(|a| a.obj).collect::<Vec<_>>(),
            [red_ref, original_refs[0]]
        );
        assert_eq!(add_content(&mut doc, 0, &red_item).unwrap(), 2);
        assert_eq!(list_added(&doc)[0].obj, red_ref);
        assert_eq!(list_added(&doc)[1].obj, original_refs[0]);
        verify(&doc, true, false);
    }

    #[test]
    fn linked_native_integer_budget_rejects_before_allocation_and_recovers() {
        // Safety precondition: never issue this large direct request to an older
        // backend. Its original failure is proved upstream with an intercepted
        // allocator, not by allocating a 256MB raster here. The pinned upstream commit contains
        // the exact raster.rs/pixels.rs correction (PR #3948, released in v0.13.0).
        let entries: Vec<_> = include_str!("../../Cargo.lock")
            .split("[[package]]")
            .filter(|entry| {
                entry
                    .lines()
                    .any(|line| line.trim() == "name = \"pdfcraft-render\"")
            })
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].lines().any(|line| line.trim() ==
            "source = \"git+https://github.com/storytold/pdfcraft?rev=64e0868b844e0230027a493c90d6e4bdc7af086f#64e0868b844e0230027a493c90d6e4bdc7af086f\""));
        for rotation in [0, 90, 180, 270] {
            let raw = fixture(4000.25, 3999.75, 2.0, rotation);
            for reject_oversize in [false, true] {
                let mut renderer = PageRenderer::new(
                    Arc::new(raw.clone()),
                    RenderConfig {
                        reject_oversize,
                        ..Default::default()
                    },
                );
                assert_eq!(renderer.page_count(), 1);
                let dimensions = if rotation % 180 == 0 {
                    "8001x8000"
                } else {
                    "8000x8001"
                };
                let expected =
                    format!("page 1 raster size {dimensions} exceeds renderer pixel limit");
                let request = RenderRequest {
                    scale: 1.0,
                    ..Default::default()
                };
                let failed = renderer.render(request);
                assert_eq!(failed.error.as_deref(), Some(expected.as_str()));
                assert_eq!((failed.width, failed.height), (0, 0));
                assert!(failed.rgba.is_empty());
                assert_eq!(renderer.stats().render_requests, 1);
                assert_eq!(renderer.stats().parser_builds, 1);
                let tile = renderer.render(RenderRequest {
                    tile: Some(pdfcraft_render::Tile {
                        x: 0,
                        y: 0,
                        w: 3,
                        h: 2,
                    }),
                    ..request
                });
                assert!(tile.error.is_none(), "{:?}", tile.error);
                assert_eq!((tile.width, tile.height), (3, 2));
                assert_eq!(&*tile.rgba, &[255; 24]);
                let repeated = renderer.render(request);
                assert_eq!(repeated.error.as_deref(), Some(expected.as_str()));
                assert_eq!((repeated.width, repeated.height), (0, 0));
                assert!(repeated.rgba.is_empty());
                assert_eq!(renderer.stats().render_requests, 3);
                assert_eq!(renderer.stats().parser_builds, 1);
            }
            let (session, _) = PdfSession::open(&raw, None).unwrap();
            let before = session.stats().unwrap();
            assert!(matches!(
                session.render(0, 72.0, None),
                Err(EngineError::ResourceLimit(_))
            ));
            assert_eq!(session.stats().unwrap(), before);
        }
    }

    #[test]
    fn rounded_full_raster_rejects_before_native_request_after_small_cached_render() {
        for rotation in [0, 90, 180, 270] {
            // UserUnit 2 produces the same 8000.5 x 7999.5 displayed-point dimensions;
            // only the 1 DPI control and 1x1/17x17 clips below are actually rasterized.
            let raw = fixture(4000.25, 3999.75, 2.0, rotation);
            let (session, payload) = PdfSession::open(&raw, None).unwrap();
            let expected = if rotation % 180 == 0 {
                [8000.5, 7999.5]
            } else {
                [7999.5, 8000.5]
            };
            assert_eq!(session.page_sizes[0], expected);
            assert_eq!(payload.pages.len(), 1);
            assert!(session.render(0, 1.0, None).is_ok());
            let before = session.stats().unwrap();
            for _ in 0..2 {
                assert!(matches!(
                    session.render(0, 72.0, None),
                    Err(EngineError::ResourceLimit(_))
                ));
                let after = session.stats().unwrap();
                assert_eq!(after.render_requests, before.render_requests);
                assert_eq!(after.rasterizations, before.rasterizations);
            }
            assert!(matches!(
                render(&raw, 0, 72.0, None),
                Err(EngineError::ResourceLimit(_))
            ));
            assert!(session.render(0, 72.0, Some((0.0, 0.0, 1.0, 1.0))).is_ok());
            assert!(session
                .render(0, 1200.0, Some((0.0, 0.0, 1.0, 1.0)))
                .is_ok());
            session.close().unwrap();
            assert!(matches!(
                session.render(0, 72.0, None),
                Err(EngineError::Closed(_))
            ));
        }
    }
}

#[cfg(test)]
#[path = "pdf_table_preservation_tests.rs"]
mod table_preservation_tests;
