# Canonical roadmap workstreams

Three agents reviewed overlap, API size and prerequisites. The resulting roadmap retains all 139 coverage IDs with **one owner each**, organized into **14 workstreams**. There are 138 open coverage items and one existing rendering capability, R1. Workstreams are GitHub planning units, not new public API concepts.

The [JSON mapping](roadmap-workstreams.json) is the authoritative ownership map. [TODO.md](../TODO.md) holds each feature's acceptance details; [the API proposal](authoring-api-design.md) describes the small Python surface. [The upstream register](upstream-roadmap.md) distinguishes confirmed source findings, existing public support and unassessed contribution candidates.

GitHub project: [pydoccraft Roadmap](https://github.com/users/narain1/projects/8), with [Backlog](https://github.com/users/narain1/projects/8/views/2), [Roadmap](https://github.com/users/narain1/projects/8/views/3), and [Upstream work](https://github.com/users/narain1/projects/8/views/4) views. Workstream issues remain the canonical discussion and acceptance records. The project is private; timeline dates are intentionally unset until scheduled.

## Workstreams and connections

| ID | Workstream | Priority | Prerequisite baseline | Ownership / upstream connections |
|---|---|---|---|---|
| [W01](https://github.com/narain1/pydoccraft/issues/3) | Compact Python API and one coverage ledger | P0 | None | Local facade/design; one inventory shared by docs and tests |
| [W02](https://github.com/narain1/pydoccraft/issues/4) | Rendering correctness, limits and reuse | P0 | Existing rendering; coordinate with W01 for additions | Adapters; U09 renderer failure contracts |
| [W03](https://github.com/narain1/pydoccraft/issues/5) | Editable lifecycle, safe persistence and diagnostics | P1 | [W01](https://github.com/narain1/pydoccraft/issues/3) | Facade/adapters; U01 preservation, U09 errors, U11 PPTX validation |
| [W04](https://github.com/narain1/pydoccraft/issues/6) | Rich text, styles, numbering and safe copying | P1 | W03 lifecycle | WordCraft adapter; U02 targeted fidelity work |
| [W05](https://github.com/narain1/pydoccraft/issues/7) | Semantic table structure, formatting and pagination | P1 | W04 content; W06 layout for pagination | WordCraft adapter; U03 |
| [W06](https://github.com/narain1/pydoccraft/issues/8) | Sections, drawings and native backgrounds | P1 | W04 content | WordCraft adapter; U04 backgrounds/anchors, U05 source layout |
| [W07](https://github.com/narain1/pydoccraft/issues/9) | Ranges, links, notes and review | P1 | W04 content; W06 geometry for export | WordCraft adapter; U06 lower-level revision routines |
| [W08](https://github.com/narain1/pydoccraft/issues/10) | Native fields, controls and references | P2 | W06 layout, W07 ranges | WordCraft; U07 coordinated evaluation |
| [W09](https://github.com/narain1/pydoccraft/issues/11) | Extraction structure, locations and portable output | P1 | Existing extraction; coordinate with W01 for new model conventions | Adapters; U05 indexing, U10 public table detection |
| [W10](https://github.com/narain1/pydoccraft/issues/12) | PDF authoring, assembly and advanced audit | P1 basics; P2 advanced | W01 conventions, W03 common lifecycle baseline | PdfCraft adapter; U10 narrow gaps; basic primitives already exist |
| [W11](https://github.com/narain1/pydoccraft/issues/13) | PPTX authoring and preservation | P2 | W01 conventions, W03 common lifecycle baseline | DeckCraft adapter; U11; public writer already exists |
| [W12](https://github.com/narain1/pydoccraft/issues/14) | Advanced Office object support audit | P2 | W01 audit, W03 preservation contract | WordCraft; U08 operation-by-operation audit |
| [W13](https://github.com/narain1/pydoccraft/issues/15) | Composed workflows and cookbook coverage | P1 | Relevant W04–W07 primitives | Local recipes; no new report/template/batch framework |
| [W14](https://github.com/narain1/pydoccraft/issues/16) | Shared validation harness and release evidence | P0 setup; continuous | Start immediately; consume feature evidence as delivered | Test infrastructure and release gates, not another product API |

Dependencies refer to the **needed baseline**, not completion of every advanced feature in a broad workstream. PDF does not wait for all DOCX editing, PPTX does not wait for PDF, and extraction/rendering improvements can ship independently. On GitHub, exact baseline requirements belong in linked issue sections; only true whole-issue blockers should use blocking relationships.

## Removed duplication and API growth

| Overlap | Canonical decision |
|---|---|
| API1 / QA1 / API7 / QA8 | One development ledger and published matrix; defer a runtime capability framework |
| E5 / PKG8 | One diagnostics model; errors and fidelity notices retain distinct severity |
| C1–C5 and expanded feature groups | C items are acceptance milestones under their owner, not duplicate implementation issues |
| API9 and existing render methods | Use PNG bytes, `Path.write_bytes` and optional Pillow; no `RenderedPage` or third render entry point |
| C4 / PDF4 | Ordinary DOCX authoring plus existing PDF conversion; no report schema/builder |
| BG2–BG4 / MED2–MED4 | Image/text placement plus layer/opacity/anchor properties; watermark/background recipes |
| BG5 / LAY3 | Section/header scope owns first/even/odd behavior |
| BG6 / TXT2–TXT3 / TBL7 | Semantic formatting properties own shading; BG6 is consistency acceptance |
| TXT7–TXT8 / FORM1 | Literal replacement owns range behavior; placeholders are a caller recipe |
| DOC5–DOC6 / FORM2 / FORM6 | One safe content-copy primitive; repeat and batch with ordinary Python loops |
| STY4 / TBL11 | Style examples and row comprehensions; no preset/dataframe subsystem |
| TBL10 / MED8 / REF6 | Objects own accessibility metadata; W08 owns automatic caption/reference calculations |
| P1 / LAY9 | Private caching and revision invalidation; no public session/transaction framework |
| TBL12 / BG8 / QA2–QA7 | Combined fixtures and release evidence; each feature owns its regression checks |

Native DOCX controls versus PDF forms, editing restrictions versus encryption, source backgrounds versus preview backgrounds, snapshots versus editable handles, and real redaction versus overlays remain distinct concepts. Consolidation must not hide these differences.

## Small API rules

Use one editable lifecycle (`new`, `edit`, `copy`, `save`, `to_bytes`), semantic insertion (`add_paragraph`, `add_table`, `add_image`, `add_run`), ordinary formatting properties, and format-specific ordered containers. Existing conversion and PNG rendering stay unchanged. Advanced native features can extend relevant objects only after verified demand and backend support.

Do not add a method because an SDK element or roadmap checkbox exists. A documented recipe is a valid feature delivery when existing primitives provide the behavior clearly. Do not over-generalize DOCX flow, PDF coordinates and PPTX slide geometry into a universal engine.

## Agent work and implementation sequence

The first implementation batch assigns every workstream to a portfolio while limiting concurrent edits to independent areas:

| Owner | Workstreams | Current deliverables |
|---|---|---|
| API foundation agent | W01, W03, W04, W07, W08 | Coverage validator, lifecycle design and [dependency sequence](implementation-sequence.md); authoring APIs remain planned |
| Rendering agent | W02, W06, W10, W11 | DPI metadata and raster bounds; [format authoring readiness](format-authoring-readiness.md) audits dependent work |
| Extraction agent | W09, W05, W12, W13 | Recursive DOCX cell extraction; [content readiness](content-readiness.md) audits table authoring, advanced Office objects and recipes |
| Integrator | W14 | Installed-wheel evidence harness, cross-review, spec synchronization and locked validation |

Source audits prepare later work; they do not complete creation/editing features. Run `python tools/check_roadmap.py` to enforce unique coverage ownership and an acyclic prerequisite graph. The [validation record](../.kiro/specs/rust-document-engine/validation.md) distinguishes implemented slices from remaining work and platform checks.

## Review corrections

- M1 includes layout invalidation immediately and links needed by its report example. Solid page color can ship before anchored watermark recipes.
- Blank creation is distinct from safe arbitrary-document editing; the latter needs no-op preservation evidence.
- Table structure can precede advanced pagination. Drawing/header primitives precede first-page watermark recipes.
- Field calculation and pagination belong to one coordinated upstream implementation; avoid a Python fixed-point loop.
- Pinned WordCraft and DeckCraft already expose writers, editing/model features and backgrounds. Do not file upstream requests to recreate them.
- Upstream candidates are marked locally. No issue has been filed in an ArtCraft repository by this planning task.
