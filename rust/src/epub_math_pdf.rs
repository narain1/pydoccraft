//! Place composed equation drawings in WordCraft's paginated PDF layout.
use std::{
    collections::{HashMap, HashSet},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use wordcraft_doc::{math::Math, para::InlineObject, CharProps, Document, Path, StoryRef};
use wordcraft_layout::{
    para::{ClKind, ParaLayout},
    DocLayout, LayoutCache, LayoutOptions, Placed,
};

use super::{
    compose::{Extra, Measurements},
    invalid, unsupported, Equation,
};
use crate::EngineError;

const OBJECT_PREFIX: &str = "\0epub-pdf-math-";
const MAX_RETAINED_DRAW_UNITS: usize = 1_000_000;
const MAX_REPLACED_DRAW_UNITS: usize = 4_000_000;

#[derive(Default)]
pub(in crate::epub) struct PdfMath {
    records: Vec<Measurements>,
    draw_units: usize,
}

impl PdfMath {
    pub(in crate::epub) fn object(
        &mut self,
        doc: &Document,
        equation: Equation,
        format: &CharProps,
    ) -> Result<InlineObject, EngineError> {
        let (math, display, extras) = match equation {
            Equation::Native {
                linear,
                display,
                math,
            } => {
                return Ok(InlineObject::Equation {
                    linear,
                    display,
                    math,
                });
            }
            Equation::Composed {
                math,
                display,
                extras,
            } => (math, display, extras),
            Equation::SpanningTable {
                table,
                mut extras,
                display,
            } => {
                let nodes = extras.push(Extra::Table(table, Box::new(format.clone())))?;
                (
                    Math {
                        nodes,
                        ..Default::default()
                    },
                    display,
                    extras,
                )
            }
        };
        let measured = extras.measure(doc, format)?;
        super::super::charge(
            &mut self.draw_units,
            measured.draw_units(),
            MAX_RETAINED_DRAW_UNITS,
            "EPUB PDF composed equations exceed their drawing budget",
        )?;
        let linear = format!("{OBJECT_PREFIX}{}", self.records.len());
        let math = measured.native(math);
        self.records.push(measured);
        Ok(InlineObject::Equation {
            linear,
            display,
            math,
        })
    }

    pub(in crate::epub) fn export(self, doc: &Document) -> Result<Vec<u8>, EngineError> {
        let options = wordcraft_pdf::PdfOptions::default();
        if self.records.is_empty() {
            return wordcraft_pdf::export(doc, &options).map_err(super::super::output::conversion);
        }
        let layout_options = LayoutOptions {
            hide_deleted: !options.include_markup,
            comment_balloons: options.comments,
            numeral: options.numeral,
            ..Default::default()
        };
        let mut layout = catch_unwind(AssertUnwindSafe(|| {
            wordcraft_layout::layout(doc, &mut LayoutCache::new(), &layout_options)
        }))
        .map_err(|_| super::super::output::conversion("EPUB PDF layout failed"))?;
        self.apply(doc, &mut layout)?;
        wordcraft_pdf::export_layout(doc, &layout, &options)
            .map_err(super::super::output::conversion)
    }

    fn apply(&self, doc: &Document, layout: &mut DocLayout) -> Result<(), EngineError> {
        let mut placed = Placements {
            records: &self.records,
            paragraphs: HashMap::new(),
            seen: HashSet::new(),
            draw_units: 0,
        };
        for page in &mut layout.pages {
            for item in page
                .items
                .iter_mut()
                .chain(&mut page.header)
                .chain(&mut page.footer)
            {
                placed.update(doc, item, page.body.h)?;
            }
        }
        if placed.seen.len() != self.records.len() {
            return Err(unsupported(
                "composed MathML could not be placed in the PDF layout",
            ));
        }
        Ok(())
    }
}

struct UpdatedParagraph {
    // Keep the source Arc alive so the cache's pointer key cannot be recycled.
    _source: Arc<ParaLayout>,
    result: Arc<ParaLayout>,
    objects: HashMap<usize, usize>,
}

struct Placements<'a> {
    records: &'a [Measurements],
    paragraphs: HashMap<(StoryRef, Path, *const ParaLayout), UpdatedParagraph>,
    seen: HashSet<usize>,
    draw_units: usize,
}

impl Placements<'_> {
    fn update(
        &mut self,
        doc: &Document,
        item: &mut Placed,
        page_height: f32,
    ) -> Result<(), EngineError> {
        match item {
            Placed::Lines {
                story,
                path,
                para,
                l0,
                l1,
                ..
            } if !para.maths.is_empty() => {
                let key = (*story, path.clone(), Arc::as_ptr(para));
                let updated = match self.paragraphs.entry(key) {
                    std::collections::hash_map::Entry::Occupied(found) => found.into_mut(),
                    std::collections::hash_map::Entry::Vacant(vacant) => {
                        let Some(source) = doc.para(*story, path) else {
                            return Ok(());
                        };
                        let mut objects = HashMap::new();
                        for (object, _) in &para.maths {
                            let Some(InlineObject::Equation { linear, .. }) =
                                source.objects.get(*object)
                            else {
                                continue;
                            };
                            if let Some(id) = linear.strip_prefix(OBJECT_PREFIX) {
                                objects.insert(*object, id.parse::<usize>().map_err(invalid)?);
                            }
                        }
                        if objects.is_empty() {
                            return Ok(());
                        }
                        let mut updated = (**para).clone();
                        for (object, layout) in &mut updated.maths {
                            if let Some(id) = objects.get(object) {
                                let record = self
                                    .records
                                    .get(*id)
                                    .ok_or_else(|| invalid("missing composed MathML PDF record"))?;
                                super::super::charge(
                                    &mut self.draw_units,
                                    record.draw_units(),
                                    MAX_REPLACED_DRAW_UNITS,
                                    "EPUB PDF composed placements exceed their drawing budget",
                                )?;
                                let layout = Arc::make_mut(layout);
                                record.resolve(layout)?;
                                layout.slots.clear();
                            }
                        }
                        vacant.insert(UpdatedParagraph {
                            _source: para.clone(),
                            result: Arc::new(updated),
                            objects,
                        })
                    }
                };
                for line in &updated.result.lines[*l0..*l1] {
                    for cluster in &updated.result.clusters[line.c0..line.c1] {
                        if let ClKind::Object(index) = cluster.kind {
                            if let Some(&id) = updated.objects.get(&index) {
                                // Layout already accounts for every reserved equation box.
                                // WordCraft can place an oversized object on an empty line.
                                if cluster.adv > line.right - line.left + 0.05
                                    || cluster.obj_h + cluster.obj_d > page_height + 0.05
                                {
                                    return Err(unsupported(
                                        "composed MathML exceeds the PDF text area",
                                    ));
                                }
                                self.seen.insert(id);
                            }
                        }
                    }
                }
                *para = updated.result.clone();
            }
            Placed::Clip { items, .. } => {
                for child in items {
                    self.update(doc, child, page_height)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
