use std::io::Cursor;

use crate::EngineError;

// Bound each RGBA raster to 256 MB, independently of the engine's side limit.
const MAX_PIXELS: u64 = 64_000_000;

/// Current displayed page points: x, y, width, height.
pub type Clip = (f64, f64, f64, f64);

pub fn validate_clip((x, y, width, height): Clip) -> Result<(), EngineError> {
    if ![x, y, width, height, x + width, y + height]
        .iter()
        .all(|value| value.is_finite())
        || width <= 0.0
        || height <= 0.0
    {
        return Err(EngineError::InvalidArgument(
            "clip must contain finite x, y and positive width, height".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl RasterRegion {
    /// Quantize outwards on the existing full-page raster grid. `pixels` preserves each
    /// backend's established rounding of the final page edge, without allocating that page.
    pub fn new(
        clip: Clip,
        page: [f64; 2],
        scale: f64,
        pixels: [f64; 2],
        max_side: u32,
    ) -> Result<Self, EngineError> {
        validate_clip(clip)?;
        if !page
            .into_iter()
            .chain(pixels)
            .chain([scale])
            .all(|value| value.is_finite() && value > 0.0)
        {
            return Err(EngineError::Malformed(
                "invalid page geometry for clipped rendering".into(),
            ));
        }
        let (x, y, width, height) = clip;
        let left = x.max(0.0);
        let top = y.max(0.0);
        let right = (x + width).min(page[0]);
        let bottom = (y + height).min(page[1]);
        if left >= right || top >= bottom {
            return Err(EngineError::InvalidArgument(
                "clip does not intersect the displayed page".into(),
            ));
        }
        let x0 = (left * scale).floor().min(pixels[0]);
        let y0 = (top * scale).floor().min(pixels[1]);
        let x1 = (right * scale).ceil().min(pixels[0]);
        let y1 = (bottom * scale).ceil().min(pixels[1]);
        if x0 >= x1 || y0 >= y1 {
            return Err(EngineError::InvalidArgument(
                "clip contains no pixels at the requested DPI".into(),
            ));
        }
        if ![x0, y0, x1, y1]
            .iter()
            .all(|value| value.is_finite() && *value <= f64::from(u32::MAX))
        {
            return Err(EngineError::ResourceLimit(
                "clip pixel coordinates exceed the supported range".into(),
            ));
        }
        validate_size(x1 - x0, y1 - y0, max_side)?;
        Ok(Self {
            x: x0 as u32,
            y: y0 as u32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        })
    }

    pub fn validate_result(self, width: u32, height: u32) -> Result<(), EngineError> {
        if (width, height) != (self.width, self.height) {
            return Err(EngineError::Malformed(
                "renderer changed the requested clip dimensions".into(),
            ));
        }
        Ok(())
    }
}

pub fn validate_dpi(dpi: f64) -> Result<(), EngineError> {
    if !dpi.is_finite() || !(1.0..=1200.0).contains(&dpi) {
        return Err(EngineError::ResourceLimit(
            "DPI must be finite and between 1 and 1200".into(),
        ));
    }
    Ok(())
}

pub fn validate_size(width: f64, height: f64, max_side: u32) -> Result<(), EngineError> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(EngineError::Malformed("invalid render dimensions".into()));
    }
    // Ceil conservatively covers both upstream rounding conventions.
    if width.ceil() > f64::from(max_side)
        || height.ceil() > f64::from(max_side)
        || width.ceil() * height.ceil() > MAX_PIXELS as f64
    {
        return Err(EngineError::ResourceLimit(
            "render exceeds the image dimension or pixel limit".into(),
        ));
    }
    Ok(())
}

pub fn validate_rgba(width: u32, height: u32, pixels: &[u8]) -> Result<(), EngineError> {
    validate_size(f64::from(width), f64::from(height), 16_000)?;
    let expected = u64::from(width) * u64::from(height) * 4;
    if pixels.len() as u64 != expected {
        return Err(EngineError::Malformed(
            "renderer returned an incomplete RGBA image".into(),
        ));
    }
    Ok(())
}

pub fn encode_png(
    width: u32,
    height: u32,
    premultiplied: &[u8],
    dpi: f64,
) -> Result<Vec<u8>, EngineError> {
    validate_dpi(dpi)?;
    validate_rgba(width, height, premultiplied)?;
    let mut rgba = premultiplied.to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut output), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels_per_meter = (dpi / 0.0254).round() as u32;
        encoder.set_pixel_dims(Some(png::PixelDimensions {
            xppu: pixels_per_meter,
            yppu: pixels_per_meter,
            unit: png::Unit::Meter,
        }));
        let mut writer = encoder
            .write_header()
            .map_err(|error| EngineError::Malformed(format!("PNG encoding failed: {error}")))?;
        writer
            .write_image_data(&rgba)
            .map_err(|error| EngineError::Malformed(format!("PNG encoding failed: {error}")))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_intersects_then_snaps_outwards_on_the_native_pixel_grid() {
        assert_eq!(
            RasterRegion::new((-2.5, -3.0, 10.0, 12.0), [100.0; 2], 2.0, [200.0; 2], 4096).unwrap(),
            RasterRegion {
                x: 0,
                y: 0,
                width: 15,
                height: 18
            }
        );
        assert_eq!(
            RasterRegion::new((0.25, 0.5, 1.25, 2.25), [100.0; 2], 1.5, [150.0; 2], 4096).unwrap(),
            RasterRegion {
                x: 0,
                y: 0,
                width: 3,
                height: 5
            }
        );
        assert_eq!(
            RasterRegion::new((99.0, 99.0, 10.0, 10.0), [100.4; 2], 1.0, [100.0; 2], 4096).unwrap(),
            RasterRegion {
                x: 99,
                y: 99,
                width: 1,
                height: 1
            }
        );
        // A backend that rounds the final page edge down has no pixel for this last sliver.
        assert!(matches!(
            RasterRegion::new((100.1, 1.0, 0.2, 1.0), [100.4; 2], 1.0, [100.0; 2], 4096),
            Err(EngineError::InvalidArgument(_))
        ));
    }

    #[test]
    fn clipping_limits_reject_before_allocation_instead_of_native_clamping() {
        for clip in [
            (0.0, 0.0, 0.0, 1.0),
            (0.0, 0.0, -1.0, 1.0),
            (f64::NAN, 0.0, 1.0, 1.0),
            (f64::MAX, 0.0, f64::MAX, 1.0),
            (100.0, 0.0, 1.0, 1.0),
        ] {
            assert!(matches!(
                RasterRegion::new(clip, [100.0; 2], 1.0, [100.0; 2], 4096),
                Err(EngineError::InvalidArgument(_))
            ));
        }
        assert!(RasterRegion::new(
            (0.0, 0.0, 4096.0, 1.0),
            [10000.0; 2],
            1.0,
            [10000.0; 2],
            4096
        )
        .is_ok());
        assert!(matches!(
            RasterRegion::new(
                (0.0, 0.0, 4096.1, 1.0),
                [10000.0; 2],
                1.0,
                [10000.0; 2],
                4096
            ),
            Err(EngineError::ResourceLimit(_))
        ));
        assert!(matches!(
            RasterRegion::new(
                (0.0, 0.0, 8001.0, 8000.0),
                [10000.0; 2],
                1.0,
                [10000.0; 2],
                16000
            ),
            Err(EngineError::ResourceLimit(_))
        ));
        assert!(matches!(
            RasterRegion::new((5e9, 0.0, 10.0, 1.0), [1e10; 2], 1.0, [1e10; 2], 16000),
            Err(EngineError::ResourceLimit(_))
        ));
        assert!(RasterRegion {
            x: 0,
            y: 0,
            width: 5,
            height: 6
        }
        .validate_result(5, 5)
        .is_err());
    }

    #[test]
    fn rejects_invalid_and_oversized_rasters_without_allocation() {
        for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(validate_size(value, 10.0, 16_000).is_err());
            assert!(validate_size(10.0, value, 16_000).is_err());
            assert!(validate_dpi(value).is_err());
        }
        assert!(validate_dpi(1200.1).is_err());
        assert!(validate_size(16_001.0, 1.0, 16_000).is_err());
        assert!(validate_size(8_001.0, 8_000.0, 16_000).is_err());
        assert!(validate_size(16_000.0, 1.0, 16_000).is_ok());
        assert!(validate_size(8_000.0, 8_000.0, 16_000).is_ok());
        assert!(validate_size(0.1, 0.1, 16_000).is_ok());
        assert!(validate_rgba(1, 1, &[0; 3]).is_err());
        assert!(validate_rgba(1, 1, &[0; 5]).is_err());
    }

    #[test]
    fn png_preserves_alpha_and_records_requested_resolution() {
        let encoded = encode_png(2, 1, &[64, 32, 0, 128, 10, 20, 30, 255], 150.0).unwrap();
        let mut reader = png::Decoder::new(Cursor::new(encoded)).read_info().unwrap();
        let dimensions = reader.info().pixel_dims.unwrap();
        assert_eq!(dimensions.unit, png::Unit::Meter);
        assert_eq!((dimensions.xppu, dimensions.yppu), (5906, 5906));
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(
            &pixels[..frame.buffer_size()],
            &[128, 64, 0, 128, 10, 20, 30, 255]
        );
    }
}
