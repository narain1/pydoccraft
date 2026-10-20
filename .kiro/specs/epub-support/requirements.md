# Requirements: EPUB support

## 1. Native in-process extraction

1. WHEN `doc_ex.open()` receives EPUB 2 or EPUB 3 bytes/path, THE SYSTEM SHALL detect `application/epub+zip` and parse it in-process through a permissively licensed Rust EPUB library.
2. THE SYSTEM SHALL preserve reading/spine order and expose one compatibility page per readable spine chapter.
3. THE SYSTEM SHALL extract semantic headings, paragraphs, exact inline hyperlinks, referenced image bytes/alt text, and publication metadata including title, authors, languages, subjects, publisher, identifiers, rights, contributors, version, and publication date when supplied.
4. Returned values SHALL be eager and owned and remain readable after document closure.

## 2. Safety and errors

1. EPUB archives SHALL use the existing member-count, path, member-size, aggregate-expansion, and compression-ratio preflight limits before upstream parsing.
2. Container, selected package, and chapter XML SHALL be validated as bounded, strict UTF-8 XML without DTDs before lossy upstream handling.
3. Encrypted/DRM resources SHALL fail explicitly as unsupported; content SHALL NOT be silently omitted.
4. Missing container, package, manifest, spine, or chapter resources and malformed XML/HTML SHALL return stable errors.
5. Input and normalized JSON limits SHALL remain identical to other formats. LexePub's eager AST/CSS allocation remains a documented upstream limit until it exposes bounded traversal.

## 3. Native creation

1. `doc_ex.new("epub" | "ebook")` SHALL return an owned reflowable EPUB authoring document.
2. Creation SHALL support required title/identifier/language metadata, ordered authors and chapters, semantic headings, paragraphs, inert ASCII-case-insensitive HTTP(S)/mailto/fragment links, and signature-checked in-memory PNG/JPEG/GIF resources. XML-illegal controls and authored SVG SHALL be rejected.
3. Authored content SHALL be capped at 64 MiB, each image at 16 MiB, aggregate encoded media at 64 MiB, and estimated/actual output at 128 MiB.
4. `to_bytes()`, path/stream `save()`, `copy()`, context management, and idempotent `close()` SHALL follow the existing editable-document lifecycle.
5. Created EPUB bytes SHALL reopen through the independent extraction adapter with metadata, chapter order, headings, links, image bytes and alt text preserved.
6. Invalid metadata, unsafe link schemes, unsupported or mislabeled image types, excessive chapters/content/media, stale/closed handles, and suffix mismatches SHALL fail explicitly without partial mutation or output.

## 4. Explicit non-goals

1. Rendering, fixed-layout pagination, CSS fidelity, scripts, DRM, media overlays, arbitrary XHTML input, editing existing EPUBs, and format conversion remain unsupported.
2. No GPL/AGPL/LGPL-only runtime dependency is permitted.

## 5. Verification

1. Tests SHALL generate EPUB fixtures locally without external executables or network access.
2. Tests SHALL cover path/bytes detection, metadata, chapter order, malformed/missing resources, archive safety, encrypted-resource rejection, lifecycle ownership, unsupported rendering, and unsupported conversions.
3. Rust formatting/tests/Clippy and installed-wheel tests SHALL pass with the committed lockfile and audited license inventory.
