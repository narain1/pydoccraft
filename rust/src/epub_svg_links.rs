//! Retain link and fragment ownership erased by the native SVG parser.
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use resvg::usvg;
use roxmltree::Node;
use wordcraft_geom::Rect;

use super::{
    coverage::{Coverage, Target},
    effects, image_href, native_tree, visit_painted_svg, NS,
};
use crate::{
    author_epub::escape_attribute,
    epub::{charge, invalid, parse_xml, rendition, structure::apply_edits, XML_BYTES},
    EngineError,
};

#[path = "epub_svg_object_links.rs"]
mod object_links;

pub(super) struct Links<'a> {
    pub(super) generated_ids: HashMap<roxmltree::NodeId, String>,
    objects: HashMap<String, &'a str>,
    text_ids: HashSet<String>,
}

impl<'a> Links<'a> {
    pub(super) fn new(root: Node<'a, '_>) -> Self {
        let mut result = Self {
            generated_ids: HashMap::new(),
            objects: HashMap::new(),
            text_ids: HashSet::new(),
        };
        let used = root
            .descendants()
            .filter_map(|node| node.attribute("id"))
            .collect::<HashSet<_>>();
        let mut next = 0;
        for node in root.descendants() {
            let is_text = node.has_tag_name((NS, "text"))
                && node.descendants().skip(1).any(|child| {
                    child.is_element()
                        && (child.attribute("id").is_some()
                            || child.has_tag_name((NS, "a")) && image_href(child).is_some())
                });
            let href = node
                .has_tag_name((NS, "a"))
                .then(|| image_href(node))
                .flatten();
            if !is_text && href.is_none() {
                continue;
            }
            let id = match node.attribute("id").filter(|id| !id.is_empty()) {
                Some(id) => id.to_owned(),
                None if !node.has_attribute("id") => loop {
                    let id = format!("pydoccraft-pdf-link-{next}");
                    next += 1;
                    if !used.contains(id.as_str()) {
                        result.generated_ids.insert(node.id(), id.clone());
                        break id;
                    }
                },
                None => continue,
            };
            if let Some(href) = href {
                result.objects.entry(id.clone()).or_insert(href);
            }
            if is_text {
                result.text_ids.insert(id);
            }
        }
        result
    }

    pub(super) fn prepare(
        &self,
        source: &[u8],
        tree: &usvg::Tree,
        options: &usvg::Options<'_>,
        has_uses: bool,
        aliases: super::use_fonts::Aliases,
    ) -> Result<Navigation, EngineError> {
        let clip = Rect::new(0.0, 0.0, tree.size().width(), tree.size().height());
        let (marked_tree, text_owners) =
            if let Some(marked) = self.mark_text(source, tree, clip, &aliases)? {
                (
                    Some(native_tree(marked.source.as_bytes(), options, has_uses)?),
                    marked.owners,
                )
            } else {
                (None, HashMap::new())
            };
        let synthetic: HashSet<&str> = self.generated_ids.values().map(String::as_str).collect();
        let instantiated = if has_uses {
            object_links::recover(self, source, tree, options, &aliases, &synthetic)?
        } else {
            object_links::ByNode::new()
        };
        Ok(Navigation {
            aliases,
            objects: self
                .objects
                .iter()
                .map(|(id, href)| (id.clone(), Arc::<str>::from(*href)))
                .collect(),
            marked_tree,
            text_owners,
            instantiated,
            synthetic_ids: self.generated_ids.values().cloned().collect(),
        })
    }

