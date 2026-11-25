//! Translate the EPUB CSS cascade into typography and box layout inputs.
use std::{collections::HashMap, sync::Arc};

use roxmltree::{Node, NodeId};
use taffy::{
    geometry::{Line, MinMax, Point, Rect, Size},
    prelude::*,
    style::{BoxSizing, ExpandedDimension, GridAutoFlow, GridPlacement, Overflow},
};
use wordcraft_doc::props::{Align, CharProps, LineSpacing, Rgb, TextColor, Underline};

use super::{
    charge,
    css::{self, Cascade, Properties},
    limited, math, XHTML,
};
use crate::EngineError;

pub(super) type Styles = HashMap<NodeId, Styled>;
pub(super) type Paint = (Rgb, f32);
pub(super) type InlineBounds = MinMax<Option<InlineBound>, Option<InlineBound>>;

#[derive(Clone, Copy)]
pub(super) struct IntrinsicBounds {
    pub inline: InlineBounds,
    pub block: MinMax<bool, bool>,
}

impl IntrinsicBounds {
    pub const NONE: Self = Self {
        inline: MinMax {
            min: None,
            max: None,
        },
        block: MinMax {
            min: false,
            max: false,
        },
    };

    pub fn has_inline(self) -> bool {
        self.inline.min.is_some() || self.inline.max.is_some()
    }

    pub fn has_block(self) -> bool {
        self.block.min || self.block.max
    }
}

#[derive(Clone, Copy)]
pub(super) enum InlineBound {
    Minimum,
    Maximum,
    FitContent(LengthPercentage),
}

#[derive(Default)]
pub(super) struct ChapterStyles {
    pub boxes: Styles,
    pub equations: HashMap<NodeId, math::Css>,
    pub calculations: css::CalculationTable,
}

pub(super) struct Styled {
    pub box_style: Style,
    pub intrinsic_bounds: IntrinsicBounds,
    pub text: Typography,
    pub inline: bool,
    pub bidi: super::bidi::Mode,
    pub hidden: bool,
    pub background: Option<Paint>,
    pub border: [Option<Paint>; 4],
    pub z_index: i32,
}

#[derive(Clone, Default)]
pub(super) struct Typography {
    pub chars: CharProps,
    pub families: Arc<[css::Family]>,
    pub axes: Arc<[css::Axis]>,
    pub font_weight: Option<f32>,
    pub font_width: Option<f32>,
    pub oblique: Option<f32>,
    pub optical_sizing_disabled: bool,
    pub align: Option<Align>,
    pub line_height: Option<LineSpacing>,
    pub preformatted: bool,
    pub invisible: bool,
}

