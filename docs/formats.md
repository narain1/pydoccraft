# Formats, rendering, and conversion

This development guide describes the 0.3.1 baseline and explicitly marked unreleased work. EPUB support is unreleased. See the [validation record](../.kiro/specs/rust-document-engine/validation.md) for tested artifacts.

The [operation evidence matrix](support-matrix.md) separates reading, creation, authored/imported editing, preservation, rendering and export for explicitly audited profiles. It links all roadmap criteria and records historical artifact evidence without claiming complete parity.

## Capability matrix

| Capability | PDF | DOCX | PPTX | EPUB |
|---|---:|---:|---:|---:|
| Structured extraction | Yes | Yes | Yes | EPUB 2 and 3 semantic profile |
| Page/slide boundaries | Source pages | WordCraft layout pages | One page per slide | One page per spine chapter |
| Text and geometry | Yes | Text; layout page metadata | Text and shape geometry | Semantic text; no physical geometry |
| URI links | Yes | Yes | Yes | Yes |
| Semantic tables | Conservative geometry inference | Yes | Yes | Yes, including nested tables and row groups |
| Embedded image bytes | Exportable images | Referenced images | Referenced pictures | Referenced images |
| Notes | No dedicated model | Owned footnote/endnote stories and body references | Speaker notes | No |
| Comments | PDF comments not normalized | Comments and replies | Comments and replies | No |
| PNG rendering | Yes | Yes | Yes | Reflowable and fixed-layout chapter preview |
| Password input | Yes | Yes | Encrypted PPTX unsupported | DRM/password unsupported |
| OCR | No | Not applicable | Not applicable | Not applicable |
| Create a new document from scratch | Authored profile | Authored profile | Authored profile | Append-only profile |

See [native authoring](authoring.md) for the supported creation/editing profiles; these entries do not imply arbitrary imported-document preservation.

## PDF

PDF support is backed by pinned PdfCraft 0.9.0 crates.

### Extracted data

- Pages in source order.
- Reading-order text represented as line-oriented paragraph blocks.
- Text and image bounding boxes in displayed page coordinates.
- Page width, height, rotation, and label.
- Document title, author, subject, keywords, creator, producer, PDF version, encryption state, and tagged state when available.
- URI link destinations and rectangles.
- Non-form images that PdfCraft can export as PNG or JPEG, including width and height.
- Conservative table candidates derived from public positioned reading blocks.

### PDF table detection

PdfCraft 0.9.0 does not expose its internal table detector as a public Rust API. `pydoccraft` therefore uses a small geometry normalizer. It reports a table only when it finds at least two rows and two columns with consistent row alignment, equal column counts, and stable column positions. Text blocks represented by the table are removed from normal paragraph output to avoid duplication.

This deliberately favors false negatives over fabricated tables. Borderless, irregular, spanning, rotated, or visually complex tables may remain plain text.

### PDF links

PdfCraft exposes link destinations, rectangles and text geometry. The adapter recovers unambiguous labels with normalized Unicode character offsets, including repeated labels and contiguous multiline labels, and Markdown attaches those targets inline. Accepted multiline annotations coalesce their consecutive paragraph blocks while retaining native spaces/newlines and unchanged plain text. Neighboring links retain correct offsets. Links inside inferred PDF table cells retain empty labels to avoid duplicating cell text. Overlapping, noncontiguous or ambiguous geometry-only links also retain empty labels and appear in the final **Links** section; the adapter does not infer a reading order across columns or guess omitted glyphs.

### PDF images

Images nested in grouped Form XObjects or encoded in formats the upstream image exporter cannot materialize may be omitted. Extraction is not an inventory of every PDF stream. Rendering remains available even when individual image extraction is unavailable.

### Scanned PDFs

A scan-only page can be rendered but produces no text unless the PDF already contains a text layer. `pydoccraft` contains no OCR engine or model.

## DOCX

DOCX support is backed by pinned WordCraft 0.8.0 crates.

### Extracted data

- Ordered body paragraphs and tables.
- Heading levels resolved through style inheritance and clamped to Markdown levels 1–6.
- Style ID, style name, and outline metadata.
- Resolved numbering definitions and unambiguous native layout labels, separate from paragraph text. See [numbering extraction](numbering-extraction.md).
- Exact hyperlink labels and character offsets.
- Referenced inline and grouped images with bytes, display dimensions, and alt text.
- Semantic tables, including content from nested table cells.
- Anchored comments and replies with author, initials, date, resolved state, and parent identifiers.
- Explicit manual page breaks and inherited `page_break_before` behavior.
- Core document metadata where available, plus bounded supported scalar custom properties.
- Table title/description in `Table.metadata`, separate from visible cell text.

