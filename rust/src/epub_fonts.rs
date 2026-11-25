//! Shape EPUB text with scoped archive fonts and bundled fallback faces.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap},
    ops::Range,
    sync::Arc,
};

use kurbo::BezPath;
use parley::{
    fontique::{Blob, FontInfoOverride, GenericFamily},
    setting::{FontVariation, Tag},
    Alignment, AlignmentOptions, BaseDirection, FontContext, FontFamily, FontFamilyName, FontStyle,
    FontVariations, FontWeight, FontWidth, InlineBox, InlineBoxKind, Layout, LayoutContext,
    LineHeight, PositionedLayoutItem, StyleProperty,
};
use skrifa::{
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{DrawSettings, OutlinePen},
    FontRef, GlyphId, MetadataProvider,
};
use wordcraft_doc::props::{
    Align, CharProps, LineSpacing, Rgb, TextColor, TextDirection, Underline,
};
use wordcraft_geom::Rect;
use wordcraft_layout::{
    display::{Draw, Stroke},
    math::{self, MItem, MathLayout},
};

use super::{
    charge, external, invalid, limited,
    math::Equation,
    output::{has_composed_math, inline_items, CssParagraph, Inline, Output, RunStyle, StyledText},
    resolve,
    style::Styles,
    unsupported, Book,
};
use crate::{model::BlockPayload, EngineError};

#[path = "epub_font_ranges.rs"]
pub(super) mod ranges;

pub(super) const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
const MAX_REGISTERED_FACES: usize = 512;
const MAX_VARIATION_SPAN_BYTES: usize = 8 * 1024 * 1024;
type Brush = [u8; 4];
type FontRequest = [u32; 2];
type FontNames = HashMap<String, HashMap<Option<FontRequest>, String>>;

pub(super) struct Face {
    pub family: String,
    pub base: String,
    pub urls: Vec<String>,
    pub weight: Option<[f32; 2]>,
    pub width: Option<[f32; 2]>,
    pub style: Option<FontStyle>,
    pub unicode_ranges: Option<Vec<(u32, u32)>>,
    pub axes: Arc<[super::css::Axis]>,
}

struct FaceVariations {
    weight: Option<[f32; 2]>,
    width: Option<[f32; 2]>,
    axes: Arc<[super::css::Axis]>,
}

pub(super) struct Fonts {
    names: FontNames,
    variations: HashMap<u64, FaceVariations>,
    fonts: FontContext,
    layout: LayoutContext<Brush>,
    shaping_bytes: usize,
}

impl Fonts {
    fn empty() -> Self {
        Self {
            names: HashMap::new(),
            variations: HashMap::new(),
            fonts: FontContext::new(),
            layout: LayoutContext::new(),
            shaping_bytes: 0,
        }
    }

    pub(super) fn bundled() -> Self {
        let mut fonts = Self::empty();
        fonts.register_bundled();
        fonts
    }

    fn register_bundled(&mut self) {
        for &data in wordcraft_fonts::bundled() {
            self.fonts
                .collection
                .register_fonts(Blob::new(Arc::new(data)), None);
        }
    }

