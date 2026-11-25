//! Compute bounded custom properties and resolve CSS variable references.
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};

use raffia::ast::*;
use roxmltree::Node;

use super::{ident, CssValue};
use crate::{
    epub::{charge, limited},
    EngineError,
};

const MAX_VALUE_BYTES: usize = 4096;
const MAX_WORK: usize = 32 * 1024 * 1024;
const MAX_DEPENDENCY_DEPTH: usize = 64;

pub(super) struct Template {
    parts: Vec<Part>,
    source_len: usize,
}

enum Part {
    Text(String),
    Var {
        name: Option<String>,
        fallback: Option<Template>,
    },
}

impl Template {
    pub(super) fn parse(
        source: &str,
        span: Range<usize>,
        values: &[ComponentValue<'_>],
    ) -> Option<Self> {
        let mut references = Vec::new();
        for value in values {
            collect(value, &mut references);
        }
        if references.is_empty() {
            return None;
        }
        Some(Self::from_references(source, span, references))
    }

    fn from_references(source: &str, span: Range<usize>, references: Vec<&Function<'_>>) -> Self {
        let mut parts = Vec::new();
        let mut cursor = span.start;
        for reference in references {
            if cursor < reference.span.start {
                parts.push(Part::Text(source[cursor..reference.span.start].to_owned()));
            }
            let (name, fallback) = variable(source, reference);
            parts.push(Part::Var { name, fallback });
            cursor = reference.span.end;
        }
        if cursor < span.end {
            parts.push(Part::Text(source[cursor..span.end].to_owned()));
        }
        Self {
            parts,
            source_len: span.len(),
        }
    }

    fn visit(
        &self,
        visit: &mut impl FnMut(&str) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        for part in &self.parts {
            if let Part::Var { name, fallback } = part {
                if let Some(name) = name {
                    visit(name)?;
                }
                if let Some(fallback) = fallback {
                    fallback.visit(visit)?;
                }
            }
        }
        Ok(())
    }

    fn substitute<'a>(
        &self,
        lookup: &impl Fn(&str) -> Option<&'a str>,
        work: &mut usize,
    ) -> Result<Option<String>, EngineError> {
        let mut value = String::new();
        for part in &self.parts {
            charge(
                work,
                1,
                MAX_WORK,
                "EPUB CSS variable work exceeds its limit",
            )?;
            match part {
                Part::Text(text) => append(&mut value, text, work)?,
                Part::Var { name, fallback } => {
                    let Some(name) = name else { return Ok(None) };
                    let fallback_value;
                    let text = if let Some(text) = lookup(name) {
                        text
                    } else if let Some(fallback) = fallback {
                        fallback_value = fallback.substitute(lookup, work)?;
                        let Some(text) = fallback_value.as_deref() else {
                            return Ok(None);
                        };
                        text
                    } else {
                        return Ok(None);
                    };
                    // A replacement produces tokens, so adjacent tokens must
                    // never turn into a number, dimension or identifier.
                    append(&mut value, " ", work)?;
                    append(&mut value, text, work)?;
                    append(&mut value, " ", work)?;
                }
            }
        }
        Ok(Some(value.trim().to_owned()))
    }
}

fn append(value: &mut String, text: &str, work: &mut usize) -> Result<(), EngineError> {
    if value.len() + text.len() > MAX_VALUE_BYTES {
        return Err(limited("EPUB CSS variable value exceeds 4 KiB"));
    }
    charge(
        work,
        text.len(),
        MAX_WORK,
        "EPUB CSS variable work exceeds its limit",
    )?;
    value.push_str(text);
    Ok(())
}

fn collect<'a, 's>(value: &'a ComponentValue<'s>, output: &mut Vec<&'a Function<'s>>) {
    match value {
        ComponentValue::Function(function) => {
            if matches!(&function.name, FunctionName::Ident(name) if ident(name).is_some_and(|value| value.eq_ignore_ascii_case("var")))
            {
                output.push(function);
            } else {
                for arg in &function.args {
                    collect(arg, output);
                }
            }
        }
        ComponentValue::BracketBlock(block) => {
            for child in &block.value {
                collect(child, output);
            }
        }
        ComponentValue::Calc(calc) => {
            collect(&calc.left, output);
            collect(&calc.right, output);
        }
        _ => {}
    }
}

fn variable(source: &str, reference: &Function<'_>) -> (Option<String>, Option<Template>) {
    let [ComponentValue::InterpolableIdent(name), rest @ ..] = reference.args.as_slice() else {
        return (None, None);
    };
    let Some(name) = ident(name).filter(|name| name.starts_with("--") && name.len() > 2) else {
        return (None, None);
    };
    if rest.is_empty() {
        return (Some(name.to_owned()), None);
    }
    let [ComponentValue::Delimiter(delimiter), fallback @ ..] = rest else {
        return (None, None);
    };
    if delimiter.kind != DelimiterKind::Comma {
        return (None, None);
    }
    let mut references = Vec::new();
    for value in fallback {
        collect(value, &mut references);
    }
    let span = delimiter.span.end..reference.span.end - 1;
    (
        Some(name.to_owned()),
        Some(Template::from_references(source, span, references)),
    )
}

