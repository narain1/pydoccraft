//! Validate morphology kernels and bound native neighborhood sampling.
use roxmltree::Node;

use super::finite_number;
use crate::{
    epub::{invalid, limited, parse_xml, structure::apply_edits, svg, unsupported, XML_BYTES},
    EngineError,
};

const MAX_RADIUS: f32 = 64.0;

fn radius(node: Node<'_, '_>) -> Result<[f32; 2], EngineError> {
    let Some(raw) = node.attribute("radius") else {
        return Ok([0.0; 2]);
    };
    let error = "invalid SVG morphology radius";
    if !raw.is_ascii() {
        return Err(invalid(error));
    }
    if let Some((x, y)) = raw.split_once(',') {
        return Ok([finite_number(x, error)?, finite_number(y, error)?]);
    }
    let mut values = raw.split_ascii_whitespace();
    let x = finite_number(values.next().ok_or_else(|| invalid(error))?, error)?;
    let y = values.next().map_or(Ok(x), |y| finite_number(y, error))?;
    if values.next().is_some() {
        return Err(invalid(error));
    }
    Ok([x, y])
}

pub(super) fn validate(node: Node<'_, '_>) -> Result<(), EngineError> {
    if !matches!(node.attribute("operator"), None | Some("erode" | "dilate")) {
        return Err(unsupported("unsupported SVG morphology operator"));
    }
    let radii = radius(node)?;
    if radii.into_iter().any(|value| value <= 0.0) {
        return Ok(());
    }
    if radii.into_iter().any(|value| !value.is_normal()) {
        return Err(unsupported(
            "SVG morphology radii require representable positive numbers",
        ));
    }
    if radii.into_iter().any(|value| value > MAX_RADIUS) {
        return Err(limited("SVG morphology radius exceeds 64"));
    }
    if node
        .parent_element()
        .and_then(|filter| filter.attribute("primitiveUnits"))
        == Some("objectBoundingBox")
        && radii.into_iter().any(|value| value > 1.0)
    {
        // Keep the native multiplication by an unparsed source bbox finite.
        return Err(limited(
            "SVG morphology objectBoundingBox radius exceeds one",
        ));
    }
    Ok(())
}

pub(in crate::epub::svg) fn normalize(source: Vec<u8>) -> Result<Vec<u8>, EngineError> {
    let xml = parse_xml(&source)?;
    let mut edits = Vec::new();
    for node in xml
        .descendants()
        .filter(|node| node.has_tag_name((svg::NS, "feMorphology")))
    {
        if radius(node)?.into_iter().all(|value| value > 0.0) {
            continue;
        }
        // The native morphology parser substitutes a positive radius. A
        // zero-deviation Gaussian primitive retains the input and its region.
        // Keep the prefix, result name, input and standalone author markup.
        let end = svg::style_insertion(&source, node)?.start;
        edits.push((end - "feMorphology".len()..end, "feGaussianBlur".into()));
        for attribute in ["operator", "radius"] {
            if let Some(attribute) = node.attribute_node(attribute) {
                edits.push((attribute.range(), String::new()));
            }
        }
        let node_range = node.range();
        // A nonempty primitive can contain comments or descriptive whitespace.
        if source[node_range.end - 2] != b'/' {
            let tag = b"feMorphology";
            let tail = &source[end..node_range.end];
            let start = tail
                .windows(tag.len())
                .rposition(|name| name == tag)
                .ok_or_else(|| invalid("invalid SVG morphology closing tag"))?
                + end;
            edits.push((start..start + tag.len(), "feGaussianBlur".into()));
        }
    }
    if edits.is_empty() {
        return Ok(source);
    }
    let source = std::str::from_utf8(&source).map_err(invalid)?;
    Ok(apply_edits(source, edits, XML_BYTES)?.into_bytes())
}

pub(super) fn work(radii: [f32; 2], layer: [usize; 2]) -> Result<usize, EngineError> {
    if radii
        .into_iter()
        .any(|value| !value.is_finite() || value <= 0.0 || value > MAX_RADIUS)
    {
        return Err(limited(
            "SVG morphology radius is outside its device budget after scaling",
        ));
    }
    // Charge all four channels for every sampled neighbor. Rounding upward
    // also covers native rounding near a fractional transformed radius.
    let [x, y] = radii.map(|value| value.ceil() as usize);
    let kernel = (2 * x + 1).min(layer[0]) * (2 * y + 1).min(layer[1]);
    Ok(8 + 4 * kernel)
}
