# Implementation Plan

- [x] 1. Change the package and source license to Apache-2.0 and document attribution obligations.
- [x] 2. Replace direct PDF parsing with pinned PdfCraft extraction, image, link, and rendering APIs.
- [x] 3. Replace direct DOCX parsing with pinned WordCraft document-model traversal, layout, rendering, and PDF export.
- [x] 4. Replace direct PPTX parsing with pinned DeckCraft presentation-model traversal, rendering, and PDF export.
- [x] 5. Replace handwritten conversion with upstream PDF→DOCX, DOCX→PDF, and PPTX→PDF implementations.
- [x] 6. Add Python rendering methods while preserving the existing `open`, `Document`, `Page`, and Markdown APIs.
- [x] 7. Remove direct parser dependencies and retain only the small normalization/adaptation layer.
- [x] 8. Add extraction, media, link, table, note/comment, render, conversion, malformed-input, and lifecycle tests.
- [x] 9. Generate and review the locked runtime license inventory; reject non-permissive dependencies.
- [x] 10. Run Rust formatting/tests/Clippy, Python tests from an installed wheel, conversion reopen tests, and report wheel/native sizes and remaining upstream limitations.

## Feature backlog

The original checklist above records the engine implementation. Track subsequent feature work in the [feature roadmap](../../../TODO.md), with these stable IDs:

| Priority | IDs | Work | Status |
|---|---|---|---|
| P0 | R1 | PDF/DOCX/PPTX rendering at a requested DPI | Implemented; source and existing tests reviewed |
| P0 | R3 | Consistent PNG metadata at requested DPI | Verified in unreleased installed wheel |
| P0 | R2, R4 | Dimensions, visible output and limit validation | Partial; broader geometry corpus and upstream failure reporting remain |
| P1 | C1–C5 | Authoring contract, DOCX/PDF creation, reports, templates | Initial native creation/edit/save profile verified; broader criteria remain open |
| P1 | E1–E6 | Structured tables, source locators, Office semantics, PDF links, diagnostics, Markdown assets | E1 DOCX/PPTX recursive cells/spans and unambiguous E4 PDF link offsets verified; E6 portable media export is verified; broader E1 and E2–E5 remain open |
| P2 | P1–P4 | Parse/layout reuse, region rendering, PDF assembly, bounded extraction | Basic native PDF assembly and bounded parse/layout reuse verified; complete preservation and broader performance criteria remain open |

Completion criteria live with each roadmap item. Update requirements/design and record installed-wheel evidence in `validation.md` as features are implemented; existing rendering signature tests do not complete R2–R4.

## Expanded Open XML feature coverage and Python authoring

These groups remain open. W01 adds a coverage validator and W14 adds an installed-wheel evidence harness; neither completes full feature parity. They refine the original creation/extraction milestones; implementation must use the selected ArtCraft crates. The [API proposal](../../../docs/authoring-api-design.md) includes draft workflows and the source crosswalk.

| Group | Scope | Milestone |
|---|---|---|
| API | Feature audit, Python objects, units, ownership, errors and usability | M0 |
| DOC / TXT / STY | Lifecycle, save/edit/copy, rich text, lists, styles and fonts | M1–M2 |
| TBL | Construction, cells, merges, sizes, borders, fills and pagination | M1–M2 |
| BG / LAY | Backgrounds, watermarks, sections, headers/footers, notes and layout | M1–M3 |
| MED | Images, drawings, groups, charts and advanced objects | M1–M3 |
| REF / FORM | Links, fields, navigation, templates and content controls | M2–M3 |
| REV | Comments, revision tracking and accept/reject operations | M2–M3 |
| PKG | Safe preservation, compatibility, relationships and serialization | All milestones |
| PDF / SLIDE | Backend-specific PDF/PPTX authoring with consistent Python conventions | M4 |
| QA | Per-operation evidence, corpus, installed wheels and support matrices | All milestones |

Detailed checkboxes and dependencies live in [TODO.md](../../../TODO.md). Priorities are scheduling choices, not assertions of upstream availability. A feature blocked upstream stays open with its reason; no alternative backend is authorized. Programmatic editing is in scope; desktop UI/editor-state crates remain excluded.

## Canonical GitHub roadmap units

Use [W01–W14](../../../docs/roadmap-workstreams.md) as the planning issues, not one ticket per coverage checkbox. [roadmap-workstreams.json](../../../docs/roadmap-workstreams.json) maps all 139 IDs exactly once. Recipe/milestone/test IDs remain traceable under their owner and do not create additional API implementations.

The [upstream register U01–U11](../../../docs/upstream-roadmap.md) marks library ownership and evidence separately. Build on already available public writers/models; contribute missing reusable format behavior upstream. Contributions are only marked here; upstream publication is a separate action.

## Native authoring baseline (unreleased)

- [x] Add typed Python creation, explicit native serialization, atomic path saves, binary-stream handling, independent copies and close semantics (DOC3/DOC4).
- [x] Verify the initial DOCX paragraph/range/table/nested-cell/image/header/footer profile using WordCraft public model/writer APIs.
- [x] Verify the initial PDF creation, conservative source-edit, page organization/import, annotation and link profile using PdfCraft public APIs.
- [x] Verify the initial PPTX text/table/image/basic-shape/background/metadata/slide-organization profile using DeckCraft public models and writers.
- [x] Audit 263 normal/build packages with the checked license-expression validator; add the pinned lower-level `pdfcraft-organize` crate and direct existing `pdfcraft-annot` dependency without upgrading upstream revisions.
- [x] Run Rust formatting, 37 Rust tests, warnings-denied Clippy and 61 installed-wheel Python tests; record artifact evidence and limits in validation.md.
- [ ] Complete arbitrary Office preservation, full styles/lists, fields, review and advanced objects; the C2/C3 basic creation criteria are covered by the later text/vector batch. Initial profile tests do not complete those coverage IDs.

## DOCX layout/story integration (unreleased)

- [x] Expose owned auxiliary `Story` values with reused normalized content, explicit part ordering, no guessed page assignment and native note reference connections.
- [x] Extend extraction traversal budgets across auxiliary parts and separators; test nested header tables and oversized grid rejection.
- [x] Expose checked table widths/uniform borders/row layout plus paragraph spacing and explicit page breaks; verify save/reopen and PDF pagination.
- [x] Verify solid DOCX page-color pixels in native PNG and exported PDF rendering (BG1).
- [x] Validate the combined 41-Rust/68-installed-Python-test artifact; record remaining scope limits separately.
- [ ] Complete linked headers, richer semantic grid copying and full lists/styles; initial lists and structured omission diagnostics are covered below.

## Text/vector content and diagnostics (unreleased)

- [x] Complete C2 basic DOCX creation with native lists and revision-bound URL links.
- [x] Complete C3 basic PDF creation with supplied-font Unicode text and native vector paths; verify visible glyphs, font rejection and geometry.
- [x] Verify blank DOCX lifecycle (DOC1), native report DOCX/PDF and structured-row recipes (C4/TBL11), plus repeated-table-header PDF rendering (part of TBL6).
- [x] Add native PDF metadata and hierarchical bookmark editing with atomic validation and explicit collection/handle lifetimes.
- [x] Add bounded model-derived DOCX/PPTX omission diagnostics and 128 MiB input/64 MiB serialized-payload guards.
- [x] Correct header/footer read side effects and child bookmark collection self-invalidation; standardize public authoring font_size spelling.
- [x] Verify 63 Rust and 85 installed-wheel Python tests, locked release artifact, formatting, Clippy and unchanged 263-package license graph.
- [ ] Complete full diagnostics/conversion coverage, complex-script font fidelity, advanced lists/styles, selective extraction and all remaining unchecked roadmap criteria.

## Styles, slide ranges and Markdown assets (unreleased)

- [x] Add bounded DOCX named paragraph/character styles, inherited/direct formatting snapshots and explicit reset/false semantics.
- [x] Complete basic character formatting (TXT2), including underline variants, strike/double-strike, scripts, highlight and shading with native round trips.
- [x] Reuse revision-bound ranges for PPTX Unicode text, replacement, formatting and URL/internal navigation; verify native PPTX/PDF output and referenced-target deletion safety.
- [x] Complete E6 with owned in-memory Markdown/media/manifest files, separate stories, shared nested-table HTML rendering and explicit resource limits.
- [x] Improve typed Python factory/collections/formatting discovery and keep canonical font_size/underline spelling across adapters.
- [x] Verify 75 Rust tests, 97 installed-wheel Python tests, formatting, Clippy and the locked artifact with unchanged dependencies/attribution.
- [ ] Complete theme/next-linked-style editing, full PPTX table semantics and remaining source-preservation/advanced criteria.

## Sections, slide tables and template recipes (unreleased)

