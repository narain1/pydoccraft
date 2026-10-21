# Requirements: Rust document engine

## 1. In-process package

**User story:** As a Python service developer, I want document extraction to execute in-process so deployment does not require desktop applications or subprocesses.

### Acceptance criteria

1. WHEN `doc_ex` is installed from a wheel, THE SYSTEM SHALL include the native extraction engine in the wheel.
2. THE SYSTEM SHALL NOT download tools or execute PdfCraft, WordCraft, DeckCraft, LibreOffice, Pandoc, PyMuPDF, or other external document processes at runtime.
3. THE SYSTEM SHALL be licensed under Apache-2.0 and SHALL use only permissively licensed runtime dependencies with a documented locked dependency inventory.
4. WHEN a supported document is opened from a path or bytes, THE SYSTEM SHALL return an eager, owned Python document model that remains independent of temporary files.

## 2. Python compatibility API

**User story:** As an existing PyMuPDF user, I want a familiar API so migration requires minimal application changes.

### Acceptance criteria

1. WHEN the caller invokes `doc_ex.open(path_or_bytes, filetype=None, password=None)`, THE SYSTEM SHALL detect or validate PDF, DOCX, and PPTX input.
2. THE SYSTEM SHALL expose `Document.page_count`, `len(document)`, iteration, integer indexing, negative indexing, `load_page()`, `to_markdown()`, context management, and idempotent `close()`.
3. THE SYSTEM SHALL expose `Page.number`, `get_text("text"|"markdown"|"dict"|"json")`, `get_links()`, `get_images(full=False)`, and `find_tables()`.
4. WHEN a closed document is accessed, THE SYSTEM SHALL raise `DocumentClosedError`.
5. WHEN unsupported or malformed input is supplied, THE SYSTEM SHALL raise a stable `DocExError` subclass rather than a raw native exception.
6. WHEN `doc_ex.extract_text(source, pages=None)` is called, THE SYSTEM SHALL return form-feed-separated text in selected page order with negative-index support; PDF SHALL use an upstream text-only path without constructing structured public models. All formats SHALL bound explicit or all-page selection to 10,000 entries and UTF-8 output to 64 MiB including separators; Python and native iterable conversion SHALL stop at a 10,001st-item lookahead, and PDF SHALL preflight glyph text before plain-text assembly. These guarantees do not bound upstream parser/glyph-construction peak memory.

## 3. Structured extraction

**User story:** As a retrieval pipeline developer, I want one normalized representation across document formats.

### Acceptance criteria

1. WHEN a PDF is opened, THE SYSTEM SHALL preserve page boundaries and extract page text, URI links, image metadata/data when directly decodable, and table-like positioned text when detectable.
2. WHEN a DOCX is opened, THE SYSTEM SHALL preserve paragraphs, heading levels, hyperlinks, semantic tables, comments, explicit page breaks, and embedded image bytes.
3. WHEN a PPTX is opened, THE SYSTEM SHALL preserve one page per slide, ordered text, semantic tables, speaker notes, comments, hyperlinks, and embedded image bytes.
4. THE SYSTEM SHALL normalize pages to zero-based numbering and represent content as typed `Block`, `Table`, `Link`, and `Image` values.
5. WHEN Markdown is requested, THE SYSTEM SHALL emit conventional headings, links, pipe tables, image references, notes, and comments.

## 4. Conversion

**User story:** As an application developer, I want basic format conversion without installing an office suite.

### Acceptance criteria

1. WHEN `doc_ex.convert(source, "docx", filetype="pdf")` is called, THE SYSTEM SHALL return a valid DOCX containing the extracted PDF text separated by page breaks.
2. WHEN `doc_ex.convert(source, "pdf", filetype="docx")` is called, THE SYSTEM SHALL return a valid PDF containing the DOCX text with deterministic pagination.
3. THE SYSTEM SHALL expose `Document.convert(output_format)` as a convenience over the same native conversion engine.
4. THE SYSTEM SHALL reject unsupported conversion pairs explicitly.
5. Conversion SHALL preserve semantic text and page separation; pixel-perfect source layout is not required for version 0.2.1.

## 5. Footprint, safety, and performance

**User story:** As a Lambda operator, I want a small and bounded parser suitable for untrusted uploads.

### Acceptance criteria

1. THE SYSTEM SHALL avoid GUI, networking, OCR-model, JavaScript, desktop editor-state, printing, and font-discovery dependencies. Programmatic editing through the selected upstream library crates is permitted; application/editor-state crates remain prohibited.
2. WHEN parsing OOXML, THE SYSTEM SHALL reject path traversal, excessive member counts, oversized members, oversized aggregate expansion, and suspicious compression ratios.
3. WHEN parsing malformed XML, ZIP, PDF object graphs, relationships, or media, THE SYSTEM SHALL fail without writing outside owned memory.
4. THE release wheel SHALL be measured and its size reported.
5. Extraction tests SHALL run without network access, cloud credentials, or external binaries.

## 6. Verification

1. THE SYSTEM SHALL include self-contained generated fixtures for PDF, DOCX, and PPTX.
2. Tests SHALL cover path and bytes input, structured text, tables, links, images, notes/comments, Markdown, close semantics, malformed input, ZIP safety, and both conversion directions.
3. The native crate SHALL pass `cargo test` and `cargo clippy -- -D warnings`.
4. The Python package SHALL build as a wheel and pass its test suite from an isolated installation.
5. Installed-wheel validation SHALL verify imported package files against the tested artifact and record test counts, platform, artifact hashes/sizes and pinned upstream revisions. Release matrix jobs SHALL run the corpus before publication.

## 7. Page rendering (existing API)

1. THE SYSTEM SHALL expose `Document.render_page(index, dpi=144)` and `Page.get_pixmap(dpi=144)` returning encoded PNG bytes for PDF, DOCX, and PPTX.
2. Page selection SHALL use zero-based indexes with normal negative indexing; PPTX pages correspond to slides and DOCX pages to WordCraft layout.
3. DPI SHALL be finite and between 1 and 1200. Oversized output SHALL fail explicitly even when DPI is within this range.
4. Rendering SHALL use the selected upstream engine in-process with scale `dpi / 72` and require an open source document.
5. PNG physical-resolution metadata SHALL reflect requested DPI within integer pixels-per-meter rounding. Each raster SHALL be bounded to 64 million pixels and the engine's maximum side; encoding SHALL reject incomplete RGBA buffers.

The API is implemented. The current implementation adds consistent PNG metadata and multi-DPI/limit tests; broader rotated/landscape fidelity and upstream render-failure reporting remain in R2/R4. Completion evidence is recorded in `validation.md`.

## 8. Planned authoring and extraction improvements

The [feature roadmap](../../../TODO.md) records prioritized TODOs and completion criteria, not additional capabilities of 0.2.1. C1–C5 cover creation of new DOCX/PDF documents and templates; E1–E6 cover extraction fidelity and provenance; P1–P4 cover performance and parity. Initial authoring signatures have been checked against public pinned upstream APIs; the implemented profile is documented in `docs/authoring.md`. Unchecked advanced coverage remains planned. Legacy binary `.doc` creation is outside this scope.

The initial E1 extraction slice SHALL retain `Table.rows` unchanged while exposing DOCX cell blocks recursively with column/row spans and merge-continuation state. Text SHALL appear once in plain text and Markdown. HTML tables inside Markdown SHALL preserve hierarchy that pipe tables cannot express. These owned extraction values SHALL remain separate from future editable native objects.

## 9. Pythonic authoring and feature parity (planned)

**User story:** As a Python developer, I want a broad document authoring API with Pillow-like usability, using Open XML as a feature reference and ArtCraft libraries as the sole document backend.

These criteria govern future implementation, not current release capabilities:

1. THE SYSTEM SHALL use readable typed document objects, snake_case methods, keyword options, clear defaults and explicit units. Common operations SHALL NOT require XML elements, relationship IDs, numeric flags, or upstream engine types.
2. THE SYSTEM SHALL preserve the existing extraction, conversion, page indexing, owned values and bytes-returning rendering contracts. New editable objects SHALL have explicit mutation, copying, saving and lifetime semantics.
3. DOCX operations SHALL use WordCraft, PDF operations PdfCraft, and PPTX operations DeckCraft library APIs. Open XML SDK, Pillow, PyMuPDF and other reference projects SHALL NOT become fallback document backends. Missing support SHALL be recorded as an upstream requirement.
4. THE authoring backlog SHALL cover text/runs/lists, styles/themes/fonts, tables and nested cells, backgrounds/watermarks/shading, sections/headers/footers, images/drawings, references/fields/notes, templates/content controls, comments/revisions, properties and safe preservation. Detailed criteria are in the API/DOC/TXT/STY/TBL/BG/LAY/MED/REF/FORM/REV/PKG roadmap groups.
5. Table authoring SHALL include rows/columns, rich and nested cell content, merges, sizing, borders, shading, alignment and pagination controls where supported upstream. Export/extraction SHALL NOT duplicate table text.
6. Background APIs SHALL distinguish document/page/slide backgrounds, placed underlays/watermarks, object shading and preview-only compositing. DOCX scope SHALL respect reflow and section/header behavior; requested effects SHALL NOT silently disappear on save/render/export.
7. Editing SHALL operate on the original upstream document model and preserve untouched supported content. WHEN requested edits or safe preservation are unavailable, THE SYSTEM SHALL return a precise error before publishing a lossy output.
8. Capability claims SHALL distinguish read, create, edit, preserve, render and export for each feature/format in one development ledger and published matrix. A runtime capability framework is not required. Unknown or partial support SHALL NOT be advertised as full parity. Basic serialization SHALL NOT imply field evaluation or layout fidelity.
9. Authoring SHALL retain bounded, in-process, offline execution. Python MAY write explicitly requested output paths/streams; native document operations SHALL work on owned data without writing caller-controlled paths.
10. EACH advertised authoring feature SHALL have installed-wheel create/edit/save/reopen evidence plus applicable extraction, rendering and export checks. Runtime dependencies, licenses, attribution, footprint and upstream revision rules remain unchanged.

See the [API proposal and reference crosswalk](../../../docs/authoring-api-design.md). PDF/SLIDE groups extend authoring to fixed pages and slides; QA defines the operation-specific completion gates. No complete Open XML SDK member parity or Word-identical layout is asserted.

## 10. Compact API, canonical ownership and upstream work (planned)

1. Every coverage ID SHALL have one canonical implementation/workflow owner in [roadmap-workstreams.json](../../../docs/roadmap-workstreams.json). Related work SHALL reference that owner rather than create a duplicate implementation/API.
2. Reports, watermark placement, basic template substitution, repetition, batch generation, style examples and data formatting SHOULD be ordinary Python recipes over existing primitives. A new public convenience requires evidence that composition is inadequate; no report/template/batch/session framework is required by this roadmap.
3. Existing PNG rendering methods SHALL remain the rendering surface. An additional image wrapper or render alias is not planned; optional Pillow use remains caller code.
4. Format algorithms, reusable writer/layout behavior and shared semantic validation SHALL be proposed to the responsible upstream library when public support is missing. Python API naming, projection, Markdown and adapter boundary checks remain local.
5. Upstream work SHALL distinguish verified source gaps, existing public support, and unassessed candidates, with pinned evidence. Public methods being present SHALL NOT imply full correctness. Linking application engines or copying their algorithms remains prohibited.
6. The roadmap SHALL connect prerequisites at the required baseline without forcing PDF, PPTX, extraction or rendering to wait for complete DOCX parity. No contribution issue SHALL be opened in an upstream repository solely because it is marked as a local candidate.

## 11. Initial native authoring profile (unreleased)

1. `new("docx" | "pdf" | "pptx")` SHALL return an independent typed editable owner over the public pinned upstream native model. `save`, `to_bytes`, `copy`, explicit `close`, context management, PNG rendering and existing conversion pairs SHALL retain clear ownership and explicit errors.
2. Path saves SHALL serialize before touching the destination and atomically replace through a same-directory temporary file. Stream saves SHALL detect short writes. Context exit SHALL NOT save implicitly.
3. DOCX creation SHALL expose paragraphs/ranges, named default styles, tables, PNG pictures, solid page/cell fills, first-section size/margins and default/first/even header/footer paragraphs. Replacing paragraph or cell text SHALL intentionally replace that object's rich content; stale descendant/range handles SHALL fail rather than refer to unrelated content.
4. PDF creation SHALL expose bounded pages, positioned searchable text, PNG/JPEG pictures, native page organization and independent page selection/import. The initial text path SHALL explicitly reject unsupported WinAnsi glyphs and overflow. Source PDF editing SHALL refuse encrypted, form and signed/certified preservation profiles until separately supported.
5. PPTX creation SHALL expose typed text/table/image shapes, native slide backgrounds, names, notes and hidden state. Basic authored slide reorder/delete SHALL preserve remaining object identity and reject unsupported relationship profiles.
6. New source-DOCX/PPTX editing SHALL remain an explicit unsupported operation until preservation diagnostics can prevent a lossy output. Creation from an owned new model does not imply arbitrary source-package preservation.
7. Scalar lengths SHALL use finite points, with explicit `pt`, `mm`, `cm` and `inches` helpers. Positioned boxes SHALL use top-left `(x, y, width, height)`. Colors SHALL have a checked RGB representation; unsupported theme/alpha settings SHALL NOT be silently discarded.
8. All features SHALL be tested by creating native output, reopening it, checking exact semantics and applicable PNG/PDF output through the installed wheel. New dependencies SHALL retain immutable upstream revisions and an audited permissive lockfile.

## 12. DOCX auxiliary stories and layout controls

1. Extraction SHALL expose owned auxiliary story values separately from computed body pages, preserving native block order and documenting part-ID ordering when original cross-part order is unavailable. Headers/footers SHALL appear once per native part, with no fabricated page assignment.
2. Story tables, links and images SHALL reuse existing normalized values; auxiliary asset identities SHALL include part ID and source path. Footnote/endnote body references SHALL retain exposed part IDs and Unicode offsets. Legacy comment page output SHALL remain compatible and be identified in the story metadata.
3. Boundary traversal SHALL bound story counts, nested tables, grids, objects, text and referenced image projection before constructing unbounded normalized output. Unsupported or excessive content SHALL fail with existing stable exceptions.
4. DOCX table layout SHALL expose checked column widths, uniform solid borders, row height rules, repeated header prefixes and row splitting. Paragraph spacing/page-break-before and explicit page breaks SHALL be delegated to native model/layout and verified in PDF pagination.

## 13. Text, vector content and bounded diagnostics

1. DOCX paragraphs SHALL support native bullet/numbered list membership, levels and explicit restart; revision-bound text ranges SHALL support checked URL links without changing labels or Unicode offsets. All text-size arguments SHALL use `font_size` in the public Python authoring API.
2. PDF creation SHALL accept bounded caller-supplied permitted fonts through PdfCraft registration/shaping/embedding, preserving searchable Unicode and rejecting missing glyphs or overflow. Basic vector paths SHALL use native content operations with explicit top-left coordinates and checked bounds.
3. PDF metadata and hierarchical bookmarks SHALL reuse native APIs, support atomic validation and explicit stale-handle behavior. Child collections SHALL remain usable after their own edits that preserve parent identity.
4. Reading an absent DOCX header/footer SHALL return an empty collection without creating a part, changing section flags or invalidating text ranges. Creation SHALL occur only on insertion.
5. Native extraction SHALL expose bounded structured diagnostics for detectable omitted or approximated model constructs. Unknown markup discarded upstream cannot be diagnosed by inspecting the resulting model; diagnostics SHALL NOT imply complete preservation or downgrade errors.
6. Python input reads and native document entry points SHALL enforce a 128 MiB input limit. Normalized JSON serialization SHALL enforce a 64 MiB encoded-payload limit before growing beyond it. These limits SHALL NOT be advertised as a bound on all parser/layout memory.
7. Report generation SHALL remain a recipe over DOCX authoring and existing PDF conversion. Reopened outputs SHALL preserve order, links and table text exactly once.

