//! Bounded authoring of new presentations through DeckCraft's public model/writer.
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;

use deckcraft_model::style::PictureFill;
use deckcraft_model::text::{
    Action, Align, Anchor, AutoFit, Hyperlink, RunProps, Strike, TextBody,
};
use deckcraft_model::{
    Background, ColorBase, ColorRef, Fill, Geom, LayoutType, Line, Presentation, Rgba, Shape,
    ShapeId, ShapeKind, Size, SlideId, Table, Theme, Xfrm,
};
use pyo3::exceptions::{PyIndexError, PyValueError};
use pyo3::prelude::*;

use crate::{DocExNativeError, EngineError};

const MAX_SLIDES: usize = 10_000;
const MAX_SHAPES: usize = 100_000;
const MAX_TEXT: usize = 16 * 1024 * 1024;
const MAX_MEDIA: usize = 64 * 1024 * 1024;
const MAX_IMAGE: usize = 16 * 1024 * 1024;
const MAX_SIDE: f64 = 14_400.0;
const MAX_PARAGRAPHS: usize = 100_000;
const MAX_RUNS: usize = 100_000;
const MAX_PROPERTIES: usize = 16 * 1024 * 1024;
const MAX_PATCH: usize = 16 * 1024;
const MAX_CELLS: usize = 100_000;
const MIN_GRID: f64 = 0.05;

type TextFormat = (
    Option<f64>,
    Option<(u8, u8, u8)>,
    Option<bool>,
    Option<bool>,
    Option<String>,
);
type Stroke = Option<((u8, u8, u8), f64)>;
type LinkValue = Option<(Option<String>, Option<u32>, String)>;

#[pyclass(module = "doc_ex._core")]
#[derive(Clone)]
pub struct EditablePptx {
    presentation: Option<Presentation>,
    text_bytes: usize,
    media_bytes: usize,
    shapes: usize,
    revision: u64,
    table_generations: HashMap<u32, u64>,
}

impl EditablePptx {
    fn table(&self, slide: usize, id: u32) -> PyResult<&Table> {
        match &self.shape(slide, id)?.kind {
            ShapeKind::Table(table) => Ok(table),
            _ => Err(PyValueError::new_err("shape is not a table")),
        }
    }

    fn mutate_table(
        &mut self,
        slide: usize,
        id: u32,
        structural: bool,
        operation: impl FnOnce(&mut Table) -> PyResult<()>,
    ) -> PyResult<()> {
        self.table(slide, id)?;
        let generation = if structural {
            self.table_generation(slide, id)?
                .checked_add(1)
                .ok_or_else(|| {
                    EngineError::ResourceLimit("table generation limit reached".into())
                })?
        } else {
            0
        };
        self.mutate(|document| {
            let shape = shape_mut(document, slide, id)?;
            let ShapeKind::Table(table) = &mut shape.kind else {
                return Err(PyValueError::new_err("shape is not a table"));
            };
            operation(table)?;
            validate_table_grid(table)?;
            let (width, height) = (table.width(), table.height());
            let transform = shape
                .xfrm
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("table has no explicit geometry"))?;
            transform.w = width;
            transform.h = height;
            Ok(())
        })?;
        if structural {
            self.table_generations.insert(id, generation);
        }
        self.refresh_budgets()
    }

    fn document(&self) -> PyResult<&Presentation> {
        self.presentation
            .as_ref()
            .ok_or_else(|| DocExNativeError::new_err("closed:document is closed"))
    }

    fn mutate<T>(
        &mut self,
        operation: impl FnOnce(&mut Presentation) -> PyResult<T>,
    ) -> PyResult<T> {
        let mut staged = self.document()?.clone();
        let result = operation(&mut staged)?;
        content_budget(&staged)?;
        let errors = staged.validate();
        if !errors.is_empty() {
            return Err(EngineError::Malformed(format!(
                "invalid authored presentation: {}",
                errors.join("; ")
            ))
            .into());
        }
        let revision = self.revision.checked_add(1).ok_or_else(|| {
            EngineError::ResourceLimit("presentation revision limit reached".into())
        })?;
        self.presentation = Some(staged);
        self.revision = revision;
        Ok(result)
    }

    fn check_revision(&self, revision: u64) -> PyResult<()> {
        self.document()?;
        if self.revision != revision {
            return Err(PyValueError::new_err(
                "text range is stale after a document change",
            ));
        }
        Ok(())
    }

    fn text_body(&self, slide: usize, id: u32) -> PyResult<&TextBody> {
        self.shape(slide, id)?
            .text
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("shape has no text body"))
    }

    fn check_slide(&self, slide: usize) -> PyResult<()> {
        if slide >= self.document()?.slides.len() {
            return Err(PyIndexError::new_err("slide index out of range"));
        }
        Ok(())
    }

    fn check_text_budget(&self, bytes: usize) -> PyResult<()> {
        if bytes > MAX_TEXT.saturating_sub(self.text_bytes) {
            return Err(
                EngineError::ResourceLimit("presentation text exceeds 16 MiB".into()).into(),
            );
        }
        Ok(())
    }

    fn check_paragraph_budget(&self, added: usize, removed: usize) -> PyResult<()> {
        let existing: usize = self
            .document()?
            .slides
            .iter()
            .map(|slide| {
                slide.notes.paragraphs.len()
                    + slide
                        .shapes
                        .iter()
                        .map(|shape| {
                            shape.text.as_ref().map_or(0, |body| body.paragraphs.len())
                                + match &shape.kind {
                                    ShapeKind::Table(table) => table
                                        .rows
                                        .iter()
                                        .flat_map(|row| &row.cells)
                                        .map(|cell| cell.text.paragraphs.len())
                                        .sum(),
                                    _ => 0,
                                }
                        })
                        .sum::<usize>()
            })
            .sum();
        if added > MAX_PARAGRAPHS.saturating_sub(existing.saturating_sub(removed)) {
            return Err(EngineError::ResourceLimit(
                "presentation exceeds 100000 paragraphs including table cells and notes".into(),
            )
            .into());
        }
        Ok(())
    }

    fn check_shape(&self, slide: usize, x: f64, y: f64, width: f64, height: f64) -> PyResult<()> {
        self.check_slide(slide)?;
        geometry(x, y, width, height)?;
        if self.shapes >= MAX_SHAPES {
            return Err(
                EngineError::ResourceLimit("presentation exceeds 100000 shapes".into()).into(),
            );
        }
        Ok(())
    }

    fn shape(&self, slide: usize, id: u32) -> PyResult<&Shape> {
        self.check_slide(slide)?;
        self.document()?.slides[slide]
            .shape(ShapeId(id))
            .ok_or_else(|| PyValueError::new_err("shape does not exist on this slide"))
    }

    fn picture_fill(&self, slide: usize, id: u32) -> PyResult<&PictureFill> {
        match &self.shape(slide, id)?.kind {
            ShapeKind::Picture { fill } => Ok(fill),
            _ => Err(PyValueError::new_err("shape is not a picture")),
        }
    }

    fn shape_transform(&self, slide: usize, id: u32) -> PyResult<Xfrm> {
        let shape = self.shape(slide, id)?;
        let document = self.document()?;
        let context = deckcraft_model::resolve::Ctx::for_slide(document, &document.slides[slide])
            .ok_or_else(|| {
            EngineError::Malformed("slide has no native master/layout context".into())
        })?;
        Ok(deckcraft_model::resolve::xfrm(&context, shape))
    }

    fn refresh_budgets(&mut self) -> PyResult<()> {
        let document = self.document()?;
        let mut live_tables = HashSet::new();
        for slide in &document.slides {
            visit_shapes(&slide.shapes, &mut |shape| {
                if matches!(shape.kind, ShapeKind::Table(_)) {
                    live_tables.insert(shape.id.0);
                }
                Ok(())
            })?;
        }
        let shapes = document.slides.iter().map(|slide| slide.shapes.len()).sum();
        let media_bytes = document.media.iter().map(|media| media.data.len()).sum();
        let text_bytes = document
            .slides
            .iter()
            .map(|slide| {
                slide.name.len()
                    + slide.notes.text().len()
                    + slide
                        .shapes
                        .iter()
                        .map(|shape| {
                            shape.descr.len()
                                + shape.text.as_ref().map_or(0, |body| body.text().len())
                                + match &shape.kind {
                                    ShapeKind::Table(table) => table
                                        .rows
                                        .iter()
                                        .flat_map(|row| &row.cells)
                                        .map(|cell| cell.text.text().len())
                                        .sum(),
                                    _ => 0,
                                }
                        })
                        .sum::<usize>()
            })
            .sum();
        self.shapes = shapes;
        self.media_bytes = media_bytes;
        self.text_bytes = text_bytes;
        self.table_generations
            .retain(|id, _| live_tables.contains(id));
        Ok(())
    }
}

#[pymethods]
impl EditablePptx {
    #[new]
    #[pyo3(signature = (width=720.0, height=540.0))]
    pub fn new(width: f64, height: f64) -> PyResult<Self> {
        dimensions(width, height)?;
        let presentation = deckcraft_model::defaults::blank_presentation(
            Size::new(width, height),
            Theme::default(),
            false,
        );
        Ok(Self {
            presentation: Some(presentation),
            text_bytes: 0,
            media_bytes: 0,
            shapes: 0,
            revision: 0,
            table_generations: HashMap::new(),
        })
    }

    pub fn revision(&self) -> PyResult<u64> {
        self.document()?;
        Ok(self.revision)
    }

    pub fn copy(&self) -> PyResult<Self> {
        self.document()?;
        Ok(self.clone())
    }

    pub fn close(&mut self) {
        self.presentation = None;
        self.table_generations.clear();
    }

    pub fn slide_count(&self) -> PyResult<usize> {
        Ok(self.document()?.slides.len())
    }

    pub fn slide_size(&self) -> PyResult<(f64, f64)> {
        let size = self.document()?.slide_size;
        Ok((size.width, size.height))
    }

    pub fn slide_id(&self, slide: usize) -> PyResult<u32> {
        self.check_slide(slide)?;
        Ok(self.document()?.slides[slide].id.0)
    }

    pub fn slide_index(&self, id: u32) -> PyResult<usize> {
        self.document()?
            .slide_index(SlideId(id))
            .ok_or_else(|| PyIndexError::new_err("slide no longer exists"))
    }

    pub fn slide_name(&self, slide: usize) -> PyResult<String> {
        self.check_slide(slide)?;
        Ok(self.document()?.slides[slide].name.clone())
    }

    pub fn set_slide_name(&mut self, slide: usize, name: &str) -> PyResult<()> {
        let old = self.slide_name(slide)?.len();
        text_value(name)?;
        if name.chars().count() > 255 {
            return Err(PyValueError::new_err("slide name exceeds 255 characters"));
        }
        self.check_text_budget(name.len().saturating_sub(old))?;
        self.mutate(|document| {
            Arc::make_mut(&mut document.slides[slide]).name = name.to_owned();
            Ok(())
        })?;
        self.text_bytes = self.text_bytes.saturating_sub(old) + name.len();
        Ok(())
    }

    pub fn slide_hidden(&self, slide: usize) -> PyResult<bool> {
        self.check_slide(slide)?;
        Ok(self.document()?.slides[slide].hidden)
    }

    pub fn set_slide_hidden(&mut self, slide: usize, hidden: bool) -> PyResult<()> {
        self.check_slide(slide)?;
        self.mutate(|document| {
            Arc::make_mut(&mut document.slides[slide]).hidden = hidden;
            Ok(())
        })
    }

    pub fn slide_notes(&self, slide: usize) -> PyResult<String> {
        self.check_slide(slide)?;
        Ok(self.document()?.slides[slide].notes.text())
    }

    pub fn set_slide_notes(&mut self, slide: usize, text: &str) -> PyResult<()> {
        let old = self.slide_notes(slide)?.len();
        text_value(text)?;
        self.check_text_budget(text.len().saturating_sub(old))?;
        self.check_paragraph_budget(
            paragraph_count(text),
            self.document()?.slides[slide].notes.paragraphs.len(),
        )?;
        self.mutate(|document| {
            Arc::make_mut(&mut document.slides[slide]).notes = text_body(text, 18.0);
            Ok(())
        })?;
        self.text_bytes = self.text_bytes.saturating_sub(old) + text.len();
        Ok(())
    }

    pub fn move_slide(&mut self, slide: usize, destination: usize) -> PyResult<()> {
        self.check_slide(slide)?;
        self.check_slide(destination)?;
        organization_profile(self.document()?)?;
        self.mutate(|document| {
            let moved = document.slides.remove(slide);
            document.slides.insert(destination, moved);
            Ok(())
        })
    }

