//! Map XHTML inline targets to the UTF-8 offsets used by both native text layouts.
use std::{ops::Range, sync::Arc};

use wordcraft_geom::Rect;
use wordcraft_layout::{para::ClKind, Page, Placed};

use crate::{model::BlockPayload, EngineError};

use super::super::{
    output::{inline_items, Inline},
    rendition::{capture_anchor, intersection, Capture},
};

#[derive(Default)]
pub(in crate::epub) struct InlineNavigation {
    links: Vec<(Range<usize>, Arc<str>)>,
    anchors: Vec<(usize, String)>,
    upstream: Vec<(usize, String)>,
    length: usize,
}

impl InlineNavigation {
    pub(in crate::epub) fn new(
        block: &BlockPayload,
        offsets: &[usize],
    ) -> Result<Self, EngineError> {
        let mut source = block
            .links
            .iter()
            .filter_map(|link| {
                let start = usize::try_from(link.metadata.get("start")?.as_u64()?).ok()?;
                let end = usize::try_from(link.metadata.get("end")?.as_u64()?).ok()?;
                (start < end).then(|| (start..end, Arc::<str>::from(link.uri.as_str())))
            })
            .collect::<Vec<_>>();
        source.sort_by_key(|(range, _)| range.start);
        let mut anchors = block
            .metadata
            .get("layout_anchors")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                Some((
                    usize::try_from(entry[0].as_u64()?).ok()?,
                    entry[1].as_str()?,
                    entry
                        .get(2)
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                ))
            })
            .peekable();
        let mut result = Self::default();
        if source.is_empty()
            && anchors.peek().is_none()
            && !block
                .images
                .iter()
                .any(|image| image.metadata.contains_key("href"))
        {
            return Ok(result);
        }
        let mut cursor = 0;
        let mut position = 0;
        let mut copy_text = |result: &mut Self, range: Range<usize>, at: usize| {
            let start = offsets[range.start];
            let limit = source.partition_point(|(link, _)| link.end <= range.start);
            for (link, href) in &source[limit..] {
                if link.start >= range.end {
                    break;
                }
                let left = at + offsets[link.start.max(range.start)] - start;
                let right = at + offsets[link.end.min(range.end)] - start;
                if left < right {
                    result.links.push((left..right, href.clone()));
                }
            }
            while let Some(&(anchor, id, upstream)) = anchors.peek() {
                if anchor >= range.end {
                    break;
                }
                let anchor = at + offsets[anchor.max(range.start)] - start;
                result.push_anchor(anchor, id, upstream);
                anchors.next();
            }
            at + offsets[range.end] - start
        };
        for (start, object) in inline_items(block, false)? {
            if start < cursor || start >= offsets.len() {
                return Err(super::super::invalid("invalid EPUB inline object position"));
            }
            position = copy_text(&mut result, cursor..start, position);
            let (end, href) = match object {
                Inline::Image(image) => (
                    start,
                    image
                        .metadata
                        .get("href")
                        .and_then(serde_json::Value::as_str)
                        .map(Arc::from),
                ),
                Inline::Equation { end, .. } if end >= start && end < offsets.len() => {
                    let first = source.partition_point(|(link, _)| link.end <= start);
                    (
                        end,
                        source
                            .get(first)
                            .filter(|(range, _)| range.start <= start && range.end >= end)
                            .map(|(_, href)| href.clone()),
                    )
                }
                _ => return Err(super::super::invalid("invalid EPUB inline object range")),
            };
            let end_position = position + wordcraft_doc::para::OBJ.len_utf8();
            if let Some(href) = href {
                result.links.push((position..end_position, href));
            }
            cursor = end;
            position = end_position;
        }
        position = copy_text(&mut result, cursor..offsets.len() - 1, position);
        for (_, id, upstream) in anchors {
            result.push_anchor(position, id, upstream);
        }
        result.length = position;
        Ok(result)
    }

    fn push_anchor(&mut self, offset: usize, id: &str, upstream: bool) {
        let anchors = if upstream {
            &mut self.upstream
        } else {
            &mut self.anchors
        };
        anchors.push((offset, id.to_owned()));
    }

    pub(in crate::epub) fn is_empty(&self) -> bool {
        self.links.is_empty() && self.anchors.is_empty() && self.upstream.is_empty()
    }

    pub(in crate::epub) fn capture_upstream(
        &self,
        end: usize,
        rect: Rect,
        rtl: bool,
        clip: Rect,
        navigation: &mut Capture<'_, '_>,
    ) {
        let first = self.upstream.partition_point(|(offset, _)| *offset < end);
        let x = if rtl { rect.x } else { rect.x + rect.w };
        for (offset, id) in &self.upstream[first..] {
            if *offset != end {
                break;
            }
            capture_anchor(
                &mut navigation.anchors,
                id,
                Rect::new(x, rect.y, 0.0, rect.h),
                clip,
            );
        }
    }

    pub(in crate::epub) fn capture(
        &self,
        range: Range<usize>,
        rect: Rect,
        clip: Rect,
        navigation: &mut Capture<'_, '_>,
    ) -> Result<(), EngineError> {
        let first = self
            .links
            .partition_point(|(link, _)| link.end <= range.start);
        if let Some((link, href)) = self.links.get(first) {
            if link.start < range.end {
                navigation.links.add(rect, clip, href.clone())?;
            }
        }
        let first = self
            .anchors
            .partition_point(|(offset, _)| *offset < range.start);
        for (offset, id) in &self.anchors[first..] {
            if *offset >= range.end {
                break;
            }
            capture_anchor(&mut navigation.anchors, id, rect, clip);
        }
        Ok(())
    }

    pub(in crate::epub) fn capture_end(
        &self,
        rect: Rect,
        clip: Rect,
        navigation: &mut Capture<'_, '_>,
    ) {
        let first = self
            .anchors
            .partition_point(|(offset, _)| *offset < self.length);
        for (offset, id) in &self.anchors[first..] {
            if *offset != self.length {
                break;
            }
            capture_anchor(&mut navigation.anchors, id, rect, clip);
        }
    }
}

