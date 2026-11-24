//! Lay out styled XHTML in its viewport, then paint with the existing native renderer.
use std::collections::HashMap;

use roxmltree::Node;
use taffy::{geometry::Size, prelude::*, style::Overflow, LayoutInput, LayoutOutput};
use wordcraft_doc::{
    props::{Rgb, TextDirection},
    Blocks, Document,
};
use wordcraft_geom::Rect;
use wordcraft_layout::{
    display::{page_display, Draw},
    LayoutCache, LayoutOptions, Page, Placed, ViewMode,
};

use super::{
    child, content,
    css::Cascade,
    fonts::{Fonts, Text as FontText},
    invalid, limited, name,
    output::{image_bytes, Output, RasterImage},
    parse_xml,
    rendition::{fixed, Region},
    style::{Paint, Styled},
    unsupported, Book, XHTML, XML_BYTES,
};
use crate::{
    model::{DocumentPayload, PagePayload},
    rendering, EngineError,
};

const REFLOW_WIDTH: f32 = 612.0;
const MAX_GEOMETRY: f32 = 20_000.0;
const MAX_BOXES: usize = 8_000;

pub(super) fn render(
    data: &[u8],
    book: &DocumentPayload,
    page: &PagePayload,
    dpi: f64,
    region: Region,
) -> Result<Option<wordcraft_render::Rendered>, EngineError> {
    let fixed = fixed(page);
    rendering::validate_dpi(dpi)?;
    let path = page
        .metadata
        .get("chapter_path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| invalid("missing EPUB chapter path"))?;
    let mut archive = Book::open(data)?;
    let source = archive.read(path, XML_BYTES)?;
    let doc = parse_xml(&source)?;
    let root = doc.root_element();
    if root.has_tag_name((super::svg::NS, "svg")) {
        return super::svg::render(root, &source, path, &mut archive, fixed, dpi, region).map(Some);
    }
    let body = child(root, XHTML, "body").ok_or_else(|| invalid("XHTML chapter has no body"))?;
    let viewport = if fixed {
        dimensions(root).ok_or_else(|| {
            unsupported("fixed-layout XHTML requires numeric viewport width and height")
        })?
    } else {
        [REFLOW_WIDTH, 792.0]
    };
    let mut cascade = Cascade::load(root, path, &mut archive, viewport)?;
    if !fixed && !cascade.needed(root) {
        return Ok(None);
    }
    let styles = Styled::compute(root, &mut cascade, viewport)?;
    let fonts = Fonts::load(&cascade.fonts, &styles, &mut archive)?;
    let manifest = book
        .metadata
        .get("manifest")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let path = item["path"].as_str()?;
            let media = item["media_type"].as_str()?;
            Some((path.to_owned(), media))
        })
        .collect::<HashMap<_, _>>();
    let mut budget = content::Budget::default();
    let mut context = content::Context {
        source: &source,
        path,
        page: page.number,
        manifest: &manifest,
        book: &mut archive,
        budget: &mut budget,
        styles: Some(&styles),
    };
    let mut layout = Layout::new(Output::new(data, book, false, dpi), viewport, fonts);
    let root_id = layout.build(root, &mut context)?;
    let id = layout.boxes[root_id].id;
    let mut root_style = layout.tree.style(id).map_err(failure)?.clone();
    root_style.size.width = Dimension::length(viewport[0]);
    if fixed {
        root_style.size.height = Dimension::length(viewport[1]);
    }
    root_style.box_sizing = taffy::style::BoxSizing::BorderBox;
    layout.tree.set_style(id, root_style).map_err(failure)?;
    let dimension = layout.place(root_id, fixed)?;
    let html_color = styles[&root.id()]
        .background
        .filter(|(_, alpha)| *alpha > 0.0);
    let paper = html_color.or(styles[&body.id()].background);
    let mut drawing = Vec::new();
    fill(
        &mut drawing,
        Rect::new(0.0, 0.0, dimension[0], dimension[1]),
        Some((Rgb::WHITE, 1.0)),
    );
    fill(
        &mut drawing,
        Rect::new(0.0, 0.0, dimension[0], dimension[1]),
        paper,
    );
    // The HTML canvas background is propagated from the root, or from body when the root has none.
    // Suppress its box fill so translucent page colors are composited once.
    let canvas_source = if html_color.is_some() {
        root.id()
    } else {
        body.id()
    };
    layout.paint(
        root_id,
        [0.0, 0.0],
        canvas_source,
        &mut HashMap::new(),
        &mut drawing,
    )?;
    raster(&layout.output.document, &drawing, dimension, dpi, region).map(Some)
}

