//! Project the supported EPUB semantics into the existing WordCraft model.
use std::sync::Arc;

use base64::Engine;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::props::{Align, LineSpacing};
use wordcraft_doc::{Block, Blocks, CharProps, Document, Paragraph, Table};

use crate::{
    docx,
    model::{BlockPayload, DocumentPayload, ImagePayload, LinkPayload, TablePayload},
    rendering, EngineError,
};

pub(super) fn render(
    data: &[u8],
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
    let mut output = Output::new(data, book, false, dpi);
    if super::rendition::fixed(page) {
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

pub(super) fn convert(
    data: &[u8],
    book: &DocumentPayload,
    target: &str,
) -> Result<Vec<u8>, EngineError> {
    let mut output = Output::new(data, book, true, 192.0);
    if target == "pdf" {
        output.pdf_math = Some(Default::default());
    }
    let mut body = Vec::new();
    for (index, page) in book.pages.iter().enumerate() {
        if super::rendition::fixed(page) {
            return Err(unsupported("fixed-layout spine entries"));
        }
        if page.metadata.contains_key("embedded_media") {
            return Err(unsupported("semantic export of embedded audio or video"));
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
    let result = if let Some(pdf_math) = output.pdf_math.take() {
        pdf_math.export(&output.document)?
    } else {
        wordcraft_docx::write(&output.document).map_err(conversion)?
    };
    crate::limits::validate_input_size(result.len())?;
    Ok(result)
}

pub(super) struct Output<'a> {
    data: &'a [u8],
    archive: Option<super::Book<'a>>,
    pub document: Document,
    preserve_links: bool,
    pub svg_text: Option<super::rendition::text::SvgImages>,
    pub navigation: Option<std::collections::HashMap<u64, super::layout::InlineNavigation>>,
    pixels: u64,
    raster_dpi: f64,
    math_nodes_remaining: usize,
    pdf_math: Option<super::math::pdf::PdfMath>,
}

pub(super) struct RasterImage {
    pub media: String,
    pub width: f32,
    pub height: f32,
}

pub(super) fn image_bytes(image: &ImagePayload) -> Result<Vec<u8>, EngineError> {
    base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .map_err(conversion)
}

pub(super) enum Inline<'a> {
    Image(&'a ImagePayload),
    Equation {
        end: usize,
        source: &'a str,
        css: Option<&'a str>,
    },
    Link {
        end: usize,
        link: &'a LinkPayload,
    },
}

impl<'a> Output<'a> {
    pub(super) fn new(
        data: &'a [u8],
        book: &DocumentPayload,
        preserve_links: bool,
        raster_dpi: f64,
    ) -> Self {
        let mut document = Document::new();
        if let Some(title) = book
            .metadata
            .get("title")
            .and_then(serde_json::Value::as_str)
        {
            document.core.title = title.to_owned();
        }
        Self {
            data,
            archive: None,
            document,
            preserve_links,
            svg_text: None,
            navigation: None,
            pixels: 0,
            raster_dpi,
            math_nodes_remaining: 50_000,
            pdf_math: None,
        }
    }

    pub(super) fn blocks(
        &mut self,
        blocks: &[BlockPayload],
        width: f32,
    ) -> Result<Blocks, EngineError> {
        let mut native = Vec::with_capacity(blocks.len());
        for block in blocks {
            if let Some(table) = &block.table {
                native.push(Arc::new(Block::Table(self.table(table, width)?)));
            } else if let Some(block) = self.spanning_block(block, width)? {
                native.push(block);
            } else {
                native.push(wordcraft_doc::para_block(
                    self.paragraph(block, width, None)?,
                ));
            }
        }
        if native.is_empty() {
            native.push(wordcraft_doc::para_block(Paragraph::new()));
        }
        Ok(native)
    }

    fn spanning_block(
        &mut self,
        block: &BlockPayload,
        width: f32,
    ) -> Result<Option<Arc<Block>>, EngineError> {
        if !has_spanning_table(block) {
            return Ok(None);
        }
        let items = inline_items(block, self.preserve_links)?;
        if let [(start, Inline::Equation { end, source, css })] = items.as_slice() {
            let text = StyledText::new(block)?;
            if *start <= *end
                && *end < text.offsets.len()
                && text.value[..text.offsets[*start]].trim().is_empty()
                && text.value[text.offsets[*end]..].trim().is_empty()
            {
                let mut format = text.style_at(*start);
                let equation = self.equation(source, *css, &mut format)?;
                match equation {
                    super::math::Equation::SpanningTable {
                        mut table, extras, ..
                    } if extras.is_empty() || self.pdf_math.is_none() => {
                        if !extras.is_empty() {
                            table.apply_editable(&extras.editable()?)?;
                        }
                        let align = CssParagraph::from_block(block)?.and_then(|style| style.align);
                        return table
                            .export(&self.document, &format, width, align)
                            .map(|table| Some(Arc::new(Block::Table(table))));
                    }
                    equation if self.pdf_math.is_some() => {
                        let object = self.equation_object(equation, &format, width)?;
                        return self
                            .paragraph(block, width, Some((object, format)))
                            .map(wordcraft_doc::para_block)
                            .map(Some);
                    }
                    _ => return Err(unsupported("MathML composed geometry in semantic export")),
                }
            }
        }
        if self.pdf_math.is_some() {
            return Ok(None);
        }
        Err(unsupported(
            "MathML table spans mixed with other paragraph content in semantic export",
        ))
    }

    fn equation_object(
        &mut self,
        equation: super::math::Equation,
        format: &CharProps,
        width: f32,
    ) -> Result<InlineObject, EngineError> {
        match equation {
            super::math::Equation::Native {
                linear,
                display,
                math,
            } => Ok(InlineObject::Equation {
                linear,
                display,
                math,
            }),
            super::math::Equation::Composed {
                mut math,
                display,
                extras,
            } if self.pdf_math.is_none() => {
                extras.editable()?.apply(&mut math)?;
                let resolved = self.document.styles.resolve_char(None, format);
                let layout = wordcraft_layout::math::layout_equation(
                    &math,
                    "",
                    &resolved,
                    display,
                    width,
                    &Default::default(),
                    &mut 0,
                );
                let section = &self.document.last_section;
                let height = section.page_h - section.margin_top - section.margin_bottom;
                if layout.width > width || layout.ascent + layout.descent > height {
                    return Err(unsupported("composed MathML exceeds the DOCX text area"));
                }
                Ok(InlineObject::Equation {
                    linear: wordcraft_doc::math::to_linear(&math.nodes),
                    display,
                    math,
                })
            }
            equation => self
                .pdf_math
                .as_mut()
                .ok_or_else(|| unsupported("MathML composed geometry in semantic export"))?
                .object(&self.document, equation, format),
        }
    }

    fn paragraph(
        &mut self,
        block: &BlockPayload,
        width: f32,
        mut prepared: Option<(InlineObject, CharProps)>,
    ) -> Result<Paragraph, EngineError> {
        let mut p = Paragraph::new();
        if let Some(style) = CssParagraph::from_block(block)? {
            p.props.align = style.align;
            p.props.line_spacing = style.line_height;
            p.props.bidi = style.bidi;
            p.props.space_before = Some(0.0);
            p.props.space_after = Some(0.0);
            p.props.widow_control = Some(false);
        } else if let Some(level) = block.level {
            p.props.style = Some(format!("Heading{level}"));
        }
        let mut text_cursor = 0;
        let text = StyledText::new(block)?;
        let mut items = inline_items(block, self.preserve_links)?
            .into_iter()
            .peekable();
        while let Some((start, item)) = items.next() {
            if start < text_cursor {
                return Err(unsupported("overlapping EPUB links and inline objects"));
            }
            text.insert(&mut p, text_cursor, start, None)?;
            text_cursor = start;
            match item {
                Inline::Image(image) => self.image(&mut p, image, width)?,
                Inline::Equation { end, source, css } => {
                    let (object, style) = if let Some(prepared) = prepared.take() {
                        prepared
                    } else {
                        let mut style = text.style_at(start);
                        let equation = self.equation(source, css, &mut style)?;
                        (self.equation_object(equation, &style, width)?, style)
                    };
                    p.insert_object(p.len(), object, &style)
                        .map_err(conversion)?;
                    text_cursor = end;
                }
                Inline::Link { end, link } => {
                    let lower = link.uri.to_ascii_lowercase();
                    if start == end
                        || items.peek().is_some_and(|(next, _)| *next < end)
                        || !(lower.starts_with("https://")
                            || lower.starts_with("http://")
                            || lower.starts_with("mailto:"))
                    {
                        return Err(unsupported(
                            "internal, image-only, overlapping, or unsupported hyperlinks",
                        ));
                    }
                    text.insert(&mut p, start, end, Some(&link.uri))?;
                    text_cursor = end;
                }
            }
        }
        text.insert(&mut p, text_cursor, text.offsets.len() - 1, None)?;
        if let Some(navigation) = self.navigation.as_mut() {
            let projected = super::layout::InlineNavigation::new(block, &text.offsets)?;
            if !projected.is_empty() {
                navigation.insert(p.rev, projected);
            }
        }
        Ok(p)
    }

    fn image(
        &mut self,
        paragraph: &mut Paragraph,
        image: &ImagePayload,
        width: f32,
    ) -> Result<(), EngineError> {
        let RasterImage {
            media,
            width: w,
            height: h,
        } = self.load_image(image)?;
        let scale = (width.max(6.0) / w).min(600.0 / h).min(0.75);
        paragraph
            .insert_object(
                paragraph.len(),
                InlineObject::Image {
                    media,
                    w: w * scale,
                    h: h * scale,
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

    pub(super) fn equation(
        &mut self,
        source: &str,
        css: Option<&str>,
        style: &mut CharProps,
    ) -> Result<super::math::Equation, EngineError> {
        let css: Option<super::math::FormulaCss> = css
            .map(serde_json::from_str)
            .transpose()
            .map_err(conversion)?;
        if let Some(css) = &css {
            style.size = css.base.size;
            style.color = css.base.color;
            style.bold = css.base.bold;
            style.italic = css.base.italic;
        }
        let base_size = self.document.styles.resolve_char(None, style).size;
        super::math::equation(
            source,
            css.as_ref(),
            style,
            base_size,
            &mut self.math_nodes_remaining,
        )
    }

    pub(super) fn load_image(&mut self, image: &ImagePayload) -> Result<RasterImage, EngineError> {
        if self.preserve_links
            && (image.metadata.contains_key("href")
                || image.metadata.contains_key("epub_svg_links"))
        {
            return Err(unsupported("linked images"));
        }
        let bytes = image_bytes(image)?;
        if image.media_type == "image/svg+xml" {
            let base = super::svg::image_base(image)?;
            if self.archive.is_none() {
                self.archive = Some(super::Book::open(self.data)?);
            }
            let archive = self
                .archive
                .as_mut()
                .ok_or_else(|| conversion("missing EPUB archive"))?;
            let tree = super::svg::embedded(&bytes, base, archive)?;
            let width = tree.size().width();
            let height = tree.size().height();
            let media = self.rasterize_svg(&tree, width * 0.75, height * 0.75)?;
            return Ok(RasterImage {
                media,
                width,
                height,
            });
        }
        let ext = match image.media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            _ => {
                return Err(unsupported(
                    "preview and conversion require PNG, JPEG, or static SVG images",
                ))
            }
        };
        let (width, height) = wordcraft_render::image_size(&bytes)
            .ok_or_else(|| conversion("invalid EPUB raster image"))?;
        self.charge_image(width, height)?;
        if wordcraft_render::decode_pixmap(&bytes).is_none() {
            return Err(conversion("EPUB raster image failed to decode"));
        }
        let media = self.add_preview_media(bytes, ext);
        Ok(RasterImage {
            media,
            width: width as f32,
            height: height as f32,
        })
    }

    pub(super) fn rasterize_svg(
        &mut self,
        tree: &resvg::usvg::Tree,
        width: f32,
        height: f32,
    ) -> Result<String, EngineError> {
        let scale = self.raster_dpi / 72.0;
        let pixels = [width, height].map(|size| (f64::from(size) * scale).ceil().max(1.0) as u32);
        self.charge_image(pixels[0], pixels[1])?;
        let bytes = super::svg::transparent_png(tree, pixels[0], pixels[1])?;
        let media = self.add_preview_media(bytes, "png");
        if let Some(capture) = self.svg_text.as_mut() {
            capture.record(&media, tree, [width, height])?;
        }
        Ok(media)
    }

    fn add_preview_media(&mut self, bytes: Vec<u8>, ext: &str) -> String {
        if self.svg_text.is_none() {
            return self.document.add_media(bytes, ext);
        }
        // Two images can have identical pixels and different searchable text.
        let media = format!("image{}.{ext}", self.document.media.len() + 1);
        self.document.media.insert(media.clone(), Arc::new(bytes));
        media
    }

    fn charge_image(&mut self, width: u32, height: u32) -> Result<(), EngineError> {
        let pixels = u64::from(width) * u64::from(height);
        if width == 0
            || height == 0
            || width > 8192
            || height > 8192
            || pixels > 16_000_000
            || self.pixels + pixels > 32_000_000
        {
            return Err(EngineError::ResourceLimit(
                "EPUB preview exceeds its image pixel budget".into(),
            ));
        }
        self.pixels += pixels;
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

#[derive(serde::Deserialize)]
pub(super) struct CssParagraph {
    pub align: Option<Align>,
    pub line_height: Option<LineSpacing>,
    pub bidi: Option<bool>,
}

impl CssParagraph {
    pub(super) fn from_block(block: &BlockPayload) -> Result<Option<Self>, EngineError> {
        block
            .metadata
            .get("css_paragraph")
            .map(|properties| serde_json::from_value(properties.clone()).map_err(conversion))
            .transpose()
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RunStyle {
    pub chars: CharProps,
    pub families: Vec<super::css::Family>,
    pub axes: Vec<super::css::Axis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oblique: Option<f32>,
    pub optical_sizing_disabled: bool,
}

pub(super) struct StyledText<'a> {
    pub value: &'a str,
    pub offsets: Vec<usize>,
    pub spans: Vec<(usize, usize, RunStyle)>,
}

impl<'a> StyledText<'a> {
    pub(super) fn new(block: &'a BlockPayload) -> Result<Self, EngineError> {
        let mut offsets = block
            .text
            .char_indices()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        offsets.push(block.text.len());
        let spans = block
            .metadata
            .get("css_spans")
            .map(|spans| serde_json::from_value(spans.clone()))
            .transpose()
            .map_err(conversion)?
            .unwrap_or_default();
        Ok(Self {
            value: &block.text,
            offsets,
            spans,
        })
    }

    pub(super) fn style_at(&self, offset: usize) -> CharProps {
        let position = self.spans.partition_point(|(_, end, _)| *end <= offset);
        self.spans
            .get(position)
            .filter(|(start, _, _)| *start <= offset)
            .map(|(_, _, style)| style.chars.clone())
            .unwrap_or_default()
    }

    fn insert(
        &self,
        paragraph: &mut Paragraph,
        start: usize,
        end: usize,
        link: Option<&str>,
    ) -> Result<(), EngineError> {
        let mut cursor = start;
        let position = self
            .spans
            .partition_point(|(_, span_end, _)| *span_end <= start);
        for (span_start, span_end, style) in &self.spans[position..] {
            if *span_start >= end {
                break;
            }
            if cursor < *span_start {
                self.insert_run(paragraph, cursor, *span_start, link, CharProps::default())?;
                cursor = *span_start;
            }
            let stop = end.min(*span_end);
            self.insert_run(paragraph, cursor, stop, link, style.chars.clone())?;
            cursor = stop;
        }
        self.insert_run(paragraph, cursor, end, link, CharProps::default())
    }

    fn insert_run(
        &self,
        paragraph: &mut Paragraph,
        start: usize,
        end: usize,
        link: Option<&str>,
        mut props: CharProps,
    ) -> Result<(), EngineError> {
        if start == end {
            return Ok(());
        }
        let text = self
            .offsets
            .get(start)
            .zip(self.offsets.get(end))
            .and_then(|(start, end)| self.value.get(*start..*end))
            .ok_or_else(|| conversion("invalid EPUB text range"))?;
        props.link = link.map(str::to_owned);
        let text = text.replace(['\n', '\r'], &wordcraft_doc::para::LINE_BREAK.to_string());
        paragraph
            .insert_text(paragraph.len(), &text, &props)
            .map(|_| ())
            .map_err(conversion)
    }
}

pub(super) fn has_spanning_table(block: &BlockPayload) -> bool {
    block
        .metadata
        .get("mathml")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry["spanning_table"].as_bool() == Some(true))
        })
}

pub(super) fn has_composed_math(block: &BlockPayload) -> bool {
    has_spanning_table(block)
        || block
            .metadata
            .get("mathml")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["composed_math"].as_bool() == Some(true))
            })
}

pub(super) fn inline_items(
    block: &BlockPayload,
    preserve_links: bool,
) -> Result<Vec<(usize, Inline<'_>)>, EngineError> {
    let mut items = Vec::new();
    for image in &block.images {
        let offset = image
            .metadata
            .get("inline_offset")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as usize;
        items.push((offset, Inline::Image(image)));
    }
    if let Some(math) = block
        .metadata
        .get("mathml")
        .and_then(serde_json::Value::as_array)
    {
        for entry in math {
            let start = inline_offset(&entry["start"])?;
            let end = inline_offset(&entry["end"])?;
            let source = entry["source"]
                .as_str()
                .ok_or_else(|| conversion("missing MathML source"))?;
            items.push((
                start,
                Inline::Equation {
                    end,
                    source,
                    css: entry["css"].as_str(),
                },
            ));
        }
    }
    if preserve_links {
        for link in &block.links {
            let start = link
                .metadata
                .get("start")
                .ok_or_else(|| conversion("missing EPUB link offset"))?;
            let end = link
                .metadata
                .get("end")
                .ok_or_else(|| conversion("missing EPUB link end"))?;
            items.push((
                inline_offset(start)?,
                Inline::Link {
                    end: inline_offset(end)?,
                    link,
                },
            ));
        }
    }
    items.sort_by_key(|item| item.0);
    Ok(items)
}

fn inline_offset(value: &serde_json::Value) -> Result<usize, EngineError> {
    value
        .as_u64()
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or_else(|| conversion("invalid EPUB inline offset"))
}

fn unsupported(reason: &str) -> EngineError {
    EngineError::Unsupported(format!("EPUB output does not support {reason}"))
}
pub(super) fn conversion(error: impl std::fmt::Display) -> EngineError {
    EngineError::Conversion(format!("EPUB output: {error}"))
}
