# Design: Rust document engine

## Summary

`pydoccraft` is a mixed Python/Rust package licensed under Apache-2.0. Python retains the stable PyMuPDF-style `doc_ex` API and Markdown rendering. A private PyO3 extension, `doc_ex._core`, adapts pinned upstream ArtCraft engine crates from PdfCraft, WordCraft, and DeckCraft. No desktop UI, CLI, MCP, networking, updater, or subprocess component is linked.

```mermaid
flowchart LR
    A[path or bytes] --> B[Python facade]
    B --> C[doc_ex._core]
    C --> D[PdfCraft adapter]
    C --> E[WordCraft adapter]
    C --> F[DeckCraft adapter]
    D --> G[Normalized payload]
    E --> G
    F --> G
    G --> H[Document / Page / Block]
    D --> I[PDF render and DOCX export]
    E --> J[DOCX render and PDF export]
    F --> K[PPTX render and PDF export]
```

## Dependency policy

Every upstream repository is pinned to an immutable release commit:

- PdfCraft v0.9.0: `645016addcfb9078c557f0189559ab47cdcee057`
- WordCraft v0.8.0: `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`
- DeckCraft v0.8.0: `22db9261f9d1a214c296fd390173130c45d76d66`
- Craft PDF backend patches use the immutable `craft-libs` revision required by WordCraft/DeckCraft.

Rendering and conversion dependencies are currently unconditional in the release wheel; only operation-specific upstream crates are linked, and no application umbrella crate is used. A locked license audit must permit Apache-2.0, MIT, BSD, ISC, Zlib, Unicode, and similarly permissive licenses; GPL, AGPL, and LGPL-only dependencies are rejected.

## Native adapters

### PdfCraft

`pdfcraft-render`, `pdfcraft-cos`, `pdfcraft-edit`, and `pdfcraft-create` open PDF bytes and expose page metadata, positioned reading-order text, links, and images. PDF→DOCX maps those public values into `pdfcraft-export::Page` and calls `pdfcraft-export::docx`, preserving the upstream conversion implementation without linking the application engine, JavaScript, automation, printing, or UI crates.

### WordCraft

`wordcraft-docx::read_with_password` returns the public `wordcraft_doc::Document`. The adapter recursively walks ordered `Block::Para` and `Block::Table` values, resolves inherited heading levels, run hyperlinks, inline images, comments, and explicit page breaks. `wordcraft-layout` supplies computed pages and `wordcraft-render` supplies page images. DOCX→PDF uses `wordcraft-pdf`, and upstream Markdown may be used for fidelity checks.

### DeckCraft

`deckcraft-pptx::import` returns the public presentation model. The adapter recursively walks grouped shapes, text paragraphs/runs, hyperlinks, tables, notes, comments, and media. `deckcraft-render` renders slides and `deckcraft-pdf` exports PDF.

## Python boundary

```python
_core.extract(data: bytes, filetype: str, password: str | None) -> str
_core.extract_text(data: bytes, pages: Iterable[int] | None, password: str | None) -> str
_core.convert(data: bytes, source_format: str, target_format: str, password: str | None) -> bytes
_core.render(data: bytes, filetype: str, page: int, dpi: float, password: str | None) -> bytes
```

JSON remains the narrow structured-extraction ABI. Images are base64 encoded only across this boundary. The additive PDF `extract_text` fast path returns bounded form-feed-separated text directly and deliberately omits structured values; DOCX/PPTX use the existing eager model fallback in Python. Both the Python facade and native PyO3 boundary collect at most 10,000 requested indexes with one bounded lookahead; the all-pages path uses an index range with the same cap. Native glyph UTF-8 lengths plus an upper bound on separators are checked before upstream `plain_text`, and Office accumulation counts UTF-8 bytes incrementally before joining. Upstream glyph construction remains outside this output-budget guarantee; streaming/peak-memory acceptance stays open. `render` returns PNG bytes. Python hydrates typed `Document`, `Page`, `Block`, `Table`, `Cell`, `Link`, and `Image` values.

## Conversion matrix

| Source | Target | Upstream implementation |
|---|---|---|
| PDF | DOCX | PdfCraft public extraction values mapped into the `pdfcraft-export` DOCX writer |
| DOCX | PDF | WordCraft PDF export |
| PPTX | PDF | DeckCraft PDF export |

Other pairs fail explicitly. Conversion fidelity and warnings are those of the pinned upstream release.

## Rendering

`Page.get_pixmap(dpi=144)` and `Document.render_page(page, dpi=144)` return PNG bytes. PDF uses PdfCraft rendering, DOCX uses WordCraft layout/render, and PPTX uses DeckCraft rendering. PDF eager open privately retains the same PdfCraft `PageRenderer` used for text extraction and reuses its parsed document for later renders; `Document.close()` drops that state while hydrated values remain owned. The one-shot `_core.render` boundary remains for non-session routing and compatibility. DOCX layout reuse is still pending. These dependencies are included unconditionally in the current wheel; extraction-only feature builds are not implemented.