- [x] Complete supported DOCX section-break profile (LAY2), including blank/parity pages and per-section PNG/PDF geometry.
- [x] Add stable section properties, bounded equal columns and independent header/footer copies; retain explicit unsupported cases.
- [x] Add native PPTX grid CRUD/merge/split, uniform borders, widths/minimum heights, covered-cell semantics and stale positional handles.
- [x] Share direct cell alignment and resolved margins/reset across DOCX/PPTX, with nested-content width checks.
- [x] Complete literal-template and independent batch recipes (FORM1/FORM6) over authored models; verify missing/unused/invalid values, cross-run replacement, links and source isolation.
- [x] Record the existing native DOCX-to-PDF report/table path as PDF4's supported flow route.
- [x] Verify 86 Rust tests, 107 installed-wheel tests, formatting, Clippy and locked artifact; dependencies/revisions remain unchanged.
- [ ] Complete unequal columns, linked stories, source template preservation, richer table formatting and other unchecked criteria.

## DOCX grid and PDF geometry

- [x] Add atomic DOCX row/column insertion/deletion through public native table operations, preserving placeholders, widths, spans and header prefixes.
- [x] Apply per-table structural generations and reject unsafe orphan vertical merges.
- [x] Complete PDF1 with native page-box inspection/reset/edit, rotated geometry and explicit effective-intersection semantics.
- [x] Add RGB PDF text color for built-in and supplied-font paths.
- [x] Reject known PDF content truncation and incomplete editable outline inspection.
- [x] Validate the locked release wheel, reopened outputs and native tests; record evidence.
- [ ] Complete ordered table column views, general rich-row copying and remaining table criteria.

## Notes and owned PDF fields

- [x] Add native footnote/endnote creation with shared editable story paragraphs and section note options.
- [x] Guard destructive generic semantic edits and account for note stories/anchor bytes in budgets.
- [x] Integrate bounded owned PDF text/checkbox/choice fields, strict values and checked native appearances.
- [x] Reuse native field deletion/flatten and validate page-organization postconditions.
- [x] Resolve the single added pinned forms crate and regenerate/review the 264-package permissive license inventory and wheel copy.
- [x] Complete combined locked native and installed-wheel validation and record exact artifact evidence.
- [ ] Complete remaining LAY7 and PDF6 criteria, including semantic deletion/copy, radio groups and safe bounded imported-field APIs; these broad criteria remain open.

## Table formatting and outline-aware imports

- [x] Add bounded count-based table construction and ordered row/column slices over existing cell handles.
- [x] Complete native direct table/cell formatting, content fitting and metadata with geometry/budget checks.
- [x] Complete optional native outline-preserving PDF imports with source/post-copy guards.
- [x] Validate reopened/rendered outputs and installed-wheel behavior; record exact artifact evidence.
- [ ] Complete upstream table fidelity gaps and outline-preserving subsets; broad TBL/PDF5 criteria stay open until their remaining acceptance cases pass.

- [x] Audit merged-through-PR24 coverage and mark C1/E1/API2/API4/API6/API8/API9/TXT7 complete with existing corpus evidence; retain broader incomplete criteria.

- [x] Complete R2 multi-DPI visible-output and P3 reopened assembly link/raster acceptance; complete TBL1/TBL2 count construction and ordered slice/grid access.

## Conditional styles and multiline PDF labels

- [x] Extend native named styles with table kinds, whole-table/conditional formatting, same-kind inheritance and existing table style/look patches.
- [x] Reuse effective native geometry checks for style/base/assignment changes and inherited margins, including nested content.
- [x] Recover unambiguous multiline PDF labels with bounded native provenance, exact offsets and neighboring-link remapping.
- [x] Validate installed-wheel conditional-style rendering/export, multiline links, full regression corpus and locked artifact; record evidence (128 Rust, 138 Python tests).
- [ ] Complete remaining theme/next-linked style semantics and upstream table fidelity gaps without adding redundant API abstractions.

## Native region rendering and DOCX tabs/breaks

- [x] Add optional native clip rendering through existing PNG methods for PDF/DOCX/PPTX with geometry, limits and DPI tests.
- [x] Add bounded DOCX paragraph/style tab stops and explicit line/page/column insertion through existing formatting/ranges; reject verified native decimal/bar failures.
- [x] Verify native placement, inheritance/clear semantics, invalid-edit rollback, full corpus and installed-wheel artifact: 137 Rust and 153 Python tests.
- [x] Promote already-locked kurbo 0.13.1 for the public WordCraft region transform; regenerate/review license inventories, unchanged at 264 packages, and retain all upstream revisions.
- [ ] Complete upstream transparency/background, decimal/bar-tab and other fidelity gaps separately; no silent approximation.

- [x] Complete LAY9 warmed-cache repagination acceptance and audit merged TBL3/TBL5/STY4/PDF5 coverage; retain documented scope and remaining pagination fixtures.

## Style relationships and document defaults

- [x] Add native next/linked relations through existing Style handles with reciprocal updates and distinct inheritance rules.
- [x] Add direct document defaults and bounded patches through Styles, preserving native resolution and validating affected geometry/tabs.
- [x] Verify native DOCX roundtrip, visible inherited/default overrides, atomic failures and installed-wheel corpus; record artifact evidence (143 Rust, 157 installed-wheel Python tests).
- [ ] Retain upstream automatic Enter-style/default-kind fallback gaps and remaining theme/import criteria without adding editor machinery.

## PDF drawing resource preservation

- [x] Guard all drawing entry points against incompatible existing resource replacement, including inherited bindings.
- [x] Verify native collision reproductions, atomic rollback, unchanged original pixels and bounded matching checks.
- [x] Promote the already-locked model crate, review regenerated license inventories and record locked installed-wheel validation.

## DOCX pagination acceptance and overflow

- [x] Complete LAY5 actual-pagination acceptance and fitting/splittable cell continuation fixtures.
- [x] Reject verified oversized unmerged unsplittable-row output through native placed-cell bounds before serialization.
- [x] Validate recovery, later style/content/page changes, existing save destinations, intentional clipping, merges and full installed-wheel corpus; record artifact evidence.
- [ ] Keep complete merged-row overflow and native allocation budgets tracked upstream; TBL6 remains open.

## PDF text areas and image transforms

- [x] Extend existing text/image methods with native bounded areas, alignment, visual crop and quarter-turn/flip properties.
- [x] Bound native wrapping work and preserve logical overflow, resource, glyph and media errors atomically.
- [x] Verify geometry, visible transforms, searchable Unicode, media retention and full installed-wheel artifact; record evidence.
- [ ] Retain automatic supplied-font wrapping and strict text clipping as upstream gaps; PDF2 remains open.

## DOCX paragraph borders

- [x] Reuse existing paragraph/style/default formatting for supported four-edge borders and native resolved snapshots.
- [x] Reject shared DOCX border substitutions/clamps and invalid vertical double/wave edges, preserving table-style meaning.
- [x] Verify native roundtrip, visible body/cell/story colors, inheritance/reset, atomic errors and installed-wheel corpus; record artifact evidence.
- [ ] Track between/bar and remaining shared-edge/style fidelity gaps upstream.

## Ownership and shading consistency

- [x] Prevent pending positional iterator handles from rebinding after PDF page/annotation/outline changes; verify stable-ID and closed-owner behavior.
- [x] Document copy/reparent/slice/iteration ownership and validate the complete installed-wheel corpus for API5.
- [x] Verify BG6 native inherited shading/reset, highlight precedence and independent page backgrounds in DOCX/PDF output.

## Inline picture lifecycle

- [x] Add owned picture bytes/replacement and collection deletion through existing source/ownership conventions.
- [x] Use native media insertion and supported-story reference accounting to retain live shared media and reclaim unused budgets.
- [x] Verify replacement/removal, pixels, serialized media, stale handles, copies, source kinds and atomic failures in the locked installed-wheel corpus.

## DOCX metadata and PDF markup

- [x] Add bounded core/custom property patches using existing metadata names; preserve native types, timestamps, clear semantics and source core extraction.
- [x] Extend existing annotation kinds with native underline/strikeout/squiggly and preflight appearance work before regeneration.
- [x] Verify native/property/pixel/ownership/rollback regressions in the complete locked installed-wheel corpus and record artifact evidence.
- [ ] Retain broader DOC7/PDF6 source-property, form, reply and fidelity gaps as explicit remaining work.

## Section geometry and authored slide layouts

- [x] Extend section patches with native left gutter and optional explicit widths, preserving existing equal-column/reset conventions.
- [x] Validate narrowest-column table/image geometry and native column/page flow without automatic scaling.
- [x] Add four native authored slide layouts through existing add/shape operations, preserving inherited references and effective geometry.
- [x] Complete the locked installed-wheel corpus for both slices and record artifact evidence.
- [ ] Keep top/mirrored gutters, continued-table reflow, general slide copy/import and additional layout/theme fidelity open with upstream evidence.

## Combined table fixtures

- [x] Generate all six TBL12 fixtures and check reopened source/export semantics, spans, logical order and actual visible colors on every page.
- [x] Verify the fixture pack against the frozen installed section/layout wheel and publish its scope/limitations index.

## Presentation pictures and text selection