### DOCX properties and field values

`document.metadata["custom"]` maps original property names to strings, booleans, signed 64-bit integers or finite floats for the supported native `lpwstr`, `bool`, `i8` and `r8` kinds. Parsing follows the same strict scalar profile as authored properties; other numeric aliases and structured/raw variants are not coerced. Unsupported kinds, invalid scalar values and ambiguous ASCII-case-insensitive names are omitted with bounded extraction diagnostics. All entries in an ambiguous name group are omitted.

The parsed inventory is capped at 1,024 properties, 64 KiB per value and 256 KiB aggregate raw metadata, including names/kinds. This is a native-model inventory, not a promise of complete source XML preservation: the upstream reader can drop malformed custom parts, omit entries and cap source property traversal before projection. Table alternative text is likewise capped by the upstream reader at 10,000 Unicode characters. Extracted `title`/`description` preserve native absent versus empty values; neither becomes a visible cell caption or a PDF accessibility tag.

Source field text uses stored cached values. Newly authored decimal page/count fields have an empty dirty cache, while PNG/PDF output is evaluated by native layout. General field refresh and convergence remain unsupported.

### DOCX pagination

WordCraft computes layout pages. A paragraph that spans multiple computed pages but contains no explicit page break is emitted once on the first page it occupies because the upstream public model does not expose exact normalized text slices for each laid-out page.

Tables are associated with their first page rather than split into separate normalized table fragments. `Table.rows` retains newline/tab cell text for compatibility; `Table.cells` additionally preserves ordered paragraph/table blocks and horizontal/vertical merge information. Markdown uses HTML for nested, merged, or rich tables. This extraction structure does not provide an editable document model.

Unreferenced files in `word/media/` are not returned.

## PPTX

PPTX support is backed by pinned DeckCraft 0.8.0 crates.

### Extracted data

- One normalized page per slide in presentation order.
- Recursive traversal of grouped shapes with transformed geometry.
- Paragraph and soft-line-break boundaries.
- Run-level URL and resolved internal-slide hyperlinks with exact labels and offsets, including table cells.
- Shape/group URL and resolved internal-slide links.
- Semantic tables and merge metadata.
- Referenced picture bytes, MIME type, alt text, intrinsic dimensions when detectable, and slide placement.
- Speaker notes.
- Comments and nested replies.
- Slide dimensions, name, and hidden state.

Resolved native slide targets use `#page=N` and zero-based `target_page` metadata, including self-links and hidden slides in presentation order. Context-dependent next/previous/first/last/last-viewed actions, end-show, media, custom-show and program actions remain explicit omission diagnostics; they are not treated as fixed page targets. Notes retain their existing text-only link behavior. Unknown action strings and unresolved source relationships can be discarded by the pinned reader before the adapter sees them, so their absence cannot be diagnosed completely without upstream preservation reports.

## Rendering

An open document reuses its native PDF parser or Office model for repeated rendering. DOCX also reuses the completed pagination layout. The public methods and PNG-byte return type are unchanged. Use a context manager or `close()` to release retained models/layouts and private source/password references; already extracted values remain usable. Retention can increase steady-state memory. Native global/thread-local caches and allocator retention are not released by this contract, and calls do not promise parallel rasterization. See [Office measurements and reproduction](office-performance.md).

```python
png = document.render_page(0, dpi=144)
png = document[0].get_pixmap(dpi=144)
detail = document.render_page(0, dpi=300, clip=(36, 36, 144, 72))
```

Both methods return encoded PNG bytes.

- PDF rendering uses PdfCraft pixels and rejects oversized output.
- DOCX rendering uses WordCraft layout and its page renderer.
- PPTX rendering uses DeckCraft slide rendering.
- EPUB renders a fixed chapter in its declared viewport and a reflowable chapter on a scrolling surface 612 points wide.
- `dpi / 72` is used as the native scale.
- DPI must be finite and between 1 and 1200.
- Out-of-range pages are rejected in Python before native dispatch.

DPI controls raster scale and embedded PNG resolution metadata for all three formats. Pixel dimensions follow each engine's rounding; physical resolution uses integer pixels per meter. The multi-DPI corpus checks dimensions, metadata, decoded PNG data and visible content. PDF rotation, crop offsets and UserUnit are covered by the geometry corpus; broader cross-format fidelity remains in the [rendering validation TODOs](../TODO.md#p0--rendering-at-a-requested-dpi).

Each output is bounded to 64 million RGBA pixels in addition to upstream maximum-side limits. DOCX/PPTX dimensions are checked before rasterization; PDF uses upstream oversize rejection. DeckCraft can internally replace a rendering panic with a blank slide. The adapter checks its process-wide panic counter and reports a detected panic as an error for rendering and PDF export. A panic on another thread can cause a spurious error; see the [upstream contract](../AGENTS.md).