A private `rendering` helper validates finite DPI, raster dimensions and RGBA length. PDF/DOCX encoding unpremultiplies color channels and writes PNG physical resolution; PPTX retains upstream `to_png_at`. The shared raster budget is 64 million pixels. WordCraft/DeckCraft dimensions are preflighted before raster allocation; PdfCraft already enforces the matching pixel budget with `reject_oversize`. DeckCraft's caught-panic blank-image behavior remains U09; local code cannot reliably distinguish that output from an intentional blank slide.

## Normalized model

The normalized payload preserves zero-based pages, ordered blocks, links, images, tables, comments/notes, optional geometry, and format-specific metadata. Markdown remains in Python. PDF tables use upstream Office-export inference for conversion; extraction exposes conservative table candidates derived from public glyph geometry because PdfCraft v0.9.0 does not publish its detector.

DOCX `TablePayload.cells` recursively owns `CellPayload.blocks`, spans and vertical merge state. The existing flat `rows` stays compatible. Python renders nested/merged/rich cells as escaped HTML tables inside Markdown and suppresses already-emitted cell assets in the page appendix. Empty cell structure is omitted from serialized PDF/PPTX tables. This schema is extraction output, not a replacement authoring document model.

## Safety and errors

Upstream bounded parsers handle object/package limits and malformed content. The adapter adds OOXML archive preflight limits and validates render DPI and output dimensions; input is bounded at 128 MiB and normalized JSON at 64 MiB, with format-specific media/story limits. Total execution time and whole-process peak memory remain caller-controlled. No native adapter performs network access. Upstream errors are mapped to stable categories: unsupported, malformed, encrypted, resource limit, and conversion failure.

## Testing

Tests use generated fixtures and upstream project fixtures where licensing permits. They cover extraction structure, links, media, tables, comments/notes, multi-DPI dimensions/metadata/visible output, conversion reopen, malformed input, close semantics, and Python error mapping. `tools/validate_installed.py` compares imported package files with the wheel before running the corpus and writes machine-readable evidence. Release validation runs formatting, tests, Clippy, a locked license audit, wheel build/install, footprint measurement, and a scan proving no direct parser or external CLI path remains.

## Planned extensions

The [feature roadmap](../../../TODO.md) distinguishes existing rendering from pending DPI validation/metadata work and new authoring work. Creation and save are now exposed by the initial native authoring profile below. Proposed DOCX authoring must adapt WordCraft's public model/writer; PDF authoring must adapt PdfCraft's public creation APIs. Verify these APIs at the pinned revisions before extending the typed Python facade and narrow native boundary. Do not serialize formats in Python or expose upstream Rust types.

Structured reports may reuse DOCX creation followed by the existing WordCraft PDF exporter. Preserve the current extraction model and bytes-returning render/conversion contracts. Source locators, recursive table content, and diagnostics should be additive, with explicit limitations when upstream lacks exact geometry or semantics. These are design directions for the backlog, not implemented behavior.

### Python authoring facade

The [authoring API proposal](../../../docs/authoring-api-design.md) defines a Pillow-like direction: a simple factory, typed editable objects, named properties, explicit units, independent copying and explicit saving. Microsoft Open XML SDK supplies feature coverage references only. The API belongs to pydoccraft and does not expose C# schema classes, engine-specific Rust objects, or raw package editing as the normal user path.

An explicit edit-copy operation must retain the full upstream model from original source bytes; extracted `Page`/`Block`/`Table` values are insufficient to reconstruct arbitrary documents safely. Existing extraction values remain owned snapshots. Editable DOCX sections represent flowing content; PDF pages and PPTX slides retain fixed geometry. Changes invalidate layout/render state consistently, with explicit handle lifetime rules.

New native operations must remain narrow and bounded. Final session/serialization interfaces are deferred to API8 until pinned public upstream APIs are audited. Python performs API validation and explicit path/stream I/O; upstream libraries own mutation, serialization, layout and format-specific algorithms. Unsupported public capabilities require upstream work rather than another backend, handwritten XML/PDF serialization or application crates.

The expanded table/background roadmap distinguishes semantic cells, native document background, anchored drawings/watermarks and raster preview backgrounds. These have different scope and preservation rules. A compatibility ledger must track read/create/edit/preserve/render/export independently; no-op save and unknown-part handling are required before advertising safe editing. Planned tests and upstream blockers are recorded against stable roadmap IDs.

### Overlap review and upstream ownership

The [canonical workstreams](../../../docs/roadmap-workstreams.md) consolidate 139 coverage IDs into 14 ownership units, with an exhaustive JSON mapping. Keep recipes and acceptance scenarios distinct from implementation primitives. Use existing PNG methods, content insertion, ordinary properties, safe copies and ordered collections; no extra rendered-page wrapper, report builder, template language, batch API or public session framework is planned.