pub(super) fn capture_native(
    doc: &wordcraft_doc::Document,
    page: &Page,
    paragraphs: &std::collections::HashMap<u64, InlineNavigation>,
    area: Rect,
    clip: Rect,
    navigation: &mut Capture<'_, '_>,
) -> Result<(), EngineError> {
    if paragraphs.is_empty() {
        return Ok(());
    }
    let cells = page
        .items
        .iter()
        .filter_map(|item| match item {
            Placed::Cell {
                rect,
                table,
                row,
                cell,
                story,
                clip: true,
                ..
            } => Some(((*story, &table.0[..], *row, *cell), *rect)),
            _ => None,
        })
        .collect::<std::collections::HashMap<_, _>>();
    for item in &page.items {
        let Placed::Lines {
            story,
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
        if turn.is_turned() {
            continue;
        }
        let Some(source) = doc.para(*story, path).and_then(|p| paragraphs.get(&p.rev)) else {
            continue;
        };
        let Some(first) = para.lines.get(*l0) else {
            continue;
        };
        let (x, y) = (area.x + x, area.y + y - first.top);
        let mut line_clip = clip;
        for end in (3..path.0.len()).step_by(3) {
            let (row, cell) = (path.0[end - 2] as usize, path.0[end - 1] as usize);
            if let Some(r) = cells.get(&(*story, &path.0[..end - 2], row, cell)) {
                line_clip =
                    intersection(line_clip, Rect::new(area.x + r.x, area.y + r.y, r.w, r.h));
            }
        }
        for line in &para.lines[*l0..*l1] {
            for index in line.c0..line.c1 {
                let cluster = &para.clusters[index];
                let (Some(left), Some(right)) = (line.cl_left(index), line.cl_right(index)) else {
                    continue;
                };
                if para.styles[cluster.style as usize].rc.hidden {
                    continue;
                }
                let (asc, desc) = match cluster.kind {
                    ClKind::Object(_) => (cluster.obj_h, cluster.obj_d),
                    ClKind::Text | ClKind::Space | ClKind::Tab => (line.text_asc, line.text_desc),
                    _ => continue,
                };
                source.capture(
                    cluster.start..cluster.end,
                    Rect::new(x + left, y + line.baseline - asc, right - left, asc + desc),
                    line_clip,
                    navigation,
                )?;
            }
        }
        if *l1 == para.lines.len() {
            if let Some(line) = para.lines.last() {
                source.capture_end(
                    Rect::new(x + line.end_x(), y + line.top, 0.0, line.height),
                    line_clip,
                    navigation,
                );
            }
        }
    }
    Ok(())
}