## 14. Named formatting, slide ranges and portable Markdown

1. DOCX SHALL expose named paragraph/character styles with stable IDs distinct from names, checked same-kind inheritance and direct/resolved formatting snapshots. Duplicate or ambiguous names, cycles, excessive chains and oversized patches SHALL fail atomically.
2. Formatting SHALL distinguish omitted properties (unchanged), None (clear direct override) and false (explicit disable). Omitted run properties SHALL inherit; table-cell spacing inspection SHALL include native table-style context. Public font-size and underline vocabulary SHALL be consistent across supported formats.
3. PPTX text shapes SHALL support revision-bound Unicode ranges, literal replacement across runs/paragraphs, formatting and URL/internal slide links through public DeckCraft edit algorithms. Appending SHALL preserve appearance without inheriting adjacent links. Deleting referenced slide targets SHALL fail before publication; reorder SHALL retain native target identities.
4. PPTX mutations SHALL bound visible text, paragraphs, runs and duplicated string properties before expensive native splits; only approved URL and Slide actions SHALL be constructed. Unsupported actions and properties SHALL fail explicitly.
5. `Document.export_markdown()` SHALL return owned file bytes for body Markdown, separate available stories, deduplicated referenced media and a placement manifest. It SHALL use generated safe relative paths, retain every placement, share the existing nested-table HTML formatter and preserve `to_markdown()` compatibility.
6. Markdown export SHALL bound returned bytes, estimated escaping expansion, content nodes and recursion; missing referenced media and cycles SHALL fail with existing stable exceptions. Native code SHALL NOT write output paths, and this export SHALL NOT claim source-layout reproduction or HTML sanitization.

## 15. Sections, table structure and ordinary recipes

1. DOCX SHALL expose stable append-only section handles with native start types, page size, margins, header/footer distances and bounded equal columns. New sections SHALL receive independent copies of story content, including explicit empty stories for missing variants, so later edits cannot accidentally inherit earlier headers.
2. Unsupported next-column and mixed-size continuous transitions SHALL fail explicitly. Body table/image width validation SHALL use the owning section/column or cell; header/footer validation SHALL use its section's full content width.
3. DOCX/PPTX cell margins SHALL use top/right/bottom/left point order with explicit reset. Cell alignment SHALL expose uniform direct paragraph overrides, reject mixed values and leave nested-table paragraphs unchanged.
4. PPTX table merge/split and row/column CRUD SHALL call public native algorithms, preserve anchors and shape identity, emit covered text once and invalidate old row/cell handles after structural changes. Partial overlaps and final-row/column deletion SHALL fail atomically.
5. PPTX row heights SHALL be identified as minimums, not exact dimensions. Grid extents SHALL remain synchronized with stored shape geometry; margins, aggregate cells and scalar dimensions SHALL be checked before publication. Inherited/mixed border inspection SHALL fail explicitly when native resolution is unavailable publicly.
6. Literal-template and batch examples SHALL compose existing copies, explicit story scopes, replacement and saves with caller-owned failure handling. Missing/unused values and replacement counts SHALL be checked; source templates SHALL remain unchanged after success or failure. These recipes SHALL NOT imply arbitrary source-DOCX preservation.

## 16. Grid structure and PDF geometry

1. DOCX row/column edits SHALL use native table operations, stage mutations atomically and preserve valid spans, placeholders, width constraints and repeated-header prefixes. Unsafe vertical-merge continuations SHALL fail explicitly. Structural edits SHALL invalidate affected positional and descendant handles while retaining the table and unrelated handles.
2. PDF page-box inspection/editing SHALL use native geometry with top-left view coordinates, rotation and UserUnit. Non-media boxes SHALL fit within media; media edits MAY clip existing effective boxes but SHALL reject empty intersections. Geometry mutations SHALL refresh cached sizes and invalidate stale handles/background knowledge.
3. Both PDF text paths SHALL support checked RGB color. Known native content truncation SHALL raise the stable resource-limit error; editable imports SHALL reject incomplete outlines.

## 17. Native notes and owned PDF fields

1. DOCX SHALL create footnote/endnote stories anchored at the end of a revision-checked same-owner body range, preserving its text. Notes SHALL reuse editable paragraphs and native numbering/layout; body-table anchors SHALL be covered by round-trip/render tests. Unsupported story anchors SHALL fail atomically.
2. Generic mutations SHALL reject destruction or duplication of existing semantic markers until a public native semantic-removal operation is integrated. Editable text SHALL retain native object replacement characters in Unicode offsets. Note parts and anchor bytes SHALL count toward bounded budgets.
3. Section note options SHALL distinguish omission from clearing inheritance, validate supported numbering/placement/restart combinations and reject out-of-range starting numbers rather than relying on native clamping.
4. Newly created PDF models and their full copies SHALL support bounded text, checkbox and choice fields through native forms APIs, with one widget per field, immutable unique names and strict typed values. Imported models SHALL NOT acquire trusted ownership merely by loading bytes.
5. Field edits SHALL be staged; unsupported glyphs, clipping/overflow, incomplete appearances, unsupported actions and invalid geometry SHALL fail explicitly. Flatten SHALL verify complete removal of terminal fields/widgets and preserve visible content within documented measured raster precision.
6. Field creation SHALL retain the pinned upstream revision and permissive dependency graph. Page organization SHALL validate supported owned fields before committing; active-field subsets SHALL remain unavailable until identity-remapping semantics are validated.

## 18. Table formatting and outline-aware imports

1. Table construction SHALL accept bounded row/column counts as a convenience over the same native rectangular-grid constructor. Python row/column slices SHALL return ordered checked cell handles with existing merged-anchor and structural-generation semantics.
2. DOCX direct table/cell formatting SHALL use bounded atomic native property patches, distinguish inheritance from explicit border suppression, and validate effective geometry rather than only declared grid widths. Metadata SHALL remain separate from visible cell text.
3. Content fitting SHALL call public native measurement/autofit functions, reject the native measurement cutoff and overflowing minimum widths, and never implement an adapter text-width algorithm. Unsupported table shading/spacing/border-fidelity cases SHALL stay explicit.
4. Optional PDF outline import SHALL call the public native file-page insertion operation and preserve supported destinations/structure under a named container. It SHALL reject incomplete inspection, unsupported source semantics and post-copy discrepancies before committing. Duplicate and omitted selections SHALL follow documented native destination mapping.

## 19. Conditional styles and multiline PDF labels

1. Named DOCX table styles SHALL share existing style IDs, same-kind inheritance and bounded patches. Whole-table and conditional regions SHALL use public native model/resolution, preserve explicit false/clear semantics and reject unsupported region properties atomically.
2. Table assignment and look flags SHALL reuse table formatting. Native region precedence and direct cell overrides SHALL be verified in serialized DOCX, PNG and exported PDF; header styling SHALL remain independent of repeating header rows.
3. Style/base changes SHALL validate all affected table geometry, including nested tables without registered Python handles, before committing. Existing section/grid/alignment validation SHALL use the same effective native geometry checks.
4. PDF multiline link labels SHALL preserve native text order, exact Unicode offsets, spaces and newlines. Only unambiguous consecutive surviving paragraph spans SHALL be coalesced; neighboring links SHALL be remapped without changing plain text or duplicating table content.
5. Overlapping, noncontiguous, empty or ambiguous labels SHALL remain explicitly unresolved. Link association work and intermediate indexes SHALL remain bounded; no custom column-reading or paragraph-layout algorithm SHALL be introduced.

## 20. Native region rendering and DOCX tabs/breaks

1. Existing PNG rendering methods SHALL accept an optional displayed-page `(x, y, width, height)` clip in points for PDF, DOCX and PPTX. Full-page behavior SHALL remain unchanged when omitted. Native region primitives SHALL render only the requested intersected raster; adapters SHALL NOT allocate the full-page raster to crop it afterward.
2. Clip validation SHALL reject invalid/empty geometry and enforce quantized raster limits before rendering, preserve requested DPI metadata and verify returned dimensions. PDF rotation/CropBox/UserUnit and native edge rounding SHALL be covered. Unsupported transparency/background replacement SHALL remain explicit upstream gaps.
3. DOCX paragraph and supported style formatting SHALL accept bounded tab stops with native alignment/leader semantics, inherited-stop clearing and direct/resolved snapshots. Writer precision and direct/effective stop limits SHALL be validated before commit; unsupported decimal/bar tabs SHALL fail explicitly.
4. Paragraph break insertion SHALL reuse native line/page/column markers and existing revision-bound Unicode ranges. Page/column breaks SHALL be limited to verified top-level body contexts; all invalid locations/limits SHALL preserve state atomically. Reopened DOCX/PDF and actual page/column placement SHALL be tested.
5. Region transforms MAY use the already-locked permissive geometry crate directly, with unchanged upstream revisions. Lockfile/license evidence SHALL be regenerated and reviewed even if the graph adds no package.

## 21. Style relationships and document defaults

1. Existing Style handles SHALL expose native next-paragraph and reciprocal paragraph/character link relationships with same-owner, kind and reference validation. Next-style self-reference/cycles SHALL remain distinct from prohibited base-style inheritance cycles.
2. Linked-style replacement/clearing SHALL update both ends atomically and reject an occupied target without disconnecting unrelated styles. Native serialization SHALL preserve relationships; the API SHALL NOT claim automatic editor Enter behavior.
3. Existing Styles SHALL expose direct document defaults and bounded character/paragraph formatting patches. Native resolution SHALL preserve omission/clear/false semantics and named/direct overrides; conditional/table-only/default-style-kind operations outside the supported profile SHALL fail explicitly. Clearing default font size SHALL fail atomically until native absent-size resolution matches DOCX readback; explicit sizes and named-style clearing remain supported. Undefined built-in linked companions SHALL be reported explicitly and permit safe clearing/replacement.
4. Default changes SHALL validate affected native table geometry and effective tab limits, invalidate derived snapshots and preserve the prior document on failure. Native DOCX roundtrip, PNG/PDF appearance, copies, stale/closed ownership and invalid references SHALL be tested.

## 22. PDF drawing resource preservation

1. Drawing text, paths, images or backgrounds SHALL preserve existing effective page Font, XObject and ExtGState bindings, including inherited resources.
2. A native resource collision SHALL fail explicitly before committing the staged document or changing counters/cached output. Native resource inspection and bounded unchanged-value checks SHALL be used; the adapter SHALL NOT rewrite PDF resource names/operators.
3. Tests SHALL demonstrate pinned native collisions and verify adapter rollback, unchanged original rendered regions, normal repeated drawing and resource-budget failures. Dependencies SHALL retain the pinned revision and permissive graph.

## 23. DOCX pagination controls and visible page bounds

1. Flow-control acceptance SHALL verify headings followed by tables, keep-together paragraphs, both widow/orphan edges and page-break-before reset using native/PDF pagination, ordered exactly-once text and visible rendering.
2. Table acceptance SHALL distinguish splitting cell paragraphs, moving fitting rows intact and oversized unsplittable rows. Serialization SHALL reject proven off-page geometry for unmerged non-splitting, exact-height and repeated-header rows before publishing output.
3. Failed save/render/export SHALL preserve existing destination files and permit recovery by changing content, geometry or row splitting. Later font, margin and text changes SHALL be rechecked. Intentional exact-height clipping and vertically merged geometry SHALL remain distinct; incomplete upstream coverage SHALL stay documented.

## 24. PDF text areas and image transforms

1. Existing draw_text SHALL accept exactly one point or bounded rectangle, retaining existing point behavior. Native built-in fonts SHALL wrap and align; supplied fonts SHALL preserve searchable Unicode through explicit lines with supported alignment.
2. Native logical metric overflow, invalid geometry, unsupported glyphs and excessive wrapping work SHALL fail atomically. Supplied static TrueType box text SHALL additionally reject shaped outline overflow using bounded public native glyph inspection. Boxed CFF evaluation SHALL fail explicitly until aggregate work can be bounded; legacy point placement remains unchanged. Built-in box fonts retain logical metrics, and automatic caller-font wrapping and strict clipping remain unavailable.
3. Existing add_image SHALL accept bounded visual crop fractions, quarter-turns and flips through native transform properties, preserving source media and existing page resources. Rotation, CropBox and UserUnit SHALL preserve displayed-point semantics.
4. Tests SHALL verify native wrapping/alignment, explicit-font text, rollback, work limits, asymmetric crop/flip/rotation pixels, outside-box pixels and extraction after serialization. PDF2 SHALL remain open for documented upstream gaps.

## 25. DOCX paragraph border formatting

1. Existing paragraph, paragraph-style and document-default formatting SHALL accept supported four-edge borders with shared color/style/width vocabulary. Direct and effective snapshots SHALL expose native whole-group inheritance and distinguish omission, per-side clearing and group reset.
2. Table-style border meaning SHALL remain unchanged. Shared DOCX border validation SHALL reject proven rendering substitutions/clamps and invalid vertical double/wave edges before mutation; PPTX rules SHALL remain independent.
3. Native DOCX readback and visible PNG/PDF tests SHALL cover body, cells, stories, style/default inheritance and atomic invalid changes. Between/bar and other unsupported native borders SHALL remain explicit upstream gaps.

## 26. Ownership and shading consistency acceptance

1. Ordered collection iteration SHALL capture handles before yielding, preserving stable identities and generation checks across structural edits. A pending positional handle SHALL NOT be rebound to the current generation after its original object moved or disappeared.
2. The API SHALL document live-handle slices, parent ownership, closed/deleted/stale access, independent whole-document copies, unsupported detached child copying/general reparenting and layout invalidation. Existing format-specific operations SHALL remain the only supported exceptions.
3. Shared run/paragraph/cell background formatting SHALL verify direct clearing restores native inheritance, independently from text highlight and page background. Native DOCX properties and actual PNG/PDF fills SHALL agree without adding a second shading API.

## 27. Inline picture replacement and removal

1. Existing picture handles SHALL expose owned source bytes and accept a replacement through the existing bounded bytes/path/stream reader. Replacement SHALL preserve size and alt text and invalidate derived outputs only after native validation succeeds.
2. Existing document/cell image collections SHALL support indexed deletion of one inline placement. Native paragraph deletion SHALL preserve the anchor paragraph and unrelated IDs; deleted handles SHALL fail explicitly.
3. Native media insertion SHALL deduplicate and allocate collision-safe keys. Commit SHALL retain media referenced by supported authored stories, including nested tables and auxiliary parts, and recompute distinct encoded-byte and placement budgets. Unsupported object profiles SHALL fail before cleanup rather than risking unrelated resources.
4. Tests SHALL verify replacement/removal, shared resources, copy independence, reclaimed budgets, save/read/render consistency and atomic failure; no new image-processing backend SHALL be introduced.

## 28. DOCX core and typed custom properties

1. Existing metadata/update_metadata naming SHALL expose supported native core values and a nested scalar custom mapping. Patches SHALL preserve omitted entries, distinguish clear/reset, preserve custom value types and native name identity, and reject ambiguous or unsupported values atomically.
2. Core whitespace and timestamp input SHALL be validated against native readback behavior. Saving SHALL retain explicit timestamps/revision without automatically changing them. Source extraction SHALL expose the supported core fields; broader imported custom/language/extended properties remain separately scoped.
3. Python SHALL bound scalars and aggregate encoding before constructing the native JSON patch; Rust SHALL bound input, stored property bytes and counts. Tests SHALL verify native XML/readback, public extraction, copy/snapshot ownership, rollback, capacity reuse and unchanged content/rendering.

