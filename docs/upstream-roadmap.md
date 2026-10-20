# Upstream ArtCraft contribution register

This register marks where format behavior belongs. It is a source audit and contribution proposal, not runtime validation, a claim that every feature is absent, or a request to file issues in upstream repositories. Only this repository's roadmap is being published. Upstream upgrades remain dedicated changes with the full dependency/license/corpus audit.

**Evidence states:** existing public support; verified gap in an inspected path; candidate requiring further audit. Public support still needs local integration, safety and fidelity tests. Shared needs do not automatically belong in `craft-libs`: place work where the responsible implementation already lives.

## Contribution candidates

| ID | Owner | Evidence / priority | Proposed contribution | Local connection |
|---|---|---|---|---|
| U01 | WordCraft DOCX reader/writer | Candidate; P1 | Operation-scoped preservation/omission diagnostics and an opt-in strict read/save contract. Audit unknown markup and writer normalization before advertising safe arbitrary-document editing | W03; DOC2, PKG1/PKG2/PKG8, E5 |
| U02 | WordCraft model/layout | Existing public support; targeted fixes only | Reuse document editing, style/numbering and content-fragment copying. Request upstream work only for reproducible remapping, shaping or serialization failures, not a second edit API | W04 |
| U03 | WordCraft table/layout | Verified permissive mutation behavior; candidate improvement | Existing merge/split operations can skip/clamp invalid requests. Add fallible semantic validation upstream if needed; Python must validate indexes/ranges regardless. Nested-cell layout already exists; advanced pagination needs fixtures, not an assumed rewrite | W05; TBL3–TBL9 |
| U04 | WordCraft document/layout/render | Existing background settings; targeted fidelity candidates | Reuse native page color, picture/watermark settings and drawing placement. Add focused rendering/export or anchor-scope fixes only when fixtures expose them | W06; BG1–BG7, MED2–MED4 |
| U05 | WordCraft layout | Existing public source index; exact slicing unassessed | Reuse layout's story/path-to-page-item index before proposing new provenance APIs. Exact automatically split paragraph/run offsets may need a focused public layout projection if existing pieces are insufficient | W06/W09; LAY8, E2 |
| U06 | WordCraft document library | Verified location problem for inspected algorithms; P2 | Move/re-export pure formatting-revision resolution routines from the application engine into a lower-level library. Keep Session/editor-state recording separate; do not link the engine or duplicate its algorithms | W07; REV3/REV4 |
| U07 | WordCraft layout/document | Partial field evaluation observed; P2 | Expose deterministic supported-field evaluation, stale/unsupported result reporting and coordinated pagination. Audit controls/bindings separately; avoid a Python field-calculation loop | W08; LAY6, REF3–REF7, FORM3–FORM5 |
| U08 | WordCraft format libraries | Candidate; chart limitation identified; P2 | Add structured diagnostics for unsupported-only chart placeholders and unsupported types omitted from mixed plots. Audit cached data, math and diagrams per operation; public chart/math/SmartArt models already exist | W12; MED6/MED7 |
| U09 | DeckCraft renderer; PdfCraft renderer | Verified gaps in selected paths; P0 for DeckCraft failures | DeckCraft: add a fallible per-call render result instead of a sized blank-image fallback on caught panics. PdfCraft: optional strict tile geometry and structured raster errors. These are separate engine contributions, not a proposed shared renderer | W02/W03; R4, P2, E5, PKG8 |
| U10 | PdfCraft export/redact | Verified narrow gaps plus existing primitives; P1/P2 | Expose bounded table detection and consumed-block masks. Vertical-writing redaction remains explicitly unsupported in the inspected API and needs verified upstream geometry work. Basic creation, page operations and redaction already exist; U09 owns rendering-error improvements | W09/W10; E1, E4, PDF1–PDF8 |
| U11 | DeckCraft model/PPTX/PDF | Existing public writer; partial validation/preservation; P2 | Improve model-reference validation and structured preservation reports when fixtures demonstrate omissions. Optional vector PDF artwork export is a separate fidelity enhancement; current PDF export rasterizes artwork with a searchable text overlay | W03/W11; SLIDE1–SLIDE6, PKG1–PKG3 |

