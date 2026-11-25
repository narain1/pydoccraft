//! Directionality of static XHTML boxes before CSS layout and text shaping.
use roxmltree::Node;
use unicode_bidi::{bidi_class, BidiClass};

use super::XHTML;

pub(super) enum Hint {
    Ltr,
    Rtl,
    Auto,
}

impl Hint {
    pub(super) fn rtl(self, node: Node<'_, '_>) -> bool {
        match self {
            Self::Ltr => false,
            Self::Rtl => true,
            Self::Auto => automatic(node).unwrap_or(false),
        }
    }
}

pub(super) fn hint(node: Node<'_, '_>) -> Option<Hint> {
    let tag = html_tag(node)?;
    explicit(node).or_else(|| {
        if tag == "bdi" {
            Some(Hint::Auto)
        } else if tag == "input"
            && node
                .attribute("type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("tel"))
        {
            Some(Hint::Ltr)
        } else {
            None
        }
    })
}

fn explicit(node: Node<'_, '_>) -> Option<Hint> {
    let value = node.attribute("dir")?;
    if value.eq_ignore_ascii_case("ltr") {
        Some(Hint::Ltr)
    } else if value.eq_ignore_ascii_case("rtl") {
        Some(Hint::Rtl)
    } else if value.eq_ignore_ascii_case("auto") {
        Some(Hint::Auto)
    } else {
        None
    }
}

fn html_tag<'input>(node: Node<'_, 'input>) -> Option<&'input str> {
    let tag = node.tag_name();
    (node.is_element() && tag.namespace().is_none_or(|ns| ns == XHTML)).then(|| tag.name())
}

fn automatic(node: Node<'_, '_>) -> Option<bool> {
    match html_tag(node) {
        Some("input") if uses_input_value(node) => first_strong(node.attribute("value")?),
        Some("textarea") => node
            .children()
            .filter(Node::is_text)
            .filter_map(|child| child.text())
            .find_map(first_strong),
        _ => contained(node),
    }
}

fn uses_input_value(node: Node<'_, '_>) -> bool {
    const OTHER_TYPES: [&str; 12] = [
        "date",
        "month",
        "week",
        "time",
        "datetime-local",
        "number",
        "range",
        "color",
        "checkbox",
        "radio",
        "file",
        "image",
    ];
    let kind = node.attribute("type").unwrap_or_default();
    !OTHER_TYPES
        .iter()
        .any(|excluded| kind.eq_ignore_ascii_case(excluded))
}

fn first_strong(text: &str) -> Option<bool> {
    text.chars()
        .find_map(|character| match bidi_class(character) {
            BidiClass::L => Some(false),
            BidiClass::R | BidiClass::AL => Some(true),
            _ => None,
        })
}

fn contained(node: Node<'_, '_>) -> Option<bool> {
    // EPUB XML is depth-bounded. Pruning independent direction roots means
    // nested auto boxes never rescan each other's text for their direction.
    for child in node.children() {
        let direction = if child.is_text() {
            child.text().and_then(first_strong)
        } else if child.is_element()
            && html_tag(child).is_none_or(|tag| {
                !matches!(tag, "bdi" | "script" | "style" | "textarea" | "template")
                    && explicit(child).is_none()
            })
        {
            contained(child)
        } else {
            None
        };
        if direction.is_some() {
            return direction;
        }
    }
    None
}
