# Feature roadmap

Reviewed against this repository's 0.2.1 source and current official PyMuPDF documentation on 2026-10-19. This is a planning backlog; unchecked criteria remain incomplete; implemented subsets are documented separately and are not release promises. P0 = next, P1 = following, P2 = later. “Document creation” below means modern `.docx` and `.pdf`, not legacy binary `.doc`.

Expanded with Microsoft's Open XML SDK feature families and a Pillow-like Python API direction. See [API design and source crosswalk](docs/authoring-api-design.md) for the reviewed repository, official references, draft examples, ownership rules, and backend boundary. **Only ArtCraft library crates implement document behavior.** Reference projects supply feature ideas, not runtime dependencies or copied API syntax.

## Roadmap navigation

- Existing capabilities and PyMuPDF comparison: below.
- Original priorities: R1–R4 rendering, C1–C5 creation, E1–E6 extraction, P1–P4 performance/parity.
- Expanded backlog: API, DOC, TXT, STY, TBL, BG, LAY, MED, REF, FORM, REV, PKG, PDF, SLIDE, QA.
- Delivery order and completion rules: after the expanded backlog.

The expanded items refine C1–C5 and E1–E6; they are not alternative implementations. Existing IDs remain stable. Priorities within each group override its default when explicitly marked.

Following the overlap/API review, these 139 IDs are a **coverage checklist**, not 139 APIs or separate implementation tickets. [14 canonical workstreams](docs/roadmap-workstreams.md) give every ID one owner, connect prerequisites, and mark upstream candidates. [The machine-readable mapping](docs/roadmap-workstreams.json) is the source for GitHub Project work items. Milestones, recipes and test gates remain traceable without duplicate implementation tickets.

## What already works

| Feature | Repository status | Evidence |
|---|---|---|
| Render a PDF page, DOCX layout page, or PPTX slide at a chosen DPI | Implemented for all three; returns PNG bytes | `Document.render_page`, `Page.get_pixmap`, three native render adapters |
| Structured extraction and Markdown across those formats | Implemented with documented fidelity limitations | [Capability matrix](docs/formats.md) |
| PDF → DOCX; DOCX → PDF; PPTX → PDF | Implemented conversions | `rust/src/convert.rs`, conversion reopen tests |
| Create DOCX, PDF or PPTX from user-supplied content | Initial native profile implemented on unreleased branch | [Authoring API](docs/authoring.md), installed-wheel authoring tests |
| Edit and save an existing document | Conservative PDF edit profile; Office source editing remains unsupported | `Document.edit()` passes original PDF bytes; authored models support explicit save/copy |

The installed-wheel suite checks multiple DPIs, dimensions, physical-resolution metadata, decoded pixels, negative indexes and invalid inputs across formats; PDF fixtures cover rotation, crop offsets and UserUnit. This does not establish general visual equivalence. Exact artifact and corpus evidence is recorded in [validation.md](.kiro/specs/rust-document-engine/validation.md).

## Comparison with PyMuPDF

Do not market these as features that “no app does.” The comparison supports a focused opportunity: a permissively licensed, self-contained package combining Office semantics, extraction, rendering, and eventually authoring. The first reproducible synthetic [performance baseline](docs/benchmarks.md) found PyMuPDF faster for matched PDF text and rendering operations; conversion and Office results remain capability baselines rather than direct comparisons.

| Feature | PyMuPDF baseline | Roadmap interpretation |
|---|---|---|
| PDF images at a chosen DPI | Already supports `Page.get_pixmap(dpi=...)` [S1] | Existing feature here; strengthen validation |
| DOCX/PPTX page images | PyMuPDF Office explicitly documents Office page rendering at a chosen DPI [S2] | Different packaging/licensing proposition, not an exclusive feature |
| Office layout | Current core documentation describes an HTML-based Office path that loses original page separation; recommends Office for layout fidelity [S3] | Benchmark layout and semantics; do not claim all PyMuPDF versions lack Office input |
| New PDFs, text/image insertion, page manipulation | Already supported [S1, S4] | Implemented native creation and page operations; remaining acceptance is tracked below |
| New editable DOCX documents | No general DOCX authoring API documented in the reviewed Document/Page references [S1, S4] | Candidate extension beyond that API, not a claim about every related package |
| Tables, Markdown, structured extraction | Already documented in the PyMuPDF product comparison [S3] | Improve specific fidelity gaps, not generic extraction claims |
| Permissive package license | PyMuPDF documents AGPL/commercial licensing; Office documents commercial licensing [S2, S3] | Preserve this repo's Apache-2.0 package and permissive runtime dependency graph |

## P0 — Rendering at a requested DPI

- [x] **R1: One rendering API for PDF, DOCX, and PPTX.** Keep `document.render_page(index, dpi=144)` and `page.get_pixmap(dpi=144)` returning PNG bytes. This is already implemented, not new work.
- [x] **R2: Verify DPI scaling and visible output.** Decode generated PNGs at 72, 144, and 300 DPI for all three formats. Compare width/height with page dimensions × DPI / 72 using documented rounding tolerance; verify known content is visible. Include rotated PDF pages, landscape DOCX, multiple DOCX layout pages, and nonstandard slide sizes. Cover both public rendering methods.
- [x] **R3: Consistent PNG resolution metadata.** PDF/DOCX now encode requested DPI as PNG physical resolution; PPTX retains upstream `to_png_at(dpi)`. Installed-wheel tests verify all three at 72, 144, 150.5 and 300 DPI within pixels-per-meter rounding, independently of dimensions. See `.kiro/specs/rust-document-engine/validation.md` for the unreleased working-tree artifact.
- [ ] **R4: Rendering limits and errors.** Test NaN, infinity, negative/zero/out-of-range DPI, invalid indexes, closed documents, and oversized pages for all formats. Preflight dimensions and decoded byte budgets before allocating where upstream permits; preserve stable explicit errors. A valid DPI can still exceed the output-size limit.

