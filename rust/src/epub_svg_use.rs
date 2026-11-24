//! Bound the SVG shadow trees before usvg clones local references.
use std::collections::{HashMap, HashSet};

use roxmltree::{Node, NodeId};

use super::{fragment_id, unqualified_attribute, NS, XLINK};
use crate::{
    epub::{invalid, limited, unsupported},
    EngineError,
};

pub(super) const MAX_DEPTH: usize = 96;
const MAX_EXPANDED_NODES: usize = 150_000;
const MAX_EXPANDED_BYTES: usize = 16 * 1024 * 1024;

pub(in crate::epub) fn href_id(value: &str) -> Result<&str, EngineError> {
    fragment_id(value).ok_or_else(|| unsupported("SVG use requires a same-document fragment"))
}

pub(in crate::epub) fn validate_use_graph(root: Node<'_, '_>) -> Result<bool, EngineError> {
    if !root
        .descendants()
        .any(|node| node.has_tag_name((NS, "use")))
    {
        return Ok(false);
    }
    let mut ids = HashMap::new();
    for node in root.descendants().filter(Node::is_element) {
        if let Some(id) = unqualified_attribute(node, "id") {
            ids.entry(id)
                .and_modify(|target| *target = None)
                .or_insert(Some(node));
        }
    }
    let mut targets = HashMap::new();
    for node in root
        .descendants()
        .filter(|node| node.has_tag_name((NS, "use")))
    {
        if node
            .children()
            .any(|child| child.is_element() && !matches!(child.tag_name().name(), "title" | "desc"))
        {
            return Err(invalid("SVG use has unexpected child elements"));
        }
        let value = unqualified_attribute(node, "href")
            .or_else(|| node.attribute((XLINK, "href")))
            .ok_or_else(|| invalid("SVG use has no href"))?;
        let id = href_id(value)?;
        let target = match ids.get(id) {
            Some(Some(target)) => *target,
            Some(None) => return Err(invalid("SVG use references a duplicate ID")),
            None => return Err(invalid("SVG use references a missing ID")),
        };
        if !matches!(
            target.tag_name().name(),
            "svg"
                | "symbol"
                | "g"
                | "use"
                | "path"
                | "rect"
                | "circle"
                | "ellipse"
                | "line"
                | "polyline"
                | "polygon"
                | "text"
                | "image"
        ) {
            return Err(unsupported("SVG use target must be graphics or a symbol"));
        }
        targets.insert(node.id(), target);
    }
    Expansion {
        targets,
        active: HashSet::new(),
        measured: HashMap::new(),
    }
    .measure(root, 0)?;
    Ok(true)
}

#[derive(Clone, Copy)]
struct Cost {
    source_nodes: usize,
    nodes: usize,
    bytes: usize,
    depth: usize,
}

impl Cost {
    fn include(&mut self, other: Self, edge_depth: usize) -> Result<(), EngineError> {
        self.nodes += other.nodes;
        self.bytes += other.bytes;
        self.depth = self.depth.max(other.depth + edge_depth);
        self.check_limits()
    }

    fn check_limits(&self) -> Result<(), EngineError> {
        if self.nodes > MAX_EXPANDED_NODES || self.bytes > MAX_EXPANDED_BYTES {
            return Err(limited("SVG use expansion exceeds its size budget"));
        }
        Ok(())
    }
}

struct Expansion<'a, 'input> {
    targets: HashMap<NodeId, Node<'a, 'input>>,
    active: HashSet<NodeId>,
    measured: HashMap<NodeId, Cost>,
}

impl<'a, 'input> Expansion<'a, 'input> {
    fn measure(&mut self, node: Node<'a, 'input>, depth: usize) -> Result<Cost, EngineError> {
        if depth > MAX_DEPTH {
            return Err(limited("SVG use expansion exceeds its depth budget"));
        }
        let id = node.id();
        if let Some(&cost) = self.measured.get(&id) {
            if depth + cost.depth > MAX_DEPTH {
                return Err(limited("SVG use expansion exceeds its depth budget"));
            }
            return Ok(cost);
        }
        if !self.active.insert(id) {
            return Err(invalid("cyclic SVG use reference"));
        }
        let mut cost = Cost {
            source_nodes: 1,
            nodes: 1,
            bytes: node.text().filter(|_| node.is_text()).map_or(0, str::len)
                + node
                    .attributes()
                    .map(|attr| attr.name().len() + attr.value().len())
                    .sum::<usize>(),
            depth: 0,
        };
        cost.check_limits()?;
        for child in node.children() {
            let child = self.measure(child, depth + 1)?;
            cost.source_nodes += child.source_nodes;
            cost.include(child, 1)?;
        }
        if let Some(&target) = self.targets.get(&id) {
            // usvg visits the referenced source subtree to check nested uses
            // before parsing the clone, and the reference adds two levels.
            let target = self.measure(target, depth + 2)?;
            cost.nodes += target.source_nodes;
            cost.include(target, 2)?;
        }
        self.active.remove(&id);
        self.measured.insert(id, cost);
        Ok(cost)
    }
}
