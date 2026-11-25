//! Project bounded Presentation MathML onto the existing WordCraft equation model.
use std::collections::BTreeMap;

use roxmltree::Node;
use serde::{Deserialize, Serialize};
use wordcraft_doc::{
    math::{self, Arg, ColJc, FracKind, LimLoc, MNode, MRun, MScr, MSty, Math, ScriptKind},
    props::{CharProps, Rgb, TextColor},
};

use super::{css, invalid, limited, parse_xml, unsupported};
use crate::EngineError;

#[path = "epub_math_compose.rs"]
pub(super) mod compose;
#[path = "epub_math_fraction.rs"]
mod fraction;
#[path = "epub_math_pdf.rs"]
pub(super) mod pdf;
#[path = "epub_math_table.rs"]
pub(super) mod table;

pub(super) const MATHML: &str = "http://www.w3.org/1998/Math/MathML";
const DEPTH: usize = 24;
const MAX_EQUATION_BYTES: usize = 256 * 1024;
const MAX_MATRIX_SIZE: usize = 32;

#[derive(Deserialize, Serialize)]
pub(super) struct FormulaCss {
    pub base: CharProps,
    pub viewport: [f32; 2],
    pub nodes: BTreeMap<u32, Css>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub(super) struct Css {
    size: Option<String>,
    ink: Option<String>,
    bold: Option<bool>,
    italic: Option<bool>,
    display_style: Option<bool>,
}

impl Css {
    pub(super) fn storage_bytes(&self) -> usize {
        std::mem::size_of::<(roxmltree::NodeId, Self)>()
            + self.size.as_ref().map_or(0, String::len)
            + self.ink.as_ref().map_or(0, String::len)
    }

