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
        self.title = title;
        self.identifier = identifier;
        self.language = language;
        self.authors = authors;
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
        self.chapters.push(Chapter {
            title,
            body: String::new(),
        });
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
        let output =
            builder.build().write().to_vec().map_err(|error| {
                EngineError::Conversion(format!("EPUB writing failed: {error}"))
            })?;
        if output.len() > MAX_OUTPUT_BYTES {
            return Err(EngineError::ResourceLimit(
                "EPUB output exceeds 128 MiB".into(),
            ));
        }
        Ok(output)
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

fn validate_text(value: &str, limit: usize, name: &str) -> Result<(), EngineError> {
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

fn validate_image(data: &[u8], media_type: &str) -> Result<&'static str, EngineError> {
    let valid = match media_type {
        "image/png" => data.starts_with(b"\x89PNG\r\n\x1a\n").then_some("png"),
        "image/jpeg" => {
            (data.starts_with(&[0xff, 0xd8]) && data.ends_with(&[0xff, 0xd9])).then_some("jpg")
        }
        "image/gif" => {
            (data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")).then_some("gif")
        }
        "image/svg+xml" => return Err(EngineError::Unsupported(
            "EPUB SVG authoring is not supported because active content cannot be guaranteed inert"
                .into(),
        )),
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

fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attribute(value: &str) -> String {
    escape_text(value).replace('"', "&quot;")
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
