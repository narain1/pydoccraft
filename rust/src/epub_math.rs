//! Project bounded Presentation MathML onto the existing WordCraft equation model.
use roxmltree::Node;
use wordcraft_doc::{
    math::{self, Arg, ColJc, FracKind, LimLoc, MNode, MRun, MScr, MSty, Math, ScriptKind},
    para::InlineObject,
};

use super::{invalid, limited, parse_xml, unsupported};
use crate::EngineError;

const MATHML: &str = "http://www.w3.org/1998/Math/MathML";
const DEPTH: usize = 24;
const MAX_EQUATION_BYTES: usize = 256 * 1024;
const MAX_MATRIX_SIZE: usize = 32;

pub(super) fn equation(source: &str, remaining: &mut usize) -> Result<InlineObject, EngineError> {
    if source.len() > MAX_EQUATION_BYTES {
        return Err(limited("MathML equation exceeds its byte budget"));
    }
    let xml = parse_xml(source.as_bytes())?;
    let root = xml.root_element();
    if !root.has_tag_name((MATHML, "math")) {
        return Err(invalid("MathML equation must have a math root"));
    }
    let nodes = math::merge_runs(elements(root, 0, remaining)?);
    Ok(InlineObject::Equation {
        linear: math::to_linear(&nodes),
        display: root.attribute("display") == Some("block"),
        math: Math {
            nodes,
            ..Default::default()
        },
    })
}

fn elements(node: Node<'_, '_>, depth: usize, remaining: &mut usize) -> Result<Arg, EngineError> {
    let mut rows = Vec::new();
    for child in node.children().filter(Node::is_element) {
        rows.extend(element(child, depth + 1, remaining)?);
    }
    Ok(rows)
}

fn element(node: Node<'_, '_>, depth: usize, remaining: &mut usize) -> Result<Arg, EngineError> {
    if depth > DEPTH || *remaining == 0 {
        return Err(limited("MathML structure exceeds its layout budget"));
    }
    *remaining -= 1;
    if node.tag_name().namespace() != Some(MATHML) {
        return Err(unsupported("foreign elements in MathML equations"));
    }
    let tag = node.tag_name().name();
    let native = match tag {
        "mi" | "mn" | "mo" | "ms" | "mtext" => token(node)?,
        "math" | "mrow" | "mstyle" | "merror" => return elements(node, depth, remaining),
        "semantics" => {
            let presentation = node
                .children()
                .find(Node::is_element)
                .ok_or_else(|| invalid("MathML semantics has no presentation child"))?;
            return element(presentation, depth + 1, remaining);
        }
        "mfrac" => {
            let [num, den] = arguments(node, depth, remaining)?;
            let kind = if node.attribute("bevelled") == Some("true") {
                FracKind::Skewed
            } else if node.attribute("linethickness").is_some_and(|value| {
                let value = value.trim();
                value == "none"
                    || value
                        .trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%')
                        .parse::<f32>()
                        .is_ok_and(|value| value <= 0.0)
            }) {
                FracKind::NoBar
            } else {
                FracKind::Bar
            };
            MNode::Frac { kind, num, den }
        }
        "msqrt" => MNode::Rad {
            deg: Vec::new(),
            deg_hide: true,
            e: elements(node, depth, remaining)?,
        },
        "mroot" => {
            let [e, deg] = arguments(node, depth, remaining)?;
            MNode::Rad {
                deg,
                deg_hide: false,
                e,
            }
        }
        "msup" | "msub" | "msubsup" | "munder" | "mover" | "munderover" => {
            let (base, sub, sup) = match tag {
                "msubsup" | "munderover" => {
                    let [base, sub, sup] = arguments(node, depth, remaining)?;
                    (base, sub, sup)
                }
                "msub" | "munder" => {
                    let [base, sub] = arguments(node, depth, remaining)?;
                    (base, sub, Vec::new())
                }
                _ => {
                    let [base, sup] = arguments(node, depth, remaining)?;
                    (base, Vec::new(), sup)
                }
            };
            scripts(base, sub, sup, tag)
        }
        "mmultiscripts" => return multiscripts(node, depth, remaining),
        "mfenced" => {
            let beg = fence(node, "open", '(')?;
            let end = fence(node, "close", ')')?;
            let sep = fence(node, "separators", ',')?;
            let elems = node
                .children()
                .filter(Node::is_element)
                .map(|child| element(child, depth + 1, remaining))
                .collect::<Result<_, _>>()?;
            MNode::Delim {
                beg,
                end,
                sep,
                grow: true,
                shp_match: false,
                elems,
            }
        }
        "mtable" => return matrix(node, depth, remaining),
        "mphantom" => MNode::Phant {
            show: false,
            zero_wid: false,
            zero_asc: false,
            zero_desc: false,
            e: elements(node, depth, remaining)?,
        },
        "menclose" => enclosure(node, depth, remaining)?,
        _ => return Err(unsupported(&format!("MathML element {tag}"))),
    };
    Ok(vec![native])
}