The [upstream register](../../../docs/upstream-roadmap.md) records a pinned-source audit. WordCraft and DeckCraft writers and numerous editing/background primitives already exist. Confirmed focused candidates include a public PdfCraft table detector, a fallible DeckCraft render path instead of blank-image fallback, and lower-level WordCraft revision-resolution routines currently located in engine code. Public availability, current integration and verified fidelity are separate states. Missing format algorithms belong upstream; adapter code must not duplicate them or link prohibited engines.

## Native authoring profile (unreleased)

`src/doc_ex/_authoring.py` owns Python object identity, units/colors, ordered collections and explicit filesystem persistence. `author_docx.rs`, `author_pdf.rs` and `author_pptx.rs` own private PyO3 native document models. These private types serialize bytes through the selected upstream writers; no upstream type crosses the Python boundary. Existing `_core.extract`, `_core.render` and `_core.convert` remain compatible.

Document creation uses WordCraft `Document`, PdfCraft COS/create/edit/organize, and DeckCraft's blank model/public PPTX writer. Native mutation operates on owned models and validates compound changes before committing. DOCX revision-bound ranges use Unicode character offsets and checked story/path locators. PPTX handles resolve stable native slide and shape IDs after slide reordering. PDF page/annotation handles are invalidated after structural changes when indexes can move. Cached Python extraction snapshots are invalidated on mutation.

Existing PDF `Document.edit()` passes original source bytes to the conservative native edit profile. Office source editing is refused until unsupported package content can be diagnosed. This avoids reconstructing editable documents from incomplete extraction snapshots. New authored Office models are editable throughout their owned lifetime.

`save(path)` serializes first, writes a same-directory temporary file, preserves an existing mode and replaces atomically; native code does not write caller paths. Stream writes report incomplete writes without claiming rollback. Scope and format limitations are published in `docs/authoring.md`. Auxiliary header/footer extraction is described below; source editing still requires separate preservation evidence.

## DOCX story projection and layout controls

The native extraction payload adds optional `stories` containing `StoryPayload { kind, blocks, metadata }`. Python hydrates the same owned `Block`, `Table`, `Cell`, `Link` and `Image` hierarchy and exposes `Document.stories`. This separates source stories from computed pages; parts are not duplicated for each rendered occurrence. `Story.get_text` reuses existing page formatting without inventing a page locator. Native part-ID ordering, legacy comment projection and note references are explicit metadata. Reference-only paragraphs retain their anchor blocks. Paragraph normalization walks ordered native formatting ranges and inline objects once, keeps incremental Unicode character counts, and slices link labels at recorded UTF-8 boundaries; it does not repeatedly rescan paragraph prefixes.

Table/paragraph layout setters remain on the same native editable owner. Column widths validate the containing page/cell and nested content; row controls use the native row properties, and uniform borders respect writer precision. Explicit page breaks insert the native page-break marker in a dedicated paragraph; page-break-before remains a separate native paragraph property. No custom pagination engine or Python table-layout algorithm is introduced.

## Text, vectors and diagnostics

DOCX list membership and range links modify public WordCraft model values. Native ranges remain revision-bound Unicode offsets; links reject unsupported schemes and oversized targets. Header/footer getters use a nonmutating part lookup, while insertion creates missing stories atomically. Lists and links share normal paragraphs and ranges; no separate list/document builder is exposed.

PDF font registration, shaping, embedding, vector serialization, metadata and outline operations use pinned public PdfCraft crates. The already-transitive `pdfcraft-content` and `pdfcraft-fonts` crates become direct dependencies without adding graph packages. PDF paths accept a bounded sequence of move/line/curve commands, opaque stroke/fill and native coordinate conversion. Bookmark paths are private, structural edits invalidate unrelated handles, and originating child collections refresh only when their parent identity is unchanged. Supplied fonts remain caller input; the Inter test fixture retains its OFL attribution.

A private bounded diagnostics accumulator groups detectable native constructs by fixed code, count and source examples. DOCX and PPTX extraction populate it while walking existing models; it does not deserialize unknown source markup or create a public capability framework. DOCX text normalization consumes object and formatting iterators monotonically, retaining exact Unicode/UTF-8 link offsets without repeated prefix scans.

Python bounded reads accept bytes, paths and binary streams, preserving caller stream ownership. Native entry points recheck input size; JSON serialization writes to a bounded buffer and accounts for escaped UTF-8 output. Diagnostics and payload budgets constrain adapter output, not every upstream parser allocation. The report example composes existing DOCX methods and conversion with no new runtime abstraction.

## Formatting and portable content projection