    pub(super) fn from_properties(values: &css::Properties) -> Option<Self> {
        let get = |key| values.get(key).map(String::as_str).unwrap_or_default();
        let bold = match get("font-weight") {
            "bold" | "bolder" => Some(true),
            "normal" | "lighter" => Some(false),
            weight => weight
                .parse::<f32>()
                .ok()
                .filter(|weight| (1.0..=1000.0).contains(weight))
                .map(|weight| weight >= 600.0),
        };
        let italic = match get("font-style") {
            "italic" | "oblique" => Some(true),
            "normal" => Some(false),
            _ => None,
        };
        let display_style = match get("math-style") {
            "normal" => Some(true),
            "compact" => Some(false),
            _ => None,
        };
        let size = values.get("font-size").cloned();
        let ink = values.get("color").cloned();
        if size.is_none()
            && ink.is_none()
            && bold.is_none()
            && italic.is_none()
            && display_style.is_none()
        {
            return None;
        }
        Some(Self {
            size,
            ink,
            bold,
            italic,
            display_style,
        })
    }
}

pub(super) fn presentation_text(root: Node<'_, '_>) -> String {
    fn append(node: Node<'_, '_>, output: &mut String) {
        if node.is_text() {
            // Preserve indentation in the source markup and omit it from equation text.
            output.push_str(node.text().unwrap_or_default().trim_matches(xml_space));
        } else if node.tag_name().namespace() == Some(MATHML)
            && matches!(node.tag_name().name(), "mi" | "mn" | "mo" | "ms" | "mtext")
        {
            append_token_text(node, output);
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

fn xml_space(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\n' | '\r')
}

fn append_token_text(node: Node<'_, '_>, output: &mut String) {
    let start = output.len();
    let mut gap = false;
    let text = node
        .descendants()
        .filter(Node::is_text)
        .filter_map(|child| child.text());
    for character in text.flat_map(str::chars) {
        if xml_space(character) {
            gap = output.len() != start;
        } else {
            if gap {
                output.push(' ');
                gap = false;
            }
            output.push(character);
        }
    }
}

pub(super) enum Equation {
    Native {
        linear: String,
        display: bool,
        math: Math,
    },
    SpanningTable {
        table: table::Table,
        extras: compose::Extras,
        display: bool,
    },
    Composed {
        math: Math,
        display: bool,
        extras: compose::Extras,
    },
}

impl Equation {
    pub(super) fn with_embedded_table(self, format: &CharProps) -> Result<Self, EngineError> {
        let Self::SpanningTable {
            table,
            mut extras,
            display,
        } = self
        else {
            return Ok(self);
        };
        let nodes = extras.push(compose::Extra::Table(Box::new((table, format.clone()))))?;
        Ok(Self::Composed {
            math: Math {
                nodes,
                ..Default::default()
            },
            display,
            extras,
        })
    }
}

pub(super) fn equation(
    source: &str,
    stylesheet: Option<&FormulaCss>,
    format: &mut CharProps,
    base_size: f32,
    line_width: f32,
    remaining: &mut usize,
) -> Result<Equation, EngineError> {
    if source.len() > MAX_EQUATION_BYTES {
        return Err(limited("MathML equation exceeds its byte budget"));
    }
    let xml = parse_xml(source.as_bytes())?;
    let root = xml.root_element();
    if !root.has_tag_name((MATHML, "math")) {
        return Err(invalid("MathML equation must have a math root"));
    }
    let display = root.attribute("display") == Some("block");
    let root_css = stylesheet.and_then(|css| css.nodes.get(&0));
    let viewport = stylesheet.map_or([0.0; 2], |css| css.viewport);
    let style = Presentation {
        base_size,
        display_style: display,
        ..Default::default()
    }
    .on(root, root_css, viewport)?;
    if let Some(size) = style.size {
        format.size = Some(size);
    }
    if let Some(ink) = style.ink {
        format.color = Some(TextColor::Rgb(ink));
    }
    let mut projection = Projection {
        root: style,
        root_id: root.id().get(),
        stylesheet,
        width_scope: WidthScope {
            root: root.id().get(),
            width: Some(line_width),
        },
        remaining,
        extras: compose::Extras::default(),
    };
    if let Some(TextColor::Rgb(ink)) = format.color {
        projection.extras.remember(ink);
    }
    if let Some(table) = table::project_group(&mut projection, root, 0, style)? {
        return Ok(Equation::SpanningTable {
            table,
            extras: projection.extras,
            display,
        });
    }
    let nodes = math::merge_runs(projection.elements(root, 0, style)?);
    if !projection.extras.is_empty() {
        return Ok(Equation::Composed {
            math: Math {
                nodes,
                ..Default::default()
            },
            display,
            extras: projection.extras,
        });
    }
    Ok(Equation::Native {
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
    bold: Option<bool>,
    italic: Option<bool>,
    display_style: bool,
}

impl<'a> Presentation<'a> {
    fn on(
        mut self,
        node: Node<'a, '_>,
        stylesheet: Option<&Css>,
        viewport: [f32; 2],
    ) -> Result<Self, EngineError> {
        let inherits_color = |value: &str| {
            value.eq_ignore_ascii_case("currentcolor") || matches!(value, "inherit" | "unset")
        };
        let css_color = stylesheet
            .and_then(|style| style.ink.as_deref())
            .filter(|value| inherits_color(value) || css::color(value).is_some());
        if let Some(color) = css_color.or_else(|| node.attribute("mathcolor")) {
            if !inherits_color(color) {
                let ([r, g, b], alpha) =
                    css::color(color).ok_or_else(|| invalid("invalid MathML mathcolor"))?;
                if alpha != 1.0 {
                    return Err(unsupported("translucent MathML mathcolor"));
                }
                self.ink = Some(Rgb(r, g, b));
            }
        }
        let parent = self.font_size();
        let css_size = stylesheet.and_then(|style| style.size.as_deref());
        let specified_size = css_size
            .and_then(|value| size_points(value, parent, viewport))
            .filter(|size| *size >= 0.0);
        if specified_size.is_some() {
            self.size = Some(checked_size(specified_size)?);
        } else if let Some(size) = node
            .attribute("mathsize")
            .map(str::trim)
            .filter(|size| *size != "normal" && !matches!(css_size, Some("inherit" | "unset")))
        {
            let points = match size {
                "small" => Some(parent / 1.2),
                "big" => Some(parent * 1.2),
                _ => size_points(size, parent, viewport),
            };
            self.size = Some(checked_size(points)?);
        }
        if let Some(variant) = node.attribute("mathvariant") {
            self.variant = Some(variant);
        }
        let display = match stylesheet.and_then(|style| style.display_style) {
            Some(display) => Some(display),
            None => boolean(node, "displaystyle")?,
        };
        if let Some(display) = display {
            self.display_style = display;
        }
        if let Some(stylesheet) = stylesheet {
            self.bold = stylesheet.bold.or(self.bold);
            self.italic = stylesheet.italic.or(self.italic);
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

fn size_points(value: &str, parent: f32, viewport: [f32; 2]) -> Option<f32> {
    value
        .strip_suffix('%')
        .and_then(|value| value.trim().parse::<f32>().ok())
        .map(|percent| percent / 100.0 * parent)
        .or_else(|| css::points(value, parent, viewport))
        .filter(|size| size.is_finite())
}

fn checked_size(size: Option<f32>) -> Result<f32, EngineError> {
    size.filter(|size| (1.0..=1638.0).contains(size))
        .ok_or_else(|| unsupported("MathML font size outside the native length range"))
}

struct Projection<'a, 'xml> {
    root: Presentation<'xml>,
    root_id: u32,
    stylesheet: Option<&'a FormulaCss>,
    width_scope: WidthScope,
    remaining: &'a mut usize,
    extras: compose::Extras,
}

#[derive(Clone, Copy)]
struct WidthScope {
    root: u32,
    width: Option<f32>,
}

fn shares_line_width(node: Node<'_, '_>) -> bool {
    node.tag_name().namespace() == Some(MATHML)
        && matches!(
            node.tag_name().name(),
            "math" | "mrow" | "mstyle" | "merror" | "semantics"
        )
}

impl<'xml> Projection<'_, 'xml> {
    fn with_cell_width<T>(
        &mut self,
        cell: Node<'xml, '_>,
        width: Option<f32>,
        project: impl FnOnce(&mut Self) -> Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let outer = std::mem::replace(
            &mut self.width_scope,
            WidthScope {
                root: cell.id().get(),
                width,
            },
        );
        let result = project(self);
        self.width_scope = outer;
        result
    }

    fn containing_width(&self, node: Node<'xml, '_>) -> Option<f32> {
        let width = self
            .width_scope
            .width
            .filter(|width| width.is_finite() && *width > 0.0)?;
        for ancestor in node.ancestors().skip(1).filter(Node::is_element) {
            if ancestor.id().get() == self.width_scope.root {
                return Some(width);
            }
            // Transparent groups keep the surrounding line width. Operators
            // must establish their own available width before using it.
            if !shares_line_width(ancestor) {
                break;
            }
        }
        None
    }

    fn style(
        &mut self,
        node: Node<'xml, '_>,
        parent: Presentation<'xml>,
    ) -> Result<Presentation<'xml>, EngineError> {
        let css = self
            .stylesheet
            .and_then(|sheet| sheet.nodes.get(&(node.id().get() - self.root_id)));
        let viewport = self.stylesheet.map_or([0.0; 2], |sheet| sheet.viewport);
        let style = parent.on(node, css, viewport)?;
        if let Some(ink) = style.ink {
            self.extras.remember(ink);
        }
        Ok(style)
    }

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
        inherited: Presentation<'xml>,
    ) -> Result<Arg, EngineError> {
        self.charge(depth)?;
        if node.tag_name().namespace() != Some(MATHML) {
            return Err(unsupported("foreign elements in MathML equations"));
        }
        let style = self.style(node, inherited)?;
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
            "mspace" => {
                validate_space(node)?;
                if compose::selected(node) {
                    let viewport = self.stylesheet.map_or([0.0; 2], |sheet| sheet.viewport);
                    return self.extras.push(compose::space(node, style, viewport)?);
                }
                return space(node, style);
            }
            "mpadded" => {
                let body = self.elements(node, depth, style)?;
                if compose::selected(node) {
                    let viewport = self.stylesheet.map_or([0.0; 2], |sheet| sheet.viewport);
                    return self
                        .extras
                        .push(compose::padding(node, body, style, viewport)?);
                }
                return padded(node, body);
            }
            "mfrac" => {
                let [num, den] = self.arguments(node, depth, style)?;
                let kind = if boolean(node, "bevelled")? == Some(true) {
                    FracKind::Skewed
                } else {
                    let viewport = self
                        .stylesheet
                        .map_or([super::layout::REFLOW_WIDTH, 792.0], |sheet| sheet.viewport);
                    let fraction = fraction::Style::read(node, style.font_size(), viewport);
                    let Some(kind) = fraction.native() else {
                        return self.extras.push(compose::Extra::Fraction(Box::new(
                            fraction::Fraction::new(fraction, style, num, den),
                        )));
                    };
                    kind
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
            "mtable" if table::has_authored_geometry(node) || table::has_spans(node) => {
                let grid = table::project(self, node, depth, style, inherited.font_size())?;
                return self.extras.push(compose::Extra::Table(Box::new((
                    grid,
                    compose::format(style),
                ))));
            }
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
    let mut text = String::new();
    append_token_text(node, &mut text);
    let mut run = MRun::new(text);
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
    let bold = style
        .bold
        .unwrap_or(matches!(variant, MSty::Bold | MSty::BoldItalic));
    let italic = style
        .italic
        .unwrap_or(matches!(variant, MSty::Italic | MSty::BoldItalic));
    run.sty = Some(match (bold, italic) {
        (true, true) => MSty::BoldItalic,
        (true, false) => MSty::Bold,
        (false, true) => MSty::Italic,
        (false, false) => MSty::Plain,
    });
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
            let row_style = self.style(row, style)?;
            let mut cells = Vec::new();
            for cell in row.children().filter(Node::is_element) {
                if !cell.has_tag_name((MATHML, "mtd")) {
                    return Err(invalid("unexpected MathML table child"));
                }
                if ["rowspan", "columnspan"].iter().any(|name| {
                    cell.attribute(*name)
                        .is_some_and(|span| span.trim().parse::<usize>() != Ok(1))
                }) {
                    return Err(unsupported("MathML table spans"));
                }
                if cells.len() == MAX_MATRIX_SIZE {
                    return Err(limited("MathML matrix exceeds its layout budget"));
                }
                self.charge(depth + 2)?;
                let cell_style = self.style(cell, row_style)?;
                cells.push(self.elements(cell, depth + 2, cell_style)?);
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

fn validate_space(node: Node<'_, '_>) -> Result<(), EngineError> {
    if node
        .children()
        .any(|child| child.is_element() || child.text().is_some_and(|text| !text.trim().is_empty()))
    {
        return Err(invalid("MathML mspace must be empty"));
    }
    if node
        .attribute("linebreak")
        .is_some_and(|value| value != "auto")
    {
        return Err(unsupported("MathML mspace linebreak"));
    }
    Ok(())
}

fn space(node: Node<'_, '_>, style: Presentation<'_>) -> Result<Arg, EngineError> {
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
