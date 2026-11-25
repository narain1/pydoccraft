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

## 2026-11-24 Staged chapter anchor references

The regression fixture renames a percent-encoded Unicode target and updates its links across a chapter batch. The earlier wheel rejected this edit and accepted removal of IDs referenced by retained non-spine XHTML, SVG or compatibility NCX. The new wheel rejects removals still targeted by those resources or the OPF guide. SVG checks honor the precedence of `href` over `xlink:href`. The tests reopen the edited archive, confirm untouched member bytes and preserve links to previously unresolved fragments as authored. [EPUB 3.3 content documents](https://www.w3.org/TR/epub-33/#sec-contentdocs)

The Linux x86-64 CPython 3.11.15 wheel passed all 123 EPUB tests. Rust 1.95 formatting and release Clippy with warnings denied passed, and the documentation check found 35 pages and 2,317 valid local references. The wheel measured 24,561,438 bytes with a 57,093,640-byte native extension. Its SHA-256 is `4a5a5b036ae69b3c85c3780488af2bd89c3e789c170f463eec7bdb3ea19c59c6`. The worktree's 1.6 GiB of Cargo output, temporary wheels and virtual environments were pruned.

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

## 2026-11-24 Same-document SVG use references

The fixed-layout regression checks nested instances, Unicode fragment IDs, href precedence over xlink:href, inherited fill and symbol viewport scaling. The original SVG bytes survive chapter insertion. The fixed-layout PDF reopens with the same dimensions and sampled pixels. Inline SVG preserves its rendered colors in reopened DOCX and PDF. An additional EPUB authoring check preserves the SVG resource bytes and renders its referenced tile. [SVG use elements](https://www.w3.org/TR/SVG2/struct.html#UseElement)

The validation cases reject remote and cross-file targets, missing and duplicate IDs, reference cycles, excessive depth, exponential node expansion, and copied style payloads above the byte budget. The style case passes the first check against source XML and fails before rendering after the computed styles are included.

All 13 entries of the unmodified *Sous le Vent with SVG in spine* EPUB rendered at 24 DPI. Its visual PDF reopened and rendered all 13 pages at matching dimensions. The sample SHA-256 is `be7d9b735474f7d433e0a842f089b12647975dcaaf0314a8ef09c5ea0664f6bc`. All 94 entries of the unmodified *Linear Algebra* EPUB also rendered at 24 DPI. The separate *SVG-in-spine* sample still fails on SVG attributes outside the supported profile. It contains gradient templates and Gaussian blur filters. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 685 installed tests, including 90 EPUB tests, with zero failures, errors or skips. It measured 23,121,075 bytes with a 53,692,376-byte native extension. Wheel SHA-256 is `d9342a868c9565c57612440dee0902a751d48118ef1b7de64fce908e6c26ead3`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output and the temporary wheel, Node addon, site build and sample archives.

## 2026-11-24 SVG gradient templates and Gaussian blur

Fixed-layout pixel checks cover inherited stops in linear and radial gradients, both fragment attribute forms, and a CSS blur on graphics instantiated by SVG use. The visual PDF reopens at the original viewport size with the same sampled colors and halo. Inline SVG retains the effect in reopened DOCX and PDF. The safety checks exercise missing, duplicate and cyclic references, template depth, nonlocal filters, unsupported primitives and filter placements, large authored and scaled blur radii, and single-layer and aggregate pixel limits. [SVG gradient templates](https://www.w3.org/TR/SVG2/pservers.html#LinearGradientElement), [Gaussian blur](https://www.w3.org/TR/filter-effects-1/#feGaussianBlurElement)

All six spine entries of the unmodified public *SVG-in-spine* EPUB rendered at 96 DPI. Its visual PDF reopened and rendered all six pages with identical dimensions. The sample SHA-256 is `ed6b1b9e99245ac7747ced1a2f90bad1acab558280cffc7daed3e54ee48e49bb`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 687 installed tests, including 92 EPUB tests, with zero failures, errors or skips. It measured 23,127,416 bytes with a 53,709,736-byte native extension. Wheel SHA-256 is `8288e329519a7388b555e6a413f190c7b4416d4ca2b14194f247825ba6b602a0`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output and the temporary wheel, Node addon, site build and sample archive.

## 2026-11-24 Explicit Opus track selection

Narration selects each of two concurrent Opus tracks in an Ogg resource with an auxiliary Theora stream. Each result matches the standalone track byte for byte. Chunked playback across two grouped links matches the independently chained PCM and keeps one active-target interval. The tests reject ambiguous and missing selections, selections on non-Ogg audio, and a corrupt page on the unselected Opus stream. [Ogg grouping and chaining](https://www.rfc-editor.org/rfc/rfc3533.html#section-4)

The package editor writes a grouped Opus resource with the EPUB core type `audio/ogg; codecs=opus` and an explicit 20 ms cue. The authored EPUB reopens with identical resource bytes and returns 960 stereo or mono samples per channel for the corresponding selection. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages. Authored EPUB SHA-256 is `075200557cf6821a3408f237473207e38b7d41cd55fc989d98132374f4dbd956`. [EPUB 3.3](https://www.w3.org/TR/epub-33/)

The Linux x86-64 CPython 3.11.15 release wheel passed 688 installed tests, including 93 EPUB tests, with zero failures, errors or skips. It measured 23,124,655 bytes with a 53,695,016-byte native extension. Wheel SHA-256 is `47c2dbed2006c81be6ba2a92e58558dcd406296c93f31f6bb46fa24b65f816e8`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.1 GiB of Cargo output across both builds, the temporary wheels, Node addon, site build and EPUBCheck fixture.

## 2026-11-24 Embedded XHTML media and audio playback

The edition regression inserts and replaces an XHTML chapter with packaged audio, video, poster and WebVTT captions. The chapter retains media source order and uses a supported source after an unsupported codec candidate. Its styled preview contains the poster pixel color. The bounded audio interval matches the source PCM and the remote source stays inert. Removing a resource while a retained chapter references it returns an error, including when that chapter is omitted from the spine. [HTML media elements](https://html.spec.whatwg.org/multipage/media.html)

All eight entries of the unmodified public *Creative Commons Shared Culture* EPUB rendered as static PNGs at 24 DPI. The inventory identified five video elements with two source options apiece, their WebVTT and TTML captions, and one MP3 soundtrack. The soundtrack yielded 48,000 stereo frames for the requested one-second interval. Legacy trigger controls stayed inert. The sample SHA-256 is `7bec3acf5ca153fd8a3b8bba50549c5cde3cb617ddd74c1d3b26fbc5eb9eebdf`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

A separate authored EPUB with local MPEG audio, Ogg video and WebVTT captions reopened and streamed its audio. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages. Authored EPUB SHA-256 is `e7f00d9f2e22e746ac28085c3d7459955aa9a5ec1308aca9c50f66380a742686`.

The Linux x86-64 CPython 3.11.15 release wheel passed 690 installed tests, including 95 EPUB tests, with zero failures, errors or skips. It measured 23,133,097 bytes with a 53,721,400-byte native extension. Wheel SHA-256 is `110afc05d4312082090348d0c702d1a986090c67c9d8b9553dbba4f686464922`. Both lockfiles and all upstream revisions match the preceding checkpoint. License audits reviewed 341 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing covered the embedded media models and audio iterator. Documentation links, support evidence and roadmap checks passed. Cleanup removed about 4.9 GiB of Cargo output, the temporary wheels, Node addon, site build and sample archives.

## 2026-11-24 Embedded MP4 video playback

The regression decodes an authored MP4 fixture with two closed GOPs and reordered B frames. It checks presentation order, red and blue RGB intervals, non-square pixels, a clip crossing the GOP boundary, an in-GOP seek, and the matching AAC soundtrack interval. Truncated MP4 returns a malformed-document error. A separate 2,048 × 2,048 sample reached the coded-area limit before picture decoding. The committed MP4 contains synthetic colour bars and a 440 Hz tone generated with FFmpeg. Its SHA-256 is `a6feed129cd0f33adbf82a90a9221830104a9ea0d77cd9d6dc0653d7f7bbc66c`. [OxideAV H.264 decoder](https://github.com/OxideAV/oxideav-h264)

The unmodified *Creative Commons Shared Culture* EPUB returned five 720 × 480 frames with a 40:33 pixel aspect ratio and 9,600 AAC samples per channel in each of its five 200 ms chapter clips. The MP4 also passed 100 ms seeks at 30, 101 and 199 seconds. A continuous read returned all 4,802 frames and contiguous timing through 200,283,616 microseconds, matching the frame count and timescale reported by ffprobe. The rolling RGB SHA-256 is `b854257933e5cad863b2e432e542971dde553739c195d4030b5eb425f7dca499` and the sample SHA-256 is `7bec3acf5ca153fd8a3b8bba50549c5cde3cb617ddd74c1d3b26fbc5eb9eebdf`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The authored EPUB reopened and decoded all 12 video frames and 48,000 AAC samples. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages. Authored EPUB SHA-256 is `c6220e68aaf0013b14f20d48dfae607bcd9dbc98e2afa188b106d3dd57838e1a`.

The Linux x86-64 CPython 3.11.15 release wheel passed 691 installed tests, including 96 EPUB tests, with zero failures, errors or skips. It measured 23,719,606 bytes with a 55,150,280-byte native extension. Wheel SHA-256 is `fc5d6ed837b1f06e7cb0e593992f41c562fede24fd1836be9028b52781683747`. The lockfile adds only oxideav-h264 0.1.12. ArtCraft engine pins and the Node lockfile match the preceding checkpoint. License audits reviewed 342 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing covered the video iterator and frame model. Documentation links, support evidence and roadmap checks passed. Cleanup removed about 5.0 GiB of Cargo output, the temporary wheels, Node addon, site build and sample archives.

## 2026-11-24 Composed MathML previews

The fixed-layout regression checks row and column placement in spanning tables inside a fenced numerator, square-root denominator, and superscript. It repeats the color and placement checks on the reopened visual PDF. The padding regression checks signed horizontal and vertical offsets, absolute and content-relative sizes, equivalent pseudo-unit expressions, vertical math spaces and rejection above the native geometry limit. Existing editable merged-cell and nested-table tests still pass. Composed geometry inside searchable or editable semantic output remains open. [MathML 3 padding](https://www.w3.org/TR/MathML3/chapter3.html#presm.mpadded)

All 94 chapters of the original *Linear Algebra* sample rendered at 24 DPI. The sample SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

A separate authored fixed-layout EPUB reopened with its composed MathML and exported a renderable visual PDF. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages. Authored EPUB SHA-256 is `6b083536448cf1f438e6ffa8c98c3f8b585e6bd64c2a530986d1cac54d4f5724`.

The Linux x86-64 CPython 3.11.15 release wheel passed 693 installed tests, including 98 EPUB tests, with zero failures, errors or skips. It measured 23,738,411 bytes with a 55,178,248-byte native extension. Wheel SHA-256 is `dfa10700a59c43d5c1d8f9d2b90b24f03755514bc393f31a867bd9077db2b4f7`. Both lockfiles and all engine revisions match the preceding checkpoint. License audits reviewed 342 core and 315 Node packages and verified the license files packaged in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 5.0 GiB of Cargo output, the temporary wheels, Node addon, site build and sample archives.

## 2026-11-24 Searchable MathML PDF and EPUB 3 compatibility navigation

The reflowable PDF regressions check searchable formula text, unique occurrences, a retained hyperlink, and colored cell bounds across pages. The fixtures cover a spanning grid next to other inline content, a composed fraction and radical inside an XHTML table, display fractions and superscripts, and a padded equation wider than the text area. The export retains the positioned glyphs and vector rules. Composed equations remain unsupported in editable DOCX.

A chapter edit, starting from a newly authored EPUB 3, removes the original spine entry and inserts a chapter containing a composed equation. Before the NCX fix, EPUBCheck reported RSC-011 on the compatibility navigation file. With the fix, EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages. The edited EPUB SHA-256 is `3fec8c9624b7a8637dd4aa88a87bd9fc2723a4ae1f7142e9625b119a0741ad41`. Its one-page PDF reopened and rendered, and all four equation terms were searchable. A separate regression checks insert, remove and reorder operations together, with matching destinations and regenerated NCX play order. [EPUB 3.3 spine requirements](https://www.w3.org/TR/epub-33/#sec-spine-elem)

The Linux x86-64 CPython 3.11.15 wheel passed 696 tests in a fresh installed environment, including 101 EPUB tests, with zero failures, errors or skips. The wheel measured 23,757,129 bytes and its native extension measured 55,226,840 bytes. Wheel SHA-256 is `736d5c3e49df2407df91e6eaa27983e24c144f1e181890cc32cf8835d9e125d9`. The lockfiles, upstream revisions and attribution files match the preceding checkpoint. The license audits reviewed the 342 core and 315 Node packages and verified the license files in the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, site links, support evidence and roadmap checks passed. Cleanup removed about 6.2 GiB of Cargo build output across the validation runs, along with the temporary wheel and virtual environment, Node addon, site build and sample archive.

## 2026-11-24 MathML fraction rules and alignment

The installed-wheel regressions measure a 6 pt rule between operands with distinct colors. They check left numerator alignment and right alignment on a barless fraction in the reflowable preview and reopened searchable PDF. The fixed-layout preview and reopened visual PDF use the equivalent 8 px rule. Additional checks compare percentage rules with the thin and thick keywords and reject excessive absolute and relative thickness. A separate fraction inside a radical retained its right-aligned denominator, colored bar, and searchable numerator. [MathML fraction attributes](https://www.w3.org/TR/MathML3/chapter3.html#presm.mfrac), [MathML Core fractions](https://www.w3.org/TR/mathml-core/#fractions-mfrac)

The original *Linear Algebra* EPUB rendered all 94 chapters at 24 DPI as a regression check for ordinary equations. The source did not use custom fraction thickness or alignment. Its SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. An authored reflowable sample using a 200% bar reopened as a searchable PDF with both operands present. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages for the authored EPUB. Its SHA-256 is `1269db46ae7fc0b9c1918652779b1590dbfe5bd2c410ab2362abd4e6810a2133`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 wheel passed 698 tests in a fresh environment, including 103 EPUB tests, with zero failures, errors or skips. It measured 23,769,408 bytes with a 55,261,512-byte native extension. Wheel SHA-256 is `87fa128b5e2e5c734a54f71ca6539aada8b53485e5ee33aa8f25ffebf93cefd5`. Both lockfiles and all engine pins match the preceding checkpoint. License audits passed for the 342 core and 315 Node packages, and the wheel included the repository's attribution files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 5.0 GiB of Cargo output, the temporary wheel and virtual environment, Node addon, site build and sample archives.

## 2026-11-24 SVG Unicode ranges and font fallback

The fixed-layout regression matches overlapping Unicode ranges and inherited fallback stacks to explicit font-run controls. It checks that declaration order changes the overlap winner, that stacks sharing a primary face keep distinct fallback order, and that a quoted generic name selects the literal packaged face. The original SVG resource survives the spine edit. The fixed-layout visual PDF reopens with the same raster pixels. A serif ligature followed by a letter from another face matches an explicit run split in both fixed-layout and inline SVG. The inline chapter and its reopened DOCX and PDF match the same explicit control. [CSS Fonts matching algorithm](https://www.w3.org/TR/css-fonts-4/#font-matching-algorithm)

The original public *SVG-in-spine* sample rendered all six entries at 96 DPI. All six reopened visual PDF pages rendered with matching dimensions. The source EPUB SHA-256 is `ed6b1b9e99245ac7747ced1a2f90bad1acab558280cffc7daed3e54ee48e49bb`. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages for a separate authored book with an SVG ligature and packaged range-limited fallback. That EPUB SHA-256 is `2e8edb019cf434ad6a0bc96964eab6555c58a99dcffd49e8541efe90bd915af5`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 wheel passed 700 tests in a fresh environment, including 105 EPUB tests, with zero failures, errors or skips. It measured 23,808,547 bytes with a 55,356,264-byte native extension. Wheel SHA-256 is `c9c571bf050d9c6bc0c5920ebfd0a89cba30f99fcae253a82a61782a40e08932`. Both lockfiles record a direct reference to the already locked `unicode-segmentation` 1.13.3 crate. The native engine pins match the preceding checkpoint. Regenerated license inventories were reviewed against 342 core and 315 Node packages and the updated attribution was verified inside the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 5.0 GiB of Cargo output, the temporary wheel and virtual environment, the Node addon, the site build and the EPUB samples.

## 2026-11-24 Embedded text tracks and temporal media fragments

The WebVTT regression checks default and explicit track selection, a UTF-8 BOM and CRLF input, overlapping cues, region and placement settings, and retained markup. A fragment starts between decoded MP4 frames. The test checks all six presentation intervals and their active cues, including a seek inside two captions. It verifies that the split intervals share each decoded RGB buffer. The matched AAC window yields 6,240 samples per channel. WAV fragments cover an omitted start, an omitted end, and an authored end beyond EOF. Chapter editing keeps fragment-referenced resources reachable. [WebVTT](https://www.w3.org/TR/webvtt1/), [Media Fragments URI](https://www.w3.org/TR/media-frags/#npttimedef)

The TTML regression checks a parallel paragraph within a sequential container, a tick-timed span, line breaks, region inheritance, preserved spaces, fractional frame and subframe timing, and an open-ended cue. Three consecutive fractional-rate frame intervals retain their exact total duration before microsecond conversion. A chapter replacement retains the track bytes and cue projection. Malformed timing, segmented timestamp maps, and tracks above the cue bound return explicit errors. [TTML timing](https://www.w3.org/TR/ttml2/#timing)

The authored captioned MP4 EPUB passed EPUBCheck 5.4.0 with zero fatals, errors, warnings and informational messages. It produced captions at the authored microsecond boundaries and 7,200 AAC samples per channel in the fragment. Its SHA-256 is `ef9830987347913206b4a48f0dfbfe97bdd32beddee0290cd4c7fc9d45e824bc`. All five 200 ms excerpts from the unmodified *Creative Commons Shared Culture* EPUB retained five video frames and 9,600 AAC samples per channel. The public sample SHA-256 is `7bec3acf5ca153fd8a3b8bba50549c5cde3cb617ddd74c1d3b26fbc5eb9eebdf`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 wheel passed 703 tests in a fresh installed environment, including 108 EPUB tests, with zero failures, errors or skips. It measured 23,832,044 bytes with a 55,406,632-byte native extension. Wheel SHA-256 is `0dad24b306d408bc065ab8b6a1e69e9b8098345f7139d691226f5fea27203554`. Both lockfiles, engine revisions and attribution match the preceding checkpoint. The license audit reviewed the same 342 core and 315 Node packages and verified the attribution files inside the wheel.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.2 GiB of Cargo output across validation runs, along with the temporary wheels, virtual environments, Node addon, site build and sample archives.

## 2026-11-24 Embedded Ogg Theora video

The installed regressions play Theora with 4:2:0 and 4:2:2 chroma. They verify exact frame intervals, cropped picture dimensions, orientation, pixel aspect ratio, a seek from the preceding keyframe, and a caption crossing a physical link boundary. The regressions join two valid Ogg segments in both orders and compare decoded PCM with the individual soundtracks. When the first picture outlasts its soundtrack by 350 ms, the returned audio contains exactly 16,800 silent samples per channel before the second soundtrack. When the soundtrack outlasts the picture by 250 ms, the video iterator skips that gap. A clip inside the final audio-only interval yields audio and no frames. The tests check local source order, explicit MP4 and Ogg selection, fallback from unsupported Ogg to MP4, and rejection of truncated Ogg. [Theora specification](https://www.theora.org/doc/Theora.pdf)

The new synthetic fixture was generated with FFmpeg using a 66 × 50 blue test source with a red upper band, 5 frames per second, 4:2:2 chroma and a 4:3 pixel aspect ratio. Its mono soundtrack is a 560 Hz sine at 48 kHz, encoded as Opus. Its SHA-256 is `be2c2ac964ca8c4dc6a31cdea6b200472ad36d012dbd7dc8a9a89d87e90eae17`. A separate 4:4:4 video-only test decoded all four expected frames, with first-frame RGB bytes matching FFmpeg exactly. A valid 1920 × 1104 coded test stream returned the expected resource-limit error before decoder allocation.

A generated EPUB with the chained resource, a caption, a time fragment and an MP4 alternative passed EPUBCheck 5.4.0 with zero fatals, errors, warnings and informational messages. Its frame and caption presentation boundaries matched the fragment and its audio contained 19,200 samples per channel. Its SHA-256 is `3d9c54a1be88e9e9aadfdb39f4b73fe7b808a0be7e54f0e465b2753cfd5e17a6`. All five 200 ms excerpts from the original *Creative Commons Shared Culture* EPUB still yielded five AVC frames and 9,600 AAC samples per channel. The public EPUB SHA-256 is `7bec3acf5ca153fd8a3b8bba50549c5cde3cb617ddd74c1d3b26fbc5eb9eebdf`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 wheel passed 704 tests in a fresh environment, including 109 EPUB tests, with zero failures, errors or skips. It measured 23,917,222 bytes with a 55,584,936-byte native extension. Wheel SHA-256 is `5897df3ff5c614515fc69c326b103d9315ca342d96884bf168eaf4716d4d31f5`. The Python feature adds the MIT-licensed OxideAV Theora 0.0.13 decoder. The locked license audits passed for 343 core and 315 Node packages and the wheel included the updated attribution files. All ArtCraft revisions and the Node lockfile match the preceding checkpoint. [OxideAV Theora](https://github.com/OxideAV/oxideav-theora)

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 5 GiB of generated output, including both Cargo profiles, the Node addon, temporary wheel and virtual environment, and test EPUBs.

## 2026-11-24 Editable MathML horizontal padding

The installed regression compares pixel positions in the EPUB previews and reopened DOCX. It checks the 8 pt nested left offset, the combined 20 pt additive advance, and the fixed 72 pt width with a 4 pt inset. Native math XML retains the three operands once each, the zero-width and zero-descent flags, and a nested fraction and radical inside a horizontally merged Word table cell. Empty padding without an operand emits a sized math space. Negative or out-of-range offsets, vertical resizing, content-relative advance changes, gaps below the native run-size resolution, and advances wider than the DOCX text area return explicit errors. [MathML padding dimensions](https://www.w3.org/TR/MathML3/chapter3.html#presm.mpadded), [OMML phantom properties](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.math.phantomproperties)

A separate authored EPUB produced searchable PDF text and editable OMML. Replacing the two OMML operands with longer text moved the follower of the additively padded operand by 158 px at 144 DPI. The follower of the fixed-width operand stayed at the same position. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages. The authored EPUB SHA-256 is `fe60902778e489c62145084e26efeaf9b1fe2acfabd25744d92045a9a29ff9a3`. The original *Linear Algebra* EPUB rendered all 94 chapters at 24 DPI. Its SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 wheel passed 705 tests in a fresh environment, including 110 EPUB tests, with zero failures, errors or skips. It measured 23,923,742 bytes with a 55,588,712-byte native extension. Wheel SHA-256 is `9f5efb5a4974bf939ca5a33d071e5a514395cf47232e5991e5551ec3a245e94e`. The core and Node inventories, lockfiles and all ArtCraft revisions match the preceding checkpoint. License audits passed for 343 core and 315 Node packages. The installed wheel includes the source attribution files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed approximately 6.2 GiB of Cargo output across the validation runs, along with the temporary wheels, virtual environments, Node addon, site builds and test EPUBs.

## 2026-11-24 CSS font-face ranges and defaults

The installed regressions match reflowable and fixed-layout XHTML previews against explicit variation controls. They cover fractional weight matching, clamping above a declared weight range, CSS weight search around 450, width preference across two fractional ranges, and cascade order between `font-width` and `font-stretch`. Descriptor defaults apply independently to glyphs from two Unicode-restricted faces. Optical sizing and element-level axis values override descriptor defaults. A fixed-layout SVG uses its `font-stretch` presentation attribute to select between two packaged width descriptors. A separate resource probe rejected more than 512 registrations with the expected limit error. [CSS font matching](https://www.w3.org/TR/css-fonts-4/#font-matching-algorithm), [Variation precedence](https://www.w3.org/TR/css-fonts-4/#feature-variation-precedence)

An authored EPUB used Roboto Flex 3.200 with declared width bounds of 75–90%, a requested width of 114%, a weight of 610.5, and a face-level grade setting. Its reflowable and fixed-layout previews matched explicit `wdth` 90, `wght` 610.5 and `GRAD` 80 controls. The reopened fixed-layout PDF matched the preview pixels. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages. The EPUB SHA-256 is `fea5f1347688334c63248cc81c773a641bd538dc5a69a4faffd319720188b3d4` and its Roboto Flex font SHA-256 is `a55c1e67f6dcf27f2bb71dc3e4c03d3abcbc5054411aac943b2f94985195825e`. The original *Linear Algebra* EPUB rendered all 94 entries at 24 DPI. Its SHA-256 is `87e523d8e16c2d1f4b211b47c0495841687a3573c828d3ef36b1f77f91b2abd4`. [Roboto Flex 3.200](https://github.com/googlefonts/roboto-flex/releases/tag/3.200), [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 706 tests in a fresh environment, including 111 EPUB tests, with zero failures, errors or skips. It measured 23,951,308 bytes with a 55,669,144-byte native extension. Wheel SHA-256 is `180f48e242029298c49b0b07b515c88c8f4e0a9fb7271581801bc4b0b53a6d75`. Both lockfiles and all ArtCraft revisions match the preceding checkpoint. License audits reviewed the 343 core and 315 Node packages and verified the wheel's attribution files.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 5 GiB of Cargo build output and removed the temporary wheels, virtual environment, Node addon, generated site and test publications.

## 2026-11-24 Embedded WebM VP8, VP9 and Opus

The installed regression checks twelve VP8 frame timestamps, a 4:3 pixel aspect ratio, red and blue frames, a seek from a preceding keyframe and WebVTT cue changes inside picture intervals. Its matched 48 kHz mono Opus soundtrack returned 55,200 playable samples. A clipped 380 ms section returned 18,240 samples, exactly matching the corresponding PCM slice. A chapter revision with an authored time fragment retained the media bytes and returned the same audio and video intervals. Truncated EBML returned a malformed-document error. The VP9 regression checks 8-bit 4:4:4 and 10-bit 4:2:0 colour, source timestamps and late seeking. [WebM container guidelines](https://www.webmproject.org/docs/container/), [Matroska codec mappings](https://www.matroska.org/technical/codec_specs.html)

The three committed WebM fixtures are synthetic FFmpeg outputs. The VP8 fixture uses a 96 × 64 field that changes from red to blue at 750 ms, at eight frames per second with a 560 Hz sine encoded as Opus. Its SHA-256 is `587563bea1546a246741bdc8c74e6f0e4807266bb84df584b3c63af3b4a465c1`. The 8-bit VP9 fixture uses a 66 × 50 red and blue field at seven frames per second with BT.709 matrix signaling. Its SHA-256 is `f3cbb9e4369379ac2281cbc3d42695a86405575248f1662feabc44e90484232b`. The 10-bit VP9 fixture uses a 64 × 48 blue-green field at six frames per second with limited range and SMPTE 170M color signaling. Its SHA-256 is `9b2539adbd969417a2765e414f8ccd44f694b48006f8c76f2513532756f5357f`.

Independent checks matched all Opus samples against libopus at 101.72 dB signal-to-noise and verified identical PCM from laced and unlaced audio-only WebM. A 12-bit VP9 fixture matched the first FFmpeg RGB frame byte for byte. A two-pass VP9 sample decoded all 48 visible pictures with five reference-only pictures carried in five superframes and passed a late seek. An unknown-size WebM segment produced six expected frames. A 2048 × 1088 coded stream was rejected before decoding. The authored captioned EPUB with a WebM source and an MP4 alternative passed EPUBCheck 5.4.0 with zero fatals, errors, warnings or informational messages. Its SHA-256 is `cedbf970ed2398f83f916d6650a8d5bc5d2c11941811ffc559c1cc1af40fce38`. The five public AVC and AAC excerpts from *Creative Commons Shared Culture* still produced five pictures and 9,600 samples per channel apiece. The public EPUB SHA-256 is `7bec3acf5ca153fd8a3b8bba50549c5cde3cb617ddd74c1d3b26fbc5eb9eebdf`. [EPUB 3 sample table](https://idpf.github.io/epub3-samples/30/samples.html)

The Linux x86-64 CPython 3.11.15 release wheel passed 708 tests in a fresh environment, including 113 EPUB tests, with zero failures, errors or skips. It measured 24,452,023 bytes with a 56,871,048-byte native extension. Wheel SHA-256 is `4f0ead2a7c8d2d540002c844b7b60c2737feb7596db1af902ff9538da4c0145b`. The Python feature adds the MIT-licensed OxideAV Matroska 0.0.12, VP8 0.2.9 and VP9 0.0.15 crates. Locked license audits verified 346 core and 315 Node packages and the updated attribution inside the wheel. The ArtCraft revisions and Node lockfile match the preceding checkpoint. [OxideAV Matroska](https://github.com/OxideAV/oxideav-mkv), [OxideAV VP8](https://github.com/OxideAV/oxideav-vp8), [OxideAV VP9](https://github.com/OxideAV/oxideav-vp9)

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default tests reported 479 unit tests and 7 integration tests passed. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 preview tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 5.1 GiB of Cargo build output plus test publications, temporary codec source downloads, the wheel and virtual environment, the generated site and the Node addon.

## 2026-11-24 WebM Vorbis audio

The installed regressions cover stereo channels with distinct tones, leading silence, VP8 and WebVTT presentation timing, clipped versus full PCM, a chapter revision with a normal-play-time fragment, and an excessive setup codebook entry count rejected before decoder allocation. The audio-only regression uses one Xiph lace and a final block group that trims 441 frames. Streamed PCM from the authored SMIL overlay matches the embedded audio and the corresponding portion of the untrimmed soundtrack at several chunk sizes. [Matroska codec mappings](https://www.matroska.org/technical/codec_specs.html), [Vorbis I specification](https://xiph.org/vorbis/doc/Vorbis_I_spec.pdf)

The VP8 fixture is synthetic FFmpeg 8.0.1 output with 96 × 64 red and blue fields at eight frames per second, a 4:3 sample aspect ratio, and 340 and 560 Hz tones encoded as 44.1 kHz stereo Vorbis. Its SHA-256 is `8752df78164cefbd7c17d8d765b121573034f6bcd45d78f315dde8faacbe3b38`. The audio-only fixture remuxes the same coded audio with lacing and 10 ms of final discard padding. Its SHA-256 is `afeb5052ab5d28473c2a997857905408ca0a187985e05881c362a1fd01d49013`. The 50,752 audible stereo frames match the independent FFmpeg reference to within one 16-bit PCM step, at 104.56 dB signal-to-noise. A separate 16 kHz mono sample reached the same one-step bound for all 7,552 audible frames. FFmpeg was used to generate fixtures and as a validation reference. Playback uses the in-process decoder.

The isolated Linux x86-64 CPython 3.11.15 wheel passed all 123 tests in the EPUB suite, including all four WebM regressions. It measured 24,651,835 bytes with a 57,307,960-byte native extension. Wheel SHA-256 is `2449cc47ce0a5f8dc38450fe20138b2077fcf6be47e5efe7b4b0c184d8e1d1ed`. The Python feature adds the MIT-licensed OxideAV Vorbis 0.0.12 decoder and its OxideAV Ogg 0.1.9 dependency. The audit reviewed 348 normal and build packages, verified Node binding licenses and compared all three wheel attribution files byte for byte. ArtCraft pins and the Node lockfile match the preceding checkpoint. Rust formatting and warnings-denied all-target Clippy passed. Cleanup removed 1.7 GiB of Cargo build output and the temporary wheel and environment. [OxideAV Vorbis](https://github.com/OxideAV/oxideav-vorbis), [OxideAV Ogg](https://github.com/OxideAV/oxideav-ogg)

## 2026-11-24 Fixed-layout PDF bookmarks

The installed-wheel regression exports nested EPUB 3 navigation and EPUB 2 NCX through the fixed-layout PDF API. It checks repeat and reordered selections, pruning omitted entries, the fallback destination for headings with retained children, a percent-encoded XHTML ID, an SVG destination under a transform and a nonzero viewBox origin, and identical preview and reopened PDF pixels. A synthetic publication with 2,048 outline entries reopens with all 2,048 entries. [EPUB navigation requirements](https://www.w3.org/TR/epub-33/#sec-nav), [PDF destinations and outlines](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf)

The unmodified *Page Blanche* and *SVG-in-spine* samples export all 10 and 6 pages respectively in reverse spine order at 48 DPI. The reopened pages match the source viewport dimensions. Their PDF outlines have two and three bookmarks with resolved destinations. The source SHA-256 values are `d2efb562f17d43cef50578d4d67dd7fd991f8f254b6f5a7c28bde7461ea783eb` and `ed6b1b9e99245ac7747ced1a2f90bad1acab558280cffc7daed3e54ee48e49bb`. EPUBCheck 5.4.0 reported zero fatals, errors, warnings and informational messages for *SVG-in-spine*. [W3C EPUB samples, 20230704 release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

XHTML inline-only and other unresolved fragments fall back to the chapter origin. Clickable content hyperlinks and searchable text remain open in the [EPUB tasks](tasks.md).

The Linux x86-64 CPython 3.11.15 wheel passed 709 tests in a fresh environment, including 114 EPUB tests, with zero failures, errors or skips. It measured 24,460,435 bytes with a 56,885,912-byte native extension. The wheel SHA-256 is `f3cd4bcbbe1393a37ae30a793648981df1516942208ecca8f4f37a5334d5858a`. The root and Node lockfiles and upstream revisions match the preceding checkpoint. The audit reviewed 346 core normal and build dependencies, verified the Node dependency licenses, and matched the wheel's three attribution files to the source.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit tests and 7 integration tests. The Python-free suite passed 401 unit tests and 8 integration tests. All 17 Node 22.22.1 tests passed. Public typing, documentation links, support evidence and roadmap checks passed. Cleanup removed about 6.4 GiB of Cargo build output across both release builds and the native test build, plus the temporary wheel, environments, samples, site and Node addon.

## 2026-11-24 Fixed-layout PDF links

The installed regressions reopen PDFs from fixed-layout XHTML and SVG and check clickable areas against layout coordinates. The XHTML cases cover wrapped text with an overflow clip, built-in and packaged fonts, a sub-point image link, a positioned block, percent-encoded inline targets, and retained links between reordered and repeated pages. The inline equation case checks that a linked fraction occupies a clickable area and appears before the next link. The SVG case checks a transformed viewport, href precedence, clip bounds, a hidden object, rejected removal of a referenced anchor, and the new destination after a chapter edit. Reopened page pixels match the EPUB previews. Missing, omitted and unsafe targets stay inert. [PDF link annotations and destinations](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf)

A separate authored EPUB crossed between XHTML and SVG anchors in both directions. A three-page reordered export retained five links and both bookmarks. EPUBCheck 5.4.0 reported zero fatals, errors, warnings or informational messages. The source SHA-256 is `4f18332023e6b9fa3362b0cdcfe1977d9edf176c97ab3371a09b3a27bc93ba42`. All 16 pages of the original *Page Blanche* and *SVG-in-spine* samples rendered and reopened in reverse order with their bookmarks and viewport dimensions. [W3C EPUB samples, 20230704 release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

Links within SVG text spans and SVG embedded in XHTML remain open, along with searchable page text. Outline destinations with unresolved fragments fall back to the chapter origin.

The Linux x86-64 CPython 3.11.15 wheel passed 712 tests in a fresh environment, including 117 EPUB tests, with zero failures, errors or skips. It measured 24,498,410 bytes with a 56,979,672-byte native extension. The wheel SHA-256 is `974e420620b192dac6ef2d16f17c42586574c96a4d2c38c9727b630f463fa37d`. The lockfiles and upstream revisions match the preceding checkpoint. The audit reviewed 346 core normal and build dependencies, verified the Node dependency licenses, and matched the wheel's three attribution files to the source.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit and 7 integration tests. The locked Python-free suite passed 401 unit and 8 integration tests. All 17 Node 22.22.1 checks passed, along with public typing, the documentation site, support evidence, and roadmap checks. Cleanup removed about 6.4 GiB of Cargo output across release and test builds, along with temporary EPUBs, wheels, virtual environments, the Node addon and the generated site.

## 2026-11-24 Searchable fixed-layout PDF

The installed regression checks repeated and reordered XHTML and SVG pages, text and search rectangles from built-in and packaged fonts, ligatures and Unicode, link retention, MathML fractions, and block and inline SVG embedded in XHTML. CSS-hidden text and text outside the clipped viewport are excluded. Searchable and visual PDF exports have identical rendered RGBA pixels. The visual export keeps its text-free default. [PDF invisible text and Unicode mapping](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf)

An independently authored EPUB passed EPUBCheck 5.4.0 with zero fatals, errors, warnings or informational messages. Its reordered export retained both bookmarks, its internal link and readable text on all three output pages. The system Poppler command was run separately as an independent PDF validation tool. It extracted the original Unicode, composed MathML and SVG text and returned empty text for the paired visual PDF. The authored EPUB SHA-256 is `6b725c2f0e6619a2449b7d82d779e4cd511277faa50ea61d3b56c9da9e8a2970`. The packaged-font check found Arabic and Hebrew phrases, the word `office` and a decomposed accent at their placed coordinates. The original *Page Blanche* and *SVG-in-spine* samples reopened with their bookmarks and matching viewports and pixels across all 16 pages. [W3C EPUB samples, 20230704 release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

Search rectangles follow glyph or cluster extents. Overlapping graphics and nonrectangular SVG clipping or masks can cover searchable text.

The Linux x86-64 CPython 3.11.15 release wheel passed 713 tests in a fresh environment, including 118 EPUB tests, with zero failures, errors or skips. It measured 24,519,101 bytes with a 57,026,776-byte native extension. Its SHA-256 is `a46749dec18f19567bec4c1a8c0ebddf9e368e3e22de85700eab326bf33ad85f`. The lockfiles and upstream revisions match the preceding checkpoint. The license audit reviewed 346 core normal and build dependencies, verified Node binding licenses and matched all three attribution files in the wheel to the source.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit and 7 integration tests. The locked Python-free suite passed 401 unit and 8 integration tests. All 17 Node 22.22.1 tests passed, along with public typing, documentation links, support evidence, and roadmap checks. Cleanup removed about 5.1 GiB of Cargo output and pruned the temporary wheel, virtual environments, EPUB and PDF fixtures, Node addon and generated site.

## 2026-11-24 XHTML oblique font-face ranges

The installed regression distinguishes otherwise identical font programs by their horizontal advances. It verifies signed range matching, the 11-degree directional boundary, reversed descriptor endpoints, variable substitution, valid angle units, ignored invalid declarations, the automatic descriptor, and the precedence of width, style and weight, including equal zero-angle and upright faces. Adjacent spans with different requested angles retain their independently selected faces. Fixed-layout visual PDFs reopen with the same pixels as their XHTML previews. [CSS font style and matching](https://www.w3.org/TR/css-fonts-4/#font-style-prop), [Font property descriptors](https://www.w3.org/TR/css-fonts-4/#font-prop-desc)

A separate authored EPUB with Roboto Flex 3.200 matches a 9.5-degree request to the declared 7.25-degree endpoint. The reopened preview matches an explicit OpenType `slnt` value of `-7.25`, and renders differently under a wider descriptor or a lower requested angle. A separate check confirms fractional slant positions, face-level defaults and element-level overrides. Chapter replacement retains the font bytes and recalculates the preview. The authored searchable PDF retains the label and viewport. EPUBCheck 5.4.0 reported zero fatals, errors, warnings or informational messages. The EPUB SHA-256 is `10b67bc82d31debded9c7ba02101ced983a38d037b52a3b4f060e1547e2399f9` and the font SHA-256 is `a55c1e67f6dcf27f2bb71dc3e4c03d3abcbc5054411aac943b2f94985195825e`. All ten pages of the original *Page Blanche* sample reopened in reverse order with their viewports and bookmarks. The source SHA-256 is `d2efb562f17d43cef50578d4d67dd7fd991f8f254b6f5a7c28bde7461ea783eb`. [Roboto Flex 3.200](https://github.com/googlefonts/roboto-flex/releases/tag/3.200), [W3C EPUB samples](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The Linux x86-64 CPython 3.11.15 wheel passed 714 tests in a fresh environment, including 119 EPUB tests, with zero failures, errors or skips. It measured 24,532,386 bytes with a 57,044,184-byte native extension. Its SHA-256 is `9a1ff1c2795986ff1ddcb711ed74554579cafd5f20c521c145db0e2135e8621f`. Both lockfiles and all upstream revisions match the preceding checkpoint. The license audit reviewed 346 core normal and build dependencies, verified Node binding licenses, and matched all three attribution files in the wheel to the source.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit and 7 integration tests. The locked Python-free suite passed 401 unit and 8 integration tests. All 17 Node 22.22.1 preview tests passed, along with public typing, documentation links, support evidence and roadmap checks. Cleanup removed about 6.4 GiB of Cargo output across both release builds and the native test build, plus the temporary wheel, virtual environments, sample EPUBs, external font fixture, generated site and Node addon.

## 2026-11-24 Fractional XHTML synthetic oblique

The earlier wheel renders both signs of a 0.75-degree request at the same upright ink position. The installed regression now verifies that a packaged upright face leans in the requested direction at subdegree angles in both chapter layouts. It distinguishes 13.25 and 13.75 degrees, retains different angles on adjacent spans, honors a declared oblique face range, and checks the explicit paint limit at 90 degrees. The fixed-layout PDF reopens with matching pixels and searchable text. [CSS Fonts 4 font-style](https://www.w3.org/TR/css-fonts-4/#font-style-prop)

An independent fixture with Roboto Flex 3.200 verifies that a positive 0.75-degree request matches an explicit `slnt` value of −0.75. The opposite request matches the upright axis endpoint. The temporary font has SHA-256 `a55c1e67f6dcf27f2bb71dc3e4c03d3abcbc5054411aac943b2f94985195825e`. [Roboto Flex release 3.200](https://github.com/googlefonts/roboto-flex/releases/tag/3.200)

The isolated Linux x86-64 CPython 3.11.15 wheel passed all 122 EPUB tests. Rust 1.95 formatting and warnings-denied release Clippy passed. The documentation checker verified 35 pages and 2,317 local references. The wheel measured 24,566,602 bytes with a 57,130,152-byte native extension and SHA-256 `630548d4ccfa56f72d0985653c74c22577c3a807875590ef45e20d9e68345a53`. Cleanup removed 1.6 GiB of Cargo output, both wheels, their virtual environments and the temporary font archive.

## 2026-11-24 SVG variable font-face ranges

Installed regressions compare fractional font weights against explicit axes, check width and signed oblique face ranking, and exercise per-face defaults in Unicode fallback. They verify that automatic optical sizing and explicit element axes take precedence, including an override in a local tspan. A single selected face retains authored textLength, while a mixed-face run that requires splitting returns an explicit unsupported error for that placement. The searchable fixed-layout PDF retains the original label and reopens with the preview pixels. [CSS font matching and variation precedence](https://www.w3.org/TR/css-fonts-4/)

An independently authored SVG-spine EPUB uses Roboto Flex 3.200. Its 360 × 90 point preview matches an explicit axis control with a weight of 610.5, width of 90, slant of −7.25, grade of 40 and optical size of 24. Widening either matched interval or changing the grade updates the pixels. Chapter replacement retains the font bytes. The visual and searchable PDFs reopen with identical pixels and only the searchable version extracts the label. EPUBCheck 5.4.0 reported zero fatals, errors, warnings or informational messages. The EPUB SHA-256 is `18447920b560ffc3fe6a68f35ad6d80ef2fef83f86b2d1bf4caf4cbce81daec3` and the font SHA-256 is `a55c1e67f6dcf27f2bb71dc3e4c03d3abcbc5054411aac943b2f94985195825e`. [Roboto Flex 3.200](https://github.com/googlefonts/roboto-flex/releases/tag/3.200)

The original *SVG-in-spine* and *Page Blanche* samples reopen in reversed order with bookmarks and match preview pixels on all 16 pages at 96 DPI. At 60 DPI, their preview and PDF raster sizes agree and 13 pages have small pixel differences. Per-instance variable typography inherited through SVG use elements and complex text fallback remain open. [W3C EPUB sample release](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The Linux x86-64 CPython 3.11.15 wheel passed 715 tests in a fresh environment, including 120 EPUB tests, with zero failures, errors or skips. It measured 24,549,796 bytes with a 57,076,568-byte native extension. Its SHA-256 is `f570ff242ab186ff60823b9cecd220ec284c04288b4720176bcf6f330516bf99`. This was a local Linux wheel. Both lockfiles and all upstream revisions match the preceding checkpoint. The license audit reviewed 346 core normal and build dependencies, verified Node binding licenses and matched all three attribution files in the wheel to the source.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit and 7 integration tests. The locked Python-free suite passed 401 unit and 8 integration tests. All 17 Node 22.22.1 tests passed, along with public typing, documentation links, support evidence and roadmap checks. Cleanup removed about 5.1 GiB of Cargo output, plus the temporary wheel, virtual environments, EPUBs, external font fixture, generated site and Node addon.

## 2026-11-24 Fixed-layout PDF pixel grid

The pre-change wheel embedded the original preview pixels but fit the image to fractional point bounds. At 47.25 DPI the reopened PDF resampled some fully covered samples. In a separate near-integer fixture the EPUB preview rounded up to an extra column that the PDF renderer omitted. The installed regression verifies XHTML and SVG pages at 47.25, 60 and 96 DPI and at a scale 0.005 pixel beyond a whole width. It checks the original PDF page size, the embedded image dimensions and the fully covered pixels.

The *SVG-in-spine* and *Page Blanche* EPUBs reopen in reverse order with bookmarks and original page sizes. All 16 reopened PDF pages match the respective preview pixels at 96 DPI. At 47.25 and 60 DPI, all 16 match within the fully covered area. Thirteen pages differ only in a partial final row or column where the PDF renderer applies its page clip. [W3C EPUB samples](https://github.com/IDPF/epub3-samples/releases/tag/20230704)

The Linux x86-64 CPython 3.11.15 wheel passed 716 tests in a fresh environment, including 121 EPUB tests, with zero failures, errors or skips. It measured 24,551,725 bytes with a 57,076,120-byte native extension. Its SHA-256 is `b32d50ee4ebe5b403e3750a92f882748d2f43e76016629890492a869e09fdaae`. Both lockfiles and upstream revisions match the preceding checkpoint. The license audit reviewed 346 core normal and build dependencies, verified Node binding licenses and matched all three attribution files in the wheel to the source.

Rust 1.95 formatting and warnings-denied all-target Clippy passed. The locked default suite passed 479 unit and 7 integration tests. The locked Python-free suite passed 401 unit and 8 integration tests. All 17 Node 22.22.1 tests passed, along with public typing, documentation links, support evidence and roadmap checks. Cleanup removed about 5.1 GiB of Cargo output and pruned the temporary wheels, virtual environments, raster fixtures, generated site and Node addon.

## 2026-11-24 Editable MathML vertical spacing and measured padding

The installed regression projects explicit ascent and descent, additive vertical padding, content-relative width and inset, and vertical math spaces to editable Office Math nodes. It compares relative glyph positions in the EPUB preview and reopened DOCX. Removing the descent leaves both formula baselines in place and moves the following paragraph up. A fraction in a native merged cell retains its operator tree and both operands. XML inspection verifies invisible zero-width phantoms and their independent ascent and descent flags. Extracted DOCX text preserves the authored labels once each and omits generated metric guides. Excessive vertical extents return an explicit unsupported error. [MathML padding](https://www.w3.org/TR/MathML3/chapter3.html#presm.mpadded), [MathML Core spacing](https://www.w3.org/TR/mathml-core/#space-mspace)

All 20 MathML tests passed against the installed Linux x86-64 CPython 3.11.15 wheel. Rust 1.95 formatting and locked release all-target Clippy with warnings denied passed. The dependency pins and lockfiles match the previous checkpoint.

## 2026-11-24 Staged SVG resource anchors

The pre-change integrated wheel allowed a packaged SVG replacement to orphan a retained fragment. The installed regression confirms a percent-encoded link from a retained or newly inserted XHTML chapter prevents removal of its existing target. The editor accepts a staged link to the SVG’s new ID and leaves the source members unchanged on rejection. The two staged chapter-anchor regressions also passed. Rust formatting and warnings-denied release Clippy passed.
