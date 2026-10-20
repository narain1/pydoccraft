//! Bounded authoring of new presentations through DeckCraft's public model/writer.
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;

use deckcraft_model::style::PictureFill;
use deckcraft_model::text::{Anchor, AutoFit, RunProps, TextBody};
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

type TextFormat = (
    Option<f64>,
    Option<(u8, u8, u8)>,
    Option<bool>,
    Option<bool>,
    Option<String>,
);
type Stroke = Option<((u8, u8, u8), f64)>;

#[pyclass(module = "doc_ex._core")]
#[derive(Clone)]
pub struct EditablePptx {
    presentation: Option<Presentation>,
    text_bytes: usize,
    media_bytes: usize,
    shapes: usize,
}

impl EditablePptx {
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
        let errors = staged.validate();
        if !errors.is_empty() {
            return Err(EngineError::Malformed(format!(
                "invalid authored presentation: {}",
                errors.join("; ")
            ))
            .into());
        }
        self.presentation = Some(staged);
        Ok(result)
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

    fn refresh_budgets(&mut self) -> PyResult<()> {
        let document = self.document()?;
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
        })
    }

    pub fn copy(&self) -> PyResult<Self> {
        self.document()?;
        Ok(self.clone())
    }

    pub fn close(&mut self) {
        self.presentation = None;
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

    pub fn add_slide(&mut self) -> PyResult<usize> {
        if self.document()?.slides.len() >= MAX_SLIDES {
            return Err(
                EngineError::ResourceLimit("presentation exceeds 10000 slides".into()).into(),
            );
        }
        self.mutate(|document| {
            let layout = deckcraft_model::defaults::layout_of_kind(document, LayoutType::Blank)
                .ok_or_else(|| EngineError::Malformed("default blank layout is missing".into()))?;
            let slide =
                deckcraft_model::defaults::new_slide(document, layout).ok_or_else(|| {
                    EngineError::ResourceLimit("presentation object IDs exhausted".into())
                })?;
            let index = document.slides.len();
            document.slides.push(Arc::new(slide));
            Ok(index)
        })
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
            if document
                .media
                .iter()
                .map(|media| media.data.len())
                .sum::<usize>()
                > MAX_MEDIA
            {
                return Err(
                    EngineError::ResourceLimit("presentation images exceed 64 MiB".into()).into(),
                );
            }
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

    pub fn shape_rotation(&self, slide: usize, id: u32) -> PyResult<f64> {
        self.shape(slide, id)?
            .xfrm
            .as_ref()
            .map(|value| value.rot)
            .ok_or_else(|| PyValueError::new_err("shape has no explicit geometry"))
    }

    pub fn set_shape_rotation(&mut self, slide: usize, id: u32, degrees: f64) -> PyResult<()> {
        self.shape(slide, id)?;
        if !degrees.is_finite() || !(-360.0..=360.0).contains(&degrees) {
            return Err(PyValueError::new_err(
                "rotation must be finite and between -360 and 360 degrees",
            ));
        }
        self.mutate(|document| {
            shape_mut(document, slide, id)?
                .xfrm
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("shape has no explicit geometry"))?
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

    pub fn set_shape_text(&mut self, slide: usize, id: u32, text: &str) -> PyResult<()> {
        let old = self.shape_text(slide, id)?.len();
        text_value(text)?;
        self.check_text_budget(text.len().saturating_sub(old))?;
        self.check_paragraph_budget(
            paragraph_count(text),
            self.shape(slide, id)?
                .text
                .as_ref()
                .map_or(0, |body| body.paragraphs.len()),
        )?;
        self.mutate(|document| {
            let body = shape_mut(document, slide, id)?
                .text
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("shape has no text body"))?;
            replace_text(body, text);
            Ok(())
        })?;
        self.text_bytes = self.text_bytes.saturating_sub(old) + text.len();
        Ok(())
    }

    pub fn shape_box(&self, slide: usize, id: u32) -> PyResult<(f64, f64, f64, f64)> {
        let transform = self
            .shape(slide, id)?
            .xfrm
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("shape has no explicit geometry"))?;
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
        self.shape(slide, id)?;
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
            }
            let transform = shape
                .xfrm
                .get_or_insert_with(|| Xfrm::new(x, y, width, height));
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
            .map(|row| row.cells.iter().map(|cell| cell.text.text()).collect())
            .collect())
    }

    pub fn set_cell_text(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        text: &str,
    ) -> PyResult<()> {
        let ShapeKind::Table(table) = &self.shape(slide, id)?.kind else {
            return Err(PyValueError::new_err("shape is not a table"));
        };
        let cell = table
            .cell(row, col)
            .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))?;
        let old = cell.text.text().len();
        let paragraphs = cell.text.paragraphs.len();
        text_value(text)?;
        self.check_text_budget(text.len().saturating_sub(old))?;
        self.check_paragraph_budget(paragraph_count(text), paragraphs)?;
        self.mutate(|document| {
            let ShapeKind::Table(table) = &mut shape_mut(document, slide, id)?.kind else {
                return Err(PyValueError::new_err("shape is not a table"));
            };
            let cell = table
                .cell_mut(row, col)
                .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))?;
            replace_text(&mut cell.text, text);
            Ok(())
        })?;
        self.text_bytes = self.text_bytes.saturating_sub(old) + text.len();
        Ok(())
    }

    pub fn cell_background(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<Option<(u8, u8, u8)>> {
        let ShapeKind::Table(table) = &self.shape(slide, id)?.kind else {
            return Err(PyValueError::new_err("shape is not a table"));
        };
        let cell = table
            .cell(row, col)
            .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))?;
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
        let ShapeKind::Table(table) = &self.shape(slide, id)?.kind else {
            return Err(PyValueError::new_err("shape is not a table"));
        };
        let cell = table
            .cell(row, col)
            .ok_or_else(|| PyIndexError::new_err("table cell index out of range"))?;
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
        self.mutate(|document| {
            let body = shape_mut(document, slide, id)?
                .text
                .as_mut()
                .ok_or_else(|| PyValueError::new_err("shape has no text body"))?;
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
        if data.len() > MAX_MEDIA.saturating_sub(self.media_bytes) {
            return Err(
                EngineError::ResourceLimit("presentation images exceed 64 MiB".into()).into(),
            );
        }
        validate_png(data)?;
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
            insert_shape(document, slide, shape)
        })?;
        // Count submitted bytes conservatively even when upstream deduplicates media.
        self.media_bytes += data.len();
        self.text_bytes += alt.len();
        self.shapes += 1;
        Ok(id)
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
            .flat_map(|paragraph| &paragraph.runs)
            .any(|run| run.props.link.is_some() || run.props.hover.is_some())
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
                shape.click.is_some()
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
        return Err(EngineError::Unsupported("slide organization requires the authored text/table/PNG profile without navigation, animation or package references".into()).into());
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

    #[test]
    fn saves_native_slides_text_tables_and_background() {
        pyo3::prepare_freethreaded_python();
        let mut author = EditablePptx::new(720.0, 540.0).unwrap();
        assert_eq!(author.add_slide().unwrap(), 0);
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
        author.add_slide().unwrap();
        let mut copied = author.copy().unwrap();
        copied.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
        exhausted.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
        author.add_slide().unwrap();
        author.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
        author.add_slide().unwrap();
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
}
