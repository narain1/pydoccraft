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
2. Native logical metric overflow, invalid geometry, unsupported glyphs and excessive wrapping work SHALL fail atomically. Automatic caller-font wrapping and strict glyph-outline clipping SHALL remain explicitly unavailable.
3. Existing add_image SHALL accept bounded visual crop fractions, quarter-turns and flips through native transform properties, preserving source media and existing page resources. Rotation, CropBox and UserUnit SHALL preserve displayed-point semantics.
4. Tests SHALL verify native wrapping/alignment, explicit-font text, rollback, work limits, asymmetric crop/flip/rotation pixels, outside-box pixels and extraction after serialization. PDF2 SHALL remain open for documented upstream gaps.

## 25. DOCX paragraph border formatting

1. Existing paragraph, paragraph-style and document-default formatting SHALL accept supported four-edge borders with shared color/style/width vocabulary. Direct and effective snapshots SHALL expose native whole-group inheritance and distinguish omission, per-side clearing and group reset.
2. Table-style border meaning SHALL remain unchanged. Shared DOCX border validation SHALL reject proven rendering substitutions/clamps and invalid vertical double/wave edges before mutation; PPTX rules SHALL remain independent.
3. Native DOCX readback and visible PNG/PDF tests SHALL cover body, cells, stories, style/default inheritance and atomic invalid changes. Between/bar and other unsupported native borders SHALL remain explicit upstream gaps.