A bounded flat scalar patch crosses the private native authoring boundary. WordCraft character/paragraph values and public overlays/resolvers implement direct-property clearing and inheritance; no Rust model is deserialized from caller JSON. Named paragraph/character style handles resolve immutable native IDs, validate the entire affected inheritance graph, and expose direct or resolved property snapshots. Native table context participates in paragraph-spacing resolution. The Python facade uses typed keyword maps and a shared range/collection vocabulary, rather than a public formatting schema or builder framework.

PPTX ranges map flat Unicode offsets (including LF paragraph separators) to public DeckCraft edit positions after strict bounds checks. Native revision counters invalidate incompatible ranges. New links construct only URL or stable SlideId actions; whole-document checks protect reorder/deletion and account for links in tables, notes and grouped shapes. Native split/allocation preflights bound run/property growth before mutations, including existing whole-shape setters. Canonical readable underline names are projected to native tokens inside Rust. Mixed hyperlink reads remain explicit errors, while assignment can replace or clear the range through a private permissive lookup; stale/range checks still run first. Link URLs share native text validation so serialization cannot silently remove invalid XML characters.

Portable Markdown returns `dict[str, bytes]`; the mapping itself is the in-memory package. A private image-reference callback flows through existing page and recursive table formatting. Content hashes deduplicate media independently of placement identity. Generated paths cannot inherit traversal from source filenames; story files use numeric names and relative media references. A preflight walks bounded nodes/depth and estimates escaping growth, then package/manifest serialization checks aggregate encoded bytes. Existing source hyperlinks remain caller-visible data and are not fetched or sanitized.

## Section identity and native grid mutation

DOCX section IDs are append-only ordinals over native section-ending paragraphs. The Python Section retains its ID and routes fixed-shape property patches to that native section. Appending materializes independent part IDs using public native part creation and clone-on-write blocks; shared immutable media stays owned by the document. Missing header/footer references become independent empty parts to avoid implicit previous-section inheritance. Synthetic section markers stay outside ordinary paragraph collections. Existing native writers generate relationships.

PPTX table structural generations are keyed by stable native shape IDs. Shared Python table/row/cell wrappers dispatch using the existing format-specific argument prefix; they do not introduce another table schema. Public native grid functions implement structural edits, while adapter preflight rejects unsupported overlap/size cases and validates resulting budgets. Covered coordinates map to an anchor, the rectangular text view emits content only there, and changed-table generations reject stale positional row/cell handles. Explicit minimum-height/border limitations follow the pinned renderer rather than being emulated locally.

Cell margins/alignment reuse native public properties in both adapters. Direct alignment getters distinguish None inheritance from an explicit setting; resolved margin getters use native defaults. Ordinary letter-template and independent-copy loops live in an example file, leaving template syntax, data formatting and batch policy outside the package API.

## Grid structure and PDF geometry

DOCX now shares the existing Python table grid verbs with PPTX. Native per-table generations protect positional handles; structural edits unregister descendants while stable table IDs and unrelated handles survive. Public WordCraft grid mutations run on staged models with pre/post validation; surviving native block identities retain synthetic cell-placeholder bookkeeping. The adapter rejects orphan vertical-merge continuations instead of copying the missing native anchor-repair algorithm.

PDF page boxes use public organize BoxSpec and PageInfo coordinate transforms. A staged write/inspect refreshes effective geometry and sizes before commit. Python invalidates page-related handles and cached render state; changed-page backgrounds become unknown. RGB color is applied to native content operations, including supplied-font glyph operations. Native ContentTruncated warnings and outline_more inspection flags map to explicit resource-limit failures.

## Native notes and owned PDF fields

DOCX note creation builds a public native PartKind story and inserts a NoteRef at the checked UTF-8 offset derived from the Python character range. Built-in note/reference styles and native section NoteProps supply rendering and numbering. The ordinary story/paragraph facade is reused; native part identities stay private. Generic commit preflight compares semantic marker identities/multiplicities and rejects orphaning or duplication. Whole-document cloning preserves note parts; arbitrary fragment copying remains upstream work. Note body text stays editable; serialization supplies the native own-note mark.

PDF fields use the lower-level pdfcraft-forms crate at the existing PdfCraft revision. Only new owned models and their clones enter bounded field enumeration; imported AcroForms remain rejected. Shared Python field collections filter by page and index by immutable name. Structural field/page generations prevent stale handles from binding to a recreated name. Native mutations clone/stage, enforce a shallow script-free single-widget profile, validate fixed Helvetica appearances with native width metrics and reject undrawn fields/NeedAppearances. Existing native flatten performs removal; postconditions gate commit. Page organization revalidates the profile. No PDF field-tree or appearance serialization is implemented locally.

## Table formatting and outline-aware imports

Count construction expands only a bounded blank string grid before entering existing native constructors. Slice access returns ordinary lists of existing cell handles, avoiding a new collection/view abstraction. Typed formatting dictionaries are bounded before native conversion; table/cell patch handlers stage native property changes and reuse existing grid/content validation. Native layout checks effective available geometry, and public measurement/autofit determines fitted columns. Direct formatting snapshots do not claim resolved conditional styles or complete shared-edge border precedence.

