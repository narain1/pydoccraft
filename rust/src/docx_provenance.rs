//! Bounded observations of public native source paths and logical selection geometry.
//! Coordinates do not assert ink bounds, clipping, or offsets in normalized output text.
use super::*;
use wordcraft_layout::Placed;

const MAP_ALLOWANCE: usize = 2048;
const RECT_ALLOWANCE: usize = 256;

#[derive(Clone, Copy)]
struct Limits {
    lines: usize,
    rects: usize,
    work: usize,
    bytes: usize,
    temporary: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            lines: 100_000,
            rects: 200_000,
            work: 8_000_000,
            bytes: 128 * 1024 * 1024,
            temporary: 16 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Budget {
    lines: usize,
    rects: usize,
    work: usize,
    bytes: usize,
    failed: bool,
}

fn limit() -> EngineError {
    EngineError::ResourceLimit(
        "DOCX provenance exceeds bounded lines, rectangles, native work or metadata memory".into(),
    )
}

fn malformed() -> EngineError {
    EngineError::Malformed("invalid native DOCX source or layout geometry".into())
}

impl Budget {
    fn add(
        &mut self,
        limits: Limits,
        lines: usize,
        rects: usize,
        work: usize,
        bytes: usize,
    ) -> Result<(), EngineError> {
        self.lines = self.lines.checked_add(lines).ok_or_else(limit)?;
        self.rects = self.rects.checked_add(rects).ok_or_else(limit)?;
        self.work = self.work.checked_add(work).ok_or_else(limit)?;
        self.bytes = self.bytes.checked_add(bytes).ok_or_else(limit)?;
        if self.lines > limits.lines
            || self.rects > limits.rects
            || self.work > limits.work
            || self.bytes > limits.bytes
        {
            return Err(limit());
        }
        Ok(())
    }
}

struct Source<'a> {
    text: &'a str,
    segments: Vec<(usize, usize)>,
    lines: Vec<Vec<Value>>,
    bytes: Vec<usize>,
}

struct Layout {
    value: Value,
    bytes: usize,
}

pub(super) struct Provenance {
    layouts: HashMap<Path, Vec<Option<Layout>>>,
    budget: Cell<Budget>,
    limits: Limits,
}

impl Provenance {
    pub(super) fn new(document: &Document, layout: &DocLayout) -> Result<Self, EngineError> {
        Self::with_limits(document, layout, Limits::default())
    }