Optional `clip=(x, y, width, height)` is available on both methods, including editable document/page/slide wrappers. Coordinates use displayed-page points with a top-left origin; PDF rotation, CropBox and UserUnit are already applied. The rectangle is intersected with the displayed page and snapped outward to its native pixel grid at the requested DPI. Empty intersections, nonfinite values and nonpositive dimensions raise `ValueError`. At page edges, each backend retains its full-page rounding convention; fractional boundaries can add an edge pixel.

Clips use native PdfCraft tiles, WordCraft region rendering and DeckCraft offsets, without allocating a full-page raster first. PDF clips are limited to 4,096 pixels per side and exactly representable tile offsets; Office clips retain the 16,000-pixel side and 64-million-pixel limits. Valid but excessive requests raise the existing resource-limit exception. The returned PNG keeps requested DPI metadata and native page/slide background. Arbitrary background replacement and faithful transparent rendering remain unavailable; clipping does not emulate them by recoloring white pixels. Native parsing/layout still processes the document, so clipping is not selective extraction or a complete memory bound.

Region and full-page rasterization can differ slightly at antialiased edges after native transform translation. The measured PDF text fixture differed by at most two pixels and the fractional-DPI PPTX rectangle by 124 pixels on one edge row, each by at most 1/255 in RGB with identical alpha. Those fixtures use explicit tolerances; full-page/boundary and rotated-PDF shape comparisons remain exact. Fractional PPTX slide edges can retain native partial alpha. These measurements do not establish universal pixel identity or fidelity tolerances for arbitrary effects.

The returned value is not a PyMuPDF `Pixmap`; write it directly or load it with an image library:

```python
from pathlib import Path

Path("page.png").write_bytes(document[0].get_pixmap())
```

## Conversion matrix

| Source | Target | Implementation | Notes |
|---|---|---|---|
| PDF | DOCX | PdfCraft extraction values → `pdfcraft-export` DOCX writer | Positioned text styling and exportable images; not semantic PDF-table conversion |
| DOCX | PDF | WordCraft layout and PDF export | Supports Unicode and tested explicit empty pages |
| PPTX | PDF | DeckCraft PDF export | Slides become PDF pages with a searchable text layer where supported |
| EPUB | DOCX | Semantic EPUB blocks → WordCraft DOCX writer | Reflowable content, with each chapter beginning on a new page |
| EPUB | PDF | Semantic EPUB blocks → WordCraft PDF export | Same content profile as EPUB to DOCX, laid out as printable pages |

DeckCraft PDF export corrupts searchable text for PPTX paragraphs containing explicit breaks; native PDF export of those documents is unsupported until the upstream defect is fixed. A visually correct slide is not evidence that its PDF text layer is correct.

Every other pair—including identity conversions, PDF→PPTX, DOCX→PPTX, and PPTX→DOCX—is rejected explicitly.

```python
import doc_ex

word_bytes = doc_ex.convert("source.pdf", "docx")
pdf_bytes = doc_ex.convert("source.docx", "pdf")
slide_pdf = doc_ex.convert("slides.pptx", "pdf")
```

Conversion is not pixel-perfect. Font substitution, unsupported PDF constructs, Office layout differences, grouped graphics, complex tables, and upstream exporter limitations can change appearance. Always reopen and validate converted output for the intended workflow.

## Document creation

`doc_ex.new("docx" | "pdf" | "pptx" | "epub")` creates an owned editable native document with copy and atomic path saving. EPUB creation appends semantic content to reflowable EPUB 3 chapters. Extracted Python values remain independent snapshots. PDF `Document.edit()` uses a conservative preservation profile. See [Authoring](authoring.md) for the supported methods.

## EPUB (unreleased)

Each spine entry maps to one compatibility `Page` in reading order, including repeated and non-linear entries. The page represents a chapter boundary. Extraction preserves semantic text, inline character offsets, nested tables, referenced images, and MathML markup with readable text. Raster image bytes are returned as stored. Static SVG passes a restrictive validation profile and remains inert data.

`Document.metadata["dublin_core"]` contains repeated Dublin Core values keyed by field name. `package_metadata` contains OPF meta records and their refinement targets. `navigation` contains nested navigation groups from an EPUB 3 navigation document or EPUB 2 NCX. `manifest` describes packaged resources. Use `Document.read_epub_resource(path, offset=0, length=None)` with a decoded, relative archive path to retrieve a resource or byte range, limited to 16 MiB per call.

