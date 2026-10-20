use std::io::{Cursor, Read};

use crate::limits::{add_uncompressed_size, validate_member, validate_member_count};
use crate::EngineError;

pub fn preflight(data: &[u8]) -> Result<(), EngineError> {
    inspect(data, false).map(|_| ())
}

pub fn document_type(data: &[u8]) -> Result<Option<&'static str>, EngineError> {
    inspect(data, true)
}

fn inspect(data: &[u8], read_content_types: bool) -> Result<Option<&'static str>, EngineError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(data))
        .map_err(|error| EngineError::Malformed(format!("invalid ZIP: {error}")))?;
    validate_member_count(archive.len())?;
    let mut total = 0_u64;
    let mut document_type = None;
    for index in 0..archive.len() {
        let mut member = archive
            .by_index(index)
            .map_err(|error| EngineError::Malformed(format!("invalid ZIP member: {error}")))?;
        let name = member.name().replace('\\', "/");
        validate_member(&name, member.size(), member.compressed_size())?;
        total = add_uncompressed_size(total, member.size())?;
        if read_content_types && name == "[Content_Types].xml" {
            let mut bytes = Vec::with_capacity(member.size() as usize);
            member.read_to_end(&mut bytes).map_err(|error| {
                EngineError::Malformed(format!("invalid ZIP member data: {error}"))
            })?;
            let types = String::from_utf8_lossy(&bytes);
            if types.contains("wordprocessingml.document.main+xml") {
                document_type = Some("docx");
            } else if types.contains("presentationml.presentation.main+xml") {
                document_type = Some("pptx");
            }
        }
    }
    Ok(document_type)
}
