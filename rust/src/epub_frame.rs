//! Archive-local XHTML replaced content and its recursive render bounds.
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
};

use roxmltree::Node;

use super::{
    content, external, invalid, limited, name, parse_xml, resolve, unsupported, XHTML, XML_BYTES,
};
use crate::{model::ImagePayload, EngineError};

pub(super) const MEDIA_TYPE: &str = "application/xhtml+xml";
const MAX_DEPTH: usize = 6;
const MAX_FRAMES: usize = 32;
const MAX_PIXELS: u64 = 32_000_000;
const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_DIMENSION: f32 = 8192.0;

pub(super) fn is_frame(node: Node<'_, '_>) -> bool {
    name(node, XHTML, "iframe")
        || name(node, XHTML, "object") && node.attribute("type") == Some(MEDIA_TYPE)
}

pub(super) fn payload(
    node: Node<'_, '_>,
    context: &mut content::Context<'_, '_>,
) -> Result<ImagePayload, EngineError> {
    let path = source_path(node, context.path, context.manifest)?;
    let source = context.book.read(&path, XML_BYTES)?;
    let xml = static_markup(&source)?;
    let mut targets = BTreeSet::new();
    if context.styles.is_none() {
        context.budget.frame()?;
        let mut ancestors = vec![context.path.to_owned(), path.clone()];
        collect_targets(
            xml.root_element(),
            &path,
            context,
            &mut ancestors,
            &mut targets,
        )?;
    }
    let dimension = |key, default| {
        node.attribute(key).map_or(Ok(default), |value| {
            let size = value
                .trim()
                .parse::<f32>()
                .map_err(|_| invalid("invalid embedded EPUB XHTML dimensions"))?;
            if !size.is_finite() || size <= 0.0 {
                return Err(invalid("invalid embedded EPUB XHTML dimensions"));
            }
            if size > MAX_DIMENSION {
                return Err(limited("embedded EPUB XHTML dimensions exceed 8192 pixels"));
            }
            Ok(size)
        })
    };
    let size = [dimension("width", 300.0)?, dimension("height", 150.0)?];
    let label = node.attribute("title").unwrap_or_default();
    let mut payload = content::encoded_image(
        &source,
        context.page,
        &path,
        MEDIA_TYPE,
        label,
        context.budget,
    )?;
    context
        .budget
        .bytes(context.path.len().saturating_mul(3) + 96)?;
    payload
        .metadata
        .insert("frame_viewport".into(), serde_json::json!(size));
    payload
        .metadata
        .insert("frame_parent".into(), serde_json::json!(context.path));
    if !targets.is_empty() {
        payload
            .metadata
            .insert("epub_frame_targets".into(), serde_json::json!(targets));
    }
    Ok(payload)
}

fn source_path(
    node: Node<'_, '_>,
    base: &str,
    manifest: &HashMap<String, &str>,
) -> Result<String, EngineError> {
    if node.has_attribute("srcdoc") {
        return Err(unsupported("EPUB iframe srcdoc"));
    }
    let uri = content::embedded_document_target(node)
        .map(str::trim)
        .filter(|uri| !uri.is_empty())
        .ok_or_else(|| invalid("embedded EPUB XHTML requires a source"))?;
    if external(uri) {
        return Err(unsupported("remote embedded EPUB XHTML"));
    }
    if uri
        .split_once('#')
        .is_some_and(|(_, fragment)| !fragment.is_empty())
    {
        return Err(unsupported("embedded EPUB XHTML fragment scrolling"));
    }
    let path = resolve(base, uri)?;
    if path == base {
        return Err(invalid("cyclic embedded EPUB XHTML"));
    }
    if manifest.get(&path) != Some(&MEDIA_TYPE) {
        return Err(unsupported(
            "embedded EPUB XHTML requires a local XHTML manifest resource",
        ));
    }
    Ok(path)
}

