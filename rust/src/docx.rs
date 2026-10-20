use std::collections::{HashMap, HashSet};

use base64::Engine;
use serde_json::{json, Map, Value};
use wordcraft_doc::para::{InlineObject, Paragraph, OBJ};
use wordcraft_doc::{Block, Blocks, Document, Path, StoryRef, Table};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions};

use crate::model::{
    BlockPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload, TablePayload,
};
use crate::EngineError;

pub fn read(data: &[u8], password: Option<&str>) -> Result<Document, EngineError> {
    crate::ooxml::preflight(data)?;
    wordcraft_docx::read_with_password(data, password).map_err(docx_error)
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    let document = read(data, password)?;
    let layout = wordcraft_layout::layout(
        &document,
        &mut LayoutCache::new(),
        &LayoutOptions::default(),
    );
    let mut adapter = Adapter::new(&document, &layout);
    adapter.visit_blocks(&document.body, &[]);
    adapter.add_comments();
    Ok(adapter.finish())
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
    let document = read(data, password)?;
    let layout = wordcraft_layout::layout(
        &document,
        &mut LayoutCache::new(),
        &LayoutOptions::default(),
    );
    let page = layout
        .pages
        .get(page)
        .ok_or_else(|| EngineError::Unsupported(format!("page {page} does not exist")))?;
    let rendered = wordcraft_render::render_page(
        &document,
        page,
        (dpi / 72.0) as f32,
        &wordcraft_render::RenderOptions::default(),
    );
    if rendered.width >= wordcraft_render::MAX_SIDE || rendered.height >= wordcraft_render::MAX_SIDE
    {
        return Err(EngineError::ResourceLimit(
            "DOCX render exceeds the image dimension limit".into(),
        ));
    }
    let png = rendered.to_png();
    if png.is_empty() {
        return Err(EngineError::Malformed("DOCX PNG encoding failed".into()));
    }
    Ok(png)
}

pub fn to_pdf(data: &[u8], password: Option<&str>) -> Result<Vec<u8>, EngineError> {
    let document = read(data, password)?;
    wordcraft_pdf::export(&document, &wordcraft_pdf::PdfOptions::default())
        .map_err(|error| EngineError::Conversion(format!("DOCX to PDF conversion failed: {error}")))
}

struct Adapter<'a> {
    document: &'a Document,
    layout: &'a DocLayout,
    pages: Vec<PagePayload>,
    comment_pages: HashMap<u32, usize>,
}

impl<'a> Adapter<'a> {
    fn new(document: &'a Document, layout: &'a DocLayout) -> Self {
        let page_count = layout.pages.len().max(1);
        let pages = (0..page_count)
            .map(|number| {
                let mut metadata = Map::new();
                if let Some(page) = layout.pages.get(number) {
                    metadata.insert("width".into(), json!(page.w));
                    metadata.insert("height".into(), json!(page.h));
                }
                PagePayload {
                    number,
                    metadata,
                    ..PagePayload::default()
                }
            })
            .collect();
        Self {
            document,
            layout,
            pages,
            comment_pages: HashMap::new(),
        }
    }

    fn finish(self) -> DocumentPayload {
        let mut metadata = Map::new();
        for (key, value) in [
            ("title", &self.document.core.title),
            ("subject", &self.document.core.subject),
            ("author", &self.document.core.creator),
            ("keywords", &self.document.core.keywords),
            ("description", &self.document.core.description),
        ] {
            if !value.is_empty() {
                metadata.insert(key.into(), Value::String(value.clone()));
            }
        }
        metadata.insert(
            "pagination".into(),
            Value::String("wordcraft-layout; blocks assigned to their first computed page".into()),
        );
        DocumentPayload {
            format: "docx".into(),
            metadata,
            pages: self.pages,
        }
    }

