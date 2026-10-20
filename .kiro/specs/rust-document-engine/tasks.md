# Implementation Plan

- [x] 1. Change the package and source license to Apache-2.0 and document attribution obligations.
- [x] 2. Replace direct PDF parsing with pinned PdfCraft extraction, image, link, and rendering APIs.
- [x] 3. Replace direct DOCX parsing with pinned WordCraft document-model traversal, layout, rendering, and PDF export.
- [x] 4. Replace direct PPTX parsing with pinned DeckCraft presentation-model traversal, rendering, and PDF export.
- [x] 5. Replace handwritten conversion with upstream PDF→DOCX, DOCX→PDF, and PPTX→PDF implementations.
- [ ] 6. Add Python rendering methods while preserving the existing `open`, `Document`, `Page`, and Markdown APIs.
- [ ] 7. Remove direct parser dependencies and retain only the small normalization/adaptation layer.
- [ ] 8. Add extraction, media, link, table, note/comment, render, conversion, malformed-input, and lifecycle tests.
- [x] 9. Generate and review the locked runtime license inventory; reject non-permissive dependencies.
- [ ] 10. Run Rust formatting/tests/Clippy, Python tests from an installed wheel, conversion reopen tests, and report wheel/native sizes and remaining upstream limitations.
