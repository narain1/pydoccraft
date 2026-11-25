//! Unicode controls used only in the XHTML inline layout stream.
use roxmltree::Node;

use super::{name, XHTML};

#[derive(Clone, Copy, Default)]
pub(super) enum Mode {
    #[default]
    Normal,
    Embed,
    Isolate,
    Override,
    IsolateOverride,
    Plaintext,
}

impl Mode {
    pub(super) fn resolve(node: Node<'_, '_>, value: Option<&str>, parent: Self) -> Self {
        match value.map(str::to_ascii_lowercase).as_deref() {
            Some("normal" | "initial" | "unset") => Self::Normal,
            Some("inherit") => parent,
            Some("embed") => Self::Embed,
            Some("isolate") => Self::Isolate,
            Some("bidi-override") => Self::Override,
            Some("isolate-override") => Self::IsolateOverride,
            Some("plaintext") => Self::Plaintext,
            _ if name(node, XHTML, "bdo") => Self::IsolateOverride,
            _ if name(node, XHTML, "bdi") || has_dir(node) => Self::Isolate,
            _ => Self::Normal,
        }
    }

    pub(super) fn scope(self, rtl: bool) -> Option<Scope> {
        let (start, end) = match (self, rtl) {
            (Self::Normal, _) => return None,
            (Self::Embed, false) => ("\u{202a}", "\u{202c}"),
            (Self::Embed, true) => ("\u{202b}", "\u{202c}"),
            (Self::Isolate, false) => ("\u{2066}", "\u{2069}"),
            (Self::Isolate, true) => ("\u{2067}", "\u{2069}"),
            (Self::Override, false) => ("\u{202d}", "\u{202c}"),
            (Self::Override, true) => ("\u{202e}", "\u{202c}"),
            (Self::IsolateOverride, false) => ("\u{2068}\u{202d}", "\u{202c}\u{2069}"),
            (Self::IsolateOverride, true) => ("\u{2068}\u{202e}", "\u{202c}\u{2069}"),
            (Self::Plaintext, _) => ("\u{2068}", "\u{2069}"),
        };
        Some(Scope { start, end })
    }
}

fn has_dir(node: Node<'_, '_>) -> bool {
    node.tag_name().namespace().is_none_or(|ns| ns == XHTML)
        && node.attribute("dir").is_some_and(|value| {
            ["ltr", "rtl", "auto"]
                .iter()
                .any(|direction| value.eq_ignore_ascii_case(direction))
        })
}

#[derive(Clone, Copy)]
pub(super) struct Scope {
    pub start: &'static str,
    pub end: &'static str,
}

pub(super) fn control(character: char) -> bool {
    matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}
