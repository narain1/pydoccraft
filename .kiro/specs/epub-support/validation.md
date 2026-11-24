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

## 2026-10-26 integration onto current checkout

The historical results above cover the original branch, not this integration. The existing ArtCraft engine pins were preserved. Rust 1.95 formatting passed; the refreshed locked normal/build license graph contains 299 packages and passed the repository audit. The initial native test attempt could not link because the container lacked Python development libraries; validation was restarted with those libraries installed. The combined locked Rust suite passed 468 tests, and all-target Clippy passed with warnings denied. A Linux x86-64 CPython 3.13 release wheel built with maturin 1.15.0, measured 19,590,038 bytes (44,301,472-byte native extension), and passed all seven focused EPUB tests and 14 API regression tests in a clean installed environment. The full installed Python suite is running.
## 2026-10-27 merge of `main` at 0.3.1 (local macOS run, not a delivery gate)
- Merged `main` (0.3.1, engines pinned to the original `storytold/*` repositories). Conflicts were in `Cargo.toml`, `README.md`, `docs/authoring.md` and the two license inventories. `Cargo.lock` was regenerated from `main`'s lock, so the only existing package that moved is `serde_json` 1.0.138 to 1.0.149 (already required by this branch). The lock gained 35 packages for LexePub and rbook (265 to 300).
- The locked license audit reviewed 299 normal/build packages with no prohibited license. `THIRD_PARTY_LICENSES.md` and `LICENSE-THIRD-PARTY` are byte-identical, and `NOTICE` retains the LexePub and rbook entries.
- macOS 26.6 arm64, Rust 1.99.1, CPython 3.13: `cargo fmt --check` clean; `cargo test --locked` 466 passed, 2 failed. The two failures (`picture_float_raw_proof`, `section_vertical_alignment_proof`) fail identically on `main` and are unrelated to EPUB.
- `cargo clippy --locked --all-targets -- -D warnings` reports 40 errors on Rust 1.99 (`chunks_exact` and needless-borrow lints) in `author_pdf.rs`, `author_docx.rs`, `author_pptx.rs`, `rendering.rs` and the `*_proof.rs` files. None are in EPUB files, and the 1.95 floor has not been rerun.
- Release wheel built with `maturin build --release --locked`: 17,784,893 bytes; extension 37,073,040 bytes. The installed-wheel Python suite passed 589 tests, including the 7 EPUB tests.
- Not yet run: the 1.95 Clippy floor, a clean Python 3.11 environment, and the CI platforms.

## Metadata entity and archive-safety investigation

A generated book with title `A & B` and author `A & B` reopened through LexePub with title `B` and two authors, `A` and `B`. A focused experiment showed that the already-locked rbook reader preserves those values and accepts the existing EPUB 2 and 3 fixtures. An adapter using that reader passed focused Rust, Go, Node and installed-wheel EPUB tests, but was reverted at that checkpoint: the current `zip 2.2.2` reader builds its public member index by name, replacing earlier duplicate names before `archive.len()` and the existing member limit are checked. Introducing a second archive reader can cause readers to select different entries; checking duplicates only after constructing another archive index would not establish a bound on the raw member count before allocation. The following section records the subsequent fix.

## 2026-11-13 bounded ZIP preflight and metadata correction

The common ZIP preflight now streams and bounds central-directory records before constructing name-keyed indexes, rejects duplicate names and inconsistent directories, and validates archive paths. EPUB additionally checks that the three upstream ZIP readers agree on entries and local headers, and requires one package rootfile. The rbook reader supplies publication metadata, preserving XML entities and repeated authors; LexePub continues to supply chapter content. The Go and Node preview bindings accept XML-sensitive metadata and titles after round-trip tests. Adversarial tests cover duplicate entries, forged counts, mismatched headers, paths, multiple rootfiles and ambiguous end records. ZIP64, multi-disk archives, trailing data, and ambiguous EPUB end signatures are explicitly unsupported; ordinary data descriptors remain supported. The upstream readers still eagerly materialize intermediate structures, so a hard intermediate-allocation ceiling is not claimed.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 477 unit tests and seven integration tests; the Python-free suite passed 400 unit tests and eight integration tests. A clean Linux x86-64 CPython 3.13 installed release wheel passed all 611 Python tests, including 16 EPUB tests. The Go preview passed 15 tests with the race detector, and the Node preview passed 17 tests. The locked license audits passed for 299 core and 289 Node normal/build packages, with the license inventories unchanged. The wheel measured 19,184,645 bytes and its uncompressed extension 44,643,088 bytes. Hosted Python 3.11 and documentation validation is tracked by the PR checks.

## 2026-11-23 in-repo EPUB reader and semantic output

The implementation was built from `acb07ad4c16ae22a91cfe9d629d24aabb0641b75` with the original upstream engine revisions:

| Dependency | Revision |
|---|---|
| PdfCraft | `03996b10258288e723c4e07334c9cd1bae1f68a8` |
| WordCraft | `bbe3547dd11bb5f613f3aa893b12f593910644de` |
| DeckCraft | `f7a7fd7f5ffda19758584908730216222e387aa6` |
| craft-libs | `92d392cff132eaf53e13afff77f38b5c11b73aa0` |

Rust 1.95 formatting and warnings-denied Clippy passed. The final locked Rust suites passed all 478 unit and 7 integration tests with default features, and 401 unit and 8 integration tests without Python. These runs used `RUST_TEST_THREADS=12`. Earlier runs at the host's default concurrency intermittently failed DOCX bookmark or comment pixel comparisons. Both assertions passed individually, and the unchanged baseline passed its full Python-free unit suite. Go 1.26.8 passed 15 preview tests with the race detector. Node 22.22.1 passed 17 preview tests. The public typing check and the documentation build and link check passed.

The locked license audit passed for 275 core and 265 Node normal/build packages. The inventory and its wheel copy match byte for byte. LexePub and its async ZIP dependency are absent. The shared backend comes from one immutable revision. `Cargo.lock` SHA-256 is `72b9000b0c62a17938af505af05d3c8e0ff597c72f6cbc3255d753d0fa8095fd`.

| Linux x86-64 release wheel (maturin 1.8.1) | Wheel bytes | Native extension bytes | Wheel SHA-256 |
|---|---:|---:|---|
| CPython 3.11 | 19,625,782 | 45,550,904 | `a477318d147b90a3d39587da92cb5e13c2c804dab8f0bbc67d2f272cca90d038` |
| CPython 3.13 | 19,618,416 | 45,527,256 | `196b01cf26ed3bb24461f13b816fdf3616e50fb56bb86840185b353be136f126` |

Each clean installed wheel passed all 616 Python tests, with zero failures, errors, or skips, on CPython 3.11.15 and 3.13.13. The validator verified every installed package file against its wheel and ran the corpus with module-level process parallelism. The 21 EPUB tests cover EPUB 2 NCX and EPUB 3 navigation, ZIP limits and archive paths, repeated and XML-sensitive OPF metadata, nested tables, inline linked images, inert SVG and MathML extraction, authoring round trips, preserving untouched members during metadata edits, PNG clipping, and reopened DOCX and PDF conversions.

These are local Linux wheels. The cross-platform release matrix, CSS fidelity, fixed-layout output, and media overlay playback remain open.