fn token(node: Node<'_, '_>) -> Result<MNode, EngineError> {
    let tag = node.tag_name().name();
    if node.children().any(|child| child.is_element()) {
        return Err(unsupported("nested content in a MathML token"));
    }
    let value: String = node
        .children()
        .filter_map(|child| child.text().filter(|_| child.is_text()))
        .collect();
    let mut run = MRun::new(value.split_whitespace().collect::<Vec<_>>().join(" "));
    if tag == "ms" {
        let left = node.attribute("lquote").unwrap_or("\"");
        let right = node.attribute("rquote").unwrap_or("\"");
        run.text = format!("{left}{}{right}", run.text);
    }
    run.nor = matches!(tag, "mtext" | "ms");
    // Literal number signs must never become WordCraft automatic equation numbers.
    run.lit = run.text.contains('#');
    let variant = node.attribute("mathvariant");
    let (style, alphabet) = match variant {
        None if tag == "mi" && run.text.chars().count() == 1 => (MSty::Italic, MScr::Roman),
        None | Some("normal") => (MSty::Plain, MScr::Roman),
        Some("bold") => (MSty::Bold, MScr::Roman),
        Some("italic") => (MSty::Italic, MScr::Roman),
        Some("bold-italic") => (MSty::BoldItalic, MScr::Roman),
        Some("double-struck") => (MSty::Plain, MScr::DoubleStruck),
        Some("script") => (MSty::Plain, MScr::Script),
        Some("bold-script") => (MSty::Bold, MScr::Script),
        Some("fraktur") => (MSty::Plain, MScr::Fraktur),
        Some("bold-fraktur") => (MSty::Bold, MScr::Fraktur),
        Some("sans-serif") => (MSty::Plain, MScr::SansSerif),
        Some("bold-sans-serif") => (MSty::Bold, MScr::SansSerif),
        Some("sans-serif-italic") => (MSty::Italic, MScr::SansSerif),
        Some("sans-serif-bold-italic") => (MSty::BoldItalic, MScr::SansSerif),
        Some("monospace") => (MSty::Plain, MScr::Monospace),
        Some(_) => return Err(unsupported("MathML token variant")),
    };
    run.sty = Some(style);
    run.scr = alphabet;
    Ok(MNode::Run(run))
}

fn arguments<const N: usize>(
    node: Node<'_, '_>,
    depth: usize,
    remaining: &mut usize,
) -> Result<[Arg; N], EngineError> {
    let mut children = node.children().filter(Node::is_element);
    let mut args = std::array::from_fn(|_| Vec::new());
    for arg in &mut args {
        let child = children
            .next()
            .ok_or_else(|| invalid("missing MathML operand"))?;
        *arg = element(child, depth + 1, remaining)?;
    }
    if children.next().is_some() {
        return Err(invalid("too many MathML operands"));
    }
    Ok(args)
}

fn single_char(arg: &Arg) -> Option<char> {
    let [MNode::Run(run)] = arg.as_slice() else {
        return None;
    };
    let mut chars = run.text.chars();
    let value = chars.next()?;
    chars.next().is_none().then_some(value)
}

fn scripts(base: Arg, sub: Arg, sup: Arg, tag: &str) -> MNode {
    let stacked = matches!(tag, "munder" | "mover" | "munderover");
    if let Some(chr) = single_char(&base).filter(|ch| math::is_nary_char(*ch)) {
        return MNode::Nary {
            chr,
            lim_loc: Some(if stacked {
                LimLoc::UndOvr
            } else {
                LimLoc::SubSup
            }),
            grow: true,
            sub_hide: sub.is_empty(),
            sup_hide: sup.is_empty(),
            sub,
            sup,
            e: Vec::new(),
        };
    }
    if !stacked {
        let kind = match tag {
            "msub" => ScriptKind::Sub,
            "msup" => ScriptKind::Sup,
            _ => ScriptKind::SubSup,
        };
        return MNode::Script {
            kind,
            base,
            sub,
            sup,
        };
    }
    match tag {
        "munder" if matches!(single_char(&sub), Some('_' | '¯' | '‾')) => MNode::Bar {
            top: false,
            e: base,
        },
        "mover" if matches!(single_char(&sup), Some('¯' | '‾')) => {
            MNode::Bar { top: true, e: base }
        }
        "munder" => MNode::Lim {
            upper: false,
            e: base,
            lim: sub,
        },
        "mover" => MNode::Lim {
            upper: true,
            e: base,
            lim: sup,
        },
        _ => MNode::Lim {
            upper: true,
            e: vec![MNode::Lim {
                upper: false,
                e: base,
                lim: sub,
            }],
            lim: sup,
        },
    }
}