- [x] Add owned picture source/alt edits and native crop/flip properties through existing typed shape handles.
- [x] Validate deduplicated live media accounting, transforms, shared backgrounds, ownership and atomic limits.
- [x] Add shared contiguous Paragraph/Shape slicing returning existing range types without another native method.
- [x] Run the complete locked installed-wheel suite and record source/PNG/PDF evidence for these slices.

## Numbering and internal navigation extraction

- [x] Project resolved DOCX list definitions and native logical labels through existing block metadata with story/segment identity and bounded accounting.
- [x] Project resolved PPTX internal slide targets through existing links and omission diagnostics, retaining text/table/group provenance.
- [x] Validate both slices in the complete locked installed-wheel corpus and record artifact evidence.
- [ ] Retain broader E3/TXT5 list authoring, native reader preservation and pre-import slide-count gaps until their acceptance cases are complete.

## List starting values and source presentation limits

- [x] Extend existing native list membership with independent starting values and explicit header/footer fidelity errors.
- [x] Reject excessive source slide references before native import, covering alternate content without another OOXML resolver.
- [x] Validate native/read/render/export, malformed/bounded input and full installed-wheel regressions; record artifact and reviewed dependency evidence.
- [ ] Retain custom-level numbering, header/footer counter fidelity and general upstream allocation/reporting gaps.

## Picture transforms and owned field geometry

- [x] Add verified DOCX crop/flip properties and reject silent native sub-point image scaling.
- [x] Make existing owned PDF field boxes writable through native appearance regeneration and atomic geometry/fit checks.
- [x] Verify complete locked/native and installed-wheel read/render/export corpus and record artifact evidence.
- [ ] Retain native DOCX rotation fidelity, floating-image/opacity and broader imported/multi-widget PDF form gaps.


## Private Office render reuse

- [x] Retain bounded native DOCX model/layout and PPTX model from extraction with shared checked rendering and synchronized close.
- [x] Reuse the existing private facade dispatch, release source/password ownership, and close sessions on failed hydration.
- [x] Verify native parse/layout counts, PNG parity, resource boundaries, owned snapshots and close races in the complete installed-wheel corpus.
- [x] Record separate-process synthetic latency/native RSS evidence while retaining broad P1/peak-layout guarantees as open work.


## Page fields and imported DOCX metadata

- [x] Append native decimal page/count fields through Paragraph and add explicit section page-number restart/continuation.
- [x] Enforce field-marker/range ownership, all-story count bounds, cached-source semantics and native render/export acceptance.
- [x] Project bounded strict custom properties and table title/description through existing metadata with factual omission diagnostics.
- [x] Verify the complete locked/native and installed-wheel corpus and record artifact evidence.
- [ ] Retain native nondecimal page-format/clamp and property-reader completeness limitations as upstream work.

## Bounded PDF page selections

- [x] Reuse bounded integer selection for PDF copy/import and reject callback-driven page organization before native dispatch.
- [x] Verify limits, ownership, callback behavior and selected output in the complete installed-wheel corpus; record artifact evidence.

## Executable usability cookbook

- [x] Compose six runnable workflows using the existing public surface and explicit output directories/DPI.
- [x] Audit naming, units, ownership and save/close conventions; distinguish current guides from historical proposals.
- [x] Verify all actual functions/CLIs and reopened rendered outputs in the complete installed-wheel suite; record artifact evidence before completing API10.

## DOCX image backgrounds

- [x] Reuse the background_image property vocabulary for native stretched DOCX PNG backgrounds and independent color reset semantics.
- [x] Extend existing media accounting for the global reference, retaining shared images/fields and explicit bounded/atomic errors.
- [x] Verify complete locked/native and installed-wheel read/render/export coverage and record artifact evidence.
- [ ] Retain aspect-preserving fit/placement, tiling and watermark fidelity as broader open/upstream work.

## Presentation cell paragraphs

- [x] Reuse Cell/Paragraph/range vocabulary and public native text editing for rich PPTX cell paragraphs, formatting and links.
- [x] Validate canonical anchors, body/table generations, native empty-body insertion, budgets, callbacks and explicit unsupported Word members.
- [x] Verify complete locked/native and installed-wheel source/render/export coverage and record artifact evidence.
- [ ] Retain paragraph spacing/indent fidelity, rich nested content and broader SLIDE3/grouped-content work.


## Everyday API typing and handle representations

- [x] Retain concrete owner/handle types for common creation/copy/context and literal text-output chains without changing runtime signatures or exports.
- [x] Bound existing child reprs and make them native-free for stale, closed and private no-ID paragraphs.
- [x] Verify the strict installed-wheel consumer and complete installed corpus; record artifact evidence.
- [ ] Retain broader property annotations, full-package typing and remaining API3 discoverability work.


## Bounded operation evidence ledger

- [x] Seed seven bounded profiles with exact historical/native/fixture references and join all existing roadmap IDs without duplicating ownership or completion state.
- [x] Generate the readable operation matrix and add offline validation of references, evidence drift, optional cells and calendar dates.
- [x] Run development checks and the complete installed-wheel corpus, recording that runtime code and artifact are unchanged.
- [ ] Expand audited profiles and reference-member dispositions before claiming complete API1/API7/QA1/QA8 coverage.


## Pinned reference-family discovery

- [x] Group representative Open XML SDK/PyMuPDF members under existing roadmap and evidence profiles with immutable source provenance.
- [x] Derive ownership/reverse joins and connect the operation matrix without copying tested behavior statuses.
- [x] Verify source hashes/member lines, malformed-reference regressions and complete installed-wheel corpus; record evidence.
- [ ] Retain full member disposition, richer native operation audits and broader API1/QA1 parity work.


## Header/footer rich content

- [x] Reuse story/table/cell/picture handles for native header/footer rich content and direct collections.
- [x] Preserve synthetic anchors, authored empty paragraphs, deleted-picture anchors and independent section/document copies.
- [x] Share native placed-frame validation and disable editor-only story dimming for document PNG output.
- [x] Verify the complete locked/native and clean installed-wheel corpus and record artifact evidence.
- [ ] Retain header/footer linkage, general movement, rich notes and unmeasured inactive-variant geometry as broader open work; owned clear is tracked separately below.


## Bounded native line numbering

- [x] Add Section.line_numbers through existing native section patches and readable restart names.
- [x] Preserve native multi-column and continuous-resume defect repros, rejecting affected authored configurations atomically.
- [x] Preflight label work before both authored full-layout validation paths and share the existing physical-bound validation pass.
- [x] Verify complete locked/native and clean installed-wheel acceptance; record artifact and upstream evidence.
- [ ] Retain multi-column labels, affected continuous page restarts and broader LAY6 numbering fidelity as upstream work.


## Incremental story and numbering evidence

- [x] Add bounded story/line operation cells with distinct artifacts, exact fixture fingerprints and existing roadmap/reference joins.
- [x] Keep unsupported imported edits, unassessed semantic extraction and broader native fidelity gaps explicit.
- [x] Verify cached reference locations, generated guides and installed-wheel development checks; record unchanged runtime evidence.


## Native direction and language formatting

- [x] Reuse existing DOCX format parsers, native properties, inheritance and direct/effective snapshots for direction/language.
- [x] Validate native mixed-script geometry, conditional table-language precedence and explicit unsupported paragraph-region direction.
- [x] Verify complete locked/native and clean installed-wheel corpus plus typed consumers; record artifact evidence.
- [ ] Retain broader complex-script/CJK font controls, missing-glyph diagnostics and TXT6 parity as open work.


## Bounded PDF geometric comments

- [x] Reuse rectangle/ellipse native annotations, existing handles and native geometry/style mutation.
- [x] Bound canonical target dictionaries and generated appearances, charging compound intermediate streams; retain tiny-path native repro.
- [x] Preserve initial closed/stale checks and add final post-normalization checks with callback regressions.
- [x] Verify complete locked/native and clean installed-wheel corpus; record artifact evidence and separate existing inventory limitations.
- [ ] Retain broader annotation/reply/form fidelity and caller-bounded upstream inventories as open work.


## PDF inventory allocation upstream evidence

- [x] Record exact pinned summary/initial-inspection allocation paths and deterministic import-safe synthetic input reproductions.
- [x] Verify fixture offsets, native repeated-reference identity and probe argument bounds against an installed wheel without changing runtime behavior.
- [ ] Add upstream fallible caller-bounded annotation inventories and strict initial inspection results, including preallocation/occurrence budgets and explicit incompleteness errors.
- [ ] Verify native at-limit/+1 and fallback propagation before claiming bounded imported inventory; preserve the separate parser/decompression scope and broader PDF6 work.


## Direction and geometric-comment operation evidence

- [x] Review concrete assertions and native symbols for two bounded profiles; connect exact historical artifacts and existing reference families/criteria.
- [x] Regenerate both development guides, verify cached reference sources and all existing ownership joins, and run the installed-wheel corpus.
- [ ] Retain broader typography, imported annotations/appearance, upstream inventory and full parity audits as open work.


## Owned DOCX bookmarks and internal links

