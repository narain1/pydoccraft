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
