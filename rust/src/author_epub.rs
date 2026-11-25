use std::io::{Cursor, Seek, SeekFrom, Write};

use crate::EngineError;

const MAX_CHAPTERS: usize = 10_000;
const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGES: usize = 10_000;
const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_MEDIA_BYTES: usize = 64 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone)]
struct Chapter {
    title: String,
    body: String,
}

#[derive(Clone)]
struct ImageResource {
    href: String,
    data: Vec<u8>,
}

#[derive(Clone)]
pub struct EditableEpub {
    title: String,
    identifier: String,
    language: String,
    authors: Vec<String>,
    publication_date: Option<String>,
    chapters: Vec<Chapter>,
    images: Vec<ImageResource>,
    content_bytes: usize,
    media_bytes: usize,
    closed: bool,
}

impl EditableEpub {
    pub fn new() -> Self {
        Self {
            title: String::new(),
            identifier: String::new(),
            language: "en".into(),
            authors: Vec::new(),
            publication_date: None,
            chapters: Vec::new(),
            images: Vec::new(),
            content_bytes: 0,
            media_bytes: 0,
            closed: false,
        }
    }

    pub fn copy(&self) -> Result<Self, EngineError> {
        self.ensure_open()?;
        Ok(Clone::clone(self))
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.chapters.clear();
        self.images.clear();
        self.content_bytes = 0;
        self.media_bytes = 0;
    }

    pub fn set_metadata(
        &mut self,
        title: String,
        identifier: String,
        language: String,
        authors: Vec<String>,
    ) -> Result<(), EngineError> {
        self.ensure_open()?;
        validate_text(&title, 65_536, "title")?;
        validate_text(&identifier, 4_096, "identifier")?;
        validate_text(&language, 128, "language")?;
        if authors.len() > 1_000 {
            return Err(EngineError::ResourceLimit(
                "EPUB authors exceed 1000".into(),
            ));
        }
        for author in &authors {
            validate_text(author, 65_536, "author")?;
        }
        if title.len()
            + identifier.len()
            + language.len()
            + authors.iter().map(String::len).sum::<usize>()
            > 128 * 1024
        {
            return Err(EngineError::ResourceLimit(
                "EPUB metadata exceeds 128 KiB".into(),
            ));
        }
        self.title = title;
        self.identifier = identifier;
        self.language = language;
        self.authors = authors;
        Ok(())
    }

    pub fn set_publication_date(&mut self, date: Option<String>) -> Result<(), EngineError> {
        self.ensure_open()?;
        if let Some(value) = &date {
            validate_text(value, 128, "publication date")?;
        }
        self.publication_date = date;
        Ok(())
    }

    pub fn add_chapter(&mut self, title: String) -> Result<usize, EngineError> {
        self.ensure_open()?;
        validate_text(&title, 65_536, "chapter title")?;
        if self.chapters.len() >= MAX_CHAPTERS {
            return Err(EngineError::ResourceLimit(
                "EPUB chapters exceed 10000".into(),
            ));
        }
        let next = self
            .content_bytes
            .checked_add(title.len() * 6)
            .filter(|size| *size <= MAX_CONTENT_BYTES)
            .ok_or_else(|| {
                EngineError::ResourceLimit("EPUB chapter titles exceed the content budget".into())
            })?;
        self.chapters.push(Chapter {
            title,
            body: String::new(),
        });
        self.content_bytes = next;
        Ok(self.chapters.len() - 1)
    }

    pub fn add_heading(
        &mut self,
        chapter: usize,
        text: String,
        level: u8,
    ) -> Result<(), EngineError> {
        if !(1..=6).contains(&level) {
            return Err(EngineError::InvalidArgument(
                "EPUB heading level must be between 1 and 6".into(),
            ));
        }
        validate_text(&text, 65_536, "heading text")?;
        let fragment = format!("<h{level}>{}</h{level}>\n", escape_text(&text));
        self.append(chapter, &fragment)
    }

    pub fn add_paragraph(&mut self, chapter: usize, text: String) -> Result<(), EngineError> {
        validate_text(&text, 65_536, "paragraph text")?;
        let fragment = format!("<p>{}</p>\n", escape_text(&text));
        self.append(chapter, &fragment)
    }

