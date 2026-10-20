use std::io::{Cursor, Read};

use quick_xml::events::Event;

use crate::limits::{add_uncompressed_size, validate_member, validate_member_count};
use crate::EngineError;

pub fn preflight(data: &[u8]) -> Result<(), EngineError> {
    inspect(data, false, false).map(|_| ())
}

pub fn document_type(data: &[u8]) -> Result<Option<&'static str>, EngineError> {
    inspect(data, true, false)
}

/// Count source slide references before the native importer can truncate them.
/// Other formats and format detection retain the ordinary ZIP-only preflight.
pub fn preflight_pptx(data: &[u8]) -> Result<(), EngineError> {
    inspect(data, false, true).map(|_| ())
}

fn inspect(
    data: &[u8],
    read_content_types: bool,
    count_presentation_slides: bool,
) -> Result<Option<&'static str>, EngineError> {
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
        if count_presentation_slides
            && !member.is_dir()
            && member.size() <= MAX_NATIVE_XML_BYTES as u64
        {
            // The native package XML accessor ignores larger members. Scan every
            // eligible member, irrespective of path/extension, so private native
            // root-relationship resolution need not be duplicated here.
            let size = member.size() as usize;
            let mut bytes = Vec::new();
            bytes.try_reserve_exact(size).map_err(|_| {
                EngineError::ResourceLimit("cannot allocate bounded PPTX preflight member".into())
            })?;
            bytes.resize(size, 0);
            member.read_exact(&mut bytes).map_err(|error| {
                EngineError::Malformed(format!("invalid ZIP member data: {error}"))
            })?;
            let mut extra = [0];
            if member.read(&mut extra).map_err(|error| {
                EngineError::Malformed(format!("invalid ZIP member data: {error}"))
            })? != 0
            {
                return Err(EngineError::ResourceLimit(
                    "ZIP member exceeds declared size".into(),
                ));
            }
            presentation_slide_count(&bytes)?;
        }
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

// Match the pinned native Package::xml raw-byte ceiling. Existing ZIP checks
// bound aggregate raw expansion to 512 MiB; only one member is held at a time.
const MAX_NATIVE_XML_BYTES: usize = 96 * 1024 * 1024;
const MAX_DECODED_XML_BYTES: usize = 144 * 1024 * 1024;
const MAX_PRESENTATION_PROLOG: u64 = 64 * 1024;
const MAX_PRESENTATION_TOKEN: u64 = 1024 * 1024;
const MAX_PRESENTATION_DEPTH: usize = 256;
const MAX_SOURCE_SLIDES: usize = 10_000;

fn decode_utf16_xml(data: &[u8]) -> Result<String, EngineError> {
    let little_endian = data.starts_with(&[0xff, 0xfe]);
    let units = data[2..].chunks_exact(2).map(|pair| {
        let pair = [pair[0], pair[1]];
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });
    // Each UTF-16 code unit emits at most three UTF-8 bytes, including lossy
    // replacements. Standard decoding also matches native's ignored odd byte.
    let maximum = ((data.len() - 2) / 2)
        .checked_mul(3)
        .filter(|maximum| *maximum <= MAX_DECODED_XML_BYTES)
        .ok_or_else(|| EngineError::ResourceLimit("PPTX decoded XML exceeds 144 MiB".into()))?;
    let mut decoded = String::new();
    decoded.try_reserve_exact(maximum).map_err(|_| {
        EngineError::ResourceLimit("cannot allocate bounded PPTX UTF-16 decoding".into())
    })?;
    for character in std::char::decode_utf16(units) {
        decoded.push(character.unwrap_or(char::REPLACEMENT_CHARACTER));
    }
    Ok(decoded)
}

