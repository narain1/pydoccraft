//! Load bounded EPUB stylesheets and cascade supported author declarations on the XHTML tree.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use parley::fontique::GenericFamily;
use raffia::{ast::*, Parser, Spanned, Syntax};
use roxmltree::Node;
use serde::{Deserialize, Serialize};

use super::{charge, external, invalid, limited, name, resolve, unsupported, Book, XHTML};
use crate::EngineError;

#[path = "epub_css_variables.rs"]
mod variables;

const SHEET_BYTES: usize = 256 * 1024;
const SELECTOR_WORK: usize = 4_000_000;
const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

pub(super) type Properties = HashMap<String, String>;
type Specificity = [u16; 3];

#[derive(Default)]
pub(super) struct Cascade {
    rules: Vec<Rule>,
    candidates: RuleIndex,
    pub(super) fonts: Vec<super::fonts::Face>,
    imports: HashSet<String>,
    bytes: usize,
    work: usize,
    viewport: [f32; 2],
    variables: variables::Scope,
}

struct Rule {
    selector: Selector,
    declarations: Arc<[DeclarationValue]>,
}

#[derive(Default)]
struct RuleIndex {
    ids: HashMap<String, Vec<usize>>,
    classes: HashMap<String, Vec<usize>>,
    tags: HashMap<String, Vec<usize>>,
    other: Vec<usize>,
}

impl RuleIndex {
    fn insert(&mut self, selector: &Selector, order: usize) {
        // Every test in the last compound must match the subject element.
        let key = selector
            .steps
            .last()
            .into_iter()
            .flat_map(|step| &step.tests)
            .filter_map(Test::index_key)
            .min_by_key(|(kind, _)| *kind);
        if let Some((kind, value)) = key {
            let index = match kind {
                KeyKind::Id => &mut self.ids,
                KeyKind::Class => &mut self.classes,
                KeyKind::Tag => &mut self.tags,
            };
            index.entry(value.to_owned()).or_default().push(order);
        } else {
            self.other.push(order);
        }
    }

    fn matching_keys(
        &self,
        node: Node<'_, '_>,
        work: &mut usize,
    ) -> Result<Vec<usize>, EngineError> {
        let mut candidates = self.other.clone();
        if let Some(tags) = self.tags.get(node.tag_name().name()) {
            candidates.extend(tags);
        }
        let mut seen = HashSet::new();
        for id in super::element_ids(node) {
            if let Some(rules) = self.ids.get(id).filter(|_| seen.insert(id)) {
                candidates.extend(rules);
            }
        }
        seen.clear();
        if !self.classes.is_empty() {
            if let Some(classes) = node.attribute("class") {
                for class in classes.split_ascii_whitespace() {
                    charge(
                        work,
                        1,
                        SELECTOR_WORK,
                        "EPUB CSS selector work limit exceeded",
                    )?;
                    if let Some(rules) = self.classes.get(class).filter(|_| seen.insert(class)) {
                        candidates.extend(rules);
                    }
                }
            }
        }
        Ok(candidates)
    }
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum KeyKind {
    Id,
    Class,
    Tag,
}

type DeclarationValue = (String, CssValue, bool);

#[derive(Clone)]
enum CssValue {
    Text(Arc<str>),
    Pending(Arc<variables::Template>, Arc<str>),
}

fn value_parser(source: &str) -> Parser<'_, '_> {
    raffia::ParserBuilder::new(source)
        .options(raffia::ParserOptions {
            try_parsing_value_in_custom_property: true,
            ..Default::default()
        })
        .build()
}

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