- [x] Reuse named mapping operations, TextRange and range.link with bounded native paired markers and all-story reference validation.
- [x] Preserve generic semantic/media safeguards and exact own-pair deletion/relocation, fresh grid lookup, copies and stale-range behavior.
- [x] Share native layout checks and prove conservative link-batch counting; retain native missing/auxiliary target, clipped hit-area, vertical-link and export-cap regressions.
- [x] Complete locked full native/release/installed-wheel validation and record exact artifact evidence.
- [ ] Retain interior targets pending upstream marker-transparent shaping, general cross-references, imported/multi-paragraph targets, tooltips, heading outlines and arbitrary transformed navigation as open work.


## PDF object-stream output choice

- [x] Reuse native SaveOptions.object_streams through existing PDF serialization methods with strict bool/default compatibility.
- [x] Keep one atomic save implementation and verify both-mode representation, content, pixels, fields, ownership, callbacks and limit behavior.
- [x] Complete the local locked native/release/clean-wheel corpus and record the new artifact.
- [x] Integrate the reviewed bookmark shaping correction and verify the complete local corpus with exact safe-profile PNG assertions.
- [x] Pass independent review and corrected prerequisite/current-main CI before merge.
- [ ] Retain optimization, version/conformance profiles, incremental writing and broader PDF8 preservation as open work.


## Owned header/footer clearing

- [x] Reuse native set_story with strict ownership and conditional no-op/cache behavior.
- [x] Preserve section references/settings and invalidate only removed child handles; reclaim exact text/cell/media usage through existing commit guards.
- [x] Verify focused rich-content, copy, no-op, rollback, semantic-anchor and boundary regressions in isolation.
- [x] Complete integrated locked native/release/installed-wheel validation after the reviewed bookmark corrections.
- [x] Pass independent review and exact current-main CI before merge.
- [ ] Retain general block movement/adoption, note removal, header linkage and broader DOC5/LAY3 parity as open work.


## Bounded native PDF text search

- [x] Reuse native matching/line geometry through existing read/editable page handles and retained sessions.
- [x] Bound query, normalization, hit and geometry work with explicit errors and preserve owner/cache semantics.
- [x] Verify focused native/public geometry, ActualText, ligature, column, Unicode, callback and exact-limit regressions in isolation.
- [x] Complete integrated locked native/release/clean-wheel validation after prerequisite bookmark fixes (276 Rust / 379 Python).
- [x] Complete independent review and exact-head/current-main CI before manual merge.
- [ ] Retain Office search geometry, normalized source offsets and broad E2 grounding parity as open work.

## PDF3 native drawing-style boundary audit

- [x] Recheck pinned public/private placement and style-writing boundaries against existing drawing APIs; retain existing U10/PDF3 ownership.
- [ ] Request typed native path painting/fill rules, collision-safe opacity resources and public native placement; do not extend local PDF operator construction.
- [ ] Verify stroke work/bounds, contour pixels, rotated CropBox/UserUnit geometry, resource preservation and default compatibility before exposing additional drawing keywords. PDF3 remains open.


## PDF mutation ownership after caller normalization

- [x] Reproduce page retargeting through numeric/color/stream/iterable callbacks and reuse existing page-generation checks in six mutation methods.
- [x] Verify focused move/delete/close/error/copy routes, native boolean rejection and integer rotation compatibility against a copied installed extension.
- [x] Complete clean installed-wheel validation (386 tests), strict typing and structural checks.
- [x] Complete independent review/current-main CI before manual merge.


## Native decimal-outline list option

- [x] Reuse Legal numbering through existing paragraph list methods with full-level identity, starts/restarts and bounded continuation lookup.
- [x] Gate top-level body/section scope and reuse shared native layout for bounded logical label geometry, malformed geometry and later output refusal/recovery.
- [x] Verify isolated native/public hierarchy, nine-level maximum-start LTR/RTL ink, source/PDF output, copy/wrapping/callback and compatibility regressions.
- [x] Complete integrated locked native/release/installed-wheel validation (280 Rust / 393 Python).
- [x] Complete independent manual review/current-main CI: corrected 280 Rust / 393 Python head passed; PR71 manually merged as e402268f5bc33f756f34587594761c41308a2a1e.
- [ ] Keep broader custom list definitions, cell/auxiliary outline placement and TXT5 completion open.


## Presentation and range callback ownership

- [x] Reproduce wrong-slide background updates and stale-ordinal shape failures, then resolve stable target IDs after normalization.
- [x] Verify focused stable-ID/control-output, early/late invalidation, copy and appended-range regressions; retain native final checks.
- [x] Complete early stale-range normalization guard and DOCX/PPTX shape/cell controls.
- [x] Complete integrated clean-wheel validation (402 tests), strict typing and structural checks.
- [x] Complete independent review and exact-head/current-main CI: 280 Rust / 402 Python tests; manually merged as d0a8ec6de3e72f2472d411a7fe9577f638af8ad2.


## Conditional table-border fidelity correction

- [x] Audit existing border APIs to avoid duplication; reproduce native conditional interior-vertical loss and nil-suppression loss.
- [x] Reject affected assignments in the existing staged parser; preserve resets and supported direct/whole-table/outer-edge controls.
- [x] Retain raw-native source/readback/placed-rule/PNG/PDF repros and focused public atomicity/inheritance/cache/copy checks.
- [x] Complete integrated locked native/release/installed-wheel validation (282 Rust / 405 Python).
- [x] Complete independent manual review/current-main CI; PR73 merged after 282 Rust / 405 installed-wheel Python tests and all required checks.
- [ ] Keep native conditional-edge selection, patterned shading and full shared-edge conflict resolution under existing U02/TBL7.


## Owned PDF field tooltip metadata

- [x] Reuse native FieldProps.tooltip and existing field handles/payloads with strict bounded Unicode strings and empty clearing.
- [x] Stage complete expected-field comparison and charge regenerated appearances plus tooltip bytes under existing budgets.
- [x] Verify focused native/public roundtrip, all kinds/transformed pages, pixels/copy/resize/value/flatten and atomic limit/lifetime controls.
- [x] Complete integrated locked native/release/installed-wheel validation (285 Rust / 409 Python).
- [x] Complete independent manual review/current-main CI; PR74 manually merged after 285 Rust / 409 installed-wheel Python tests and all required checks.
- [ ] Retain owned radio groups, imported forms and reply identity/appearance/lifecycle as separate existing PDF6/U10 gaps.


## Incremental navigation, lifecycle and output evidence

- [x] Add five scoped profiles with reviewed fingerprints and the exact complete a94a5ba artifact; preserve the seven-operation distinction and unsupported redaction evidence.
- [x] Connect existing roadmap/reference families and remove stale claims that owned clear, bookmarks or native search are absent; do not close broader criteria or duplicate ownership.
- [x] Validate generated guides, offline evidence checks and focused tests against the existing installed wheel; record the documentation-only result without claiming a new release artifact.


## Bounded PDF comment editing locks

- [x] Audit native flag semantics, reference/dictionary clone behavior and link incompatibility; reuse the existing Annotation handle.
- [x] Stage canonical flag-only mutations with bounded borrowed preflight and unchanged-appearance/target postconditions.
- [x] Complete focused native/public fidelity, boundary, rollback and lifetime checks.
- [x] Complete integrated locked native/release/installed-wheel validation (288 Rust / 413 Python).
- [x] Complete independent review and exact-head/current-main CI before manual merge.
- [ ] Keep PDF6/U10 broader imported inventory, reply-thread and annotation capabilities open.


## Field-tooltip and comment-lock evidence followup

- [x] Review actual tooltip/lock assertions and source fingerprints against their exact validated artifacts; retain historical scopes and geometric creation evidence.
- [x] Reuse owned-fields and one orthogonal comment-lock profile with existing PDF6/reference joins and explicit imported limits.
- [x] Validate the 17-profile/119-cell generated guides, cached immutable reference citations and existing offline drift checks.
- [x] Complete independent evidence review; keep broader PDF6/API1/API7/QA1/QA8 acceptance open.


## Native owned-note conversion

- [x] Probe public semantic conversion for stable IDs/paths/styles, source readback, native placement/counters and complete long-note continuation.
- [x] Add Notes.convert through bounded native work and exact conversion-specific anchor validation, preserving no-op ranges/cache and ordinary commit guards.
- [x] Retain raw native shared-reference-run color-loss evidence; reject unsafe mixed runs with custom-style/marker-only recovery controls.
- [x] Run five focused native groups, all-targets Clippy/debug build and six new public workflows within a 64-test related regression suite in isolation.
- [x] Complete integrated locked native/release/clean installed-wheel validation: 293 Rust / 419 Python tests, strict typing and unchanged license audit.
- [x] Complete independent review/current-main CI after resolving prerequisite PR71 raster discrepancy.
- [ ] Retain semantic note deletion and arbitrary anchored-fragment remapping under existing LAY7/U02/U07 ownership.


## Owned PDF field read-only state