Already usable today:

```python
from pathlib import Path
import doc_ex

# Substitute an existing .pdf, .docx, or .pptx file.
source = Path("report.docx")
with doc_ex.open(source) as document:
    png = document.render_page(0, dpi=300)  # first page / slide
    Path("page-1.png").write_bytes(png)
    # Equivalent: document[0].get_pixmap(dpi=300)
```

DPI must be finite and in 1–1200; the default is 144. Indexes are zero-based and negative indexes are supported. DOCX uses WordCraft pagination, which can differ from Microsoft Word. The returned value is encoded bytes, not a PyMuPDF `Pixmap` object.

## P1 — Create DOCX and PDF documents

- [x] **C1: Define the authoring contract and verify upstream support.** Define a small editable lifecycle, semantic insertion primitives and properties with explicit units and bytes output. Verify pinned public creation/writer APIs before freezing signatures. Keep existing extraction APIs compatible; no parallel builder/content-schema framework. Use upstream writers and track required upstream changes separately.
- [x] **C2: Create editable DOCX from scratch.** Support paragraphs, runs, headings/styles, lists, links, images with alt text, basic tables, page size/margins, and explicit page breaks. Reopen outputs, assert order/styles/links/table content, render pages, and convert to PDF. Unsupported content must fail explicitly. Advanced headers/footers/sections can follow after the basic writer passes.
- [x] **C3: Create PDF from scratch.** Support new pages with explicit sizes, positioned searchable text, permitted font embedding, images, links, and basic vector shapes through PdfCraft. Reopen and verify dimensions, Unicode text, links, and renders. This is PyMuPDF parity. Reject unsupported glyph/font cases rather than silently dropping text.
- [x] **C4: Generate a report as editable DOCX and PDF.** Document a recipe using ordinary DOCX content primitives and existing `convert("pdf")`; do not introduce a report builder or second content schema. Preserve section order, table text exactly once, and links. Test both outputs and record layout differences.
- [ ] **C5: Fill DOCX templates.** After C2, replace placeholders even when split across runs, and support repeating table rows while preserving surrounding formatting. Define missing-value and repeated-field behavior; never execute macros or fetch external relationships. Reopen/render fixtures before promising preservation of unsupported template features. Treat as a workflow opportunity, not proven market uniqueness.

### Implemented authoring baseline (unreleased)

The installed-wheel baseline now covers typed creation/save/copy for all three formats; DOCX paragraphs/ranges, nested table cells, PNG images, page/cell backgrounds, margins and header/footer variants; PDF page organization/import, text/images and note/highlight/link editing; PPTX slide/shape creation, stacking, metadata, notes, fills and PNG backgrounds. See [the exact surface and limitations](docs/authoring.md) and [validation evidence](.kiro/specs/rust-document-engine/validation.md). C2/C3 creation and the C4 report recipe now have installed-wheel evidence, including lists/links and supplied-font PDF text/vectors. Broader TXT/TBL/LAY/MED/PDF/SLIDE criteria remain open. DOCX table widths/uniform borders/row layout and explicit page breaks are also implemented, and reopened auxiliary content is exposed in `document.stories`. Arbitrary source Office editing, full style/theme/list coverage, DOCX calculated fields/review and advanced preservation remain open. Native DOCX notes and bounded owned PDF forms are implemented. Named paragraph/character styles, explicit formatting inheritance/resets, PPTX rich ranges/links and portable Markdown media export are now validated; their broader workstreams remain open.

## P1 — Extraction fidelity and useful differentiation

- [x] **E1: Structural tables without ambiguous flattening.** Add optional recursive cell content and row/column span information while preserving the existing `Table.rows` view. DOCX and PPTX expose ordered rich cells with span/covered metadata while retaining `Table.rows`; inferred PDF tables retain documented detection limitations. Test nesting, merged cells, source order, and no duplicated text in plain text, Markdown, or conversion. For PDF detection, use a public upstream detector when available; do not build a new table engine.
- [ ] **E2: Trace extracted content back to the source.** Add stable source identifiers, paragraph/run or shape paths, exact text offsets, and geometry where upstream exposes it. Expose DOCX page spans and an explicit limitation when an automatically split paragraph cannot be mapped exactly. Verify every available locator against a rendered page. Benchmark against PyMuPDF's current structured/grounding APIs before claiming a gap.
- [ ] **E3: Complete Office semantic coverage.** Audit public upstream support for DOCX headers/footers, footnotes/endnotes, lists, tracked changes and comment anchors; PPTX internal slide links, hidden-slide filtering and note/comment anchors. Preserve ordering and distinguish body text from auxiliary content. Add one fixture per supported construct and explicitly report unavailable constructs.
- [x] **E4: Exact PDF hyperlink labels.** Recover labels and offsets from upstream text geometry, including repeated labels and multi-line links. Test alignment with normalized text and Markdown. Keep ambiguous geometry-only links explicit instead of guessing a label.
- [ ] **E5: Extraction/conversion diagnostics.** Expose structured notices for known omissions, font substitution, unsupported constructs, and approximate page mapping where detectable. Distinguish exact source semantics from inferred PDF tables and geometry. Test that unsupported conversions and resource failures remain errors; notices must not turn failures into silent success.
- [x] **E6: Portable Markdown plus media.** Export Markdown with referenced image bytes and a manifest so image references resolve without a separate caller-written inventory. Validate safe relative names and deduplicate assets without losing placement. Define nested-table fallback explicitly. Package bytes in memory; native code must not write caller-controlled paths.

