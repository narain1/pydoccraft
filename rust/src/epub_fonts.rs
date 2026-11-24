//! Shape EPUB text with scoped archive fonts and bundled fallback faces.
use std::{cell::RefCell, collections::HashSet, ops::Range, sync::Arc};

use kurbo::BezPath;
use parley::{
    fontique::{Blob, FontInfoOverride},
    Alignment, AlignmentOptions, BaseDirection, FontContext, FontFamily, FontStyle, FontWeight,
    InlineBox, InlineBoxKind, Layout, LayoutContext, LineHeight, PositionedLayoutItem,
    StyleProperty,
};
use skrifa::{
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
    FontRef, GlyphId, MetadataProvider,
};
use wordcraft_doc::{
    para::InlineObject,
    props::{Align, CharProps, LineSpacing, Rgb, TextColor, TextDirection, Underline},
};
use wordcraft_geom::Rect;
use wordcraft_layout::{
    display::{Draw, Stroke},
    math::{self, MItem},
};

use super::{
    charge, external, invalid, limited,
    output::{inline_items, CssParagraph, Inline, Output, StyledText},
    resolve,
    style::Styles,
    unsupported, Book,
};
use crate::{model::BlockPayload, EngineError};

const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
type Brush = [u8; 4];

pub(super) struct Face {
    pub family: String,
    pub base: String,
    pub urls: Vec<String>,
    pub weight: Option<f32>,
    pub style: Option<FontStyle>,
    pub unicode_range: bool,
}

pub(super) struct Fonts {
    names: HashSet<String>,
    fonts: FontContext,
    layout: LayoutContext<Brush>,
    shaping_bytes: usize,
}

impl Fonts {
    pub(super) fn load(
        faces: &[Face],
        styles: &Styles,
        book: &mut Book<'_>,
    ) -> Result<Option<Self>, EngineError> {
        if faces.is_empty() {
            return Ok(None);
        }
        let used: HashSet<_> = styles
            .values()
            .filter(|style| !style.hidden)
            .filter_map(|style| {
                style
                    .text
                    .chars
                    .font
                    .as_deref()
                    .map(str::to_ascii_lowercase)
            })
            .collect();
        let mut result = Self {
            names: HashSet::new(),
            fonts: FontContext::new(),
            layout: LayoutContext::new(),
            shaping_bytes: 0,
        };
        let mut budget = 0;
        for face in faces {
            let key = face.family.to_ascii_lowercase();
            if !used.contains(&key) {
                continue;
            }
            let (Some(weight), Some(style)) = (face.weight, face.style) else {
                return Err(unsupported("EPUB font-face weight and style ranges"));
            };
            if face.unicode_range {
                return Err(unsupported("EPUB font-face unicode-range"));
            }
            let data = read_face(face, book)?;
            charge(
                &mut budget,
                data.len(),
                16 * 1024 * 1024,
                "EPUB decoded fonts exceed 16 MiB",
            )?;
            let registrations = result.fonts.collection.register_fonts(
                Blob::new(Arc::new(data)),
                Some(FontInfoOverride {
                    family_name: Some(&face.family),
                    weight: Some(FontWeight::new(weight)),
                    style: Some(style),
                    ..Default::default()
                }),
            );
            if registrations.is_empty() {
                return Err(invalid("EPUB font-face has no usable font"));
            }
            result.names.insert(key);
        }
        if result.names.is_empty() {
            return Ok(None);
        }
        for &data in wordcraft_fonts::bundled() {
            result
                .fonts
                .collection
                .register_fonts(Blob::new(Arc::new(data)), None);
        }
        Ok(Some(result))
    }