- [x] Verify native READ_ONLY/set_props/set_value contracts and reuse existing owned-field accounting and postconditions.
- [x] Permit only READ_ONLY in addition to existing kind flags; preserve imported/multiwidget/script restrictions and native filling refusal.
- [x] Complete focused native/public/typing tests for fidelity, saved flags, allowed edits, recovery and atomic bounds/lifetime errors.
- [x] Complete integrated locked native/release/installed-wheel checks: 296 Rust / 423 Python tests, strict typing and unchanged license audit.
- [x] Complete independent review and fresh current-main CI before manual merge.
- [ ] Keep broader PDF6/U10 field/widget capabilities open.


## Native slide background inheritance reset

- [x] Audit existing hidden/navigation/background overlaps and pinned native inheritance, writer, renderer and PDF paths.
- [x] Extend only Slide.background=None, with explicit getter semantics, stable ownership and bounded authored-media reclamation.
- [x] Complete focused native/public/typing inheritance, output, sharing, rollback and lifetime evidence.
- [x] Complete integrated release/clean installed-wheel validation: 299 Rust / 426 Python tests, strict typing and unchanged license audit.
- [x] Complete independent review and fresh current-main CI before manual merge.
- [ ] Retain broader SLIDE4/master/theme/imported-media preservation work.


## Field-state, note-kind and slide-reset evidence followup

- [x] Audit exact 419/423/426 runtime assertions and module fingerprints; separate note-kind mutation, same-format persistence and format export.
- [x] Extend owned-fields and add two scoped mutation profiles, reusing current criteria/reference families without changing feature completion or unrelated historical profiles.
- [x] Validate 19-profile/133-cell generated guides, cached immutable sources and offline evidence drift checks; preserve independently corrected outline evidence.
- [x] Complete independent evidence review; broader feature/API/QA acceptance remains open.


## Native DOCX character spacing

- [x] Probe native cluster advances, expanded/condensed PNG/PDF output and twip write/read semantics without a local positioning algorithm.
- [x] Reuse CharPatch, snapshots, existing style/default/region dispatch and public typing; retain explicit PPTX rejection and all existing guards.
- [x] Add three focused native groups and six public workflows covering precision, output, scopes, inheritance, callbacks, ownership and atomic failures; run related regressions and strict copied-package typing.
- [x] Complete integrated locked native/release/clean installed-wheel validation: 302 Rust / 432 Python tests, strict typing and unchanged license audit.
- [x] Complete independent review and exact-head/current-main CI before manual merge.
- [ ] Keep broader TXT6 font/shaping controls and diagnostics under their existing owner.


## Shared PPTX character spacing

- [x] Audit public native storage/inheritance/tracking/autofit/PDF paths and reuse the existing format key with the shared bounded precision profile.
- [x] Add five native groups and five public workflows for storage, output, invisible text positions, shape/cell ownership, links and atomic failures; explicitly retire the obsolete negative-PPTX group.
- [x] Complete 33 related Python regressions and strict copied-package typing.
- [x] Complete integrated locked release/installed-wheel validation: 307 Rust / 436 Python tests, strict typing and unchanged license audit.
- [x] Complete independent review/current-main CI; broader SLIDE3/TXT6 acceptance remains open.


## Slide-cell append lifetime checks

- [x] Reproduce the preexisting callback-before-closed-check behavior with the supported font key.
- [x] Add a narrow initial target lookup while retaining final native validation; verify invalid targets, live reorder, deletion/reset/close and caller exceptions.
- [x] Complete integrated installed-wheel regression validation: 439 Python tests and strict typing, with the byte-identical native extension retaining the preceding 307-test validation.
- [x] Complete independent review and exact-head/current-main CI before manual merge.


## Owned note content clearing

- [x] Audit public native set_story, note own-mark writing and canonical style behavior; distinguish clearing from reference deletion.
- [x] Reuse existing Note.clear, ordinary commit checks and staged story-local resource/identity cleanup without changing header/footer policies.
- [x] Add three native groups and five public workflows covering blank/long/converted notes, actual mark/content ink, exact accounting, ownership/no-op/cache and protected-anchor rollback; run related story/note/field regressions.
- [x] Complete integrated locked release/clean installed-wheel validation: 310 Rust / 444 Python tests, strict typing and unchanged license audit; refresh only affected story-clear evidence.
- [x] Complete independent review and exact-head/current-main CI before manual merge.
- [ ] Retain broader DOC5/LAY7 imported note, semantic deletion and fragment-remapping work.


## PDF integer raster preallocation boundary

- [x] Audit all-format output preflight and prove the native continuous-area/integer-rounding gap without a large allocation.
- [x] Reuse native device-pixel rounding in the common PDF full-page renderer; preserve native oversize and region checks.
- [x] Add safe arithmetic, request-counter, view geometry and thin-raster regressions; complete focused native and public checks.
- [x] Complete integrated release/installed-wheel validation: 312 Rust / 446 Python tests, strict typing and unchanged license audit.
- [x] Complete independent review and exact-head/current-main CI before manual merge.
- [ ] Keep R4/U09 open for upstream fallible per-call renderer failures and broader resource guarantees.


## PDF field creation callback ownership

- [x] Reproduce existing coordinate callback retargeting on the reviewed installed 439-test release using supported text-field creation.
- [x] Retain initial validation and add final original-page resolution after options/coordinate normalization.
- [x] Validate coordinate/iterator reordering, deletion, close, early-invalid precedence, callback exceptions, copies and exact callback-state source/PNG preservation.
- [x] Complete integrated installed-wheel validation: 449 Python tests, strict typing and native extension identity with the 312-test artifact.
- [x] Complete independent review and exact-head/current-main CI; radio support remains separate.


## Owned auxiliary paragraph deletion

- [x] Audit public native remove_block/sentinel behavior and distinguish adapter locator maintenance from semantic-reference remapping.
- [x] Reuse integer collection deletion for owned direct header/footer/note paragraphs, with staged identity/accounting changes, bounds and ordinary semantic guards.
- [x] Add three native groups and five public workflows for actual output, surviving nested ownership, sentinel/note behavior and atomic callback/scope/resource failures; run 41 related tests and strict copied-package typing.
- [x] Complete integrated locked release/clean installed-wheel artifact: 315 Rust / 454 Python tests, strict typing and unchanged license audit.
- [x] Complete independent review and exact-head/current-main CI before manual merge.
- [ ] Retain body identity migration, cell/PPTX deletion and broader DOC5/TXT1 insertion/movement/adoption work.


## Cumulative owned PDF field appearance accounting

- [x] Reproduce undercharged repeated text/choice fills and initial/configured creation generations with small native owned fixtures; distinguish checkbox state reuse and already-accounted property redraws.
- [x] Add shared appearance work reservation, both creation-generation charges, value-generation charges and successful-fill item accounting without runtime object scans.
- [x] Add native count/byte/copy/boundary/rotation controls and public repeated-write/source/PNG/copy/flatten coverage; run related field regressions.
- [x] Complete integrated locked release/clean installed-wheel validation: 318 Rust / 455 Python tests, strict typing and unchanged license audit.
- [x] Complete independent package review and exact-head/current-main CI before manual merge; radio expansion remains separate.
- [ ] Keep broader PDF6/U10 imported-form and multiwidget acceptance open.


## PDF field value normalization and ownership

- [x] Reproduce overridden encode replacing a field and accepting a stale write on the installed story-deletion release.
- [x] Normalize actual string content with base operations, retain initial lifetime precedence and recheck identity after payload construction.
- [x] Validate three focused public groups and fifteen related field tests against an isolated copied installed package.
- [x] Complete integrated installed-wheel validation: 458 Python tests, strict typing and byte-identical native extension from the 318-test artifact.
- [x] Complete independent review and exact-head/current-main CI before manual merge.


## Owned DOCX body identity separation

- [x] Audit every body allocation/enumeration/placeholder consumer, distinguish physical section/layout paths from editable IDs, and retain pinned native behavior.
- [x] Reuse the existing checked monotonic registry allocator; stage body publication and bound temporary reverse views without introducing public body operations.
- [x] Update classified native test paths without relaxing format/geometry/output assertions, and add focused native/public identity, section and rollback coverage.
- [x] Complete three focused native groups, four new public workflows, 59 related public workflows, formatter/Clippy/debug build and strict copied-package consumer typing.
- [x] Complete the integrated local artifact and independent review: 321 recorded native tests and 462 clean installed-wheel tests; all 462 tests independently rerun with matching wheel bytes, typing and the locked license audit.
- [x] Complete final exact-head/current-main CI and manual merge: PR93, run39530041981.
- [ ] Retain separately scoped body section/semantic deletion, cell/PPTX deletion and broader DOC5/TXT1 insertion/movement/adoption work. The historical spec78 identity-migration prerequisite is superseded by this internal change.


## Bounded owned PDF radio groups