## 29. PDF text-markup kinds and appearance budgets

1. Existing annotation operations SHALL support native underline, strikeout and squiggly kinds with current-view coordinates, metadata changes, movement, recoloring, deletion and reopened persistence. New authoring SHALL use one quadrilateral and reject unsupported resizing/target changes.
2. Appearance work SHALL be bounded before native property inspection or regeneration. Direct native QuadPoints SHALL have bounded counts and finite geometry; newly generated appearance bytes SHALL count toward document mutation budgets. Deletion SHALL not rebuild an appearance merely to identify its subtype.
3. Tests SHALL verify actual pixels across rotations/CropBox/UserUnit, preserved page text, cache invalidation, reopened metadata, stale handles and atomic budget/geometry failures. Broader PDF6 form/import/reply criteria SHALL remain open.

4. Imported text-markup mutation SHALL reject quadrilateral coordinates outside ±1,000,000 PDF user units before native appearance evaluation, preventing nonfinite derived geometry; removal SHALL remain available without rebuilding the appearance.

## 30. Section binding space and unequal columns

1. Existing section properties SHALL expose a left gutter and explicit column widths through native SectionProps/Columns. Column widths SHALL match the configured count, retain the shared gap/separator, and fit usable page width without clamping or scaling. Clearing widths or assigning the existing columns tuple SHALL restore equal columns.
2. Native layout SHALL position and paginate text across columns and explicit breaks. Table/image validation SHALL conservatively use the narrowest body column until upstream reflows continued tables across changing widths; header/footer content SHALL retain full usable width. Geometry changes SHALL validate existing nested content atomically.
3. Tests SHALL cover native DOCX readback, mixed page geometry, actual text coordinates and PNG/PDF pixels, ordering, continuation, cache invalidation, copies, invalid widths and rollback. Mirrored/top gutters and per-column gaps remain explicitly outside this profile.

## 31. Native authored slide layouts

1. Existing slides.add SHALL accept the verified blank, title, title_and_content and two_content native layouts. Other known layouts SHALL fail explicitly until their fidelity is verified, and unknown names SHALL fail validation. Native constructors SHALL own layout/master relationships and fresh object identities.
2. Existing shapes SHALL expose a read-only placeholder role and native resolved box/rotation. Text edits SHALL preserve inheritance; geometry edits SHALL materialize only the affected transform while retaining unrelated transform values and placeholder references. No separate placeholder collection or layout engine SHALL be introduced.
3. Native ID/content budgets SHALL be checked before mutation. Tests SHALL cover actual placeholder text in native PNG/PDF, saved PPTX relationships, unchanged masters, geometry edits, ownership and atomic invalid/limit failures. General slide copy/import and layout/theme editing remain separately scoped.

## 32. Combined table fixture acceptance

1. The installed-wheel corpus SHALL generate invoice, merged-header, nested, long repeated-header, RTL and image-cell fixtures through existing public APIs. No fixture-specific runtime methods or external processors SHALL be added.
2. Saved DOCX and exported PDF SHALL reopen with matching page counts and exactly-once body content. Semantic table spans/order SHALL remain distinct from rendered RTL placement and intentional repeated PDF headers. Every rendered page SHALL have checked visible representative fills; nested/image source pixels SHALL survive export.
3. The fixture index SHALL record covered combinations and distinguish this acceptance from broader table fidelity, accessibility and import-preservation criteria.

## 33. Owned presentation picture editing

1. Existing slide image insertion/shape collections SHALL return an owned picture handle sharing data/alt vocabulary with DOCX pictures and existing shape geometry/link/order behavior. Source replacement SHALL preserve all placement properties and unrelated shared picture/background references. Slide identity SHALL be resolved after caller source/coordinate conversion so callbacks cannot redirect placement after reordering.
2. Picture crop SHALL use checked native LTRB fractions in 0..0.45; None SHALL reset. Boolean flips SHALL use native transforms together with existing rotation. Original bytes SHALL remain available, and native serialization precision SHALL be documented.
3. Input validation, native media deduplication/pruning and live encoded-byte budgets SHALL precede staged commit. Tests SHALL verify source/XML preservation, asymmetric transform pixels in PNG/PDF, source kinds, ownership, cache invalidation, reclaimed/shared media and atomic invalid/limit failures.

## 34. Contiguous paragraph and slide text selection

1. Paragraph and text-bearing shape slicing SHALL return their existing revision-bound ranges through existing native range operations. Normal Python Unicode-character slice normalization SHALL apply; integer indexing and steps other than one SHALL be rejected. Empty forward slices SHALL select an insertion point.
2. Existing range text/format/link operations SHALL retain their semantics, stale-owner checks and semantic-anchor guards. Removing all text SHALL preserve the paragraph; no DOCX cross-paragraph range, new Run class or duplicate insertion/deletion method SHALL be introduced. Slide ranges SHALL retain their existing LF-separated paragraph behavior, internal links and tooltips; table indexing SHALL remain unchanged.
3. Tests SHALL cover Unicode/whitespace, clamped/negative bounds, insertion/deletion, surrounding formatting, links, copies, auxiliary/cell scopes, invalid inputs, mutation during index conversion, semantic-anchor rollback, slide ordering/placeholder preservation and unsupported nontext selection.

## 35. Native DOCX numbering extraction

1. Existing paragraph metadata SHALL expose effective native numbering identity, level, parsed format/template and optional logical layout label without prefixing paragraph text or changing link offsets. Cleared numbering SHALL be absent; missing definitions SHALL remain null rather than guessed.
2. Native style/table resolution and layout placements SHALL own numbering semantics. Labels SHALL attach to the segment containing the native label line; body, auxiliary stories and nested tables SHALL retain separate identities. Conflicting repeated labels SHALL be omitted, and empty numbered paragraphs SHALL remain represented.
3. Definitions, observed paths, placements, individual strings, output metadata and adapter-native definition lookup work SHALL be bounded. Repeated and missing definition results SHALL be cached without replacing native resolution. Tests SHALL cover continuation/restart, all nine levels, styles, start overrides, breaks/wrapping, stories, symbol fonts, RTL logical labels and native reader normalization. Broader list authoring and exact automatic page slicing remain separately scoped.

## 36. PPTX fixed internal navigation extraction

1. Existing Link values SHALL normalize resolved native slide actions to one-based #page=N URIs and zero-based target_page metadata in current presentation order, including hidden/self targets. Source page, exact Unicode label offsets, table source paths and grouped-shape traversal SHALL remain unchanged.
2. A bounded native SlideId index SHALL be shared by payload projection and diagnostics. Duplicate native identities SHALL fail explicitly. Contextual actions SHALL remain diagnosed omissions rather than invented fixed destinations; native reader discards SHALL be documented as upstream gaps.
3. Tests SHALL cover source relationship variants, reorder/hidden/self links, repeated labels, table/group links, URLs, omission diagnostics and model bounds. No raw XML link parser or new public navigation API SHALL be introduced.

## 37. Explicit native list starts

1. Existing Paragraph.set_list SHALL accept an optional non-boolean integer start in 1..32767 for top-level numbered lists. Supplying start SHALL allocate a fresh native abstract/instance sequence and level-zero start override; omitted start SHALL retain existing nearest-sequence continuation. No existing paragraph SHALL be reassigned implicitly.
2. Unsupported list kinds/levels and invalid integers SHALL fail before mutation. Supplied starts in header/footer stories, including nested cells, SHALL fail explicitly until native measurement preserves the requested visible counter. Existing omission behavior SHALL remain unchanged.
3. Native list-definition budgets, story identity, copies, closed/deleted handles and atomic failures SHALL remain enforced. Tests SHALL verify saved native overrides, extracted labels, visible PDF text, body/cell/note scope, continuation and rollback without a local counter algorithm.

## 38. PPTX source slide-count preflight

1. PPTX input SHALL reject more than 10,000 source slide references before the native reader silently truncates them. The check SHALL use bounded generic XML events, retain native prefix/encoding recognition, and avoid reproducing private package relationship resolution or alternate-content selection.
2. A conservative count of every sldId element beneath each presentation root SHALL cover alternate branches and section lists as well as the primary list; unused presentation parts and redundant references count toward this stricter profile. Other-format/detection preflight SHALL retain its existing path. Binary media SHALL NOT be rejected merely for not being XML.
3. Eligible member inflation, UTF-16 decoding, prolog, token and nesting work SHALL be bounded. Tests SHALL cover 10,000/+1, relocation and unusual part names, UTF-8/UTF-16, leading text, alternate content, large media and all public processing paths. A shared XML parser is a safety dependency only; document semantics remain in DeckCraft.

## 39. DOCX picture crop and mirroring

1. Existing Picture handles SHALL expose crop and boolean horizontal/vertical flips using the same vocabulary as presentation pictures. Crop SHALL use finite LTRB fractions in 0..0.45 with None reset; original media and uncropped frame size SHALL remain intact. Replacement SHALL retain transforms.
2. Native public image/Float properties SHALL own serialization and rendering. DOCX frame dimensions below the native one-point minimum SHALL be rejected on insertion/resize instead of silently enlarged. Rotation SHALL remain unavailable until its native edge-loss fixture is fixed; no local rotation/resampling engine SHALL be added.
3. Tests SHALL verify native readback, original bytes, actual asymmetric PNG/PDF pixels, replacements, copies, cell/story containment, cache invalidation, deleted/closed handles and atomic invalid values. Existing media and geometry budgets SHALL remain enforced.

## 40. Owned PDF field geometry

1. Existing PdfField.box SHALL be writable in current-page displayed coordinates. Moving/resizing SHALL retain native field/widget identity, name, value, options and appearance orientation; it SHALL NOT transfer the field to another page or broaden the single-widget owned-form profile.
2. Native public field properties and appearance regeneration SHALL operate on staged state. Bounds, current value and all choice-option fit SHALL be validated before commit. Failed geometry/resource changes SHALL preserve the document and caches; caller coordinate conversions SHALL finish before final stale-handle validation.
3. Tests SHALL cover text/checkbox/choice widgets, actual old/new pixels, native save/readback and flattening, rotation/CropBox/UserUnit, copies, warmed caches, stale/deleted/closed handles, callback changes and atomic fit failures.

## 41. Private Office render sessions

1. Opening DOCX/PPTX SHALL retain the native parsed model used for eager extraction and, for DOCX, its completed layout. Existing public rendering methods SHALL reuse that state without reparsing or repagination. No public session class or additional render vocabulary SHALL be introduced.
2. Shared checked render helpers SHALL preserve one-shot PNG output, clipping, DPI/dimension/index errors and source preflight. DOCX retained state SHALL reject more than 10,000 laid-out pages or one million placed items after native layout; these are not peak-layout allocation guarantees. Temporary layout caches SHALL not be retained.
3. Close SHALL serialize with render, release owned native state and private source/password references, and be idempotent. Already extracted values SHALL remain owned and usable. Python projection failure SHALL close unreturned sessions even if its traceback survives. Global/thread-local renderer caches are outside the session lifetime guarantee.
4. Validation SHALL prove one native parse/layout, mixed-page full/clip PNG parity, source lifetime, concurrent close, resource/error behavior and installed-wheel acceptance. Separate-process synthetic measurements SHALL distinguish open, cold/warm/multi-page latency and native RSS/peak RSS. Spec92 supplies repeated large/image-heavy synthetic workload evidence across all three formats and closes the bounded P1 reuse criterion. These are representative workload shapes, not a representative real-world corpus; broader platform/corpus validation and allocation limits remain QA7/P4 work.


## 42. Decimal page and count fields

1. Paragraph.add_field(kind) SHALL append native page, page_count or section_page_count fields and return an ordinary revision-bound TextRange. Native instructions SHALL explicitly request decimal formatting; arbitrary field instructions, local evaluation and implicit section-format inheritance SHALL not be exposed.
2. Existing Section.page_number_start SHALL accept None for continuation or a non-boolean integer in 1..32767 for restart. Newly appended sections SHALL continue numbering unless explicitly restarted, even if their predecessor restarted. Continuous sections with explicit restarts SHALL be rejected because native layout ignores that combination. Changes SHALL be staged and invalidate derived output atomically.
3. Native insertion/removal and range formatting SHALL preserve field-marker identity. Arbitrary caller text SHALL not forge object markers. At most 10,000 native fields SHALL be allowed across authored stories, including copied headers/footers. Empty cached source values SHALL remain distinct from native layout-evaluated PNG/PDF values; no writer-side cache synthesis SHALL be added.
4. Tests SHALL verify body/table/header/footer/note fields, totals and section restarts after repagination, serialization, actual displayed output, copies, source extraction, safe range deletion/formatting, bounds, stale/deleted/closed handles and callback edits. Page fields SHALL compose with authored images in either insertion order, including media replacement, deletion and copy cleanup; non-media field markers SHALL not prevent referenced-image accounting. Section-page-count fields in continuous-section scopes SHALL be rejected because native counting excludes a shared first physical page; ordinary page/total-page fields remain supported there. Nondecimal inherited format inconsistencies and native numeral clamps SHALL remain documented upstream limitations.

## 43. Imported DOCX property and table metadata

1. Existing document.metadata['custom'] SHALL expose the same strict scalar profile as authored properties: native lpwstr, bool, i8 and finite r8 values. Conversion SHALL share a private helper. Unsupported/malformed variants and ambiguous ASCII-case-insensitive property names SHALL produce bounded omission diagnostics; no conflicting value SHALL silently overwrite another.
2. Before cloning values, extraction SHALL enforce 1,024 parsed properties, 64 KiB per value and 256 KiB aggregate raw metadata. Diagnostic provenance SHALL identify bounded property positions without copying arbitrary raw XML or large untrusted property strings.
3. Existing extracted Table.metadata SHALL carry native title/description separately from visible cell content, including nested and auxiliary-story tables. These strings SHALL count toward the existing extraction text budget and preserve absent versus explicitly empty values. No duplicate writable API SHALL be added; table.format already owns creation.
4. Tests SHALL verify authored/imported types, escaping/Unicode, duplicate/malformed/unsupported values, exact bounds, owned snapshots, nested/story table metadata and unchanged text/Markdown/render semantics. Native property-reader omission/truncation and 10,000-character table alternative-text truncation SHALL be documented; this inventory SHALL not claim complete source-property preservation.

## 44. Bounded PDF page selections

1. Existing PDF copy and page insertion SHALL consume at most 10,000 selected page indexes plus one unconverted lookahead item. They SHALL share the existing selected-text iterator normalization, retain Python integer-index semantics and reject integers outside the native signed-index range. Selection order, duplicates and negative indexes SHALL remain supported.
2. Before caller-controlled index conversion, source reads or selection iteration, each operation SHALL capture the destination page generation and count. After normalization it SHALL recheck open state and reject structural changes before subset/import dispatch. Source snapshots SHALL remain usable if a distinct source subsequently closes; ordinary content/metadata edits SHALL not be rejected as page organization. Caller callback effects cannot be rolled back, but the outer operation SHALL perform no additional subset/import.
3. Tests SHALL cover finite/infinite selections, the exact cap and unconverted excess item, invalid/oversized integers, ordered duplicate/negative selections, callback mutations/close and unchanged output after rejected operations. No new public selection API or local PDF algorithm SHALL be introduced.

## 45. Executable usability cookbook

