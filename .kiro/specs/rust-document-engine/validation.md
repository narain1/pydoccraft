# Validation evidence

## 2026-10-20 DOCX properties and PDF markup (unreleased)

- Runtime commits `849de87` and `70b636a` passed the combined offline Linux x86_64 corpus: Rust 1.95.0 formatting, 186 native tests, warnings-denied Clippy, locked Maturin 1.8.1 release build and 203 installed-wheel Python 3.11.2 tests, zero skips, with verified artifact identity.
- DOCX metadata tests verify typed core/custom XML and native readback, ordinary source core extraction, escaping and whitespace rules, unchanged caller timestamps/revision, independent nested snapshots/copies, atomic invalid batches, retained ranges on failure, bounded JSON encoding, count/serialized-value capacity and reclamation. Existing content and warmed PNG output remain unchanged. No raw XML or property wrapper is exposed. DOC7 stays open for source custom inventory, language and extended/application properties.
- PDF markup tests cover underline/strikeout/squiggly creation through the existing annotation API, searchable body text, metadata updates, moving/recoloring/deleting, reopened native values, stale handles and warmed-render rollback. Actual colored pixel bands follow all four rotations with CropBox/UserUnit. Appearance preflight rejects excessive/malformed quads and finite-geometry/work violations before native props/regeneration; generated raw AP bytes are charged without imported-stream decoding. Native squiggly limits coarsen the complete line. PDF6 remains open for broader imported-field, radio and reply/appearance support.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,316,919 bytes; native extension: 40,160,184 bytes. SHA-256: `726684366a87cdd198c8cf14edac95502eaec4618f2e5e02cbc63bbeb4a8c8f2`. Lockfile SHA-256: `fcfb4a174387cf25cceddadeba9e6be930ec790c840dca0bb5f169431a6c089c`. Dependencies, all four upstream revisions, the previously reviewed 264-package permissive graph and attributed assets are unchanged.

## 2026-10-20 integrated picture lifecycle, ownership and shading acceptance (unreleased)

- Runtime integration commit `1750702` includes merged parser reuse/selective extraction (PR32/34), paragraph borders, the supplied-font reuse/ink corrections and the picture/ownership batch. Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 177 Rust tests, warnings-denied Clippy, locked release build and 196 installed-wheel Python tests passed with zero skips and verified artifact identity.
- Picture.data replacement accepts bounded bytes/path/stream sources, preserves placement/alt text, changes actual cached preview pixels and keeps copies independent. Native public add_media and story/path traversal deduplicate live bytes and reclaim orphans after picture/cell/grid deletion. Tests cover all-story references, collision-safe reinsertion, 1,001 add/remove cycles, the 1,000-placement limit and exact 64 MiB aggregate capacity with atomic overflow/recovery. Deletion retains the editable anchor paragraph and invalidates the removed picture. MED1 is complete for the documented PNG authoring profile.
- Collection iteration now captures handles before yielding, preventing pending PDF positions from silently acquiring a new generation after page/annotation/outline edits. Stable DOCX/slide identity, slice membership, independent copying, retained parent lifetime and closed-owner behavior pass; unsupported detached child copies/reparenting are documented. API5 is complete within the existing format-specific ownership contract.
- BG6 acceptance verifies actual DOCX/PDF fills and native XML: run shading reset, highlight disable versus inheritance, paragraph/cell style restoration and document-default clearing remain independent from page backgrounds. No duplicate shading API was added.
- The supplied-font public regression now passes against the installed wheel, including two/six stacked marks, cached-render rollback, four rotated CropBox/UserUnit views, ordinary Unicode and unchanged point behavior. This supersedes the pending installed-wheel statement in the earlier ink-correction note; it is combined integration evidence, not PR35's isolated CI artifact.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,299,619 bytes; native extension: 40,127,688 bytes. SHA-256: `8e1245a864d8233025b7097070daf08c665946665572518e2ec959404cbc6d44`. Lockfile SHA-256: `fcfb4a174387cf25cceddadeba9e6be930ec790c840dca0bb5f169431a6c089c`. The direct already-locked Skrifa edge and attribution are recorded below; the regenerated 264-package permissive graph, all four upstream revisions and assets remain unchanged. Broader unchecked criteria and upstream fidelity gaps remain open.

## PDF supplied-font ink review correction

- Independent review reproduced stacked combining marks rendering 26 points above a requested text box. Boxed static TrueType text now checks actual native shaped outlines against the box before commit, retaining legacy point placement. Built-in fonts still use logical metrics; boxed CFF is explicitly unavailable until native evaluation can be bounded.
- Resource validation uses public parsed component iterators before recursive outline lookup: 4,096 distinct glyphs, 100,000 composite visits, depth 16, 8,192 points per glyph, 100,000 aggregate path operations, at most 16,384 operations per glyph and 1 MiB scratch per glyph. Native unhinted outlines and Kurbo curve extrema supply geometry; no glyph parser or replacement shaping engine is added.
- All 44 PDF native tests, formatting and locked all-targets warnings-denied Clippy passed. A public regression covers two/six stacked marks, cached-output rollback, four rotations with CropBox/UserUnit, ordinary Unicode and unchanged point placement; its installed-wheel run passed in the exact-head integration validation below.
- The only lockfile change adds a direct edge to already-locked skrifa 0.44.0 (MIT OR Apache-2.0). Regenerated and reviewed license inventories are byte-identical at 264 permissive normal/build packages, with no prohibited crates. NOTICE records direct outline use; all upstream revisions remain unchanged.

### PDF text-box integration review

- Exact runtime head `52fd5f801000f28fcdc79364df909010090a095a` passed CI 39467474191: formatting, 165 Rust tests, warnings-denied Clippy, 184 installed-wheel Python tests (no skips), and the regenerated 264-package license audit. Both review corrections and current main's parser-session/selective-extraction changes are included.
- CPython 3.11.17 / Linux x86_64 artifact: wheel 17,286,676 bytes; native extension 40,094,512 bytes; wheel SHA-256 `2bb96a9e6457a5ea196454cd7ed4ac4877709a88c23a296c852f4febfaa8e836`. The only dependency change is the already-locked Skrifa direct edge; all upstream revisions remain unchanged.

## 2026-10-20 DOCX paragraph borders (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 168 Rust tests, warnings-denied Clippy, locked release build and 176 installed-wheel Python tests passed with zero skips and verified artifact identity. The initial Python run rejected a test's unsupported named color; using its explicit RGB value fixed the fixture, and the complete installed suite passed.
- Existing paragraph/style/default formatting shares four-edge borders, native whole-group inheritance, direct per-edge patches and clear/reset semantics. Native DOCX readback, independent copies, visible body/cell/header/footer PNG/PDF borders and atomic invalid edits pass. Table-style borders retain their six-edge table meaning. TXT3 is complete within the documented supported border profile.
- Native regressions show unsupported border substitutions and width clamps, absent between borders, empty vertical waves and overlapping vertical double lines. These inputs now fail explicitly across shared DOCX paragraph/table/cell/style formatting. Horizontal double/wave and supported single/dotted/dashed/thick widths remain native; remaining shared-edge/style fidelity stays upstream work.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,225,412 bytes; native extension: 39,902,104 bytes. SHA-256: `42a7fc8f333f10171e212b5a9490938ec5d353fed0787973b6b716f9b79e7003`. Lockfile SHA-256: `f5a35e9549115b905196b81ceebfad1475f01571a5c6416e6e15e10da1bdc984`. Dependencies, all four upstream revisions and the reviewed 264-package permissive license graph are unchanged. This artifact includes the supplied-font reuse correction below, but predates the separate PDF ink-bounds review correction; it does not clear that pending review.

## PDF supplied-font reuse correction after text-box validation

- A deterministic native regression shows the bounded font lookup corner: an 81,446-byte imported source with 256 PCFull-prefixed font bindings and 8,200 earlier reachable objects plus a 411,640-byte font produces three full font programs for two lines while prior accounting charges only once. Boundary controls with 255 bindings or a smaller object graph reuse one program.
- Supplied-font point/box drawing now rejects 256 or more effective inherited PCFull bindings before embedding. Direct/inherited cases retain serialized bytes and counters on failure; the 255-binding boundary and ordinary repeated font calls pass.
- Focused validation: all 42 PDF native tests, formatting and locked all-targets warnings-denied Clippy passed. No dependencies changed. The preceding 159-Rust/173-Python artifact below predates this correction; exact updated-head artifact validation is required before PR #35 merges and will be recorded by independent review.

## 2026-10-20 PDF text boxes and image transforms (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 159 Rust tests, warnings-denied Clippy, locked release build and 173 installed-wheel Python tests passed with zero skips and verified artifact identity. A test initially used a nonexistent Page.images property; it was corrected to the existing get_images API and the complete wheel suite rerun successfully.
- Existing draw_text accepts point or box geometry. Native built-in wrapping/alignment, supplied-font explicit Unicode lines, logical metric fits, exact-height boundary/overflow rollback and conservative wrapping-work limits pass. No new public text-box object, custom wrapping engine or glyph-outline clipping is introduced. Automatic supplied-font wrapping/justification and strict text clipping remain unavailable; PDF2 stays open.
- Existing add_image accepts native visual crop, quarter-turns and flips. Asymmetric fixtures verify all rotations/flips, both crop-side orders, composed transforms, outside-box pixels and retained embedded media. Rotated CropBox/UserUnit tests verify current-view placement for text and images. Both drawing paths retain staged resource-preservation checks.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,220,113 bytes; native extension: 39,892,088 bytes. SHA-256: `86014f3579ad5c8dec1bcd6bb6cf119e6c554e0d5f55436aa424389708581f45`. Lockfile SHA-256: `f5a35e9549115b905196b81ceebfad1475f01571a5c6416e6e15e10da1bdc984`. Dependencies, all four upstream revisions and the reviewed 264-package permissive license graph are unchanged.

## 2026-10-20 DOCX pagination acceptance and overflow guard (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 153 Rust tests, warnings-denied Clippy, locked release build and 168 installed-wheel Python tests passed with zero skips and verified artifact identity.
- LAY5 acceptance now covers heading/table keep-with-next, fitting paragraph keep-together, both widow/orphan edges and page-break-before reset. Native DOCX/PDF page counts, exact ordered labels, in-page PDF text bounds and visible PNGs are checked. Fitting non-splitting rows move intact; splitting cell paragraphs continue across pages without loss or duplication.
- A native upstream reproducer confirms an oversized unsplittable row places the final PDF line around y353–365 on a 300pt page. Before authoring serialization, public native placed-cell geometry now rejects physical-page overflow in unmerged non-splitting, exact-height and repeated-header rows. This protects save, preview, page count and PDF export after text/font/margin changes. Failed saves preserve existing files; enabling splitting recovers complete visible output.
- Intentional exact-height clipping within a page, ordinary header/footer tables and multi-page vertical merges pass. The narrow guard excludes vertical-merge geometry and checks 10,000-page/2,000,000-item traversal limits after native layout, not before allocation. Guarded documents incur an extra layout pass on serialization. TBL6 remains open for complete merged-row overflow semantics and upstream resource improvements.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,216,051 bytes; native extension: 39,884,920 bytes. SHA-256: `796226eb1d731a250a2c8b2583c74ad9126b184925e77a65138f0061097bf036`. Lockfile SHA-256: `f5a35e9549115b905196b81ceebfad1475f01571a5c6416e6e15e10da1bdc984`. Dependencies, all four upstream revisions and the reviewed 264-package permissive license graph are unchanged.

## 2026-10-20 PDF drawing resource preservation (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 148 Rust tests, warnings-denied Clippy, locked release build and 159 installed-wheel Python tests passed with zero skips and verified artifact identity.
- Native reproductions demonstrate original pixels changing after fixed font-name, opacity-name and image-name collisions, including inherited page resources. All five drawing paths now verify original effective bindings before committing. Rejected operations retain serialized bytes, counters, cached PNGs and source values. Unaffected writers still add text/images while the original rendered region remains byte-identical.
- Matching checks use shared native object identity or bounded scalar dictionaries; no recursive object comparison, stream decoding or resource rewriting is added. Tests cover same-reference changed objects, exact numeric comparison, binding-count limits and ordinary repeated drawing. Complex direct dictionaries can fail conservatively when preservation cannot be proved; collision-safe allocation remains upstream work.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,213,441 bytes; native extension: 39,873,528 bytes. SHA-256: `f4dc68990f5a7d10a738cbde047658d8f054bc2cb548c68e024633ae4377c9c9`. Lockfile SHA-256: `f5a35e9549115b905196b81ceebfad1475f01571a5c6416e6e15e10da1bdc984`. The only lockfile change promotes already-transitive pdfcraft-model to a direct dependency at the unchanged PdfCraft revision. Both license inventories were regenerated byte-identically and reviewed: 264 permissive normal/build packages, no prohibited crates. NOTICE records the direct model use; all four upstream revisions remain unchanged.

## 2026-10-20 style relationships and document defaults (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 143 Rust tests, warnings-denied Clippy, locked release build and 157 installed-wheel Python tests passed with zero skips and verified artifact identity.
- Existing Style handles expose next/linked relationships with atomic reciprocal updates, valid next-style cycles, name/ID lookup, occupied-target rejection, copy independence and explicit closed-owner errors. Native XML/readback preserves both ends. Undefined native built-in heading companions are reported explicitly; clearing/replacing safely repairs the link.
- Existing Styles exposes direct document defaults and shared character/paragraph patches. Native inherited values, named/direct overrides, false versus clear, visible red/blue PNG/PDF output, exactly-once extracted content, cache invalidation and independent copies pass. Invalid patches, effective tab overflow and nested-table geometry changes leave documents unchanged.
- Every supported default clear is compared through native effective character/paragraph snapshots before and after DOCX serialization. A separate upstream regression proves omitted font size changes from raw 11pt to reopened 10pt. Default font-size reset is rejected atomically; explicit sizes and named-style reset remain supported. Automatic Enter behavior and arbitrary default-kind selection remain upstream work. STY2 stays open for the documented reset inconsistency.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,210,759 bytes; native extension: 39,866,488 bytes. SHA-256: `b23c4ecd7a85856ebd667f9430887080114db80e9784ebdbcc9fbeb2b813f636`. Lockfile SHA-256: `74b1b0b0d3986082768c09a4f4642cd10c1a36ce1bf63bdff22f43438e7c0d97`. Dependencies, all four upstream revisions and the reviewed 264-package permissive license graph are unchanged.

## 2026-10-20 region rendering and DOCX tabs/breaks (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 137 Rust tests, warnings-denied Clippy, locked release build and 153 installed-wheel Python tests passed with zero skips and exact artifact identity.
- Native clipping covers PDF/DOCX/PPTX through existing read/edit rendering methods, page intersections, fractional DPI and edge rounding, PNG resolution metadata, PDF CropBox/UserUnit/all rotations, invalid geometry/lifetimes and small regions on pages whose full rasters exceed output budgets. No full-page crop buffer or replacement renderer is used.
- Measured native raster differences are fixture-specific: PDF text at 144 DPI differs at one pixel and at 150.5 DPI at two pixels; PPTX at 150.5 DPI differs at 124 pixels on one rectangle edge row. Maximum RGB difference is 1/255 and alpha remains exact. PDF at 72 DPI, PPTX at 72/144, DOCX at all tested DPI, full-page/boundary crops and rotated PDF solid shapes compare exactly. Translated native transforms plus f32 path conversion suggest rounding; this is an inference, not a universal fidelity guarantee. Native fractional PPTX edges may retain partial alpha.
- Tabs cover native left/center/right alignment, five leaders, style/table context, inheritance/clear, writer precision and direct/effective 64-stop limits. Paragraph snapshots share native conditional resolution. Native placement tests demonstrate the pinned decimal-tab defect (segment right edge rather than separator at stop); decimal/bar authoring explicitly fails and remains upstream work. Line/page/column breaks verify native placement, consecutive blank pages, ranges, invalid contexts and reopened DOCX/PDF.
- Integration testing corrected explicit break segment assignment after columns and automatic wrapping. Source-byte starts map to upstream placed-line ranges using bounded indexing/binary searches. Independent review caught cell-owned asset pages diverging from page inventories; both now share the mapping. A source fixture with wrapping, breaks, three links and an image verifies matching pages/identities and exactly-once Markdown. Automatic wrapping inside a segment still uses first-page projection.
- LAY9 now has warmed-cache acceptance: font, margin, table-height and content changes repaginate; cached PNGs agree byte-for-byte with freshly reopened DOCX, PDF page counts/dimensions and ordered text agree, stale ranges fail, and copies/extracted values remain independent. Independent merged-PR26 audit also completes TBL3 (DOCX rich cells), TBL5 (native sizing/fitting), STY4 (ordinary style recipes) and PDF5 (supported metadata/link/outline operations), using existing corpus evidence rather than duplicate APIs. TBL6/LAY5 retain specific pagination fixture gaps.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,205,005 bytes; native extension: 39,852,504 bytes. SHA-256: `098fcdbdc26e37bed2bf8d8778a83ce3215104b116a911346e66e4ce41581066`. Lockfile SHA-256: `74b1b0b0d3986082768c09a4f4642cd10c1a36ce1bf63bdff22f43438e7c0d97`. Kurbo 0.13.1 is now a direct dependency for WordCraft's public affine transform; it was already locked/transitive. The only lock change is that dependency edge. Both license inventories were regenerated byte-identically and reviewed: 264 permissive normal/build packages, no prohibited crates. NOTICE records the direct use; upstream revisions/assets are unchanged.
- P2 and TXT4 remain open for faithful transparency/background replacement and decimal/bar-tab fidelity. Imported field inspection and other upstream/unchecked criteria remain open. This is not a release declaration.

## 2026-10-20 conditional table styles and multiline PDF links (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 128 Rust tests, warnings-denied Clippy, locked release build and 138 installed-wheel Python tests passed with zero skips and verified artifact identity.
- STY1/TBL9 evidence covers all 13 native table-style regions, same-kind inheritance, IDs/names, explicit false/clear, border suppression, band sizes, native precedence, RTL/merged cells, direct overrides, look flags separate from repeating headers, owned copies and invalid/stale/closed usage. Native DOCX reopening, colored PNG/PDF output and exactly-once table text are verified. Effective geometry checks cover base/assignment/margin changes and unregistered nested tables, with atomic rollback.
- E4 evidence covers repeated/Unicode/ligature labels, native inserted spaces/newlines, partial multiline endpoints, adjacent links and disjoint labels sharing a boundary line. Coalescing retains identical plain text and exact Markdown offsets. Overlap, skipped glyphs, ambiguous duplicate identities, inferred table cells and empty rectangles remain explicitly unresolved; work-budget failures are tested. This does not claim inferred column order or quadpoint grouping.
- Incorporated the PR25 review corrections for alignment activating indentation, section geometry shrinking percentage tables and structural parent edits squeezing nested tables. Native and Python regressions check unchanged bytes/handles after rejection. The landscape rendering fixture uses 24-point text with unchanged visible-pixel thresholds to avoid dependence on small-font antialiasing.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,185,256 bytes; native extension: 39,814,360 bytes. SHA-256: `fefbc1a8507e7e72860674393ee61a6b72c272e3397902cb3f9ac717087d5cac`. Lockfile SHA-256: `48265d3bcdda6a4ede5f8262680c7e7c324f5722eb889407c0608e0b89819f51`. All pinned revisions, dependencies and attribution assets remain unchanged; the locked license audit passed for 264 permissive normal/build packages.
- Remaining conditional paragraph-property, wider column-band, patterned-fill and shared-edge border gaps are marked upstream. Theme/next-linked styles, broader source preservation and other unchecked criteria remain open. This is not a release declaration.

## 2026-10-20 PR25 independent review correction (unreleased)