    pub fn add_list(
        &mut self,
        chapter: usize,
        items: Vec<String>,
        ordered: bool,
    ) -> Result<(), EngineError> {
        let tag = if ordered { "ol" } else { "ul" };
        if items.len() > 10_000 {
            return Err(EngineError::ResourceLimit(
                "EPUB list exceeds 10000 items".into(),
            ));
        }
        let mut fragment = format!("<{tag}>");
        for item in items {
            push_item(&mut fragment, &item, "li")?;
        }
        fragment.push_str(&format!("</{tag}>\n"));
        self.append(chapter, &fragment)
    }

    pub fn add_table(
        &mut self,
        chapter: usize,
        rows: Vec<Vec<String>>,
        header: bool,
    ) -> Result<(), EngineError> {
        let width = rows.first().map_or(0, Vec::len);
        if width == 0
            || width > 1000
            || rows.len().saturating_mul(width) > 10_000
            || rows.iter().any(|row| row.len() != width)
        {
            return Err(EngineError::InvalidArgument(
                "EPUB table must be a nonempty rectangular grid of at most 10000 cells".into(),
            ));
        }
        let mut fragment = "<table>".to_string();
        for (index, row) in rows.iter().enumerate() {
            fragment.push_str("<tr>");
            let tag = if header && index == 0 { "th" } else { "td" };
            for item in row {
                push_item(&mut fragment, item, tag)?;
            }
            fragment.push_str("</tr>");
        }
        fragment.push_str("</table>\n");
        self.append(chapter, &fragment)
    }

    pub fn add_preformatted(&mut self, chapter: usize, text: String) -> Result<(), EngineError> {
        let mut fragment = String::new();
        push_item(&mut fragment, &text, "pre")?;
        self.append(chapter, &fragment)
    }

    pub fn add_link(
        &mut self,
        chapter: usize,
        text: String,
        href: String,
    ) -> Result<(), EngineError> {
        validate_text(&text, 65_536, "link text")?;
        validate_link(&href)?;
        let fragment = format!(
            "<p><a href=\"{}\">{}</a></p>\n",
            escape_attribute(&href),
            escape_text(&text)
        );
        self.append(chapter, &fragment)
    }

    pub fn add_image(
        &mut self,
        chapter: usize,
        data: Vec<u8>,
        media_type: String,
        alt: String,
    ) -> Result<(), EngineError> {
        self.ensure_open()?;
        if self.images.len() >= MAX_IMAGES {
            return Err(EngineError::ResourceLimit(
                "EPUB images exceed 10000".into(),
            ));
        }
        if data.is_empty() || data.len() > MAX_IMAGE_BYTES {
            return Err(EngineError::ResourceLimit(
                "EPUB image must contain at most 16 MiB".into(),
            ));
        }
        let next_media = self
            .media_bytes
            .checked_add(data.len())
            .filter(|size| *size <= MAX_MEDIA_BYTES)
            .ok_or_else(|| EngineError::ResourceLimit("EPUB images exceed 64 MiB".into()))?;
        validate_text(&alt, 65_536, "image alt text")?;
        let extension = validate_image(&data, &media_type)?;
        let href = format!("images/image-{}.{}", self.images.len() + 1, extension);
        let fragment = format!(
            "<p><img src=\"{}\" alt=\"{}\" /></p>\n",
            href,
            escape_attribute(&alt)
        );
        let next_content = self.checked_append(chapter, &fragment)?;
        self.images.try_reserve(1).map_err(|_| {
            EngineError::ResourceLimit("cannot allocate bounded EPUB image storage".into())
        })?;
        self.chapters[chapter]
            .body
            .try_reserve(fragment.len())
            .map_err(|_| {
                EngineError::ResourceLimit("cannot allocate bounded EPUB chapter storage".into())
            })?;
        self.chapters[chapter].body.push_str(&fragment);
        self.images.push(ImageResource { href, data });
        self.content_bytes = next_content;
        self.media_bytes = next_media;
        Ok(())
    }

