//! Measure MathML table alignment points without changing editable runs.
use roxmltree::{Node, NodeId};
use wordcraft_layout::math::MathLayout;

use super::super::{append_token_text, xml_space, DEPTH};
use super::{indexed, invalid, limited, unsupported, MATHML};
use crate::EngineError;

#[derive(Clone, Copy)]
pub(super) enum Anchor {
    Left,
    Center,
    Right,
    Decimal(usize, Edge),
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
            Self::Right => Ok(layout.width),
            Self::Decimal(offset, edge) => edge
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
    Decimal(Edge),
    Explicit(Edge),
}

impl Group {
    pub(super) fn resolve(self, caret: Option<usize>) -> Result<Anchor, EngineError> {
        match self.anchor {
            RequestedAnchor::Automatic(anchor) => Ok(anchor),
            RequestedAnchor::Decimal(edge) => caret
                .map(|offset| Anchor::Decimal(offset, edge))
                .ok_or_else(|| invalid("MathML decimal alignment number was not projected")),
            RequestedAnchor::Explicit(edge) => caret
                .map(|offset| Anchor::Marked(offset, edge))
                .ok_or_else(|| invalid("MathML alignment mark was not projected")),
        }
    }
}

#[derive(Clone, Copy)]
enum Cursor {
    Mark(NodeId),
    Number(NodeId, usize),
}

#[derive(Clone, Copy)]
pub(in crate::epub::math) struct Scope {
    cell: NodeId,
    opening: NodeId,
    cursor: Option<Cursor>,
    pub(in crate::epub::math) caret: Option<usize>,
}

impl Scope {
    pub(in crate::epub::math) fn seeking_caret(self) -> bool {
        self.cursor.is_some() && self.caret.is_none()
    }

    pub(in crate::epub::math) fn has_numeric_cursor(self) -> bool {
        matches!(self.cursor, Some(Cursor::Number(..)))
    }

    pub(in crate::epub::math) fn is_selected_mark(self, node: Node<'_, '_>) -> bool {
        matches!(self.cursor, Some(Cursor::Mark(mark)) if mark == node.id())
    }

    pub(in crate::epub::math) fn numeric_offset(self, node: Node<'_, '_>) -> Option<usize> {
        match self.cursor {
            Some(Cursor::Number(number, offset)) if number == node.id() => Some(offset),
            _ => None,
        }
    }

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
        cursor: None,
        caret: None,
    };
    if let Some(mark) = first_mark(cell, 0)? {
        scope.cursor = Some(Cursor::Mark(mark.id()));
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
    let anchor = match requested {
        "left" => RequestedAnchor::Automatic(Anchor::Left),
        "center" => RequestedAnchor::Automatic(Anchor::Center),
        "right" => RequestedAnchor::Automatic(Anchor::Right),
        "decimalpoint" => match first_baseline_number(cell, 0)? {
            Some(number) => {
                let offset = decimal_offset(cell, number)?;
                scope.cursor = Some(Cursor::Number(number.id(), offset));
                // A leading separator uses the start of its own run after
                // any native spacing around the preceding operator.
                let edge = if offset == 0 { Edge::Left } else { Edge::Right };
                RequestedAnchor::Decimal(edge)
            }
            None => RequestedAnchor::Automatic(Anchor::Right),
        },
        _ => return Err(unsupported("MathML group alignment")),
    };
    Ok(Group { scope, anchor })
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

fn decimal_offset(cell: Node<'_, '_>, number: Node<'_, '_>) -> Result<usize, EngineError> {
    if number
        .ancestors()
        .skip(1)
        .take_while(|node| *node != cell)
        .any(|node| !matches!(tag(node), Some("mrow" | "mstyle" | "semantics")))
    {
        return Err(unsupported(
            "MathML decimal alignment inside a nontransparent operator",
        ));
    }
    if number
        .ancestors()
        .take_while(|node| *node != cell)
        .any(|node| {
            node.attribute("dir")
                .is_some_and(|direction| direction.trim() == "rtl")
        })
    {
        return Err(unsupported("right-to-left MathML decimal alignment"));
    }
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
    Ok(text
        .chars()
        .take_while(|&character| character != point)
        .count())
}

fn first_baseline_number<'xml, 'input>(
    node: Node<'xml, 'input>,
    depth: usize,
) -> Result<Option<Node<'xml, 'input>>, EngineError> {
    if depth > DEPTH {
        return Err(limited("MathML structure exceeds its layout budget"));
    }
    let count = match tag(node) {
        Some("mn") => return Ok(Some(node)),
        Some(
            "mtd" | "mrow" | "mstyle" | "mpadded" | "mphantom" | "menclose" | "mfenced" | "msqrt",
        ) => usize::MAX,
        Some(
            "msub" | "msup" | "msubsup" | "munder" | "mover" | "munderover" | "mmultiscripts"
            | "mroot" | "semantics",
        ) => 1,
        _ => return Ok(None),
    };
    for child in node.children().filter(Node::is_element).take(count) {
        if let Some(number) = first_baseline_number(child, depth + 1)? {
            return Ok(Some(number));
        }
    }
    Ok(None)
}