    fn mark_text(
        &self,
        source: &[u8],
        tree: &usvg::Tree,
        clip: Rect,
        aliases: &super::use_fonts::Aliases,
    ) -> Result<Option<MarkedText>, EngineError> {
        if self.text_ids.is_empty() {
            return Ok(None);
        }
        let source = std::str::from_utf8(source).map_err(invalid)?;
        let xml = parse_xml(source.as_bytes())?;
        let mut edits = Vec::new();
        // Choose colors absent from the real tree so unmarked runs cannot be
        // mistaken for a target, even when they inherit arbitrary author paint.
        let mut occupied = HashSet::new();
        visit_painted_svg(tree, clip, None, |paint, _, _| {
            if let usvg::Node::Text(text) = paint.node {
                occupied.extend(
                    text.layouted()
                        .iter()
                        .filter_map(|span| color_index(span.fill.as_ref())),
                );
            }
            Ok(())
        })?;
        let mut colors = (1..=0xff_ffffu32).filter(|color| !occupied.contains(color));
        let mut owners = HashMap::new();
        let mut bytes = 0;
        for text in xml
            .descendants()
            .filter(|node| node.has_tag_name((NS, "text")))
        {
            if !text
                .attribute("id")
                .is_some_and(|id| self.text_ids.contains(authored_id(aliases, id)))
            {
                continue;
            }
            for node in text.descendants().filter(|node| {
                node.is_element()
                    && (node.has_tag_name((NS, "tref"))
                        || node.children().any(|child| child.is_text()))
            }) {
                let mut owner = Owner::default();
                for ancestor in node.ancestors().take_while(|ancestor| *ancestor != text) {
                    if let Some(id) = ancestor.attribute("id").filter(|id| !id.is_empty()) {
                        let id = authored_id(aliases, id);
                        charge(
                            &mut bytes,
                            id.len() + 24,
                            XML_BYTES,
                            "EPUB SVG link marks exceed 8 MiB",
                        )?;
                        owner.anchors.push(id.into());
                    }
                    if owner.href.is_none() && ancestor.has_tag_name((NS, "a")) {
                        if let Some(href) = image_href(ancestor) {
                            charge(
                                &mut bytes,
                                href.len() + 24,
                                XML_BYTES,
                                "EPUB SVG link marks exceed 8 MiB",
                            )?;
                            owner.href = Some(href.into());
                        }
                    }
                }
                if owner.is_empty() {
                    continue;
                }
                crate::limits::validate_member_count(owners.len() + 1)?;
                let color = colors
                    .next()
                    .ok_or_else(|| invalid("missing EPUB SVG link marker color"))?;
                owners.insert(color, owner);
                let (range, style) = if let Some(style) = node.attribute_node("style") {
                    (style.range(), style.value())
                } else {
                    (super::style_insertion(source.as_bytes(), node)?, "")
                };
                let style = style.trim_end_matches(';');
                let separator = if style.is_empty() { "" } else { ";" };
                edits.push((
                    range,
                    format!(
                        r#" style="{}{separator}fill:#{color:06x} !important;""#,
                        escape_attribute(style)
                    ),
                ));
            }
        }
        let source = apply_edits(source, edits, XML_BYTES)?;
        Ok(Some(MarkedText { source, owners }))
    }
}

pub(in crate::epub) struct Navigation {
    aliases: super::use_fonts::Aliases,
    objects: HashMap<String, Arc<str>>,
    marked_tree: Option<usvg::Tree>,
    text_owners: HashMap<u32, Owner>,
    // Native node addresses belong to the paired tree's heap-owned groups.
    instantiated: object_links::ByNode,
    synthetic_ids: HashSet<String>,
}