PNG preview applies supported author CSS to XHTML. Stylesheets and imports resolve relative to their archive members. The cascade covers inline declarations, specificity, source order, `!important`, supported type, class, ID and attribute selectors, and basic media queries. Rendered properties include typography, spacing, solid borders, backgrounds, block flow, flex and simple grid, and positioned boxes. Fixed-layout EPUB honors the package default and spine overrides. XHTML uses its numeric viewport meta tag and static SVG uses its viewBox. In page metadata, `viewport` is measured in points, with one CSS pixel equal to 0.75 points. [EPUB 3.3 layout specification](https://www.w3.org/TR/epub-33/#sec-fixed-layouts)

The renderer loads local `@font-face` sources relative to their stylesheet, including imports. It supports TrueType, OpenType, WOFF and WOFF2 with static font weight and style descriptors. The font collection is scoped to each rendered chapter. It supports the standard IDPF font obfuscation declared in `META-INF/encryption.xml`. Resource reads return the original packaged bytes. Archive edits preserve those bytes and the publication identifier. [EPUB 3.3 font obfuscation](https://www.w3.org/TR/epub-33/#sec-font-obfuscation)

Static SVG supports embedded CSS and archive-local CSS linked by an `xml-stylesheet` prolog instruction. Its supported CSS covers paint and typography, including archive-local fonts and same-document paint references. SVG `image` elements can reference packaged PNG or JPEG files by `href` or `xlink:href`. SVG embedded in XHTML or linked through an image renders with transparency. Positioned SVG uses its CSS box and destination resolution. Semantic DOCX and PDF export rasterize static SVG, retaining inline placement and alternative text where the target supports it.

DOCX and PDF export use WordCraft's semantic reflow profile. They support headings, paragraphs, PNG/JPEG images and rasterized static SVG, basic nested or merged tables, and native equations. Fixed-layout exports and source CSS projection to DOCX and PDF remain open. Conversion rejects internal, overlapping and image-only hyperlinks and features that cannot be projected.

The Presentation MathML profile preserves inline and display formulas in previews and exports. Supported features include fractions, roots, accents, movable limits, multiple pre- and postscript pairs, varied fences and separators, simple matrices, phantoms, horizontal spaces, and zero-width, zero-height or zero-depth padding. MathML color, size and variant attributes inherit within supported formulas. DOCX stores native editable equations. Extracted math text uses the primary presentation and keeps annotations in the source markup. CSS rules inside formulas, matrix spans, custom rule thickness, arbitrary padding shifts, and nonzero vertical space remain open.

EPUB 3 media overlays are exposed under `Document.metadata["media_overlays"]`, keyed by manifest ID. They retain nested SMIL sequences, structural roles, target element IDs, and local audio paths. `Document.epub_media_cues(page)` gives a chapter's ordered cues with audio clip bounds in integer microseconds. An absent clip end means the end of the source audio. For playback, `Document.epub_narration(page)` decodes local MPEG Layer III, AAC-LC in MP4 and PCM WAVE to signed 16-bit PCM WAVE and extracts readable text for text-only cues. `Document.play_epub_narration(page, ...)` sequences the caller's blocking audio, speech and active-target callbacks. Both endpoints round up to the next sample timestamp when needed, and the end sample is excluded. Source audio is bounded to 32 MiB and each buffered clip to 16 MiB of PCM. MP4 playback supports one continuous track edit and applies it to cue timing. MP3 uses the MPEG frame timeline, which may include encoder priming and padding. Ogg Opus, fragmented MP4, built-in speech synthesis, and an audio-device implementation remain open. The structure editor can attach or detach chapter associations and generate SMIL with sequential text and audio cues. [EPUB 3.3 media overlay specification](https://www.w3.org/TR/epub-33/#sec-media-overlays)

The reader parses bounded UTF-8 XML and recognizes plain HTML and selected legacy XHTML, NCX and SVG doctype declarations. Parsing is offline. Custom DTDs and script execution are unsupported. Broader CSS coverage, font-face Unicode ranges and variable axis selection, synthetic spreads, and DRM remain outside the supported preview profile. SVG scripts, external and data URLs, referenced SVG images inside SVG, and `use` elements are outside the supported SVG profile. See the [EPUB specification](../.kiro/specs/epub-support/requirements.md) for details.

ZIP safety preflight for DOCX, PPTX and EPUB bounds raw central-directory entries and rejects duplicate names and malformed directories before constructing archive indexes. ZIP64, multi-disk archives, ZIPs with trailing data, and archives with ambiguous end records are explicitly unsupported. EPUB additionally rejects multiple package rootfiles, inconsistent local/central headers and end-record signatures in the ZIP comment. ZIP data descriptors remain supported.
