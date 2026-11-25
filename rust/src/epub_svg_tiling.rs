//! Keep native tile sampling in a provable color space and raster domain.
use std::collections::HashMap;

use resvg::{
    tiny_skia::Transform,
    usvg::{
        filter::{ColorInterpolation, Filter, Input, Kind, Primitive, Tile},
        NonZeroRect,
    },
};

use crate::{epub::unsupported, EngineError};

#[derive(Clone, Copy)]
struct TileSeed {
    region: NonZeroRect,
    srgb: bool,
}

pub(super) fn passes_input(tile: &Tile, filter: &Filter, mut previous: &[Primitive]) -> bool {
    let mut input = tile.input();
    while let Input::Reference(name) = input {
        let Some(index) = previous
            .iter()
            .rposition(|primitive| primitive.result() == name)
        else {
            return false;
        };
        let primitive = &previous[index];
        if let Kind::Offset(offset) = primitive.kind() {
            // The renderer uses the offset's effective input region.
            input = offset.input();
            previous = &previous[..index];
        } else {
            return primitive.rect() == filter.rect();
        }
    }
    true
}

pub(super) fn validate(
    filter: &Filter,
    scale: [f32; 2],
    device: Transform,
    viewport: [u32; 2],
) -> Result<(), EngineError> {
    if !filter
        .primitives()
        .iter()
        .any(|primitive| matches!(primitive.kind(), Kind::Tile(_)))
    {
        return Ok(());
    }
    let region = filter.rect();
    let original = TileSeed { region, srgb: true };
    let mut results = HashMap::<&str, TileSeed>::new();
    for primitive in filter.primitives() {
        let seed = |input: &Input| match input {
            Input::SourceGraphic | Input::SourceAlpha => Some(original),
            Input::Reference(name) => results.get(name.as_str()).copied(),
        };
        let srgb = primitive.color_interpolation() == ColorInterpolation::SRGB;
        let mut result = TileSeed {
            region: primitive.rect(),
            srgb,
        };
        match primitive.kind() {
            Kind::Tile(tile) => {
                if let Input::Reference(_) = tile.input() {
                    let input = seed(tile.input())
                        .ok_or_else(|| unsupported("SVG tile input has no prior native result"))?;
                    if !input.srgb {
                        return Err(unsupported("SVG tile requires an sRGB input result"));
                    }
                    validate_seed(input.region, region, scale, device, viewport)?;
                }
                result.srgb = true;
            }
            Kind::Flood(_) => result.srgb = true,
            Kind::Offset(offset) => {
                if let Some(input) = seed(offset.input()) {
                    result = input;
                }
            }
            // The native kernel can pass the incoming buffer through without
            // converting its color space when the effect is too small.
            Kind::GaussianBlur(blur) => {
                result.srgb &= seed(blur.input()).is_some_and(|input| input.srgb);
            }
            Kind::DropShadow(shadow) => {
                result.srgb &= seed(shadow.input()).is_some_and(|input| input.srgb);
            }
            _ => {}
        }
        results.insert(primitive.result(), result);
    }
    Ok(())
}

fn validate_seed(
    seed: NonZeroRect,
    filter: NonZeroRect,
    [sx, sy]: [f32; 2],
    device: Transform,
    [width, height]: [u32; 2],
) -> Result<(), EngineError> {
    if seed.x() < filter.x()
        || seed.y() < filter.y()
        || seed.right() > filter.right()
        || seed.bottom() > filter.bottom()
    {
        return Err(unsupported(
            "SVG tile input region exceeds its filter region",
        ));
    }
    let seed_on_canvas = seed
        .transform(device)
        .ok_or_else(|| unsupported("SVG tile input region has an invalid rendered transform"))?;
    // The native layer can clamp its canvas before it copies the tile. An
    // off-canvas seed can then repeat a different region or clear the layer.
    if seed_on_canvas.x() < 0.0
        || seed_on_canvas.y() < 0.0
        || seed_on_canvas.right() > width as f32
        || seed_on_canvas.bottom() > height as f32
    {
        return Err(unsupported(
            "SVG tile input must fit within the raster viewport",
        ));
    }
    if seed == filter {
        return Ok(());
    }
    for period in [seed.width() * sx, seed.height() * sy] {
        if period < 1.0 || (period - period.round()).abs() > 0.0001 {
            return Err(unsupported("SVG tile periods must align to device pixels"));
        }
    }
    Ok(())
}
