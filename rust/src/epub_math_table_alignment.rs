//! Measure MathML table alignment points without changing editable runs.
use roxmltree::{Node, NodeId};
use wordcraft_doc::math::MNode;
use wordcraft_layout::math::MathLayout;

use super::super::{append_token_text, xml_space, DEPTH};
use super::{indexed, invalid, limited, unsupported, MATHML};
use crate::EngineError;

#[derive(Clone, Copy)]
pub(super) enum Anchor {
    Left,
    Center,
    Right,
    Decimal(Option<usize>),
    Marked(usize, Edge),
}

#[derive(Clone, Copy)]
pub(super) enum Edge {
    Left,
    Right,
}

impl Anchor {
    pub(super) fn measure(self, layout: &MathLayout) -> Result<f32, EngineError> {
        match self {
            Self::Left => Ok(0.0),
            Self::Center => Ok(layout.width / 2.0),
            Self::Right | Self::Decimal(None) => Ok(layout.width),
            Self::Decimal(Some(offset)) => Edge::Right
                .caret(layout, offset)
                .ok_or_else(|| unsupported("MathML decimal alignment caret position")),
            Self::Marked(offset, edge) => edge
                .caret(layout, offset)
                .ok_or_else(|| unsupported("MathML alignment mark caret position")),
        }
    }
}

impl Edge {
    fn caret(self, layout: &MathLayout, offset: usize) -> Option<f32> {
        let slots = layout
            .slots
            .iter()
            .filter(|slot| slot.path.is_empty() && slot.off == offset)
            .map(|slot| slot.x);
        // Operator spacing gives a shared logical offset two visual stops.
        let x = match self {
            Self::Left => slots.max_by(f32::total_cmp),
            Self::Right => slots.min_by(f32::total_cmp),
        };
        x.or_else(|| (offset == 0 && layout.width == 0.0).then_some(0.0))
            .filter(|x| (0.0..=layout.width).contains(x))
    }
}

#[derive(Clone, Copy)]
pub(super) struct Group {
    pub(super) scope: Scope,
    anchor: RequestedAnchor,
}

#[derive(Clone, Copy)]
enum RequestedAnchor {
    Automatic(Anchor),
    Explicit(Edge),
}

impl Group {
    pub(super) fn resolve(
        self,
        caret: Option<usize>,
        nodes: &mut [MNode],
    ) -> Result<Anchor, EngineError> {
        let anchor = match self.anchor {
            RequestedAnchor::Explicit(edge) => {
                let offset =
                    caret.ok_or_else(|| invalid("MathML alignment mark was not projected"))?;
                Anchor::Marked(offset, edge)
            }
            RequestedAnchor::Automatic(anchor) => anchor,
        };
        if matches!(anchor, Anchor::Decimal(_)) {
            let [MNode::Run(run)] = nodes else {
                return Err(invalid("MathML decimal alignment token projection differs"));
            };
            // A decimal separator belongs to the single editable numeric token.
            run.lit = true;
        }
        Ok(anchor)
    }
}

#[derive(Clone, Copy)]
pub(in crate::epub::math) struct Scope {
    cell: NodeId,
    opening: NodeId,
    pub(in crate::epub::math) mark: Option<NodeId>,
    pub(in crate::epub::math) caret: Option<usize>,
}

impl Scope {
    pub(in crate::epub::math) fn removes(self, node: Node<'_, '_>) -> bool {
        node.id() == self.opening
            || tag(node) == Some("malignmark")
                && node
                    .ancestors()
                    .skip(1)
                    .find(|ancestor| tag(*ancestor) == Some("mtd"))
                    .is_some_and(|cell| cell.id() == self.cell)
    }
}

fn tag<'input>(node: Node<'_, 'input>) -> Option<&'input str> {
    (node.tag_name().namespace() == Some(MATHML)).then(|| node.tag_name().name())
}