impl ChapterStyles {
    pub(super) fn compute(
        root: Node<'_, '_>,
        cascade: &mut Cascade,
        viewport: [f32; 2],
    ) -> Result<Self, EngineError> {
        let mut styles = Self::default();
        let mut formula_end = 0;
        let mut annotation_end = 0;
        let mut equation_bytes = 0;
        let mut font_values = HashMap::new();
        let mut axis_values = HashMap::new();
        let mut box_context = Vec::<(usize, css::logical::Inherited)>::new();
        for node in root.descendants().filter(Node::is_element) {
            let range = node.range();
            if range.start < annotation_end {
                continue;
            }
            let tag = node.tag_name();
            let is_math = tag.namespace() == Some(math::MATHML);
            let in_formula = range.start < formula_end;
            if in_formula && is_math && matches!(tag.name(), "annotation" | "annotation-xml") {
                annotation_end = range.end;
                continue;
            }
            if in_formula && !is_math {
                continue;
            }
            while box_context
                .last()
                .is_some_and(|(end, _)| *end <= range.start)
            {
                box_context.pop();
            }
            let (mut values, flow, inherited_calculations) =
                cascade.properties(node, box_context.last().map(|(_, parent)| parent))?;
            if is_math {
                if let Some(css) = math::Css::from_properties(&values) {
                    charge(
                        &mut equation_bytes,
                        css.storage_bytes(),
                        64 * 1024 * 1024,
                        "EPUB equation styles exceed their byte budget",
                    )?;
                    styles.equations.insert(node.id(), css);
                }
                if in_formula {
                    continue;
                }
                if tag.name() == "math" {
                    formula_end = range.end;
                }
            }
            if styles.boxes.len() == 8_000 {
                return Err(limited("EPUB styled chapter exceeds 8000 box elements"));
            }
            let parent = node
                .parent_element()
                .and_then(|parent| styles.boxes.get(&parent.id()));
            let mut text = parent.map(|parent| parent.text.clone()).unwrap_or_default();
            let font = values.get("font-family").and_then(|source| {
                font_values
                    .entry(source.clone())
                    .or_insert_with(|| css::parse_font_family(source))
                    .as_ref()
            });
            if let Some(source) = values.get("font-variation-settings") {
                if let Some(css::AxisValue::Settings(axes)) = axis_values
                    .entry(source.clone())
                    .or_insert_with(|| css::parse_axes(source))
                {
                    text.axes = axes.clone();
                }
            }
            if let Some(enabled) = values
                .get("font-optical-sizing")
                .and_then(|value| css::optical_sizing(value))
            {
                text.optical_sizing_disabled = !enabled;
            }
            text.apply(node, &values, viewport, font);
            text.chars.rtl = flow.rtl;
            let font = text.chars.size.unwrap_or(12.0);
            let calculations = css::logical::resolve_calculated_geometry(
                &mut values,
                flow,
                font,
                viewport,
                &mut styles.calculations,
                inherited_calculations,
            )?;
            let tag = node.tag_name().name();
            let position = match value(&values, "position") {
                "fixed" => Position::Fixed,
                "absolute" => Position::Absolute,
                "relative" | "sticky" => Position::Relative,
                _ => Position::Static,
            };
            let display = value(&values, "display");
            let hidden = display == "none"
                || node.has_attribute("hidden")
                || matches!(
                    tag,
                    "head" | "style" | "link" | "meta" | "script" | "template" | "title"
                );
            let inline = !position.is_out_of_flow()
                && match display {
                    "inline" | "inline-block" | "inline-flex" | "inline-grid" => true,
                    "" => {
                        node.tag_name().namespace().is_some_and(|ns| ns != XHTML) || !block_tag(tag)
                    }
                    _ => false,
                };
            let bidi = super::bidi::Mode::resolve(
                node,
                values.get("unicode-bidi").map(String::as_str),
                parent.map_or(super::bidi::Mode::Normal, |parent| parent.bidi),
            );
            let auto = |key| {
                calculations.get(key).copied().map_or_else(
                    || dimension::<LengthPercentageAuto>(value(&values, key), font, viewport),
                    LengthPercentageAuto::calc,
                )
            };
            let table_track = tag_has_mixed_size_as_auto(node);
            let is_auto = |key| {
                table_track
                    && calculations
                        .get(key)
                        .is_some_and(|handle| styles.calculations.has_fixed_part(*handle))
            };
            let auto_width = is_auto("width");
            let auto_height = is_auto("height");
            let dim = |key| {
                if (key == "width" && auto_width) || (key == "height" && auto_height) {
                    return Dimension::auto();
                }
                let value = value(&values, key);
                let handle = calculations.get(key).copied();
                if matches!(key, "width" | "height") {
                    if let Some(dimension) = intrinsic_dimension(value, handle) {
                        return dimension;
                    }
                }
                handle.map_or_else(
                    || dimension::<Dimension>(value, font, viewport),
                    Dimension::calc,
                )
            };
            let len = |key| {
                calculations.get(key).copied().map_or_else(
                    || length(value(&values, key), font, viewport),
                    LengthPercentage::calc,
                )
            };
            let overflow = |key| match values
                .get(key)
                .map_or_else(|| value(&values, "overflow"), String::as_str)
            {
                "hidden" | "clip" => Overflow::Hidden,
                "scroll" | "auto" => Overflow::Scroll,
                _ => Overflow::Visible,
            };
            let border_size = |side| {
                if matches!(
                    value(&values, &format!("border-{side}-style")),
                    "" | "none" | "hidden"
                ) {
                    return LengthPercentage::length(0.0);
                }
                let key = format!("border-{side}-width");
                match value(&values, &key) {
                    "thin" => LengthPercentage::length(0.75),
                    "" | "medium" => LengthPercentage::length(2.25),
                    "thick" => LengthPercentage::length(3.75),
                    value => length(value, font, viewport),
                }
            };
            let spacing = if matches!(
                tag,
                "p" | "blockquote" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            ) {
                font * 0.55
            } else {
                0.0
            };
            let ratio = value(&values, "aspect-ratio")
                .split_once('/')
                .and_then(|(a, b)| {
                    Some(a.trim().parse::<f32>().ok()? / b.trim().parse::<f32>().ok()?)
                })
                .filter(|n| n.is_finite() && *n > 0.0);
            let mut tracks = |key, axis| {
                let source = value(&values, key);
                if source.eq_ignore_ascii_case("inherit") {
                    return Ok(parent
                        .map(|p| {
                            let style = &p.box_style;
                            let (tracks, names) = match axis {
                                taffy::AbsoluteAxis::Horizontal => (
                                    &style.grid_template_columns,
                                    &style.grid_template_column_names,
                                ),
                                taffy::AbsoluteAxis::Vertical => {
                                    (&style.grid_template_rows, &style.grid_template_row_names)
                                }
                            };
                            css::grid::Template {
                                tracks: tracks.clone(),
                                names: names.clone(),
                            }
                        })
                        .unwrap_or_default());
                }
                css::grid::resolve(source, font, viewport, &mut styles.calculations)
            };
            let columns = tracks("grid-template-columns", taffy::AbsoluteAxis::Horizontal)?;
            let rows = tracks("grid-template-rows", taffy::AbsoluteAxis::Vertical)?;
            let mut implicit_tracks = |key, axis| {
                let source = value(&values, key);
                if source.eq_ignore_ascii_case("inherit") {
                    return Ok(parent
                        .map(|p| match axis {
                            taffy::AbsoluteAxis::Horizontal => {
                                p.box_style.grid_auto_columns.clone()
                            }
                            taffy::AbsoluteAxis::Vertical => p.box_style.grid_auto_rows.clone(),
                        })
                        .unwrap_or_default());
                }
                css::grid::resolve_auto_tracks(source, font, viewport, &mut styles.calculations)
            };
            let auto_columns =
                implicit_tracks("grid-auto-columns", taffy::AbsoluteAxis::Horizontal)?;
            let auto_rows = implicit_tracks("grid-auto-rows", taffy::AbsoluteAxis::Vertical)?;
            let placement = |key, edge: fn(&Style) -> &GridPlacement<String>| {
                css::grid::resolve_placement(value(&values, key), || {
                    parent.map_or(GridPlacement::Auto, |p| edge(&p.box_style).clone())
                })
            };
            let intrinsic_bound = |key| {
                let source = value(&values, key);
                let dimension = intrinsic_dimension(source, calculations.get(key).copied());
                match dimension.map(Dimension::expand) {
                    Some(ExpandedDimension::MinContent) => Some(InlineBound::Minimum),
                    Some(ExpandedDimension::MaxContent) => Some(InlineBound::Maximum),
                    Some(ExpandedDimension::FitContentPx(limit)) => {
                        Some(InlineBound::FitContent(LengthPercentage::length(limit)))
                    }
                    Some(ExpandedDimension::FitContentPercent(fraction)) => {
                        Some(InlineBound::FitContent(LengthPercentage::percent(fraction)))
                    }
                    Some(ExpandedDimension::FitContentCalc(handle)) => {
                        Some(InlineBound::FitContent(LengthPercentage::calc(handle)))
                    }
                    _ => None,
                }
            };
            let block_bound = |key| {
                let value = value(&values, key);
                value.eq_ignore_ascii_case("min-content")
                    || value.eq_ignore_ascii_case("max-content")
            };
            let intrinsic_bounds = IntrinsicBounds {
                inline: InlineBounds {
                    min: intrinsic_bound("min-width"),
                    max: intrinsic_bound("max-width"),
                },
                block: MinMax {
                    min: block_bound("min-height"),
                    max: block_bound("max-height"),
                },
            };
            let box_style = Style {
                display: match display {
                    "flex" | "inline-flex" => Display::Flex,
                    "grid" | "inline-grid" => Display::Grid,
                    "flow-root" => Display::FlowRoot,
                    _ if hidden => Display::None,
                    _ => Display::Block,
                },
                direction: if text.chars.rtl == Some(true) {
                    taffy::style::Direction::Rtl
                } else {
                    taffy::style::Direction::Ltr
                },
                position,
                box_sizing: if value(&values, "box-sizing") == "border-box" {
                    BoxSizing::BorderBox
                } else {
                    BoxSizing::ContentBox
                },
                size: Size {
                    width: dim("width"),
                    height: dim("height"),
                },
                min_size: Size {
                    width: auto("min-width"),
                    height: auto("min-height"),
                },
                max_size: Size {
                    width: auto("max-width"),
                    height: auto("max-height"),
                },
                aspect_ratio: ratio,
                inset: Rect {
                    top: auto("top"),
                    right: auto("right"),
                    bottom: auto("bottom"),
                    left: auto("left"),
                },
                margin: Rect {
                    top: values
                        .get("margin-top")
                        .map_or(LengthPercentageAuto::length(spacing), |_| {
                            auto("margin-top")
                        }),
                    bottom: values
                        .get("margin-bottom")
                        .map_or(LengthPercentageAuto::length(spacing), |_| {
                            auto("margin-bottom")
                        }),
                    right: values
                        .get("margin-right")
                        .map_or(LengthPercentageAuto::length(0.0), |_| auto("margin-right")),
                    left: values
                        .get("margin-left")
                        .map_or(LengthPercentageAuto::length(0.0), |_| auto("margin-left")),
                },
                padding: Rect {
                    top: len("padding-top"),
                    right: len("padding-right"),
                    bottom: len("padding-bottom"),
                    left: len("padding-left"),
                },
                border: Rect {
                    top: border_size("top"),
                    right: border_size("right"),
                    bottom: border_size("bottom"),
                    left: border_size("left"),
                },
                overflow: Point {
                    x: overflow("overflow-x"),
                    y: overflow("overflow-y"),
                },
                gap: Size {
                    height: len("row-gap"),
                    width: len("column-gap"),
                },
                flex_direction: match value(&values, "flex-direction") {
                    "column" => FlexDirection::Column,
                    "column-reverse" => FlexDirection::ColumnReverse,
                    "row-reverse" => FlexDirection::RowReverse,
                    _ => FlexDirection::Row,
                },
                flex_wrap: match value(&values, "flex-wrap") {
                    "wrap" => FlexWrap::Wrap,
                    "wrap-reverse" => FlexWrap::WrapReverse,
                    _ => FlexWrap::NoWrap,
                },
                flex_grow: number(value(&values, "flex-grow"), 0.0, 100.0).unwrap_or(0.0),
                flex_shrink: number(value(&values, "flex-shrink"), 0.0, 100.0).unwrap_or(1.0),
                flex_basis: dim("flex-basis"),
                align_items: align(value(&values, "align-items")).unwrap_or(AlignItems::STRETCH),
                align_self: align(value(&values, "align-self")),
                justify_items: align(value(&values, "justify-items"))
                    .unwrap_or(AlignItems::STRETCH),
                justify_self: align(value(&values, "justify-self")),
                justify_content: match value(&values, "justify-content") {
                    "center" => JustifyContent::CENTER,
                    "end" | "flex-end" => JustifyContent::END,
                    "space-between" => JustifyContent::SPACE_BETWEEN,
                    "space-around" => JustifyContent::SPACE_AROUND,
                    "space-evenly" => JustifyContent::SPACE_EVENLY,
                    _ => JustifyContent::START,
                },
                grid_template_columns: columns.tracks,
                grid_template_column_names: columns.names,
                grid_template_rows: rows.tracks,
                grid_template_row_names: rows.names,
                grid_template_areas: css::grid::resolve_areas(
                    value(&values, "grid-template-areas"),
                    || parent.and_then(|parent| parent.box_style.grid_template_areas.clone()),
                )?,
                grid_row: Line {
                    start: placement("grid-row-start", |style| &style.grid_row.start)?,
                    end: placement("grid-row-end", |style| &style.grid_row.end)?,
                },
                grid_column: Line {
                    start: placement("grid-column-start", |style| &style.grid_column.start)?,
                    end: placement("grid-column-end", |style| &style.grid_column.end)?,
                },
                grid_auto_columns: auto_columns,
                grid_auto_rows: auto_rows,
                grid_auto_flow: css::grid::resolve_flow(value(&values, "grid-auto-flow"), || {
                    parent.map_or(GridAutoFlow::Row, |p| p.box_style.grid_auto_flow)
                })?,
                ..Default::default()
            };
            box_context.push((
                range.end,
                css::logical::Inherited::new(flow, &values, font, viewport, spacing, calculations),
            ));
            let background = css::color(value(&values, "background-color")).map(ink);
            let border = ["top", "right", "bottom", "left"].map(|side| {
                css::color(value(&values, &format!("border-{side}-color")))
                    .map(ink)
                    .or_else(|| {
                        text.chars.color.and_then(|color| match color {
                            TextColor::Rgb(rgb) => Some((rgb, 1.0)),
                            _ => None,
                        })
                    })
                    .or(Some((Rgb::BLACK, 1.0)))
            });
            let z_index = value(&values, "z-index").parse().unwrap_or(0);
            styles.boxes.insert(
                node.id(),
                Styled {
                    box_style,
                    intrinsic_bounds,
                    text,
                    inline,
                    bidi,
                    hidden,
                    background,
                    border,
                    z_index,
                },
            );
        }
        Ok(styles)
    }
}