1. Six short workflows SHALL demonstrate a letter, invoice, branded report, authored template filling, PDF assembly and slide previews through the existing public API. Examples SHALL be import-safe, accept explicit output directories and selectable DPI, use ordinary Python value formatting, and require no runtime document executables, downloads or optional dependencies.
2. The existing template example SHALL retain its explicit two-placeholder/scoped replacement contract. Cookbook functions SHALL remain ordinary example code rather than additional exported library methods or a recipe framework. Saved native documents, PDF conversions and PNG previews SHALL use existing save/convert/render operations.
3. Installed-wheel tests SHALL execute the actual functions and CLIs, reopen every advertised document format, render all pages and verify relevant text order, links, table data, backgrounds, visible pixels, dimensions and resolution metadata. Existing specialized tests SHALL retain ownership of semantic edge cases.
4. Documentation SHALL review names, defaults, units, unsupported profiles, snapshots, stale handles, independent copies and explicit save/close behavior. Historical design sketches SHALL be clearly distinguished from current executable examples. Completing this current-profile API10 gate SHALL not close the broader parity ledger or representative corpus requirements.

## 46. Native DOCX image backgrounds

1. Existing WordDocument SHALL expose background_image using the presentation property vocabulary: owned PNG bytes or None on read, bounded supported source input or None on assignment. Native PagePicture frame fill SHALL stretch the image over every page, independently of the background color; transparent pixels reveal that color. This profile SHALL NOT imply aspect-preserving fit, tiling, section-local placement or watermark fidelity.
2. Native public media insertion and staged commit SHALL own serialization, rendering and resource references. The single global background reference SHALL share existing distinct-resource/encoded-byte accounting with inline and story images, retain shared resources and reclaim only unused media. It SHALL remain separate from inline image collections.
3. Existing 16 MiB PNG input, 8,192-side/16-million-pixel decoding and 64 MiB aggregate referenced-media bounds SHALL remain enforced. Invalid media, missing native references and resource failures SHALL leave committed state unchanged. Caller stream reads SHALL remain bounded and recheck the owner's lifetime; independent owned bytes and copies SHALL survive closure.
4. Tests SHALL verify source readback, mixed page dimensions, native/PDF pixels, transparency over color, DPI/clipping, warmed output, fields/notes/nested-image composition, replacement/clear/delete/copy, resource reclamation and atomic failures. Reproduced native watermark and tile divergences SHALL remain upstream requirements rather than local rendering compensation.

## 47. Rich presentation cell paragraphs

1. Existing editable Cell.paragraphs and Cell.add_paragraph SHALL work for authored PPTX tables through the existing Paragraph/range vocabulary. Paragraphs SHALL support text, direct alignment, appended runs, contiguous Unicode slices, literal find/replace, character formatting and URL/internal-slide links with tooltips. Unsupported Word-only styles/lists/fields/breaks/spacing/resolution and nested images/tables SHALL fail explicitly. Single-paragraph edits SHALL reject LF; new paragraphs SHALL use add_paragraph.
2. Native public TextBody and edit operations SHALL own text changes and formatting. Covered coordinates SHALL resolve to their merge anchor. Paragraph handles SHALL retain a bounded cell body generation, table generation and ordinal; whole-cell text replacement and grid organization SHALL invalidate them, while append-only paragraphs and slide reordering SHALL preserve prior paragraph identities. Text ranges SHALL retain existing document-revision invalidation.
3. Empty native bodies exposed by splitting SHALL accept their first paragraph at ordinal zero without a leading separator. Existing empty paragraphs SHALL remain present when another paragraph is appended. Native text/run/paragraph/property and aggregate-cell budgets SHALL remain enforced before cloning/inserting and on staged commit; per-cell generation entries SHALL be pruned with table organization/deletion and owner close.
4. Tests SHALL verify saved PPTX/PDF semantics, actual formatted pixels/links, local paragraph alignment, merged aliases, Unicode offsets, copies/reordering, stale/closed/callback behavior, exact budget failures and empty/split/grid cases. Native row fitting SHALL remain distinct from stored geometry; merged-row/vertical-cell overflow and broader slide content remain open. No exported paragraph subclass, local text-layout algorithm or second range API SHALL be introduced.
5. Shared literal find/replace SHALL operate on base string values after strict type checks, without invoking subclass truth/length/equality/conversion hooks during range creation or staged edits. Regression coverage SHALL include DOCX paragraphs, slide shapes and slide-cell paragraphs.

## 48. Everyday API typing and handle representations

1. Existing factory/context/copy chains SHALL retain their concrete successful owner types. Common creation, cell merge, page import, bookmark and annotation methods SHALL describe their returned handles. Page, editable PDF page and slide get_text overloads SHALL distinguish string modes from dictionary output, retaining a union for dynamic mode strings. Runtime arguments, defaults, exports and native behavior SHALL remain unchanged.
2. Existing Paragraph, EditableTable and Style representations SHALL use bounded stored identity and owner closed state without resolving native content, copying text or suppressing unexpected errors. Private paragraph subclasses without a DOCX identifier SHALL remain representable after invalidation and closure.
3. A pinned development-only static consumer SHALL verify representative public chains against the installed wheel, including table indexing and PPTX cell paragraphs. Runtime tests SHALL verify large/stale/closed representations. This selected consumer gate SHALL NOT claim full-package strict typing or completion of broader API3 discoverability.


## 49. Bounded operation evidence ledger

1. One development ledger SHALL associate bounded format profiles with explicit read/create/edit-authored/edit-imported/preserve/render/export statuses. Supported cells SHALL cite scope, exact test selectors, fixture provenance, native public symbols at pinned revisions and a named historical artifact. Upstream-blocked cells SHALL cite a concrete upstream register entry and native constraint; unsupported and unassessed SHALL remain distinct.
2. Every existing roadmap ID SHALL join to profiles or an explicit remaining audit statement. Ownership, role and acceptance checkboxes SHALL come from existing roadmap files, without duplicating a capability database or treating a checked criterion as support for all operations. Recipes/gates without an audited profile SHALL remain unassessed rather than receiving invented format verdicts.
3. An offline standard-library checker SHALL validate coverage joins, references, native pins, evidence fingerprints, selector existence, artifact-section hashes and exact generated output. It SHALL NOT infer semantic correctness, refresh historical evidence, add runtime capability APIs or claim complete Open XML/PyMuPDF parity. The initial seven-profile inventory SHALL retain broader API1/API7/QA1/QA8 work as open.


## 50. Pinned reference-family discovery

1. A development inventory SHALL group representative Open XML SDK and PyMuPDF members into existing feature families, with immutable source URLs, digests, line references and explicit local/remaining scope. It SHALL distinguish reference capabilities from tested local behavior, separate core and Office products, and avoid uniqueness or exhaustive-parity claims.
2. Family mappings SHALL reference existing roadmap IDs, upstream candidates and evidence profiles. Owner and reverse criterion joins SHALL be derived from existing data rather than maintained twice. No reference SDK/application/runtime dependency or corresponding public class-per-member hierarchy SHALL be introduced.
3. Offline checks SHALL reject unknown/duplicate mappings, floating or mismatched snapshot URLs, invalid source extents and stale generated output. Optional explicit cached-source validation SHALL check exact hashes and member lines without downloads. Full member dispositions and behavioral parity SHALL remain open.


## 51. Rich header/footer content

1. Existing EditableStory SHALL expose direct tables/images collections and add_table/add_image using the existing table, cell, paragraph and picture handles. Missing-story reads SHALL not create a part. Notes SHALL reject rich content explicitly, with closed-owner errors taking precedence.
2. Native insertion SHALL use public WordCraft containers, distinguish synthetic trailing anchors from authored empty paragraphs, and preserve existing locators, source order and independent section/document copies. Picture removal SHALL retain its empty anchor as an ordinary story paragraph. Existing aggregate text/cell/media and registered-location limits SHALL apply.
3. Serialization SHALL reuse the existing bounded native layout inspection to reject header/footer table/picture frames outside the physical page, respecting explicit native exact-height clipping. Geometry-dependent operations on inactive variants SHALL fail explicitly and atomically when native measurements are unavailable. This check SHALL not claim full text-ink containment or preallocation layout bounds.
4. DOCX PNG rendering SHALL disable native editor-only header/footer dimming in shared full/region and retained/one-shot paths. Tests SHALL verify native source ordering, variants, rich cells, fields/media composition, copies, callback closure, retained anchors, geometry failures and actual PNG/PDF colors. No new runtime dependency or native object type SHALL be introduced.


## 52. Bounded native line numbering

1. Existing Section.line_numbers SHALL accept None or (count_by, start, distance_points, restart). Counts SHALL be non-boolean integers in 1..100 and 1..32768; finite distance SHALL be 0..1584 points, with zero selecting the native default. Restart SHALL reuse each_page/each_section/continuous names. Native model/writer/layout SHALL own all numbering, placement and source serialization.
2. Numbered sections SHALL be single-column. After earlier numbering, a continuous each_page section SHALL require an immediately preceding each_page section, preventing verified stale-counter behavior. All mutations and serialization SHALL recheck the supported combinations atomically. Native defect repros and mixed-section positive controls SHALL be retained.
3. Before either authored-document full-layout validation path, numbered top-level body paragraphs SHALL remain within 100,000 work units: four per UTF-8 source byte, one per paragraph and one per inline object. Suppressed paragraphs SHALL still count. This additional label-work bound SHALL not be represented as a total-memory guarantee.
4. The existing serialization layout pass SHALL inspect actual native line-label logical extents and reject off-page/wrapped labels, preserving explicit errors and existing page/item limits. Tests SHALL verify source/PNG/PDF output, automatic pages, restarts/intervals, tables/suppression, copies/reset, callback closure, bounds and rollback. Semantic DOCX extraction SHALL not invent layout labels; PDF ordering/classification SHALL remain native.


## 53. Incremental operation evidence

1. New tested profiles SHALL join the existing operation ledger and reference families without duplicating ownership or broad completion state. Header/footer content and line numbering SHALL cite their distinct exact historical artifacts and fingerprinted tests.
2. Supported read/create/edit/preserve/render/export claims SHALL remain separate; unassessed auxiliary extraction, unsupported imported editing and known native fidelity limits SHALL remain explicit. Reference members SHALL retain immutable source citations and shall not stand in for local capability tests.


## 54. Native paragraph direction and run language

1. Existing DOCX paragraph formatting SHALL accept direction rtl/ltr/None, preserving explicit LTR versus inherited direction. Existing character formatting SHALL accept language/None, with nonempty ASCII alphanumeric subtags separated by single hyphens and at most 63 bytes. No full BCP47, font selection or spelling guarantee SHALL be implied.
2. Native property inheritance SHALL apply through existing styles/defaults and owned paragraphs/ranges in body/cell/story/note scopes. Whole-table paragraph direction and conditional table character language SHALL reuse current native dispatch; conditional paragraph direction SHALL remain rejected. Direct/effective snapshots SHALL expose the fields. Table visual order and vertical cell orientation SHALL remain distinct.
3. Native layout SHALL own bidirectional ordering and logical start-edge alignment/indent/tab behavior; adapters SHALL not rewrite coordinates, source Unicode or alignment. Tests SHALL verify native glyph geometry, actual PNG/PDF placement, source roundtrip, defaults/styles/reset, copies, callbacks and atomic invalid values. Broad TXT6 script/font/diagnostic scope SHALL remain open.


## 55. Bounded geometric PDF annotations

1. Existing annotations.add SHALL accept rectangle/ellipse, reusing annotation properties/update/deletion and native Square/Circle shapes. Native set_rect SHALL own resizing. The initial profile SHALL be solid/unfilled/opaque with a two-user-unit stroke and dimensions at least three native user units. Current view coordinates/rotation/CropBox/UserUnit SHALL remain authoritative.
2. Imported mutation SHALL validate a bounded canonical target dictionary before native props/appearance work, rejecting unsupported styles/insets/actions/replies/extensions without decoding foreign appearance streams. Metadata-only edits SHALL retain the normal appearance; geometry/color edits SHALL explicitly regenerate native standard appearance. Deletion SHALL remain available for rejected edit profiles.
3. Generated appearances SHALL have conservative preflight allowance and actual staged raw-byte accounting, including superseded intermediate streams. Target mutation bounds SHALL not be represented as bounds on preexisting global summaries or initial native inspection. Existing input/item caps SHALL remain.
4. Facade annotation add/add_link/update SHALL preserve initial closed/stale checks and recheck after callback-capable normalization before dispatch. Tests SHALL verify actual pixels, original content, rotation/CropBox/UserUnit, canonical/rejected imports, minimum size, budgets/copies/stale handles and callback rollback.


## 56. Direction and geometric-comment evidence joins

1. The existing operation ledger SHALL include distinct DOCX direction/language and PDF geometric-comment profiles, with exact historical artifacts, test fingerprints and native public symbols. TXT6/PDF6 SHALL retain their existing owners and unchecked broader scope.
2. Direction metadata/source preservation SHALL remain distinct from broad glyph/shaping or PDF-tagging guarantees. Geometric same-format save evidence SHALL remain under preservation, not a new conversion claim; custom appearances, opacity/fill and earlier inventory allocation limits SHALL stay explicit.


## 57. Owned DOCX named ranges and internal navigation

1. WordDocument.bookmarks SHALL provide indexing/assignment/deletion, length and source-ordered name iteration. Lookup SHALL return the existing revision-bound TextRange. Names SHALL be case-sensitive ASCII identifiers of at most 40 characters beginning with a letter, with at most 1,000 pairs. Existing range.link SHALL accept a matching #name; no new bookmark handle, link-target hierarchy or generic mapping update API SHALL be added.
2. Targets SHALL cover a whole owned body paragraph or an empty point at its start/end, including supported horizontal nested cells. Interior targets SHALL fail explicitly until native marker-transparent shaping exists. Fresh lookup SHALL resolve current native marker paths after grid changes. Marker offsets SHALL remain native object characters. Native insertion/deletion SHALL own the pair; relocation may select only its own validated markers, and generic semantic protections SHALL remain. Cross-paragraph/auxiliary/imported targets and selected other semantic/evaluated-field anchors SHALL fail explicitly.
3. All-story references, unique ordered pairs and paragraph-endpoint placement SHALL be checked centrally before every staged commit and output. Logical endpoint checks SHALL discount only bookmark markers in one ordered scan. Later edits that would make markers interior SHALL fail atomically, while whole-target replacement and paragraph formatting remain available. Deleting a referenced name SHALL fail atomically. Explicit pair deletion SHALL have only a validated name-specific semantic exception. Image/media cleanup SHALL permit only the two added media-free bookmark variants; unrelated unknown objects SHALL remain unsupported.
4. Shared native layout SHALL validate actual physical-page carets, linked source orientation, unsupported linked display wrappers and a conservative 10,000 nonempty linked glyph batches per page. All links and repeated stories SHALL count. Pair creation/movement and internal-link assignment SHALL stage these checks; later layout edits SHALL fail explicitly on output and permit recovery. Native preallocation, complete glyph containment or arbitrary transformed navigation SHALL not be claimed.
5. Tests SHALL verify Unicode/source ordering, source link offsets, pair movement and deletion, native PDF destinations after repagination, nested-cell annotation geometry, copies/grid staleness/media/fields/notes, callback closure, counts and saved-file atomicity. Native missing/header targets, clipped hit areas, lost vertical links and silent annotation caps SHALL have direct repros.
6. Bookmark layout validation SHALL conservatively bound cumulative native caret lookup and adapter geometry/display scans before repeating them, including repeated destination resolution during PDF export. At-limit/+1 and repeated-target regressions SHALL preserve revision/content on refusal and demonstrate recovery; this SHALL not be claimed as a preallocation bound on native layout or global document inventory.


