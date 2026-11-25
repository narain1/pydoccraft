//! Specialize inherited CSS values in native SVG use instances.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use roxmltree::{Attribute, Node, NodeId};

use super::{
    canonical_style_value, content, effects, fragment_id, style_insertion,
    svg_fonts::{CssFont, Families},
    svg_property, use_refs, NS, XLINK,
};
use crate::{
    author_epub::escape_attribute,
    epub::{
        charge, css, invalid, limited, parse_xml,
        structure::{append, apply_edits},
        XML_BYTES,
    },
    EngineError,
};

const WORK_LIMIT: usize = 150_000;
const FONT_PROPERTIES: [&str; 6] = [
    "font-family",
    "font-weight",
    "font-stretch",
    "font-style",
    "font-variation-settings",
    "font-optical-sizing",
];

type Edit = (Range<usize>, String);
pub(super) type Aliases = HashMap<String, String>;

pub(super) fn specialize(
    source: Vec<u8>,
    original: Node<'_, '_>,
    families: &mut Families,
    cascade: &mut css::Cascade,
    variables: Option<(Vec<css::InstanceStyle>, css::VariableScope)>,
) -> Result<(Vec<u8>, Aliases), EngineError> {
    let xml = parse_xml(&source)?;
    let root = xml.root_element();
    let mut instances = Instances::new(
        xml.input_text(),
        root,
        original,
        families,
        cascade,
        variables,
    )?;
    let mut plan = instances.plan(root, None, 0)?;
    if !instances.definitions.is_empty() {
        let end = root.range().end;
        let close = instances.source[..end]
            .rfind("</")
            .ok_or_else(|| invalid("missing SVG root end tag"))?;
        plan.edits.push((
            close..close,
            format!(r#"<defs xmlns="{NS}">{}</defs>"#, instances.definitions),
        ));
    }
    let Instances {
        source: xml,
        aliases,
        ..
    } = instances;
    let output = if plan.edits.is_empty() {
        source
    } else {
        apply_edits(xml, plan.edits, XML_BYTES)?.into_bytes()
    };
    Ok((output, aliases))
}

struct NodeStyle {
    declarations: css::Properties,
    original: CssFont,
    variables: Option<css::InstanceStyle>,
}

struct Instances<'a, 'original, 'input> {
    source: &'a str,
    families: &'a mut Families,
    ids: HashMap<&'a str, Node<'a, 'a>>,
    root: Node<'a, 'a>,
    styles: HashMap<NodeId, NodeStyle>,
    originals: HashMap<NodeId, Node<'original, 'input>>,
    shadow_styles: HashMap<(NodeId, NodeId), css::InstanceStyle>,
    cascade: &'a mut css::Cascade,
    chosen: HashMap<(NodeId, CssFont, usize), Option<String>>,
    scope: Option<css::VariableScope>,
    effects: Option<effects::Effects<'a, 'a>>,
    occupied: HashSet<String>,
    aliases: Aliases,
    next_id: usize,
    definitions: String,
    visits: usize,
    style_bytes: usize,
}

#[derive(Default)]
struct Plan {
    edits: Vec<Edit>,
    changes_typography: bool,
}

