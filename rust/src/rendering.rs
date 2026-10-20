use std::io::Cursor;

use crate::EngineError;

// Bound each RGBA raster to 256 MB, independently of the engine's side limit.
const MAX_PIXELS: u64 = 64_000_000;

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
