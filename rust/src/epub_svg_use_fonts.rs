//! Specialize variable font requests in native SVG use instances.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use roxmltree::{Attribute, Node, NodeId};

use super::{
    content, style_insertion,
    svg_fonts::{CssFont, Families},
    use_refs, NS, XLINK,
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
    families: &mut Families,
) -> Result<(Vec<u8>, Aliases), EngineError> {
    let xml = parse_xml(&source)?;
    let root = xml.root_element();
    let mut instances = Instances::new(xml.input_text(), root, families)?;
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
}

struct Instances<'a> {
    source: &'a str,
    families: &'a mut Families,
    ids: HashMap<&'a str, Node<'a, 'a>>,
    styles: HashMap<NodeId, NodeStyle>,
    chosen: HashMap<(NodeId, CssFont), Option<String>>,
    occupied: HashSet<String>,
    aliases: Aliases,
    next_id: usize,
    definitions: String,
    visits: usize,
}

#[derive(Default)]
struct Plan {
    edits: Vec<Edit>,
    changes_typography: bool,
}

impl<'a> Instances<'a> {
    fn new(
        source: &'a str,
        root: Node<'a, 'a>,
        families: &'a mut Families,
    ) -> Result<Self, EngineError> {
        // These are the same bounds as the native, pre-specialization use graph.
        use_refs::validate_use_graph(root)?;
        let mut ids = HashMap::new();
        let mut styles = HashMap::<_, NodeStyle>::new();
        for node in root.descendants().filter(Node::is_element) {
            if let Some(id) = node.attribute("id") {
                // The graph check already rejects duplicate IDs if referenced.
                ids.entry(id).or_insert(node);
            }
            let mut declarations = node
                .attribute("style")
                .map(css::svg_font_declarations)
                .transpose()?
                .unwrap_or_default();
            for property in FONT_PROPERTIES {
                if let Some(raw) = node.attribute(property) {
                    declarations
                        .entry(property.to_owned())
                        .or_insert_with(|| raw.to_owned());
                }
            }
            let parent = node
                .parent_element()
                .and_then(|parent| styles.get(&parent.id()));
            let inherited = parent.map_or_else(CssFont::default, |style| style.original.clone());
            let original = compute(inherited, &declarations);
            styles.insert(
                node.id(),
                NodeStyle {
                    declarations,
                    original,
                },
            );
        }
        Ok(Self {
            source,
            families,
            occupied: ids.keys().map(|id| (*id).to_owned()).collect(),
            ids,
            styles,
            chosen: HashMap::new(),
            aliases: HashMap::new(),
            next_id: 0,
            definitions: String::new(),
            visits: 0,
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
        for node in root.descendants().filter(Node::is_element) {
            charge(
                &mut self.visits,
                1,
                WORK_LIMIT,
                "SVG font instance expansion exceeds its work budget",
            )?;
            let entry = self
                .styles
                .get(&node.id())
                .ok_or_else(|| invalid("missing SVG font declarations"))?;
            let original = entry.original.clone();
            let font = if let Some(inherited) = inherited {
                let parent = node
                    .parent_element()
                    .and_then(|parent| current.get(&parent.id()));
                let font = compute(parent.unwrap_or(inherited).clone(), &entry.declarations);
                current.insert(node.id(), font.clone());
                font
            } else {
                original.clone()
            };
            if matches!(node.tag_name().name(), "text" | "tspan" | "a") {
                if font != original
                    && (Families::is_variable(&font) || Families::is_variable(&original))
                {
                    plan.changes_typography = true;
                }
                if let Some(request) = self.families.variant(&font)? {
                    plan.edits.push(self.select(node, &request)?);
                }
            }
            if node.has_tag_name((NS, "use")) {
                let href = use_href(node).ok_or_else(|| invalid("SVG use has no href"))?;
                let target = self
                    .ids
                    .get(use_refs::href_id(href.value())?)
                    .copied()
                    .ok_or_else(|| invalid("SVG use references a missing ID"))?;
                let choice = self.template(target, &font, depth + 1)?;
                if inherited.is_some() && font != original {
                    let original_choice = self.template(target, &original, depth + 1)?;
                    plan.changes_typography |= choice != original_choice;
                }
                if let Some(id) = choice {
                    plan.edits.push((href.range_value(), format!("#{id}")));
                }
            }
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
            return Err(limited(
                "SVG font instance expansion exceeds its depth budget",
            ));
        }
        let original_parent = root
            .parent_element()
            .and_then(|parent| self.styles.get(&parent.id()));
        if original_parent.is_some_and(|style| &style.original == inherited) {
            return Ok(None);
        }
        let key = (root.id(), inherited.clone());
        if let Some(choice) = self.chosen.get(&key) {
            return Ok(choice.clone());
        }
        let mut plan = self.plan(root, Some(inherited), depth)?;
        let choice = if plan.changes_typography {
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
        for node in root.descendants().filter(Node::is_element) {
            if let Some(original) = node.attribute_node("id") {
                let name = if node == root {
                    id.clone()
                } else {
                    self.private_id()
                };
                self.aliases
                    .insert(name.clone(), original.value().to_owned());
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
        append(&mut self.definitions, template, XML_BYTES)?;
        Ok(id)
    }

    fn private_id(&mut self) -> String {
        loop {
            let id = format!("pydoccraft-svg-use-font-{}", self.next_id);
            self.next_id += 1;
            if self.occupied.insert(id.clone()) {
                return id;
            }
        }
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

fn compute(mut font: CssFont, declarations: &css::Properties) -> CssFont {
    for property in FONT_PROPERTIES {
        if let Some(value) = declarations.get(property) {
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