    pub(super) fn load_svg(
        root: Node<'_, '_>,
        base: &str,
        book: &mut Book<'_>,
        viewport: [f32; 2],
    ) -> Result<Self, EngineError> {
        let mut cascade = Self {
            viewport,
            ..Default::default()
        };
        for instruction in root
            .document()
            .root()
            .children()
            .take_while(|node| !node.is_element())
            .filter_map(|node| node.pi())
        {
            if let Some((href, media)) = super::svg::stylesheet(instruction)? {
                if cascade.media_matches(&media) {
                    cascade.external_sheet(&href, base, book, 0)?;
                }
            }
        }
        for node in root
            .descendants()
            .filter(|n| n.has_tag_name((super::svg::NS, "style")))
        {
            if node.attribute("type").unwrap_or("text/css") != "text/css"
                || !cascade.media_matches(node.attribute("media").unwrap_or(""))
            {
                continue;
            }
            let source = node
                .children()
                .filter(Node::is_text)
                .filter_map(|child| child.text())
                .collect::<String>();
            cascade.sheet(&source, base, book, 0)?;
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
        let sheet = value_parser(&source)
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
                            self.candidates.insert(&selector, self.rules.len());
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
        for order in self.candidates.matching_keys(node, &mut self.work)? {
            let rule = &self.rules[order];
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
                        (*important, false, rule.selector.specificity, order),
                    );
                }
            }
        }
        if let Some(style) = node.attribute("style") {
            self.charge_bytes(style)?;
            let source = checked_css(&format!("{{{style}}}"))?;
            let block = value_parser(&source)
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
                update(&mut selected, &key, &value, (important, true, [0; 3], 0));
            }
        }
        let custom = selected
            .extract_if(|name, _| name.starts_with("--"))
            .map(|(name, (_, value))| (name, value))
            .collect();
        self.variables.enter(node, &custom)?;
        let mut output = Properties::new();
        for (name, (_, value)) in selected {
            let value = match value {
                CssValue::Text(value) => value.to_string(),
                CssValue::Pending(template, property) => {
                    let Some(value) = self.variables.substitute(&template)? else {
                        continue;
                    };
                    if !valid_computed(&property, &value) {
                        continue;
                    }
                    if name == property.as_ref() {
                        value
                    } else {
                        let Some((_, expanded)) = expand(&property, &value)
                            .into_iter()
                            .find(|(part, _)| part == &name)
                        else {
                            continue;
                        };
                        expanded
                    }
                }
            };
            output.insert(name, value);
        }
        Ok(output)
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

type Priority = (bool, bool, Specificity, usize);

fn update(
    values: &mut HashMap<String, (Priority, CssValue)>,
    name: &str,
    value: &CssValue,
    priority: Priority,
) {
    if values
        .get(name)
        .is_none_or(|(winner, _)| *winner <= priority)
    {
        values.insert(name.to_owned(), (priority, value.clone()));
    }
}

fn font_face(
    block: &SimpleBlock<'_>,
    source: &str,
    base: &str,
) -> Result<Option<super::fonts::Face>, EngineError> {
    let values: Properties = declarations(block, source)?
        .into_iter()
        .filter_map(|(key, value, _)| match value {
            CssValue::Text(text) => Some((key, text.to_string())),
            CssValue::Pending(..) => None,
        })
        .collect();
    let Some(family) = values.get("font-family") else {
        return Ok(None);
    };
    let Some(FontFamilyValue::Families(families)) = parse_font_family(family) else {
        return Err(invalid("invalid EPUB font-face family"));
    };
    let [Family {
        name: family,
        generic: false,
    }] = families.as_ref()
    else {
        return Err(invalid("invalid EPUB font-face family"));
    };
    if family.len() > 255 {
        return Err(invalid("invalid EPUB font-face family"));
    }
    let mut urls = Vec::new();
    let mut unicode_ranges = None;
    for statement in &block.statements {
        let Statement::Declaration(declaration) = statement else {
            continue;
        };
        let Some(key) = ident(&declaration.name) else {
            continue;
        };
        if key.eq_ignore_ascii_case("unicode-range") {
            if let Some(ranges) = unicode_range_list(&declaration.value)? {
                unicode_ranges = Some(ranges);
            }
            continue;
        }
        if !key.eq_ignore_ascii_case("src") {
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
        family: family.clone(),
        base: base.into(),
        urls,
        weight,
        style,
        unicode_ranges,
    }))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Family {
    pub name: String,
    pub generic: bool,
}

#[derive(Clone)]
pub(super) enum FontFamilyValue {
    Families(Arc<[Family]>),
    Inherit,
    Initial,
}

pub(super) fn parse_font_family(source: &str) -> Option<FontFamilyValue> {
    let declaration = format!("font-family:{source}");
    let declaration = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .ok()?;
    font_family_value(&declaration.value)
}

