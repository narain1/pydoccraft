//! Project the supported EPUB semantics into the existing WordCraft model.
use std::sync::Arc;

use base64::Engine;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Block, Blocks, CharProps, Document, Paragraph, Table};

use crate::{
    docx,
    model::{BlockPayload, DocumentPayload, ImagePayload, TablePayload},
    rendering, EngineError,
};

pub(super) fn render(
    book: &DocumentPayload,
    page: usize,
    dpi: f64,
    clip: Option<rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    rendering::validate_dpi(dpi)?;
    let page = book
        .pages
        .get(page)
        .ok_or_else(|| EngineError::PageIndex("EPUB chapter is out of range".into()))?;
    let mut output = Output::new(book, false)?;
    if page
        .metadata
        .get("fixed_layout")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Err(unsupported("fixed-layout spine entries"));
    }
    output.document.body = output.blocks(&page.blocks, 468.0)?;
    let layout = wordcraft_layout::layout(
        &output.document,
        &mut wordcraft_layout::LayoutCache::new(),
        &wordcraft_layout::LayoutOptions {
            view: wordcraft_layout::ViewMode::Web,
            web_width: 612.0,
            ..Default::default()
        },
    );
    if layout.pages.len() != 1 {
        return Err(unsupported(
            "the chapter could not be rendered as one reflowable surface",
        ));
    }
    if layout.pages[0].items.len() > 100_000 {
        return Err(EngineError::ResourceLimit(
            "EPUB chapter layout exceeds 100000 items".into(),
        ));
    }
    docx::render_with(&output.document, &layout, 0, dpi, clip)
}

pub(super) fn convert(book: &DocumentPayload, target: &str) -> Result<Vec<u8>, EngineError> {
    let mut output = Output::new(book, true)?;
    let mut body = Vec::new();
    for (index, page) in book.pages.iter().enumerate() {
        if page
            .metadata
            .get("fixed_layout")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        {
            return Err(unsupported("fixed-layout spine entries"));
        }
        let mut blocks = output.blocks(&page.blocks, 468.0)?;
        if index > 0 {
            if let Some(Block::Para(p)) = blocks.first_mut().map(Arc::make_mut) {
                p.props.page_break_before = Some(true);
            } else {
                let mut paragraph = Paragraph::new();
                paragraph.props.page_break_before = Some(true);
                body.push(wordcraft_doc::para_block(paragraph));
            }
        }
        body.extend(blocks);
    }
    output.document.body = body;
    let result = if target == "pdf" {
        wordcraft_pdf::export(&output.document, &wordcraft_pdf::PdfOptions::default())
            .map_err(conversion)?
    } else {
        wordcraft_docx::write(&output.document).map_err(conversion)?
    };
    crate::limits::validate_input_size(result.len())?;
    Ok(result)
}

struct Output {
    document: Document,
    preserve_links: bool,
    pixels: u64,
}

