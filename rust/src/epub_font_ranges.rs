//! Apply the CSS character range to in-memory copies of packaged font faces.
use std::collections::BTreeMap;

use skrifa::{charmap::MapVariant, FontRef, MetadataProvider};
use write_fonts::{
    dump_table,
    read::{FontRef as TableFont, TableProvider},
    tables::cmap::{
        Cmap, Cmap12, Cmap14, CmapSubtable, DefaultUvs, EncodingRecord, NonDefaultUvs, PlatformId,
        SequentialMapGroup, UnicodeRange, UvsMapping, VariationSelector,
    },
    types::Uint24,
    FontBuilder,
};

use super::{charge, invalid, limited, EngineError, MAX_FONT_BYTES};

const MAPPING_LIMIT: usize = 2 * 0x110000;
// The SFNT directory stores its binary-search range in 16 bits.
const MAX_SFNT_TABLES: usize = u16::MAX as usize / 16;

pub(in crate::epub) enum RewriteError {
    ByteLimit,
    Engine(EngineError),
}

impl From<EngineError> for RewriteError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl RewriteError {
    pub(in crate::epub) fn into_engine(self) -> EngineError {
        match self {
            Self::ByteLimit => limited("EPUB Unicode-restricted font exceeds 8 MiB"),
            Self::Engine(error) => error,
        }
    }
}

fn reserve_font_bytes(rewritten: &mut usize, amount: usize) -> Result<(), RewriteError> {
    *rewritten = rewritten
        .checked_add(amount)
        .filter(|&bytes| bytes <= MAX_FONT_BYTES)
        .ok_or(RewriteError::ByteLimit)?;
    Ok(())
}

pub(in crate::epub) fn register(
    data: &[u8],
    ranges: &[(u32, u32)],
    work: &mut usize,
    mut register: impl FnMut(Vec<u8>) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    let covered = |cp| {
        ranges
            .get(ranges.partition_point(|&(_, end)| end < cp))
            .is_some_and(|&(first, _)| first <= cp)
    };
    let mut fonts = TableFont::fonts(data).enumerate().peekable();
    if fonts.peek().is_none() {
        return Err(invalid("EPUB font-face has no usable font"));
    }
    let mut rewritten = 0;
    for (index, source) in fonts {
        if index >= 256 {
            return Err(limited("EPUB font collection exceeds 256 faces"));
        }
        if let Some(bytes) = rewrite(
            source.map_err(invalid)?,
            FontRef::from_index(data, index as u32).map_err(invalid)?,
            &covered,
            work,
            &mut rewritten,
        )
        .map_err(RewriteError::into_engine)?
        {
            register(bytes)?;
        }
    }
    Ok(())
}

pub(in crate::epub) fn filter_face(
    data: &[u8],
    index: u32,
    covered: impl Fn(u32) -> bool,
    work: &mut usize,
) -> Result<Option<Vec<u8>>, RewriteError> {
    rewrite(
        TableFont::from_index(data, index).map_err(invalid)?,
        FontRef::from_index(data, index).map_err(invalid)?,
        &covered,
        work,
        &mut 0,
    )
}

fn rewrite(
    source: TableFont<'_>,
    font: FontRef<'_>,
    covered: &dyn Fn(u32) -> bool,
    work: &mut usize,
    rewritten: &mut usize,
) -> Result<Option<Vec<u8>>, RewriteError> {
    if source.table_directory().table_records().len() > MAX_SFNT_TABLES {
        return Err(limited("EPUB font table count exceeds the SFNT directory limit").into());
    }
    let chars = font.charmap();
    let mut groups: Vec<SequentialMapGroup> = Vec::new();
    for (code, glyph) in chars.mappings() {
        charge(
            work,
            1,
            MAPPING_LIMIT,
            "EPUB font character map work limit exceeded",
        )?;
        let glyph = glyph.to_u32();
        if glyph == 0
            || glyph > u32::from(u16::MAX)
            || char::from_u32(code).is_none()
            || !covered(code)
        {
            continue;
        }
        if let Some(last) = groups.last_mut().filter(|last| {
            last.end_char_code + 1 == code
                && last.start_glyph_id + (code - last.start_char_code) == glyph
        }) {
            last.end_char_code = code;
        } else {
            groups.push(SequentialMapGroup::new(code, code, glyph));
        }
    }
    let mut variants: BTreeMap<u32, (Vec<UnicodeRange>, Vec<UvsMapping>)> = BTreeMap::new();
    for (code, selector, glyph) in chars.variant_mappings() {
        charge(
            work,
            1,
            MAPPING_LIMIT,
            "EPUB font character map work limit exceeded",
        )?;
        if !covered(code) || char::from_u32(code).is_none() || char::from_u32(selector).is_none() {
            continue;
        }
        let (default, mapped) = variants.entry(selector).or_default();
        match glyph {
            MapVariant::UseDefault => {
                if let Some(last) = default.last_mut().filter(|last| {
                    last.additional_count < u8::MAX
                        && last.start_unicode_value.to_u32() + u32::from(last.additional_count) + 1
                            == code
                }) {
                    last.additional_count += 1;
                } else {
                    default.push(UnicodeRange::new(Uint24::new(code), 0));
                }
            }
            MapVariant::Variant(id) => {
                let id = u16::try_from(id.to_u32()).map_err(invalid)?;
                mapped.push(UvsMapping::new(Uint24::new(code), id));
            }
        }
    }
    if groups.is_empty() && variants.is_empty() {
        return Ok(None);
    }
    let full = CmapSubtable::Format12(Cmap12::new(0, groups));
    let mut records = vec![EncodingRecord::new(PlatformId::Unicode, 4, full.clone())];
    if !variants.is_empty() {
        let selectors: Vec<_> = variants
            .into_iter()
            .map(|(selector, (default, mapped))| {
                VariationSelector::new(
                    Uint24::new(selector),
                    (!default.is_empty()).then(|| DefaultUvs::new(default.len() as u32, default)),
                    (!mapped.is_empty()).then(|| NonDefaultUvs::new(mapped.len() as u32, mapped)),
                )
            })
            .collect();
        let mut table = Cmap14::new(0, selectors.len() as u32, selectors);
        table.length = dump_table(&table).map_err(invalid)?.len() as u32;
        records.push(EncodingRecord::new(
            PlatformId::Unicode,
            5,
            CmapSubtable::Format14(table),
        ));
    }
    records.push(EncodingRecord::new(PlatformId::Windows, 10, full));
    let cmap = dump_table(&Cmap::new(records)).map_err(invalid)?;
    let mut builder = FontBuilder::new();
    reserve_font_bytes(rewritten, cmap.len() + 32)?;
    builder.add_raw(write_fonts::types::Tag::new(b"cmap"), cmap);
    for record in source.table_directory().table_records() {
        let tag = record.tag();
        if matches!(
            tag.to_be_bytes(),
            [b'c', b'm', b'a', b'p'] | [b'D', b'S', b'I', b'G']
        ) {
            continue;
        }
        let bytes = source
            .data_for_tag(tag)
            .ok_or_else(|| invalid("invalid EPUB font table"))?;
        reserve_font_bytes(rewritten, bytes.len() + 19)?;
        builder.add_raw(tag, bytes.as_bytes());
    }
    Ok(Some(builder.build()))
}