## 58. Native PDF object-stream output choice

1. Existing editable PDF to_bytes/save SHALL accept keyword-only object_streams with strict bool values and default True. Other formats SHALL keep their signatures. Both modes SHALL delegate to the same pinned native full writer; no optimizer, registry, compression algorithm or new backend dependency SHALL be introduced.
2. False SHALL select native direct objects/classic xref output, with current owned-field/import restrictions and output limits. Rendering/conversion/internal serialization SHALL retain default behavior. The option SHALL not claim content-stream recompression, smaller files, incremental save, PDF-version downgrade or archival/accessibility conformance.
3. Shared atomic-path/stream save behavior SHALL remain, including suffix checks, file mode, short writes and callback ordering. Closed-owner checks SHALL precede option/destination processing; invalid values SHALL not coerce truthiness. Serialization SHALL preserve model/handle/cache state and use the same postallocation returned-output bound in both modes.
4. Native/public tests SHALL verify representation, repair-free reopen, actual pixels/text/images/fonts/links/outlines/metadata/fields, default equality, deterministic repeat/copy behavior, imports/rotations, both-mode limits, failures and unchanged other-format saves.


## 59. Owned header/footer clearing

1. Existing header/footer containers SHALL expose clear() returning None, using native Document.set_story while retaining explicit section references, flags, distances and page settings. Missing or already-cleared synthetic stories SHALL be exact no-ops, including revisions, ranges and cached snapshots. Authored empty paragraphs SHALL count as real content. Note clearing and shared-part aliases SHALL fail explicitly.
2. A real clear SHALL stage existing semantic, internal-reference, layout, field and media checks before commit; only removed story descendants and their generations/placeholders SHALL be invalidated. IDs SHALL remain monotonic. Independent copies and unrelated child handles SHALL survive, with existing global TextRange revision semantics unchanged.
3. Text/cell/media resource reclamation SHALL match insertion accounting. Only actual image object markers SHALL be excluded from text usage; page-field, note and bookmark markers SHALL remain charged when copying or removing content. Shared media SHALL remain while referenced by other stories or backgrounds.
4. Tests SHALL verify actual source/PNG/PDF content removal, rich ordering, variants/copies, no-op cache/range preservation, stale children, protected-anchor rollback, outgoing links, closed/note errors and exact field/cell/media accounting at limits. General block adoption, linkage and DOC5/LAY3 completion SHALL remain open.


## 60. Bounded native PDF text search

1. Read Page and editable PdfPage SHALL expose search_for(text) returning owned rectangle tuples in displayed page points. Native PageText matching, reading order, case/whitespace handling and line_rects SHALL remain authoritative. No matcher, coordinate transform, result class or normalized-offset reconstruction SHALL be added.
2. Live/stale ownership SHALL be checked before query work and rechecked after callback-capable page-number normalization. String subclasses SHALL not override validation operations. Invalid Unicode SHALL fail explicitly. DOCX/PPTX pages SHALL reject search geometry; empty ASCII-whitespace PDF queries SHALL return an empty list without rendering or creating an editable snapshot.
3. Query bytes SHALL be capped at 4,096 before native interpretation. Subsequent search SHALL preflight one million glyphs/scalar slots, four MiB aggregate source text before character scans, one million selected-glyph visits, and 10,000 matches/output rectangles. Invalid native ranges/index vectors/nonfinite or reversed rectangles SHALL fail explicitly. Existing native truncation failures SHALL propagate; no preallocation bound on parsing/text interpretation SHALL be claimed.
4. Tests SHALL verify actual rotation/CropBox/UserUnit geometry and pixels, ligatures, native Unicode/whitespace, ActualText, mirrored text, native column order, clipped/invisible text, exact resource boundaries, owned lifetime, query callbacks, caches, copies and explicit Office errors. E2/REF-PM-EXTRACTION SHALL retain their existing owners and broader incomplete scope.


## 61. PDF page identity across caller callbacks

1. Existing PdfPage drawing, image insertion, page-box, background and rotation mutations SHALL check owner/lifetime before normalizing arguments and recheck page generation after all callback-capable normalization, immediately before native dispatch. A callback that moves or deletes pages SHALL not retarget an existing handle or refresh it onto a different page.
2. Rotation SHALL normalize integer-index arguments before borrowing the native owner. Existing accepted integers, validation errors, native rendering/serialization and ordinary geometry-handle refresh SHALL remain compatible. No new public method or native operation SHALL be introduced.
3. Callback effects SHALL remain caller-owned: the requested operation SHALL add no mutation after invalidation, and callback exceptions SHALL propagate without an implicit rollback of the callback itself. Tests SHALL compare bytes and rendered pixels after move/delete callbacks, preserve closed/stale precedence and verify independent copies and unchanged ordinary drawing behavior.


## 62. Native decimal-outline lists

1. Existing Paragraph.set_list/list_style SHALL accept decimal_outline through native ListKind::Legal, levels 0..8 and existing restart/start semantics. Top-level starts SHALL be non-boolean integers in 1..32767; fresh starts/restarts SHALL create independent native definitions. No new list handle, property, local counter or label generator SHALL be added.
2. Supported-kind identity SHALL compare all nine effective native levels, distinguish Legal from ordinary Numbered, and preserve Num.start_overrides continuation. Repeated definition lookup SHALL use operation-local caching and bounded work; existing bullet/numbered behavior SHALL remain compatible.
3. Decimal outlines SHALL be limited to top-level horizontal body paragraphs in single-column non-continuous sections not immediately followed by a continuous section. Cells and auxiliary stories SHALL fail explicitly. Assignment and later section changes SHALL validate the profile atomically; shared serialization layout SHALL recheck actual native source-section ownership and label/first-line logical body containment after formatting changes.
4. Target source work SHALL be bounded before layout; observed label/glyph/caret work SHALL be bounded before scans, with finite ordered geometry and explicit missing/ambiguous-placement errors. These bounds SHALL not claim native preallocation limits or arbitrary-font tight-ink containment.
5. Tests SHALL cover hierarchy, all nine levels, maximum starts, continuation among ordinary lists, restarts/copies, RTL/empty/wrapped text, source numbering roundtrip, actual PNG/PDF label ink, unsupported scope, callback closure, malformed geometry, atomic refusal and output recovery. Broader TXT5 custom markers and list layout remain open.


## 63. Stable presentation targets during caller normalization

1. Slide.background and Shape.rotation/fill/box/color/font_size/add_run SHALL validate initial owner/target identity before callback-capable normalization, then resolve the stable slide ID immediately before native dispatch. A surviving PPTX handle SHALL continue addressing the same logical slide/shape after reorder; deleted or closed targets SHALL fail explicitly.
2. Existing native stable shape IDs and mutation-time checks SHALL remain authoritative. No public handle type, native operation or callback transaction framework SHALL be introduced. Callback changes and exceptions SHALL retain their current semantics; failed requested operations SHALL not add mutations to a different target.
3. Generic TextRange.format SHALL reject an already-closed or revision-stale range before normalization using the existing owner revision, without copying selected text. Native final revision/target validation SHALL remain, rejecting range-invalidating callbacks atomically.
4. Tests SHALL cover matching/different shape positions, stable IDs, exact control serialization and PNG/PDF pixels, deleted/closed targets, copies, callback errors and appended-range revision/formatting. Range controls SHALL distinguish expected post-callback stale refusal from incorrect retargeting across DOCX paragraphs and PPTX shapes/cells.


## 64. Explicit conditional vertical-border limitations

1. Existing DOCX conditional table-style formatting SHALL reject non-null assignments whose mapped vertical edge is discarded by the pinned native renderer: row-region inside_vertical; first-column/left-corner right; last-column/right-corner left; column-band left/right. Visible borders and explicit style=none suppression SHALL both reject before commit.
2. Side/group None resets, supported outer/horizontal conditional borders and direct-cell/direct-table/whole-table formatting SHALL remain available. Acceptance SHALL not depend on current one-column table attachments, since styles may later be reused on wider tables. No new border API, local conflict-resolution algorithm or renderer compensation SHALL be added.
3. A raw-native regression SHALL retain serialized/readback definitions and actual placed-rule/PNG/PDF evidence of the omission, bypassing the local guard. Public tests SHALL verify atomic compound refusal, range/cache/copy preservation, reset/inheritance and direct controls. TBL7 SHALL remain partial for native conditional-edge fidelity, shared-edge conflicts and patterned shading.


## 65. Owned PDF field tooltips

1. Existing PdfField SHALL expose a writable tooltip string for owned text/checkbox/choice fields, with absent metadata represented by an empty string and empty assignment clearing it. None/non-string values, NUL, malformed Unicode and values exceeding 4096 UTF-8 bytes SHALL fail explicitly. Whitespace/newlines and Unicode metadata SHALL be preserved independently of painted-value font restrictions.
2. Python SHALL validate owner/structural identity before processing input and bound base-string length before copying/encoding, bypassing subclass conversion callbacks. Native validation SHALL repeat byte/NUL limits; all tooltip bytes SHALL count toward the existing 4 MiB aggregate owned-field text budget.
3. Mutation SHALL delegate to public native FieldProps.tooltip/set_props on a staged clone and compare complete field state against the expected tooltip-only metadata change. Since native set_props regenerates appearances, existing conservative peak reservation and actual raw appearance accounting SHALL apply before commit. Successful updates SHALL preserve handle generations and invalidate cached snapshots normally; failures SHALL preserve document bytes/state/cache.
4. Tests SHALL verify native metadata roundtrip, clear/removal, all three owned kinds, values/options/flags/widgets, exact visible-output preservation, four rotated CropBox/UserUnit profiles, copies/resize/value/flatten and lifetime/budget failures. Imported AcroForms, radios and general widget lifecycle SHALL remain outside this profile.


## 66. Bounded PDF comment editing locks

1. Existing Annotation SHALL expose a strict writable locked boolean for the seven supported comment kinds. Links and foreign kinds SHALL fail explicitly. Initial owner/generation checks SHALL precede validation; arbitrary truth-value conversion SHALL not run.
2. Mutation SHALL use public native set_locked on a staged document and compare the bounded target dictionary against the expected flag-only change. AP references, geometry, metadata and other flags SHALL remain unchanged. Existing geometric validation SHALL admit the canonical Locked bit without relaxing style/shape bounds.
3. Before native annotation-array/dictionary cloning, the setter SHALL enforce the existing 10,000-entry reference-only page array limit and a bounded canonical target profile. Unsupported nested/reply/flag/style/appearance combinations SHALL reject; metadata/quadrilateral limits and document mutation budgets SHALL remain authoritative. Global native imported summaries and initial inspection remain upstream limitations.
4. Locked geometry/color/author/deletion requests SHALL fail atomically through native semantics; comment text SHALL remain editable, and unlocking SHALL restore supported edits. This flag SHALL not be described as encryption, document-level protection or arbitrary reply-thread security.
5. Tests SHALL cover all seven kinds, actual pixels, four rotated CropBox/UserUnit profiles, saved native source, copy/lifetime behavior, unchanged AP and complete target state except F, compound rollback, malformed/unsupported imported profiles and exact resource boundaries.


## 67. Incremental navigation, lifecycle and output evidence

1. Existing operation evidence SHALL add bounded profiles for DOCX endpoint/whole-paragraph bookmark navigation, owned header/footer clear, PDF object-stream serialization, native PDF search and DOCX decimal outlines. Joins SHALL reuse current coverage IDs, owners and reference families without changing criterion completion. The explicit unsupported redaction profile SHALL remain present.
2. Artifact citations SHALL identify the exact validated runtime/wheel and reviewed test-file fingerprints. Authored bookmark lookup SHALL remain distinct from imported source inventory; same-format serialization SHALL be assessed as preservation rather than conversion. Search rectangles SHALL not imply visible-only matching, normalized source offsets or OCR. Narrow list-label evidence SHALL not certify auxiliary Office coverage.



## 68. Native owned-note kind conversion

1. Existing WordDocument.notes SHALL expose convert(kind="footnote" | "endnote") returning the number changed, delegating to public WordCraft Document::convert_notes. No swap mode, per-note conversion framework or local marker/counter algorithm SHALL be added. Destination section note options SHALL remain authoritative; part IDs, paragraph paths, content, custom styles and unrelated references SHALL be preserved.
2. Input SHALL validate the owner before bounded literal string normalization without subclass callbacks. An empty/already-matching collection SHALL preserve native revision, ranges and cached rendering. Actual conversion SHALL stage once, retain stable handles and apply normal range/cache invalidation only after success.
3. Preflight SHALL bound aggregate native paragraph/object/run comparison work, including the reference-count × run-count scan, to two million units under existing document budgets. The native style operation SHALL reject mixed-content standard-style runs if converting a marker could restyle ordinary text, other objects or unconverted markers; custom styles and runs containing only converted markers SHALL remain supported.
4. The private conversion commit path SHALL require the exact full semantic-anchor multiset differing only in expected existing note kinds, unchanged part IDs and correct destination kinds. All existing field/navigation/media and output-geometry guards SHALL remain. Missing, duplicate, malformed or unrelated anchor changes and unsafe destination field scopes SHALL fail atomically.
5. Tests SHALL cover raw native source/readback, standard/custom styles, body/nested references, actual PNG/PDF placement/counters and long-note continuation, shared-run failure/recovery, copies/stable handles/no-op ranges/cache, bounded inputs/work and rollback. Broad LAY7 semantic deletion and cross-container adoption SHALL remain open.

## 69. Scoped field-tooltip and comment-lock evidence

1. The existing owned-field evidence profile SHALL incorporate tooltip read/edit/preservation/render assertions against the exact completed tooltip artifact, preserving its historical creation citation and unsupported imported/conversion scope. Evidence SHALL not claim imported tooltip inventory from saved-output rendering alone.
2. One orthogonal comment-lock mutation profile SHALL reuse PDF6 and the existing annotation reference family without replacing geometric creation/style evidence. Imported coverage SHALL distinguish the dedicated canonical rectangle fixture from saved/reopened native seven-kind output. Creation and conversion cells SHALL remain not applicable for this flag-only profile.
3. New test fingerprints and artifact citations SHALL match reviewed assertions and exact validated source snapshots. Existing historical artifacts, coverage IDs, owners and completion flags SHALL remain unchanged. Generated support/reference guides SHALL stay consistent; broad form/annotation, security and imported-inventory claims SHALL remain excluded.


## 70. Owned PDF field read-only state

1. Existing PdfField SHALL expose a strict writable read_only boolean, initially false, for owned text/checkbox/choice fields. Only the native READ_ONLY flag SHALL extend the existing canonical field settings; imported forms, multiwidgets, radio, scripts and other flags SHALL remain outside the profile.
2. Mutation SHALL use public native FieldProps.read_only/set_props on a staged document, reuse existing appearance peak/raw-byte accounting, and compare the complete field with an expected flags-only change. Widget identity/geometry, values/defaults/options, tooltip and other settings SHALL be retained. Regenerated AP references need not remain identical.
3. Valid value assignments SHALL be refused by native filling while read-only, including identical values and clearing. Existing earlier input/fit errors SHALL remain compatible. Unsetting read_only SHALL restore filling; tooltip/geometry changes, copies, deletion and flattening SHALL remain available. The property SHALL not be described as security or viewer-only enforcement.
4. Initial lifetime/structural checks SHALL precede strict bool validation; no truth conversion SHALL execute. Existing model/text/input/mutation/output limits and stale-handle behavior SHALL remain. Failure SHALL preserve bytes/state/cache.
5. Tests SHALL verify all three kinds across four rotated CropBox/UserUnit profiles, actual ink/native saved flags, complete field/AP-content preservation, programmatic write refusal/recovery, allowed edits, copy/page/deletion/flatten behavior, strict input/lifetime checks and atomic budget failures.


