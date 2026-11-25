//! Paint a stationary WebVTT region using the EPUB glyph shaper.
use parley::Alignment;
use wordcraft_doc::{props::Rgb, Document};
use wordcraft_geom::Rect;
use wordcraft_layout::display::Draw;

use super::{
    fonts::{Fonts, GlyphPaths},
    invalid, limited, EngineError,
};

pub(crate) type Run = (String, [u8; 4], bool, String, bool, bool);
pub(crate) type Paint = ([f32; 4], [u8; 4], [u8; 4], f32, String);

const MAX_PIXELS: u64 = 1920 * 1088;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_RUNS: usize = 1024;

pub(crate) fn overlay(
    size: (u32, u32),
    paint: Paint,
    mut runs: Vec<Run>,
) -> Result<Vec<u8>, EngineError> {
    let (width, height) = size;
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || width > wordcraft_render::MAX_SIDE
        || height > wordcraft_render::MAX_SIDE
    {
        return Err(limited("EPUB caption painting exceeds its viewport limit"));
    }
    let ([x, y, w, h], background, cue_background, opacity, align) = paint;
    if [x, y, w, h]
        .iter()
        .any(|value| !value.is_finite() || value.abs() > super::layout::MAX_PAINT_COORDINATE)
        || w < 0.0
        || h < 0.0
        || !(0.0..=1.0).contains(&opacity)
    {
        return Err(invalid("invalid EPUB caption paint geometry"));
    }
    if runs.len() > MAX_RUNS
        || runs.iter().map(|(text, ..)| text.len()).sum::<usize>() > MAX_TEXT_BYTES
    {
        return Err(limited("EPUB caption painting exceeds its text budget"));
    }
    if w == 0.0 || h == 0.0 || opacity == 0.0 {
        return Ok(Vec::new());
    }
    let align = match align.as_str() {
        "left" => Alignment::Left,
        "right" => Alignment::Right,
        "center" => Alignment::Center,
        _ => return Err(invalid("invalid EPUB caption text alignment")),
    };
    let text = collapse_whitespace(&mut runs);
    if text.is_empty() || text.bytes().all(|byte| byte == b'\n') {
        return Ok(Vec::new());
    }
    let mut fonts = Fonts::bundled();
    let text = fonts.shape_caption(
        &text,
        &runs,
        height as f32 * 0.05,
        height as f32 * 0.06,
        align,
    )?;
    let visible = text.size(w)[1].min(h);
    let area = Rect::new(x, y + h - visible, w, visible);
    let mut drawing = vec![fill(area, background)];
    if cue_background[3] != 0 {
        drawing.extend(
            text.caption_backgrounds(area)
                .into_iter()
                .map(|rect| fill(rect, cue_background)),
        );
    }
    text.draw(area, &mut GlyphPaths::default(), &mut drawing, None, None)?;
    let clipped = Draw::Clip {
        rect: area,
        items: drawing,
    };
    let surface = Rect::new(0.0, 0.0, width as f32, height as f32);
    let mut image = wordcraft_render::render_items(
        &Document::new(),
        &[clipped],
        surface,
        1.0,
        &wordcraft_render::RenderOptions::default(),
    );
    if (image.width, image.height) != size {
        return Err(invalid("EPUB caption renderer changed the viewport"));
    }
    crate::rendering::validate_rgba(width, height, &image.pixels)?;
    if opacity < 1.0 {
        for value in &mut image.pixels {
            *value = (f32::from(*value) * opacity).round() as u8;
        }
    }
    Ok(image.pixels)
}

fn collapse_whitespace(runs: &mut [Run]) -> String {
    let mut line_started = false;
    let mut pending: Option<usize> = None;
    let mut normalized = vec![String::new(); runs.len()];
    for (index, (text, ..)) in runs.iter().enumerate() {
        for character in text.chars() {
            match character {
                ' ' | '\t' => {
                    if line_started && pending.is_none() {
                        pending = Some(index);
                    }
                }
                '\n' => {
                    pending = None;
                    line_started = false;
                    normalized[index].push('\n');
                }
                _ => {
                    if let Some(owner) = pending.take() {
                        normalized[owner].push(' ');
                    }
                    normalized[index].push(character);
                    line_started = true;
                }
            }
        }
    }
    for ((text, ..), replacement) in runs.iter_mut().zip(normalized) {
        *text = replacement;
    }
    runs.iter().map(|(text, ..)| text.as_str()).collect()
}

fn fill(rect: Rect, [r, g, b, alpha]: [u8; 4]) -> Draw {
    Draw::Fill {
        rect,
        color: Rgb(r, g, b),
        alpha: f32::from(alpha) / 255.0,
        text_bg: false,
    }
}

pub(crate) fn composite(rgb: &[u8], overlay: &[u8]) -> Result<Vec<u8>, EngineError> {
    let count = overlay.len() / 4;
    if !overlay.len().is_multiple_of(4) || count as u64 > MAX_PIXELS || rgb.len() != count * 3 {
        return Err(invalid(
            "EPUB caption overlay does not match the video frame",
        ));
    }
    let mut output = rgb.to_vec();
    for (pixel, over) in output.chunks_exact_mut(3).zip(overlay.chunks_exact(4)) {
        let inverse = u16::from(255 - over[3]);
        for (channel, paint) in pixel.iter_mut().zip(over) {
            *channel =
                (u16::from(*paint) + (u16::from(*channel) * inverse + 127) / 255).min(255) as u8;
        }
    }
    Ok(output)
}