Outline-aware imports add one optional keyword to the existing page insertion method. The adapter preflights source native bookmark snapshots/semantic keys, then calls insert_file_pages with native conflict policy. Source/post-copy snapshots verify count, depth, title/structure and supported destination mapping; the adapter does not serialize or remap outlines itself. Existing page generations invalidate related handles. Default imports and subset copies keep their documented no-outline policy.

## Conditional styles and multiline PDF labels

Table styles extend the existing style kind and formatting dispatch. WordCraft Style table parts and StyleSheet resolution own inheritance and conditional properties. The facade adds one optional region argument and table style/look patch keys, avoiding parallel cell-style or conditional-format builder objects. Native layout supplies one cell-width snapshot for affected-table validation, including ancestor and unregistered nested tables. Explicit margins and changed inherited margins must retain valid content allocation. Table-style snapshots distinguish base-chain resolution from final cell appearance.

PDF links share one bounded native text/provenance index containing normalized text, glyph byte/Unicode offsets and uniquely matched surviving paragraphs. Native line/text APIs determine spaces and newlines. Candidate selected glyphs must be contiguous and uniquely owned; accepted multiline block intervals merge only when they overlap. A single block rebuild joins exact native text and remaps all accepted labels, including adjacent single-line annotations. Failed identity, geometry or substring checks leave labels unresolved. This annotation-driven normalization neither infers columns nor changes plain-text extraction.

## Native region rendering and DOCX tabs/breaks

One optional clip tuple passes through the existing render entry points to `_core.render`. A shared private raster-region helper intersects displayed-page coordinates and snaps outward against the backend's existing scale/pixel extent. PdfCraft tile requests, WordCraft affine region rendering and DeckCraft offsets perform native rendering. Full-page branches retain their prior behavior. Native result dimensions must match the checked region; no recoloring/cropping workaround substitutes for unsupported transparency. Kurbo is promoted from an existing transitive dependency to an exact direct dependency solely for the public WordCraft transform type.

DOCX tabs extend the existing paragraph patch/snapshot representation. Native ParaProps tab values and style resolution own stop accumulation/clearing; preflight rejects unsupported enums, imprecise/duplicate positions and effective lists exceeding native serialization capacity. Break insertion uses public native text operations with a checked marker and existing text budgets/semantic guards, returning the normal revision-bound range. Page/column context checks keep unsupported story/table pagination explicit. No new tab collection, break object or pagination algorithm is introduced.

Extraction retains explicit segment source-byte starts and maps them to native placed-line ranges from the existing layout index. Binary searches over ordered segment starts avoid rescanning every line for each break. Column segments may share a page; earlier automatic wrapping no longer shifts explicit segment assignment. Remaining automatic wrapping within a segment retains the first-page projection limitation.

## Style relationships and document defaults

Style information adds canonical next/linked IDs; the facade returns the same owned Style handles. A bounded private relation patch stages native metadata changes and reciprocal link updates, while inheritance validation remains limited to based_on edges. Next relationships permit cycles; occupied linked partners are rejected before changes to the old pair. The implementation does not invoke excluded editor commands.

Document defaults reuse native default_chr/default_para and the existing character/paragraph patch parsers. Paragraph edits mark the native default paragraph part present for serialization. A flat direct snapshot follows the existing formatting vocabulary, with paragraph shading owning the background key. Native style/context resolution and shared affected-table/tab validation gate commit. No separate defaults owner, style-selection engine or property-inheritance implementation is introduced.

The pinned reader materializes an omitted default font size as 10pt while the raw resolver falls back to 11pt; default-size reset is therefore rejected before mutation. Other supported clears have effective-formatting readback coverage. Built-in heading links can reference absent companions: the facade reports that unsupported relation, while native clearing/replacement repairs the source without touching unrelated styles.

## PDF drawing resource preservation

Drawing mutations already stage a cloned native document. Before committing, a shared guard obtains effective inherited resources through public pdfcraft_model page traversal and compares existing font/image/graphics-state bindings. Unchanged native objects and a small supported scalar-dictionary equality profile allow normal repeated drawing; unprovable replacements fail explicitly. This protects original content without copying a PDF resource allocator or adding public API. The already-transitive model crate is a direct same-revision dependency.

## DOCX pagination visibility guard

Before native serialization, collect rows whose native layout cannot split them and whose cells have no vertical-merge geometry. Skip layout when that set is empty. Otherwise inspect public WordCraft placed-cell rectangles against physical page height, with bounded page/item traversal and explicit failures. This uses the native paginator and protects all authoring snapshot/render/export/save paths, including changes to fonts, margins and text. Traversal bounds follow layout allocation and are not a streaming memory guarantee. Exact-height clipping within a page is intentional; vertical merge anchors span rows and require separate upstream overflow evidence.