## 71. Native slide background inheritance reset

1. Existing Slide.background SHALL accept None to remove the explicit slide override through the public native model. Its getter SHALL remain explicit str-or-None, not an effective inherited-color query. Inherited backgrounds SHALL be resolved by native layout/master/theme behavior without a local resolver or transparency promise.
2. Reset SHALL preserve stable slide/shape identity, unrelated content/links and independently copied documents. Existing initial lifetime and final callback-safe stable-ID checks SHALL remain. Successful mutations SHALL retain normal revision/cache semantics.
3. Removing an owned picture override SHALL preserve shared authored media and reclaim only the supported authored-media graph. Direct inherited-picture combinations outside that profile SHALL fail before mutation when local reclamation cannot preserve them; no generic imported media walker SHALL be added.
4. Tests SHALL cover native inheritance and save/reopen/PNG/PDF output, explicit-versus-inherited getter semantics, shared media and final reclamation, copy/lifetime/callback behavior and atomic invalid/resource failures. Broader SLIDE4/theme/imported preservation SHALL remain open.



## 72. Native DOCX character spacing

1. Existing character-format patches SHALL accept letter_spacing/None for DOCX add_run, ranges, paragraph/character styles, defaults and supported whole-table/conditional character regions. No new handle or formatting method SHALL be added. The initial DOCX-only PPTX rejection is superseded by the bounded slide extension in requirement 74.
2. Values SHALL be finite points in [-100, 100] on the native one-twip (0.05 pt) grid. Machine-precision arithmetic noise SHALL be accepted without accepting user-visible finer precision. Canonicalization SHALL precede native f32 storage; direct/resolved snapshots SHALL expose canonical points. Omission, explicit 0 and None SHALL preserve their existing unchanged/override/reset semantics.
3. Native shaping/layout/writing SHALL own cluster advances and serialized output. Negative spacing SHALL retain intentional condensed/overlapping behavior; source Unicode SHALL stay unchanged. Inferred PDF reading order/spaces SHALL not be promised identical when visual positions overlap or spread. Existing semantic-anchor, geometry and resource guards SHALL remain unchanged.
4. Tests SHALL cover all 4,001 accepted twips through native write/read; real cluster and source/PDF ink at bounds and ordinary values; inheritance/default/style/region precedence; body/story/note/cell scopes; copied/stale/closed ownership and callback/invalid-batch rollback. Broader TXT6 typography parity SHALL remain open.


## 73. Field-state, note-kind and slide-reset operation evidence

1. Existing pdf.owned-fields evidence SHALL add read-only state using its exact 423-test artifact while retaining historical creation and unsupported imported/export scopes. New word.note-kind-conversion and slides.background-reset profiles SHALL describe only their existing mutations and measured output behavior. Notes.convert SHALL be classified as note-kind editing, with separate existing DOCX-to-PDF evidence in the export cell.
2. Source/test fingerprints SHALL be reviewed against each exact 419/423/426-test runtime. Existing unrelated historical artifacts, pending corrected outline evidence, criterion completion flags and ownership SHALL remain intact. Reference joins SHALL reuse existing families; the representative Widget.field_flags member SHALL not imply arbitrary flags or multiwidget parity.
3. Generated support/reference guides and offline drift checks SHALL pass without claiming a new runtime build or wider imported source preservation. Broader API1/API7/QA1/QA8 and feature acceptance SHALL remain open.


## 74. Shared character spacing for owned slides

1. Existing PPTX shape/cell range format and add_run SHALL accept letter_spacing through public native RunProps.spacing, without a new API family or dependency. The initial shared profile SHALL match DOCX's finite [-100,100] points and 0.05-point precision, with canonicalization before native serialization. None SHALL reset inheritance; 0 SHALL remain an explicit override.
2. Native resolution, grapheme tracking, autofit, wrapping, writer and PDF export SHALL own behavior. Source text/links/identity SHALL remain intact. Intentional negative overlap SHALL not imply a reading-order guarantee. Visible raster and invisible PDF text-layer geometry SHALL both receive meaningful acceptance checks.
3. Existing content/property/run budgets, staged mutation, revision/cell generations and ownership checks SHALL remain authoritative. Invalid mixed patches and resource failures SHALL not mutate state. Tests SHALL cover native storage bounds, inheritance, shape/cell editing, source/PNG/PDF output, copies, links and callbacks. The obsolete DOCX negative-PPTX test SHALL be explicitly replaced by the new capability suite; historical artifact evidence SHALL remain historical.


## 75. Slide-cell append lifetime checks

1. Existing PPTX cell paragraph add_run SHALL validate its current owner, slide/table/cell and paragraph generations before evaluating caller formatting callbacks. It SHALL retain final native target validation after normalization.
2. Live slide reordering during normalization SHALL retain the original stable target; callback deletion/reset/close or exceptions SHALL not append text or formatting beyond the callback's own changes. No new public API or broader callback-order guarantee SHALL be introduced.


## 76. Owned note content clearing

1. Existing Note.clear SHALL remove owned note content while retaining its part identity, body reference, kind, ordering and section note options. No new public lifecycle handle or deletion method SHALL be added. Native set_story and note-number serialization SHALL own format behavior; cleared notes SHALL retain a canonical FootnoteText/EndnoteText paragraph rather than change to Normal.
2. A real clear SHALL stage model/resource changes before publishing child invalidation, preserving unrelated owned handles and monotonic identifiers. Existing ordinary semantic-anchor, field/navigation, media and resource guards SHALL remain without a deletion exemption. Whole-document copies SHALL remain independent.
3. Only an already-cleared canonical synthetic placeholder SHALL be a no-op, preserving bytes, revisions, ranges and render caches, including after native note-kind conversion. An explicitly authored empty paragraph SHALL be removed once. Add-after-clear SHALL reuse the placeholder with a new owned paragraph identity. Closed-owner checks SHALL precede work.
4. Tests SHALL verify both note kinds, native own marks/styles and source/PDF ink, long-note repagination, exact charged text/field/cell/media reclamation, copy/no-op/stale handles, conversion, protected-anchor rollback and existing header/footer contracts. Imported note editing, semantic reference deletion, rich note insertion and broad DOC5/LAY7 completion SHALL remain excluded.


## 77. PDF integer raster preallocation boundary

1. Full-page PDF rendering SHALL validate native rounded integer output dimensions against the existing side and 64-million-pixel limits before requesting a native raster. Existing native oversize rejection SHALL remain enabled; no new API or raster-sizing algorithm SHALL be introduced.
2. One-shot, retained-session and cached-small-render paths SHALL share this guard, using inspected native CropBox/rotation/UserUnit display dimensions. Existing clipped-region behavior SHALL remain separate so a small valid region of a large page remains renderable.
3. Tests SHALL demonstrate the continuous-area versus rounded-pixel boundary arithmetically, rejection before native request counters advance, all four rotations and offset CropBox/UserUnit, and small valid full/region controls without allocating the oversized raster. Existing all-format invalid DPI/index/lifetime checks SHALL remain. This correction SHALL NOT complete broad R4 or U09's upstream failure-reporting work.


## 80. PDF field creation callback ownership

1. Existing page.fields.add SHALL retain its initial owner/page lifetime check, normalize callback-capable options and box coordinates, then validate the original page again immediately before native creation. It SHALL NOT reuse a page ordinal captured before caller callbacks.
2. Callback page reorder/deletion or owner closure SHALL reject the outer creation without changes beyond callback effects. Callback exceptions SHALL propagate without an outer field or cache mutation. Independent document copies SHALL not invalidate a live target. This correction SHALL not introduce radio support, a new public API or a blanket callback guarantee for unrelated operations.


## 78. Owned auxiliary paragraph deletion

1. Existing header/footer/Note paragraph collections SHALL support integer-index deletion through native Document.remove_block. Negative indexes SHALL follow normal Python behavior. No new public manager, deletion alias, slice/batch operation, insertion/movement or adoption API SHALL be introduced. Body/cell/PPTX paragraph deletion SHALL remain explicitly unsupported.
2. Deletion SHALL retain surviving owned identities, table generations and native semantic references while invalidating only the removed paragraph handle; TextRanges SHALL retain normal global revision semantics. Required native empty/trailing paragraphs SHALL remain in the model as hidden reusable placeholders, with canonical note style/own-mark behavior when the last note paragraph is removed.
3. Model/resource/locator changes SHALL be staged before publication. Existing normal semantic-anchor/media/field/navigation guards SHALL remain without deletion exemptions. Independent part ownership, section-boundary exclusion, bounded locator shapes/counts and initial/final callback lifetime checks SHALL be enforced. Snapshot selection SHALL never silently retarget after callback mutation.
4. Tests SHALL verify source/PDF text/ink and native sentinel order, surviving nested table/cell/picture handles, note reference/style/counter behavior, exact resource charges, copies/iterators/callbacks and atomic invalid/unsupported edits. General DOC5/TXT1 parity and body identity migration SHALL remain open.


## 82. Cumulative owned PDF field appearance accounting

1. Owned text/checkbox/choice creation SHALL reserve work for both native initial and configured appearance generations before staging, then charge their generated raw stream bytes before commit. It SHALL preserve the existing final font/orientation/profile validation; inspection of the temporary initial appearance SHALL not impose final view-relative fit prematurely.
2. Text/choice value writes SHALL reserve bounded appearance work and charge their newly generated raw streams even for unchanged or empty values. Canonical complete checkbox state changes SHALL retain their native no-new-appearance behavior. Every successful fill SHALL consume one existing mutation item. Existing box/tooltip/read_only generation charges SHALL remain consistent.
3. Cumulative bytes/items SHALL survive copy and SHALL not be refunded merely because fields or appearance references are removed. Budget failures SHALL preserve bytes, counters and native state. No unbounded runtime object scan or orphan-stream deletion SHALL be introduced. Tests SHALL measure small owned native object stores, exact generation charges, boundary rollback, rotation and public output/copy/flatten behavior. Imported forms/radio groups and whole-process memory guarantees SHALL remain outside this correction.


## 83. PDF field value normalization and ownership

1. PdfField.value SHALL inspect actual base-string content and code-point/UTF-8 bounds without invoking overridden str subclass methods. It SHALL normalize its JSON payload before a final existing field-identity check, while retaining the initial closed/stale check before input validation. Ordinary bool/None and native field-kind/value semantics SHALL remain unchanged.
2. Size, invalid Unicode, unsupported native glyph/value and lifetime failures SHALL preserve source, pixels and warmed caches. Tests SHALL reproduce the former callback-driven replacement-field write and prove subclass callbacks are not invoked by corrected normalization. This correction SHALL not add radio support or claim universal callback safety.


## 81. Owned DOCX body identity separation

1. Owned DOCX body paragraph, table and picture handles SHALL use the existing monotonic owner-local locator registry independently of physical native block positions. This internal migration SHALL NOT enable body/cell/PPTX deletion, body insertion/movement or cross-document adoption, and SHALL NOT add public methods, handles or dependency changes.
2. Body collection enumeration SHALL retain native source order and hide required synthetic paragraphs. Native section boundaries, paths, layout/field/list targets and serialized structure SHALL remain physical and unchanged. Picture deletion SHALL retain its empty paragraph identity; existing cell/grid/story invalidation and whole-copy independence SHALL remain intact.
3. The allocator SHALL check lifetime overflow before mutation and publish its counter/registry/classification/placeholder/accounting changes only after normal commit succeeds. Existing active-location, physical-body, cell, text, media and layout limits SHALL remain. Enumeration SHALL use a bounded temporary reverse view, not repeated unbounded searches or a second persistent registry.
4. Tests SHALL preserve existing native format/geometry/output assertions while resolving owned IDs through locations. They SHALL cover interleaved allocation, identity-key order different from source order, hidden section markers, copies, picture-to-paragraph identity, child invalidation, no-op reads and failed creation/commit rollback. Numeric private `_id` and diagnostic repr changes SHALL be documented explicitly; no public numeric-ID persistence contract SHALL be introduced.


## 85. Native PDF comment opacity

Existing Annotation.opacity and update(opacity=...) SHALL support finite [0,1] values for the seven existing comment kinds through native set_style. Links, malformed values, unsupported target dictionaries and locked style changes SHALL fail explicitly and atomically. The bounded canonical profile SHALL admit finite fractional direct CA without widening metadata/geometry/reply handling. Getter SHALL avoid native appearance reconstruction. Native AP regeneration, compound intermediate streams and note appearance work SHALL be bounded/accounted; callback ownership checks SHALL preserve identity. This does not complete imported-form/reply support, PDF6 or PDF8.


## 79. Bounded owned PDF radio groups

1. Existing page.fields.add SHALL accept kind="radio" with explicit boxes and one distinct literal option per box, preserving existing singular-box calls. Groups SHALL be atomically created with 2–32 immutable same-page widgets through public native Radio primitives. All field kinds SHALL expose an owned immutable boxes tuple; singular radio box access/mutation SHALL fail explicitly. No child manager, arbitrary imported-form support or mutable group shape SHALL be introduced.
2. Native canonical name helpers SHALL preserve Unicode/# literal identities. Off, empty/control-bearing/duplicate values and oversized inputs SHALL fail. Canonical RADIO|NO_TOGGLE_TO_OFF and optional READ_ONLY flags SHALL retain native fill/clear refusal. Existing tooltip, copy, page organization, deletion and flattening semantics SHALL remain; flatten SHALL return logical field count after separately verifying native widget count and complete removal.
3. Total logical fields and widgets SHALL be bounded independently at 1,000; initial creation SHALL preflight encoded aggregate text, geometry and N(N+3)/2 member-generation work before staging. Conservative 64 KiB intermediate-pair charges and actual final AP bytes SHALL accumulate under existing history/work budgets without runtime orphan scans. All-member metadata/value work SHALL be counted; copies retain counters and failed updates remain atomic.
4. Acceptance SHALL cover actual selection pixels/source/flatten parity across all rotations with CropBox/UserUnit, encoded name aliases, exact work/widget boundaries, metadata regeneration, read-only/no-toggle behavior, immutable snapshots, callbacks/lifetime and existing three-kind regressions. Broad PDF6/U10 imported/group/appearance parity SHALL remain partial.


## 84. Owned body paragraph deletion

1. Existing document.paragraphs integer-index deletion SHALL remove ordinary direct owned DOCX body paragraphs through public native remove_block, with no new method/manager. Section-bearing paragraphs, cell/PPTX and slice/batch operations SHALL remain unsupported; normal semantic-anchor/navigation checks SHALL remain without exemptions.
2. Every ordered SectionProps value/count and valid unique section endpoint SHALL be preserved. If removal leaves a section-bearing tail, public insert_block SHALL append at most one plain unregistered final paragraph without relocating properties. Native empty/table-tail sentinels SHALL remain intact and reusable; explicit authored empty paragraphs SHALL remain real content.
3. Staged locator/accounting changes SHALL retain surviving IDs, nested paths, table generations and auxiliary ownership, shift only later physical body roots, invalidate the removed handle and preserve the monotonic allocator. Callback selection SHALL retain captured identity with final lifetime/scope checks. Failure SHALL publish no additional mutation; normal global range/cache invalidation SHALL apply on success.
4. Tests SHALL prove raw native tail behavior, exact ordered section values, next/continuous/odd/even boundaries and restart controls, DOCX/PDF source order and visible PNG content, protected references, nested handles, copies, placeholders, resource/revision errors and callbacks. Broader DOC5/TXT1 ordered insertion/movement/adoption and semantic section removal SHALL remain open. Historical specs78/81 describe their narrower original scope; this separate acceptance expands ordinary body deletion only.