pub(super) fn dimensions(root: Node<'_, '_>) -> Option<[f32; 2]> {
    let head = child(root, XHTML, "head")?;
    let meta = head.children().find(|node| {
        name(*node, XHTML, "meta")
            && node
                .attribute("name")
                .is_some_and(|value| value.eq_ignore_ascii_case("viewport"))
    })?;
    let mut input = meta.attribute("content")?;
    let separator = |ch: char| ch.is_ascii_whitespace() || matches!(ch, ',' | ';');
    let mut dims = [None; 2];
    loop {
        input = input.trim_start_matches(separator);
        if input.is_empty() {
            break;
        }
        let key_len = input
            .find(|ch| separator(ch) || ch == '=')
            .unwrap_or(input.len());
        let (key, remaining) = input.split_at(key_len);
        let remaining = remaining.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        let Some(value) = remaining.strip_prefix('=') else {
            input = remaining;
            continue;
        };
        let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        let (value, remaining) = value.split_at(value.find(separator).unwrap_or(value.len()));
        input = remaining;
        let index = if key.eq_ignore_ascii_case("width") {
            0
        } else if key.eq_ignore_ascii_case("height") {
            1
        } else {
            continue;
        };
        if dims[index].is_none() {
            let pixels = numeric_prefix(value)?;
            let points = pixels * 0.75;
            if !points.is_finite() || points <= 0.0 || points > MAX_GEOMETRY {
                return None;
            }
            dims[index] = Some(points);
        }
    }
    Some([dims[0]?, dims[1]?])
}