    pub fn remove_slide(&mut self, slide: usize) -> PyResult<()> {
        self.check_slide(slide)?;
        reject_referenced_slide(self.document()?, self.document()?.slides[slide].id)?;
        organization_profile(self.document()?)?;
        self.mutate(|document| {
            document.slides.remove(slide);
            prune_media(document);
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn to_bytes(&self) -> PyResult<Vec<u8>> {
        let document = self.document()?;
        if document.slides.is_empty() {
            return Err(PyValueError::new_err(
                "add at least one slide before saving",
            ));
        }
        let output = deckcraft_pptx::export(document)
            .map_err(|error| EngineError::Conversion(format!("PPTX save failed: {error}")))?;
        if output.len() > 128 * 1024 * 1024 {
            return Err(EngineError::ResourceLimit("PPTX output exceeds 128 MiB".into()).into());
        }
        Ok(output)
    }

    #[pyo3(signature = (layout="blank"))]
    pub fn add_slide(&mut self, layout: &str) -> PyResult<usize> {
        let document = self.document()?;
        if document.slides.len() >= MAX_SLIDES {
            return Err(
                EngineError::ResourceLimit("presentation exceeds 10000 slides".into()).into(),
            );
        }
        let kind = match layout {
            "blank" => LayoutType::Blank,
            "title" => LayoutType::Title,
            "title_and_content" => LayoutType::TitleAndContent,
            "two_content" => LayoutType::TwoContent,
            "section_header"
            | "comparison"
            | "title_only"
            | "content_with_caption"
            | "picture_with_caption"
            | "title_and_vertical_text"
            | "vertical_title_and_text"
            | "custom" => {
                return Err(EngineError::Unsupported(format!(
                    "slide layout {layout} is not available in the verified authoring profile"
                ))
                .into())
            }
            _ => {
                return Err(PyValueError::new_err(
                    "layout must be blank, title, title_and_content or two_content",
                ))
            }
        };
        let layout = deckcraft_model::defaults::layout_of_kind(document, kind)
            .ok_or_else(|| EngineError::Malformed("native default layout is missing".into()))?;
        let (_, definition) = document
            .layout(layout)
            .ok_or_else(|| EngineError::Malformed("native layout definition is missing".into()))?;
        let added = definition
            .shapes
            .iter()
            .filter(|shape| {
                shape
                    .ph
                    .as_ref()
                    .is_some_and(|ph| !ph.kind.is_footer_kind())
            })
            .count();
        if added > MAX_SHAPES.saturating_sub(self.shapes) {
            return Err(
                EngineError::ResourceLimit("presentation exceeds 100000 shapes".into()).into(),
            );
        }
        self.check_paragraph_budget(added, 0)?;
        let identifiers = u32::try_from(added.saturating_add(1))
            .ok()
            .and_then(|count| document.next_id.max(1).checked_add(count));
        if identifiers.is_none() {
            return Err(
                EngineError::ResourceLimit("presentation object IDs exhausted".into()).into(),
            );
        }
        let index = self.mutate(|document| {
            let mut slide =
                deckcraft_model::defaults::new_slide(document, layout).ok_or_else(|| {
                    EngineError::ResourceLimit("presentation object IDs exhausted".into())
                })?;
            // Native layout placeholders start with zero paragraphs. Use the public
            // editor's empty insertion to create its editable empty paragraph without
            // adding any direct text-style or geometry overrides.
            for shape in &mut slide.shapes {
                if let Some(body) = &mut shape.text {
                    deckcraft_model::edit::insert(body, (0, 0), "", None);
                }
            }
            let index = document.slides.len();
            document.slides.push(Arc::new(slide));
            Ok(index)
        })?;
        self.refresh_budgets()?;
        Ok(index)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (slide, text, x, y, width, height, size=18.0))]
    pub fn add_text(
        &mut self,
        slide: usize,
        text: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        size: f64,
    ) -> PyResult<u32> {
        self.check_shape(slide, x, y, width, height)?;
        text_value(text)?;
        font_size(size)?;
        self.check_text_budget(text.len())?;
        self.check_paragraph_budget(paragraph_count(text), 0)?;
        let shape = Shape {
            xfrm: Some(Xfrm::new(x, y, width, height)),
            text: Some(text_body(text, size)),
            text_box: true,
            fill: Some(Fill::None),
            ..Shape::default()
        };
        let id = self.mutate(|document| insert_shape(document, slide, shape))?;
        self.text_bytes += text.len();
        self.shapes += 1;
        Ok(id)
    }

    pub fn background(&mut self, slide: usize, color: (u8, u8, u8)) -> PyResult<()> {
        self.check_slide(slide)?;
        self.mutate(|document| {
            Arc::make_mut(&mut document.slides[slide]).background = Some(Background::Fill {
                fill: Fill::solid(ColorRef::rgb(Rgba::rgb(color.0, color.1, color.2))),
            });
            prune_media(document);
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn background_image(&self, slide: usize) -> PyResult<Option<Vec<u8>>> {
        self.check_slide(slide)?;
        let document = self.document()?;
        if let Some(Background::Fill {
            fill: Fill::Picture(fill),
        }) = &document.slides[slide].background
        {
            return document
                .media(fill.media)
                .map(|media| Some(media.data.as_ref().clone()))
                .ok_or_else(|| {
                    EngineError::Malformed("background image media is missing".into()).into()
                });
        }
        Ok(None)
    }

    pub fn set_background_image(&mut self, slide: usize, data: &[u8]) -> PyResult<()> {
        self.check_slide(slide)?;
        validate_png(data)?;
        self.mutate(|document| {
            let media = document
                .add_media("background.png", "image/png", data.to_vec())
                .ok_or_else(|| {
                    EngineError::ResourceLimit("presentation object IDs exhausted".into())
                })?;
            Arc::make_mut(&mut document.slides[slide]).background = Some(Background::Fill {
                fill: Fill::Picture(PictureFill {
                    media,
                    ..PictureFill::default()
                }),
            });
            prune_media(document);
            check_live_media(document)?;
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn shape_ids(&self, slide: usize) -> PyResult<Vec<(u32, &'static str)>> {
        self.check_slide(slide)?;
        Ok(self.document()?.slides[slide]
            .shapes
            .iter()
            .map(|shape| {
                (
                    shape.id.0,
                    match &shape.kind {
                        ShapeKind::Table(_) => "table",
                        ShapeKind::Picture { .. } => "image",
                        _ if shape.text.is_some() => "text",
                        _ => "shape",
                    },
                )
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (slide, kind, x, y, width, height, fill=None, stroke=None, stroke_width=1.0))]
    pub fn add_shape(
        &mut self,
        slide: usize,
        kind: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        fill: Option<(u8, u8, u8)>,
        stroke: Option<(u8, u8, u8)>,
        stroke_width: f64,
    ) -> PyResult<u32> {
        self.check_slide(slide)?;
        let preset = match kind {
            "rectangle" => "rect",
            "ellipse" => "ellipse",
            "line" => "line",
            _ => {
                return Err(PyValueError::new_err(
                    "shape kind must be rectangle, ellipse or line",
                ))
            }
        };
        if kind == "line" {
            line_geometry(x, y, width, height)?;
        } else {
            geometry(x, y, width, height)?;
        }
        if self.shapes >= MAX_SHAPES {
            return Err(
                EngineError::ResourceLimit("presentation exceeds 100000 shapes".into()).into(),
            );
        }
        stroke_size(stroke_width)?;
        let shape = Shape {
            xfrm: Some(Xfrm::new(x, y, width, height)),
            geom: Geom::preset(preset),
            fill: Some(fill.map_or(Fill::None, solid)),
            line: Some(stroke.map_or_else(Line::none, |color| {
                Line::solid(color_ref(color), stroke_width)
            })),
            ..Shape::default()
        };
        let id = self.mutate(|document| insert_shape(document, slide, shape))?;
        self.shapes += 1;
        Ok(id)
    }

    pub fn shape_placeholder(&self, slide: usize, id: u32) -> PyResult<Option<&'static str>> {
        use deckcraft_model::PhType;
        Ok(self.shape(slide, id)?.ph.as_ref().map(|ph| match ph.kind {
            PhType::Title => "title",
            PhType::CtrTitle => "center_title",
            PhType::SubTitle => "subtitle",
            PhType::Body => "body",
            PhType::Obj => "content",
            PhType::Chart => "chart",
            PhType::Table => "table",
            PhType::ClipArt => "clip_art",
            PhType::Diagram => "diagram",
            PhType::Media => "media",
            PhType::Picture => "picture",
            PhType::Date => "date",
            PhType::Footer => "footer",
            PhType::SlideNum => "slide_number",
            PhType::Header => "header",
            PhType::SlideImage => "slide_image",
        }))
    }

    pub fn shape_rotation(&self, slide: usize, id: u32) -> PyResult<f64> {
        Ok(self.shape_transform(slide, id)?.rot)
    }

    pub fn set_shape_rotation(&mut self, slide: usize, id: u32, degrees: f64) -> PyResult<()> {
        let effective = self.shape_transform(slide, id)?;
        if !degrees.is_finite() || !(-360.0..=360.0).contains(&degrees) {
            return Err(PyValueError::new_err(
                "rotation must be finite and between -360 and 360 degrees",
            ));
        }
        self.mutate(|document| {
            shape_mut(document, slide, id)?
                .xfrm
                .get_or_insert(effective)
                .rot = degrees;
            Ok(())
        })
    }

    pub fn move_shape(&mut self, slide: usize, id: u32, destination: usize) -> PyResult<()> {
        self.shape(slide, id)?;
        if destination >= self.document()?.slides[slide].shapes.len() {
            return Err(PyIndexError::new_err(
                "shape destination index out of range",
            ));
        }
        organization_profile(self.document()?)?;
        self.mutate(|document| {
            let shapes = &mut Arc::make_mut(&mut document.slides[slide]).shapes;
            let current = shapes
                .iter()
                .position(|shape| shape.id == ShapeId(id))
                .ok_or_else(|| PyValueError::new_err("shape does not exist"))?;
            let shape = shapes.remove(current);
            shapes.insert(destination, shape);
            Ok(())
        })
    }

    pub fn delete_shape(&mut self, slide: usize, id: u32) -> PyResult<()> {
        self.shape(slide, id)?;
        organization_profile(self.document()?)?;
        self.mutate(|document| {
            Arc::make_mut(&mut document.slides[slide])
                .shapes
                .retain(|shape| shape.id != ShapeId(id));
            prune_media(document);
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn shape_fill(&self, slide: usize, id: u32) -> PyResult<Option<(u8, u8, u8)>> {
        let shape = self.shape(slide, id)?;
        let fill = match &shape.kind {
            ShapeKind::Shape => &shape.fill,
            ShapeKind::Table(table) => &table.background,
            _ => {
                return Err(EngineError::Unsupported(
                    "solid fill is available on text, shapes and tables".into(),
                )
                .into())
            }
        };
        rgb_fill(fill)
    }

    #[pyo3(signature = (slide, id, color=None))]
    pub fn set_shape_fill(
        &mut self,
        slide: usize,
        id: u32,
        color: Option<(u8, u8, u8)>,
    ) -> PyResult<()> {
        self.shape(slide, id)?;
        self.mutate(|document| {
            let shape = shape_mut(document, slide, id)?;
            let target = match &mut shape.kind {
                ShapeKind::Shape => &mut shape.fill,
                ShapeKind::Table(table) => &mut table.background,
                _ => {
                    return Err(EngineError::Unsupported(
                        "solid fill is available on text, shapes and tables".into(),
                    )
                    .into())
                }
            };
            *target = Some(color.map_or(Fill::None, solid));
            Ok(())
        })
    }

    pub fn shape_stroke(&self, slide: usize, id: u32) -> PyResult<Stroke> {
        let shape = self.shape(slide, id)?;
        if !matches!(shape.kind, ShapeKind::Shape) {
            return Err(EngineError::Unsupported(
                "outline is available on text and basic shapes".into(),
            )
            .into());
        }
        match &shape.line {
            Some(line) => Ok(rgb_fill(&line.fill)?.map(|color| (color, line.width.unwrap_or(1.0)))),
            None => Ok(None),
        }
    }

    #[pyo3(signature = (slide, id, color=None, width=1.0))]
    pub fn set_shape_stroke(
        &mut self,
        slide: usize,
        id: u32,
        color: Option<(u8, u8, u8)>,
        width: f64,
    ) -> PyResult<()> {
        self.shape(slide, id)?;
        stroke_size(width)?;
        self.mutate(|document| {
            let shape = shape_mut(document, slide, id)?;
            if !matches!(shape.kind, ShapeKind::Shape) {
                return Err(EngineError::Unsupported(
                    "outline is available on text and basic shapes".into(),
                )
                .into());
            }
            shape.line =
                Some(color.map_or_else(Line::none, |color| Line::solid(color_ref(color), width)));
            Ok(())
        })
    }

    pub fn slide_background(&self, slide: usize) -> PyResult<Option<(u8, u8, u8)>> {
        self.check_slide(slide)?;
        match &self.document()?.slides[slide].background {
            None => Ok(None),
            Some(Background::Fill {
                fill:
                    Fill::Solid {
                        color:
                            ColorRef {
                                base: ColorBase::Rgb { rgb },
                                ..
                            },
                    },
            }) => Ok(Some((rgb.r, rgb.g, rgb.b))),
            _ => Err(
                EngineError::Unsupported("slide background is not an RGB solid fill".into()).into(),
            ),
        }
    }

    pub fn shape_text(&self, slide: usize, id: u32) -> PyResult<String> {
        self.shape(slide, id)?
            .text
            .as_ref()
            .map(TextBody::text)
            .ok_or_else(|| PyValueError::new_err("shape has no text body"))
    }

    pub fn text_range(
        &self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
    ) -> PyResult<(String, u64)> {
        let body = self.text_body(slide, id)?;
        let (a, b) = text_positions(body, start, end)?;
        Ok((deckcraft_model::edit::text_range(body, a, b), self.revision))
    }

    pub fn range_text(
        &self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
    ) -> PyResult<String> {
        self.check_revision(revision)?;
        Ok(self.text_range(slide, id, start, end)?.0)
    }

    #[pyo3(signature = (slide, id, text, patch="{}"))]
    pub fn add_run_patch(
        &mut self,
        slide: usize,
        id: u32,
        text: &str,
        patch: &str,
    ) -> PyResult<(usize, usize, u64)> {
        let patch = FormatPatch::parse(patch)?;
        let start = self.text_body(slide, id)?.char_len();
        let body = edited_body(
            self.document()?,
            slide,
            id,
            start,
            start,
            text,
            Some(&patch),
        )?;
        self.mutate(|document| {
            shape_mut(document, slide, id)?.text = Some(body);
            Ok(())
        })?;
        self.refresh_budgets()?;
        Ok((start, start + text.chars().count(), self.revision))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn replace_range(
        &mut self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        text: &str,
    ) -> PyResult<(usize, usize, u64)> {
        self.check_revision(revision)?;
        let body = edited_body(self.document()?, slide, id, start, end, text, None)?;
        self.mutate(|document| {
            shape_mut(document, slide, id)?.text = Some(body);
            Ok(())
        })?;
        self.refresh_budgets()?;
        Ok((start, start + text.chars().count(), self.revision))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn format_range_patch(
        &mut self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        patch: &str,
    ) -> PyResult<u64> {
        self.check_revision(revision)?;
        let patch = FormatPatch::parse(patch)?;
        let body = self.text_body(slide, id)?;
        let (a, b) = text_positions(body, start, end)?;
        preflight_format(
            self.document()?,
            body,
            a,
            b,
            &|props| patch.apply(props),
            false,
        )?;
        self.mutate(|document| {
            let body = shape_mut(document, slide, id)?
                .text
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("shape has no text body"))?;
            deckcraft_model::edit::format(body, a, b, &|props| patch.apply(props));
            Ok(())
        })?;
        Ok(self.revision)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (slide, id, start, end, revision, allow_mixed=false))]
    pub fn range_link(
        &self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        allow_mixed: bool,
    ) -> PyResult<LinkValue> {
        self.check_revision(revision)?;
        let body = self.text_body(slide, id)?;
        let (a, b) = text_positions(body, start, end)?;
        if start == end {
            return Err(PyValueError::new_err(
                "a link requires a nonempty text range",
            ));
        }
        let mut found: Option<&Option<Hyperlink>> = None;
        for (pi, paragraph) in body.paragraphs.iter().enumerate().take(b.0 + 1).skip(a.0) {
            let from = if pi == a.0 { a.1 } else { 0 };
            let to = if pi == b.0 { b.1 } else { paragraph.char_len() };
            let mut offset = 0;
            for run in &paragraph.runs {
                let next = offset + run.char_len();
                if offset < to && next > from {
                    if found.is_some_and(|value| value != &run.props.link) {
                        if allow_mixed {
                            return Ok(None);
                        }
                        return Err(PyValueError::new_err(
                            "range contains mixed hyperlink targets or tooltips",
                        ));
                    }
                    found = Some(&run.props.link);
                }
                offset = next;
            }
        }
        link_value(found.and_then(Option::as_ref))
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (slide, id, start, end, revision, url=None, target_slide=None, tooltip=""))]
    pub fn set_range_link(
        &mut self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        url: Option<&str>,
        target_slide: Option<u32>,
        tooltip: &str,
    ) -> PyResult<u64> {
        self.check_revision(revision)?;
        let link = make_link(self.document()?, url, target_slide, tooltip)?;
        let body = self.text_body(slide, id)?;
        let (a, b) = text_positions(body, start, end)?;
        if start == end || !range_has_text(body, a, b) {
            return Err(PyValueError::new_err(
                "a link requires a range containing text",
            ));
        }
        preflight_format(
            self.document()?,
            body,
            a,
            b,
            &|props| props.link = link.clone(),
            true,
        )?;
        self.mutate(|document| {
            let body = shape_mut(document, slide, id)?
                .text
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("shape has no text body"))?;
            // A labeled link never changes the insertion/end-mark formatting.
            let ends: Vec<_> = body.paragraphs[a.0..=b.0]
                .iter()
                .map(|p| p.end_props.clone())
                .collect();
            deckcraft_model::edit::format(body, a, b, &|props| props.link = link.clone());
            for (paragraph, props) in body.paragraphs[a.0..=b.0].iter_mut().zip(ends) {
                paragraph.end_props = props;
            }
            Ok(())
        })?;
        Ok(self.revision)
    }

    pub fn shape_link(&self, slide: usize, id: u32) -> PyResult<LinkValue> {
        link_value(self.shape(slide, id)?.click.as_ref())
    }

    #[pyo3(signature = (slide, id, url=None, target_slide=None, tooltip=""))]
    pub fn set_shape_link(
        &mut self,
        slide: usize,
        id: u32,
        url: Option<&str>,
        target_slide: Option<u32>,
        tooltip: &str,
    ) -> PyResult<()> {
        self.shape(slide, id)?;
        let link = make_link(self.document()?, url, target_slide, tooltip)?;
        self.mutate(|document| {
            shape_mut(document, slide, id)?.click = link;
            Ok(())
        })
    }

    pub fn set_shape_text(&mut self, slide: usize, id: u32, text: &str) -> PyResult<()> {
        let end = self.text_body(slide, id)?.char_len();
        let body = edited_body(self.document()?, slide, id, 0, end, text, None)?;
        self.mutate(|document| {
            shape_mut(document, slide, id)?.text = Some(body);
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn shape_box(&self, slide: usize, id: u32) -> PyResult<(f64, f64, f64, f64)> {
        let transform = self.shape_transform(slide, id)?;
        Ok((transform.x, transform.y, transform.w, transform.h))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_shape_box(
        &mut self,
        slide: usize,
        id: u32,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> PyResult<()> {
        let effective = self.shape_transform(slide, id)?;
        if self.shape(slide, id)?.geom.preset_name() == Some("line") {
            line_geometry(x, y, width, height)?;
        } else {
            geometry(x, y, width, height)?;
        }
        self.mutate(|document| {
            let shape = shape_mut(document, slide, id)?;
            if let ShapeKind::Table(table) = &mut shape.kind {
                let x_scale = width / table.width();
                let y_scale = height / table.height();
                for column in &mut table.cols {
                    *column *= x_scale;
                }
                for row in &mut table.rows {
                    row.height *= y_scale;
                }
                validate_table_grid(table)?;
            }
            let transform = shape.xfrm.get_or_insert(effective);
            transform.x = x;
            transform.y = y;
            transform.w = width;
            transform.h = height;
            Ok(())
        })
    }

    pub fn table_rows(&self, slide: usize, id: u32) -> PyResult<Vec<Vec<String>>> {
        let ShapeKind::Table(table) = &self.shape(slide, id)?.kind else {
            return Err(PyValueError::new_err("shape is not a table"));
        };
        Ok(table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        if cell.is_covered() {
                            String::new()
                        } else {
                            cell.text.text()
                        }
                    })
                    .collect()
            })
            .collect())
    }

    pub fn table_generation(&self, slide: usize, id: u32) -> PyResult<u64> {
        self.table(slide, id)?;
        Ok(*self.table_generations.get(&id).unwrap_or(&0))
    }

    pub fn column_widths(&self, slide: usize, id: u32) -> PyResult<Vec<f64>> {
        Ok(self.table(slide, id)?.cols.clone())
    }

    pub fn set_column_widths(&mut self, slide: usize, id: u32, widths: Vec<f64>) -> PyResult<()> {
        if widths.len() != self.table(slide, id)?.n_cols() || widths.iter().any(|v| !grid_size(*v))
        {
            return Err(PyValueError::new_err(
                "column widths require one finite 0.05..14400 point value per column",
            ));
        }
        dimensions(widths.iter().sum(), self.table(slide, id)?.height())?;
        self.mutate_table(slide, id, false, |table| {
            table.cols = widths;
            Ok(())
        })
    }

    pub fn row_height(
        &self,
        slide: usize,
        id: u32,
        row: usize,
    ) -> PyResult<(Option<f64>, &'static str)> {
        let row = self
            .table(slide, id)?
            .rows
            .get(row)
            .ok_or_else(|| PyIndexError::new_err("table row index out of range"))?;
        Ok((Some(row.height), "at_least"))
    }

    #[pyo3(signature = (slide, id, row, height=None, rule="auto"))]
    pub fn set_row_height(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        height: Option<f64>,
        rule: &str,
    ) -> PyResult<()> {
        self.row_height(slide, id, row)?;
        let height = match (height, rule) {
            (Some(v), "at_least") if grid_size(v) => v,
            _ => return Err(PyValueError::new_err("PPTX row height requires (0.05..14400 points, at_least); exact and auto are unavailable")),
        };
        self.mutate_table(slide, id, false, |table| {
            table.rows[row].height = height;
            Ok(())
        })
    }

    pub fn table_borders(&self, slide: usize, id: u32) -> PyResult<Option<(String, f64)>> {
        let mut found: Option<Option<(String, f64)>> = None;
        for cell in self
            .table(slide, id)?
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .filter(|cell| !cell.is_covered())
        {
            for line in &cell.borders {
                let line = line.as_ref().ok_or_else(|| EngineError::Unsupported("table borders inherit a style; resolved border inspection is unavailable upstream".into()))?;
                let value = border_value(line)?;
                if found.as_ref().is_some_and(|before| before != &value) {
                    return Err(PyValueError::new_err("table borders are not uniform"));
                }
                found = Some(value);
            }
        }
        Ok(found.flatten())
    }

    #[pyo3(signature = (slide, id, color="#000000", width=0.5))]
    pub fn set_table_borders(
        &mut self,
        slide: usize,
        id: u32,
        color: &str,
        width: f64,
    ) -> PyResult<()> {
        self.table(slide, id)?;
        let color = parse_color(color)?;
        if !width.is_finite() || !(0.25..=12.0).contains(&width) {
            return Err(PyValueError::new_err(
                "PPTX border width must be finite and 0.25..12 points",
            ));
        }
        let line = Line::solid(color, width);
        self.mutate_table(slide, id, false, |table| {
            for cell in table.rows.iter_mut().flat_map(|row| &mut row.cells) {
                cell.borders = std::array::from_fn(|_| Some(line.clone()));
            }
            Ok(())
        })
    }

    pub fn clear_table_borders(&mut self, slide: usize, id: u32) -> PyResult<()> {
        self.mutate_table(slide, id, false, |table| {
            for cell in table.rows.iter_mut().flat_map(|row| &mut row.cells) {
                cell.borders = std::array::from_fn(|_| Some(Line::none()));
            }
            Ok(())
        })
    }

    pub fn cell_text(&self, slide: usize, id: u32, row: usize, col: usize) -> PyResult<String> {
        Ok(table_cell(self.table(slide, id)?, row, col)?.text.text())
    }

    pub fn cell_margins(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<(f64, f64, f64, f64)> {
        let [left, right, top, bottom] = table_cell(self.table(slide, id)?, row, col)?
            .margins
            .unwrap_or([7.2, 7.2, 3.6, 3.6]);
        Ok((top, right, bottom, left))
    }

    #[pyo3(signature = (slide, id, row, col, margins=None))]
    pub fn set_cell_margins(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        margins: Option<(f64, f64, f64, f64)>,
    ) -> PyResult<()> {
        let table = self.table(slide, id)?;
        let (row, col) = cell_anchor(table, row, col)?;
        let value = if let Some((top, right, bottom, left)) = margins {
            let width = table.cols[col..col + table.rows[row].cells[col].grid_span.max(1) as usize]
                .iter()
                .sum::<f64>();
            if [top, right, bottom, left]
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=MAX_SIDE).contains(v))
                || left + right >= width
                || top + bottom > MAX_SIDE
            {
                return Err(PyValueError::new_err("cell margins must be finite nonnegative points, leave positive text width and total at most 14400 points vertically"));
            }
            Some([left, right, top, bottom])
        } else {
            None
        };
        self.mutate_table(slide, id, false, |table| {
            table.rows[row].cells[col].margins = value;
            Ok(())
        })
    }

    pub fn cell_alignment(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<Option<&'static str>> {
        let cell = table_cell(self.table(slide, id)?, row, col)?;
        let mut found = None;
        for paragraph in &cell.text.paragraphs {
            if found.is_some_and(|value| value != paragraph.props.align) {
                return Err(PyValueError::new_err(
                    "cell paragraphs have mixed direct alignment",
                ));
            }
            found = Some(paragraph.props.align);
        }
        Ok(found.flatten().map(|value| match value {
            Align::Left => "left",
            Align::Center => "center",
            Align::Right => "right",
            Align::Justify => "justify",
            Align::JustLow => "justify_low",
            Align::Distributed => "distributed",
        }))
    }

    #[pyo3(signature = (slide, id, row, col, alignment=None))]
    pub fn set_cell_alignment(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        alignment: Option<&str>,
    ) -> PyResult<()> {
        let (row, col) = cell_anchor(self.table(slide, id)?, row, col)?;
        let value = match alignment {
            None => None,
            Some("left") => Some(Align::Left),
            Some("center") => Some(Align::Center),
            Some("right") => Some(Align::Right),
            Some("justify") => Some(Align::Justify),
            _ => {
                return Err(PyValueError::new_err(
                    "cell alignment must be left, center, right, justify or None",
                ))
            }
        };
        self.mutate_table(slide, id, false, |table| {
            for paragraph in &mut table.rows[row].cells[col].text.paragraphs {
                paragraph.props.align = value;
            }
            Ok(())
        })
    }

    pub fn merge_cells(
        &mut self,
        slide: usize,
        id: u32,
        r0: usize,
        c0: usize,
        r1: usize,
        c1: usize,
    ) -> PyResult<()> {
        let table = self.table(slide, id)?;
        validate_merge(table, r0, c0, r1, c1)?;
        self.mutate_table(slide, id, true, |table| {
            if !table.merge(r0, c0, r1, c1) {
                return Err(PyValueError::new_err(
                    "upstream could not merge the requested rectangle",
                ));
            }
            Ok(())
        })
    }

    #[pyo3(signature = (slide, id, row, col, columns=None))]
    pub fn split_cell(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        columns: Option<usize>,
    ) -> PyResult<()> {
        let table = self.table(slide, id)?;
        let (row, col) = cell_anchor(table, row, col)?;
        let cell = &table.rows[row].cells[col];
        if cell.grid_span <= 1 && cell.row_span <= 1 {
            return Err(PyValueError::new_err(
                "PPTX split requires an existing merged cell",
            ));
        }
        if columns.is_some_and(|columns| cell.row_span != 1 || columns != cell.grid_span as usize) {
            return Err(PyValueError::new_err("PPTX explicit split columns must match an existing horizontal merge; omit columns to restore the merged grid"));
        }
        self.mutate_table(slide, id, true, |table| {
            if !table.split(row, col) {
                return Err(PyValueError::new_err(
                    "upstream could not split the merged cell",
                ));
            }
            Ok(())
        })
    }

    pub fn insert_row(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        let table = self.table(slide, id)?;
        if index > table.n_rows() {
            return Err(PyIndexError::new_err(
                "table insertion row index out of range",
            ));
        }
        preflight_grid_insert(self.document()?, table, true)?;
        let height = table.rows[index.saturating_sub(1)].height;
        dimensions(table.width(), table.height() + height)?;
        self.mutate_table(slide, id, true, |table| {
            table.insert_row(index);
            Ok(())
        })
    }

    pub fn delete_row(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        let table = self.table(slide, id)?;
        if index >= table.n_rows() {
            return Err(PyIndexError::new_err("table row index out of range"));
        }
        if table.n_rows() == 1 {
            return Err(PyValueError::new_err("cannot delete the final table row"));
        }
        self.mutate_table(slide, id, true, |table| {
            if !table.delete_row(index) {
                return Err(PyValueError::new_err(
                    "upstream could not delete the table row",
                ));
            }
            Ok(())
        })
    }

    pub fn insert_column(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        let table = self.table(slide, id)?;
        if index > table.n_cols() {
            return Err(PyIndexError::new_err(
                "table insertion column index out of range",
            ));
        }
        preflight_grid_insert(self.document()?, table, false)?;
        dimensions(
            table.width() + table.cols[index.saturating_sub(1)],
            table.height(),
        )?;
        self.mutate_table(slide, id, true, |table| {
            table.insert_col(index);
            Ok(())
        })
    }

    pub fn delete_column(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        let table = self.table(slide, id)?;
        if index >= table.n_cols() {
            return Err(PyIndexError::new_err("table column index out of range"));
        }
        if table.n_cols() == 1 {
            return Err(PyValueError::new_err(
                "cannot delete the final table column",
            ));
        }
        self.mutate_table(slide, id, true, |table| {
            if !table.delete_col(index) {
                return Err(PyValueError::new_err(
                    "upstream could not delete the table column",
                ));
            }
            Ok(())
        })
    }

    pub fn set_cell_text(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        text: &str,
    ) -> PyResult<()> {
        let cell = table_cell(self.table(slide, id)?, row, col)?;
        text_value(text)?;
        let count = paragraph_count(text);
        let props = deckcraft_model::edit::props_at(&cell.text, (0, 0));
        let defaults = cell
            .text
            .paragraphs
            .first()
            .map_or(0, |p| props_cost(&p.def_props) + props_cost(&p.end_props));
        let projected = ContentBudget {
            text: text.len(),
            paragraphs: count,
            runs: count,
            properties: count
                .saturating_mul(defaults.saturating_add(props_cost(&props).saturating_mul(2))),
            ..Default::default()
        };
        content_budget(self.document()?)?
            .replacing(body_budget(&cell.text), projected)
            .check()?;
        self.mutate(|document| {
            let cell = cell_mut(document, slide, id, row, col)?;
            replace_text(&mut cell.text, text);
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn cell_background(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<Option<(u8, u8, u8)>> {
        let cell = table_cell(self.table(slide, id)?, row, col)?;
        Ok(match &cell.fill {
            Some(Fill::Solid {
                color:
                    ColorRef {
                        base: ColorBase::Rgb { rgb },
                        mods,
                    },
            }) if mods.is_empty() => Some((rgb.r, rgb.g, rgb.b)),
            _ => None,
        })
    }

    pub fn set_cell_background(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        color: (u8, u8, u8),
    ) -> PyResult<()> {
        self.shape(slide, id)?;
        self.mutate(|document| {
            cell_mut(document, slide, id, row, col)?.fill = Some(Fill::solid(ColorRef::rgb(
                Rgba::rgb(color.0, color.1, color.2),
            )));
            Ok(())
        })
    }

    pub fn cell_vertical_alignment(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<Option<&'static str>> {
        let cell = table_cell(self.table(slide, id)?, row, col)?;
        Ok(cell.anchor.map(|value| match value {
            Anchor::Top => "top",
            Anchor::Middle => "center",
            Anchor::Bottom => "bottom",
            Anchor::Justified => "justified",
            Anchor::Distributed => "distributed",
        }))
    }

    pub fn set_cell_vertical_alignment(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        alignment: &str,
    ) -> PyResult<()> {
        self.shape(slide, id)?;
        let anchor = match alignment {
            "top" => Anchor::Top,
            "center" => Anchor::Middle,
            "bottom" => Anchor::Bottom,
            _ => {
                return Err(PyValueError::new_err(
                    "vertical alignment must be top, center or bottom",
                ))
            }
        };
        self.mutate(|document| {
            cell_mut(document, slide, id, row, col)?.anchor = Some(anchor);
            Ok(())
        })
    }

    pub fn set_text_color(&mut self, slide: usize, id: u32, color: (u8, u8, u8)) -> PyResult<()> {
        self.set_text_format(slide, id, None, Some(color), None, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (slide, id, size=None, color=None, bold=None, italic=None, font=None))]
    pub fn set_text_format(
        &mut self,
        slide: usize,
        id: u32,
        size: Option<f64>,
        color: Option<(u8, u8, u8)>,
        bold: Option<bool>,
        italic: Option<bool>,
        font: Option<&str>,
    ) -> PyResult<()> {
        self.shape(slide, id)?;
        if let Some(size) = size {
            font_size(size)?;
        }
        if let Some(font) = font {
            text_value(font)?;
            if font.trim().is_empty() || font.chars().count() > 255 {
                return Err(PyValueError::new_err(
                    "font family must contain 1 to 255 characters",
                ));
            }
        }
        let apply = |props: &mut RunProps| {
            if let Some(value) = size {
                props.size = Some(value);
            }
            if let Some(value) = color {
                props.fill = Some(Fill::solid(ColorRef::rgb(Rgba::rgb(
                    value.0, value.1, value.2,
                ))));
            }
            if let Some(value) = bold {
                props.bold = Some(value);
            }
            if let Some(value) = italic {
                props.italic = Some(value);
            }
            if let Some(value) = font {
                props.font = Some(value.to_owned());
            }
        };
        let body = self.text_body(slide, id)?;
        let old = body_budget(body);
        let mut projected = old;
        for paragraph in &body.paragraphs {
            for props in paragraph
                .runs
                .iter()
                .map(|run| &run.props)
                .chain([&paragraph.def_props, &paragraph.end_props])
            {
                let mut updated = props.clone();
                apply(&mut updated);
                projected.properties = projected
                    .properties
                    .saturating_sub(props_cost(props))
                    .saturating_add(props_cost(&updated));
            }
        }
        content_budget(self.document()?)?
            .replacing(old, projected)
            .check()?;
        self.mutate(|document| {
            let body = shape_mut(document, slide, id)?
                .text
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("shape has no text body"))?;
            deckcraft_model::edit::format_all(body, &apply);
            for paragraph in &mut body.paragraphs {
                apply(&mut paragraph.def_props);
            }
            Ok(())
        })
    }

    pub fn text_format(&self, slide: usize, id: u32) -> PyResult<TextFormat> {
        let body = self
            .shape(slide, id)?
            .text
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("shape has no text body"))?;
        let values: Vec<_> = body
            .paragraphs
            .iter()
            .flat_map(|paragraph| {
                if paragraph.runs.is_empty() {
                    vec![&paragraph.end_props]
                } else {
                    paragraph.runs.iter().map(|run| &run.props).collect()
                }
            })
            .collect();
        Ok((
            consistent(values.iter().map(|props| props.size)),
            consistent(values.iter().map(|props| match &props.fill {
                Some(Fill::Solid {
                    color:
                        ColorRef {
                            base: ColorBase::Rgb { rgb },
                            mods,
                        },
                }) if mods.is_empty() => Some((rgb.r, rgb.g, rgb.b)),
                _ => None,
            })),
            consistent(values.iter().map(|props| props.bold)),
            consistent(values.iter().map(|props| props.italic)),
            consistent(values.iter().map(|props| props.font.clone())),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (slide, rows, x, y, width, height, size=18.0))]
    pub fn add_table(
        &mut self,
        slide: usize,
        rows: Vec<Vec<String>>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        size: f64,
    ) -> PyResult<u32> {
        self.check_shape(slide, x, y, width, height)?;
        font_size(size)?;
        let columns = rows.first().map_or(0, Vec::len);
        if rows.is_empty()
            || columns == 0
            || rows.len() > 1000
            || columns > 1000
            || rows.len().saturating_mul(columns) > 100_000
            || rows.iter().any(|row| row.len() != columns)
        {
            return Err(PyValueError::new_err(
                "table must be rectangular, nonempty, at most 1000 rows/columns and 100000 cells",
            ));
        }
        let mut projected = content_budget(self.document()?)?;
        projected.cells = projected
            .cells
            .saturating_add(rows.len().saturating_mul(columns));
        projected.check()?;
        if !grid_size(width / columns as f64) || !grid_size(height / rows.len() as f64) {
            return Err(PyValueError::new_err(
                "table grid dimensions must be at least 0.05 points",
            ));
        }
        let mut bytes = 0_usize;
        for text in rows.iter().flatten() {
            text_value(text)?;
            bytes = bytes
                .checked_add(text.len())
                .ok_or_else(|| EngineError::ResourceLimit("table text size overflow".into()))?;
        }
        self.check_text_budget(bytes)?;
        let paragraphs = rows
            .iter()
            .flatten()
            .map(|text| paragraph_count(text))
            .sum();
        self.check_paragraph_budget(paragraphs, 0)?;
        let mut table = Table::new(rows.len(), columns, width, height / rows.len() as f64);
        for (row, values) in table.rows.iter_mut().zip(rows) {
            for (cell, text) in row.cells.iter_mut().zip(values) {
                cell.text = text_body(&text, size);
            }
        }
        let shape = Shape {
            xfrm: Some(Xfrm::new(x, y, width, height)),
            kind: ShapeKind::Table(table),
            ..Shape::default()
        };
        let id = self.mutate(|document| insert_shape(document, slide, shape))?;
        self.text_bytes += bytes;
        self.shapes += 1;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (slide, data, x, y, width, height, alt=""))]
    pub fn add_image(
        &mut self,
        slide: usize,
        data: &[u8],
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        alt: &str,
    ) -> PyResult<u32> {
        self.check_shape(slide, x, y, width, height)?;
        text_value(alt)?;
        self.check_text_budget(alt.len())?;
        validate_png(data)?;
        if data.len() > MAX_MEDIA.saturating_sub(self.media_bytes)
            && !self
                .document()?
                .media
                .iter()
                .any(|media| media.data.as_slice() == data)
        {
            return Err(EngineError::ResourceLimit(
                "presentation images exceed 64 MiB of live media".into(),
            )
            .into());
        }
        let id = self.mutate(|document| {
            let media = document
                .add_media("image.png", "image/png", data.to_vec())
                .ok_or_else(|| {
                    EngineError::ResourceLimit("presentation object IDs exhausted".into())
                })?;
            let shape = Shape {
                xfrm: Some(Xfrm::new(x, y, width, height)),
                kind: ShapeKind::Picture {
                    fill: PictureFill {
                        media,
                        ..PictureFill::default()
                    },
                },
                descr: alt.to_owned(),
                ..Shape::default()
            };
            let id = insert_shape(document, slide, shape)?;
            check_live_media(document)?;
            Ok(id)
        })?;
        self.refresh_budgets()?;
        Ok(id)
    }

    pub fn picture_data(&self, slide: usize, id: u32) -> PyResult<Vec<u8>> {
        let fill = self.picture_fill(slide, id)?;
        let media = self
            .document()?
            .media(fill.media)
            .ok_or_else(|| EngineError::Malformed("picture media is missing".into()))?;
        if media.link.is_some() || media.content_type != "image/png" {
            return Err(EngineError::Unsupported(
                "picture editing requires owned PNG media".into(),
            )
            .into());
        }
        if media.data.len() > MAX_IMAGE {
            return Err(EngineError::ResourceLimit("picture data exceeds 16 MiB".into()).into());
        }
        Ok(media.data.as_ref().clone())
    }

    pub fn set_picture_data(&mut self, slide: usize, id: u32, data: &[u8]) -> PyResult<()> {
        self.picture_fill(slide, id)?;
        organization_profile(self.document()?)?;
        validate_png(data)?;
        self.mutate(|document| {
            let media = document
                .add_media("image.png", "image/png", data.to_vec())
                .ok_or_else(|| {
                    EngineError::ResourceLimit("presentation object IDs exhausted".into())
                })?;
            let ShapeKind::Picture { fill } = &mut shape_mut(document, slide, id)?.kind else {
                return Err(PyValueError::new_err("shape is not a picture"));
            };
            fill.media = media;
            prune_media(document);
            check_live_media(document)?;
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn picture_alt(&self, slide: usize, id: u32) -> PyResult<String> {
        self.picture_fill(slide, id)?;
        Ok(self.shape(slide, id)?.descr.clone())
    }

    pub fn set_picture_alt(&mut self, slide: usize, id: u32, alt: &str) -> PyResult<()> {
        self.picture_fill(slide, id)?;
        text_value(alt)?;
        let old = self.shape(slide, id)?.descr.len();
        self.check_text_budget(alt.len().saturating_sub(old))?;
        self.mutate(|document| {
            shape_mut(document, slide, id)?.descr = alt.to_owned();
            Ok(())
        })?;
        self.refresh_budgets()
    }

    pub fn picture_crop(&self, slide: usize, id: u32) -> PyResult<(f64, f64, f64, f64)> {
        let [left, top, right, bottom] = self.picture_fill(slide, id)?.crop;
        Ok((left, top, right, bottom))
    }

    pub fn set_picture_crop(
        &mut self,
        slide: usize,
        id: u32,
        crop: (f64, f64, f64, f64),
    ) -> PyResult<()> {
        self.picture_fill(slide, id)?;
        let crop = [crop.0, crop.1, crop.2, crop.3];
        if crop
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=0.45).contains(v))
        {
            return Err(PyValueError::new_err(
                "crop fractions must be finite and between 0 and 0.45 in left/top/right/bottom order",
            ));
        }
        self.mutate(|document| {
            let ShapeKind::Picture { fill } = &mut shape_mut(document, slide, id)?.kind else {
                return Err(PyValueError::new_err("shape is not a picture"));
            };
            fill.crop = crop;
            Ok(())
        })
    }

    pub fn picture_flips(&self, slide: usize, id: u32) -> PyResult<(bool, bool)> {
        self.picture_fill(slide, id)?;
        let transform = self.shape_transform(slide, id)?;
        Ok((transform.flip_h, transform.flip_v))
    }

    #[pyo3(signature = (slide,id,flip_h=None,flip_v=None))]
    pub fn set_picture_flips(
        &mut self,
        slide: usize,
        id: u32,
        flip_h: Option<bool>,
        flip_v: Option<bool>,
    ) -> PyResult<()> {
        self.picture_fill(slide, id)?;
        let effective = self.shape_transform(slide, id)?;
        self.mutate(|document| {
            let transform = shape_mut(document, slide, id)?
                .xfrm
                .get_or_insert(effective);
            if let Some(value) = flip_h {
                transform.flip_h = value;
            }
            if let Some(value) = flip_v {
                transform.flip_v = value;
            }
            Ok(())
        })
    }
}

enum FormatChange {
    Bold(Option<bool>),
    Italic(Option<bool>),
    Size(Option<f64>),
    Font(Option<String>),
    Color(Option<ColorRef>),
    Underline(Option<String>),
    Strike(Option<Strike>),
    Baseline(Option<f64>),
    Highlight(Option<ColorRef>),
}

struct FormatPatch(Vec<FormatChange>);

// Keep public names aligned with DOCX; project to DrawingML only at this adapter.
const UNDERLINES: &[(&str, &str)] = &[
    ("none", "none"),
    ("single", "sng"),
    ("words", "words"),
    ("double", "dbl"),
    ("thick", "heavy"),
    ("dotted", "dotted"),
    ("dash", "dash"),
    ("dot_dash", "dotDash"),
    ("dot_dot_dash", "dotDotDash"),
    ("wave", "wavy"),
    ("double_wave", "wavyDbl"),
    ("thick_dotted", "dottedHeavy"),
    ("thick_dash", "dashHeavy"),
    ("long_dash", "dashLong"),
    ("thick_long_dash", "dashLongHeavy"),
    ("thick_dot_dash", "dotDashHeavy"),
    ("thick_dot_dot_dash", "dotDotDashHeavy"),
    ("thick_wave", "wavyHeavy"),
];

impl FormatPatch {
    fn parse(source: &str) -> PyResult<Self> {
        if source.len() > MAX_PATCH {
            return Err(EngineError::ResourceLimit("format patch exceeds 16 KiB".into()).into());
        }
        let value: serde_json::Value = serde_json::from_str(source)
            .map_err(|_| PyValueError::new_err("format patch must be a JSON object"))?;
        let object = value
            .as_object()
            .ok_or_else(|| PyValueError::new_err("format patch must be a flat object"))?;
        let mut changes = Vec::new();
        for (key, value) in object {
            let invalid =
                || PyValueError::new_err(format!("invalid PPTX formatting value for {key}"));
            let change = match key.as_str() {
                "bold" | "italic" => {
                    let v = if value.is_null() {
                        None
                    } else {
                        Some(value.as_bool().ok_or_else(invalid)?)
                    };
                    if key == "bold" {
                        FormatChange::Bold(v)
                    } else {
                        FormatChange::Italic(v)
                    }
                }
                "font_size" => {
                    let v = if value.is_null() {
                        None
                    } else {
                        let v = value.as_f64().ok_or_else(invalid)?;
                        font_size(v)?;
                        Some(v)
                    };
                    FormatChange::Size(v)
                }
                "font" => {
                    let v = if value.is_null() {
                        None
                    } else {
                        let v = value.as_str().ok_or_else(invalid)?;
                        text_value(v)?;
                        if v.trim().is_empty()
                            || v.chars().count() > 255
                            || v.chars().any(char::is_control)
                        {
                            return Err(invalid());
                        }
                        Some(v.to_owned())
                    };
                    FormatChange::Font(v)
                }
                "color" | "highlight" => {
                    let v = if value.is_null() {
                        None
                    } else {
                        Some(parse_color(value.as_str().ok_or_else(invalid)?)?)
                    };
                    if key == "color" {
                        FormatChange::Color(v)
                    } else {
                        FormatChange::Highlight(v)
                    }
                }
                "underline" => {
                    let v = match value {
                        serde_json::Value::Null => None,
                        serde_json::Value::Bool(v) => Some(if *v { "sng" } else { "none" }.into()),
                        serde_json::Value::String(v) => Some(
                            UNDERLINES
                                .iter()
                                .find(|(name, _)| *name == v)
                                .map(|(_, native)| (*native).to_owned())
                                .ok_or_else(invalid)?,
                        ),
                        _ => return Err(invalid()),
                    };
                    FormatChange::Underline(v)
                }
                "strike" => {
                    let v = match value {
                        serde_json::Value::Null => None,
                        serde_json::Value::Bool(v) => {
                            Some(if *v { Strike::Single } else { Strike::None })
                        }
                        serde_json::Value::String(v) if v == "single" => Some(Strike::Single),
                        serde_json::Value::String(v) if v == "double" => Some(Strike::Double),
                        _ => return Err(invalid()),
                    };
                    FormatChange::Strike(v)
                }
                "script" => FormatChange::Baseline(match value.as_str() {
                    Some("baseline") => Some(0.0),
                    Some("superscript") => Some(0.3),
                    Some("subscript") => Some(-0.25),
                    _ if value.is_null() => None,
                    _ => return Err(invalid()),
                }),
                _ => {
                    return Err(PyValueError::new_err(format!(
                        "unsupported PPTX formatting property: {key}"
                    )))
                }
            };
            changes.push(change);
        }
        Ok(Self(changes))
    }

    fn apply(&self, props: &mut RunProps) {
        for change in &self.0 {
            match change {
                FormatChange::Bold(v) => props.bold = *v,
                FormatChange::Italic(v) => props.italic = *v,
                FormatChange::Size(v) => props.size = *v,
                FormatChange::Font(v) => props.font = v.clone(),
                FormatChange::Color(v) => props.fill = v.clone().map(Fill::solid),
                FormatChange::Underline(v) => props.underline = v.clone(),
                FormatChange::Strike(v) => props.strike = *v,
                FormatChange::Baseline(v) => props.baseline = *v,
                FormatChange::Highlight(v) => props.highlight = v.clone(),
            }
        }
    }
}

fn parse_color(value: &str) -> PyResult<ColorRef> {
    let v = value.strip_prefix('#').unwrap_or(value);
    if v.len() != 6 || !v.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(PyValueError::new_err(
            "color must be a six-digit RGB hex string",
        ));
    }
    let rgb = u32::from_str_radix(v, 16).map_err(|_| PyValueError::new_err("invalid RGB color"))?;
    Ok(color_ref(((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)))
}

type TextPositions = (deckcraft_model::edit::Pos, deckcraft_model::edit::Pos);

fn text_positions(body: &TextBody, start: usize, end: usize) -> PyResult<TextPositions> {
    if start > end || end > body.char_len() || body.paragraphs.is_empty() {
        return Err(PyIndexError::new_err(
            "text range is outside the shape text",
        ));
    }
    let position = |mut offset: usize| {
        for (index, paragraph) in body.paragraphs.iter().enumerate() {
            let size = paragraph.char_len();
            if offset <= size {
                return (index, offset);
            }
            offset -= size + 1;
        }
        // Bounds checked above; the final paragraph end is a valid caret.
        (
            body.paragraphs.len() - 1,
            body.paragraphs.last().map_or(0, |p| p.char_len()),
        )
    };
    Ok((position(start), position(end)))
}

fn range_has_text(
    body: &TextBody,
    a: deckcraft_model::edit::Pos,
    b: deckcraft_model::edit::Pos,
) -> bool {
    body.paragraphs
        .iter()
        .enumerate()
        .take(b.0 + 1)
        .skip(a.0)
        .any(|(pi, p)| {
            let from = if pi == a.0 { a.1 } else { 0 };
            let to = if pi == b.0 { b.1 } else { p.char_len() };
            to > from
        })
}

fn validate_uri(value: &str) -> PyResult<()> {
    text_value(value)?;
    let valid = ["https://", "http://", "mailto:"].iter().any(|prefix| {
        value
            .get(..prefix.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
            && value.len() > prefix.len()
    });
    if value.len() > 8192 || !valid || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(PyValueError::new_err("link must be a nonempty http, https or mailto URI of at most 8192 bytes without whitespace or controls"));
    }
    Ok(())
}

fn make_link(
    document: &Presentation,
    url: Option<&str>,
    target: Option<u32>,
    tooltip: &str,
) -> PyResult<Option<Hyperlink>> {
    text_value(tooltip)?;
    if tooltip.len() > 4096 || tooltip.chars().any(char::is_control) {
        return Err(PyValueError::new_err(
            "link tooltip must be at most 4096 bytes without controls",
        ));
    }
    let action = match (url, target) {
        (Some(url), None) => {
            validate_uri(url)?;
            Action::Url { url: url.into() }
        }
        (None, Some(id)) => {
            if document.slide(SlideId(id)).is_none() {
                return Err(PyValueError::new_err("target slide no longer exists"));
            }
            Action::Slide { slide: SlideId(id) }
        }
        (None, None) if tooltip.is_empty() => return Ok(None),
        (None, None) => return Err(PyValueError::new_err("a tooltip requires a link target")),
        _ => {
            return Err(PyValueError::new_err(
                "choose either a URL or a target slide",
            ))
        }
    };
    Ok(Some(Hyperlink {
        action,
        tooltip: tooltip.into(),
        highlight_click: false,
        sound: None,
        stop_sound: false,
        zoom_return: None,
    }))
}

fn link_value(link: Option<&Hyperlink>) -> PyResult<LinkValue> {
    match link {
        None => Ok(None),
        Some(link) => match &link.action {
            Action::Url { url } => Ok(Some((Some(url.clone()), None, link.tooltip.clone()))),
            Action::Slide { slide } => Ok(Some((None, Some(slide.0), link.tooltip.clone()))),
            _ => Err(EngineError::Unsupported("hyperlink action is unavailable".into()).into()),
        },
    }
}

#[derive(Default, Clone, Copy)]
struct ContentBudget {
    text: usize,
    properties: usize,
    runs: usize,
    paragraphs: usize,
    cells: usize,
}

impl ContentBudget {
    fn check(self) -> PyResult<Self> {
        if self.text > MAX_TEXT
            || self.properties > MAX_PROPERTIES
            || self.runs > MAX_RUNS
            || self.paragraphs > MAX_PARAGRAPHS
            || self.cells > MAX_CELLS
        {
            return Err(EngineError::ResourceLimit("presentation exceeds text (16 MiB), formatting/link properties (16 MiB), runs (100000), paragraphs (100000) or table cells (100000) limit".into()).into());
        }
        Ok(self)
    }
    fn add(&mut self, other: Self) {
        self.text = self.text.saturating_add(other.text);
        self.properties = self.properties.saturating_add(other.properties);
        self.runs = self.runs.saturating_add(other.runs);
        self.paragraphs = self.paragraphs.saturating_add(other.paragraphs);
        self.cells = self.cells.saturating_add(other.cells);
    }
    fn replacing(self, old: Self, new: Self) -> Self {
        Self {
            text: self.text.saturating_sub(old.text).saturating_add(new.text),
            properties: self
                .properties
                .saturating_sub(old.properties)
                .saturating_add(new.properties),
            runs: self.runs.saturating_sub(old.runs).saturating_add(new.runs),
            paragraphs: self
                .paragraphs
                .saturating_sub(old.paragraphs)
                .saturating_add(new.paragraphs),
            cells: self
                .cells
                .saturating_sub(old.cells)
                .saturating_add(new.cells),
        }
    }
}

fn link_cost(link: &Option<Hyperlink>) -> usize {
    link.as_ref().map_or(0, |link| {
        link.tooltip.len().saturating_add(match &link.action {
            Action::Url { url } => url.len(),
            Action::Program { path } => path.len(),
            Action::CustomShow { name, .. } => name.len(),
            _ => 0,
        })
    })
}

fn props_cost(props: &RunProps) -> usize {
    [
        &props.font,
        &props.font_ea,
        &props.font_cs,
        &props.font_sym,
        &props.underline,
        &props.lang,
    ]
    .iter()
    .map(|v| v.as_ref().map_or(0, String::len))
    .sum::<usize>()
    .saturating_add(link_cost(&props.link))
    .saturating_add(link_cost(&props.hover))
}

fn body_budget(body: &TextBody) -> ContentBudget {
    let mut budget = ContentBudget {
        paragraphs: body.paragraphs.len(),
        text: body.paragraphs.len().saturating_sub(1),
        ..Default::default()
    };
    for paragraph in &body.paragraphs {
        budget.properties = budget
            .properties
            .saturating_add(props_cost(&paragraph.def_props))
            .saturating_add(props_cost(&paragraph.end_props));
        budget.runs = budget.runs.saturating_add(paragraph.runs.len());
        for run in &paragraph.runs {
            budget.text = budget.text.saturating_add(run.content().len());
            budget.properties = budget.properties.saturating_add(props_cost(&run.props));
        }
    }
    budget
}

fn content_budget(document: &Presentation) -> PyResult<ContentBudget> {
    let mut budget = ContentBudget::default();
    for slide in &document.slides {
        budget.text = budget.text.saturating_add(slide.name.len());
        budget.add(body_budget(&slide.notes));
        visit_shapes(&slide.shapes, &mut |shape| {
            budget.text = budget.text.saturating_add(shape.descr.len());
            budget.properties = budget
                .properties
                .saturating_add(link_cost(&shape.click))
                .saturating_add(link_cost(&shape.hover));
            if let Some(body) = &shape.text {
                budget.add(body_budget(body));
            }
            if let ShapeKind::Table(table) = &shape.kind {
                validate_table_grid(table)?;
                budget.cells = budget
                    .cells
                    .saturating_add(table.n_rows().saturating_mul(table.n_cols()));
                for cell in table.rows.iter().flat_map(|row| &row.cells) {
                    budget.add(body_budget(&cell.text));
                }
            }
            budget.check().map(|_| ())
        })?;
        budget.check()?;
    }
    budget.check()
}

fn visit_shapes(
    shapes: &[Shape],
    visitor: &mut impl FnMut(&Shape) -> PyResult<()>,
) -> PyResult<()> {
    let mut stack = vec![(shapes.iter(), 0usize)];
    let mut count = 0usize;
    while let Some((iter, depth)) = stack.last_mut() {
        let Some(shape) = iter.next() else {
            stack.pop();
            continue;
        };
        count += 1;
        if count > MAX_SHAPES || *depth > 32 {
            return Err(EngineError::ResourceLimit(
                "presentation shape count or group depth exceeds limit".into(),
            )
            .into());
        }
        visitor(shape)?;
        if let ShapeKind::Group { children, .. } = &shape.kind {
            let next = *depth + 1;
            stack.push((children.iter(), next));
        }
    }
    Ok(())
}

fn edited_body(
    document: &Presentation,
    slide: usize,
    id: u32,
    start: usize,
    end: usize,
    text: &str,
    patch: Option<&FormatPatch>,
) -> PyResult<TextBody> {
    text_value(text)?;
    let body = document
        .slides
        .get(slide)
        .and_then(|s| s.shape(ShapeId(id)))
        .and_then(|s| s.text.as_ref())
        .ok_or_else(|| PyValueError::new_err("shape has no text body"))?;
    let (a, b) = text_positions(body, start, end)?;
    // props_at describes the character before the caret. Replacement takes the
    // first selected character's appearance, including at a run boundary.
    let props_position = if start < end && a.1 < body.paragraphs[a.0].char_len() {
        (a.0, a.1 + 1)
    } else {
        a
    };
    let mut props = deckcraft_model::edit::props_at(body, props_position);
    if let Some(patch) = patch {
        // Appending inherits appearance, not an adjacent label's navigation target.
        props.link = None;
        props.hover = None;
        patch.apply(&mut props);
    }
    let total = content_budget(document)?;
    let old = body_budget(body);
    let removed = deckcraft_model::edit::text_range(body, a, b).len();
    if total
        .text
        .saturating_sub(removed)
        .saturating_add(text.len())
        > MAX_TEXT
    {
        return Err(EngineError::ResourceLimit("presentation text exceeds 16 MiB".into()).into());
    }
    let mut edited = body.clone();
    deckcraft_model::edit::delete(&mut edited, a, b);
    let mut projected = body_budget(&edited);
    let added_paragraphs = paragraph_count(text) - 1;
    // Preflight cloning of insertion and paragraph properties before upstream insert.
    projected.paragraphs = projected.paragraphs.saturating_add(added_paragraphs);
    projected.runs = projected
        .runs
        .saturating_add(added_paragraphs)
        .saturating_add(3);
    projected.text = projected.text.saturating_add(text.len());
    let paragraph_defaults = edited
        .paragraphs
        .get(a.0)
        .map_or(0, |p| props_cost(&p.def_props) + props_cost(&p.end_props));
    projected.properties = projected
        .properties
        .saturating_add((added_paragraphs + 3).saturating_mul(props_cost(&props)))
        .saturating_add(added_paragraphs.saturating_mul(paragraph_defaults));
    total.replacing(old, projected).check()?;
    deckcraft_model::edit::insert(&mut edited, a, text, Some(props));
    total.replacing(old, body_budget(&edited)).check()?;
    Ok(edited)
}

fn preflight_format(
    document: &Presentation,
    body: &TextBody,
    a: deckcraft_model::edit::Pos,
    b: deckcraft_model::edit::Pos,
    apply: &impl Fn(&mut RunProps),
    preserve_ends: bool,
) -> PyResult<()> {
    let old = body_budget(body);
    let mut projected = old;
    for (pi, paragraph) in body.paragraphs.iter().enumerate().take(b.0 + 1).skip(a.0) {
        let from = if pi == a.0 { a.1 } else { 0 };
        let to = if pi == b.0 { b.1 } else { paragraph.char_len() };
        let mut offset = 0;
        for run in &paragraph.runs {
            let next = offset + run.char_len();
            if offset < to && next > from {
                let mut props = run.props.clone();
                apply(&mut props);
                let extra = usize::from(offset < from) + usize::from(next > to);
                projected.runs = projected.runs.saturating_add(extra);
                projected.properties = projected
                    .properties
                    .saturating_sub(props_cost(&run.props))
                    .saturating_add(props_cost(&props))
                    .saturating_add(extra.saturating_mul(props_cost(&run.props)));
            }
            offset = next;
        }
        if !preserve_ends
            && (paragraph.runs.is_empty() || (a == b && from == to) || to == paragraph.char_len())
        {
            let mut props = paragraph.end_props.clone();
            apply(&mut props);
            projected.properties = projected
                .properties
                .saturating_sub(props_cost(&paragraph.end_props))
                .saturating_add(props_cost(&props));
        }
    }
    content_budget(document)?
        .replacing(old, projected)
        .check()
        .map(|_| ())
}

fn grid_size(value: f64) -> bool {
    value.is_finite() && (MIN_GRID..=MAX_SIDE).contains(&value)
}

fn validate_table_grid(table: &Table) -> PyResult<()> {
    if table.n_rows() == 0
        || table.n_cols() == 0
        || table.n_rows() > 1000
        || table.n_cols() > 1000
        || table.n_rows().saturating_mul(table.n_cols()) > MAX_CELLS
        || table
            .rows
            .iter()
            .any(|row| row.cells.len() != table.n_cols())
    {
        return Err(EngineError::ResourceLimit(
            "table must have a rectangular grid of 1..1000 rows/columns and at most 100000 cells"
                .into(),
        )
        .into());
    }
    if table.cols.iter().any(|v| !grid_size(*v))
        || table.rows.iter().any(|row| !grid_size(row.height))
    {
        return Err(PyValueError::new_err(
            "table grid widths and minimum heights must be finite 0.05..14400 point values",
        ));
    }
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| !cell.is_covered())
        {
            let end_row = r.saturating_add(cell.row_span.max(1) as usize);
            let end_col = c.saturating_add(cell.grid_span.max(1) as usize);
            if end_row > table.n_rows() || end_col > table.n_cols() {
                return Err(
                    EngineError::Malformed("table merge extends outside its grid".into()).into(),
                );
            }
            if let Some([left, right, top, bottom]) = cell.margins {
                let width = table.cols[c..end_col].iter().sum::<f64>();
                if [left, right, top, bottom]
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=MAX_SIDE).contains(v))
                    || left + right >= width
                    || top + bottom > MAX_SIDE
                {
                    return Err(PyValueError::new_err(
                        "table layout would overflow explicit cell margins",
                    ));
                }
            }
        }
    }
    dimensions(table.width(), table.height())
}

fn cell_anchor(table: &Table, row: usize, col: usize) -> PyResult<(usize, usize)> {
    table
        .cell(row, col)
        .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))?;
    let (mut r, mut c) = (row, col);
    let invalid = || EngineError::Malformed("table covered cell has no valid merge anchor".into());
    while table.cell(r, c).ok_or_else(invalid)?.h_merge {
        c = c.checked_sub(1).ok_or_else(invalid)?;
    }
    while table.cell(r, c).ok_or_else(invalid)?.v_merge {
        r = r.checked_sub(1).ok_or_else(invalid)?;
    }
    let cell = table.cell(r, c).ok_or_else(invalid)?;
    let end_row = r.saturating_add(cell.row_span.max(1) as usize);
    let end_col = c.saturating_add(cell.grid_span.max(1) as usize);
    if cell.is_covered()
        || row >= end_row
        || col >= end_col
        || end_row > table.n_rows()
        || end_col > table.n_cols()
    {
        return Err(invalid().into());
    }
    Ok((r, c))
}

fn table_cell(table: &Table, row: usize, col: usize) -> PyResult<&deckcraft_model::Cell> {
    let (row, col) = cell_anchor(table, row, col)?;
    table
        .cell(row, col)
        .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))
}

fn validate_merge(table: &Table, r0: usize, c0: usize, r1: usize, c1: usize) -> PyResult<()> {
    validate_table_grid(table)?;
    if r0 > r1 || c0 > c1 || r1 >= table.n_rows() || c1 >= table.n_cols() || (r0 == r1 && c0 == c1)
    {
        return Err(PyValueError::new_err(
            "merge requires an ordered rectangle of at least two cells within the grid",
        ));
    }
    // Prevent upstream's intentional merge expansion: only wholly enclosed
    // merges may intersect the requested rectangle. Native merge owns the edit.
    for (r, row) in table.rows.iter().enumerate() {
        for (c, cell) in row
            .cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| !cell.is_covered())
        {
            let last_r = r.saturating_add(cell.row_span.max(1) as usize - 1);
            let last_c = c.saturating_add(cell.grid_span.max(1) as usize - 1);
            if last_r >= table.n_rows() || last_c >= table.n_cols() {
                return Err(
                    EngineError::Malformed("table merge extends outside its grid".into()).into(),
                );
            }
            let intersects = r <= r1 && last_r >= r0 && c <= c1 && last_c >= c0;
            if intersects && !(r >= r0 && c >= c0 && last_r <= r1 && last_c <= c1) {
                return Err(PyValueError::new_err(
                    "merge cannot cut or partially overlap an existing merged cell",
                ));
            }
        }
    }
    Ok(())
}

fn preflight_grid_insert(document: &Presentation, table: &Table, row: bool) -> PyResult<()> {
    let (rows, cols) = (
        table.n_rows() + usize::from(row),
        table.n_cols() + usize::from(!row),
    );
    if rows > 1000 || cols > 1000 || rows.saturating_mul(cols) > MAX_CELLS {
        return Err(EngineError::ResourceLimit(
            "insertion exceeds table row, column or cell limit".into(),
        )
        .into());
    }
    let added = if row { table.n_cols() } else { table.n_rows() };
    let mut budget = content_budget(document)?;
    budget.cells = budget.cells.saturating_add(added);
    budget.paragraphs = budget.paragraphs.saturating_add(added);
    budget.check().map(|_| ())
}

fn border_value(line: &Line) -> PyResult<Option<(String, f64)>> {
    if matches!(&line.fill, Some(Fill::None)) {
        return Ok(None);
    }
    if line.dash.is_some() || line.compound.is_some() || line.head.is_some() || line.tail.is_some()
    {
        return Err(EngineError::Unsupported(
            "table border is not a uniform solid RGB line".into(),
        )
        .into());
    }
    match &line.fill {
        Some(Fill::Solid {
            color:
                ColorRef {
                    base: ColorBase::Rgb { rgb },
                    mods,
                },
        }) if mods.is_empty() && rgb.a == 255 => Ok(Some((
            format!("#{:02X}{:02X}{:02X}", rgb.r, rgb.g, rgb.b),
            line.width.unwrap_or(1.0),
        ))),
        _ => Err(
            EngineError::Unsupported("table border is not an explicit solid RGB line".into())
                .into(),
        ),
    }
}

fn color_ref(color: (u8, u8, u8)) -> ColorRef {
    ColorRef::rgb(Rgba::rgb(color.0, color.1, color.2))
}

fn solid(color: (u8, u8, u8)) -> Fill {
    Fill::solid(color_ref(color))
}

fn rgb_fill(fill: &Option<Fill>) -> PyResult<Option<(u8, u8, u8)>> {
    match fill {
        None | Some(Fill::None) => Ok(None),
        Some(Fill::Solid {
            color:
                ColorRef {
                    base: ColorBase::Rgb { rgb },
                    mods,
                },
        }) if mods.is_empty() => Ok(Some((rgb.r, rgb.g, rgb.b))),
        _ => Err(EngineError::Unsupported("fill is not an explicit RGB solid color".into()).into()),
    }
}

// Only new presentations are mutable: media references can originate in authored
// picture shapes and slide backgrounds. Other model profiles are not imported.
fn check_live_media(document: &Presentation) -> PyResult<usize> {
    let bytes = document
        .media
        .iter()
        .fold(0usize, |sum, media| sum.saturating_add(media.data.len()));
    if bytes > MAX_MEDIA {
        return Err(EngineError::ResourceLimit(
            "presentation images exceed 64 MiB of live media".into(),
        )
        .into());
    }
    Ok(bytes)
}

fn prune_media(document: &mut Presentation) {
    let mut used = HashSet::new();
    for slide in &document.slides {
        if let Some(Background::Fill {
            fill: Fill::Picture(fill),
        }) = &slide.background
        {
            used.insert(fill.media);
        }
        for shape in &slide.shapes {
            if let ShapeKind::Picture { fill } = &shape.kind {
                used.insert(fill.media);
            }
        }
    }
    document.media.retain(|media| used.contains(&media.id));
}

fn stroke_size(width: f64) -> PyResult<()> {
    if !width.is_finite() || !(0.01..=100.0).contains(&width) {
        return Err(PyValueError::new_err(
            "stroke width must be finite and between 0.01 and 100 points",
        ));
    }
    Ok(())
}

fn line_geometry(x: f64, y: f64, width: f64, height: f64) -> PyResult<()> {
    if [x, y, width, height]
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_SIDE)
        || width < 0.0
        || height < 0.0
        || width + height < 1.0
    {
        return Err(PyValueError::new_err("line coordinates must be finite and within 14400 points, with nonnegative dimensions and at least 1 point extent"));
    }
    Ok(())
}

fn consistent<T: PartialEq>(mut values: impl Iterator<Item = Option<T>>) -> Option<T> {
    let first = values.next()??;
    if values.all(|value| value.as_ref() == Some(&first)) {
        Some(first)
    } else {
        None
    }
}

fn organization_profile(document: &Presentation) -> PyResult<()> {
    let body_links = |body: &TextBody| {
        body.paragraphs
            .iter()
            .flat_map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|r| &r.props)
                    .chain([&paragraph.def_props, &paragraph.end_props])
            })
            .any(|props| !safe_link(props.link.as_ref(), document) || props.hover.is_some())
    };
    let extra_refs = !document.sections.is_empty()
        || !document.custom_shows.is_empty()
        || document.show.range.is_some()
        || document.show.custom_show.is_some()
        || !document.kept_parts.is_empty()
        || document.raw_ext.is_some();
    let unsupported = document.slides.iter().any(|slide| {
        !slide.animations.is_empty()
            || !slide.comments.is_empty()
            || slide.transition.is_some()
            || slide.raw_ext.is_some()
            || !slide.tags.is_empty()
            || body_links(&slide.notes)
            || matches!(
                &slide.background,
                Some(Background::Fill {
                    fill: Fill::Picture(fill)
                }) if fill.artistic.is_some()
            )
            || slide.shapes.iter().any(|shape| {
                !safe_link(shape.click.as_ref(), document)
                    || shape.hover.is_some()
                    || shape.raw_ext.is_some()
                    || !shape.tags.is_empty()
                    || shape.diagram.is_some()
                    || matches!(&shape.fill, Some(Fill::Picture(_)))
                    || shape.text.as_ref().is_some_and(&body_links)
                    || match &shape.kind {
                        ShapeKind::Shape => false,
                        ShapeKind::Picture { fill } => fill.artistic.is_some(),
                        ShapeKind::Table(table) => {
                            matches!(&table.background, Some(Fill::Picture(_)))
                                || table.rows.iter().flat_map(|row| &row.cells).any(|cell| {
                                    body_links(&cell.text)
                                        || matches!(&cell.fill, Some(Fill::Picture(_)))
                                })
                        }
                        _ => true,
                    }
            })
    });
    if extra_refs || unsupported {
        return Err(EngineError::Unsupported("slide organization requires the authored text/table/PNG profile with URL or slide links and without animation or package references".into()).into());
    }
    Ok(())
}

