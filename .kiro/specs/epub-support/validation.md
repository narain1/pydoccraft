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

## 2026-11-24 chapter editing, media overlay cues and equation output

The continuation adds archive-preserving XHTML replacements, a bounded SMIL reader and local resource ranges, and native output for the supported Presentation MathML structures. The original engine pins and lockfiles are unchanged. The license inventory audit still covers 275 core normal/build packages and 265 for the Node preview.

The Linux x86-64 CPython 3.11.15 release wheel passed all 620 installed tests, including 25 EPUB cases, with zero failures, errors or skips. The validator compared the installed files with the wheel and ran the corpus in parallel by module. The narrated EPUB fixture exercises shared overlays, nested sequence roles, both published SMIL namespace spellings, element IDs, time ranges, bounded reads and unchanged audio bytes after a chapter edit. The equation fixture expands the PNG height relative to plain text, verifies native fraction, script, radical and matrix structures in DOCX, and reopens both DOCX and PDF.

Rust 1.95 formatting and warnings-denied Clippy passed. The full suites before the final SMIL namespace compatibility change passed 478 unit and 7 integration tests with the default feature set, and 401 unit and 8 integration tests with Python disabled. All integration tests passed again with the final change. Public typing, documentation links, and roadmap and support evidence checks passed.

Wheel size was 19,665,214 bytes and native extension size was 45,630,024 bytes. Wheel SHA-256 is `dea85aa834601919aa51005368f5b86d4fdb99081eed7d08841530a655dfaa0d`. The Cargo build directories and local documentation build output were removed after validation.

Source CSS and fixed layout, remaining MathML elements and styling, real-time narration playback, and structural chapter operations are tracked in the [EPUB plan](tasks.md).

## 2026-11-24 CSS and fixed-layout previews

The EPUB renderer now uses the source XHTML cascade for supported author styles and renders XHTML and SVG spine documents in their declared fixed viewports. Three new cases verify fixed XHTML and SVG geometry, a reflowable spine override, local linked and imported styles, selector precedence and inheritance, media queries, positioned PNGs, overlapping colored boxes, flex and grid placement, and bounded clipping. The existing MathML round-trip fixture now also exercises the styled preview path. Remote and cyclic stylesheet imports fail explicitly.

The final Linux x86-64 CPython 3.11.15 release wheel passed all 623 installed tests, including 28 EPUB tests, with zero failures, errors or skips. The validator compared installed files with the exact wheel and split tests by module. Wheel size was 21,546,751 bytes, its native extension was 50,102,776 bytes, and SHA-256 was `21347dd5c4d13fd74e0282e3c3dd3253db79b43cc2e793af24d1852b006745b7`.

Rust 1.95 formatting, warnings-denied Clippy, and the locked default suite passed (478 unit and 7 integration tests). The locked Python-free suite (401 unit and 8 integration tests), Go preview (15 with race detection) and Node preview (17) passed before the final Clippy-only delimiter refactor. Public typing, the documentation link check, and the roadmap and support evidence checks passed. All engine revisions remain unchanged. Audited graphs contain 297 core and 288 Node normal/build packages. Both license inventories match. The root lockfile SHA-256 is `4512d701d5a1a5e0e9c78bdc8c75ba0c4df76a98b45c931e495b8be9f04cdf94`.

The branch keeps broader CSS and fixed-layout fidelity, packaged fonts, CSS-aware export, and the remaining equation, overlay and chapter editing work in the [EPUB plan](tasks.md). Temporary Cargo and documentation build output was pruned.

## 2026-11-24 structural chapter and resource edits

Four new installed-wheel tests cover EPUB 3 navigation and EPUB 2 NCX updates, original spine itemrefs, nested entries with inherited namespaces, local resources, and atomic validation failures. A fixed-layout chapter fixture verifies that absolute positioning uses the containing block's geometry through intermediate static elements. Removed spine entries stay in the archive so surviving links and shared media overlays can still resolve.

The Linux x86-64 CPython 3.11.15 release wheel built with maturin 1.8.1 passed 627 installed tests, including 32 EPUB tests, with zero failures, errors or skips. The validator compared installed files with the wheel. The wheel measured 21,623,959 bytes and the native extension measured 50,267,848 bytes. Wheel SHA-256 is `cd9a4ca6e0f5f3f284a5398fc5501f6f596975b42fd569bb8fc96d438d47ea77`. The root lockfile and its four engine and backend pins are unchanged from the CSS preview run.

EPUBCheck 5.4.0 reported zero errors or warnings after the edition reordered and extended a local copy of the 22-entry Accessible EPUB 3 publication. The smoke test removed the source's plain HTML doctype declarations before opening because the reader currently rejects them. Acceptance of inert doctypes and a wider conformance corpus remain open. [EPUB 3 samples](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The locked Python-free suite passed 401 unit tests and 8 integration tests. Both ran with 12 test threads. Public typing, the documentation link check, the support and roadmap evidence checks, and the unchanged locked license inventories passed. Cargo cleanup removed 4.3 GiB of build files across the release and debug profiles.

## 2026-11-24 offline media-overlay playback and inert doctypes

Five new cases exercise XHTML doctype acceptance and custom declaration rejection, half-open WAV clip boundaries, MP3 waveform and overlapping clips, AAC-LC MP4 presentation timing with encoder priming, inherited SMIL roles, text-only narration, and active-target callbacks on success and sink failure. The MP3 and M4A fixtures contain local synthetic 500 Hz tones. A separate EPUB 2 smoke test accepts the published NCX public identifier and preserves the declaration through a structural edit.

