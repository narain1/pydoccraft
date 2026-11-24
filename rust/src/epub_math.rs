//! Project bounded Presentation MathML onto the existing WordCraft equation model.
use roxmltree::Node;
use wordcraft_doc::{
    math::{self, Arg, ColJc, FracKind, LimLoc, MNode, MRun, MScr, MSty, Math, ScriptKind},
    para::InlineObject,
    props::{CharProps, Rgb, TextColor},
};

use super::{css, invalid, limited, parse_xml, unsupported};
use crate::EngineError;

const MATHML: &str = "http://www.w3.org/1998/Math/MathML";
const DEPTH: usize = 24;
const MAX_EQUATION_BYTES: usize = 256 * 1024;
const MAX_MATRIX_SIZE: usize = 32;

pub(super) fn presentation_text(root: Node<'_, '_>) -> String {
    fn append(node: Node<'_, '_>, output: &mut String) {
        if node.is_text() {
            output.push_str(node.text().unwrap_or_default());
        } else if node.has_tag_name((MATHML, "semantics")) {
            if let Some(presentation) = node.children().find(Node::is_element) {
                append(presentation, output);
            }
        } else if !node.has_tag_name((MATHML, "annotation"))
            && !node.has_tag_name((MATHML, "annotation-xml"))
        {
            for child in node.children() {
                append(child, output);
            }
        }
    }
    let mut output = String::new();
    append(root, &mut output);
    output
}

pub(super) fn equation(
    source: &str,
    format: &mut CharProps,
    base_size: f32,
    remaining: &mut usize,
) -> Result<InlineObject, EngineError> {
    if source.len() > MAX_EQUATION_BYTES {
        return Err(limited("MathML equation exceeds its byte budget"));
    }
    let xml = parse_xml(source.as_bytes())?;
    let root = xml.root_element();
    if !root.has_tag_name((MATHML, "math")) {
        return Err(invalid("MathML equation must have a math root"));
    }
    let display = root.attribute("display") == Some("block");
    let style = Presentation {
        base_size,
        display_style: display,
        ..Default::default()
    }
    .on(root)?;
    if let Some(size) = style.size {
        format.size = Some(size);
    }
    if let Some(ink) = style.ink {
        format.color = Some(TextColor::Rgb(ink));
    }
    let mut projection = Projection {
        root: style,
        remaining,
    };
    let nodes = math::merge_runs(projection.elements(root, 0, style)?);
    Ok(InlineObject::Equation {
        linear: math::to_linear(&nodes),
        display,
        math: Math {
            nodes,
            ..Default::default()
        },
    })
}

#[derive(Clone, Copy, Default)]
struct Presentation<'a> {
    base_size: f32,
    size: Option<f32>,
    ink: Option<Rgb>,
    variant: Option<&'a str>,
    display_style: bool,
}

impl<'a> Presentation<'a> {
    fn on(mut self, node: Node<'a, '_>) -> Result<Self, EngineError> {
        if let Some(color) = node.attribute("mathcolor") {
            if !color.eq_ignore_ascii_case("currentcolor") {
                let ([r, g, b], alpha) =
                    css::color(color).ok_or_else(|| invalid("invalid MathML mathcolor"))?;
                if alpha != 1.0 {
                    return Err(unsupported("translucent MathML mathcolor"));
                }
                self.ink = Some(Rgb(r, g, b));
            }
        }
        if let Some(size) = node
            .attribute("mathsize")
            .map(str::trim)
            .filter(|size| *size != "normal")
        {
            let parent = self.font_size();
            let points = match size {
                "small" => Some(parent / 1.2),
                "big" => Some(parent * 1.2),
                _ => size
                    .strip_suffix('%')
                    .and_then(|v| v.trim().parse::<f32>().ok())
                    .map(|percent| percent / 100.0 * parent)
                    .or_else(|| css::points(size, parent, [0.0; 2])),
            };
            self.size = Some(
                points
                    .filter(|size| (1.0..=1638.0).contains(size))
                    .ok_or_else(|| {
                        unsupported("MathML mathsize outside the native length range")
                    })?,
            );
        }
        if let Some(variant) = node.attribute("mathvariant") {
            self.variant = Some(variant);
        }
        if let Some(display) = boolean(node, "displaystyle")? {
            self.display_style = display;
        }
        Ok(self)
    }