fn safe_link(link: Option<&Hyperlink>, document: &Presentation) -> bool {
    link.is_none_or(|link| {
        link.sound.is_none()
            && !link.stop_sound
            && link.zoom_return.is_none()
            && match &link.action {
                Action::Url { url } => validate_uri(url).is_ok(),
                Action::Slide { slide } => document.slide(*slide).is_some(),
                _ => false,
            }
    })
}

fn reject_referenced_slide(document: &Presentation, target: SlideId) -> PyResult<()> {
    let references = |link: Option<&Hyperlink>| {
        link.is_some_and(|link| {
            matches!(link.action, Action::Slide { slide } if slide == target)
                || link.zoom_return == Some(target)
        })
    };
    let body_references = |body: &TextBody| {
        body.paragraphs.iter().any(|paragraph| {
            paragraph
                .runs
                .iter()
                .map(|run| &run.props)
                .chain([&paragraph.def_props, &paragraph.end_props])
                .any(|props| references(props.link.as_ref()) || references(props.hover.as_ref()))
        })
    };
    for slide in document.slides.iter().filter(|slide| slide.id != target) {
        let mut found = body_references(&slide.notes);
        visit_shapes(&slide.shapes, &mut |shape| {
            found |= references(shape.click.as_ref()) || references(shape.hover.as_ref());
            found |= shape.text.as_ref().is_some_and(&body_references);
            if let ShapeKind::Table(table) = &shape.kind {
                found |= table
                    .rows
                    .iter()
                    .flat_map(|row| &row.cells)
                    .any(|cell| body_references(&cell.text));
            }
            Ok(())
        })?;
        if found {
            return Err(PyValueError::new_err(
                "target slide is referenced by a surviving hyperlink",
            ));
        }
    }
    Ok(())
}