pub(super) fn leading_group<'xml, 'input>(
    mut node: Node<'xml, 'input>,
) -> Option<Node<'xml, 'input>> {
    loop {
        node = node.children().find(Node::is_element)?;
        match tag(node)? {
            "maligngroup" => return Some(node),
            "mrow" | "mstyle" | "semantics" => {}
            _ => return None,
        }
    }
}

pub(super) fn table_has_groups(table: Node<'_, '_>) -> bool {
    table
        .children()
        .filter(Node::is_element)
        .flat_map(|row| row.children().filter(Node::is_element))
        .any(|cell| leading_group(cell).is_some())
}

pub(super) fn read(
    table: Node<'_, '_>,
    row: Node<'_, '_>,
    cell: Node<'_, '_>,
    marker: Node<'_, '_>,
    column: usize,
) -> Result<Group, EngineError> {
    if marker
        .children()
        .any(|child| child.is_element() || child.text().is_some_and(|text| !text.trim().is_empty()))
    {
        return Err(invalid("MathML alignment group must be empty"));
    }
    if marker
        .ancestors()
        .find_map(|ancestor| ancestor.attribute("dir"))
        == Some("rtl")
    {
        return Err(unsupported("right-to-left MathML alignment groups"));
    }
    match indexed(table.attribute("alignmentscope"), column).unwrap_or("true") {
        "true" => {}
        "false" => return Err(unsupported("MathML alignment groups across table scopes")),
        _ => return Err(invalid("invalid MathML alignment scope")),
    }
    let mut scope = Scope {
        cell: cell.id(),
        opening: marker.id(),
        mark: None,
        caret: None,
    };
    if let Some(mark) = first_mark(cell, 0)? {
        scope.mark = Some(mark.id());
        return Ok(Group {
            scope,
            anchor: RequestedAnchor::Explicit(mark_edge(cell, mark)?),
        });
    }
    let requested = if let Some(value) = marker.attribute("groupalign") {
        value.trim()
    } else if let Some(value) = cell.attribute("groupalign") {
        value
            .split_ascii_whitespace()
            .next()
            .ok_or_else(|| invalid("empty MathML group alignment list"))?
    } else if let Some(value) = row.attribute("groupalign") {
        column_alignment(value, column)?
    } else if let Some(value) = table.attribute("groupalign") {
        column_alignment(value, column)?
    } else {
        "left"
    };
    let automatic = match requested {
        "left" => Anchor::Left,
        "center" => Anchor::Center,
        "right" => Anchor::Right,
        "decimalpoint" => decimal(cell, marker)?,
        _ => return Err(unsupported("MathML group alignment")),
    };
    Ok(Group {
        scope,
        anchor: RequestedAnchor::Automatic(automatic),
    })
}

fn first_mark<'xml, 'input>(
    node: Node<'xml, 'input>,
    depth: usize,
) -> Result<Option<Node<'xml, 'input>>, EngineError> {
    if depth > DEPTH {
        return Err(limited("MathML structure exceeds its layout budget"));
    }
    match tag(node) {
        Some("malignmark") => return Ok(Some(node)),
        Some("mtable" | "annotation" | "annotation-xml") => return Ok(None),
        _ => {}
    }
    for child in node.children().filter(Node::is_element) {
        let result = first_mark(child, depth + 1)?;
        if result.is_some() || tag(node) == Some("semantics") {
            return Ok(result);
        }
    }
    Ok(None)
}

pub(in crate::epub::math) fn check_mark_empty(mark: Node<'_, '_>) -> Result<(), EngineError> {
    if mark
        .children()
        .any(|child| child.is_element() || child.text().is_some_and(|text| !text.trim().is_empty()))
    {
        return Err(invalid("MathML alignment mark must be empty"));
    }
    Ok(())
}

