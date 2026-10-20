use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use base64::Engine;
use serde_json::{json, Map, Value};
use wordcraft_doc::para::{InlineObject, Paragraph, OBJ};
use wordcraft_doc::props::{NumRef, VMerge};
use wordcraft_doc::{Block, Blocks, Document, PartKind, Path, StoryRef, Table};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions};

use crate::diagnostics::Diagnostics;
use crate::model::{
    BlockPayload, CellPayload, DocumentPayload, ImagePayload, LinkPayload, PagePayload,
    StoryPayload, TablePayload,
};
use crate::EngineError;

#[cfg(test)]
thread_local! {
    static SESSION_BUILDS: Cell<(u64, u64)> = const { Cell::new((0, 0)) };
}

pub fn read(data: &[u8], password: Option<&str>) -> Result<Document, EngineError> {
    crate::ooxml::preflight(data)?;
    #[cfg(test)]
    SESSION_BUILDS.with(|counts| {
        let (parses, layouts) = counts.get();
        counts.set((parses + 1, layouts));
    });
    wordcraft_docx::read_with_password(data, password).map_err(docx_error)
}

struct RenderState {
    document: Document,
    layout: DocLayout,
    render_requests: u64,
}

impl RenderState {
    fn open(data: &[u8], password: Option<&str>) -> Result<Self, EngineError> {
        crate::limits::validate_input_size(data.len())?;
        let document = read(data, password)?;
        validate_stories(&document)?;
        #[cfg(test)]
        SESSION_BUILDS.with(|counts| {
            let (parses, layouts) = counts.get();
            counts.set((parses, layouts + 1));
        });
        // The temporary measurement cache is not needed to render the owned layout.
        let layout = wordcraft_layout::layout(
            &document,
            &mut LayoutCache::new(),
            &LayoutOptions::default(),
        );
        validate_retained_layout(&layout)?;
        Ok(Self {
            document,
            layout,
            render_requests: 0,
        })
    }
}

// These checks bound retained state, not the peak allocations inside native layout.
fn validate_retained_layout(layout: &DocLayout) -> Result<(), EngineError> {
    if layout.pages.len() > 10_000 {
        return Err(EngineError::ResourceLimit(
            "DOCX layout exceeds 10000 pages".into(),
        ));
    }
    let mut items = 0usize;
    for page in &layout.pages {
        items = items.saturating_add(page.items.len());
        if items > 1_000_000 {
            return Err(EngineError::ResourceLimit(
                "DOCX layout exceeds 1000000 placed items".into(),
            ));
        }
    }
    Ok(())
}

pub struct DocxSession {
    state: Mutex<Option<RenderState>>,
}