## PDF text areas and image transforms

The facade adds optional parameters to existing draw_text/add_image. A private text-box entry point shares native helpers with legacy point text; public native line/width/occupied-rectangle APIs validate built-in wrapping, while existing shaped font placement retains explicit-line behavior. A conservative bounded wrapping-work estimate gates repeated native layout. Images pass checked crop, rotation and flip properties to AddedImage, translating Python left/top/right/bottom order to the native order. Every staged path retains the resource-preservation guard; no raster crop, replacement text layout or new public drawing class is introduced.

Supplied TrueType box text uses native shaped glyph IDs/positions and bounded public Skrifa outline drawing; Kurbo computes curve extrema. Preflight traverses native parsed component references before recursive native lookup, then bounds scratch and path accumulation. Cached bounds reuse repeated glyphs without duplicating font parsing or shaping algorithms. Boxed CFF is rejected because the public evaluator cannot enforce an aggregate work limit. Built-in text retains logical metrics.

## DOCX paragraph borders

ParaPatch reuses the bounded border-value parser and records optional side changes, applying them to existing direct border properties. Native StyleSheet resolution supplies effective whole-group inheritance; no per-side paragraph cascade is invented. Paragraph and default snapshots expose four edges, while table-style snapshots retain the six-edge table border group. Shared side-aware DOCX validation excludes proven unsupported styles, widths and orientations. The facade reuses its existing border mapping normalizer; no new border object or method family is added.

## Ownership and shading acceptance

The private collection iterator captures its ordinary owned handles together with membership before yielding. Existing per-object identity/generation checks remain authoritative; the iterator does not mint fresh positional generations after a structural mutation. No public collection framework is added. Shading acceptance exercises existing native character/paragraph/table-style resolution and existing background/highlight patches, including independent page color; it requires no runtime implementation.

## Inline picture lifecycle

A small Pictures specialization adds deletion to the existing ordered collection; Picture.data uses native image_data/set_image_data through the ordinary source reader. The native document's add_media provides deduplication and collision-safe keys. The adapter follows public para_paths over body and all parts to account for its narrow authored Image/NoteRef profile, then prunes unreferenced bytes on the staged candidate before commit. It does not parse relationships or invent a general media-reference graph. Native paragraph deletion removes the sole picture marker while retaining the paragraph's stable locator. Broader graphics/background/OLE cleanup remains an upstream reference-traversal requirement.

Supplied TrueType box text uses native shaped glyph IDs/positions and bounded public Skrifa outline drawing; Kurbo computes curve extrema. Preflight traverses native parsed component references before recursive native lookup, then bounds scratch and path accumulation. Cached bounds reuse repeated glyphs without duplicating font parsing or shaping algorithms. Boxed CFF is rejected because the public evaluator cannot enforce an aggregate work limit. Built-in text retains logical metrics.

## DOCX metadata

WordDocument shares metadata/update_metadata vocabulary with PDF. Python preflights ordinary scalar values and bounded JSON chunks; native patches stage CoreProps/CustomProp values and use the existing commit path. Native typed values serialize through the DOCX writer, with authored-property budgets separate from body text/media. The extraction adapter adds supported core fields through its existing metadata projection. No generic property object, raw XML endpoint or package writer is added.

## PDF markup appearances

Existing annotation dispatch maps three additional kind strings to native Markup values. A narrow borrowed dictionary lookup preflights direct QuadPoints before native props() can build an appearance; resource estimates follow the native bounded vertex count without generating geometry. Staged generated AP stream lengths are read without decoding imported streams and charged to existing input accounting. Deletion reads only subtype. Native add/move/style/delete and view transforms remain responsible for PDF semantics and appearance paths.

## Section gutters and native columns

Two properties extend the existing section patch/snapshot boundary. WordCraft SectionProps.gutter and Columns.widths own serialization and native column_boxes own layout geometry. Explicit widths use the existing shared gap, with no gap after the final column. Validation rejects values the native helper would clamp and prevents summed widths from exceeding usable space. The narrowest native column bounds body tables/images and nested geometry because pinned table placement reuses its initial measured grid on later columns. New unequal-section body tables start at fixed native width equal to the narrowest column. Existing tables retain their properties and must satisfy geometry checks; serialization rechecks actual native table layout after content/font changes. No column collection, orientation alias or local flow algorithm is added.

## Authored slide layouts and placeholders

Slides.add selects a supported LayoutType through native defaults::layout_of_kind and defaults::new_slide. Native edit::insert initializes empty placeholder text bodies without direct style overrides. Existing shape handles project native placeholder roles, and resolve::Ctx/resolve::xfrm provide effective geometry. Setters copy the resolved transform only when needed, then change the requested fields. The shared staged model validation protects content/ID budgets and ownership. Masters, themes and layout references remain native model data; no template copier or new placeholder hierarchy is introduced.

