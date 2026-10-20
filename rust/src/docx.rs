use std::collections::{HashMap, HashSet};

use base64::Engine;
use serde_json::{json, Map, Value};
use wordcraft_doc::para::{InlineObject, Paragraph, OBJ};
use wordcraft_doc::props::VMerge;
use wordcraft_doc::{Block, Blocks, Document, PartKind, Path, StoryRef, Table};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions};

use crate::model::{
    BlockPayload, CellPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload,
    StoryPayload, TablePayload,
};
use crate::EngineError;

pub fn read(data: &[u8], password: Option<&str>) -> Result<Document, EngineError> {
    crate::ooxml::preflight(data)?;
    wordcraft_docx::read_with_password(data, password).map_err(docx_error)
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    let document = read(data, password)?;
    validate_stories(&document)?;
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
    crate::rendering::validate_dpi(dpi)?;
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
    let scale = (dpi / 72.0) as f32;
    // Reject oversized requests before the upstream rasterizer allocates or clamps them.
    crate::rendering::validate_size(
        f64::from(page.w * scale),
        f64::from(page.h * scale),
        wordcraft_render::MAX_SIDE,
    )?;
    let rendered = wordcraft_render::render_page(
        &document,
        page,
        scale,
        &wordcraft_render::RenderOptions::default(),
    );
    crate::rendering::encode_png(rendered.width, rendered.height, &rendered.pixels, dpi)
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

fn validate_stories(document: &Document) -> Result<(), EngineError> {
    if document.parts.len() > 10_000 {
        return Err(EngineError::ResourceLimit(
            "DOCX exceeds 10000 auxiliary stories".into(),
        ));
    }
    let separators = separator_stories(document);
    let roots = std::iter::once(&document.body)
        .chain(document.parts.values().map(|part| &part.blocks))
        .chain(separators.iter().map(|(_, _, blocks)| *blocks));
    let mut count = 0usize;
    let mut text_bytes = 0usize;
    let mut image_bytes = 0usize;
    for root in roots {
        validate_table_depth(root)?;
        let mut pending = vec![root];
        while let Some(blocks) = pending.pop() {
            for block in blocks {
                count = count.saturating_add(1);
                match block.as_ref() {
                    Block::Table(table) => {
                        for row in &table.rows {
                            for cell in &row.cells {
                                pending.push(&cell.blocks);
                            }
                        }
                    }
                    Block::Para(paragraph) => {
                        text_bytes = text_bytes.saturating_add(paragraph.text.len());
                        let mut objects: Vec<_> = paragraph
                            .objects
                            .iter()
                            .map(|object| (object, 0usize))
                            .collect();
                        while let Some((object, depth)) = objects.pop() {
                            count = count.saturating_add(1);
                            match object {
                                InlineObject::Image { media, .. } => {
                                    image_bytes = image_bytes.saturating_add(
                                        document.media.get(media).map_or(0, |data| data.len()),
                                    );
                                }
                                InlineObject::Group { children, .. } => {
                                    if depth >= 64 {
                                        return Err(EngineError::ResourceLimit(
                                            "DOCX grouped objects exceed 64 levels".into(),
                                        ));
                                    }
                                    objects.extend(
                                        children.iter().map(|child| (&child.obj, depth + 1)),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
                if count > 100_000
                    || text_bytes > 32 * 1024 * 1024
                    || image_bytes > 64 * 1024 * 1024
                {
                    return Err(EngineError::ResourceLimit("DOCX extraction exceeds 100000 blocks/objects, 32 MiB text or 64 MiB referenced images".into()));
                }
            }
        }
    }
    Ok(())
}

fn separator_stories(document: &Document) -> Vec<(&'static str, &'static str, &Blocks)> {
    let mut stories = Vec::new();
    for (kind, separators) in [
        ("footnote_separator", &document.footnote_separators),
        ("endnote_separator", &document.endnote_separators),
    ] {
        for (role, blocks) in [
            ("separator", &separators.separator),
            ("continuation_separator", &separators.continuation_separator),
            ("continuation_notice", &separators.continuation_notice),
        ] {
            if let Some(blocks) = blocks {
                stories.push((kind, role, blocks));
            }
        }
    }
    stories
}

// Bound recursive payload depth before layout and normalization. Each nested table adds
// several JSON container levels; this also keeps the standard JSON decoder usable.
fn validate_table_depth(blocks: &Blocks) -> Result<(), EngineError> {
    let mut pending = vec![(blocks, 0_usize)];
    while let Some((blocks, depth)) = pending.pop() {
        for block in blocks {
            if let Block::Table(table) = block.as_ref() {
                if depth >= 16 {
                    return Err(EngineError::ResourceLimit(
                        "DOCX table nesting exceeds 16 levels".into(),
                    ));
                }
                for row in &table.rows {
                    let columns = row.cells.iter().fold(
                        (row.props.grid_before as usize)
                            .saturating_add(row.props.grid_after as usize),
                        |count, cell| count.saturating_add(cell.props.span.max(1) as usize),
                    );
                    if columns > wordcraft_doc::table::MAX_COLS {
                        return Err(EngineError::ResourceLimit(
                            "DOCX table grid exceeds 63 columns".into(),
                        ));
                    }
                    for cell in &row.cells {
                        pending.push((&cell.blocks, depth + 1));
                    }
                }
            }
        }
    }
    Ok(())
}

impl<'a> Adapter<'a> {
    fn stories(&self) -> Vec<StoryPayload> {
        let comments: HashMap<_, _> = self
            .document
            .comments
            .iter()
            .map(|(id, comment)| (comment.part, (id, comment)))
            .collect();
        let mut stories: Vec<_> = self
            .document
            .parts
            .iter()
            .map(|(id, part)| {
                let kind = match part.kind {
                    PartKind::Header => "header",
                    PartKind::Footer => "footer",
                    PartKind::Footnote => "footnote",
                    PartKind::Endnote => "endnote",
                    PartKind::Comment => "comment",
                    PartKind::TextBox => "text_box",
                };
                let source = Map::from_iter([("story_part_id".into(), json!(id))]);
                let mut metadata = Map::from_iter([
                    ("part_id".into(), json!(id)),
                    (
                        "ordering".into(),
                        json!("upstream-part-id; source order within story"),
                    ),
                ]);
                if part.kind == PartKind::TextBox {
                    if let Some(next) = part.body.next {
                        metadata.insert("next_part_id".into(), json!(next));
                    }
                }
                if let Some((comment_id, comment)) = comments.get(id) {
                    metadata.extend(Map::from_iter([
                        ("comment_id".into(), json!(comment_id)),
                        ("author".into(), json!(comment.author)),
                        ("initials".into(), json!(comment.initials)),
                        ("date".into(), json!(comment.date)),
                        ("resolved".into(), json!(comment.resolved)),
                        ("legacy_page_projection".into(), json!(true)),
                    ]));
                    if let Some(parent) = comment.parent {
                        metadata.insert("parent_id".into(), json!(parent));
                    }
                }
                StoryPayload {
                    kind: kind.into(),
                    blocks: self.owned_blocks(&part.blocks, &[], None, Some(&source)),
                    metadata,
                }
            })
            .collect();
        for (kind, role, blocks) in separator_stories(self.document) {
            let source = Map::from_iter([
                ("story_kind".into(), json!(kind)),
                ("story_role".into(), json!(role)),
            ]);
            stories.push(StoryPayload {
                kind: kind.into(), blocks: self.owned_blocks(blocks, &[], None, Some(&source)),
                metadata: Map::from_iter([("role".into(), json!(role)), ("ordering".into(), json!("after parts; footnote then endnote; separator, continuation_separator, continuation_notice"))]),
            });
        }
        stories
    }

    // The same semantic normalizer serves body table cells and independent stories.
    // A missing page means no layout association is asserted, even for headers that repeat.
    fn owned_blocks(
        &self,
        blocks: &Blocks,
        prefix: &[u32],
        page_number: Option<usize>,
        story: Option<&Map<String, Value>>,
    ) -> Vec<BlockPayload> {
        let mut output = Vec::new();
        for (index, block) in blocks.iter().enumerate() {
            let path = Path([prefix, &[index as u32]].concat());
            match block.as_ref() {
                Block::Para(paragraph) => {
                    let pages = if page_number.is_some() {
                        self.paragraph_pages(&path)
                    } else {
                        Vec::new()
                    };
                    for (segment_index, mut segment) in self
                        .paragraph_segments(paragraph, &pages)
                        .into_iter()
                        .enumerate()
                    {
                        segment.set_source(&path.0, segment_index);
                        let mut payload = BlockPayload::paragraph(segment.text);
                        payload.style = paragraph.props.style.clone();
                        if let Some(level) = self
                            .document
                            .styles
                            .resolve_para(&paragraph.props)
                            .outline_level
                        {
                            let level = level.saturating_add(1).clamp(1, 6);
                            payload.kind = "heading".into();
                            payload.level = Some(level);
                            payload
                                .metadata
                                .insert("outline_level".into(), json!(level));
                        }
                        payload.links = segment.links;
                        payload.images = segment.images;
                        payload.metadata.insert("source_path".into(), json!(path.0));
                        if !segment.story_refs.is_empty() {
                            payload
                                .metadata
                                .insert("story_refs".into(), json!(segment.story_refs));
                        }
                        let page = page_number.and_then(|fallback| {
                            pages
                                .get(segment_index)
                                .copied()
                                .or_else(|| pages.last().copied())
                                .or(Some(fallback))
                        });
                        for link in &mut payload.links {
                            link.page = page;
                        }
                        for image in &mut payload.images {
                            image.page = page;
                        }
                        if let Some(source) = story {
                            payload.metadata.extend(source.clone());
                            for metadata in payload
                                .links
                                .iter_mut()
                                .map(|link| &mut link.metadata)
                                .chain(payload.images.iter_mut().map(|image| &mut image.metadata))
                            {
                                metadata.extend(source.clone());
                            }
                        }
                        output.push(payload);
                    }
                }
                Block::Table(table) => output.push(BlockPayload {
                    kind: "table".into(),
                    table: Some(
                        self.table_payload(
                            table,
                            &path,
                            page_number
                                .map(|_| self.table_page(table, &path).min(self.pages.len() - 1)),
                            story,
                        ),
                    ),
                    ..BlockPayload::default()
                }),
            }
        }
        output
    }

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
        let stories = self.stories();
        DocumentPayload {
            format: "docx".into(),
            metadata,
            pages: self.pages,
            stories,
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
            if !segment.story_refs.is_empty() {
                block
                    .metadata
                    .insert("story_refs".into(), json!(segment.story_refs));
            }
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
            if !block.text.is_empty()
                || !block.images.is_empty()
                || block.metadata.contains_key("story_refs")
            {
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
        let table = self.table_payload(table, path, Some(page_number), None);
        let page = &mut self.pages[page_number];
        page.tables.push(table.clone());
        page.blocks.push(BlockPayload {
            kind: "table".into(),
            table: Some(table),
            ..BlockPayload::default()
        });
    }

    fn table_payload(
        &self,
        table: &Table,
        path: &Path,
        page_number: Option<usize>,
        story: Option<&Map<String, Value>>,
    ) -> TablePayload {
        let mut cells: Vec<Vec<CellPayload>> = Vec::with_capacity(table.rows.len());
        let mut active_merges: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut next_merges = HashMap::new();
            let mut column = row.props.grid_before as usize;
            let mut output_row = Vec::with_capacity(row.cells.len());
            for (cell_index, cell) in row.cells.iter().enumerate() {
                let prefix = [path.0.as_slice(), &[row_index as u32, cell_index as u32]].concat();
                let blocks = self.owned_blocks(&cell.blocks, &prefix, page_number, story);
                let key = (column, cell.span());
                let mut covered = false;
                if cell.props.vmerge == VMerge::Restart {
                    next_merges.insert(key, (row_index, cell_index));
                } else if cell.props.vmerge == VMerge::Continue {
                    if let Some(&(anchor_row, anchor_cell)) = active_merges.get(&key) {
                        cells[anchor_row][anchor_cell].row_span += 1;
                        next_merges.insert(key, (anchor_row, anchor_cell));
                        covered = true;
                    }
                }
                output_row.push(CellPayload {
                    blocks,
                    col_span: cell.span(),
                    row_span: 1,
                    covered,
                    metadata: Map::from_iter([
                        ("grid_column".into(), json!(column)),
                        (
                            "vertical_merge".into(),
                            json!(match cell.props.vmerge {
                                VMerge::None => "none",
                                VMerge::Restart => "restart",
                                VMerge::Continue => "continue",
                            }),
                        ),
                    ]),
                });
                column += cell.span();
            }
            cells.push(output_row);
            active_merges = next_merges;
        }
        TablePayload {
            rows: table
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .map(|cell| self.cell_text(&cell.blocks))
                        .collect()
                })
                .collect(),
            cells,
            page: page_number,
            metadata: Map::from_iter([("column_widths".into(), json!(table.grid))]),
            ..TablePayload::default()
        }
    }

    fn collect_cell_assets(&mut self, blocks: &Blocks, prefix: &[u32]) {
        for (index, block) in blocks.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index as u32);
            match block.as_ref() {
                Block::Para(paragraph) => {
                    let page_indexes = self.paragraph_pages(&Path(path.clone()));
                    for (segment_index, mut segment) in self
                        .paragraph_segments(paragraph, &page_indexes)
                        .into_iter()
                        .enumerate()
                    {
                        segment.set_source(&path, segment_index);
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
        let mut objects = paragraph.objects.iter();
        let mut runs = paragraph.run_ranges().peekable();
        for (offset, character) in paragraph.text.char_indices() {
            if character == '\u{000c}' {
                segments.push(Segment::default());
                segment_index += 1;
                continue;
            }
            while runs.peek().is_some_and(|(range, _)| offset >= range.end) {
                runs.next();
            }
            let props = runs.peek().map_or(&paragraph.mark, |(_, props)| *props);
            if character == OBJ {
                if let Some(object) = objects.next() {
                    match object {
                        InlineObject::CommentStart { id } | InlineObject::CommentEnd { id } => {
                            segments[segment_index]
                                .comments
                                .push((*id, pages.get(segment_index).copied().unwrap_or(0)));
                        }
                        _ => {
                            let text_offset = segments[segment_index].char_count;
                            if let InlineObject::NoteRef { id, kind, custom } = object {
                                segments[segment_index].story_refs.push(json!({"part_id": id, "kind": match kind { wordcraft_doc::para::NoteKind::Footnote => "footnote", wordcraft_doc::para::NoteKind::Endnote => "endnote" }, "offset": text_offset, "custom_mark": custom}));
                            }
                            for id in object.text_boxes() {
                                segments[segment_index].story_refs.push(json!({"part_id": id, "kind": "text_box", "offset": text_offset}));
                            }
                            let first_image = segments[segment_index].images.len();
                            self.add_object(object, &mut segments[segment_index].images);
                            for image in &mut segments[segment_index].images[first_image..] {
                                image.metadata.insert("source_offset".into(), json!(offset));
                            }
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
    char_count: usize,
    links: Vec<LinkPayload>,
    images: Vec<ImagePayload>,
    comments: Vec<(u32, usize)>,
    story_refs: Vec<Value>,
    linked_uri: Option<String>,
    linked_start: usize,
    linked_byte_start: usize,
}

impl Segment {
    fn set_source(&mut self, path: &[u32], segment: usize) {
        for metadata in self
            .links
            .iter_mut()
            .map(|link| &mut link.metadata)
            .chain(self.images.iter_mut().map(|image| &mut image.metadata))
        {
            metadata.insert("source_path".into(), json!(path));
            metadata.insert("segment".into(), json!(segment));
        }
    }

    fn push_text(&mut self, text: &str, link: Option<&str>) {
        for character in text.chars() {
            self.push_char(character, link);
        }
    }

    fn push_char(&mut self, character: char, link: Option<&str>) {
        if self.linked_uri.as_deref() != link {
            self.finish_link();
            self.linked_uri = link.map(str::to_owned);
            self.linked_start = self.char_count;
            self.linked_byte_start = self.text.len();
        }
        self.text.push(character);
        self.char_count += 1;
    }

    fn finish_link(&mut self) {
        let Some(uri) = self.linked_uri.take() else {
            return;
        };
        let end = self.char_count;
        if end <= self.linked_start {
            return;
        }
        let text = self.text[self.linked_byte_start..].to_owned();
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
    let mut objects = paragraph.objects.iter();
    for character in paragraph.text.chars() {
        if character == OBJ {
            if let Some(object) = objects.next() {
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

#[cfg(test)]
mod offset_tests {
    use super::*;

    #[test]
    fn many_short_unicode_runs_skip_empty_runs_and_keep_link_boundaries() {
        use wordcraft_doc::para::Run;
        use wordcraft_doc::props::CharProps;

        let prefix = "λ🙂ab".repeat(1024);
        let mut paragraph = Paragraph::with_text(&prefix, CharProps::default());
        paragraph.props.style = Some("TypedParagraph".into());
        for index in 0..2000 {
            let props = CharProps {
                link: Some(format!("https://example.com/{}", index % 2)),
                bold: Some(index % 2 == 0),
                ..CharProps::default()
            };
            paragraph.runs.push(Run {
                len: 0,
                props: CharProps {
                    link: Some("https://invalid.example/empty".into()),
                    ..CharProps::default()
                },
            });
            paragraph.text.push_str("é🙂");
            paragraph.runs.push(Run {
                len: "é🙂".len(),
                props,
            });
        }
        // An uncovered final character uses paragraph mark formatting, matching props_of_char.
        paragraph.text.push('後');
        paragraph.mark.link = Some("https://example.com/mark".into());
        let document = Document::new();
        let layout = DocLayout::default();
        let mut adapter = Adapter::new(&document, &layout);
        adapter.add_paragraph(&paragraph, &Path(vec![0]));
        let block = &adapter.pages[0].blocks[0];
        assert_eq!(block.style.as_deref(), Some("TypedParagraph"));
        assert_eq!(block.text, paragraph.text);
        assert_eq!(block.links.len(), 2001);
        for (index, link) in block.links[..2000].iter().enumerate() {
            assert_eq!(link.uri, format!("https://example.com/{}", index % 2));
            assert_eq!(link.text, "é🙂");
            assert_eq!(link.metadata["start"], 4096 + index * 2);
            assert_eq!(link.metadata["end"], 4096 + index * 2 + 2);
        }
        assert_eq!(block.links[2000].uri, "https://example.com/mark");
        assert_eq!(block.links[2000].text, "後");
        assert_eq!(block.links[2000].metadata["start"], 8096);
        assert_eq!(block.links[2000].metadata["end"], 8097);
    }

    #[test]
    fn long_unicode_prefix_many_anchors_and_object_text_keep_exact_offsets() {
        let prefix = "λ🙂ab".repeat(32768);
        let prefix_chars = 4 * 32768;
        let mut text = prefix.clone();
        text.extend(std::iter::repeat_n(OBJ, 1003));
        let mut paragraph = Paragraph::with_text(&prefix, Default::default());
        // with_text intentionally strips structural markers; construct this public
        // model fixture directly to avoid quadratic insert_object setup work.
        paragraph.text = text;
        paragraph.runs[0].len = paragraph.text.len();
        paragraph.objects = (0..1000)
            .map(|id| InlineObject::NoteRef {
                id,
                kind: wordcraft_doc::para::NoteKind::Footnote,
                custom: String::new(),
            })
            .collect();
        paragraph.objects.push(InlineObject::Equation {
            linear: "é🙂".into(),
            display: false,
            math: Default::default(),
        });
        paragraph.objects.push(InlineObject::NoteRef {
            id: 1000,
            kind: wordcraft_doc::para::NoteKind::Endnote,
            custom: String::new(),
        });
        // The last marker deliberately has no object: it must not panic or invent a reference.
        let document = Document::new();
        let layout = DocLayout::default();
        let adapter = Adapter::new(&document, &layout);
        let segments = adapter.paragraph_segments(&paragraph, &[0]);
        assert_eq!(segments.len(), 1);
        let segment = &segments[0];
        assert_eq!(segment.char_count, prefix_chars + 2);
        assert_eq!(segment.story_refs.len(), 1001);
        assert_eq!(segment.story_refs[0]["offset"], prefix_chars);
        assert_eq!(segment.story_refs[999]["offset"], prefix_chars);
        assert_eq!(segment.story_refs[1000]["offset"], prefix_chars + 2);
        assert_eq!(segment.text, format!("{prefix}é🙂"));
        assert_eq!(visible_text(&paragraph), segment.text);
    }

    #[test]
    fn unicode_link_labels_use_byte_boundaries_and_character_offsets() {
        let mut segment = Segment::default();
        segment.push_text(&"λ🙂ab".repeat(32768), None);
        let mut offset = 4 * 32768;
        for index in 0..1000 {
            segment.push_text("é🙂", Some("https://example.com"));
            segment.push_char(' ', None);
            let link = &segment.links[index];
            assert_eq!(link.text, "é🙂");
            assert_eq!(link.metadata["start"], offset);
            assert_eq!(link.metadata["end"], offset + 2);
            offset += 3;
        }
        segment.push_text("終", Some("https://example.org"));
        segment.finish_links();
        assert_eq!(segment.links.last().unwrap().text, "終");
        assert_eq!(segment.links.last().unwrap().metadata["start"], offset);
        assert_eq!(segment.char_count, offset + 1);
    }
}
