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

Every upstream repository is pinned to an immutable commit:

- PdfCraft v0.9.0, reviewed contribution fork `narain1/pdfcraft`: `516caea956c0aaf15f2bd1f35a15e07c73a9ead6` (reviewed parent `60a8d7998b64c7718e08e9054e99f1f2eb1eda8d`, original upstream base `645016addcfb9078c557f0189559ab47cdcee057`)
- WordCraft v0.8.0, reviewed contribution fork `narain1/wordcraft`: `5312722fb1cefed837f00227e3afe919a6451b54` (reviewed parent `f04bc4ca09e009c7cf076a31a044aa4c9b8a87ef`, original upstream `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`)
- DeckCraft v0.8.0, reviewed contribution fork `narain1/deckcraft`: `c9c7fff189c808bf7266d0b888c3636a1880080d` (reviewed parent `dd4fd70b6347dd2ce782e33f030a2b6c0abb0106`, original upstream base `22db9261f9d1a214c296fd390173130c45d76d66`)
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

`Page.get_pixmap(dpi=144)` and `Document.render_page(page, dpi=144)` return PNG bytes. PDF uses PdfCraft rendering, DOCX uses WordCraft layout/render, and PPTX uses DeckCraft rendering. PDF eager open privately retains the same PdfCraft `PageRenderer` used for text extraction and reuses its parsed document for later renders; `Document.close()` drops that state while hydrated values remain owned. The one-shot `_core.render` boundary remains for non-session routing and compatibility. DOCX privately retains its parsed model and native layout, and PPTX retains its parsed presentation, for repeated rendering; closing the document releases these Office sessions. These dependencies are included unconditionally in the current wheel; extraction-only feature builds are not implemented.

A private `rendering` helper validates finite DPI, raster dimensions and RGBA length. PDF/DOCX encoding unpremultiplies color channels and writes PNG physical resolution; PPTX retains upstream `to_png_at`. The shared raster budget is 64 million pixels. WordCraft/DeckCraft dimensions are preflighted before raster allocation. Full-page PDF requests additionally apply the shared budget to public native `device_pixels` dimensions derived from inspected CropBox/rotation/UserUnit display geometry before calling the renderer. Native `reject_oversize` remains enabled: its continuous f32 area check alone does not bound the product of separately rounded integer sides. The selected DeckCraft PNG path calls the reviewed fork's `try_render_slide`; native failures map through private `rendering_failure` to existing DocExError. Native PDF export propagates the same fallible result through existing conversion_failure. A valid blank slide stays successful; global counters and pixel colors are not failure signals. Broader upstream allocation/work limits and other entry points remain U09.

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

The [upstream register](../../../docs/upstream-roadmap.md) records a pinned-source audit. WordCraft and DeckCraft writers and numerous editing/background primitives already exist. Confirmed focused candidates include a public PdfCraft table detector, the now-contributed fallible DeckCraft render path instead of blank-image fallback, and lower-level WordCraft revision-resolution routines currently located in engine code. Public availability, current integration and verified fidelity are separate states. Missing format algorithms belong upstream; adapter code must not duplicate them or link prohibited engines.

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

PDF fields use the lower-level pdfcraft-forms crate at the existing PdfCraft revision. Only new owned models and their clones enter bounded field enumeration; imported AcroForms remain rejected. Shared Python field collections filter by page and index by immutable name. Structural field/page generations prevent stale handles from binding to a recreated name. Native mutations clone/stage, enforce a shallow script-free owned single-widget or immutable same-page radio profile, validate fixed Helvetica text/choice appearances with native width metrics and reject undrawn fields/NeedAppearances. Existing native flatten performs removal; postconditions gate commit. Page organization revalidates the profile. No PDF field-tree or appearance serialization is implemented locally.

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

The selected reader now preserves an omitted default font size and the native resolver’s existing 11pt fallback (spec127/131). This changes imported parsed-but-unsized styles from the former 10pt fallback. Default-size reset remains rejected before mutation pending separate owned-reset admission (spec132). Other supported clears have effective-formatting readback coverage. Built-in heading links can reference absent companions: the facade reports that unsupported relation, while native clearing/replacement repairs the source without touching unrelated styles.

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

TBL7/BG7 retain the existing formatting surface. Cell background snapshots remain direct values; `None` removes the direct override and restores inheritance. Whole-table and conditional style fills already provide native solid table appearance. Do not add a direct table/row fill setter while native layout ignores those properties, or interpret white/`None` as explicit transparency. Patterns, editable theme identity and cancellation of inherited fill need upstream model and resolution support. Existing conditional-border guards remain independent of these fill gaps.

