//! Linked native metadata preservation, without Python language-key admission.
use super::*;
use std::io::{Cursor, Read, Write};
use wordcraft_doc::props::CharProps;
use wordcraft_doc::CoreProps;

fn replace_core(bytes: &[u8], core: &str) -> Vec<u8> {
    let mut source = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut target = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..source.len() {
        let mut entry = source.by_index(index).unwrap();
        let mut value = Vec::new();
        entry.read_to_end(&mut value).unwrap();
        target
            .start_file(entry.name(), zip::write::SimpleFileOptions::default())
            .unwrap();
        target
            .write_all(if entry.name() == "docProps/core.xml" {
                core.as_bytes()
            } else {
                &value
            })
            .unwrap();
    }
    target.finish().unwrap().into_inner()
}

fn core_xml(bytes: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut value = String::new();
    archive
        .by_name("docProps/core.xml")
        .unwrap()
        .read_to_string(&mut value)
        .unwrap();
    value
}

fn assert_ink(bytes: &[u8]) {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut output = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut output).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(
        info.buffer_size(),
        info.width as usize * info.height as usize * 4
    );
    assert!(
        output[..info.buffer_size()]
            .chunks_exact(4)
            .filter(|p| p[0] < 160 && p[1] < 160 && p[2] < 160 && p[3] > 0)
            .count()
            > 50
    );
}

#[test]
fn linked_core_language_survives_read_write_and_keeps_content_output() {
    let old: CoreProps = serde_json::from_value(json!({"title":"TitleKeep"})).unwrap();
    let mut base = Document::new();
    base.core = old;
    base.core.creator = "CreatorKeep".into();
    base.core.created = "2020-01-02T03:04:05Z".into();
    base.core.modified = "2021-02-03T04:05:06Z".into();
    base.styles.default_chr.font = Some("DejaVu Sans".into());
    base.styles.default_chr.size = Some(12.0);
    base.body = vec![wordcraft_doc::para_block(Paragraph::with_text(
        "LanguageKeep",
        CharProps {
            lang: Some("de-DE".into()),
            link: Some("https://example.test/language-keep".into()),
            ..Default::default()
        },
    ))];
    let original = wordcraft_docx::write(&base).unwrap();
    let original_core = core_xml(&original);
    assert!(!original_core.contains("dc:language"));
    let present = original_core.replace(
        "</cp:coreProperties>",
        "<dc:language> fr-CA </dc:language></cp:coreProperties>",
    );
    assert_ne!(present, original_core);
    let imported = replace_core(&original, &present);
    let mut model = read(&imported, None).unwrap();
    assert_eq!(model.core.title, "TitleKeep");
    assert_eq!(model.core.creator, "CreatorKeep");
    assert_eq!(model.core.created, base.core.created);
    assert_eq!(model.core.modified, base.core.modified);
    assert_eq!(model.plain_text(StoryRef::Body), "LanguageKeep");
    assert_eq!(
        model.para(StoryRef::Body, &Path::top(0)).unwrap().runs[0]
            .props
            .lang
            .as_deref(),
        Some("de-DE")
    );
    // Old-compatible serde access makes this exact desired regression executable
    // against the original pin; no Python metadata surface is involved.
    assert_eq!(
        serde_json::to_value(&model.core).unwrap()["language"],
        "fr-CA"
    );
    assert_eq!(serde_json::to_value(&base.core).unwrap()["language"], "");
    let original_png = render(&original, 0, 72.0, None).unwrap();
    let original_pdf = to_pdf(&original, None).unwrap();
    let original_pdf_png = crate::pdf::render(&original_pdf, 0, 72.0, None).unwrap();
    assert_ink(&original_png);
    assert_ink(&original_pdf_png);
    for _ in 0..3 {
        let written = wordcraft_docx::write(&model).unwrap();
        assert!(core_xml(&written).contains("<dc:language>fr-CA</dc:language>"));
        let reopened = read(&written, None).unwrap();
        assert_eq!(reopened.core, model.core);
        assert_eq!(reopened.body, model.body);
        let payload = extract(&written, None).unwrap();
        assert_eq!(payload.pages.len(), 1);
        assert_eq!(payload.pages[0].blocks[0].text, "LanguageKeep");
        assert!(payload.pages[0]
            .links
            .iter()
            .any(|link| link.uri == "https://example.test/language-keep"));
        let pdf = to_pdf(&written, None).unwrap();
        let parsed = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(parsed.pages.len(), 1);
        assert!(parsed.pages[0]
            .blocks
            .iter()
            .any(|block| block.text.contains("LanguageKeep")));
        assert!(parsed.pages[0]
            .links
            .iter()
            .any(|link| link.uri == "https://example.test/language-keep"));
        assert_eq!(render(&written, 0, 72.0, None).unwrap(), original_png);
        assert_eq!(
            crate::pdf::render(&pdf, 0, 72.0, None).unwrap(),
            original_pdf_png
        );
        model = reopened;
    }
    let mut core = serde_json::to_value(&model.core).unwrap();
    core["language"] = json!("");
    model.core = serde_json::from_value(core).unwrap();
    let cleared = wordcraft_docx::write(&model).unwrap();
    assert_eq!(core_xml(&cleared), original_core);
    assert_eq!(read(&cleared, None).unwrap().core, base.core);
    assert_eq!(render(&cleared, 0, 72.0, None).unwrap(), original_png);
    assert_eq!(read(&imported, None).unwrap().core.title, "TitleKeep");
}
