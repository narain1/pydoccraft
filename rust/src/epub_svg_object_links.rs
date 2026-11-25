//! Attribute native painted objects to source elements whose use instances lose IDs.
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use resvg::usvg;
use roxmltree::Node;

use super::{authored_id, native_tree, validate_layout, Links};
use crate::epub::svg::use_refs::{scope_contains, used_sources};
use crate::{
    author_epub::escape_attribute,
    epub::{
        charge, invalid, limited, parse_xml, structure::apply_edits, svg, unsupported, XML_BYTES,
    },
    EngineError,
};

type Span = (usize, usize);
pub(super) type ByNode = HashMap<usize, Vec<Claim>>;

const MAX_NATIVE_VISITS: usize = 250_000;
const MAX_NATIVE_DEPTH: usize = 256;

pub(super) struct Claim {
    pub(super) id: Option<Arc<str>>,
    pub(super) href: Option<Arc<str>>,
}

struct Tag {
    authored: Arc<str>,
    claim: Claim,
}

pub(super) fn recover(
    links: &Links<'_>,
    source: &[u8],
    tree: &usvg::Tree,
    options: &usvg::Options<'_>,
    aliases: &svg::use_fonts::Aliases,
    synthetic: &HashSet<&str>,
) -> Result<ByNode, EngineError> {
    let source = std::str::from_utf8(source).map_err(invalid)?;
    let xml = parse_xml(source.as_bytes())?;
    let scope = used_sources(xml.root_element());
    let mut original = None;
    let mut opacities = None;
    let mut tags = HashMap::new();
    let mut edits = Vec::new();
    for node in xml.descendants().filter(Node::is_element) {
        let Some(id) = node.attribute("id") else {
            continue;
        };
        let authored = authored_id(aliases, id);
        // Text spans use their own native glyph ownership pass.
        if !renderable(node)
            || !scope_contains(&scope, node)
            || node.has_tag_name((svg::NS, "a"))
                && node
                    .ancestors()
                    .skip(1)
                    .any(|outer| outer.has_tag_name((svg::NS, "text")))
        {
            continue;
        }
        let href = links
            .objects
            .get(authored)
            .map(|href| Arc::<str>::from(*href));
        let keep_id = !synthetic.contains(authored) && !authored.is_empty();
        let authored = Arc::<str>::from(authored);
        let claim = Claim {
            id: keep_id.then(|| authored.clone()),
            href,
        };
        if claim.id.is_none() && claim.href.is_none() {
            continue;
        }
        if original.is_none() {
            let index = Index::new(tree)?;
            let occupied = index
                .nodes
                .iter()
                .filter_map(|(node, _)| opacity(node))
                .collect::<HashSet<_>>();
            opacities = Some(
                (0.25f32.to_bits()..0.5f32.to_bits()).filter(move |bits| !occupied.contains(bits)),
            );
            original = Some(index);
        }
        let bits = opacities
            .as_mut()
            .and_then(Iterator::next)
            .ok_or_else(|| limited("EPUB SVG use link probe has too many markers"))?;
        crate::limits::validate_member_count(tags.len() + 1)?;
        tags.insert(bits, Tag { authored, claim });
        let (range, prior) = match node.attribute_node("style") {
            Some(style) => (style.range(), style.value()),
            None => (svg::style_insertion(source.as_bytes(), node)?, ""),
        };
        let value = f32::from_bits(bits);
        let prior = prior.trim_end_matches(';');
        let separator = if prior.is_empty() { "" } else { ";" };
        edits.push((
            range,
            format!(
                r#" style="{}{separator}opacity:{value} !important;""#,
                escape_attribute(prior)
            ),
        ));
    }
    let Some(original) = original else {
        return Ok(HashMap::new());
    };
    let marked = apply_edits(source, edits, XML_BYTES)?;
    let probe = native_tree(marked.as_bytes(), options, true)?;
    let probe = Index::new(&probe)?;
    if original.leaves.len() != probe.leaves.len()
        || !original
            .leaves
            .iter()
            .zip(probe.leaves)
            .all(|(a, b)| same_leaf(a, b))
    {
        return Err(invalid(
            "EPUB SVG use link probe changed native paint geometry",
        ));
    }

    let mut groups = probe
        .nodes
        .into_iter()
        .filter_map(|(node, span)| {
            let bits = opacity(node)?;
            (tags.contains_key(&bits) && span.0 < span.1).then_some((span, bits))
        })
        .collect::<Vec<_>>();
    // Parent ranges precede child ranges that share a starting leaf. A marker
    // inherited explicitly through SVG opacity can reappear inside its group.
    groups.sort_unstable_by_key(|&((start, end), bits)| (start, Reverse(end), bits));
    let mut last = HashMap::<u32, Span>::new();
    let mut by_span = HashMap::<Span, Vec<&Tag>>::new();
    for (span, bits) in groups {
        if last
            .get(&bits)
            .is_some_and(|previous| previous.0 <= span.0 && previous.1 >= span.1)
        {
            continue;
        }
        last.insert(bits, span);
        by_span.entry(span).or_default().push(&tags[&bits]);
    }
    // Equal paint-leaf ranges identify the same original artwork even when the
    // probe introduced a group on a path. Prefer the innermost real node.
    let mut by_node = HashMap::<Span, usize>::new();
    let mut represented = HashSet::<(Span, &str)>::new();
    for (node, span) in original.nodes {
        if by_span.contains_key(&span) {
            by_node
                .entry(span)
                .or_insert(std::ptr::from_ref(node).addr());
            if !node.id().is_empty() {
                represented.insert((span, authored_id(aliases, node.id())));
            }
        }
    }
    let mut output = ByNode::new();
    for (span, owners) in by_span {
        let Some(&node) = by_node.get(&span) else {
            return Err(unsupported(
                "the original SVG use link has no matching native paint group",
            ));
        };
        for tag in owners {
            if !represented.contains(&(span, tag.authored.as_ref())) {
                output.entry(node).or_default().push(Claim {
                    id: tag.claim.id.clone(),
                    href: tag.claim.href.clone(),
                });
            }
        }
    }
    Ok(output)
}