fn font_family_value(values: &[ComponentValue<'_>]) -> Option<FontFamilyValue> {
    if let [ComponentValue::InterpolableIdent(value)] = values {
        match ident(value)?.to_ascii_lowercase().as_str() {
            "inherit" | "unset" => return Some(FontFamilyValue::Inherit),
            "initial" | "revert" | "revert-layer" => return Some(FontFamilyValue::Initial),
            _ => {}
        }
    }
    let mut families = Vec::new();
    for entry in values.split(|value| {
        matches!(
            value,
            ComponentValue::Delimiter(Delimiter {
                kind: DelimiterKind::Comma,
                ..
            })
        )
    }) {
        let (name, generic) = if let [ComponentValue::InterpolableStr(value)] = entry {
            (string_value(value)?.to_owned(), false)
        } else {
            let mut words = Vec::new();
            for value in entry {
                let ComponentValue::InterpolableIdent(value) = value else {
                    return None;
                };
                let name = ident(value)?;
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "initial" | "inherit" | "unset" | "revert" | "revert-layer" | "default"
                ) {
                    return None;
                }
                words.push(name);
            }
            let name = words.join(" ");
            let generic = GenericFamily::parse(&name.to_ascii_lowercase()).is_some();
            (name, generic)
        };
        if name.is_empty() {
            return None;
        }
        families.push(Family {
            name: if generic {
                name.to_ascii_lowercase()
            } else {
                name
            },
            generic,
        });
    }
    Some(FontFamilyValue::Families(families.into()))
}

pub(super) type Axis = ([u8; 4], f32);

pub(super) enum AxisValue {
    Settings(Arc<[Axis]>),
    Inherit,
}

pub(super) fn parse_axes(source: &str) -> Option<AxisValue> {
    let declaration = format!("font-variation-settings:{source}");
    let declaration = Parser::new(&declaration, Syntax::Css)
        .parse::<Declaration>()
        .ok()?;
    axis_value(&declaration.value)
}