    pub fn chapter_count(&self) -> Result<usize, EngineError> {
        self.ensure_open()?;
        Ok(self.chapters.len())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, EngineError> {
        self.ensure_open()?;
        if self.title.trim().is_empty()
            || self.identifier.trim().is_empty()
            || self.language.trim().is_empty()
        {
            return Err(EngineError::InvalidArgument(
                "EPUB title, identifier and language are required".into(),
            ));
        }
        if self.chapters.is_empty() {
            return Err(EngineError::InvalidArgument(
                "EPUB requires at least one chapter".into(),
            ));
        }
        let metadata_bytes = self.authors.iter().try_fold(
            self.title
                .len()
                .checked_add(self.identifier.len())
                .and_then(|size| size.checked_add(self.language.len()))
                .ok_or_else(|| EngineError::ResourceLimit("EPUB metadata size overflow".into()))?,
            |size, author| {
                size.checked_add(author.len())
                    .ok_or_else(|| EngineError::ResourceLimit("EPUB metadata size overflow".into()))
            },
        )?;
        self.content_bytes
            .checked_add(self.media_bytes)
            .and_then(|size| size.checked_add(metadata_bytes))
            .and_then(|size| size.checked_add(self.chapters.len().checked_mul(4096)?))
            .and_then(|size| size.checked_add(self.images.len().checked_mul(1024)?))
            .and_then(|size| size.checked_add(1024 * 1024))
            .filter(|size| *size <= MAX_OUTPUT_BYTES)
            .ok_or_else(|| EngineError::ResourceLimit("EPUB output exceeds 128 MiB".into()))?;

        let mut builder = rbook::Epub::builder()
            .identifier(self.identifier.as_str())
            .title(self.title.as_str())
            .language(self.language.as_str());
        for author in &self.authors {
            builder = builder.author(author.as_str());
        }
        for image in &self.images {
            builder = builder.resource((image.href.as_str(), image.data.as_slice()));
        }
        for (index, chapter) in self.chapters.iter().enumerate() {
            builder = builder.chapter(
                rbook::epub::EpubChapter::new(chapter.title.as_str())
                    .href(format!("chapter-{}.xhtml", index + 1))
                    .xhtml_body(chapter.body.as_str()),
            );
        }
        if let Some(date) = &self.publication_date {
            builder = builder.published_date(date.as_str());
        }
        let mut output = BoundedOutput::default();
        if let Err(error) = builder.build().write().write(&mut output) {
            return Err(if output.exceeded {
                EngineError::ResourceLimit(
                    "EPUB output exceeds 128 MiB or cannot be allocated".into(),
                )
            } else {
                EngineError::Conversion(format!("EPUB writing failed: {error}"))
            });
        }
        Ok(output.into_bytes())
    }
}

impl Default for EditableEpub {
    fn default() -> Self {
        Self::new()
    }
}

impl EditableEpub {
    fn ensure_open(&self) -> Result<(), EngineError> {
        if self.closed {
            Err(EngineError::Closed("EPUB document is closed".into()))
        } else {
            Ok(())
        }
    }

    fn checked_append(&self, chapter: usize, fragment: &str) -> Result<usize, EngineError> {
        self.ensure_open()?;
        if chapter >= self.chapters.len() {
            return Err(EngineError::InvalidArgument(
                "EPUB chapter does not exist".into(),
            ));
        }
        self.content_bytes
            .checked_add(fragment.len())
            .filter(|size| *size <= MAX_CONTENT_BYTES)
            .ok_or_else(|| EngineError::ResourceLimit("EPUB content exceeds 64 MiB".into()))
    }

