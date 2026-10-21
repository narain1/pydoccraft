//! Exact owned closed-shape admission and public native edits, not a renderer.
use super::*;
use wordcraft_doc::para::{ShapeKind, WrapText};

pub(super) const MAX_SHAPES: usize = 1000;
#[derive(Clone, Copy, Debug)]
pub(super) struct Bounds {
    pub w: f32,
    pub h: f32,
    pub stroke: f32,
}
pub(super) type Inventory = BTreeMap<Path, Bounds>;
fn unsupported() -> PyErr {
    EngineError::Unsupported("DOCX shapes require an owned direct-body rectangle or ellipse in horizontal sections without rotation, text, groups, effects or auxiliary placement".into()).into()
}
fn invalid(message: &str) -> PyErr {
    PyValueError::new_err(message.to_owned())
}
fn finite_length(value: f32, min: f32, max: f32) -> bool {
    value.is_finite()
        && (min..=max).contains(&value)
        && (value * 20.0 - (value * 20.0).round()).abs()
            <= 4.0 * f32::EPSILON * (value * 20.0).abs().max(1.0)
}
pub(super) fn supported(object: &InlineObject) -> bool {
    let InlineObject::Shape {
        kind,
        w,
        h,
        stroke,
        stroke_width,
        float,
        story,
        freeform,
        effects,
        extra,
        ..
    } = object
    else {
        return false;
    };
    matches!(kind, ShapeKind::Rectangle | ShapeKind::Ellipse)
        && finite_length(*w, 1.0, 4000.0)
        && finite_length(*h, 1.0, 4000.0)
        && finite_length(*stroke_width, 0.25, 100.0)
        && (stroke.is_none() || *stroke_width <= w.min(*h))
        && story.is_none()
        && freeform.is_none()
        && *effects == Default::default()
        && extra.is_empty()
        && float.wrap_text == WrapText::BothSides
        && float.h_align.is_none()
        && float.v_align.is_none()
        && float.rot == 0.0
        && !float.flip_h
        && !float.flip_v
        && !float.hidden
        && float.effect == [0.0; 4]
        && float.wrap_polygon.is_none()
        && float.alt == Default::default()
        && if float.wrap == Wrap::Inline {
            float.h_rel == Anchor::Column
                && float.v_rel == Anchor::Paragraph
                && float.x == 0.0
                && float.y == 0.0
                && float.dist == 0.0
                && float.dist_top == 0.0
                && float.dist_bottom == 0.0
        } else {
            matches!(
                float.wrap,
                Wrap::Square | Wrap::TopAndBottom | Wrap::BehindText
            ) && matches!(
                (float.h_rel, float.v_rel),
                (Anchor::Page, Anchor::Page)
                    | (Anchor::Margin, Anchor::Margin)
                    | (Anchor::Column, Anchor::Paragraph)
            ) && finite_length(float.x, -31680.0, 31680.0)
                && finite_length(float.y, -31680.0, 31680.0)
                && finite_length(float.dist, 0.0, 1584.0)
                && finite_length(float.dist_top, 0.0, 1584.0)
                && finite_length(float.dist_bottom, 0.0, 1584.0)
        }
}
fn charge(used: &mut usize, n: usize, max: usize) -> PyResult<()> {
    *used = used
        .checked_add(n)
        .filter(|x| *x <= max)
        .ok_or_else(|| limit("shape validation exceeds bounded source work"))?;
    Ok(())
}
pub(super) fn inventory(d: &Document) -> PyResult<Inventory> {
    inventory_with_limit(d, MAX_LAYOUT_ITEMS)
}
fn inventory_with_limit(d: &Document, max: usize) -> PyResult<Inventory> {
    if d.body.len() > MAX_BLOCKS {
        return Err(limit("shape validation exceeds 100000 body blocks"));
    }
    let mut work = 0;
    charge(&mut work, d.body.len(), max)?;
    charge(&mut work, d.parts.len(), max)?;
    let sections = d.sections();
    if sections.len() > MAX_SECTIONS {
        return Err(limit("document exceeds 128 sections"));
    }
    charge(&mut work, sections.len(), max)?;
    let mut pending = Vec::new();
    for (blocks, body) in
        std::iter::once((&d.body, true)).chain(d.parts.values().map(|p| (&p.blocks, false)))
    {
        charge(&mut work, blocks.len(), max)?;
        pending.push((blocks, body, 0usize));
    }
    for separators in [&d.footnote_separators, &d.endnote_separators] {
        for blocks in [
            &separators.separator,
            &separators.continuation_separator,
            &separators.continuation_notice,
        ]
        .into_iter()
        .flatten()
        {
            charge(&mut work, blocks.len(), max)?;
            pending.push((blocks, false, 0));
        }
    }
    let mut result = BTreeMap::new();
    let mut blocks_seen = 0;
    let mut cells = 0;
    let mut opaque_drawing = false;
    while let Some((blocks, body, depth)) = pending.pop() {
        if depth > 16 {
            return Err(limit("shape validation exceeds 16 table levels"));
        }
        charge(&mut blocks_seen, blocks.len(), MAX_BLOCKS)?;
        for (index, block) in blocks.iter().enumerate() {
            match block.as_ref() {
                Block::Para(p) => {
                    charge(&mut work, p.objects.len(), max)?;
                    for o in &p.objects {
                        match o {
                            InlineObject::Shape {
                                w,
                                h,
                                stroke,
                                stroke_width,
                                ..
                            } => {
                                if !body
                                    || depth != 0
                                    || p.text != "\u{fffc}"
                                    || p.objects.len() != 1
                                    || !supported(o)
                                {
                                    return Err(unsupported());
                                }
                                if result.len() >= MAX_SHAPES {
                                    return Err(limit(
                                        "document exceeds 1000 closed shape placements",
                                    ));
                                }
                                if let InlineObject::Shape { float, w, .. } = o {
                                    if float.wrap == Wrap::Inline {
                                        charge(&mut work, sections.len(), max)?;
                                        let section = sections
                                            .iter()
                                            .find(|(end, _)| index <= *end)
                                            .map(|(_, s)| *s)
                                            .ok_or_else(|| {
                                                invalid("shape has no native section")
                                            })?;
                                        if *w > column_width(section) + 0.001 {
                                            return Err(invalid(
                                                "inline shape width exceeds its section column",
                                            ));
                                        }
                                    }
                                }
                                let index = u32::try_from(index)
                                    .map_err(|_| limit("shape path exceeds native range"))?;
                                result.insert(
                                    Path::top(index as usize),
                                    Bounds {
                                        w: *w,
                                        h: *h,
                                        stroke: if stroke.is_some() { *stroke_width } else { 0.0 },
                                    },
                                );
                            }
                            InlineObject::Group { .. } => return Err(unsupported()),
                            InlineObject::Graphic { .. }
                            | InlineObject::Image { ole: Some(_), .. } => opaque_drawing = true,
                            _ => {}
                        }
                    }
                }
                Block::Table(t) => {
                    charge(&mut work, t.rows.len(), max)?;
                    for row in &t.rows {
                        charge(&mut work, row.cells.len(), max)?;
                        charge(&mut cells, row.cells.len(), MAX_CELLS)?;
                        for cell in &row.cells {
                            charge(&mut work, cell.blocks.len(), max)?;
                            pending.push((&cell.blocks, false, depth + 1));
                        }
                    }
                }
            }
        }
    }
    if !result.is_empty()
        && (opaque_drawing
            || sections
                .iter()
                .any(|(_, section)| section.text_direction != TextDirection::Horizontal))
    {
        return Err(unsupported());
    }
    Ok(result)
}
pub(super) fn registry(owner: &EditableDocx, inventory: &Inventory) -> PyResult<()> {
    if owner.shape_ids.len() > MAX_SHAPES || owner.locations.len() > MAX_BLOCKS {
        return Err(limit("shape registry exceeds owned limits"));
    }
    if owner.shape_ids.len() != inventory.len() {
        return Err(invalid("shape registry does not match native placements"));
    }
    for (id, (story, path)) in &owner.locations {
        if *story == StoryRef::Body && inventory.contains_key(path) && !owner.shape_ids.contains(id)
        {
            return Err(invalid("shape path aliases another owned child"));
        }
    }
    let mut found = BTreeSet::new();
    for id in &owner.shape_ids {
        let (story, path) = owner
            .locations
            .get(id)
            .ok_or_else(|| invalid("shape has no owned location"))?;
        if *story != StoryRef::Body
            || path.0.len() != 1
            || !inventory.contains_key(path)
            || !found.insert(path)
            || owner.images.contains(id)
            || owner.table_generations.contains_key(id)
            || owner.body_placeholder == Some(path.0[0] as usize)
        {
            return Err(invalid("invalid or aliased shape location"));
        }
    }
    Ok(())
}
fn get(owner: &EditableDocx, id: usize) -> PyResult<&InlineObject> {
    owner.document()?;
    if !owner.shape_ids.contains(&id) {
        return Err(PyIndexError::new_err(
            "shape index does not identify a live owned shape",
        ));
    }
    let (story, path) = owner
        .locations
        .get(&id)
        .ok_or_else(|| invalid("shape location is missing"))?;
    if *story != StoryRef::Body || path.0.len() != 1 {
        return Err(unsupported());
    }
    let p = owner
        .document()?
        .para(*story, path)
        .ok_or_else(|| invalid("shape paragraph is missing"))?;
    if p.text != "\u{fffc}" || p.objects.len() != 1 || !supported(&p.objects[0]) {
        return Err(unsupported());
    }
    Ok(&p.objects[0])
}
fn color(value: &Value) -> PyResult<Option<Rgb>> {
    if value.is_null() {
        return Ok(None);
    }
    let values = value
        .as_array()
        .filter(|v| v.len() == 3)
        .ok_or_else(|| invalid("shape color requires RGB or null"))?;
    let channels = values
        .iter()
        .map(|v| {
            v.as_u64()
                .filter(|c| *c <= 255)
                .map(|c| c as u8)
                .ok_or_else(|| invalid("shape RGB channels require integers from 0 to 255"))
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(Some(Rgb(channels[0], channels[1], channels[2])))
}
fn length(value: &Value, min: f64, max: f64) -> PyResult<f32> {
    let v = value
        .as_f64()
        .filter(|v| v.is_finite() && (min..=max).contains(v))
        .ok_or_else(|| invalid("shape length is outside its finite supported range"))?;
    let n = (v * 20.0).round() / 20.0;
    if (n - v).abs() > 4.0 * f64::EPSILON * v.abs().max(1.0) {
        return Err(invalid("shape lengths require 0.05-point increments"));
    }
    Ok(n as f32)
}
fn properties(object: &mut InlineObject, input: &str, require_size: bool) -> PyResult<()> {
    if input.len() > 1024 {
        return Err(limit("shape properties exceed 1 KiB"));
    }
    let map: Map<String, Value> = serde_json::from_str(input)
        .map_err(|_| invalid("shape properties require a JSON object"))?;
    if require_size && !map.contains_key("size") {
        return Err(invalid("shape creation requires width and height"));
    }
    for (key, value) in map {
        match key.as_str() {
            "size" => {
                let pair = value
                    .as_array()
                    .filter(|v| v.len() == 2)
                    .ok_or_else(|| invalid("shape size requires two lengths"))?;
                object.set_size(
                    length(&pair[0], 1.0, 4000.0)?,
                    length(&pair[1], 1.0, 4000.0)?,
                );
            }
            "fill" => {
                let InlineObject::Shape { fill, .. } = object else {
                    unreachable!()
                };
                *fill = color(&value)?;
            }
            "stroke" => {
                let InlineObject::Shape {
                    stroke,
                    stroke_width,
                    ..
                } = object
                else {
                    unreachable!()
                };
                if value.is_null() {
                    *stroke = None;
                    *stroke_width = 1.0;
                } else {
                    let pair = value
                        .as_array()
                        .filter(|v| v.len() == 2)
                        .ok_or_else(|| invalid("shape stroke requires color and width"))?;
                    *stroke = color(&pair[0])?;
                    if stroke.is_none() {
                        return Err(invalid(
                            "stroke color cannot be null; clear stroke with null",
                        ));
                    }
                    *stroke_width = length(&pair[1], 0.25, 100.0)?;
                }
            }
            _ => return Err(invalid("unsupported shape property")),
        }
    }
    if !supported(object) {
        return Err(unsupported());
    }
    Ok(())
}
pub(super) fn ids(owner: &EditableDocx) -> PyResult<Vec<usize>> {
    let map = inventory(owner.document()?)?;
    registry(owner, &map)?;
    let mut ids = owner
        .shape_ids
        .iter()
        .map(|id| (owner.locations[id].1.clone(), *id))
        .collect::<Vec<_>>();
    ids.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(ids.into_iter().map(|(_, id)| id).collect())
}
pub(super) fn info(owner: &EditableDocx, id: usize) -> PyResult<String> {
    let InlineObject::Shape {
        kind,
        w,
        h,
        fill,
        stroke,
        stroke_width,
        ..
    } = get(owner, id)?
    else {
        unreachable!()
    };
    let rgb = |c: &Rgb| [c.0, c.1, c.2];
    Ok(json!({"kind":if *kind==ShapeKind::Rectangle{"rectangle"}else{"ellipse"},"size":[w,h],"fill":fill.as_ref().map(rgb),"stroke":stroke.as_ref().map(|c|(rgb(c),stroke_width))}).to_string())
}
pub(super) fn formatting(owner: &EditableDocx, id: usize) -> PyResult<String> {
    let (_, _, float) = get(owner, id)?.frame().unwrap();
    picture_snapshot(float)
}
fn revision(owner: &EditableDocx, expected: u64) -> PyResult<()> {
    owner.document()?;
    if owner.revision != expected {
        return Err(invalid("document changed while converting shape values"));
    }
    Ok(())
}
pub(super) fn add(
    owner: &mut EditableDocx,
    kind: &str,
    input: &str,
    expected: u64,
) -> PyResult<usize> {
    revision(owner, expected)?;
    owner.reserve_block()?;
    if owner.shape_ids.len() >= MAX_SHAPES {
        return Err(limit("document exceeds 1000 closed shape placements"));
    }
    let kind = match kind {
        "rectangle" => ShapeKind::Rectangle,
        "ellipse" => ShapeKind::Ellipse,
        "line" | "textbox" => return Err(unsupported()),
        _ => return Err(invalid("shape kind must be rectangle or ellipse")),
    };
    let mut object = InlineObject::Shape {
        kind,
        w: 1.0,
        h: 1.0,
        fill: None,
        stroke: None,
        stroke_width: 1.0,
        float: Float {
            v_rel: Anchor::Paragraph,
            ..Default::default()
        },
        story: None,
        freeform: None,
        effects: Default::default(),
        extra: Default::default(),
    };
    properties(&mut object, input, true)?;
    let mut paragraph = Paragraph::new();
    paragraph
        .insert_object(0, object, &CharProps::default())
        .map_err(model_error)?;
    let (id, next) = owner.next_identifier()?;
    let mut staged = owner.copy()?;
    let mut candidate = owner.document()?.clone();
    if candidate.body.len() >= MAX_BLOCKS {
        return Err(limit("document exceeds 100000 blocks"));
    }
    let index = owner.body_placeholder.unwrap_or(candidate.body.len());
    if owner.body_placeholder.is_some() {
        *candidate
            .body
            .get_mut(index)
            .ok_or_else(|| invalid("body placeholder is missing"))? =
            wordcraft_doc::para_block(paragraph);
    } else {
        candidate
            .insert_block(StoryRef::Body, &Path::top(index), Block::Para(paragraph))
            .map_err(model_error)?;
    }
    staged.next_block_id = next;
    staged.body_placeholder = None;
    staged.shape_ids.insert(id);
    staged
        .locations
        .insert(id, (StoryRef::Body, Path::top(index)));
    staged.commit(candidate)?;
    *owner = staged;
    Ok(id)
}
pub(super) fn update(
    owner: &mut EditableDocx,
    id: usize,
    input: &str,
    expected: u64,
    placement: bool,
) -> PyResult<()> {
    revision(owner, expected)?;
    let mut object = get(owner, id)?.clone();
    if placement {
        let old = object.frame().unwrap().2;
        let new = picture_patch(old, input)?;
        *object.float_mut().unwrap() = new;
        if !supported(&object) {
            return Err(unsupported());
        }
    } else {
        properties(&mut object, input, false)?;
    }
    if &object == get(owner, id)? {
        return Ok(());
    }
    let mut candidate = owner.document()?.clone();
    let p = owner.paragraph_mut(&mut candidate, id)?;
    p.objects[0] = object;
    p.touch();
    owner.commit(candidate)
}
pub(super) fn remove(owner: &mut EditableDocx, id: usize, expected: u64) -> PyResult<()> {
    revision(owner, expected)?;
    get(owner, id)?;
    let mut staged = owner.copy()?;
    let mut candidate = owner.document()?.clone();
    owner
        .paragraph_mut(&mut candidate, id)?
        .delete(0, wordcraft_doc::para::OBJ.len_utf8())
        .map_err(model_error)?;
    staged.shape_ids.remove(&id);
    staged.commit(candidate)?;
    *owner = staged;
    Ok(())
}
pub(super) fn page_bounds(
    page: &wordcraft_layout::Page,
    map: &Inventory,
    seen: &mut BTreeSet<Path>,
) -> PyResult<()> {
    if map.is_empty() {
        return Ok(());
    }
    for item in &page.items {
        if let wordcraft_layout::Placed::Object {
            story: StoryRef::Body,
            path,
            off,
            rect,
            spin,
            text_box,
            ..
        } = item
        {
            let Some(expected) = map.get(path) else {
                continue;
            };
            if *off != 0
                || text_box.is_some()
                || spin.deg != 0.0
                || spin.flip_h
                || spin.flip_v
                || !seen.insert(path.clone())
            {
                return Err(unsupported());
            }
            let right = rect.x + rect.w;
            let bottom = rect.y + rect.h;
            let half = expected.stroke / 2.0;
            let edges = [rect.x - half, rect.y - half, right + half, bottom + half];
            if [
                page.w, page.h, rect.x, rect.y, rect.w, rect.h, right, bottom,
            ]
            .iter()
            .any(|v| !v.is_finite())
                || page.w <= 0.0
                || page.h <= 0.0
                || edges.iter().any(|v| !v.is_finite())
                || !expected.stroke.is_finite()
                || expected.stroke < 0.0
                || rect.w != expected.w
                || rect.h != expected.h
                || rect.x - half < -0.0001
                || rect.y - half < -0.0001
                || right + half > page.w + 0.0001
                || bottom + half > page.h + 0.0001
            {
                return Err(EngineError::Unsupported("shape fill or outline exceeds its physical page, or native layout changed its dimensions".into()).into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "author_docx_shapes_tests.rs"]
mod tests;