fn axis_value(values: &[ComponentValue<'_>]) -> Option<AxisValue> {
    if let [ComponentValue::InterpolableIdent(value)] = values {
        return match ident(value)?.to_ascii_lowercase().as_str() {
            "normal" | "initial" => Some(AxisValue::Settings(Arc::default())),
            "inherit" | "unset" | "revert" | "revert-layer" => Some(AxisValue::Inherit),
            _ => None,
        };
    }
    let mut axes = BTreeMap::new();
    for entry in values.split(|value| {
        matches!(
            value,
            ComponentValue::Delimiter(Delimiter {
                kind: DelimiterKind::Comma,
                ..
            })
        )
    }) {
        let [ComponentValue::InterpolableStr(tag), ComponentValue::Number(value)] = entry else {
            return None;
        };
        let tag: [u8; 4] = string_value(tag)?.as_bytes().try_into().ok()?;
        if !tag.iter().all(|byte| (0x20..=0x7e).contains(byte)) || !value.value.is_finite() {
            return None;
        }
        axes.insert(tag, value.value);
    }
    Some(AxisValue::Settings(axes.into_iter().collect()))
}

pub(super) fn optical_sizing(value: &str) -> Option<bool> {
    if value.eq_ignore_ascii_case("none") {
        Some(false)
    } else if value.eq_ignore_ascii_case("auto") || value.eq_ignore_ascii_case("initial") {
        Some(true)
    } else {
        None
    }
}

fn unicode_range_list(
    values: &[ComponentValue<'_>],
) -> Result<Option<Vec<(u32, u32)>>, EngineError> {
    if values.len() > 511 {
        return Err(limited("EPUB font-face exceeds 256 Unicode ranges"));
    }
    if values.len().is_multiple_of(2) {
        return Ok(None);
    }
    let mut ranges = Vec::new();
    for (index, value) in values.iter().enumerate() {
        match value {
            ComponentValue::UnicodeRange(range)
                if index.is_multiple_of(2) && range.start <= range.end && range.end <= 0x10ffff =>
            {
                ranges.push((range.start, range.end));
            }
            ComponentValue::Delimiter(Delimiter {
                kind: DelimiterKind::Comma,
                ..
            }) if !index.is_multiple_of(2) => {}
            _ => return Ok(None),
        }
    }
    ranges.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (first, last) in ranges {
        if let Some(range) = merged.last_mut().filter(|range| first <= range.1 + 1) {
            range.1 = range.1.max(last);
        } else {
            merged.push((first, last));
        }
    }
    Ok(Some(merged))
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
        let custom = key.starts_with("--");
        let name = if custom {
            key.to_owned()
        } else {
            key.to_ascii_lowercase()
        };
        let span = declaration
            .value
            .first()
            .zip(declaration.value.last())
            .map(|(first, last)| first.span().start..last.span().end)
            .unwrap_or(0..0);
        let value = source[span.clone()].trim();
        if output.len() >= 512 || value.len() > 4096 {
            return Err(limited("EPUB CSS declarations exceed their budget"));
        }
        let important = declaration.important.is_some();
        if let Some(template) = variables::Template::parse(source, span, &declaration.value) {
            let value = CssValue::Pending(Arc::new(template), Arc::from(name.as_str()));
            for part in shorthand_parts(&name).unwrap_or_else(|| vec![name]) {
                output.push((part, value.clone(), important));
            }
        } else if custom {
            output.push((name, CssValue::Text(Arc::from(value)), important));
        } else if !value.is_empty() && valid_literal(&name, value, &declaration.value) {
            for (part, value) in expand(&name, value) {
                output.push((part, CssValue::Text(Arc::from(value)), important));
            }
        }
    }
    Ok(output)
}

fn valid_literal(name: &str, value: &str, components: &[ComponentValue<'_>]) -> bool {
    match name {
        "font-family" => font_family_value(components).is_some(),
        "font-variation-settings" => axis_value(components).is_some(),
        "font-optical-sizing" => {
            optical_sizing(value).is_some()
                || matches!(
                    value.to_ascii_lowercase().as_str(),
                    "inherit" | "unset" | "revert" | "revert-layer"
                )
        }
        _ => true,
    }
}

fn valid_computed(name: &str, value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    let source = format!("{name}:{value}");
    Parser::new(&source, Syntax::Css)
        .parse::<Declaration>()
        .is_ok_and(|declaration| {
            declaration.span.end == source.len()
                && declaration.important.is_none()
                && valid_literal(name, value, &declaration.value)
        })
}

fn side_names(name: &str) -> Option<[String; 4]> {
    match name {
        "inset" => Some(SIDES.map(str::to_owned)),
        "border-width" => Some(SIDES.map(|side| format!("border-{side}-width"))),
        "border-color" => Some(SIDES.map(|side| format!("border-{side}-color"))),
        "margin" | "padding" => Some(SIDES.map(|side| format!("{name}-{side}"))),
        _ => None,
    }
}

fn border_names() -> impl Iterator<Item = String> {
    SIDES.into_iter().flat_map(|side| {
        ["width", "color", "style"].map(move |name| format!("border-{side}-{name}"))
    })
}

fn shorthand_parts(name: &str) -> Option<Vec<String>> {
    match name {
        "background" => Some(vec!["background-color".to_owned()]),
        "border" => Some(border_names().collect()),
        _ => side_names(name).map(Vec::from),
    }
}

fn expand(name: &str, value: &str) -> Vec<(String, String)> {
    if let Some(names) = side_names(name) {
        let pieces = value.split_ascii_whitespace().collect::<Vec<_>>();
        return sides(&pieces)
            .map(|pieces| names.into_iter().zip(pieces.map(str::to_owned)).collect())
            .unwrap_or_default();
    }
    if name == "background" && color(value).is_some() {
        return vec![("background-color".into(), value.into())];
    }
    if name == "border" {
        let mut width = "0";
        let mut ink = "currentColor";
        let mut style = "none";
        for word in value.split_ascii_whitespace() {
            if points(word, 12.0, [0.0; 2]).is_some() || matches!(word, "thin" | "medium" | "thick")
            {
                width = word;
            } else if color(word).is_some() {
                ink = word;
            } else if matches!(word, "solid" | "none" | "hidden") {
                style = word;
            }
        }
        return border_names()
            .zip([width, ink, style].into_iter().cycle().map(str::to_owned))
            .collect();
    }
    vec![(name.to_owned(), value.to_owned())]
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
    fn index_key(&self) -> Option<(KeyKind, &str)> {
        match self {
            Self::Id(value) => Some((KeyKind::Id, value)),
            Self::Class(value) => Some((KeyKind::Class, value)),
            Self::Tag(value) => Some((KeyKind::Tag, value)),
            _ => None,
        }
    }

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

// For archive edits, inspect every branch regardless of viewport. A declaration
// that is inactive in our preview can still refer to an asset in another reader.
pub(super) fn resource_references(
    source: &str,
    inline: bool,
    check: &mut impl FnMut(&str, bool) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    if source.len() > SHEET_BYTES {
        return Err(limited("EPUB stylesheet exceeds 256 KiB"));
    }
    let inline_source;
    let source = if inline {
        inline_source = format!("{{{source}}}");
        &inline_source
    } else {
        source.trim_start_matches('\u{feff}')
    };
    let source = checked_css(source)?;
    let mut parser = raffia::ParserBuilder::new(&source)
        .options(raffia::ParserOptions {
            try_parsing_value_in_custom_property: true,
            ..Default::default()
        })
        .build();
    let statements = if inline {
        let block = parser.parse::<SimpleBlock>().map_err(css_error)?;
        if block.span.end != source.len() {
            return Err(invalid("EPUB inline CSS has trailing input"));
        }
        block.statements
    } else {
        parser.parse::<Stylesheet>().map_err(css_error)?.statements
    };
    css_references(&statements, check)
}

fn css_error(error: raffia::error::Error) -> EngineError {
    invalid(format!(
        "CSS at byte {}: {:?}",
        error.span.start, error.kind
    ))
}

fn css_references(
    statements: &[Statement<'_>],
    check: &mut impl FnMut(&str, bool) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    for statement in statements {
        match statement {
            Statement::AtRule(rule) => {
                if let Some(AtRulePrelude::Import(import)) = &rule.prelude {
                    match &import.href {
                        ImportPreludeHref::Str(value) => {
                            if let Some(href) = string_value(value) {
                                check(href, true)?;
                            }
                        }
                        ImportPreludeHref::Url(url) => check_url(url, true, check)?,
                    }
                } else if rule.name.name.eq_ignore_ascii_case("import") {
                    return Err(unsupported("checking dependencies of unparsed CSS imports"));
                }
                if let Some(block) = &rule.block {
                    css_references(&block.statements, check)?;
                }
            }
            Statement::QualifiedRule(rule) => css_references(&rule.block.statements, check)?,
            Statement::Declaration(rule) => value_references(
                &rule.value,
                ident(&rule.name).is_some_and(|value| value.starts_with("--")),
                check,
            )?,
            _ => {}
        }
    }
    Ok(())
}

fn check_url(
    url: &Url<'_>,
    stylesheet: bool,
    check: &mut impl FnMut(&str, bool) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    match &url.value {
        Some(UrlValue::Raw(url)) => check(url.value.as_ref(), stylesheet),
        Some(UrlValue::Str(value)) => {
            string_value(value).map_or(Ok(()), |href| check(href, stylesheet))
        }
        None => Ok(()),
        _ => Err(unsupported("checking dependencies of unparsed CSS URLs")),
    }
}

fn value_references(
    values: &[ComponentValue<'_>],
    strings: bool,
    check: &mut impl FnMut(&str, bool) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    for value in values {
        match value {
            ComponentValue::Url(url) => check_url(url, false, check)?,
            ComponentValue::Function(function) => {
                let takes_url_strings = match &function.name {
                    FunctionName::Ident(name) => ident(name).is_some_and(|name| {
                        ["image", "image-set", "-webkit-image-set", "src"]
                            .iter()
                            .any(|key| name.eq_ignore_ascii_case(key))
                    }),
                    _ => false,
                };
                value_references(&function.args, takes_url_strings, check)?;
            }
            ComponentValue::BracketBlock(block) => value_references(&block.value, strings, check)?,
            ComponentValue::InterpolableStr(value) if strings => {
                if let Some(value) = string_value(value) {
                    check(value, false)?;
                }
            }
            ComponentValue::TokenWithSpan(token) => {
                if matches!(
                    &token.token,
                    raffia::token::Token::Ident(_)
                        | raffia::token::Token::Str(_)
                        | raffia::token::Token::UrlRaw(_)
                ) {
                    return Err(unsupported(
                        "checking dependencies of unparsed CSS declaration values",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}