    fn visit_blocks(&mut self, blocks: &Blocks, prefix: &[u32]) {
        for (index, block) in blocks.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index as u32);
            match block.as_ref() {
                Block::Para(paragraph) => self.add_paragraph(paragraph, &Path(path)),
                Block::Table(table) => self.add_table(table, &Path(path)),
            }
        }
    }

    fn add_paragraph(&mut self, paragraph: &Paragraph, path: &Path) {
        let page_indexes = self.paragraph_pages(path);
        let segments = self.paragraph_segments(paragraph, &page_indexes);
        for (segment_index, segment) in segments.into_iter().enumerate() {
            let page_number = page_indexes
                .get(segment_index)
                .copied()
                .or_else(|| page_indexes.last().copied())
                .unwrap_or(0)
                .min(self.pages.len() - 1);
            for (id, _) in &segment.comments {
                self.comment_pages.entry(*id).or_insert(page_number);
            }
            let mut block = BlockPayload::paragraph(segment.text);
            block.links = segment.links;
            block.images = segment.images;
            block.style = paragraph.props.style.clone();
            let resolved = self.document.styles.resolve_para(&paragraph.props);
            if let Some(level) = resolved.outline_level {
                let level = level.saturating_add(1).clamp(1, 6);
                block.kind = "heading".into();
                block.level = Some(level);
                block
                    .metadata
                    .insert("outline_level".into(), Value::from(level));
            }
            if let Some(style_id) = paragraph.props.style.as_deref() {
                if let Some(style) = self.document.styles.get(style_id) {
                    block
                        .metadata
                        .insert("style_name".into(), Value::String(style.name.clone()));
                }
            }
            for link in &mut block.links {
                link.page = Some(page_number);
            }
            for image in &mut block.images {
                image.page = Some(page_number);
            }
            let page = &mut self.pages[page_number];
            page.links.extend(block.links.clone());
            page.images.extend(block.images.clone());
            if !block.text.is_empty() || !block.images.is_empty() {
                page.blocks.push(block);
            }
        }
    }

    fn add_table(&mut self, table: &Table, path: &Path) {
        let page_number = self.table_page(table, path).min(self.pages.len() - 1);
        for (row_index, row) in table.rows.iter().enumerate() {
            for (cell_index, cell) in row.cells.iter().enumerate() {
                self.collect_cell_assets(
                    &cell.blocks,
                    &[path.0.as_slice(), &[row_index as u32, cell_index as u32]].concat(),
                );
            }
        }
        let rows = table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| self.cell_text(&cell.blocks))
                    .collect()
            })
            .collect();
        let table = TablePayload {
            rows,
            page: Some(page_number),
            ..TablePayload::default()
        };
        let page = &mut self.pages[page_number];
        page.tables.push(table.clone());
        page.blocks.push(BlockPayload {
            kind: "table".into(),
            table: Some(table),
            ..BlockPayload::default()
        });
    }

    fn collect_cell_assets(&mut self, blocks: &Blocks, prefix: &[u32]) {
        for (index, block) in blocks.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index as u32);
            match block.as_ref() {
                Block::Para(paragraph) => {
                    let page_indexes = self.paragraph_pages(&Path(path));
                    for (segment_index, mut segment) in self
                        .paragraph_segments(paragraph, &page_indexes)
                        .into_iter()
                        .enumerate()
                    {
                        let page_number = page_indexes
                            .get(segment_index)
                            .copied()
                            .or_else(|| page_indexes.last().copied())
                            .unwrap_or(0)
                            .min(self.pages.len() - 1);
                        for (id, _) in segment.comments {
                            self.comment_pages.entry(id).or_insert(page_number);
                        }
                        for link in &mut segment.links {
                            link.page = Some(page_number);
                        }
                        for image in &mut segment.images {
                            image.page = Some(page_number);
                        }
                        self.pages[page_number].links.extend(segment.links);
                        self.pages[page_number].images.extend(segment.images);
                    }
                }
                Block::Table(table) => {
                    for (row_index, row) in table.rows.iter().enumerate() {
                        for (cell_index, cell) in row.cells.iter().enumerate() {
                            self.collect_cell_assets(
                                &cell.blocks,
                                &[path.as_slice(), &[row_index as u32, cell_index as u32]].concat(),
                            );
                        }
                    }
                }
            }
        }
    }

    fn paragraph_pages(&self, path: &Path) -> Vec<usize> {
        let mut pages = self
            .layout
            .index
            .get(&(StoryRef::Body, path.clone()))
            .into_iter()
            .flatten()
            .map(|(page, _)| *page)
            .collect::<Vec<_>>();
        pages.dedup();
        if pages.is_empty() {
            pages.push(0);
        }
        pages
    }

    fn table_page(&self, table: &Table, path: &Path) -> usize {
        let mut first = None;
        for (row_index, row) in table.rows.iter().enumerate() {
            for (cell_index, cell) in row.cells.iter().enumerate() {
                self.first_block_page(
                    &cell.blocks,
                    &[path.0.as_slice(), &[row_index as u32, cell_index as u32]].concat(),
                    &mut first,
                );
            }
        }
        first.unwrap_or_else(|| {
            self.layout
                .pages
                .iter()
                .position(|page| page.first_block == path.last())
                .unwrap_or(0)
        })
    }

    fn first_block_page(&self, blocks: &Blocks, prefix: &[u32], first: &mut Option<usize>) {
        for (index, block) in blocks.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index as u32);
            match block.as_ref() {
                Block::Para(_) => {
                    if let Some(page) = self
                        .layout
                        .index
                        .get(&(StoryRef::Body, Path(path)))
                        .and_then(|positions| positions.first())
                        .map(|(page, _)| *page)
                    {
                        *first = Some(first.map_or(page, |current| current.min(page)));
                    }
                }
                Block::Table(table) => {
                    for (row_index, row) in table.rows.iter().enumerate() {
                        for (cell_index, cell) in row.cells.iter().enumerate() {
                            self.first_block_page(
                                &cell.blocks,
                                &[path.as_slice(), &[row_index as u32, cell_index as u32]].concat(),
                                first,
                            );
                        }
                    }
                }
            }
        }
    }

    fn cell_text(&self, blocks: &Blocks) -> String {
        let mut values = Vec::new();
        for block in blocks {
            match block.as_ref() {
                Block::Para(paragraph) => {
                    let text = visible_text(paragraph).replace('\u{000c}', "\n");
                    if !text.is_empty() {
                        values.push(text);
                    }
                }
                Block::Table(table) => {
                    values.extend(table.rows.iter().map(|row| {
                        row.cells
                            .iter()
                            .map(|cell| self.cell_text(&cell.blocks))
                            .collect::<Vec<_>>()
                            .join("\t")
                    }));
                }
            }
        }
        values.join("\n")
    }

    fn paragraph_segments(&self, paragraph: &Paragraph, pages: &[usize]) -> Vec<Segment> {
        let mut segments = vec![Segment::default()];
        let mut segment_index = 0;
        for (offset, character) in paragraph.text.char_indices() {
            if character == '\u{000c}' {
                segments.push(Segment::default());
                segment_index += 1;
                continue;
            }
            let props = paragraph.props_of_char(offset);
            if character == OBJ {
                if let Some(object) = paragraph.object_at(offset) {
                    match object {
                        InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => {
                            segments[segment_index]
                                .comments
                                .push((*id, pages.get(segment_index).copied().unwrap_or(0)));
                        }
                        _ => {
                            self.add_object(object, &mut segments[segment_index].images);
                            let text = object.plain_text();
                            segments[segment_index].push_text(text, props.link.as_deref());
                        }
                    }
                }
            } else {
                segments[segment_index].push_char(character, props.link.as_deref());
            }
        }
        for segment in &mut segments {
            segment.finish_links();
        }
        segments
    }

    fn add_object(&self, object: &InlineObject, images: &mut Vec<ImagePayload>) {
        match object {
            InlineObject::Image {
                media, w, h, alt, ..
            } => {
                if let Some(bytes) = self.document.media.get(media) {
                    images.push(ImagePayload {
                        name: format!("images/{media}"),
                        data: base64::engine::general_purpose::STANDARD.encode(bytes.as_ref()),
                        media_type: media_type(media).into(),
                        alt: if alt.is_empty() {
                            media.clone()
                        } else {
                            alt.clone()
                        },
                        metadata: Map::from_iter([
                            ("width".into(), json!(w)),
                            ("height".into(), json!(h)),
                        ]),
                        ..ImagePayload::default()
                    });
                }
            }
            InlineObject::Group { children, .. } => {
                for child in children {
                    self.add_object(&child.obj, images);
                }
            }
            _ => {}
        }
    }

    fn add_comments(&mut self) {
        let ids = self.document.comments.keys().copied().collect::<Vec<_>>();
        for id in ids {
            let Some(comment) = self.document.comments.get(&id) else {
                continue;
            };
            let page_number = self.comment_page(id).min(self.pages.len() - 1);
            let text = self.document.plain_text(StoryRef::Part(comment.part));
            let mut metadata = Map::from_iter([
                ("comment_id".into(), Value::from(id)),
                ("author".into(), Value::String(comment.author.clone())),
                ("initials".into(), Value::String(comment.initials.clone())),
                ("date".into(), Value::String(comment.date.clone())),
                ("resolved".into(), Value::Bool(comment.resolved)),
            ]);
            if let Some(parent) = comment.parent {
                metadata.insert("parent_id".into(), Value::from(parent));
            }
            self.pages[page_number].blocks.push(BlockPayload {
                kind: "comment".into(),
                text,
                metadata,
                ..BlockPayload::default()
            });
        }
    }

    fn comment_page(&self, id: u32) -> usize {
        let mut current = Some(id);
        let mut seen = HashSet::new();
        while let Some(comment_id) = current {
            if !seen.insert(comment_id) {
                break;
            }
            if let Some(page) = self.comment_pages.get(&comment_id) {
                return *page;
            }
            current = self
                .document
                .comments
                .get(&comment_id)
                .and_then(|comment| comment.parent);
        }
        0
    }
}