impl Typography {
    fn apply(
        &mut self,
        node: Node<'_, '_>,
        values: &Properties,
        viewport: [f32; 2],
        font: Option<&css::FontFamilyValue>,
    ) {
        let tag = node.tag_name().name();
        let size = self.chars.size.unwrap_or(12.0);
        if let Some(new_size) = css::points(value(values, "font-size"), size, viewport)
            .or_else(|| percent(value(values, "font-size")).map(|n| n * size))
        {
            self.chars.size = Some(new_size.clamp(0.75, 480.0));
        } else if value(values, "font-size").is_empty() {
            let heading_size = match tag {
                "h1" => 2.0,
                "h2" => 1.5,
                "h3" => 1.17,
                "h4" => 1.0,
                "h5" => 0.83,
                "h6" => 0.67,
                _ => 0.0,
            };
            if heading_size > 0.0 {
                self.chars.size = Some(size * heading_size);
            }
        }
        match font {
            Some(css::FontFamilyValue::Families(families)) => {
                let family = &families[0];
                let name = if family.generic {
                    match family.name.as_str() {
                        "serif" | "ui-serif" => "Times New Roman",
                        "sans-serif" | "system-ui" | "ui-sans-serif" => "Arial",
                        "monospace" | "ui-monospace" => "Courier New",
                        _ => &family.name,
                    }
                } else {
                    &family.name
                };
                self.chars.font = Some(name.to_owned());
                self.families = families.clone();
            }
            Some(css::FontFamilyValue::Initial) => {
                self.chars.font = None;
                self.families = Arc::default();
            }
            Some(css::FontFamilyValue::Inherit) => {}
            None if matches!(tag, "pre" | "code" | "kbd" | "samp") => {
                self.chars.font = Some("Courier New".to_owned());
                self.families = Arc::default();
            }
            None => {}
        }
        let request = value(values, "font-weight");
        let weight = if request.is_empty()
            && matches!(
                tag,
                "b" | "strong" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th"
            ) {
            Some(700.0)
        } else {
            css::relative_font_weight(request, self.font_weight.unwrap_or(400.0))
        };
        if let Some(weight) = weight {
            self.font_weight = Some(weight);
            self.chars.bold = Some(weight >= 600.0);
        }
        let width = value(values, "font-stretch");
        if width.eq_ignore_ascii_case("initial") {
            self.font_width = Some(100.0);
        } else if let Some(width) = css::font_width(width) {
            self.font_width = Some(width);
        }
        use parley::FontStyle;
        let slope = value(values, "font-style");
        let style = match slope.to_ascii_lowercase().as_str() {
            "initial" => Some(FontStyle::Normal),
            "" | "revert" | "revert-layer" if matches!(tag, "i" | "em" | "cite") => {
                Some(FontStyle::Italic)
            }
            "" | "inherit" | "unset" | "revert" | "revert-layer" => None,
            _ => css::font_style(slope),
        };
        if let Some(style) = style {
            self.chars.italic = Some(style != FontStyle::Normal);
            self.oblique = match style {
                FontStyle::Oblique(angle) => angle,
                _ => None,
            };
        }
        if let Some(color) = css::color(value(values, "color")) {
            self.chars.color = Some(TextColor::Rgb(ink(color).0));
            self.invisible = color.1 == 0.0;
        }
        match value(values, "visibility") {
            "hidden" | "collapse" => self.invisible = true,
            "visible" => self.invisible = false,
            _ => {}
        }
        if let Some(color) =
            css::color(value(values, "background-color")).filter(|color| color.1 == 1.0)
        {
            if !block_tag(tag) {
                self.chars.shading = Some(ink(color).0);
            }
        }
        let decoration = values
            .get("text-decoration-line")
            .or_else(|| values.get("text-decoration"));
        if let Some(decoration) = decoration {
            self.chars.underline = if decoration
                .split_ascii_whitespace()
                .any(|word| word == "underline")
            {
                Some(Underline::Single)
            } else {
                Some(Underline::None)
            };
            self.chars.strike = Some(
                decoration
                    .split_ascii_whitespace()
                    .any(|word| word == "line-through"),
            );
        } else if tag == "u" || tag == "a" && node.has_attribute("href") {
            self.chars.underline = Some(Underline::Single);
        } else if matches!(tag, "s" | "del") {
            self.chars.strike = Some(true);
        }
        self.align = match value(values, "text-align") {
            "left" | "start" => Some(Align::Left),
            "center" => Some(Align::Center),
            "right" | "end" => Some(Align::Right),
            "justify" => Some(Align::Justify),
            _ => self.align,
        };
        if let Some(line_height) = values.get("line-height") {
            let size = self.chars.size.unwrap_or(12.0);
            let points = css::points(line_height, size, viewport)
                .or_else(|| percent(line_height).map(|n| n * size));
            self.line_height = points
                .map(|points| LineSpacing::Exactly(points.max(0.75)))
                .or_else(|| number(line_height, 0.0, 20.0).map(LineSpacing::Multiple));
        }
        if let Some(spacing) = css::points(value(values, "letter-spacing"), size, viewport) {
            self.chars.spacing = Some(spacing);
        }
        self.preformatted = match value(values, "white-space") {
            "pre" | "pre-wrap" | "break-spaces" => true,
            "normal" | "nowrap" | "pre-line" => false,
            _ => tag == "pre" || self.preformatted,
        };
    }
}