- Reviewed runtime head `13fb855d1afe9aadcca238c2d76abdbaecbde13e` against merged main `7a1559ca775c16e52927159cd1fec315bba085e8`. [CI run 39459140907](https://github.com/narain1/pydoccraft/actions/runs/39459140907) passed formatting, all 118 Rust tests, warnings-denied Clippy, the locked 264-package license audit, release build and all 131 installed-wheel Python tests with zero skips.
- Independent review reproduced three geometry-validation bypasses: alignment activating an oversized indent, section changes squeezing percentage tables with explicit margins, and fixed-parent grid splitting squeezing nested percentage tables. Each now delegates to the same native layout validation before commit; affected section tables share one layout snapshot. Native and Python regressions verify unchanged bytes/revisions/handles on rejection and successful subsequent valid edits with reopening/export.
- All 128 baseline installed-wheel Python tests passed independently. The initial CI runner found an environment-sensitive 72-DPI colored-text fixture (10 fully saturated pixels versus a requirement greater than 10). Increasing the fixture font to 24 points retains the existing visibility threshold, dimension/metadata checks and 72/144/300-DPI cases; all checks pass on the corrected CI artifact. No pixel-fidelity guarantee is inferred.
- Corrected artifact: CPython 3.11.17, Linux x86_64/glibc 2.39; `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,156,437 bytes; native extension 39,756,480 bytes; SHA-256 `f250594330a5f4f70d0d36916e94b400831e704e1fca774f4a42ec845278f1c7`. Lockfile SHA-256 remains `48265d3bcdda6a4ede5f8262680c7e7c324f5722eb889407c0608e0b89819f51`; all upstream revisions, dependencies and attribution assets unchanged. The limitations recorded below remain open.

## 2026-10-20 table formatting, outline imports and rendering acceptance (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 117 Rust tests, warnings-denied Clippy, locked release build and 128 installed-wheel Python tests passed with zero skips and exact artifact identity.
- Table evidence covers count constructors/slices in DOCX/PPTX, merged aliases/stale handles, direct patches/roundtrip snapshots, nullable automatic border colors, per-side border styles, metadata, RTL native placement, percentage/absolute preferences, native fitting and the 4,000-paragraph cutoff. Native geometry validates nested tables/images and explicit margins; invalid edits retain prior content, handles and geometry. Existing conservative nested-width acceptance remains unchanged.
- Outline import tests cover named containers, nested/duplicate/omitted targets, source semantic rejection, title/depth/count limits, existing destination forms, URI/internal links, save/render and atomic rollback when the native copier cannot retain a source destination. Omitted links remain destinationless annotations. The source/public resolver accepting deeper destination wrappers than the native copier has a regression and explicit failure.
- R2 now verifies known visible content at 72/144/300 DPI across PDF rotations/crop/UserUnit, landscape two-page DOCX, and genuinely nonstandard 413-by-257-point PPTX. Both public render methods agree, dimensions follow point scaling within documented rounding, and existing physical-resolution metadata checks remain passing. P3 now verifies URI/internal remapping and identical rendered content after native merge/subset/reorder/delete and reopening. TBL1/TBL2 count and ordered-slice criteria are complete; broader native span/style/fidelity criteria stay open.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,155,601 bytes; native extension: 39,749,656 bytes. SHA-256: `c83f7857323e39918e86d9b907b6b485467ff61c15e9a66634e618849e196bc0`. Lockfile SHA-256: `48265d3bcdda6a4ede5f8262680c7e7c324f5722eb889407c0608e0b89819f51`. All pinned revisions, attribution assets and dependencies unchanged; the locked license audit passed for 264 permissive normal/build packages.
- Full conditional table styles, patterned fills/cell spacing/shared-edge conflicts, PDF subset outlines, secure redaction and the DeckCraft fallible-render blocker remain open. Tests do not establish pixel-perfect or cross-platform fidelity; this is not a release declaration.

## 2026-10-20 merged coverage audit

An independent read-only agent reviewed source, documentation and tests through merged PR24, excluding current uncommitted table/import work. C1/API2/API4/API6/API8 are supported by the authored-model architecture and lifecycle/units/atomicity corpus in `test_authoring.py`, style tests and PDF/PPTX tests. API9 has the existing PNG/Pillow interoperability recipes. TXT7 has cross-run, explicitly scoped body/header replacement and rollback evidence in `test_template_recipes.py`, plus range/anchor protection tests. E1 has recursive DOCX nesting/order/span/exactly-once evidence in `test_structure.py`, the native PPTX rich-cell roundtrip and merged PR22 slide-table/PDF tests. These eight criteria are now checked without adding duplicate APIs. Broad API conformance/discovery/ownership, diagnostic, locator, review and preservation criteria remain open. Stale descriptions of missing authoring, PDF link labels, nested cells and rendering geometry were corrected. This audit reused recorded passing artifacts; it did not run new tests or establish additional fidelity.

## 2026-10-20 native notes and owned PDF fields (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 108 Rust tests, warnings-denied Clippy, locked release build and 119 installed-wheel Python tests passed, with zero skips and matching artifact identity.
- Notes cover Unicode ranges, stale/closed ownership, nested body-table anchors, editable independent story copies, section placement/restart/numbering, aggregate/anchor/count bounds, atomic destructive-edit rejection, separate extracted stories and DOCX/PNG/PDF output with note text once and page-bottom placement. Generic commit guards are exercised by the full existing authoring corpus.
- Forms cover typed values, invalid-edit rollback, missing glyphs/overflow, choice clearing, field/page/annotation handle invalidation, copying, page organization, all four rotations and UserUnit, strict appearance completion, removal and flattening. Unrotated combined form output is PNG-byte-identical after flatten. The rotated/UserUnit fixture measured one RGB pixel differing by 1/255; its explicit tolerance is at most four changed pixels and one channel step, with strict geometry/text checks. This is consistent with coordinate rounding, not a claim of universally identical raster output.
- Review found and fixed field deletion retargeting positional annotation handles. Empty flatten preserves handles; options are bounded before native allocation. A conversion probe demonstrated active form text loss in PDF-to-DOCX, now rejected explicitly; flatten-first conversion is reopened and verified.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 17,106,162 bytes; native extension: 39,642,184 bytes. SHA-256: `cb38f3a9471dbc0de577ef08fa98a10d4d20ba43f6bdf6453ab0c0f307c71740`. Lockfile SHA-256: `48265d3bcdda6a4ede5f8262680c7e7c324f5722eb889407c0608e0b89819f51`.
- Only `pdfcraft-forms` was added at the existing immutable PdfCraft revision. Regenerated/reviewed inventory and wheel copy contain 264 normal/build packages, all permissive and no prohibited application crates; NOTICE updated. Other upstream revisions/assets unchanged. LAY7/PDF6 and other broad criteria remain open; semantic deletion/copy, imported forms, radio groups and broader appearances are not complete. This is not a release declaration.

## 2026-10-20 PR23 independent review correction (unreleased)

- Reviewed runtime head `d8ccb4345ccadce5a6c80835446be9f78b862b77` against merged main `bb9f21b40ef500433f04ba2a0061fc71ea682b51`. [CI run 39454556564](https://github.com/narain1/pydoccraft/actions/runs/39454556564) passed formatting, all 98 Rust tests, warnings-denied Clippy, the locked 263-package license audit, release build and 113 installed-wheel Python tests with zero skips.
- Review corrected DOCX grid edits and column resizing that could leave plain-text cells narrower than their explicit horizontal margins. Delete/split/resize now reject that state before committing or invalidating handles. Native and Python regressions verify preserved geometry, generations, descendant access and successful edits after reducing margins. Independent PDF probes also verified new drawing placement after crop edits at every quarter-turn rotation.
- CI platform: Linux x86_64, Rust 1.95.0 and Python 3.11.17. Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: 16,828,225 bytes; uncompressed extension: 39,002,432 bytes. Wheel SHA-256: `75178287de004f21aa0648f7056412bbd80f421ae291f26754ade71eba82cd59`. The harness verified installed package identity against that wheel.
- Lockfile SHA-256 remains `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`; upstream revisions, dependencies and attribution assets are unchanged. The correction does not complete remaining roadmap criteria or broaden format fidelity claims.

## 2026-10-20 DOCX grid and PDF geometry (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1: formatting, 97 Rust tests, warnings-denied Clippy, locked release build and 112 installed-wheel Python tests passed, with zero skips and matching installed artifact identity.
- DOCX tests cover native row/column CRUD, rich content, nested generations, placeholders, header prefixes, span validation, width/budget rollback, stale descendants and reopened DOCX/PNG/PDF output. PDF tests cover four rotations, nonzero origins, UserUnit, inherited boxes, reset, sibling preservation, failed-edit rollback, background invalidation and searchable colored built-in/supplied-font text. Content-truncation mapping is tested directly; no oversized render fixture is claimed.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 16,828,282 bytes; extension: 38,999,256 bytes. SHA-256: `874550581aeae32e6279508ce0f00cd6fdce314749aba48592b01fd126233cca`. Lockfile SHA-256: `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`. All pinned revisions, the 263-package audited graph and assets are unchanged.
- PDF1 is completed within documented geometry semantics. Ordered table column views, rich semantic row copying, unsafe vertical-merge repairs and remaining unchecked criteria stay open. This is not a release declaration.

## 2026-10-20 sections, slide tables and template recipes (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1. Formatting, 86 Rust tests, warnings-denied Clippy, locked release build and 107 installed-wheel Python tests passed; zero skips and exact installed artifact identity.
- Native and Python tests verify per-section PNG/PDF dimensions/order, parity and blank sections, independent header/footer copies and absent-story isolation, section-owned width validation and atomic invalid geometry. PPTX coverage verifies native merge/split/grid edits, anchor migration, covered aliases, stale handles, stored box synchronization, minimum-row expansion, explicit border clearing, scalar/aggregate-cell limits and reopened/rendered/exported output. Cell margin ordering, direct alignment/reset, mixed values and nested-width protection are tested in DOCX.
- The executable template/batch recipe verifies cross-run body/header replacement, preserved unrelated links, missing/unused/invalid-value rejection, explicit output saves and unchanged original template across independent DOCX/PDF outputs. Both reviewed PR21 link corrections are included and regression-tested.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 16,789,517 bytes; native extension: 38,903,512 bytes. SHA-256: `efbcc3186ce956b1d2558abbd6416c506905ac42493ac831ea1a11cae8b162da`. Lockfile SHA-256: `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`. All immutable upstream revisions, 263-package audited graph and attribution assets remain unchanged.
- Next-column and mixed-size continuous DOCX transitions, linked stories, unequal columns, arbitrary source-template preservation, DOCX grid CRUD and inherited/mixed PPTX border resolution remain explicitly unavailable. Minimum PPTX rows may render beyond stored box height. Remaining unchecked roadmap items are not completed by this batch; this is not a release declaration.


## 2026-10-20 PR21 independent review corrections (unreleased)

- Reviewed runtime head `00c7f237aed55ca51b7346ad7858625778208bbf` against merged main `76881093b7966b45389e9c4d811ecdb9c185aa47`. [CI run 39450961572](https://github.com/narain1/pydoccraft/actions/runs/39450961572) passed formatting, 76 Rust tests, warnings-denied Clippy, the locked 263-package license audit, the release build and 99 installed-wheel Python tests with zero skips.
- Independent review corrected two PPTX link defects: explicit range assignment could not replace/clear mixed links, and invalid XML characters in URLs were silently removed on save. Reads still reject mixed targets; assignment preserves a uniform tooltip and clears mixed tooltips. Existing native text validation now rejects U+FFFE/U+FFFF before mutation. Regressions verify stale/invalid ranges, failed-edit atomicity and saved link targets.
- CI platform: Linux x86_64, Rust 1.95.0 and Python 3.11.17. Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: 16,704,307 bytes; uncompressed extension: 38,704,080 bytes. Wheel SHA-256: `88067a0e3af05a1cdfebef285ce53a183c423c78aaeff81014aeb379a007cda8`. The installed-wheel harness verified artifact identity.
- Lockfile SHA-256 remains `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`; all four pinned upstream revisions, dependencies and attribution assets are unchanged. Broader roadmap and format limitations remain open.

## 2026-10-20 styles, slide ranges and portable Markdown (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1. Formatting, 75 Rust tests, warnings-denied Clippy and the locked release build passed. Installed-wheel validation passed 97 Python tests with zero skips and matching installed package members.
- New evidence covers style graph/name validation, native inheritance/reset/false semantics, direct/resolved snapshots, omitted run-property inheritance, table-style spacing, every supported DOCX underline/highlight enum, supplied formatting persistence and DOCX PNG/PDF output. PPTX tests cover Unicode/run-boundary and paragraph replacement, exact URL labels, tooltip persistence, native internal PDF destinations after reorder, referenced-target deletion rejection, copied owners, failed-edit rollback and property/fragmentation limits.
- Markdown export tests cover actual PDF/DOCX/PPTX extraction, separate story files, nested HTML cells, content deduplication with repeated placements, safe names/escaped image attributes, cross-page inventory suppression, closed lifetimes, missing media, cyclic/deep/ragged tables and byte/node budgets. Existing extraction/conversion/render suites remain passing.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 16,703,209 bytes. Native extension: 38,700,424 bytes. Wheel SHA-256: `e6380ff66348ce6616b6001177cd80cde6292ff4346e87cb27953646c7598331`. Lockfile SHA-256: `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`. All four immutable upstream revisions and the reviewed 263-package license graph remain unchanged; no new code or attribution assets were added.
- Table/theme/next-linked styles, general Office source editing, rich PPTX table-cell ranges, full internal-link extraction and broader unchecked roadmap criteria remain open. PPTX run shading is explicitly unsupported; highlighting is distinct. Markdown is a bounded extraction package, not layout reproduction or an HTML sanitizer. This is not a release declaration.


## 2026-10-20 text/vector content and diagnostics (unreleased)

- Validated offline on Linux x86_64 with Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1; all four upstream revisions remain unchanged.
- Formatting, 63 Rust tests, warnings-denied Clippy and the locked release build passed. Installed-wheel validation passed 85 Python tests with zero skips and verified imported package members against the artifact.
- Coverage includes DOCX lists/links, explicit initial/consecutive blank pages, native report DOCX/PDF order and single table text, supplied-font searchable Unicode and visible glyphs, vector pixels/invalid-edit rollback, metadata/bookmark persistence, nonmutating absent-story lookup, stable originating child collections, bounded inputs/JSON and structured extraction diagnostics. Existing PDF/DOCX/PPTX extraction/render/conversion checks remain passing.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 16,600,451 bytes. Native extension: 38,425,016 bytes. Wheel SHA-256: `390dbe8015a307f7512d94741070c50592fbe3ae43aed37acb5aaecc21cb8a84`. Lockfile SHA-256: `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`.
- `pdfcraft-content` and `pdfcraft-fonts` become direct dependencies at the same pinned revision; both were already transitive. The regenerated/reviewed normal/build inventory remains 263 packages, with no prohibited application crates or non-permissive-only license. NOTICE and the adjacent complete OFL license cover the unmodified Inter regression fixture from pinned PdfCraft; the runtime asset set is unchanged.
- Diagnostics cover detectable native model constructs, not unknown markup discarded by upstream readers. Full conversion diagnostics, arbitrary Office source editing, complex-script conformance, advanced styles/sections/fields/review, attachment resource controls and unchecked roadmap criteria remain open. This is feature validation, not a release declaration.


## 2026-10-20 PR19 independent review corrections (unreleased)

- Reviewed runtime commit `c00e2dee4e1659668ebfb0a2939370020de083d3` against merged main `2340edd867f40cf43b33a5017c190ca01b1d769d`. [CI run 39447521531](https://github.com/narain1/pydoccraft/actions/runs/39447521531) passed formatting, all 45 Rust tests, warnings-denied Clippy, the locked license audit (263 normal/build packages), release wheel build and all 71 installed-wheel Python tests with no skips.
- Review found and corrected dropped reference-only paragraph anchors, lost leading/consecutive blank pages from explicit page breaks, and repeated paragraph-prefix scans while normalizing references, links and formatting. Regressions cover note-only/text-box-only anchors, native DOCX and PDF blank-page positions, and long Unicode prefixes with many objects, links and formatting runs. The normalization changes use public native ordered objects/run ranges and retain exact character offsets.
- CI platform: Linux x86_64, Python 3.11.17, Rust 1.95.0; wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`. Wheel size: 16,420,760 bytes; uncompressed native extension: 38,038,256 bytes. Wheel SHA-256: `56ffc0203f45b3d4985d8481b02d1c5e776331084ddb8de0f887e6dc709ba8a4`. Installed package identity was checked against that artifact.
- Lockfile SHA-256 remains `1b2bcaa846d1ffe42cc844fce299750cf9c288e1aab00a3e8aafc40b7a14b3c9`; all four pinned upstream revisions and the runtime dependency graph are unchanged. These corrections do not complete other open roadmap criteria or establish cross-platform visual fidelity.

## 2026-10-20 DOCX layout controls and auxiliary stories (unreleased)

- Same offline Rust 1.95.0/Linux x86_64/Python 3.11.2 environment and unchanged locked upstream revisions as the authoring baseline.
- Formatting, 41 full Rust tests and warnings-denied Clippy passed. Clean installed-wheel validation passed 68 Python tests with no skips, including the separately reviewed PPTX CR/CRLF regression.
- New coverage verifies DOCX rich auxiliary header/footer/note/comment stories, source reference connections, owned values after close, nested-story grid/depth limits, and page-color pixels in both native PNG and PDF-rendered output. Table column widths/borders/row properties round-trip; explicit breaks yield three correctly ordered PDF pages.
- Wheel: 16,419,239 bytes; uncompressed native extension: 38,033,224 bytes. Wheel SHA-256: `40664a866303fab98964c30c095a654d065924132507031853fbf75356406e34`. Lockfile SHA-256 remains `1b2bcaa846d1ffe42cc844fce299750cf9c288e1aab00a3e8aafc40b7a14b3c9`.
- Story order is native part-ID order, not an invented original archive ordering. Auxiliary objects have no guessed page assignment. Legacy comments remain in page output for compatibility and are flagged when also exposed as stories. General Office source editing, multiple-section mutation, advanced fills and other unchecked roadmap criteria remain open.


## 2026-10-20 native authoring baseline (unreleased)

- Built offline in the Rust 1.95.0 Debian container on Linux x86_64; Python 3.11.2, Maturin 1.8.1. All builds used `--locked`; upstream revisions remain unchanged from AGENTS.md.
- `cargo fmt --all -- --check`, `cargo test --locked` (37 tests), and `cargo clippy --locked --all-targets -- -D warnings` passed.
- Clean installed-wheel validation passed all 61 Python tests with no skips. Package files were verified against the wheel; DOCX/PDF/PPTX authored outputs and conversions were reopened. Tests cover failed-edit atomicity, native ownership/copies, stale handles, rich/nested cells, header/footer PDF text, page/slide organization, shape/background/media retention, PDF annotation CRUD, exact PDF link labels and rotated/cropped/UserUnit raster geometry.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`; 16,382,442 bytes. Native extension: 37,941,224 bytes. Wheel SHA-256: `9e96852313dd58f2aeafba2f1da412b0c9f787f0bb55fa8a87c7c70e89487bdc`. Lockfile SHA-256: `1b2bcaa846d1ffe42cc844fce299750cf9c288e1aab00a3e8aafc40b7a14b3c9`.
- Reviewed normal/build graph: 263 packages; only `pdfcraft-organize` is newly added to the graph. `pdfcraft-annot` was already transitive and is now direct. Both use the existing immutable PdfCraft revision. Regenerated/reviewed both license inventories and NOTICE; no prohibited application crate or non-permissive-only license was found. CI now checks the generated inventory.
- The initial PPTX invalid-character test was corrected after source audit: U+FFFC is valid literal DeckCraft text, unlike WordCraft's object placeholder. A native save/reopen test verifies it; NUL/U+FFFE/U+FFFF remain rejected. Annotation geometry assertions allow upstream f32 rounding within 0.0001 point, with failed resize preserving geometry.
- Supported scope and explicit limits are in `docs/authoring.md`. Existing source DOCX/PPTX edits are refused; source PDF edits refuse encrypted/form/signed/certified profiles. PDF text remains Helvetica/WinAnsi; Office image authoring currently accepts PNG. Annotation creation supports notes/highlights/links and does not implement redaction. Header/footer authoring/export is verified, but DOCX story extraction remains separate work. Advanced roadmap coverage is not marked complete by this baseline.


## 2026-10-20 PR preparation against updated main

Rebased the completed roadmap foundation and feature commits onto `f0baff5` (community guidelines and package URL metadata). Repeated formatting, locked Rust tests (5 passed), Clippy, the locked release build, and all 36 Python tests in a new clean CPython 3.11 environment. Container networking remained disconnected. The runtime dependency graph and upstream revisions are unchanged.

The resulting Linux x86_64 wheel is 14,411,717 bytes, with a 33,068,744-byte native extension. SHA-256: `d216dea47b1882808df2984cc8e0a8cd7f2f7ad317bac487a3e4179b69385cd8`. The installed-wheel identity check passed. This supersedes the pre-rebase artifact below for PR review; the broader authoring workstreams remain open.

## 2026-10-20 first agent implementation batch (unreleased)

Three agents covered the W01/W03/W04/W07/W08, W02/W06/W10/W11, and W09/W05/W12/W13 portfolios; the integrator owned W14. Implementation in this working tree is limited to rendering metadata/bounds, recursive DOCX extraction, roadmap validation and the installed-wheel harness. Dependent authoring portfolios have pinned-source readiness audits, not completed runtime features. No new `new`, `edit`, or save API is advertised.

Validation used Rust 1.95.0 in `rust:1.95-bookworm`, CPython 3.11.2, Maturin 1.8.1 and Linux x86_64. The committed `Cargo.lock` and all four upstream revisions remain unchanged. The final installed-wheel corpus ran with the container disconnected from its Docker network.

| Check | Result |
|---|---|
| Formatting, `cargo test --locked`, Clippy with warnings denied | Passed; 5 Rust tests |
| Release wheel build with committed lockfile | Passed |
| Clean installed-wheel suite | 36 passed, zero skipped: 28 runtime/API tests and 8 roadmap-validator tests |
| DPI metadata and dimensions | PDF/DOCX/PPTX at 72, 144, 150.5 and 300 DPI; decoded PNG data and representative visible output checked |
| Render errors/lifetimes | Invalid DPI, oversize requests, indexes and closed documents checked; intentional blank DOCX page remains valid |
| Recursive tables | Nested content, source order, exact links, images, merged cells including omitted leading grid cells, cross-page fallback deduplication, owned lifetime and 16/17-level boundary checked |
| Conversion reopen | Existing three advertised pairs plus nested DOCX-to-PDF no-duplication assertion passed |
| Artifact identity | Harness compares installed package/native members with wheel; deliberately mismatched wheel rejected before tests/evidence output |
| Coverage ledger | 139 IDs, 14 workstreams, unique owners, valid roles/upstream references, acyclic dependencies |
| Attribution | Wheel LICENSE/NOTICE/LICENSE-THIRD-PARTY match source; third-party wheel copy matches reviewed inventory |
| Repository checks | Workflow YAML parses; `git diff --check` passes |

Artifact: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 14,411,386 bytes; native extension 33,068,744 bytes. Wheel SHA-256: `22d11eb7e734f41339b576fe2445dfa17db77228cca4348bd5d4d6814028e5d5`. Lockfile SHA-256: `36084d8fdc39107d12a67bcb28e8454d919e8ee888828992e90a75c2ea81e13d`. Machine-readable evidence is generated at `dist/validation.json` and remains a build artifact. The previous size baseline was AArch64, so these sizes are not a same-platform regression comparison.

Review corrected a PyO3 module-name collision, cross-page table asset duplication and omitted-grid merge positioning. WordCraft adds a required empty paragraph after a nested table; the structural regression deliberately retains and asserts that normalization instead of dropping empty paragraphs.

R3 is complete for this tested working-tree artifact. R2/R4 remain open for rotated/landscape/nonstandard geometry coverage and the upstream DeckCraft silent blank-on-panic contract. E1 remains partial for broader PPTX/PDF semantics and pagination coverage. The 16-level table guard applies to DOCX body extraction normalization, not all parser/stories/operations. No dependency upgrade, new runtime library or upstream issue was introduced. Remote CI/release matrices, cross-platform visual baselines and broad authoring acceptance remain unverified; this is not a release declaration.

GitHub progress was read back after updating all 14 issues: each remains open, 139 coverage IDs remain unique, and only R1/R3 are checked. Project W01/W02/W09/W14 are In Progress; dependent authoring work remains Todo with source-audit findings attached. Issue notes explicitly identify these changes as local, uncommitted and unreleased. Local documentation checks covered 73 file links and parsed the readiness-document Python snippets.

## 2026-10-20 GitHub roadmap issue publication

- Published and read back 14 canonical roadmap issues in `narain1/pydoccraft`, numbers 3–16. The [workstream map](../../../docs/roadmap-workstreams.md) links each issue; its JSON records the issue URLs.
- Verified issue bodies against the generated scope/checklists: all 139 coverage IDs are owned exactly once, preserving 138 open goals and existing rendering capability R1. Added priorities, backend labels, qualified upstream-candidate labels, related-work links and prerequisite baselines.
- Used native GitHub blocking relationships only for whole-issue API-contract prerequisites. Existing rendering and extraction work remain independent; partial prerequisites are explicit body links rather than broad blockers.
- Local checks passed: 61 local links/anchors across 14 Markdown files, draft Python example syntax, exhaustive unique coverage, acyclic workstream dependencies and whitespace checks. No runtime tests or native builds were rerun for these planning changes.
- After user authorization, created [pydoccraft Roadmap, project 8](https://github.com/users/narain1/projects/8) and linked it to the repository. The project retains GitHub's private visibility default.
- Added all 14 issue workstreams exactly once and read back every item's status, priority, work kind, ownership and upstream-candidate fields. Verified the repository link, project README, Backlog table, Roadmap timeline and filtered Upstream work view. Start/target dates remain unset; no delivery commitments were invented.

## 2026-10-19 overlap, API surface and upstream source review

- Three explicitly requested agents reviewed duplicated scope, public API breadth and dependencies, then inspected the pinned WordCraft/PdfCraft/DeckCraft repositories through GitHub APIs.
- Consolidated all 139 coverage IDs into 14 canonical workstreams, each ID with one owner. Higher-level workflows, wrappers and data conveniences were reduced to recipes or acceptance checks; existing API contracts remain unchanged.
- Recorded source evidence and ownership in [upstream-roadmap.md](../../../docs/upstream-roadmap.md). Writers and many model/edit/background operations already exist. Narrow verified candidates include PdfCraft's private table detector, DeckCraft's sized blank-image fallback on caught render panics, and WordCraft formatting-revision algorithms located in engine code.
- This is source inspection and planning, not new runtime evidence. No native behavior/dependencies/upstream revisions changed and no issue was filed in an upstream repository.

## 2026-10-19 Open XML feature-parity planning (documentation only)

- Reviewed Microsoft's `dotnet/Open-XML-SDK` repository README, `WordprocessingDocument.cs`, WordprocessingML schema inventory, and official feature references for text/styles, tables/cells, backgrounds, sections, fields, content controls, revisions and validation. Reviewed Pillow's Image API for ergonomic conventions. Source links and review scope are recorded in [authoring-api-design.md](../../../docs/authoring-api-design.md).
- Expanded the feature roadmap with grouped acceptance TODOs and milestones for a Pythonic authoring/editing API. Added draft creation, table/background, edit-copy, PDF and slide workflows; all are explicitly marked unimplemented.
- Recorded the sole-backend policy: WordCraft/PdfCraft/DeckCraft public library APIs. Open XML SDK is a feature reference, not a runtime or required validation dependency. No upstream feature availability was inferred from Open XML support.
- Added explicit per-operation coverage requirements for read/create/edit/preserve/render/export and blockers for missing upstream APIs. Rendering existing files remains implemented; new authoring capabilities have not been implemented or runtime-validated.
- Updated requirements to distinguish permitted programmatic library editing from prohibited desktop editor-state dependencies. No source code, dependency, lockfile, license, attribution asset or upstream revision changed in this planning extension.
- Documentation validation passed: 38 local links/anchors, balanced code fences across nine Markdown files, syntax checks for six Python examples, 139 unique roadmap IDs (138 open, one implemented), and whitespace checks. Draft authoring examples were not executed; no native builds or runtime tests were rerun for these documentation changes.

## 2026-10-19 feature roadmap review (documentation only)

- Reviewed the Python facade, native dispatch and all three rendering adapters, conversion dispatch, existing tests, and format documentation.
- Confirmed source implementations of `Document.render_page(index, dpi=144)` and `Page.get_pixmap(dpi=144)` for PDF, DOCX, and PPTX, including finite 1–1200 DPI validation and `dpi / 72` scaling.
- Existing tests assert PNG signatures, default/72 DPI behavior, negative indexes, and zero-DPI rejection. They do not establish multi-DPI dimensions, visual fidelity, or consistent PNG physical-resolution metadata; these remain roadmap R2–R4.
- Confirmed there is no public authoring/mutation/save API. Added DOCX/PDF creation TODOs C1–C5 and prioritized extraction/performance work in [TODO.md](../../../TODO.md).
- Compared capabilities with linked official PyMuPDF documentation. PDF rendering/creation already exist there; Office rendering is also documented. The roadmap distinguishes parity work from candidate differentiation and makes no market-wide uniqueness claim.
- This change adds documentation only. No native build or runtime tests were rerun; historical installed-wheel results below remain separate evidence. No dependencies, lockfile, licenses, or upstream revisions changed.
- Documentation checks passed: 22 local links (including referenced headings), balanced code fences across seven Markdown files, 19 unique roadmap IDs (one implemented and 18 open), and `git diff --check`.

## 2026-10-19 repository and CI initialization

This is pre-release validation for the initial GitHub publication, not a declaration that task 10 or every release-candidate platform check is complete.

### Environment

- Host: macOS on Apple Silicon
- Validation container: Linux AArch64, Rust 1.95, CPython 3.13
- Build backend: Maturin 1.8.1
- Locked upstream revisions: PdfCraft `645016addcfb9078c557f0189559ab47cdcee057`, WordCraft `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`, DeckCraft `22db9261f9d1a214c296fd390173130c45d76d66`, craft-libs `c3487320b2d1de657b860b3c8abb44e1f2b32bd8`

### Results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo test --locked` | Passed: 3 tests |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| Maturin release wheel build with `--locked` | Passed |
| Tests against the installed wheel | Passed: 13 tests |
| Conversion outputs reopened by tests | Passed for PDF→DOCX, DOCX→PDF, and PPTX→PDF |
| PDF, DOCX, and PPTX extraction and PNG rendering | Passed in the installed-wheel suite |
| GitHub Actions workflow lint | Passed with `actionlint` |
| Prohibited package scan | No PdfCraft engine/JavaScript, WinPrint, UI, MCP, automation, or OCR package linked |
| Locked license inventory | 262 normal/build packages; no undeclared or GPL-/AGPL-/LGPL-only license |

The Linux AArch64 CPython 3.13 wheel measured 13,192,166 bytes. The uncompressed native extension measured 28,648,840 bytes.

### Dependency and conversion correction

The pre-existing manifest referenced the PdfCraft application engine while the lockfile referenced the lower-level export crate. The manifest now pins `pdfcraft-export` at the existing PdfCraft revision, and PDF→DOCX maps public PdfCraft extraction values into `pdfcraft_export::Page` before calling the upstream DOCX writer. This removes the application engine and its prohibited JavaScript/printing graph. `Cargo.lock` and `THIRD_PARTY_LICENSES.md` were regenerated and reviewed.

### Remaining release checks

- GitHub CI must confirm the clean Python 3.11 installed-wheel path after the initial push.
- Tagged release jobs must confirm Python 3.11–3.13 wheels on GitHub-hosted Linux, macOS, and Windows runners.
- PyPI trusted publishing must be configured for the `pydoccraft` project from `narain1/pydoccraft` and the `pypi` GitHub environment before the first release tag.
- Known upstream fidelity limitations remain those documented in `README.md` and `design.md`; conversion is not claimed to be pixel-perfect.

## 2026-10-20 pydoccraft 0.2.1 packaging validation

- Renamed the PyPI distribution to `pydoccraft`; the compatible Python import remains `doc_ex`.
- Package, crate, lockfile, and Python versions agree on `0.2.1`.
- Replaced invalid Git patches to PdfCraft's excluded `vendor/` directory with exact vendored Apache/MIT snapshots of Hayro and lopdf from PdfCraft revision `645016addcfb9078c557f0189559ab47cdcee057`.
- Built `pydoccraft-0.2.1-cp311-cp311-manylinux_2_17_aarch64.manylinux2014_aarch64.whl` with Rust 1.95 and Maturin 1.8.1.
- Installed the wheel in a clean Python 3.11 container; `importlib.metadata.version("pydoccraft")` and `doc_ex.__version__` both returned `0.2.1`.
- Installed-wheel test suite passed: 13 tests.
- Wheel size: 13,198,306 bytes.
- Wheel metadata reports Apache-2.0 and includes `LICENSE`, `NOTICE`, and `LICENSE-THIRD-PARTY`; the latter matches `THIRD_PARTY_LICENSES.md` byte-for-byte.
- `pydoccraft` was not yet published when checked. First publication requires a matching pending PyPI Trusted Publisher for repository `narain1/pydoccraft`, workflow `release.yml`, and environment `pypi`.

## 2026-10-20 final v0.2.1 release validation

- Repository renamed to `narain1/pydoccraft`; local origin, `main`, GitHub environment, existing v0.2.0 tag, and release were verified after the rename.
- Rust 1.95 clean-target validation passed: formatting, 3 Rust tests, and Clippy with warnings denied.
- Final CPython 3.11 Linux AArch64 wheel built with the committed lockfile and installed in a clean container.
- Installed-wheel Python suite passed: 13 tests covering extraction, rendering, lifecycle, safety, and reopenable PDF→DOCX, DOCX→PDF, and PPTX→PDF conversions.
- Final wheel size: 13,198,994 bytes; native extension remains 28,648,840 bytes uncompressed.
- Wheel metadata reports `pydoccraft` 0.2.1 under Apache-2.0.
- Wheel includes `LICENSE`, the reviewed `NOTICE`, and `LICENSE-THIRD-PARTY`; the latter matches `THIRD_PARTY_LICENSES.md` byte-for-byte.
- Final lockfile contains 246 unique package names and no prohibited engine, UI, CLI, MCP, automation, OCR, JavaScript, print, signing, or WinPrint package.
- Documentation links, workflow YAML, and `git diff --check` passed.
- PyPI trusted publishing was reported configured for project `pydoccraft`, owner `narain1`, repository `pydoccraft`, workflow `release.yml`, and environment `pypi`.

## 2026-10-20 published-wheel performance baseline

- Benchmarked the published pydoccraft 0.2.1 macOS ARM64 wheel on Apple M5 Pro, macOS 26.6.2, and CPython 3.13.12.
- Used independently generated 25-page PDF/DOCX, 25-slide PPTX, and 20,000-cell XLSX inputs; generation and correctness checks were outside timed regions.
- After two warmups and seven measured iterations, median PDF plain-text extraction was 78.42 ms for pydoccraft and 24.10 ms for PyMuPDF 2.0; PyMuPDF was 3.25× faster.
- Median open plus first-page 144-DPI RGBA PNG rendering was 96.90 ms for pydoccraft and 40.22 ms for PyMuPDF; PyMuPDF was 2.41× faster. pydoccraft eagerly extracts all pages before rendering and the native render path reopens source bytes.
- pydoccraft median capability baselines were 20.16 ms for DOCX extraction, 5.07 ms for PPTX extraction, 9.84 ms for PDF→DOCX, 22.67 ms for DOCX→PDF, and 154.19 ms for PPTX→PDF.
- openpyxl 3.1.5 was measured only as a separate XLSX baseline because it has no equivalent PDF/DOCX/PPTX parsing, rendering, or conversion operation.
- Conversion outputs reopened and contained text; render outputs had valid PNG signatures. See `docs/benchmarks.md` for methodology, environment, hashes, limitations, and rough-edge tracking links.

## 2026-10-20 anonymized local-file performance baseline

- Benchmarked five local files without publishing filenames, paths, hashes, or extracted text: three PDFs, one DOCX, and one PPTX.
- Used one warmup and five measured iterations on Apple M5 Pro / macOS 26.6.2 / CPython 3.13.12; generated outputs were validated after timing and not retained.
- Across PDFs of 14, 65, and 391 pages, PyMuPDF text extraction was 4.16×, 4.57×, and 3.15× faster respectively.
- First-page 144-DPI RGBA rendering favored PyMuPDF by 1.93×, 5.41×, and 59.83×. The 391-page result confirms that pydoccraft first-page latency is dominated by eager whole-document extraction plus render reopening.
- pydoccraft PDF→DOCX conversion medians ranged from 69.38 ms to 759.18 ms. The 2-page DOCX extracted in 5.18 ms, rendered in 16.49 ms, and converted to PDF in 6.65 ms. The 14-slide PPTX extracted in 41.93 ms, rendered in 72.73 ms, and converted to PDF in 168.11 ms.
- Full anonymized numeric results and methodology are in `docs/benchmarks.md`; the reusable local runner is `benchmarks/benchmark_test_files.py`.

### PR27 independent benchmark review

- Ran the published-wheel benchmark in a fresh Linux x86_64/CPython 3.13.13 environment with the complete pinned comparator lock; `pip check` passed. A small smoke run and the full default 25-page/25-slide/20,000-cell corpus both completed all 12 scenarios. The full run used two warmups and seven measured iterations; conversions reopened with text and both renderers returned PNG. This independently validates the harness, not the historical macOS timings.
- Corrected post-save OOXML core timestamps: openpyxl overwrites `modified` during serialization, so ZIP timestamp normalization alone did not yield stable XLSX bytes. A regression injects different writer clocks and verifies identical normalized bytes and exact reopened cell values. Historical recorded hashes remain labeled as pre-correction evidence.
- Rejected `.md` JSON output paths before generation to prevent the companion Markdown report from overwriting JSON. Both focused benchmark-only regression tests passed; this tooling remains outside runtime dependencies and the normal installed-wheel suite.

### PR28 independent local-corpus benchmark review

- Reviewed owner-authored tooling against merged main without reading or uploading private corpus files. In the previously installed pinned Linux/CPython 3.13.13 benchmark environment, all five focused harness regressions passed. A generated four-format local corpus completed 13 scenarios (one warmup/two measured iterations); a synthetic smoke completed 12 scenarios and recorded the installed native artifact identity. Input filename/path canaries were absent from JSON, Markdown and stdout reports.
- Corrected the new local harness's JSON/Markdown path collision and unchecked negative/zero CLI counts. Repeatability now compares semantic counts/dimensions, excluding serialization byte lengths that can vary with metadata. Python exception details are suppressed at the local-corpus CLI boundary to avoid exposing document paths/content; backend diagnostics are explicitly outside this report guarantee. Historical private-corpus timing claims were not independently reproduced.

### Anonymous PDF stage profile

- Five-iteration medians isolated detection, native extraction, JSON decode, Python hydration, direct native render, and public open+render on the three local PDFs.
- Detection was below 0.01 ms. Native eager extraction consumed approximately 98%, 96%, and 73% of public open latency for the 391-, 65-, and 14-page inputs.
- Public open medians were 1,307.87 ms, 202.44 ms, and 104.78 ms; direct native first-page render medians were 70.63 ms, 26.67 ms, and 70.30 ms. Public open+render was approximately their sum because rendering reopens the source independently.
- Direct native rendering was 1.61× and 1.28× faster than PyMuPDF's public open+render path on the 65- and 14-page PDFs, but PyMuPDF was 3.24× faster on the 391-page PDF. The major public-path regression is therefore eager extraction plus duplicate open state, not uniformly slow rasterization.
- JSON decode plus Python hydration added 11.18–28.71 ms. Media-heavy inputs can add separate base64/ownership costs.
- The prioritized feature improvement remains W02/P1: a bounded private native parse/render session, followed by bounded selected-page/text-only extraction under W09/P4.

## Selective text extraction feature branch

- Added `doc_ex.extract_text(source, filetype=None, password=None, pages=None)` with form-feed page separation, ordered/duplicate selections, negative indexes, stable `IndexError`, 10,000 selected-page limit, and 64 MiB output limit.
- PDF uses a single PdfCraft text renderer and skips COS table/image traversal, media transfer, bounded JSON serialization, and Python public-model hydration. DOCX/PPTX retain eager compatibility fallback behavior.
- Independent Rust 1.95 formatting/tests/Clippy and release-wheel validation passed: 128 Rust tests and 139 installed-wheel Python tests.
- Anonymous Linux ARM64 medians versus eager `open()` were: all-page text 1.95×, 2.25×, and 4.95× faster; first-page text 19.91×, 42.32×, and 41.93× faster across the 391-, 65-, and 14-page PDFs.
- This PR is independent of PDF render-session reuse. Selective structured extraction, Office fast paths, streaming/chunked output, and peak-memory evidence remain open.

## PDF parser-session reuse feature branch

- Added a private native PDF session that retains the PdfCraft `PageRenderer` used during eager extraction and routes subsequent `Document.render_page` / `Page.get_pixmap` calls through the same parsed document.
- Public `doc_ex.open`, eager owned values, PNG bytes, negative indexes, errors, conversion/edit source bytes, and idempotent close remain compatible. `close()` releases the private session while previously extracted values remain usable.
- Rust 1.95 validation passed: 128 tests and Clippy with warnings denied. The installed Linux ARM64 CPython 3.13 wheel passed 139 Python tests, including parser-build-count, repeated render, close, DPI, and output regressions.
- Anonymous repeated page-1 render medians versus the legacy one-shot native reopen were: 6.52 vs 59.69 ms (9.16×) for 391 pages, 23.62 vs 26.00 ms (1.10×) for 65 pages, and 71.89 vs 78.52 ms (1.09×) for 14 pages. Parser-build count remained one after repeated renders.
- Scope remains partial: eager full-document extraction still dominates first-render latency; DOCX layout reuse, selected-page/text-only extraction, all-page/peak-RSS evidence, and upstream inspect/COS consolidation remain open.

### PDF session integration review

- Exact review head `29cc3df5e43ef698ae7648fe73dd428620335f5b` passed CI 39465695584: 153 Rust tests, 172 installed-wheel Python tests, formatting, warnings-denied Clippy and the unchanged 264-package locked license audit. Existing full-raster limits, oversized-page small clips, blank pages and cross-format rendering passed alongside the new session tests.
- Merging current main required retaining native clipping in the session path. A shared renderer helper now uses immutable inspected displayed dimensions and the existing pixel rounding, geometry, precision and output guards for both one-shot and retained rendering. Tests compare exact full/clipped PNG bytes at fractional DPI with CropBox/UserUnit and all rotations; parser-build count remains one.
- A concurrent author merge added explicit-close expectations while dropping its close implementation. The reviewed correction releases the parser through Mutex<Option<PageRenderer>> and immutable PyO3 borrows, including poisoned-lock cleanup. Python captures a local handle, marks the document closed and detaches its handle before native close; retained handles report stable closed errors. Thread/lifetime tests pass, but this does not claim concurrent raster execution because current extension calls retain the GIL.
- Historical private-corpus timings above remain author-reported; this integration review validates correctness and reuse counters, not those private performance measurements. No dependency or public API was added by the review corrections.


### Selective extraction review corrections

- Review identified unbounded Python/PyO3 page collection, missing Office and all-page selection/output checks, oversized-index error inconsistency, and a PDF string allocation preceding the output check. Both boundaries now consume at most 10,001 inputs; ordinary and arbitrary-size invalid indexes use IndexError; all formats cap selected pages and UTF-8 output. PDF preflights glyph byte lengths and possible separators before the upstream string builder.
- Focused regressions cover generator lookahead, direct native conversion, large signed indexes, duplicate non-ASCII Office output, all-page fallback caps, empty-page separators, UTF-8 chunk boundaries, native page-count limits and glyph-budget checks. Upstream text-device construction can still allocate before output preflight; P4 remains open for peak-memory/streaming guarantees. Reviewed runtime head `57efdc8e6872bfad09275b07d2646ccf0a90d356` passed CI 39466407602: 155 Rust tests, 178 installed-wheel Python tests (no skips), formatting, warnings-denied Clippy and the unchanged 264-package locked license audit. The later main merge changed no files.

- CI artifact: CPython 3.11.17 / Linux x86_64, wheel 17,224,825 bytes, native extension 39,902,544 bytes, wheel SHA-256 `60935ab53de271b8727d5dc5bfbd5328eb9f2d6ac94a848e4fbfe58d9e752544`. Upstream revisions and runtime dependency graph remain unchanged. Historical private-corpus timing claims remain author-reported.

### Paragraph border integration review

- Exact runtime head `bca6a47e7b39d596d37f97b5c9e48ad7ab4e0928` passed CI 39467518674: formatting, 172 Rust tests, warnings-denied Clippy, 187 installed-wheel Python tests (no skips), and 264-package license audit. Independent review also ran all 176 Python tests against the original frozen feature wheel. The later main merge added only PDF review evidence.
- CPython 3.11.17 / Linux x86_64 artifact: wheel 17,290,310 bytes; native extension 40,109,520 bytes; wheel SHA-256 `020b291f27eac331d8a5260f836d8dfe310af45ddec012050d17223e810a13fd`. No dependencies changed in the border feature; integrated dependency and upstream revisions match reviewed main.

### Picture lifecycle and ownership independent review

- Independently ran all 196 Python tests against the frozen installed package for `ef0b577ac5cf11ba45936bb2541ec1c72a6d4535`; all passed. CI 39467831926 on that exact head also passed formatting, 177 Rust tests, warnings-denied Clippy, 196 installed-wheel Python tests (no skips) and the 264-package locked license audit. Integration of the reviewed PR35/36 histories changed no runtime, tests, dependency or attribution files.
- CI artifact: CPython 3.11.17 / Linux x86_64, wheel 17,299,520 bytes, native extension 40,130,544 bytes, wheel SHA-256 `6191b9e0be1ba7a56b6fe7ab4df64cbbc87cb7b6dc65842aabe5041133d699e3`. Reviewed native reference traversal, bounded media accounting, retained paragraph anchors and generation-captured iteration; no additional findings.

### Imported markup coordinate review correction

- Independently reproduced a tiny imported Underline whose finite ±1e308 right-edge coordinates overflow native interpolation while the guarded left height and bottom length remain finite. Recolor was accepted and serialized literal `inf` into the appearance. The adapter now applies the existing ±1,000,000 PDF-user-unit page-box coordinate limit before any native markup appearance work.
- Native extreme/boundary assertions and a public imported-PDF regression cover all four markup kinds, compound recolor/text rollback and deletion without appearance regeneration. The original frozen PR39 package independently passed all 203 Python tests; corrected runtime head `98c035c8e8b5c2a49d640a43a4c96f7cbd812b47` passed CI 39469758507: formatting, 186 Rust tests, warnings-denied Clippy, 204 installed-wheel Python tests (no skips), and the unchanged 264-package license audit.
- CI artifact: CPython 3.11.17 / Linux x86_64, wheel 17,317,570 bytes, native extension 40,163,296 bytes, wheel SHA-256 `d13f133870d8e0d3ae2390ab566b68460e60ea0568476f1a1d16c8e5ebb67abc`. No dependencies or upstream revisions changed in this feature.

## 2026-10-20 section geometry and authored slide layouts

- Rust 1.95 locked/offline formatting, 194 Rust tests and all-targets warnings-denied Clippy pass. A clean CPython 3.11.2 installed wheel on Linux x86_64 passes 212 Python tests with no skips, errors or failures. Package bytes, including the native extension, match the artifact.
- WordCraft gutter/unequal-column acceptance covers native text positions, explicit column/page breaks, actual separator pixels and clearing, DOCX readback, PNG/PDF output, copy/cache ownership and nested child rollback. New unequal-section body tables use fixed native widths no wider than the narrowest column. A long-content regression proves autofit relocation can exceed a later narrower column; serialization rejects it and explicit fixed-width recovery succeeds. The safe spanning-table fixture accounts for native half-border placement and keeps rows once. LAY1/LAY4 are complete for the documented profile; top/mirrored gutters remain LAY10.
- DeckCraft blank/title/title_and_content/two_content layouts preserve native master/layout references and fresh identities. Filled placeholders remain editable and visible in PNG and searchable PDF; native resolved transforms support geometry edits without changing unrelated inherited state. ID/shape/paragraph budget, invalid-layout and stale-owner regressions pass. Additional layouts, source theme/master import and safe slide-copy remapping remain open under SLIDE1/SLIDE2.
- This integrated artifact includes the independently identified PR39 markup-coordinate correction: finite but extreme imported quad coordinates now fail before native interpolation can emit infinity. All four markup kinds preserve bytes on failed recolor/text patches; deletion remains available.
- Wheel: 17,324,430 bytes; uncompressed extension: 40,173,336 bytes; SHA-256 `173e785889b3888af2f52485c8c3b42dbffdb725f0e94e85a1f16d1d40c4d3dd`. Cargo.lock SHA-256 remains `fcfb4a174387cf25cceddadeba9e6be930ec790c840dca0bb5f169431a6c089c`. No dependency or attribution changes; the 264-package permissive graph and all four pinned upstream revisions remain unchanged.

## 2026-10-20 author-reported PyMuPDF-only post-optimization benchmark

- The PR author compared only pydoccraft and PyMuPDF 2.0 on three anonymous PDFs using Linux ARM64 CPython 3.13 release wheels. Independent review does not have these private inputs and did not reproduce the timings; the report predates stronger matching-output checks.
- Warm first-page RGBA rendering now favors pydoccraft by 2.86×, 1.84×, and 1.12× after parser-session reuse.
- Cold open+render remains 45.75×, 4.49×, and 1.82× slower because pydoccraft eagerly builds the complete structured model before rendering; this is not a rasterizer-only comparison.
- The selective all-page text path was 2.06×, 2.30×, and 1.22× slower than PyMuPDF after removing its redundant valid-document `inspect()` parse.
- The single-parse change improved pydoccraft versus its preceding merged implementation by approximately 1.10×, 1.04×, and 1.00×. Rust 1.95 tests/Clippy passed: 194 tests.
- Remaining structured-open costs are duplicate PdfCraft text interpretation for table detection and unconditional image traversal; exact semantic optimizations require focused upstream PdfCraft APIs rather than local PDF heuristics.

## 2026-10-20 combined table fixture pack

- The new single acceptance test runs six generated fixtures: invoice, merged header, nested table, long repeated-header table, RTL table and picture-containing cell. All pass against the clean installed `/opt/sectionlayouttest` artifact recorded above (194 Rust /212 prior Python baseline); the new fixture test is additional to that baseline. No runtime code or dependency changed in this slice.
- Both DOCX and PDF reopen with matching page counts, body labels exactly once and visible representative colors on every rendered page. The repeating-header fixture checks one semantic source header and one visible header on each PDF page. RTL source order and reversed visual cell placement, merged semantic spans, nested fill and source-image colors are independently asserted. Independent review subsequently ran the complete suite against the same frozen installed package: all 213 Python tests passed. Runtime and dependency files remain byte-identical to the 194-Rust-test section/layout baseline; exact current-main CI is required before merge. See `docs/table-corpus.md`.


## 2026-10-20 owned slide pictures and shared text selection

- Locked/offline Rust 1.95 formatting, all 198 Rust tests and all-targets warnings-denied Clippy pass. After the Python-only selection/re-entrancy corrections, the unchanged native build was repackaged and installed into a fresh CPython 3.11.2 environment. All 224 installed-wheel Python tests pass with no skips, errors or failures, including the table corpus. Installed package/native bytes match the final wheel.
- Native and public picture tests verify original PNG ownership, alt text, actual asymmetric crop/flip/quarter-turn pixels in native PNG and exported PDF, retained links/geometry/stacking, source kinds, copies, deleted/closed handles, warmed-cache invalidation and exact invalid-input errors. Native 64 MiB tests verify deduplicated admission, shared picture/background retention, atomic replacement rejection and budget reclamation. Caller stream/coordinate callbacks reproduce old stale-slide targeting and now retain stable target identity after reordering.
- Shared contiguous slicing returns existing paragraph/slide range types with normal clamped/negative Unicode offsets. Tests cover combining marks/emoji/whitespace selection, empty insertion/full deletion, retained surrounding styles, URL/internal links and tooltips, native placeholder/master geometry, LF-separated slide paragraphs, cell/header/note scopes, stale/closed/deleted owners, callback mutation detection and semantic-anchor rollback. Nontext shapes fail explicitly; table indexing retains its row/column meaning. DOCX cross-paragraph ranges/comments and broader SLIDE3/TXT8 criteria remain open.
- The first 221-test integration run found one test expectation missing the already-correct UnsupportedFormatError for non-PNG input. Each invalid case now asserts its exact expected exception; subsequent new callback tests also reproduced the original facade's identity bug before the fix. The final 224-test run includes both corrections and the slide-slicing additions.
- Final wheel: 17,333,954 bytes; uncompressed extension: 40,198,168 bytes; SHA-256 `4d0e36bb1fe2eed379175df38012576bd8571f202b03fc2ba12a85ff9ce53a99`. Cargo.lock SHA-256 remains `fcfb4a174387cf25cceddadeba9e6be930ec790c840dca0bb5f169431a6c089c`; the 264-package permissive graph, attribution and all pinned upstream revisions are unchanged. Source picture crops retain native 1/100000 serialization precision; encoded-media bounds are not a total-memory guarantee.


## 2026-10-20 DOCX numbering and PPTX internal navigation

- Rust 1.95 locked/offline formatting, all 204 Rust tests and warnings-denied all-targets Clippy pass. The release wheel was installed into a fresh CPython 3.11.2 environment; all 235 installed-wheel Python tests pass with no skips, errors or failures. Installed facade/native bytes match the wheel. This includes the reviewed callback identity correction from PR42, but not PR43's later PDF optimization/benchmark tests.
- DOCX fixtures cover all nine levels, continuation/restart, empty numbered paragraphs, inherited/cleared numbering, imported start overrides/templates, nested tables, initial explicit page/column breaks, wrapping, distinct body/header/note identities, missing definitions, symbol-font mapping and logical RTL labels. Native tests cover ambiguous repeated placements and string/aggregate budgets. Definition metadata describes the native parsed model: the reader truncates imported templates to 64 Unicode characters, and repeated headers with varying labels omit the label. Source text, Unicode link offsets and Markdown remain unprefixed.
- PPTX fixtures cover native and action-less internal relationships, reordered/hidden/self targets, repeated Unicode labels, links crossing paragraph boundaries, nested groups, table runs, group/shape actions and unchanged URLs. Known contextual actions retain omission diagnostics. The corpus records upstream unknown/unresolved source action loss and verifies native remapping of unusually large shape IDs. Synthetic native duplicate-ID/count tests protect adapter indexing; source-count truncation is not solved by that post-import check.
- Final wheel: 17,347,990 bytes; uncompressed extension: 40,233,864 bytes; SHA-256 `cc8b188984ac3f426b3f54d02b9f3016561dd4a6956055945adac1c8b22c6b51`. Cargo.lock SHA-256 remains `fcfb4a174387cf25cceddadeba9e6be930ec790c840dca0bb5f169431a6c089c`; the 264-package permissive graph and pinned upstream revisions are unchanged. E3 and TXT5 remain partial; no new public methods or backend dependencies were added.

### Slide creation identity independent review

- Review reproduced a callback-bearing coordinate or row iterator reordering slides during `add_text`, `add_shape` or `add_table`, inserting into the wrong slide. All three methods now normalize inputs before resolving the stable target slide. The regression verifies original target identity and the returned handle for numeric and row-generator callbacks.
- The original frozen picture/selection package independently passed all 224 Python tests; its unchanged native package with the corrected facade passed all 225. Exact corrected head `da5814d52402f978622f4d7b036c9f54d4441339` passed CI 39473099033: formatting, 198 Rust tests, warnings-denied Clippy, 225 installed-wheel Python tests without skips and the unchanged 264-package license audit.
- CI artifact: CPython 3.11.17 / Linux x86_64; wheel 17,334,217 bytes; uncompressed extension 40,201,024 bytes; wheel SHA-256 `248e087547542396f16a78162beba1c45c196645c787a42b4dc080228904d86c`. No dependencies or upstream revisions changed.

### Single-parser text and benchmark independent review

- The pinned renderer uses the same native PDF parser as inspection. Valid text extraction now avoids inspection's extra parse and structural metadata traversal; zero-page/failed parses still use inspection for stable error classification. The new native regression verifies missing/wrong/correct passwords, blank-page separators, selected ordering, empty selection, out-of-range selection and malformed input.
- Benchmark review corrected input-file overwrite through direct paths or aliases, required text page counts to match the source and PNG dimensions to match across libraries, and guaranteed warm-document cleanup. Four dependency-free harness tests pass. Blank text remains valid; no extracted content is published, and native backend diagnostics are explicitly outside the CLI's exception-message suppression. Private timing claims are author-reported, not independently reproduced or a fidelity guarantee.
- Exact combined head `cd0970887770b37481f18e031f4f17c2ca847569` passed CI 39473827086: formatting, 199 Rust tests, warnings-denied Clippy, 229 installed-wheel Python tests without skips and the unchanged 264-package locked license audit. Merging current main produces the identical tested tree.
- CI artifact: CPython 3.11.17 / Linux x86_64; wheel 17,334,700 bytes; uncompressed extension 40,201,568 bytes; wheel SHA-256 `5fc1a091666dd7aed59ced9f8d51bedbea6cc332ca64241860e94ba2225e77f8`. No dependencies or upstream revisions changed.


## 2026-10-20 explicit list starts and PPTX source-count bounds

- Complete Rust 1.95 locked/offline formatting, 213 Rust tests and warnings-denied all-targets Clippy pass. A newly built release wheel installed into a clean CPython 3.11.2 environment passes all 248 Python tests with no skips, errors or failures. The installed facade/native bytes match the wheel. This batch includes PR43's reviewed selective PDF parser/benchmark changes and PR44's bounded numbering-definition cache correction.
- List-start fixtures verify fresh native abstract/instance IDs, saved level-zero overrides, source/PDF labels 7/20/21/32767/32768, nearest-sequence continuation, unchanged previously assigned IDs, restart/clear, nested cells, footnotes/endnotes, copies and warmed PNG invalidation. Invalid values, deleted/closed targets, caller index callbacks and native list-definition budgets fail atomically. A native fixture reproduces measurement consuming header/footer counters; supplied starts in those scopes fail explicitly rather than compensating locally. Broader custom-level/list authoring remains open.
- PPTX preflight accepts 10,000 source entries and rejects 10,001 before extraction, native rendering or PDF conversion. Tests cover repeated relationships, relocated non-XML-named main parts, UTF-8 BOM, UTF-16 LE/BE, leading text, selected AlternateContent, comments/CDATA, nesting/token/prolog bounds, unused presentation parts and large binary/referenced PNG media with actual visible pixels. The prior frozen wheel demonstrably imports a 10,001-entry selected alternate branch as 10,000 slides; the new guard rejects it. Every sldId descendant counts conservatively, including section lists and unselected branches, so the effective supported slide count can be lower when references are redundant. No OPC or markup-compatibility resolver was added.
- Promoted already-locked MIT quick-xml 0.41.0 to a direct safety dependency. Cargo.lock changes only the root dependency edge. Both third-party license inventories were regenerated and reviewed, remaining byte-identical at 264 allowed normal/build packages; existing attribution already covers the crate. No prohibited application package or upstream revision changed.
- Final wheel: 17,358,227 bytes; uncompressed extension: 40,254,792 bytes; SHA-256 `6bd2955603084d12a8852651c83d635d4333378fdabdbdcf3cfd7abb27898e9f`. Cargo.lock SHA-256: `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. The PPTX scan holds at most one 96 MiB member plus up to 144 MiB UTF-16 decoding storage, within the existing 512 MiB aggregate expansion profile; this is not a total process-memory or native-layout allocation guarantee.
### Numbering and navigation independent review

- The original frozen feature package independently passed all 235 Python tests. Review of the pinned native numbering API found repeated linear instance/abstract scans during metadata projection. The adapter now caches native definition results and missing references and charges a conservative cumulative lookup-work budget, without introducing a local resolver. Native layout costs remain outside this adapter budget.
- The regression covers 2,000 repeated last-definition references, cached missing references and rejection of distinct queries exceeding the work bound. An initial test-only path index type mismatch was corrected before the successful build. Exact integrated head `b66d2f1c2d96ee70a28052d1e5d9fb7b05a16792` passed CI 39474958430: formatting, 206 Rust tests, warnings-denied Clippy, 239 installed-wheel Python tests without skips and the 264-package locked license audit. Merging current main produces the same tested tree.
- CI artifact: CPython 3.11.17 / Linux x86_64; wheel 17,345,997 bytes; uncompressed extension 40,230,384 bytes; wheel SHA-256 `ae5cc1404594e1468a1af97746c5da7e7f9bc2aeac10fbaa65b2aa09ce984751`. No dependency or upstream revision changes.


## 2026-10-20 picture transforms and owned field geometry

- Rust 1.95 locked/offline formatting, 219 Rust tests and warnings-denied all-targets Clippy pass. The release wheel installed into a clean CPython 3.11.2 environment passes all 256 Python tests with no skips, failures or errors; loaded facade/native bytes match the artifact. The unchanged lockfile passes the 264-package normal/build license audit; all four upstream revisions remain pinned.
- DOCX crop and independent horizontal/vertical mirroring preserve original image bytes and frame dimensions through native readback, replacement, copying and nested story/table placement. Asymmetric source and exported PDF pixels verify transforms and cache invalidation. Invalid transforms, deleted/closed handles and sub-point frames fail atomically. Native regressions retain the one-point layout clamp and quarter-turn edge-loss evidence; DOCX rotation remains unavailable pending upstream fidelity repair.
- Owned PDF text/checkbox/choice fields move and resize in current page-view coordinates while retaining native widget identity, values/options and appearance orientation. Fixtures cover all four page rotations, CropBox/UserUnit, fields created before page rotation, visible vacated/new pixels, text bounds, copies, stale-handle callbacks and warmed caches. Flattening retains byte-identical rendered pixels. Invalid fit/geometry and cumulative resource limits roll back; imported/multi-widget forms remain outside this profile.
- Linux x86_64 wheel: 17,367,230 bytes; uncompressed extension: 40,273,928 bytes; SHA-256 `cf6467de8eb10c54187a1359f326f382c5290d3a65bdeaf9412c9abdca0dfe08`. Cargo.lock SHA-256: `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. No dependency changes or broad MED3/PDF6 completion claims.


## 2026-10-20 private Office render reuse

- Complete Rust 1.95 locked/offline formatting, 227 Rust tests, warnings-denied all-targets Clippy and the unchanged 264-package license audit pass. A new release wheel in clean CPython 3.11.2 passes all 261 Python tests without skips, failures or errors; imported facade/native bytes match the artifact. An initial public fixture called paragraph formatting for a text color; it was corrected to the existing TextRange formatting API before the final passing corpus.
- Native build counters independently prove one parse/layout across repeated DOCX renders and one import across repeated PPTX renders. Full/region PNGs are byte-identical to one-shot calls at 72/144/300 DPI across mixed pages/hidden slides. Tests cover bounds, oversized-full/valid-small-clips, malformed inputs, source lifetime, owned payloads, Send+Sync, close/render races, direct native close and poisoned-lock cleanup. Weak model references prove owned native state release without relying on RSS. Python hydration-failure tests retain the session reference and verify it is already closed.
- The [Office benchmark](../../../docs/office-performance.md) measures original 20-page/20-slide fixtures in separate processes at 144 DPI. Warm medians: DOCX 23.88 ms one-shot vs 11.85 ms retained; PPTX 7.79 ms vs 4.42 ms. Full 20-page traversal:478.02 ms vs 235.37 ms and158.41 ms vs 88.86 ms respectively. Same-page PNG hashes match. Peak RSS is 61.66/61.49 MiB for DOCX and54.86/52.29 MiB for PPTX (one-shot/retained). These are single-process samples with five warm repetitions, not a representative corpus or stable cross-platform speed claim. Native cache/allocator RSS did not drop at close; session model release is verified separately.
- Wheel 17,369,972 bytes; uncompressed extension 40,281,240 bytes; SHA-256 `1f1e4cbd1c973fd299caa31b1f9ac7f40cb3bccf23e8cbfc97a61ff10a12192b`; Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Linux x86_64; all upstream pins unchanged. No dependencies added. P1 remains open for representative large-corpus/peak-layout/resource-cache evidence; post-layout bounds do not constrain earlier native allocation.


## 2026-10-20 page fields and imported DOCX metadata

- Rust 1.95 locked/offline formatting, 234 native tests, warnings-denied all-targets Clippy and the unchanged 264-package license audit pass. A newly built release wheel in clean CPython 3.11.2 passes all 272 Python tests without skips, errors or failures. Imported facade/native bytes match the wheel. All upstream revisions remain unchanged.
- Native decimal PAGE/NUMPAGES/SECTIONPAGES fields preserve ordinary range formatting, replacement/deletion and revision checks. Fixtures verify body/cell/header/footer/footnote/endnote numbers, section starts/continuation, 3-to-4-page repagination, copies and warm raster invalidation, actual red glyph pixels, serialized native instructions and intentionally empty cached source results. All-story field limits, invalid values and owner callbacks fail atomically. New sections continue numbering; continuous restarts and affected section-page counts are rejected. Endnote scope follows public native placement options, including document-end notes whose reference occurs in a different section.
- Native regressions reproduce inconsistent implicit numeral formatting by story, Roman/alphabetic saturation, ignored continuous restarts and shared-page SECTIONPAGES undercount. These remain upstream limitations; the adapter does not evaluate or repair counters.
- Imported custom metadata shares the authored strict scalar conversion and preflights parsed count/value/raw aggregate bounds before cloning. Tests cover exact boundaries, unsupported/malformed values, omission of all ASCII-case-insensitive collisions, bounded diagnostics, owned snapshots and unchanged text/PNG output. Nested/header table title/description retains absent-versus-empty semantics without duplicating visible text/Markdown. Separate source fixtures demonstrate native malformed-part dropping, the source property cap and table alternative-text prefix truncation; no completeness notice is fabricated after the native reader discards data.
- Linux x86_64 wheel: 17,379,748 bytes; uncompressed extension: 40,304,088 bytes; SHA-256 `e5432b03c1449c05fe1b7e2d048258b238e6469d4fb9e4f97d3cd536cb5e7d80`. Cargo.lock SHA-256: `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. No new dependencies. LAY6/REF3/DOC7/TBL10 remain broader open criteria; TODO and support prose record these verified subsets rather than closing them.

## 2026-10-20 PDF benchmark text parity and malformed-page fixtures

- The benchmark now requires at least 95% multiset token overlap after validating each text result's page count against the source. Two empty token sets count as a match; one empty result versus nonempty text does not. Preflight strings are released before timing. Dependency-free regressions cover blank pages, missing pages, repeated tokens and Unicode; existing image-size/output-path/privacy checks remain intact.
- Original two-object PDFs exercise missing Kids and cyclic page trees through both public and private selective text entry points, including empty selections. They return the existing stable malformed-document exception and do not poison later valid extraction. These fixtures test existing error containment: the pinned parser constructs its page inventory eagerly inside its existing guarded parse operation; no new runtime panic wrapper or claimed lazy-tree fix is included.
- All 11 focused benchmark/selective-bound tests passed against the copied installed PR47 wheel. This change leaves runtime source, dependencies and upstream pins identical to main PR47. Full current-head CI evidence is recorded in the PR review. Private-corpus overlap figures remain explicitly author-reported and are not independent fidelity evidence.

## 2026-10-20 page-field independent review correction

- Independent review found that an intermediate section-end endnote collector can flush notes anchored in earlier document-end sections. The original page-field artifact above did not cover this ownership case. The guard now follows the first native section-end collector at or after each anchor, otherwise the final section. It rejects affected continuous-section SECTIONPAGES fields and later placement/start changes before commit; an earlier safe collector remains supported. This corrects adapter validation, not native page-counter evaluation. Native and public regressions reproduce the original bypass and assert exact serialized rollback.
- The corrected source includes the reviewed PDF benchmark/malformed-page regressions. Rust 1.95 locked/offline formatting, all 235 native tests and warnings-denied all-targets Clippy pass. A replacement release wheel installed into clean CPython 3.11.2 passes all 276 Python tests with zero skips, failures or errors; the harness verifies installed facade/native bytes against that artifact. The unchanged 264-package license audit passes.
- Linux x86_64 replacement wheel: 17,379,909 bytes; uncompressed extension: 40,304,664 bytes; SHA-256 `abdcffd591dc8b84b019e8bdfe294258b7fadcf6c7949be704f354bb212984d3`. Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`; all upstream revisions unchanged. This artifact supersedes the earlier page-field artifact for correctness. Broader field/property limitations remain open.

## 2026-10-20 page-field and image composition review correction

- Further independent integration review reproduced rejected page-field/image combinations: the authored image-retention walk did not recognize media-free field markers. It now accepts native fields with empty legacy form XML while retaining rejection of raw form payloads, unsupported graphics and OLE. A native regression checks both insertion orders and raw-form rollback of bytes, revision and media accounting. Two public regressions verify actual DOCX/PDF colors and evaluated fields, body/nested-cell/header/footer/footnote composition, shared-media deduplication, replacement/deletion, copy independence and reclamation. No public API was added.
- The replacement artifact passes Rust 1.95 locked/offline formatting, all 236 native tests, warnings-denied all-targets Clippy and the unchanged 264-package license audit. Clean CPython 3.11.2 installed-wheel validation passes all 278 Python tests with zero skips, failures or errors, including exact installed-package/artifact byte comparison.
- Linux x86_64 wheel: 17,380,309 bytes; uncompressed extension: 40,304,728 bytes; SHA-256 `852fb47ab7173c2a2fb72e33544837eef1cd0ce2b65e69840404134912232a62`. Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`; upstream pins and dependencies unchanged. This artifact supersedes both earlier page-field artifacts for combined field/image correctness. Broader page-field and native property-reader limitations remain open.

## 2026-10-20 bounded PDF selection validation

- Existing copy/subset and page insertion now share the selected-text iterator bound. Six regressions cover the exact 10,000-index facade boundary, one unconverted excess item, infinite input termination, invalid/oversized indexes, ordered duplicates/negative indexes, active-form error precedence and unchanged output on failure. Callback tests cover page additions, moves, destination close, insertion-index conversion and source reads; distinct source closure after snapshot and ordinary metadata edits remain valid.
- A new release wheel incorporating the reviewed field/image correction passes all 284 tests in clean installed CPython 3.11.2, with zero skips, failures or errors and verified wheel/package byte identity. Native sources, dependencies and lockfile are identical to the preceding 236-test Rust/Clippy/fmt/license-validated field/image artifact; this slice changes only facade selection validation, tests and documentation.
- Linux x86_64 wheel: 17,380,431 bytes; uncompressed extension: 40,304,728 bytes; SHA-256 `fee1008f39d352dbc6d5293e5853e46ec9f9aedbccaf267f3e4b34458f01ddf6`. Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. This does not broaden PDF preservation, active-form subset or page-count guarantees; it bounds Python normalization before existing native operations.

## 2026-10-20 executable cookbook and API10 usability

- Five new examples plus the existing authored template recipe now expose import-safe functions and explicit output-directory/DPI CLIs. Seven installed-wheel tests execute all six workflows and CLI entry points, reopen native and PDF outputs, render every document page, verify semantic order/tables/links, actual colors, background/header/footer content, independent assembly/template sources, PNG dimensions/resolution and a clipped slide preview. All generated fixtures are synthetic and require no optional image library, assets, network or document executable. Test subprocesses invoke only the Python example CLI.
- The usability review records new PDF/PPTX page defaults, point/XYWH units, story methods, snapshot versus editable cells, stale page/range handles, independent copies and explicit save/convert/render/close behavior. Current guides correct stale notes/navigation claims, and historical sketches remain clearly separated from executable examples. The source diff adds zero library public methods/classes or dependencies. API10 is complete for the current authored profile; API1/API3/API7 and broader QA corpus/support inventory remain open.
- A newly packaged wheel passes all 291 tests in clean installed CPython 3.11.2 with zero skips, failures or errors; package bytes match the artifact. Native sources and locked license graph are unchanged from the 236-test Rust/Clippy/fmt/264-package-audited base. Linux x86_64 wheel: 17,380,454 bytes; uncompressed extension: 40,304,728 bytes; SHA-256 `2dc178192188236205942e96991bc790cf35e31c2167bebeae1663ea2bb6f0ac`; Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. All upstream pins remain unchanged.

## 2026-10-20 native DOCX image backgrounds

- Existing background_image vocabulary now covers owned native DOCX frame fills. Two native regressions and four public groups verify original media/readback, mixed page dimensions at 72/144 DPI, PNG/PDF color/transparency and clipped output, independent page color, fields/notes/nested shared images, copy/cache behavior, replacement/delete/clear/reclamation, missing native references, exact encoded-budget checks, bounded sources and owner-close callbacks. The global background reference counts once in existing resource accounting and remains outside inline image collections.
- Rust 1.95 locked/offline formatting, all 238 native tests, warnings-denied all-targets Clippy and the unchanged 264-package license audit pass. The newly built wheel installed into clean CPython 3.11.2 passes all 295 Python tests with zero skips, failures or errors; installed facade/native bytes match the artifact.
- The committed synthetic upstream reproducer confirms watermark ink-bound/color and excessive-tile PNG/PDF divergence on this installed wheel. Exact outputs and pinned source links are recorded in the upstream register. These are upstream fidelity requirements, not local algorithms; BG2/BG3/BG4/BG5 remain broader open criteria.
- Linux x86_64 wheel: 17,382,496 bytes; uncompressed extension: 40,309,528 bytes; SHA-256 `42b03bb2085767fc3fadb228d428b0496301bde91946753e00ef49e7e04e1065`; Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. No dependencies or upstream revisions changed.


## 2026-10-20 rich presentation cell paragraphs

- PPTX cells reuse the existing Cell/Paragraph/TextRange vocabulary for native paragraph insertion, alignment, run formatting, replacement and external/internal links. Three native regressions and six public groups cover canonical merged-cell anchors, copies and slide reorder, reset/grid/range invalidation, empty bodies after split, rejected Word-only members, callback ownership, resource budgets and atomic errors. Saved PPTX and exported PDF fixtures verify text order, links and visible red/blue/highlight pixels; no additional public class or method family was exported.
- Rust 1.95 locked/offline formatting, all 241 native tests, warnings-denied all-targets Clippy and the unchanged 264-package license audit pass. A newly built release wheel installed into clean CPython 3.11.2 passes all 301 Python tests with zero skips, failures or errors; installed facade/native bytes match the artifact.
- Linux x86_64 wheel: 17,401,998 bytes; uncompressed extension: 40,360,696 bytes; SHA-256 `fbff6422d361b0c32877798de685847d3564e521e77545a2083e269cefd2fc98`; Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Dependencies and all upstream revisions remain unchanged. Paragraph spacing/indent fidelity, nested rich content and broader SLIDE3 work remain open; native row growth is not an automatic page-containment guarantee.

Independent review reproduced a shared literal-replacement callback defect: a string subclass could reset a cell during the staged operation, after which commit overwrote that caller change. Find/replace now converts validated operands with base str semantics before inspecting values or capturing handles. A new regression passes for DOCX paragraphs, slide shapes and slide-cell paragraphs, including Unicode, duplicate matches, strict errors and suppressed subclass hooks. The 301-test artifact above predates this correction; corrected exact-head installed-wheel evidence is required before merge. Native code and the license graph are unchanged by the correction.


## 2026-10-20 everyday API typing and native-free reprs

- Common factory/context/copy and creation/import/merge/bookmark/annotation chains now retain concrete owner/handle types. Literal text-output modes distinguish strings from dictionaries. AST comparison against the reviewed rich-cell base verifies unchanged runtime parameter names, kinds and defaults; no public exports or native methods were added. Existing Paragraph/EditableTable/Style reprs use stored identity and owner closure, avoiding native reads for large, stale or closed children. Regressions cover the actual private PPTX cell paragraph without a DOCX ID and propagation of unexpected owner errors.
- A pinned development-only mypy 1.18.2 strict consumer passes against the newly installed wheel, including rich PPTX cell paragraphs. The earlier identical core consumer failed with 42 diagnostics against the frozen background-only package, confirming sensitivity to erased owner/handle and text-output types. Imported-module diagnostics are silenced to scope the gate to these consumer chains; this is not a full-package strict-typing claim. CI now runs this check after installing the wheel.
- The final artifact incorporates PR53's reviewed base-string normalization correction. All 304 Python tests pass in clean installed CPython 3.11.2 with zero skips, failures or errors and exact package/artifact byte matching. Native source and the locked dependency graph remain identical to the 241-test Rust/fmt/Clippy/264-package-audited rich-cell base. The earlier local 303-test typing artifact predates the literal correction and is superseded by this artifact.
- Linux x86_64 wheel: 17,403,265 bytes; uncompressed extension: 40,360,696 bytes; SHA-256 `2baae14c7095ef5ac4526fc0ee1d5f20e145cf31adf892e8753cb8d8222f004e`; Cargo.lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Upstream pins and runtime dependencies are unchanged. Broader property annotations and API3 remain open.


## 2026-10-20 bounded operation evidence ledger

- Seven initial profiles contain 49 explicit operation cells and join all 139 existing roadmap IDs. Eight criteria have profile seeds; the remaining audit scope is explicitly unassessed, including recipes/gates. Ownership, roles and checkboxes are derived from existing roadmap files. No runtime capability API, engine, dependency or source behavior changed. API1/API7/QA1/QA8 remain open.
- Source review checked 20 native path/symbol citations at immutable cached revisions and confirmed the seven cited fixture files match the historical cookbook artifact's source. The matrix keeps that artifact's dated hash and the distinct source-audit baseline visible. These historical test results do not certify subsequent facade/native changes; source fingerprints detect drift only.
- Fourteen new offline tests verify coverage/ownership joins, selectors and fingerprints, pinned references, section-specific artifact hashes, concrete upstream references, paths, malformed/optional evidence, calendar dates and exact generation. They pass alongside the complete 318-test corpus against the unchanged clean installed typing/review-corrected wheel, with zero skips, failures or errors and exact artifact/package byte checks. Native evidence remains the preceding 241-test Rust/fmt/Clippy/264-package audit; no new native build was needed for this development-only change.
- Reused Linux x86_64 CPython 3.11.2 artifact: wheel 17,403,265 bytes; extension 40,360,696 bytes; SHA-256 `2baae14c7095ef5ac4526fc0ee1d5f20e145cf31adf892e8753cb8d8222f004e`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. The checker and roadmap ownership/dependency validation pass offline. Complete SDK/PyMuPDF member dispositions and broader operation audits remain separate work.


## 2026-10-20 pinned reference-family inventory

- The bounded comparison groups 194 representative members into 44 families, using Open XML SDK v3.5.1 (`3139fdfd27414548a41555f7848d5728f6e71a42`) and PyMuPDF 2.0 (`b8904b27efa0c4e2258f0e46ab4498a6173ffa22`). All 17 cited primary-source files passed exact cached byte/SHA-256/line-count checks; every member exists at its recorded line. No comparison library or source file is linked, vendored or downloaded at runtime.
- All 139 criteria receive a derived reverse join, with 131 linked directly to a reference family and eight explicitly without a direct family in this bounded pass. Ownership derives from the existing workstream data. All seven initial operation profiles have reference-family connections, without copying their tested statuses into the comparison inventory. The local-surface audit remains scoped to its recorded source commit; full member dispositions and parity remain open.
- Four new offline regression groups cover pinned URL/line/identity drift, unknown criteria/profile/upstream joins, forbidden duplicated ownership, cached-source hashes/member locations and generated consistency. An integration regression initially exposed re-reading the on-disk ledger during generation from an already supplied validated ledger; the generator now passes that ledger's profile identities explicitly. Existing malformed-file/no-write assertions remain unchanged and pass.
- The complete 322-test corpus passes against the unchanged clean installed CPython 3.11.2 wheel with zero skips, failures or errors. Exact source-cache verification, both generated guides and the roadmap checker pass offline. Reused wheel: 17,403,265 bytes; extension: 40,360,696 bytes; SHA-256 `2baae14c7095ef5ac4526fc0ee1d5f20e145cf31adf892e8753cb8d8222f004e`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Native source/runtime dependencies remain the preceding 241-test Rust/fmt/Clippy/license-validated base.

### Independent member-citation correction

Review found that word-only source matching accepted descriptions such as “write” beside `Story.draw`, “fit” beside `Story.place`, and “save” beside `Document.can_save_incrementally`. Nine citations now point to public declarations or exact member entries. Cached validation requires explicit RST API roles/directives/calls, schema `ClassName` values or source call/declaration tokens. A fifth regression group rejects the false matches. All 17 cached source hashes and 194 corrected member locations pass; the complete unchanged-wheel suite passed 323 tests independently (53.463 seconds), superseding the initial 322-test review run. New inventory and ledger tool/test text I/O is explicitly UTF-8; their 19 focused tests pass with default-encoding warnings treated as errors. Final current-main CI rebuilds and checks the corrected commit before manual merge. No runtime source, public API, dependency or upstream pin changes.


## 2026-10-20 header/footer rich content

- Header/footer variants reuse existing table/cell/paragraph/picture handles for direct tables and images, rich nested content, and decimal fields. Synthetic trailing anchors remain distinct from explicitly authored empty paragraphs. Picture deletion retains an ordered empty paragraph; section/document copies keep independent stories and media references. Notes remain paragraph-only.
- Two new native groups and six public workflows verify source part order, first/default/even selection, mixed page sizes, copies, fields/background/media accounting, callback closure, stale pictures, inactive-variant atomic errors, later content/font overflow and intentional exact-height clipping. Header/footer PNG output disables the upstream editor dimming option; actual saturated colors match native PDF in full/region and retained/one-shot paths, while body colors remain unchanged.
- Complete locked Rust tests: 243 passed. Formatting and all-targets Clippy pass; the unchanged runtime/build graph audits 264 packages. Release wheel installed into a new CPython 3.11.2 environment passes all 329 Python tests with zero skips, failures or errors and exact installed artifact byte matching. The strict installed-wheel typing consumer, roadmap checker, generated support/reference guides and cached reference-source audit also pass.
- Linux x86_64 wheel: 17,421,532 bytes; uncompressed extension: 40,405,080 bytes; SHA-256 `fe069561df57ea88495deadff69e4d05a19c0ae395f80f2937aeae09ab48c356`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. All upstream revisions and dependencies remain unchanged.
- Physical checks inspect native table/picture frames after layout and respect explicit cell clipping; they do not establish full text-ink containment or peak-memory bounds. Inactive variants may lack cell geometry for nested operations and later section changes. Header/footer linkage, clearing/general movement, rich notes and broader LAY3/BG5 remain open.


## 2026-10-20 bounded native line numbering

- Section.line_numbers reuses native section properties and decimal page/section/continuous labels. Six native groups and eight public workflows verify source properties, inheritance/copy/reset, automatic pages, displayed intervals, native suppression/table exclusions, mixed sections, source Unicode, actual margin ink, PDF labels, invalid scalar/work bounds and output-time logical extents. The integrated story/line fixture keeps both overflow guards active after edits and restores the original rendered output after correction.
- Raw native regressions retain overlapping multi-column labels and stale each_page counters after intervening unnumbered pages. Affected configurations fail atomically. Root review found table-bearing section mutation could paginate a candidate before commit; both authored full-layout entry points now preflight label work directly, with helper-level and public rollback regressions. The conservative 100,000-unit bound covers additional label work, not overall process memory or imported-source layout.
- Complete locked Rust tests: 249 passed. Formatting, all-targets Clippy and the unchanged 264-package license audit pass. The release wheel in a new CPython 3.11.2 environment passes all 337 Python tests with zero skips, failures or errors and exact artifact/package byte matching. Strict installed-wheel typing and roadmap/support/reference checkers also pass.
- Linux x86_64 wheel: 17,425,780 bytes; uncompressed extension: 40,413,112 bytes; SHA-256 `69b9da8f4227e2e2efe02ada4c9f94b3ee7275ff17c22accb2cdbe870c2fe114`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Dependencies and all upstream pins remain unchanged.
- Native label geometry is checked without local counter/font/layout reconstruction. DOCX semantic extraction omits layout labels; PDF classification/order stays native. Multi-column placement, affected continuous page restarts, nondecimal page formats and broader LAY6 remain open with upstream evidence.


## 2026-10-20 incremental story and numbering evidence

- The existing ledger now contains nine bounded profiles and 63 operation cells. LAY3/BG5/LAY6 gain profile joins; all 139 criteria retain their canonical ownership and broad completion state. Story and line-number profiles cite their distinct 329-test/337-test artifacts and source commits, with byte-identical fixture fingerprints checked against those commits. Read, authored/imported edit, preservation, rendering and PDF export remain separate; unsupported/unassessed scope is explicit.
- The reference inventory retains 44 families and now cites 195 representative members. LineNumberType points to the exact ClassName entry at line 22696 of the already pinned SDK schema; all 17 cached source hashes/line counts and member locations pass. Public native insert_block, LineNumbering/LineNumberRestart, Placed/layout, rendering and PDF export symbols were checked in the pinned source cache. No reference product becomes a dependency.
- The CLI regression initially exposed its temporary repository omitting Python adapter sources now referenced by the new profiles. The fixture copies src alongside existing native/tool inputs; malformed-ledger/no-write assertions remain unchanged. All 19 checker regressions and the complete 337-test installed-wheel corpus pass after this correction, with zero skips/failures/errors. Generated guides and roadmap validation pass.
- Runtime source and dependencies are unchanged from the preceding 249-test native/fmt/Clippy/264-package-audited line-number baseline. Reused Linux x86_64 CPython 3.11.2 wheel: 17,425,780 bytes; extension: 40,413,112; SHA-256 `69b9da8f4227e2e2efe02ada4c9f94b3ee7275ff17c22accb2cdbe870c2fe114`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. This development-only extension does not complete API1/API7/QA1/QA8 or broader format parity.


## 2026-10-20 native paragraph direction and run language

- Existing DOCX formatting accepts paragraph direction and bounded literal run-language metadata, with native style/default inheritance, explicit LTR versus reset, direct/effective snapshots and existing table-region dispatch. No method/object family or backend dependency is added. The language bound rejects values the pinned reader would drop; conditional table paragraph direction remains explicitly unavailable.
- Three native groups and five public workflows prove native mixed Persian/Latin/digit glyph geometry with nonzero glyph IDs, actual PNG/PDF colored extents within two pixels at 144 DPI, explicit LTR placement/reset recovery, source Unicode preservation, body/nested-cell/story/note scopes, whole-table versus direct direction, conditional language precedence, copies, stale ranges, callback closure and atomic invalid tags/values. Hebrew/emoji/combining range preservation is not a broad glyph/shaping claim.
- Complete locked Rust tests: 252 passed. Formatting, all-targets Clippy and the unchanged 264-package runtime/build license audit pass. A new clean CPython 3.11.2 environment passes all 342 installed-wheel Python tests with zero skips/failures/errors and exact installed artifact byte matching. Updated strict typing consumer calls direction/language through the existing format methods; roadmap/support/reference checks pass.
- Linux x86_64 wheel: 17,427,100 bytes; extension: 40,415,864; SHA-256 `6f4f6d5ac0fae778490b20edd13dd224eafca5e221af27ad5938155e9f45db71`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. All upstream pins and dependencies remain unchanged. Broader TXT6 complex-script/CJK font controls and missing-glyph/substitution diagnostics remain open.


## 2026-10-20 standard PDF geometric annotations — initial artifact (superseded)

- This initial artifact used the unmerged public spelling `oval`; final API review replaces it with `ellipse` to match existing shape APIs and Pillow. The replacement artifact evidence follows below. Native `Shape::Oval` remains unchanged.
- Rectangle/oval comments reuse existing annotation creation/properties/update/deletion and native shape/set_rect/set_style behavior. The bounded profile has opaque unfilled two-user-unit outlines, canonical imported dictionary checks and dimensions at least three native user units. A direct-native regression preserves tiny-oval disappearance; actual minimum-size controls render visibly at the tested DPI/UserUnit.
- Three native groups and seven new public workflows verify real outline/empty-center pixels, preservation of original page text/image pixels, saved PNG equality, all rotations with CropBox/UserUnit, native resizing, metadata/color, copies/deletion, rejected imports, minimum/work/input/item limits and compound appearance costs. Resize-plus-recolor charges both native streams, including its superseded intermediate appearance. No foreign appearance stream is decoded by the profile check.
- Callback regression against the previous facade reproduced edits to replacement pages after page reordering. Normalization now occurs before the final identity check; root review also retained the initial checks so already-closed/stale handles never evaluate invalid or callback-capable arguments. Reordering, closure and deletion-stale tests pass without retargeted edits.
- Complete locked Rust tests: 255 passed. Formatting, all-targets Clippy and the unchanged 264-package license audit pass. A clean CPython 3.11.2 environment passes all 349 installed-wheel Python tests with zero skips/failures/errors and exact artifact byte matching. Strict typing and roadmap/support/reference checks pass.
- Linux x86_64 wheel: 17,436,457 bytes; extension: 40,437,576; SHA-256 `383d4eab71c4763a3bf1e4980f354d6092c0f9c4d450c11f869243549c896ccc`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. All upstream pins and dependencies remain unchanged.
- Target mutation/appearance bounds do not cover existing global summary/initial inspection allocations, which are recorded as a separate upstream inventory candidate. Broad PDF6 imported forms, radio groups, replies and arbitrary appearance preservation remain open.


## 2026-10-20 geometric annotation vocabulary review and replacement artifact

- Final API review changes the unmerged public kind/readback to `ellipse`, matching existing slide shapes and Pillow vocabulary; no redundant `oval` alias is introduced. Native `Shape::Oval` and the tiny-oval upstream regression retain their native names. Runtime source commit: `8949ec6`.
- A fresh release wheel and separate clean CPython 3.11.2 environment pass all 349 installed-wheel tests with zero skips/failures/errors and exact artifact byte matching. All 255 locked native tests, formatting, all-targets Clippy, strict typing, roadmap/support/reference checks and the unchanged 264-package license audit pass.
- Linux x86_64 wheel: 17,436,840 bytes; extension: 40,437,672; SHA-256 `c668514ce02ec8cd94e7056aa21970e7331eacb7e20bd17763a79b377086cf31`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. This supersedes the initial geometric artifact above. All profile limits, upstream pins and broader open criteria remain unchanged.


## 2026-10-20 PDF inventory upstream reproducer

- Development-only source audit and import-safe synthetic fixture tool record preexisting native summary copies, QuadPoints tail truncation and repeated-reference amplification. This does not change annotation support, introduce a local imported-profile restriction or implement a parser. The public geometric spelling is ellipse; native Circle/Oval identifiers are unchanged.
- Two focused fixture tests passed against the existing installed CPython 3.11 wheel in `/opt/ellipseannotationtest`: deterministic xref offsets/native repeated-reference identity and strict bounded fixture arguments. The three-case probe returned one color-less summary for 100,000 C values (200,503 input bytes; 145 JSON bytes), accepted 8,193 QuadPoints values (16,909 input bytes; 148 JSON bytes), and returned three summaries for one repeated reference (518 input bytes; 442 JSON bytes). Geometry updates rejected the first two separately with UnsafeArchiveError/UnsupportedFormatError. These are observed native-boundary results, not memory measurements or assertions that unsafe inventory acceptance must remain.
- Exact pinned source citations distinguish secondary summary allocation from earlier initial inspection, lazy object parsing and decompression. The upstream acceptance asks for fallible preallocation/occurrence budgets and strict error propagation through fallback. PDF6 remains open. No runtime/dependency/pin changes, Cargo rebuild, new wheel or complete-corpus validation is claimed by this isolated documentation patch; the preceding artifact evidence remains separately scoped.

- Root integration reran the probe and all 351 installed-wheel Python tests (zero skips/failures/errors) using the unchanged ellipse artifact, with byte matching. Roadmap/support/reference checks pass. This validates the added development evidence alongside the existing runtime; native tests and wheel sizes remain the preceding artifact's 255-test/build evidence.


## 2026-10-20 direction and geometric-comment operation evidence

- Two new bounded profiles extend the ledger to eleven profiles and 77 cells, preserving all 139 coverage owners/checks and 44 reference families/195 representative members. Historical direction and reviewed ellipse artifacts are separately named; two fingerprinted test files and native public-property/annotation/writer/render sources were reviewed. No runtime/dependency or public API change.
- Offline roadmap, support and cached-source reference checks pass. The corpus passes all 351 installed-wheel Python tests with zero skips/failures/errors and exact byte matching against the unchanged ellipse wheel (`c668514ce02ec8cd94e7056aa21970e7331eacb7e20bd17763a79b377086cf31`; 17,436,840 bytes; extension 40,437,672). The preceding 255 native-test/build evidence remains scoped to that artifact; no new native build is claimed. Full parity remains open.


## 2026-10-20 owned DOCX bookmarks and internal navigation

- Existing TextRange and range.link now support named owned body/nested-cell targets through a small mapping. Native paired markers, source-order enumeration, fresh path lookup, private exact-pair deletion and all-story reference checks preserve semantics and ownership. Root review expanded same-paragraph relocation/self-assignment/partial-endpoint coverage, while other semantic/evaluated-field selections remain rejected. Runtime source commit: `8051068`.
- Five native groups and seven public workflows cover Unicode/source offsets, empty targets, pair limits/deletion, copies/grid staleness, repagination carets, actual nested-cell PDF link geometry, shared media/page-field/note composition, callback closure, missing targets and atomic failure/recovery. Direct-native repros retain header-target omission, clipped hit areas, vertical-link loss and 10,001 linked batches producing only 10,000 native annotations; the admitted local profile rejects these cases without synthesizing destinations or link rectangles.
- Complete locked native suite: 260 passed. Formatting, all-targets Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass. A separate clean CPython 3.11.2 environment passes all 358 installed-wheel tests with zero skips/failures/errors and exact installed byte matching.
- Linux x86_64 wheel: 17,451,049 bytes; extension: 40,467,960; SHA-256 `2777ca4d601b58d1413d595b7c7d0250a2e596994db75dd4e280e725b5eeb6f2`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Dependencies/upstream pins remain unchanged. Public native layout/display generation precedes observed-result bounds; no preallocation or broad glyph/transform guarantee is implied. REF1/REF2/REF8 remain open beyond this tested profile.

Independent review found repeated target-page scans and native caret resolution could multiply across bookmarks and exported links. A cumulative 2,000,000-unit conservative lookup/scan guard now preflights indexed pieces/lines/clusters and charges repeated internal linked batches. Native exact-limit/+1 tests and a 700-line target with 30 linked labels verify rejection before export, unchanged revision/bytes, and recovery with one linked label. Seven focused native bookmark tests (six DOCX plus the existing PDF test), all eight public DOCX bookmark tests against a copied corrected debug extension, and warnings-denied all-targets Clippy pass. The preceding 260/358 release artifact predates this correction; final corrected exact-head CI remains required.


Independent exact-head CI then exposed a substantive native shaping defect: adding interior markers to the original `Before Target 😀 after` test changed rendered PNG bytes. Direct native probes show split ligatures in `office` and broken Arabic joining in `سلام`; no pixel assertion was relaxed. The supported target profile is now whole paragraphs or empty paragraph-endpoint points. The existing all-story scan enforces this invariant before every staged commit/output, with one logical-offset pass that discounts only bookmark markers. Later prefix/suffix/point insertion fails atomically; whole-target replacement, formatting, own-pair relocation and deletion remain supported. Eight focused native bookmark tests (seven DOCX plus the existing PDF test), nine public workflows against the exact copied corrected debug extension, and all-targets Clippy pass. The original Unicode PNG equality remains exact, the 700-line work test still uses the expensive final caret, and new regressions cover Arabic/ligature/interior refusal, mixed font-size endpoints and unchanged bytes/revision on failure. All preceding release artifacts predate this shaping correction; final exact-head full CI/release evidence remains required. U07 records the upstream marker-transparent shaping requirement.


## Native PDF object-stream output choice — local artifact

- Runtime snapshot `9a32d5a` adds strict keyword-only `object_streams` to editable PDF serialization/save and incorporates both independently reviewed bookmark work-budget corrections. The native full writer remains authoritative; no dependency, revision, runtime license or conversion-matrix change. PDF8 remains partial.
- Three native groups and five public workflows verify actual object/xref representation, repair-free reopen, text/fonts/images/links/outlines/metadata, actual PNG equality, fields/flattening, imported rotations/CropBox/UserUnit, copy/cache stability, strict values, callback ordering, postallocation limits and atomic/short-write save behavior.
- Local locked suite: 264 Rust tests and 364 clean installed-wheel Python tests, zero skips/failures/errors. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass. Installed bytes match the named wheel.
- Linux x86_64 / CPython 3.11.2 wheel: 17,458,225 bytes; extension: 40,485,816; SHA-256 `7a74e56e164a4542de0ee8ba9e681802e5c96ab3ee131c5948dd6a1473e4d60b`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Local evidence is `dist/pdf-object-stream-reviewed/validation.json` and `/opt/pdfobjectstreamreviewedtest`; earlier provisional object-stream artifacts are superseded for local validation.
- **Merge remains blocked by the prerequisite bookmark PR's CI rendering failure.** CI on that branch detects changed PNG pixels immediately after inserting a bookmark in the Unicode fixture; this local artifact passes the same assertion but does not resolve the environment-dependent failure. Independent investigation and corrected exact-head/current-main CI are required. The work-budget fixes address a separate resource issue.

## Owned header/footer clearing — focused integration evidence

- Runtime snapshot `280a44d` integrates native set_story clearing through the existing EditableStory handle. It keeps section references/settings, invalidates only removed children, preserves exact no-op cache/range behavior, and reclaims text/cell/media resources through existing staged validation. Native insertion accounting excludes image-marker bytes only; field/note/bookmark markers remain charged.
- Isolated handoff validation passed 122 DOCX native tests, all-targets Clippy and 56 related public Python tests against the matching debug extension/facade. Three new native groups and four new public workflows include six paragraph/table/image-first header/footer subcases, actual DOCX/PDF pixels, independent copies/variants, shared resources, no-op versus authored-empty behavior, stale children, outgoing links, protected-anchor rollback and exact field/cell limits.
- Root integration separately passes formatting, all three new native groups and all-targets warnings-denied Clippy. Roadmap/support/cached-reference checks pass. No dependency, pin, attribution or license graph changes.
- This is focused development evidence, not a new clean release-wheel/full-corpus certificate. The prerequisite bookmark CI shaping failure remains open; integrated full validation and independent review are still required before merge. DOC5/LAY3 remain partial, and linkage, general block adoption/movement and note removal remain unavailable.

## Bounded PDF text search — focused integration evidence

- Runtime snapshot `f433314` adds search through the retained PDF session and existing read/editable page handles. Native matching, line rectangles and displayed-page coordinate transforms remain authoritative. No dependency, revision, license or conversion-matrix changes. E2 remains partial; normalized text offsets and Office geometry search are not claimed.
- The isolated final implementation passed eight native groups, warnings-denied all-targets Clippy, formatting and 23 related public Python tests against its matching debug extension/facade (ten new search workflows). Root integration separately passes formatting and roadmap/support/cached-reference checks; full combined native/release/installed-wheel and strict typing validation remain pending.
- Native evidence covers every Unicode scalar's conservative fold accounting, source/query byte and scalar limits, exactly 10,000/+1 matches and line rectangles, one million/+1 selected-glyph visits, index/range/finite ordered geometry failures and retained-session parser/raster statistics. Four rotated CropBox/UserUnit ligature fixtures retain actual pixels and independently projected rectangles. Public cases add ActualText replacement, mirrored x/y text, native cross-column order, clipped/invisible text, read/editable lifetime, cache invalidation/copy, callback-resistant queries and page-index ownership checks.
- This is focused development evidence, not a new release artifact or complete-corpus certificate. The prerequisite bookmark CI shaping correction and independent review remain required before merge. Native search bounds follow text interpretation, and no process-memory ceiling, tight-ink guarantee or normalized-table ordering is implied.


## PDF output choice with corrected bookmark shaping — replacement artifact

- Runtime `1280428` includes the reviewed central bookmark endpoint invariant and preserves exact Unicode/Arabic/ligature PNG controls while rejecting unsafe interior targets. This artifact supersedes the earlier object-stream artifacts for local validation; it does not claim interior bookmark support.
- All 265 locked native tests and 365 clean installed-wheel Python tests pass, with zero skips/failures/errors and exact installed byte matching. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass. Prerequisite exact-head CI and independent manual review remain the merge gates.
- Linux x86_64 / CPython 3.11.2 wheel: 17,458,809 bytes; extension: 40,486,744; SHA-256 `5a1b1a24c261c5c983305ff6264c141ceeade378d27f91aa2522961fc9f1d9ef`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/pdf-output-safe/validation.json`, `/opt/pdfoutputsafetest`. Upstream pins, dependencies and broad PDF8/REF criteria remain unchanged.


## Owned header/footer clearing — complete local artifact

- Runtime `0493379` integrates story clearing with the reviewed bookmark resource/shaping fixes. All 268 locked native tests and 369 clean installed-wheel Python tests pass, zero skips/failures/errors, with exact installed-byte matching. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass.
- Linux x86_64 / CPython 3.11.2 wheel: 17,472,507 bytes; extension: 40,514,440; SHA-256 `52aa84308041fea19fb983df35d5c362e298e446de616aee343769d10f498efd`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/story-clear-safe/validation.json` and `/opt/storyclearsafetest`. No upstream pin, dependency or conversion-matrix changes.
- This supersedes the focused-only clearing evidence above for local acceptance. Independent exact-head/current-main CI and manual review remain required before merge. General block movement/adoption, linkage and note deletion remain unavailable; DOC5/LAY3 remain partial.


## Bounded PDF search — complete corrected local artifact

- Runtime `063b193` includes the reviewed bookmark shaping/resource fixes and rejects oversized string subclasses before copying or encoding them. All 276 locked Rust tests and 379 clean installed-wheel Python tests pass, zero skips/failures/errors, with exact installed-byte matching. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass.
- Linux x86_64 / CPython 3.11.2 wheel: 17,474,546 bytes; extension: 40,533,864; SHA-256 `e13f0f4ed6ba48fa0e83ef13447dfc707f3bbfc663ea300e8789e8a5dd4cbb48`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/pdf-search-reviewed/validation.json` and `/opt/pdfsearchreviewedtest`. No dependency, upstream revision or conversion-matrix changes.
- This supersedes the focused-only search evidence for local acceptance. Native matching and line geometry remain authoritative, with documented source-order, ligature/ActualText and invisible/clipped-text semantics. Interpretation precedes search-specific budgets; tight ink, normalized-text offsets and Office geometry are not certified. E2 remains partial. Independent review and exact-head/current-main CI remain required before manual merge.


## PDF page callback ownership — focused evidence

- Independent reproduction against the reviewed PDF search wheel shows a coordinate callback moving page zero to one causes the previous facade to draw on the other page. Color and crop callbacks similarly retarget background/box changes; crop also incorrectly refreshed the stale handle onto the replacement page. Native rotation conversion attempted reentry while its mutable owner was borrowed.
- The six-method Python correction normalizes inputs before a final existing lifetime/generation check. Seven new public groups exercise 16 callback routes; 35 focused tests pass against a copied installed release extension and corrected facade. Move/delete cases compare post-callback bytes and actual pixels; callback exceptions and copy-only operations retain their semantics. Rendering probes use a captured snapshot and do not reproduce a wrong-target render; no rendering code changes.
- No Rust, dependency, pin, attribution, public API or conversion-matrix change. Complete clean-wheel validation and independent manual review remain pending; this focused evidence is not a new release artifact.


## PDF page callback ownership — complete local wheel

- Runtime `9758561` passes all 386 clean installed-wheel Python tests with zero skips/failures/errors and exact installed-byte matching. Strict installed-package typing and roadmap/support/cached-reference checks pass. Rust is unchanged from the 276-test PDF search runtime; its locked native/fmt/Clippy results and unchanged 264-package license audit remain applicable. The release build used the committed lockfile offline.
- Linux x86_64 / CPython 3.11.2 wheel: 17,474,607 bytes; extension: 40,533,864; SHA-256 `ad89b38cbd9b199b38ec691b2ae3a6ed7e000ba6f7c596a55d67f42e49711fef`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/pdf-page-ownership/validation.json` and `/opt/pdfpageownershiptest`. No upstream revision, dependency or conversion-matrix change.
- This supersedes focused-only ownership evidence for local acceptance. Independent review and exact-head/current-main CI remain required before manual merge; broad PDF criteria are unchanged.


## Native decimal outlines — focused integration evidence

- The isolated implementation passes 128 DOCX native tests, all-targets warnings-denied Clippy and 28 related public tests against a matching copied debug extension/facade. Four new native groups and seven public workflows cover exact effective-definition identity, Num.start_overrides continuation, restart/copy, all nine levels at start 32767 in LTR/RTL, empty/wrapped items, scope/section guards, work limits and atomic failures/recovery.
- Independent root review added rejection of non-finite interior caret values (which floating min/max can otherwise ignore), finite body-edge sums, once-per-operation canonical definitions and actual native override-scan accounting. Malformed-layout regressions and decoded source/PDF label-only ink controls pass; no broad font-ink guarantee is inferred.
- Native ListKind::Legal remains responsible for all numbering and layout. No dependency, pin, license or conversion-matrix change. Complete integrated locked native/release/installed-wheel validation and independent review remain pending. TXT5 remains partial.


## Native decimal outlines — complete local artifact

- Runtime `bea5ea9` includes lazy matching of only referenced Legal numbering IDs, avoiding repeated full-definition matching for unused restart definitions. All 280 locked Rust tests and 393 clean installed-wheel Python tests pass, zero skips/failures/errors, with exact installed-byte matching. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass.
- Linux x86_64 / CPython 3.11.2 wheel: 17,484,671 bytes; extension: 40,562,760; SHA-256 `12279825f40795d8ff4147c9af19ab3825e799384c997d3f10c7cc27569e0b02`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/docx-decimal-outline/validation.json` and `/opt/docxdecimaloutlinetest`. No upstream revision, dependency or conversion-matrix change.
- The corpus retains all-nine-level maximum-start LTR/RTL decoded label ink controls, source numbering identities, ordinary list compatibility, scope/geometry/budget refusal and recovery. This supersedes focused-only evidence for local acceptance. Independent review and exact-head/current-main CI remain required before manual merge; broader TXT5 custom definitions and cell/auxiliary placement remain open.


## Stable presentation callback targets — focused evidence

- Reproduction against the reviewed release confirms slide background callbacks can reorder slides and paint the other slide. Distinct global shape IDs prevent wrong-shape corruption, but six shape mutation routes incorrectly retain the earlier slide ordinal and reject a surviving target. The correction performs initial identity validation and resolves the stable slide ID after argument normalization.
- Five new public groups and 31 related tests pass against a copied reviewed native extension and corrected facade. Fourteen same/different-position reorder controls verify exact stable IDs, PPTX bytes, source/PDF pixels and unchanged other slides, with additional early/late deletion, closure, errors, copies and appended-run revision/color/bold checks. Inherited checks also accept ordinary pictures and tables during geometry/reorder probes.
- Separate range probes confirm native final revision checks already prevent formatting after callback text edits or slide reorders. Already-stale ranges still invoke a color callback before refusing; the two-line early revision guard now rejects that case without copying text or adding a native API. Four new cross-format range groups and 27 related tests pass against the matching copied release extension/facade. No Rust, dependency, pin or conversion-matrix change. Full wheel evidence and independent review remain pending.


## Presentation/range callback ownership — complete local wheel

- Runtime `a94a5ba` integrates the reviewed decimal-outline lazy matcher with the Python presentation/range correction. All 402 clean installed-wheel Python tests pass, zero skips/failures/errors, with exact installed-byte matching. Strict installed-package typing and roadmap/support/cached-reference checks pass. Rust remains identical to the 280-test decimal-outline runtime; its locked native/fmt/Clippy results and unchanged 264-package license audit remain applicable. Release packaging used the committed lockfile offline.
- Linux x86_64 / CPython 3.11.2 wheel: 17,484,800 bytes; extension: 40,562,760; SHA-256 `56fec6ff5490415b8230dbf22dd391890061ca5e04fb8f9d7f65734cfb07290f`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/pptx-shape-ownership/validation.json` and `/opt/pptxshapeownershiptest`. No upstream revision, dependency or conversion-matrix change.
- This supersedes focused-only evidence for local acceptance. Independent review and exact-head/current-main CI remain required before manual merge. Stable slide/shape IDs and native range revision checks retain their distinct semantics; no broad parity criterion is newly completed.


## Conditional table vertical borders — focused correction evidence

- Read-only audit confirms per-edge border mappings already exist on table/cell/style formatting, so no new surface is needed. Pinned native conditional region mapping stores vertical edges on per-cell left/right, but interior selection reads cleared inside_v. Installed-wheel probes show first-row inside-vertical and interior-facing column/corner borders serialize yet produce zero red pixels in both DOCX and PDF, while direct/outer controls remain visible. Explicit nil suppression is also lost.
- The 31-line staged parser guard rejects 14 affected region/side combinations, including explicit nil, and preserves None resets and direct/whole-table controls. Two native groups, warnings-denied all-targets Clippy, debug build and 18 related public tests pass in isolation (three new public workflows). A committed raw-native test bypasses the guard and retains source/readback, placed-rule and actual PNG/PDF evidence; public tests cover failed compound updates, ownership/ranges/caches/copies and inherited/explicit border controls.
- No dependency, revision, API or conversion-matrix change. Complete integrated artifact and independent review remain pending. TBL7 stays open; the correction does not provide patterned shading or full shared-edge conflict resolution.


## Conditional table vertical borders — complete local artifact

- Runtime `1cef0262d2a73c0aab2d5fbe4b838ee858e2f732` passes all 282 locked Rust tests and 405 clean installed-wheel Python tests, zero skips/failures/errors, with exact installed-byte matching. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass.
- Linux x86_64 / CPython 3.11.2 wheel: 17,485,323 bytes; extension: 40,563,912; SHA-256 `2411b9c817d58c9d8be1aea9b1dd44526d6c83a5a917e37c8545100b0a37cadb`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/docx-conditional-borders/validation.json` and `/opt/docxconditionalborderstest`. No upstream revision, dependency or conversion-matrix change.
- This supersedes focused-only correction evidence for local acceptance. The raw native regression retains both dropped visible borders and failed nil suppression, while supported direct/whole-table controls remain visible. Independent review and exact-head/current-main CI remain required before manual merge; U02/TBL7 remain open.


## Owned PDF field tooltips — focused integration evidence

- The isolated implementation passes three native groups, all-targets warnings-denied Clippy, debug build and 13 related public field/geometry tests (four new workflows). Native tests verify exact Field state except tooltip, TU source roundtrip/removal and raw appearance equality; public controls cover every supported kind across four rotated CropBox/UserUnit profiles, exact metadata-only PNG equality, copies, later geometry/value edits, paired flatten controls and stale/deleted/closed handles.
- UTF-8 limit/+1, malformed Unicode/NUL/types, hostile string subclasses, aggregate exact 4 MiB/+1 live metadata and mutation-budget refusal preserve bytes/state/cache. Synthetic aggregate fixtures remain bounded to the existing 1,000 owned-field cap. No imported-form expansion or new container is introduced.
- Native set_props redraws the owned appearance even for tooltip metadata. The existing peak helper is shared with box edits, and actual generated AP bytes plus metadata are charged before commit. No dependency/pin/license or conversion-matrix change. Integrated full artifact and independent review remain pending; PDF6 remains partial.


## Owned PDF field tooltips — complete local artifact

- Runtime `b3917b84379a356f2b7a00d36b757d5bf66d4550` passes all 285 locked Rust tests and 409 clean installed-wheel Python tests, zero skips/failures/errors, with exact installed-byte matching. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass.
- Linux x86_64 / CPython 3.11.2 wheel: 17,487,665 bytes; extension: 40,569,352; SHA-256 `850d56884901018551cecea6beb1e741023ab4c01f444c286c539acee1c20f37`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/pdf-field-tooltips/validation.json` and `/opt/pdffieldtooltipstest`. No upstream revision, dependency or conversion-matrix change.
- This supersedes focused-only tooltip evidence for local acceptance. The complete corpus retains exact metadata-only rendering/appearance controls, owned-field operations, transformed pages and atomic input/aggregate/resource failures. Independent review and exact-head/current-main CI remain required before manual merge; PDF6 remains partial and imported forms, radio/widget lifecycle and replies are not certified.


## Incremental navigation, lifecycle and output evidence — documentation audit

The operation ledger now has 16 profiles and still 139 coverage joins, 14 workstream owners and 44 immutable reference families. Five additions cover bounded DOCX bookmark navigation, owned header/footer clear, PDF object-stream output, native PDF search and DOCX decimal outlines. Existing IDs/owners/checkboxes are unchanged; unsupported vertical-redaction evidence remains last for the checker fixture. Generated reference descriptions remove stale absence claims without declaring broader feature completion. Same-format output is preservation, not conversion; authored bookmark lookup is not a general imported source inventory.

Every new evidence cell cites the already completed runtime `a94a5ba65d332e5cc897cb19fd368d771b911d0b` artifact in **Presentation/range callback ownership — complete local wheel** above: 402 clean installed-wheel Python tests, unchanged 280-test native runtime, wheel SHA-256 `56fec6ff5490415b8230dbf22dd391890061ca5e04fb8f9d7f65734cfb07290f`. The five test modules were read and fingerprinted after reviewing their assertions and fixture provenance; their bytes exactly match that runtime commit. Existing historical evidence remains linked to its original artifact.

Isolated documentation validation on the 1e28d49 source baseline passed 14 support-matrix tests, 5 reference-inventory tests, generated-file checks, all 17 cached reference-source hashes/member citations and the 139-ID roadmap checker. The 35 relevant existing behavioral tests passed again against `/opt/pptxshapeownershiptest` in 10.879 seconds, with tests loaded from the isolated snapshot and `doc_ex` from the installed wheel. No runtime/native source, dependency, pin or wheel changed; no Cargo build, new release certification or complete-corpus rerun is claimed by this evidence-only update.



## DOCX shading and page-border source audit

- Documentation-only audit against runtime `b3917b84379a356f2b7a00d36b757d5bf66d4550` and pinned WordCraft `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`. Exact model/reader/writer/layout references are recorded in the existing [upstream register](../../../docs/upstream-roadmap.md#redaction-and-table-fidelity-audit). No runtime, dependency, ownership ID or completion checkbox changes.
- Existing `test_shading_inheritance.py` and `TableStyleTests.test_style_and_cell_fills_survive_native_render_and_pdf_export` already exercise direct/style solid fills, reset and separate page color with DOCX/PDF pixels. This audit reads that evidence; it does not rerun or expand it. Direct table/row-exception fill resolution, richer fill identity and explicit transparent cancellation remain native gaps under TBL7/BG7.
- The page-border offset mismatch is source inspection only: layout measures from page edges, writer emits text-relative offsets and reader retains no offset mode. No page-border reproduction, cross-application comparison, PNG/PDF acceptance or new wheel/build is claimed. The next gate is the raw native asymmetric-margin fixture described in design; TBL7/BG7 and U02 remain open.
- Documentation checks pass: roadmap ownership/DAG (139 IDs/14 workstreams), support evidence (11 profiles/139 joins), cached reference inventory (44 families), 14 support-ledger tests and five reference-inventory tests. These checks validate documentation structure only; no behavioral tests or Cargo commands were run for this audit.

Root integration additionally validates the current 16-profile ledger and unchanged 139-ID/44-family joins; no runtime or behavioral evidence changes.


### Annotation editing locks — isolated focused evidence (2026-10-20)

Base `fe23c89`; no dependency or upstream revision change. The isolated implementation adds `Annotation.locked` for seven existing comment kinds through public native `set_locked`, a reference-only page-array/canonical target preflight and a staged flag-only dictionary postcondition. Native `set_info` also checks Locked, so author changes are inhibited; comment text remains editable. Links are excluded because their separate native setter does not enforce the flag. This is not access control or a general imported-inventory safety claim.

Rust 1.95 locked/offline focused `annotation_lock` tests: **3 passed**, 285 filtered, 1.38 s. All-target locked/offline Clippy with warnings denied and formatting passed; a debug extension was built. Public tests ran against that isolated extension and matching copied facade: **15 passed** in 29.257 s (`test_annotation_lock`, unchanged `test_pdf_geometric_annotations`, unchanged `test_pdf_markup`), including **4 new groups**. They cover all seven kinds across four rotations with CropBox/UserUnit, actual annotation ink, exact PNG preservation, saved native roundtrips, independent copies, text/blocked compound edits, stale/closed handles, strict bool without callbacks, canonical imported acceptance and malformed/unsupported rollback with warm caches. Native tests additionally compare the complete target dictionary and raw appearance bytes, verify note popup removal after unlock, exercise 10,000/+1 reference counts, 1,024/+1 quadrilateral bounds, encoded metadata maxima, noncanonical flags/arrays and mutation/input-budget failures.

Native `Object::as_int` accepts finite integral Reals. The new mutation deliberately requires direct `Object::Int` canonical flags, with a regression rejecting `/F 4.0`; existing inventory remains permissive. Native lock refusal retains the existing `conversion_failure` → `DocExError` mapping.

Consumer typing assertions cover the bool getter/setter and existing str PdfField.tooltip identity/getter/setter. Strict mypy passed against a separate environment containing the copied matching package/debug extension; it is not release-wheel installation evidence. The parent will repeat the check against the final installed wheel. Documentation append context was rebased onto08ae30d; native/facade sources remain the validated fe23c89 baseline plus this patch.

Full integrated locked tests, release wheel, installed-wheel corpus and independent review remain pending. Existing global annotation summaries and initial inspection can allocate before adapter caps; this slice bounds only the new target mutation and does not close PDF6/U10.


## Annotation editing locks — complete local artifact

- Runtime `a21fc5c80abe556ac03b471f1906a7ec6b6a3f16` passes all 288 locked Rust tests and 413 clean installed-wheel Python tests, zero skips/failures/errors, with exact installed-byte matching. Formatting, all-targets warnings-denied Clippy, strict installed-package typing, roadmap/support/cached-reference checks and the unchanged 264-package license audit pass.
- Linux x86_64 / CPython 3.11.2 wheel: 17,495,963 bytes; extension: 40,586,536; SHA-256 `2c54fbb1b41472fd5a07e7ba8e9019c91bdfa125eebc45c4bb770203cb759d5f`; lock SHA-256 `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Evidence: `dist/pdf-annotation-lock/validation.json` and `/opt/pdfannotationlocktest`. No upstream revision, dependency or conversion-matrix change.
- This supersedes focused-only lock evidence for local acceptance. All seven comment kinds retain tested appearances and pixels across transformed pages; native lock refusals preserve staged state. Inventory/initial-inspection limits, foreign profiles, links and reply-thread semantics remain outside this mutation profile. Independent review and exact-head/current-main CI remain required before manual merge; PDF6/U10 stay open.


## Independent manual merge evidence through PR 75

GitHub merge state was verified on 2026-10-20. Each change passed independent review, required fixes and fresh exact-head/current-main CI before manual merge; repository auto-merge remains disabled. These gates complete the corresponding delivery tasks, not broader roadmap criteria. Historical artifact sections above retain their original validation scope.

| PR | Merge commit | Final CI Rust / installed Python |
|---|---|---|
| [64](https://github.com/narain1/pydoccraft/pull/64) | `fd6b1b1f6fafd04dea520ca7ea9ce3d4888b818d` | 262 / 360 |
| [65](https://github.com/narain1/pydoccraft/pull/65) | `de2b0b44c3479108157da1d5e04f70edbe480997` | 265 / 365 |
| [67](https://github.com/narain1/pydoccraft/pull/67) | `9accfa2a667c6521cf6501c3603ec0a0cb6c092d` | 268 / 369 |
| [68](https://github.com/narain1/pydoccraft/pull/68) | `b733c6ab41c44728920e97e6ba3b31e782b36aaa` | 276 / 379 |
| [69](https://github.com/narain1/pydoccraft/pull/69) | `e59de48ce0c13c5ade87a626d25f7e2e74dc122a` | 276 / 379 |
| [70](https://github.com/narain1/pydoccraft/pull/70) | `c99ba86f2c341c1229f6a24e5436cf98dae73c14` | 276 / 386 |
| [71](https://github.com/narain1/pydoccraft/pull/71) | `e402268f5bc33f756f34587594761c41308a2a1e` | 280 / 393 |
| [72](https://github.com/narain1/pydoccraft/pull/72) | `d0a8ec6de3e72f2472d411a7fe9577f638af8ad2` | 280 / 402 |
| [73](https://github.com/narain1/pydoccraft/pull/73) | `0a47b4b001c52b1fa21ab40829d502620022c37e` | 282 / 405 |
| [74](https://github.com/narain1/pydoccraft/pull/74) | `57fcc94216e58d74b967b24cf2ced68f9f45b742` | 285 / 409 |
| [75](https://github.com/narain1/pydoccraft/pull/75) | `c34ef0bc722963ed6f0213da9934f52c361167ba` | 285 / 409 |
| [76](https://github.com/narain1/pydoccraft/pull/76) | `65d9535e87d502af92d1c252bc06c7e8ab76b7d9` | 285 / 409 |

Formatting, warnings-denied Clippy, strict installed-package typing and the unchanged 264-package runtime/build license audit passed in each final CI run. PR69 changes source-audit documentation/data only.


## Field-tooltip and comment-lock evidence — documentation audit

The current operation ledger contains **17 profiles / 119 operation cells**, with the same139 coverage joins,14 workstream owners and44 immutable reference families. `pdf.owned-fields` gains tooltip read/edit/preservation/render evidence while retaining its historical creation artifact and unsupported imported/conversion scope. The new `pdf.comment-locks` profile covers only flag mutation; it does not replace or widen `pdf.geometric-comments` creation/style evidence. Unsupported vertical redaction remains the final profile. No criterion completion flags or ownership IDs change.

Tooltip cells cite runtime `b3917b84379a356f2b7a00d36b757d5bf66d4550`, the completed409-test artifact above (wheel SHA-256 `850d56884901018551cecea6beb1e741023ab4c01f444c286c539acee1c20f37`). Lock cells cite runtime `a21fc5c80abe556ac03b471f1906a7ec6b6a3f16`, the completed413-test artifact above (wheel SHA-256 `2c54fbb1b41472fd5a07e7ba8e9019c91bdfa125eebc45c4bb770203cb759d5f`). New module fingerprints were reviewed against actual assertions and exact source snapshots: tooltip `569ecc1cfbc33a6a22f32fe456639a7c2560ff11c0af1cecfcc9b22bf56f28f4`; lock `b10af9191b773aac61be358a7fe3123e581628a8a636f2756b082c3373efd3d9`. Existing field/geometry test fingerprints also match the409 artifact. Native field-tooltip source roundtrip evidence is not promoted to imported Python inventory; lock imported scope is the dedicated canonical rectangle fixture plus saved/reopened native seven-kind outputs.

Isolated documentation checks on the6573ea0 context passed **14 support-matrix tests and5 reference-inventory tests**, generated-byte checks, all17 cached immutable source hashes/member-line checks and the139-ID roadmap validator. The two added reference members use already fingerprinted PyMuPDF sources: `field_label` at widget.rst100 and `set_flags` at annot.rst301, with local default/subset differences explicit. Four tooltip behavioral tests passed again against their exact installed409-test wheel in0.168s; four lock tests passed against their exact installed413-test wheel in8.526s. Tests loaded from the isolated snapshot; `doc_ex` resolved to the respective installed environment, not a source package.

This documentation-only followup changes no runtime, native source, dependency, pin or artifact. It does not rerun the complete corpus or certify broader form/annotation/security/imported-inventory behavior. API1/API7/QA1/QA8 and remaining PDF6 scope stay open; independent evidence review remains required.


## Owned note conversion — focused isolated evidence

- Isolated base `8c0c8643c0277d879a29bfc3af8ba2d0d2adf2db`: five new native groups, formatting, all-targets warnings-denied Clippy and a debug extension build pass. Six new public workflows pass against the copied extension/facade; the related notes/fields/stories/grid/media/background/bookmark/authoring/section suite passes 64 tests. This is focused integration evidence, not a release-wheel or complete-corpus certification.
- Raw native placement changes a short footnote from page 0 near y=324 points with marker 3 to an endnote on page 1 near y=62 with marker 7. A 45-paragraph note continues without missing/duplicate text across three footnote pages or two endnote pages. Public 144-DPI DOCX/PDF pixels verify actual note position and multi-page colored ink, together with source/readback counters, standard/custom styles, nested anchors, copies and reference identity.
- A raw native regression preserves the shared-standard-run defect: converting one footnote restyles ordinary text and an unconverted endnote reference from red to blue in PNG/PDF. The narrow preflight guard rejects mixed text/objects/unconverted markers, permits converted-marker-only runs, and has public custom-style recovery. Exact two-million work-bound/+1, malformed/missing/duplicate/unrelated anchor rejection, unsafe field destinations and closed/bounded literal input cases remain atomic; successful no-ops preserve revision/ranges/cache.
- No dependency, upstream revision, license or conversion-matrix change. Complete integrated artifact and independent review remain pending; LAY7 remains partial.


## Owned note conversion — complete local artifact

Runtime commit `c2e64845c52dcc72204b13a58c725f3f3810e349` passed **293 Rust tests and 419 clean installed-wheel Python tests**, with zero failures, errors or skips. The Python corpus ran in 113.655 seconds under Python 3.11.2 / Rust 1.95 on Linux x86_64 (glibc 2.36); loaded package files were byte-compared with the built wheel. Formatting, locked/offline warnings-denied all-target Clippy, release build, strict installed-package typing and license audit passed. The complete corpus reopens advertised outputs and verifies extraction, rendering and conversion across PDF/DOCX/PPTX. Independent source review found no blocking note-conversion defect.

Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, **17,506,182 bytes**; uncompressed native extension **40,605,992 bytes**. Wheel SHA-256: `01ca47af4beb9a6e0f01b75a4fca5e7158cfe610260f339cb14ebc97787554bc`. Cargo.lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`; the audited normal/build graph remains 264 packages with no prohibited linked application/runtime crates or changed attribution. All PdfCraft/WordCraft/DeckCraft/craft-libs revisions remain those pinned above.

This local artifact retains the original decimal-outline assertion, which passes here. It does **not** resolve prerequisite PR71's current-main CI raster discrepancy; that investigation and independent final current-main CI remain merge gates for this stacked branch. LAY7 deletion/adoption and broader parity remain open.


## Owned PDF field read-only state — isolated focused validation

On 2026-10-20, an isolated source snapshot based on `d0ce053` passed three new native test groups, formatting, locked/offline all-targets warnings-denied Clippy and a debug extension build with Rust 1.95. The matching copied Python facade/extension passed 17 focused tests: four new read-only groups plus existing field, tooltip and geometry regressions. Strict mypy consumer checks passed against a temporary environment containing that same package, including bool getter/setter assertions. These are focused development results, not a new installed release artifact or a complete-corpus certification.

Fixtures cover text, checkbox and choice fields across all four rotations with nonzero CropBox/UserUnit geometry; exact before/after pixels; native saved-source flags, values, options and tooltip; complete field equality except READ_ONLY; and unchanged regenerated appearance contents. They verify native refusal of identical, different and clear-value assignments, recovery after clearing the flag, independent copy, allowed tooltip/box edits, deletion, flattening, page organization, strict bool/lifetime behavior and atomic resource rejection. Unsupported additional flags remain rejected. Flatten comparisons use a matching flag-cleared control so the existing native transformed-page rounding behavior is not broadened into a pixel-perfect conversion claim.

The existing public native `FieldProps.read_only`/`set_props` path regenerates appearances, including repeated assignments; references, constant allocation and cache-preserving no-ops are not promised. Existing owned-field/model/text, peak appearance and actual raw appearance accounting remain in force. Read-only is native field editing behavior, not security or encryption. Imported AcroForms, multiwidget/radio forms and broader PDF6/U10 capabilities remain outside this slice. No dependency, upstream revision, conversion or license-graph change. Complete integrated locked tests, release wheel, clean installed corpus and independent review remain pending.


## Owned PDF field read-only state — complete local artifact

Runtime commit `6af0b8d313fd585dcdedf138aad27f3205b76873` passed **296 Rust tests and 423 clean installed-wheel Python tests**, zero failures/errors/skips. The Python corpus ran in 121.134 seconds on Linux x86_64 / Python 3.11.2 / Rust 1.95; installed facade/native bytes match the wheel. Formatting, locked/offline all-target warnings-denied Clippy, release build, strict installed-package typing and the unchanged 264-package license audit pass. The complete corpus verifies PDF/DOCX/PPTX extraction, PNG rendering and reopened converted outputs, including the new read-only workflows.

Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: **17,507,974 bytes**, native extension **40,610,984 bytes**, SHA-256 `52d36a6006b504e2de511846845e77705abe5228db89ac6472c6fbe8f65466c4`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. PdfCraft/WordCraft/DeckCraft/craft-libs revisions remain pinned above; no dependency, attribution or prohibited-runtime change.

This historical local artifact uses the original decimal-outline test. PR71's independent diagnostic review has identified one-level coverage quantization and is validating a corrected spatial comparison separately; this artifact does not replace its current-main CI gate. Independent review and fresh current-main CI remain required for this branch. Broader PDF6 form/imported/widget work stays open.


## Slide background inheritance reset — isolated focused evidence

On 2026-10-20, isolated base `c2e6484` passed three new native groups (293 filtered), formatting, locked/offline all-targets warnings-denied Clippy and a matching debug extension build. Twelve focused Python tests pass in13.873s: three new reset workflows plus existing slide/picture callback and transform regressions. Strict consumer typing passes against a temporary environment containing the matching copied package, checking the explicit str-or-None getter and None/color setters. This is development evidence, not a new release artifact or complete installed corpus.

Native fixtures verify theme, master, layout, gradient and picture inheritance through source readback, exact before/after PNG and exact before/after exported-PDF PNG. Getter None remains explicit absence rather than an effective color. The gradient fixture's native samples are [250,0,62,255] and [60,0,250,255]; an initial linear-byte color assumption was corrected to assert actual direction/opacity while preserving exact reset comparisons. Native source-call arity and a mypy post-assignment narrowing expectation were corrected in test code before final checks; runtime behavior needed no change from those failures.

Shared picture/other-slide media survives reset and is reclaimed after its last supported reference disappears. Raw explicit-picture plus inherited master/layout picture fixtures demonstrate the authored-only pruning limitation and now fail atomically before reset; solid-override removal retains inherited assets without pruning. Revision-limit, invalid-page, closed/deleted, color-callback reorder/deletion, cache/range invalidation, copied ownership, actual source p:bg removal and retained PDF text/links are exercised. This does not certify imported theme/master-shape asset graphs or new theme/layout editing.

No dependency, upstream revision, license graph, conversion pair or hidden-slide policy changes. Native PDF conversion already skips hidden slides by default; that pre-existing behavior is not changed or newly certified here. Complete integrated release/installed validation and independent review remain pending; broader SLIDE4/E3 coverage stays open.


## Slide background reset — complete local artifact

Runtime commit `58c1eaa53f5b37bb167958d8d6a059c2c9603a7c` passed **299 Rust tests and 426 clean installed-wheel Python tests**, zero failures/errors/skips. The Python corpus ran in 114.75 seconds on Linux x86_64 / Python 3.11.2 / Rust 1.95. Installed facade/native files match the wheel exactly. Formatting, locked/offline all-target warnings-denied Clippy, release build, strict installed-package typing and the unchanged 264-package license audit pass. The full corpus verifies extraction, PNG rendering and reopened conversions for PDF/DOCX/PPTX; native reset controls compare real inherited appearance before/after and retain authored media and identity limits.

Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: **17,508,811 bytes**; native extension **40,614,440 bytes**; SHA-256 `e084a6f29a4346eaeeaf67ca151475d1a4d35c4e2d8cdaf9ca40d47a5fd980c5`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. PdfCraft/WordCraft/DeckCraft/craft-libs revisions remain pinned above; no dependency/attribution/prohibited-runtime changes.

This local artifact retains the original decimal-outline test and does not supersede PR71's separate corrected-test/current-main CI validation. Final independent branch review and current-main CI remain required. Reset restores native inheritance; it does not imply arbitrary imported theme/master editing, complete referenced-media accounting or transparent output. SLIDE4 remains partial.


## Recent roadmap overlap and stale-wording audit

Documentation-only review on 2026-10-20 distinguishes delivered bounded operations from broad partial criteria. TODO now identifies existing hidden-slide metadata/fixed links/plain notes, native page/count fields, note conversion and field/comment properties without proposing duplicate managers. Slide insertion at a position composes existing append/move; slide duplication and imported graph remapping remain open. The workstream guide removes its obsolete138-open count and labels original agent assignments as historical rather than current capability claims.

BG5 now joins the existing word.story-clear profile already used by DOC5/LAY3; its remainder no longer incorrectly lists owned clear as absent. This does not broaden that profile's historical artifact scope. All17 profile definitions,119 operation cells, artifact references, fixture fingerprints and source-audit baseline remain unchanged, including the pending decimal-outline evidence. All139 criterion IDs and their46 checked flags,14 owners and dependency JSON are unchanged. Newly implemented notes/read-only/reset behavior is not automatically promoted into historical operation certifications.

The isolated documentation snapshot passes14 support-ledger tests, exact generated-matrix consistency, the139-ID/14-workstream ownership validator and the cached44-family reference-source check. No Cargo, behavioral corpus, new wheel or upstream source changes are involved. Broad E3/PDF6/SLIDE/API/QA criteria retain their prior partial/open status.


## Decimal-outline raster comparison — independent CI correction

Final-main CI runs `39503201911` and diagnostic-only `39504027755` exposed an overstrict aggregate ink-count assertion in the nine-level/max-start fixture. At LTR 144 DPI, source and PDF ink bounds were exactly `(145, 662, 150, 548)`, but thresholded counts were 5481 versus 5482. Full-image diagnostics found 140 changed pixels with maximum RGBA-channel difference 1: exactly one pixel at `(267, 248)` crossed from `(128,128,128,255)` to `(127,127,127,255)`. Independent repeated local probes also found deterministic one-channel differences while their aggregate counts happened to match. This is renderer coverage quantization, not evidence of a layout or shaping change.

The corrected expectation retains exact image dimensions, exact thresholded ink bounds, visible ink, direction placement and all nine source numbering labels. It additionally compares every source/PDF RGBA channel with maximum difference 1, replacing aggregate count equality with spatially stronger evidence. No runtime implementation, native guard or dependency changed. Earlier local 393-test and downstream artifact results retain their original test-file scope; fresh corrected-test installed-wheel and final current-main CI validation remain required before merge.


## Decimal-outline raster comparison — corrected CI artifact

- Corrected final-main head `d5912b9aad74520b13a9753e6b6ee4b27c2caa3d` passes [CI 39504970072](https://github.com/narain1/pydoccraft/actions/runs/39504970072): all 280 locked Rust tests and 393 installed-wheel Python tests, zero failures/errors/skips, formatting, warnings-denied Clippy, strict typing and the unchanged 264-package audit. Independent corrected-test verification against the copied local release also passes all 393 tests (79.069s). PR71 was manually merged as `e402268f5bc33f756f34587594761c41308a2a1e`; auto-merge remains disabled.
- CI CPython 3.11.17, Linux x86_64/glibc 2.39: wheel 17,485,056 bytes; extension 40,565,584 bytes; wheel SHA-256 `016327f2fbb8f0a55634dfb3ed9e700e6be92e4a548358a1e23e03801a6b3816`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`; upstream revisions and dependencies are unchanged. Installed package bytes match the wheel. Evidence was downloaded to `/tmp/pr71-final-ci-evidence/validation.json`.
- Only the decimal-outline profile is superseded by this exact corrected-test artifact and test-source fingerprint. The other four recently added profiles retain their original 402-test artifact and scope; later local artifacts with the old outline assertion remain historical. Exact dimensions, thresholded ink bounds, direction and all nine source labels remain checked, together with every RGBA channel within one value of the other native renderer. The measured CI 128-to-127 threshold crossing and independent review rationale are recorded separately. No runtime behavior or broad criterion completion changes.

Root synchronization also ran all seven corrected decimal-outline tests against the exact installed 426-test slide-reset wheel in 7.863 seconds; all passed. This is a focused rerun with the corrected test source, not a replacement full-corpus certification for that historical wheel. No package bytes or native runtime changed.


## Field-state, note-kind and slide-reset evidence — documentation followup

On 2026-10-20, the operation ledger expands to 19 profiles/133 cells with unchanged 139 criteria, 46 checked items and 14 workstream owners. Existing owned-fields read/edit/preserve/render cells add strict read_only behavior using runtime 6af0b8d's completed 423-test artifact (SHA-256 52d36a6006b504e2de511846845e77705abe5228db89ac6472c6fbe8f65466c4); its historical creation cell and unsupported imported/format-export scopes remain unchanged. The word.note-kind-conversion profile cites runtime c2e6484's 419-test artifact (SHA-256 01ca47af4beb9a6e0f01b75a4fca5e7158cfe610260f339cb14ebc97787554bc), classifying Notes.convert as semantic editing and separately describing the existing DOCX-to-PDF output assertions. The slides.background-reset profile cites runtime 58c1eaa's 426-test artifact (SHA-256 e084a6f29a4346eaeeaf67ca151475d1a4d35c4e2d8cdaf9ca40d47a5fd980c5), with explicit getter/inherited-media scope. These are exact historical artifacts, not a new runtime certification.

Reviewed new module fingerprints match their respective runtime commits: test_field_read_only.py `f3055f423c0f98cd9eb02bfc21927a7b251bed81da2764c4d765bb2d36108cb7`; test_note_conversion.py `4d9fbcb9330d703c8643e42fdcbce9d72126d0a16ecf3df91533f771a281dbd0`; test_slide_background_reset.py `a76f9249da5695435353315417936c88c3a4ebf209ec41a7bf20e24af1c1f7f1`. Existing field/geometry/tooltip module hashes also match 423's source. All unrelated profile definitions, historical artifact references and fingerprints are retained, including the independently corrected decimal-outline CI profile; unsupported redaction remains last.

Reference joins reuse the existing notes/presentation/forms families. Only representative Widget.field_flags at cached immutable widget.rst line 110 is added, bringing the inventory to 198 members across 44 families; this does not imply arbitrary flags, multiwidget synchronization or security parity. New coverage joins reuse LAY7/E3 and SLIDE4; the prior BG5/story-clear connection remains intact.

The isolated documentation snapshot passes 14 support-ledger plus 5 reference-inventory tests, exact generated-byte checks, the 139-ID/14-owner roadmap validator and cached immutable source/member checks. The three new property suites were rerun with tests loaded from the isolated snapshot and doc_ex from their exact installed artifact environments: 4 read-only tests in 0.213 s on 423, 6 note-conversion tests in 12.452 s on 419, and 3 background-reset tests in 0.106 s on 426, all passing. No Cargo, new wheel, dependency or source-pin change. Broader API1/API7/QA1/QA8 and format acceptance remain open; independent evidence review is still required.

PR72 final head `9588c1c54de9d0dd819fa8e096bbd4120abf2a04` passed [current-main CI 39505927934](https://github.com/narain1/pydoccraft/actions/runs/39505927934) after integrating the independently reviewed outline raster expectation. The runtime and Cargo graph were unchanged by that integration; append-only validation history and the callback target-scope correction were retained.


## Native character spacing — isolated acceptance pending integration

The isolated DOCX letter_spacing profile reuses native optional character spacing with [-100, 100] point bounds and 0.05 pt precision; no method/handle/dependency is added. Three native groups cover all 4,001 integer-twip values through native write/read, native cluster/source/PDF ink and default/style/table-region precedence plus rollback. Six public workflows pass, and a 60-test related suite passes in 43.836 seconds against the copied debug extension/facade. Formatting, all-targets warnings-denied Clippy, debug compilation and a pinned mypy 1.18.2 strict copied-package consumer pass. Root's complete locked release/clean installed-wheel artifact and independent current-main review remain required.

Raw condensed-text probes found negative native advances but no source/PDF ink mismatch beyond one channel unit. Large negative spacing intentionally overlaps/reverses glyph positions; inferred PDF extraction order/spaces differs independently of exact source Unicode. No unsupported-negative guard was added from a monotonicity heuristic. In the combined nested-table/note fixture, source/PDF grayscale grid pixels differed by 109 at 72 DPI, with exactly the same 4,530-pixel count of differences after all character spacing was removed. Therefore rich-context acceptance compares exact chromatic text bounds and every chromatic text channel, while isolated text retains full-image channel comparisons. These observations do not broaden table-stroke fidelity or typography completion claims.


## Native character spacing — complete local artifact

Runtime commit `d8333dceda02962880e472d60928251cbc6a7f77` passed **302 Rust tests and 432 clean installed-wheel Python tests**, zero failures/errors/skips. The Python corpus ran in 122.144 seconds on Linux x86_64 / Python 3.11.2 / Rust 1.95. Installed facade/native files match the wheel exactly. Formatting, locked/offline all-target warnings-denied Clippy, release build, strict installed-package typing and the unchanged 264-package license audit pass. The full corpus verifies PDF/DOCX/PPTX extraction, PNG rendering and reopened conversions, including the independently corrected outline raster test.

Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: **17,517,771 bytes**; native extension **40,616,104 bytes**; SHA-256 `6d2e1a54c6e065917499d7102d50b21a96c15b643f2c64d224e8df1ea8fa5768`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. All upstream revisions remain pinned above; dependencies, attribution and the prohibited-runtime audit are unchanged.

Spacing is native shaping/writing through the existing formatting patch. The rich-context chromatic-text comparisons do not certify grayscale table-rule parity; extreme condensation can reverse physical glyph positions and inferred PDF reading order. Final independent review and exact-head/current-main CI remain required. TXT6 stays partial.

PR73 final head `c5cbc8bf8a8d156e9bb438df91ff0eb9080ac1e4` passed [current-main CI 39506841575](https://github.com/narain1/pydoccraft/actions/runs/39506841575): 282 Rust / 405 installed-wheel Python tests and all required checks. Independent review cleared the conditional-border correction, and PR73 was manually merged as `0a47b4b001c52b1fa21ab40829d502620022c37e`. Auto-merge remains disabled; native conditional-edge selection and broader TBL7 remain open.


## PPTX character spacing — isolated acceptance pending integration

The isolated same-key extension adds no dependency or public method. Five native groups pass: all 4,001 accepted values roundtrip through native hundredth-point storage; None/0 inheritance and native autofit scaling; shape/cell rollback and generation/resource failures; 15 Latin/combining/Arabic raster fixtures including -100/+100 boundaries; and independent invisible PDF glyph-origin displacement checks. Source-reopen native pixels are exact for the raster fixtures, and PNG/PDF whole-image channels differ by at most one at 200 DPI. Whole-pixel page dimensions avoid conflating existing source round-versus-PDF ceil dimension quantization with character-spacing fidelity. Extracted PDF glyph displacements for ABC at 0/+2/-1/-100/+100 agree with native placed origins within 0.002 point; no local text-placement repair is used.

All-targets warnings-denied Clippy, formatting and debug compilation pass. Five new public workflows pass in 29.101 seconds against the isolated copied debug package; 33 related DOCX/slide/range/ownership/table/navigation regressions pass in 23.067 seconds. The strict copied-package typing consumer also passes. They cover source properties/reset/append, Unicode visible width changes and output, invalid patches/copies/caches, merged cell links/reordering and revision-bound callback protection. The prior DOCX-only negative PPTX test is intentionally removed; the original 432-test artifact remains historical, and this feature's expected net public suite grows by four to 436 before independent later corrections. Full integrated release/clean installed-wheel validation and independent review remain required. This does not claim arbitrary imported editing, typography completeness or identical PDF text inference for overlapping/spread glyphs.


## Shared slide character spacing — complete local artifact

Runtime commit `c5a67d033ddbdb8b1e42971b814bb8cfc433be45` passed **307 Rust tests and 436 clean installed-wheel Python tests**, zero failures/errors/skips. The Python corpus ran in 137.488 seconds on Linux x86_64 / Python 3.11.2 / Rust 1.95. Installed facade/native bytes match the wheel. Formatting, locked/offline all-target warnings-denied Clippy, release build, strict installed-package typing and the unchanged 264-package license audit pass. Native spacing tests include visible source/PDF channels and independent invisible PDF glyph displacement. The full corpus verifies extraction, PNG rendering and reopened conversions across all three formats.

Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: **17,509,816 bytes**; native extension **40,616,840 bytes**; SHA-256 `2fa1a551c6d5f0b36c29160f020f2a839e2a0e4d2135a79ac9c6dbdb9e8a9c39`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`. Upstream revisions, dependencies and attribution remain unchanged.

The original 432-test DOCX artifact retains its historical rejection-test scope. This artifact deliberately replaces that obsolete PPTX rejection method with five capability workflows, a net increase of four Python tests. Shared spacing uses the existing key; native 200-DPI raster and text-layer evidence does not imply arbitrary-DPI pixel identity or stable inferred reading order for overlapping glyphs. Final independent review and exact-head/current-main CI remain required; SLIDE3/TXT6 stay partial.

PR84 independent source review and all 432 tests against its copied frozen release package passed in 126.169 seconds, with strict typing and matching wheel hash. This is independent confirmation of the recorded DOCX artifact; its final main integration/CI is still pending.

PR74 final head `60a568fbc8e04e0e15e048ed3f0ee7dad3a570db` passed [current-main CI 39507569770](https://github.com/narain1/pydoccraft/actions/runs/39507569770): 285 Rust / 409 installed-wheel Python tests and all required checks. It was manually merged as `57fcc94216e58d74b967b24cf2ced68f9f45b742` after independent review. Auto-merge remains disabled; broader PDF6 stays open.


## Slide-cell append lifetime — isolated Python correction

On 2026-10-20, an unmodified /opt/slidebackgroundresettest installed release reproduced the preexisting issue: close the presentation, then call cell.paragraphs[0].add_run("x", font=Font("Inter")) where Font.__len__ raises. The caller RuntimeError occurred before DocumentClosedError. The narrow facade correction checks an existing native alignment lookup first, without copying the paragraph text, then preserves final append validation after normalization.

Three new test groups pass in 0.024 seconds against an isolated copy of that installed native release plus the corrected facade. They cover already-closed/deleted/stale body and grid targets without callbacks; reordering two slides with same-position tables while retaining the original logical cell; and live callback deletion/reset/close/raising with no append beyond callback effects. Ordinary source output and cached PNG bytes remain unchanged after a raising callback. No Cargo, native code, dependency or pin change. These are focused correction results, not a new full release artifact or a blanket callback-order guarantee for other methods.


## Slide-cell append lifetime — complete local artifact

Facade commit `f49d8c69511a0ba811703121974c12a80a0fb146` passes **439 clean installed-wheel Python tests**, zero failures/errors/skips, in 141.697 seconds on Linux x86_64 / Python 3.11.2. The native extension is byte-identical to the preceding c5a67d0 artifact that passed all **307 locked Rust tests**, formatting and warnings-denied Clippy: extension SHA-256 `caf0984cd1be54394e9ecedc8d12c8c756ab0d89aefcf4ff8ec230c130924f37`. The Rust suite is applicable unchanged evidence, not a newly claimed rerun. Release packaging and strict installed-package typing pass; installed package bytes match the wheel.

Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: **17,509,869 bytes**; native extension **40,616,840 bytes**; SHA-256 `438a5c25ae873258e4247c01abc7b773395de705c675f661cb83b2cee05c7bee`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`; pinned revisions and the audited 264-package license graph remain unchanged. No native dependency or attribution changes.

The full corpus checks all three formats, actual rendering/reopened conversions and the three new callback/lifetime workflows. This is a narrow early-check correction for PPTX cell add_run; other methods do not acquire a blanket error-order guarantee. Final independent review and exact-head/current-main CI remain required before manual merge.


## Owned note content clearing — isolated implementation evidence

Implementation based on d8333dc, spec76; complete integrated release/installed-wheel artifact is pending. Three new focused native groups passed using the locked Rust1.95 container build, followed by formatting, all-targets Clippy and a copied debug extension. Five new public workflows plus existing story-clear, note, note-conversion and page-field modules passed: 23 tests. An added blue FootnoteText/red EndnoteText own-mark PNG/PDF assertion then passed in a separate focused run. These checks are isolated evidence, not a release or broad DOC5/LAY7 completion claim.

Native checks retain part identity/body options, external anchors, canonical style and exactly one writer-generated own mark after clear; repeated and clear→convert→clear calls leave revisions unchanged. They check copied/exact near-limit charged text/field accounting, synthetic placeholder reuse/monotonic IDs, shared media and raw native note-cell reclamation, wrong/missing/closed identities, and full-state rollback for unexpected own/foreign note, bookmark and comment anchors. Native-only rich-content fixtures test cleanup internals; public rich note insertion remains unsupported.

Public tests verify long footnotes/endnotes actually span pages before clear and return to one page after clear, while copies retain every paragraph; warmed render caches invalidate only on actual changes. Source XML keeps original note IDs/options and emits one own mark, erased text disappears from extracted note stories and rendered PDF, untouched second notes keep their next counter, and distinct note styles produce actual blue/red own-mark pixels in native PNG and PDF. Existing header/footer clear, conversion guards and field scope tests remain passing. The previous public assertion that Note.clear is unsupported was deliberately replaced with the newly supported content-clear behavior; semantic note deletion remains unsupported. No dependencies, pins, license files, local format algorithms or new public method names changed.


PR75 final head `88a27cc7d8f5e843245ca8303edc1a5a24a70364` passed [current-main CI 39508256843](https://github.com/narain1/pydoccraft/actions/runs/39508256843), including 285 Rust / 409 installed-wheel Python tests and all required checks. Independent review retained the corrected 393-test outline artifact separately from the other four historical 402-test profiles and cleared the documentation update. PR75 was manually merged as `c34ef0bc722963ed6f0213da9934f52c361167ba`; no runtime/Cargo change or broad completion flag was introduced.

PR85 independent source review and all 436 tests against its copied frozen package pass in 143.308 seconds, with strict typing and matching wheel hash. Final main integration/CI remains queued; this independent run does not replace the exact local artifact evidence above.


## Owned note content clear — artifact evidence

Runtime `fecebc36f13a2e7ae7912f4e6ca28a25400f6124` passed **310 locked Rust tests and 444 clean installed-wheel Python tests**, zero failures/errors/skips. The Python corpus ran in 150.483 seconds on Linux x86_64 / Python 3.11.2 / Rust 1.95. Formatting, warnings-denied all-target Clippy, release build, strict installed-package typing and the unchanged 264-package license audit pass. Installed facade/native bytes match the built wheel. The corpus includes all three formats, actual PNG output and reopened conversions, plus the intentional Note.clear contract replacement.

Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: **17,520,377 bytes**; extension **40,620,456 bytes**; SHA-256 `73ab05afb724346031146a0eaf2d3f6c9b63cabf31330ead8540a1b446af60d8`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`; all upstream revisions, dependencies and attribution remain unchanged.

The existing word.story-clear profile reuses this exact artifact for its edit-authored/preserve/render/export cells, retaining the header/footer assertions and adding bounded note clearing. The reviewed test_story_clear fingerprint is `ea43d861092700d6763636b622d56b261da0803c2d79916d0d8c0a688349d2ce`; new test_note_clear fingerprint is `c1db35d457f3a637cf6e9e85d7faad86e49614796702224e0870d51264e5c10c`. The formerly unsupported note assertion is deliberately replaced, not silently certified against the old wheel. Historical ownership-2026-10-20 references/sections and every unrelated profile remain unchanged. This keeps 19 profiles/133 operation cells, 139 criteria, 46 checked flags, 14 owners and 44 reference families/198 members. Existing LAY7 and note-family joins reuse the same story-clear profile. Generated guides, cached immutable references and the full corpus’s 19 evidence checks pass.

Clearing retains the reference/kind/options and native number mark, not zero-height removal or semantic note deletion. Native-only media/cell cleanup tests do not expose public rich-note insertion. Independent review and exact-head/current-main CI remain required; broader LAY7/DOC5 are partial.

PR86 independent source review and all 439 tests against its copied frozen package pass in 145.448 seconds, with strict typing and matching wheel hash. Its extension was independently confirmed byte-identical to PR85; final main integration/CI remains queued.


## PDF integer raster preallocation — focused isolated validation

Source code is based on c5a67d033ddbdb8b1e42971b814bb8cfc433be45; these documentation additions preserve the subsequent Note.clear head fecebc36f13a2e7ae7912f4e6ca28a25400f6124 and its prior artifact history. No upstream pin, dependency, runtime API or attribution changed. This is focused validation, not a new complete installed-wheel artifact.

The pinned public native scale/rounding helpers prove that an 8000.5 by 7999.5 point view at 72 DPI passes continuous f32 area limits but rounds to 8001 by 8000 pixels (64,008,000). The adapter now applies its existing integer-pixel/side checks before the common full-page native render call, retaining native reject_oversize. No oversized raster was allocated in the regression. Original synthetic empty PDFs use offset CropBox, distinct MediaBox, UserUnit 2 and all four rotations. Tests warm a small cached raster, prove oversized requests leave native render/raster counters unchanged, and accept tiny clips at 72/1200 DPI. A thin 8192-by-1 control checks the native side boundary safely; exact 8000-by-8000 acceptance is arithmetic-only.

Rust 1.95 locked/offline formatting, two new native groups (0.03 seconds), warnings-denied all-targets Clippy and debug compilation pass. Two new public workflows pass against the isolated copied debug extension (0.080 seconds). Twenty-six related all-format rendering, geometry/clipping, retained sessions and PDF-box tests pass (32.641 seconds), including fractional DPI, blank pages, invalid DPI/index/lifetime cases and visible pixel controls. Full integrated release/clean installed-wheel validation and independent exact-head review remain required.

Broad R4/U09 remains open: pinned DeckCraft's caught-panic path returns a correctly sized blank image through the selected public renderer, and no per-call fallible alternative was found. This correction does not certify parsing/layout/intermediate allocations, total process memory or hostile-input execution time. Existing 64-million-pixel output limits and clipped-region bounds remain unchanged.


## PDF integer raster preallocation — complete local artifact

Runtime `6c27539c9b0ffefaaf05ae1f62f5d422c567c829` passed **312 locked Rust tests and 446 clean installed-wheel Python tests**, zero failures/errors/skips. The Python corpus ran in 149.041 seconds on Linux x86_64 / Python 3.11.2 / Rust 1.95. Formatting, warnings-denied all-target Clippy, locked/offline release build, strict installed-package typing and the unchanged 264-package license audit pass. Installed facade/native bytes match the wheel. The corpus checks extraction, PNG rendering and reopened conversions across PDF, DOCX and PPTX.

Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: **17,520,699 bytes**; extension **40,620,200 bytes**; SHA-256 `2f65675cb156e74d163c950d718ba15a9a8aa29e818b37b3463b9e20be78d0e8`. Lock SHA-256 remains `4c984b3524c75b77e651cf9d201c432dd1b6baa35ca3c206100e680bd403b7f0`; all upstream revisions, dependencies and attribution remain unchanged.

The new boundary tests reject rounded oversized full-page rasters before native rendering and preserve valid tiny clipped requests. They do not allocate the oversized raster or certify total process memory. Broad R4/U09 remains open for the documented upstream fallible-rendering gap. Independent review and exact-head/current-main CI remain required before manual merge.

PR87 independent source review and all 444 tests against its copied frozen package pass in 149.329 seconds, with strict typing, evidence checks and matching wheel hash. The canonical mark/style semantics and narrowly migrated four story-clear evidence cells were reviewed; final main integration/CI remains queued.

PR76 final head `5f5503c05788db0f26588582737fac30e5b19420` passed [current-main CI 39509341862](https://github.com/narain1/pydoccraft/actions/runs/39509341862), including 285 Rust / 409 installed-wheel Python tests and all required checks. Independent review cleared its source-backed shading/border gap documentation, and it was manually merged as `65d9535e87d502af92d1c252bc06c7e8ab76b7d9`. Auto-merge remains disabled; broader TBL7/BG7 remains open.


## PDF field creation ownership — focused isolated correction

A supported text-field call reproduced retargeting against /opt/slidecellappendlifetimetest (the reviewed 439-test release): a float-subclass coordinate moved original page zero to one, but page.fields.add reused its earlier zero ordinal and created the field on the unrelated page. The original page handle correctly reported itself stale afterward, establishing the missing final check.

The isolated facade correction preserves initial validation, normalizes options/coordinates, then validates the original page again before native creation. Three new public test groups pass against a copied 439-release extension (0.020 seconds); 22 related field, geometry, tooltip, read-only and PDF drawing ownership tests pass (3.435 seconds). Coordinate and options-iterator callbacks cover move/delete/close and exceptions; already-invalid handles do not evaluate inputs. Both pages' post-callback bytes, text and small cached PNGs remain unchanged by rejected outer creation, and independent copies leave valid original targets usable. Only Python code/tests/docs change; there is no Cargo build, dependency or pin change for this correction. Full integrated release packaging/installed-wheel validation and independent review remain required.
