//! Bounded admission for native page-body alignment; no placement algorithm.
use super::*;
use wordcraft_doc::section::PageVAlign;

pub(super) fn name(value: PageVAlign) -> &'static str {
    match value {
        PageVAlign::Top => "top",
        PageVAlign::Center => "center",
        PageVAlign::Bottom => "bottom",
        PageVAlign::Justify => "justify",
    }
}
pub(super) fn parse(value: &Value) -> PyResult<PageVAlign> {
    match value.as_str() {
        Some("top") => Ok(PageVAlign::Top),
        Some("center") => Ok(PageVAlign::Center),
        Some("bottom") => Ok(PageVAlign::Bottom),
        Some("justify") => Ok(PageVAlign::Justify),
        _ => Err(PyValueError::new_err(
            "vertical_alignment must be top, center, bottom or justify",
        )),
    }
}
fn unsupported() -> PyErr {
    EngineError::Unsupported("non-top section alignment requires horizontal next-page single-column sections with horizontal cells and without notes, active even/odd header/footer mode or floating objects; restore top before combining these features".into()).into()
}
fn charge(work: &mut usize, amount: usize, maximum: usize) -> PyResult<()> {
    *work = work
        .checked_add(amount)
        .filter(|n| *n <= maximum)
        .ok_or_else(|| limit("section alignment validation exceeds 2000000 source items"))?;
    Ok(())
}
pub(super) fn profile(document: &Document) -> PyResult<bool> {
    profile_with_limit(document, MAX_LAYOUT_ITEMS)
}
// Called after the existing bounded exact-model inventory at commit and output.
// The independent charge also bounds this scan and its reference stack.
fn profile_with_limit(document: &Document, maximum: usize) -> PyResult<bool> {
    if document.body.len() > MAX_BLOCKS {
        return Err(limit(
            "section alignment validation exceeds 100000 body blocks",
        ));
    }
    let mut work = 0;
    charge(&mut work, document.body.len(), maximum)?;
    let sections = document.sections();
    if sections.len() > MAX_SECTIONS {
        return Err(limit("document exceeds 128 sections"));
    }
    charge(&mut work, sections.len(), maximum)?;
    if sections
        .iter()
        .all(|(_, section)| section.valign == PageVAlign::Top)
    {
        return Ok(false);
    }
    charge(&mut work, sections.len(), maximum)?;
    // Section cloning creates inactive empty even-story parts; only the native
    // document setting activates even selection, and ensure_story enables it.
    if document.settings.even_odd_headers
        || sections.iter().any(|(_, s)| {
            s.start != SectionStart::NextPage
                || s.columns.count != 1
                || s.columns.widths.len() > 1
                || s.text_direction != TextDirection::Horizontal
        })
    {
        return Err(unsupported());
    }
    charge(&mut work, document.parts.len(), maximum)?;
    charge(&mut work, document.parts.len(), maximum)?;
    let mut pending = Vec::new();
    for blocks in
        std::iter::once(&document.body).chain(document.parts.values().map(|part| &part.blocks))
    {
        charge(&mut work, blocks.len(), maximum)?;
        pending.extend(blocks.iter().map(|block| (block.as_ref(), 0usize)));
    }
    if document
        .parts
        .values()
        .any(|part| matches!(part.kind, PartKind::Footnote | PartKind::Endnote))
    {
        return Err(unsupported());
    }
    while let Some((block, depth)) = pending.pop() {
        if depth > 16 {
            return Err(limit(
                "section alignment validation exceeds 16 table levels",
            ));
        }
        match block {
            Block::Para(p) => {
                charge(&mut work, p.objects.len(), maximum)?;
                // Native group children are independently bounded before inspection too.
                let mut objects: Vec<_> = p.objects.iter().collect();
                while let Some(object) = objects.pop() {
                    let float = match object {
                        InlineObject::NoteRef { .. } => return Err(unsupported()),
                        InlineObject::Image { float, .. }
                        | InlineObject::Shape { float, .. }
                        | InlineObject::Graphic { float, .. } => Some(float),
                        InlineObject::Group {
                            float, children, ..
                        } => {
                            charge(&mut work, children.len(), maximum)?;
                            objects.extend(children.iter().map(|child| &child.obj));
                            Some(float)
                        }
                        _ => None,
                    };
                    if float.is_some_and(|value| value.wrap != Wrap::Inline) {
                        return Err(unsupported());
                    }
                }
            }
            Block::Table(table) => {
                charge(&mut work, table.rows.len(), maximum)?;
                for row in &table.rows {
                    charge(&mut work, row.cells.len(), maximum)?;
                    for cell in &row.cells {
                        if cell.props.text_direction != TextDirection::Horizontal {
                            return Err(unsupported());
                        }
                        charge(&mut work, cell.blocks.len(), maximum)?;
                        pending.extend(cell.blocks.iter().map(|block| (block.as_ref(), depth + 1)));
                    }
                }
            }
        }
    }
    Ok(true)
}