fn mark_edge(cell: Node<'_, '_>, mark: Node<'_, '_>) -> Result<Edge, EngineError> {
    check_mark_empty(mark)?;
    let edge = match mark.attribute("edge").unwrap_or("left").trim() {
        "left" => Edge::Left,
        "right" => Edge::Right,
        _ => return Err(invalid("invalid MathML alignment mark edge")),
    };
    let mut ancestors = mark.ancestors().skip(1);
    let parent = ancestors
        .next()
        .ok_or_else(|| invalid("MathML alignment mark has no parent"))?;
    let token = matches!(tag(parent), Some("mi" | "mn" | "mo" | "ms" | "mtext"));
    if !token && parent != cell && !matches!(tag(parent), Some("mrow" | "mstyle")) {
        return Err(unsupported(
            "MathML alignment mark outside the cell baseline",
        ));
    }
    if parent != cell
        && ancestors
            .take_while(|ancestor| *ancestor != cell)
            .any(|ancestor| !matches!(tag(ancestor), Some("mrow" | "mstyle" | "semantics")))
        || mark
            .ancestors()
            .take_while(|ancestor| *ancestor != cell)
            .any(|ancestor| {
                ancestor
                    .attribute("dir")
                    .is_some_and(|direction| direction.trim() == "rtl")
            })
    {
        return Err(unsupported(
            "MathML alignment mark outside the left-to-right cell baseline",
        ));
    }
    let visible = |node: Node<'_, '_>| {
        if token {
            node.is_text()
                && node
                    .text()
                    .is_some_and(|text| text.chars().any(|ch| !xml_space(ch)))
        } else {
            node.is_element() && !matches!(tag(node), Some("maligngroup" | "malignmark"))
        }
    };
    // At an empty side of an explicit group, choose the stop inside that
    // group rather than the opposite side of its surrounding operator gap.
    Ok(match edge {
        Edge::Left if !mark.next_siblings().skip(1).any(visible) => Edge::Right,
        Edge::Right if !mark.prev_siblings().skip(1).any(visible) => Edge::Left,
        _ => edge,
    })
}

fn column_alignment(mut values: &str, column: usize) -> Result<&str, EngineError> {
    let mut selected = None;
    let mut index = 0;
    while !values.trim().is_empty() {
        let (group, rest) = values
            .trim_start()
            .strip_prefix('{')
            .and_then(|values| values.split_once('}'))
            .ok_or_else(|| invalid("invalid MathML group alignment list"))?;
        let value = group
            .split_ascii_whitespace()
            .next()
            .ok_or_else(|| invalid("empty MathML group alignment list"))?;
        if index <= column {
            selected = Some(value);
        }
        index += 1;
        values = rest;
    }
    selected.ok_or_else(|| invalid("empty MathML group alignment list"))
}

fn decimal(cell: Node<'_, '_>, marker: Node<'_, '_>) -> Result<Anchor, EngineError> {
    let number = sole_number(cell, marker);
    let Some(number) = number else {
        // Numeric content outside the supported single-token layout may
        // have its alignment point within an operator or a styled group.
        if cell.descendants().any(|node| tag(node) == Some("mn")) {
            return Err(unsupported(
                "MathML decimal alignment requires a single numeric token",
            ));
        }
        return Ok(Anchor::Right);
    };
    let point = number
        .ancestors()
        .filter(|node| matches!(tag(*node), Some("mstyle" | "math")))
        .find_map(|node| node.attribute("decimalpoint"))
        .unwrap_or(".");
    let mut chars = point.chars();
    let point = chars
        .next()
        .filter(|_| chars.next().is_none())
        .ok_or_else(|| invalid("invalid MathML decimal point"))?;
    let mut text = String::new();
    append_token_text(number, &mut text);
    Ok(Anchor::Decimal(
        text.chars().position(|character| character == point),
    ))
}

fn sole_number<'xml, 'input>(
    mut node: Node<'xml, 'input>,
    marker: Node<'xml, 'input>,
) -> Option<Node<'xml, 'input>> {
    loop {
        let mut children = node
            .children()
            .filter(|child| child.is_element() && *child != marker);
        let child = children.next()?;
        if tag(node) != Some("semantics") && children.next().is_some() {
            return None;
        }
        match tag(child)? {
            "mn" => return Some(child),
            "mrow" | "mstyle" | "semantics" => node = child,
            _ => return None,
        }
    }
}