fn renderable(node: Node<'_, '_>) -> bool {
    matches!(
        node.tag_name().name(),
        "a" | "g"
            | "use"
            | "svg"
            | "symbol"
            | "path"
            | "rect"
            | "circle"
            | "ellipse"
            | "line"
            | "polyline"
            | "polygon"
            | "image"
            | "text"
    )
}

struct Index<'a> {
    nodes: Vec<(&'a usvg::Node, Span)>,
    leaves: Vec<&'a usvg::Node>,
    work: usize,
}

impl<'a> Index<'a> {
    fn new(tree: &'a usvg::Tree) -> Result<Self, EngineError> {
        let mut index = Self {
            nodes: Vec::new(),
            leaves: Vec::new(),
            work: 0,
        };
        for node in tree.root().children() {
            index.walk(node, 0)?;
        }
        Ok(index)
    }

    fn walk(&mut self, node: &'a usvg::Node, depth: usize) -> Result<(), EngineError> {
        if depth >= MAX_NATIVE_DEPTH {
            return Err(limited("EPUB SVG use link probe exceeds its drawing depth"));
        }
        charge(
            &mut self.work,
            1,
            MAX_NATIVE_VISITS,
            "EPUB SVG use link probe exceeds its drawing budget",
        )?;
        let start = self.leaves.len();
        if let usvg::Node::Group(group) = node {
            for child in group.children() {
                self.walk(child, depth + 1)?;
            }
        } else {
            self.leaves.push(node);
        }
        self.nodes.push((node, (start, self.leaves.len())));
        Ok(())
    }
}

fn opacity(node: &usvg::Node) -> Option<u32> {
    if let usvg::Node::Group(group) = node {
        Some(group.opacity().get().to_bits())
    } else {
        None
    }
}

fn same_leaf(a: &usvg::Node, b: &usvg::Node) -> bool {
    a.abs_transform() == b.abs_transform()
        && a.abs_stroke_bounding_box() == b.abs_stroke_bounding_box()
        && match (a, b) {
            (usvg::Node::Path(a), usvg::Node::Path(b)) => a.data() == b.data(),
            (usvg::Node::Image(a), usvg::Node::Image(b)) => a.bounding_box() == b.bounding_box(),
            (usvg::Node::Text(a), usvg::Node::Text(b)) => validate_layout(a, b).is_ok(),
            _ => false,
        }
}