fn insert_shape(document: &mut Presentation, slide: usize, mut shape: Shape) -> PyResult<u32> {
    let id = document
        .alloc_id()
        .ok_or_else(|| EngineError::ResourceLimit("presentation object IDs exhausted".into()))?;
    shape.id = ShapeId(id);
    shape.name = format!("Object {id}");
    Arc::make_mut(&mut document.slides[slide])
        .shapes
        .push(shape);
    Ok(id)
}

fn shape_mut(document: &mut Presentation, slide: usize, id: u32) -> PyResult<&mut Shape> {
    Arc::make_mut(
        document
            .slides
            .get_mut(slide)
            .ok_or_else(|| PyIndexError::new_err("slide index out of range"))?,
    )
    .shape_mut(ShapeId(id))
    .ok_or_else(|| PyValueError::new_err("shape does not exist on this slide"))
}

fn cell_mut(
    document: &mut Presentation,
    slide: usize,
    id: u32,
    row: usize,
    col: usize,
) -> PyResult<&mut deckcraft_model::Cell> {
    let ShapeKind::Table(table) = &mut shape_mut(document, slide, id)?.kind else {
        return Err(PyValueError::new_err("shape is not a table"));
    };
    let (row, col) = cell_anchor(table, row, col)?;
    table
        .cell_mut(row, col)
        .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))
}

