//! Load bounded EPUB stylesheets and cascade supported author declarations on the XHTML tree.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use raffia::{ast::*, Parser, Spanned, Syntax};
use roxmltree::Node;

use super::{charge, external, invalid, limited, name, resolve, unsupported, Book, XHTML};
use crate::EngineError;

const SHEET_BYTES: usize = 256 * 1024;
const SELECTOR_WORK: usize = 4_000_000;
const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

pub(super) type Properties = HashMap<String, String>;
type Specificity = [u16; 3];

#[derive(Default)]
pub(super) struct Cascade {
    rules: Vec<Rule>,
    pub(super) fonts: Vec<super::fonts::Face>,
    imports: HashSet<String>,
    bytes: usize,
    work: usize,
    viewport: [f32; 2],
}

struct Rule {
    selector: Selector,
    declarations: Arc<[DeclarationValue]>,
}

type DeclarationValue = (String, String, bool);

impl Cascade {
    pub(super) fn load(
        root: Node<'_, '_>,
        base: &str,
        book: &mut Book<'_>,
        viewport: [f32; 2],
    ) -> Result<Self, EngineError> {
        let mut cascade = Self {
            viewport,
            ..Default::default()
        };
        for node in root.descendants().filter(Node::is_element) {
            if !cascade.media_matches(node.attribute("media").unwrap_or("")) {
                continue;
            }
            if name(node, XHTML, "style") {
                let text: String = node
                    .children()
                    .filter(Node::is_text)
                    .filter_map(|node| node.text())
                    .collect();
                cascade.sheet(&text, base, book, 0)?;
            } else if name(node, XHTML, "link")
                && node.attribute("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|part| part.eq_ignore_ascii_case("stylesheet"))
                })
                && !node.attribute("rel").is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|part| part.eq_ignore_ascii_case("alternate"))
                })
            {
                let href = super::attribute(node, "href")?;
                cascade.external_sheet(href, base, book, 0)?;
            }
        }
        Ok(cascade)
    }

    pub(super) fn needed(&self, root: Node<'_, '_>) -> bool {
        self.bytes != 0
            || root
                .descendants()
                .any(|node| node.attribute("style").is_some())
    }

    fn external_sheet(
        &mut self,
        href: &str,
        base: &str,
        book: &mut Book<'_>,
        depth: usize,
    ) -> Result<(), EngineError> {
        if external(href) {
            return Err(unsupported("remote EPUB stylesheets"));
        }
        let path = resolve(base, href)?;
        if depth > 8 {
            return Err(limited("EPUB stylesheet imports exceed depth 8"));
        }
        if !self.imports.insert(path.clone()) {
            return Err(invalid("cyclic EPUB stylesheet import"));
        }
        let bytes = book.read(&path, SHEET_BYTES)?;
        self.sheet(
            std::str::from_utf8(&bytes).map_err(invalid)?,
            &path,
            book,
            depth,
        )?;
        self.imports.remove(&path);
        Ok(())
    }

    fn sheet(
        &mut self,
        source: &str,
        base: &str,
        book: &mut Book<'_>,
        depth: usize,
    ) -> Result<(), EngineError> {
        self.charge_bytes(source)?;
        let source = checked_css(source.trim_start_matches('\u{feff}'))?;
        let sheet = Parser::new(&source, Syntax::Css)
            .parse::<Stylesheet>()
            .map_err(|error| {
                invalid(format!(
                    "CSS at byte {}: {:?}",
                    error.span.start, error.kind
                ))
            })?;
        self.statements(&sheet.statements, &source, base, book, depth)
    }

    fn statements(
        &mut self,
        statements: &[Statement<'_>],
        source: &str,
        base: &str,
        book: &mut Book<'_>,
        depth: usize,
    ) -> Result<(), EngineError> {
        for statement in statements {
            match statement {
                Statement::QualifiedRule(rule) => {
                    let values: Arc<[_]> = declarations(&rule.block, source)?.into();
                    if values.is_empty() {
                        continue;
                    }
                    for selector in &rule.selector.selectors {
                        if let Some(selector) = Selector::compile(selector) {
                            if self.rules.len() >= 4_096 {
                                return Err(limited("EPUB stylesheet rule limit exceeded"));
                            }
                            self.rules.push(Rule {
                                selector,
                                declarations: values.clone(),
                            });
                        }
                    }
                }
                Statement::AtRule(rule) if rule.name.name.eq_ignore_ascii_case("font-face") => {
                    if let Some(block) = &rule.block {
                        if let Some(face) = font_face(block, source, base)? {
                            if self.fonts.len() >= 32 {
                                return Err(limited("EPUB stylesheet font-face limit exceeded"));
                            }
                            self.fonts.push(face);
                        }
                    }
                }
                Statement::AtRule(rule) => match &rule.prelude {
                    Some(AtRulePrelude::Import(import)) => {
                        if import.layer.is_some() || import.supports.is_some() {
                            continue;
                        }
                        if import.media.as_ref().is_some_and(|media| {
                            !self.media_matches(&source[media.span.start..media.span.end])
                        }) {
                            continue;
                        }
                        let href = match &import.href {
                            ImportPreludeHref::Str(string) => string_value(string),
                            ImportPreludeHref::Url(url) => match &url.value {
                                Some(UrlValue::Raw(url)) => Some(url.value.as_ref()),
                                Some(UrlValue::Str(string)) => string_value(string),
                                _ => None,
                            },
                        }
                        .ok_or_else(|| invalid("invalid EPUB stylesheet import URL"))?;
                        self.external_sheet(href, base, book, depth + 1)?;
                    }
                    Some(AtRulePrelude::Media(media))
                        if self.media_matches(&source[media.span.start..media.span.end]) =>
                    {
                        if let Some(block) = &rule.block {
                            self.statements(&block.statements, source, base, book, depth)?;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    }

    fn charge_bytes(&mut self, source: &str) -> Result<(), EngineError> {
        if source.len() > SHEET_BYTES {
            return Err(limited("EPUB stylesheet exceeds 256 KiB"));
        }
        charge(
            &mut self.bytes,
            source.len(),
            1024 * 1024,
            "EPUB stylesheet text exceeds 1 MiB",
        )
    }

    pub(super) fn properties(&mut self, node: Node<'_, '_>) -> Result<Properties, EngineError> {
        let mut selected = HashMap::new();
        for rule in &self.rules {
            if rule
                .selector
                .matches(node, rule.selector.steps.len(), &mut self.work)?
            {
                for (key, value, important) in rule.declarations.iter() {
                    charge(
                        &mut self.work,
                        1,
                        SELECTOR_WORK,
                        "EPUB CSS cascade work limit exceeded",
                    )?;
                    update(
                        &mut selected,
                        key,
                        value,
                        (*important, false, rule.selector.specificity),
                    );
                }
            }
        }
        if let Some(style) = node.attribute("style") {
            self.charge_bytes(style)?;
            let source = checked_css(&format!("{{{style}}}"))?;
            let block = Parser::new(&source, Syntax::Css)
                .parse::<SimpleBlock>()
                .map_err(|error| {
                    invalid(format!(
                        "inline CSS at byte {}: {:?}",
                        error.span.start, error.kind
                    ))
                })?;
            if block.span.end != source.len() {
                return Err(invalid("EPUB inline CSS has trailing input"));
            }
            for (key, value, important) in declarations(&block, &source)? {
                update(&mut selected, &key, &value, (important, true, [0; 3]));
            }
        }
        Ok(selected
            .into_iter()
            .map(|(key, (_, value))| (key, value))
            .collect())
    }

    fn media_matches(&self, query: &str) -> bool {
        let query = query.trim();
        query.is_empty()
            || query.split(',').any(|query| {
                query.split(" and ").all(|term| {
                    let term = term.trim();
                    if matches!(term, "all" | "screen" | "only screen") {
                        return true;
                    }
                    let Some(query) = term
                        .strip_prefix('(')
                        .and_then(|term| term.strip_suffix(')'))
                    else {
                        return false;
                    };
                    let Some((feature, value)) = query.split_once(':') else {
                        return false;
                    };
                    let feature = feature.trim();
                    if feature == "orientation" {
                        let landscape = self.viewport[0] > self.viewport[1];
                        return value.trim() == if landscape { "landscape" } else { "portrait" };
                    }
                    let Some(value) = points(value.trim(), 12.0, self.viewport) else {
                        return false;
                    };
                    match feature {
                        "width" => self.viewport[0] == value,
                        "height" => self.viewport[1] == value,
                        "min-width" => self.viewport[0] >= value,
                        "max-width" => self.viewport[0] <= value,
                        "min-height" => self.viewport[1] >= value,
                        "max-height" => self.viewport[1] <= value,
                        _ => false,
                    }
                })
            })
    }
}

type Priority = (bool, bool, Specificity);

fn update(
    values: &mut HashMap<String, (Priority, String)>,
    name: &str,
    value: &str,
    priority: Priority,
) {
    if values
        .get(name)
        .is_none_or(|(winner, _)| *winner <= priority)
    {
        values.insert(name.to_owned(), (priority, value.to_owned()));
    }
}

fn font_face(
    block: &SimpleBlock<'_>,
    source: &str,
    base: &str,
) -> Result<Option<super::fonts::Face>, EngineError> {
    let values: Properties = declarations(block, source)?
        .into_iter()
        .map(|(key, value, _)| (key, value))
        .collect();
    let Some(family) = values
        .get("font-family")
        .map(|family| family.trim_matches(['\'', '"']))
    else {
        return Ok(None);
    };
    if family.is_empty() || family.len() > 255 || family.contains(',') {
        return Err(invalid("invalid EPUB font-face family"));
    }
    let mut urls = Vec::new();
    for statement in &block.statements {
        let Statement::Declaration(declaration) = statement else {
            continue;
        };
        if !ident(&declaration.name).is_some_and(|name| name.eq_ignore_ascii_case("src")) {
            continue;
        }
        urls.clear();
        for value in &declaration.value {
            if let ComponentValue::Url(url) = value {
                match &url.value {
                    Some(UrlValue::Raw(raw)) => urls.push(raw.value.to_string()),
                    Some(UrlValue::Str(string)) => {
                        if let Some(url) = string_value(string) {
                            urls.push(url.to_owned());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let weight = match values.get("font-weight").map(String::as_str) {
        None | Some("normal") => Some(400.0),
        Some("bold") => Some(700.0),
        Some(value) => value
            .parse::<f32>()
            .ok()
            .filter(|weight| (1.0..=1000.0).contains(weight)),
    };
    let style = match values.get("font-style").map(String::as_str) {
        None | Some("normal") => Some(parley::FontStyle::Normal),
        Some("italic") => Some(parley::FontStyle::Italic),
        Some("oblique") => Some(parley::FontStyle::Oblique(None)),
        _ => None,
    };
    Ok(Some(super::fonts::Face {
        family: family.into(),
        base: base.into(),
        urls,
        weight,
        style,
        unicode_range: values.contains_key("unicode-range"),
    }))
}

fn declarations(
    block: &SimpleBlock<'_>,
    source: &str,
) -> Result<Vec<DeclarationValue>, EngineError> {
    let mut output = Vec::new();
    for statement in &block.statements {
        let Statement::Declaration(declaration) = statement else {
            continue;
        };
        let Some(key) = ident(&declaration.name) else {
            continue;
        };
        let Some((first, last)) = declaration.value.first().zip(declaration.value.last()) else {
            continue;
        };
        let value = source[first.span().start..last.span().end].trim();
        if output.len() >= 512 || value.len() > 4_096 {
            return Err(limited("EPUB CSS declarations exceed their budget"));
        }
        let key = key.to_ascii_lowercase();
        let important = declaration.important.is_some();
        if matches!(
            key.as_str(),
            "margin" | "padding" | "inset" | "border-width" | "border-color"
        ) {
            let words: Vec<_> = value.split_ascii_whitespace().collect();
            if let Some(items) = sides(&words) {
                for (side, value) in SIDES.into_iter().zip(items) {
                    let name = match key.as_str() {
                        "inset" => side.to_owned(),
                        "border-width" => format!("border-{side}-width"),
                        "border-color" => format!("border-{side}-color"),
                        _ => format!("{key}-{side}"),
                    };
                    output.push((name, value.to_owned(), important));
                }
            }
        } else if key == "background" && color(value).is_some() {
            output.push(("background-color".into(), value.into(), important));
        } else if key == "border" {
            let mut width = "0";
            let mut ink = "currentColor";
            let mut raw_style = "none";
            for word in value.split_ascii_whitespace() {
                if points(word, 12.0, [0.0; 2]).is_some()
                    || matches!(word, "thin" | "medium" | "thick")
                {
                    width = word;
                } else if color(word).is_some() {
                    ink = word;
                } else if matches!(word, "solid" | "none" | "hidden") {
                    raw_style = word;
                }
            }
            for side in SIDES {
                for (name, value) in [("width", width), ("color", ink), ("style", raw_style)] {
                    output.push((format!("border-{side}-{name}"), value.into(), important));
                }
            }
        } else {
            output.push((key, value.to_owned(), important));
        }
    }
    Ok(output)
}

fn sides<'a>(parts: &[&'a str]) -> Option<[&'a str; 4]> {
    match parts {
        [a] => Some([a, a, a, a]),
        [a, b] => Some([a, b, a, b]),
        [a, b, c] => Some([a, b, c, b]),
        [a, b, c, d] => Some([a, b, c, d]),
        _ => None,
    }
}

fn ident<'a>(value: &'a InterpolableIdent<'_>) -> Option<&'a str> {
    match value {
        InterpolableIdent::Literal(value) => Some(&value.name),
        _ => None,
    }
}

fn string_value<'a>(value: &'a InterpolableStr<'_>) -> Option<&'a str> {
    match value {
        InterpolableStr::Literal(value) => Some(&value.value),
        _ => None,
    }
}

struct Selector {
    steps: Vec<Step>,
    specificity: Specificity,
}

struct Step {
    tests: Vec<Test>,
    relation: Option<CombinatorKind>,
}

enum Test {
    Tag(String),
    Id(String),
    Class(String),
    Attribute {
        name: String,
        rule: Option<(AttributeSelectorMatcherKind, String)>,
        insensitive: bool,
    },
    Root,
    First,
    Last,
    Only,
    Empty,
    Link,
    Choice {
        selectors: Vec<Selector>,
        negated: bool,
    },
}

impl Selector {
    fn compile(selector: &ComplexSelector<'_>) -> Option<Self> {
        if selector.children.len() > 31 {
            return None;
        }
        let mut steps: Vec<Step> = Vec::new();
        let mut specificity = [0; 3];
        let mut relation = None;
        for child in &selector.children {
            match child {
                ComplexSelectorChild::Combinator(combinator)
                    if combinator.kind != CombinatorKind::Column =>
                {
                    relation = Some(combinator.kind.clone())
                }
                ComplexSelectorChild::CompoundSelector(compound) => {
                    let mut tests = Vec::new();
                    for simple in &compound.children {
                        let (test, score) = match simple {
                            SimpleSelector::Type(TypeSelector::TagName(tag))
                                if tag.name.prefix.is_none() =>
                            {
                                (Test::Tag(ident(&tag.name.name)?.into()), [0, 0, 1])
                            }
                            SimpleSelector::Type(TypeSelector::Universal(tag))
                                if tag.prefix.is_none() =>
                            {
                                continue
                            }
                            SimpleSelector::Id(id) => {
                                (Test::Id(ident(&id.name)?.into()), [1, 0, 0])
                            }
                            SimpleSelector::Class(class) => {
                                (Test::Class(ident(&class.name)?.into()), [0, 1, 0])
                            }
                            SimpleSelector::Attribute(attr) if attr.name.prefix.is_none() => {
                                let name = ident(&attr.name.name)?.to_owned();
                                let rule = match (&attr.matcher, &attr.value) {
                                    (None, None) => None,
                                    (Some(op), Some(value)) => {
                                        let value = match value {
                                            AttributeSelectorValue::Ident(value) => ident(value),
                                            AttributeSelectorValue::Str(value) => {
                                                string_value(value)
                                            }
                                            _ => None,
                                        }?;
                                        Some((op.kind.clone(), value.into()))
                                    }
                                    _ => return None,
                                };
                                let insensitive = attr
                                    .modifier
                                    .as_ref()
                                    .and_then(|value| ident(&value.ident))
                                    .is_some_and(|value| value.eq_ignore_ascii_case("i"));
                                (
                                    Test::Attribute {
                                        name,
                                        rule,
                                        insensitive,
                                    },
                                    [0, 1, 0],
                                )
                            }
                            SimpleSelector::PseudoClass(pseudo) => Self::pseudo(pseudo)?,
                            _ => return None,
                        };
                        tests.push(test);
                        for (a, b) in specificity.iter_mut().zip(score) {
                            *a = u16::saturating_add(*a, b);
                        }
                    }
                    steps.push(Step {
                        tests,
                        relation: relation.take(),
                    });
                }
                _ => return None,
            }
        }
        Some(Self { steps, specificity })
    }

    fn pseudo(pseudo: &PseudoClassSelector<'_>) -> Option<(Test, Specificity)> {
        let id = ident(&pseudo.name)?;
        if let Some(arg) = &pseudo.arg {
            let PseudoClassSelectorArgKind::SelectorList(list) = &arg.kind else {
                return None;
            };
            if !matches!(id, "is" | "not" | "where") {
                return None;
            }
            let selectors: Vec<_> = list
                .selectors
                .iter()
                .map(Self::compile)
                .collect::<Option<_>>()?;
            let specificity = if id == "where" {
                [0; 3]
            } else {
                selectors.iter().map(|sel| sel.specificity).max()?
            };
            return Some((
                Test::Choice {
                    selectors,
                    negated: id == "not",
                },
                specificity,
            ));
        }
        let test = match id {
            "root" => Test::Root,
            "first-child" => Test::First,
            "last-child" => Test::Last,
            "only-child" => Test::Only,
            "empty" => Test::Empty,
            "link" | "any-link" => Test::Link,
            _ => return None,
        };
        Some((test, [0, 1, 0]))
    }

    fn matches(
        &self,
        node: Node<'_, '_>,
        step_count: usize,
        work: &mut usize,
    ) -> Result<bool, EngineError> {
        let Some(index) = step_count.checked_sub(1) else {
            return Ok(false);
        };
        let step = &self.steps[index];
        charge(
            work,
            step.tests.len() + 1,
            SELECTOR_WORK,
            "EPUB CSS selector work limit exceeded",
        )?;
        for test in &step.tests {
            if !test.matches(node, work)? {
                return Ok(false);
            }
        }
        if index == 0 {
            return Ok(true);
        }
        match step.relation.as_ref() {
            Some(CombinatorKind::Child) => match node.parent_element() {
                Some(parent) => self.matches(parent, index, work),
                None => Ok(false),
            },
            Some(CombinatorKind::NextSibling) => match node.prev_sibling_element() {
                Some(sibling) => self.matches(sibling, index, work),
                None => Ok(false),
            },
            Some(CombinatorKind::Descendant) => {
                for parent in node.ancestors().skip(1).filter(Node::is_element) {
                    if self.matches(parent, index, work)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Some(CombinatorKind::LaterSibling) => {
                let mut sibling = node.prev_sibling_element();
                while let Some(previous) = sibling {
                    if self.matches(previous, index, work)? {
                        return Ok(true);
                    }
                    sibling = previous.prev_sibling_element();
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }
}

impl Test {
    fn matches(&self, node: Node<'_, '_>, work: &mut usize) -> Result<bool, EngineError> {
        let yes = match self {
            Self::Tag(tag) => node.tag_name().name() == tag,
            Self::Id(id) => super::element_ids(node).any(|value| value == id),
            Self::Class(class) => node.attribute("class").is_some_and(|classes| {
                classes.split_ascii_whitespace().any(|value| value == class)
            }),
            Self::Attribute {
                name,
                rule,
                insensitive,
            } => node.attribute(name.as_str()).is_some_and(|actual| {
                let Some((operation, expected)) = rule else {
                    return true;
                };
                let (actual, expected) = if *insensitive {
                    (
                        std::borrow::Cow::Owned(actual.to_ascii_lowercase()),
                        std::borrow::Cow::Owned(expected.to_ascii_lowercase()),
                    )
                } else {
                    (
                        std::borrow::Cow::Borrowed(actual),
                        std::borrow::Cow::Borrowed(expected.as_str()),
                    )
                };
                use AttributeSelectorMatcherKind::*;
                let expected = expected.as_ref();
                match operation {
                    Exact => actual == expected,
                    MatchWord => actual.split_ascii_whitespace().any(|part| part == expected),
                    ExactOrPrefixThenHyphen => {
                        actual == expected
                            || actual
                                .strip_prefix(expected)
                                .is_some_and(|part| part.starts_with('-'))
                    }
                    Prefix => !expected.is_empty() && actual.starts_with(expected),
                    Suffix => !expected.is_empty() && actual.ends_with(expected),
                    Substring => !expected.is_empty() && actual.contains(expected),
                }
            }),
            Self::Root => node.parent_element().is_none(),
            Self::First => node.prev_sibling_element().is_none(),
            Self::Last => node.next_sibling_element().is_none(),
            Self::Only => {
                node.prev_sibling_element().is_none() && node.next_sibling_element().is_none()
            }
            Self::Empty => !node.children().any(|child| {
                child.is_element()
                    || child.is_text() && child.text().is_some_and(|text| !text.is_empty())
            }),
            Self::Link => node.tag_name().name() == "a" && node.attribute("href").is_some(),
            Self::Choice { selectors, negated } => {
                for selector in selectors {
                    if selector.matches(node, selector.steps.len(), work)? {
                        return Ok(!negated);
                    }
                }
                *negated
            }
        };
        Ok(yes)
    }
}

pub(super) fn points(value: &str, font: f32, viewport: [f32; 2]) -> Option<f32> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    for (unit, multiplier) in [
        ("rem", 12.0),
        ("em", font),
        ("px", 0.75),
        ("pt", 1.0),
        ("pc", 12.0),
        ("in", 72.0),
        ("cm", 72.0 / 2.54),
        ("mm", 72.0 / 25.4),
        ("vw", viewport[0] / 100.0),
        ("vh", viewport[1] / 100.0),
    ] {
        if let Some(number) = value.strip_suffix(unit) {
            return number
                .parse::<f32>()
                .ok()
                .map(|n| n * multiplier)
                .filter(|n| n.is_finite() && n.abs() <= 25_000.0);
        }
    }
    None
}

pub(super) fn color(value: &str) -> Option<([u8; 3], f32)> {
    let value = color::parse_color(value.trim())
        .ok()?
        .to_alpha_color::<color::Srgb>()
        .to_rgba8();
    Some(([value.r, value.g, value.b], f32::from(value.a) / 255.0))
}

// Raffia parses nested CSS functions and blocks recursively. Bound nesting before
// calling it, and replace comments with spaces while retaining byte spans.
fn checked_css(source: &str) -> Result<String, EngineError> {
    let mut bytes = source.as_bytes().to_vec();
    let mut pos = 0;
    let mut delimiters = Vec::new();
    let mut quote = 0;
    while pos < bytes.len() {
        let byte = bytes[pos];
        if byte == b'\\' {
            pos += 2;
            continue;
        }
        if quote != 0 {
            if byte == quote {
                quote = 0;
            }
        } else {
            match byte {
                b'\'' | b'"' => quote = byte,
                b'/' if bytes.get(pos + 1) == Some(&b'*') => {
                    let start = pos;
                    let Some(end) = bytes[pos + 2..].windows(2).position(|pair| pair == b"*/")
                    else {
                        return Err(invalid("unclosed EPUB CSS comment"));
                    };
                    pos += end + 4;
                    bytes[start..pos].fill(b' ');
                    continue;
                }
                b'{' | b'(' | b'[' => {
                    if delimiters.len() == 32 {
                        return Err(limited("EPUB CSS nesting exceeds 32"));
                    }
                    delimiters.push(match byte {
                        b'{' => b'}',
                        b'(' => b')',
                        _ => b']',
                    });
                }
                b'}' | b')' | b']' if delimiters.pop() != Some(byte) => {
                    return Err(invalid("mismatched EPUB CSS delimiter"));
                }
                _ => {}
            }
        }
        pos += 1;
    }
    if !delimiters.is_empty() || quote != 0 {
        return Err(invalid("unclosed EPUB CSS delimiter or string"));
    }
    String::from_utf8(bytes).map_err(invalid)
}