    pub(super) fn load(
        faces: &[Face],
        styles: &Styles,
        book: &mut Book<'_>,
    ) -> Result<Option<Self>, EngineError> {
        let mut declared: HashMap<String, Vec<&Face>> = HashMap::new();
        for face in faces {
            declared
                .entry(face.family.to_ascii_lowercase())
                .or_default()
                .push(face);
        }
        let mut used: HashMap<String, BTreeSet<FontRequest>> = HashMap::new();
        let mut needs_stack = false;
        for style in styles.values().filter(|style| !style.hidden) {
            let families = &style.text.families;
            let weight = style.text.font_weight.unwrap_or(400.0);
            let width = style.text.font_width.unwrap_or(100.0);
            needs_stack |= families.len() > 1 || unusual_weight(weight) || width != 100.0;
            let mut record = |name: &str| {
                let key = name.to_ascii_lowercase();
                if declared.contains_key(&key) {
                    used.entry(key)
                        .or_default()
                        .insert([weight.to_bits(), width.to_bits()]);
                }
            };
            if families.is_empty() {
                if let Some(name) = &style.text.chars.font {
                    record(name);
                }
            } else {
                for family in families.iter().filter(|family| !family.generic) {
                    record(&family.name);
                }
            }
        }
        if faces.is_empty() {
            return Ok(needs_stack.then(Self::bundled));
        }
        let mut result = Self::empty();
        let mut budget = 0;
        let mut mapping_work = 0;
        let mut rewritten_bytes = 0;
        let mut registered_faces = 0;
        let mut families = 0;
        let mut matched_widths = HashMap::new();
        for (name, requests) in &used {
            let group = &declared[name];
            if group.iter().any(|face| {
                face.width.is_some() || face.weight.is_some_and(|[low, high]| low != high)
            }) {
                let mut widths = HashMap::new();
                for [_, width] in requests {
                    widths
                        .entry(*width)
                        .or_insert_with(|| match_width(group, f32::from_bits(*width)));
                }
                matched_widths.insert(name.as_str(), widths);
            }
        }
        // Fontique tries matching faces in registration order. CSS composite faces
        // try the last declaration first when multiple faces cover a character.
        for face in faces.iter().rev() {
            let key = face.family.to_ascii_lowercase();
            let Some(requests) = used.get(&key) else {
                continue;
            };
            let choices: Vec<_> = if let Some(widths) = matched_widths.get(key.as_str()) {
                let [low, high] = face.width.unwrap_or([100.0; 2]);
                requests
                    .iter()
                    .copied()
                    .filter(|[_, width]| f32::from_bits(*width).clamp(low, high) == widths[width])
                    .map(Some)
                    .collect()
            } else {
                vec![None]
            };
            if choices.is_empty() {
                continue;
            }
            let style = face
                .style
                .ok_or_else(|| unsupported("EPUB font-face style range"))?;
            let interval = face.weight.unwrap_or([400.0; 2]);
            let data = read_face(face, book)?;
            charge(
                &mut budget,
                data.len(),
                16 * 1024 * 1024,
                "EPUB decoded fonts exceed 16 MiB",
            )?;
            // Keep author families separate from bundled names so a glyph outside
            // an author face's range can reach the next family on the CSS list.
            let mut register = |data: Vec<u8>| {
                let blob = Blob::new(Arc::new(data));
                if face.weight.is_some() || face.width.is_some() || !face.axes.is_empty() {
                    result.variations.insert(
                        blob.id(),
                        FaceVariations {
                            weight: face.weight,
                            width: face.width,
                            axes: face.axes.clone(),
                        },
                    );
                }
                let names = result.names.entry(key.clone()).or_default();
                for &choice in &choices {
                    charge(
                        &mut registered_faces,
                        1,
                        MAX_REGISTERED_FACES,
                        "EPUB font-face selection exceeds 512 registered variants",
                    )?;
                    let alias = names.entry(choice).or_insert_with(|| {
                        families += 1;
                        format!("\0EPUB font family {families}")
                    });
                    let [weight, width] =
                        choice.map_or([interval[0], 100.0], |axes| axes.map(f32::from_bits));
                    let nominal = weight.clamp(interval[0], interval[1]);
                    let overrides = FontInfoOverride {
                        family_name: Some(alias),
                        weight: Some(FontWeight::new(nominal)),
                        width: Some(FontWidth::from_percentage(width)),
                        style: Some(style),
                        axes: None,
                    };
                    let registrations = result
                        .fonts
                        .collection
                        .register_fonts(blob.clone(), Some(overrides));
                    if registrations.is_empty() {
                        return Err(invalid("EPUB font-face has no usable font"));
                    }
                    charge(
                        &mut registered_faces,
                        registrations
                            .iter()
                            .map(|(_, fonts)| fonts.len())
                            .sum::<usize>()
                            .saturating_sub(1),
                        MAX_REGISTERED_FACES,
                        "EPUB font-face selection exceeds 512 registered variants",
                    )?;
                }
                Ok(())
            };
            match face.unicode_ranges.as_deref() {
                None | Some([(0, 0x10ffff)]) => register(data)?,
                Some(ranges) => ranges::register(&data, ranges, &mut mapping_work, |data| {
                    charge(
                        &mut rewritten_bytes,
                        data.len(),
                        16 * 1024 * 1024,
                        "EPUB Unicode-restricted fonts exceed 16 MiB",
                    )?;
                    register(data)
                })?,
            }
        }
        if result.names.is_empty() && !needs_stack {
            return Ok(None);
        }
        result.register_bundled();
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
        if has_composed_math(block) {
            return Ok(true);
        }
        Ok(StyledText::new(block)?.spans.iter().any(|(_, _, style)| {
            if unusual_weight(run_weight(style)) || run_width(style) != 100.0 {
                true
            } else if style.families.is_empty() {
                style
                    .chars
                    .font
                    .as_deref()
                    .is_some_and(|name| self.names.contains_key(&name.to_ascii_lowercase()))
            } else {
                style.families.len() > 1
                    || style.families.iter().any(|family| {
                        !family.generic
                            && self.names.contains_key(&family.name.to_ascii_lowercase())
                    })
            }
        }))
    }

