//! Retry mixed-font text when native glyph replacement cannot match shaping clusters.
use std::collections::{HashMap, HashSet};

use resvg::usvg::{
    self,
    fontdb::{Database, ID},
};
use roxmltree::Node;
use unicode_segmentation::UnicodeSegmentation;

use super::{
    length::{prepare_lengths, TextLength},
    CssFont, Resolver, Selection, MAX_FACES, MAX_WORK,
};
use crate::{
    author_epub::{escape_attribute, escape_text},
    epub::{
        charge, invalid, limited, parse_xml,
        structure::{append, apply_edits},
        svg::NS,
        unsupported, XML_BYTES,
    },
    EngineError,
};

pub(in crate::epub::svg) struct FallbackRuns {
    pub(in crate::epub::svg) source: String,
    pub(in crate::epub::svg) lengths: Vec<TextLength>,
}

impl Resolver {
    pub(in crate::epub::svg) fn split_fallback_runs(
        &self,
        source: &[u8],
        tree: &usvg::Tree,
    ) -> Result<Option<FallbackRuns>, EngineError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| invalid("EPUB SVG font selection failed"))?;
        let missing = state.missing(tree);
        if missing.is_empty() && state.requests.is_empty() {
            return Ok(None);
        }
        let source = std::str::from_utf8(source).map_err(invalid)?;
        let xml = parse_xml(source.as_bytes())?;
        let mut edits = Vec::new();
        let mut bytes = 0;
        let mut length_roots = HashMap::new();
        let mut length_owners = HashSet::new();
        let mut ids = HashMap::new();
        for id in xml.descendants().filter_map(|node| node.attribute("id")) {
            ids.entry(id)
                .and_modify(|duplicate| *duplicate = true)
                .or_insert(false);
        }
        let mut next_id = 0;
        for node in xml.descendants().filter(Node::is_text) {
            let Some(parent) = node.parent_element() else {
                continue;
            };
            let Some(family) = effective_family(parent) else {
                continue;
            };
            let Some(base) = state.family_plans.get(family).copied().flatten() else {
                continue;
            };
            let Some(text) = node.text() else { continue };
            let repair = text.chars().any(|ch| missing.contains(&(base, ch)));
            let request = state.requests.get(family).cloned();
            let Some(candidates) = state.candidates.get(&base).cloned() else {
                continue;
            };
            let variable = request.as_ref().is_some_and(|request| {
                candidates
                    .iter()
                    .any(|id| request.settings(state.variants.get(id)) != request.author_settings())
            });
            if !repair && !variable {
                continue;
            }
            if !parent
                .ancestors()
                .any(|node| node.has_tag_name((NS, "text")))
            {
                continue;
            }
            let Some(runs) = state.select_runs(text, base, &candidates, tree.fontdb())? else {
                if variable {
                    return Err(unsupported(
                        "EPUB SVG variable-font run could not retain all glyphs",
                    ));
                }
                continue;
            };
            if !repair
                && request.as_ref().is_none_or(|request| {
                    runs.iter().all(|(id, _)| {
                        request.settings(state.variants.get(id)) == request.author_settings()
                    })
                })
            {
                continue;
            }
            let can_keep_owner = parent
                .ancestors()
                .skip(1)
                .filter(Node::is_element)
                .find(|ancestor| {
                    ancestor.has_attribute("textLength") || ancestor.has_tag_name((NS, "text"))
                })
                .is_none_or(|ancestor| !ancestor.has_attribute("textLength"));
            let (range, replacement) = match (runs.as_slice(), parent.attribute_node("style")) {
                ([(id, _)], Some(style))
                    if variable
                        && can_keep_owner
                        && parent
                            .children()
                            .filter(|node| node.is_element() || node.is_text())
                            .count()
                            == 1 =>
                {
                    // Keep layout attributes such as textLength on their original element.
                    let (family, axes) = state.resolved_style(*id, request.as_ref())?;
                    let tail = style
                        .value()
                        .strip_prefix("font-family:")
                        .and_then(|style| style.split_once(';'))
                        .map_or(style.value(), |(_, style)| style);
                    let merged = format!("font-family:{family};{tail}{axes}");
                    (
                        style.range(),
                        format!(r#" style="{}""#, escape_attribute(&merged)),
                    )
                }
                _ => {
                    let (root, owner) = split_text(parent)?;
                    if let Some(owner) = owner.filter(|owner| length_owners.insert(owner.id())) {
                        length_roots
                            .entry(root.id())
                            .or_insert((root, Vec::new()))
                            .1
                            .push(owner);
                    }
                    (node.range(), state.encode_runs(runs, request.as_ref())?)
                }
            };
            charge(
                &mut bytes,
                replacement.len(),
                XML_BYTES,
                "EPUB SVG fallback spans exceed 8 MiB",
            )?;
            edits.push((range, replacement));
        }
        if edits.is_empty() {
            return if missing.is_empty() {
                Ok(None)
            } else {
                Err(unsupported(
                    "EPUB SVG mixed-font text could not retain all glyphs",
                ))
            };
        }
        let mut owners = length_roots.into_values().collect::<Vec<_>>();
        owners.sort_unstable_by_key(|(root, _)| root.range().start);
        let lengths = owners
            .into_iter()
            .map(|(root, owners)| {
                prepare_lengths(root, owners, &ids, &mut next_id, source, &mut edits)
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(FallbackRuns {
            source: apply_edits(source, edits, XML_BYTES)?,
            lengths,
        }))
    }

    pub(in crate::epub::svg) fn check_rendered(
        &self,
        tree: &usvg::Tree,
    ) -> Result<(), EngineError> {
        self.check()?;
        let state = self
            .0
            .lock()
            .map_err(|_| invalid("EPUB SVG font selection failed"))?;
        if !state.missing(tree).is_empty() {
            return Err(unsupported(
                "EPUB SVG mixed-font text could not retain all glyphs",
            ));
        }
        Ok(())
    }
}

