//! Python binding for the independent presentation authoring engine.
use crate::author_pptx::{self, CellParagraphInfo, CellTarget, LinkValue, Stroke, TextFormat};
use pyo3::prelude::*;

#[pyclass(module = "doc_ex._core", name = "EditablePptx")]
pub struct NativeEditablePptx {
    inner: author_pptx::EditablePptx,
}

#[pymethods]
impl NativeEditablePptx {
    #[new]
    #[pyo3(signature = (width=720.0, height=540.0))]
    #[allow(clippy::too_many_arguments)]
    fn new(width: f64, height: f64) -> PyResult<Self> {
        author_pptx::EditablePptx::new(width, height)
            .map(|inner| Self { inner })
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn revision(&self) -> PyResult<u64> {
        self.inner.revision().map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn copy(&self) -> PyResult<Self> {
        self.inner
            .copy()
            .map(|inner| Self { inner })
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn close(&mut self) {
        self.inner.close()
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_count(&self) -> PyResult<usize> {
        self.inner.slide_count().map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_size(&self) -> PyResult<(f64, f64)> {
        self.inner.slide_size().map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_id(&self, slide: usize) -> PyResult<u32> {
        self.inner.slide_id(slide).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_index(&self, id: u32) -> PyResult<usize> {
        self.inner.slide_index(id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_name(&self, slide: usize) -> PyResult<String> {
        self.inner.slide_name(slide).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_slide_name(&mut self, slide: usize, name: &str) -> PyResult<()> {
        self.inner.set_slide_name(slide, name).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_hidden(&self, slide: usize) -> PyResult<bool> {
        self.inner.slide_hidden(slide).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_slide_hidden(&mut self, slide: usize, hidden: bool) -> PyResult<()> {
        self.inner
            .set_slide_hidden(slide, hidden)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_notes(&self, slide: usize) -> PyResult<String> {
        self.inner.slide_notes(slide).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_slide_notes(&mut self, slide: usize, text: &str) -> PyResult<()> {
        self.inner.set_slide_notes(slide, text).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn move_slide(&mut self, slide: usize, destination: usize) -> PyResult<()> {
        self.inner
            .move_slide(slide, destination)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn remove_slide(&mut self, slide: usize) -> PyResult<()> {
        self.inner.remove_slide(slide).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn to_bytes(&self) -> PyResult<Vec<u8>> {
        self.inner.to_bytes().map_err(Into::into)
    }

    #[pyo3(signature = (layout="blank"))]
    #[allow(clippy::too_many_arguments)]
    fn add_slide(&mut self, layout: &str) -> PyResult<usize> {
        self.inner.add_slide(layout).map_err(Into::into)
    }

    #[pyo3(signature = (slide, text, x, y, width, height, size=18.0))]
    #[allow(clippy::too_many_arguments)]
    fn add_text(
        &mut self,
        slide: usize,
        text: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        size: f64,
    ) -> PyResult<u32> {
        self.inner
            .add_text(slide, text, x, y, width, height, size)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn background(&mut self, slide: usize, color: (u8, u8, u8)) -> PyResult<()> {
        self.inner.background(slide, color).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn clear_background(&mut self, slide: usize) -> PyResult<()> {
        self.inner.clear_background(slide).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn background_image(&self, slide: usize) -> PyResult<Option<Vec<u8>>> {
        self.inner.background_image(slide).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_background_image(&mut self, slide: usize, data: &[u8]) -> PyResult<()> {
        self.inner
            .set_background_image(slide, data)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_ids(&self, slide: usize) -> PyResult<Vec<(u32, &'static str)>> {
        self.inner.shape_ids(slide).map_err(Into::into)
    }

    #[pyo3(signature = (slide, kind, x, y, width, height, fill=None, stroke=None, stroke_width=1.0))]
    #[allow(clippy::too_many_arguments)]
    fn add_shape(
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
        self.inner
            .add_shape(slide, kind, x, y, width, height, fill, stroke, stroke_width)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_placeholder(&self, slide: usize, id: u32) -> PyResult<Option<&'static str>> {
        self.inner.shape_placeholder(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_rotation(&self, slide: usize, id: u32) -> PyResult<f64> {
        self.inner.shape_rotation(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_shape_rotation(&mut self, slide: usize, id: u32, degrees: f64) -> PyResult<()> {
        self.inner
            .set_shape_rotation(slide, id, degrees)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn move_shape(&mut self, slide: usize, id: u32, destination: usize) -> PyResult<()> {
        self.inner
            .move_shape(slide, id, destination)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn delete_shape(&mut self, slide: usize, id: u32) -> PyResult<()> {
        self.inner.delete_shape(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_fill(&self, slide: usize, id: u32) -> PyResult<Option<(u8, u8, u8)>> {
        self.inner.shape_fill(slide, id).map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, color=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_shape_fill(
        &mut self,
        slide: usize,
        id: u32,
        color: Option<(u8, u8, u8)>,
    ) -> PyResult<()> {
        self.inner
            .set_shape_fill(slide, id, color)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_stroke(&self, slide: usize, id: u32) -> PyResult<Stroke> {
        self.inner.shape_stroke(slide, id).map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, color=None, width=1.0))]
    #[allow(clippy::too_many_arguments)]
    fn set_shape_stroke(
        &mut self,
        slide: usize,
        id: u32,
        color: Option<(u8, u8, u8)>,
        width: f64,
    ) -> PyResult<()> {
        self.inner
            .set_shape_stroke(slide, id, color, width)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn slide_background(&self, slide: usize) -> PyResult<Option<(u8, u8, u8)>> {
        self.inner.slide_background(slide).map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn shape_text(&self, slide: usize, id: u32, cell: Option<CellTarget>) -> PyResult<String> {
        self.inner.shape_text(slide, id, cell).map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, start, end, cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn text_range(
        &self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        cell: Option<CellTarget>,
    ) -> PyResult<(String, u64)> {
        self.inner
            .text_range(slide, id, start, end, cell)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, start, end, revision, cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn range_text(
        &self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        cell: Option<CellTarget>,
    ) -> PyResult<String> {
        self.inner
            .range_text(slide, id, start, end, revision, cell)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, text, patch="{}", cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn add_run_patch(
        &mut self,
        slide: usize,
        id: u32,
        text: &str,
        patch: &str,
        cell: Option<CellTarget>,
    ) -> PyResult<(usize, usize, u64)> {
        self.inner
            .add_run_patch(slide, id, text, patch, cell)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, start, end, revision, text, cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn replace_range(
        &mut self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        text: &str,
        cell: Option<CellTarget>,
    ) -> PyResult<(usize, usize, u64)> {
        self.inner
            .replace_range(slide, id, start, end, revision, text, cell)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, start, end, revision, patch, cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn format_range_patch(
        &mut self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        patch: &str,
        cell: Option<CellTarget>,
    ) -> PyResult<u64> {
        self.inner
            .format_range_patch(slide, id, start, end, revision, patch, cell)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, start, end, revision, allow_mixed=false, cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn range_link(
        &self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        allow_mixed: bool,
        cell: Option<CellTarget>,
    ) -> PyResult<LinkValue> {
        self.inner
            .range_link(slide, id, start, end, revision, allow_mixed, cell)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, start, end, revision, url=None, target_slide=None, tooltip="", cell=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_range_link(
        &mut self,
        slide: usize,
        id: u32,
        start: usize,
        end: usize,
        revision: u64,
        url: Option<&str>,
        target_slide: Option<u32>,
        tooltip: &str,
        cell: Option<CellTarget>,
    ) -> PyResult<u64> {
        self.inner
            .set_range_link(
                slide,
                id,
                start,
                end,
                revision,
                url,
                target_slide,
                tooltip,
                cell,
            )
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_link(&self, slide: usize, id: u32) -> PyResult<LinkValue> {
        self.inner.shape_link(slide, id).map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, url=None, target_slide=None, tooltip=""))]
    #[allow(clippy::too_many_arguments)]
    fn set_shape_link(
        &mut self,
        slide: usize,
        id: u32,
        url: Option<&str>,
        target_slide: Option<u32>,
        tooltip: &str,
    ) -> PyResult<()> {
        self.inner
            .set_shape_link(slide, id, url, target_slide, tooltip)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_shape_text(&mut self, slide: usize, id: u32, text: &str) -> PyResult<()> {
        self.inner
            .set_shape_text(slide, id, text)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_box(&self, slide: usize, id: u32) -> PyResult<(f64, f64, f64, f64)> {
        self.inner.shape_box(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_shape_box(
        &mut self,
        slide: usize,
        id: u32,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> PyResult<()> {
        self.inner
            .set_shape_box(slide, id, x, y, width, height)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn table_rows(&self, slide: usize, id: u32) -> PyResult<Vec<Vec<String>>> {
        self.inner.table_rows(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn table_generation(&self, slide: usize, id: u32) -> PyResult<u64> {
        self.inner.table_generation(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn column_widths(&self, slide: usize, id: u32) -> PyResult<Vec<f64>> {
        self.inner.column_widths(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_column_widths(&mut self, slide: usize, id: u32, widths: Vec<f64>) -> PyResult<()> {
        self.inner
            .set_column_widths(slide, id, widths)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn row_height(
        &self,
        slide: usize,
        id: u32,
        row: usize,
    ) -> PyResult<(Option<f64>, &'static str)> {
        self.inner.row_height(slide, id, row).map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, row, height=None, rule="auto"))]
    #[allow(clippy::too_many_arguments)]
    fn set_row_height(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        height: Option<f64>,
        rule: &str,
    ) -> PyResult<()> {
        self.inner
            .set_row_height(slide, id, row, height, rule)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn table_borders(&self, slide: usize, id: u32) -> PyResult<Option<(String, f64)>> {
        self.inner.table_borders(slide, id).map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, color="#000000", width=0.5))]
    #[allow(clippy::too_many_arguments)]
    fn set_table_borders(
        &mut self,
        slide: usize,
        id: u32,
        color: &str,
        width: f64,
    ) -> PyResult<()> {
        self.inner
            .set_table_borders(slide, id, color, width)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn clear_table_borders(&mut self, slide: usize, id: u32) -> PyResult<()> {
        self.inner
            .clear_table_borders(slide, id)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_text(&self, slide: usize, id: u32, row: usize, col: usize) -> PyResult<String> {
        self.inner
            .cell_text(slide, id, row, col)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_margins(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<(f64, f64, f64, f64)> {
        self.inner
            .cell_margins(slide, id, row, col)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, row, col, margins=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_cell_margins(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        margins: Option<(f64, f64, f64, f64)>,
    ) -> PyResult<()> {
        self.inner
            .set_cell_margins(slide, id, row, col, margins)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_alignment(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<Option<&'static str>> {
        self.inner
            .cell_alignment(slide, id, row, col)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, row, col, alignment=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_cell_alignment(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        alignment: Option<&str>,
    ) -> PyResult<()> {
        self.inner
            .set_cell_alignment(slide, id, row, col, alignment)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn merge_cells(
        &mut self,
        slide: usize,
        id: u32,
        r0: usize,
        c0: usize,
        r1: usize,
        c1: usize,
    ) -> PyResult<()> {
        self.inner
            .merge_cells(slide, id, r0, c0, r1, c1)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, row, col, columns=None))]
    #[allow(clippy::too_many_arguments)]
    fn split_cell(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        columns: Option<usize>,
    ) -> PyResult<()> {
        self.inner
            .split_cell(slide, id, row, col, columns)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_row(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        self.inner.insert_row(slide, id, index).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn delete_row(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        self.inner.delete_row(slide, id, index).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_column(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        self.inner
            .insert_column(slide, id, index)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn delete_column(&mut self, slide: usize, id: u32, index: usize) -> PyResult<()> {
        self.inner
            .delete_column(slide, id, index)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_paragraph_info(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<CellParagraphInfo> {
        self.inner
            .cell_paragraph_info(slide, id, row, col)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn move_cell_paragraph(
        &mut self,
        slide: usize,
        id: u32,
        source: CellTarget,
        destination: CellTarget,
        revision: u64,
    ) -> PyResult<bool> {
        self.inner
            .move_cell_paragraph(slide, id, source, destination, revision)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn remove_cell_paragraph(&mut self, slide: usize, id: u32, cell: CellTarget) -> PyResult<()> {
        self.inner
            .remove_cell_paragraph(slide, id, cell)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_add_paragraph(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        text: &str,
    ) -> PyResult<CellTarget> {
        self.inner
            .cell_add_paragraph(slide, id, row, col, text)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_add_break(
        &mut self,
        slide: usize,
        id: u32,
        cell: CellTarget,
        kind: &str,
        revision: u64,
    ) -> PyResult<(usize, usize, u64)> {
        self.inner
            .cell_add_break(slide, id, cell, kind, revision)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_paragraph_alignment(
        &self,
        slide: usize,
        id: u32,
        cell: CellTarget,
    ) -> PyResult<Option<&'static str>> {
        self.inner
            .cell_paragraph_alignment(slide, id, cell)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_paragraph_formatting(
        &self,
        slide: usize,
        id: u32,
        cell: CellTarget,
    ) -> PyResult<String> {
        self.inner
            .cell_paragraph_formatting(slide, id, cell)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn format_cell_paragraph(
        &mut self,
        slide: usize,
        id: u32,
        cell: CellTarget,
        patch: &str,
        revision: u64,
    ) -> PyResult<()> {
        self.inner
            .format_cell_paragraph(slide, id, cell, patch, revision)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, cell, alignment=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_cell_paragraph_alignment(
        &mut self,
        slide: usize,
        id: u32,
        cell: CellTarget,
        alignment: Option<&str>,
    ) -> PyResult<()> {
        self.inner
            .set_cell_paragraph_alignment(slide, id, cell, alignment)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_cell_text(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        text: &str,
    ) -> PyResult<()> {
        self.inner
            .set_cell_text(slide, id, row, col, text)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_background(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<Option<(u8, u8, u8)>> {
        self.inner
            .cell_background(slide, id, row, col)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_cell_background(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        color: (u8, u8, u8),
    ) -> PyResult<()> {
        self.inner
            .set_cell_background(slide, id, row, col, color)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn cell_vertical_alignment(
        &self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
    ) -> PyResult<Option<&'static str>> {
        self.inner
            .cell_vertical_alignment(slide, id, row, col)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_cell_vertical_alignment(
        &mut self,
        slide: usize,
        id: u32,
        row: usize,
        col: usize,
        alignment: &str,
    ) -> PyResult<()> {
        self.inner
            .set_cell_vertical_alignment(slide, id, row, col, alignment)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_text_color(&mut self, slide: usize, id: u32, color: (u8, u8, u8)) -> PyResult<()> {
        self.inner
            .set_text_color(slide, id, color)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, id, size=None, color=None, bold=None, italic=None, font=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_text_format(
        &mut self,
        slide: usize,
        id: u32,
        size: Option<f64>,
        color: Option<(u8, u8, u8)>,
        bold: Option<bool>,
        italic: Option<bool>,
        font: Option<&str>,
    ) -> PyResult<()> {
        self.inner
            .set_text_format(slide, id, size, color, bold, italic, font)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn text_format(&self, slide: usize, id: u32) -> PyResult<TextFormat> {
        self.inner.text_format(slide, id).map_err(Into::into)
    }

    #[pyo3(signature = (slide, rows, x, y, width, height, size=18.0))]
    #[allow(clippy::too_many_arguments)]
    fn add_table(
        &mut self,
        slide: usize,
        rows: Vec<Vec<String>>,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        size: f64,
    ) -> PyResult<u32> {
        self.inner
            .add_table(slide, rows, x, y, width, height, size)
            .map_err(Into::into)
    }

    #[pyo3(signature = (slide, data, x, y, width, height, alt=""))]
    #[allow(clippy::too_many_arguments)]
    fn add_image(
        &mut self,
        slide: usize,
        data: &[u8],
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        alt: &str,
    ) -> PyResult<u32> {
        self.inner
            .add_image(slide, data, x, y, width, height, alt)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn picture_data(&self, slide: usize, id: u32) -> PyResult<Vec<u8>> {
        self.inner.picture_data(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_picture_data(&mut self, slide: usize, id: u32, data: &[u8]) -> PyResult<()> {
        self.inner
            .set_picture_data(slide, id, data)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn picture_alt(&self, slide: usize, id: u32) -> PyResult<String> {
        self.inner.picture_alt(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_picture_alt(&mut self, slide: usize, id: u32, alt: &str) -> PyResult<()> {
        self.inner
            .set_picture_alt(slide, id, alt)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn picture_crop(&self, slide: usize, id: u32) -> PyResult<(f64, f64, f64, f64)> {
        self.inner.picture_crop(slide, id).map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn set_picture_crop(
        &mut self,
        slide: usize,
        id: u32,
        crop: (f64, f64, f64, f64),
    ) -> PyResult<()> {
        self.inner
            .set_picture_crop(slide, id, crop)
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    fn picture_flips(&self, slide: usize, id: u32) -> PyResult<(bool, bool)> {
        self.inner.picture_flips(slide, id).map_err(Into::into)
    }

    #[pyo3(signature = (slide,id,flip_h=None,flip_v=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_picture_flips(
        &mut self,
        slide: usize,
        id: u32,
        flip_h: Option<bool>,
        flip_v: Option<bool>,
    ) -> PyResult<()> {
        self.inner
            .set_picture_flips(slide, id, flip_h, flip_v)
            .map_err(Into::into)
    }
}