U09's DeckCraft failure contract and U10's public table detector are concrete upstream candidates. Broad feature names in U02–U05/U08/U11 are **not** proof that upstream lacks those features.

The [W12 source audit](content-readiness.md#advanced-office-audit) narrows U08: pinned WordCraft's [chart drawing reader](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/read/chart.rs) can draw a placeholder for unsupported-only charts or omit unsupported types in a mixed plot. A reusable limitation report is an upstream candidate; model existence or successful rasterization is not proof of complete chart fidelity. Fixture validation and the broader math/diagram inventory remain open.

## Pinned source evidence

### WordCraft

Revision: `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`.

- The [DOCX library](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/lib.rs) exposes writing APIs. New DOCX creation is local facade/integration work, not a request to invent a writer. Unknown-markup handling still needs preservation tests.
- [Writer normalization](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/write/mod.rs#L135) balances orphan field markers. Explicit skipped/repaired-content reporting is a candidate; comprehensive diagnostics availability has not been audited.
- [Table mutation](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/table.rs#L228) has permissive merge/split behavior; [nested layout](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/layout/src/table.rs#L765) already exists. Add strict semantic validation rather than another table algorithm.
- The [document model](https://github.com/storytold/wordcraft/tree/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src) provides document construction, nested cell content, editing and fragment operations, plus background settings. The source audit found public layout indexing as well; inspect [layout sources](https://github.com/storytold/wordcraft/tree/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/layout/src) before declaring provenance unavailable.
- [Formatting-revision routines](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/engine/src/cmd/fmt_revisions.rs#L265) include pure resolution logic located in the engine. The contribution is extracting usable library logic, not adding an engine dependency.
- [Layout fields](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/layout/src/fields.rs) evaluate a subset and can retain cached results. Creating a field does not establish complete evaluation support.
- [Background settings](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/lib.rs#L570) include dedicated picture/watermark models with limited options. [Watermark writing](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/write/watermark.rs) already exists. Audit general drawing placement before proposing arbitrary scope/opacity/rotation extensions.

### PdfCraft

Revision: `645016addcfb9078c557f0189559ab47cdcee057`.

- [Table types/detection](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/export/src/lib.rs#L54): `Cell`/`Table` are public, but the inspected `tables(blocks)` function is `pub(crate)`. A public bounded detector with its consumed-block mask would replace local inference while avoiding duplicated text.
- [Raster requests/results](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/render/src/raster.rs#L197): whole-page oversize rejection exists; tile requests clamp geometry, and failures expose strings. Strict tile validation and typed failures are focused improvements. Current PDF PNG DPI metadata is encoded locally and belongs in our adapter.
- [Creation](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/create/src/lib.rs#L59), [page organization](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/organize/src/lib.rs#L195), [editing](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/edit/src/lib.rs#L655) and [annotations](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/annot/src/lib.rs#L1015) already have public primitives. The simple text creator has restricted character support; broad Unicode authoring is not established by its existence.
- [Text geometry](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/render/src/text.rs#L41) supports glyph/line inspection. Try local normalized-offset mapping before proposing a shared convenience upstream.
- [Redaction](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/redact/src/lib.rs#L1) exists with verification and explicit errors, including unsupported vertical text. Integrating it requires a dependency audit and deletion corpus; do not advertise safe redaction merely from source inspection.

### DeckCraft

Revision: `22db9261f9d1a214c296fd390173130c45d76d66`.

- [PPTX API](https://github.com/storytold/deckcraft/blob/22db9261f9d1a214c296fd390173130c45d76d66/crates/pptx/src/lib.rs) already exposes `export`, `export_as` and `import_theme`. The writer is present; preservation and facade integration remain to be verified.
- [Renderer](https://github.com/storytold/deckcraft/blob/22db9261f9d1a214c296fd390173130c45d76d66/crates/render/src/lib.rs#L334) catches render panics and can return a correctly sized blank image. Its global counter is not a safe per-call failure signal. The current local adapter's dimension/PNG checks cannot distinguish this fallback; a public fallible call is a high-priority upstream contribution.
- [Model validation](https://github.com/storytold/deckcraft/blob/22db9261f9d1a214c296fd390173130c45d76d66/crates/model/src/lib.rs#L1105) covers selected IDs/references. Opaque parts, masters, layouts and themes already have model support; exhaustive preservation is unassessed.
- [PDF export](https://github.com/storytold/deckcraft/blob/22db9261f9d1a214c296fd390173130c45d76d66/crates/pdf/src/lib.rs) offers options, links/bookmarks and searchable text, with rasterized slide artwork. Validate selected slide indexes locally rather than relying on filtered invalid entries. Vector export is an optional upstream improvement.

## Work that stays local

Python naming, typed convenience values, source/path/stream handling, owned extraction models, Markdown formatting, recipes, published support tables, and API compatibility belong here. Rust adapters own native error mapping, operation dispatch, normalized offsets, boundary limits and our PNG encoding metadata. Upstream should own shared document-format semantics, layout and writer algorithms. If a candidate turns out to be already supported, reclassify it as integration/validation work instead of filing a redundant upstream request.

## Additional verified integration findings (2026-10-20)

- U10/PdfCraft: vector content serialization, supplied-font validation/shaping/embedding, metadata and hierarchical bookmark primitives are publicly usable at the pinned revision and now integrated locally. These are no longer speculative missing upstream features. Unicode tests cover Latin extended/Greek/Cyrillic, combining marks and ligature words; broader complex-script conformance remains a corpus task.
- U10/PdfCraft attachment candidate: the inspected rendering attachment-data API has a fixed 1 GiB decode cap, with no caller-selectable smaller bound, and no suitable public lower-level attachment writer was found. Request a bounded library API rather than implementing PDF name trees/file specs in this adapter. Outline listing also has a depth-32 ceiling; explicit truncation reporting/configuration is an upstream improvement candidate.
- U11/DeckCraft writer normalization: CRLF inserted as literal native text produces an extra blank paragraph after PPTX reopen. The local authoring boundary now rejects CR/CRLF and accepts LF; a source regression verifies both rejected mutations and retained LF text. Upstream normalization consistency is a targeted candidate, not a request for a second writer.
- U01/U08/U11 diagnostics: local adapters now report detectable extraction projections from public models, including chart data omission, equation linearization and cached fields. Unsupported markup discarded before model construction remains outside these notices; reusable reader/writer preservation reports still belong upstream.

## Additional pinned-source evidence: styles, slide text and Markdown

WordCraft public style resolution, overlays and native paragraph/range editing support named paragraph/character styles and explicit overrides; this is local adapter/API work, not a missing upstream feature. DeckCraft public `edit::{insert,delete,format,text_range,props_at}` and Hyperlink/Action values support rich ranges and URL/SlideId targets, including native PPTX serialization and PDF navigation. Adapters validate offsets because the native edit functions clamp inputs, and guard dangling slide targets because the writer can omit them. No engine crate or upstream revision change is needed.

Portable Markdown media packaging is Python projection over owned extracted values; it is not an upstream format writer. It shares the existing Markdown/HTML fallback and retains source diagnostics. The next PPTX table audit confirms stored row heights are minimums expanded by public rendering layout; exact-height authoring must not be inferred from a numeric model field.

## Section and table adapter findings

WordCraft public section enumeration/mutation, part creation and native clone-on-write blocks support independent section stories without copying relationship serialization. The pinned layout treats NextColumn as a page transition; that start remains unavailable locally, and a public column-transition layout implementation is upstream work. Mixed-size continuous transitions are also rejected explicitly. General linked headers and unequal columns are not implied by the independent-section profile.

DeckCraft public table grid mutations support the local merge/split/insert/delete profile. Its public renderer expands minimum row heights; exact height must not be advertised. `deckcraft_render::table::look` resolves inherited table styles but its module is private: a bounded public resolved-cell-style accessor is a useful upstream candidate for inspection. Local explicit-border editing does not need a new library API.

## Grid and geometry findings

WordCraft public row deletion removes the row without repairing a removed vertical-merge anchor; inserted rows can retain continuation properties. A public fallible span-preserving grid edit is an upstream candidate. The local adapter currently rejects orphan continuations atomically. General rich fragment copy also needs upstream note/comment/bookmark identity remapping before supporting semantic row cloning.

PdfCraft page-box transforms, text colors and effective geometry are publicly available and integrated locally. ContentTruncated and outline_more signals now cause explicit local failures; finer caller-configurable inspection budgets remain upstream work.

## Notes and owned PDF fields

WordCraft's lower-level native parts, NoteRef objects and section NoteProps support creation and layout of footnotes/endnotes without engine dependencies. Complete paired-anchor deletion and cross-document fragment copying still need a public semantic mutation API: the current native generic delete only balances content controls, and fragment copy collects text-box parts without remapping note/comment/bookmark identities. Local authoring guards reject destructive anchored edits. Public control value/display synchronization and lock validation should move out of engine commands before FORM3 is implemented; constructing model variants alone would not satisfy editable-control behavior.

PdfCraft's public forms crate supports creation, typed filling, deletion and appearances; existing edit flatten supports verified appearance preservation. The local integration uses a bounded, script-free, single-widget creation profile. Arbitrary imported-field traversal needs a fallible caller-bounded API. Appearance generation should report overflow explicitly: current autosizing floors at four points and multiline output can omit trailing lines. The local profile uses fixed font size, native Helvetica width checks and complete-appearance postconditions; radio group/widget identity and safe removal of an empty AcroForm remain separate integration/audit work.

## Redaction and table fidelity audit

PDF7 is blocked on strict native removal semantics: pinned PdfCraft redaction retains partially intersecting vector operators behind clipping, can retain clip-and-paint geometry and clips shadings without comprehensive residual reporting. Its verification checks glyph/inline-image residue, not complete vector recoverability; stream decoding also lacks configurable aggregate limits. A nonzero paths-clipped guard alone cannot make this safe. Request strict removal, comprehensive residual/unsupported reporting and operation-wide resource limits upstream before exposing secure redaction. No redaction dependency or API has been added.

WordCraft public native table measurement/fitting is usable, but measurement stops after 4,000 direct paragraphs and must be preflighted. Patterned shading, cell spacing and complete shared-edge border conflict resolution need upstream model/layout work. Table-wide shading is serialized but not rendered at this pin; cell and conditional-style fills are available. DOCX table alternative-text title/description are modeled, while PDF Table/TR/TH/TD tagging and visible caption association are not established. These remain distinct from local direct formatting integration.

Conditional table styles are supported by public `Style::table_part`/`set_table_part` and `StyleSheet::table_style` resolution, so their local API integration requires no upstream engine or revision change. Remaining targeted upstream work includes conditional paragraph properties beyond alignment, column band widths greater than one, patterned fills and full shared-edge border conflict resolution. Whole-table style fill renders natively; the direct table-property shading gap above is separate.

PdfCraft native reading-order glyphs and line/text APIs support exact contiguous multiline annotation labels. The local adapter can connect this provenance to surviving normalized blocks without a new reading-order engine. Ambiguous overlaps, noncontiguous selections and labels inside inferred tables remain explicitly unresolved; this does not establish quadpoint grouping or semantic table-link support upstream.

All three pinned renderers expose native region primitives: PdfCraft `RenderRequest.tile`, WordCraft `render_region` and DeckCraft render offsets/size. Their local integration requires validation around native rounding/clamping, not a new rasterizer. Faithful transparency/background control needs upstream work: PDF rendering clears white and reading-color options alter content paint; WordCraft paints page color/paper, and DeckCraft paints the slide background. These are distinct from document background authoring.

Imported PDF field enumeration remains incomplete at the pinned APIs. Render inspection silently stops deep field trees, flattens multiple choice values to text and exposes only one widget. Forms enumeration has a deeper silent cutoff and skips unknown kinds without caller-supplied aggregate field/widget/string/option limits. Request a fallible bounded enumeration API with explicit completion/error, inherited typed values, every widget identity and unknown-kind reporting. Adapter postchecks after unbounded enumeration cannot establish safe complete inspection.

WordCraft tab stops, leaders and native line/page/column break markers are available in public model/layout/writer APIs. Keep their 64-stop serialization limit explicit. Bar tabs need native rendering support; current layout treats them as left tabs without drawing a bar. A native decimal-tab placement fixture shows the segment right edge at the stop and the decimal point displaced left; authoring decimal stops is rejected until native layout aligns the decimal separator correctly. Locale-aware separators and page/column break semantics inside cells/auxiliary stories need separate native evidence. Next-style metadata is public, while automatic next-style application on Enter remains in excluded editor command code; move that operation to a lower-level public library before promising editor behavior.