## Combined table corpus

A development-only fixture builder composes existing heading, table, nested-cell, image, row and formatting operations. One acceptance loop reopens DOCX/PDF and checks semantic labels independently from actual representative pixels. Expected repeated-header multiplicity is explicit for PDF and does not contaminate source semantic counts. The fixture index links the six names to their evidence; no runtime abstraction or additional document backend is introduced.

## Picture source edits and text selection

SlidePicture specializes the existing Shape handle for native PictureFill media, shape description and placement transforms. The ordinary shape factory retains its image discriminator, and source reads use the same bounded Python reader as existing authoring. Native add_media supplies deduplication; the existing authored-profile reference pruning retains picture/background resources and live-byte checks gate staged mutations. No new image collection or rendering backend is introduced.

A private _slice_range helper serves Paragraph and text-bearing Shape indexing, normalizes a contiguous slice against owned text, checks the revision around native text_range and returns the same range type used by find/add_run. SlideTable retains EditableTable indexing through its existing inheritance order. Picture source/coordinate conversions finish before resolving the stable slide identity to an ordinal. Replacement, styling and links continue through existing native methods. No range-tracking structure, native method or document-standard logic is added.

## Numbering and fixed slide navigation extraction

DOCX Adapter builds a bounded (StoryRef, Path) observation map from existing native Placed::Lines once per layout. Effective NumRef and Label.text come from native resolved paragraphs; unlaid-out stories fall back to native style/table resolution. Labels use the native label-line byte offset to select an existing explicit-break segment. Conflicting repeated placements suppress only the label. Native numbering definitions provide parsed formats/templates, including the reader's 64-character template normalization. A conservative escaped-byte counter bounds repeated metadata across page and semantic table projections. Native definition results, including misses, are cached by instance/level. A conservative cumulative scan budget accounts for the pinned resolver's repeated linear instance, abstract and override traversals before each uncached call; exhaustion rejects the complete payload. Earlier native layout costs remain separate. No local list counter, style parser or visible-glyph reordering is added.

PPTX uses a bounded SlideId-to-page map shared by run/table/shape projection and diagnostics. Action::Slide and Action::Url reuse LinkPayload; contextual actions retain factual omissions. Native relationship parsing remains upstream, including its unknown/unresolved-action losses. The map guards parsed identities but is not a pre-import allocation or source-count guarantee.

## List starts and source slide-count bounds

The existing native list setter receives an optional checked start. It creates a fresh native list definition and sets Num.start_overrides for level zero; WordCraft still owns counters, labels, layout and serialization. Reusing the setter preserves continuation/restart semantics and avoids another list object. Header/footer ancestry is checked through the existing StoryRef registry because native measurement can consume a counter before visible placement.

PPTX preflight scans one bounded eligible ZIP member at a time using borrowed quick-xml events, stopping on non-presentation roots. It counts all sldId descendants conservatively, including alternate content and secondary lists, instead of resolving OPC relationships or selecting markup branches. UTF-16 BOM decoding follows the pinned reader's normalization. Generic ZIP safety limits still apply; parsed model indexing remains a second defensive check. The already-locked MIT quick-xml 0.41.0 is a direct dependency with no new package or upstream revision.

## Existing picture and field geometry properties

DOCX Picture properties project InlineObject::Image.crop and Float.flip_h/flip_v through typed native accessors. Staged edits retain the media key, frame and unrelated Float properties; replacement continues to change only the media reference. Shared DOCX image sizing rejects the pinned layout's sub-point clamp. Native rotation fidelity is a separate upstream correction.

PdfField.box reuses current-view rectangle validation/transforms and pdfcraft_forms::set_props for widget zero. Existing checked-owned-form validation protects field semantics and fixed appearance fit before staged commit. The Python setter converts caller coordinates before its final generation check and invalidates the normal output cache on success. Field structural generations remain unchanged because widget identity and page membership are preserved.


## Private Office render reuse

A private `_open_office` boundary dispatches to format-owned sessions and returns the same bounded normalized JSON alongside a private `_OfficeSession`. DOCX owns Document and DocLayout, discarding transient LayoutCache; PPTX owns Presentation. Mutex-protected optional state shares the existing PDF close/render lifetime pattern, with public model snapshots remaining in Python. Each adapter factors its existing raster validation into one checked helper used by cached and one-shot rendering. No image result cache or new public object family is added. Private counters distinguish parse/layout builds and render attempts reaching a live session; boundary-rejected malformed clip tuples do not enter those counters.

Python keeps the existing Document._render_session dispatch and strong local reference during calls. Close clears the private source/password and consumes retained native state; failed Python hydration also explicitly closes unreturned state. DeckCraft's private thread-local renderer and allocator/global caches are not owned by this session and cannot be explicitly flushed through its public API. Retaining a model/layout trades steady-state memory for avoiding repeated import/layout; post-layout page/item checks cannot constrain upstream peak allocation.