- [x] Probe public native group creation, canonical Unicode/hash identities, clear/read-only behavior, fixed vector appearances and retained generation counts.
- [x] Integrate immutable same-page groups through existing field containers/properties with independent field/widget bounds and cumulative preflight/accounting.
- [x] Add native and public source/pixel/geometry/flatten/callback/identity/budget tests and consumer typing assertions.
- [x] Complete integrated release/clean installed-wheel corpus: 326 native and 467 Python tests, typing and unchanged license audit.
- [x] Complete independent exact-head/current-main review and manual merge: PR94, run39548202131.
- [ ] Retain imported inventory, arbitrary mutable/multi-page groups and broader PDF6/U10 appearance work.


## Owned body paragraph deletion

- [x] Prove the native final-section tail case and one-blank preservation with exact section values, DOCX readback, PNG dimensions and PDF page geometry.
- [x] Extend existing staged collection deletion to ordinary direct body paragraphs; retain section/semantic guards, bounded locator updates and existing auxiliary behavior.
- [x] Add four native groups and five public workflows for output, identity, section starts/restarts, protected anchors, callbacks, placeholder reuse and rollback; deliberately update obsolete body-Unsupported assertions while preserving cell/PPTX/section checks.
- [x] Complete focused native tests, fmt/Clippy/debug build and copied-package public/related checks; record exact results below.
- [x] Complete integrated locked release and clean installed-wheel corpus: 330 native/472 Python tests, strict typing, license audit and independent472-test review.
- [x] Complete final exact-head/current-main CI and manual merge for body paragraph deletion: PR96, run39549509766.
- [ ] Keep section-boundary/semantic deletion, cell/PPTX deletion and broader DOC5/TXT1 insertion/movement/adoption open.


## Native PDF comment opacity

- [x] Audit pinned native Style/set_style and AP-local ExtGState generation; retain the seven existing comment kinds and canonical metadata profile.
- [x] Reuse Annotation property/update, bounded target validation and staged native appearance/accounting.
- [x] Complete three focused native groups plus six related native groups, 19 copied-package public tests (four new plus 15 related), formatter/Clippy/debug build and strict consumer typing.
- [x] Migrate only the geometric-comment and lock evidence affected by intentional fractional-CA fixture changes to the exact new integrated artifact.
- [x] Complete locked full native/release/installed corpus and independent artifact review: 333 native and 476 installed Python tests.
- [x] Complete final exact-head/current-main CI and manual merge for annotation opacity: PR97, run39550636008.
- [ ] Retain broader PDF6 imported-inventory/reply and PDF8 encryption/profile work.


## Owned paragraph movement

- [x] Audit public native removal/insertion and execute raw table-tail/singleton/section-destination proofs before implementation.
- [x] Reuse the existing paragraph collection move spelling and native staged transaction, bounded stable-ID/root-path updates, section/semantic guards and callback revision checks.
- [x] Add six native groups and five public workflows for native padding, identity, rollback, output, auxiliary stories, no-ops, copies, iteration and callbacks; retain all existing tests unchanged.
- [x] Complete six focused native groups, fmt/Clippy/debug build, five new plus 25 related copied-package workflows, final media/field checks and strict consumer typing; record exact local results below.
- [x] Complete integrated locked release/installed-wheel corpus and independent review: 339 native and 481 installed Python tests.
- [x] Complete final exact-head/current-main CI and manual merge for paragraph movement: PR99, run39551991262.
- [ ] Keep anchored/cross-section/cross-container/cell/PPTX movement, imported preservation and broader DOC5/TXT1 parity open.


## Native-backed everyday property types

- [x] Audit selected native getter signatures and nullable JSON fields; retain current factory/copy ownership typing.
- [x] Annotate existing property access and simple setters without aliases, runtime coercions or dynamic Any placeholders.
- [x] Extend strict static consumer assertions and verify expected invalid typed uses are rejected; confirm runtime AST is unchanged after removing annotations.
- [x] Complete integrated installed-package strict typing, unchanged native-byte verification and independent 481-test package review.
- [x] Complete final exact-head/current-main CI and manual merge for property typing: PR100, run39553367804.
- [ ] Retain unreviewed mappings/properties, documentation and IDE coverage under API3.


## Reproducible Office workload measurements

- [x] Preserve the default 20-page generator and add bounded 200-page/image-heavy configurations plus source manifests.
- [x] Add ordered all-page PNG and input hashes to the existing benchmark, retaining explicit process-memory/cache caveats.
- [x] Verify new image fixtures reopen, cached/one-shot smoke parity, four pre-creation budget rejections and existing-directory protection.
- [x] Complete 36 sequential fresh-process measurements across six inputs and two modes, with three alternating-order repetitions and all-page byte/counter parity.
- [x] Record exact source/tool/wheel identities, timings, memory ranges and limitations; independent benchmark source review found no blocker.
- [x] Complete independent verification of all 36 samples, source/tool/artifact hashes, page digests, counters, table medians/ratios and memory ranges.
- [x] Complete required current-main CI and manual merge for Office benchmark evidence: PR101, run39554746117.
- [x] Supply the independently reproducible PDF memory/full-document counterpart in spec92; retain broader real-input/platform QA7 work.


## Native title and author fields

- [x] Execute two raw native all-story evaluation, empty/nonempty cache and atomic-width proofs before adapter implementation.
- [x] Extend existing add_field with title/author, every-commit expansion limits and bounded native placement preflight; retain the existing API surface and numeric-field behavior.
- [x] Verify four new native groups, 28 related field groups, 13 copied-package public workflows, fmt/Clippy/debug build, strict typing and 19 support/reference documentation tests; record artifact scope.
- [x] Complete integrated locked release/corpus, strict typing, license audit and independent artifact review: 343 native and 484 installed Python tests.
- [x] Complete final exact-head/current-main CI and manual merge for title/author fields. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Keep general REF3 field kinds, REF4 refresh and imported preservation open.


## Guarded native DOCX picture placement

- [x] Audit pinned native Float/read/write/layout APIs and execute four raw fixture groups before implementing the passing direct-body subset.
- [x] Reuse Picture.format/formatting with bounded native patching, canonical lengths, ownership/revision callbacks and existing one-layout geometry preflight.
- [x] Add focused native/public workflows for output, wrapping, transforms, identity, reset, rollback, later geometry and callbacks; document upstream front paint-order and overlap/layoutInCell prerequisites.
- [x] Complete six native groups, five new plus28 related copied-package tests, fmt/Clippy/debug build, strict typing and static-site/roadmap checks; record exact artifact evidence.
- [x] Complete integrated locked full native/release/installed-wheel corpus and independent artifact review: 349 native and 490 installed Python tests.
- [x] Complete final exact-head/current-main CI and manual merge for picture placement. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Keep full MED2/BG2–BG4/BG8, front/contours/nested scope, opacity/rotation and independent layering/overlap controls open. BG5 later closes only its bounded section-first letterhead criterion under spec105.


## Cross-format render-session evidence

- [x] Extend the shared development benchmark to PDF while preserving the old Office CLI/counter field; add optional native PDF fixture export and reopened counts.
- [x] Verify two-page native report/deck PDF smoke parity and legacy Office counter compatibility.
- [x] Complete30 sequential fresh-process PDF samples across five inputs/two modes/three repetitions, checking source identity, all-page byte parity and exact retained renderer counters.
- [x] Independently verify all samples, timings, memory ranges and hashes; retain the slower small-PDF case and separate eager-extraction counter limits.
- [x] Combine spec91/spec92 evidence to close bounded P1 reuse, retaining QA7 real-input/platform and P4 allocation work.
- [x] Complete final documentation/source checks and independent final evidence review: 19 documentation tests and a 29-page/1,716-link site.
- [x] Complete required exact-head/current-main CI and manual merge. Completed through reviewed aggregate PR112, run39563664023.


## Combined native DOCX cell layout acceptance

- [x] Audit existing padding/orientation/RTL/alignment native paths; prove six combined public matrix profiles before final acceptance.
- [x] Add a separate test module for the orientation/alignment matrix, independent paragraph/table direction and empty-cell copy/reset geometry; preserve historical fingerprinted fixtures and runtime APIs.
- [x] Complete final three-workflow multi-DPI DOCX/PDF acceptance, including absolute native row geometry, static-site links and roadmap consistency.
- [x] Complete independent final source review and all three workflows against the exact installed picture release wheel, including an independent rerun.
- [x] Complete required exact-head/current-main CI and manual merge for the combined cell-layout corpus. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Keep native inter-cell spacing, full vertical-script fidelity and complete TBL8 open.


## Owned DOCX cell paragraph deletion

- [x] Execute raw native body/header/footer, merged/nested, required-blank and authored-empty identity/output proofs before adapter implementation.
- [x] Extend existing deletion with container-relative locator/sentinel maintenance and unchanged section/reference/resource guards; verify four focused native groups.
- [x] Complete 13 related native tests, fmt/Clippy/debug build, 19 copied-package public deletion/identity workflows and 27 support/reference/roadmap checks; record exact evidence.
- [x] Complete integrated locked release/full corpus and independent artifact review: 355 native and 498 installed Python tests.
- [x] Complete final exact-head/current-main CI and manual merge for cell paragraph deletion. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Keep broader DOC5/TXT1 composition, cell movement, imported editing and PPTX deletion open.