pub(super) fn page_bounds(page: &wordcraft_layout::Page) -> PyResult<()> {
    let bad = || {
        EngineError::Unsupported("non-top section alignment places body content outside its vertical page bounds; reduce content or formatting, enlarge the page, or restore top".into()).into()
    };
    let body_top = page.body.y;
    let body_bottom = page.body.bottom();
    if !page.h.is_finite()
        || page.h <= 0.0
        || !body_top.is_finite()
        || !body_bottom.is_finite()
        || body_top < 0.0
        || body_bottom < body_top
        || body_bottom > page.h + 0.01
    {
        return Err(bad());
    }
    for item in &page.items {
        let (top, bottom) = match item {
            wordcraft_layout::Placed::Lines {
                story: StoryRef::Body,
                para,
                l0,
                l1,
                y,
                turn,
                ..
            } => {
                if *turn != TextDirection::Horizontal {
                    return Err(bad());
                }
                if l0 > l1 || *l1 > para.lines.len() {
                    return Err(bad());
                }
                if l0 == l1 {
                    continue;
                }
                let Some(first) = para.lines.get(*l0) else {
                    return Err(bad());
                };
                let Some(last) = l1.checked_sub(1).and_then(|index| para.lines.get(index)) else {
                    return Err(bad());
                };
                if !first.top.is_finite()
                    || !last.top.is_finite()
                    || !last.height.is_finite()
                    || last.height < 0.0
                {
                    return Err(bad());
                }
                (*y, *y + last.top - first.top + last.height)
            }
            wordcraft_layout::Placed::Cell {
                story: StoryRef::Body,
                rect,
                ..
            } => (rect.y, rect.bottom()),
            _ => continue,
        };
        if !top.is_finite()
            || !bottom.is_finite()
            || bottom < top
            || top < body_top - 0.1
            || bottom > body_bottom + 0.1
        {
            return Err(bad());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn aligned(mode: &str) -> EditableDocx {
        let mut d = EditableDocx::new();
        d.set_page_size(320.0, 420.0).unwrap();
        d.set_margins(52.0, 36.0, 52.0, 36.0).unwrap();
        d.add_paragraph("Body value").unwrap();
        set(&mut d, mode).unwrap();
        d
    }
    fn set(d: &mut EditableDocx, mode: &str) -> PyResult<()> {
        d.set_section_props(0, &json!({"vertical_alignment":mode}).to_string())
    }
    fn unsupported_error(result: PyResult<()>) {
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("non-top section alignment"));
    }
    fn png(color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 4, 4);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&color.repeat(16))
                .unwrap();
        }
        bytes
    }
    #[test]
    fn section_alignment_values_copy_revision_and_output_recovery() {
        pyo3::prepare_freethreaded_python();
        let mut d = aligned("top");
        let original = d.to_bytes().unwrap();
        for mode in ["center", "bottom", "justify", "top"] {
            let before = d.revision;
            set(&mut d, mode).unwrap();
            assert_eq!(d.revision, before + 1);
            assert_eq!(
                serde_json::from_str::<Value>(&d.section_props(0).unwrap()).unwrap()
                    ["vertical_alignment"],
                mode
            );
            let parsed = wordcraft_docx::read(&d.to_bytes().unwrap()).unwrap();
            assert_eq!(name(parsed.last_section.valign), mode);
            let mut copy = d.copy().unwrap();
            set(&mut copy, if mode == "top" { "center" } else { "top" }).unwrap();
            assert_eq!(name(d.document().unwrap().last_section.valign), mode);
        }
        assert_eq!(d.to_bytes().unwrap(), original);
        let revision = d.revision;
        for value in [
            json!(null),
            json!(1),
            json!(true),
            json!("both"),
            json!("Center"),
            json!([]),
        ] {
            assert!(d
                .set_section_props(0, &json!({"vertical_alignment":value}).to_string())
                .unwrap_err()
                .to_string()
                .contains("vertical_alignment"));
            assert_eq!(d.revision, revision);
            assert_eq!(d.to_bytes().unwrap(), original);
        }
        let oversized = " ".repeat(MAX_FORMAT_PATCH_BYTES + 1);
        assert!(d
            .set_section_props(0, &oversized)
            .unwrap_err()
            .to_string()
            .contains("16 KiB"));
        d.revision = u64::MAX - 1;
        set(&mut d, "center").unwrap();
        assert_eq!(d.revision, u64::MAX);
        let bytes = d.to_bytes().unwrap();
        assert!(set(&mut d, "top")
            .unwrap_err()
            .to_string()
            .contains("revision limit"));
        assert_eq!(d.to_bytes().unwrap(), bytes);
        d.revision = revision;
        d.document.as_mut().unwrap().last_section.columns.count = 2;
        assert!(d
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("non-top section alignment"));
        let snapshot = wordcraft_docx::write(d.document().unwrap()).unwrap();
        unsupported_error(d.add_paragraph("Rejected").map(|_| ()));
        assert_eq!(
            wordcraft_docx::write(d.document().unwrap()).unwrap(),
            snapshot
        );
        assert_eq!(d.revision, revision);
        set(&mut d, "top").unwrap();
        assert!(!d.to_bytes().unwrap().is_empty());
    }
    #[test]
    fn section_alignment_profile_later_edits_sections_and_registry_are_atomic() {
        pyo3::prepare_freethreaded_python();
        for mode in ["center", "bottom", "justify"] {
            let mut d = aligned(mode);
            let p = d.paragraph_ids().unwrap()[0];
            let second = d.add_section("next_page").unwrap();
            assert_eq!(name(d.section(second).unwrap().valign), mode);
            assert!(d.section(second).unwrap().headers.even.is_some());
            assert!(!d.document().unwrap().settings.even_odd_headers);
            d.add_paragraph("Second section").unwrap();
            let before = d.to_bytes().unwrap();
            let revision = d.revision;
            let locations = d.locations.clone();
            let next = d.next_block_id;
            unsupported_error(d.ensure_story("header", "even", second).map(|_| ()));
            unsupported_error(d.set_section_props(second, r#"{"columns":[2,20,false]}"#));
            unsupported_error(d.add_section("continuous").map(|_| ()));
            unsupported_error(d.add_section("odd_page").map(|_| ()));
            unsupported_error(
                d.add_note(p, 0, 0, revision, "Note", "footnote")
                    .map(|_| ()),
            );
            assert_eq!(d.to_bytes().unwrap(), before);
            assert_eq!(d.revision, revision);
            assert_eq!(d.locations, locations);
            assert_eq!(d.next_block_id, next);
            // Reset every non-top section; only then can active even stories be admitted.
            set(&mut d, "top").unwrap();
            unsupported_error(d.ensure_story("header", "even", second).map(|_| ()));
            d.set_section_props(second, r#"{"vertical_alignment":"top"}"#)
                .unwrap();
            d.ensure_story("header", "even", second).unwrap();
            unsupported_error(set(&mut d, mode));
        }
        for direction in ["up", "down"] {
            for mode in ["center", "bottom", "justify"] {
                let mut d = aligned("top");
                let table = d.add_table(vec![vec!["Cell".into()]]).unwrap();
                let nested = d
                    .cell_add_table(table, 0, 0, vec![vec!["Nested".into()]])
                    .unwrap();
                let patch = json!({"text_direction": direction}).to_string();
                d.cell_format_patch(nested, 0, 0, &patch).unwrap();
                let before = d.to_bytes().unwrap();
                let revision = d.revision;
                unsupported_error(set(&mut d, mode));
                assert_eq!(d.to_bytes().unwrap(), before);
                assert_eq!(d.revision, revision);
                d.cell_format_patch(nested, 0, 0, r#"{"text_direction":null}"#)
                    .unwrap();
                set(&mut d, mode).unwrap();
                let before = d.to_bytes().unwrap();
                let revision = d.revision;
                unsupported_error(d.cell_format_patch(table, 0, 0, &patch));
                unsupported_error(d.cell_format_patch(nested, 0, 0, &patch));
                assert_eq!(d.to_bytes().unwrap(), before);
                assert_eq!(d.revision, revision);
                set(&mut d, "top").unwrap();
                d.cell_format_patch(table, 0, 0, &patch).unwrap();
                assert!(!d.to_bytes().unwrap().is_empty());
            }
        }
        let mut d = aligned("top");
        let image = d
            .add_image(&png([255, 0, 0, 255]), Some(16.0), Some(16.0), "Inline")
            .unwrap();
        set(&mut d, "center").unwrap();
        let before = d.to_bytes().unwrap();
        unsupported_error(d.format_image(
            image,
            r#"{"wrap":"behind","anchor":"page","position":[100,100]}"#,
            d.revision,
        ));
        assert_eq!(d.to_bytes().unwrap(), before);
        set(&mut d, "top").unwrap();
        d.format_image(
            image,
            r#"{"wrap":"behind","anchor":"page","position":[100,100]}"#,
            d.revision,
        )
        .unwrap();
        unsupported_error(set(&mut d, "center"));
    }
    #[test]
    fn section_alignment_source_work_bound_and_nested_foreign_object_guard() {
        pyo3::prepare_freethreaded_python();
        let mut d = Document::new();
        d.last_section.valign = PageVAlign::Center;
        d.body = vec![wordcraft_doc::para_block(Paragraph::with_text(
            "Text",
            Default::default(),
        ))];
        // One body entry scanned for sections, two section passes, one root block.
        assert!(profile_with_limit(&d, 4).unwrap());
        assert!(profile_with_limit(&d, 3)
            .unwrap_err()
            .to_string()
            .contains("source items"));
        for direction in [TextDirection::Up, TextDirection::Down] {
            d.last_section.text_direction = direction;
            assert!(profile(&d)
                .unwrap_err()
                .to_string()
                .contains("non-top section alignment"));
        }
        d.last_section.text_direction = TextDirection::Horizontal;
        let snapshot = wordcraft_docx::write(&d).unwrap();
        assert!(profile_with_limit(&d, 4).unwrap());
        assert_eq!(wordcraft_docx::write(&d).unwrap(), snapshot);
        let mut table = Table::new(1, 1, 200.0);
        let mut p = Paragraph::new();
        p.insert_object(
            0,
            InlineObject::Group {
                w: 16.0,
                h: 16.0,
                float: Float::default(),
                ch_w: 16.0,
                ch_h: 16.0,
                canvas: None,
                children: vec![wordcraft_doc::para::GroupChild {
                    x: 0.0,
                    y: 0.0,
                    obj: InlineObject::Image {
                        media: "unused".into(),
                        w: 16.0,
                        h: 16.0,
                        alt: String::new(),
                        float: Float {
                            wrap: Wrap::BehindText,
                            ..Default::default()
                        },
                        crop: [0.0; 4],
                        ole: None,
                    },
                }],
            },
            &Default::default(),
        )
        .unwrap();
        table.rows[0].cells[0].blocks = vec![wordcraft_doc::para_block(p)];
        d.body.insert(0, Arc::new(Block::Table(table)));
        assert!(profile(&d)
            .unwrap_err()
            .to_string()
            .contains("non-top section alignment"));
        d.last_section.valign = PageVAlign::Top;
        assert!(!profile(&d).unwrap());
    }
    #[test]
    fn section_alignment_vertical_output_guard_is_finite_and_horizontal_independent() {
        pyo3::prepare_freethreaded_python();
        let d = aligned("center");
        let l = wordcraft_layout::layout(
            d.document().unwrap(),
            &mut Default::default(),
            &Default::default(),
        );
        let mut page = l.pages[0].clone();
        page_bounds(&page).unwrap();
        page.body.x = -1000.0;
        page.body.w = -1000.0;
        page_bounds(&page).unwrap(); // This guard deliberately does not constrain horizontal advances.
        for bad_y in [f32::NAN, f32::INFINITY, -100.0, 1000.0] {
            let mut page = l.pages[0].clone();
            let line = page
                .items
                .iter_mut()
                .find(|item| {
                    matches!(
                        item,
                        wordcraft_layout::Placed::Lines {
                            story: StoryRef::Body,
                            ..
                        }
                    )
                })
                .unwrap();
            if let wordcraft_layout::Placed::Lines { y, .. } = line {
                *y = bad_y;
            }
            assert!(page_bounds(&page)
                .unwrap_err()
                .to_string()
                .contains("vertical page bounds"));
        }
        for direction in [TextDirection::Up, TextDirection::Down] {
            let mut page = l.pages[0].clone();
            for item in &mut page.items {
                if let wordcraft_layout::Placed::Lines { turn, .. } = item {
                    *turn = direction;
                }
            }
            assert!(page_bounds(&page)
                .unwrap_err()
                .to_string()
                .contains("vertical page bounds"));
        }
        let mut malformed = l.pages[0].clone();
        for item in &mut malformed.items {
            if let wordcraft_layout::Placed::Lines { l0, l1, para, .. } = item {
                *l0 = para.lines.len() + 1;
                *l1 = *l0;
            }
        }
        assert!(page_bounds(&malformed).is_err());
        malformed = l.pages[0].clone();
        malformed.h = f32::NAN;
        assert!(page_bounds(&malformed).is_err());
        let mut page = l.pages[0].clone();
        page.body.y = f32::NAN;
        assert!(page_bounds(&page).is_err());
        let mut d = aligned("bottom");
        let p = d.paragraph_ids().unwrap()[0];
        d.format_range_patch(p, 0, 10, d.revision, r#"{"font_size":400}"#)
            .unwrap();
        assert!(d
            .to_bytes()
            .unwrap_err()
            .to_string()
            .contains("vertical page bounds"));
        d.format_range_patch(p, 0, 10, d.revision, r#"{"font_size":12}"#)
            .unwrap();
        assert!(!d.to_bytes().unwrap().is_empty());
    }
    fn rgba(bytes: &[u8]) -> (u32, Vec<u8>) {
        let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        pixels.truncate(info.buffer_size());
        (info.width, pixels)
    }
    fn red_bounds(width: u32, pixels: &[u8]) -> (u32, u32, u32, u32) {
        let mut bounds = (u32::MAX, u32::MAX, 0, 0);
        let mut count = 0;
        for (i, p) in pixels.chunks_exact(4).enumerate() {
            if i32::from(p[0]) > 120
                && i32::from(p[0]) > i32::from(p[1]) + 60
                && i32::from(p[0]) > i32::from(p[2]) + 60
            {
                let (x, y) = (i as u32 % width, i as u32 / width);
                bounds = (
                    bounds.0.min(x),
                    bounds.1.min(y),
                    bounds.2.max(x + 1),
                    bounds.3.max(y + 1),
                );
                count += 1;
            }
        }
        assert!(count > 5);
        bounds
    }
    #[test]
    fn section_alignment_background_color_picture_and_source_readback_pdf_compose() {
        pyo3::prepare_freethreaded_python();
        for mode in ["center", "bottom", "justify"] {
            let mut d = aligned(mode);
            let p = d.paragraph_ids().unwrap()[0];
            d.format_range_patch(p, 0, 10, d.revision, r##"{"color":"#FF0000"}"##)
                .unwrap();
            d.add_paragraph("Second body").unwrap();
            d.set_background(Some("#DFF0FA")).unwrap();
            for picture in [false, true] {
                if picture {
                    d.set_background_image(Some(&png([32, 96, 192, 255])))
                        .unwrap();
                }
                let bytes = d.to_bytes().unwrap();
                let parsed = wordcraft_docx::read(&bytes).unwrap();
                let original = wordcraft_layout::layout(
                    d.document().unwrap(),
                    &mut Default::default(),
                    &Default::default(),
                );
                let reopened =
                    wordcraft_layout::layout(&parsed, &mut Default::default(), &Default::default());
                assert_eq!(original.pages.len(), 1);
                assert_eq!(reopened.pages.len(), 1);
                for source in [d.document().unwrap(), &parsed] {
                    let layout = wordcraft_layout::layout(
                        source,
                        &mut Default::default(),
                        &Default::default(),
                    );
                    let pdf = wordcraft_pdf::export(source, &Default::default()).unwrap();
                    let text = crate::pdf::extract(&pdf, None).unwrap().pages[0]
                        .blocks
                        .iter()
                        .map(|b| b.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    assert_eq!(text.matches("Body value").count(), 1);
                    for dpi in [72.0, 144.0] {
                        let mut options = wordcraft_render::RenderOptions::default();
                        options.display.dim_header = false;
                        let native = wordcraft_render::render_page(
                            source,
                            &layout.pages[0],
                            dpi as f32 / 72.0,
                            &options,
                        )
                        .to_png();
                        let converted = crate::pdf::render(&pdf, 0, dpi, None).unwrap();
                        let serialized = crate::docx::render(&bytes, 0, dpi, None).unwrap();
                        let expected = if picture {
                            [32, 96, 192, 255]
                        } else {
                            [223, 240, 250, 255]
                        };
                        let mut bounds = Vec::new();
                        for rendered in [&native, &converted, &serialized] {
                            let (width, pixels) = rgba(rendered);
                            for (actual, expected) in pixels[..4].iter().zip(expected) {
                                assert!((i32::from(*actual) - expected).abs() <= 1);
                            }
                            bounds.push(red_bounds(width, &pixels));
                        }
                        for other in &bounds[1..] {
                            let a = bounds[0];
                            for (x, y) in [
                                (a.0, other.0),
                                (a.1, other.1),
                                (a.2, other.2),
                                (a.3, other.3),
                            ] {
                                assert!(x.abs_diff(y) <= 2, "{mode}/{picture}: {bounds:?}");
                            }
                        }
                    }
                }
                assert_eq!(name(parsed.last_section.valign), mode);
                assert_eq!(parsed.settings.page_picture.is_some(), picture);
            }
            d.set_background_image(None).unwrap();
            assert_eq!(d.background().unwrap(), Some("#DFF0FA".into()));
        }
    }
}