## P2 — Performance and PDF parity

- [ ] **P1: Reuse parse/layout work across renders.** PDF `Document` objects now retain the private PdfCraft parser used during eager extraction and reuse it for later renders, releasing it on close; anonymous repeated-render measurements improved by 1.09×–9.16× while parser-build count remained one. DOCX still recomputes layout, first-page public latency still includes eager extraction, and peak-memory/full-document evidence remains open. Retain deterministic close/error behavior and owned extracted values.
- [ ] **P2: Render selected regions and previews.** Native clipping is implemented for PDF/DOCX/PPTX; faithful transparent/background replacement remains upstream work. Add clipping and explicit background/alpha behavior through existing render entry points where upstream permits. Specify coordinates and DPI interactions; test rotation and clipping boundaries. Thumbnail resizing remains a caller-side image recipe. These are parity/convenience features [S1].
- [x] **P3: PDF assembly.** Add merge, split, reorder, and delete operations through PdfCraft public APIs. Verify page counts, text, links, and renders after reopening. Basic assembly and reopened link/raster preservation are verified; broader outline/document-property policies remain in PDF5. This is PyMuPDF parity [S4].
- [ ] **P4: Bounded large-document processing.** Added an additive `extract_text(..., pages=...)` profile: PDF requests use one PdfCraft text renderer, skip COS table/image traversal, media transfer, JSON, and public-model hydration, and now avoid a redundant valid-document inspection parse while retaining inspection as an error fallback. Author-reported anonymous measurements put all-page text within 1.22×–2.30× of PyMuPDF and warm rendering 1.12×–2.86× faster than PyMuPDF. Current eager `open()` remains unchanged; selective structured extraction, DOCX/PPTX fast paths, peak-memory evidence, and hostile/large limit coverage remain open.

## Expanded feature parity backlog