    pub(super) fn shape(
        &mut self,
        block: &BlockPayload,
        output: &mut Output<'_>,
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
                    css,
                } => {
                    if end < start || end >= source.offsets.len() {
                        return Err(invalid("invalid EPUB equation text range"));
                    }
                    (
                        end,
                        equation(markup, css, source.style_at(start), output, width)?,
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
        let mut build = |overrides: Vec<(Range<usize>, Vec<FontVariation>)>| {
            let mut builder = self
                .layout
                .ranged_builder(&mut self.fonts, &text, 1.0, false);
            builder.push_default(StyleProperty::FontSize(12.0));
            builder.push_default(StyleProperty::FontFamily(FontFamily::Source(
                "Source Sans 3".into(),
            )));
            builder.push_default(StyleProperty::Brush([0, 0, 0, 255]));
            let mut align = Alignment::Start;
            if let Some(properties) = &properties {
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
                        LineSpacing::Exactly(n) | LineSpacing::AtLeast(n) => {
                            LineHeight::Absolute(n)
                        }
                    }));
                }
                align = match properties.align.unwrap_or_default() {
                    Align::Left => Alignment::Start,
                    Align::Center => Alignment::Center,
                    Align::Right => Alignment::End,
                    Align::Justify | Align::Distribute => Alignment::Justify,
                };
            }
            for (range, run) in &spans {
                let style = &run.chars;
                let weight = run_weight(run);
                let width = run_width(run);
                let request = [weight.to_bits(), width.to_bits()];
                let mut families: Vec<_> = run
                    .families
                    .iter()
                    .map(|family| {
                        if family.generic {
                            match family.name.as_str() {
                                "serif" | "ui-serif" => FontFamilyName::named("Source Serif 4"),
                                "sans-serif" | "system-ui" | "ui-sans-serif" => {
                                    FontFamilyName::named("Source Sans 3")
                                }
                                "monospace" | "ui-monospace" => {
                                    FontFamilyName::named("JetBrains Mono")
                                }
                                name => GenericFamily::parse(name).map_or_else(
                                    || FontFamilyName::named(name),
                                    FontFamilyName::Generic,
                                ),
                            }
                        } else {
                            named_family(&self.names, &family.name, request)
                        }
                    })
                    .collect();
                if families.is_empty() {
                    if let Some(name) = style.font.as_deref() {
                        families.push(named_family(&self.names, name, request));
                    }
                }
                if !families.is_empty() {
                    families.push(FontFamilyName::named("Source Sans 3"));
                    builder.push(
                        StyleProperty::FontFamily(FontFamily::List(families.into())),
                        range.clone(),
                    );
                }
                let variations = variations(run, None);
                if !variations.is_empty() {
                    builder.push(
                        StyleProperty::FontVariations(FontVariations::List(variations.into())),
                        range.clone(),
                    );
                }
                if let Some(size) = style.size {
                    builder.push(StyleProperty::FontSize(size), range.clone());
                }
                if weight != 400.0 {
                    builder.push(
                        StyleProperty::FontWeight(FontWeight::new(weight)),
                        range.clone(),
                    );
                }
                if width != 100.0 {
                    builder.push(
                        StyleProperty::FontWidth(FontWidth::from_percentage(width)),
                        range.clone(),
                    );
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
                    builder.push(StyleProperty::Underline(true), range.clone());
                }
            }
            for (range, settings) in overrides {
                builder.push(
                    StyleProperty::FontVariations(FontVariations::List(settings.into())),
                    range,
                );
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
            (builder.build(&text), align)
        };
        let (mut layout, align) = build(Vec::new());
        if !self.variations.is_empty() {
            // Defaults are properties of the face selected for these glyphs.
            // Ask the native shaper to resolve fallback before applying them.
            layout.break_all_lines(None);
            let mut overrides = Vec::new();
            let mut variation_bytes = 0;
            for line in layout.lines() {
                for run in line.runs() {
                    let Some(face) = self.variations.get(&run.font().data.id()) else {
                        continue;
                    };
                    let range = run.text_range();
                    let first = spans.partition_point(|(span, _)| span.end <= range.start);
                    for (span, author) in &spans[first..] {
                        if span.start >= range.end {
                            break;
                        }
                        let overlap = span.start.max(range.start)..span.end.min(range.end);
                        if !overlap.is_empty() {
                            charge(
                                &mut variation_bytes,
                                64 + (face.axes.len() + author.axes.len()).saturating_mul(8),
                                MAX_VARIATION_SPAN_BYTES,
                                "EPUB face variation spans exceed 8 MiB",
                            )?;
                            overrides.push((overlap, variations(author, Some(face))));
                        }
                    }
                }
            }
            if !overrides.is_empty() {
                layout = build(overrides).0;
            }
        }
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

