//! Serialize the bounded text layer using PdfCraft's embedded Unicode mappings.
use std::collections::HashMap;

use pdfcraft_content::{serialize_ops, Op};
use pdfcraft_cos::{Dict, Document, ObjRef, Object, PdfString, Stream};
use skrifa::{
    instance::{LocationRef, Size},
    FontRef, MetadataProvider,
};

use super::TextLayer;
use crate::EngineError;

impl TextLayer {
    pub(in crate::epub::rendition) fn attach(
        &self,
        document: &mut Document,
        pages: &[ObjRef],
        heights: impl Iterator<Item = f64>,
    ) -> Result<(), EngineError> {
        if !self
            .pages
            .iter()
            .any(|page| page.iter().any(|cluster| !cluster.text.trim().is_empty()))
        {
            return Ok(());
        }
        let bytes = *wordcraft_fonts::bundled()
            .first()
            .ok_or_else(|| conversion("missing bundled font"))?;
        let font = FontRef::new(bytes).map_err(conversion)?;
        let glyph = font
            .charmap()
            .map('H')
            .ok_or_else(|| conversion("missing bundled font glyph"))?;
        let location = LocationRef::default();
        let font_metrics = font.metrics(Size::new(1.0), location);
        let advance = font
            .glyph_metrics(Size::new(1.0), location)
            .advance_width(glyph)
            .filter(|width| width.is_finite() && *width > 0.0)
            .ok_or_else(|| conversion("invalid bundled font advance"))?;
        let ascent = f64::from(font_metrics.ascent);
        let height = f64::from(font_metrics.ascent - font_metrics.descent);
        if !height.is_finite() || height <= 0.0 {
            return Err(conversion("invalid bundled font height"));
        }

        // The invisible glyph supplies selection geometry. ToUnicode supplies all source text.
        // This never embeds a font from a publication or depends on a platform font.
        let mut codes = HashMap::<&str, u16>::new();
        let mut mappings = Vec::new();
        for cluster in self.pages.iter().flatten() {
            if cluster.text.len() > 4096 {
                return Err(super::super::super::limited(
                    "EPUB PDF text cluster exceeds 4096 bytes",
                ));
            }
            if let std::collections::hash_map::Entry::Vacant(entry) = codes.entry(&cluster.text) {
                if mappings.len() >= 65_534 {
                    return Err(super::super::super::limited(
                        "EPUB PDF text mapping exceeds 65534 clusters",
                    ));
                }
                mappings.push((cluster.text.clone(), glyph));
                entry.insert(mappings.len() as u16);
            }
        }

        let mut embedded = Dict::new();
        let key = pdfcraft_fonts::embedding::embed_mapping(
            document,
            &mut embedded,
            bytes,
            "SourceSans3",
            &mappings,
        )
        .map_err(conversion)?;
        let resource = embedded
            .get(key.as_bytes())
            .cloned()
            .ok_or_else(|| conversion("missing embedded text font"))?;
        for ((clusters, &reference), page_height) in self.pages.iter().zip(pages).zip(heights) {
            if clusters.is_empty() {
                continue;
            }
            let object = document.get(reference);
            let page = object
                .as_dict()
                .ok_or_else(|| conversion("missing generated PDF page"))?;
            let mut resources = page
                .get(b"Resources")
                .and_then(|r| document.dict(r))
                .unwrap_or_default();
            let mut fonts = resources
                .get(b"Font")
                .and_then(|r| document.dict(r))
                .unwrap_or_default();
            let name = (0..)
                .map(|i| format!("EPUBText{i}"))
                .find(|key| !fonts.contains(key.as_bytes()))
                .ok_or_else(|| conversion("missing PDF font resource name"))?;
            fonts.set(name.as_bytes().to_vec(), resource.clone());
            resources.set(b"Font".to_vec(), Object::Dict(fonts));

            let mut ops = vec![
                Op::new("q", vec![]),
                Op::new("BT", vec![]),
                Op::new("Tr", vec![Object::Int(3)]),
                Op::new("Tf", vec![Object::name(&name), Object::Int(1)]),
            ];
            for cluster in clusters {
                let rect = cluster.rect;
                let x = f64::from(rect.x);
                let sx = f64::from(rect.w) / f64::from(advance);
                let sy = f64::from(rect.h) / height;
                let baseline = page_height - (f64::from(rect.y) + ascent * sy);
                let matrix = [sx, 0.0, 0.0, sy, x, baseline].map(Object::Real);
                ops.push(Op::new("Tm", matrix.to_vec()));
                let code = codes[cluster.text.as_str()];
                ops.push(Op::new(
                    "Tj",
                    vec![Object::String(PdfString::literal(
                        code.to_be_bytes().to_vec(),
                    ))],
                ));
            }
            ops.push(Op::new("ET", vec![]));
            ops.push(Op::new("Q", vec![]));
            let mut contents = match page.get(b"Contents") {
                Some(content) => match &*document.resolve(content) {
                    Object::Array(items) => items.clone(),
                    _ => vec![content.clone()],
                },
                None => Vec::new(),
            };
            contents.push(Object::Ref(document.add(Object::Stream(Stream::flate(
                Dict::new(),
                &serialize_ops(&ops),
            )))));
            document
                .update_dict(reference, |dict| {
                    dict.set(b"Resources".to_vec(), Object::Dict(resources));
                    dict.set(b"Contents".to_vec(), Object::Array(contents));
                })
                .map_err(conversion)?;
        }
        Ok(())
    }
}

fn conversion(error: impl std::fmt::Display) -> EngineError {
    EngineError::Conversion(format!("EPUB fixed-layout PDF text: {error}"))
}