impl Output {
    fn new(book: &DocumentPayload, preserve_links: bool) -> Result<Self, EngineError> {
        if book
            .metadata
            .get("package_metadata")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry["property"] == "rendition:layout" && entry["value"] == "pre-paginated"
                })
            })
        {
            return Err(unsupported("fixed-layout EPUB packages"));
        }
        let mut document = Document::new();
        if let Some(title) = book
            .metadata
            .get("title")
            .and_then(serde_json::Value::as_str)
        {
            document.core.title = title.to_owned();
        }
        Ok(Self {
            document,
            preserve_links,
            pixels: 0,
        })
    }

    fn blocks(&mut self, blocks: &[BlockPayload], width: f32) -> Result<Blocks, EngineError> {
        let mut native = Vec::with_capacity(blocks.len());
        for block in blocks {
            if let Some(table) = &block.table {
                native.push(Arc::new(Block::Table(self.table(table, width)?)));
            } else {
                native.push(wordcraft_doc::para_block(self.paragraph(block, width)?));
            }
        }
        if native.is_empty() {
            native.push(wordcraft_doc::para_block(Paragraph::new()));
        }
        Ok(native)
    }

    fn paragraph(&mut self, block: &BlockPayload, width: f32) -> Result<Paragraph, EngineError> {
        if block.metadata.contains_key("mathml") {
            return Err(unsupported("MathML layout"));
        }
        let mut p = Paragraph::new();
        if let Some(level) = block.level {
            p.props.style = Some(format!("Heading{level}"));
        }
        let mut text_cursor = 0;
        let mut boundaries = block
            .text
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        boundaries.push(block.text.len());
        let mut links = block.links.iter().peekable();
        let mut images = block.images.iter().peekable();
        while links.peek().is_some() || images.peek().is_some() {
            let next_link = links
                .peek()
                .and_then(|link| link.metadata["start"].as_u64())
                .map(|n| n as usize);
            let next_image = images.peek().map(|image| {
                image
                    .metadata
                    .get("inline_offset")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(text_cursor, |n| n as usize)
            });
            if next_image.is_some_and(|offset| next_link.is_none_or(|start| offset <= start)) {
                let offset = next_image.unwrap_or(text_cursor);
                self.text_range(&mut p, &block.text, &boundaries, text_cursor, offset, None)?;
                text_cursor = offset;
                let image = images
                    .next()
                    .ok_or_else(|| conversion("missing mapped image"))?;
                self.image(&mut p, image, width)?;
            } else {
                let link = links
                    .next()
                    .ok_or_else(|| conversion("missing mapped link"))?;
                let start = next_link.ok_or_else(|| conversion("missing EPUB link offset"))?;
                let end = link.metadata["end"]
                    .as_u64()
                    .ok_or_else(|| conversion("missing EPUB link end"))?
                    as usize;
                if !self.preserve_links {
                    continue;
                }
                let lower = link.uri.to_ascii_lowercase();
                if start == end
                    || next_image.is_some_and(|offset| offset < end)
                    || !(lower.starts_with("https://")
                        || lower.starts_with("http://")
                        || lower.starts_with("mailto:"))
                {
                    return Err(unsupported(
                        "internal, image-only, or unsupported hyperlinks",
                    ));
                }
                self.text_range(&mut p, &block.text, &boundaries, text_cursor, start, None)?;
                self.text_range(
                    &mut p,
                    &block.text,
                    &boundaries,
                    start,
                    end,
                    Some(&link.uri),
                )?;
                text_cursor = end;
            }
        }
        self.text_range(
            &mut p,
            &block.text,
            &boundaries,
            text_cursor,
            boundaries.len() - 1,
            None,
        )?;
        Ok(p)
    }

    fn text_range(
        &self,
        para: &mut Paragraph,
        text: &str,
        offsets: &[usize],
        start: usize,
        end: usize,
        link: Option<&str>,
    ) -> Result<(), EngineError> {
        let text = offsets
            .get(start)
            .zip(offsets.get(end))
            .and_then(|(start, end)| text.get(*start..*end))
            .ok_or_else(|| conversion("invalid EPUB text range"))?;
        let props = CharProps {
            link: link.map(str::to_owned),
            ..Default::default()
        };
        let text = text.replace(['\n', '\r'], &wordcraft_doc::para::LINE_BREAK.to_string());
        if !text.is_empty() {
            para.insert_text(para.len(), &text, &props)
                .map_err(conversion)?;
        }
        Ok(())
    }

    fn image(
        &mut self,
        paragraph: &mut Paragraph,
        image: &ImagePayload,
        width: f32,
    ) -> Result<(), EngineError> {
        let ext = match image.media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            _ => {
                return Err(unsupported(
                    "preview and conversion require PNG or JPEG images",
                ))
            }
        };
        if self.preserve_links && image.metadata.contains_key("href") {
            return Err(unsupported("linked images"));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.data)
            .map_err(conversion)?;
        let (w, h) = wordcraft_render::image_size(&bytes)
            .ok_or_else(|| conversion("invalid EPUB raster image"))?;
        let pixels = u64::from(w) * u64::from(h);
        if w == 0
            || h == 0
            || w > 8192
            || h > 8192
            || pixels > 16_000_000
            || self.pixels + pixels > 32_000_000
        {
            return Err(EngineError::ResourceLimit(
                "EPUB preview exceeds its image pixel budget".into(),
            ));
        }
        if wordcraft_render::decode_pixmap(&bytes).is_none() {
            return Err(conversion("EPUB raster image failed to decode"));
        }
        self.pixels += pixels;
        let scale = (width.max(6.0) / w as f32).min(600.0 / h as f32).min(0.75);
        let media = self.document.add_media(bytes, ext);
        paragraph
            .insert_object(
                paragraph.len(),
                InlineObject::Image {
                    media,
                    w: w as f32 * scale,
                    h: h as f32 * scale,
                    alt: image.alt.clone(),
                    crop: [0.0; 4],
                    float: Default::default(),
                    ole: None,
                },
                &Default::default(),
            )
            .map_err(conversion)?;
        Ok(())
    }

    fn table(&mut self, source: &TablePayload, width: f32) -> Result<Table, EngineError> {
        let columns = source.rows.first().map_or(0, Vec::len);
        if columns == 0
            || columns > wordcraft_doc::table::MAX_COLS
            || source.cells.len().saturating_mul(columns) > 10_000
        {
            return Err(unsupported(
                "table dimensions cannot be represented by the output engine",
            ));
        }
        if self.preserve_links
            && (source.metadata.contains_key("html_column_groups")
                || source
                    .cells
                    .iter()
                    .flatten()
                    .any(|cell| cell.metadata.keys().any(|key| key.starts_with("html_"))))
        {
            return Err(unsupported(
                "table column styling or explicit HTML header associations",
            ));
        }
        let mut table = Table::new(source.cells.len(), columns, width);
        let mut merges = Vec::new();
        for (row, cells) in source.cells.iter().enumerate() {
            for cell in cells {
                let col = cell.metadata["grid_column"]
                    .as_u64()
                    .ok_or_else(|| conversion("missing EPUB table position"))?
                    as usize;
                let last_col = col + cell.col_span - 1;
                let last_row = row + cell.row_span - 1;
                let blocks =
                    self.blocks(&cell.blocks, width * cell.col_span as f32 / columns as f32)?;
                table.rows[row].cells[col].blocks = blocks;
                if row != last_row || col != last_col {
                    merges.push((row, last_row, col, last_col));
                }
            }
        }
        for (row, last_row, col, last_col) in merges {
            table.merge(row, last_row, col, last_col);
        }
        Ok(table)
    }
}

fn unsupported(reason: &str) -> EngineError {
    EngineError::Unsupported(format!("EPUB output does not support {reason}"))
}
fn conversion(error: impl std::fmt::Display) -> EngineError {
    EngineError::Conversion(format!("EPUB output: {error}"))
}