## Native slide duplication prerequisite (spec96)

- [x] Audit pinned public model and private editor copying/remapping; record basic-validation and owned-generation identity distinction.
- [x] Specify same-deck native copy prerequisite and concrete reference/media/layout/limits fixture plan without expanding the API.
- [ ] Obtain and verify a public bounded native copy operation before implementing owned slide duplication; keep cross-presentation import separate and SLIDE1 open.


## Native-backed snapshot output types

- [x] Inventory exact native keys, omission/nullability and list/tuple shapes for five existing metadata/formatting getters; identify polymorphic snapshots separately.
- [x] Add private output TypedDict declarations and strict consumer assertions without changing accepted runtime inputs, public exports or method bodies/defaults; correct only existing cell-list/border-replay annotations.
- [x] Verify positive mypy, ten intended negative diagnostics, old-facade sensitivity, runtime snapshot/key introspection and annotation/type-declaration-erased AST equality; complete independent source/static review.
- [x] Verify the exact installed candidate, unchanged native bytes/runtime bodies, strict positive/negative consumers and runtime snapshot probes, including independent review.
- [x] Complete fresh full installed-corpus current-main CI and manual merge for snapshot typing. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Keep polymorphic paragraph/style/default snapshots and broader API3 discoverability open.


## Same-cell paragraph movement (spec97)

- [x] Prove native remove/insert behavior in ordinary/nested body/header/footer cells and merged aliases before implementation.
- [x] Extend existing move transaction and preserve parent-local identity/sentinel/accounting guards without a new API.
- [x] Complete focused public/native/typing/documentation evidence, including cell anchor/alias/resource guards.
- [x] Complete independent final source review and focused replay (four workflows,24.049s).
- [x] Complete integrated locked release/full corpus and independent artifact review: 361 native and 502 installed Python tests.
- [x] Complete exact-head/current-main CI and manual merge; retain broader DOC5/TXT1 work. Completed through reviewed aggregate PR112, run39563664023.


## Owned DOCX theme source audit (spec99)

- [x] Trace pinned theme Settings, property read/write, inheritance, layout and PDF; distinguish metadata preservation from live theme identity.
- [x] Connect the verified prerequisite to existing U02/STY3 and specify native fixtures without adding a speculative API/test.
- [x] Independently review pinned source findings and validate documentation links.
- [ ] Obtain native theme-reference/resolution support and execute the proposed save/render/export fixtures before implementing theme changes; leave STY3 open.


## Owned PPTX cell paragraph deletion (spec101)

- [x] Prove intact native model deletion, survivor properties, ordinary/horizontal/vertical/rectangular merges and final-blank writer behavior before integration: two raw groups pass, 30.86 s; fmt/Clippy pass.
- [x] Integrate existing collection syntax with canonical generations, staged resource/revision guards and preserved DOCX callback behavior; source review clear.
- [x] Verify native/public rollback, copies, empty append, invalidation and actual PPTX/PNG/PDF output; independent source and exact-wheel review pass.
- [x] Complete locked full corpus: 366 native tests and two independent 506-test installed-wheel runs pass.
- [x] Complete exact-head/current-main CI and manual merge; retain broader DOC5/TXT1/SLIDE3 work. Completed through reviewed aggregate PR112, run39563664023.


## Bounded owned DOCX comments (spec98)

- [x] Execute raw native storage, metadata, durable-ID, endpoint/interior shaping, overlapping shading, multiline/split-page and ordinary/merged/nested cell controls before adapter implementation.
- [x] Reuse one collection/add operation and existing ranges for plain-text root/direct-reply CRUD and typed metadata/navigation properties.
- [x] Stage private locator/block/text accounting, monotonic IDs and exact-candidate inventory; reuse the proof in navigation/media/output without relaxing other anchors.
- [x] Add native exact/+1, malformed topology/registry, revision/allocator exhaustion, Unicode author, media/reference and rollback controls; retain all historical fixtures unchanged.
- [x] Complete exact integrated native/installed-wheel corpus and independent final review: 379 native tests and two clean 512-test installed runs pass.
- [x] Complete exact-head/current-main CI and manual merge before merge acceptance. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Retain arbitrary interior/cross-paragraph roots, rich comment stories, nested replies, imported editing and review-display selection under existing REV1/TXT8/REV5 and U02 ownership.


## DOCX extraction provenance (spec100)

- [x] Audit pinned source/path/layout/normalization boundaries and independently review the bounded proposal.
- [x] Execute three raw native groups for exact source intervals, selection helper parity, repeated/turned/clipped cells, story collisions and PNG/PDF location controls; fmt/Clippy pass.
- [x] Complete bounded adapter/schema, source/copy/resource/malformed geometry and public compatibility acceptance, including the measured retained-allocation correction.
- [x] Complete independent frozen-source/profile review and final four-workflow replay, including the exact known 200-page fixture.
- [x] Complete integrated full corpus and independent artifact review: 387 native tests and two clean 516-test installed-wheel runs pass, including the caught and corrected signed-advance regression.
- [x] Complete exact-head/current-main CI and manual merge. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Retain E2/LAY8/U05 normalized slicing, auxiliary/object/table geometry and broader parity scope.


## Office caller-font source audit (spec103)

- [x] Separate existing PDF supplied-font support and STY3 theme identity from remaining Office STY5 acceptance.
- [x] Trace pinned public font storage/admission, global selection, face lifetime and native PDF export behavior; connect precise prerequisites to U02/U11.
- [x] Complete independent pinned-source review and documentation/link checks.
- [ ] Obtain native document-scoped resolution/embedding contracts and execute the proposed valid-font isolation/save/render/export fixture gate before adding Office caller-font inputs; STY5 remains open.


## Formatting snapshot annotations (spec104)

- [x] Audit exact native direct/resolved/default/region serializers and PPTX alignment-only behavior before selecting conservative private output schemas.
- [x] Annotate existing getters without new exports, input changes, runtime branching or native changes; preserve behavior with a normalized AST comparison.
- [x] Check strict copied-package consumer, deliberate invalid assignments/keys and populated/default snapshot shapes across all native kinds.
- [x] Complete independent final source/serializer review and replay positive/negative typing, runtime-key/schema and AST checks.
- [x] Complete exact installed-wheel/source/native verification and agreed corpus: repacked annotation-only artifact and subsequent 395-native/523-Python integration pass.
- [x] Complete current-main CI and manual merge. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Retain remaining API3 property/documentation/IDE work; do not claim full-package strict typing.


## Owned DOCX table deletion (spec102)

- [x] Execute two raw native structure/output groups before integration, distinguishing required and authored blanks and retaining orphan/protected-reference diagnostics.
- [x] Implement whole-table deletion through existing body/story/cell collections and public native remove_block with exact-parent shifts, preserved tail identity and subtree invalidation.
- [x] Preflight bounded source/global associations, stage checked resource reclamation and retain semantic, section, comment/navigation/media and callback guards.
- [x] Pass six focused native groups, formatting, all-target Clippy and a debug build after final tail-invariant review corrections.
- [x] Pass six focused public workflows and independent replay, strict positive/four-negative typing checks and synchronized roadmap/support/reference/site checks; final source review clear.
- [x] Complete exact integrated corpus and independent artifact review: 395 native tests and two clean 523-test installed runs pass.
- [x] Complete current-head CI and manual merge. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Retain broader DOC5 movement, duplication, insertion and cross-container/reference adoption work.


## Section-first letterhead composition (spec105)

- [x] Audit existing cookbook/story/section tests and preserve their historical sources/fingerprints.
- [x] Probe native reflow/variant/numbering behavior through the installed fixed-provenance wheel; replace the proposed +1 shift assumption with the observed exact +2 parity-preserving shift.
- [x] Add one ordinary caller-PNG example and one combined fixture with exact body/link/table preservation, independent sections, filler checks and bounded three-DPI appearance controls.
- [x] Complete independent source/replay review and documentation checks: one independently replayed workflow passes, with 19 support/reference tests and the site/roadmap checks passing.
- [x] Complete integrated corpus and independent review: both 523-test installed runs pass; BG5 satisfies its original bounded section/header-anchor and repagination criterion.
- [x] Complete exact-head/current-main CI and manual merge; retain BG8 and documented broader limitations. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Keep BG8 and broader section-fill/linkage/continuous/floating/watermark coverage open.


## Owned table PAGE guard (spec107)

- [x] Reproduce stale table PAGE context independently of movement/cache with native direct/nested fresh and read/write controls; retain correct direct PAGE and table count behavior.
- [x] Reuse one private scope check for upfront field insertion, candidate editing and defensive serialization after bounded source inventory; allow unsupported-object removal as recovery.
- [x] Pass two focused native groups, related field and affected accounting controls, formatting, all-target Clippy and a frozen debug build.
- [x] Pass 21 focused public/field/lifecycle tests and 27 roadmap/support/reference checks; final runtime/native/public source review clear.
- [x] Complete independent exact copied-extension replay: 18 tests pass; preserve deliberate fixture corrections and unchanged ledger fingerprints.
- [x] Complete final docs review, 397-native validation and corrected integrated acceptance: both clean 528-test installed runs pass; failed 524-test evidence and deliberate sixth fixture correction remain recorded.
- [x] Complete exact-head/current-main CI and manual merge. Completed through reviewed aggregate PR112, run39563664023.
- [ ] Retain native table field-context repair under U07/LAY6/REF3; spec106 movement separately passes its revised raw gate with supported count fields and the PAGE guard retained.