impl Navigation {
    pub(in crate::epub) fn collect(
        &self,
        tree: &usvg::Tree,
        density: [f32; 2],
        raster: [u32; 2],
    ) -> Result<Geometry, EngineError> {
        let capture = effects::CaptureGeometry::new(tree, density);
        let mut geometry = Geometry::default();
        let clip = Rect::new(0.0, 0.0, tree.size().width(), tree.size().height());
        let mut text_targets = HashMap::<_, VecDeque<_>>::new();
        if let Some(marked_tree) = &self.marked_tree {
            visit_painted_svg(marked_tree, clip, None, |paint, _, _| {
                if let usvg::Node::Text(text) = paint.node {
                    // Instance expansion can discard IDs. Identical keys retain paint order.
                    text_targets
                        .entry(text_key(text))
                        .or_default()
                        .push_back(text.as_ref());
                }
                Ok(())
            })?;
        }
        let mut metrics = HashMap::new();
        let mut coverage = Coverage::new(tree, density, raster);
        let mut instance_anchors = Geometry::default();
        visit_painted_svg(tree, clip, Some(&capture), |paint, clip, effects| {
            let node = paint.node;
            if let Some(claims) = self.instantiated.get(&std::ptr::from_ref(node).addr()) {
                if paint.unproven_geometry {
                    for claim in claims {
                        if claim.href.is_some() {
                            return Err(effects::geometry_error());
                        }
                        if let Some(id) = &claim.id {
                            geometry.block_anchor(id)?;
                        }
                    }
                } else if let Some(rect) = paint.object_bounds(true) {
                    if let Some(rect) =
                        coverage.refine(paint, effects, rect, clip, Target::PaintedObject)?
                    {
                        for claim in claims {
                            if let Some(id) = claim
                                .id
                                .as_deref()
                                .filter(|id| !instance_anchors.anchors.contains_key(*id))
                            {
                                instance_anchors.anchor(id, rect, clip)?;
                            }
                            if let Some(href) = &claim.href {
                                geometry.link(rect, clip, href.clone())?;
                            }
                        }
                    }
                }
            }
            let id = authored_id(&self.aliases, node.id());
            if !id.is_empty() && !self.synthetic_ids.contains(id) {
                if paint.unproven_geometry {
                    geometry.block_anchor(id)?;
                } else if let Some(rect) = paint.object_bounds(false) {
                    if let Some(rect) =
                        coverage.refine(paint, effects, rect, clip, Target::Object)?
                    {
                        geometry.anchor(id, rect, clip)?;
                    }
                }
            }
            if let Some(href) = self.objects.get(id) {
                if paint.unproven_geometry {
                    return Err(effects::geometry_error());
                }
                if let Some(rect) = paint.object_bounds(true) {
                    if let Some(rect) =
                        coverage.refine(paint, effects, rect, clip, Target::Object)?
                    {
                        geometry.link(rect, clip, href.clone())?;
                    }
                }
            }
            if let usvg::Node::Text(text) = node {
                if let Some(probe) = text_targets
                    .get_mut(&text_key(text))
                    .and_then(VecDeque::pop_front)
                {
                    let assignment = probe
                        .layouted()
                        .iter()
                        .map(|span| {
                            color_index(span.fill.as_ref())
                                .and_then(|index| self.text_owners.get(&index))
                        })
                        .collect::<Vec<_>>();
                    if assignment.iter().all(Option::is_none) {
                        return Ok(());
                    }
                    validate_layout(text, probe)?;
                    super::visit_glyphs(
                        tree.fontdb(),
                        text,
                        &mut metrics,
                        |index| assignment[index].is_some_and(|owner| !owner.is_empty()),
                        |index, glyph, rect, ink| {
                            let Some(owner) = assignment[index] else {
                                return Ok(());
                            };
                            if glyph.is_empty() {
                                return Ok(());
                            }
                            if paint.unproven_geometry {
                                if owner.href.is_some() {
                                    return Err(effects::geometry_error());
                                }
                                for id in &owner.anchors {
                                    if !self.synthetic_ids.contains(id.as_str()) {
                                        geometry.block_anchor(id)?;
                                    }
                                }
                                return Ok(());
                            }
                            let Some(rect) =
                                coverage.refine(paint, effects, rect, clip, Target::Glyph(ink))?
                            else {
                                return Ok(());
                            };
                            if let Some(href) = &owner.href {
                                geometry.link(rect, clip, href.clone())?;
                            }
                            for id in &owner.anchors {
                                if !self.synthetic_ids.contains(id.as_str()) {
                                    geometry.anchor(id, rect, clip)?;
                                }
                            }
                            Ok(())
                        },
                    )?;
                }
            }
            Ok(())
        })?;
        for (id, rect) in instance_anchors.anchors {
            if !geometry.anchors.contains_key(&id) {
                geometry.anchor(&id, rect, clip)?;
            }
        }
        Ok(geometry)
    }
}

fn authored_id<'a>(aliases: &'a super::use_fonts::Aliases, id: &'a str) -> &'a str {
    aliases.get(id).map_or(id, String::as_str)
}