    pub(super) fn used_in(&self, block: &BlockPayload) -> Result<bool, EngineError> {
        if let Some(table) = &block.table {
            for cell in table.cells.iter().flatten() {
                for block in &cell.blocks {
                    if self.used_in(block)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(StyledText::new(block)?.spans.iter().any(|(_, _, style)| {
            style
                .font
                .as_deref()
                .is_some_and(|name| self.names.contains(&name.to_ascii_lowercase()))
        }))
    }

    pub(super) fn shape(
        &mut self,
        block: &BlockPayload,
        output: &mut Output,
        width: f32,
    ) -> Result<Text, EngineError> {
        if block.table.is_some() {
            return Err(unsupported("packaged fonts inside EPUB tables"));
        }
        let source = StyledText::new(block)?;
        charge(
            &mut self.shaping_bytes,
            source.value.len(),
            512 * 1024,
            "EPUB packaged-font shaping exceeds 512 KiB",
        )?;
        let mut inline = Vec::new();
        let mut text = String::with_capacity(source.value.len());
        let mut spans = Vec::new();
        let mut cursor = 0;
        for (start, item) in inline_items(block, false)? {
            if start < cursor || start >= source.offsets.len() {
                return Err(invalid("overlapping EPUB inline objects"));
            }
            append_span(&source, cursor..start, &mut text, &mut spans);
            let (end, drawing) = match item {
                Inline::Image(image) => {
                    let picture = output.load_image(image)?;
                    let scale = (width / picture.width)
                        .min(600.0 / picture.height)
                        .min(0.75);
                    let width = picture.width * scale;
                    let height = picture.height * scale;
                    (
                        start,
                        Fragment {
                            width,
                            height,
                            baseline: None,
                            items: vec![Draw::Image {
                                rect: Rect::new(0.0, 0.0, width, height),
                                media: picture.media,
                                crop: [0.0; 4],
                                alpha: 1.0,
                                alt: image.alt.clone(),
                                decorative: false,
                            }],
                        },
                    )
                }
                Inline::Equation {
                    end,
                    source: markup,
                } => {
                    if end < start || end >= source.offsets.len() {
                        return Err(invalid("invalid EPUB equation text range"));
                    }
                    (
                        end,
                        equation(markup, source.style_at(start), output, width)?,
                    )
                }
                Inline::Link { .. } => {
                    return Err(invalid("unexpected EPUB preview link replacement"))
                }
            };
            let index = text.len();
            text.push('\u{fffc}');
            inline.push((index, drawing));
            cursor = end;
        }
        append_span(
            &source,
            cursor..source.offsets.len() - 1,
            &mut text,
            &mut spans,
        );
        let properties = CssParagraph::from_block(block)?;
        let mut builder = self
            .layout
            .ranged_builder(&mut self.fonts, &text, 1.0, false);
        builder.push_default(StyleProperty::FontSize(12.0));
        builder.push_default(StyleProperty::FontFamily(FontFamily::Source(
            "Source Sans 3".into(),
        )));
        builder.push_default(StyleProperty::Brush([0, 0, 0, 255]));
        let mut align = Alignment::Start;
        if let Some(properties) = properties {
            if let Some(rtl) = properties.bidi {
                builder.set_base_direction(if rtl {
                    BaseDirection::Rtl
                } else {
                    BaseDirection::Ltr
                });
            }
            if let Some(line) = properties.line_height {
                builder.push_default(StyleProperty::LineHeight(match line {
                    LineSpacing::Multiple(n) => LineHeight::FontSizeRelative(n),
                    LineSpacing::Exactly(n) | LineSpacing::AtLeast(n) => LineHeight::Absolute(n),
                }));
            }
            align = match properties.align.unwrap_or_default() {
                Align::Left => Alignment::Start,
                Align::Center => Alignment::Center,
                Align::Right => Alignment::End,
                Align::Justify | Align::Distribute => Alignment::Justify,
            };
        }
        for (range, style) in spans {
            if let Some(name) = style.font.as_deref() {
                let family = if self.names.contains(&name.to_ascii_lowercase()) {
                    name
                } else {
                    match name {
                        "Arial" => "Source Sans 3",
                        "Times New Roman" => "Source Serif 4",
                        _ => name,
                    }
                };
                builder.push(
                    StyleProperty::FontFamily(FontFamily::Source(family.into())),
                    range.clone(),
                );
            }
            if let Some(size) = style.size {
                builder.push(StyleProperty::FontSize(size), range.clone());
            }
            if style.bold == Some(true) {
                builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
            }
            if style.italic == Some(true) {
                builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
            }
            if let Some(TextColor::Rgb(Rgb(r, g, b))) = style.color {
                builder.push(StyleProperty::Brush([r, g, b, 255]), range.clone());
            }
            if let Some(spacing) = style.spacing {
                builder.push(StyleProperty::LetterSpacing(spacing), range.clone());
            }
            if style.strike == Some(true) {
                builder.push(StyleProperty::Strikethrough(true), range.clone());
            }
            if style.underline.is_some_and(|line| line != Underline::None) {
                builder.push(StyleProperty::Underline(true), range);
            }
        }
        for (id, (index, fragment)) in inline.iter().enumerate() {
            builder.push_inline_box(InlineBox {
                id: id as u64,
                kind: InlineBoxKind::InFlow,
                index: *index,
                width: fragment.width,
                height: fragment.height,
                baseline: fragment.baseline,
                vertical_align: Default::default(),
            });
        }
        let layout = builder.build(&text);
        Ok(Text {
            layout: RefCell::new(layout),
            align,
            inline: inline
                .into_iter()
                .map(|(_, drawing)| drawing.items)
                .collect(),
        })
    }
}

struct Fragment {
    width: f32,
    height: f32,
    baseline: Option<f32>,
    items: Vec<Draw>,
}

fn append_span<'a>(
    source: &'a StyledText<'_>,
    range: Range<usize>,
    text: &mut String,
    spans: &mut Vec<(Range<usize>, &'a CharProps)>,
) {
    if range.is_empty() {
        return;
    }
    let base = text.len();
    let offset = source.offsets[range.start];
    text.push_str(&source.value[offset..source.offsets[range.end]]);
    let first = source
        .spans
        .partition_point(|(_, end, _)| *end <= range.start);
    for (start, end, style) in &source.spans[first..] {
        if *start >= range.end {
            break;
        }
        let start = base + source.offsets[(*start).max(range.start)] - offset;
        let end = base + source.offsets[(*end).min(range.end)] - offset;
        spans.push((start..end, style));
    }
}

fn equation(
    markup: &str,
    mut style: CharProps,
    output: &mut Output,
    width: f32,
) -> Result<Fragment, EngineError> {
    let InlineObject::Equation {
        linear,
        display,
        math: equation,
    } = super::math::equation(markup, &mut output.math_nodes_remaining)?
    else {
        return Err(invalid("invalid EPUB equation"));
    };
    style.font = None;
    let style = output.document.styles.resolve_char(None, &style);
    let layout = math::layout_equation(
        &equation,
        &linear,
        &style,
        display,
        width,
        &Default::default(),
        &mut 0,
    );
    let base = layout.ascent;
    let mut items = Vec::new();
    for item in layout.items {
        match item {
            MItem::Glyphs {
                face,
                size,
                color,
                synth_bold,
                synth_italic,
                glyphs,
                text,
            } => items.push(Draw::Glyphs {
                face,
                size,
                color,
                synth_bold,
                synth_italic,
                text,
                glyphs: glyphs
                    .into_iter()
                    .map(|(id, x, y)| (id, x, base - y))
                    .collect(),
                alpha: 1.0,
                link: None,
                ranges: Vec::new(),
                text_bg: None,
                x_scale: 1.0,
            }),
            MItem::Rect { x, y, w, h, color } => items.push(Draw::Fill {
                rect: Rect::new(x, base - y - h, w, h),
                color,
                alpha: 1.0,
                text_bg: false,
            }),
            MItem::Line {
                x0,
                y0,
                x1,
                y1,
                width,
                color,
                dotted,
            } => items.push(Draw::Line {
                x0,
                y0: base - y0,
                x1,
                y1: base - y1,
                width,
                color,
                alpha: 1.0,
                stroke: if dotted {
                    Stroke::Dotted
                } else {
                    Stroke::Solid
                },
                text_bg: None,
            }),
            MItem::ScreenOnly(_) => {}
        }
    }
    Ok(Fragment {
        width: layout.width,
        height: base + layout.descent,
        baseline: Some(base),
        items,
    })
}

fn read_face(face: &Face, book: &mut Book<'_>) -> Result<Vec<u8>, EngineError> {
    let mut failure = None;
    for url in &face.urls {
        if external(url) || url.contains('#') {
            continue;
        }
        let path = resolve(&face.base, url)?;
        if book.archive.index_for_name(&path).is_none() {
            failure = Some(invalid("EPUB font-face source is absent from the archive"));
            continue;
        }
        let bytes = book.read_font(&path, MAX_FONT_BYTES)?;
        match wordcraft_fonts::decode_font_file(&bytes) {
            Ok(data) if data.len() <= MAX_FONT_BYTES => return Ok(data),
            Ok(_) => return Err(limited("EPUB decoded font exceeds 8 MiB")),
            Err(err) => failure = Some(invalid(err)),
        }
    }
    Err(failure.unwrap_or_else(|| unsupported("EPUB font-face requires a supported local source")))
}

pub(super) struct Text {
    layout: RefCell<Layout<Brush>>,
    align: Alignment,
    inline: Vec<Vec<Draw>>,
}

impl Text {
    pub(super) fn size(&self, width: f32) -> [f32; 2] {
        let mut layout = self.layout.borrow_mut();
        layout.break_all_lines(Some(width.max(2.0)));
        [layout.width(), layout.height()]
    }

    pub(super) fn draw(
        &self,
        area: Rect,
        remaining: &mut usize,
        output: &mut Vec<Draw>,
    ) -> Result<f32, EngineError> {
        self.size(area.w);
        let mut layout = self.layout.borrow_mut();
        layout.set_alignment(self.align, AlignmentOptions::default());
        for line in layout.lines() {
            for item in line.items() {
                let run = match item {
                    PositionedLayoutItem::InlineBox(placed) => {
                        let items = self
                            .inline
                            .get(placed.id as usize)
                            .ok_or_else(|| invalid("missing EPUB inline object"))?;
                        output.push(Draw::Turned {
                            x: area.x + placed.x,
                            y: area.y + placed.y,
                            turn: TextDirection::Horizontal,
                            items: items.clone(),
                        });
                        continue;
                    }
                    PositionedLayoutItem::GlyphRun(run) => run,
                };
                let data = run.run().font();
                let font = FontRef::from_index(data.data.data(), data.index).map_err(invalid)?;
                let outlines = font.outline_glyphs();
                let size = run.run().font_size();
                let coordinates: Vec<_> = run
                    .run()
                    .normalized_coords()
                    .iter()
                    .map(|coordinate| NormalizedCoord::from_bits(coordinate.to_bits()))
                    .collect();
                let synthesis = run.run().synthesis();
                let skew = synthesis.skew().unwrap_or_default().to_radians().tan();
                let style = run.style();
                let [red, green, blue, alpha] = style.brush;
                let mut pen = Pen {
                    path: BezPath::new(),
                    x: 0.0,
                    y: 0.0,
                    skew,
                    remaining,
                    truncated: false,
                };
                for glyph in run.positioned_glyphs() {
                    let Some(outline) = outlines.get(GlyphId::new(glyph.id)) else {
                        continue;
                    };
                    pen.x = area.x + glyph.x;
                    pen.y = area.y + glyph.y;
                    let settings =
                        DrawSettings::unhinted(Size::new(size), LocationRef::new(&coordinates));
                    outline.draw(settings, &mut pen).map_err(invalid)?;
                    if pen.truncated {
                        return Err(limited("EPUB packaged-font outline budget exceeded"));
                    }
                }
                let color = Rgb(red, green, blue);
                let alpha = f32::from(alpha) / 255.0;
                if !pen.path.is_empty() {
                    output.push(Draw::Art {
                        path: pen.path,
                        fx: Box::new(wordcraft_doc::wordart::TextEffects {
                            outline: synthesis.embolden().then_some(
                                wordcraft_doc::wordart::TextOutline {
                                    color: Some(color),
                                    width: size / 30.0,
                                    transparency: 0.0,
                                },
                            ),
                            ..Default::default()
                        }),
                        color,
                        alpha,
                        text_bg: None,
                    });
                }
                let metrics = run.run().font_metrics();
                for (decoration, default_offset, default_size) in [
                    (
                        style.underline.as_ref(),
                        metrics.underline_offset,
                        metrics.underline_size,
                    ),
                    (
                        style.strikethrough.as_ref(),
                        metrics.strikethrough_offset,
                        metrics.strikethrough_size,
                    ),
                ] {
                    if let Some(decoration) = decoration {
                        let thickness = decoration.size.unwrap_or(default_size).max(0.25);
                        let y =
                            area.y + run.baseline() - decoration.offset.unwrap_or(default_offset);
                        output.push(Draw::Fill {
                            rect: Rect::new(area.x + run.offset(), y, run.advance(), thickness),
                            color,
                            alpha,
                            text_bg: false,
                        });
                    }
                }
            }
        }
        Ok(layout.height())
    }
}

struct Pen<'a> {
    path: BezPath,
    x: f32,
    y: f32,
    skew: f32,
    remaining: &'a mut usize,
    truncated: bool,
}

impl Pen<'_> {
    fn point(&self, x: f32, y: f32) -> (f64, f64) {
        (f64::from(self.x + x + self.skew * y), f64::from(self.y - y))
    }
    fn charge(&mut self) -> bool {
        if *self.remaining == 0 {
            self.truncated = true;
            false
        } else {
            *self.remaining -= 1;
            true
        }
    }
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.charge() {
            self.path.move_to(self.point(x, y));
        }
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if self.charge() {
            self.path.line_to(self.point(x, y));
        }
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if self.charge() {
            self.path.quad_to(self.point(cx, cy), self.point(x, y));
        }
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        if self.charge() {
            self.path
                .curve_to(self.point(cx0, cy0), self.point(cx1, cy1), self.point(x, y));
        }
    }
    fn close(&mut self) {
        if self.charge() {
            self.path.close_path();
        }
    }
}
