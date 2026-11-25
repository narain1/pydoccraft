//! Bounded author styles for the projected WebVTT cue tree.
use std::sync::Arc;

use raffia::{ast::*, Spanned};

use super::{FontStyle, TextStyle, MAX_STYLE_LABEL};
use crate::{
    epub::{charge, css, invalid, limited, unsupported},
    EngineError,
};

pub(super) const MAX_STYLESHEET_BYTES: usize = 64 * 1024;
const MAX_STYLE_BLOCKS: usize = 32;
const MAX_SELECTORS: usize = 256;
const MAX_SELECTOR_TERMS: usize = 16;
const MAX_DECLARATIONS: usize = 64;
const MAX_MATCH_WORK: usize = 4_000_000;

type Rgba = [u8; 4];
type Specificity = [u16; 3];
type Rank = (bool, Specificity);

#[derive(Default)]
pub(super) struct Cascade {
    rules: Vec<Rule>,
    region_rules: Vec<Rule>,
    bytes: usize,
    blocks: usize,
    selectors: usize,
    work: usize,
    timed: PlaybackState,
}

#[derive(Clone, Copy, Default)]
pub(super) struct PlaybackState {
    pub past: bool,
    pub future: bool,
}

#[derive(Default)]
pub(super) struct Target<'a> {
    pub tag: Option<&'a str>,
    pub id: Option<&'a str>,
    pub classes: &'a [String],
    pub voice: Option<&'a str>,
    pub language: Option<&'a str>,
    pub language_attribute: Option<&'a str>,
    pub playback: PlaybackState,
}

#[derive(Clone, Default)]
pub(crate) struct RegionPaint {
    pub style: TextStyle,
    pub opacity: Option<f64>,
    pub visibility: Option<&'static str>,
}

struct Rule {
    selector: Selector,
    declarations: Arc<[Declaration]>,
}

struct Selector {
    tests: Vec<Test>,
    specificity: Specificity,
}

enum Test {
    Universal,
    Root,
    Past,
    Future,
    Tag(String),
    Id(String),
    Class(String),
    Language(Vec<String>),
    Attribute {
        kind: AttributeKind,
        value: AttributeValue,
        insensitive: bool,
    },
}

enum AttributeKind {
    Voice,
    Language,
}

enum AttributeValue {
    Present,
    Exact(String),
    Prefix(String),
}

#[derive(Clone, Copy)]
enum Color {
    Fixed(Rgba),
    Current,
}

impl Color {
    fn get(self, current: Option<Rgba>) -> Option<Rgba> {
        match self {
            Self::Fixed(value) => Some(value),
            Self::Current => Some(current.unwrap_or([255; 4])),
        }
    }
}

#[derive(Clone, Copy)]
enum Value {
    Color(Color),
    Background(Color),
    Bold(bool),
    Slant(FontStyle),
    Decoration([bool; 3]),
    Opacity(f64),
    Visibility(&'static str),
}

impl Value {
    fn region_only(self) -> bool {
        matches!(self, Self::Opacity(_) | Self::Visibility(_))
    }

    fn applies_to_timed_selector(self) -> bool {
        !(self.region_only() || matches!(self, Self::Bold(_) | Self::Slant(_)))
    }