## Page fields and property metadata

Paragraph.add_field uses native insert_object to append a Field marker and returns the existing TextRange. Explicit PAGE/NUMPAGES/SECTIONPAGES instructions with Arabic numeric switches avoid the pinned layout's inconsistent implicit page-format inheritance. Section setup reuses its existing JSON patch with page_number_start; section append clears inherited restart. Commit preflight counts fields through public paragraph/story paths, so copies and auxiliary-story edits share the bound. Native layout evaluates values; DOCX stores an empty dirty cache rather than an adapter-computed number. Range replacement/deletion remains native and must not allow raw object-marker injection. Authored fields without legacy form XML contain no media references and pass the existing image-retention walk; opaque/raw field payloads and other unsupported objects remain outside that authored media profile.

A private strict native-property-to-JSON scalar helper is shared by the existing authoring metadata snapshot and extraction. Extraction preflights count/name/kind/value bytes and ambiguous folded names before copying; unsupported/malformed/ambiguous entries are omitted with bounded position-only notices. Existing table payload traversal projects native caption/description under title/description, with their bytes included in the story text budget. The native reader's lossy paths are retained as explicit upstream limits, not repaired by a second XML parser.

## Bounded PDF page selections

PDF copy/subset and page insertion reuse the private `_selected_pages` facade helper already used by selective text extraction. Bounded integer collection precedes native dispatch; copy resolves valid negative indexes against one stable final count. Destination page generation plus count detects incompatible callback mutations, including append-only page additions that preserve existing positional handles. Open-state validation runs before stale-selection validation. Native PdfCraft remains responsible for page import/subset semantics and preservation restrictions; source bytes are snapshotted before consuming selection callbacks.

## EPUB extension architecture

`rust/src/epub.rs` is a narrow LexePub adapter with shared ZIP and strict package/XML preflight; `rust/src/author_epub.rs` is a bounded semantic adapter over rbook. Python retains compatibility objects and save/lifecycle behavior. These exact-locked Apache-2.0 backends are explicitly authorized by the [EPUB design](../epub-support/design.md) because the selected ArtCraft repositories do not provide an EPUB engine. They do not authorize local reimplementation of EPUB packaging, rendering, conversion, CSS, DRM, or fixed-layout behavior.
## Executable usability cookbook

Five new standalone example modules plus the existing template recipe compose native authoring and PNG-byte rendering. Each exposes a normal function with an explicit output directory and a guarded argparse entry point. They format domain data with Python/Decimal, preserve native document ownership and write returned conversion/render bytes explicitly. No example implementation enters the public doc_ex surface. Installed-wheel acceptance imports the actual modules and runs their CLIs in temporary directories; shared PNG decoders verify artwork and requested resolution without requiring Pillow.

## DOCX image backgrounds

WordDocument.background_image reuses bounded Python source reading and native PNG validation, with private native background_image/set_background_image methods on the existing editable owner. The adapter assigns public Settings.page_picture to a PagePicture with frame fill and a media key returned by Document.add_media. Image and color remain independent native settings. The existing authored media-retention walk includes this one global reference before paragraph images; shared data counts once, and it is not counted as an inline placement on each rendered page. Empty-media commits still validate a nonempty background reference. No separate background object, image compositor, layout logic or image-processing dependency is introduced.

## Presentation cell paragraphs

A private Paragraph adapter reuses existing slide ranges and shared slicing/find/replace helpers. Its private target contains canonical row/column, paragraph ordinal, table generation and cell body generation; optional private keyword forwarding lets existing native text/range methods select a cell paragraph without duplicating a method family. Native helpers resolve the public TextBody and relative Pos values, while DeckCraft edit functions own insertion, deletion, formatting and link runs. Appending to a body with no paragraphs uses native first-paragraph insertion without a separator. Grid edits prune cell-generation entries and advance the table generation so old raw targets cannot revive. No cross-paragraph identity remapping is implemented.

Shared literal find/replace normalizes validated string operands with base str semantics before truth, length, equality, handle capture or staged mutation. String-subclass hooks must not execute inside a replacement transaction and overwrite caller-owned changes.

## Everyday API typing and handle representations

Private standard-library input aliases, typing.Self and literal overloads retain existing runtime signatures and small root exports. Deferred annotations avoid importing authoring implementations into extraction at runtime. Existing factory overloads and generic collections remain the common ownership vocabulary. Three child reprs inspect only stored identity and owner.closed, with a no-ID fallback for the private slide-cell paragraph adapter. A strict mypy consumer checks installed-package result types; imported-module diagnostics are silenced only to scope this gate to user chains, not to certify internal package typing. Pinned checker dependencies live exclusively in development tooling.