fn multiscripts(
    node: Node<'_, '_>,
    depth: usize,
    remaining: &mut usize,
) -> Result<Arg, EngineError> {
    let mut children = node.children().filter(Node::is_element);
    let base = children
        .next()
        .ok_or_else(|| invalid("missing MathML script base"))?;
    let mut base = element(base, depth + 1, remaining)?;
    let mut pre = false;
    let mut scripts = Vec::new();
    for child in children {
        if child.has_tag_name((MATHML, "mprescripts")) {
            if pre || scripts.len() % 2 != 0 {
                return Err(invalid("invalid MathML prescript position"));
            }
            pre = true;
        } else {
            let arg = if child.has_tag_name((MATHML, "none")) {
                if *remaining == 0 {
                    return Err(limited("MathML structure exceeds its layout budget"));
                }
                *remaining -= 1;
                Vec::new()
            } else {
                element(child, depth + 1, remaining)?
            };
            scripts.push((pre, arg));
        }
    }
    if scripts.len() > 4 {
        return Err(unsupported(
            "multiple MathML subscript and superscript pairs",
        ));
    }
    let mut groups = scripts.into_iter();
    while let Some((pre, sub)) = groups.next() {
        let (sup_pre, sup) = groups
            .next()
            .ok_or_else(|| invalid("incomplete MathML script pair"))?;
        if sup_pre != pre {
            return Err(invalid("incomplete MathML script pair"));
        }
        let kind = if pre {
            ScriptKind::Pre
        } else {
            ScriptKind::SubSup
        };
        base = vec![MNode::Script {
            kind,
            base,
            sub,
            sup,
        }];
    }
    Ok(base)
}

fn fence(node: Node<'_, '_>, name: &str, default: char) -> Result<Option<char>, EngineError> {
    let Some(value) = node.attribute(name) else {
        return Ok(Some(default));
    };
    let mut chars = value.trim().chars();
    let first = chars.next();
    if chars.next().is_some() {
        return Err(unsupported("multi-character MathML fences or separators"));
    }
    Ok(first)
}

fn matrix(node: Node<'_, '_>, depth: usize, remaining: &mut usize) -> Result<Arg, EngineError> {
    let mut rows = Vec::new();
    for row in node.children().filter(Node::is_element) {
        if !row.has_tag_name((MATHML, "mtr")) {
            return Err(unsupported("MathML table row type"));
        }
        let mut cells = Vec::new();
        for cell in row.children().filter(Node::is_element) {
            if !cell.has_tag_name((MATHML, "mtd")) {
                return Err(invalid("unexpected MathML table child"));
            }
            if ["rowspan", "columnspan"]
                .iter()
                .any(|name| cell.attribute(*name).is_some_and(|span| span != "1"))
            {
                return Err(unsupported("MathML table spans"));
            }
            if cells.len() == MAX_MATRIX_SIZE || *remaining == 0 {
                return Err(limited("MathML matrix exceeds its layout budget"));
            }
            *remaining -= 1;
            cells.push(elements(cell, depth + 1, remaining)?);
        }
        if rows.len() == MAX_MATRIX_SIZE {
            return Err(limited("MathML matrix exceeds its layout budget"));
        }
        rows.push(cells);
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return Err(invalid("MathML matrix has no cells"));
    }
    let mut col_jc = node
        .attribute("columnalign")
        .unwrap_or("center")
        .split_whitespace()
        .map(|alignment| match alignment {
            "left" => Ok(ColJc::Left),
            "center" => Ok(ColJc::Center),
            "right" => Ok(ColJc::Right),
            _ => Err(unsupported("MathML table column alignment")),
        })
        .take(columns)
        .collect::<Result<Vec<_>, _>>()?;
    col_jc.resize(columns, col_jc.last().copied().unwrap_or_default());
    for row in &mut rows {
        row.resize_with(columns, Vec::new);
    }
    Ok(vec![MNode::Matrix { rows, col_jc }])
}

fn enclosure(
    node: Node<'_, '_>,
    depth: usize,
    remaining: &mut usize,
) -> Result<MNode, EngineError> {
    let mut hide = [true; 4];
    let mut strike = [false; 4];
    for part in node
        .attribute("notation")
        .unwrap_or("longdiv")
        .split_whitespace()
    {
        match part {
            "box" => hide.fill(false),
            "top" => hide[0] = false,
            "bottom" => hide[1] = false,
            "left" => hide[2] = false,
            "right" => hide[3] = false,
            "horizontalstrike" => strike[0] = true,
            "verticalstrike" => strike[1] = true,
            "updiagonalstrike" => strike[2] = true,
            "downdiagonalstrike" => strike[3] = true,
            _ => return Err(unsupported("MathML enclosure notation")),
        }
    }
    Ok(MNode::BorderBox {
        hide,
        strike,
        e: elements(node, depth, remaining)?,
    })
}
