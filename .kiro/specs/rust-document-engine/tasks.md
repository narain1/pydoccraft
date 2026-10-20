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
| P1 | C1–C5 | Authoring contract, DOCX/PDF creation, reports, templates | TODO |
| P1 | E1–E6 | Structured tables, source locators, Office semantics, PDF links, diagnostics, Markdown assets | E1 DOCX recursive cells/spans verified; broader E1 and E2–E6 remain open |
| P2 | P1–P4 | Parse/layout reuse, region rendering, PDF assembly, bounded extraction | TODO |

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