fn static_markup(source: &[u8]) -> Result<roxmltree::Document<'_>, EngineError> {
    let xml = parse_xml(source)?;
    let root = xml.root_element();
    if !name(root, XHTML, "html") || super::child(root, XHTML, "body").is_none() {
        return Err(invalid("embedded EPUB XHTML requires a document body"));
    }
    if root.descendants().filter(Node::is_element).any(|node| {
        name(node, XHTML, "script")
            || node
                .attributes()
                .any(|attr| attr.namespace().is_none() && attr.name().starts_with("on"))
    }) {
        return Err(unsupported("scripted embedded EPUB XHTML"));
    }
    Ok(xml)
}

fn collect_targets(
    root: Node<'_, '_>,
    path: &str,
    context: &mut content::Context<'_, '_>,
    ancestors: &mut Vec<String>,
    targets: &mut BTreeSet<(String, String)>,
) -> Result<(), EngineError> {
    for node in root.descendants().filter(Node::is_element) {
        if is_frame(node) {
            if ancestors.len() > MAX_DEPTH {
                return Err(limited(
                    "embedded EPUB XHTML exceeds its frame depth budget",
                ));
            }
            let next = source_path(node, path, context.manifest)?;
            if ancestors.contains(&next) {
                return Err(invalid("cyclic embedded EPUB XHTML"));
            }
            context.budget.frame()?;
            let source = context.book.read(&next, XML_BYTES)?;
            let xml = static_markup(&source)?;
            ancestors.push(next.clone());
            collect_targets(xml.root_element(), &next, context, ancestors, targets)?;
            ancestors.pop();
        } else if let Some(href) = content::hyperlink_target(node).filter(|href| !external(href)) {
            if let Some((_, id)) = href.split_once('#').filter(|(_, id)| !id.is_empty()) {
                let target = (resolve(path, href)?, super::percent_decode(id)?);
                context.budget.bytes(
                    target
                        .0
                        .len()
                        .saturating_add(target.1.len())
                        .saturating_mul(3)
                        + 64,
                )?;
                targets.insert(target);
            }
        }
    }
    Ok(())
}

pub(super) fn charge_count(count: &mut usize) -> Result<(), EngineError> {
    super::charge(
        count,
        1,
        MAX_FRAMES,
        "embedded EPUB XHTML exceeds its frame count budget",
    )
}

pub(super) fn size(payload: &ImagePayload) -> Result<[f32; 2], EngineError> {
    serde_json::from_value(payload.metadata["frame_viewport"].clone())
        .map_err(|_| invalid("missing embedded EPUB XHTML viewport"))
}

#[derive(Default)]
pub(super) struct Work {
    paths: RefCell<Vec<String>>,
    frames: Cell<usize>,
    pixels: Cell<u64>,
    bytes: Cell<usize>,
}

impl Work {
    pub(super) fn enter(&self, path: &str, pixels: u64) -> Result<Scope<'_>, EngineError> {
        let mut paths = self.paths.borrow_mut();
        if paths.iter().any(|ancestor| ancestor == path) {
            return Err(invalid("cyclic embedded EPUB XHTML"));
        }
        let frames = self.frames.get() + 1;
        let total = self.pixels.get().saturating_add(pixels);
        if paths.len() >= MAX_DEPTH || frames > MAX_FRAMES || total > MAX_PIXELS {
            return Err(limited("embedded EPUB XHTML exceeds its frame work budget"));
        }
        self.frames.set(frames);
        self.pixels.set(total);
        paths.push(path.to_owned());
        Ok(Scope(self))
    }

    pub(super) fn charge_read(&self, bytes: usize) -> Result<(), EngineError> {
        let mut total = self.bytes.get();
        super::charge(
            &mut total,
            bytes,
            MAX_BYTES,
            "embedded EPUB XHTML exceeds its read budget",
        )?;
        self.bytes.set(total);
        Ok(())
    }
}

pub(super) struct Scope<'a>(&'a Work);

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.0.paths.borrow_mut().pop();
    }
}
