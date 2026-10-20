# Formats, rendering, and conversion

This document describes the current source tree, including unreleased improvements since `pydoccraft` 0.2.1. See the [validation record](../.kiro/specs/rust-document-engine/validation.md) for tested artifacts.

## Capability matrix

| Capability | PDF | DOCX | PPTX |
|---|---:|---:|---:|
| Structured extraction | Yes | Yes | Yes |
| Page/slide boundaries | Source pages | WordCraft layout pages | One page per slide |
| Text and geometry | Yes | Text; layout page metadata | Text and shape geometry |
| URI links | Yes | Yes | Yes |
| Semantic tables | Conservative geometry inference | Yes | Yes |
| Embedded image bytes | Exportable images | Referenced images | Referenced pictures |
| Notes | No dedicated model | Comments as comment blocks | Speaker notes |
| Comments | PDF comments not normalized | Comments and replies | Comments and replies |
| PNG rendering | Yes | Yes | Yes |
| Password input | Yes | Yes | Encrypted PPTX unsupported |
| OCR | No | Not applicable | Not applicable |
| Create a new document from scratch | No | No | No |

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

PdfCraft exposes link destinations, rectangles and text geometry. The adapter recovers unambiguous labels with normalized character offsets, including repeated labels, and Markdown attaches those targets inline. Links inside inferred PDF table cells currently retain empty labels to avoid duplicating cell text. Ambiguous geometry-only links retain empty labels and appear in the final **Links** section; successful multiline-label recovery remains tracked in E4.

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
- Exact hyperlink labels and character offsets.
- Referenced inline and grouped images with bytes, display dimensions, and alt text.
- Semantic tables, including content from nested table cells.
- Anchored comments and replies with author, initials, date, resolved state, and parent identifiers.
- Explicit manual page breaks and inherited `page_break_before` behavior.
- Core document metadata where available.

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
- Run-level URL hyperlinks with exact labels and offsets.
- Shape-level URL links.
- Semantic tables and merge metadata.
- Referenced picture bytes, MIME type, alt text, intrinsic dimensions when detectable, and slide placement.
- Speaker notes.
- Comments and nested replies.
- Slide dimensions, name, and hidden state.

Only URL hyperlink actions are normalized. Internal navigation, media actions, custom shows, and other PowerPoint actions remain upstream model details and are not exposed as Python `Link` values.

## Rendering

```python
png = document.render_page(0, dpi=144)
png = document[0].get_pixmap(dpi=144)
```

Both methods return encoded PNG bytes.

- PDF rendering uses PdfCraft pixels and rejects oversized output.
- DOCX rendering uses WordCraft layout and its page renderer.
- PPTX rendering uses DeckCraft slide rendering.
- `dpi / 72` is used as the native scale.
- DPI must be finite and between 1 and 1200.
- Out-of-range pages are rejected in Python before native dispatch.

DPI controls raster scale and embedded PNG resolution metadata for all three formats. Pixel dimensions follow each engine's rounding; physical resolution uses integer pixels per meter. The multi-DPI corpus checks dimensions, metadata, decoded PNG data and visible content. PDF rotation, crop offsets and UserUnit are covered by the geometry corpus; broader cross-format fidelity remains in the [rendering validation TODOs](../TODO.md#p0--rendering-at-a-requested-dpi).

Each output is bounded to 64 million RGBA pixels in addition to upstream maximum-side limits. DOCX/PPTX dimensions are checked before rasterization; PDF uses upstream oversize rejection. DeckCraft can internally replace a rendering panic with a blank image: this remains a documented upstream blocker, because an intentionally blank slide is also valid output.

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

Every other pair—including identity conversions, PDF→PPTX, DOCX→PPTX, and PPTX→DOCX—is rejected explicitly.

```python
import doc_ex

word_bytes = doc_ex.convert("source.pdf", "docx")
pdf_bytes = doc_ex.convert("source.docx", "pdf")
slide_pdf = doc_ex.convert("slides.pptx", "pdf")
```

Conversion is not pixel-perfect. Font substitution, unsupported PDF constructs, Office layout differences, grouped graphics, complex tables, and upstream exporter limitations can change appearance. Always reopen and validate converted output for the intended workflow.

## Document creation

The conversion methods produce new files from existing documents. There is currently no public builder for creating a DOCX or PDF from scratch, nor a mutation/save API for writing changes to extracted Python objects back to a document. [Creation TODOs](../TODO.md#p1--create-docx-and-pdf-documents) cover upstream-backed authoring of modern DOCX and PDF; legacy `.doc` output is outside the current scope.