fn presentation_slide_count(data: &[u8]) -> Result<(), EngineError> {
    if data.len() > MAX_NATIVE_XML_BYTES {
        return Ok(());
    }
    let decoded = if data.starts_with(&[0xff, 0xfe]) || data.starts_with(&[0xfe, 0xff]) {
        Some(decode_utf16_xml(data)?)
    } else {
        None
    };
    let mut declaration_bytes = 0;
    let data = if let Some(decoded) = &decoded {
        // Native strips an initial encoding declaration after UTF-16 decoding.
        declaration_bytes = decoded
            .strip_prefix("<?xml")
            .and_then(|rest| rest.find("?>").map(|offset| offset + 7))
            .unwrap_or(0);
        &decoded.as_bytes()[declaration_bytes..]
    } else {
        data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(data)
    };
    // Borrow tokens directly from the bounded buffer: even a binary member's
    // large text token never allocates another token buffer or an XML tree.
    let mut reader = quick_xml::Reader::from_reader(data);
    reader.config_mut().check_end_names = false;
    reader.config_mut().allow_unmatched_ends = true;
    let mut presentation = false;
    let mut depth = 0_usize;
    let mut slides = 0_usize;
    loop {
        let before = reader.buffer_position();
        let event = reader.read_event();
        let token_bytes = reader.buffer_position().saturating_sub(before);
        if presentation && token_bytes > MAX_PRESENTATION_TOKEN {
            return Err(EngineError::ResourceLimit(
                "PPTX presentation XML token exceeds 1 MiB".into(),
            ));
        }
        let event = match event {
            Ok(event) => event,
            // A parse failure cannot become a native presentation tree. Leave
            // native import to report malformed main parts; binary media passes.
            Err(_) => return Ok(()),
        };
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let name = element.name();
                let local = name
                    .as_ref()
                    .rsplit(|byte| *byte == b':')
                    .next()
                    .unwrap_or_default();
                let empty = matches!(event, Event::Empty(_));
                if !presentation {
                    if local != b"presentation" {
                        return Ok(());
                    }
                    presentation = true;
                    if before.saturating_add(declaration_bytes as u64) > MAX_PRESENTATION_PROLOG {
                        return Err(EngineError::ResourceLimit(
                            "PPTX presentation XML prolog exceeds 64 KiB".into(),
                        ));
                    }
                    if token_bytes > MAX_PRESENTATION_TOKEN {
                        return Err(EngineError::ResourceLimit(
                            "PPTX presentation XML token exceeds 1 MiB".into(),
                        ));
                    }
                    if empty {
                        return Ok(());
                    }
                    depth = 1;
                    continue;
                }
                // Native markup-compatibility processing can promote list or ID
                // elements from AlternateContent. Count all source IDs under the
                // presentation, including section/unused branch references, rather
                // than implementing another branch-selection algorithm.
                if local == b"sldId" {
                    slides += 1;
                    if slides > MAX_SOURCE_SLIDES {
                        return Err(EngineError::ResourceLimit(
                            "PPTX source exceeds 10000 slide entries".into(),
                        ));
                    }
                }
                if !empty {
                    if depth >= MAX_PRESENTATION_DEPTH {
                        return Err(EngineError::ResourceLimit(
                            "PPTX presentation XML exceeds 256 levels".into(),
                        ));
                    }
                    depth += 1;
                }
            }
            Event::End(_) if presentation => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Ok(());
                }
            }
            Event::Eof => return Ok(()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn presentation(entries: usize) -> String {
        format!(
            "<p:presentation xmlns:p=\"presentation\"><p:sldIdLst>{}</p:sldIdLst></p:presentation>",
            "<p:sldId id=\"1\"/>".repeat(entries)
        )
    }

    fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
        let mut bytes = if little_endian {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in text.encode_utf16() {
            bytes.extend(if little_endian {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        bytes
    }

    fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in parts {
            archive
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    #[test]
    fn counts_source_entries_before_native_relationship_resolution_and_truncation() {
        let at_limit = presentation(10_000);
        assert!(presentation_slide_count(at_limit.as_bytes()).is_ok());
        let excessive = presentation(10_001);
        assert!(matches!(
            presentation_slide_count(excessive.as_bytes()),
            Err(EngineError::ResourceLimit(_))
        ));
        let bytes = package(&[("relocated/main.bin", excessive.as_bytes())]);
        assert!(matches!(
            preflight_pptx(&bytes),
            Err(EngineError::ResourceLimit(_))
        ));
        // Ordinary OOXML detection/preflight do not run the PPTX-only scan.
        assert!(preflight(&bytes).is_ok());
        assert!(document_type(&bytes).unwrap().is_none());
    }

    #[test]
    fn native_root_recognition_handles_boms_leading_text_and_utf16_both_orders() {
        let text = presentation(10_001);
        for prefix in [
            "",
            " \n\t",
            "plain text before root",
            "<!--prolog-->",
            "<?xml version=\"1.0\"?>",
        ] {
            let source = format!("{prefix}{text}");
            for bytes in [
                source.as_bytes().to_vec(),
                [b"\xef\xbb\xbf".as_slice(), source.as_bytes()].concat(),
                utf16(&source, true),
                utf16(&source, false),
            ] {
                assert!(
                    matches!(
                        presentation_slide_count(&bytes),
                        Err(EngineError::ResourceLimit(_))
                    ),
                    "prefix={prefix}"
                );
            }
        }
        for little_endian in [true, false] {
            let source = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-16\"?>{}",
                presentation(1)
            );
            let mut bytes = utf16(&source, little_endian);
            bytes.push(0); // Native ignores an incomplete final UTF-16 unit.
            assert!(presentation_slide_count(&bytes).is_ok());
        }
    }

    #[test]
    fn all_slide_id_elements_are_counted_without_resolving_markup_compatibility() {
        let many = "<p:sldId/>".repeat(10_001);
        for text in [
            format!("<p:presentation><p:extLst>{many}</p:extLst><p:sldIdLst><p:sldId/></p:sldIdLst></p:presentation>"),
            format!("<p:presentation><p:sldIdLst/><p:sldIdLst>{many}</p:sldIdLst></p:presentation>"),
            format!("<p:presentation><mc:AlternateContent><mc:Fallback><p:sldIdLst>{many}</p:sldIdLst></mc:Fallback></mc:AlternateContent></p:presentation>"),
            format!("<p:presentation><p:sldIdLst><p:ext>{many}</p:ext></p:sldIdLst></p:presentation>"),
        ] { assert!(matches!(presentation_slide_count(text.as_bytes()), Err(EngineError::ResourceLimit(_)))); }
        for text in [
            format!("<p:presentation><!--{many}--><p:sldIdLst><![CDATA[{many}]]><p:sldId/></p:sldIdLst></p:presentation>"),
            format!("<p:other>{}</p:other>", presentation(10_001)),
        ] { assert!(presentation_slide_count(text.as_bytes()).is_ok()); }
        let starts = format!(
            "<p:presentation><p:sldIdLst>{}</p:sldIdLst></p:presentation>",
            "<p:sldId></p:sldId>".repeat(10_001)
        );
        assert!(matches!(
            presentation_slide_count(starts.as_bytes()),
            Err(EngineError::ResourceLimit(_))
        ));
    }

    #[test]
    fn presentation_only_token_prolog_and_depth_budgets_are_explicit() {
        let at_token = format!(
            "<presentation><other>{}</other></presentation>",
            "x".repeat(MAX_PRESENTATION_TOKEN as usize)
        );
        assert!(presentation_slide_count(at_token.as_bytes()).is_ok());
        let above_token = format!(
            "<presentation><other>{}</other></presentation>",
            "x".repeat(MAX_PRESENTATION_TOKEN as usize + 1)
        );
        assert!(matches!(
            presentation_slide_count(above_token.as_bytes()),
            Err(EngineError::ResourceLimit(_))
        ));
        let at_prolog = format!(
            "{}<presentation/>",
            " ".repeat(MAX_PRESENTATION_PROLOG as usize)
        );
        assert!(presentation_slide_count(at_prolog.as_bytes()).is_ok());
        let above_prolog = format!(
            "{}<presentation/>",
            " ".repeat(MAX_PRESENTATION_PROLOG as usize + 1)
        );
        assert!(matches!(
            presentation_slide_count(above_prolog.as_bytes()),
            Err(EngineError::ResourceLimit(_))
        ));
        let declaration = format!(
            "<?xml {}?>{}",
            " ".repeat(MAX_PRESENTATION_PROLOG as usize),
            presentation(1)
        );
        assert!(matches!(
            presentation_slide_count(&utf16(&declaration, true)),
            Err(EngineError::ResourceLimit(_))
        ));
        for (nested, accepted) in [(255, true), (256, false)] {
            let text = format!(
                "<presentation>{}{}<sldIdLst/></presentation>",
                "<other>".repeat(nested),
                "</other>".repeat(nested)
            );
            assert_eq!(presentation_slide_count(text.as_bytes()).is_ok(), accepted);
        }
        let binary = vec![0xff; 2 * MAX_PRESENTATION_TOKEN as usize];
        let non_presentation = format!(
            "{}<other/>",
            "x".repeat(2 * MAX_PRESENTATION_TOKEN as usize)
        );
        assert!(preflight_pptx(&package(&[
            ("media/binary.bin", &binary),
            ("unrelated.xml", non_presentation.as_bytes())
        ]))
        .is_ok());
    }
}