Page-border acceptance first needs a raw native asymmetric-margin fixture comparing the serialized offset mode with public placed rules, then reader/write retention and visible PNG/PDF output. The current source-only page-edge/text-relative mismatch is recorded under existing U02/BG7 ownership in the [upstream register](../../../docs/upstream-roadmap.md#redaction-and-table-fidelity-audit); it does not authorize a facade setter, coordinate workaround or new completed criterion.

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

## Executable usability cookbook

Five new standalone example modules plus the existing template recipe compose native authoring and PNG-byte rendering. Each exposes a normal function with an explicit output directory and a guarded argparse entry point. They format domain data with Python/Decimal, preserve native document ownership and write returned conversion/render bytes explicitly. No example implementation enters the public doc_ex surface. Installed-wheel acceptance imports the actual modules and runs their CLIs in temporary directories; shared PNG decoders verify artwork and requested resolution without requiring Pillow.

## DOCX image backgrounds

WordDocument.background_image reuses bounded Python source reading and native PNG validation, with private native background_image/set_background_image methods on the existing editable owner. The adapter assigns public Settings.page_picture to a PagePicture with frame fill and a media key returned by Document.add_media. Image and color remain independent native settings. The existing authored media-retention walk includes this one global reference before paragraph images; shared data counts once, and it is not counted as an inline placement on each rendered page. Empty-media commits still validate a nonempty background reference. No separate background object, image compositor, layout logic or image-processing dependency is introduced.

## Presentation cell paragraphs

A private Paragraph adapter reuses existing slide ranges and shared slicing/find/replace helpers. Its private target contains canonical row/column, paragraph ordinal, table generation and cell body generation; optional private keyword forwarding lets existing native text/range methods select a cell paragraph without duplicating a method family. Native helpers resolve the public TextBody and relative Pos values, while DeckCraft edit functions own insertion, deletion, formatting and link runs. Appending to a body with no paragraphs uses native first-paragraph insertion without a separator. Grid edits prune cell-generation entries and advance the table generation so old raw targets cannot revive. No cross-paragraph identity remapping is implemented.

Shared literal find/replace normalizes validated string operands with base str semantics before truth, length, equality, handle capture or staged mutation. String-subclass hooks must not execute inside a replacement transaction and overwrite caller-owned changes.

## Everyday API typing and handle representations

Private standard-library input aliases, typing.Self and literal overloads retain existing runtime signatures and small root exports. Deferred annotations avoid importing authoring implementations into extraction at runtime. Existing factory overloads and generic collections remain the common ownership vocabulary. Three child reprs inspect only stored identity and owner.closed, with a no-ID fallback for the private slide-cell paragraph adapter. A strict mypy consumer checks installed-package result types; imported-module diagnostics are silenced only to scope this gate to user chains, not to certify internal package typing. Pinned checker dependencies live exclusively in development tooling.


## Bounded operation evidence ledger

The development JSON records references, fingerprinted fixture sources, seven initial operation profiles and all 139 coverage joins. The stdlib generator derives criterion text, checkbox state, owner and role from the existing roadmap and renders one Markdown support matrix. It validates structural/reference drift without importing document engines or running conversions. Supported cells retain the historical cookbook artifact identity and explicit audit baseline; matching test hashes do not promote that evidence to changed native/facade code. Separate SDK/PyMuPDF member enumeration remains unfinished, and there is no public support-query object or dispatch framework.


## Pinned reference-family discovery

A separate comparison inventory describes 44 families with 194 representative members from 17 cited immutable source files. It stores source provenance and family-to-criterion/profile mappings, not tested operation statuses or duplicated ownership. A stdlib checker derives the readable guide and all-criterion reverse join, optionally verifies an explicitly supplied local source cache, and performs no network access. The support matrix links matching reference families after validating the inventory. Comparison-product names remain references; ArtCraft format crates alone own runtime behavior.


## 51. Header/footer rich-content reuse

EditableStory delegates table/picture creation and direct-child discovery to private native methods, reusing existing input normalization and atomic owner staging. Native adapters validate Header/Footer ownership and use the minimum full text width of referencing sections. Notes remain explicitly unsupported. A separate story_placeholders map tracks only synthetic empty anchors; table insertion leaves a trailing anchor, while image/text insertion consumes it. Section/document copies preserve or remap this state. Removing a direct picture promotes its retained locator into the ordered paragraph collection without reusing identifiers.

The existing serialization layout guard activates for rich header/footer parts as well as guarded table rows. Public placed Cell/Image/Object frames provide physical-page checks; explicit clipping-cell path prefixes permit intentionally clipped descendants while the clipping frame itself remains checked. The existing page and placed-item caps apply after native layout. No header layout or text measurement algorithm is reproduced. Inactive variants can lack measurements for nested operations and later section geometry edits; such failures leave the owner unchanged.

The shared DOCX renderer sets RenderOptions.display.dim_header=false: the upstream default is an editor preview effect, while this package exports document images. Full/region and retained/one-shot paths therefore keep saturated story colors consistent with native PDF without changing body rendering or other display settings.


## 52. Section line numbering through native layout

Section.line_numbers follows the existing tuple/property vocabulary and passes a bounded JSON patch to SectionProps.line_numbers. Native LineNumbering and LineNumberRestart represent the settings, with source start offsets and twip conversion delegated to the native writer. The public count/start bounds match the pinned parser's representable range. No independent line counter, font measurement or page mapping is introduced.

A shared candidate validator checks section combinations and conservatively bounds numbered body work. Four units per UTF-8 byte covers native case-expansion clusters; native fields and note references shape atomically into one cluster, and inline objects add one unit. Ordinary line breaking consumes clusters plus at most a trailing empty line, while beside-float filler lines are not numbered. Tables and auxiliary stories use separate paths. Single-column restriction also excludes native column-balancing replay. Both full-layout entry points preflight directly, covering section/table geometry validation before commit as well as serialization; commit and serialization retain defensive checks.

The existing authored pagination guard activates for numbered sections and shares one layout with table/header/footer frame checks. Native DECOR_STORY items with empty paths identify labels; their placed line extents are inspected without reconstructing glyphs or counters. Label paragraphs must remain one horizontal line. Logical bounds and post-layout page/item caps do not claim peak-memory or full glyph-ink containment.


## 53. Story and line-number evidence joins

Two bounded DOCX profiles extend the existing development ledger to nine profiles and 63 operation cells. Each cites its own frozen wheel/source commit and unchanged test fingerprints. Three additional roadmap criteria gain profile joins, retaining their existing canonical owner and unchecked broad scope. The headers/footers and page-fields reference families connect to these profiles; LineNumberType is cited from the already pinned SDK schema snapshot. No runtime metadata API or duplicate capability database is introduced.


## 54. Shared formatting keys for direction and language

CharPatch maps language to native CharProps.lang; ParaPatch maps direction to ParaProps.bidi. Existing parse/clear/overlay and direct/resolved snapshot helpers carry both fields. Explicit ltr stores Some(false), while None clears inheritance. The language profile prevents values of 64 or more bytes from disappearing in the pinned reader; case and spelling remain caller-owned. Existing Python formatting TypedDicts and consumer checks gain the keys without another method family.

Native paragraph bidi resolves through styles/defaults and whole-table paragraph properties. PartFormat conditional regions retain their current special alignment support and character properties: language is supported there, direction remains explicitly invalid. Native shaping and logical alignment determine visible placement; TableProps.bidi_visual and cell text orientation remain separate. Concrete mixed Persian/Latin/digit glyph geometry and visible PNG/PDF extents validate this bounded profile, while Hebrew/emoji/combining source preservation does not imply complete typography coverage.


## 55. Standard PDF shape comments

The existing annotation adapter maps rectangle/ellipse to native Shape variants and Square/Circle readback. Existing native set_rect/set_style regenerate appearances transactionally. A borrowed dictionary validator admits only small direct geometry/color/solid-border values, bounded metadata and an optional N-only appearance reference; foreign AP bytes are not decoded. Native three-decimal rectangle/four-cubic appearance generation has fixed complexity under bounded coordinates. A conservative 16KiB allowance is checked before native work, then actual compressed stream bytes are charged, including both resize and recolor streams. This is not a zlib/process-memory guarantee.

The minimum three-user-unit dimensions retain at least one unit after native stroke inset and avoid a reproduced tiny-oval disappearance. No local drawing or path repair is introduced. Initial facade ownership checks remain before conversion, with final checks after box/color/page-index normalization so callbacks cannot retarget pages or annotations. Existing summary/inspection allocation precedes these mutation guards and remains separately documented upstream work.


## 56. Direction and geometric-comment evidence joins

The development ledger extends to eleven profiles and 77 operation cells. The direction profile cites its own 342-test artifact; the geometric-comment profile cites the reviewed ellipse artifact rather than the superseded oval spelling. Existing reference formatting/annotation families and TXT6/PDF6 join these profiles. The 44 reference families, 195 representative members and 139 criterion owners/checks remain unchanged. No runtime capability query, public method or backend dependency is added.


## 57. Named ranges reuse existing text handles

A small DOCX mapping delegates to four narrow native bookmark methods. Native para_paths/object_offsets locate validated BookmarkStart/End pairs; lookup reuses registered paragraph paths and returns an ordinary TextRange. Source-order iteration uses Document.bookmarks. End-first deletion and end-before-start insertion use public native operations. Same-paragraph relocation adjusts only the two removed own markers; it is not a general range-tracking layer. Existing semantic-anchor checks allow one exact validated pair deletion and preserve every other anchor.

The central all-story scan validates paired names and all internal link targets. It records marker offsets once per paragraph, discounts only preceding bookmark-marker bytes and requires each marker at logical offset zero or the content end. This invariant applies to every staged commit and output, including later text edits. Targets therefore cover whole paragraphs or endpoint points; interior markers split native shaping segments and are rejected. Default marker properties remain unchanged: native Marker clusters have zero advance and are excluded from line-height/trailing-alignment metrics. The shared authored pagination pass activates when bookmarks exist; it uses native layout.caret and public page_display. In the admitted authored-inline-object profile, every exported link rectangle comes from at most one nonempty linked Glyphs batch, and wrapper recursion does not multiply those calls. The conservative batch count includes repeated auxiliary stories and external links, avoiding the native 10,000 rectangle cap without duplicating native coalescing. Warped/unknown graphics remain excluded.

Source-linked vertical Placed::Lines require a separate check because their display has already lost link metadata. Linked Clip/Turned/Rotated/Figure contexts are rejected; Figure is an unaudited profile restriction, while clipped overflow and vertical disappearance have concrete native repros. All actual target carets must be on a physical page, with no fallback. Creation/movement/internal-link assignment validate staged geometry, while subsequent ordinary formatting edits are rechecked on output. Existing layout page/item limits, a 2,000,000 observed path/display-node bound and depth 32 remain distinct from native allocation guarantees.


Bookmark lookup preflight charges public body-index lengths plus each indexed paragraph layout's line/cluster lengths before calling native caret, then charges destination-page scans and per-placement source-run checks. Native PDF export caches names to positions, not resolved carets; the same conservative per-name estimate is charged for every observed internal linked glyph batch. A shared 2,000,000-unit cumulative budget also covers the display walk. Native coalescing is not reproduced. Layout/display allocation and initial document-wide name inventory remain outside this observed-work estimate.


## 58. One PDF writer with a representation switch

The Python PDF overrides add one keyword and strict bool validation; the shared private save callback retains a single filesystem/stream implementation. Destination normalization still precedes serialization. Native PyO3 exports to_bytes with a defaulted object_streams bool while an internal Rust to_bytes wrapper preserves current callers. Both call the same private serialization helper, changing only SaveOptions.object_streams and checking the complete returned vector against the existing 256 MiB limit. A small private limit parameter permits actual boundary tests for both native output representations without allocating huge fixtures. No caller-controlled output path enters Rust.


## 59. Clear a story without replacing its owner

EditableStory.clear delegates to one native clear_story operation returning whether a mutation occurred. Missing/synthetic empty stories return false; the facade invalidates its cached read snapshot only on true. Native part-kind and actual section-reference checks precede the public Document.set_story call. The replacement retains a native empty paragraph as a synthetic insertion anchor, without exposing it as an authored child or changing section inheritance.

Existing staged commit checks run before publishing location, picture, table-generation and placeholder cleanup. Only the selected story's children are invalidated, and stable IDs are not recycled. content_usage uses native object_offsets to subtract image-marker bytes only, matching established insertion accounting for fields/notes/bookmarks and correcting copied-story and cell/grid removal totals. No new resource graph, XML edit path or layout algorithm is added.


## 60. Search through the retained PDF session

A narrow private session method requests native text interpretation, checks content warnings and bounded glyph/index vectors, then delegates literal matching to PageText.find_opts_limited and geometry to line_rects. Native KMP runs in amortized O(source+query) time, but builds its source character buffer before honoring the result cap. The adapter therefore preflights UTF-8 source bytes, conservative lowercase scalar expansion and possible separators before calling it. Result ranges/line transitions are counted before geometry allocation; native rectangles remain unchanged.

The Python facade only validates query/ownership and wraps owned tuples. Editable pages reuse their checked current read snapshot. Ordinary read documents reuse the retained native parser; manually constructed documents without a session use and close a temporary native session. No raster/text-result cache or equality to normalized table order is implied. ActualText and ligature boxes can cover more than the matched substring; native reading order can cross columns.


## 61. Recheck the PDF page after normalization

The facade retains each method's initial PdfPage._check, normalizes coordinate/color/font/image-stream inputs into local values, then calls the same check immediately before dispatch. Native arguments at that point have no remaining supported user callback conversions. rotate uses operator.index before this final check, avoiding conversion while PyO3 holds a mutable owner borrow. set_box/rotate refresh the handle generation only after successful native mutation, as before.

This uses the existing generation invariant without a transaction or callback framework. Page moves/deletions performed by callbacks remain effective, while the stale requested drawing/geometry change fails. Copy-only callbacks leave the original page live and permit the operation. Read rendering continues through its captured snapshot; this change does not introduce a new rendering cache or mutable-handle rule.


## 62. Reuse native Legal numbering and placed geometry

One private matcher compares nine effective native Level values against once-per-operation native bullet/numbered/Legal definitions. It does not compare start overrides or evaluate counters. Continuation retains the existing nearest earlier same-story sequence rule, caches unique num matches and bounds cumulative native vector/override lookup work at 2,000,000 units. All authored definitions remain subject to the existing 10,000-definition cap.

A cheap canonical-Legal activation pass collects top-level source paragraph indices with owning section indices. It bounds body visits at 100,000 and target work at 100,000 (four per UTF-8 byte, one per paragraph and inline object). The existing native pagination pass supplies Page.body, section identity, Label extents and first-line caret coordinates. The adapter checks finite logical extents, resolved-direction nonintersection and body containment, without computing indentation, bidi placement or counters. Empty text carets are permitted; missing/duplicate label placements fail. Existing 10,000-page and 2,000,000-item limits remain, with a cumulative 2,000,000 label-glyph/run/caret scan budget.

Assignment validates staged geometry; every commit rechecks cheap scope, and save/render/export recheck geometry after later formatting. There is no cell-map framework, ancestor transform reconstruction or second paginator. The new profile does not change the old bullet/numbered narrow-cell limitations or imply arbitrary glyph-ink containment.


## 63. Resolve presentation ordinals after normalization

PPTX handles retain stable slide and shape IDs, unlike the conservative PDF page-generation model. For Slide.background and Shape.rotation/fill/box/color/font_size/add_run, the facade first validates a slide through its existing number lookup or a shape through the cheap native shape_placeholder lookup. It then normalizes numeric/color/format inputs and resolves the current slide index again for native mutation. Existing native per-slide shape lookup rejects deleted shapes; no identifier-to-ordinal cache or replacement handle is added.

The same callback discipline applies to generic range formatting: an initial existing revision query provides fast stale/closed rejection without retrieving text, while the native mutation still enforces the captured range revision after callbacks. Changes made by the caller are preserved; this is not automatic rollback of callback effects.


## 64. Guard native conditional-edge loss in the existing style parser

The staged conditional table-style border branch first runs the shared border parser, then examines only explicitly supplied non-null sides against the native TablePart. Unsupported combinations raise the existing unsupported error before candidate commit. There is no table-layout scan or new resource accounting: the restriction follows the reusable style definition independently of current attachments.

Pinned region_borders converts conditional edges into cell left/right values and clears inside_v; subsequent interior selection reads the cleared inside_v instead of those mapped values. The adapter records and rejects that native limitation, retaining native source generation and layout as the sole authorities. Explicit nil suppression is affected in the same way; None remains a safe removal of the direct override.


## 65. Tooltip metadata through the existing owned-field model

field_payload adds the existing native Field.tooltip string, and one narrow set_field_tooltip operation sets FieldProps.tooltip on a cloned document. Empty input removes native TU. Full expected-field equality checks that value/default/options/flags/widget geometry and identity remain unchanged, then existing owned-field appearance inspection charges regenerated raw bytes. The conservative appearance reservation helper is shared with widget-box updates; tooltip metadata has its own bounded input charge and participates in the existing aggregate live text budget.

The facade uses base str length/conversion/encoding operations and retains initial/final structural checks. No generic form-property patch language, new field container, imported-form reader or metadata renderer is introduced. Native metadata updates can allocate replacement appearances even when pixels remain identical; this is not a constant-allocation operation or secure removal promise.


## 66. Native lock flag with a bounded target profile

The existing annotation JSON adds Summary.locked, and one private setter delegates to public pdfcraft_annot::set_locked. The facade uses an actual-bool check. The setter first borrows the page Annots array, rejects inline entries before native annot_ref can clone them, and validates the selected dictionary against a <=20-entry canonical schema. It permits bounded scalar metadata, finite direct geometry, existing bounded markup quads, canonical geometric borders and a reference normal appearance. It does not decode appearance streams or traverse imported reply graphs.

Notes use native flags28/156; the other six supported comment kinds use4/132. The existing rectangle/ellipse guard admits4/132 so locked text editing and later unlocking remain usable. Copies are reserved conservatively using the page reference count and bounded target data; a flag change charges a small scalar mutation and no appearance regeneration. The staged target must equal the bounded original with only F changed. No field/widget framework, ownership tracker, format parser or general mutation language is added.

Native move/set_rect/set_style/set_info/delete check Locked; set_contents and set_locked do not. Links use a different setter that does not enforce the flag, so the property rejects links. Native Invalid retains its existing DocExError mapping. Readback uses the existing permissive Summary flags interpretation (including integral real numbers) and retains its inventory limitations; it does not certify imported dictionary validity.


## Incremental navigation, lifecycle and output evidence

Five additional profiles reuse the existing seven-operation ledger schema and generated support/reference guides. Existing roadmap ownership remains the sole source of owners and checkboxes. The new profiles cite the already validated a94a5ba runtime artifact; their reviewed test-file digests match that runtime exactly. This documentation join does not build or certify a new wheel. Bookmark source inventory, broad imported preservation, advanced output profiles and auxiliary numbering remain explicitly outside the corresponding tested scopes.



## 68. Note conversion through native semantic editing

Notes.convert normalizes a short literal kind, calls the native conversion entry point and invalidates the cached snapshot only when the returned count is nonzero. Native preflight charges paragraph/object/run work through public paragraph paths within the existing authored nesting bound; the potentially quadratic converted-reference/run comparisons are charged before calling Document::convert_notes. Text scanning remains bounded by the existing live-text budget.

The conversion-specific private commit allowance validates original part IDs/kinds and the exact expected semantic-anchor map before running the ordinary commit guards. Ordinary edits cannot use this allowance. No locator, ID, cell/media/text budget or section setting is rebuilt: native conversion changes part/reference kinds and standard note styles in place on the staged model.

The native operation changes an entire containing run's standard reference style. Preflight uses public object offsets to permit changed standard-style runs only when their complete byte spans contain converted markers. Other shared content fails explicitly; custom styles remain unchanged. The raw native source/PNG/PDF regression records the upstream behavior, while public recovery uses existing style/range formatting. There is no local run split, counter evaluation or note placement algorithm.

## 69. Extend coherent profiles without duplicating certifications

The existing pdf.owned-fields profile adds field.tooltip and public native FieldProps/Field references. Only read/edit_authored/preserve/render cells that gain tooltip evidence move to the exact tooltip artifact; creation keeps its historical citation. Existing creation/value/geometry tests and the new tooltip module are fingerprinted at that artifact, but saved output pixels do not establish imported field metadata inspection.

A separate pdf.comment-locks profile covers the flag property across seven comment kinds without certifying their creation/style APIs again. It reuses PDF6 and REF-PM-ANNOTATIONS, keeps bounded imported rectangle/reopened-native evidence explicit, and leaves creation/export not applicable. Cached immutable reference citations add Widget.field_label and Annot.set_flags while stating local default/subset differences. No capability-query framework, ownership database or new criterion is introduced.


## 70. Reuse owned-field flag and appearance paths

field_payload adds Field::read_only(), and one private set_field_read_only method toggles only flags::READ_ONLY in the expected native Field before calling FieldProps.read_only/set_props. checked_owned_fields masks only that bit when checking the existing canonical zero or COMBO base flags. No generic public flags setter, imported reader or additional field kind is introduced.

The existing owned_field_appearance_peak and owned_field_appearance_bytes helpers reserve/charge redraw work just as for tooltip and box changes. Complete Field equality verifies unrelated metadata/widgets/values/defaults/options. set_props permits changes to read-only fields and regenerates AP; native set_value independently refuses filling. Its existing conversion_failure mapping remains DocExError. The facade adds no override path and retains earlier value-validation errors. Widget Locked is separate and is not exposed by this property.


## 71. Remove the existing slide background override

A narrow private clear_background(slide) stages Slide.background=None through the existing model mutation path. The facade accepts None on the existing background property; color assignment keeps existing coercion and initial/final identity checks. Getter behavior stays direct: no override yields None even when the native effective background is an inherited image or gradient. Native resolve::background owns layout/master/theme fallback, serialization omits p:bg, and native rendering/PDF export observes that same model.

Only explicit-picture removal invokes existing authored media pruning and live-budget refresh; solid/absent reset cannot release media. Before that pruning, direct picture backgrounds in native master/layout fixtures are rejected rather than removed accidentally. This narrow safety check does not enable arbitrary imported asset graphs or add a generic reference walker. Successful repeated reset still follows normal revision/cache invalidation.



## 72. Character spacing through the existing native patch

CharPatch maps letter_spacing to CharProps.spacing and clears that optional field on None. A small scalar validator rejects nonnumeric/nonfinite/out-of-bound or non-twip values, allowing at most four f64 machine-epsilon units scaled by magnitude for ordinary arithmetic. It canonicalizes the accepted integer-twip value before f32 conversion; snapshots convert stored twips back to point values without float-storage noise. Native resolver limits [-100, 100] are checked before mutation instead of silently clamping. No per-character positioning loop, layout cache or new public object is added.

Existing overlay/resolution carries defaults, paragraph/character styles and conditional table character regions. Existing Python formatting TypedDicts and the strict consumer gain one key. The initial PPTX unknown-property rejection is superseded by the bounded existing-key extension in section 74. Native source fixtures distinguish intentional negative overlap from corruption: all source text stays owned/exact, while visual PDF text inference may reorder or insert spaces. Pure-text acceptance compares whole images; rich-story fixtures compare every chromatic text pixel and exact text bounds because the zero-spacing baseline already has independent grayscale table-grid raster differences. This is no broad pixel-perfect conversion claim.


## 73. Reuse operation profiles and distinguish semantic mutation from export

Owned-field read/edit/preserve/render cells add read_only assertions and the exact 423 artifact; creation retains its earlier artifact and imported/format-export limits are unchanged. A new note-kind mutation profile uses the 419 artifact, with stable owned handles and explicit custom-style/native-placement limits. Its export cell refers to the separate document.convert("pdf") operation. A slide background-reset profile uses the 426 artifact and distinguishes explicit getter absence from native effective inheritance. Raw inherited-model controls do not establish arbitrary imported source editing.

All profiles keep the existing seven-operation schema, 139 coverage joins, 14 owners and immutable reference families. The existing widget reference file supplies one additional representative field_flags member, with local READ_ONLY-only behavior explicit. Profile generation/checking validates links and drift, not semantics; exact historical test/runtime evidence remains necessary.


## 74. One slide formatting case, native placement

FormatPatch adds a Spacing(Option<f64>) case and sets only RunProps.spacing. The small scalar guard uses the same point/range/precision policy as DOCX while retaining f64 storage for DeckCraft; no shared layout helper or public options registry is introduced. Existing preflight_format/edited_body and native edit::format/insert own run splitting, inheritance, limits and commits. None clears the optional field; explicit zero survives native hundredth-point serialization. No source model snapshot/getter or native interface is widened.

Native resolve::run fills omitted properties from paragraph/list/placeholder defaults. Native text layout scales tracking during autofit and applies it after drawn graphemes/spaces. PDF export uses the same placed origins for its invisible glyph layer, independently of its visible slide raster. Tests compare same-glyph displacement rather than equating font advance boxes with ink bounds. The source/PDF raster fixtures use whole-pixel page dimensions at the native 200-DPI export resolution; they do not hide or repair existing arbitrary-DPI dimension quantization differences.


## 75. Validate the cell paragraph before append normalization

_SlideCellParagraph.add_run performs an existing native cell_paragraph_alignment lookup before _format_patch. This checks the canonical target and both generations and returns only an optional static enum without copying paragraph text or introducing a new native endpoint. Existing final add_run_patch validation remains after callbacks and resolves the current stable slide identity. The change is separate from character-spacing support and applies to existing supported formatting keys.


## 76. Owned note content clearing

Note.clear uses a private clear_note(id)->bool native call through the existing EditableStory method. It validates owned part kind, builds a canonical styled blank paragraph, and uses public Document.set_story without changing Part.kind/body or external references. The native writer supplies the note's own number mark as it does for existing add_note. The editable owned model never synthesizes an additional own-reference marker.

A small private clear_owned_story_content helper shares only the previous header/footer commit/accounting/publication sequence. It subtracts content_usage charges (excluding only uncharged image markers), runs unchanged ordinary commit validation, then discards that story's child locations/images/table generations/cell placeholders. The story stays owned with an empty explicit paragraph list and a synthetic placeholder at0. Monotonic child IDs are not reset. Header/footer reference/alias/no-op policies remain in their caller; note no-op requires the exact canonical styled paragraph for its current kind.

The bool result controls Python cache invalidation; false does not create a revision. A real clear preserves Note/unrelated handles but follows global range invalidation. Generic protected anchors still reject deletion, including unexpected imported-style own references; there is no semantic exception. Native conversion changes the canonical note style so clear→convert→clear remains a no-op. Blank notes still render their own mark and occupy native layout space.


## 77. PDF integer raster preallocation boundary

The existing one-shot and retained PDF paths share the same full-page preflight, including after a smaller cached render. At 72 DPI, an 8000.5 by 7999.5 point native view has a continuous f32 area of 64,000,000 but rounds to 8001 by 8000 device pixels (64,008,000). The adapter rejects that request before native output allocation rather than relying on the later PNG buffer check. It calls public native rounding, not a duplicate raster-sizing algorithm. Region rendering retains its existing quantization, tile-side and offset checks; small clips from a large page remain supported. Parser/layout/intermediate/encoder memory is not bounded by this one-buffer limit, and broader DeckCraft allocation/work limits remain U09/R4; spec114 subsequently replaces the selected PNG/PDF fallback paths with per-call failures.


## PDF field creation callback ownership

PdfFields.add keeps its initial _page_number validation before touching caller inputs. It materializes the existing bounded options sequence and point coordinates, then resolves the same page handle again before native add_field. Page organization increments the owner's existing generation, so a callback cannot retarget the outer operation to another page at the old ordinal. Native staging/name/kind/field limits and normal cache/generation updates remain unchanged. Copying a document during normalization leaves the original target live.


## 78. Owned auxiliary paragraph deletion

A private typed paragraph collection adds __delitem__ while reusing existing factories/handles. It checks owner, snapshots IDs, converts the integer index, checks owner again, normalizes the index and calls private native remove_paragraph by stable ID. An index callback may shift earlier siblings without changing selection; clearing/replacing the selected paragraph leaves a stale ID that fails. PPTX and slice deletion fail explicitly.

Native removal accepts only a direct owned header/footer/note Paragraph. Header/footer parts must have one explicit section reference; unexpected paragraph.section properties are rejected. Body/cell deletion is not emulated with blank text. A staged model calls public remove_block; the returned block supplies exact existing content_usage charges. The native mandatory empty/trailing paragraph is retained and tracked as synthetic, with canonical note style if the note becomes empty.

Only adapter positions move: later root path components in the same Part decrease by one, including nested children and cell placeholders. Existing 100,000 location/cell limits and the authored 16-level/49-component path profile bound bookkeeping. Surviving native IDs/styles/numbering/media references and table generations are unchanged. Deleted handle removal, ordered story IDs and placeholder positions are staged before normal commit; publication occurs only on success. Existing range/cache invalidation and media reclamation remain unchanged. Body ID==position assumptions require separate migration before broader support.


## Cumulative owned-field appearance accounting

The existing private appearance work estimate uses 64 KiB plus sixteen times bounded text bytes; creation reserves two generations and filling includes prospective value input. These are conservative appearance work/storage reservations, not total native memory guarantees. Initial native field creation and subsequent font/orientation set_props each retain an appearance generation in the document object store. The adapter reads only the known initial field's bounded AP and charges it separately from the final configured AP. The complete owned profile is validated after configuration, preserving rotated choice-field fit semantics.

Text/choice set_value always emits a new AP, including identical/empty writes; the final known AP raw size is charged with input JSON and the existing mutation counter advances. Complete checkbox values switch existing states and charge no new AP bytes, but still consume a mutation item. Existing geometry/tooltip/read_only setters already charge their new generation. No runtime scan of all objects or imported AP decoding is needed: the single-widget profile and absence of scripts/calculations/duplicate field names establish which appearances were generated. Native field-tree inspection limits are otherwise unchanged. Full-document copies retain cumulative counters; deleting fields does not reclaim historical generation charges.


## PDF field value normalization

The existing value setter checks its handle first, uses str.__len__, str.__str__ and str.encode to validate the actual string with a 4,096-code-point precheck and exact 4,096-byte UTF-8 ceiling, then serializes the normalized scalar. A final _check precedes native dispatch. This mirrors tooltip normalization and prevents overridden encode from deleting/replacing a field before a stale handle writes by the same name. Bool/None dispatch and native staged validation remain unchanged; no native API or generic normalization framework is added.


## 81. Owned DOCX body identity separation

All editable block creators share the existing monotonic registry allocator, now named next_block_id. Body insertion still selects a native physical position from body_placeholder or body length, calls the same public native insertion/model operation, and records that location under an independently allocated ID. The prospective counter is checked before mutation and published with existing bookkeeping only after ordinary commit validation succeeds. Body identities are never cast into native u32 paths.

Body collection enumeration builds a transient bounded position-to-ID BTreeMap from top-level Body locations and follows native block order, with a bounded picture-ID set for paragraph filtering. Duplicate/out-of-bounds locations and unregistered tables in malformed internal state fail explicitly; unregistered structural paragraphs stay hidden. body_placeholder remains a physical position, and paragraph filtering compares resolved story/path rather than numeric identity. The existing active 100,000-location budget remains distinct from the physical body limit and total serialized block count.

Section endpoints, section ordinals, hidden section-marker paragraphs, native layout paths, numbering and field scopes remain untouched. Copy clones allocator/maps; picture removal changes classification without replacing the empty anchor's ID. Part/grid invalidation and normal global TextRange revisions are unchanged. The historical spec78 audit preceded this migration; its body-deletion restriction remains, but the identity prerequisite is now complete. Body section-boundary/semantic deletion policies still require a separate accepted change. Private repr/ID numbers change intentionally and do not promise positions or cross-document identity.


## 85. Native PDF comment opacity

The existing optional update dispatch gains opacity and forwards color/opacity through one pdfcraft_annot::set_style call. The existing lock-target validator is reused for opacity reads/writes; only its CA scalar and the geometric validator expand from 1 to finite [0,1]. A single-target accessor reads validated CA, defaulting absent CA to 1, without props(). Python normalizes numeric inputs between initial/final ownership checks. Notes reserve a fixed 64 KiB appearance bound; generated stream bytes plus a conservative 512-byte ExtGState resource allowance are charged per native style/shape-resize AP generation, including subsequent color-only writes, identical writes and the already measured intermediate shape-resize AP. Canonical note recoloring uses the same bounded target guard and appearance ceiling as note opacity. Native Highlight Multiply and CA/ca resources remain upstream behavior. No dependencies or public manager are added.


## Owned radio groups

A private add_radio_field boundary keeps old add_field callers unchanged. It caps boxes JSON at 16 KiB before deserialization, validates 2–32 tuples/options, native user geometry, canonical encoded names and the existing aggregate text/field/widget/history budgets, then stages native NewField::Radio calls. Public fields.add normalizes the actual string discriminator, options and all coordinates before its final original-page check. No callbacks choose a different native kind.

The owned validator admits only same-page, distinct, visible radio widgets with canonical states, no scripts or Opt mappings, RADIO|NO_TOGGLE_TO_OFF plus optional READ_ONLY, and one value matching a member. Native name_text/name_bytes provide encoding. Field payloads expose ordered boxes/options; radio box mutation is unsupported. The original single-widget profile and font-fit behavior remain. Native set_props retains full-field equality postconditions; radio vector appearances have no text-fit requirement.

Fixed native frame/circle output is bounded by finite ±1e6 coordinates, ≥12-unit dimensions and the pinned three-decimal formatter. The small fixed path has fewer than 128 numeric operands and 2 KiB of operators, below 4 KiB raw per pair; 64 KiB is a conservative stored-pair/work allowance, not a generic compression-memory guarantee. For N members, creation emits N initial pairs plus N(N+1)/2 redraw pairs. Initial pairs are charged conservatively because the public final-field reader cannot reach them; live newly generated pairs are charged by raw stream length. Member work consumes the existing 10,000-item budget. No objects are scanned at runtime or erased locally. Deletion/flatten retains cumulative history; flatten validates native drawn widget count but returns logical fields.


## 84. Owned body paragraph deletion

The existing remove_paragraph transaction selects Body or owned Part and retains the same public collection syntax. Body deletion rejects a section-bearing target before mutation, snapshots bounded native SectionProps, then calls remove_block. Only when its remaining tail is section-bearing does public insert_block append a plain unregistered blank. The invariant helper requires at most100000 physical blocks, a final non-section paragraph, at most128 strictly ordered in-bounds section endpoints ending at the body tail, and identical ordered native section values. No section-property relocation, format reconstruction or semantic-ID remapping occurs.

Existing staged bookkeeping removes the selected ID and shifts later same-story root components; nested suffixes, table generations, part locators and allocator remain unchanged. Existing content_usage reclaims exact charged text, while ordinary commit performs anchor/navigation/media/field/list/style checks. The required blank uses body_placeholder and a later addition receives a fresh ID. Cell/PPTX/slice operations and section-boundary deletion remain unsupported. Earlier spec78/spec81 restrictions are historical; the separately validated ordinary body scope supersedes their current absence without changing those original artifacts.


## 86. Owned paragraph movement

`_Paragraphs.move` captures membership IDs and native revision, normalizes each integer under lifetime/revision checks, then passes two stable IDs and the captured revision. A changed bool invalidates the Python snapshot only for a real move. Native selection rejects section-bearing endpoints and compares the original containing section; comparing SectionProps alone is insufficient because moving after a boundary can enter the next section without changing any section value.

The adapter validates bounded direct IDs, locators and placeholders, then composes native removal and insertion at the original destination root position. Equal IDs return before removal so the upstream mandatory paragraph does not duplicate a singleton. Native table-tail padding is retained once. A root-index permutation updates every same-story location and cell placeholder while keeping nested suffixes, table generations, images, allocator and text/cell charges unchanged. Auxiliary direct paragraph IDs are reordered after validating the entire bounded registry. Body enumeration already follows physical source order. Staged publication uses normal commit, with no semantic exception; moved note/bookmark/comment anchors are outside the first supported profile. Physical block and section invariants remain bounded before/after native operations.


## 87. Native-backed everyday property types

Python annotations describe existing PyO3 return contracts and normalized JSON: displayed boxes are four-float tuples, sizes two-float tuples, colors optional hex strings where native values can be absent, outline destinations optional integers, and PPTX mixed/unset text format fields optional values. Bookmark.children returns the existing typed collection. Existing private source/box/color/size aliases are reused for inputs without adding exported vocabulary. The strict static consumer exercises property/collection/copy chains; runtime AST comparison with annotations removed establishes unchanged execution bodies. No Rust changes are involved.


## 91. Reproducible Office workload measurements

Extend the existing fixture generator with bounded page count, image dimensions, distinct deterministic PNG payloads and displayed size. Native writers create the documents; reopening produces a source manifest. The existing render-session benchmark hashes the input before measurement and accumulates fixed-length page PNG digests in source order while retaining only one PNG at a time. Fresh processes compare the same installed wheel with its retained native session versus that session explicitly closed before rendering. Three alternating-order repetitions provide timing medians and observed memory ranges; JSON retains every sample and source/tool/artifact identity. The input hash warms filesystem cache, and OS peak/current RSS counters do not establish native object reclamation or allocation limits.


## 89. Native title and author fields

The two new kind strings map to existing native `InlineObject::Field` instructions TITLE/AUTHOR with empty results. Unlike page/count fields they have no Arabic numeric switch. WordCraft's public layout context obtains title/author from `Document.core`; its native field evaluator uses those values only when the stored result is empty. Python invalidates its immutable rendered snapshot after metadata mutation, so subsequent preview/export uses a fresh model/layout. No general cache refresh operation is added.

The existing field traversal counts every field and conservatively includes the referenced metadata bytes in paragraph/document text budgets before commit. Repeated fields consume repeated display bytes; metadata changes recheck the same contract. The pagination preflight reuses its native layout for metadata-field line-box checks, with bounded item/line/cluster scans and public bidi-aware cluster edges. Native atomic fields remain indivisible; unsupported overflow, turned text and explicitly clipped cells fail rather than being reshaped locally. This checks native advance and line boxes, not glyph outlines. Existing save atomicity protects destination paths.


## 90. Guarded native DOCX picture placement

Python normalizes only the bounded existing format vocabulary, captures the native revision before numeric conversions, and rechecks ownership/revision afterwards. Rust parses at most1KiB of JSON, validates a copied native Float, clears conflicting alignment values, and mutates the original native Image in a cloned document. Native size/media/alt/crop/flips remain unchanged. A direct body path is mandatory for a floating wrap. Omitted properties retain values; inline resets irrelevant placement. A0.05-point grid avoids silent native EMU conversion changes and provides canonical snapshot tuples without promising arbitrary floating-point identity.

The existing unsplittable-page preflight collects floating body picture paths under the body-block bound. Its early-return trigger includes those paths; the one native layout pass checks actual Placed::Image frames under existing page/item budgets. The same validator runs before committing placement and at serialization, so later source or page edits cannot silently clip a floating picture. This is native-result boundary checking, not adapter pagination. The native writer's fixed overlap/layoutInCell and its source-order front paint defect constrain the exposed profile; no reordering or raster workaround is used.


## 92. Cross-format render-session evidence

A shared development runner extends the existing Office harness to PDF and retains the old CLI as a small compatibility entry point. Named initial/final counters distinguish PdfCraft PageRenderer from native Office sessions; the historical Office tuple remains available. Optional fixture PDF export uses existing native conversion and verifies reopened page counts. Five PDF inputs, two modes and three alternating repetitions yield30 fresh-process samples. The retained renderer's eager page-text requests precede rasterization; one-shot mode closes that session before every subsequent public render uses the existing fallback. Historical Office JSON/tool identities remain immutable. No new runtime cache, renderer, dispatcher or public abstraction is introduced.


## 94. Combined native DOCX cell layout acceptance

A new public test module generates six 3×3 tables through existing methods. Rows vary vertical alignment, columns vary paragraph alignment, and separate fixtures vary orientation plus table/paragraph RTL. Native XML establishes written properties; semantic extraction establishes source order/content identity. Distinct cell fills locate actual native frames, and a separate red-text mask locates glyph ink. Frame/ink comparisons use the declared border width and a two-pixel native/PDF bound tolerance; no table standards or pagination algorithm is implemented in Python. Absolute native row steps and solid-fill heights are checked from the uniform fixture's known exact-height/bottom-padding/border inputs, in addition to source/PDF parity. Additional controls separate paragraph direction from table RTL and prove empty-cell formatting, copy independence and inheritance reset. Inter-cell spacing remains an upstream prerequisite rather than a local margin workaround.


## Owned DOCX cell paragraph deletion

The existing remove_paragraph transaction derives the parent-prefix/depth from the registered native Path, obtains its block list through Document.container and calls public remove_block. The same bounded locator registry now shifts only the coordinate inside that parent, preserving suffixes for following nested tables and their media/paragraph descendants. A merged covered coordinate already resolves through cell_anchor to the same native cell and paragraph IDs. Grid structure/generations are unchanged.

Native removal owns required blank creation. Only a newly appended cell blank enters the existing cell_placeholders set; preexisting authored empty paragraphs remain visible. Existing cell append consumes the hidden blank. Cell deletion does not alter direct story children or body/story sentinel indexes. Ordinary semantic-anchor, incoming-link, independent-header/footer ownership, ordered body-section and resource/revision guards remain part of the staged transaction. This extends the existing collection operation without adding movement, imported editing or reference remapping.


### 96. Native slide-copy boundary

Pinned DeckCraft derives Clone for Slide/Shape and exposes fallible ID allocation, but slide duplication, shape/connector re-ID and animation remapping live in private editor code. Basic model validation checks shape IDs per slide; raw clone with a fresh slide ID therefore does not establish safe owned adoption. Request a pure bounded native same-deck copy operation, with explicit self-link semantics, before designing one Slide.copy operation composing existing slide movement. Cross-deck import remains separate. Keep media/layout references shared within the owner while new shape identities isolate table/cell generation bookkeeping; prove these properties with native fixtures before implementing any facade.


## Native-backed snapshot output types

The five existing getters return output-only private TypedDict shapes derived from author_docx metadata/table/cell/picture serializers and author_pdf metadata_keys/info. Word metadata inherits optional core keys and adds a required custom mapping without postponed Required/NotRequired key-introspection ambiguity. Border snapshots share a required style/color/width shape; table look reuses the existing conservative six-boolean patch type. Input patches retain their distinct optional-key/reset/color contracts. The cell margins annotation additionally records the existing list input; table/cell border annotations accept either the original partial-patch dictionaries or exact output-border dictionaries, retaining dict-only runtime validation. This handles mutable TypedDict/dict variance without widening to unsupported Mapping objects or changing runtime validators.

Polymorphic formatting is deliberately separate: the original snapshot audit observed alignment-only PPTX cell paragraphs (spec121 later adds six direct spacing/indentation values), paragraph styles include both character and paragraph properties, character styles omit paragraph keys, and table styles add regions while even their resolved native snapshots retain nullable direct values. Annotating all of these as one nonnullable resolved mapping would misstate successful native results. This slice adds neither a new snapshot manager nor a general JSON schema layer.


## 97. Cell-local extension of paragraph movement

Generalize the existing transaction from root depth0 to an equal native parent prefix, obtained through Document.container. Public remove_block/insert_block move the intact Block at its final native destination coordinate. A bounded permutation changes only the selected parent's child coordinate in adapter locators and cell-placeholder paths; nested suffixes and unrelated roots/stories/cells survive. Direct body/story registries and sentinels remain unchanged for a cell move. Merged aliases already canonicalize through cell_anchor. Track an exact-parent synthetic tail separately from descendant nested-cell tails, reject malformed/multiple sentinels and duplicate descendant locators, and bypass native mutation for validated identical IDs. The normal staged commit and output preflights retain semantic, layout, field and resource checks; no semantic-anchor exemptions are added.


## 99. Theme metadata versus native property identity

Pinned WordCraft Settings stores theme metadata, but ordinary CharProps values and TextColor lack theme slot/tint/shade/font-reference identity. The reader resolves references to concrete values and the writer emits concrete property values. Native style and paragraph layout resolve those values independently of theme Settings; rendering/PDF consume the result. Graphic-text minor-font fallback is a distinct limited path. Private editor theme commands rewrite selected styles heuristically and are not an allowed substitute. Defer facade design until public native model/read/write/resolution support preserves this distinction; reuse existing formatting operations where fixed formatting is intended.


## 101. Native PPTX cell paragraph removal

Remove one public `TextBody.paragraphs` value inside the existing staged presentation mutation. Native text-range deletion joins paragraphs and retains the first paragraph's properties; it is unsuitable for deleting an intact first paragraph. The public model vector preserves all survivor values without reproducing a document standard. Existing content validation traverses the same shape graph before publication that refresh_budgets uses afterward.

Validate the existing canonical CellTarget (row, column, paragraph, grid generation, body generation), preflight the next body generation, then publish native model/revision and update that cell's generation and accounting. Existing table and unrelated cell identities are unchanged. Owned zero-paragraph bodies already exist after splitting covered cells; use that same state after deleting the final paragraph. Native serialization alone emits its required empty paragraph, and existing append handles initialization. No new sentinel or identity allocator is needed. The Python collection dispatches to the native PPTX operation and scopes callback revision checks to PPTX, preserving DOCX captured stable-ID behavior.


## 98. Native comments with one exact-candidate proof

Public WordCraft Comment, comment map, PartKind::Comment, try_add_part, insert_object/delete_range and set_story provide owned CRUD. The facade adds one Comments collection and Comment handle over existing TextRange navigation; reply creation reuses add with a root handle. Each mutation stages the native document, next-comment counter and existing private paragraph locator/block accounting. IDs never reuse; the bounded monotonic ID supplies the native durable_id so deleting an earlier comment cannot renumber remaining durable identities. The native part allocator may reuse storage safely because private locators are removed exactly and handles use comment IDs.

A bounded CommentInventory proves the exact staged candidate's unique parts, one plain paragraph per part, canonical metadata, direct-parent relation, root pairs and actual endpoint byte offsets. It scans body/all native parts with scope classification and cell ancestry, rejecting orphan markers/parts, unsupported stories, extra inline objects and malformed registry/sentinel state. The same inventory is passed to navigation and media admission, and output reuses its own exact-candidate proof in the existing geometry pass. There is no generic allow-comments flag and no silent cleanup. A private removal permission identifies only the prevalidated root/pair/map/part (or a markerless leaf reply); all other semantic guards remain intact.

The scan precharges two part/map passes plus blocks, private runs, rows, cells and objects before their bounded scans/allocations. Canonical author/date/durable/initials/date_utc fields bound individual metadata serialization before aggregate accounting. Source text is bounded separately. Root marker bytes and private text count in existing owner text/block budgets; copied allocators and revision counters preserve exhaustion/rollback behavior. Private comment paragraph locations never enter public story lists.

Raw proof shows interior native markers alter ligatures, Arabic and combining-text ink/clusters, and overlapping IDs confuse PNG shading. Roots therefore occupy actual paragraph endpoints, not endpoints calculated after discarding controls. Whole-paragraph multiline/automatically split body and ordinary/nested/merged body-cell controls preserve corresponding-stage unshaded ink and default-PDF output at 72/144 DPI. Cell rotation/exact-height ancestry stays excluded. Native nested-table read/write itself changes one unmarked 144-DPI control, so the evidence compares marked/unmarked at corresponding stages and does not claim universal round-trip pixel parity. Resolved state is stored separately from native PNG display; no local shaping, XML repair or layout algorithm is introduced.


## 100. Native DOCX source and line observations

A private provenance collector observes each native Body paragraph's explicit source-break intervals once, then walks existing page.items in source layout order. Public ParaLayout.x_spans supplies bidi-aware logical selection extents; native turn_rect maps them to page points. It does not call selection_rects per paragraph because that API traverses all story paths on each call. Source ranges stay in native Paragraph.text UTF-8 bytes, including object placeholders, and never reuse normalized link character offsets. Existing source_path/segment/story keys extend to paragraph/table wrappers and assets. Page metadata carries native body_bbox, section and display_number.

Per-segment layout metadata contains ordered page/source_range/rects line records. No single bbox is fabricated for disjoint or multi-page text; missing layout means the supported projection is unavailable. Source-only story paths remain available. Native clipping, list labels, drop caps and floating objects are not reinterpreted into ink geometry. Whole automatically split normalized paragraphs still occur once on the first assigned page.

Counters bound line observations, rectangles, conservative native cluster work and projected metadata before allocation. Native ks/boxes/out and stable-sort scratch are precharged separately as a simultaneous temporary peak. Finite xs/vis/line/frame values and UTF-8/cluster bounds are checked before helpers can filter invalid values. Collector map/vector/key storage is reserved before construction. Each layout attachment separately reserves two complete output representations before cloning, so extra attachments cannot bypass accounting; each source metadata emission similarly reserves page/block copies using Value-sized path elements and conservative container allowances. Deferred source-budget exhaustion prevents further metadata allocations and makes the whole extraction fail at finish, following the existing bounded-normalization pattern. No dependency, runtime Python method or immutable-container promise is added.


### Spec100 correction: finite signed cluster advances

Pinned WordCraft adds character spacing to shaped advances and preserves signed adjacent-position differences in bidi VisCl widths. Negative widths are valid for condensed clusters. Provenance validates finite x, width and x+width before calling native helpers, without imposing nonnegative advances or taking absolute values. Native x_spans filters nonpositive bidi cluster widths; the projection retains its existing omission of nonpositive returned spans. Existing source ranges, line/cluster cardinalities, nonnegative line height, output geometry and all work/copy/allocation budgets remain unchanged.


## 103. Font storage is separate from document resolution

Pinned WordCraft exposes embedded-part extraction and FontDb admission but not a typed DOCX embedding operation found by this audit. DeckCraft already exposes embedded-font/media serialization and supported-wrapper extraction. Neither storage path supplies the document-owned resolver needed by ordinary text/layout, which currently uses globals; admission keeps existing family/style matches and selected FaceRef handles retain faces for process lifetime. Public independent FontDb construction alone cannot inject the context into all document operations. Defer local API design until native context/lifetime, permission, collision and diagnostic behavior are proven. Reuse existing formatting and lifecycle operations after that gate; native PDF subsetting already exists and must be validated rather than reimplemented.


## 104. Conservative private formatting output schemas

Private TypedDict snapshots mirror existing native serializers. Character fields are required for styles/defaults; Style adds optional paragraph/table fields and retains nullable scalar types even for resolved snapshots, because resolved table inheritance can still be unset. Paragraph direct output requires only alignment and permits Word-only fields; resolved Word paragraph output and defaults retain their required fields. Tabs have required position/alignment/leader keys; table regions have direct character and region fields without recursion or paragraph-only properties. Reuse existing border output values. Do not change input TypedDicts, native serializers or runtime-kind hierarchy. Returned dictionaries remain detached mutable values.


## 102. Native DOCX table removal with stable identities

A private specialization of the existing table collection delegates captured stable IDs to the native owned remove_table transaction; no public manager or alias is added. It resolves the registered StoryRef/Path, checks direct/ordinary-cell ownership and independently referenced header/footer parts, then uses public Document.container and remove_block. Parent prefix/depth determines exactly which coordinate shifts; existing invalidate_descendants retires the removed subtree. Ancestor and surviving grid generations are unchanged.

Before allocation-producing content_usage, the original bounded comment/source inventory validates text, objects, parts, depth and physical cells, and registry checks reject missing/duplicate image associations, unrelated invalid table generations, malformed locators and sentinels. This prevents existing global pruning from silently repairing unrelated state. Descendant section-bearing paragraphs are rejected. Every accepted parent has a final non-section Paragraph; after native removal, its length must decrease by one and its final Arc must be identical. Raw native required-blank creation in synthetic sole/table-ended containers is diagnostic evidence, not authorization to adopt malformed owned state.

The staged transaction subtracts checked returned-subtree text/cell usage, shifts only the selected parent coordinate, preserves authored blanks and existing hidden tails, and invokes ordinary commit. Commit builds a fresh proof from the candidate, preserving comment, navigation, field and media guards and reclaiming only unreferenced media. Private comment locations remain private. Capturing collection IDs before operator.index preserves existing DOCX callback semantics; cell source lookup checks stale grid generations before callbacks. Copies and monotonic IDs prevent deleted handles from retargeting later insertions.


## 105. Letterhead belongs to a section story

An ordinary example returns an owned WordDocument with inline PNGs in first headers and text-only even/default variants. A second next-page section receives copied independent parts and replaces only its first logo/text; existing paragraph insertion drives native body reflow. Native headers_footers selects first actual section pages, then displayed-number parity. Native section_side may insert a preceding-section filler before a restarted next-page section to align physical parity when even/odd headers are enabled. Do not compensate with explicit breaks, page-index edits, XML rewrites or artwork compositing. The fixture treats exact filler/header behavior separately from actual body-line growth and semantic preservation.


## 107. One owned field-scope guard

The existing native Path convention distinguishes direct story paragraphs (one coordinate) from cell paragraphs (one plus three per table depth). A private helper rejects field_name=PAGE when path depth exceeds one. add_paragraph_field invokes it before creating an object or changing accounting; page_field_count invokes it during ordinary candidate validation; to_bytes calls the same existing field validation after bounded comment/source inventory and registry checks, before layout/write. Save, render and conversion reuse serialization. No evaluator or second layout pass is added.

This conservative supported profile excludes cell PAGE even where a current first-page value happens to be correct; pagination changes can expose stale context. Candidate validation still permits removal of that unsupported object, and direct note/header/footer paths remain direct. Imported extraction/rendering continues through native readers and is not covered by the owned guard. Existing table count fields retain continuous-section restrictions; metadata fields retain their separate expansion/geometry checks.


## Callback-safe page/slide selection (spec109)

PdfPages reuses its owner's existing page generation and count. Two private helpers obtain that snapshot and normalize/recheck an index. A PDF slice uses normal Python list slicing over bounded page positions, then rechecks before creating handles; no handle captures a newer generation for an older selection. Delete/move dispatch only after the checks. Slides compares its existing ordered native ID snapshot after each conversion; no generation or public collection class is introduced. PDF page-box/rotation operations already invalidate page generation and therefore invalidate selection even when page count stays unchanged. Metadata edits and slide-name/visibility edits can leave selection valid. These checks do not roll back arbitrary user callback code.


## 108. Document-wide mirrored margins reuse section geometry

The Python facade adds only `WordDocument.mirror_margins: bool`, with open-owner validation before exact bool validation. The native setter clones the current Document, assigns `settings.mirror_margins`, explicitly invokes existing `validate_unsplittable_page_bounds`, then uses ordinary staged `commit`. Geometry is not implicitly checked by every commit; this setter follows the existing picture-placement preflight pattern. No native dependency, coordinate algorithm or API manager is added. Ordinary commit retains anchor, reference, metadata, field, resource and revision guards, including spec107 table PAGE rejection.

Pinned WordCraft Settings/read/write preserve the boolean. Native layout translates even displayed-page body/header/footer items and the body frame; native anchor resolution keeps page-relative artwork fixed. Margin values keep their established order: left+gutter is inside, right is outside when mirroring. A separate native NextPage branch inserts restarted physical-sheet parity fillers only with even/odd headers; mirroring alone does not activate that branch. Model part IDs are snapshot-local and may be reassigned by readback; tests compare each reopened model's own references and exact text instead of inventing durable numeric identity.

Same-value assignment follows existing document-setting mutation semantics rather than adding a special no-op contract. Failures leave private accounting, registries, native bytes, revision and Python cache untouched. Equal columns and single-column labels remain separate profiles; labels that exceed the outside margin fail before this setting commits. Existing output checks guard later edits. The feature does not expose imported editing, binding/print imposition, top gutters or unverified broader layout settings.


## 106. Intact table movement without a second block API

`_Tables.move` reuses the paragraph collection's selection/revision pattern and calls native `move_table(id, target, revision)`. A narrow private `validate_table_operation` shares spec102's original bounded inventory, ownership, global associations, ordinary tail and subtree section checks with deletion; deletion retains its prior mutation and reclamation behavior. No generic block registry or manager is introduced.

Movement rejects different parents/sections and source semantic anchors before no-op. It removes and reinserts the intact native block at the original destination coordinate, verifies unchanged parent length/final paragraph Arc and section metadata, and rotates only the selected parent coordinate of every locator and hidden blank. Descendant suffixes and every table generation remain intact; auxiliary direct-paragraph IDs are sorted using validated resulting locations. No pruning or new handle allocation occurs. Normal staged commit builds fresh candidate comment proof for retained references, media/navigation and output validation.

The raw gate covers 336 structural and 24 colored-output profiles. A requested PAGE-positive control exposed the native stale table field context; spec107 now rejects owned table PAGE. Revised106 retains that native failure as an explicit diagnostic, pairs it with exact owned rejection, and uses NUMPAGES/SECTIONPAGES for supported movement output. It neither evaluates fields locally nor broadens imported editing. Final table ordinal semantics preserve intervening paragraph order, not reversibility of every interleaved block position.


## 113. Release compiler selection

Set the existing pinned maturin action input `rust-toolchain: 1.95.0` on both the Linux container build and the macOS/Windows build. The pinned action resolves this input before toolchain-file/default discovery, installs/selects it inside the Linux build container, and selects it through rustup on host builds. No runtime dependency, API, target, source engine, version or publish condition changes.

## 111. One section property with native alignment and bounded admission

The Python Section property adds one literal-valued entry to existing `_props`/`_set`; exact strings avoid coercion callbacks. Native section props map directly to public PageVAlign and its native DOCX reader/writer. The private alignment module provides strict parsing, an exact-candidate source-profile check and vertical output geometry validation; it does not lay out text, distribute gaps or move coordinates.

Central commit calls the profile after the existing bounded comment/source inventory. Output calls the same profile from the existing geometry preflight, and non-top alignment activates its existing layout pass. A checked two-million-item scan precharges section/body/part/block/row/cell/object/group-child inspection and reference-stack allocation. The section/body/depth limits remain in force. All-top skips the new composition restrictions. Native inactive even-story placeholders remain valid because actual selection is gated by `Settings.even_odd_headers`; activating an even story fails atomically while any non-top section remains.

Document-wide restrictions avoid predicting shared physical-page boundaries: all sections must be horizontal single-column NextPage; rotated cells, notes, active even/odd selection and floats are excluded. Public raw proof demonstrates columns exceeding the body, page-anchored art moving, unequal even-header overlap and note-driven source/readback spacing drift. Default/first stories, normal tables, inline art and backgrounds stay native. Background painting occurs outside shifted layout items and receives executed PNG/PDF/readback controls. The output check rejects native rotated line origins rather than interpreting them as horizontal coordinates, then validates finite page/body vertical bounds and body line/cell extents without imposing a nonnegative horizontal-advance requirement. Typography may commit and later fail serialization/rendering; shortening/resetting it or restoring Top can recover.

Literal rectangle comparison in the raw note-free roundtrip permits less than 0.0001pt for native width serialization 237.20001→237.2, while observed body/header/footer line positions are exact and PNG/PDF color bounds retain the 2px tolerance. This does not excuse the 1–2pt note-dependent body drift; notes stay outside non-top admission. U02 owns those confirmed layout defects, and LAY10 remains partial.


## 110. Unused styles without retargeting or handle resurrection

`Styles.__delitem__` resolves one current canonical ID; integer conversion retains an owner revision snapshot. The native adapter checks exact identity, protected flags/native builtin IDs and bounded live/retired storage, then performs a read-only typed reference walk. It scans raw fields rather than resolved styles or the incomplete private uses_style/public restyle helpers: defaults, all parts and separator slots, paragraphs/marks/runs, table props/row exceptions/nested cells, graph edges, named and raw conditional character properties, abstract levels and level overrides. Native PartFormat has character properties plus alignment/fill/borders, with no conditional ParaProps carrier. Target-internal edges may disappear with the target, but its profile/work is still checked. Ruby, controls and their opaque rPr/end-property XML, arbitrary fields, OLE/graphics, passthrough, template updates and tracked histories are rejected; the five authored simple fields and recognized native note separators remain available.

A 2,000,000-unit checked budget charges visited records and reference bytes, with 100,000 aggregate block/cell and 16-level nesting bounds. After preflight the adapter calls public StyleSheet.remove on a staged copy. With no incoming references its native rebasing/relationship cleanup has no other target. Ordinary commit guards remain active, including the table-PAGE restriction. A prepared retired-ID set is assigned only after successful commit.

The private lifetime registry is copied with the owner and released on close. Native new_id receives a temporary reservation-only stylesheet containing live and retired IDs; it is never serialized or injected into the actual model. Enforce 1024 lifetime IDs including builtins,259-byte IDs, duplicate/disjoint checks and explicit native allocator-fallback collision refusal. Same-name recreation receives a fresh ID. Native handle-facing targets use exact get; intentional collection/string name lookup keeps find. Python checks every Style-reference consumer and target-format/index conversion for revision/identity changes. No public manager, exception type, renamed style, automatic reference rewrite or imported-edit surface is added.


## 115. Native layout value contracts

Annotate the existing facade signatures directly from the DOCX/PPTX PyO3 return contracts. Keep row height as `tuple[float | None, str]`, not a nullable whole tuple; borders are `tuple[str, float] | None`, widths are `list[float]`, and spacing is a three-float tuple. Existing setters keep their bodies and input normalization: column widths accept `Iterable[float]`, while heterogeneous list inputs use conservative element unions and retain runtime positional validation. Already typed paragraph/row setter arguments receive explicit `None` returns. No aliases, exports, methods or native changes are required. Unsupported PPTX row flags and auto/exact height modes remain unsupported. Cross-format range-link types and other property/method annotations remain separate API3 work.


## 114. Reviewed native per-call rendering errors

Use the explicitly identified DeckCraft contribution fork for all ten selected transitive/direct crates at one immutable revision, keeping PdfCraft, WordCraft and craft-libs fixed. The six-file native change retains successful rendering and adds typed per-call render errors plus PDF propagation; the original renderer and PDF algorithms remain native. No application or new runtime dependency is linked.

The adapter replaces only its existing production render call with try_render_slide and maps its error to private EngineError::Rendering. Existing `_native` explicitly maps rendering_failure to DocExError. PDF export retains conversion_failure. DPI/index/size/region/RGBA/PNG metadata checks, retained-session ownership and request counters remain in their original order. Actual nested callbacks reach the production full/clip/session/PNG and PDF paths; no injectable API or private encoder abstraction is added.

Current support citations update only the DeckCraft repository/revision and selected renderer symbols after native-file audit. Historical artifact references, fixture hashes, operation statuses and scope statements remain historical evidence. Package tests exercise reachable Busy; caught Panicked and thread isolation are separately proven by the native contribution tests. No claim covers process aborts, universal allocation limits or other renderer entry points.


## 116. Native PDF shape stroke correction

All nine selected WordCraft crates use one reviewed immutable contribution-fork revision. The native change sets round cap/join values on PDF Draw::Shape to match existing raster stroke defaults; paths, effects, layout, serialization and other drawing operations are unchanged. Existing local imports/rendering/conversion select the corrected library without an adapter, shape facade or geometry algorithm change.

A small committed synthetic DOCX comes from the frozen native-writer fixture, with recipe and hash provenance. Native and Python integration tests exercise actual imported rendering and PDF conversion at72/144/300DPI, positively sample an interior rounded corner and negatively sample the old miter-only corner. Source/readback geometry and raster equality, file/bytes/retained/one-shot paths and PDF reopen are explicit. Earlier native/raw coverage masks retain broader sampled evidence and actual excluded failures. This dependency upgrade does not admit shape authoring or effects, interpret imported input as safely editable, or silently impose a new imported-shape profile.

Only current WordCraft repository/revision/audited checkpoint changes in the support ledger. Eleven cited native files are audited; ten are byte-identical and the PDF library is the reviewed correction. Historical fixtures, artifacts, scope and statuses remain unchanged. DeckCraft, PdfCraft and craft-libs pins and all package versions/edges are fixed.


## 112. Closed shapes through the existing owned object model

A private native shapes module supplies an exact native-object predicate, bounded source inventory and shape-ID registry over the existing locations/monotonic allocator. It uses Paragraph::insert_object and Document::insert_block for creation and Paragraph::delete for removal; the latter leaves an empty paragraph and its section intact. No relationship generator, geometry algorithm, new public manager or duplicate box alias is introduced. The Python private Word handle shares Picture placement conversion and native picture_patch/picture_snapshot while retaining exact owner/ID checks; existing Picture error wording stays unchanged.

The native predicate excludes all story/freeform/effect/extra/group/rotation/hidden/connector carriers. Media cleanup and unused-style reference scans reuse that predicate after preserving their existing typed checks. Central candidate commit verifies the source/registry and runs the existing single native pagination preflight when shapes exist. Existing explicit preflight sites defer to that commit in shape-bearing documents, retaining their prior behavior otherwise. The native placed Object frame supplies actual draw placement; dimensions/cardinality/full centered stroke and finite page geometry are verified independently of nominal anchor offsets. No silent native dimension substitution is accepted.

Table/cell descendant invalidation occurs only after source validation or central commit proves all shape paths are direct Body paths of length one, so shapes cannot be pruned as descendants. Public copy validates nonempty shape registries but retains historical diagnostic snapshots when the private registry is empty; destructive paragraph/table operations validate the complete source before pruning, and all candidate/output checks remain unconditional. Body deletion/movement applies the existing permutation to every locator. Malformed alias/nested associations reject before pruning; shape deletion removes membership while keeping its paragraph locator. Nonhorizontal sections reject whenever shapes exist, independently of per-object spin. Staged copies publish identity/model only after successful commit; allocator/revision failures roll back.

The first profile caps outlines at100 points (native read/write ceiling) and at min(width,height); full-range corner/outline fidelity is separately gated by spec116 native evidence. The local adapter neither widens upstream clamps nor compensates for cap/join or thick-ellipse differences.


## 120. Reuse the release matrix for pre-release evidence

One boolean workflow input, `validation_only`, keeps the existing nine wheel jobs as the single implementation. The existing `release_tag` input accepts a full commit SHA only in this mode. Before builds, the Python guard verifies its syntax and checked-out identity and refuses `publish=true`. Both publication jobs explicitly exclude validation-only dispatches; wheel jobs retain read-only repository permissions. Ordinary release dispatches and tag pushes still require `v<project.version>`. No package API, version, dependency or runtime behavior changes.

The repository text checkout policy uses Git text auto-detection with LF output; binary fixtures remain binary. Roadmap fixture reads name UTF-8. The atomic-save test records the actual mode after chmod, retains exact0640 admission on POSIX, and verifies mode preservation on failure and success on every platform. Runtime save code is unchanged. Its single source fingerprint migration records the deliberate portability correction without rewriting historical wheel artifacts.


## 117. Native PDF budget upgrade without another API

The native contribution checks the widened integer product returned by the existing render_size helper immediately before Pixels::zeroed, using the existing normal error channel. Its allocator-observation harness is entirely upstream cfg(test); no hook, global counter or new renderer enters this package's runtime. Existing local preflight still prevents a native request for the same oversized public operation.

The integration changes all11 direct/13 resolved PdfCraft source identities together, retaining the complete normalized dependency lock and vendored backend bytes otherwise. One native integration group validates the exact unique pdfcraft-render lock entry before issuing direct overcap requests, then checks both modes/four rotations/UserUnit2, normal errors, empty output, native request counts, tiny tiles and parser reuse. The old public limit workflows pass on both artifacts and are not duplicated or edited.


## 118. PPTX cell paragraph movement through the existing collection

`_Paragraphs.move` retains its Word path and adds a PPTX branch using the existing factory's canonical `CellTarget` tuples. Both indexes are coerced under captured revision after collection/grid validation. One private native `move_cell_paragraph` validates source/destination targets and same-cell scope, bounded content and native identity/layout consistency. Equality returns `false` before checking exhausted mutation counters; changed movement uses public `TextBody.paragraphs.remove/insert` inside existing staged `mutate`, then publishes the checked body generation. No text-range deletion, layout compensation, identity registry or new public API is needed.

All live text/property/media/shape counts and table-grid generations stay unchanged because the native values are permuted intact. PPTX paragraph handles are ordinal: every captured handle in the changed cell becomes stale; other-cell handles and cell/table identities remain live. Revision-bound ranges follow existing invalidation. No-op retains cached document/render objects, bytes and handles. Existing native table fitting and merged/vertical overflow behavior remains authoritative, rather than being recalculated locally.

Raw proof deliberately separates model/default-property preservation from visible-order fixtures. Default native table styles inject a text color into runs lacking explicit fill, so the first inherited-color visual expectation failed before movement as well. Preserve that diagnostic; neutral-style explicit-run fixtures require positive unmoved ink before moved72/144/200DPI output comparisons. Only200DPI uses the native PDF export grid and a channel<=1 comparison;72/144 prove actual colored order/relationships/media, not general pixel identity.


## 122. Targeted reference inventory maintenance

Update the existing seven family summaries in `reference-api-inventory.json` and regenerate its Markdown; do not add a second inventory. Preserve all 44 family IDs, 198 representative member citations, reference snapshots, coverage/owner joins, upstream candidates, dispositions and support-profile joins. The current source checkpoint is the reviewed spec118 integration. Remaining limitations stay explicit, including arbitrary insertion/adoption, imported Office editing, unsupported picture/comment contexts and broader raster resource bounds.


## 119. Native typed placement without Python API expansion

The selected PdfCraft contribution adds Placement and add_content_with_placement. Existing add_content delegates to Overlay. Private placement returns the exact new ObjRef; typed writing attaches its existing metadata to that reference and finds its current page-local index. Prepending A then B yields B, A, original drawing order. Positions are mutable indexes, not persistent identities. Updates replace the selected exact stream in place; deletion removes that stream while preserving surviving relative order. Callers refresh positional indexes. Raw public stamping remains overlay-only.

The package upgrade changes all11 direct and13 resolved PdfCraft source identities together to516caea956c0aaf15f2bd1f35a15e07c73a9ead6, directly based on60a8d7998b64c7718e08e9054e99f1f2eb1eda8d. Native raster.rs and pixels.rs are byte-identical to that reviewed parent; the allocation regression checks the unique locked render-package source before any oversized request. One additional small linked-native test exercises underlay identity/order, first/last update/delete, legacy overlay, preserved URI/text, real revealed/occluded color and glyph ink at72/144DPI through both save forms. Python source/tests are unchanged.

No adapter-built Form/layer system, resource-name rewrite or new manager is introduced. Opaque originals may hide underlays; native fixed resource names can collide with originals, and generic stamp stream-state/scanner bounds are not solved by typed placement. A later Python slice needs independently bounded transactional admission and stable handle semantics.


## 121. Thin cell paragraph property projection

`_SlideCellParagraph` alone dispatches the existing seven-key format vocabulary. It validates its canonical target before normalization, normalizes one scalar at a time through the existing bounded helper, and rechecks revision/target before the next caller callback. A private native method clones the selected public `ParaProps`, applies a finite allowlisted patch, then uses existing staged `mutate` for content-budget/native-validation/revision checks. There is no generation change, local layout algorithm, new manager or dependency. The snapshot reports direct optional values and rejects native Pts line spacing or Pct before/after values rather than changing units silently. Empty facade patches retain the prior validated no-op behavior.

Native writer/read bounds determine the domains: 4032 pt margins, signed 4032 pt first-line, 1584 pt point spacing and reader-capped 100 multiplier. The initial multiplier minimum 1 is based on DeckCraft's natural=max(ascent+descent,font_size) normal-height formula; lower multipliers intentionally compress/overlap native line advances. Raw public `place_slide` confirms all 48 glyphs survive, and native COS/content APIs confirm the actual text-showing operators/ToUnicode streams are identical to the 1.0 baseline, while normalized PDF extraction loses overlapping repetitions. Preserve this as an extraction limitation; do not rewrite the evaluator or deduplicator. Raw 0/.06/.5/.99 probes are diagnostics, not extra public admission.

Native EMU/point/percent serialization owns rounding. Existing table row fitting, merged/vertical stored heights and finite-page overflow limits remain authoritative. No sampled endpoint or fitting fixture establishes that extreme admitted offsets keep all ink visible. Existing body settings suppress first-before/last-after spacing. Raw thin linked glyphs expose the known low-DPI decoration/resampling discrepancy; unchanged baseline controls and exact 200 DPI comparisons isolate it. Fitting bold underline-off carriers preserve links and use the original 2 px color-band tolerance, with 200 DPI channel<=1 comparisons. Pixel components are not claimed to be native line boxes.


## 123. A placement keyword on typed PDF drawing

The two existing methods accept behind: bool=False. The facade validates actual bool after page lifetime checking and before source/geometry callbacks. Built-in point and box text share the existing native text layout; image insertion retains native PNG/JPEG embedding and image transforms. Their staged writer invokes add_content_with_placement with Underlay or Overlay, followed by the unchanged resource-binding comparison before publishing counters and model. No stream scanning, resource rewriting, generated Form or added-item registry is added locally.

Supplied-font point text remains on its existing raw-stamp entrypoint, which has no placement argument. The facade rejects behind=True before reading a font, and boxed-font native dispatch rejects that combination explicitly. Ordinary callback-driven content edits retain existing semantics; callbacks that close or organize pages still fail the outer operation through the existing generation checks. Underlays A then B paint B, A, original. Opaque originals can hide them while their text remains extractable.

Adding defaultfalse PyO3 parameters requires53 mechanical trailingfalse arguments at existing Rust test callsites. A token audit removes only those arguments and the new test include: all previous argument tokens and all other historical test tokens match. Python historical fixtures and dependencies remain unchanged. Three new native groups and four public workflows cover the actual adapter; the discrete transform matrix does not imply every parameter combination or general pixel-perfect composition.


## 125. Criterion closure without capability promotion

The acceptance audit compares the original ece6c85 wording with the tested source. API7 concerns one support-inventory/error mechanism, QA8 honest publication and sketch/recipe separation, and QA6 the enumerated representative malformed/edit/ownership/nesting/media/save corpus. None requires every operation to be implemented or a universal safety proof. Existing per-operation statuses and unassessed remainder fields remain unchanged apart from explanatory joins for these three gates; API1/QA1 still own completeness.

One evidence document links exact selectors and historical validation. TODO checkboxes remain the only completion source, and the existing matrix generator derives their status. Volatile seven/nineteen profile descriptions are replaced by current generated-inventory links. Existing native/resource constraints and platform/font limitations are not narrowed or removed.


## 128. Preserve native logical break coordinates in PDF text

The reviewed DeckCraft contribution c9c7fff189c808bf7266d0b888c3636a1880080d, directly based on dd4fd70b6347dd2ce782e33f030a2b6c0abb0106, replaces two concatenations of stored Run.text with the existing Paragraph.text() projection. Break runs store empty text but contribute one native VT character to layout offsets. Omitting that character shifted the text slices passed to PDF export. The two corrected sites are normal slide text and notes/handout master text; native model, layout, renderer, fonts, error propagation and serializers are otherwise unchanged.

All four direct and ten resolved DeckCraft source identities move together. No adapter or facade method changes. The original synthetic imported fixture uses public native model/edit/writer calls, explicit run colors/navigation and a generated one-pixel PNG; no third-party content is introduced. Package regressions verify native readback, normalized LF break offsets, searchable PDF labels and links, image/glyph positives and same-grid 200 DPI output. Lower DPI controls establish visible content/order, not general pixel-perfect parity. Native contribution controls separately cover Unicode fallback fonts, master/notes paths and unchanged no-break PDFs/Busy errors.

Current ledger provenance and source citations follow the selected revision; existing profile scopes, statuses, historical artifacts and fixture fingerprints remain unchanged. Cell add_break and a coherent public LF read projection require their own later bounded adapter review.
## 126. Composed representative corpus

Three builder functions live in examples, returning ordinary owned documents for caller-selected close/save/render/copy operations. The resume combines named headings, English/French/German Latin content, language metadata, a shaded semantic table and exact hyperlink; the review combines a decision table and stories with whole-paragraph comment/reply metadata; the PDF intake form combines text/graphics/link and existing owned fields. Existing letter/invoice/branded-report/letterhead fixtures supply the other QA2 categories. Source comment display and deliberately clean PDF output remain distinct. No new builder framework, template language, form manager, font registry or optional runtime is introduced.


## 133. Paragraph formatting overloads

The shared direct/resolved paragraph output declares direction as the exact native ltr/rtl literal (nullable only for direct inheritance). Existing Paragraph.format and its PPTX cell override expose two typing overloads: the unchanged ParagraphFormatting input patch and the full direct snapshot. The latter also accepts the resolved value set; a redundant third overload is unnecessary. The implementation accepts object-valued keyword arguments internally and retains its exact validation/dispatch body. Defaults/style input/output declarations are unchanged. No public helper, manager, method or general mapping adapter is introduced.
### Spec120 explicit font inputs for measured fixtures

Existing formatting selects Source Sans3 for the measured decimal-outline, pagination, bookmark and letterhead fixtures. The metadata-field and behind-image positive carriers use its bold face so exact-color checks measure actual solid glyph interiors even at72DPI. The letterhead recipe selects this font through its existing document defaults. Its font-specific golden has four appendix pages and four expanded main-paragraph layout pages; all physical pages still execute the existing logo/header/footer/content/link/table and72/144/300DPI checks. No layout compensation, renderer change, new font registry, test skip or tolerance widening is introduced. Requested family and emitted PDF BaseFont labels do not prove exact selected program bytes or isolate the native global font registry.


## Dedicated Word default-size policy correction (spec131)

Five direct and nine resolved Word crates select the reviewed 5312722 contribution. Its sole native runtime change removes the reader’s injection of Some(10) when parsed docDefaults omits ordinary size. Effective ordinary size continues to use the existing 11pt resolver fallback; absent complex-script size inherits ordinary size before that fallback, while explicit complex-script values remain independent. Missing ZIP styles parts retain initialized 12pt defaults. The shared Flat OPC and Word2003 converter paths are verified in the native contribution; Word2003 conversion emits a styles part, so its implicit case follows parsed-but-unsized 11pt behavior. None of these values is claimed as a universal standards or Word application default.

The package adds no formatting algorithm, new method, input format or reset admission. Existing default-size None refusal remains atomic with the same exception class and a truthful pending-admission explanation. One native diagnostic deliberately changes from preserving the old mismatch to preserving native None/effective 11 after serialization. One original synthetic imported fixture checks explicit/missing-style controls and source/PDF glyph output through the unchanged public API. Historical contribution and package artifacts remain distinct.
## 130. Native cell break insertion and a one-to-one readable projection

The cell paragraph override validates its owner/target, captures revision and uses builtin str normalization to avoid executing string-subclass hooks. The private native cell_add_break entrypoint revalidates revision and target, rejects unsupported kinds, inventories source content before cloning inherited properties, clears click/hover navigation, and charges one text byte, one run and property cost. Public DeckCraft edit::insert inserts literal VT at the paragraph end; its returned caret and unchanged paragraph count are checked before the existing staged mutation is published. No generation changes accompany this content edit.

Only the cell branch of shape_text/text_range and the cell_text/table_rows projections replace VT with LF. range_text delegates to the same path. This preserves character offsets used by selection/find/replace without changing native content, general shape/notes reads, input validators or extraction normalization. A returned break range can delete or replace the structural marker using ordinary text; generic raw newline insertion remains unavailable. Cell.text conflates paragraph separators and break markers by design and is not a structural round-trip API.

The unchanged original spec124 raw proof is retained byte-for-byte after its separately accepted spec128 native PDF correction. The feature adds three adapter groups and four public workflows, reuses the existing strict typing consumer, and intentionally changes only one historical Unsupported fixture from default line to still-unsupported page. Other negatives and historical support fingerprints remain unchanged. Exact/MAX tests construct actual text, run and property model states; they do not claim huge-document layout acceptance.


## Owned default-size inheritance (spec132)

The dedicated131 pin already preserves native absence. The adapter now removes only the special null refusal in `EditableDocx::format_defaults`; existing `CharPatch` parsing and application set `CharProps.size=None`. The normal document clone, tab/table validation and ordinary commit remain unchanged. No Python/Rust default-size constant or new input type is introduced. Existing `_StyleProperties` already permits None; the current method docstring and typed consumer reflect the admitted operation.

Clearing explicit10pt produces directNone and inherited11pt when no other ordinary size applies. It does not recreate initialized12pt defaults. Explicit paragraph/character/direct and complex-script values remain independent, and source/readback comparisons verify the writer omits ordinary `<w:sz>` rather than materializing a value. Existing same-value setters still advance revision and invalidate cached snapshots; callers retain owned previously returned values and independent copies. The document-wide formatter retains existing ordinary normalization callback edits, while closure causes the outer native call to refuse.

New fixtures identify body/header/cell inherited glyphs and fixed/direct overrides with five separate colors. DOCX headers remain separate extracted stories and are not duplicated in body text; PDF extraction includes their painted text. The native Hyperlink style supplies its own color, so the body carrier uses a color-only direct override while retaining inherited size. These are fixture controls, not changes to source normalization or styling behavior. Existing table checks are retained, but this slice does not add a general clipping detector or assert that arbitrary large indentation must be rejected.
## 135. Unit-helper annotations

The four existing helper parameter annotations become Real | float. This retains custom/native Real subclasses while expressing Python's ordinary int-to-float numeric typing relationship. SupportsFloat would falsely admit objects rejected by the actual Real guard, so it is not used. Runtime boolean/nonfinite/Decimal/float-only-object checks remain unchanged.