fn paragraph_count(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + 1
}

fn replace_text(body: &mut TextBody, text: &str) {
    let props = deckcraft_model::edit::props_at(body, (0, 0));
    let end = deckcraft_model::edit::end(body);
    deckcraft_model::edit::delete(body, (0, 0), end);
    deckcraft_model::edit::insert(body, (0, 0), text, Some(props));
}

fn geometry(x: f64, y: f64, width: f64, height: f64) -> PyResult<()> {
    dimensions(width, height)?;
    if !x.is_finite() || !y.is_finite() || x.abs() > MAX_SIDE || y.abs() > MAX_SIDE {
        return Err(PyValueError::new_err(
            "shape position must be finite and between -14400 and 14400 points",
        ));
    }
    Ok(())
}

fn dimensions(width: f64, height: f64) -> PyResult<()> {
    if [width, height]
        .iter()
        .any(|value| !value.is_finite() || !(1.0..=MAX_SIDE).contains(value))
    {
        return Err(PyValueError::new_err(
            "dimensions must be finite and between 1 and 14400 points",
        ));
    }
    Ok(())
}

fn font_size(size: f64) -> PyResult<()> {
    if !size.is_finite() || !(1.0..=400.0).contains(&size) {
        return Err(PyValueError::new_err(
            "font size must be finite and between 1 and 400 points",
        ));
    }
    Ok(())
}

fn text_value(text: &str) -> PyResult<()> {
    if text.contains('\r') {
        return Err(PyValueError::new_err(
            "text must use LF newlines; carriage returns are unavailable",
        ));
    }
    if text.chars().any(|character| matches!(character, '\u{0}'..='\u{8}' | '\u{b}'..='\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}')) {
        return Err(PyValueError::new_err("text contains characters unavailable in XML 1.0"));
    }
    Ok(())
}

fn text_body(text: &str, size: f64) -> TextBody {
    let mut body = TextBody::from_text(text);
    body.body.autofit = Some(AutoFit::None);
    body.body.wrap = Some(true);
    for paragraph in &mut body.paragraphs {
        paragraph.def_props.size = Some(size);
        paragraph.end_props.size = Some(size);
        for run in &mut paragraph.runs {
            run.props.size = Some(size);
        }
    }
    body
}