    fn append(&mut self, chapter: usize, fragment: &str) -> Result<(), EngineError> {
        let next = self.checked_append(chapter, fragment)?;
        self.chapters[chapter].body.push_str(fragment);
        self.content_bytes = next;
        Ok(())
    }
}

pub(crate) fn validate_text(value: &str, limit: usize, name: &str) -> Result<(), EngineError> {
    let valid_xml = value.chars().all(|character| {
        matches!(character, '\u{9}' | '\u{A}' | '\u{D}')
            || ('\u{20}'..='\u{D7FF}').contains(&character)
            || ('\u{E000}'..='\u{FFFD}').contains(&character)
            || ('\u{10000}'..='\u{10FFFF}').contains(&character)
    });
    if value.len() > limit || !valid_xml {
        Err(EngineError::InvalidArgument(format!(
            "EPUB {name} is invalid or exceeds {limit} bytes"
        )))
    } else {
        Ok(())
    }
}

fn validate_link(href: &str) -> Result<(), EngineError> {
    validate_text(href, 16_384, "link")?;
    let lower = href.to_ascii_lowercase();
    if lower.starts_with("https://")
        || lower.starts_with("http://")
        || lower.starts_with("mailto:")
        || (href.starts_with('#') && !href.contains(".."))
    {
        Ok(())
    } else {
        Err(EngineError::Unsupported(
            "EPUB links require http, https, mailto or a safe fragment".into(),
        ))
    }
}

pub(crate) fn validate_image(data: &[u8], media_type: &str) -> Result<&'static str, EngineError> {
    let valid = match media_type {
        "image/png" => data.starts_with(b"\x89PNG\r\n\x1a\n").then_some("png"),
        "image/jpeg" => {
            (data.starts_with(&[0xff, 0xd8]) && data.ends_with(&[0xff, 0xd9])).then_some("jpg")
        }
        "image/gif" => {
            (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")).then_some("gif")
        }
        "image/svg+xml" => {
            crate::epub::validate_svg_resource(data).map_err(|error| match error {
                EngineError::Malformed(message) => EngineError::InvalidArgument(message),
                other => other,
            })?;
            Some("svg")
        }
        "image/webp" => {
            (data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP")).then_some("webp")
        }
        _ => {
            return Err(EngineError::Unsupported(format!(
                "EPUB image type {media_type} is not supported"
            )))
        }
    };
    valid.ok_or_else(|| {
        EngineError::InvalidArgument(format!(
            "EPUB image bytes do not match declared type {media_type}"
        ))
    })
}

pub(crate) fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\r', "&#13;")
}

pub(crate) fn escape_attribute(value: &str) -> String {
    escape_text(value)
        .replace('"', "&quot;")
        .replace('\t', "&#9;")
        .replace('\n', "&#10;")
}

fn push_item(fragment: &mut String, text: &str, tag: &str) -> Result<(), EngineError> {
    validate_text(text, 65_536, "content")?;
    let escaped = escape_text(text);
    if fragment.len() + escaped.len() + tag.len() * 2 + 5 > 4 * 1024 * 1024 {
        return Err(EngineError::ResourceLimit(
            "EPUB structure exceeds its content budget".into(),
        ));
    }
    fragment.push_str(&format!("<{tag}>{escaped}</{tag}>"));
    Ok(())
}

#[derive(Default)]
pub(crate) struct BoundedOutput {
    inner: Cursor<Vec<u8>>,
    pub(crate) exceeded: bool,
}

impl BoundedOutput {
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.inner.into_inner()
    }
}

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let current_len = self.inner.get_ref().len();
        let end = usize::try_from(self.inner.position())
            .ok()
            .and_then(|position| position.checked_add(bytes.len()));
        let Some(end) = end.filter(|end| *end <= MAX_OUTPUT_BYTES) else {
            self.exceeded = true;
            return Err(std::io::Error::other("EPUB output budget exceeded"));
        };
        if self
            .inner
            .get_mut()
            .try_reserve(end.saturating_sub(current_len))
            .is_err()
        {
            self.exceeded = true;
            return Err(std::io::Error::other("cannot allocate EPUB output"));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for BoundedOutput {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_aggregate_media_before_mutation() {
        let mut book = EditableEpub::new();
        let chapter = book.add_chapter("One".into()).unwrap();
        book.media_bytes = MAX_MEDIA_BYTES;
        let before = book.chapters[chapter].body.clone();
        let error = book
            .add_image(
                chapter,
                b"\x89PNG\r\n\x1a\n".to_vec(),
                "image/png".into(),
                String::new(),
            )
            .unwrap_err();
        assert!(matches!(error, EngineError::ResourceLimit(_)));
        assert_eq!(book.chapters[chapter].body, before);
        assert!(book.images.is_empty());
    }

    #[test]
    fn rejects_xml_controls_and_active_svg() {
        assert!(validate_text("bad\u{1}", 100, "text").is_err());
        assert!(validate_image(b"<svg><script/></svg>", "image/svg+xml").is_err());
        assert!(validate_link("HTTPS://example.com").is_ok());
    }
}
