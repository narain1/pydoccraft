# EPUB validation evidence

## 2026-10-20 initial native extraction slice

- Selected `lexepub 0.1.2` from crates.io, Apache-2.0, checksum `6d35d0fae6dc461c3bd7055d8d12ffd9e50ec255f488041d006b8b975bdef596`.
- Reviewed and rejected the common `epub 2.1.5` crate because it is GPL-3.0.
- Default LexePub features are disabled; no CLI, C-FFI, WASM, Embassy, JavaScript, networking, or external executable is used.
- EPUB bytes pass the existing ZIP member/path/member-size/aggregate/compression-ratio preflight before upstream parsing.
- Detection requires the `application/epub+zip` mimetype. `META-INF/encryption.xml`, supplied passwords, missing/malformed package data, empty spines, and incomplete upstream chapter extraction fail explicitly.
- One spine chapter maps to one zero-based compatibility page. This is not physical pagination. Publication metadata and chapter-ordered plain text are eager and owned.
- Rendering and all conversion pairs are explicitly unsupported.

### Validation

| Check | Result |
|---|---|
| Rust 1.95 formatting | Passed |
| `cargo test --locked` | Passed |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| Installed-wheel Python suite | Passed: 174 tests |
| Generated EPUB bytes/path, metadata, order, ownership | Passed |
| DRM/encryption, malformed package, render/convert errors | Passed |
| Locked license audit | 291 normal/build packages; no undeclared or GPL-/AGPL-/LGPL-only license |
| License inventory copies | Byte-for-byte identical |
| Linux ARM64 CPython 3.13 wheel | 15,912,406 bytes |

The dependency graph adds LexePub's async ZIP, HTML/XML, and unconditional non-WASM Diplomat support dependencies. All are permissively licensed, but the footprint should be compared with a lower-level feature split before release. No ArtCraft revision changed.

## Historical deferred scope at the initial slice

At the initial extraction-only checkpoint, navigation links, images, semantic XHTML blocks/headings, and authoring were deferred. The expansion below completes those items; CSS, media overlays, fixed-layout pagination/rendering, conversion, and edit-existing preservation remain deferred.

## 2026-10-20 semantic extraction and native creation expansion

- Added strict XHTML well-formedness checks and LexePub AST mapping for ordered headings, paragraphs, exact inline links, referenced image bytes/type/alt text, and EPUB 2/3 metadata/spine chapters.
- Fixed detection precedence so affirmative DOCX/PPTX content types cannot be shadowed by a colliding EPUB mimetype member.
- Added native `doc_ex.new("epub" | "ebook")` creation through rbook 0.7.12 (`write` feature only, default features disabled). The semantic append-only API supports required metadata, authors, ordered chapters, headings, paragraphs, inert HTTP(S)/mailto/fragment links, signature-checked PNG/JPEG/GIF resources, bytes/path/stream save, copy and close. Authored SVG is rejected because arbitrary SVG cannot be guaranteed inert.
- Created EPUBs reopen through the independent LexePub extraction path with metadata, order, headings, links and image bytes preserved. Raw XHTML/OPF/CSS/scripts and caller archive paths are not exposed.
- Added EPUB 2, OOXML collision, malformed XHTML, supplied-password, archive traversal, semantic extraction, image/link, authoring roundtrip, copy, save, close, invalid metadata/heading/link/image tests.
- Rust 1.95 validation passed: 227 tests and Clippy with warnings denied. The complete installed-wheel suite passed: 268 tests.
- Locked license inventory: 298 normal/build packages, no undeclared or GPL-/AGPL-/LGPL-only license; `THIRD_PARTY_LICENSES.md` and `LICENSE-THIRD-PARTY` are byte-identical.
- Linux ARM64 CPython 3.13 wheel size: 16,311,442 bytes. rbook authoring adds approximately 399 KiB over the extraction-only EPUB wheel and is accepted for this feature profile.
- Rendering, CSS fidelity, fixed layout, scripts, DRM, media overlays, format conversion and editing existing EPUBs remain explicit non-goals.

## 2026-10-20 independent-review hardening

- Merged current `main` and preserved relocated/encoded OOXML detection while requiring an exact main-part content-type override whose target exists; both EPUB/OOXML collision directions are covered.
- Added strict bounded UTF-8 container, selected OPF, and chapter XHTML scans with DTD, attribute, token, and depth rejection before LexePub handling.
- Preserved readable list, quote, preformatted, caption, definition, and table-cell text alongside headings/paragraphs; verified Unicode link offsets and case-insensitive external schemes.
- Authoring now rejects XML-illegal controls, active SVG, and mislabeled PNG/JPEG/GIF bytes; each image is capped at 16 MiB, aggregate encoded media at 64 MiB, content at 64 MiB, and conservative/actual output at 128 MiB. Image mutations stage every validation/allocation before committing chapter/resource state.
- After merging current `main`, Rust 1.95 exact-state checks passed: formatting, 243 locked tests, and warnings-denied all-targets Clippy.
- A clean Linux ARM64 CPython 3.13 installed wheel passed 311 tests with zero failures/errors/skips. Wheel size was 16,343,534 bytes; uncompressed native extension size was 35,733,128 bytes.
- The locked license audit reviewed 298 normal/build packages; the two license inventories remain byte-identical with no prohibited runtime license.
- Remaining upstream limits are explicit: LexePub materializes/clones its complete CSS/chapter AST before adapter output limits, and rbook writes into an in-memory vector. The profile does not claim hard intermediate-allocation ceilings until bounded upstream traversal/sink APIs exist.

## 2026-10-27 merge of `main` at 0.3.1 (local macOS run, not a delivery gate)
- Merged `main` (0.3.1, engines pinned to the original `storytold/*` repositories). Conflicts were in `Cargo.toml`, `README.md`, `docs/authoring.md` and the two license inventories. `Cargo.lock` was regenerated from `main`'s lock, so the only existing package that moved is `serde_json` 1.0.138 to 1.0.149 (already required by this branch). The lock gained 35 packages for LexePub and rbook (265 to 300).
- The locked license audit reviewed 299 normal/build packages with no prohibited license. `THIRD_PARTY_LICENSES.md` and `LICENSE-THIRD-PARTY` are byte-identical, and `NOTICE` retains the LexePub and rbook entries.
- macOS 26.6 arm64, Rust 1.99.1, CPython 3.13: `cargo fmt --check` clean; `cargo test --locked` 466 passed, 2 failed. The two failures (`picture_float_raw_proof`, `section_vertical_alignment_proof`) fail identically on `main` and are unrelated to EPUB.
- `cargo clippy --locked --all-targets -- -D warnings` reports 40 errors on Rust 1.99 (`chunks_exact` and needless-borrow lints) in `author_pdf.rs`, `author_docx.rs`, `author_pptx.rs`, `rendering.rs` and the `*_proof.rs` files. None are in EPUB files, and the 1.95 floor has not been rerun.
- Release wheel built with `maturin build --release --locked`: 17,784,893 bytes (`main` baseline 17,241,542 on the same platform); extension 37,073,040 bytes. The installed-wheel Python suite passed 589 tests, including the 7 EPUB tests.
- Not yet run: the 1.95 Clippy floor, a clean Python 3.11 environment, and the CI platforms.