type Values = HashMap<String, Option<Arc<str>>>;

struct Layer {
    end: usize,
    serial: usize,
    values: Values,
}

#[derive(Default)]
pub(in crate::epub) struct Scope {
    layers: Vec<Layer>,
    work: usize,
    serial: usize,
}

impl Scope {
    pub(in crate::epub) fn mark(&self) -> usize {
        self.layers.len()
    }

    pub(in crate::epub) fn key(&self) -> usize {
        self.layers.last().map_or(0, |layer| layer.serial)
    }

    pub(in crate::epub) fn restore(&mut self, mark: usize) {
        self.layers.truncate(mark);
    }

    pub(super) fn enter_at(
        &mut self,
        node: Node<'_, '_>,
        declarations: &HashMap<String, CssValue>,
        base: usize,
    ) -> Result<(), EngineError> {
        let span = node.range();
        while self.layers[base..]
            .last()
            .is_some_and(|layer| layer.end <= span.start)
        {
            self.layers.pop();
        }
        if declarations.is_empty() {
            return Ok(());
        }
        let mut resolver = Resolver {
            declarations,
            inherited: &self.layers,
            results: HashMap::new(),
            cyclic: HashSet::new(),
            work: &mut self.work,
        };
        for name in declarations.keys() {
            if reaches(
                name,
                name,
                declarations,
                &mut HashSet::new(),
                resolver.work,
                0,
            )? {
                resolver.cyclic.insert(name.clone());
            }
        }
        for name in declarations.keys() {
            resolver.compute(name, 0)?;
        }
        self.serial += 1;
        self.layers.push(Layer {
            end: span.end,
            serial: self.serial,
            values: resolver.results,
        });
        Ok(())
    }

    pub(super) fn substitute(
        &mut self,
        template: &Template,
    ) -> Result<Option<String>, EngineError> {
        template.substitute(&|name| lookup(&self.layers, name), &mut self.work)
    }
}

fn lookup<'a>(layers: &'a [Layer], name: &str) -> Option<&'a str> {
    layers
        .iter()
        .rev()
        .find_map(|layer| layer.values.get(name))
        .and_then(Option::as_deref)
}

// A definition is cyclic only when a dependency path reaches it again.
// References in unused fallbacks also participate in this graph.
fn reaches(
    root: &str,
    name: &str,
    declarations: &HashMap<String, CssValue>,
    visited: &mut HashSet<String>,
    work: &mut usize,
    depth: usize,
) -> Result<bool, EngineError> {
    let Some(CssValue::Pending(template, _)) = declarations.get(name) else {
        return Ok(false);
    };
    if !visited.insert(name.to_owned()) {
        return Ok(false);
    }
    if depth == MAX_DEPENDENCY_DEPTH {
        return Err(limited("EPUB CSS variable dependencies exceed depth 64"));
    }
    charge(
        work,
        template.source_len,
        MAX_WORK,
        "EPUB CSS variable work exceeds its limit",
    )?;
    let mut found = false;
    template.visit(&mut |dependency| {
        if !found {
            found = dependency == root
                || reaches(root, dependency, declarations, visited, work, depth + 1)?;
        }
        Ok(())
    })?;
    Ok(found)
}

struct Resolver<'a> {
    declarations: &'a HashMap<String, CssValue>,
    inherited: &'a [Layer],
    results: Values,
    cyclic: HashSet<String>,
    work: &'a mut usize,
}

impl Resolver<'_> {
    fn compute(&mut self, name: &str, depth: usize) -> Result<(), EngineError> {
        if self.results.contains_key(name) {
            return Ok(());
        }
        let Some(value) = self.declarations.get(name).cloned() else {
            return Ok(());
        };
        charge(
            self.work,
            1,
            MAX_WORK,
            "EPUB CSS variable work exceeds its limit",
        )?;
        if self.cyclic.contains(name) {
            self.results.insert(name.to_owned(), None);
            return Ok(());
        }
        if depth == MAX_DEPENDENCY_DEPTH {
            return Err(limited("EPUB CSS variable dependencies exceed depth 64"));
        }
        let value = match value {
            CssValue::Text(text) => {
                if text.eq_ignore_ascii_case("initial") {
                    None
                } else if ["inherit", "unset", "revert", "revert-layer"]
                    .iter()
                    .any(|keyword| text.eq_ignore_ascii_case(keyword))
                {
                    lookup(self.inherited, name).map(Arc::from)
                } else {
                    Some(text.clone())
                }
            }
            CssValue::Pending(template, _) => {
                charge(
                    self.work,
                    template.source_len,
                    MAX_WORK,
                    "EPUB CSS variable work exceeds its limit",
                )?;
                template.visit(&mut |dependency| self.compute(dependency, depth + 1))?;
                template
                    .substitute(
                        &|name| {
                            self.results
                                .get(name)
                                .map_or_else(|| lookup(self.inherited, name), Option::as_deref)
                        },
                        self.work,
                    )?
                    .map(Arc::from)
            }
        };
        self.results.insert(name.to_owned(), value);
        Ok(())
    }
}
