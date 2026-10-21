//! Bounded owned-style identity and unused-reference preflight. Native code still
//! owns name allocation, graph editing, serialization and style resolution.
use super::*;

const MAX_ID_BYTES: usize = 259;
const MAX_WORK: usize = 2_000_000;

pub(super) fn storage(document: &Document, retired: &BTreeSet<String>) -> PyResult<()> {
    let styles = &document.styles.styles;
    if styles
        .len()
        .checked_add(retired.len())
        .is_none_or(|n| n > MAX_STYLES)
    {
        return Err(limit(
            "document exceeds 1024 lifetime named style identifiers",
        ));
    }
    // Bound bytes before constructing a duplicate-detection set or reservation copy.
    for id in styles.iter().map(|s| &s.id).chain(retired) {
        if id.is_empty() || id.len() > MAX_ID_BYTES {
            return Err(limit("style identifier exceeds the 259-byte owned limit"));
        }
    }
    let mut ids = BTreeSet::new();
    for style in styles {
        if style.name.len() > 255 || !ids.insert(style.id.as_str()) || retired.contains(&style.id) {
            return Err(PyValueError::new_err(
                "invalid or reused owned style identity",
            ));
        }
    }
    Ok(())
}

pub(super) fn new_id(
    document: &Document,
    retired: &BTreeSet<String>,
    name: &str,
) -> PyResult<String> {
    storage(document, retired)?;
    if document.styles.styles.len() + retired.len() >= MAX_STYLES {
        return Err(limit(
            "document exceeds 1024 lifetime named style identifiers",
        ));
    }
    let mut reservation = StyleSheet::empty();
    reservation.styles = document
        .styles
        .styles
        .iter()
        .map(|s| &s.id)
        .chain(retired)
        .map(|id| Style {
            id: id.clone(),
            ..Default::default()
        })
        .collect();
    let id = reservation.new_id(name);
    if id.len() > MAX_ID_BYTES || reservation.get(&id).is_some() {
        return Err(PyValueError::new_err(
            "native style identifier allocation collided with a live or retired identifier",
        ));
    }
    Ok(id)
}

fn unsupported() -> PyErr {
    EngineError::Unsupported("unused style deletion requires owned content without opaque objects, controls, template updates or tracked property history".into()).into()
}