fn run_weight(run: &RunStyle) -> f32 {
    run.font_weight.unwrap_or(if run.chars.bold == Some(true) {
        700.0
    } else {
        400.0
    })
}

fn unusual_weight(weight: f32) -> bool {
    weight != 400.0 && weight != 700.0
}

fn run_width(run: &RunStyle) -> f32 {
    run.font_width.unwrap_or(100.0)
}

fn match_width(faces: &[&Face], requested: f32) -> f32 {
    let rank = |candidate: f32| {
        let preferred = if requested <= 100.0 {
            candidate <= requested
        } else {
            candidate >= requested
        };
        (!preferred, (candidate - requested).abs())
    };
    faces
        .iter()
        .map(|face| {
            let [low, high] = face.width.unwrap_or([100.0; 2]);
            requested.clamp(low, high)
        })
        .min_by(|&a, &b| {
            let (side_a, distance_a) = rank(a);
            let (side_b, distance_b) = rank(b);
            side_a
                .cmp(&side_b)
                .then_with(|| distance_a.total_cmp(&distance_b))
        })
        .unwrap_or(100.0)
}

fn variations(run: &RunStyle, face: Option<&FaceVariations>) -> Vec<FontVariation> {
    let mut axes = BTreeMap::new();
    if let Some(face) = face {
        if let Some([low, high]) = face.weight {
            axes.insert(*b"wght", run_weight(run).clamp(low, high));
        }
        if let Some([low, high]) = face.width {
            axes.insert(*b"wdth", run_width(run).clamp(low, high));
        }
        axes.extend(face.axes.iter().copied());
    }
    if !run.optical_sizing_disabled {
        // XHTML layout measures the computed font size in typographic points.
        axes.insert(*b"opsz", run.chars.size.unwrap_or(12.0));
    }
    axes.extend(run.axes.iter().copied());
    axes.into_iter()
        .map(|(axis, value)| FontVariation::new(Tag::from_bytes(axis), value))
        .collect()
}

fn named_family<'a>(
    packaged: &'a FontNames,
    name: &'a str,
    request: FontRequest,
) -> FontFamilyName<'a> {
    let lower = name.to_ascii_lowercase();
    let alias = packaged
        .get(&lower)
        .and_then(|names| names.get(&Some(request)).or_else(|| names.get(&None)));
    FontFamilyName::named(if let Some(alias) = alias {
        alias
    } else {
        match lower.as_str() {
            "arial" => "Source Sans 3",
            "times new roman" => "Source Serif 4",
            "courier new" => "JetBrains Mono",
            _ => name,
        }
    })
}

fn append_span<'a>(
    source: &'a StyledText<'_>,
    range: Range<usize>,
    text: &mut String,
    spans: &mut Vec<(Range<usize>, &'a RunStyle)>,
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
    css: Option<&str>,
    mut style: CharProps,
    output: &mut Output<'_>,
    width: f32,
) -> Result<Fragment, EngineError> {
    let equation = output.equation(markup, css, &mut style)?;
    style.font = None;
    let resolved = output.document.styles.resolve_char(None, &style);
    let layout = match equation {
        Equation::Native {
            linear,
            display,
            math: equation,
        } => math::layout_equation(
            &equation,
            &linear,
            &resolved,
            display,
            width,
            &Default::default(),
            &mut 0,
        ),
        Equation::SpanningTable { table, extras, .. } => {
            let measured = extras.measure(&output.document, &style)?;
            table
                .layout(&output.document, &style, &measured)?
                .into_math()
        }
        Equation::Composed {
            math,
            display,
            extras,
        } => extras
            .measure(&output.document, &style)?
            .layout(&math, "", &resolved, display, width)?,
    };
    Ok(Fragment {
        width: layout.width,
        height: layout.ascent + layout.descent,
        baseline: Some(layout.ascent),
        items: equation_items(layout),
    })
}

fn equation_items(layout: MathLayout) -> Vec<Draw> {
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
    items
}

pub(super) fn read_face(face: &Face, book: &mut Book<'_>) -> Result<Vec<u8>, EngineError> {
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