## 86. Owned paragraph movement

1. Existing DOCX paragraph collections SHALL expose `move(index, destination) -> None`, with integer/negative indexes and destination defined as the final paragraph index. The supported profile SHALL be direct ordinary paragraphs within one owned body section/header/footer/note. Appending followed by movement SHALL compose insertion without adding another creator or manager.
2. Movement SHALL call public native remove/insert block operations in a single staged transaction, retain native block content, preserve surviving IDs/descendant locations/table generations/accounting and keep other blocks in relative order. A required native table-tail blank SHALL remain hidden/reusable exactly once. The body SHALL retain ordered native section properties, valid physical endpoints and moved-content section ownership.
3. Section-bearing source/destination paragraphs, cross-section/container movement, cell/PPTX scope, aliased header/footer parts and moved note/bookmark/comment anchors SHALL fail explicitly. Ordinary commit guards SHALL remain without semantic exemptions. A supported no-op SHALL preserve bytes, revision, ranges and cached output. Python index normalization SHALL recheck owner lifetime/revision and SHALL NOT retarget after callback edits.
4. Validation SHALL prove raw native sentinel/section behavior, native identity/rollback invariants, actual DOCX/PNG/PDF reorder, auxiliary note marks, section preservation, callbacks, copies and captured iteration order. Broader DOC5/TXT1 parity and anchored/adopted/imported movement SHALL remain incomplete.


## 87. Native-backed everyday property types

Existing annotation, picture, outline, page/slide and shape properties SHALL expose their native scalar, tuple, collection and nullable contracts to static consumers. Known nullable native returns SHALL not be declared nonnullable. Annotation-only changes SHALL preserve runtime method bodies, names, defaults, dispatch and errors; no new public aliases/managers or Any placeholders SHALL be introduced. Existing factory/copy/context chains SHALL retain concrete handle types. API3 SHALL remain open for unreviewed mappings/properties, documentation and IDE coverage.


## 91. Reproducible Office workload measurements

Development benchmark tools SHALL reuse the installed public document APIs, retain the original small workload by default and bound optional page/image fixture parameters before creating output. Each fresh-process result SHALL identify input bytes/hash, actual page count, DPI, ordered all-page PNG digest, retained-session counters, timings and process memory observations. Paired comparisons SHALL use identical inputs and require all-page byte parity. Evidence SHALL distinguish synthetic workload observations from real-input fidelity, allocation bounds, cold-disk latency, historical-wheel comparisons and general performance guarantees. No package runtime or public API change is required.


## 89. Native title and author fields

1. Existing DOCX `Paragraph.add_field(kind)` SHALL accept `title` and `author` in addition to the three decimal page/count kinds, returning the existing revision-bound TextRange. Native TITLE/AUTHOR evaluation SHALL use public core metadata; no custom evaluator, arbitrary instructions, metadata alias or additional field manager SHALL be introduced.
2. Authored fields SHALL retain empty dirty cached results in DOCX. Native PNG/PDF rendering SHALL evaluate current title/author, while source extraction SHALL preserve empty stored results and its existing diagnostic. Metadata edits SHALL invalidate snapshots; copies SHALL retain independent values. Nonempty imported cached results SHALL NOT be claimed refreshed by this feature.
3. Every commit SHALL bound evaluated metadata text using the existing paragraph/document text limits and combined field count. Later metadata changes and duplicated story content SHALL not bypass the limits. Metadata used by fields SHALL be single-line without tabs. Source metadata without such fields SHALL retain existing behavior.
4. Serialization SHALL reject unsupported native atomic-field placement before publishing output, using bounded public native line/cluster geometry within the existing layout preflight. The initial profile SHALL exclude turned/clipped cells and field advance outside the available line or physical page; it SHALL NOT promise glyph-outline containment or invent wrapping. Validation SHALL cover all six supported stories, current/cleared values, cache/readback semantics, PNG/PDF output, ranges, copies, byte budgets, geometry rejection and recovery. Broader REF3 and REF4 SHALL remain open.


## 90. Guarded native DOCX picture placement

1. Existing Picture handles SHALL expose format(**properties) and an owned formatting snapshot, using native Float properties. The initial supported profile SHALL be direct owned body pictures, wraps inline/square/top_bottom/behind, and page/margin/paragraph references. No separate image creator, placement or watermark manager SHALL be added.
2. Position SHALL use finite point pairs bounded by native offsets and 0.05-point increments. Distance SHALL use scalar or top/right/bottom/left points in 0..1584 with equal left/right values. Inline reset SHALL clear placement and retain original media, dimensions, crop, flips and alt text. Closed/stale IDs and callback revision changes SHALL fail before retargeting; native patch JSON SHALL be bounded and changes staged atomically.
3. Actual native placed picture frames SHALL remain within physical pages. Existing bounded pagination preflight SHALL perform one layout and revalidate at output after later paragraph/page changes. Existing container-width and media/decode/resource limits SHALL remain. Floating nested/auxiliary pictures, front/contour wrapping, alignment, independent layer/overlap controls, opacity and rotation SHALL remain explicit unsupported scope.
4. Acceptance SHALL prove native save/readback/geometry/media identity, 72/144/300-DPI PNG and PDF placement, square/top-bottom exclusions, paragraph-owned page breaks, crop/flips/copy/reset, failed patch rollback, later invalid geometry and callbacks. The reproduced upstream front-layer defect and hardcoded overlap/layoutInCell writer settings SHALL be documented. MED2/BG2–BG5 SHALL remain incomplete.


## 92. Cross-format render-session evidence

The bounded P1 reuse criterion SHALL include reproducible PDF measurements alongside spec91 Office evidence. Identical native-exported inputs SHALL be rendered in fresh cached/one-shot processes with repeated timings, process memory observations, source identities and ordered every-page PNG digests. PDF counters SHALL retain their own parser/request/rasterization meanings and SHALL NOT imply a single parse across separate eager extraction subsystems. All observed regressions and measurement limits SHALL remain explicit. Closing reuse acceptance SHALL NOT close broader QA7 real-input/platform coverage or P4 allocation limits, and SHALL NOT add package API or runtime dependencies.


## 94. Combined native DOCX cell layout acceptance

Existing public cell/table formatting SHALL have a combined matrix covering asymmetric padding, three vertical alignments, three paragraph alignments, horizontal/up/down orientation, RTL table and paragraph direction, and true empty cells. Tests SHALL inspect native-written DOCX properties, reopen DOCX/PDF, preserve logical rows and each label once, and verify actual visible cell/glyph frames at 72/144/300 DPI. Alignment along the writing direction SHALL remain distinct from movement across turned cells and from table column reversal. Empty-cell copies/additions/reset SHALL preserve ownership and geometry without invented text. No runtime API, standard algorithm, dependency or historical fixture change SHALL be introduced. Inter-cell spacing and full vertical-script fidelity SHALL remain explicit incomplete TBL8/U03 scope.


## 93. Owned DOCX cell paragraph deletion

1. Existing `del cell.paragraphs[index]` SHALL remove ordinary owned DOCX body/header/footer cell paragraphs through public native remove_block, with normal integer/negative indexes, including merged aliases and supported nested cells. No new manager, alias, movement or standards algorithm SHALL be added. Image paragraphs, rich note cells, PPTX cells and slice deletion SHALL remain outside this profile.
2. Locator shifts SHALL affect only the selected native parent container. Surviving paragraph/table/cell/picture identities, table generations, neighboring cells, other stories, section properties and referenced media SHALL remain intact. Deleted handles SHALL become invalid; copies SHALL remain independent, while existing ranges/caches follow ordinary revision invalidation. Index callbacks SHALL retain the captured target and reject stale cell generations before normalization.
3. Native required final blanks SHALL remain hidden and reusable through existing cell placeholders. Authored empty paragraphs SHALL retain their identities until individually deleted. Accounting, locator, placeholder and document mutation SHALL be staged together; normal reference, ownership, field/media, traversal and resource guards SHALL not be weakened.
4. Validation SHALL execute raw native proofs before integration, distinguish authored and generated blanks by identity, verify merged/nested and auxiliary scopes, actual DOCX/PNG/PDF content removal, stable descendants/siblings, precise resource charges, protected-anchor rollback, malformed locators, copies, callbacks and closed owners. Broader DOC5/TXT1 composition and cell movement SHALL remain incomplete. Earlier deletion specifications retain their historical narrower scope.


### 96. Owned PPTX slide duplication prerequisite

Keep SLIDE1 duplication/import open until a public lower-level native operation can allocate fresh identities and preserve or explicitly reject all semantic references. Existing slide append/move and whole-document copy remain the available composition operations. Native per-slide shape uniqueness is insufficient for the owned adapter's globally keyed table generations. No local copy/remapping algorithm or editor-engine dependency is permitted.


## 95. Native-backed snapshot output types

Existing WordDocument/PdfDocument.metadata and DOCX Picture/EditableTable/EditableCell.formatting getters SHALL expose verified output mapping types. Absent metadata keys SHALL differ from present nullable direct formatting values; Word custom SHALL remain a required non-null scalar mapping. Returned lists, tuples, nested borders and literal enums SHALL retain actual native/Python shapes. Private TypedDict declarations MAY describe these shapes, reusing accurate existing types; no public export, manager, accepted runtime input or method body/default SHALL change. Input annotations MAY be corrected only for already-accepted cell margin lists and table/cell snapshot-border dictionaries so typed snapshot replay remains valid; runtime state/combination guards SHALL remain intact. Strict positive/negative consumer checks, successful type-hint introspection and an annotation/type-declaration-erased AST comparison SHALL validate the change. Polymorphic paragraph/style/default snapshots and broader API3 SHALL remain open.


## 97. Owned DOCX same-cell paragraph movement

The existing paragraph collection move operation shall support ordinary paragraphs in one canonical owned body/header/footer cell, including nested and merged cells. Preserve native contents, other block order, surviving paragraph/table/picture identities, grid generations, neighboring containers, ordered body sections and exact resource accounting. Hide only native-created required tail paragraphs and reuse them on later append; authored empty paragraphs remain editable. No-op leaves revision/bytes/cache unchanged. Keep source semantic-anchor rejection, callback revision checks and normal commit guards; cross-container, imported, rich-note and PPTX movement remain unavailable.


## 99. Owned DOCX theme identity prerequisite

Do not expose theme editing based only on palette/font metadata serialization. STY3 requires persistent theme references, tint/shade/mapping and font roles, native resolution after replacement, preservation of explicit overrides and independent DOCX/PNG/PDF acceptance. Existing fixed-color/font style/default formatting remains supported. Keep missing native semantics under U02; no new color/style manager or string-token workaround.


## 101. Owned PPTX cell paragraph deletion

Existing `del cell.paragraphs[index]` shall remove one intact native PPTX Paragraph using integer/negative indexes, including horizontal, vertical and rectangular merged aliases. Retain surviving paragraph/run properties, links, cell formatting and unrelated content. Deleting the final owned paragraph leaves an empty collection; the native writer supplies the required blank on serialization, and existing append initializes it without a leading separator. No new public API, placeholder registry or text-join workaround shall be introduced.

Canonical cell body generations shall advance atomically, invalidating that cell's old paragraph handles; grid/Cell handles and unrelated cells remain valid. Copies remain independent, and deleted handles cannot revive after append. Native revision/content checks and checked generation exhaustion apply before publication. PPTX index callbacks that change or close the owner shall reject the outer deletion; existing DOCX stable-ID callback behavior remains unchanged. Validate source/readback/render/PDF output, first/middle/last/empty removal, merged aliases, resource reclamation, exact/+1 limits and rollback. Broader DOC5/TXT1/SLIDE3 and paragraph movement remain open.


## 98. Bounded owned DOCX comment roots and replies

1. Owned documents SHALL expose one comments collection whose add operation accepts an existing whole-paragraph TextRange for a root or root Comment for a direct reply. Plain text, author, date and resolved properties SHALL use public native Comment/part/marker APIs. Fresh range and parent properties SHALL retain owner/revision/Unicode semantics; no rich-story manager or arbitrary source adoption SHALL be added.
2. A root SHALL occupy the exact endpoints of one nonempty body paragraph, including ordinary/merged/nested body cells with horizontal, non-exact-height ancestry. Other inline objects, semantic anchors, interior/cross-paragraph/empty roots and nested replies SHALL reject explicitly. Every candidate commit and output SHALL validate the exact marker/map/part topology, including after later text or geometry edits. Roots with replies SHALL require explicit leaf deletion first.
3. Monotonic owner IDs and explicit native durable IDs SHALL prevent deleted-handle and earlier-deletion retargeting. Private comment paragraphs SHALL reuse existing block/text accounting without appearing in public body/header/note collections. All registry/counter/document changes SHALL commit atomically; malformed/sentinel/extra-locator states SHALL reject rather than be cleaned silently.
4. Limits SHALL include 1,000 comments, 200 direct replies per root, 1 MiB comment text, 1,024 author UTF-8 bytes, 256 KiB native metadata, existing document/block budgets, checked identity/revision exhaustion and a 2,000,000-item source preflight. Authors SHALL be nonempty trimmed single-line values (default Unknown); dates SHALL be empty or valid UTC timestamps. Comment parts SHALL be plain native text and remain separate from body content.
5. Evidence SHALL distinguish stored comments from rendered review state: native PNG shades supported anchors even when resolved; default PDF omits comment markup/text; legacy extraction projection remains unchanged. Preserve raw interior-shaping, overlapping-shading and durable-ID controls as upstream evidence. REV1/TXT8/REV5 SHALL remain incomplete beyond this bounded slice.


## 100. Bounded DOCX extraction provenance

Existing Block/Table/Page metadata SHALL expose native source paths, explicit paragraph segments and available Body-story line selection observations without adding an inspection manager, public method or duplicate bbox alias. Source identity SHALL remain local to an extracted snapshot and distinguish stories; native UTF-8 source ranges SHALL NOT be described as offsets into normalized text. Existing mutable detached models, text ordering, table hierarchy, first-page paragraph assignment and post-close ownership SHALL remain compatible.

Native x_spans and turn_rect SHALL own selection and coordinate transforms. Body indexed placements SHALL preserve page/item/line order and repeated occurrences. Auxiliary/Clip projections SHALL remain unavailable, and logical selection SHALL NOT imply glyph ink, clip containment, object bounds or complete semantic page slices. Block/Table bbox SHALL remain unset. Validate ranges and finite inputs before native helpers; precharge native temporary vectors/sort scratch, work, records, source metadata and copied representations. Limits SHALL fail explicitly, with exact/+1, malformed-input, render/PDF, deterministic-source and compatibility acceptance. E2/LAY8 remain open beyond this bounded slice.


## 103. Office caller-font isolation prerequisite

Keep STY5 open until public native APIs admit bounded caller programs with deterministic document-scoped face selection, lifetime and collision behavior across layout, PNG, PDF and Office save/reopen. Require explicit native embedding/subsetting policy and substitution/missing-glyph diagnostics. Existing PDF supplied-font drawing and literal Office family formatting remain supported. Do not mutate process-global font registries, fabricate OOXML font tables, link application loaders or add a duplicate font manager. Connect the prerequisite to existing U02/U11; keep theme-role identity under STY3.


## 104. Existing formatting snapshot output types

