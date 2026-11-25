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
    nodes
        .into_iter()
        .filter_map(|node| {
            if let MNode::Phant {
                show: false,
                zero_wid: true,
                zero_asc,
                zero_desc,
                e,
            } = &node
            {
                if let [MNode::Run(run)] = e.as_slice() {
                    if run.nor
                        && run.color == Some(GUIDE_COLOR)
                        && run.font.as_deref() == Some(GUIDE_FONT)
                        && matches!(
                            (run.text.as_str(), zero_asc, zero_desc),
                            ("H", false, true) | ("g", true, false)
                        )
                    {
                        removed.set(true);
                        return None;
                    }
                }
            }
            Some(map_args(node, &|arg| remove_guides(arg, removed)))
        })
        .collect()
}
