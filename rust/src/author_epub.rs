use pyo3::prelude::*;

use crate::EngineError;

const MAX_CHAPTERS: usize = 10_000;
const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGES: usize = 10_000;

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

#[pyclass(module = "doc_ex._core")]
#[derive(Clone)]
pub struct EditableEpub {
    title: String,
    identifier: String,
    language: String,
    authors: Vec<String>,
    chapters: Vec<Chapter>,
    images: Vec<ImageResource>,
    content_bytes: usize,
    closed: bool,
}

#[pymethods]
impl EditableEpub {
    #[new]
    pub fn new() -> Self {
        Self {
            title: String::new(),
            identifier: String::new(),
            language: "en".into(),
            authors: Vec::new(),
            chapters: Vec::new(),
            images: Vec::new(),
            content_bytes: 0,
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
        let fragment = format!("<h{level}>{}</h{level}>\n", escape_text(&text));
        self.append(chapter, &fragment)
    }

    pub fn add_paragraph(&mut self, chapter: usize, text: String) -> Result<(), EngineError> {
        let fragment = format!("<p>{}</p>\n", escape_text(&text));
        self.append(chapter, &fragment)
    }

    pub fn add_link(
        &mut self,
        chapter: usize,
        text: String,
        href: String,
    ) -> Result<(), EngineError> {
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
        if data.is_empty() || data.len() > 128 * 1024 * 1024 {
            return Err(EngineError::ResourceLimit(
                "EPUB image must contain at most 128 MiB".into(),
            ));
        }
        validate_text(&alt, 65_536, "image alt text")?;
        let extension = match media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/svg+xml" => "svg",
            _ => {
                return Err(EngineError::Unsupported(format!(
                    "EPUB image type {media_type} is not supported"
                )))
            }
        };
        let href = format!("images/image-{}.{}", self.images.len() + 1, extension);
        let fragment = format!(
            "<p><img src=\"{}\" alt=\"{}\" /></p>\n",
            href,
            escape_attribute(&alt)
        );
        self.append(chapter, &fragment)?;
        self.images.push(ImageResource { href, data });
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
        builder
            .build()
            .write()
            .to_vec()
            .map_err(|error| EngineError::Conversion(format!("EPUB writing failed: {error}")))
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

    fn append(&mut self, chapter: usize, fragment: &str) -> Result<(), EngineError> {
        self.ensure_open()?;
        let next = self
            .content_bytes
            .checked_add(fragment.len())
            .filter(|size| *size <= MAX_CONTENT_BYTES)
            .ok_or_else(|| EngineError::ResourceLimit("EPUB content exceeds 64 MiB".into()))?;
        let target = self
            .chapters
            .get_mut(chapter)
            .ok_or_else(|| EngineError::InvalidArgument("EPUB chapter does not exist".into()))?;
        target.body.push_str(fragment);
        self.content_bytes = next;
        Ok(())
    }
}

fn validate_text(value: &str, limit: usize, name: &str) -> Result<(), EngineError> {
    if value.len() > limit || value.contains('\0') {
        Err(EngineError::InvalidArgument(format!(
            "EPUB {name} is invalid or exceeds {limit} bytes"
        )))
    } else {
        Ok(())
    }
}

fn validate_link(href: &str) -> Result<(), EngineError> {
    validate_text(href, 16_384, "link")?;
    if href.starts_with("https://")
        || href.starts_with("http://")
        || href.starts_with("mailto:")
        || (href.starts_with('#') && !href.contains(".."))
    {
        Ok(())
    } else {
        Err(EngineError::Unsupported(
            "EPUB links require http, https, mailto or a safe fragment".into(),
        ))
    }
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