Annotate the existing paragraph/style/default formatting getters without changing runtime dispatch, accepted inputs, exports or native behavior. Preserve required common keys, native-kind-specific optional keys, nullability, list-valued tabs/margins and nonrecursive table regions. PPTX paragraph direct snapshots contain alignment alone; successful resolved paragraph snapshots remain Word-only. Do not advertise Style.kind as a static discriminator or treat table inheritance as concrete paragraph/character resolution. API3 remains open beyond this slice.


## 102. Owned DOCX table deletion

1. Existing table collections SHALL support integer/negative deletion in the body, independently owned headers/footers and ordinary nested/merged cells through public native remove_block. Rich note/comment stories, imported editing, slice deletion, arbitrary block movement and cross-container adoption SHALL remain outside this slice.
2. Removal SHALL retire the target subtree only, shift surviving locators and hidden blanks within the exact native parent, and preserve surviving handles, ancestor/unrelated grid generations, ordered section metadata and shared media. Valid owned containers SHALL retain their existing final paragraph by identity; malformed sole/table-ended containers SHALL reject rather than fabricate or adopt a blank. Authored empty paragraphs SHALL remain editable and hidden required tails reusable.
3. Original bounded source/comment inventory and global locator/image/generation consistency checks SHALL precede native content usage and any global pruning. Subtree section metadata and protected references SHALL reject. Text/cell/media reclamation, registry changes and exact candidate comment/navigation/media validation SHALL commit atomically, with existing identity, revision and resource limits.
4. Validation SHALL distinguish raw-native structural capability from accepted ownership, prove output removal and preserved sibling/media/reference identities, exercise exact/+1 preflight and allocator/revision boundaries, malformed associations, copies, callbacks and closed/stale handles. Broader DOC5 SHALL remain incomplete.


## 105. Section-first inline letterhead composition

Reuse existing header/footer variants, PNG picture handles, ranges and section lifecycle for a caller-owned example; add no library API or runtime behavior. Verify first-slot precedence, independent copied section content, displayed-number even/default selection and native repagination after inserting earlier body text. Preserve original text/order, one body link and semantic table content exactly once across DOCX/PDF. Validate actual source/exported appearance at 72/144/300 DPI using full-page 72 controls and bounded higher-DPI header regions. Record intentional parity filler pages explicitly; keep section fills/linkage/continuous/floating/watermark exclusions honest.


## 107. Reject unsupported owned table PAGE fields

Owned DOCX PAGE field insertion SHALL reject table-cell paragraphs at every nesting depth and merged alias. The same scope rule SHALL validate every staged candidate and serialization after bounded source inventory, preventing later edits or privately constructed state from emitting a known incorrect page number. Direct body/header/footer/note paragraph PAGE and supported table NUMPAGES/SECTIONPAGES/TITLE/AUTHOR SHALL retain their existing behavior and limits. Removing an unsupported field SHALL remain a recovery operation. No local evaluation, result rewriting or imported mutation API SHALL be introduced.

Tests SHALL preserve the native stale-context reproducer and successful direct/count controls, prove precise Unsupported errors and atomic byte/revision/registry/range/cache rollback, and verify output/lifetime behavior. Earlier successful cell-PAGE fixture setup SHALL be deliberately replaced with supported count fields while preserving resource/identity/geometry checks, with the correction documented and evidence fingerprints checked rather than refreshed broadly. U07/LAY6/REF3 SHALL remain open.


## 109. Preserve page and slide selection across index callbacks

PDF page lookup (including slices), deletion and movement SHALL capture page generation and count before caller index conversion. They SHALL reject an invalidated selection before returning current-generation handles or dispatching an outer mutation. Each movement argument SHALL be checked after its conversion. Slide deletion/movement SHALL equivalently retain a native slide-ID sequence snapshot. Callback-owned mutations remain visible; the outer operation SHALL not target a replacement item. Closed owners fail before caller conversion, ordinary negative indexes/bounds retain Python behavior, and metadata/content edits that leave the selection snapshot valid remain permitted. Existing stable-ID slide lookup remains unchanged. No public method, native mutation algorithm or new persistent identity registry is required.


## 108. Owned DOCX mirrored margins

Provide one exact-boolean `WordDocument.mirror_margins` property, default false, using pinned public native Settings and existing section margins/gutter. Preserve original margin values and native displayed-page parity, including section restarts and the independent even/odd-header filler rule. Use native serialization, layout, rendering and PDF export without local coordinate compensation or another settings manager.

Acceptance covers false/true/reset and independent copies; closed/type/revision/cache behavior; asymmetric margins/gutter, headers/footers, tables, native selection geometry, supported floating anchors, single-column labels and equal-column natural reflow. Preserve exact body/link/table content. Reject invalid resulting geometry and existing semantic/resource violations atomically; keep multi-column numbering and table PAGE fields unsupported. Reopen native DOCX and PDF and check actual bounded image/label geometry at declared DPIs. Broader LAY10 settings remain open.


## 106. Same-parent owned DOCX table movement

1. Existing body/header/footer/cell table collections SHALL support `move(index, destination)` with final table ordinal semantics and negative indexes. Only one exact owned parent and body section SHALL be accepted, including canonical merged/nested cells and independently owned auxiliary stories.
2. Public native remove_block/insert_block SHALL preserve moved and displaced subtree IDs, all table generations, descendant suffix paths, media and resource counters. Container length and its exact existing final paragraph SHALL remain unchanged. No descendant invalidation, synthetic-tail fabrication or cross-container adoption SHALL occur.
3. Original bounded source/comment/global registry preflight SHALL precede scans and mutation. Moved note/bookmark/comment anchors and section descendants SHALL reject even for no-op; retained sibling anchors SHALL validate against a fresh exact-candidate inventory. Table PAGE SHALL remain rejected under spec107; count fields retain native evaluation.
4. Index callbacks SHALL not retarget an owner changed by either callback; stale cells SHALL reject before callbacks. A validated no-op SHALL preserve revision/cache/ranges at revision MAX; actual movement SHALL stage one checked revision increment or atomically reject. Tests SHALL cover malformed associations, ownership, callbacks, copied and held handles, required/authored blanks, source-work boundaries, supported count output and unchanged deletion regressions. DOC5 SHALL remain partial.


## 113. Release compiler selection

Every Linux, macOS and Windows release-wheel build must explicitly select the same Rust 1.95.0 toolchain used by required CI. Keep the existing nine-wheel matrix, immutable actions, locked dependency graph and release/publish gates. Selecting a compiler is build configuration, not permission to publish an unfinished feature set.

## 111. Bounded owned DOCX section vertical alignment

1. Existing `Section.vertical_alignment` SHALL accept exactly `top`, `center`, `bottom` or `justify`, defaulting to native Top and delegating placement/readback/PDF behavior to public native PageVAlign. No new settings manager, coordinate compensation, nullable reset or parse aliases SHALL be introduced.
2. If any section is non-top, the exact staged document SHALL require horizontal single-column NextPage sections, horizontal cells, no notes, no active even/odd header/footer mode and no floating objects in any story or nested group. Inactive even-story placeholders created by section copying SHALL remain valid. Ordinary body/nested tables, default/first stories, inline pictures and verified color/PNG backgrounds SHALL remain composable. Restoring all sections to Top SHALL recover the existing profile.
3. The source restriction SHALL apply centrally to later mutations and ordinary output, after bounded source inventory, with bounded independent work accounting. Non-top output SHALL reuse the existing layout pass to reject nonfinite or out-of-body vertical geometry, including after typography changes. Horizontal signed advances SHALL retain their existing behavior; commit-time source validity SHALL not promise later output geometry.
4. Validation SHALL preserve the native column, float, even-header and note-readback failures as diagnostics; positively verify note-free source/readback geometry, native PNG/PDF, field values, backgrounds, strict values, copy/section inheritance, cache/revision/registry rollback and Top recovery. Broader LAY10 SHALL remain pending.


## 110. Unused owned DOCX custom-style deletion

Existing Styles SHALL support single-item deletion by name/ID or integer index, with negative indexes and no slice/batch deletion. Only unused custom paragraph/character/table definitions SHALL be removable. Protect builtin/default/implicit native styles and reject every incoming raw reference across all stored stories, separators, nested table/row properties, document defaults, other style definitions/conditional regions and numbering levels/overrides. Opaque/history carriers outside the owned profile SHALL fail explicitly. Native removal SHALL be the only style edit; no automatic restyling, import or local standards algorithm is permitted.

Exact-ID handle targets and conversion revision/identity checks SHALL prevent deleted handles from aliasing recreated names. Retired IDs SHALL be bounded, copied with owners and reserved using native allocation: live plus retired IDs at most 1024 including builtins, stored IDs at most 259 UTF-8 bytes. Prepare retirement before ordinary staged commit and publish it only after success; failed operations preserve bytes, counters, handles, revision/cache and history. Limit failures retain the existing UnsafeArchiveError resource_limit mapping. Verify raw premises, exact/+1 bounds, all callback consumers, copies and source/reopened DOCX/PNG/PDF preservation. STY6 import/general cleanup remains open.


## 115. Existing layout value annotations

Existing paragraph, table and row properties must retain concrete successful result types in Python consumers: boolean page-break/row flags, a spacing triple, a list of column widths, a nullable uniform-border pair and a height/rule pair whose height may be absent. Setter annotations retain documented iterable/tuple/list forms and nullable resets. This is static API3 discoverability work, not new format support or changed runtime validation.


## 114. Fallible native PPTX rendering and PDF propagation

1. Every selected DeckCraft Git crate SHALL use one immutable reviewed contribution-fork revision, with explicit upstream parent/provenance. Other engine pins and runtime package versions/edges SHALL remain unchanged unless a separately reviewed dependency requirement demands otherwise. Regenerate/review lock, attribution and both license inventories.
2. The selected PNG render call SHALL return explicit per-call native failures through private rendering_failure and existing public DocExError. PDF export SHALL propagate the native renderer error through existing conversion_failure. Legitimate blank slides SHALL remain successful. Do not infer errors from global counters or white pixels, add a public exception or duplicate a rendering algorithm.
3. Preserve preallocation DPI/index/geometry/region checks, RGBA/PNG validation, retained-session ownership/counters and close behavior. Real native reentry SHALL exercise full/clip/session and PDF paths with exact error mapping, unchanged outer pixels/extent and subsequent healthy recovery. Boundary-only Python error tests SHALL be identified as such.
4. Audit cited native files/symbols and migrate current engine provenance without relabeling historical fixture/artifact evidence. Complete locked native, installed-wheel, independent, typing, license and size gates before merge. Broader R4 allocation/work limits and other native entry points remain open; no version release is implied.


## 116. Dedicated native WordCraft stroke upgrade

1. Select every WordCraft crate at the same reviewed immutable contribution-fork commit; retain exact upstream parent, attribution and all other pins. Regenerate and review the lock/license inventories without linking applications or adding runtime assets/downloads.
2. Native PDF shape caps/joins SHALL match the existing native round-stroke contract in the verified profile. Keep the correction in the native PDF library; no adapter geometry compensation, new shape API, unrelated renderer rewrite or imported-mutation claim belongs in this upgrade.
3. A meaningful imported native-generated DOCX regression SHALL fail against the original PDF corner behavior and pass through selected native and packaged PNG/PDF paths, with positive/negative visible controls at72/144/300DPI, readback and reopen. Preserve original native baseline failures and excluded geometry/serialization diagnostics.
4. Keep historical support evidence unchanged and complete full locked native, installed-wheel, independent, typing, license/size and current-source audit gates before manual merge. Owned shape authoring and broad MED4/U04 completion remain separate; no version release follows from the pin change alone.


## 112. Bounded owned DOCX closed shapes

1. WordDocument SHALL expose one add_shape creator, its existing-style shapes collection and private owned handles for direct-body unrotated rectangle/ellipse geometry. The handle SHALL reuse Picture placement vocabulary and offer only kind, size, fill, stroke and formatting.
2. Geometry SHALL use public native models/factories/paragraph insert/delete/layout/render/write APIs. Native marker/section behavior SHALL be preserved; deleted shape identities SHALL never retarget, and copies SHALL own independent registries.
3. Source admission SHALL require horizontal sections and an exact reference-free native shape profile, finite canonical dimensions and stroke widths no greater than 100 points or the smaller shape dimension. Every candidate/output SHALL verify requested dimensions, exactly one native placement per shape and full-outline physical-page containment without coordinate correction.
4. Source work, shape count, patch bytes and existing block/cell/page/item limits SHALL be bounded before unbounded copying or layout. Existing style/media/semantic/field/alignment guards SHALL remain active. Caller conversions SHALL revalidate owner revision and exact target identity before mutation.
5. DOCX readback, native PNG and PDF SHALL retain demonstrated source/order/links/tables/notes/images without fake extraction geometry. Line, textbox, rotation, front, groups, effects, auxiliary/cell placement and imported editing SHALL remain explicitly unsupported. MED4 SHALL remain incomplete.


## 117. Native PDF final integer allocation budget

1. All direct and resolved PdfCraft crates SHALL use the same immutable reviewed contribution revision; unrelated packages, engine pins and local vendor source SHALL remain unchanged.
2. The native renderer SHALL reject final integer rasters above its existing pixel budget before allocation in ordinary and strict modes, preserving accepted raster geometry/pixels and bounded tiles.
3. Existing local rounded-dimension preflight and stable Python errors SHALL remain in place. The backend fix SHALL not be presented as newly available public behavior; existing public limit tests SHALL remain unchanged.
4. Actual linked-native refusal/recovery and unchanged local preflight SHALL be tested separately. A test-only exact locked-source precondition SHALL prevent direct overcap tests from running against the unsafe old backend; original failure evidence SHALL use the independently reviewed intercepted allocator.
5. Dedicated integration SHALL include source/lock/license/asset provenance review, complete native and installed-wheel extraction/conversion/reopen/render checks, measured artifact sizes and independent review. Broader U09/R4 and overall release work SHALL remain open.


## 118. Same-cell owned PPTX paragraph movement

1. The existing `cell.paragraphs.move(index, destination)` SHALL reorder intact public native paragraph values within one canonical owned PPTX table cell, including merged aliases, normal negative indexes and final-index semantics. No new collection, manager, imported editing or cross-cell adoption is introduced.
2. Preserve paragraph/run/default/end properties, links, authored empty paragraphs, neighboring cells, media, native grid and source order. A real move SHALL advance only the existing cell-local paragraph generation plus document revision; old handles in that cell and all revision-bound ranges become stale. Other-cell paragraph and Cell/table handles remain usable; copied documents remain independent.
3. Validate revision, both canonical targets, bounded source and native identity/layout consistency before no-op acceptance. A valid no-op SHALL preserve bytes, revision, caches and handles, including at exhausted mutation counters. A changed move SHALL check generation/revision exhaustion and stage native mutation atomically, publishing no partial state on failure. Caller index callbacks SHALL be bracketed by lifetime/revision checks, and stale grids SHALL reject before callbacks.
4. Prove exact model permutation, native readback, actual PNG/PDF order/links/media, empty/singleton cases, aliases, copy/lifetime/iteration, exact/+1 source budget and MAX counters. Preserve all existing deletion tests and historical support evidence. DOC5/TXT1/SLIDE3 remain open for broader content operations.


## 122. Keep reference dispositions consistent with accepted features

The pinned reference inventory SHALL distinguish implemented bounded operations from remaining work. Existing DOCX paragraph/table movement, mirrored margins, owned floating pictures/comments and fallible DeckCraft rendering SHALL not be described as wholly absent after acceptance. The PPTX same-cell movement addition SHALL reuse its existing family and workstream connections. Correcting prose SHALL not promote operation profiles, checklist completion, historical fingerprints or reference-version scope.