fn numeric_prefix(source: &str) -> Option<f32> {
    let bytes = source.as_bytes();
    let mut end = usize::from(bytes.first().is_some_and(|b| matches!(b, b'+' | b'-')));
    end += bytes[end..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        end += bytes[end..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
    }
    if bytes.get(end).is_some_and(|b| matches!(b, b'e' | b'E')) {
        let mut exponent = end + 1;
        exponent += usize::from(
            bytes
                .get(exponent)
                .is_some_and(|b| matches!(b, b'+' | b'-')),
        );
        let digits = bytes[exponent..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if digits != 0 {
            end = exponent + digits;
        }
    }
    source[..end].parse().ok()
}

struct Layout<'book> {
    tree: TaffyTree<usize>,
    boxes: Vec<PaintBox>,
    output: Output<'book>,
    cache: LayoutCache,
    viewport: [f32; 2],
    measure_count: usize,
    glyph_paths_remaining: usize,
    fonts: Option<Fonts>,
}

struct PaintBox {
    id: NodeId,
    dom: Option<roxmltree::NodeId>,
    children: Vec<usize>,
    content: Option<Contents>,
    background: Option<Paint>,
    borders: [Option<Paint>; 4],
    z_index: i32,
    clip_overflow: bool,
    invisible: bool,
}

enum Contents {
    Text(Blocks),
    FontText(Vec<FontText>),
    Image(Picture, String),
}

enum Picture {
    Raster(RasterImage),
    Svg(Box<resvg::usvg::Tree>),
}

impl Picture {
    fn size(&self) -> [f32; 2] {
        match self {
            Self::Raster(image) => [image.width, image.height],
            Self::Svg(tree) => [tree.size().width(), tree.size().height()],
        }
    }

    fn media(&self, output: &mut Output<'_>, rect: Rect) -> Result<String, EngineError> {
        match self {
            Self::Raster(image) => Ok(image.media.clone()),
            Self::Svg(tree) => output.rasterize_svg(tree, rect.w, rect.h),
        }
    }
}

fn is_svg(node: Node<'_, '_>) -> bool {
    node.has_tag_name((super::svg::NS, "svg"))
}

impl<'book> Layout<'book> {
    fn new(output: Output<'book>, viewport: [f32; 2], fonts: Option<Fonts>) -> Self {
        let mut tree = TaffyTree::new();
        tree.disable_rounding();
        Self {
            tree,
            boxes: Vec::new(),
            output,
            cache: LayoutCache::new(),
            viewport,
            measure_count: 0,
            glyph_paths_remaining: 1_000_000,
            fonts,
        }
    }

    fn build(
        &mut self,
        node: Node<'_, '_>,
        context: &mut content::Context<'_, '_>,
    ) -> Result<usize, EngineError> {
        let styled = context
            .styles
            .and_then(|styles| styles.get(&node.id()))
            .ok_or_else(|| invalid("missing EPUB node style"))?;
        let mut style = styled.box_style.clone();
        let mut children = Vec::new();
        let mut local = None;
        if !styled.hidden {
            let image = if is_svg(node) {
                Some(content::inline_svg(node, context)?)
            } else if name(node, XHTML, "img") && node.has_attribute("src") {
                Some(content::image(
                    super::attribute(node, "src")?,
                    node.attribute("alt").unwrap_or_default(),
                    None,
                    context,
                )?)
            } else {
                None
            };
            if let Some(source) = image {
                let picture = if source.media_type == "image/svg+xml" {
                    let base = super::svg::image_base(&source)?;
                    Picture::Svg(Box::new(super::svg::embedded(
                        &image_bytes(&source)?,
                        base,
                        context.book,
                    )?))
                } else {
                    Picture::Raster(self.output.load_image(&source)?)
                };
                let [width, height] = picture.size();
                style.item_is_replaced = true;
                style.aspect_ratio.get_or_insert(width / height);
                for (dim, value) in [
                    (&mut style.size.width, node.attribute("width")),
                    (&mut style.size.height, node.attribute("height")),
                ] {
                    if dim.is_auto() {
                        if let Some(n) = value
                            .and_then(|n| n.parse::<f32>().ok())
                            .filter(|n| *n > 0.0 && *n <= MAX_GEOMETRY)
                        {
                            *dim = Dimension::length(n * 0.75);
                        }
                    }
                }
                local = Some(Contents::Image(picture, source.alt));
            } else if name(node, XHTML, "table") {
                let blocks = content::layout_piece(node, &[], context)?;
                if self.uses_packaged_fonts(&blocks)? {
                    let table = blocks
                        .iter()
                        .find_map(|block| block.table.as_ref())
                        .ok_or_else(|| invalid("missing EPUB table grid"))?;
                    children = self.font_table(node, table, &mut style, context)?;
                } else {
                    local = Some(self.text_contents(&blocks)?);
                }
            } else {
                let mut run = Vec::new();
                let styles = context.styles;
                let visible = node.children().filter(|child| {
                    !child.is_element() || !styles.is_some_and(|styles| styles[&child.id()].hidden)
                });
                let mut significant = visible.clone().filter(|child| {
                    child.is_element() || child.text().is_some_and(|text| !text.trim().is_empty())
                });
                let only_image = significant
                    .next()
                    .is_some_and(|child| name(child, XHTML, "img") || is_svg(child))
                    && significant.next().is_none();
                for child in visible {
                    let layout_child = child.is_element()
                        && context.styles.is_some_and(|styles| {
                            !styles[&child.id()].inline
                                || matches!(style.display, Display::Flex | Display::Grid)
                                || only_image
                        })
                        && (child.tag_name().namespace().is_none_or(|ns| ns == XHTML)
                            || is_svg(child));
                    if layout_child {
                        self.inline_run(node, &run, &mut children, context)?;
                        run.clear();
                        children.push(self.build(child, context)?);
                    } else if child.is_element() || child.is_text() {
                        run.push(child);
                    }
                }
                if children.is_empty() {
                    local = self.text(node, &run, context)?;
                } else {
                    self.inline_run(node, &run, &mut children, context)?;
                }
            }
        }
        let clip_overflow =
            style.overflow.x != Overflow::Visible || style.overflow.y != Overflow::Visible;
        self.add(
            style,
            PaintBox {
                id: NodeId::from(0usize),
                dom: Some(node.id()),
                children,
                content: local,
                background: styled.background,
                borders: styled.border,
                z_index: styled.z_index,
                clip_overflow,
                invisible: styled.text.invisible,
            },
        )
    }

    fn font_table(
        &mut self,
        node: Node<'_, '_>,
        table: &crate::model::TablePayload,
        style: &mut Style,
        context: &mut content::Context<'_, '_>,
    ) -> Result<Vec<usize>, EngineError> {
        use taffy::style_helpers::{fr, line, span};

        let columns = table.rows.first().map_or(0, Vec::len);
        if columns == 0 || columns > 128 {
            return Err(limited("EPUB styled table exceeds 128 columns or is empty"));
        }
        let rows = node
            .children()
            .filter(Node::is_element)
            .flat_map(|section| {
                if name(section, XHTML, "tr") {
                    vec![section]
                } else if ["thead", "tbody", "tfoot"]
                    .iter()
                    .any(|group| name(section, XHTML, group))
                {
                    section.children().filter(Node::is_element).collect()
                } else {
                    Vec::new()
                }
            })
            .collect::<Vec<_>>();
        if rows.len() != table.cells.len() {
            return Err(invalid("EPUB styled table row count differs from its grid"));
        }
        style.display = Display::Grid;
        if style.grid_template_columns.is_empty() {
            style.grid_template_columns = vec![fr(1.0); columns];
        }
        let mut children = Vec::new();
        if let Some(caption) = super::child(node, XHTML, "caption") {
            let index = self.build(caption, context)?;
            let id = self.boxes[index].id;
            let mut placement = self.tree.style(id).map_err(failure)?.clone();
            placement.grid_column.start = line(1);
            placement.grid_column.end = span(columns as u16);
            placement.grid_row.start = line(1);
            self.tree.set_style(id, placement).map_err(failure)?;
            children.push(index);
        }
        let first_row = 1 + children.len();
        for (row, (element, cells)) in rows.into_iter().zip(&table.cells).enumerate() {
            let mut positions = element.children().filter(Node::is_element);
            for cell in cells {
                let element = positions
                    .next()
                    .ok_or_else(|| invalid("EPUB styled table cell is missing"))?;
                let index = self.build(element, context)?;
                let id = self.boxes[index].id;
                let column = cell.metadata["grid_column"]
                    .as_u64()
                    .ok_or_else(|| invalid("EPUB styled table cell has no column"))?
                    as usize;
                let mut placement = self.tree.style(id).map_err(failure)?.clone();
                placement.grid_column.start = line((column + 1) as i16);
                placement.grid_column.end = span(cell.col_span as u16);
                placement.grid_row.start = line((row + first_row) as i16);
                placement.grid_row.end = span(cell.row_span as u16);
                self.tree.set_style(id, placement).map_err(failure)?;
                children.push(index);
            }
            if positions.next().is_some() {
                return Err(invalid(
                    "EPUB styled table cell count differs from its grid",
                ));
            }
        }
        Ok(children)
    }

    fn uses_packaged_fonts(
        &self,
        blocks: &[crate::model::BlockPayload],
    ) -> Result<bool, EngineError> {
        if let Some(fonts) = &self.fonts {
            for block in blocks {
                if fonts.used_in(block)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn text<'a, 'input>(
        &mut self,
        parent: Node<'a, 'input>,
        nodes: &[Node<'a, 'input>],
        context: &mut content::Context<'_, '_>,
    ) -> Result<Option<Contents>, EngineError> {
        let blocks = content::layout_piece(parent, nodes, context)?;
        if blocks.is_empty() {
            return Ok(None);
        }
        self.text_contents(&blocks).map(Some)
    }

    fn text_contents(
        &mut self,
        blocks: &[crate::model::BlockPayload],
    ) -> Result<Contents, EngineError> {
        if let Some(fonts) = &mut self.fonts {
            for block in blocks {
                if fonts.used_in(block)? {
                    return blocks
                        .iter()
                        .map(|block| fonts.shape(block, &mut self.output, self.viewport[0]))
                        .collect::<Result<Vec<_>, _>>()
                        .map(Contents::FontText);
                }
            }
        }
        Ok(Contents::Text(
            self.output.blocks(blocks, self.viewport[0])?,
        ))
    }

    fn inline_run<'a, 'input>(
        &mut self,
        parent: Node<'a, 'input>,
        run: &[Node<'a, 'input>],
        children: &mut Vec<usize>,
        context: &mut content::Context<'_, '_>,
    ) -> Result<(), EngineError> {
        if let Some(content) = self.text(parent, run, context)? {
            let index = self.add(
                Style {
                    display: Display::Block,
                    ..Default::default()
                },
                PaintBox {
                    id: NodeId::from(0usize),
                    dom: None,
                    children: Vec::new(),
                    content: Some(content),
                    background: None,
                    borders: [None; 4],
                    z_index: 0,
                    clip_overflow: false,
                    invisible: false,
                },
            )?;
            children.push(index);
        }
        Ok(())
    }

    fn add(&mut self, style: Style, mut paint_box: PaintBox) -> Result<usize, EngineError> {
        let index = self.boxes.len();
        if index >= MAX_BOXES {
            return Err(limited("EPUB layout exceeds 8000 boxes"));
        }
        paint_box.id = if paint_box.children.is_empty() {
            self.tree
                .new_leaf_with_context(style, index)
                .map_err(failure)?
        } else {
            let children: Vec<_> = paint_box
                .children
                .iter()
                .map(|index| self.boxes[*index].id)
                .collect();
            self.tree
                .new_with_children(style, &children)
                .map_err(failure)?
        };
        self.boxes.push(paint_box);
        Ok(index)
    }

    fn place(&mut self, root: usize, fixed: bool) -> Result<[f32; 2], EngineError> {
        let mut error = None;
        self.tree
            .compute_layout_with_measure(
                self.boxes[root].id,
                Size {
                    width: AvailableSpace::Definite(self.viewport[0]),
                    height: if fixed {
                        AvailableSpace::Definite(self.viewport[1])
                    } else {
                        AvailableSpace::MaxContent
                    },
                },
                |input, _, index, _| {
                    let size = if error.is_none() {
                        let content = index.and_then(|index| self.boxes[*index].content.as_ref());
                        match measure(
                            content,
                            input,
                            &mut self.output.document,
                            &mut self.cache,
                            &mut self.measure_count,
                        ) {
                            Ok(value) => value,
                            Err(err) => {
                                error = Some(err);
                                Size::ZERO
                            }
                        }
                    } else {
                        Size::ZERO
                    };
                    LayoutOutput::from_outer_size(size)
                },
            )
            .map_err(failure)?;
        if let Some(error) = error {
            return Err(error);
        }
        let root = self.tree.layout(self.boxes[root].id).map_err(failure)?;
        let height = if fixed {
            self.viewport[1]
        } else {
            root.scroll_height().max(root.size.height).max(1.0)
        };
        if !height.is_finite() || height > MAX_GEOMETRY {
            return Err(limited("EPUB laid-out chapter is too tall"));
        }
        Ok([self.viewport[0], height])
    }

    fn paint(
        &mut self,
        index: usize,
        origin: [f32; 2],
        canvas_source: roxmltree::NodeId,
        positioned_origins: &mut HashMap<NodeId, [f32; 2]>,
        output: &mut Vec<Draw>,
    ) -> Result<(), EngineError> {
        let node = &self.boxes[index];
        let layout = *self.tree.layout(node.id).map_err(failure)?;
        let origin = positioned_origins.remove(&node.id).unwrap_or(origin);
        let [x, y] = [origin[0] + layout.location.x, origin[1] + layout.location.y];
        // Taffy reports out-of-flow locations relative to their containing
        // block. Paint in DOM order but use that block's geometry.
        for &child in self.tree.hoisted_children(node.id).map_err(failure)? {
            positioned_origins.insert(child, [x, y]);
        }
        let rectangle = Rect::new(x, y, layout.size.width, layout.size.height);
        if ![x, y, rectangle.w, rectangle.h]
            .into_iter()
            .all(|n| n.is_finite() && n.abs() <= MAX_GEOMETRY * 4.0)
        {
            return Err(limited("EPUB box geometry exceeds renderer limits"));
        }
        let border = layout.border;
        let padding = layout.padding;
        let content_box = Rect::new(
            x + border.left + padding.left,
            y + border.top + padding.top,
            (rectangle.w - border.left - border.right - padding.left - padding.right).max(0.0),
            (rectangle.h - border.top - border.bottom - padding.top - padding.bottom).max(0.0),
        );
        let mut inner = Vec::new();
        if !node.invisible {
            if node.dom != Some(canvas_source) {
                fill(output, rectangle, node.background);
            }
            for (rect, color) in [
                Rect::new(x, y, rectangle.w, border.top),
                Rect::new(x + rectangle.w - border.right, y, border.right, rectangle.h),
                Rect::new(
                    x,
                    y + rectangle.h - border.bottom,
                    rectangle.w,
                    border.bottom,
                ),
                Rect::new(x, y, border.left, rectangle.h),
            ]
            .into_iter()
            .zip(node.borders)
            {
                fill(output, rect, color);
            }
            match node.content.as_ref() {
                Some(Contents::Text(blocks)) if content_box.w > 0.0 => {
                    let page = layout_text(
                        blocks,
                        content_box.w,
                        &mut self.output.document,
                        &mut self.cache,
                        &mut self.measure_count,
                    )?;
                    let draws = page_display(
                        &self.output.document,
                        &page,
                        &wordcraft_layout::display::DisplayOptions::default(),
                    );
                    inner.push(Draw::Turned {
                        x: content_box.x,
                        y: content_box.y,
                        turn: TextDirection::Horizontal,
                        items: draws,
                    });
                }
                Some(Contents::FontText(blocks)) if content_box.w > 0.0 => {
                    let mut area = content_box;
                    for block in blocks {
                        area.y += block.draw(area, &mut self.glyph_paths_remaining, &mut inner)?;
                    }
                }
                Some(Contents::Image(image, alt)) if content_box.w > 0.0 && content_box.h > 0.0 => {
                    inner.push(Draw::Image {
                        rect: content_box,
                        media: image.media(&mut self.output, content_box)?,
                        crop: [0.0; 4],
                        alpha: 1.0,
                        alt: alt.clone(),
                        decorative: false,
                    });
                }
                _ => {}
            }
        }
        let clip = node.clip_overflow;
        let mut children = node.children.clone();
        children.sort_by_key(|&child| self.boxes[child].z_index);
        for child in children {
            self.paint(child, [x, y], canvas_source, positioned_origins, &mut inner)?;
        }
        if clip && !inner.is_empty() {
            let rect = Rect::new(
                x + border.left,
                y + border.top,
                (rectangle.w - border.left - border.right).max(0.0),
                (rectangle.h - border.top - border.bottom).max(0.0),
            );
            output.push(Draw::Clip { rect, items: inner });
        } else {
            output.extend(inner);
        }
        Ok(())
    }
}

fn measure(
    content: Option<&Contents>,
    input: LayoutInput,
    doc: &mut Document,
    cache: &mut LayoutCache,
    work: &mut usize,
) -> Result<Size<f32>, EngineError> {
    let known = input.known_dimensions;
    if let (Some(width), Some(height)) = (known.width, known.height) {
        return Ok(Size { width, height });
    }
    let natural = match content {
        Some(Contents::Image(image, _)) => {
            let [mut width, mut height] = image.size().map(|n| n * 0.75);
            if let Some(w) = known.width {
                height *= w / width;
                width = w;
            } else if let Some(h) = known.height {
                width *= h / height;
                height = h;
            }
            Size { width, height }
        }
        Some(Contents::FontText(blocks)) => {
            super::charge(
                work,
                blocks.len(),
                16_000,
                "EPUB text layout work limit exceeded",
            )?;
            let width = available_text_width(input);
            let mut total = Size::<f32>::ZERO;
            for block in blocks {
                let [w, h] = block.size(width);
                total.width = total.width.max(w);
                total.height += h;
            }
            total
        }
        Some(Contents::Text(blocks)) => {
            let width = available_text_width(input);
            let page = layout_text(blocks, width, doc, cache, work)?;
            text_size(&page)
        }
        None => Size::ZERO,
    };
    Ok(Size {
        width: known.width.unwrap_or(natural.width),
        height: known.height.unwrap_or(natural.height),
    })
}

fn available_text_width(input: LayoutInput) -> f32 {
    input
        .known_dimensions
        .width
        .unwrap_or(match input.available_space.width {
            AvailableSpace::Definite(width) => width,
            AvailableSpace::MinContent => 2.0,
            AvailableSpace::MaxContent => MAX_GEOMETRY,
        })
}

fn layout_text(
    blocks: &Blocks,
    width: f32,
    doc: &mut Document,
    cache: &mut LayoutCache,
    work: &mut usize,
) -> Result<Page, EngineError> {
    super::charge(
        work,
        blocks.len().max(1),
        16_000,
        "EPUB text layout work limit exceeded",
    )?;
    doc.body.clone_from(blocks);
    doc.last_section = wordcraft_doc::SectionProps {
        page_w: width.clamp(2.0, MAX_GEOMETRY),
        page_h: MAX_GEOMETRY,
        margin_left: 0.0,
        margin_top: 0.0,
        margin_right: 0.0,
        margin_bottom: 0.0,
        gutter: 0.0,
        ..Default::default()
    };
    let result = wordcraft_layout::layout(
        doc,
        cache,
        &LayoutOptions {
            view: ViewMode::Print,
            ..Default::default()
        },
    );
    cache.clear();
    if result.pages.len() != 1 {
        return Err(limited("EPUB text box exceeds native page height"));
    }
    result
        .pages
        .into_iter()
        .next()
        .ok_or_else(|| invalid("empty EPUB text layout"))
}

fn text_size(page: &Page) -> Size<f32> {
    let mut size = Size::ZERO;
    for entry in &page.items {
        match entry {
            Placed::Lines {
                x: _,
                y,
                para,
                l0,
                l1,
                ..
            } => {
                let lines = &para.lines[*l0..*l1];
                if let (Some(first), Some(last)) = (lines.first(), lines.last()) {
                    size.height = size.height.max(y + last.top - first.top + last.height);
                    for line in lines {
                        if let (Some(left), Some(right)) = (line.xs.first(), line.xs.last()) {
                            size.width = size.width.max((right - left).abs());
                        }
                    }
                }
            }
            Placed::Cell { rect, .. }
            | Placed::Image { rect, .. }
            | Placed::Object { rect, .. } => {
                size.height = size.height.max(rect.y + rect.h);
                size.width = size.width.max(rect.x + rect.w);
            }
            _ => {}
        }
    }
    size
}

fn fill(output: &mut Vec<Draw>, rect: Rect, color: Option<Paint>) {
    if let Some((color, alpha)) =
        color.filter(|(_, alpha)| *alpha > 0.0 && rect.w > 0.0 && rect.h > 0.0)
    {
        output.push(Draw::Fill {
            rect,
            color,
            alpha,
            text_bg: false,
        });
    }
}

fn raster(
    doc: &Document,
    drawing: &[Draw],
    dimensions: [f32; 2],
    dpi: f64,
    region: Region,
) -> Result<wordcraft_render::Rendered, EngineError> {
    let scale = dpi / 72.0;
    let region = region.for_page(dimensions.map(f64::from), dpi)?;
    // render_items uses its area only to determine dimensions and transform. Choose
    // dimensions inside the quantized pixel interval to absorb its f32 rounding.
    let area = Rect::new(
        (f64::from(region.x) / scale) as f32,
        (f64::from(region.y) / scale) as f32,
        ((f64::from(region.width) - 0.125) / scale) as f32,
        ((f64::from(region.height) - 0.125) / scale) as f32,
    );
    let rendered = wordcraft_render::render_items(
        doc,
        drawing,
        area,
        scale as f32,
        &wordcraft_render::RenderOptions::default(),
    );
    region.validate_result(rendered.width, rendered.height)?;
    Ok(rendered)
}

fn failure(error: impl std::fmt::Display) -> EngineError {
    EngineError::Conversion(format!("EPUB layout failed: {error}"))
}
