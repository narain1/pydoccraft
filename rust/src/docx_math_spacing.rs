//! Keep generated equation spacing guides out of normalized DOCX text.
use std::{borrow::Cow, cell::Cell};

use wordcraft_doc::{
    math::{to_linear, Arg, MNode},
    math_linear::map_args,
    para::InlineObject,
    props::Rgb,
};

pub(crate) const GUIDE_FONT: &str = "Liberation Serif";
pub(crate) const GUIDE_COLOR: Rgb = Rgb(1, 2, 3);
pub(crate) const SHIFT_GUIDE_COLOR: Rgb = Rgb(1, 2, 4);

pub(super) fn visible_text(object: &InlineObject) -> Cow<'_, str> {
    if let InlineObject::Equation { linear, math, .. } = object {
        if linear.contains("⟡H") || linear.contains("⟡g") {
            let removed = Cell::new(false);
            let nodes = remove_guides(math.nodes.clone(), &removed);
            if removed.get() {
                return Cow::Owned(to_linear(&nodes));
            }
        }
    }
    Cow::Borrowed(object.plain_text())
}

fn remove_guides(nodes: Arg, removed: &Cell<bool>) -> Arg {
    let mut result = Vec::with_capacity(nodes.len());
    for node in nodes {
        if let Some(body) = shifted_body(&node) {
            removed.set(true);
            result.extend(remove_guides(body.to_vec(), removed));
        } else if guide_flags(&node, GUIDE_COLOR)
            .is_some_and(|flags| matches!(flags, ("H", false, true) | ("g", true, false)))
        {
            removed.set(true);
        } else {
            result.push(map_args(node, &|arg| remove_guides(arg, removed)));
        }
    }
    result
}

fn shifted_body(node: &MNode) -> Option<&Arg> {
    let MNode::Phant {
        show: true,
        zero_asc: true,
        zero_desc: true,
        e,
        ..
    } = node
    else {
        return None;
    };
    let [MNode::EqArr { rows }] = e.as_slice() else {
        return None;
    };
    let [row] = rows.as_slice() else {
        return None;
    };
    let [MNode::Phant {
        show: true,
        zero_wid: false,
        zero_asc: true,
        zero_desc: true,
        e,
    }, marker, ..] = row.as_slice()
    else {
        return None;
    };
    (guide_flags(marker, SHIFT_GUIDE_COLOR) == Some(("H", true, true))).then_some(e)
}

fn guide_flags(node: &MNode, color: Rgb) -> Option<(&str, bool, bool)> {
    let MNode::Phant {
        show: false,
        zero_wid: true,
        zero_asc,
        zero_desc,
        e,
    } = node
    else {
        return None;
    };
    match e.as_slice() {
        [MNode::Run(run)]
            if run.nor && run.color == Some(color) && run.font.as_deref() == Some(GUIDE_FONT) =>
        {
            Some((run.text.as_str(), *zero_asc, *zero_desc))
        }
        _ => None,
    }
}
