//! Linked typed native metadata, without Python extended-key admission.
use super::*;
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use wordcraft_doc::props::{CharProps, TextColor};
use wordcraft_doc::{para_block, Rgb};
use wordcraft_docx::{
    extended_properties, patch_extended_properties, ExtendedProperties, ExtendedPropertiesError,
    ExtendedPropertyPatch as P,
};

fn package(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut value = Vec::new();
            entry.read_to_end(&mut value).unwrap();
            (entry.name().to_owned(), value)
        })
        .collect()
}

fn assert_ink(bytes: &[u8]) {
    let mut reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (300, 180));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(info.buffer_size(), 300 * 180 * 4);
    assert!(
        pixels[..info.buffer_size()]
            .chunks_exact(4)
            .filter(|p| p[0] < 100 && p[1] < 100 && p[2] < 100 && p[3] > 128)
            .count()
            > 50
    );
}

#[test]
fn linked_extended_properties_keep_native_values_and_content_output() {
    let mut base = Document::new();
    base.core.title = "TitleKeep".into();
    base.core.language = "fr-CA".into();
    base.core.created = "2020-01-02T03:04:05Z".into();
    base.core.modified = "2021-02-03T04:05:06Z".into();
    base.styles.default_chr.font = Some("DejaVu Sans".into());
    base.styles.default_chr.size = Some(12.0);
    base.last_section.page_w = 300.0;
    base.last_section.page_h = 180.0;
    base.last_section.margin_left = 24.0;
    base.last_section.margin_right = 24.0;
    base.last_section.margin_top = 24.0;
    base.last_section.margin_bottom = 24.0;
    base.body = vec![para_block(Paragraph::with_text(
        "ExtendedKeep",
        CharProps {
            color: Some(TextColor::Rgb(Rgb(0, 0, 0))),
            lang: Some("de-DE".into()),
            link: Some("https://example.test/extended-keep".into()),
            ..Default::default()
        },
    ))];
    assert_eq!(
        extended_properties(&base).unwrap(),
        ExtendedProperties::default()
    );
    let original = wordcraft_docx::write(&base).unwrap();
    let original_parts = package(&original);
    let original_png = render(&original, 0, 72.0, None).unwrap();
    let original_pdf = to_pdf(&original, None).unwrap();
    let original_pdf_png = crate::pdf::render(&original_pdf, 0, 72.0, None).unwrap();
    assert_ink(&original_png);
    assert_ink(&original_pdf_png);
    let mut model = base.clone();
    let patches = [
        P::Template(Some("Template🌿<&>".into())),
        P::Manager(Some("".into())),
        P::Company(Some(" Company Keep ".into())),
        P::HyperlinkBase(Some("https://example.test/base".into())),
        P::ScaleCrop(Some(true)),
        P::LinksUpToDate(Some(false)),
        P::SharedDoc(Some(true)),
        P::HyperlinksChanged(Some(false)),
    ];
    patch_extended_properties(&mut model, &patches).unwrap();
    let expected = ExtendedProperties {
        template: Some("Template🌿<&>".into()),
        manager: Some("".into()),
        company: Some(" Company Keep ".into()),
        hyperlink_base: Some("https://example.test/base".into()),
        scale_crop: Some(true),
        links_up_to_date: Some(false),
        shared_doc: Some(true),
        hyperlinks_changed: Some(false),
    };
    assert_eq!(extended_properties(&model).unwrap(), expected);
    let mut other_state = model.clone();
    other_state.passthrough = base.passthrough.clone();
    assert_eq!(other_state, base);
    let written = wordcraft_docx::write(&model).unwrap();
    let changed_parts = package(&written);
    assert_eq!(
        original_parts.keys().collect::<Vec<_>>(),
        changed_parts.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        original_parts
            .iter()
            .filter(|(name, value)| changed_parts.get(*name) != Some(*value))
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["docProps/app.xml"]
    );
    for changes in [&[][..], &patches[..]] {
        patch_extended_properties(&mut model, changes).unwrap();
        assert_eq!(wordcraft_docx::write(&model).unwrap(), written);
    }
    let unchanged = model.clone();
    for (value, error) in [
        ("x".repeat(2049), ExtendedPropertiesError::Limit),
        (
            "\u{fffe}".into(),
            ExtendedPropertiesError::InvalidValue("Company"),
        ),
    ] {
        assert_eq!(
            patch_extended_properties(&mut model, &[P::Company(Some(value))]),
            Err(error)
        );
        assert_eq!(model, unchanged);
    }
    assert_eq!(wordcraft_docx::write(&base).unwrap(), original);
    let clear = [
        P::Template(None),
        P::Manager(None),
        P::Company(None),
        P::HyperlinkBase(None),
        P::ScaleCrop(None),
        P::LinksUpToDate(None),
        P::SharedDoc(None),
        P::HyperlinksChanged(None),
    ];
    let mut cleared = model.clone();
    patch_extended_properties(&mut cleared, &clear).unwrap();
    assert_eq!(cleared, base);
    assert_eq!(wordcraft_docx::write(&cleared).unwrap(), original);
    for _ in 0..3 {
        let bytes = wordcraft_docx::write(&model).unwrap();
        let reopened = read(&bytes, None).unwrap();
        assert_eq!(extended_properties(&reopened).unwrap(), expected);
        assert_eq!(reopened.core, base.core);
        assert_eq!(reopened.plain_text(StoryRef::Body), "ExtendedKeep");
        assert_eq!(
            reopened.para(StoryRef::Body, &Path::top(0)).unwrap().runs[0]
                .props
                .lang
                .as_deref(),
            Some("de-DE")
        );
        let flat = wordcraft_docx::write_flat(&reopened).unwrap();
        assert_eq!(
            extended_properties(&wordcraft_docx::read_xml(&flat).unwrap()).unwrap(),
            expected
        );
        let payload = extract(&bytes, None).unwrap();
        assert_eq!(payload.pages.len(), 1);
        assert_eq!(payload.pages[0].blocks[0].text, "ExtendedKeep");
        assert!(payload.pages[0]
            .links
            .iter()
            .any(|link| link.uri == "https://example.test/extended-keep"));
        let pdf = to_pdf(&bytes, None).unwrap();
        let parsed = crate::pdf::extract(&pdf, None).unwrap();
        assert_eq!(parsed.pages.len(), 1);
        assert!(parsed.pages[0]
            .blocks
            .iter()
            .any(|block| block.text == "ExtendedKeep"));
        assert!(parsed.pages[0]
            .links
            .iter()
            .any(|link| link.uri == "https://example.test/extended-keep"));
        assert_eq!(render(&bytes, 0, 72.0, None).unwrap(), original_png);
        assert_eq!(
            crate::pdf::render(&pdf, 0, 72.0, None).unwrap(),
            original_pdf_png
        );
        model = reopened;
    }
    let mut private = model.clone();
    private.settings.remove_personal_info = true;
    assert_eq!(extended_properties(&private).unwrap(), expected);
    let reopened = read(&wordcraft_docx::write(&private).unwrap(), None).unwrap();
    let mut filtered = expected;
    filtered.manager = None;
    filtered.company = None;
    assert_eq!(extended_properties(&reopened).unwrap(), filtered);
    assert_eq!(reopened.plain_text(StoryRef::Body), "ExtendedKeep");
}
