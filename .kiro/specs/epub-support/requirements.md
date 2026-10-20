# Requirements: EPUB support

## 1. Native in-process extraction

1. WHEN `doc_ex.open()` receives EPUB 2 or EPUB 3 bytes/path, THE SYSTEM SHALL detect `application/epub+zip` and parse it in-process through a permissively licensed Rust EPUB library.
2. THE SYSTEM SHALL preserve reading/spine order and expose one compatibility page per readable spine chapter.
3. THE SYSTEM SHALL extract chapter plain text and publication metadata including title, authors, languages, subjects, publisher, identifiers, rights, contributors, version, and publication date when supplied.
4. Returned values SHALL be eager and owned and remain readable after document closure.

## 2. Safety and errors

1. EPUB archives SHALL use the existing member-count, path, member-size, aggregate-expansion, and compression-ratio preflight limits before upstream parsing.
2. Encrypted/DRM resources SHALL fail explicitly as unsupported; content SHALL NOT be silently omitted.
3. Missing container, package, manifest, spine, or chapter resources and malformed XML/HTML SHALL return stable errors.
4. Input and normalized JSON limits SHALL remain identical to other formats.

## 3. Initial scope

1. Version 1 SHALL support detection, metadata, spine-ordered chapter text, plain text, dict/JSON, and Markdown through existing `Document`/`Page` APIs.
2. EPUB rendering, images, CSS/layout fidelity, navigation links, footnotes, fixed-layout pagination, authoring, and conversion SHALL remain explicitly unsupported until separate acceptance criteria and upstream capabilities are reviewed.
3. No GPL/AGPL/LGPL-only runtime dependency is permitted.

## 4. Verification

1. Tests SHALL generate EPUB fixtures locally without external executables or network access.
2. Tests SHALL cover path/bytes detection, metadata, chapter order, malformed/missing resources, archive safety, encrypted-resource rejection, lifecycle ownership, unsupported rendering, and unsupported conversions.
3. Rust formatting/tests/Clippy and installed-wheel tests SHALL pass with the committed lockfile and audited license inventory.
