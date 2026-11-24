//! Retry mixed-font text when native glyph replacement cannot match shaping clusters.
use std::collections::HashSet;

use resvg::usvg::{
    self,
    fontdb::{Database, ID},
};
use roxmltree::Node;
use unicode_segmentation::UnicodeSegmentation;

use super::{Resolver, Selection, MAX_FACES, MAX_WORK};
use crate::{
    author_epub::escape_text,
    epub::{
        charge, invalid, limited, parse_xml,
        structure::{append, apply_edits},
        svg::NS,
        unsupported, XML_BYTES,
    },
    EngineError,
};

impl Resolver {
    pub(in crate::epub::svg) fn split_fallback_runs(
        &self,
        source: &[u8],
        tree: &usvg::Tree,
    ) -> Result<Option<String>, EngineError> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| invalid("EPUB SVG font selection failed"))?;
        let missing = state.missing(tree);
        if missing.is_empty() {
            return Ok(None);
        }
        let source = std::str::from_utf8(source).map_err(invalid)?;
        let xml = parse_xml(source.as_bytes())?;
        let mut edits = Vec::new();
        let mut bytes = 0;
        for node in xml.descendants().filter(Node::is_text) {
            let Some(parent) = node.parent_element() else {
                continue;
            };
            if !can_split(parent) {
                continue;
            }
            let Some(family) = effective_family(parent) else {
                continue;
            };
            let Some(base) = state.family_plans.get(family).copied().flatten() else {
                continue;
            };
            let Some(text) = node.text() else { continue };
            if !text.chars().any(|ch| missing.contains(&(base, ch))) {
                continue;
            }
            let Some(candidates) = state.candidates.get(&base).cloned() else {
                continue;
            };
            let Some(split) = state.split_text(text, base, &candidates, tree.fontdb())? else {
                continue;
            };
            charge(
                &mut bytes,
                split.len(),
                XML_BYTES,
                "EPUB SVG fallback spans exceed 8 MiB",
            )?;
            edits.push((node.range(), split));
        }
        if edits.is_empty() {
            return Err(unsupported(
                "EPUB SVG mixed-font text could not retain all glyphs",
            ));
        }
        Ok(Some(apply_edits(source, edits, XML_BYTES)?))
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

    fn split_text(
        &mut self,
        text: &str,
        base: ID,
        candidates: &[ID],
        db: &Database,
    ) -> Result<Option<String>, EngineError> {
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
            let has_cluster = |id| cluster.chars().all(|ch| self.supports(db, id, ch.into()));
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
        let mut output = String::new();
        for (id, text) in runs {
            let next = self.direct_fonts.len();
            if !self.direct_fonts.contains_key(&id) && next >= MAX_FACES {
                return Err(limited("EPUB SVG font fallback exceeds 512 face variants"));
            }
            let alias = self
                .direct_fonts
                .entry(id)
                .or_insert_with(|| format!("doccraft-svg-resolved-{next}"));
            self.direct_names.insert(alias.clone(), id);
            let span = format!(
                r#"<tspan xmlns="{}" font-family="{alias}">{}</tspan>"#,
                NS,
                escape_text(text)
            );
            append(&mut output, &span, XML_BYTES)?;
        }
        Ok(Some(output))
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

fn can_split(parent: Node<'_, '_>) -> bool {
    for ancestor in parent.ancestors().filter(Node::is_element) {
        if ancestor.attribute("textLength").is_some() || ancestor.has_tag_name((NS, "textPath")) {
            return false;
        }
        if ancestor.has_tag_name((NS, "text")) {
            return true;
        }
    }
    false
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
