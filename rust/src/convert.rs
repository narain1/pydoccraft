use std::sync::Arc;

use crate::EngineError;

pub fn pdf_to_docx(data: &[u8], password: Option<&str>) -> Result<Vec<u8>, EngineError> {
    let bytes = Arc::new(data.to_vec());
    let info = pdfcraft_render::inspect(bytes.clone(), password).map_err(pdf_error)?;
    if info.pages.is_empty() {
        return Err(EngineError::Malformed("PDF contains no pages".into()));
    }
    if !info.fields.is_empty() {
        return Err(EngineError::Unsupported(
            "PDF-to-DOCX conversion cannot preserve active form appearances; flatten owned fields first".into(),
        ));
    }
    let cos = pdfcraft_cos::Document::open_with_password(bytes, password)
        .map_err(|error| EngineError::Malformed(format!("invalid PDF object graph: {error}")))?;

    let mut pages = Vec::with_capacity(info.pages.len());
    for (index, page_info) in info.pages.iter().enumerate() {
        // The exporter expects displayed page coordinates in points with a bottom-left origin.
        let visible_rect = |rect: [f64; 4]| {
            let a = page_info.user_to_view(rect[0] as f32, rect[1] as f32);
            let b = page_info.user_to_view(rect[2] as f32, rect[3] as f32);
            [
                f64::from(a[0].min(b[0])),
                f64::from(page_info.height - a[1].max(b[1])),
                f64::from(a[0].max(b[0])),
                f64::from(page_info.height - a[1].min(b[1])),
            ]
        };
        let crop_width = f64::from(page_info.crop[2] - page_info.crop[0]);
        let display_width = f64::from(if page_info.rotation % 180 == 0 {
            page_info.width
        } else {
            page_info.height
        });
        let unit = if crop_width > 0.0 && crop_width.is_finite() && display_width.is_finite() {
            display_width / crop_width
        } else {
            1.0
        };

        let blocks = pdfcraft_edit::reading_blocks(&cos, index)
            .map_err(|error| {
                EngineError::Malformed(format!("PDF text extraction failed: {error}"))
            })?
            .into_iter()
            .filter(|block| !block.text.trim().is_empty())
            .map(|block| {
                let font = block.base_font.to_ascii_lowercase();
                pdfcraft_export::Block {
                    text: block.text,
                    rect: visible_rect(block.rect),
                    size: block.size * unit,
                    bold: font.contains("bold") || font.contains("black") || font.contains("heavy"),
                    italic: font.contains("italic") || font.contains("oblique"),
                    color: block.color,
                    rotation: block.rotation - f64::from(page_info.rotation),
                }
            })
            .collect();

        let mut images = Vec::new();
        for image in pdfcraft_edit::reading_images(&cos, index)
            .map_err(|error| {
                EngineError::Malformed(format!("PDF image extraction failed: {error}"))
            })?
            .into_iter()
            .filter(|image| !image.is_form)
        {
            let object = image.object.ok_or_else(|| {
                EngineError::Conversion(format!("PDF image {} has no source object", image.name))
            })?;
            let (extension, data) = pdfcraft_create::image_file(&cos, object).map_err(|error| {
                EngineError::Conversion(format!("PDF image export failed: {error}"))
            })?;
            images.push(pdfcraft_export::Image {
                ext: if extension == "jpg" { "jpg" } else { "png" },
                bytes: data.into(),
                rect: visible_rect(image.rect),
                size: [
                    image.matrix[0].hypot(image.matrix[1]) * unit,
                    image.matrix[2].hypot(image.matrix[3]) * unit,
                ],
                rotation: image.matrix[1].atan2(image.matrix[0]).to_degrees()
                    - f64::from(page_info.rotation),
            });
        }

        pages.push(pdfcraft_export::Page {
            width: f64::from(page_info.width),
            height: f64::from(page_info.height),
            blocks,
            images,
            background: None,
        });
    }

    Ok(pdfcraft_export::docx(
        &pages,
        info.title.as_deref().unwrap_or("Document"),
    ))
}

fn pdf_error(error: pdfcraft_render::OpenError) -> EngineError {
    match error {
        pdfcraft_render::OpenError::NeedsPassword | pdfcraft_render::OpenError::WrongPassword => {
            EngineError::Encrypted
        }
        pdfcraft_render::OpenError::Unsupported(message) => EngineError::Unsupported(message),
        pdfcraft_render::OpenError::Invalid(message) => {
            EngineError::Malformed(format!("invalid PDF: {message}"))
        }
    }
}

pub fn docx_to_pdf(data: &[u8], password: Option<&str>) -> Result<Vec<u8>, EngineError> {
    crate::docx::to_pdf(data, password)
}

pub fn pptx_to_pdf(data: &[u8]) -> Result<Vec<u8>, EngineError> {
    crate::pptx::to_pdf(data)
}