impl<'a, 'original, 'input> Instances<'a, 'original, 'input> {
    fn new(
        source: &'a str,
        root: Node<'a, 'a>,
        original: Node<'original, 'input>,
        families: &'a mut Families,
        cascade: &'a mut css::Cascade,
        variables: Option<(Vec<css::InstanceStyle>, css::VariableScope)>,
    ) -> Result<Self, EngineError> {
        // These are the same bounds as the native, pre-specialization use graph.
        use_refs::validate_use_graph(root)?;
        let mut ids = HashMap::new();
        let mut styles = HashMap::<_, NodeStyle>::new();
        let mut originals = HashMap::new();
        let mut original = original
            .descendants()
            .filter(|node| node.is_element() && !node.has_tag_name((NS, "style")));
        let (mut variables, scope) = variables.map_or((None, None), |(styles, scope)| {
            (Some(styles.into_iter()), Some(scope))
        });
        for node in root.descendants().filter(Node::is_element) {
            if scope.is_some() {
                let raw = original
                    .next()
                    .ok_or_else(|| invalid("missing SVG instance source node"))?;
                originals.insert(node.id(), raw);
            }
            if let Some(id) = node.attribute("id") {
                // The graph check already rejects duplicate IDs if referenced.
                ids.entry(id).or_insert(node);
            }
            let declarations = node
                .attribute("style")
                .map(css::svg_declarations)
                .transpose()?
                .unwrap_or_default();
            let parent = node
                .parent_element()
                .and_then(|parent| styles.get(&parent.id()));
            let inherited = parent.map_or_else(CssFont::default, |style| style.original.clone());
            let original = compute(inherited, &declarations, node);
            styles.insert(
                node.id(),
                NodeStyle {
                    declarations,
                    original,
                    variables: variables.as_mut().and_then(Iterator::next),
                },
            );
        }
        if scope.is_some()
            && (original.next().is_some()
                || variables.as_mut().and_then(Iterator::next).is_some()
                || styles.values().any(|style| style.variables.is_none()))
        {
            return Err(invalid("mismatched SVG instance declarations"));
        }
        Ok(Self {
            source,
            root,
            families,
            occupied: ids.keys().map(|id| (*id).to_owned()).collect(),
            ids,
            styles,
            originals,
            shadow_styles: HashMap::new(),
            cascade,
            chosen: HashMap::new(),
            scope,
            effects: None,
            aliases: HashMap::new(),
            next_id: 0,
            definitions: String::new(),
            visits: 0,
            style_bytes: 0,
        })
    }

    fn plan(
        &mut self,
        root: Node<'a, 'a>,
        inherited: Option<&CssFont>,
        depth: usize,
    ) -> Result<Plan, EngineError> {
        let mut plan = Plan::default();
        let mut current = HashMap::<NodeId, CssFont>::new();
        let instance_scope =
            inherited.and_then(|_| self.scope.as_ref().map(css::VariableScope::mark));
        for node in root.descendants().filter(Node::is_element) {
            charge(
                &mut self.visits,
                1,
                WORK_LIMIT,
                "SVG instance expansion exceeds its work budget",
            )?;
            let entry = self
                .styles
                .get(&node.id())
                .ok_or_else(|| invalid("missing SVG instance style declarations"))?;
            let style = match &mut self.scope {
                Some(scope) if instance_scope.is_some() => {
                    let cache_key = (root.id(), node.id());
                    let declarations = match self.shadow_styles.entry(cache_key) {
                        std::collections::hash_map::Entry::Occupied(cached) => cached.into_mut(),
                        std::collections::hash_map::Entry::Vacant(cache) => {
                            let source = self.originals[&node.id()];
                            let instance_root = self.originals[&root.id()];
                            cache.insert(self.cascade.shadow_style(
                                source,
                                instance_root.id(),
                                svg_property,
                            )?)
                        }
                    };
                    let mut style = css::Properties::new();
                    for (property, value) in declarations.compute(scope, node, instance_scope)? {
                        let value = match value {
                            Some(value) if property == "font-family" => {
                                self.families.encode(&value)?
                            }
                            Some(value) => canonical_style_value(
                                self.root,
                                node,
                                property,
                                value,
                                &mut self.effects,
                            )?,
                            None => None,
                        };
                        if let Some(value) = value {
                            style.insert(property.to_owned(), value);
                        }
                    }
                    for &property in effects::paint_properties(node.tag_name().name()) {
                        if let Some(value) = node.attribute(property) {
                            if !style.contains_key(property) {
                                style.insert(
                                    property.to_owned(),
                                    effects::filter_paint(property, value)?,
                                );
                            }
                        }
                    }
                    Some(style)
                }
                Some(scope) => {
                    if let Some(styles) = &entry.variables {
                        styles.compute(scope, node, None)?;
                    }
                    None
                }
                None => None,
            };
            let original = entry.original.clone();
            let font = if let Some(inherited) = inherited {
                let parent = node
                    .parent_element()
                    .and_then(|parent| current.get(&parent.id()));
                let declarations = style.as_ref().unwrap_or(&entry.declarations);
                let font = compute(parent.unwrap_or(inherited).clone(), declarations, node);
                current.insert(node.id(), font.clone());
                font
            } else {
                original.clone()
            };
            let mut request = None;
            if matches!(node.tag_name().name(), "text" | "tspan" | "a")
                && self.families.tracks_variations()
            {
                if font != original
                    && (Families::is_variable(&font) || Families::is_variable(&original))
                {
                    plan.changes_typography = true;
                }
                request = self.families.variant(&font)?;
            }
            let style_edit = if let Some(style) = style {
                if node.has_attribute("style") || !style.is_empty() || request.is_some() {
                    Some(self.select_properties(node, style, request.as_deref())?)
                } else {
                    None
                }
            } else {
                request
                    .as_deref()
                    .map(|request| self.select(node, request))
                    .transpose()?
            };
            if let Some(edit) = style_edit {
                charge(
                    &mut self.style_bytes,
                    edit.1.len(),
                    XML_BYTES,
                    "SVG instance styles exceed 8 MiB",
                )?;
                plan.edits.push(edit);
            }
            if node.has_tag_name((NS, "use")) {
                let href = use_href(node).ok_or_else(|| invalid("SVG use has no href"))?;
                let target = self
                    .ids
                    .get(use_refs::href_id(href.value())?)
                    .copied()
                    .ok_or_else(|| invalid("SVG use references a missing ID"))?;
                let choice = self.template(target, &font, depth + 1)?;
                if self.scope.is_none() && inherited.is_some() && font != original {
                    let original_choice = self.template(target, &original, depth + 1)?;
                    plan.changes_typography |= choice != original_choice;
                }
                if let Some(id) = choice {
                    plan.edits.push((href.range_value(), format!("#{id}")));
                }
            }
        }
        if let Some((scope, start)) = self.scope.as_mut().zip(instance_scope) {
            scope.restore(start);
        }
        Ok(plan)
    }