pub(super) fn block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "body"
            | "html"
            | "p"
            | "div"
            | "main"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "nav"
            | "aside"
            | "figure"
            | "figcaption"
            | "address"
            | "pre"
            | "blockquote"
            | "ol"
            | "ul"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "table"
            | "tr"
            | "td"
            | "th"
            | "hr"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

// A mixed percentage and nonzero length computes normally, but HTML table
// tracks use auto for it in the width and height properties.
fn tag_has_mixed_size_as_auto(node: Node<'_, '_>) -> bool {
    node.tag_name().namespace() == Some(XHTML)
        && matches!(
            node.tag_name().name(),
            "col" | "colgroup" | "tr" | "thead" | "tbody" | "tfoot" | "td" | "th"
        )
}

fn value<'a>(values: &'a Properties, key: &str) -> &'a str {
    values.get(key).map_or("", String::as_str)
}

fn percent(value: &str) -> Option<f32> {
    let value = value.strip_suffix('%')?.trim().parse::<f32>().ok()? / 100.0;
    (value.is_finite() && value.abs() <= 100.0).then_some(value)
}

fn number(value: &str, min: f32, max: f32) -> Option<f32> {
    value
        .parse::<f32>()
        .ok()
        .filter(|value| *value >= min && *value <= max)
}