## Page and slide index callbacks (spec109)

- [x] Reproduce wrong-target PDF delete/move/lookup and slide deletion on the installed pre-correction artifact.
- [x] Reuse existing PDF generation/count and native slide-ID sequence snapshots; guard every mutation argument and PDF handle construction without new public APIs.
- [x] Pass four focused workflows and 22 combined related workflows, strict typing and independent source/four-workflow replay.
- [x] Complete exact combined installed artifact/corpus alongside the corrected spec107 fixture: two clean 528-test installed runs pass with unchanged tested native bytes.
- [x] Complete current-main CI and manual merge. Completed through reviewed aggregate PR112, run39563664023.



## Mirrored DOCX margins (spec108)

- [x] Audit pinned public model/read/write/layout; independently clear a two-group raw gate without adapter coordinate logic.
- [x] Execute raw asymmetric geometry, displayed/physical parity, header inheritance, native selection and source/PDF pixel tests; document corrected part-ID and filler assumptions.
- [x] Add one typed document-wide boolean using explicit existing geometry preflight and ordinary commit; preserve all semantic/resource/output guards.
- [x] Add three native adapter groups and four public workflows for revision/copy/reset, strict bool/lifetime, geometry/cache rollback, section/header/table output, line labels, columns and natural growth; strict consumer and deliberate typing negatives pass.
- [x] Complete independent frozen code/docs and four-workflow replay review; documentation/site checks pass.
- [x] Complete integrated corpus and independent artifact review: 410 native tests and two clean 538-test installed runs pass.
- [x] Complete current-head CI and manual merge through reviewed PR118, run39566211261.
- [ ] Retain broader LAY10 settings and existing unsupported layout combinations under their current owners.


## Same-parent owned DOCX table movement (spec106)

- [x] Preserve original failed native PAGE proof, add spec107 rejection, and pass revised raw2 with 336 structural/24 output profiles and supported native count values.
- [x] Implement only existing table collection movement with exact-parent coordinate rotation, stable descendant/generation identity and normal candidate guards.
- [x] Share narrow spec102 preflight while retaining all deletion behavior; pass movement6, unchanged deletion6 and PAGE-guard2 native groups, formatting and all-target Clippy.
- [x] Pass focused copied-package public/typing checks and independent source/six-workflow review; synchronize documentation with the bounded profile.
- [x] Complete integrated full native/installed-wheel corpus and independent artifact review: 410 native tests and two clean 538-test installed runs pass.
- [x] Complete exact-head CI and manual merge through reviewed PR118, run39566211261.
- [ ] Retain broader DOC5 insertion, duplication, adoption and semantic-anchor relocation gaps.


## 113. Release compiler selection

- [x] Verify the immutable maturin action declares and uses its toolchain input for both container and host builds; explicitly select CI Rust 1.95.0 in both release steps.
- [x] Independently review the exact pinned action input/source and verify only the two compiler-selection keys change the workflow.
- [x] Complete required current-main CI and manual merge through reviewed PR118/PR119.
- [ ] Validate all nine platform/Python wheels at the eventual release gate. No new tag or release is authorized by this task alone.


## Section vertical alignment (spec111)

- [x] Audit native model/read/write/layout/PDF and run three raw groups; retain desired-geometry failures and precise note/readback drift separately from accepted note-free controls.
- [x] Add one typed Section property, strict native enum parsing, central candidate/output profile checks and vertical-only existing-layout bounds.
- [x] Pass five native adapter groups, unchanged raw replay, four public workflows, strict consumer, formatting and Clippy; verify solid/PNG backgrounds explicitly.
- [x] Resolve the reviewed rotated-cell coordinate gap with central horizontal section/cell admission and defensive output refusal; pass corrected independent four-workflow replay and documentation/site checks.
- [x] Complete frozen documentation/source review and integrated425-native/two547-test clean installed-wheel corpora.
- [x] Complete final exact-head/current-main CI and manual merge through reviewed PR120, run39569285162.
- [ ] Retain broader LAY10 settings and excluded native compositions under existing U02; do not mark the roadmap criterion complete.


## Unused owned DOCX styles (spec 110)

- [x] Independently review pinned remove/find/get/new_id/reference-carrier premises and pass two raw groups structural/readback/PNG/PDF/identity proof.
- [x] Add only existing-collection single-item deletion, complete bounded reference refusal and exact handle identity with atomic bounded retirement/native allocation.
- [x] Pass five native adapter groups, all 26 selected style regressions, formatting/all-target Clippy/debug build, five copied-package public workflows,14 historical style/note workflows and strict consumer typing with three intended negative key diagnostics.
- [x] Complete independent frozen source and five-workflow replay review (2.250s).
- [x] Complete synchronized documentation review.
- [x] Complete integrated425-native/release and two547-test clean installed-wheel corpora with independent artifact review.
- [x] Complete final exact-head/current-main CI and manual merge through reviewed PR120, run39569285162.
- [ ] Keep STY6 open for style import, collision policy and general cleanup; no batch prune or automatic restyling.


## Existing layout value annotations (spec115)

- [x] Review native successful result signatures and retain iterable widths, tuple/list forms and nullable resets; preserve runtime unsupported-format checks.
- [x] Annotate 17 existing signatures and independently prove annotation-erased AST identity with the prior facade.
- [x] Pass the strict installed consumer, ten intentional negative typing diagnostics and four unchanged DOCX/PPTX layout workflows against an explicitly repacked wheel with identical tested native bytes.
- [x] Complete independent artifact/static/probe/documentation review and final547-test installed corpus against the explicit Python-only repack.
- [x] Complete final exact-head/current-main CI and manual merge through reviewed PR120, run39569285162.
- [ ] Keep broader API3 discoverability and IDE coverage open.


## Fallible PPTX rendering integration (spec114)

- [x] Independently review the separate native renderer/PDF contribution, actual panic/Busy/thread/recovery evidence, original successful byte comparisons and unchanged upstream baseline limitations.
- [x] Publish the reviewed six-file contribution to an explicitly identified fork; verify exact parent and patch identity without claiming upstream acceptance.
- [x] Integrate all DeckCraft pins together, retain other engine revisions, use one fallible native PNG call and existing Python exception/PDF conversion mapping.
- [x] Add actual production-path native Busy/preflight/recovery controls and explicitly scoped Python boundary/blank workflows.
- [x] Complete final locked native/fmt/Clippy/release, clean installed corpus, independent exact artifact, typing, license/size and current source-ledger checks.
- [x] Complete exact-head/current-main CI and manual merge through reviewed PR121, required CI39572467318 and documentation CI39572467390.
- [ ] Retain broad R4/U09 limits and other native entry points; no version/tag/release before overall completion gates.


## Native WordCraft PDF shape stroke upgrade (spec116)

- [x] Review and publish the five-file native contribution with exact parent/diff identity; preserve baseline failures and package-scoped evidence without claiming upstream acceptance.
- [x] Preserve original shape matrix and additional ratio/refusal diagnostics; retain a conservative future profile without creating a new API here.
- [x] Update every WordCraft source together and add one native plus one public imported-document regression using the original generated fixture and explicit provenance.
- [x] Complete final428-native/full550-installed independent artifact, fmt/Clippy/typing/license/size and current source/ledger/documentation checks.
- [x] Complete exact-head/current-main CI and independent manual merge through PR122 and required CI39575658890.
- [ ] Keep shape authoring and broader MED4/U04 work separate; no version/tag/release before overall completion gates.


## Owned DOCX closed shapes (spec112)

- [x] Independently review historical shape/textbox raw2 and additional native primitive/coupled-anchor raw2; preserve rejected Line/overflow/rotation cases without relaxing pixel tolerances.
- [x] Add one creator/collection/private handle, shared Picture placement parsing, exact ownership/reference profile and candidate/output full-outline/cardinality checks.
- [x] Pass six native adapter groups, raw2, copied-package six public workflows,14 historical picture/media/style regressions and strict typing with six intended negative diagnostics.
- [x] Complete final horizontal-section defensive guard native/public replay.
- [x] Complete independent frozen source/public review (six workflows15.775s).
- [x] Complete spec116 native closed-shape corner/outline parity gate, including min/max width and stroke-to-dimension boundaries; preserve 200-point/native-clamp diagnostics.
- [x] Complete corrected436-native and two clean556-test installed corpora, exact-artifact/source review, strict typing and synchronized bounded support documentation.
- [ ] Complete closed-shape exact-head/current-main CI and manual merge; dependency PR122 is merged.
- [ ] Keep MED4 open for lines/textboxes and excluded contexts; no new U-owner or broad shape-parity claim.
