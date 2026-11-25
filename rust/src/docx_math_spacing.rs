//! Keep generated equation spacing guides out of normalized DOCX text.
use std::{borrow::Cow, cell::Cell};

use wordcraft_doc::{
    math::{to_linear, Arg, ColJc, MNode},
    math_linear::map_args,
    para::InlineObject,
    props::Rgb,
};

pub(crate) const GUIDE_FONT: &str = "Liberation Serif";
pub(crate) const GUIDE_COLOR: Rgb = Rgb(1, 2, 3);
pub(crate) const SHIFT_GUIDE_COLOR: Rgb = Rgb(1, 2, 4);
pub(crate) const GRID_GUIDE_COLOR: Rgb = Rgb(1, 2, 5);
pub(crate) const GROUP_SPACE_COLOR: Rgb = Rgb(1, 2, 6);
pub(crate) const NEGATIVE_PADDING_SPACE_COLOR: Rgb = Rgb(1, 2, 7);
pub(crate) const NEGATIVE_PADDING_MARKER: &str = "⁢";

pub(super) fn visible_text(object: &InlineObject) -> Cow<'_, str> {
    if let InlineObject::Equation { linear, math, .. } = object {
        if linear.contains('□') || linear.contains("⟡H") || linear.contains("⟡g") {
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
        } else if let Some(body) = borrowed_padding_body(&node) {
            removed.set(true);
            result.extend(remove_guides(body, removed));
        } else if let Some(matrix) = grid_body(&node) {
            removed.set(true);
            result.push(map_args(matrix, &|arg| remove_guides(arg, removed)));
        } else if is_grid_marker(&node)
            || is_grid_gap(&node)
            || guide_flags(&node, GUIDE_COLOR)
                .is_some_and(|flags| matches!(flags, ("H", false, true) | ("g", true, false)))
        {
            removed.set(true);
        } else {
            result.push(map_args(node, &|arg| remove_guides(arg, removed)));
        }
    }
    result
}

fn grid_body(node: &MNode) -> Option<MNode> {
    let MNode::Boxed { e } = node else {
        return None;
    };
    let [MNode::Phant {
        show: true,
        zero_wid: false,
        zero_asc: true,
        zero_desc: true,
        e,
    }, ..] = e.as_slice()
    else {
        return None;
    };
    let (marker, matrices) = e.split_last()?;
    if !is_grid_marker(marker) {
        return None;
    }
    if let [matrix @ MNode::Matrix { .. }] = matrices {
        return Some(matrix.clone());
    }
    let MNode::Matrix { rows, .. } = matrices.first()? else {
        return None;
    };
    // A 32-column MathML grid may also have two outer tracks for labels.
    if matrices.len() > 34 || !(1..=128).contains(&rows.len()) {
        return None;
    }
    // A tightly spaced MathML grid stores one single-column native matrix
    // per column. Reassemble its logical rows before normalizing cell text.
    let mut grid = vec![Vec::new(); rows.len()];
    for matrix in matrices {
        let MNode::Matrix { rows, col_jc } = matrix else {
            return None;
        };
        if rows.len() != grid.len() || col_jc != &[ColJc::Left] {
            return None;
        }
        for (output, row) in grid.iter_mut().zip(rows) {
            let [cell] = row.as_slice() else {
                return None;
            };
            output.push(cell.clone());
        }
    }
    Some(MNode::Matrix {
        rows: grid,
        col_jc: vec![ColJc::Left; matrices.len()],
    })
}

fn is_grid_marker(node: &MNode) -> bool {
    guide_flags(node, GRID_GUIDE_COLOR) == Some(("H", true, true))
}

fn is_grid_gap(node: &MNode) -> bool {
    let MNode::Phant {
        show: false,
        zero_wid: false,
        zero_asc: true,
        zero_desc: true,
        e,
    } = node
    else {
        return false;
    };
    matches!(e.as_slice(), [_, marker] if is_grid_marker(marker))
}

fn borrowed_padding_body(node: &MNode) -> Option<Arg> {
    let MNode::Boxed { e } = node else {
        return None;
    };
    let (marker, contents) = e.split_last()?;
    let tagged = |node: &MNode| is_math_space(node, NEGATIVE_PADDING_SPACE_COLOR);
    if !matches!(marker, MNode::Run(run) if run.text == NEGATIVE_PADDING_MARKER
        && run.lit && !run.nor && run.size == Some(1.0)
        && run.color == Some(NEGATIVE_PADDING_SPACE_COLOR))
        || !contents.last().is_some_and(tagged)
    {
        return None;
    }
    Some(
        contents
            .iter()
            .filter(|node| {
                !tagged(node)
                    && !matches!(
                        node,
                        MNode::Phant {
                            show: false,
                            zero_wid: true,
                            zero_asc: true,
                            zero_desc: true,
                            e,
                        } if matches!(e.as_slice(), [space] if tagged(space))
                    )
            })
            .cloned()
            .collect(),
    )
}

fn is_math_space(node: &MNode, color: Rgb) -> bool {
    matches!(node, MNode::Run(run) if run.color == Some(color)
        && run.lit && !run.nor && run.size.is_some()
        && !run.text.is_empty() && run.text.bytes().all(|byte| byte == b' '))
}

fn shifted_body(node: &MNode) -> Option<&[MNode]> {
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
    if guide_flags(marker, SHIFT_GUIDE_COLOR) != Some(("H", true, true)) {
        return None;
    }
    let spaces = e
        .iter()
        .take_while(|node| is_math_space(node, GROUP_SPACE_COLOR))
        .count();
    Some(&e[spaces..])
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