    fn template(
        &mut self,
        root: Node<'a, 'a>,
        inherited: &CssFont,
        depth: usize,
    ) -> Result<Option<String>, EngineError> {
        if depth > use_refs::MAX_DEPTH {
            return Err(limited("SVG instance expansion exceeds its depth budget"));
        }
        let original_parent = root
            .parent_element()
            .and_then(|parent| self.styles.get(&parent.id()));
        if self.scope.is_none() && original_parent.is_some_and(|style| &style.original == inherited)
        {
            return Ok(None);
        }
        let key = (
            root.id(),
            inherited.clone(),
            self.scope.as_ref().map_or(0, css::VariableScope::key),
        );
        if let Some(choice) = self.chosen.get(&key) {
            return Ok(choice.clone());
        }
        let mut plan = self.plan(root, Some(inherited), depth)?;
        let choice = if self.scope.is_some() || plan.changes_typography {
            let id = self.materialize(root, &mut plan.edits)?;
            Some(id)
        } else {
            None
        };
        self.chosen.insert(key, choice.clone());
        Ok(choice)
    }

    fn materialize(
        &mut self,
        root: Node<'a, 'a>,
        edits: &mut Vec<Edit>,
    ) -> Result<String, EngineError> {
        let id = self.private_id();
        let mut renamed = HashMap::new();
        for node in root.descendants().filter(Node::is_element) {
            if let Some(original) = node.attribute_node("id") {
                let name = if node == root {
                    id.clone()
                } else {
                    self.private_id()
                };
                self.aliases
                    .insert(name.clone(), original.value().to_owned());
                if renamed.insert(original.value(), name.clone()).is_some() && self.scope.is_some()
                {
                    return Err(invalid("duplicate SVG ID inside a styled instance"));
                }
                edits.push((original.range_value(), name));
            }
        }
        let span = root.range();
        for (range, _) in edits.iter_mut() {
            range.start -= span.start;
            range.end -= span.start;
        }
        let template = apply_edits(&self.source[span], std::mem::take(edits), XML_BYTES)?;
        let template = content::namespace_complete(root, template.as_bytes())?;
        let template = std::str::from_utf8(&template).map_err(invalid)?;
        if self.scope.is_some() {
            let template = remap_paint_references(template, &renamed)?;
            append(&mut self.definitions, &template, XML_BYTES)?;
        } else {
            append(&mut self.definitions, template, XML_BYTES)?;
        }
        Ok(id)
    }

    fn private_id(&mut self) -> String {
        loop {
            let id = format!("pydoccraft-svg-instance-{}", self.next_id);
            self.next_id += 1;
            if self.occupied.insert(id.clone()) {
                return id;
            }
        }
    }