The Linux x86-64 CPython 3.11.15 wheel built with maturin 1.8.1 passed 632 installed tests, including 37 EPUB tests, with zero failures, errors or skips. The validator compared installed files with the wheel. The wheel measured 22,120,575 bytes, the native extension measured 51,365,608 bytes, and wheel SHA-256 is `1ca425481e9b007ee93f4325824f8b7ef035bc54f76ccf5c4dfc75788558d08c`. The root lockfile SHA-256 is `827aa3f518e5e45639a97e34ac4358318caa560fd484a6bfdf86f90ccc911c04`. The ArtCraft engine and shared backend revisions are those listed for the in-repo EPUB reader above.

The original 22-entry Accessible EPUB 3 sample now opens with its XHTML doctypes intact. A reordered copy with a new chapter reopened, rendered and passed EPUBCheck 5.4.0 with zero errors or warnings. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The original narrated Moby-Dick sample opened with 144 spine entries. The installed wheel decoded all 40 AAC-LC audio cues across the two narrated chapters to 30,947,175 PCM frames. Each WAVE header agreed with the returned sample rate and frame count. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit and 7 integration tests, and the locked Python-free suite passed 401 unit and 8 integration tests. The license audits reviewed 311 core and 288 Node normal/build rows. The wheel copies of NOTICE and LICENSE-THIRD-PARTY match their source files. Public typing, site links, and the roadmap and support evidence checks passed. Cargo cleanup removed 5.0 GiB across the release and debug profiles.

Playback buffers one cue at a time. Ogg Opus, long-cue streaming, broader playback navigation, and the remaining CSS, MathML and authoring work are tracked in the [EPUB plan](tasks.md).

## 2026-11-24 packaged fonts and SVG preview

Six new EPUB cases cover archive-relative font-face URLs from imported CSS, per-publication font isolation, packaged fonts with inline SVG and MathML, styled table cells and spans, standard font obfuscation through a chapter reorder, and positioned and semantic SVG output. The fixed-layout fixture checks transparent image backgrounds, pixel placement, viewport and clipping at two DPIs. Semantic export reopens both DOCX and PDF with the source text and two SVG placements. The audit regression distinguishes Parley's pinned registry shaping library from application engines and unreviewed sources.

The Linux x86-64 CPython 3.11.15 release wheel built with maturin 1.8.1 passed 638 installed tests, including 43 EPUB tests, with zero failures, errors or skips. The validator compared installed files with the wheel. The wheel measured 22,502,324 bytes and the extension measured 52,303,096 bytes. Wheel SHA-256 is `3c18dfef6cdd6215e6de5553c55444de4fc5909e07542255a33bd49a861ff3dd`. Root and Node lockfile SHA-256 values are `9f703707b1114e2e0080197abd519fa328c449f38f22b2d8ffcd027777d043d5` and `c8af662d758e995f4e3467fd98bea9a6c1bbbae5636380bce3a5743f8e2021f2`. The ArtCraft engine and shared backend Git pins match the in-repo reader record.

All 22 chapters of the original Accessible EPUB 3 sample rendered at 48 DPI, including the chapter with styled tables. A standalone smoke test rendered identical PNGs from TTF, WOFF and WOFF2 versions of the existing Inter fixture. After one OpenType font in a local copy of the public sample was obfuscated with the EPUB OCF algorithm, previews from both books matched on three chapters. EPUBCheck 5.4.0 reported zero errors or warnings and one informational message for the obfuscated member. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The public Sous le Vent SVG-in-spine sample uses local SVG stylesheet processing instructions and packaged raster references. Opening it highlights a remaining SVG profile gap. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests, and the locked Python-free suite passed 401 unit tests and 8 integration tests. Node 22.22.1 passed 17 preview tests with the updated locked graph. The license audit reviewed 338 core and 315 Node normal/build rows. The wheel notices match their source files. Public typing, documentation links, and roadmap and support evidence checks passed. Cargo cleanup removed 5.1 GiB of generated release and debug output. The temporary Node addon and documentation build were removed as well.

## 2026-11-24 archive-local SVG resources

Three new EPUB tests cover SVG stylesheet processing instructions, archive-relative CSS imports, media selection, important declarations and selector precedence, local font aliases, original SVG bytes, and PNG raster placement. They verify separate resource bases for inline and linked SVG, and reopen both DOCX and PDF exports to check the two rendered placements. They also check rejection of external and unsafe archive paths and enforce a raster placement limit.

The official *Sous le Vent* SVG-in-spine sample opened and rendered all 13 pages at 96 DPI. Each page was 368 × 581 pixels and returned its original SVG bytes. The two JPEG-backed pages each had 213,808 nonwhite pixels. The source sample SHA-256 is `be7d9b735474f7d433e0a842f089b12647975dcaaf0314a8ef09c5ea0664f6bc`. Fixed-layout PDF and DOCX conversion returned the documented unsupported-format error. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The Linux x86-64 CPython 3.11.15 wheel built with maturin 1.8.1 passed 641 installed tests, including 46 EPUB tests, with zero failures, errors or skips. Its size is 22,682,725 bytes and the extension is 52,700,856 bytes. Wheel SHA-256 is `a0f81ed26ca4b10865f16eef0e2d5514123238fdb6fb6072075c482accdbcf6b`. Root and Node lockfile SHA-256 values are `f231834167f311625d15a6a6213167bd876abfe0833afe0d9b1f6f342e708269` and `0102ab2f2d5efbbde97dc57820ed668fd4ae24384aba0482931f653c5b710cbe`. The raster feature adds edges to existing permissive crates in both lockfiles. The 338-row core inventory and 315-row Node review passed, and the regenerated inventory and wheel notices match the checked-in copies. ArtCraft and craft-libs pins match the reader record.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. Node 22.22.1 passed 17 preview tests with the updated lockfile. Public typing and documentation checks passed. Cargo cleanup removed 4.9 GiB, and the temporary addon, documentation build, and wheel archive were removed after validation.

