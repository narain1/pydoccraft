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
4. Validation SHALL prove one native parse/layout, mixed-page full/clip PNG parity, source lifetime, concurrent close, resource/error behavior and installed-wheel acceptance. Separate-process synthetic measurements SHALL distinguish open, cold/warm/multi-page latency and native RSS/peak RSS. P1 remains open pending representative large-corpus evidence.


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