    fn font_size(self) -> f32 {
        self.size.unwrap_or(self.base_size)
    }

    fn apply(self, run: &mut MRun) {
        run.size = self.size;
        run.color = self.ink;
    }
}

struct Projection<'a, 'xml> {
    root: Presentation<'xml>,
    remaining: &'a mut usize,
}

impl<'xml> Projection<'_, 'xml> {
    fn charge(&mut self, depth: usize) -> Result<(), EngineError> {
        if depth > DEPTH || *self.remaining == 0 {
            return Err(limited("MathML structure exceeds its layout budget"));
        }
        *self.remaining -= 1;
        Ok(())
    }

    fn elements(
        &mut self,
        node: Node<'xml, '_>,
        depth: usize,
        style: Presentation<'xml>,
    ) -> Result<Arg, EngineError> {
        let mut rows = Vec::new();
        for child in node.children().filter(Node::is_element) {
            rows.extend(self.element(child, depth + 1, style)?);
        }
        Ok(rows)
    }

    fn element(
        &mut self,
        node: Node<'xml, '_>,
        depth: usize,
        style: Presentation<'xml>,
    ) -> Result<Arg, EngineError> {
        self.charge(depth)?;
        if node.tag_name().namespace() != Some(MATHML) {
            return Err(unsupported("foreign elements in MathML equations"));
        }
        let style = style.on(node)?;
        let tag = node.tag_name().name();
        let native = match tag {
            "mi" | "mn" | "mo" | "ms" | "mtext" => token(node, style)?,
            "math" | "mrow" | "mstyle" | "merror" => return self.elements(node, depth, style),
            "semantics" => {
                let presentation = node
                    .children()
                    .find(Node::is_element)
                    .ok_or_else(|| invalid("MathML semantics has no presentation child"))?;
                return self.element(presentation, depth + 1, style);
            }
            "mspace" => return space(node, style),
            "mpadded" => return padded(node, self.elements(node, depth, style)?),
            "mfrac" => {
                let [num, den] = self.arguments(node, depth, style)?;
                let kind = if boolean(node, "bevelled")? == Some(true) {
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
                e: self.elements(node, depth, style)?,
            },
            "mroot" => {
                let [e, deg] = self.arguments(node, depth, style)?;
                MNode::Rad {
                    deg,
                    deg_hide: false,
                    e,
                }
            }
            "msup" | "msub" | "msubsup" | "munder" | "mover" | "munderover" => {
                let (base, sub, sup) = match tag {
                    "msubsup" | "munderover" => {
                        let [base, sub, sup] = self.arguments(node, depth, style)?;
                        (base, sub, sup)
                    }
                    "msub" | "munder" => {
                        let [base, sub] = self.arguments(node, depth, style)?;
                        (base, sub, Vec::new())
                    }
                    _ => {
                        let [base, sup] = self.arguments(node, depth, style)?;
                        (base, Vec::new(), sup)
                    }
                };
                scripts(node, base, sub, sup, style.display_style, self.root)?
            }
            "mmultiscripts" => return self.multiscripts(node, depth, style),
            "mfenced" => self.fenced(node, depth, style)?,
            "mtable" => return self.matrix(node, depth, style),
            "mphantom" => MNode::Phant {
                show: false,
                zero_wid: false,
                zero_asc: false,
                zero_desc: false,
                e: self.elements(node, depth, style)?,
            },
            "menclose" => enclosure(node, self.elements(node, depth, style)?)?,
            _ => return Err(unsupported(&format!("MathML element {tag}"))),
        };
        Ok(vec![native])
    }

    fn arguments<const N: usize>(
        &mut self,
        node: Node<'xml, '_>,
        depth: usize,
        style: Presentation<'xml>,
    ) -> Result<[Arg; N], EngineError> {
        let mut children = node.children().filter(Node::is_element);
        let mut args = std::array::from_fn(|_| Vec::new());
        for arg in &mut args {
            let child = children
                .next()
                .ok_or_else(|| invalid("missing MathML operand"))?;
            *arg = self.element(child, depth + 1, style)?;
        }
        if children.next().is_some() {
            return Err(invalid("too many MathML operands"));
        }
        Ok(args)
    }

    fn fenced(
        &mut self,
        node: Node<'xml, '_>,
        depth: usize,
        style: Presentation<'xml>,
    ) -> Result<MNode, EngineError> {
        let (beg, leading) = fence(node.attribute("open").unwrap_or("("));
        let (end, trailing) = fence(node.attribute("close").unwrap_or(")"));
        let separators = node.attribute("separators").unwrap_or(",");
        let mut separators = separators.chars().filter(|c| !c.is_ascii_whitespace());
        let mut elems = Vec::new();
        let mut row = Vec::new();
        let mut sep = separators.next();
        // OMML delimiters have one repeating separator. A varying sequence
        // uses ordinary math runs between operands inside the outer fences.
        let use_runs =
            leading.is_some() || trailing.is_some() || separators.clone().next().is_some();
        if let Some(leading) = leading {
            self.charge(depth + 1)?;
            row.push(styled_run(leading, style));
        }
        for (index, child) in node.children().filter(Node::is_element).enumerate() {
            let elem = self.element(child, depth + 1, style)?;
            if !use_runs {
                elems.push(elem);
                continue;
            }
            if let Some(separator) = sep.filter(|_| index != 0) {
                self.charge(depth + 1)?;
                row.push(styled_run(separator.to_string(), style));
                sep = separators.next().or(sep);
            }
            row.extend(elem);
        }
        if let Some(trailing) = trailing {
            self.charge(depth + 1)?;
            row.push(styled_run(trailing, style));
        }
        if use_runs {
            elems.push(row);
            sep = None;
        }
        Ok(MNode::Delim {
            beg,
            end,
            sep,
            grow: true,
            shp_match: false,
            elems,
        })
    }
}

