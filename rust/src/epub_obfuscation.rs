//! Reverse the EPUB OCF font-only obfuscation of a local font while rendering.
use std::collections::HashSet;

use sha1::{Digest, Sha1};

use super::{
    attribute, child, name, parse_xml, required, resolve, unsupported, Book, DC, METADATA_BYTES,
    OCF, OPF,
};
use crate::EngineError;

const XMLENC: &str = "http://www.w3.org/2001/04/xmlenc#";
const IDPF: &str = "http://www.idpf.org/2008/embedding";

pub(super) struct FontObfuscation {
    paths: HashSet<String>,
    key: [u8; 20],
}

impl FontObfuscation {
    pub(super) fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }

    pub(super) fn decode(&self, path: &str, bytes: &mut [u8]) {
        if self.contains(path) {
            for (output, mask) in bytes.iter_mut().take(1040).zip(self.key.iter().cycle()) {
                *output ^= *mask;
            }
        }
    }
}

pub(super) fn load(
    book: &mut Book<'_>,
    encryption_path: &str,
) -> Result<FontObfuscation, EngineError> {
    let encrypted = || unsupported("encrypted resources and DRM are not supported");
    let encryption = book.read(encryption_path, METADATA_BYTES)?;
    let xml = parse_xml(&encryption)?;
    let root = xml.root_element();
    if root.tag_name().namespace() != Some(OCF) || root.tag_name().name() != "encryption" {
        return Err(encrypted());
    }
    let mut paths = HashSet::new();
    for entry in root.children().filter(roxmltree::Node::is_element) {
        if !entry.has_tag_name((XMLENC, "EncryptedData"))
            || entry.children().filter(roxmltree::Node::is_element).count() != 2
        {
            return Err(encrypted());
        }
        let method = child(entry, XMLENC, "EncryptionMethod").ok_or_else(encrypted)?;
        let cipher = child(entry, XMLENC, "CipherData").ok_or_else(encrypted)?;
        let reference = child(cipher, XMLENC, "CipherReference").ok_or_else(encrypted)?;
        if method.attribute("Algorithm") != Some(IDPF)
            || method.children().any(|node| node.is_element())
            || cipher
                .children()
                .filter(roxmltree::Node::is_element)
                .count()
                != 1
            || reference.children().any(|node| node.is_element())
        {
            return Err(encrypted());
        }
        let uri = attribute(reference, "URI")?;
        if uri.is_empty() || uri.contains(['#', '?']) {
            return Err(encrypted());
        }
        let path = resolve("container.xml", uri)?;
        if book.archive.index_for_name(&path).is_none() || !paths.insert(path) {
            return Err(encrypted());
        }
    }
    if paths.is_empty() {
        return Err(encrypted());
    }
    let package_path = book.package_path()?;
    let package = book.read(&package_path, METADATA_BYTES)?;
    let package = parse_xml(&package)?;
    let root = package.root_element();
    let metadata = required(root, "metadata")?;
    let manifest = required(root, "manifest")?;
    let identifier_ref = attribute(root, "unique-identifier")?;
    let identifier = metadata
        .children()
        .find(|node| {
            node.has_tag_name((DC, "identifier")) && node.attribute("id") == Some(identifier_ref)
        })
        .ok_or_else(encrypted)?;
    if identifier.children().any(|node| node.is_element()) {
        return Err(encrypted());
    }
    let mut font_paths = HashSet::new();
    for item in manifest.children().filter(|node| name(*node, OPF, "item")) {
        let href = attribute(item, "href")?;
        if super::external(href) {
            continue;
        }
        let path = resolve(&package_path, href)?;
        if paths.contains(&path) {
            if !is_font(attribute(item, "media-type")?) {
                return Err(encrypted());
            }
            font_paths.insert(path);
        }
    }
    if paths != font_paths {
        return Err(encrypted());
    }
    let mut ident = String::new();
    for piece in identifier
        .children()
        .filter(roxmltree::Node::is_text)
        .filter_map(|node| node.text())
    {
        ident.extend(
            piece
                .chars()
                .filter(|ch| !matches!(ch, ' ' | '\t' | '\n' | '\r')),
        );
    }
    if ident.is_empty() {
        return Err(encrypted());
    }
    Ok(FontObfuscation {
        paths,
        key: Sha1::digest(ident.as_bytes()).into(),
    })
}

fn is_font(media: &str) -> bool {
    matches!(
        media,
        "font/ttf"
            | "font/otf"
            | "font/woff"
            | "font/woff2"
            | "font/collection"
            | "application/font-sfnt"
            | "application/font-woff"
            | "application/vnd.ms-opentype"
            | "application/x-font-ttf"
            | "application/x-font-opentype"
    )
}