    fn select_properties(
        &self,
        node: Node<'_, '_>,
        mut properties: css::Properties,
        family: Option<&str>,
    ) -> Result<Edit, EngineError> {
        let family = family
            .map(str::to_owned)
            .or_else(|| properties.remove("font-family"));
        let mut entries: Vec<_> = properties
            .into_iter()
            .filter(|(key, _)| key != "font-family")
            .collect();
        entries.sort_unstable_by(|(key, _), (other, _)| key.cmp(other));
        // The fallback pass expects the selected family to lead the style.
        let entries = family
            .into_iter()
            .map(|family| ("font-family".to_owned(), family))
            .chain(entries);
        let mut style = String::new();
        for (key, value) in entries {
            style.push_str(&key);
            style.push(':');
            style.push_str(&value);
            style.push(';');
        }
        let range = match node.attribute_node("style") {
            Some(style) => style.range(),
            None => style_insertion(self.source.as_bytes(), node)?,
        };
        Ok((range, format!(r#" style="{}""#, escape_attribute(&style))))
    }

    fn select(&self, node: Node<'_, '_>, alias: &str) -> Result<Edit, EngineError> {
        let (range, prior) = match node.attribute_node("style") {
            Some(style) => (style.range(), style.value()),
            None => (style_insertion(self.source.as_bytes(), node)?, ""),
        };
        // The fallback pass reads the private family at the start of a style.
        let prior = prior
            .strip_prefix("font-family:")
            .and_then(|style| style.split_once(';'))
            .map_or(prior, |(_, tail)| tail);
        let style = escape_attribute(&format!("font-family:{alias};{prior}"));
        Ok((range, format!(r#" style="{style}""#)))
    }
}

fn remap_paint_references(
    template: &str,
    ids: &HashMap<&str, String>,
) -> Result<String, EngineError> {
    let mut edits = Vec::new();
    let tree = parse_xml(template.as_bytes())?;
    for node in tree.descendants().filter(Node::is_element) {
        if effects::is_gradient(node.tag_name().name()) {
            if let Some(href) = use_href(node) {
                if let Some(id) = fragment_id(href.value()).and_then(|id| ids.get(id)) {
                    edits.push((href.range_value(), format!("#{id}")));
                }
            }
        }
        for attr in node.attributes().filter(|attr| {
            attr.namespace().is_none()
                && matches!(
                    attr.name(),
                    "style"
                        | "fill"
                        | "stroke"
                        | "clip-path"
                        | "mask"
                        | "filter"
                        | "marker"
                        | "marker-start"
                        | "marker-mid"
                        | "marker-end"
                )
        }) {
            let raw = attr.value();
            let lower = raw.to_ascii_lowercase();
            let mut start = 0;
            let mut output = String::new();
            while let Some((begin, end)) = lower[start..].find("url(").and_then(|begin| {
                let begin = begin + start + 4;
                let end = lower[begin..].find(')')? + begin;
                Some((begin, end))
            }) {
                output.push_str(&raw[start..begin]);
                let url = raw[begin..end].trim().trim_matches(['\'', '"']);
                if let Some(id) = fragment_id(url).and_then(|id| ids.get(id)) {
                    output.push('#');
                    output.push_str(id);
                } else {
                    output.push_str(&raw[begin..end]);
                }
                start = end;
            }
            if !output.is_empty() {
                output.push_str(&raw[start..]);
                if output != raw {
                    edits.push((attr.range_value(), escape_attribute(&output)));
                }
            }
        }
    }
    apply_edits(template, edits, XML_BYTES)
}

fn compute(mut font: CssFont, declarations: &css::Properties, node: Node<'_, '_>) -> CssFont {
    for property in FONT_PROPERTIES {
        if let Some(value) = declarations
            .get(property)
            .map(String::as_str)
            .or_else(|| node.attribute(property))
        {
            font.apply(property, value);
        }
    }
    font
}

fn use_href<'a, 'input>(node: Node<'a, 'input>) -> Option<Attribute<'a, 'input>> {
    node.attributes()
        .find(|attr| attr.name() == "href" && attr.namespace().is_none())
        .or_else(|| node.attribute_node((XLINK, "href")))
}
