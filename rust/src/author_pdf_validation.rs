//! Python-independent validation shared by PDF authoring operations.

use std::io::Cursor;

use crate::EngineError;

pub(super) const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn validate_page_size(width: f64, height: f64) -> Result<(), EngineError> {
    if !width.is_finite()
        || !height.is_finite()
        || !(3.0..=14_400.0).contains(&width)
        || !(3.0..=14_400.0).contains(&height)
    {
        return Err(EngineError::InvalidArgument(
            "page sides must be finite and between 3 and 14400 points".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_position(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), EngineError> {
    if !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 || x >= width || y >= height {
        return Err(EngineError::InvalidArgument(
            "position must be finite and inside the page".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_png(data: &[u8]) -> Result<(), EngineError> {
    if data.len() > MAX_IMAGE_BYTES {
        return Err(EngineError::ResourceLimit(
            "image input exceeds 64 MiB".into(),
        ));
    }
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(EngineError::Unsupported(
            "PDF image insertion currently accepts PNG bytes".into(),
        ));
    }
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_limits(png::Limits {
        bytes: MAX_IMAGE_BYTES,
    });
    let reader = decoder
        .read_info()
        .map_err(|error| EngineError::Malformed(format!("invalid PNG image: {error}")))?;
    let info = reader.info();
    crate::rendering::validate_size(f64::from(info.width), f64::from(info.height), 16_000)?;
    if reader
        .output_buffer_size()
        .is_none_or(|size| size > 256_000_000)
    {
        return Err(EngineError::ResourceLimit(
            "decoded image exceeds 256 MB".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_and_position_validation_return_native_errors() {
        assert!(validate_page_size(3.0, 14_400.0).is_ok());
        assert!(matches!(
            validate_page_size(f64::NAN, 10.0),
            Err(EngineError::InvalidArgument(_))
        ));
        assert!(matches!(
            validate_page_size(2.99, 10.0),
            Err(EngineError::InvalidArgument(_))
        ));
        assert!(validate_position(0.0, 9.0, 10.0, 10.0).is_ok());
        assert!(matches!(
            validate_position(10.0, 0.0, 10.0, 10.0),
            Err(EngineError::InvalidArgument(_))
        ));
    }

    #[test]
    fn png_validation_returns_native_errors() {
        assert!(matches!(
            validate_png(b"not png"),
            Err(EngineError::Unsupported(_))
        ));
        assert!(matches!(
            validate_png(b"\x89PNG\r\n\x1a\n"),
            Err(EngineError::Malformed(_))
        ));
        let png = crate::rendering::encode_png(1, 1, &[0, 0, 0, 255], 72.0).unwrap();
        assert!(validate_png(&png).is_ok());
    }
}