    fn with_limits(
        document: &Document,
        layout: &DocLayout,
        limits: Limits,
    ) -> Result<Self, EngineError> {
        fn sources<'a>(
            blocks: &'a Blocks,
            prefix: &[u32],
            result: &mut HashMap<Path, Source<'a>>,
            budget: &mut Budget,
            limits: Limits,
        ) -> Result<(), EngineError> {
            if prefix.len() > 48 {
                return Err(limit());
            }
            for (index, block) in blocks.iter().enumerate() {
                let index = u32::try_from(index).map_err(|_| malformed())?;
                budget.add(limits, 0, 0, 1, 256 + prefix.len() * 16)?;
                let path = Path([prefix, &[index]].concat());
                match block.as_ref() {
                    Block::Para(p) => {
                        let mut segments = Vec::new();
                        let mut start = 0;
                        for (offset, ch) in p.text.char_indices() {
                            if matches!(ch, '\u{c}' | '\u{e}') {
                                budget.add(limits, 0, 0, 0, 256)?;
                                segments.push((start, offset));
                                start = offset + ch.len_utf8();
                            }
                        }
                        budget.add(limits, 0, 0, 0, 256)?;
                        segments.push((start, p.text.len()));
                        let lines = (0..segments.len()).map(|_| Vec::new()).collect();
                        let bytes = vec![0; segments.len()];
                        result.insert(
                            path,
                            Source {
                                text: &p.text,
                                segments,
                                lines,
                                bytes,
                            },
                        );
                    }
                    Block::Table(table) => {
                        for (row, value) in table.rows.iter().enumerate() {
                            budget.add(limits, 0, 0, 1, 0)?;
                            let row = u32::try_from(row).map_err(|_| malformed())?;
                            for (cell, value) in value.cells.iter().enumerate() {
                                budget.add(limits, 0, 0, 1, 0)?;
                                let cell = u32::try_from(cell).map_err(|_| malformed())?;
                                sources(
                                    &value.blocks,
                                    &[path.0.as_slice(), &[row, cell]].concat(),
                                    result,
                                    budget,
                                    limits,
                                )?;
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        let mut budget = Budget::default();
        let mut source = HashMap::new();
        sources(&document.body, &[], &mut source, &mut budget, limits)?;
        for (page_number, page) in layout.pages.iter().enumerate() {
            if ![
                page.w,
                page.h,
                page.body.x,
                page.body.y,
                page.body.w,
                page.body.h,
                page.body.right(),
                page.body.bottom(),
            ]
            .iter()
            .all(|v| v.is_finite())
                || page.w <= 0.0
                || page.h <= 0.0
                || page.body.w < 0.0
                || page.body.h < 0.0
            {
                return Err(malformed());
            }
            budget.add(limits, 0, 0, 0, 512)?;
            for item in &page.items {
                let Placed::Lines {
                    story: StoryRef::Body,
                    path,
                    para,
                    l0,
                    l1,
                    x,
                    y,
                    turn,
                } = item
                else {
                    continue;
                };
                let value = source.get_mut(path).ok_or_else(malformed)?;
                let lines = para
                    .lines
                    .get(*l0..*l1)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(malformed)?;
                let first_top = lines[0].top;
                if ![first_top, *x, *y].iter().all(|v| v.is_finite()) {
                    return Err(malformed());
                }
                for (offset, line) in lines.iter().enumerate() {
                    let clusters = para.clusters.get(line.c0..line.c1).ok_or_else(malformed)?;
                    let n = clusters.len();
                    let work = if line.vis.is_empty() {
                        n.checked_mul(2).and_then(|v| v.checked_add(1))
                    } else {
                        n.checked_mul((usize::BITS - n.leading_zeros()) as usize + 4)
                            .and_then(|v| v.checked_add(1))
                    }
                    .ok_or_else(limit)?;
                    // Native x_spans retains ks while Line::spans creates boxes/out and sorts.
                    // 64 bytes/cluster covers indices, both pair buffers and sorting scratch.
                    let temporary = n
                        .checked_mul(64)
                        .and_then(|v| v.checked_add(256))
                        .ok_or_else(limit)?;
                    if temporary > limits.temporary {
                        return Err(limit());
                    }
                    budget.add(limits, 0, 0, work, 0)?;
                    if line.start > line.stop
                        || line.stop > value.text.len()
                        || !value.text.is_char_boundary(line.start)
                        || !value.text.is_char_boundary(line.stop)
                        || line.xs.len() != n + 1
                        || (!line.vis.is_empty() && line.vis.len() != n)
                        || ![line.top, line.height, line.baseline, line.left, line.right]
                            .iter()
                            .all(|v| v.is_finite())
                        || line.height < 0.0
                        || !line.xs.iter().all(|v| v.is_finite())
                        // Native condensed tracking can produce signed visual advances.
                        // Keep finite endpoints; x_spans owns positive-span selection.
                        || !line
                            .vis
                            .iter()
                            .all(|v| v.x.is_finite() && v.w.is_finite() && (v.x + v.w).is_finite())
                        || !clusters.iter().all(|c| {
                            c.start <= c.end
                                && c.end <= value.text.len()
                                && value.text.is_char_boundary(c.start)
                                && value.text.is_char_boundary(c.end)
                        })
                    {
                        return Err(malformed());
                    }
                    let begin = value
                        .segments
                        .partition_point(|(_, stop)| *stop < line.start);
                    let end = value
                        .segments
                        .partition_point(|(start, _)| *start <= line.stop);
                    for segment in begin..end {
                        let (start, stop) = value.segments[segment];
                        let from = start.max(line.start);
                        let to = stop.min(line.stop);
                        if from >= to && start != stop {
                            continue;
                        }
                        // Charge each helper call, including multiple empty explicit segments.
                        budget.add(limits, 1, 0, work, MAP_ALLOWANCE)?;
                        let spans = para.x_spans(*l0 + offset, from, to);
                        let count = spans.iter().filter(|(a, b)| b > a).count();
                        let rect_bytes = count.checked_mul(RECT_ALLOWANCE).ok_or_else(limit)?;
                        budget.add(limits, 0, count, 0, rect_bytes)?;
                        value.bytes[segment] = value.bytes[segment]
                            .checked_add(MAP_ALLOWANCE)
                            .and_then(|v| v.checked_add(rect_bytes))
                            .ok_or_else(limit)?;
                        let mut rects = Vec::with_capacity(count);
                        for (a, b) in spans {
                            if !a.is_finite() || !b.is_finite() {
                                return Err(malformed());
                            }
                            if b <= a {
                                continue;
                            }
                            let mut r = page.body;
                            r.x = a;
                            r.y = line.top - first_top;
                            r.w = b - a;
                            r.h = line.height;
                            let r = wordcraft_layout::turn_rect(*turn, *x, *y, r);
                            let coords = [r.x, r.y, r.right(), r.bottom()];
                            if !coords.iter().all(|v| v.is_finite()) {
                                return Err(malformed());
                            }
                            rects.push(json!(coords));
                        }
                        value.lines[segment].push(json!({"page": page_number, "source_range": [from, to], "rects": rects}));
                    }
                }
            }
        }
        let mut layouts = HashMap::new();
        for (path, value) in source {
            let mut segments = Vec::with_capacity(value.lines.len());
            for (lines, bytes) in value.lines.into_iter().zip(value.bytes) {
                if lines.is_empty() {
                    segments.push(None);
                } else {
                    budget.add(limits, 0, 0, 0, MAP_ALLOWANCE)?;
                    let bytes = bytes.checked_add(MAP_ALLOWANCE).ok_or_else(limit)?;
                    segments.push(Some(Layout { bytes, value: json!({
                        "coordinate_space": "page-points", "offset_space": "paragraph-utf8", "geometry": "native-selection", "lines": lines
                    }) }));
                }
            }
            layouts.insert(path, segments);
        }
        Ok(Self {
            layouts,
            budget: Cell::new(budget),
            limits,
        })
    }

    // Metadata appears in page and block/table copies. Charge both before constructing it.
    fn charge_bytes(&self, bytes: Option<usize>) -> bool {
        let mut budget = self.budget.get();
        if budget.failed
            || bytes
                .and_then(|n| budget.add(self.limits, 0, 0, 0, n).ok())
                .is_none()
        {
            budget.failed = true;
        }
        self.budget.set(budget);
        !budget.failed
    }

    pub(super) fn charge_sources(&self, path: &[u32], count: usize) -> bool {
        self.charge_bytes(
            path.len()
                .checked_mul(std::mem::size_of::<Value>())
                .and_then(|v| v.checked_add(MAP_ALLOWANCE))
                .and_then(|v| v.checked_mul(count))
                .and_then(|v| v.checked_mul(2)),
        )
    }

    pub(super) fn metadata(
        &self,
        path: &Path,
        segment: Option<usize>,
        story: Option<StoryRef>,
    ) -> Map<String, Value> {
        if !self.charge_sources(&path.0, 1) {
            return Map::new();
        }
        let mut metadata = Map::from_iter([("source_path".into(), json!(path.0))]);
        if let Some(segment) = segment {
            metadata.insert("segment".into(), json!(segment));
            if story == Some(StoryRef::Body) {
                if let Some(Some(layout)) = self.layouts.get(path).and_then(|v| v.get(segment)) {
                    // Reserve both current payload and its page/table clone on every call.
                    if self.charge_bytes(layout.bytes.checked_mul(2)) {
                        metadata.insert("layout".into(), layout.value.clone());
                    }
                }
            }
        }
        metadata
    }

    pub(super) fn finish(&self) -> Result<(), EngineError> {
        if self.budget.get().failed {
            Err(limit())
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn fixture() -> (Document, DocLayout) {
        let mut doc = Document::from_text("λ English עברית words");
        let Block::Para(p) = Arc::make_mut(&mut doc.body[0]) else {
            unreachable!()
        };
        p.props.bidi = Some(true);
        let layout =
            wordcraft_layout::layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
        (doc, layout)
    }

    #[test]
    fn provenance_preserves_native_signed_advances_for_condensed_rtl_text() {
        pyo3::prepare_freethreaded_python();
        let text = "سلام Word";
        for spacing in [0, 2, -1] {
            let mut author = crate::author_docx::EditableDocx::new();
            author
                .set_section_props(0, r#"{"page_size":[400,250],"margins":[30,30,30,30]}"#)
                .unwrap();
            author
                .format_defaults(r##"{"color":"#FF0000","font_size":24}"##)
                .unwrap();
            let id = author.add_paragraph(text).unwrap();
            author
                .format_paragraph(id, r#"{"direction":"rtl"}"#)
                .unwrap();
            author
                .format_range_patch(
                    id,
                    0,
                    text.chars().count(),
                    author.revision().unwrap(),
                    &json!({"letter_spacing": spacing}).to_string(),
                )
                .unwrap();
            let doc = wordcraft_docx::read(&author.to_bytes().unwrap()).unwrap();
            let layout =
                wordcraft_layout::layout(&doc, &mut LayoutCache::new(), &LayoutOptions::default());
            let mut negative = 0;
            let mut expected = Vec::new();
            for (page_index, page) in layout.pages.iter().enumerate() {
                for item in &page.items {
                    let Placed::Lines {
                        story: StoryRef::Body,
                        path,
                        para,
                        l0,
                        l1,
                        x,
                        y,
                        turn,
                    } = item
                    else {
                        continue;
                    };
                    assert_eq!(path, &Path(vec![0]));
                    let first_top = para.lines[*l0].top;
                    for index in *l0..*l1 {
                        let line = &para.lines[index];
                        negative += line.vis.iter().filter(|v| v.w < 0.0).count();
                        assert!(line.vis.iter().all(|v| v.x.is_finite()
                            && v.w.is_finite()
                            && (v.x + v.w).is_finite()));
                        let spans = para.x_spans(index, line.start, line.stop);
                        assert!(!spans.is_empty());
                        let rects: Vec<_> = spans
                            .into_iter()
                            .filter(|(a, b)| b > a)
                            .map(|(a, b)| {
                                let mut rect = page.body;
                                rect.x = a;
                                rect.y = line.top - first_top;
                                rect.w = b - a;
                                rect.h = line.height;
                                let rect = wordcraft_layout::turn_rect(*turn, *x, *y, rect);
                                json!([rect.x, rect.y, rect.right(), rect.bottom()])
                            })
                            .collect();
                        expected.push(json!({"page": page_index, "source_range": [line.start, line.stop], "rects": rects}));
                    }
                }
            }
            assert_eq!(
                negative > 0,
                spacing < 0,
                "native signed advance for spacing {spacing}"
            );
            let projection = Provenance::new(&doc, &layout).unwrap();
            assert_eq!(
                projection.layouts[&Path(vec![0])][0]
                    .as_ref()
                    .unwrap()
                    .value["lines"],
                json!(expected)
            );
            // Compare the collector independently with the native document selection API,
            // not only the x_spans/turn_rect assembly above.
            let actual: Vec<_> = projection.layouts[&Path(vec![0])][0]
                .as_ref()
                .unwrap()
                .value["lines"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|line| {
                    line["rects"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(move |rect| json!([line["page"], rect]))
                })
                .collect();
            let native: Vec<_> = layout
                .selection_rects(
                    &doc,
                    &wordcraft_doc::Pos::new(StoryRef::Body, Path(vec![0]), 0),
                    &wordcraft_doc::Pos::new(StoryRef::Body, Path(vec![0]), text.len()),
                    0,
                )
                .into_iter()
                .filter(|(_, rect)| rect.w > 0.0)
                .map(|(page, rect)| json!([page, [rect.x, rect.y, rect.right(), rect.bottom()]]))
                .collect();
            assert_eq!(
                actual, native,
                "native document selection for spacing {spacing}"
            );
            projection.finish().unwrap();
        }
    }

    #[test]
    fn provenance_budgets_accept_exact_and_reject_plus_one_before_native_allocation() {
        let (doc, layout) = fixture();
        let proof = Provenance::new(&doc, &layout).unwrap();
        let used = proof.budget.get();
        let mut exact = Limits {
            lines: used.lines,
            rects: used.rects,
            work: used.work,
            bytes: used.bytes,
            ..Limits::default()
        };
        let maximum = layout
            .pages
            .iter()
            .flat_map(|p| &p.items)
            .filter_map(|item| {
                if let Placed::Lines { para, l0, l1, .. } = item {
                    Some(
                        para.lines[*l0..*l1]
                            .iter()
                            .map(|l| (l.c1 - l.c0) * 64 + 256)
                            .max()
                            .unwrap(),
                    )
                } else {
                    None
                }
            })
            .max()
            .unwrap();
        exact.temporary = maximum;
        Provenance::with_limits(&doc, &layout, exact).unwrap();
        for kind in 0..5 {
            let mut below = exact;
            match kind {
                0 => below.lines -= 1,
                1 => below.rects -= 1,
                2 => below.work -= 1,
                3 => below.bytes -= 1,
                _ => below.temporary -= 1,
            }
            assert!(matches!(
                Provenance::with_limits(&doc, &layout, below),
                Err(EngineError::ResourceLimit(_))
            ));
        }
        let mut overflow = Budget {
            bytes: usize::MAX,
            ..Budget::default()
        };
        assert!(matches!(
            overflow.add(Limits::default(), 0, 0, 0, 1),
            Err(EngineError::ResourceLimit(_))
        ));
    }

    #[test]
    fn provenance_rejects_invalid_ranges_and_nonfinite_values_before_native_filtering() {
        let (doc, original) = fixture();
        for kind in 0..13 {
            let mut layout = original.clone();
            if kind == 9 {
                layout.pages[0].h = f32::NAN;
            } else {
                let item = layout.pages[0]
                    .items
                    .iter_mut()
                    .find(|i| matches!(i, Placed::Lines { .. }))
                    .unwrap();
                let Placed::Lines { para, l0, x, .. } = item else {
                    unreachable!()
                };
                let para = Arc::make_mut(para);
                match kind {
                    0 => *l0 = usize::MAX,
                    1 => para.lines[0].start = 1, // middle of UTF-8 lambda
                    2 => para.lines[0].stop = usize::MAX,
                    3 => para.lines[0].xs[0] = f32::NAN,
                    4 => para.lines[0].vis[0].w = f32::NAN, // native w > 0 would hide it
                    5 => para.lines[0].vis[0].x = f32::INFINITY,
                    6 => para.lines[0].height = -1.0,
                    7 => *x = f32::INFINITY,
                    8 => para.lines[0].c1 = usize::MAX,
                    10 => para.lines[0].vis[0].w = f32::NEG_INFINITY,
                    11 => {
                        para.lines[0].vis[0].x = f32::MAX;
                        para.lines[0].vis[0].w = f32::MAX;
                    }
                    _ => {
                        para.lines[0].vis[0].x = -f32::MAX;
                        para.lines[0].vis[0].w = -f32::MAX;
                    }
                }
            }
            assert!(
                matches!(
                    Provenance::new(&doc, &layout),
                    Err(EngineError::Malformed(_))
                ),
                "case {kind}"
            );
        }
    }

    #[test]
    fn provenance_source_copy_reservations_are_atomic_and_owned() {
        let (doc, layout) = fixture();
        let used = Provenance::new(&doc, &layout).unwrap().budget.get().bytes;
        let path = Path(vec![0]);
        let initial = Provenance::new(&doc, &layout).unwrap();
        let layout_cost = initial.layouts[&path][0].as_ref().unwrap().bytes * 2;
        let cost = (MAP_ALLOWANCE + std::mem::size_of::<Value>()) * 2 + layout_cost;
        for delta in [0, 1] {
            let p = Provenance::with_limits(
                &doc,
                &layout,
                Limits {
                    bytes: used + cost - delta,
                    ..Limits::default()
                },
            )
            .unwrap();
            let value = p.metadata(&path, Some(0), Some(StoryRef::Body));
            if delta == 0 {
                assert_eq!(value["source_path"], json!([0]));
                p.finish().unwrap();
            } else {
                assert!(!value.contains_key("layout"));
                assert!(matches!(p.finish(), Err(EngineError::ResourceLimit(_))));
            }
        }
        let p = Provenance::new(&doc, &layout).unwrap();
        let initial_bytes = p.budget.get().bytes;
        let mut first = p.metadata(&path, Some(0), Some(StoryRef::Body));
        assert_eq!(p.budget.get().bytes, initial_bytes + cost);
        first["layout"]["lines"][0]["source_range"][0] = json!(999);
        let second = p.metadata(&path, Some(0), Some(StoryRef::Body));
        assert_eq!(p.budget.get().bytes, initial_bytes + 2 * cost);
        assert_eq!(second["layout"]["lines"][0]["source_range"][0], 0);
        assert!(!p
            .metadata(&path, Some(0), Some(StoryRef::Part(0)))
            .contains_key("layout"));
        assert!(!p
            .metadata(&path, None, Some(StoryRef::Body))
            .contains_key("layout"));
        assert!(!p.charge_sources(&path.0, usize::MAX));
        assert!(matches!(p.finish(), Err(EngineError::ResourceLimit(_))));
    }

    #[test]
    fn provenance_extract_preserves_text_and_deterministic_paths_through_nested_tables() {
        let mut doc = Document::from_text("First\nLast");
        let mut nested = Table::new(1, 1, 100.0);
        nested.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(Paragraph::with_text(
            "Nested λ",
            Default::default(),
        ))];
        let mut table = Table::new(1, 1, 200.0);
        table.rows[0].cells[0]
            .blocks
            .insert(0, Arc::new(Block::Table(nested)));
        doc.body.insert(1, Arc::new(Block::Table(table)));
        let bytes = wordcraft_docx::write(&doc).unwrap();
        let first = extract(&bytes, None).unwrap();
        let second = extract(&bytes, None).unwrap();
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        let table = &first.pages[0].tables[0];
        let empty = table.cells[0][0].blocks.last().unwrap();
        assert!(empty.text.is_empty());
        let empty_line = &empty.metadata["layout"]["lines"][0];
        assert_eq!(empty_line["source_range"], json!([0, 0]));
        assert_eq!(empty_line["rects"], json!([]));
        assert_eq!(table.metadata["source_path"], json!([1]));
        let nested = table.cells[0][0].blocks[0].table.as_ref().unwrap();
        let paragraph = &nested.cells[0][0].blocks[0];
        assert_eq!(
            paragraph.metadata["source_path"],
            json!([1, 0, 0, 0, 0, 0, 0])
        );
        assert_eq!(paragraph.text, "Nested λ");
        assert!(!paragraph.metadata["layout"]["lines"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(paragraph.bbox.is_none() && table.bbox.is_none());
        assert_eq!(first.pages[0].blocks[0].text, "First");
        assert_eq!(first.pages[0].blocks.last().unwrap().text, "Last");
        assert!(first.pages[0].metadata.contains_key("body_bbox"));
    }
}