fn length(value: &str, font: f32, viewport: [f32; 2]) -> LengthPercentage {
    percent(value)
        .map(LengthPercentage::percent)
        .unwrap_or_else(|| {
            LengthPercentage::length(css::points(value, font, viewport).unwrap_or(0.0).max(0.0))
        })
}

fn dimension<T>(value: &str, font: f32, viewport: [f32; 2]) -> T
where
    T: taffy::style_helpers::TaffyAuto
        + taffy::style_helpers::FromLength
        + taffy::style_helpers::FromPercent,
{
    percent(value)
        .map(T::from_percent)
        .or_else(|| css::points(value, font, viewport).map(T::from_length))
        .unwrap_or(T::AUTO)
}

fn intrinsic_dimension(value: &str, handle: Option<taffy::CalcHandle>) -> Option<Dimension> {
    Some(match value.to_ascii_lowercase().as_str() {
        "min-content" => Dimension::min_content(),
        "max-content" => Dimension::max_content(),
        "stretch" => Dimension::stretch(),
        "fit-content" => handle.map_or_else(Dimension::fit_content, Dimension::fit_content_calc),
        _ => {
            let limit = css::logical::fit_content_argument(value)?;
            percent(limit)
                .map(Dimension::fit_content_percent)
                .or_else(|| css::points(limit, 12.0, [0.0; 2]).map(Dimension::fit_content_px))?
        }
    })
}

fn align(value: &str) -> Option<AlignItems> {
    match value {
        "center" => Some(AlignItems::CENTER),
        "start" | "flex-start" => Some(AlignItems::START),
        "end" | "flex-end" => Some(AlignItems::END),
        "stretch" => Some(AlignItems::STRETCH),
        "baseline" => Some(AlignItems::BASELINE),
        _ => None,
    }
}

fn ink(([r, g, b], alpha): ([u8; 3], f32)) -> Paint {
    (Rgb(r, g, b), alpha)
}