Unchecked items below remain incomplete even when an initial slice is implemented. Public ArtCraft support at the pinned revisions must be assessed before implementation. Each item needs separate evidence for applicable read/create/edit/preserve/render/export operations. A listed Open XML feature is not evidence that WordCraft already implements it. Source families are mapped in the [reference crosswalk](docs/authoring-api-design.md#open-xml-feature-crosswalk).

### API — Python ergonomics and capability audit (P0)

- [ ] **API1: Build the parity inventory.** Map Open XML Wordprocessing feature families and relevant SDK members to user operations, existing coverage, upstream public symbols, gaps, and fixtures. Record reference versions and immutable repository commits when freezing the inventory; do not equate schema presence with working layout.
- [x] **API2: Establish the object model.** Define editable documents, sections, paragraphs, runs, tables/cells, pictures, and format-specific pages/slides. Keep extracted `Table.rows` and other existing public values compatible; editable objects must be distinct typed objects.
- [ ] **API3: Make everyday operations discoverable.** Finalize `new`, `edit`, `copy`, `save`, `to_bytes`, `add_paragraph`, `add_table`, and related methods with sensible defaults. Supply type hints, helpful reprs, docstrings, and IDE completion; avoid numeric flags and raw XML parameters.
- [x] **API4: Define units, colors, and coordinates.** Use points plus explicit `mm`/`cm`/`inches` helpers; document percentages, RGB/RGBA, theme colors, top-left coordinates, and rectangle conventions. Reject ambiguous units, non-finite values, and invalid ranges before changes are applied.
- [x] **API5: Specify collections and ownership.** Support ordered iteration, indexing and documented slicing for new collections. Define merged-cell addressing, parent ownership, detached copies, reparenting, deleted objects, and layout invalidation; ensure no dangling native references.
- [x] **API6: Specify mutation semantics.** Add operations return their new object; edits mutate that document; copy returns independent state. Validate compound edits before committing, and define failure rollback without introducing a desktop editor-state dependency.
- [ ] **API7: One support inventory and useful errors.** Use the same development ledger as API1/QA1 and publish its operation-specific support matrix. Keep a public capability-query framework deferred. Runtime unsupported operations identify what is missing; existing exceptions stay compatible.
- [x] **API8: Keep the facade independent.** Document a narrow, bounded native authoring/session interface; isolate upstream types and revision differences inside adapters. Do not mirror C# overloads or Rust structures into Python, and do not use a normalized extraction snapshot as a complete editing model.
- [x] **API9: Document image interoperability.** Use existing PNG bytes with `Path.write_bytes` or optional Pillow decoding for size inspection, thumbnails and other image operations. No additional rendering alias or result wrapper is planned; keep Pillow a caller dependency only.
- [ ] **API10: Test API usability.** Write short cookbook examples for a letter, invoice table, branded report, template fill, PDF assembly, and slide preview. Review consistency of names, defaults, units, errors, and object lifetimes before freezing the API.

### DOC — Document lifecycle and saving (P1; refines C1–C2)

- [x] **DOC1: Create a valid blank DOCX.** `new("docx")` provides a usable first section and default styles using WordCraft. Save, reopen, extract, and render an empty document without needing Word or an external template.
- [ ] **DOC2: Open for editing without changing extraction.** Add an explicit edit-copy operation using original source data and the upstream model. Verify that edits do not change the source file, original document object, or previously extracted values.
- [x] **DOC3: Save and serialize.** Support paths and binary streams plus `to_bytes()`, explicit format validation, and defined overwrite behavior. Native writers return bytes; Python owns filesystem writes. Path saves should avoid leaving a truncated destination on serialization failure.
- [x] **DOC4: Copy and close.** Deep copies own media/styles and survive source closure. Context-manager exit never implicitly saves. Document resource cleanup, reuse after save, and errors after closing editable handles.
- [ ] **DOC5: Insert, remove, move and duplicate content.** Support ordered block operations and copying between compatible document containers. Remap styles, numbering, bookmarks, notes and media IDs through upstream APIs; test collisions and ordering.
- [ ] **DOC6: Compose documents.** Append selected sections/content from another DOCX with explicit policies for styles, headers, page setup and numbering. Preserve links and footnotes; reject combinations that the upstream writer cannot preserve.
- [ ] **DOC7: Metadata and document properties.** Bounded DOCX core updates, scalar custom properties and supported core extraction are implemented. Imported custom inventory, core language and extended/application properties remain open. Read/write title, author, subject, keywords, language, core/extended/custom properties and typed custom values. Define modification-time behavior and deterministic-output options; verify saved types and values.
- [ ] **DOC8: Templates and document variants.** Instantiate supported `.dotx` templates into standalone DOCX, if upstream permits, without external template links. List supported Strict/Transitional variants; reject macro-enabled variants until explicitly supported by a separate reviewed requirement.

### TXT — Text, paragraphs and lists (P1)

- [ ] **TXT1: Paragraph/run CRUD.** Create and edit ordered paragraphs and runs, preserving significant whitespace, empty paragraphs, Unicode, tabs and line breaks. Reopen and compare exact text without duplicating table or auxiliary content.
- [x] **TXT2: Character formatting.** Add bold, italic, underline styles, strikeout, superscript/subscript, font family/size, color and highlighting. Test combinations, explicit clearing, inherited values, and the distinction between font color and shading.
- [x] **TXT3: Paragraph formatting.** Add alignment, indents, hanging indents, line spacing, before/after spacing, borders and shading. Define units and defaults; verify effects after save and in native rendering.
- [ ] **TXT4: Tabs and breaks.** Native left/center/right tabs and line/page/column breaks are implemented; decimal/bar tabs remain explicitly unsupported pending upstream rendering fixes. Add tab stops with alignment/leaders plus explicit line, page and column breaks. Ensure page/column breaks affect upstream layout rather than merely appearing as text markers.
- [ ] **TXT5: Bullets and numbering.** Support multi-level lists, custom markers, restart/continue, starting numbers and indentation. Preserve numbering identities during edits/copying; verify rendered labels and structured extraction separately.
- [ ] **TXT6: Language and typography.** Support language metadata, RTL direction, mixed-script runs, CJK settings and supported font/shaping options. Record missing glyphs and unsupported shaping explicitly; no system-font dependency or silently substituted layout claims.
- [x] **TXT7: Find and replace.** Start with literal search/replacement across runs and explicit story scope. Preserve surrounding formatting/anchors, report replacement counts and reject ambiguous edits. Callers may use Python regex for match selection over documented offsets; do not add a separate regex/template engine.
- [ ] **TXT8: Rich-text ranges.** Contiguous paragraph slices reuse revision-bound ranges for insertion/deletion, formatting and links; comments and cross-paragraph ranges remain open. Expose revision-bound ranges for styling, links, comments and removal without manual run splitting. Define Unicode offsets and explicit invalidation after incompatible edits; test repeated text, emoji, combining marks and paragraph boundaries. Avoid a live range-tracking framework.

### STY — Styles, themes and fonts (P1)

- [x] **STY1: Named styles.** Create/read/update paragraph, character and table styles with IDs distinct from display names. Validate uniqueness and references and make commonly needed defaults available offline.
- [ ] **STY2: Style inheritance.** Base/next/linked styles, document defaults and effective inspection are implemented. Default font-size reset remains explicitly unsupported because native absent-size resolution changes on DOCX reopen; missing built-in linked companions are reported and repairable. Support base styles, next/linked styles, document defaults and effective formatting inspection through upstream resolution. Reject inheritance cycles; distinguish absent properties from explicit false/reset values.
- [ ] **STY3: Theme colors and fonts.** Preserve theme references, tint/shade values, and explicit overrides. Changing a theme must affect only inheriting content; test save/render/export behavior independently.
- [x] **STY4: Reusable style recipes.** Document ordinary style values for reports/headings/tables without a preset subsystem or proprietary assets. Only bundle assets after a demonstrated need, with license/attribution and footprint evidence.
- [ ] **STY5: Font inputs and embedding.** Accept caller-provided font bytes through a bounded Python-owned read path when upstream supports them. Define embedding permission checks, subsetting, fallback diagnostics and deterministic selection; never download fonts.
- [ ] **STY6: Style import and cleanup.** Copy styles with an explicit collision policy and remove only unused definitions proven safe by upstream references. Verify formatting remains unchanged after merging or pruning.

### TBL — Full table authoring (P1; refines C2 and E1)

- [x] **TBL1: Table construction.** Create from rectangular rows or explicit row/column counts; choose a clear policy for ragged data and empty tables. Provide optional header rows and verify data order after save, extraction and conversion.
- [x] **TBL2: Row/column/cell access.** DOCX/PPTX grid insertion/deletion, ordered row/column slices and stale-handle protection are implemented; unsupported native span repairs fail explicitly. Offer `table[row, column]`, ordered row/column views and insert/delete operations. Preserve widths and spans; define how addressing works when cells are merged or structurally absent.
- [x] **TBL3: Rich cell content.** Complete for DOCX; broader PPTX rich-cell editing remains under SLIDE3. Allow paragraphs, runs, hyperlinks, images and nested tables in a cell. Keep that hierarchy editable and extractable; a plain-text convenience view must not destroy it implicitly.
- [ ] **TBL4: Cell merges and splits.** Support horizontal/vertical spans, rectangular merges, explicit split semantics and content placement. Reject overlapping or nonrectangular merges; test reversibility where content permits it.
- [x] **TBL5: Widths and layout.** Add table alignment/indent, absolute/relative width, fixed/autofit layout and column sizing through WordCraft. Specify behavior when content overflows rather than guessing widths in Python.
- [ ] **TBL6: Row heights and pagination.** Fitting rows and oversized splittable cells have visible multi-page acceptance. Oversized unmerged unsplittable rows are rejected before serialization; complete merged-row overflow handling remains upstream work. Support exact/minimum height, repeated header rows and row-splitting controls. Test multi-page tables and cells containing paragraphs spanning pages; record upstream limits.
- [ ] **TBL7: Borders and fills.** Set outer/inner/per-cell borders, line styles, colors and thickness, cell backgrounds and patterned shading where supported. Verify precedence at shared edges and consistent DOCX/PNG/PDF output.
- [ ] **TBL8: Cell spacing and alignment.** Support padding, table cell spacing, vertical alignment, text direction and paragraph alignment without conflating them. Cover RTL tables and empty cells.
- [x] **TBL9: Conditional table styles.** Support header/footer rows, first/last columns and row/column banding with documented style precedence. Test local cell overrides and removal of inherited formatting.
- [ ] **TBL10: Captions and accessible structure.** Add table title/description, header designation and caption association where upstream exposes them. Preserve these separately from visible cell text and test round trips.
- [x] **TBL11: Structured data recipes.** Demonstrate Python row comprehensions and explicit string/number/date formatting before `add_table`. Do not add dataframe adapters, formatter registries or automatic conversion of arbitrary objects to the core.
- [x] **TBL12: Table parity fixture pack.** Include an invoice, merged header, nested table, long repeating-header table, RTL table and image-containing cells. Assert semantic content once, inspect native renders, and reopen PDF exports.

### BG — Backgrounds, shading and watermarks (P1)

- [x] **BG1: Solid document background.** Add DOCX page color using the upstream native background model. Test stored color, native PNG and exported PDF; expose limitations if viewers/exporters treat it differently.
- [ ] **BG2: Background image recipe.** Reuse MED2/MED3 image placement with explicit fit, anchor, layer, offset and opacity. Preserve aspect ratio by default and verify clipping/layer order; do not introduce another image API.
- [ ] **BG3: Text watermark recipe.** Reuse MED4 positioned text/shapes and LAY3 header/section scope, with font, color, rotation and opacity properties. Verify body content remains intact; no dedicated watermark method family.
- [ ] **BG4: Image watermark recipe.** Reuse MED1–MED3 picture placement for logos/stamps with size, opacity, layering and anchors. Test asset deduplication and placement rather than building a separate watermark subsystem.
- [ ] **BG5: Section and first-page behavior.** Define supported scope using section/header anchors, including first/even/odd pages. Do not treat a computed DOCX page number as a permanent editable object; test repagination after insertion.
- [x] **BG6: Shading consistency acceptance.** TXT2/TXT3 and TBL7 own run/paragraph/cell formatting. Verify those properties share naming, inheritance and reset conventions while keeping native backgrounds and highlighting distinct; no duplicate implementation.
- [ ] **BG7: Advanced fills and page borders (P2).** Audit upstream gradients, patterns, theme fills and page border effects. Add only supported parameters and reject unsupported effects before saving; never silently flatten an editable fill.
- [ ] **BG8: Background parity tests.** Use a colored report, image letterhead, first-page-only logo and rotated watermark. Check saved DOCX, 72/144/300-DPI PNGs and PDF independently; keep preview-only background compositing separate from source edits.

### LAY — Sections, pagination and auxiliary stories (P1)

- [x] **LAY1: Page setup.** Set paper presets/custom dimensions, orientation, margins and gutter per section; use upstream layout. Test mixed portrait/landscape documents and invalid sizes.
- [x] **LAY2: Section breaks.** Add next-page, continuous, odd-page and even-page breaks as supported. Test intentional blank pages and page-count changes after editing.
- [ ] **LAY3: Headers and footers.** Create/edit/remove content with first-page and even/odd variants, linkage to previous sections and offset from page edges. Test inheritance and independently linked headers/footers.
- [x] **LAY4: Columns.** Add equal/unequal columns, gaps and separator lines. Ensure upstream lays out text in the right order and respects column breaks; do not simulate columns with spaces.
- [x] **LAY5: Flow controls.** Support keep-with-next, keep-lines-together, widow/orphan controls and page-break-before. Test headings followed by tables and long paragraphs without promising Word-identical pagination.
- [ ] **LAY6: Numbering and page fields.** Configure page-number start, format and section restart plus line numbering where supported. Verify headers/footers and PDF text contain correct evaluated values or explicit stale-field diagnostics.
- [ ] **LAY7: Footnotes and endnotes.** Create/edit references and note bodies, numbering/restart and placement. Preserve links between notes and source text when copying/deleting content; expose separate extraction scope.
- [ ] **LAY8: Layout inspection.** Expose page sizes and available block/line/page-span information from WordCraft. Clearly mark approximate spans; never infer exact paragraph slices from plain text alone.
- [x] **LAY9: Repagination after edits.** Invalidate native caches when fonts, margins, tables or content change. Rendering and conversion must observe the same document revision and report stale handles predictably.
- [ ] **LAY10: Layout settings (P2).** Inventory hyphenation, language-specific grid, vertical text alignment, mirrored margins and supported compatibility settings. Record unsupported settings and test each setting that materially changes rendering.

### MED — Pictures, drawings and advanced objects (P1; advanced items P2)

- [x] **MED1: Inline images.** Add/replace/remove images from bytes, paths or binary streams with size, preserved aspect ratio and alt text. Validate formats/dimensions before decode and retain only referenced media.
- [ ] **MED2: Floating images.** Support anchoring to page/margin/paragraph, wrapping, distances, z-order and overlap options exposed upstream. Test anchoring after text edits and image removal.
- [ ] **MED3: Image transforms.** Add crop, rotation, flip and opacity where upstream supports rendering and serialization. Keep original media and editable transforms rather than destructively resampling without request.
- [ ] **MED4: Shapes and text boxes.** Add basic shapes, lines, fills, borders and rich-text boxes with explicit placement. Use upstream geometry/rendering; list unsupported shape types and effects.
- [ ] **MED5: Groups and stacking.** Group/ungroup supported drawings while preserving local transforms, ordering and hyperlinks. Test nested groups and extraction of their contents.
- [ ] **MED6: Charts (P2).** Inventory chart creation, series/category updates, labels, axes, legend and style support. Keep cached and embedded data consistent through upstream writers; unsupported chart families stay tracked as blocked, not rasterized silently.
- [ ] **MED7: Equations and diagrams (P2).** Audit Office math and diagram/SmartArt support separately for reading, preservation, creation, rendering and export. Require public upstream implementations; no custom math layout or diagram engine.
- [ ] **MED8: Captions and media references.** Attach captions, alt descriptions and stable source identifiers; distinguish decorative images. Test picture replacement without breaking unrelated captions, links or references.

### REF — Links, bookmarks, fields and navigation (P1; calculated indexes P2)

- [ ] **REF1: External/internal hyperlinks.** Create/edit/remove exact labeled links, including bookmark targets, tooltips and links crossing runs. Never fetch the target; validate URI/relationship policy without rewriting label text.
- [ ] **REF2: Bookmarks and cross-references.** Manage named ranges and references with unique IDs. Define behavior on deletion/copying and detect dangling references before save.
- [ ] **REF3: Typed fields.** Add page/count, date, document-property, reference and merge fields through typed methods. Preserve field instructions and cached results; specify which fields can actually be evaluated by upstream.
- [ ] **REF4: Field refresh.** Offer explicit recalculation for supported fields with deterministic date/time inputs and stale-result reporting. Never run external applications or pretend that setting an “update on open” flag computes results.
- [ ] **REF5: Table of contents (P2).** Create/update from heading semantics with links, levels, leaders and page numbers when upstream layout supports it. Verify content and page numbers after repagination; otherwise expose an explicit upstream blocker.
- [ ] **REF6: Captions and lists of figures/tables (P2).** Support sequence fields and cross-references with predictable restart rules. Verify references after inserting and removing items.
- [ ] **REF7: Index and bibliography (P2).** Inventory index entries, citations and bibliography data/preservation; support generation only through verified upstream APIs. Publish supported citation/field subsets rather than promising a full Word field engine.
- [ ] **REF8: Navigation export.** Preserve heading outline/bookmark/link destinations into supported PDF exports. Verify target page/position and distinguish unsupported navigation from plain text extraction.

### FORM — Content controls and templates (P1; advanced controls P2)

- [x] **FORM1: Literal template recipe.** Use TXT7/TXT8 replacement over explicit story scopes for caller-chosen placeholders. Demonstrate missing/unused-value checks in Python and preserve runs/anchors; do not invent a template language.
- [ ] **FORM2: Repeating content recipe.** Loop over input rows/blocks and reuse DOC5 safe copy/insert primitives with reference remapping. Document empty-input behavior and input limits; no separate repeating-region engine.
- [ ] **FORM3: Basic content controls.** Expose rich/plain text, image, checkbox, date and choice controls where supported. Keep names/tags, IDs, values and placeholder display separate; validate values before mutation.
- [ ] **FORM4: Repeating/locked controls (P2).** Audit repeating-section controls, supported locking and placeholder rules. State that editing restrictions are not encryption or access security.
- [ ] **FORM5: Structured bindings (P2).** Map named data to supported content controls and custom XML through upstream APIs. No arbitrary code evaluation, external data retrieval or unbounded XPath engine in the adapter.
- [x] **FORM6: Batch generation recipe.** Use a Python loop over independent editable copies and explicit save operations, with caller-owned failure handling. Test no state leakage between copies; do not add a batch API.

### REV — Comments, revisions and review (P2; basic comments can ship in P1)

- [ ] **REV1: Comment CRUD.** Create/edit/remove anchored comments with author/date metadata and replies. Preserve exact range association across run boundaries and serialize without duplicating comment text in the body.
- [ ] **REV2: Revision inspection.** Expose insertions, deletions, moves and formatting/table changes with author/time and original/final views. Identify unsupported revision types rather than treating them as accepted content.
- [ ] **REV3: Track edits.** Record supported editing operations as revisions via WordCraft public APIs; define document-level defaults and caller metadata. Confirm both original and final content can be recovered after reopening.
- [ ] **REV4: Accept/reject revisions.** Support selected/all revisions with correct range and table semantics. Test adjacent and nested changes, note/comment anchors and repeated acceptance/rejection.
- [ ] **REV5: Review display.** Define whether rendering/export shows final text, original text, markup or comments for each backend-supported mode. Keep stored revision history distinct from displayed text.
- [ ] **REV6: Selective privacy cleanup recipe.** Compose explicit review and metadata operations, reporting what was removed and what could not be inspected. No general sanitize-everything method or exhaustive safety claim for opaque parts.

### PKG — Preservation, compatibility and safe serialization (P0 audit; P1 implementation)

- [ ] **PKG1: No-op save fidelity.** Open/edit/save without changes and compare meaningful structure, relationships, media and supported formatting. Distinguish semantic preservation from byte identity; block editing if upstream cannot retain required untouched content.
- [ ] **PKG2: Unknown feature handling.** Inventory unknown parts, extension markup, alternate content, VML and embedded objects. Preserve opaquely only when upstream safely supports it; otherwise refuse the affected save with a precise diagnostic.
- [ ] **PKG3: Relationship integrity.** Validate internal targets, IDs and references through upstream APIs after deletes/copies. Reject traversal and unsafe resource relationships; keep external hyperlinks inert and never resolve network resources.
- [ ] **PKG4: Settings and compatibility.** Read/preserve supported document settings, conformance/version metadata and custom properties. Report conversions between dialects explicitly and verify output opens without repair for tested fixtures.
- [ ] **PKG5: Protection and encryption.** Keep edit restrictions distinct from encryption. Inventory read/write password support independently for each format; do not implement unsupported encryption or imply protected content is securely encrypted.
- [ ] **PKG6: Embedded and active content.** Inventory OLE, attached templates, external links, macros and signature parts. Never execute/fetch them. Explicitly reject unsupported edits or invalidated signed outputs; adding signing or macro execution is outside scope.
- [ ] **PKG7: Output budgets and determinism.** Bound members, media, total output bytes and expansion during authoring as well as input parsing. Offer reproducible metadata/ID behavior where upstream permits; test failure before a partial destination is published.
- [ ] **PKG8: Diagnostic preservation report.** Report feature/location, affected operation and upstream limitation before a save/export that cannot meet its advertised contract. Default to failing on unsupported requested authoring or preservation, not silently omitting content.

### PDF — PDF creation and editing parity (P1 basics; advanced items P2)

- [x] **PDF1: Page authoring.** Create fixed-size pages, set boxes/rotation and insert/remove/reorder pages through PdfCraft. Reopen and verify geometry, rendered orientation and page count.
- [ ] **PDF2: Text and images.** Bounded native text boxes/alignment and visual image crop/quarter-turns/flips are implemented; supplied static TrueType boxes reject outline overflow, while boxed CFF remains unavailable. Automatic supplied-font wrapping and strict glyph-outline text clipping remain unsupported. Draw/searchable text, text boxes and embedded images with explicit positioning, fonts, colors and clipping. Validate Unicode/shaping and overflow behavior through upstream public APIs.
- [ ] **PDF3: Vector graphics and backgrounds.** Add basic paths/shapes, strokes/fills and underlays/overlays/watermarks. Preserve text extraction and layering; use PdfCraft drawing primitives rather than writing PDF operators.
- [x] **PDF4: Flowed reports and tables.** Use the WordCraft report-to-PDF path for semantic flowing content, or a verified PdfCraft layout API for positioned tables. Document which path applies; do not introduce a Python pagination/table layout engine.
- [x] **PDF5: Links, outlines and metadata.** Create/edit URI/internal links, bookmarks and supported metadata. Verify destinations survive assembly, deletion and rotation.
- [ ] **PDF6: Annotations and forms (P2).** Native notes, highlight/underline/strikeout/squiggly markup and owned text/checkbox/choice fields are implemented; imported forms, radio groups and broader appearance/reply support remain open. Inventory comments/highlights and form field read/create/fill/flatten support. Separate PDF form fields from DOCX content controls and reject unsupported field/appearance types.
- [ ] **PDF7: Real redaction (P2).** Expose only verified upstream removal of content, not a painted rectangle. Test text/image/object recoverability after saving; keep the feature unavailable until its deletion guarantees are demonstrated.
- [ ] **PDF8: Preservation and output profiles (P2).** Audit tagging, attachments, encryption, incremental save, compression and archival/accessibility profiles independently. Do not claim PDF/A or PDF/UA conformance without appropriate validators and corpus evidence.

### SLIDE — PPTX authoring consistency (P2)

- [ ] **SLIDE1: Presentation/slide creation.** Audit DeckCraft public export first; add new presentations and slide insert/copy/delete/reorder only when supported. Keep slide dimensions, names and hidden state explicit.
- [ ] **SLIDE2: Layouts, masters and themes.** Authored blank/title/title-and-content/two-content layouts and role-addressed inherited placeholders are implemented; source master/theme import and slide-copy remapping remain open. Preserve/import supported master/layout relationships and expose placeholders by role. Do not flatten editable placeholders or break theme inheritance when copying slides.
- [ ] **SLIDE3: Text, pictures and tables.** Reuse Python naming and units while retaining slide-specific geometry. Cover rich runs, grouped objects, cell merges, images, alt text and order through DeckCraft.
- [ ] **SLIDE4: Slide backgrounds and watermarks.** Support per-slide/theme background fills, images and layered marks; test PNG and PDF export as well as saved PPTX.
- [ ] **SLIDE5: Notes, comments and navigation.** Add/edit notes, comment anchors and internal/URL links where public APIs exist; retain extraction scope and source order.
- [ ] **SLIDE6: Advanced preservation.** Inventory charts, media, animation and transitions separately for preservation versus authoring/rendering. Unsupported content must remain visible in capability reports; do not add playback or application engines.

### QA — Definition of feature parity (P0 infrastructure; required for every milestone)

- [ ] **QA1: Feature coverage ledger.** Record each roadmap ID with read/create/edit/preserve/render/export status, public upstream symbol/revision, fixtures, test evidence and known limitations. An upstream blocker remains open; a fixture passing one operation does not complete the feature.
- [ ] **QA2: Authoring fixture corpus.** Generate representative letters, resumes, invoices, branded reports, forms, reviews and multilingual documents. Include combined features so tables/backgrounds/headers are tested together, not only in isolation.
- [ ] **QA3: Round-trip tests.** Create/edit, save, reopen and compare structure and content for each operation. Test repeated saves and copies, unaffected-part preservation, exact link offsets, nested tables and no text duplication.
- [ ] **QA4: Rendering/export checks.** Verify real image dimensions, DPI metadata, representative pixels and readable text at several DPIs. Reopen every advertised conversion; maintain visual baselines with documented tolerances and platform/font differences.
- [ ] **QA5: Independent validity evidence.** Use public upstream validators where available plus independent file/viewer checks for release evidence. Open XML SDK validation is a reference concept, not a required .NET dependency. Distinguish schema-valid, reopenable and visually correct outputs.
- [ ] **QA6: Error and hostile-input corpus.** Test malformed source data, invalid edits, stale handles, excessive nesting/media and save failures. Keep subprocess/network access absent and never weaken existing parser limits to achieve parity.
- [ ] **QA7: Performance and distribution budgets.** Measure parse/edit/save/render time, peak memory, wheel and extension size against the current baseline. Validate clean installed wheels across supported platforms and keep dependency/license audits current.
- [ ] **QA8: Publish honest support matrices.** Document supported operations, upstream blockers and partial fidelity per format/feature. Keep runnable cookbook examples separate from proposed API sketches; do not label the package fully Open XML compatible until the enumerated scope is verified.

## Coverage milestones

These are acceptance scenarios. Implementation ownership and prerequisites are defined by the canonical workstreams; the checklist below does not create separate tickets or additional public methods.

| Milestone | Scope | Exit criterion |
|---|---|---|
| M0: API and upstream feasibility | Compact API contract; PKG1–PKG2 feasibility; one QA ledger; R2–R4 | Small Python vocabulary, backend coverage/blocker inventory and reliable DPI rendering evidence; no rendering wrapper gate |
| M1: Useful DOCX authoring | DOC1–DOC4, LAY9, basic text/styles/tables/sections, BG1, MED1, REF1 and C4 | A styled invoice/report with a table, solid background, image, link and header/footer saves, reopens, renders and exports; safe editing is gated separately from blank creation |
| M2: Rich DOCX editing | Remaining P1 text/styles/tables/layout/media, DOC5–DOC8, FORM1–FORM3/FORM6, REF1–REF4/REF8, E1–E6 | Edit a real template while preserving untouched supported content; no silent omissions |
| M3: Advanced Word feature coverage | P2 DOCX features, REV, preservation and field calculation work | Per-feature evidence or explicit upstream blocker; no blanket completion claim |
| Independent PDF/PPTX lanes | W10 and W11; existing extraction/rendering work continues independently | Each backend ships verified capabilities without waiting for full DOCX parity or the other backend |

PKG safety/preservation requirements and applicable QA checks are release gates for **every** milestone, not cleanup deferred until M3. M1 needs safe upstream serialization even if later compatibility work remains open. Feature breadth may require dedicated upstream contributions/upgrades; do not bypass that work using another backend.

Background-image/watermark recipes additionally require MED2–MED4 placement and LAY3 scope. Calculated page fields require coordinated upstream layout/evaluation; a Python iteration engine is not planned. Validation infrastructure starts immediately and contributes acceptance checks throughout rather than blocking all implementation until its full release audit is complete.

## Delivery rules

For each unchecked feature, add concrete requirements/design details and link tests in the [active specification](.kiro/specs/rust-document-engine/tasks.md) before marking it complete. Validate against an installed wheel, reopen created/converted documents, render representative pages, and record limitations and footprint. Changes to dependencies require lockfile/license/attribution review; upstream upgrades are separate changes. No subprocesses, runtime downloads, cloud requirements, prohibited engine/application crates, or handwritten format engines.

OCR, legacy `.doc`/`.ppt`, XLSX, arbitrary new conversion pairs, signature creation, macro execution, and a desktop editor UI remain outside this backlog's implementation scope. Programmatic document editing is now explicitly in scope. Excluded features still need explicit reject/preserve behavior where they occur in input documents.

## Comparison sources

Official documentation consulted on 2026-10-19 (the live docs describe versions through 2.0). Pin the comparison version when publishing benchmark results.

- **S1:** [PyMuPDF Page API: rendering and content operations](https://pymupdf.readthedocs.io/en/latest/page.html).
- **S2:** [PyMuPDF Office: supported formats, licensing, and images](https://pymupdf.readthedocs.io/en/latest/pymupdf-office/index.html).
- **S3:** [PyMuPDF feature comparison and license](https://pymupdf.readthedocs.io/en/latest/about.html).
- **S4:** [PyMuPDF Document API: creation and page operations](https://pymupdf.readthedocs.io/en/latest/document.html).

The expanded feature inventory uses the [Open XML SDK repository and official references](docs/authoring-api-design.md#open-xml-feature-crosswalk) reviewed on 2026-10-19; API style also references [Pillow's Image module](https://pillow.readthedocs.io/en/stable/reference/Image.html). Proposed Python methods and acceptance criteria are this project's design, not copied SDK signatures or claims about existing ArtCraft support.