impl Selection {
    fn missing(&self, tree: &usvg::Tree) -> HashSet<(ID, char)> {
        if self.attempted.is_empty() && self.direct_fonts.is_empty() {
            return HashSet::new();
        }
        let mut seen = HashSet::new();
        let mut missing = HashSet::new();
        inspect_subroot(tree.root(), &mut seen, &mut missing, self);
        missing
    }

    fn select_runs<'a>(
        &mut self,
        text: &'a str,
        base: ID,
        candidates: &[ID],
        db: &Database,
    ) -> Result<Option<Vec<(ID, &'a str)>>, EngineError> {
        let mut runs: Vec<(ID, &str)> = Vec::new();
        let mut last = candidates[0];
        let mut start = 0;
        for &id in candidates {
            self.characters(id, db)?;
        }
        for (index, cluster) in text.grapheme_indices(true) {
            charge(
                &mut self.comparison_work,
                cluster.len().saturating_mul(candidates.len()),
                MAX_WORK,
                "EPUB SVG font fallback exceeds its work budget",
            )?;
            let has_cluster = |id| {
                cluster.chars().all(|ch| {
                    matches!(ch, '\u{200c}' | '\u{200d}' | '\u{fe00}'..='\u{fe0f}' | '\u{e0100}'..='\u{e01ef}')
                        || ch.is_control() || self.supports(db, id, ch.into())
                })
            };
            let selected = candidates
                .iter()
                .copied()
                .find(|&id| has_cluster(id))
                .or_else(|| {
                    cluster
                        .chars()
                        .filter_map(|ch| self.attempted.get(&(base, ch)).copied())
                        .find(|&id| has_cluster(id))
                });
            let Some(id) = selected else { return Ok(None) };
            if id != last {
                if index != 0 {
                    runs.push((last, &text[start..index]));
                }
                start = index;
                last = id;
            }
        }
        runs.push((last, &text[start..]));
        Ok(Some(runs))
    }

    fn encode_runs(
        &mut self,
        runs: Vec<(ID, &str)>,
        request: Option<&CssFont>,
    ) -> Result<String, EngineError> {
        let mut output = String::new();
        for (id, text) in runs {
            let (family, axes) = self.resolved_style(id, request)?;
            let style = if axes.is_empty() {
                String::new()
            } else {
                format!(r#" style="{}""#, escape_attribute(&axes))
            };
            let span = format!(
                r#"<tspan xmlns="{}" font-family="{family}"{style}>{}</tspan>"#,
                NS,
                escape_text(text),
            );
            append(&mut output, &span, XML_BYTES)?;
        }
        Ok(output)
    }

    fn resolved_style(
        &mut self,
        id: ID,
        request: Option<&CssFont>,
    ) -> Result<(String, String), EngineError> {
        let next = self.direct_fonts.len();
        if !self.direct_fonts.contains_key(&id) && next >= MAX_FACES {
            return Err(limited("EPUB SVG font fallback exceeds 512 face variants"));
        }
        let alias = self
            .direct_fonts
            .entry(id)
            .or_insert_with(|| format!("doccraft-svg-resolved-{next}"))
            .clone();
        self.direct_names.insert(alias.clone(), id);
        let mut style = String::new();
        if let Some(request) = request {
            use std::fmt::Write;
            let settings = request.settings(self.variants.get(&id));
            style.push_str("font-variation-settings:");
            if settings.is_empty() {
                style.push_str("normal");
            }
            for (index, (tag, setting)) in settings.iter().enumerate() {
                let tag = std::str::from_utf8(tag).map_err(invalid)?;
                if tag.contains('\\') || tag.contains('"') && tag.contains('\'') {
                    return Err(unsupported("EPUB SVG variation axis cannot be encoded"));
                }
                if index != 0 {
                    style.push(',');
                }
                let quote = if tag.contains('"') { '\'' } else { '"' };
                write!(style, "{quote}{tag}{quote} {}", f32::from_bits(*setting))
                    .map_err(invalid)?;
            }
            style.push(';');
        }
        Ok((alias, style))
    }
}

