//! Measure MathML table alignment points without changing editable runs.
use roxmltree::Node;
use wordcraft_layout::math::MathLayout;

use super::super::append_token_text;
use super::{indexed, invalid, unsupported, MATHML};
use crate::EngineError;

#[derive(Clone, Copy)]
pub(super) enum Anchor {
    Left,
    Center,
    Right,
    Decimal(Option<usize>),
}

impl Anchor {
    pub(super) fn measure(self, layout: &MathLayout) -> Result<f32, EngineError> {
        match self {
            Self::Left => Ok(0.0),
            Self::Center => Ok(layout.width / 2.0),
            Self::Right | Self::Decimal(None) => Ok(layout.width),
            Self::Decimal(Some(offset)) => layout
                .slots
                .iter()
                .filter(|slot| slot.path.is_empty() && slot.off == offset)
                .map(|slot| slot.x)
                .min_by(f32::total_cmp)
                .filter(|x| (0.0..=layout.width).contains(x))
                .ok_or_else(|| unsupported("MathML decimal alignment caret position")),
        }
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
) -> Result<Anchor, EngineError> {
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
    match requested {
        "left" => Ok(Anchor::Left),
        "center" => Ok(Anchor::Center),
        "right" => Ok(Anchor::Right),
        "decimalpoint" => decimal(cell, marker),
        _ => Err(unsupported("MathML group alignment")),
    }
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
