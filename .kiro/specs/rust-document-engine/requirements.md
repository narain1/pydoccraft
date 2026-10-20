# Requirements: Rust document engine

## 1. In-process package

**User story:** As a Python service developer, I want document extraction to execute in-process so deployment does not require desktop applications or subprocesses.

### Acceptance criteria

1. WHEN `doc_ex` is installed from a wheel, THE SYSTEM SHALL include the native extraction engine in the wheel.
2. THE SYSTEM SHALL NOT download tools or execute PdfCraft, WordCraft, DeckCraft, LibreOffice, Pandoc, PyMuPDF, or other external document processes at runtime.
3. THE SYSTEM SHALL be licensed under Apache-2.0 and SHALL use only permissively licensed runtime dependencies with a documented locked dependency inventory.
4. WHEN a supported document is opened from a path or bytes, THE SYSTEM SHALL return an eager, owned Python document model that remains independent of temporary files.

## 2. Python compatibility API

**User story:** As an existing PyMuPDF user, I want a familiar API so migration requires minimal application changes.

### Acceptance criteria

1. WHEN the caller invokes `doc_ex.open(path_or_bytes, filetype=None, password=None)`, THE SYSTEM SHALL detect or validate PDF, DOCX, and PPTX input.
2. THE SYSTEM SHALL expose `Document.page_count`, `len(document)`, iteration, integer indexing, negative indexing, `load_page()`, `to_markdown()`, context management, and idempotent `close()`.
3. THE SYSTEM SHALL expose `Page.number`, `get_text("text"|"markdown"|"dict"|"json")`, `get_links()`, `get_images(full=False)`, and `find_tables()`.
4. WHEN a closed document is accessed, THE SYSTEM SHALL raise `DocumentClosedError`.
5. WHEN unsupported or malformed input is supplied, THE SYSTEM SHALL raise a stable `DocExError` subclass rather than a raw native exception.

## 3. Structured extraction

**User story:** As a retrieval pipeline developer, I want one normalized representation across document formats.

### Acceptance criteria

1. WHEN a PDF is opened, THE SYSTEM SHALL preserve page boundaries and extract page text, URI links, image metadata/data when directly decodable, and table-like positioned text when detectable.
2. WHEN a DOCX is opened, THE SYSTEM SHALL preserve paragraphs, heading levels, hyperlinks, semantic tables, comments, explicit page breaks, and embedded image bytes.
3. WHEN a PPTX is opened, THE SYSTEM SHALL preserve one page per slide, ordered text, semantic tables, speaker notes, comments, hyperlinks, and embedded image bytes.
4. THE SYSTEM SHALL normalize pages to zero-based numbering and represent content as typed `Block`, `Table`, `Link`, and `Image` values.
5. WHEN Markdown is requested, THE SYSTEM SHALL emit conventional headings, links, pipe tables, image references, notes, and comments.

## 4. Conversion

**User story:** As an application developer, I want basic format conversion without installing an office suite.

### Acceptance criteria

1. WHEN `doc_ex.convert(source, "docx", filetype="pdf")` is called, THE SYSTEM SHALL return a valid DOCX containing the extracted PDF text separated by page breaks.
2. WHEN `doc_ex.convert(source, "pdf", filetype="docx")` is called, THE SYSTEM SHALL return a valid PDF containing the DOCX text with deterministic pagination.
3. THE SYSTEM SHALL expose `Document.convert(output_format)` as a convenience over the same native conversion engine.
4. THE SYSTEM SHALL reject unsupported conversion pairs explicitly.
5. Conversion SHALL preserve semantic text and page separation; pixel-perfect source layout is not required for version 0.2.1.

## 5. Footprint, safety, and performance

**User story:** As a Lambda operator, I want a small and bounded parser suitable for untrusted uploads.

### Acceptance criteria

1. THE SYSTEM SHALL avoid GUI, networking, OCR-model, JavaScript, editing, printing, and font-discovery dependencies.
2. WHEN parsing OOXML, THE SYSTEM SHALL reject path traversal, excessive member counts, oversized members, oversized aggregate expansion, and suspicious compression ratios.
3. WHEN parsing malformed XML, ZIP, PDF object graphs, relationships, or media, THE SYSTEM SHALL fail without writing outside owned memory.
4. THE release wheel SHALL be measured and its size reported.
5. Extraction tests SHALL run without network access, cloud credentials, or external binaries.

## 6. Verification

1. THE SYSTEM SHALL include self-contained generated fixtures for PDF, DOCX, and PPTX.
2. Tests SHALL cover path and bytes input, structured text, tables, links, images, notes/comments, Markdown, close semantics, malformed input, ZIP safety, and both conversion directions.
3. The native crate SHALL pass `cargo test` and `cargo clippy -- -D warnings`.
4. The Python package SHALL build as a wheel and pass its test suite from an isolated installation.