fn inspect_subroot(
    group: &usvg::Group,
    seen: &mut HashSet<*const usvg::Group>,
    missing: &mut HashSet<(ID, char)>,
    state: &Selection,
) {
    if !seen.insert(group as *const usvg::Group) {
        return;
    }
    for child in group.children() {
        match child {
            usvg::Node::Group(group) => inspect_subroot(group, seen, missing, state),
            usvg::Node::Text(text) => {
                for span in text.layouted().iter().filter(|span| span.visible) {
                    for glyph in span
                        .positioned_glyphs
                        .iter()
                        .filter(|glyph| glyph.id.0 == 0)
                    {
                        for ch in glyph.text.chars() {
                            if state.attempted.contains_key(&(glyph.font, ch))
                                || state.direct_fonts.contains_key(&glyph.font)
                            {
                                missing.insert((glyph.font, ch));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        child.subroots(|group| inspect_subroot(group, seen, missing, state));
    }
}

type TextContext<'a, 'input> = (Node<'a, 'input>, Option<Node<'a, 'input>>);

fn split_text<'a, 'input>(
    parent: Node<'a, 'input>,
) -> Result<TextContext<'a, 'input>, EngineError> {
    let mut owner = None;
    let mut overlapping = false;
    for ancestor in parent.ancestors().filter(Node::is_element) {
        if ancestor.has_tag_name((NS, "textPath")) {
            return Err(unsupported("EPUB SVG mixed-font text path"));
        }
        if ancestor.has_tag_name((NS, "text")) {
            if overlapping && !ancestor.has_attribute("textLength") {
                return Err(unsupported(
                    "EPUB SVG mixed-font overlapping textLength owners",
                ));
            }
            return Ok((
                ancestor,
                ancestor
                    .has_attribute("textLength")
                    .then_some(ancestor)
                    .or(owner),
            ));
        }
        if ancestor.has_attribute("textLength") {
            overlapping |= owner.replace(ancestor).is_some();
        }
    }
    Err(invalid("missing EPUB SVG text root"))
}

fn effective_family<'a>(parent: Node<'a, '_>) -> Option<&'a str> {
    parent
        .ancestors()
        .filter_map(|node| node.attribute("style"))
        .filter_map(|style| {
            style
                .strip_prefix("font-family:")?
                .split_once(';')
                .map(|(family, _)| family)
        })
        .find(|family| *family != "inherit")
}
