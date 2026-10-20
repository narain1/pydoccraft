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
| P2 | P1–P4 | Parse/layout reuse, region rendering, PDF assembly, bounded extraction | Basic native PDF assembly verified; complete preservation and performance criteria remain open |

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
- [ ] Complete table-style/theme/next-linked-style editing, full PPTX table semantics and remaining source-preservation/advanced criteria.

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
- [ ] Complete conditional table styles, upstream table fidelity gaps and outline-preserving subsets; broad TBL/PDF5 criteria stay open until their remaining acceptance cases pass.

- [x] Audit merged-through-PR24 coverage and mark C1/E1/API2/API4/API6/API8/API9/TXT7 complete with existing corpus evidence; retain broader incomplete criteria.

- [x] Complete R2 multi-DPI visible-output and P3 reopened assembly link/raster acceptance; complete TBL1/TBL2 count construction and ordered slice/grid access.
