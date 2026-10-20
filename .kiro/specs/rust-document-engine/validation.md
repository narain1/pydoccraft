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
