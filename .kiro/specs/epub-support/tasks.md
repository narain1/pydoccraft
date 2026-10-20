# EPUB implementation plan

- [x] 1. Select and license-review a permissive native EPUB parser; reject GPL alternatives.
- [x] 2. Add EPUB ZIP detection, shared archive preflight, and encrypted-resource rejection.
- [x] 3. Add thin Rust metadata and spine-text extraction adapter.
- [x] 4. Route EPUB through the Python facade and existing owned compatibility models.
- [x] 5. Add generated EPUB fixtures and extraction/error/lifecycle tests.
- [x] 6. Regenerate Cargo.lock and runtime license inventories.
- [x] 7. Run locked Rust/Clippy and installed-wheel validation; record size and limitations.
- [x] 8. Add semantic headings, inline links, referenced image extraction, strict XHTML checks, EPUB 2 coverage, and OOXML collision protection.
- [x] 9. Add native EPUB creation with metadata, ordered chapters, headings, paragraphs, links, images, save/copy/close, and extraction round trips.
- [x] 10. Complete exact-head license/wheel-size/corpus review and record the accepted writer footprint.
- [ ] 11. Scope CSS, richer navigation, fixed layout/rendering, conversion, and edit-existing preservation separately.