struct Walk<'a> {
    target: &'a str,
    ignore_refs: bool,
    work: usize,
    cap: usize,
    blocks: usize,
    cells: usize,
}
impl Walk<'_> {
    fn charge(&mut self, n: usize) -> PyResult<()> {
        self.work = self
            .work
            .checked_add(n)
            .filter(|n| *n <= self.cap)
            .ok_or_else(|| limit("unused style reference scan exceeds its work limit"))?;
        Ok(())
    }
    fn reference(&mut self, value: Option<&str>) -> PyResult<()> {
        self.charge(1)?;
        if let Some(value) = value {
            if value.len() > MAX_ID_BYTES {
                return Err(limit("style reference exceeds the 259-byte owned limit"));
            }
            self.charge(value.len())?;
            if !self.ignore_refs && value == self.target {
                return Err(PyValueError::new_err(
                    "style is referenced; clear its incoming references before deletion",
                ));
            }
        }
        Ok(())
    }
    fn chr(&mut self, props: &CharProps) -> PyResult<()> {
        self.charge(1)?;
        if props.fmt_change.is_some() || props.ins.is_some() || props.del.is_some() {
            return Err(unsupported());
        }
        self.reference(props.style.as_deref())
    }
    fn para(&mut self, props: &ParaProps) -> PyResult<()> {
        self.charge(1)?;
        if props.fmt_change.is_some() || props.num_change.is_some() {
            return Err(unsupported());
        }
        self.reference(props.style.as_deref())
    }
    fn table(&mut self, props: &wordcraft_doc::props::TableProps) -> PyResult<()> {
        self.charge(1)?;
        if props.fmt_change.is_some() {
            return Err(unsupported());
        }
        self.reference(props.style.as_deref())
    }
    fn blocks(&mut self, blocks: &wordcraft_doc::Blocks, depth: usize) -> PyResult<()> {
        if depth > 16 {
            return Err(limit("unused style scan exceeds 16 nested table levels"));
        }
        self.charge(1)?;
        self.blocks = self
            .blocks
            .checked_add(blocks.len())
            .filter(|n| *n <= MAX_BLOCKS)
            .ok_or_else(|| limit("unused style scan exceeds 100000 blocks"))?;
        self.charge(blocks.len())?;
        for block in blocks {
            match &**block {
                Block::Para(p) => {
                    self.para(&p.props)?;
                    self.chr(&p.mark)?;
                    if p.section.as_ref().is_some_and(|s| s.fmt_change.is_some()) {
                        return Err(unsupported());
                    }
                    self.charge(p.runs.len())?;
                    for run in &p.runs {
                        self.chr(&run.props)?;
                    }
                    self.charge(p.objects.len())?;
                    for object in &p.objects {
                        match object {
                            InlineObject::Image { ole: None, .. }
                            | InlineObject::NoteRef { .. }
                            | InlineObject::BookmarkStart { .. }
                            | InlineObject::BookmarkEnd { .. }
                            | InlineObject::CommentStart { .. }
                            | InlineObject::CommentEnd { .. } => (),
                            InlineObject::Field { instr, form, .. } => {
                                self.charge(instr.len())?;
                                if !form.is_empty()
                                    || !matches!(
                                        instr.trim(),
                                        "PAGE"
                                            | "NUMPAGES"
                                            | "SECTIONPAGES"
                                            | "PAGE \\* Arabic"
                                            | "NUMPAGES \\* Arabic"
                                            | "SECTIONPAGES \\* Arabic"
                                            | "TITLE"
                                            | "AUTHOR"
                                    )
                                {
                                    return Err(unsupported());
                                }
                            }
                            _ if object.as_note_separator().is_some() => (),
                            _ => return Err(unsupported()),
                        }
                    }
                }
                Block::Table(table) => {
                    if !table.controls.is_empty() {
                        return Err(unsupported());
                    }
                    self.table(&table.props)?;
                    self.charge(table.grid.len())?;
                    self.charge(table.rows.len())?;
                    for row in &table.rows {
                        if row.props.fmt_change.is_some() || !row.controls.is_empty() {
                            return Err(unsupported());
                        }
                        if let Some(props) = &row.props.exceptions {
                            self.table(props)?;
                        }
                        self.cells = self
                            .cells
                            .checked_add(row.cells.len())
                            .filter(|n| *n <= MAX_CELLS)
                            .ok_or_else(|| limit("unused style scan exceeds 100000 cells"))?;
                        self.charge(row.cells.len())?;
                        for cell in &row.cells {
                            if cell.props.fmt_change.is_some() || !cell.controls.is_empty() {
                                return Err(unsupported());
                            }
                            self.blocks(&cell.blocks, depth + 1)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    fn level(&mut self, level: &wordcraft_doc::numbering::Level) -> PyResult<()> {
        self.charge(1)?;
        self.reference(level.style.as_deref())?;
        self.chr(&level.chr)
    }
}

pub(super) fn unused(document: &Document, id: &str) -> PyResult<()> {
    scan(document, id, MAX_WORK).map(|_| ())
}

fn scan(document: &Document, id: &str, cap: usize) -> PyResult<usize> {
    let target = exact_style(&document.styles, id)?;
    if target.builtin || target.default || StyleSheet::builtin().get(id).is_some() {
        return Err(PyValueError::new_err(
            "builtin, default and implicit styles cannot be deleted",
        ));
    }
    if !document.passthrough.is_empty()
        || document.styles.latent_styles.is_some()
        || document.settings.link_styles
        || !document.revisions.is_empty()
        || document.last_section.fmt_change.is_some()
    {
        return Err(unsupported());
    }
    let mut walk = Walk {
        target: id,
        ignore_refs: false,
        work: 0,
        cap,
        blocks: 0,
        cells: 0,
    };
    walk.chr(&document.styles.default_chr)?;
    walk.para(&document.styles.default_para)?;
    walk.charge(document.styles.styles.len())?;
    for style in &document.styles.styles {
        // Its own edges disappear with this definition. Still validate/budget its
        // property carriers so opaque history cannot bypass the owned profile.
        walk.ignore_refs = style.id == id;
        for reference in [&style.based_on, &style.next, &style.linked] {
            walk.reference(reference.as_deref())?;
        }
        walk.para(&style.para)?;
        walk.chr(&style.chr)?;
        if let Some(table) = &style.table {
            for props in [
                &table.header_chr,
                &table.band_chr,
                &table.first_col_chr,
                &table.total_chr,
                &table.last_col_chr,
                &table.col_band_chr,
            ] {
                walk.chr(props)?;
            }
            walk.charge(table.regions.len())?;
            for part in table.regions.values() {
                walk.chr(&part.chr)?;
            }
        }
    }
    walk.ignore_refs = false;
    walk.blocks(&document.body, 0)?;
    walk.charge(document.parts.len())?;
    for part in document.parts.values() {
        walk.blocks(&part.blocks, 0)?;
    }
    for separators in [&document.footnote_separators, &document.endnote_separators] {
        for blocks in [
            &separators.separator,
            &separators.continuation_separator,
            &separators.continuation_notice,
        ]
        .into_iter()
        .flatten()
        {
            walk.blocks(blocks, 0)?;
        }
    }
    walk.charge(document.numbering.abstracts.len())?;
    for definition in &document.numbering.abstracts {
        walk.charge(definition.levels.len())?;
        for level in &definition.levels {
            walk.level(level)?;
        }
    }
    walk.charge(document.numbering.nums.len())?;
    for instance in &document.numbering.nums {
        walk.charge(instance.start_overrides.len())?;
        walk.charge(instance.level_overrides.len())?;
        for (_, level) in &instance.level_overrides {
            walk.level(level)?;
        }
    }
    Ok(walk.work)
}

#[cfg(test)]
#[path = "author_docx_style_delete_tests.rs"]
mod tests;