impl DocxSession {
    pub fn open(
        data: &[u8],
        password: Option<&str>,
    ) -> Result<(Self, DocumentPayload), EngineError> {
        let state = RenderState::open(data, password)?;
        let mut adapter = Adapter::new(&state.document, &state.layout)?;
        adapter.visit_blocks(&state.document.body, &[]);
        adapter.add_comments();
        let payload = adapter.finish()?;
        Ok((
            Self {
                state: Mutex::new(Some(state)),
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
            .map_err(|_| EngineError::Malformed("DOCX render session is unavailable".into()))?;
        let state = guard
            .as_mut()
            .ok_or_else(|| EngineError::Closed("DOCX render session is closed".into()))?;
        // Count live attempts, including invalid page/DPI/clip requests. Closed or
        // poisoned attempts are excluded. There is no retained bitmap cache.
        state.render_requests = state.render_requests.saturating_add(1);
        render_with(&state.document, &state.layout, page, dpi, clip)
    }

    pub fn stats(&self) -> Result<(u64, u64, u64), EngineError> {
        let guard = self
            .state
            .lock()
            .map_err(|_| EngineError::Malformed("DOCX render session is unavailable".into()))?;
        let state = guard
            .as_ref()
            .ok_or_else(|| EngineError::Closed("DOCX render session is closed".into()))?;
        Ok((1, 1, state.render_requests))
    }

    pub fn close(&self) -> Result<(), EngineError> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(())
    }
}

pub fn extract(data: &[u8], password: Option<&str>) -> Result<DocumentPayload, EngineError> {
    DocxSession::open(data, password).map(|(_, payload)| payload)
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
    let state = RenderState::open(data, password)?;
    render_with(&state.document, &state.layout, page, dpi, clip)
}

fn render_with(
    document: &Document,
    layout: &DocLayout,
    page: usize,
    dpi: f64,
    clip: Option<crate::rendering::Clip>,
) -> Result<Vec<u8>, EngineError> {
    crate::rendering::validate_dpi(dpi)?;
    let page = layout
        .pages
        .get(page)
        .ok_or_else(|| EngineError::Unsupported(format!("page {page} does not exist")))?;
    let scale = (dpi / 72.0) as f32;
    let rendered = if let Some(clip) = clip {
        let region = crate::rendering::RasterRegion::new(
            clip,
            [f64::from(page.w), f64::from(page.h)],
            f64::from(scale),
            [
                f64::from((page.w * scale).ceil().max(1.0)),
                f64::from((page.h.min(1e6) * scale).ceil().max(1.0)),
            ],
            wordcraft_render::MAX_SIDE,
        )?;
        let rendered = wordcraft_render::render_region(
            document,
            page,
            region.width,
            region.height,
            kurbo::Affine::new([
                f64::from(scale),
                0.0,
                0.0,
                f64::from(scale),
                -f64::from(region.x),
                -f64::from(region.y),
            ]),
            &wordcraft_render::RenderOptions::default(),
        );
        region.validate_result(rendered.width, rendered.height)?;
        rendered
    } else {
        // Reject oversized requests before the upstream rasterizer allocates or clamps them.
        crate::rendering::validate_size(
            f64::from(page.w * scale),
            f64::from(page.h * scale),
            wordcraft_render::MAX_SIDE,
        )?;
        wordcraft_render::render_page(
            document,
            page,
            scale,
            &wordcraft_render::RenderOptions::default(),
        )
    };
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
    numbering: HashMap<(StoryRef, Path), NumberingObservation>,
    numbering_bytes: Cell<usize>,
    numbering_definitions: RefCell<HashMap<(u32, u8), NumberingDefinition<'a>>>,
    numbering_lookup_work: Cell<usize>,
    numbering_lookup_cost: usize,
}

const MAX_NUMBERING_BYTES: usize = 16 * 1024 * 1024;
const MAX_NUMBERING_STRING: usize = 64 * 1024;
const MAX_NUMBERING_LOOKUP_WORK: usize = 10_000_000;

#[derive(Clone, Copy)]
struct NumberingDefinition<'a> {
    abstract_id: Option<u32>,
    level: Option<&'a wordcraft_doc::numbering::Level>,
}

fn numbering_limit() -> EngineError {
    EngineError::ResourceLimit(
        "DOCX numbering exceeds bounded paths, placements, metadata bytes or native lookup work"
            .into(),
    )
}

struct NumberingObservation {
    reference: Option<NumRef>,
    label: Option<(usize, String)>,
    ambiguous_label: bool,
}

fn numbering_observations(
    layout: &DocLayout,
) -> Result<HashMap<(StoryRef, Path), NumberingObservation>, EngineError> {
    let mut observations: HashMap<(StoryRef, Path), NumberingObservation> = HashMap::new();
    let mut items = 0usize;
    let mut bytes = 0usize;
    for page in &layout.pages {
        for item in &page.items {
            items += 1;
            if items > 1_000_000 {
                return Err(numbering_limit());
            }
            let wordcraft_layout::Placed::Lines {
                story,
                path,
                para,
                l0,
                l1,
                ..
            } = item
            else {
                continue;
            };
            let reference = para.rp.numbering.filter(|value| value.num != 0);
            if reference.is_none() && para.label.is_none() {
                continue;
            }
            let observation =
                observations
                    .entry((*story, path.clone()))
                    .or_insert(NumberingObservation {
                        reference,
                        label: None,
                        ambiguous_label: false,
                    });
            if observation.reference != reference {
                return Err(EngineError::Unsupported(
                    "DOCX paragraph numbering differs between native placements".into(),
                ));
            }
            let line = para.label_line();
            if *l0 <= line && line < *l1 {
                if let (Some(label), Some(line)) = (&para.label, para.lines.get(line)) {
                    if label.text.len() > MAX_NUMBERING_STRING {
                        return Err(numbering_limit());
                    }
                    bytes = bytes.saturating_add(label.text.len());
                    if bytes > MAX_NUMBERING_BYTES {
                        return Err(numbering_limit());
                    }
                    if !observation.ambiguous_label {
                        if let Some((offset, text)) = &observation.label {
                            if *offset != line.start || text != &label.text {
                                observation.label = None;
                                observation.ambiguous_label = true;
                            }
                        } else {
                            observation.label = Some((line.start, label.text.clone()));
                        }
                    }
                }
            }
            if observations.len() > 100_000 {
                return Err(numbering_limit());
            }
        }
    }
    Ok(observations)
}

fn extraction_diagnostics(document: &Document) -> Value {
    let mut diagnostics = Diagnostics::default();
    diagnose_blocks(
        document,
        &document.body,
        &[],
        &json!({"story": "body"}),
        &mut diagnostics,
    );
    for (id, part) in &document.parts {
        diagnose_blocks(
            document,
            &part.blocks,
            &[],
            &json!({"story_part_id": id}),
            &mut diagnostics,
        );
    }
    for (kind, role, blocks) in separator_stories(document) {
        diagnose_blocks(
            document,
            blocks,
            &[],
            &json!({"story_kind": kind, "story_role": role}),
            &mut diagnostics,
        );
    }
    diagnostics.into_json()
}

fn diagnose_blocks(
    document: &Document,
    blocks: &Blocks,
    prefix: &[u32],
    story: &Value,
    diagnostics: &mut Diagnostics,
) {
    for (index, block) in blocks.iter().enumerate() {
        let path = [prefix, &[index as u32]].concat();
        match block.as_ref() {
            Block::Table(table) => {
                for (row, value) in table.rows.iter().enumerate() {
                    for (column, cell) in value.cells.iter().enumerate() {
                        diagnose_blocks(
                            document,
                            &cell.blocks,
                            &[path.as_slice(), &[row as u32, column as u32]].concat(),
                            story,
                            diagnostics,
                        );
                    }
                }
            }
            Block::Para(paragraph) => {
                for (index, object) in paragraph.objects.iter().enumerate() {
                    let mut source = story.clone();
                    source["source_path"] = json!(path);
                    source["object_path"] = json!([index]);
                    diagnose_object(document, object, source, diagnostics);
                }
            }
        }
    }
}

fn diagnose_object(
    document: &Document,
    object: &InlineObject,
    source: Value,
    diagnostics: &mut Diagnostics,
) {
    let notice = match object {
        InlineObject::Graphic { graphic, .. } => Some(match graphic.kind {
            wordcraft_doc::graphic::GraphicKind::Chart => ("chart_data_omitted", "warning", "Semantic extraction omits chart data and chart drawing content."),
            wordcraft_doc::graphic::GraphicKind::Diagram => ("diagram_structure_omitted", "warning", "Semantic extraction omits diagram structure and drawing content."),
            wordcraft_doc::graphic::GraphicKind::Ink => ("ink_geometry_omitted", "warning", "Semantic extraction omits ink strokes and their graphic preview."),
        }),
        InlineObject::Shape { .. } => Some(("shape_geometry_omitted", "info", "Semantic extraction omits drawing shape geometry; text box stories are exposed separately.")),
        InlineObject::Opaque { .. } => Some(("opaque_content_text_only", "warning", "An opaque object contributes only its stored text fallback; its structure is not extracted.")),
        InlineObject::Equation { .. } => Some(("equation_linearized", "info", "An equation contributes its linear text; structured mathematics is not extracted.")),
        InlineObject::Ruby { .. } => Some(("ruby_annotation_omitted", "warning", "Ruby text contributes the base text; the phonetic annotation is not extracted.")),
        InlineObject::Field { dirty: true, .. } | InlineObject::FieldStart { dirty: true, .. } => Some(("dirty_field_cached_result", "warning", "A field marked dirty contributes cached result text; extraction does not refresh it.")),
        InlineObject::Image { ole: Some(_), .. } => Some(("embedded_object_preview_only", "warning", "An embedded object contributes its image preview; embedded document data is not extracted.")),
        _ => None,
    };
    if let Some((code, severity, message)) = notice {
        diagnostics.record(code, severity, message, source.clone());
    }
    match object {
        InlineObject::Image { media, .. } if !document.media.contains_key(media) => diagnostics.record("missing_image_data", "warning", "An image reference has no media bytes in the parsed model and is omitted from extraction.", source),
        InlineObject::Group { children, .. } => {
            for (index, child) in children.iter().enumerate() {
                let mut child_source = source.clone();
                if let Some(path) = child_source["object_path"].as_array_mut() { path.push(json!(index)); }
                diagnose_object(document, &child.obj, child_source, diagnostics);
            }
        }
        _ => {}
    }
}

fn validate_stories(document: &Document) -> Result<(), EngineError> {
    if document.numbering.nums.len() > 100_000 || document.numbering.abstracts.len() > 100_000 {
        return Err(numbering_limit());
    }
    let mut numbering_bytes = 0usize;
    let mut levels = 0usize;
    for level in document
        .numbering
        .abstracts
        .iter()
        .flat_map(|value| &value.levels)
        .chain(
            document
                .numbering
                .nums
                .iter()
                .flat_map(|value| value.level_overrides.iter().map(|(_, level)| level)),
        )
    {
        levels += 1;
        numbering_bytes = numbering_bytes.saturating_add(level.text.len());
        if levels > 100_000
            || level.text.len() > MAX_NUMBERING_STRING
            || numbering_bytes > MAX_NUMBERING_BYTES
        {
            return Err(numbering_limit());
        }
    }
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
    fn numbering_definition(&self, reference: NumRef) -> Option<NumberingDefinition<'a>> {
        let key = (reference.num, reference.level);
        if let Some(value) = self.numbering_definitions.borrow().get(&key) {
            return Some(*value);
        }
        let work = self
            .numbering_lookup_work
            .get()
            .saturating_add(self.numbering_lookup_cost);
        self.numbering_lookup_work.set(work);
        if work > MAX_NUMBERING_LOOKUP_WORK || self.numbering_definitions.borrow().len() >= 100_000
        {
            self.numbering_lookup_work
                .set(MAX_NUMBERING_LOOKUP_WORK + 1);
            return None;
        }
        // Keep resolution native, including first-match IDs and level overrides. Cache misses
        // and unresolved references too; otherwise repeated invalid references rescan all definitions.
        let value = NumberingDefinition {
            abstract_id: self
                .document
                .numbering
                .num(reference.num)
                .map(|num| num.abstract_id),
            level: self
                .document
                .numbering
                .level(reference.num, reference.level),
        };
        self.numbering_definitions.borrow_mut().insert(key, value);
        Some(value)
    }

