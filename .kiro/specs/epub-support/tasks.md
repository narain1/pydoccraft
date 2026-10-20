# EPUB implementation plan

- [x] 1. Select and license-review a permissive native EPUB parser; reject GPL alternatives.
- [x] 2. Add EPUB ZIP detection, shared archive preflight, and encrypted-resource rejection.
- [x] 3. Add thin Rust metadata and spine-text extraction adapter.
- [x] 4. Route EPUB through the Python facade and existing owned compatibility models.
- [x] 5. Add generated EPUB fixtures and extraction/error/lifecycle tests.
- [x] 6. Regenerate Cargo.lock and runtime license inventories.
- [x] 7. Run locked Rust/Clippy and installed-wheel validation; record size and limitations.
- [ ] 8. Scope links, media, semantic XHTML, rendering, conversion, and authoring separately.