pub(super) fn actual_rect(rect: usvg::Rect) -> Rect {
    Rect::new(rect.x(), rect.y(), rect.width(), rect.height())
}

fn text_key(text: &usvg::Text) -> (String, [u32; 6]) {
    let t = text.abs_transform();
    (
        text.id().to_owned(),
        [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty].map(f32::to_bits),
    )
}

fn validate_layout(real: &usvg::Text, marked: &usvg::Text) -> Result<(), EngineError> {
    let unchanged = real.layouted().len() == marked.layouted().len()
        && real
            .layouted()
            .iter()
            .zip(marked.layouted())
            .all(|(real, marked)| {
                real.positioned_glyphs.len() == marked.positioned_glyphs.len()
                    && real
                        .positioned_glyphs
                        .iter()
                        .zip(&marked.positioned_glyphs)
                        .all(|(real, marked)| {
                            real.text == marked.text
                                && real.id == marked.id
                                && real.outline_transform() == marked.outline_transform()
                        })
            });
    if unchanged {
        Ok(())
    } else {
        Err(invalid(
            "EPUB SVG link glyph layout changed in the annotation pass",
        ))
    }
}

fn color_index(fill: Option<&usvg::Fill>) -> Option<u32> {
    match fill?.paint() {
        usvg::Paint::Color(color) => {
            Some(u32::from_be_bytes([0, color.red, color.green, color.blue]))
        }
        _ => None,
    }
}

struct MarkedText {
    source: String,
    owners: HashMap<u32, Owner>,
}

#[derive(Default)]
struct Owner {
    href: Option<Arc<str>>,
    anchors: Vec<String>,
}

impl Owner {
    fn is_empty(&self) -> bool {
        self.href.is_none() && self.anchors.is_empty()
    }
}

#[derive(Default)]
pub(in crate::epub) struct Geometry {
    anchors: HashMap<String, Rect>,
    links: rendition::LinkAreas,
    anchor_bytes: usize,
    blocked_anchors: HashSet<String>,
}

impl Geometry {
    pub(in crate::epub) fn byte_cost(&self) -> usize {
        self.anchor_bytes + self.links.bytes
    }

    fn anchor(&mut self, id: &str, rect: Rect, clip: Rect) -> Result<(), EngineError> {
        if rendition::anchor_position(rect, clip).is_none() {
            return Ok(());
        }
        let rect = rendition::intersection(rect, clip);
        if let Some(previous) = self.anchors.get_mut(id) {
            *previous = previous.union(&rect);
            return Ok(());
        }
        charge(
            &mut self.anchor_bytes,
            id.len() + 48,
            XML_BYTES,
            "EPUB SVG anchor geometry exceeds 8 MiB",
        )?;
        self.anchors.insert(id.into(), rect);
        Ok(())
    }

    fn block_anchor(&mut self, id: &str) -> Result<(), EngineError> {
        if !self.blocked_anchors.contains(id) {
            charge(
                &mut self.anchor_bytes,
                id.len() + 48,
                XML_BYTES,
                "EPUB SVG anchor geometry exceeds 8 MiB",
            )?;
            self.blocked_anchors.insert(id.into());
        }
        Ok(())
    }

    fn link(&mut self, rect: Rect, clip: Rect, href: Arc<str>) -> Result<(), EngineError> {
        self.links.add(rect, clip, href)
    }

    pub(in crate::epub) fn place(
        &self,
        map: kurbo::Affine,
        clip: Rect,
        capture: &mut rendition::Capture<'_, '_>,
    ) -> Result<(), EngineError> {
        let transformed = |rect| rendition::text::transformed(rect, map);
        if capture
            .anchors
            .as_ref()
            .is_some_and(|targets| targets.keys().any(|id| self.blocked_anchors.contains(*id)))
        {
            return Err(effects::geometry_error());
        }
        for (id, rect) in &self.anchors {
            rendition::capture_anchor(&mut capture.anchors, id, transformed(*rect), clip);
        }
        for (_, rect, href) in &self.links.entries {
            capture.links.add(transformed(*rect), clip, href.clone())?;
        }
        Ok(())
    }
}