fn token(node: Node<'_, '_>, style: Presentation<'_>) -> Result<MNode, EngineError> {
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
    // Preserve literal symbols used by WordCraft for equation numbering and alignment.
    run.lit = run.text.contains(['#', '&']);
    style.apply(&mut run);
    let (variant, alphabet) = match style.variant {
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
    run.sty = Some(variant);
    run.scr = alphabet;
    Ok(MNode::Run(run))
}

fn single_char(arg: &Arg) -> Option<char> {
    let [MNode::Run(run)] = arg.as_slice() else {
        return None;
    };
    let mut chars = run.text.chars();
    let value = chars.next()?;
    chars.next().is_none().then_some(value)
}

fn boolean(node: Node<'_, '_>, key: &str) -> Result<Option<bool>, EngineError> {
    match node.attribute(key).map(str::trim) {
        Some("true") => Ok(Some(true)),
        Some("false") => Ok(Some(false)),
        None => Ok(None),
        Some(_) => Err(invalid(format!("invalid MathML {key} boolean"))),
    }
}

fn operator<'a, 'input>(mut node: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    while node.tag_name().namespace() == Some(MATHML) {
        match node.tag_name().name() {
            "mo" => return Some(node),
            "mrow" | "mstyle" => {
                let mut children = node.children().filter(Node::is_element);
                node = children.next()?;
                if children.next().is_some() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    None
}

fn scripts(
    node: Node<'_, '_>,
    base: Arg,
    sub: Arg,
    sup: Arg,
    display_style: bool,
    root: Presentation<'_>,
) -> Result<MNode, EngineError> {
    let tag = node.tag_name().name();
    let stacked = matches!(tag, "munder" | "mover" | "munderover");
    let mut operands = node.children().filter(Node::is_element);
    let base_operator = operands.next().and_then(operator);
    let sub_accent = if matches!(tag, "munder" | "munderover") {
        accent(
            node,
            "accentunder",
            operands.next().and_then(operator),
            &sub,
            false,
            root,
        )?
    } else {
        None
    };
    let sup_accent = if matches!(tag, "mover" | "munderover") {
        accent(
            node,
            "accent",
            operands.next().and_then(operator),
            &sup,
            true,
            root,
        )?
    } else {
        None
    };
    let mut sideways = !stacked;
    if let Some(op) = base_operator {
        let nary = single_char(&base).filter(|ch| math::is_nary_char(*ch));
        let movable = boolean(op, "movablelimits")?
            .unwrap_or_else(|| nary.is_some_and(|ch| !math::is_integral(ch)));
        sideways |= !display_style && movable;
        if let Some(chr) = nary {
            let grow = boolean(op, "largeop")?.unwrap_or(true);
            if sub_accent.is_none() && sup_accent.is_none() && generated_operator_style(&base, root)
            {
                return Ok(MNode::Nary {
                    chr,
                    lim_loc: Some(if sideways {
                        LimLoc::SubSup
                    } else {
                        LimLoc::UndOvr
                    }),
                    grow,
                    sub_hide: sub.is_empty(),
                    sup_hide: sup.is_empty(),
                    sub,
                    sup,
                    e: Vec::new(),
                });
            }
        }
    }
    if sideways && sub_accent.is_none() && sup_accent.is_none() {
        let kind = match tag {
            "msub" | "munder" => ScriptKind::Sub,
            "msup" | "mover" => ScriptKind::Sup,
            _ => ScriptKind::SubSup,
        };
        return Ok(MNode::Script {
            kind,
            base,
            sub,
            sup,
        });
    }
    let base = if matches!(tag, "munder" | "munderover") {
        let under = match sub_accent {
            Some(mark) => mark.attach(base),
            None => MNode::Lim {
                upper: false,
                e: base,
                lim: sub,
            },
        };
        if tag == "munder" {
            return Ok(under);
        }
        vec![under]
    } else {
        base
    };
    Ok(match sup_accent {
        Some(mark) => mark.attach(base),
        None => MNode::Lim {
            upper: true,
            e: base,
            lim: sup,
        },
    })
}

fn generated_operator_style(arg: &Arg, root: Presentation<'_>) -> bool {
    matches!(arg.as_slice(), [MNode::Run(run)]
        if run.size == root.size && run.color == root.ink && run.scr == MScr::Roman
            && run.sty == Some(MSty::Plain))
}

enum Accent {
    Over(char),
    Underbar,
    Stretch(char, bool),
}

impl Accent {
    fn attach(self, e: Arg) -> MNode {
        match self {
            Self::Over(chr) => MNode::Acc { chr, e },
            Self::Underbar => MNode::Bar { top: false, e },
            Self::Stretch(chr, top) => MNode::GroupChr { chr, top, e },
        }
    }
}

fn accent(
    node: Node<'_, '_>,
    attribute: &str,
    operator: Option<Node<'_, '_>>,
    arg: &Arg,
    top: bool,
    root: Presentation<'_>,
) -> Result<Option<Accent>, EngineError> {
    let explicit = boolean(node, attribute)?.or(operator
        .map(|op| boolean(op, "accent"))
        .transpose()?
        .flatten());
    let character = operator.and_then(|_| single_char(arg));
    let inferred = character.is_some_and(|c| {
        matches!(
            c,
            '^' | 'ˆ'
                | '~'
                | '˜'
                | '¯'
                | '‾'
                | '_'
                | '˙'
                | '¨'
                | 'ˇ'
                | '˘'
                | '⏞'
                | '⏟'
                | '⏜'
                | '⏝'
                | '→'
                | '←'
                | '↔'
                | '\u{0300}'..='\u{036f}'
        )
    });
    if !explicit.unwrap_or(inferred) {
        return Ok(None);
    }
    let ch = character
        .ok_or_else(|| unsupported("MathML accents require a single operator character"))?;
    if !generated_operator_style(arg, root) {
        if explicit == Some(true) {
            return Err(unsupported("independently styled MathML accent operators"));
        }
        return Ok(None);
    }
    let accent = match (ch, top) {
        ('_' | '¯' | '‾' | '\u{0332}', false) => Accent::Underbar,
        ('⏞' | '⏟' | '⏜' | '⏝' | '→' | '←' | '↔', _) | (_, false) => {
            Accent::Stretch(ch, top)
        }
        ('^' | 'ˆ', true) => Accent::Over('\u{0302}'),
        ('~' | '˜', true) => Accent::Over('\u{0303}'),
        ('˙', true) => Accent::Over('\u{0307}'),
        ('¨', true) => Accent::Over('\u{0308}'),
        ('ˇ', true) => Accent::Over('\u{030c}'),
        ('˘', true) => Accent::Over('\u{0306}'),
        _ => Accent::Over(ch),
    };
    Ok(Some(accent))
}

impl<'xml> Projection<'_, 'xml> {
    fn multiscripts(
        &mut self,
        node: Node<'xml, '_>,
        depth: usize,
        style: Presentation<'xml>,
    ) -> Result<Arg, EngineError> {
        let mut children = node.children().filter(Node::is_element);
        let base = children
            .next()
            .ok_or_else(|| invalid("missing MathML script base"))?;
        // Each script pair nests one native script node. Charge this depth before
        // descending into the base or any script operand.
        let count = children.clone().count();
        if count > 17 {
            return Err(limited("MathML multiscript pair limit exceeds eight"));
        }
        let levels = count / 2;
        let mut base = self.element(base, depth + levels.max(1), style)?;
        let mut pre = false;
        let mut scripts = Vec::new();
        for child in children {
            if child.has_tag_name((MATHML, "mprescripts")) {
                if pre || scripts.len() % 2 != 0 {
                    return Err(invalid("invalid MathML prescript position"));
                }
                self.charge(depth + 1)?;
                pre = true;
            } else {
                let arg = if child.has_tag_name((MATHML, "none")) {
                    self.charge(depth + 1)?;
                    if child.children().any(|node| node.is_element()) {
                        return Err(invalid("MathML none has unexpected children"));
                    }
                    Vec::new()
                } else {
                    self.element(child, depth + levels.max(1), style)?
                };
                scripts.push((pre, arg));
            }
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

    fn matrix(
        &mut self,
        node: Node<'xml, '_>,
        depth: usize,
        style: Presentation<'xml>,
    ) -> Result<Arg, EngineError> {
        let mut rows = Vec::new();
        for row in node.children().filter(Node::is_element) {
            if !row.has_tag_name((MATHML, "mtr")) {
                return Err(unsupported("MathML table row type"));
            }
            self.charge(depth + 1)?;
            let row_style = style.on(row)?;
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
                if cells.len() == MAX_MATRIX_SIZE {
                    return Err(limited("MathML matrix exceeds its layout budget"));
                }
                self.charge(depth + 2)?;
                cells.push(self.elements(cell, depth + 2, row_style.on(cell)?)?);
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
}

fn fence(value: &str) -> (Option<char>, Option<&str>) {
    let value = value.trim();
    let mut characters = value.chars();
    let first = characters.next();
    if characters.next().is_some() {
        (None, Some(value))
    } else {
        (first, None)
    }
}

fn styled_run(text: impl Into<String>, style: Presentation<'_>) -> MNode {
    let mut run = MRun::plain(text);
    run.lit = run.text.contains(['#', '&']);
    style.apply(&mut run);
    MNode::Run(run)
}

fn space(node: Node<'_, '_>, style: Presentation<'_>) -> Result<Arg, EngineError> {
    if node
        .children()
        .any(|child| child.is_element() || child.text().is_some_and(|text| !text.trim().is_empty()))
    {
        return Err(invalid("MathML mspace must be empty"));
    }
    for name in ["height", "depth"] {
        if node
            .attribute(name)
            .is_some_and(|value| !zero_length(value))
        {
            return Err(unsupported("vertical MathML mspace dimensions"));
        }
    }
    if node
        .attribute("linebreak")
        .is_some_and(|value| value != "auto")
    {
        return Err(unsupported("MathML mspace linebreak"));
    }
    let width = node.attribute("width").unwrap_or("0").trim();
    if zero_length(width) {
        return Ok(Vec::new());
    }
    let em = style.font_size();
    let named = match width {
        "veryverythinmathspace" => Some(1.0),
        "verythinmathspace" => Some(2.0),
        "thinmathspace" => Some(3.0),
        "mediummathspace" => Some(4.0),
        "thickmathspace" => Some(5.0),
        "verythickmathspace" => Some(6.0),
        "veryverythickmathspace" => Some(7.0),
        _ => None,
    };
    // WordCraft advances a normal space by 0.4 times the run's point size
    // and preserves it in OMML. It participates in native script scaling.
    let size = named
        .map(|units| em * units / 18.0)
        .or_else(|| css::points(width, em, [0.0; 2]))
        .map(|width| width / 0.4)
        .filter(|size| (1.0..=1638.0).contains(size))
        .ok_or_else(|| unsupported("MathML mspace width outside the native length range"))?;
    Ok(vec![MNode::Run(MRun {
        size: Some(size),
        ..MRun::plain(" ")
    })])
}

fn zero_length(value: &str) -> bool {
    let value = value.trim();
    value
        .strip_suffix('%')
        .unwrap_or(value)
        .parse::<f32>()
        .is_ok_and(|value| value == 0.0)
        || css::points(value, 12.0, [100.0; 2]) == Some(0.0)
}

fn padded(node: Node<'_, '_>, e: Arg) -> Result<Arg, EngineError> {
    for key in ["lspace", "voffset"] {
        if node.attribute(key).is_some_and(|value| !zero_length(value)) {
            return Err(unsupported("nonzero MathML mpadded offsets"));
        }
    }
    let metric = |key| match node.attribute(key).map(str::trim) {
        None => Ok(false),
        Some(value) if zero_length(value) => Ok(!value.starts_with(['+', '-'])),
        Some(_) => Err(unsupported("nonzero MathML mpadded dimensions")),
    };
    let (zero_wid, zero_asc, zero_desc) = (metric("width")?, metric("height")?, metric("depth")?);
    if !zero_wid && !zero_asc && !zero_desc {
        return Ok(e);
    }
    Ok(vec![MNode::Phant {
        show: true,
        zero_wid,
        zero_asc,
        zero_desc,
        e,
    }])
}

fn enclosure(node: Node<'_, '_>, e: Arg) -> Result<MNode, EngineError> {
    let notation = node.attribute("notation").unwrap_or("longdiv");
    if notation.trim() == "radical" {
        return Ok(MNode::Rad {
            deg: Vec::new(),
            deg_hide: true,
            e,
        });
    }
    let mut hide = [true; 4];
    let mut strike = [false; 4];
    for part in notation.split_whitespace() {
        match part {
            "box" => hide.fill(false),
            "actuarial" => {
                hide[0] = false;
                hide[3] = false;
            }
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
    Ok(MNode::BorderBox { hide, strike, e })
}
