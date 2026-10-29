pub mod api;
#[cfg(feature = "python")]
mod author_docx;
pub mod author_epub;
pub mod author_pdf;
pub mod author_pptx;
#[cfg(feature = "python")]
mod author_pptx_python;
#[path = "convert.rs"]
mod conversion;
mod diagnostics;
mod docx;
mod epub;
mod limits;
mod model;
mod ooxml;
mod pdf;
mod pptx;
mod rendering;
pub mod session_api;

#[cfg(feature = "python")]
mod python;
#[cfg(feature = "python")]
use python::DocExNativeError;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("invalid_argument:{0}")]
    InvalidArgument(String),
    #[error("closed:{0}")]
    Closed(String),
    #[error("unsupported:{0}")]
    Unsupported(String),
    #[error("malformed:{0}")]
    Malformed(String),
    #[error("unsafe_archive:{0}")]
    UnsafeArchive(String),
    #[error("resource_limit:{0}")]
    ResourceLimit(String),
    #[error("index:{0}")]
    PageIndex(String),
    #[error("encrypted:document is password-protected or the supplied password is incorrect")]
    Encrypted,
    #[error("conversion_failure:{0}")]
    Conversion(String),
    #[error("rendering_failure:{0}")]
    Rendering(String),
}