    fn segment_numbering(
        &self,
        paragraph: &Paragraph,
        path: &Path,
        story: Option<StoryRef>,
        segments: &[Segment],
    ) -> Vec<Option<Value>> {
        let observation = story.and_then(|story| self.numbering.get(&(story, path.clone())));
        let reference = observation
            .map(|value| value.reference)
            .unwrap_or_else(|| {
                let table = story.and_then(|story| {
                    let (table, _, _) = path.cell()?;
                    let table = self.document.table(story, &table)?;
                    self.document
                        .styles
                        .table_style(table.props.style.as_deref()?)
                });
                self.document
                    .styles
                    .resolve_para_in(&paragraph.props, table.as_ref().map(|style| &style.para))
                    .numbering
            })
            .filter(|value| value.num != 0);
        let Some(reference) = reference else {
            return vec![None; segments.len()];
        };
        if self.numbering_bytes.get() > MAX_NUMBERING_BYTES {
            return vec![None; segments.len()];
        }
        let Some(definition) = self.numbering_definition(reference) else {
            return vec![None; segments.len()];
        };
        let level = definition.level;
        let label = observation
            .and_then(|value| value.label.as_ref())
            .map(|(offset, text)| {
                (
                    segments
                        .partition_point(|segment| segment.byte_start <= *offset)
                        .saturating_sub(1),
                    text,
                )
            });
        segments
            .iter()
            .enumerate()
            .map(|(index, _)| {
                // Account for escaping and the duplicated page.tables projection before cloning.
                let strings = level.map_or(0, |level| level.text.len()).saturating_add(
                    label
                        .filter(|(segment, _)| *segment == index)
                        .map_or(0, |(_, text)| text.len()),
                );
                let bytes = self
                    .numbering_bytes
                    .get()
                    .saturating_add(strings.saturating_mul(12).saturating_add(512));
                self.numbering_bytes.set(bytes);
                if bytes > MAX_NUMBERING_BYTES {
                    return None;
                }
                let mut value = json!({
                    "num_id": reference.num,
                    "abstract_id": definition.abstract_id,
                    "level": reference.level,
                    "format": level.map(|level| level.format.ooxml()),
                    "template": level.map(|level| level.text.as_str()),
                });
                if let Some((segment, text)) = label {
                    if segment == index {
                        value["label"] = json!(text);
                    }
                }
                Some(value)
            })
            .collect()
    }

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
                    blocks: self.owned_blocks(
                        &part.blocks,
                        &[],
                        None,
                        Some(&source),
                        Some(StoryRef::Part(*id)),
                    ),
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
                kind: kind.into(), blocks: self.owned_blocks(blocks, &[], None, Some(&source), None),
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
        native_story: Option<StoryRef>,
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
                    let segments = self.paragraph_segments(paragraph, &pages);
                    let segment_pages = page_number.map(|fallback| {
                        self.paragraph_segment_pages(
                            &path,
                            &segments,
                            pages.first().copied().unwrap_or(fallback),
                        )
                    });
                    let numbering =
                        self.segment_numbering(paragraph, &path, native_story, &segments);
                    for (segment_index, mut segment) in segments.into_iter().enumerate() {
                        segment.set_source(&path.0, segment_index);
                        let mut payload = BlockPayload::paragraph(segment.text);
                        payload.style = paragraph.props.style.clone();
                        if let Some(value) = numbering[segment_index].clone() {
                            payload.metadata.insert("numbering".into(), value);
                        }
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
                        let page = segment_pages.as_ref().map(|pages| pages[segment_index]);
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
                            native_story,
                        ),
                    ),
                    ..BlockPayload::default()
                }),
            }
        }
        output
    }

    fn new(document: &'a Document, layout: &'a DocLayout) -> Result<Self, EngineError> {
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
        Ok(Self {
            document,
            layout,
            pages,
            comment_pages: HashMap::new(),
            numbering: numbering_observations(layout)?,
            numbering_bytes: Cell::new(0),
            numbering_definitions: RefCell::new(HashMap::new()),
            numbering_lookup_work: Cell::new(0),
            // num() once plus level()'s two num() scans, an abstract scan and a level-override
            // scan. This is conservative for the pinned native API, not a local resolver.
            numbering_lookup_cost: document
                .numbering
                .nums
                .len()
                .saturating_mul(3)
                .saturating_add(document.numbering.abstracts.len())
                .saturating_add(
                    document
                        .numbering
                        .nums
                        .iter()
                        .map(|num| num.level_overrides.len())
                        .max()
                        .unwrap_or(0),
                )
                .max(1),
        })
    }

    fn finish(self) -> Result<DocumentPayload, EngineError> {
        let mut metadata = Map::new();
        for (key, value) in [
            ("title", &self.document.core.title),
            ("subject", &self.document.core.subject),
            ("author", &self.document.core.creator),
            ("keywords", &self.document.core.keywords),
            ("description", &self.document.core.description),
            ("category", &self.document.core.category),
            ("last_modified_by", &self.document.core.last_modified_by),
            ("created", &self.document.core.created),
            ("modified", &self.document.core.modified),
        ] {
            if !value.is_empty() {
                metadata.insert(key.into(), Value::String(value.clone()));
            }
        }
        if self.document.core.revision > 0 {
            metadata.insert("revision".into(), json!(self.document.core.revision));
        }
        metadata.insert(
            "pagination".into(),
            Value::String("wordcraft-layout; blocks assigned to their first computed page".into()),
        );
        let stories = self.stories();
        metadata.insert("diagnostics".into(), extraction_diagnostics(self.document));
        if self.numbering_bytes.get() > MAX_NUMBERING_BYTES
            || self.numbering_lookup_work.get() > MAX_NUMBERING_LOOKUP_WORK
        {
            return Err(numbering_limit());
        }
        Ok(DocumentPayload {
            format: "docx".into(),
            metadata,
            pages: self.pages,
            stories,
        })
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
        let segment_pages = self.paragraph_segment_pages(path, &segments, page_indexes[0]);
        let numbering = self.segment_numbering(paragraph, path, Some(StoryRef::Body), &segments);
        for (segment_index, segment) in segments.into_iter().enumerate() {
            let page_number = segment_pages[segment_index].min(self.pages.len() - 1);
            for (id, _) in &segment.comments {
                self.comment_pages.entry(*id).or_insert(page_number);
            }
            let mut block = BlockPayload::paragraph(segment.text);
            if let Some(value) = numbering[segment_index].clone() {
                block.metadata.insert("numbering".into(), value);
            }
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
                || block.metadata.contains_key("numbering")
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
        let table = self.table_payload(table, path, Some(page_number), None, Some(StoryRef::Body));
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
        native_story: Option<StoryRef>,
    ) -> TablePayload {
        let mut cells: Vec<Vec<CellPayload>> = Vec::with_capacity(table.rows.len());
        let mut active_merges: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut next_merges = HashMap::new();
            let mut column = row.props.grid_before as usize;
            let mut output_row = Vec::with_capacity(row.cells.len());
            for (cell_index, cell) in row.cells.iter().enumerate() {
                let prefix = [path.0.as_slice(), &[row_index as u32, cell_index as u32]].concat();
                let blocks =
                    self.owned_blocks(&cell.blocks, &prefix, page_number, story, native_story);
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
                    let native_path = Path(path.clone());
                    let page_indexes = self.paragraph_pages(&native_path);
                    let segments = self.paragraph_segments(paragraph, &page_indexes);
                    let segment_pages =
                        self.paragraph_segment_pages(&native_path, &segments, page_indexes[0]);
                    for (segment_index, mut segment) in segments.into_iter().enumerate() {
                        segment.set_source(&path, segment_index);
                        let page_number = segment_pages[segment_index].min(self.pages.len() - 1);
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

    fn paragraph_segment_pages(
        &self,
        path: &Path,
        segments: &[Segment],
        fallback: usize,
    ) -> Vec<usize> {
        let mut pages = vec![None; segments.len()];
        // Native placed lines retain source byte ranges, including column/page breaks.
        // Locate explicit segment starts directly; do not count columns or infer pagination.
        // Binary-search segment starts per piece rather than rescanning every line per break.
        if let Some(positions) = self.layout.index.get(&(StoryRef::Body, path.clone())) {
            for (page, item) in positions {
                let Some(wordcraft_layout::Placed::Lines { para, l0, l1, .. }) = self
                    .layout
                    .pages
                    .get(*page)
                    .and_then(|page| page.items.get(*item))
                else {
                    continue;
                };
                let Some(first) = para.lines.get(*l0) else {
                    continue;
                };
                let Some(last) = l1.checked_sub(1).and_then(|index| para.lines.get(index)) else {
                    continue;
                };
                let begin = segments.partition_point(|segment| segment.byte_start < first.start);
                let end = segments.partition_point(|segment| segment.byte_start < last.stop);
                if begin < end {
                    for target in &mut pages[begin..end] {
                        target.get_or_insert(*page);
                    }
                }
            }
        }
        pages
            .into_iter()
            .map(|page| page.unwrap_or(fallback))
            .collect()
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
                    let text = visible_text(paragraph).replace(['\u{000c}', '\u{000e}'], "\n");
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
            if matches!(character, '\u{000c}' | '\u{000e}') {
                segments.push(Segment {
                    byte_start: offset + character.len_utf8(),
                    ..Segment::default()
                });
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
    byte_start: usize,
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
mod session_tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn fixture() -> Vec<u8> {
        let mut document = Document::from_text("First page Ω\nSecond page 🦀");
        let Block::Para(paragraph) = Arc::make_mut(&mut document.body[1]) else {
            unreachable!()
        };
        paragraph.props.page_break_before = Some(true);
        document.last_section.page_w = 300.0;
        document.last_section.page_h = 240.0;
        wordcraft_docx::write(&document).unwrap()
    }

    #[test]
    fn session_reuses_one_parse_and_layout_for_full_and_region_renders() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<DocxSession>();
        let data = fixture();
        let expected: Vec<_> = [72.0, 144.0, 300.0]
            .into_iter()
            .flat_map(|dpi| [0, 1].into_iter().map(move |page| (page, dpi)))
            .map(|(page, dpi)| {
                (
                    page,
                    dpi,
                    render(&data, page, dpi, None).unwrap(),
                    render_region(&data, page, dpi, None, (65.0, 65.0, 130.0, 115.0)).unwrap(),
                )
            })
            .collect();
        SESSION_BUILDS.with(|counts| counts.set((0, 0)));
        let (session, payload) = DocxSession::open(&data, None).unwrap();
        drop(data);
        assert_eq!(payload.pages.len(), 2);
        for (page, dpi, full, region) in expected {
            assert_eq!(session.render(page, dpi, None).unwrap(), full);
            assert_eq!(
                session
                    .render(page, dpi, Some((65.0, 65.0, 130.0, 115.0)))
                    .unwrap(),
                region
            );
        }
        assert_eq!(SESSION_BUILDS.with(Cell::get), (1, 1));
        assert_eq!(session.stats().unwrap(), (1, 1, 12));
        let weak = {
            let guard = session.state.lock().unwrap();
            Arc::downgrade(&guard.as_ref().unwrap().document.body[0])
        };
        session.close().unwrap();
        session.close().unwrap();
        assert!(weak.upgrade().is_none());
        assert!(payload.pages[0]
            .blocks
            .iter()
            .any(|block| block.text.contains("First page")));
        assert!(matches!(
            session.render(0, 72.0, None),
            Err(EngineError::Closed(_))
        ));
        assert!(matches!(session.stats(), Err(EngineError::Closed(_))));
    }

    #[test]
    fn session_keeps_render_error_order_and_accepts_small_regions_of_oversized_pages() {
        let data = fixture();
        let (session, _) = DocxSession::open(&data, None).unwrap();
        for (page, dpi, clip) in [
            (99, f64::NAN, None),
            (99, 72.0, None),
            (0, 72.0, Some((0.0, 0.0, 0.0, 10.0))),
            (0, 72.0, Some((500.0, 500.0, 510.0, 510.0))),
        ] {
            let expected = render_impl(&data, page, dpi, None, clip)
                .unwrap_err()
                .to_string();
            assert_eq!(
                session.render(page, dpi, clip).unwrap_err().to_string(),
                expected
            );
        }
        assert_eq!(session.stats().unwrap(), (1, 1, 4));
        let mut big = Document::from_text("Small region of a large page");
        big.last_section.page_w = 1584.0;
        big.last_section.page_h = 1584.0;
        let bytes = wordcraft_docx::write(&big).unwrap();
        let (large, _) = DocxSession::open(&bytes, None).unwrap();
        assert!(matches!(
            large.render(0, 1200.0, None),
            Err(EngineError::ResourceLimit(_))
        ));
        assert_eq!(
            large.render(0, 1200.0, Some((0.0, 0.0, 8.0, 8.0))).unwrap(),
            render_region(&bytes, 0, 1200.0, None, (0.0, 0.0, 8.0, 8.0)).unwrap()
        );
        assert!(DocxSession::open(b"not a DOCX", None).is_err());
        let (plain_password, _) = DocxSession::open(&data, Some("unused password")).unwrap();
        plain_password.close().unwrap();
    }

    #[test]
    fn session_close_races_are_serialized_and_poisoned_close_releases_state() {
        let (session, _) = DocxSession::open(&fixture(), None).unwrap();
        let session = Arc::new(session);
        let barrier = Arc::new(Barrier::new(2));
        let rendering = {
            let session = session.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                for _ in 0..4 {
                    match session.render(0, 72.0, None) {
                        Ok(bytes) => assert!(bytes.starts_with(b"\x89PNG")),
                        Err(EngineError::Closed(_)) => return,
                        Err(error) => panic!("unexpected render failure: {error}"),
                    }
                }
            })
        };
        barrier.wait();
        session.close().unwrap();
        rendering.join().unwrap();
        let (poisoned, _) = DocxSession::open(&fixture(), None).unwrap();
        let poisoned = Arc::new(poisoned);
        let weak = {
            let guard = poisoned.state.lock().unwrap();
            Arc::downgrade(&guard.as_ref().unwrap().document.body[0])
        };
        let poisoner = poisoned.clone();
        assert!(std::thread::spawn(move || {
            let _guard = poisoner.state.lock().unwrap();
            panic!("intentional lock poison");
        })
        .join()
        .is_err());
        assert!(matches!(
            poisoned.render(0, 72.0, None),
            Err(EngineError::Malformed(_))
        ));
        poisoned.close().unwrap();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn retained_layout_page_budget_rejects_before_payload_or_render() {
        let native = Document::new();
        let mut layout =
            wordcraft_layout::layout(&native, &mut LayoutCache::new(), &LayoutOptions::default());
        // Empty pages keep the fixture small while exercising the retained-layout boundary.
        let mut page = layout.pages[0].clone();
        page.items.clear();
        layout.pages = vec![page; 10_001];
        assert!(matches!(
            validate_retained_layout(&layout),
            Err(EngineError::ResourceLimit(_))
        ));
        layout.pages.pop();
        validate_retained_layout(&layout).unwrap();
    }
}

#[cfg(test)]
mod numbering_tests {
    use super::*;
    use std::sync::Arc;
    use wordcraft_doc::numbering::ListKind;

    fn numbered(text: &str, num: u32, level: u8) -> Arc<Block> {
        let mut paragraph = Paragraph::new();
        paragraph.insert_text(0, text, &Default::default()).unwrap();
        paragraph.props.numbering = Some(NumRef { num, level });
        wordcraft_doc::para_block(paragraph)
    }

    #[test]
    fn labels_come_from_native_layout_and_follow_the_label_line_after_a_break() {
        let mut document = Document::new();
        let num = document.numbering.try_add_list(ListKind::Numbered).unwrap();
        document.body = vec![
            numbered("\u{c}After break", num, 0),
            numbered("Second", num, 0),
            numbered("", num, 0),
        ];
        let data = wordcraft_docx::write(&document).unwrap();
        let payload = extract(&data, None).unwrap();
        assert_eq!(payload.pages.len(), 2);
        let blocks: Vec<_> = payload.pages.iter().flat_map(|page| &page.blocks).collect();
        assert_eq!(
            blocks
                .iter()
                .map(|block| block.text.as_str())
                .collect::<Vec<_>>(),
            ["", "After break", "Second", ""]
        );
        assert!(!blocks[0].metadata["numbering"]
            .as_object()
            .unwrap()
            .contains_key("label"));
        for (block, label) in blocks[1..].iter().zip(["1.", "2.", "3."]) {
            assert_eq!(block.metadata["numbering"]["label"], label);
            assert_eq!(block.metadata["numbering"]["format"], "decimal");
            assert_eq!(block.metadata["numbering"]["template"], "%1.");
            assert_eq!(block.metadata["numbering"]["num_id"], num);
        }
    }

    #[test]
    fn conflicting_repeated_labels_are_not_asserted_and_missing_definitions_are_null() {
        let mut document = Document::new();
        let num = document.numbering.try_add_list(ListKind::Numbered).unwrap();
        document.body = vec![numbered("Repeated", num, 0)];
        let mut layout = wordcraft_layout::layout(
            &document,
            &mut LayoutCache::new(),
            &LayoutOptions::default(),
        );
        let mut repeated = layout.pages[0]
            .items
            .iter()
            .find(|item| matches!(item, wordcraft_layout::Placed::Lines { .. }))
            .unwrap()
            .clone();
        let wordcraft_layout::Placed::Lines { para, .. } = &mut repeated else {
            unreachable!()
        };
        Arc::make_mut(para).label.as_mut().unwrap().text = "99.".into();
        layout.pages[0].items.push(repeated);
        let mut adapter = Adapter::new(&document, &layout).unwrap();
        adapter.visit_blocks(&document.body, &[]);
        let metadata = &adapter.pages[0].blocks[0].metadata["numbering"];
        assert_eq!(metadata["format"], "decimal");
        assert!(!metadata.as_object().unwrap().contains_key("label"));

        document.body = vec![
            numbered("Missing", 9999, 7),
            numbered("Missing level", num, 20),
            numbered("Unnumbered", 0, 0),
        ];
        let empty_layout = DocLayout::default();
        let mut adapter = Adapter::new(&document, &empty_layout).unwrap();
        adapter.visit_blocks(&document.body, &[]);
        let missing = &adapter.pages[0].blocks[0].metadata["numbering"];
        assert_eq!(missing["num_id"], 9999);
        assert!(missing["abstract_id"].is_null());
        assert!(missing["format"].is_null());
        assert!(missing["template"].is_null());
        let missing_level = &adapter.pages[0].blocks[1].metadata["numbering"];
        assert!(missing_level["abstract_id"].is_number());
        assert!(missing_level["format"].is_null());
        assert!(!adapter.pages[0].blocks[2]
            .metadata
            .contains_key("numbering"));
    }

    #[test]
    fn repeated_definition_queries_are_cached_and_distinct_query_work_is_bounded() {
        let mut document = Document::new();
        document.numbering.try_add_list(ListKind::Numbered).unwrap();
        let original = document.numbering.nums[0].clone();
        for id in 2..=2000 {
            let mut number = original.clone();
            number.id = id;
            document.numbering.nums.push(number);
        }
        let layout = DocLayout::default();
        let mut adapter = Adapter::new(&document, &layout).unwrap();
        let block = numbered("Repeated", 2000, 0);
        let Block::Para(paragraph) = block.as_ref() else {
            unreachable!()
        };
        for index in 0..2000 {
            adapter.add_paragraph(paragraph, &Path::top(index));
        }
        assert_eq!(
            adapter.numbering_lookup_work.get(),
            adapter.numbering_lookup_cost
        );
        let missing = NumRef {
            num: 9999,
            level: 0,
        };
        for _ in 0..2000 {
            let value = adapter.numbering_definition(missing).unwrap();
            assert!(value.abstract_id.is_none() && value.level.is_none());
        }
        assert_eq!(
            adapter.numbering_lookup_work.get(),
            2 * adapter.numbering_lookup_cost
        );
        let payload = adapter.finish().unwrap();
        assert_eq!(payload.pages[0].blocks.len(), 2000);
        assert!(payload.pages[0]
            .blocks
            .iter()
            .all(|block| block.metadata["numbering"]["num_id"] == 2000
                && block.metadata["numbering"]["format"] == "decimal"));

        let mut adapter = Adapter::new(&document, &layout).unwrap();
        for id in 1..=2000 {
            let block = numbered("Distinct", id, 0);
            let Block::Para(paragraph) = block.as_ref() else {
                unreachable!()
            };
            adapter.add_paragraph(paragraph, &Path::top((id - 1) as usize));
        }
        assert!(matches!(
            adapter.finish(),
            Err(EngineError::ResourceLimit(_))
        ));
    }

    #[test]
    fn numbering_templates_and_repeated_output_metadata_are_bounded() {
        let mut document = Document::new();
        let num = document.numbering.try_add_list(ListKind::Numbered).unwrap();
        document.numbering.abstracts[0].levels[0].text = "x".repeat(MAX_NUMBERING_STRING + 1);
        assert!(matches!(
            validate_stories(&document),
            Err(EngineError::ResourceLimit(_))
        ));
        document.numbering.abstracts[0].levels[0].text = "x".repeat(1024);
        let paragraph = numbered("Small paragraph", num, 0);
        let layout = DocLayout::default();
        let mut adapter = Adapter::new(&document, &layout).unwrap();
        let Block::Para(paragraph) = paragraph.as_ref() else {
            unreachable!()
        };
        for index in 0..2000 {
            adapter.add_paragraph(paragraph, &Path::top(index));
        }
        assert!(matches!(
            adapter.finish(),
            Err(EngineError::ResourceLimit(_))
        ));
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use std::sync::Arc;
    use wordcraft_doc::graphic::{Embedded, Graphic, GraphicKind};

    #[test]
    fn native_chart_file_reports_omission_at_its_body_source() {
        let mut document = Document::new();
        let spec = wordcraft_doc::chart::ChartSpec::sample(wordcraft_doc::chart::ChartType::Column);
        let mut paragraph = Paragraph::new();
        paragraph
            .insert_object(
                0,
                InlineObject::Graphic {
                    w: 300.0,
                    h: 200.0,
                    alt: "Sales".into(),
                    float: Default::default(),
                    graphic: Arc::new(Graphic {
                        kind: GraphicKind::Chart,
                        items: wordcraft_docx::chart_items(
                            &spec,
                            &document.settings.theme_colors,
                            300.0,
                            200.0,
                        ),
                        w: 300.0,
                        h: 200.0,
                        chart: Some(Arc::new(spec)),
                        ..Graphic::default()
                    }),
                },
                &Default::default(),
            )
            .unwrap();
        document.body = vec![wordcraft_doc::para_block(paragraph)];
        let bytes = wordcraft_docx::write(&document).unwrap();
        let extracted = extract(&bytes, None).unwrap();
        assert_eq!(
            extracted.metadata["diagnostics"][0]["code"],
            "chart_data_omitted"
        );
        assert_eq!(extracted.metadata["diagnostics"][0]["count"], 1);
        assert_eq!(
            extracted.metadata["diagnostics"][0]["sources"][0]["source_path"],
            json!([0])
        );
    }

    #[test]
    fn identifies_graphic_fallbacks_and_missing_embedded_media() {
        let document = Document::new();
        let mut diagnostics = Diagnostics::default();
        for (index, kind) in [GraphicKind::Chart, GraphicKind::Diagram, GraphicKind::Ink]
            .into_iter()
            .enumerate()
        {
            let object = InlineObject::Graphic {
                w: 10.0,
                h: 10.0,
                alt: "Graphic".into(),
                float: Default::default(),
                graphic: Arc::new(Graphic {
                    kind,
                    ..Graphic::default()
                }),
            };
            diagnose_object(
                &document,
                &object,
                json!({"story_part_id": 9, "source_path": [0], "object_path": [index]}),
                &mut diagnostics,
            );
        }
        let image = InlineObject::Image {
            media: "missing.png".into(),
            w: 10.0,
            h: 10.0,
            alt: String::new(),
            float: Default::default(),
            crop: [0.0; 4],
            ole: Some(Arc::new(Embedded::default())),
        };
        diagnose_object(
            &document,
            &image,
            json!({"story": "body", "source_path": [1], "object_path": [0]}),
            &mut diagnostics,
        );
        let opaque = InlineObject::Opaque {
            format: "docx".into(),
            xml: "<opaque/>".into(),
            text: "Fallback".into(),
        };
        diagnose_object(
            &document,
            &opaque,
            json!({"story": "body", "source_path": [2], "object_path": [0]}),
            &mut diagnostics,
        );
        let values = diagnostics.into_json();
        let values = values.as_array().unwrap();
        let codes: Vec<_> = values
            .iter()
            .map(|value| value["code"].as_str().unwrap())
            .collect();
        assert_eq!(
            codes,
            [
                "chart_data_omitted",
                "diagram_structure_omitted",
                "ink_geometry_omitted",
                "embedded_object_preview_only",
                "missing_image_data",
                "opaque_content_text_only"
            ]
        );
        assert_eq!(values[0]["sources"][0]["story_part_id"], 9);
        assert!(values
            .iter()
            .all(|value| value["count"] == 1 && value["operation"] == "extract"));
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
        let mut adapter = Adapter::new(&document, &layout).unwrap();
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
        let adapter = Adapter::new(&document, &layout).unwrap();
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