## 2026-11-24 inherited MathML styles and equation geometry

Three new EPUB tests exercise inherited and overridden color, size and math variants, primary presentation text without duplicated annotations, editable OMML accents and limits, multi-pair tensor indices, varying fence separators, horizontal spaces and zeroed padding dimensions. The display checks measure a one-em space in native PNG output and check colored formula runs after reopening both DOCX and PDF. The tests also cover explicit errors for unsupported vertical spaces and padding offsets. [MathML 3 presentation markup](https://www.w3.org/TR/MathML3/chapter3.html)

The official *Linear Algebra* sample has 94 spine entries and 10,281 MathML fragments. The original EPUB still reaches the bounded semantic output limit. A local excerpt retained all package members and selected ten spine entries with 1–13 equations each. Nine entries rendered successfully at 24 DPI. The remaining entry failed explicitly at a MathML spanning table cell. The original SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 644 installed tests, including 49 EPUB tests, with zero failures, errors or skips. The wheel is 22,683,515 bytes and its native extension is 52,692,216 bytes. Wheel SHA-256 is `8f67b6cd8b03310cc7da7da1b130d208dded8e825d9ef7fadef23ef38f7f4837`. The lockfiles and engine pins match the preceding checkpoint. The locked license audit verified 338 core and 315 Node packages, and the wheel notices match the source copies.

Rust 1.95 formatting and warnings-denied Clippy passed. The locked default suite passed 478 unit and 7 integration tests. The Python-free suite passed 401 unit and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, site links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, along with the temporary wheel archive, Node addon, site build and external sample.

## 2026-11-24 resource removal and media overlay authoring

Four new EPUB cases verify staged resource deletion, retained package and off-spine dependencies, imported CSS including extensionless members and print or alternate styles, responsive image URLs, adding narration to a new chapter, retaining an existing shared overlay, and jointly detaching and removing SMIL and audio. One authored cue targets a Unicode element ID and decodes to 4,000 PCM frames. A separate EPUB 2 check protected the legacy cover reference and confirmed the EPUB 3 requirement for overlay authoring.

EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages for both an authored EPUB with MPEG Layer III and text-only cues and its detached copy after removing the SMIL and audio members. The respective SHA-256 values are `3915f765e0eb31f6a9008ebd07187a70b60db8b1ed40c5697f8b3ebb47149a5a` and `bceff1f299fb077a657c8ea7a0ce47ffd59274df78704419e1a89f2bab8a4810`. [EPUB media overlay package metadata](https://www.w3.org/TR/epub-33/#x9-3-5-2-overlays-package-metadata)

The Linux x86-64 CPython 3.11.15 release wheel passed 648 installed tests, including 53 EPUB tests, with zero failures, errors or skips. The wheel is 22,741,921 bytes and its native extension is 52,844,280 bytes. Wheel SHA-256 is `0d4e376d3b15d62ebd753a28a44cb426e2175dcd1b1f50d8e78fbc822c425ef6`. Locked engine pins and permissive inventories match the preceding checkpoint. The audited graph contains 338 core and 315 Node packages, and the wheel notices match the source copies.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output and the temporary wheel, addon, site build and authored sample archives.

## 2026-11-24 fixed-layout spreads and visual PDF

Four EPUB regression cases exercise left-to-right and right-to-left page progression, blank and centered viewports, mixed dimensions, per-entry spread overrides, non-linear entries, and duplicate or unit-suffixed viewport metadata. They compare a gutter clip with the full spread raster and exercise a clipped render when the full spread exceeds the raster limit. The visual export test reopens a reordered selection of XHTML and SVG pages and checks their dimensions and painted pixels.

The public *Page Blanche* sample opened as ten fixed-layout chapters. At 72 DPI all landscape previews measured 1,800 × 1,183 pixels. The four facing pairs returned identical PNGs from either page in the pair. Portrait previews matched individual-page rendering. The exported PDF reopened with ten full-page images, and Poppler reported a viewport of 900 × 1,182.75 points on every page. EPUB sample SHA-256 is `d2efb562f17d43cef50578d4d67dd7fd991f8f254b6f5a7c28bde7461ea783eb`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The XHTML edition of *Sous le Vent* exposed four standard SVG rendering hints that the static validator rejected. A focused regression covers those hints. The XHTML and standalone SVG editions now both render all 13 chapters as single portrait pages and reopen with 13 full-page PDF images. At 96 DPI their portrait PNG sizes are 1,474 × 2,324 and 368 × 581 pixels, respectively. The XHTML edition SHA-256 is `1ac68b0a334d19ac97338574d6ad0d7a30723315c668c6d74021540b6b3f816d`. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The Linux x86-64 CPython 3.11.15 wheel passed 652 installed tests, including 57 EPUB tests, with zero failures, errors or skips. The wheel measured 22,737,576 bytes and the native extension measured 52,829,608 bytes. Wheel SHA-256 is `a39ae31beb9d643919d260a2ac953ef8fde30a2085a205b9fcf341f343226ffd`. Both lockfiles and the immutable upstream pins match the preceding checkpoint. License reviews verified 338 core and 315 Node packages, and the wheel notices matched the source copies.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed 6.1 GiB of Cargo output across the two validation builds, along with the temporary wheel archive, addon, documentation build and sample files.

## 2026-11-24 streamed and seekable media-overlay playback

Four EPUB regression cases verify sample-identical WAV, MP3 and AAC-LC output across chunk boundaries, an open-ended MPEG clip that ends on a chunk boundary, cue seeking before role filtering, and a long WAV cue whose 8,388,609 frames exceed the buffered PCM limit. The long cue streams in 33 chunks with one active-target interval, and the test compares a hash of its complete PCM against the synthetic source. A Xing/Info-tagged MP3 case verifies both clip bounds on the trimmed presentation timeline and rejects a mismatched frame count.

FFmpeg 8.0.1 decoded the synthetic gapless fixture to the same 17,640 samples as the wheel. The maximum difference was one 16-bit quantization step, and the mean absolute difference was 0.002778. FFmpeg generated the fixture and served as a validation reference. Playback uses the in-process decoder. [FFmpeg MPEG audio demuxer](https://www.ffmpeg.org/doxygen/trunk/mp3dec_8c_source.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 656 installed tests, including 61 EPUB tests, with zero failures, errors or skips. The validator compared installed files with the wheel. The wheel measured 22,742,307 bytes and its native extension measured 52,838,312 bytes. Wheel SHA-256 is `56da4545b1ed2ef7e9abcde12c3019fb0ba4b15cee34456c4e7dc2cc7d33ee25`. Both lockfiles and engine pins match the preceding checkpoint. The 338-package core and 315-package Node license reviews passed and the distributed license files match the source copies.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, along with the temporary wheel archive, Node addon, site build and reference PCM.

## 2026-11-24 Ogg Opus media overlays

Two EPUB regression cases exercise a 48 kHz stereo stream with distinct tones in the left and right channels, startup pre-skip, end trimming, exact buffered and streamed cue intervals, and a clip beyond the presentation end. The transport case rejects truncated data, a failed Ogg page checksum, and chained streams. A separate smoke test changed header gain by +6 dB and checked all 84,000 samples in the selected cue. It also rejected an impossible final granule on a page with a valid checksum. The fixture was generated from synthetic tones. [Ogg Opus specification](https://www.rfc-editor.org/rfc/rfc7845.html)

A libopus reference decode run with FFmpeg 8.0.1 produced the same 55,200 stereo frames, with a maximum difference of one PCM quantization step across 110,400 samples. A separate synthetic voice-mode mono encode yielded 39,360 frames and matched libopus byte for byte. The wheel uses only the in-process Rust decoder and demuxer. [Opus reference implementation](https://opus-codec.org/)

The Linux x86-64 CPython 3.11.15 release wheel passed 658 installed tests, including 63 EPUB tests, with zero failures, errors or skips. The validator compared installed files with the wheel. The wheel measured 22,912,369 bytes and its native extension measured 53,193,912 bytes. Wheel SHA-256 is `0e0265780db85ac9b9608221d8db876f8d6283a99654f1737d63089feddfe536`. The root lockfile SHA-256 is `80a0953addb4f4abc1625f73937d4237697e54ed70e9dc8382815d213c845a65`. The license review covered 341 core and 315 Node packages. The wheel contains the updated BSD and MIT attribution. The Node lockfile and the four ArtCraft Git pins match the preceding checkpoint.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, the decoder probe's build, the temporary wheel archive, the Node addon, the site build and the reference audio files.

## 2026-11-24 IDPF-obfuscated font edits

The EPUB tests replace an obfuscated font from a plain TrueType source and confirm identical rendering, the expected packaged bytes, and exact preservation of the encryption declaration. The removal regression covers a URL-escaped font path, an active stylesheet reference that blocks removal, partial declaration removal with the original comment intact, and a staged stylesheet rewrite that permits removal of the last font and encryption document.

EPUBCheck 5.4.0 reported zero fatals, errors and warnings for the replacement and removal samples. It reported one informational message for the replacement because it skips an obfuscated font's contents. The installed wheel reopened and rendered the sample with the replacement font. The replacement EPUB SHA-256 is `f1fd5d15df580ff0380e3350780e089235cd976fc479719ea5c2a2fc0c9794c7`. The removal EPUB SHA-256 is `1c3f3b71dfcf19e3147929b763147edf9628ce563be82852eebe29549434d886`. [EPUB font obfuscation specification](https://www.w3.org/TR/epub-33/#sec-font-obfuscation)

The Linux x86-64 CPython 3.11.15 wheel passed 659 installed tests, including 64 EPUB tests, with zero failures, errors or skips. The wheel measured 22,906,617 bytes, its native extension measured 53,176,808 bytes, and wheel SHA-256 is `810a2ff07f7370eace7c915d399fb10dfd0fa595ee6adec9e4e51c23433bf601`. Both lockfiles and the immutable engine pins match the preceding checkpoint. The core and Node license checks verified 341 and 315 packages respectively, and all wheel license files match their source copies.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, the temporary wheel archive, Node addon, site build and EPUBCheck samples.

## 2026-11-24 nested SMIL authoring

Two EPUB regression cases exercise nested sequence and cue order, chapter fragment links including Unicode IDs, inherited playback roles, role skipping, and package duration metadata. The validation cases reject empty groups, missing sequence targets, cyclic input and excess timing entries. The original publication remains usable after invalid edits.

EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages for an authored book with a nested sidebar, MPEG Layer III narration and text-only cues. The EPUB reopened with all three target fragments in source order. Sample SHA-256 is `e1c5438cb8e339b4bc98da2b62b55c601a8c97f40875cb0a6c3cb92994b0c317`. [EPUB 3.3 overlay structure](https://www.w3.org/TR/epub-33/#sec-media-overlays-structure)

The Linux x86-64 CPython 3.11.15 release wheel passed 661 installed tests, including 66 EPUB tests, with zero failures, errors or skips. The wheel measured 22,911,408 bytes and its native extension measured 53,190,488 bytes. Wheel SHA-256 is `264b3e4f250a0ec111886f6ad3027ca90347224b5e4c91b6b0ec01da2ef16e47`. Both lockfiles and engine pins match the preceding checkpoint. The core and Node license reviews covered 341 and 315 packages, respectively. The wheel contains the source license and notice files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, including a nested-overlay authoring example, documentation links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, the temporary wheel archive, Node addon, site build and EPUBCheck sample.

## 2026-11-24 CSS for MathML equations

Two regression cases verify relative and inherited font sizes, color precedence over presentation hints, bold and italic native runs, and compact and raised placement of movable limits. The reflow case reopens DOCX and PDF exports and checks their equation colors. The fixed-layout case compares the entire PNG against an equivalent absolute font size and reopens the visual PDF at the declared viewport. Public extraction retains the original MathML source. [MathML Core math-style](https://www.w3.org/TR/mathml-core/#the-math-style-property)

Ten chapters containing 1–12 formulas each rendered from a local excerpt of the public *Linear Algebra* EPUB. The complete 94-chapter book still reaches the semantic output budget. Extracting all 94 spine entries in batches yielded 55,397,335 bytes of page JSON, of which 9,693,158 bytes were page-level table payloads also represented in blocks. The source SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

EPUBCheck 5.4.0 reported OPF-014 after an XHTML replacement introduced MathML into a chapter. A local copy with the required `mathml` manifest property reported zero errors, warnings, and informational messages. The chapter editor's manifest update remains open. The EPUB preview and reopened DOCX and PDF rendered the styled equation. The conformant copy's SHA-256 is `2a7056d4aa99eca604224fa80af8b2e5d643f19f46538c91d8599778b4bd2060`. [EPUB package resource properties](https://www.w3.org/TR/epub-33/#sec-item-resource-properties)

The Linux x86-64 CPython 3.11.15 release wheel passed 663 installed tests, including 68 EPUB tests, with zero failures, errors or skips. The wheel measured 22,969,631 bytes and its native extension measured 53,339,640 bytes. Wheel SHA-256 is `05ebf9375ac5836989b9818ba468157ee1af1c51674c772ea05fe475c7b9f78a`. Both lockfiles and engine pins match the preceding checkpoint. License reviews covered 341 core and 315 Node packages, and the wheel contains the source license files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, the temporary wheel, Node addon, site build and EPUB samples.

## 2026-11-24 chapter manifest declarations

The EPUB regressions introduce MathML and SVG into existing chapters, insert a new chapter with both types of markup, and remove declarations when replacing those chapters with plain XHTML. They verify preservation of other manifest properties and the EPUB 2 package vocabulary. The original chapter replacement case continues to require byte-identical OPF when no property update is needed.

EPUBCheck 5.4.0 reported zero fatals, errors, warnings, and informational messages for package outputs after replacement, insertion, and removal. This resolves the OPF-014 result from the MathML stylesheet checkpoint. The EPUB SHA-256 values are `d4c3a6a24f78ec822560659b7919617c67f7423a9dd2175ea09790fe731ee488`, `cbc6d289f8fa7a54f2148ab009b9f19ffa6ae774bd788c4b71f7941a3f268613`, and `b31afe4bfd8d0b17b9f9c8b95b293e407274b6598952606a13ed19547a1a1b68`, respectively. [EPUB resource properties](https://www.w3.org/TR/epub-33/#sec-item-resource-properties)

The Linux x86-64 CPython 3.11.15 release wheel passed 665 installed tests, including 70 EPUB tests, with zero failures, errors or skips. The wheel measured 22,964,955 bytes and its native extension measured 53,324,040 bytes. Wheel SHA-256 is `373e8cb64567c2a5203487746d5825baf3e87258980ea172d1d9c9e20367701f`. Both lockfiles and the four upstream repositories' revisions match the preceding checkpoint. Core and Node license audits covered 341 and 315 packages, respectively.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, the temporary wheel, Node addon, site build and edited EPUB samples.

## 2026-11-24 MathML whitespace and full Linear Algebra extraction

The whitespace regression compares equivalent compact and indented formulas, including an annotation and explicit nonbreaking spaces. It checks extracted text and character offsets, exact PNG parity, normalized native DOCX text, and preservation of the original MathML markup. [MathML whitespace rules](https://www.w3.org/TR/MathML3/chapter2.html#fund.collapse)

The unmodified public *Linear Algebra* sample opened with all 94 chapters and 10,281 formulas. Native extraction produced a 31,054,438-byte JSON payload within the existing limits. The previous build returned a semantic output budget error for the same file. Analysis of the source found approximately 15.6 MB of XML formatting whitespace in the primary math presentation. Source EPUB SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The complete spine check at 24 DPI produced 69 PNGs. Fourteen chapters hit the 8,000-element CSS style limit and eleven returned the explicit unsupported error for spanning MathML table cells. The CSS-limited chapters contain 8,533–20,160 elements each, with 746–1,459 elements outside MathML. All ten chapters in the prior excerpt rendered from the full source at 36 DPI.

The Linux x86-64 CPython 3.11.15 release wheel passed 666 installed tests, including 71 EPUB tests, with zero failures, errors or skips. The wheel measured 22,977,316 bytes and its native extension measured 53,341,128 bytes. Wheel SHA-256 is `9429b1f3db2fe34ea5af4f3d0258c6f7091ff41d0f3c43e00f3c42fb36f5799f`. Both lockfiles and the four upstream revisions match the preceding checkpoint. The core and Node license reviews covered 341 and 315 packages, respectively.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed 4.9 GiB of Cargo output, the temporary wheel, Node addon, site build and public sample archive.

## 2026-11-24 CSS scaling and tall reflowable previews

The equation style regression compares a formula containing 8,100 empty MathML rows with its compact equivalent. It checks exact PNG parity and confirms that a later equally specific rule wins in both the EPUB preview and the reopened DOCX. A chapter with 8,100 XHTML spans still hits the box limit. A separate tall chapter verifies a full 204 × 8,023 preview at 24 DPI, the full-raster rejection at 96 DPI, and a 16 × 20 clipped preview of the bottom block at 96 DPI. [CSS cascade order](https://www.w3.org/TR/css-cascade-5/#cascade-sort)

The unmodified 94-chapter *Linear Algebra* sample produced 75 PNGs at 24 DPI. Separate equation styles exposed a selector work limit on chapter 39. Indexing rules let that chapter reach layout, where the earlier height limit stopped it. It now renders at 204 × 6,894 pixels and supports a clipped preview at 96 DPI. The other 19 chapters report unsupported MathML table spans. The sample has 65 spanning cells across those chapters. Its SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 668 installed tests, including 73 EPUB tests, with zero failures, errors or skips. The wheel measured 22,985,020 bytes and its native extension measured 53,376,248 bytes. Wheel SHA-256 is `4dd4250333f2603679542983560038f856c5d501b6b792cae90a10966baf0c6a`. Both lockfiles and the four upstream revisions match the preceding checkpoint. License reviews covered 341 core and 315 Node packages, and the wheel contains the source license files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.1 GiB of Cargo output, the temporary wheels, Node addon, site build and public sample archive.

## 2026-11-24 MathML table spans

The span regressions check colored cell positions and alignment in reflowable PNG previews, inline placement on unstyled chapters, rejection of overlapping spans, and fixed-layout visual PDF output. DOCX stores merged cells and five editable equations. Reopened DOCX and PDF retain each cell label once and both outputs render. [MathML table cell spans](https://www.w3.org/TR/mathml-core/#entry-in-table-or-matrix-mtd)

All 94 chapters in the original *Linear Algebra* EPUB now produce PNGs at 24 DPI. The prior result was 75. The 19 newly rendered chapters contain 43 formulas with 65 spanning cells. Chapter 17 includes a 33-row equation table and renders at 204 × 2,957 pixels. An isolated six-row table from that chapter also exported with the expected DOCX merged cell and editable equations. Both its DOCX and PDF outputs reopened and rendered. The original EPUB SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 670 installed tests, including 75 EPUB tests, with zero failures, errors or skips. The wheel measured 23,010,957 bytes and its native extension measured 53,441,784 bytes. Wheel SHA-256 is `fcba4fbe51a645bd72ce84438091293ecc5631e3f1274daa5067516ca7819f04`. Both lockfiles and the four upstream revisions match the preceding checkpoint. License checks passed for the 341 core and 315 Node packages. The wheel contains the source license files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.1 GiB of Cargo output, the temporary wheels, Node addon, site build and public sample archive.

## 2026-11-24 SVG spine chapter editing

The installed-wheel regressions insert a fixed-layout SVG between two XHTML chapters with a stylesheet, PNG, and text-only media overlay in the same edit. They verify the escaped navigation path, the SVG media type and viewport, PNG colors, and narration text. A batch SVG and XHTML replacement changes the graphic and its viewport, preserves the OPF, navigation, media overlays and untouched member bytes, and renders the revised graphic from a reopened fixed-layout PDF. Missing navigation and narration targets, invalid viewBox values, mismatched roots and extensions, missing SVG dependencies, and active SVG elements are rejected.

EPUBCheck 5.4.0 reported zero fatals, errors, warnings or informational messages for the insertion and replacement with a path without spaces. Both outputs reopened and rendered. Their SHA-256 values are `b65156f43070e708e3c83e0497a2b0fa530fb8eee4afeb5f4bd61ed73ce25b5f` and `81496b992a981de1a0bcf2620800a909f555cbac7f83ca56877c1aaf6e040662`, respectively. A separate pair used a spaced archive filename. EPUBCheck reported only PKG-010 for older reading system interoperability. [EPUB SVG content documents](https://www.w3.org/TR/epub-33/#sec-svg)

The Linux x86-64 CPython 3.11.15 release wheel passed 672 installed tests, including 77 EPUB tests, with zero failures, errors or skips. The wheel measured 22,996,107 bytes and its native extension measured 53,399,848 bytes. Wheel SHA-256 is `84eaf8deb6b5aaf24801643af1ef179c3920df036a8d666a8f791e1366418d40`. Both lockfiles and the four upstream revisions match the preceding checkpoint. License checks passed for the 341 core and 315 Node packages. The wheel contains the source license files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output, the temporary wheel, Node addon, site build and generated EPUBs.

## 2026-11-24 Chained Ogg Opus narration

The chained-stream regression decodes 55,200 stereo frames followed by 16,800 mono frames at 48 kHz. Whole-track and streamed output match the concatenated standalone PCM exactly after duplicating the mono samples into both output channels. A 20 ms SMIL cue straddles the stream boundary and a later cue seeks backward on the same resource. Transport regressions cover a duplicate serial, truncated second stream, checksum failure in either stream, and concurrently multiplexed pages. [Ogg stream chaining](https://www.rfc-editor.org/rfc/rfc3533.html#section-4), [Ogg Opus presentation timing](https://www.rfc-editor.org/rfc/rfc7845.html#section-4)

The mono fixture is a locally generated 340 Hz sine wave with a 350 ms duration and 6 dB Opus header gain. Its SHA-256 is `9e9e0c6a394faad302225ab8357f9f143996090b6fb32e1b0e2392ebcd48012b`. FFmpeg 8.0.1 with libopus decoded the same frame counts. The relative RMS differences between the in-process PCM and reference were 0.000008 for the stereo segment and 0.000070 for the mono segment.

The Linux x86-64 CPython 3.11.15 release wheel passed 673 installed tests, including 78 EPUB tests, with zero failures, errors or skips. The wheel measured 23,001,236 bytes and its native extension measured 53,409,736 bytes. Wheel SHA-256 is `0631781d7d6ced71c58ddbf8a979e8eccccd28245e95def25822531f15015591`. Both lockfiles and the four upstream revisions match the preceding checkpoint. License checks passed for the 341 core and 315 Node packages. The wheel contains the source license files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output, the temporary wheel, Node addon and site build.

## 2026-11-24 EPUB XHTML font Unicode ranges

The composite font regression compares rendered pixels with an explicit per-run font assignment. It checks wildcard and absolute ranges, a later equal-style face winning on an overlap, and bundled fallback for a glyph outside both ranges. Reversing the face declarations changes the rendered result. A font declaring 4,096 SFNT table records returns the resource-limit error before rewriting. [CSS font-face character ranges](https://www.w3.org/TR/css-fonts-4/#unicode-range-desc)

The root and Node lockfiles now record a direct dependency on the already-resolved `write-fonts` 0.48.1 package. No package versions or upstream Git pins changed. The regenerated license files match the prior inventory, and the audits cover 341 core and 315 Node packages.

The Linux x86-64 CPython 3.11.15 release wheel passed 674 installed tests, including 79 EPUB tests, with zero failures, errors or skips. The wheel measured 23,069,375 bytes and its native extension measured 53,562,792 bytes. Wheel SHA-256 is `efd46ca8cba31bdd96cc3cd5791867e2b48dc07ab643f5e476c61be173d9b56d`. The audit also verified the three license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.1 GiB of Cargo output, the temporary wheels, Node addon and site build.

## 2026-11-24 EPUB XHTML font-family stacks

The regression compares rendered pixels with single-family controls. It checks inherited lists, unavailable names, Unicode coverage gaps in the first face, quoted and escaped commas, generic precedence, and a quoted name identical to a generic keyword. An author face named like a bundled font still passes uncovered characters to the next declared family. [CSS font-family matching](https://www.w3.org/TR/css-fonts-4/#font-family-prop)

A 320 × 120 fixed-layout preview using a fallback stack matched its single-family control. The converted visual PDF reopened and rendered at the same dimensions. All 94 chapters of the unmodified *Linear Algebra* EPUB rendered at 24 DPI. The sample SHA-256 remains `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 675 installed tests, including 80 EPUB tests, with zero failures, errors or skips. The wheel measured 23,066,712 bytes and its native extension measured 53,554,920 bytes. Wheel SHA-256 is `cd48309f4d587296ff0a41ae601c2bcfd7fdc401577091d3b70d957d24392861`. Both lockfiles and all upstream revisions match the previous checkpoint. License audits reviewed 341 core and 315 Node packages and verified the wheel's license files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output, the temporary wheel, Node addon, site build and public sample archive.

## 2026-11-24 EPUB variable font axes

The XHTML regression compares rasterized outlines from the packaged Inter variable font. It checks inherited and direct settings, duplicate axes, clamping, unknown axes, and a reset on a child. It also compares automatic optical sizing with an explicit `opsz` value and verifies that requesting an unavailable axis leaves a preview using the bundled static font pixel-identical. The SVG regression inserts a fixed-layout SVG chapter with a staged font and compares normal, heavier and optically sized glyphs after reopening the edited book. [CSS font variation settings](https://www.w3.org/TR/css-fonts-4/#font-variation-settings-def)

The unmodified Inter v4.1 font fixture has SHA-256 `4989b125924991b90d05b2d16e0e388c48f7d5bb8b30539bbf9c755278d0ccaf`. Its SIL Open Font License matches the fixture license already in the repository. The font is a test asset and is excluded from the wheel. [Inter v4.1](https://github.com/rsms/inter/tree/e3a3d4c57d5ecc01453a575621882a384c1995a3)

All 94 spine entries of the unmodified *Linear Algebra* sample rendered at 24 DPI. The sample SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 677 installed tests, including 82 EPUB tests, with zero failures, errors or skips. The wheel measured 23,088,249 bytes and its native extension measured 53,607,160 bytes. Wheel SHA-256 is `587379b27515ef457d34d2dd938412638f6d25f21db1add6bf3c3b3fdc440ec5`. Both lockfiles and the upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the three license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.1 GiB of Cargo output across the builds, plus the temporary wheels, Node addon, site build and sample archive.

## 2026-11-24 Nested spanning MathML tables

A nested table combines an outer cell spanning two rows with an inner cell spanning two columns. The regression compares the rendered bounds of four colored cells and verifies that DOCX stores the inner table, both merges and the original equations. The converted DOCX and PDF both reopen with each cell's text present once. Malformed inner spans, a nested table combined with another expression, excessive nesting, and implicit grid entries beyond the shared budget return explicit errors. [MathML table cells](https://www.w3.org/TR/mathml-core/#entry-in-table-or-matrix-mtd)

All 94 spine entries of the unmodified *Linear Algebra* EPUB rendered at 24 DPI. The sample SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 678 installed tests, including 83 EPUB tests, with zero failures, errors or skips. The wheel measured 23,074,002 bytes and its native extension measured 53,577,912 bytes. Wheel SHA-256 is `77503aba4f3ad409a6ff7fae0899c83db191b733feef721cfc242748b37e3fb1`. Both lockfiles and upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 478 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.1 GiB of Cargo output across the builds, plus the temporary wheels, Node addon, site build and sample archive.

## 2026-11-24 Surround Ogg Opus narration

The EPUB regression decodes a synthetic 7.1 fixture with 19,200 frames at 48 kHz. It checks each channel's tone, the WAVE speaker mask, exact buffered and streamed PCM, a cue crossing the link boundary, and routing a chained mono link to the center channel. A Rust regression rejects a packet whose coded substreams have inconsistent durations, along with truncated self-delimited framing. [Ogg Opus channel mapping](https://www.rfc-editor.org/rfc/rfc7845.html#section-5.1.1.2), [WAVE speaker masks](https://learn.microsoft.com/en-us/windows/win32/api/mmreg/ns-mmreg-waveformatextensible)

The checked-in fixture uses separately generated sine tones and has SHA-256 `b839dd398366070970a2d7baaac3440f738229b53062ee09d3494fca1f6b6ff7`. FFmpeg 8.0.1 with the libopus decoder gave the same frame counts and at most one 16-bit PCM step of difference across all channels for 3.0, quad, 5.0, 5.1, 6.1 and 7.1 streams. A 6.1 to 7.1 chain produced nine output channel positions with mask `0x73f` and matched the standalone PCM after aligning speakers and inserting silence. [Opus reference implementation](https://opus-codec.org/)

The unmodified narrated *Moby-Dick* EPUB also streamed 30,947,175 PCM frames from all 40 AAC-LC cues across its two narrated chapters. The run checked the channel count, sample rate and frame count in each WAVE chunk. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The Linux x86-64 CPython 3.11.15 release wheel passed 679 installed tests, including 84 EPUB tests, with zero failures, errors or skips. It measured 23,077,723 bytes with a 53,584,824-byte native extension. Wheel SHA-256 is `68c7ac2cb9bbf94ec1dd6d4921ecd3b1bd376d5292cb1797ccf4bedba91d92cb`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output and the temporary wheel, Node addon, site build and reference audio files.

## 2026-11-24 Multiplexed Ogg narration

The generated Theora-and-Opus fixture has SHA-256 `c2e2c70ffa01449133b5eec35c2241f407e10db40d9ab3f6c45f6059cec67f35`. FFmpeg 8.0.1 encoded the synthetic video and copied the existing stereo Opus test packets. The 55,200 decoded PCM frames match the standalone audio byte for byte under all three accepted Ogg media types. Streaming a grouped segment followed by a standalone mono link matches the standalone chain, including a cue crossing the boundary. Splitting the Opus comment packet around an interleaved Theora page preserves the same PCM. [Ogg grouping and chaining](https://www.rfc-editor.org/rfc/rfc3533.html#section-4), [Ogg Opus media types](https://www.rfc-editor.org/rfc/rfc7845.html#section-9)

Transport regressions check an auxiliary stream ending after Opus, a nil EOS page, a one-page auxiliary stream, a corrupt trailing auxiliary page and missing EOS. Data pages before the last BOS and competing Opus tracks return explicit errors. An atomic edit adds the multiplexed resource, updates both shared overlay associations and reorders the chapters. The reopened EPUB preserves the resource bytes and plays the selected 960-frame cue.

The Linux x86-64 CPython 3.11.15 release wheel passed 680 installed tests, including 85 EPUB tests, with zero failures, errors or skips. It measured 23,068,182 bytes with a 53,559,080-byte native extension. Wheel SHA-256 is `9a3c2b0bfaa00aae9f293ce91b1db8bc6b8b581dc5ffedf954791bee260fd926`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

The unmodified narrated *Moby-Dick* EPUB streamed 30,947,175 PCM frames from all 40 AAC-LC cues across its two narrated chapters. Each WAVE chunk matched the returned audio geometry. [EPUB 3 sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output and the temporary wheel, Node addon, site build and generated fixture copy.

## 2026-11-24 Shared SMIL chapter edits

The regressions detach one chapter of a shared SMIL, replace the narration of one chapter while reordering the spine, and retain an existing cue without a clip end. The replacement case retains the nested sequence IDs and roles, retargets the shared body and sequence, measures audio staged in the same edit, and removes an audio file freed by pruning. A chapter with no timed cues can leave the shared association while the edit updates the retained audio duration and preserves the SMIL bytes. [EPUB Media Overlay structure](https://www.w3.org/TR/epub-33/#sec-media-overlays-structure)

Open-ended timing probes verified retained PCM and package duration against playback for MP3 with and without gapless metadata, AAC-LC MP4, standalone Opus and Opus multiplexed with Theora. The existing WAVE regression checks 11,000 retained frames and a 160-frame replacement cue. EPUBCheck 5.4.0 reported zero errors or warnings on a standards-valid shared-overlay fixture before and after detaching one chapter and reordering the spine. [EPUB Media Overlay durations](https://www.w3.org/TR/epub-33/#x9-3-5-2-overlays-package-metadata)

The Linux x86-64 CPython 3.11.15 release wheel passed 681 installed tests, including 86 EPUB tests, with zero failures, errors or skips. It measured 23,098,392 bytes with a 53,622,472-byte native extension. Wheel SHA-256 is `f07bbc69157a23b7d269b4a9f7b74ff6ea1b2624c60a559079f7909def740fd5`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.1 GiB of Cargo output across builds, plus the temporary wheels, Node addon, site build and EPUBCheck artifacts.

## 2026-11-24 CSS custom properties in EPUB previews

The XHTML regression compares archive-imported custom properties with literal rendered output. It covers case-sensitive names, inherited computed values, inline and important declarations, a dependency cycle through an unused fallback, and shorthand and longhand order. A repeated expansion beyond 4 KiB returns a resource-limit error. Follow-up checks verified empty fallbacks, escaped identifiers, scoped initial values, and numeric token boundaries. [CSS custom properties](https://www.w3.org/TR/css-variables-1/)

The fixed-layout SVG regression compares inherited variable paint with literal fill colors. A variable resolving to an external paint URL returns the same explicit unsupported error as the equivalent literal URL.

All 94 spine entries of the unmodified *Linear Algebra* EPUB rendered at 24 DPI. The sample SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 683 installed tests, including 88 EPUB tests, with zero failures, errors or skips. It measured 23,116,535 bytes with a 53,687,352-byte native extension. Wheel SHA-256 is `eda0ad500b01bda4aece9420573826da32b36c5f46b20086827ef4ebba0f6cf6`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output and the temporary wheel, Node addon, site build and sample archive.
