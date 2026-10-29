//! Stateful, in-process render sessions for Rust callers.
//!
//! Opening returns an owned normalized payload separately from the session. The
//! payload remains usable after closing or dropping the session. Page indexes
//! are zero-based; clipping rectangles are in displayed page points.

use crate::{api, docx, limits, pdf, pptx, EngineError};

enum Backend {
    Pdf(pdf::PdfSession),
    Docx(docx::DocxSession),
    Pptx(pptx::PptxSession),
}

/// An owned renderer that reuses the document's parsed native state.
///
/// `close` releases the retained renderer/model and is idempotent. Operations
/// already in progress are serialized with close by the underlying session.
pub struct DocumentSession {
    backend: Backend,
}

impl DocumentSession {
    /// Open a PDF, DOCX or PPTX and return its owned normalized payload.
    ///
    /// PPTX passwords have the same behavior as the stateless API: encrypted
    /// presentations are rejected, and a supplied password is not used.
    pub fn open(
        data: &[u8],
        filetype: &str,
        password: Option<&str>,
    ) -> Result<(Self, api::DocumentPayload), EngineError> {
        limits::validate_input_size(data.len())?;
        let (backend, payload) = match api::normalize(filetype)? {
            "pdf" => {
                let (session, payload) = pdf::PdfSession::open(data, password)?;
                (Backend::Pdf(session), payload)
            }
            "docx" => {
                let (session, payload) = docx::DocxSession::open(data, password)?;
                (Backend::Docx(session), payload)
            }
            "pptx" => {
                let (session, payload) = pptx::PptxSession::open(data)?;
                (Backend::Pptx(session), payload)
            }
            "epub" => {
                return Err(EngineError::Unsupported(
                    "EPUB rendering is not supported".into(),
                ))
            }
            _ => unreachable!(),
        };
        limits::bounded_json(&payload, limits::MAX_PAYLOAD_SIZE)?;
        Ok((Self { backend }, payload))
    }

    /// Render a page as PNG, optionally clipping to a rectangle in page points.
    pub fn render(
        &self,
        page: usize,
        dpi: f64,
        clip: Option<api::Clip>,
    ) -> Result<Vec<u8>, EngineError> {
        if let Some(clip) = clip {
            crate::rendering::validate_clip(clip)?;
        }
        match &self.backend {
            Backend::Pdf(session) => session.render(page, dpi, clip),
            Backend::Docx(session) => session.render(page, dpi, clip),
            Backend::Pptx(session) => session.render(page, dpi, clip),
        }
    }

    /// Search a PDF page and return displayed-page rectangles `[x0, y0, x1, y1]`.
    /// Search is not supported for DOCX or PPTX sessions.
    pub fn search_for(&self, page: usize, query: &str) -> Result<Vec<[f32; 4]>, EngineError> {
        match &self.backend {
            Backend::Pdf(session) => session.search_for(page, query),
            Backend::Docx(_) | Backend::Pptx(_) => Err(EngineError::Unsupported(
                "search is only supported for PDF sessions".into(),
            )),
        }
    }

    /// Release retained native state. Calling this more than once is safe.
    pub fn close(&self) -> Result<(), EngineError> {
        match &self.backend {
            Backend::Pdf(session) => session.close(),
            Backend::Docx(session) => session.close(),
            Backend::Pptx(session) => session.close(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pdf_fixture() -> Vec<u8> {
        let stream = "BT /F1 18 Tf 20 50 Td (Hello session) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{}\nendstream", stream.len(), stream),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        pdf
    }

    #[test]
    fn pdf_search_render_and_close() {
        let (session, payload) = DocumentSession::open(&pdf_fixture(), "pdf", None).unwrap();
        assert_eq!(payload.pages.len(), 1);
        assert!(!session.search_for(0, "Hello").unwrap().is_empty());
        assert!(session
            .render(0, 72.0, Some((0.0, 0.0, 50.0, 50.0)))
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        session.close().unwrap();
        assert!(matches!(
            session.search_for(0, "Hello"),
            Err(EngineError::Closed(_))
        ));
        assert_eq!(payload.pages.len(), 1);
    }

    #[test]
    fn office_sessions_return_owned_payload_and_close_idempotently() {
        for (bytes, kind) in [
            (
                &include_bytes!("../../tests/data/wordcraft-rectangle-stroke100.docx")[..],
                ".WORD",
            ),
            (
                &include_bytes!("../../tests/data/deckcraft-table-line-breaks.pptx")[..],
                "PPTX",
            ),
        ] {
            let owned = bytes.to_vec();
            let (session, payload) = DocumentSession::open(&owned, kind, None).unwrap();
            drop(owned);
            assert!(!payload.pages.is_empty());
            assert!(session
                .render(0, 72.0, None)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n"));
            assert!(matches!(
                session.render(usize::MAX, 72.0, None),
                Err(EngineError::Unsupported(_))
            ));
            assert!(matches!(
                session.search_for(0, "text"),
                Err(EngineError::Unsupported(_))
            ));
            session.close().unwrap();
            session.close().unwrap();
            assert!(matches!(
                session.render(0, 72.0, None),
                Err(EngineError::Closed(_))
            ));
            drop(session);
            assert!(!payload.pages.is_empty());
        }
    }

    #[test]
    fn rejects_unsupported_formats_and_invalid_inputs() {
        assert!(matches!(
            DocumentSession::open(b"", "epub", None),
            Err(EngineError::Unsupported(_))
        ));
        assert!(matches!(
            DocumentSession::open(b"", "unknown", None),
            Err(EngineError::Unsupported(_))
        ));
        assert!(DocumentSession::open(b"bad", "pdf", None).is_err());
    }
}