#[derive(Default)]
struct Segment {
    text: String,
    links: Vec<LinkPayload>,
    images: Vec<ImagePayload>,
    comments: Vec<(u32, usize)>,
    linked_uri: Option<String>,
    linked_start: usize,
}

impl Segment {
    fn push_text(&mut self, text: &str, link: Option<&str>) {
        for character in text.chars() {
            self.push_char(character, link);
        }
    }

    fn push_char(&mut self, character: char, link: Option<&str>) {
        if self.linked_uri.as_deref() != link {
            self.finish_link();
            self.linked_uri = link.map(str::to_owned);
            self.linked_start = self.text.chars().count();
        }
        self.text.push(character);
    }

    fn finish_link(&mut self) {
        let Some(uri) = self.linked_uri.take() else {
            return;
        };
        let end = self.text.chars().count();
        if end <= self.linked_start {
            return;
        }
        let text = self
            .text
            .chars()
            .skip(self.linked_start)
            .take(end - self.linked_start)
            .collect();
        self.links.push(LinkPayload {
            uri,
            text,
            metadata: Map::from_iter([
                ("start".into(), Value::from(self.linked_start)),
                ("end".into(), Value::from(end)),
            ]),
            ..LinkPayload::default()
        });
    }

    fn finish_links(&mut self) {
        self.finish_link();
    }
}

fn media_type(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
}

fn visible_text(paragraph: &Paragraph) -> String {
    let mut output = String::new();
    for (offset, character) in paragraph.text.char_indices() {
        if character == OBJ {
            if let Some(object) = paragraph.object_at(offset) {
                output.push_str(object.plain_text());
            }
        } else {
            output.push(character);
        }
    }
    output
}

fn docx_error(error: wordcraft_docx::DocxError) -> EngineError {
    match error {
        wordcraft_docx::DocxError::PasswordRequired | wordcraft_docx::DocxError::WrongPassword => {
            EngineError::Encrypted
        }
        wordcraft_docx::DocxError::Limit(message) => EngineError::ResourceLimit(message),
        wordcraft_docx::DocxError::NotWord(message) => EngineError::Unsupported(message),
        wordcraft_docx::DocxError::NotWordXml => {
            EngineError::Unsupported("input is not a Word document".into())
        }
        wordcraft_docx::DocxError::Zip(message)
        | wordcraft_docx::DocxError::MissingPart(message)
        | wordcraft_docx::DocxError::Xml(message)
        | wordcraft_docx::DocxError::Encryption(message) => EngineError::Malformed(message),
    }
}