    fn slot(self) -> usize {
        match self {
            Self::Color(_) => 0,
            Self::Background(_) => 1,
            Self::Bold(_) => 2,
            Self::Slant(_) => 3,
            Self::Decoration(_) => 4,
            Self::Opacity(_) => 5,
            Self::Visibility(_) => 6,
        }
    }
}

pub(super) struct Paint([Option<(Rank, Value)>; 7]);

#[derive(Clone, Copy)]
struct Declaration {
    value: Value,
    important: bool,
}

impl Cascade {
    pub(super) fn sheet(&mut self, source: &str) -> Result<(), EngineError> {
        charge(
            &mut self.blocks,
            1,
            MAX_STYLE_BLOCKS,
            "WebVTT exceeds 32 STYLE blocks",
        )?;
        charge(
            &mut self.bytes,
            source.len(),
            MAX_STYLESHEET_BYTES,
            "WebVTT stylesheets exceed 64 KiB",
        )?;
        let source = css::checked_css(source)?;
        let sheet = css::value_parser(&source)
            .parse::<Stylesheet>()
            .map_err(css::css_error)?;
        css::css_references(&sheet.statements, &mut |_, _| {
            Err(unsupported("WebVTT stylesheets cannot reference resources"))
        })?;
        for statement in sheet.statements {
            let Statement::QualifiedRule(rule) = statement else {
                return Err(unsupported(
                    "WebVTT stylesheet at-rules and non-cue statements",
                ));
            };
            charge(
                &mut self.selectors,
                rule.selector.selectors.len(),
                MAX_SELECTORS,
                "WebVTT stylesheets exceed their selector count budget",
            )?;
            let declarations: Arc<[_]> = declarations(&rule.block, &source)?.into();
            let mut cue_declarations: Option<Arc<[Declaration]>> = None;
            let mut timed_declarations: Option<Arc<[Declaration]>> = None;
            for selector in &rule.selector.selectors {
                let Some((selector, region)) = Selector::from_css(selector)? else {
                    continue;
                };
                if region {
                    if !declarations.is_empty() {
                        self.region_rules.push(Rule {
                            selector,
                            declarations: declarations.clone(),
                        });
                    }
                    continue;
                }
                let past = selector.tests.iter().any(|test| matches!(test, Test::Past));
                let future = selector
                    .tests
                    .iter()
                    .any(|test| matches!(test, Test::Future));
                let declarations = if past || future {
                    timed_declarations.get_or_insert_with(|| {
                        declarations
                            .iter()
                            .copied()
                            .filter(|declaration| declaration.value.applies_to_timed_selector())
                            .collect()
                    })
                } else {
                    cue_declarations.get_or_insert_with(|| {
                        if declarations
                            .iter()
                            .all(|declaration| !declaration.value.region_only())
                        {
                            return declarations.clone();
                        }
                        declarations
                            .iter()
                            .copied()
                            .filter(|declaration| !declaration.value.region_only())
                            .collect()
                    })
                };
                if !declarations.is_empty() {
                    self.timed.past |= past;
                    self.timed.future |= future;
                    self.rules.push(Rule {
                        selector,
                        declarations: declarations.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn timed(&self) -> PlaybackState {
        self.timed
    }

    pub(super) fn paint(&mut self, target: Target<'_>) -> Result<Paint, EngineError> {
        paint_rules(&self.rules, target, &mut self.work)
    }

    pub(super) fn paint_region(&mut self, id: &str) -> Result<RegionPaint, EngineError> {
        let mut paint = paint_rules(
            &self.region_rules,
            Target {
                id: Some(id),
                ..Default::default()
            },
            &mut self.work,
        )?;
        let opacity = match paint.0[5].take() {
            Some((_, Value::Opacity(value))) => Some(value),
            _ => None,
        };
        let visibility = match paint.0[6].take() {
            Some((_, Value::Visibility(value))) => Some(value),
            _ => None,
        };
        let mut style = TextStyle::default();
        style.background_color = paint.apply(&TextStyle::default(), &mut style);
        Ok(RegionPaint {
            style,
            opacity,
            visibility,
        })
    }
}

fn paint_rules(rules: &[Rule], target: Target<'_>, work: &mut usize) -> Result<Paint, EngineError> {
    let mut winners: [Option<(Rank, Value)>; 7] = [None; 7];
    for rule in rules {
        charge(
            work,
            1,
            MAX_MATCH_WORK,
            "WebVTT stylesheet selector work exceeds its budget",
        )?;
        if !rule.selector.matches(&target, work)? {
            continue;
        }
        charge(
            work,
            rule.declarations.len(),
            MAX_MATCH_WORK,
            "WebVTT stylesheet cascade exceeds its work budget",
        )?;
        for declaration in rule.declarations.iter() {
            let rank = (declaration.important, rule.selector.specificity);
            let slot = &mut winners[declaration.value.slot()];
            if slot.is_none_or(|(previous, _)| previous <= rank) {
                *slot = Some((rank, declaration.value));
            }
        }
    }
    Ok(Paint(winners))
}

impl Paint {
    pub(super) fn apply(self, inherited: &TextStyle, style: &mut TextStyle) -> Option<Rgba> {
        let mut background = None;
        for (_, value) in self.0.into_iter().flatten() {
            match value {
                Value::Color(color) => style.color = color.get(inherited.color),
                Value::Background(color) => background = color.get(style.color),
                Value::Bold(bold) => style.bold = bold,
                Value::Slant(slant) => style.font_style = slant,
                Value::Decoration([underline, line_through, overline]) => {
                    // Inline decoration is drawn through descendants even when
                    // their own decoration declaration is 'none'.
                    style.underline = inherited.underline || underline;
                    style.line_through = inherited.line_through || line_through;
                    style.overline = inherited.overline || overline;
                }
                Value::Opacity(_) | Value::Visibility(_) => {}
            }
        }
        background
    }
}

impl Selector {
    fn from_css(selector: &ComplexSelector<'_>) -> Result<Option<(Self, bool)>, EngineError> {
        let [ComplexSelectorChild::CompoundSelector(compound)] = selector.children.as_slice()
        else {
            return Ok(None);
        };
        let (cue, video_qualified) = match compound.children.as_slice() {
            [SimpleSelector::PseudoElement(cue)] => (cue, false),
            [SimpleSelector::Type(TypeSelector::TagName(tag)), SimpleSelector::PseudoElement(cue)]
                if tag.name.prefix.is_none() && css::ident(&tag.name.name) == Some("video") =>
            {
                (cue, true)
            }
            _ => return Ok(None),
        };
        let name = css::ident(&cue.name).unwrap_or_default();
        if name.eq_ignore_ascii_case("cue-region") {
            return Self::region(cue, video_qualified)
                .map(|value| value.map(|selector| (selector, true)));
        }
        if video_qualified || !name.eq_ignore_ascii_case("cue") {
            return Ok(None);
        }
        let Some(argument) = &cue.arg else {
            return Ok(Some((
                Self {
                    tests: vec![Test::Root],
                    specificity: [0, 0, 1],
                },
                false,
            )));
        };
        let PseudoElementSelectorArgKind::CompoundSelector(compound) = &argument.kind else {
            return Err(unsupported("unsupported WebVTT ::cue selector"));
        };
        if compound.children.len() > MAX_SELECTOR_TERMS {
            return Err(limited("WebVTT ::cue has too many selector terms"));
        }
        let mut tests = Vec::new();
        let mut specificity = [0, 0, 1];
        for simple in &compound.children {
            let Some((test, score)) = Test::from_css(simple)? else {
                return Ok(None);
            };
            for (value, add) in specificity.iter_mut().zip(score) {
                *value += add;
            }
            tests.push(test);
        }
        Ok(Some((Self { tests, specificity }, false)))
    }

    fn region(
        cue: &PseudoElementSelector<'_>,
        video_qualified: bool,
    ) -> Result<Option<Self>, EngineError> {
        let mut specificity = [0, 0, 1 + u16::from(video_qualified)];
        let test = if let Some(argument) = &cue.arg {
            let PseudoElementSelectorArgKind::CompoundSelector(compound) = &argument.kind else {
                return Ok(None);
            };
            if compound.children.len() > MAX_SELECTOR_TERMS {
                return Err(limited("WebVTT ::cue-region has too many selector terms"));
            }
            let [SimpleSelector::Id(id)] = compound.children.as_slice() else {
                return Ok(None);
            };
            specificity[0] = 1;
            Test::Id(label(css::ident(&id.name))?)
        } else {
            Test::Universal
        };
        Ok(Some(Self {
            tests: vec![test],
            specificity,
        }))
    }

    fn matches(&self, target: &Target<'_>, work: &mut usize) -> Result<bool, EngineError> {
        for test in &self.tests {
            charge(
                work,
                test.cost(target),
                MAX_MATCH_WORK,
                "WebVTT stylesheet selector work exceeds its budget",
            )?;
            if !test.matches(target) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl Test {
    fn from_css(simple: &SimpleSelector<'_>) -> Result<Option<(Self, Specificity)>, EngineError> {
        let compiled = match simple {
            SimpleSelector::Type(TypeSelector::Universal(selector))
                if selector.prefix.is_none() =>
            {
                Some((Self::Universal, [0; 3]))
            }
            SimpleSelector::Type(TypeSelector::TagName(selector))
                if selector.name.prefix.is_none() =>
            {
                Some((
                    Self::Tag(label(css::ident(&selector.name.name))?),
                    [0, 0, 1],
                ))
            }
            SimpleSelector::Class(class) => {
                Some((Self::Class(label(css::ident(&class.name))?), [0, 1, 0]))
            }
            SimpleSelector::Id(id) => Some((Self::Id(label(css::ident(&id.name))?), [1, 0, 0])),
            SimpleSelector::Attribute(attribute) => {
                attribute_test(attribute)?.map(|test| (test, [0, 1, 0]))
            }
            SimpleSelector::PseudoClass(pseudo) => {
                pseudo_test(pseudo)?.map(|test| (test, [0, 1, 0]))
            }
            _ => None,
        };
        Ok(compiled)
    }

    fn cost(&self, target: &Target<'_>) -> usize {
        1 + match self {
            Self::Universal | Self::Root | Self::Past | Self::Future => 0,
            Self::Tag(value) | Self::Id(value) => value.len(),
            Self::Class(class) => class.len() * target.classes.len(),
            Self::Language(ranges) => ranges.iter().map(String::len).sum(),
            Self::Attribute { value, .. } => match value {
                AttributeValue::Present => 0,
                AttributeValue::Exact(value) | AttributeValue::Prefix(value) => value.len(),
            },
        }
    }

    fn matches(&self, target: &Target<'_>) -> bool {
        match self {
            Self::Universal => true,
            Self::Root => target.tag.is_none(),
            Self::Past => target.playback.past,
            Self::Future => target.playback.future,
            Self::Tag(tag) => target.tag == Some(tag.as_str()),
            Self::Id(id) => target.id == Some(id.as_str()),
            Self::Class(class) => target.classes.contains(class),
            Self::Language(ranges) => target.language.is_some_and(|language| {
                ranges
                    .iter()
                    .any(|range| range == "*" || language_prefix(language, range, true))
            }),
            Self::Attribute {
                kind,
                value,
                insensitive,
            } => {
                let actual = match kind {
                    AttributeKind::Voice if target.tag == Some("v") => target.voice,
                    AttributeKind::Language => target.language_attribute,
                    _ => None,
                };
                actual.is_some_and(|actual| match value {
                    AttributeValue::Present => true,
                    AttributeValue::Exact(value) => equal(actual, value, *insensitive),
                    AttributeValue::Prefix(value) => language_prefix(actual, value, *insensitive),
                })
            }
        }
    }
}

fn attribute_test(attribute: &AttributeSelector<'_>) -> Result<Option<Test>, EngineError> {
    if attribute.name.prefix.is_some() {
        return Ok(None);
    }
    let kind = match css::ident(&attribute.name.name) {
        Some("voice") => AttributeKind::Voice,
        Some("lang") => AttributeKind::Language,
        _ => return Ok(None),
    };
    let insensitive = match attribute
        .modifier
        .as_ref()
        .and_then(|modifier| css::ident(&modifier.ident))
    {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("s") => false,
        Some(value) if value.eq_ignore_ascii_case("i") => true,
        _ => return Ok(None),
    };
    let value = if let Some(matcher) = &attribute.matcher {
        let raw = match &attribute.value {
            Some(AttributeSelectorValue::Ident(value)) => css::ident(value),
            Some(AttributeSelectorValue::Str(value)) => css::string_value(value),
            _ => return Ok(None),
        };
        match matcher.kind {
            AttributeSelectorMatcherKind::Exact => AttributeValue::Exact(label(raw)?),
            AttributeSelectorMatcherKind::ExactOrPrefixThenHyphen => {
                AttributeValue::Prefix(label(raw)?)
            }
            _ => return Ok(None),
        }
    } else {
        AttributeValue::Present
    };
    Ok(Some(Test::Attribute {
        kind,
        value,
        insensitive,
    }))
}

fn pseudo_test(pseudo: &PseudoClassSelector<'_>) -> Result<Option<Test>, EngineError> {
    let name = css::ident(&pseudo.name).unwrap_or_default();
    if pseudo.arg.is_none() {
        return Ok(match name.to_ascii_lowercase().as_str() {
            "root" => Some(Test::Root),
            "past" => Some(Test::Past),
            "future" => Some(Test::Future),
            _ => None,
        });
    }
    if name.eq_ignore_ascii_case("lang") {
        if let Some(PseudoClassSelectorArgKind::LanguageRangeList(list)) =
            pseudo.arg.as_ref().map(|argument| &argument.kind)
        {
            if list.ranges.len() > MAX_SELECTOR_TERMS {
                return Err(limited(
                    "WebVTT :lang selector exceeds its range count budget",
                ));
            }
            let names = list
                .ranges
                .iter()
                .map(|value| {
                    label(match value {
                        LanguageRange::Str(value) => css::string_value(value),
                        LanguageRange::Ident(value) => css::ident(value),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if names.iter().any(|name| name != "*" && name.contains('*')) {
                return Ok(None);
            }
            return Ok(Some(Test::Language(names)));
        }
    }
    Ok(None)
}

fn label(value: Option<&str>) -> Result<String, EngineError> {
    let value = value.ok_or_else(|| invalid("invalid WebVTT stylesheet selector label"))?;
    if value.len() > MAX_STYLE_LABEL {
        return Err(limited(
            "WebVTT stylesheet selector label exceeds its byte budget",
        ));
    }
    Ok(value.to_owned())
}

fn equal(actual: &str, requested: &str, insensitive: bool) -> bool {
    if insensitive {
        actual.eq_ignore_ascii_case(requested)
    } else {
        actual == requested
    }
}

fn language_prefix(actual: &str, requested: &str, insensitive: bool) -> bool {
    equal(actual, requested, insensitive)
        || (actual.as_bytes().get(requested.len()) == Some(&b'-')
            && actual
                .get(..requested.len())
                .is_some_and(|prefix| equal(prefix, requested, insensitive)))
}

fn declarations(block: &SimpleBlock<'_>, source: &str) -> Result<Vec<Declaration>, EngineError> {
    if block.statements.len() > MAX_DECLARATIONS {
        return Err(limited(
            "WebVTT stylesheet declarations exceed their rule budget",
        ));
    }
    let mut output = Vec::new();
    for statement in &block.statements {
        let Statement::Declaration(declaration) = statement else {
            return Err(unsupported(
                "WebVTT stylesheets require plain CSS declarations",
            ));
        };
        let Some(name) = css::ident(&declaration.name) else {
            continue;
        };
        let Some((first, last)) = declaration.value.first().zip(declaration.value.last()) else {
            continue;
        };
        let value = if let [ComponentValue::InterpolableIdent(value)] = declaration.value.as_slice()
        {
            css::ident(value).unwrap_or_default()
        } else {
            &source[first.span().start..last.span().end]
        };
        if let Some(value) = declaration_value(&name.to_ascii_lowercase(), value) {
            output.push(Declaration {
                value,
                important: declaration.important.is_some(),
            });
        }
    }
    Ok(output)
}

fn declaration_value(name: &str, value: &str) -> Option<Value> {
    let keyword = value.to_ascii_lowercase();
    match name {
        "color" => parse_color(value).map(Value::Color),
        "background" if keyword == "none" => Some(Value::Background(Color::Fixed([0; 4]))),
        "background" | "background-color" => parse_color(value).map(Value::Background),
        "font-weight" => match keyword.as_str() {
            "normal" => Some(Value::Bold(false)),
            "bold" => Some(Value::Bold(true)),
            _ => value
                .parse::<f32>()
                .ok()
                .filter(|weight| (1.0..=1000.0).contains(weight))
                .map(|weight| Value::Bold(weight >= 600.0)),
        },
        "font-style" => Some(Value::Slant(match keyword.as_str() {
            "normal" => FontStyle::Normal,
            "italic" => FontStyle::Italic,
            "oblique" => FontStyle::Oblique,
            _ => return None,
        })),
        "opacity" => {
            let (number, scale) = value
                .strip_suffix('%')
                .map_or((value, 1.0), |number| (number, 0.01));
            number
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .map(|number| Value::Opacity((number * scale).clamp(0.0, 1.0)))
        }
        "visibility" => Some(Value::Visibility(match keyword.as_str() {
            "visible" => "visible",
            "hidden" => "hidden",
            "collapse" => "collapse",
            _ => return None,
        })),
        "text-decoration" | "text-decoration-line" => {
            let mut lines = [false; 3];
            if keyword != "none" {
                for word in keyword.split_ascii_whitespace() {
                    let slot = match word {
                        "underline" => 0,
                        "line-through" => 1,
                        "overline" => 2,
                        _ => return None,
                    };
                    if std::mem::replace(&mut lines[slot], true) {
                        return None;
                    }
                }
            }
            Some(Value::Decoration(lines))
        }
        _ => None,
    }
}

fn parse_color(value: &str) -> Option<Color> {
    if value.eq_ignore_ascii_case("currentcolor") {
        return Some(Color::Current);
    }
    let ([r, g, b], alpha) = css::color(value)?;
    Some(Color::Fixed([r, g, b, (alpha * 255.0).round() as u8]))
}

pub(super) fn background(top: Option<Rgba>, bottom: Option<Rgba>) -> Option<Rgba> {
    match (top, bottom) {
        (Some(top), Some(bottom)) if top[3] < 255 => {
            let top_alpha = u32::from(top[3]);
            let bottom_alpha = u32::from(bottom[3]) * (255 - top_alpha);
            let top_alpha = top_alpha * 255;
            let alpha = top_alpha + bottom_alpha;
            if alpha == 0 {
                return Some([0; 4]);
            }
            let component = |index: usize| {
                let sum =
                    u32::from(top[index]) * top_alpha + u32::from(bottom[index]) * bottom_alpha;
                ((sum + alpha / 2) / alpha) as u8
            };
            Some([
                component(0),
                component(1),
                component(2),
                ((alpha + 127) / 255) as u8,
            ])
        }
        (top, bottom) => top.or(bottom),
    }
}