fn validate_png(data: &[u8]) -> PyResult<()> {
    if data.len() > MAX_IMAGE {
        return Err(EngineError::ResourceLimit("PNG input exceeds 16 MiB".into()).into());
    }
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(EngineError::Unsupported(
            "PPTX image authoring currently accepts PNG bytes".into(),
        )
        .into());
    }
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_limits(png::Limits { bytes: MAX_MEDIA });
    let mut reader = decoder
        .read_info()
        .map_err(|error| EngineError::Malformed(format!("invalid PNG: {error}")))?;
    if reader.info().width > 8192 || reader.info().height > 8192 {
        return Err(EngineError::ResourceLimit("PNG exceeds 8192 pixels per side".into()).into());
    }
    if reader.info().animation_control.is_some() {
        return Err(
            EngineError::Unsupported("animated PNG authoring is unavailable".into()).into(),
        );
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= MAX_MEDIA)
        .ok_or_else(|| EngineError::ResourceLimit("decoded PNG exceeds 64 MiB".into()))?;
    let mut pixels = vec![0; size];
    reader
        .next_frame(&mut pixels)
        .map_err(|error| EngineError::Malformed(format!("invalid PNG pixels: {error}")))?;
    reader
        .finish()
        .map_err(|error| EngineError::Malformed(format!("invalid PNG trailer: {error}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture_fixture(padding: usize, color: [u8; 3]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            let mut writer = encoder.write_header().unwrap();
            if padding > 0 {
                writer
                    .write_chunk(png::chunk::ChunkType(*b"caAs"), &vec![0; padding])
                    .unwrap();
            }
            writer.write_image_data(&color).unwrap();
        }
        bytes
    }

    fn picture_quadrants() -> Vec<u8> {
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let rgba = (0..20)
            .flat_map(|y| (0..20).flat_map(move |x| colors[(y / 10) * 2 + x / 10]))
            .collect::<Vec<_>>();
        crate::rendering::encode_png(20, 20, &rgba, 72.0).unwrap()
    }

    fn picture_pixels(png: &[u8]) -> (usize, Vec<u8>) {
        let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        (info.width as usize, pixels)
    }

    #[test]
    fn picture_transforms_use_native_crop_then_flips_and_clockwise_rotation() {
        pyo3::prepare_freethreaded_python();
        let image = picture_quadrants();
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
        ];
        let cases = [
            (0.0, false, false, (0.0, 0.0, 0.0, 0.0), [0, 1, 2, 3]),
            (90.0, false, false, (0.0, 0.0, 0.0, 0.0), [2, 0, 3, 1]),
            (180.0, false, false, (0.0, 0.0, 0.0, 0.0), [3, 2, 1, 0]),
            (270.0, false, false, (0.0, 0.0, 0.0, 0.0), [1, 3, 0, 2]),
            (0.0, true, false, (0.0, 0.0, 0.0, 0.0), [1, 0, 3, 2]),
            (0.0, false, true, (0.0, 0.0, 0.0, 0.0), [2, 3, 0, 1]),
            (90.0, true, false, (0.0, 0.0, 0.0, 0.0), [3, 1, 2, 0]),
            (0.0, false, false, (0.45, 0.45, 0.0, 0.0), [3, 3, 3, 3]),
            (90.0, true, false, (0.45, 0.0, 0.0, 0.0), [3, 1, 3, 1]),
        ];
        for (rotation, h, v, crop, expected) in cases {
            let mut author = EditablePptx::new(160.0, 160.0).unwrap();
            author.add_slide("blank").unwrap();
            let id = author
                .add_image(0, &image, 40.0, 40.0, 80.0, 80.0, "Quadrants")
                .unwrap();
            author.set_picture_crop(0, id, crop).unwrap();
            author.set_picture_flips(0, id, Some(h), Some(v)).unwrap();
            author.set_shape_rotation(0, id, rotation).unwrap();
            assert_eq!(author.picture_data(0, id).unwrap(), image);
            assert_eq!(author.picture_crop(0, id).unwrap(), crop);
            assert_eq!(author.picture_flips(0, id).unwrap(), (h, v));
            let bytes = author.to_bytes().unwrap();
            let reopened = deckcraft_pptx::import(&bytes).unwrap();
            let shape = &reopened.slides[0].shapes[0];
            let ShapeKind::Picture { fill } = &shape.kind else {
                panic!("picture");
            };
            assert_eq!(fill.crop, [crop.0, crop.1, crop.2, crop.3]);
            assert_eq!(
                (shape.xfrm.unwrap().flip_h, shape.xfrm.unwrap().flip_v),
                (h, v)
            );
            for png in [crate::pptx::render(&bytes, 0, 72.0).unwrap(), {
                let pdf = deckcraft_pdf::export(
                    &reopened,
                    &deckcraft_pdf::PdfOptions {
                        dpi: 72.0,
                        ..Default::default()
                    },
                )
                .unwrap();
                crate::pdf::render(&pdf, 0, 72.0, None).unwrap()
            }] {
                let (width, pixels) = picture_pixels(&png);
                for ((x, y), color) in [(60, 60), (100, 60), (60, 100), (100, 100)]
                    .into_iter()
                    .zip(expected)
                {
                    assert_eq!(
                        &pixels[(y * width + x) * 4..][..4],
                        &colors[color],
                        "rotation {rotation}, flip {h}/{v}, crop {crop:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn picture_replacement_preserves_identity_properties_links_and_shared_sources() {
        pyo3::prepare_freethreaded_python();
        let old = picture_fixture(0, [255, 0, 0]);
        let replacement = picture_fixture(0, [0, 255, 0]);
        let mut author = EditablePptx::new(300.0, 300.0).unwrap();
        author.add_slide("blank").unwrap();
        author.set_background_image(0, &old).unwrap();
        let a = author
            .add_image(0, &old, 20.0, 20.0, 50.0, 50.0, "Old alt")
            .unwrap();
        let b = author
            .add_image(0, &old, 90.0, 20.0, 50.0, 50.0, "Sibling")
            .unwrap();
        assert_eq!(author.document().unwrap().media.len(), 1);
        assert_eq!(author.media_bytes, old.len());
        author.set_picture_crop(0, a, (0.1, 0.2, 0.3, 0.4)).unwrap();
        author.set_picture_flips(0, a, Some(true), None).unwrap();
        author.set_shape_rotation(0, a, 90.0).unwrap();
        author
            .set_shape_link(
                0,
                a,
                Some("https://example.com/picture"),
                None,
                "Picture link",
            )
            .unwrap();
        let before = author.shape(0, a).unwrap().clone();
        let order = author.shape_ids(0).unwrap();
        let mut independent = author.picture_data(0, a).unwrap();
        independent[0] = 0;
        assert_eq!(author.picture_data(0, a).unwrap(), old);
        author.set_picture_data(0, a, &replacement).unwrap();
        let after = author.shape(0, a).unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.xfrm, before.xfrm);
        assert_eq!(after.descr, before.descr);
        assert_eq!(after.click, before.click);
        assert_eq!(author.picture_crop(0, a).unwrap(), (0.1, 0.2, 0.3, 0.4));
        assert_eq!(author.shape_ids(0).unwrap(), order);
        assert_eq!(author.picture_data(0, b).unwrap(), old);
        assert_eq!(author.background_image(0).unwrap(), Some(old.clone()));
        assert_eq!(author.document().unwrap().media.len(), 2);
        author.set_picture_alt(0, a, "New café ✓").unwrap();
        assert_eq!(author.picture_alt(0, a).unwrap(), "New café ✓");
        let copy = author.copy().unwrap();
        author.set_picture_data(0, b, &replacement).unwrap();
        assert_eq!(author.document().unwrap().media.len(), 2); // background retains old bytes
        author.background(0, (255, 255, 255)).unwrap();
        assert_eq!(author.document().unwrap().media.len(), 1);
        assert_eq!(author.media_bytes, replacement.len());
        assert_eq!(copy.picture_data(0, b).unwrap(), old);
        author.delete_shape(0, a).unwrap();
        assert_eq!(author.document().unwrap().media.len(), 1);
        author.delete_shape(0, b).unwrap();
        assert_eq!(author.media_bytes, 0);
        assert!(author.picture_data(0, a).is_err());
    }

    #[test]
    fn picture_invalid_updates_preserve_model_counters_and_revision() {
        pyo3::prepare_freethreaded_python();
        let png = picture_fixture(0, [255, 0, 0]);
        let mut author = EditablePptx::new(200.0, 200.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_image(0, &png, 20.0, 20.0, 50.0, 50.0, "Before")
            .unwrap();
        let text = author
            .add_text(0, "Text", 20.0, 90.0, 100.0, 30.0, 18.0)
            .unwrap();
        let before = author.document().unwrap().clone();
        let counters = (
            author.text_bytes,
            author.media_bytes,
            author.shapes,
            author.revision,
        );
        for crop in [
            (-0.1, 0.0, 0.0, 0.0),
            (0.0, 0.46, 0.0, 0.0),
            (0.0, 0.0, f64::NAN, 0.0),
            (0.0, 0.0, 0.0, f64::INFINITY),
        ] {
            assert!(author.set_picture_crop(0, id, crop).is_err());
        }
        assert!(author.set_picture_data(0, id, b"not png").is_err());
        assert!(author
            .set_picture_data(0, id, &png[..png.len() - 12])
            .is_err());
        assert!(author
            .set_picture_data(0, id, &vec![0; MAX_IMAGE + 1])
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert!(author.set_picture_alt(0, id, "bad\u{0}").is_err());
        assert!(author
            .set_picture_alt(0, id, &"x".repeat(MAX_TEXT + 1))
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert!(author
            .set_picture_crop(0, text, (0.0, 0.0, 0.0, 0.0))
            .is_err());
        assert!(author.picture_data(0, text).is_err());
        assert_eq!(author.document().unwrap(), &before);
        assert_eq!(
            (
                author.text_bytes,
                author.media_bytes,
                author.shapes,
                author.revision
            ),
            counters
        );
        author.close();
        assert!(author
            .picture_flips(0, id)
            .unwrap_err()
            .to_string()
            .contains("closed:"));
    }

    #[test]
    fn picture_live_media_budget_allows_dedup_and_reclaims_last_replaced_source() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(200.0, 200.0).unwrap();
        author.add_slide("blank").unwrap();
        let mut ids = Vec::new();
        for red in 0..4 {
            let png = picture_fixture(15 * 1024 * 1024, [red, 0, 0]);
            ids.push(
                author
                    .add_image(0, &png, 20.0, 20.0, 50.0, 50.0, "")
                    .unwrap(),
            );
        }
        let extra = picture_fixture(15 * 1024 * 1024, [4, 0, 0]);
        let before = author.document().unwrap().clone();
        let counters = (author.media_bytes, author.shapes, author.revision);
        assert!(author
            .add_image(0, &extra, 20.0, 20.0, 50.0, 50.0, "")
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(author.document().unwrap(), &before);
        assert_eq!(
            (author.media_bytes, author.shapes, author.revision),
            counters
        );
        let shared = author.picture_data(0, ids[0]).unwrap();
        let duplicate = author
            .add_image(0, &shared, 20.0, 20.0, 50.0, 50.0, "")
            .unwrap();
        assert_eq!(author.media_bytes, counters.0);
        let before = author.document().unwrap().clone();
        let revision = author.revision;
        assert!(author
            .set_picture_data(0, ids[0], &extra)
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(author.document().unwrap(), &before);
        assert_eq!(author.revision, revision);
        author.delete_shape(0, duplicate).unwrap();
        author.set_picture_data(0, ids[0], &extra).unwrap();
        assert_eq!(author.document().unwrap().media.len(), 4);
        assert_eq!(author.media_bytes, counters.0);
        assert_eq!(author.picture_data(0, ids[0]).unwrap(), extra);
    }

    #[test]
    fn authored_layouts_create_fresh_inherited_placeholders_and_roundtrip() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        let masters = author.document().unwrap().masters.clone();
        let mut ids = HashSet::new();
        let cases: [(&str, &[&str]); 4] = [
            ("blank", &[]),
            ("title", &["center_title", "subtitle"]),
            ("title_and_content", &["title", "content"]),
            ("two_content", &["title", "content", "content"]),
        ];
        for (name, roles) in cases {
            let index = author.add_slide(name).unwrap();
            assert!(ids.insert(author.slide_id(index).unwrap()));
            let shapes = author.shape_ids(index).unwrap();
            assert_eq!(shapes.len(), roles.len());
            for ((id, kind), role) in shapes.into_iter().zip(roles) {
                assert!(ids.insert(id));
                assert_eq!(kind, "text");
                assert_eq!(author.shape_placeholder(index, id).unwrap(), Some(*role));
                assert!(author.shape(index, id).unwrap().xfrm.is_none());
                let rect = author.shape_box(index, id).unwrap();
                assert!(rect.2 > 0.0 && rect.3 > 0.0);
                assert_eq!(author.shape_rotation(index, id).unwrap(), 0.0);
                author
                    .set_shape_text(index, id, &format!("Label {index} {id}"))
                    .unwrap();
                assert!(author.shape(index, id).unwrap().xfrm.is_none());
            }
        }
        assert_eq!(author.shapes, 7);
        assert_eq!(author.document().unwrap().masters, masters);
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        assert!(reopened.validate().is_empty());
        for slide in &reopened.slides {
            assert!(reopened.layout(slide.layout).is_some());
            for shape in &slide.shapes {
                assert!(shape.ph.is_some());
                assert!(shape.xfrm.is_none());
                assert!(shape.text.as_ref().unwrap().text().starts_with("Label "));
            }
        }
        let plain = author
            .add_text(0, "Ordinary", 10.0, 10.0, 200.0, 40.0, 18.0)
            .unwrap();
        assert_eq!(author.shape_placeholder(0, plain).unwrap(), None);
    }

    #[test]
    fn authored_layouts_render_actual_title_and_column_text_and_export_pdf() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        let mut labels = Vec::new();
        for layout in ["title", "title_and_content", "two_content"] {
            let slide = author.add_slide(layout).unwrap();
            let mut expected = Vec::new();
            for (order, (id, _)) in author.shape_ids(slide).unwrap().into_iter().enumerate() {
                let text = format!("Label {} {}", slide + 1, order + 1);
                author.set_shape_text(slide, id, &text).unwrap();
                expected.push((text, author.shape_box(slide, id).unwrap()));
            }
            labels.push(expected);
        }
        let bytes = author.to_bytes().unwrap();
        for (slide, expected) in labels.iter().enumerate() {
            let png = crate::pptx::render(&bytes, slide, 72.0).unwrap();
            let mut reader = png::Decoder::new(Cursor::new(png)).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), (720, 540));
            for (_, (x, y, w, h)) in expected {
                let count = (*y as usize..(y + h).ceil() as usize)
                    .flat_map(|py| (*x as usize..(x + w).ceil() as usize).map(move |px| (px, py)))
                    .filter(|&(px, py)| pixels[(py * 720 + px) * 4..][..3].iter().any(|c| *c < 180))
                    .count();
                assert!(count > 20, "slide {slide} has no visible placeholder ink");
            }
        }
        let options = deckcraft_pdf::PdfOptions {
            dpi: 72.0,
            ..Default::default()
        };
        let pdf = deckcraft_pdf::export(author.document().unwrap(), &options).unwrap();
        let extracted = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted.pages.len(), 3);
        for (page, expected) in extracted.pages.iter().zip(&labels) {
            let actual = page
                .blocks
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            for (text, _) in expected {
                assert_eq!(actual.matches(text).count(), 1, "{actual}");
            }
        }
    }

    #[test]
    fn placeholder_geometry_changes_resolve_native_inheritance_only_for_edited_shape() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("title_and_content").unwrap();
        let shapes = author.shape_ids(0).unwrap();
        let title = shapes[0].0;
        let content = shapes[1].0;
        // Give the native parent a transform not produced by the default fixtures.
        let layout = author.document().unwrap().slides[0].layout;
        let native = author.presentation.as_mut().unwrap();
        let master = Arc::make_mut(&mut native.masters[0]);
        let definition = master.layouts.iter_mut().find(|l| l.id == layout).unwrap();
        for shape in &mut definition.shapes {
            if let Some(transform) = &mut shape.xfrm {
                transform.rot = 17.0;
                transform.flip_h = true;
            }
        }
        let masters = author.document().unwrap().masters.clone();
        let original = author.shape_transform(0, title).unwrap();
        let placeholder = author.shape(0, title).unwrap().ph.clone();
        assert_eq!(author.shape_rotation(0, title).unwrap(), 17.0);
        author.set_shape_rotation(0, title, 35.0).unwrap();
        assert_eq!(
            author.shape_box(0, title).unwrap(),
            (original.x, original.y, original.w, original.h)
        );
        assert!(author.shape(0, title).unwrap().xfrm.unwrap().flip_h);
        assert!(author.shape(0, content).unwrap().xfrm.is_none());
        author
            .set_shape_box(0, content, 20.0, 130.0, 300.0, 250.0)
            .unwrap();
        assert_eq!(author.shape_rotation(0, content).unwrap(), 17.0);
        assert!(author.shape(0, content).unwrap().xfrm.unwrap().flip_h);
        assert_eq!(author.document().unwrap().masters, masters);
        assert_eq!(author.shape(0, title).unwrap().ph, placeholder);
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        assert!(reopened.slides[0].shapes.iter().all(|s| s.ph.is_some()));
        assert_eq!(reopened.slides[0].shapes[0].xfrm.unwrap().rot, 35.0);
        assert_eq!(reopened.slides[0].shapes[1].xfrm.unwrap().rot, 17.0);
    }

    #[test]
    fn authored_layout_validation_id_and_shape_budgets_are_atomic() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let before = author.document().unwrap().clone();
        let revision = author.revision;
        for name in [
            "unknown",
            "Title",
            "",
            "title_and_vertical_text",
            "vertical_title_and_text",
            "picture_with_caption",
            "custom",
        ] {
            assert!(author.add_slide(name).is_err());
            assert_eq!(author.document().unwrap(), &before);
            assert_eq!(author.revision, revision);
        }
        author.shapes = MAX_SHAPES - 1;
        assert!(author
            .add_slide("title")
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(author.document().unwrap(), &before);
        author.shapes = 0;
        author.presentation.as_mut().unwrap().next_id = u32::MAX - 1;
        let before = author.document().unwrap().clone();
        assert!(author
            .add_slide("two_content")
            .unwrap_err()
            .to_string()
            .contains("resource_limit:"));
        assert_eq!(author.document().unwrap(), &before);
        assert_eq!(author.revision, revision);
        author.close();
        assert!(author
            .add_slide("title")
            .unwrap_err()
            .to_string()
            .contains("closed:"));
    }

    #[test]
    fn saves_native_slides_text_tables_and_background() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        assert_eq!(author.add_slide("blank").unwrap(), 0);
        author
            .add_text(0, "Hello λ\nSecond line", 20.0, 20.0, 400.0, 80.0, 18.0)
            .unwrap();
        author
            .add_table(
                0,
                vec![
                    vec!["Name".into(), "Value".into()],
                    vec!["A".into(), "9".into()],
                ],
                20.0,
                120.0,
                400.0,
                100.0,
                16.0,
            )
            .unwrap();
        author.background(0, (220, 230, 240)).unwrap();
        let bytes = author.to_bytes().unwrap();
        let reopened = deckcraft_pptx::import(&bytes).unwrap();
        assert!(reopened.validate().is_empty());
        assert_eq!(reopened.slides.len(), 1);
        assert_eq!(reopened.slide_size, Size::new(720.0, 540.0));
        assert_eq!(
            reopened.slides[0].shapes[0].text.as_ref().unwrap().text(),
            "Hello λ\nSecond line"
        );
        assert!(matches!(
            reopened.slides[0].shapes[1].kind,
            ShapeKind::Table(_)
        ));
        assert!(reopened.slides[0].background.is_some());
    }

    #[test]
    fn copies_are_independent_and_rejected_changes_are_atomic() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let mut copied = author.copy().unwrap();
        copied.add_slide("blank").unwrap();
        assert_eq!(author.slide_count().unwrap(), 1);
        let before = author.document().unwrap().clone();
        assert!(author
            .add_text(0, "bad", f64::NAN, 0.0, 100.0, 50.0, 18.0)
            .is_err());
        assert!(author
            .add_table(
                0,
                vec![vec!["A".into()], vec![]],
                0.0,
                0.0,
                100.0,
                50.0,
                18.0
            )
            .is_err());
        assert_eq!(author.document().unwrap(), &before);
        author.close();
        assert!(author.slide_count().is_err());
        assert_eq!(copied.slide_count().unwrap(), 2);
    }

    #[test]
    fn validates_png_before_adding_owned_media() {
        pyo3::prepare_freethreaded_python();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[255, 0, 0]).unwrap();
        }
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        author
            .add_image(0, &png, 20.0, 20.0, 100.0, 100.0, "Red pixel")
            .unwrap();
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        assert!(matches!(
            reopened.slides[0].shapes[0].kind,
            ShapeKind::Picture { .. }
        ));
        assert!(reopened
            .media
            .iter()
            .any(|media| media.data.as_slice() == png));
        let before = author.document().unwrap().clone();
        assert!(author
            .add_image(
                0,
                &png[..png.len() - 12],
                20.0,
                20.0,
                100.0,
                100.0,
                "Truncated"
            )
            .is_err());
        assert_eq!(author.document().unwrap(), &before);

        let mut exhausted = EditablePptx::new(720.0, 540.0).unwrap();
        exhausted.add_slide("blank").unwrap();
        exhausted.presentation.as_mut().unwrap().next_id = u32::MAX - 1;
        let before = exhausted.document().unwrap().clone();
        assert!(exhausted
            .add_image(0, &png, 20.0, 20.0, 100.0, 100.0, "No ID left")
            .is_err());
        assert_eq!(exhausted.document().unwrap(), &before);
        assert_eq!(exhausted.media_bytes, 0);
    }

    #[test]
    fn updates_typed_objects_without_losing_format_or_geometry() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let text_id = author
            .add_text(0, "Old", 10.0, 20.0, 300.0, 100.0, 36.0)
            .unwrap();
        author.set_text_color(0, text_id, (255, 255, 255)).unwrap();
        author
            .set_shape_text(0, text_id, "New λ\nSecond line")
            .unwrap();
        assert_eq!(author.shape_text(0, text_id).unwrap(), "New λ\nSecond line");
        assert_eq!(
            author
                .shape(0, text_id)
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .paragraphs[0]
                .runs[0]
                .props
                .size,
            Some(36.0)
        );
        let table_id = author
            .add_table(
                0,
                vec![vec!["A".into(), "B".into()]],
                10.0,
                140.0,
                400.0,
                80.0,
                18.0,
            )
            .unwrap();
        author.set_cell_text(0, table_id, 0, 1, "Changed").unwrap();
        author
            .set_shape_box(0, table_id, 20.0, 150.0, 500.0, 100.0)
            .unwrap();
        assert_eq!(
            author.table_rows(0, table_id).unwrap(),
            vec![vec!["A", "Changed"]]
        );
        assert_eq!(
            author.shape_box(0, table_id).unwrap(),
            (20.0, 150.0, 500.0, 100.0)
        );
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        let ShapeKind::Table(table) = &reopened.slides[0].shapes[1].kind else {
            panic!("table missing")
        };
        assert_eq!(table.width(), 500.0);
        assert_eq!(table.height(), 100.0);
        assert_eq!(table.cell(0, 1).unwrap().text.text(), "Changed");
    }

    #[test]
    fn formatting_reports_mixed_values_without_averaging() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "First\nSecond", 0.0, 0.0, 400.0, 100.0, 24.0)
            .unwrap();
        author
            .set_text_format(
                0,
                id,
                Some(30.0),
                Some((10, 20, 30)),
                Some(true),
                Some(false),
                Some("Arial"),
            )
            .unwrap();
        let expected = (
            Some(30.0),
            Some((10, 20, 30)),
            Some(true),
            Some(false),
            Some("Arial".to_owned()),
        );
        assert_eq!(author.text_format(0, id).unwrap(), expected);
        shape_mut(author.presentation.as_mut().unwrap(), 0, id)
            .unwrap()
            .text
            .as_mut()
            .unwrap()
            .paragraphs[1]
            .runs[0]
            .props
            .size = Some(48.0);
        let mixed = author.text_format(0, id).unwrap();
        assert_eq!(mixed.0, None);
        assert_eq!(mixed.1, Some((10, 20, 30)));
        let before = author.document().unwrap().clone();
        assert!(author
            .set_text_format(0, id, Some(f64::NAN), None, None, None, None)
            .is_err());
        assert_eq!(author.document().unwrap(), &before);
    }

    #[test]
    fn slide_organization_preserves_identity_and_rejects_references() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        author.add_slide("blank").unwrap();
        let first = author.slide_id(0).unwrap();
        let second = author.slide_id(1).unwrap();
        author.move_slide(0, 1).unwrap();
        assert_eq!(author.slide_index(first).unwrap(), 1);
        assert_eq!(author.slide_id(0).unwrap(), second);
        author.remove_slide(0).unwrap();
        assert!(author.slide_index(second).is_err());
        assert_eq!(author.slide_index(first).unwrap(), 0);
        author.presentation.as_mut().unwrap().show.range = Some((1, 1));
        let before = author.document().unwrap().clone();
        assert!(author.remove_slide(0).is_err());
        assert_eq!(author.document().unwrap(), &before);
    }

    #[test]
    fn slide_metadata_notes_and_cell_style_survive_native_round_trip() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        author.set_slide_name(0, "Quarterly λ").unwrap();
        author.set_slide_hidden(0, true).unwrap();
        author
            .set_slide_notes(0, "First note\nSecond note")
            .unwrap();
        let id = author
            .add_table(0, vec![vec!["Value".into()]], 20.0, 20.0, 300.0, 80.0, 18.0)
            .unwrap();
        author
            .set_cell_background(0, id, 0, 0, (240, 220, 10))
            .unwrap();
        author
            .set_cell_vertical_alignment(0, id, 0, 0, "center")
            .unwrap();
        assert_eq!(
            author.cell_background(0, id, 0, 0).unwrap(),
            Some((240, 220, 10))
        );
        assert_eq!(
            author.cell_vertical_alignment(0, id, 0, 0).unwrap(),
            Some("center")
        );
        let bytes = author.to_bytes().unwrap();
        let reopened = deckcraft_pptx::import(&bytes).unwrap();
        assert_eq!(reopened.slides[0].name, "Quarterly λ");
        assert!(reopened.slides[0].hidden);
        assert_eq!(reopened.slides[0].notes.text(), "First note\nSecond note");
        let ShapeKind::Table(table) = &reopened.slides[0].shapes[0].kind else {
            panic!("table missing")
        };
        assert_eq!(table.rows[0].cells[0].anchor, Some(Anchor::Middle));
        let extracted = crate::pptx::extract(&bytes).unwrap();
        let notes: Vec<_> = extracted.pages[0]
            .blocks
            .iter()
            .filter(|block| block.kind == "notes")
            .collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, "First note\nSecond note");
    }

    #[test]
    fn rejects_excessive_paragraphs_before_building_text_model() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let before = author.document().unwrap().clone();
        assert!(author
            .add_text(
                0,
                &"\n".repeat(MAX_PARAGRAPHS),
                0.0,
                0.0,
                100.0,
                100.0,
                18.0
            )
            .is_err());
        assert_eq!(author.document().unwrap(), &before);
    }

    #[test]
    fn drawings_keep_geometry_style_and_identity_through_stack_changes() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let rectangle = author
            .add_shape(
                0,
                "rectangle",
                10.0,
                20.0,
                100.0,
                80.0,
                Some((20, 30, 40)),
                Some((240, 230, 220)),
                2.5,
            )
            .unwrap();
        let ellipse = author
            .add_shape(
                0,
                "ellipse",
                150.0,
                20.0,
                100.0,
                80.0,
                Some((80, 90, 100)),
                None,
                1.0,
            )
            .unwrap();
        let line = author
            .add_shape(
                0,
                "line",
                0.0,
                200.0,
                400.0,
                0.0,
                None,
                Some((10, 20, 30)),
                3.0,
            )
            .unwrap();
        author.set_shape_rotation(0, rectangle, 45.0).unwrap();
        author
            .set_shape_box(0, rectangle, 30.0, 40.0, 120.0, 90.0)
            .unwrap();
        assert_eq!(author.shape_rotation(0, rectangle).unwrap(), 45.0);
        assert_eq!(author.shape_fill(0, rectangle).unwrap(), Some((20, 30, 40)));
        assert_eq!(
            author.shape_stroke(0, rectangle).unwrap(),
            Some(((240, 230, 220), 2.5))
        );
        author.move_shape(0, line, 0).unwrap();
        assert_eq!(
            author.shape_ids(0).unwrap(),
            vec![(line, "shape"), (rectangle, "shape"), (ellipse, "shape")]
        );
        author.delete_shape(0, ellipse).unwrap();
        assert!(author.shape_box(0, ellipse).is_err());
        let before = author.document().unwrap().clone();
        assert!(author.set_shape_rotation(0, rectangle, f64::NAN).is_err());
        assert!(author.move_shape(0, rectangle, 2).is_err());
        assert_eq!(author.document().unwrap(), &before);
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        let shapes = &reopened.slides[0].shapes;
        assert_eq!(shapes.len(), 2);
        assert_eq!(shapes[0].geom, Geom::preset("line"));
        assert_eq!(shapes[0].xfrm.as_ref().unwrap().h, 0.0);
        assert_eq!(shapes[1].geom, Geom::preset("rect"));
        assert_eq!(shapes[1].xfrm.as_ref().unwrap().rot, 45.0);
    }

    #[test]
    fn image_backgrounds_retain_shared_media_until_last_reference_is_removed() {
        pyo3::prepare_freethreaded_python();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[12, 34, 56])
                .unwrap();
        }
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        author.set_background_image(0, &png).unwrap();
        let image = author
            .add_image(0, &png, 10.0, 20.0, 100.0, 100.0, "Pixel")
            .unwrap();
        assert_eq!(author.background_image(0).unwrap(), Some(png.clone()));
        assert!(author.slide_background(0).is_err());
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        let Some(Background::Fill {
            fill: Fill::Picture(fill),
        }) = &reopened.slides[0].background
        else {
            panic!("background picture missing")
        };
        assert_eq!(reopened.media(fill.media).unwrap().data.as_slice(), png);
        author.background(0, (255, 255, 255)).unwrap();
        assert_eq!(author.background_image(0).unwrap(), None);
        assert!(!author.document().unwrap().media.is_empty());
        author.delete_shape(0, image).unwrap();
        assert!(author.document().unwrap().media.is_empty());
        assert_eq!(author.media_bytes, 0);
        assert_eq!(author.text_bytes, 0);
    }

    #[test]
    fn object_replacement_character_is_literal_pptx_text() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "Before", 0.0, 0.0, 300.0, 80.0, 18.0)
            .unwrap();
        author
            .set_shape_text(0, id, "Literal \u{fffc} text")
            .unwrap();
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        assert_eq!(
            reopened.slides[0].shapes[0].text.as_ref().unwrap().text(),
            "Literal \u{fffc} text"
        );
        for invalid in ["\0", "\u{fffe}", "\u{ffff}"] {
            assert!(author.set_shape_text(0, id, invalid).is_err());
        }
        assert_eq!(author.shape_text(0, id).unwrap(), "Literal \u{fffc} text");
    }

    #[test]
    fn rejects_carriage_returns_without_mutating_presentation() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let text_id = author
            .add_text(0, "a\nb", 0.0, 0.0, 100.0, 80.0, 18.0)
            .unwrap();
        let table_id = author
            .add_table(0, vec![vec!["cell".into()]], 0.0, 100.0, 100.0, 80.0, 18.0)
            .unwrap();
        let before = author.document().unwrap().clone();
        for invalid in ["a\rb", "a\r\nb"] {
            assert!(author
                .add_text(0, invalid, 0.0, 0.0, 100.0, 80.0, 18.0)
                .is_err());
            assert!(author.set_shape_text(0, text_id, invalid).is_err());
            assert!(author
                .add_table(0, vec![vec![invalid.into()]], 0.0, 0.0, 100.0, 80.0, 18.0)
                .is_err());
            assert!(author.set_cell_text(0, table_id, 0, 0, invalid).is_err());
            assert!(author.set_slide_notes(0, invalid).is_err());
            assert!(author.set_slide_name(0, invalid).is_err());
            assert_eq!(author.document().unwrap(), &before);
        }
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        assert_eq!(
            reopened.slides[0].shapes[0].text.as_ref().unwrap().text(),
            "a\nb"
        );
    }

    #[test]
    fn rich_ranges_preserve_unicode_paragraphs_and_roundtrip_formatting() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "Aλ", 20.0, 20.0, 500.0, 200.0, 18.0)
            .unwrap();
        let (_, _, revision) = author.add_run_patch(0, id, "日本\nSecond", r##"{"bold":true,"italic":true,"font_size":24,"color":"#224466","underline":true,"strike":"double","script":"superscript","highlight":"#FFFF00"}"##).unwrap();
        assert_eq!(author.text_range(0, id, 1, 7).unwrap().0, "λ日本\nSe");
        let (start, end, current) = author
            .replace_range(0, id, 1, 7, revision, "é🙂\nX")
            .unwrap();
        assert_eq!((start, end), (1, 5));
        assert_eq!(author.shape_text(0, id).unwrap(), "Aé🙂\nXcond");
        assert!(author.range_text(0, id, 1, 7, revision).is_err());
        let current = author.format_range_patch(0, id, 5, 9, current, r#"{"bold":false,"italic":null,"font_size":null,"underline":false,"strike":false,"script":"baseline","highlight":null}"#).unwrap();
        assert_eq!(author.range_text(0, id, 5, 9, current).unwrap(), "cond");
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        let body = reopened.slides[0].shapes[0].text.as_ref().unwrap();
        assert_eq!(body.text(), "Aé🙂\nXcond");
        let props = &body.paragraphs[1].runs.last().unwrap().props;
        assert_eq!(props.bold, Some(false));
        assert_eq!(props.italic, None);
        assert_eq!(props.size, None);
        assert_eq!(props.underline.as_deref(), Some("none"));
        assert_eq!(props.strike, Some(Strike::None));
        assert_eq!(props.baseline, Some(0.0));
        assert_eq!(props.highlight, None);
        assert!(matches!(props.fill, Some(Fill::Solid { .. })));
    }

    #[test]
    fn replacement_at_run_boundary_uses_selected_character_format() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "Prefix ", 20.0, 20.0, 500.0, 100.0, 18.0)
            .unwrap();
        let (_, _, revision) = author
            .add_run_patch(0, id, "{{name}}", r#"{"bold":true,"font_size":24}"#)
            .unwrap();
        author
            .replace_range(0, id, 7, 15, revision, "Acme")
            .unwrap();
        let body = author.text_body(0, id).unwrap();
        assert_eq!(body.text(), "Prefix Acme");
        assert_eq!(body.paragraphs[0].runs[0].props.bold, None);
        assert_eq!(
            body.paragraphs[0].runs.last().unwrap().props.bold,
            Some(true)
        );
        assert_eq!(
            body.paragraphs[0].runs.last().unwrap().props.size,
            Some(24.0)
        );
    }

    #[test]
    fn readable_underline_names_roundtrip_through_native_writer() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "Styled", 20.0, 20.0, 500.0, 100.0, 18.0)
            .unwrap();
        for (name, native) in UNDERLINES {
            let patch = serde_json::json!({"underline": name}).to_string();
            author
                .format_range_patch(0, id, 0, 6, author.revision().unwrap(), &patch)
                .unwrap();
            let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
            let props = &reopened.slides[0].shapes[0]
                .text
                .as_ref()
                .unwrap()
                .paragraphs[0]
                .runs[0]
                .props;
            assert_eq!(props.underline.as_deref(), Some(*native), "{name}");
        }
        let revision = author.revision().unwrap();
        let before = author.document().unwrap().clone();
        for native in [
            "sng", "dbl", "heavy", "dotDash", "wavy", "wavyDbl", "invented",
        ] {
            let patch = serde_json::json!({"underline": native}).to_string();
            assert!(author
                .format_range_patch(0, id, 0, 6, revision, &patch)
                .is_err());
        }
        assert_eq!(author.document().unwrap(), &before);
        assert_eq!(author.revision().unwrap(), revision);
    }

    #[test]
    fn links_cross_runs_preserve_labels_and_export_native_navigation() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        author.add_slide("blank").unwrap();
        let first = author.slide_id(0).unwrap();
        let target = author.slide_id(1).unwrap();
        let id = author
            .add_text(0, "Before λ", 20.0, 20.0, 500.0, 100.0, 18.0)
            .unwrap();
        let (_, _, revision) = author
            .add_run_patch(0, id, "日本 after", r#"{"bold":true}"#)
            .unwrap();
        let url = "https://example.com/deck?a=1&b=2";
        let revision = author
            .set_range_link(0, id, 7, 10, revision, Some(url), None, "Website")
            .unwrap();
        assert_eq!(
            author.range_link(0, id, 7, 10, revision, false).unwrap(),
            Some((Some(url.into()), None, "Website".into()))
        );
        assert!(author.range_link(0, id, 6, 10, revision, false).is_err());
        author
            .set_shape_link(0, id, None, Some(target), "Last page")
            .unwrap();
        author.move_slide(1, 0).unwrap();
        assert_eq!(author.slide_id(1).unwrap(), first);
        let snapshot = author.document().unwrap().clone();
        let revision = author.revision().unwrap();
        assert!(author.remove_slide(0).is_err());
        assert_eq!(author.document().unwrap(), &snapshot);
        assert_eq!(author.revision().unwrap(), revision);
        let bytes = author.to_bytes().unwrap();
        let reopened = deckcraft_pptx::import(&bytes).unwrap();
        let source = &reopened.slides[1].shapes[0];
        assert!(
            matches!(source.click.as_ref().unwrap().action, Action::Slide { slide } if slide == reopened.slides[0].id)
        );
        assert_eq!(source.click.as_ref().unwrap().tooltip, "Last page");
        assert_eq!(source.text.as_ref().unwrap().text(), "Before λ日本 after");
        let extracted = crate::pptx::extract(&bytes).unwrap();
        assert_eq!(
            extracted.pages[1]
                .links
                .iter()
                .filter(|l| l.uri == url)
                .map(|l| l.text.as_str())
                .collect::<String>(),
            "λ日本"
        );
        let pdf = crate::pptx::to_pdf(&bytes).unwrap();
        let extracted_pdf = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(extracted_pdf.pages.len(), 2);
        assert!(extracted_pdf.pages[1].links.iter().any(|l| l.uri == url));
        assert!(extracted_pdf.pages[1]
            .links
            .iter()
            .any(|l| l.uri == "#page=1"));
        author.set_shape_link(1, id, None, None, "").unwrap();
        author.remove_slide(0).unwrap();
        assert_eq!(author.shape_text(0, id).unwrap(), "Before λ日本 after");
    }

    #[test]
    fn rejects_invalid_ranges_patches_and_links_atomically() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "text\nnext", 20.0, 20.0, 500.0, 100.0, 18.0)
            .unwrap();
        let revision = author.revision().unwrap();
        let before = author.document().unwrap().clone();
        for patch in [
            r#"{"unknown":1}"#,
            r#"{"bold":1}"#,
            r#"{"font_size":true}"#,
            r#"{"font_size":0}"#,
            r#"{"font":[]}"#,
            r#"{"underline":"not-style"}"#,
            r#"{"background":"FFFFFF"}"#,
            r#"{"color":{"r":1}}"#,
            "[]",
        ] {
            assert!(author
                .format_range_patch(0, id, 0, 2, revision, patch)
                .is_err());
        }
        for url in [
            "file:///tmp/a",
            "javascript:alert(1)",
            "https://",
            "mailto:",
            "https://x/a b",
            "https://x/\n",
            "https://x/\u{fffe}",
            "https://x/\u{ffff}",
        ] {
            assert!(author
                .set_range_link(0, id, 0, 2, revision, Some(url), None, "")
                .is_err());
        }
        assert!(author
            .format_range_patch(0, id, 0, 100, revision, "{}")
            .is_err());
        assert!(author.replace_range(0, id, 3, 2, revision, "bad").is_err());
        assert!(author.add_run_patch(0, id, "\r", "{}").is_err());
        assert!(author
            .set_range_link(0, id, 4, 5, revision, Some("https://example.com"), None, "")
            .is_err());
        assert!(author
            .set_shape_link(0, id, None, Some(u32::MAX), "")
            .is_err());
        assert!(author
            .set_shape_link(
                0,
                id,
                Some("https://example.com"),
                Some(author.slide_id(0).unwrap()),
                ""
            )
            .is_err());
        assert!(author
            .set_shape_link(0, id, None, None, "orphan tooltip")
            .is_err());
        assert_eq!(author.document().unwrap(), &before);
        assert_eq!(author.revision().unwrap(), revision);
        let current = author
            .set_range_link(0, id, 0, 4, revision, Some("https://example.com"), None, "")
            .unwrap();
        assert!(author.range_text(0, id, 0, 4, revision).is_err());
        let current = author
            .set_range_link(0, id, 0, 4, current, None, None, "")
            .unwrap();
        assert_eq!(
            author.range_link(0, id, 0, 4, current, false).unwrap(),
            None
        );
        let current = author
            .set_range_link(0, id, 5, 9, current, Some("https://example.com"), None, "")
            .unwrap();
        let (_, _, latest) = author.add_run_patch(0, id, " extra", "{}").unwrap();
        assert!(latest > current);
        assert_eq!(
            author.range_link(0, id, 9, 15, latest, false).unwrap(),
            None
        );
    }

    #[test]
    fn mixed_link_assignment_lookup_retains_range_and_revision_validation() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "AB", 20.0, 20.0, 500.0, 100.0, 18.0)
            .unwrap();
        let revision = author.revision().unwrap();
        let revision = author
            .set_range_link(0, id, 0, 1, revision, Some("https://a.example"), None, "A")
            .unwrap();
        let revision = author
            .set_range_link(0, id, 1, 2, revision, Some("https://b.example"), None, "B")
            .unwrap();
        assert!(author.range_link(0, id, 0, 2, revision, false).is_err());
        assert_eq!(
            author.range_link(0, id, 0, 2, revision, true).unwrap(),
            None
        );
        assert!(author.range_link(0, id, 0, 3, revision, true).is_err());
        let current = author
            .set_range_link(0, id, 0, 2, revision, Some("https://all.example"), None, "")
            .unwrap();
        assert!(author.range_link(0, id, 0, 2, revision, true).is_err());
        assert_eq!(
            author.range_link(0, id, 0, 2, current, false).unwrap(),
            Some((Some("https://all.example".into()), None, "".into()))
        );
        let current = author
            .set_range_link(0, id, 0, 1, current, None, None, "")
            .unwrap();
        assert_eq!(author.range_link(0, id, 0, 2, current, true).unwrap(), None);
        let current = author
            .set_range_link(0, id, 0, 2, current, None, None, "")
            .unwrap();
        assert_eq!(
            author.range_link(0, id, 0, 2, current, false).unwrap(),
            None
        );
    }

    #[test]
    fn deletion_detects_table_and_nested_group_references() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        author.add_slide("blank").unwrap();
        let target = author.slide_id(1).unwrap();
        let id = author
            .add_table(
                0,
                vec![vec!["Linked cell".into()]],
                20.0,
                20.0,
                400.0,
                100.0,
                18.0,
            )
            .unwrap();
        let link = make_link(
            author.document().unwrap(),
            None,
            Some(target),
            "Cell target",
        )
        .unwrap();
        let document = author.presentation.as_mut().unwrap();
        cell_mut(document, 0, id, 0, 0).unwrap().text.paragraphs[0].runs[0]
            .props
            .link = link;
        let before = author.document().unwrap().clone();
        assert!(author.remove_slide(1).is_err());
        assert_eq!(author.document().unwrap(), &before);
        let document = author.presentation.as_mut().unwrap();
        let slide = Arc::make_mut(&mut document.slides[0]);
        let child = slide.shapes.remove(0);
        slide.shapes.push(Shape {
            id: ShapeId(id + 1),
            kind: ShapeKind::Group {
                children: vec![child],
                child: Xfrm::new(0.0, 0.0, 400.0, 100.0),
            },
            ..Default::default()
        });
        assert!(reject_referenced_slide(author.document().unwrap(), SlideId(target)).is_err());
        assert!(reject_referenced_slide(
            author.document().unwrap(),
            SlideId(author.slide_id(0).unwrap())
        )
        .is_ok());
    }

    #[test]
    fn rich_text_preflights_fragmentation_and_cloned_link_budgets() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_text(0, "text", 20.0, 20.0, 500.0, 100.0, 18.0)
            .unwrap();
        let paragraph = &mut shape_mut(author.presentation.as_mut().unwrap(), 0, id)
            .unwrap()
            .text
            .as_mut()
            .unwrap()
            .paragraphs[0];
        paragraph.runs = (0..3000)
            .map(|i| {
                deckcraft_model::text::Run::with(
                    "x",
                    RunProps {
                        bold: Some(i % 2 == 0),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let revision = author.revision().unwrap();
        let before = author.document().unwrap().clone();
        let uri = format!("https://example.com/{}", "x".repeat(8000));
        assert!(author
            .set_range_link(0, id, 0, 3000, revision, Some(&uri), None, "")
            .is_err());
        assert_eq!(author.document().unwrap(), &before);
        let paragraph = &mut shape_mut(author.presentation.as_mut().unwrap(), 0, id)
            .unwrap()
            .text
            .as_mut()
            .unwrap()
            .paragraphs[0];
        paragraph.runs = (0..MAX_RUNS)
            .map(|i| {
                deckcraft_model::text::Run::with(
                    "abc",
                    RunProps {
                        bold: Some(i % 2 == 0),
                        ..Default::default()
                    },
                )
            })
            .collect();
        assert!(author
            .format_range_patch(0, id, 1, 2, revision, r#"{"italic":true}"#)
            .is_err());
        assert_eq!(author.revision().unwrap(), revision);
    }

    #[test]
    fn table_layout_roundtrips_and_native_rows_expand_to_fit_content() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let long = "Line one\nLine two\nLine three";
        let id = author
            .add_table(
                0,
                vec![
                    vec!["Heading".into(), "Value".into()],
                    vec![long.into(), "Second".into()],
                ],
                20.0,
                30.0,
                400.0,
                100.0,
                18.0,
            )
            .unwrap();
        let generation = author.table_generation(0, id).unwrap();
        assert!(author.table_borders(0, id).is_err());
        author.set_column_widths(0, id, vec![150.0, 250.0]).unwrap();
        author
            .set_row_height(0, id, 1, Some(8.0), "at_least")
            .unwrap();
        author
            .set_cell_margins(0, id, 1, 0, Some((2.0, 3.0, 4.0, 5.0)))
            .unwrap();
        author
            .set_cell_alignment(0, id, 1, 0, Some("center"))
            .unwrap();
        author
            .set_cell_vertical_alignment(0, id, 1, 0, "center")
            .unwrap();
        author
            .set_cell_background(0, id, 1, 0, (220, 230, 240))
            .unwrap();
        author.set_table_borders(0, id, "#123456", 0.5).unwrap();
        assert_eq!(
            author.table_borders(0, id).unwrap(),
            Some(("#123456".into(), 0.5))
        );
        assert_eq!(
            author.cell_margins(0, id, 1, 0).unwrap(),
            (2.0, 3.0, 4.0, 5.0)
        );
        assert_eq!(
            author.row_height(0, id, 1).unwrap(),
            (Some(8.0), "at_least")
        );
        assert_eq!(author.shape_box(0, id).unwrap(), (20.0, 30.0, 400.0, 58.0));
        assert_eq!(author.table_generation(0, id).unwrap(), generation);
        let bytes = author.to_bytes().unwrap();
        let reopened = deckcraft_pptx::import(&bytes).unwrap();
        let ShapeKind::Table(table) = &reopened.slides[0].shapes[0].kind else {
            panic!("missing table")
        };
        assert_eq!(table.cols, vec![150.0, 250.0]);
        assert_eq!(table.rows[1].height, 8.0);
        let cell = &table.rows[1].cells[0];
        assert_eq!(cell.margins, Some([5.0, 3.0, 2.0, 4.0]));
        assert!(cell
            .text
            .paragraphs
            .iter()
            .all(|p| p.props.align == Some(Align::Center)));
        assert_eq!(cell.anchor, Some(Anchor::Middle));
        assert!(cell.borders.iter().all(Option::is_some));
        let placed = deckcraft_render::place_slide(&reopened, 0);
        let long_cell = placed
            .texts
            .iter()
            .find(|text| text.body.text() == long)
            .unwrap();
        assert!(long_cell.layout.inner.height() > 8.0);
        let png = crate::pptx::render(&bytes, 0, 72.0).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let pdf = crate::pptx::to_pdf(&bytes).unwrap();
        let pdf = crate::pdf::extract(&pdf, None).unwrap();
        let text = pdf.pages[0]
            .blocks
            .iter()
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Heading") && text.contains("Line one") && text.contains("Second"));
        author.clear_table_borders(0, id).unwrap();
        assert_eq!(author.table_borders(0, id).unwrap(), None);
        author.set_cell_margins(0, id, 1, 0, None).unwrap();
        assert_eq!(
            author.cell_margins(0, id, 1, 0).unwrap(),
            (3.6, 7.2, 3.6, 7.2)
        );
        author.set_cell_alignment(0, id, 1, 0, None).unwrap();
        assert_eq!(author.cell_alignment(0, id, 1, 0).unwrap(), None);
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        let ShapeKind::Table(table) = &reopened.slides[0].shapes[0].kind else {
            panic!("missing table")
        };
        assert!(table
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .flat_map(|c| &c.borders)
            .all(|b| matches!(b.as_ref().and_then(|b| b.fill.as_ref()), Some(Fill::None))));
    }

    #[test]
    fn table_merges_alias_covered_cells_and_split_preserves_text_once() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_table(
                0,
                vec![
                    vec!["A".into(), "B".into(), "C".into()],
                    vec!["D".into(), "E".into(), "F".into()],
                    vec!["G".into(), "H".into(), "I".into()],
                ],
                20.0,
                20.0,
                360.0,
                180.0,
                18.0,
            )
            .unwrap();
        author.merge_cells(0, id, 0, 0, 1, 1).unwrap();
        assert_eq!(author.table_generation(0, id).unwrap(), 1);
        assert_eq!(author.cell_text(0, id, 1, 1).unwrap(), "A\nB\nD\nE");
        assert_eq!(author.table_rows(0, id).unwrap()[1], vec!["", "", "F"]);
        author.set_cell_background(0, id, 1, 1, (1, 2, 3)).unwrap();
        author
            .set_cell_vertical_alignment(0, id, 1, 1, "bottom")
            .unwrap();
        author
            .set_cell_alignment(0, id, 1, 1, Some("right"))
            .unwrap();
        author
            .set_cell_margins(0, id, 1, 1, Some((1.0, 2.0, 3.0, 4.0)))
            .unwrap();
        assert_eq!(
            author.cell_background(0, id, 0, 0).unwrap(),
            Some((1, 2, 3))
        );
        assert_eq!(
            author.cell_vertical_alignment(0, id, 0, 0).unwrap(),
            Some("bottom")
        );
        assert_eq!(author.cell_alignment(0, id, 0, 0).unwrap(), Some("right"));
        assert_eq!(
            author.cell_margins(0, id, 0, 0).unwrap(),
            (1.0, 2.0, 3.0, 4.0)
        );
        assert_eq!(author.table_generation(0, id).unwrap(), 1);
        let snapshot = author.document().unwrap().clone();
        let revision = author.revision().unwrap();
        assert!(author.merge_cells(0, id, 1, 1, 2, 2).is_err());
        assert!(author.split_cell(0, id, 1, 1, Some(2)).is_err());
        assert_eq!(author.document().unwrap(), &snapshot);
        assert_eq!(author.revision().unwrap(), revision);
        let extracted = crate::pptx::extract(&author.to_bytes().unwrap()).unwrap();
        assert_eq!(extracted.pages[0].tables[0].rows[0][0], "A\nB\nD\nE");
        assert!(extracted.pages[0].tables[0].cells[1][1].covered);
        author.set_cell_text(0, id, 1, 1, "Merged").unwrap();
        author.split_cell(0, id, 1, 1, None).unwrap();
        assert_eq!(author.table_generation(0, id).unwrap(), 2);
        assert_eq!(author.cell_text(0, id, 0, 0).unwrap(), "Merged");
        assert_eq!(author.cell_text(0, id, 1, 1).unwrap(), "");
        author.merge_cells(0, id, 2, 0, 2, 1).unwrap();
        author.split_cell(0, id, 2, 1, Some(2)).unwrap();
        assert_eq!(author.table_rows(0, id).unwrap()[2], vec!["G\nH", "", "I"]);
        let reopened = deckcraft_pptx::import(&author.to_bytes().unwrap()).unwrap();
        let ShapeKind::Table(table) = &reopened.slides[0].shapes[0].kind else {
            panic!("missing table")
        };
        assert!(table
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .all(|c| !c.is_covered() && c.grid_span == 1 && c.row_span == 1));
    }

    #[test]
    fn table_native_grid_edits_preserve_merge_anchors_and_generations() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        author.add_slide("blank").unwrap();
        let source = author.slide_id(0).unwrap();
        let id = author
            .add_table(
                0,
                vec![vec!["A".into(), "B".into()], vec!["C".into(), "D".into()]],
                20.0,
                20.0,
                200.0,
                100.0,
                18.0,
            )
            .unwrap();
        author.merge_cells(0, id, 0, 0, 1, 1).unwrap();
        author.insert_row(0, id, 1).unwrap();
        author.insert_column(0, id, 1).unwrap();
        assert_eq!(author.table(0, id).unwrap().rows[0].cells[0].row_span, 3);
        assert_eq!(author.table(0, id).unwrap().rows[0].cells[0].grid_span, 3);
        assert_eq!(author.shape_box(0, id).unwrap(), (20.0, 20.0, 300.0, 150.0));
        author.delete_row(0, id, 0).unwrap();
        author.delete_column(0, id, 0).unwrap();
        assert_eq!(author.cell_text(0, id, 1, 1).unwrap(), "A\nB\nC\nD");
        assert_eq!(author.shape_box(0, id).unwrap(), (20.0, 20.0, 200.0, 100.0));
        assert_eq!(author.table_generation(0, id).unwrap(), 5);
        author.move_slide(0, 1).unwrap();
        assert_eq!(author.slide_index(source).unwrap(), 1);
        assert_eq!(author.table_generation(1, id).unwrap(), 5);
        let mut copy = author.copy().unwrap();
        copy.insert_row(1, id, 2).unwrap();
        assert_eq!(copy.table_generation(1, id).unwrap(), 6);
        assert_eq!(author.table_generation(1, id).unwrap(), 5);
        author.delete_shape(1, id).unwrap();
        assert!(author.table_generation(1, id).is_err());
        assert!(author.table_generations.is_empty());
    }

    #[test]
    fn table_geometry_changes_and_rejected_edits_are_atomic() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let id = author
            .add_table(
                0,
                vec![vec!["A".into(), "B".into()], vec!["C".into(), "D".into()]],
                20.0,
                20.0,
                200.0,
                100.0,
                18.0,
            )
            .unwrap();
        author.set_column_widths(0, id, vec![80.0, 120.0]).unwrap();
        author
            .set_shape_box(0, id, 40.0, 30.0, 400.0, 150.0)
            .unwrap();
        assert_eq!(author.column_widths(0, id).unwrap(), vec![160.0, 240.0]);
        assert_eq!(
            author.row_height(0, id, 0).unwrap(),
            (Some(75.0), "at_least")
        );
        author
            .set_cell_margins(0, id, 0, 0, Some((2.0, 50.0, 2.0, 50.0)))
            .unwrap();
        let before = author.document().unwrap().clone();
        let revision = author.revision().unwrap();
        for widths in [
            vec![f64::NAN, 1.0],
            vec![10.0],
            vec![0.0, 10.0],
            vec![8000.0, 8000.0],
            vec![90.0, 120.0],
        ] {
            assert!(author.set_column_widths(0, id, widths).is_err());
        }
        assert!(author.set_row_height(0, id, 0, None, "auto").is_err());
        assert!(author
            .set_row_height(0, id, 0, Some(100.0), "exact")
            .is_err());
        assert!(author
            .set_row_height(0, id, 0, Some(f64::INFINITY), "at_least")
            .is_err());
        assert!(author.set_shape_box(0, id, 0.0, 0.0, 100.0, 100.0).is_err());
        assert!(author
            .set_cell_margins(0, id, 0, 0, Some((0.0, 200.0, 0.0, 0.0)))
            .is_err());
        assert!(author.set_cell_alignment(0, id, 0, 0, Some("ctr")).is_err());
        assert!(author.set_table_borders(0, id, "#123456", 0.1).is_err());
        assert!(author.insert_row(0, id, 3).is_err());
        assert!(author.insert_column(0, id, 3).is_err());
        assert!(author.split_cell(0, id, 0, 0, None).is_err());
        assert_eq!(author.document().unwrap(), &before);
        assert_eq!(author.revision().unwrap(), revision);
        assert_eq!(author.table_generation(0, id).unwrap(), 0);
        author.delete_row(0, id, 1).unwrap();
        author.delete_column(0, id, 1).unwrap();
        let revision = author.revision().unwrap();
        assert!(author.delete_row(0, id, 0).is_err());
        assert!(author.delete_column(0, id, 0).is_err());
        assert_eq!(author.revision().unwrap(), revision);
    }

    #[test]
    fn table_insertions_preflight_aggregate_cells_even_without_paragraphs() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        author.add_slide("blank").unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let mut table = Table::new(100, 500, 1000.0, 1.0);
            for cell in table.rows.iter_mut().flat_map(|row| &mut row.cells) {
                cell.text.paragraphs.clear();
            }
            let document = author.presentation.as_mut().unwrap();
            ids.push(
                insert_shape(
                    document,
                    0,
                    Shape {
                        xfrm: Some(Xfrm::new(0.0, 0.0, 1000.0, 100.0)),
                        kind: ShapeKind::Table(table),
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
        }
        assert_eq!(
            content_budget(author.document().unwrap()).unwrap().cells,
            MAX_CELLS
        );
        let revision = author.revision().unwrap();
        assert!(author.insert_row(0, ids[0], 1).is_err());
        assert!(author.insert_column(0, ids[1], 1).is_err());
        assert!(author
            .add_table(0, vec![vec![String::new()]], 0.0, 0.0, 100.0, 50.0, 18.0)
            .is_err());
        assert_eq!(author.revision().unwrap(), revision);
        assert_eq!(author.table_generation(0, ids[0]).unwrap(), 0);
        assert_eq!(author.table(0, ids[0]).unwrap().n_rows(), 100);
        assert_eq!(author.table(0, ids[1]).unwrap().n_cols(), 500);
    }
}
