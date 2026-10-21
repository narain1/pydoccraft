# Python API

`pydoccraft` exposes an eager, PyMuPDF-style API. Opening a document reads the source into memory, invokes the native Rust engine, and returns Python objects that own their extracted text and image bytes.

This page describes the extraction API. The unreleased branch also has [native authoring](authoring.md) through `new`, typed content objects and explicit save/copy operations. The [design crosswalk](authoring-api-design.md) and [feature backlog](../TODO.md) describe broader parity work; unchecked items are not implemented promises.

See the [operation evidence matrix](support-matrix.md) for bounded capability profiles, exact test/native references and remaining audit scope. A supported creation or rendering path does not establish imported-document preservation.

## Open a document

```python
import doc_ex

with doc_ex.open("report.pdf") as document:
    print(document.page_count)
    print(document[0].get_text())
```

```python
doc_ex.open(
    path_or_bytes,
    filetype=None,
    password=None,
) -> Document
```

`path_or_bytes` accepts `str`, `pathlib.Path`, `bytes`, `bytearray`, `memoryview`, or a binary stream. Streams are read from their current position, handle short reads and remain open; text streams are rejected. Input is bounded to 128 MiB, including memoryview byte size. Paths are expanded with `Path.expanduser()` and missing paths raise `FileNotFoundError`.

Recognized explicit `filetype` values are:

| Canonical format | Accepted values |
|---|---|
| PDF | `pdf`, `.pdf` |
| Word | `docx`, `.docx`, `word` |
| PowerPoint | `pptx`, `.pptx`, `powerpoint` |

For paths, the suffix is checked against the detected content. For bytes, the native detector identifies PDF and OOXML content. A mismatched explicit type raises `UnsupportedFormatError`.

Passwords are supported for encrypted PDF and DOCX input. Encrypted PPTX is rejected by the upstream DeckCraft importer.

## Fast text extraction

```python
text = doc_ex.extract_text("report.pdf")
selected = doc_ex.extract_text("report.pdf", pages=[0, 4, -1])
```

`extract_text(source, *, filetype=None, password=None, pages=None)` returns selected pages separated by form-feed (`\f`) characters. Page indexes preserve caller order, allow duplicates, and support normal negative indexing; invalid indexes raise `IndexError`. Selections are consumed only through a 10,001st-item lookahead and reject more than 10,000 entries; selecting all pages has the same cap. Each format enforces a 64 MiB UTF-8 output limit, including separators. PDF preflights the native glyph string before plain-text assembly; Office uses eager extraction. These are selection/output limits, not a streaming or peak-memory guarantee: upstream parsing and glyph construction happen before the output check.

For PDF, this is a native text-only path: it returns PdfCraft's raw reading-order plain text and skips table/image traversal, media transfer, normalized JSON, and public model hydration. Raw PDF whitespace and table separators can therefore differ from `Page.get_text("text")`, which uses the structured normalized model. DOCX and PPTX use the existing eager extraction model as a compatibility fallback. Every format caps explicit/all-page selections at 10,000 entries and UTF-8 output at 64 MiB. Use `open()` when links, tables, images, geometry, metadata, Markdown, or owned structured values are required.

## Convert a document

```python
pdf = doc_ex.convert(
    "report.docx",
    "pdf",
)

with open("report.pdf", "wb") as output:
    output.write(pdf)
```

```python
doc_ex.convert(
    source,
    output_format,
    *,
    filetype=None,
    password=None,
) -> bytes
```

The supported pairs are PDF→DOCX, DOCX→PDF, and PPTX→PDF. Unsupported and identity pairs raise `UnsupportedFormatError`. See [Formats and fidelity](formats.md) for conversion details.

## `Document`

A document supports context management, iteration, integer indexing, and negative indexing:

```python
with doc_ex.open("slides.pptx") as document:
    assert len(document) == document.page_count
    first = document[0]
    last = document[-1]
    same_first = document.load_page(0)
    for page in document:
        print(page.number)
```

### Properties and methods

| API | Result |
|---|---|
| `document.metadata` | Format and document metadata as `dict` |
| `document.stories` (unreleased) | Owned DOCX auxiliary stories, separate from computed pages |
| `document.page_count` | Number of normalized pages |
| `len(document)` | Same as `page_count` |
| `document[index]` | `Page`; negative indexes are supported |
| `document.load_page(index)` | `Page` |
| `document.to_markdown()` | Markdown for all non-empty pages |
| `document.render_page(index, dpi=144, *, clip=None)` | Rendered page/slide or selected region as PNG `bytes` |
| `document.convert(output_format)` | Converted document `bytes` |
| `document.close()` | Idempotently close the document |
| `document.edit()` (unreleased) | Independent editable PDF; unsupported Office preservation profiles fail explicitly |

Slices are not supported. An out-of-range page raises `IndexError`.

After `close()`, document operations raise `DocumentClosedError`. Previously obtained `Page`, `Block`, `Table`, `Cell`, `Link`, and `Image` data remains readable because extraction is eager. `Page.get_pixmap()` still requires its originating document and therefore fails after that document closes.

## `Page`

`Page.number` is always zero-based. DOCX pages are WordCraft layout pages, PDF pages retain source order, and PPTX pages correspond one-to-one with slides.

### Text

```python
page.get_text()             # plain text
page.get_text("text")       # plain text
page.get_text("markdown")   # conventional Markdown
page.get_text("dict")       # structured dictionary
page.get_text("json")       # JSON string
```

An unknown option raises `ValueError`. Plain text preserves table rows using tabs between cells. Markdown emits headings, inline links when exact offsets are known, pipe tables, images, notes, comments, and a final link list for unmatched links.

### Structured content

```python
blocks = page.blocks
links = page.get_links()
images = page.get_images()
numbered_images = page.get_images(full=True)
tables = page.find_tables()
```

`get_links()`, `get_images()`, and `find_tables()` return new lists containing the underlying model objects. `get_images(full=True)` returns one-based `(ordinal, Image)` tuples for loose PyMuPDF compatibility.

### Rendering

```python
png = page.get_pixmap(dpi=144)
detail = page.get_pixmap(dpi=300, clip=(36, 36, 144, 72))
```

The result is encoded PNG bytes, not a PyMuPDF `Pixmap` object. DPI must be finite and between 1 and 1200. PNG physical-resolution metadata records the requested DPI, rounded to integer pixels per meter. Rendering behavior and limits are described in [Formats and fidelity](formats.md) and [Safety](safety.md).

## Data models

All public models are slotted dataclasses and expose `to_dict()`.

### `Block`

```python
Block(
    kind: str,
    text: str = "",
    bbox: tuple[float, float, float, float] | None = None,
    level: int | None = None,
    style: str | None = None,
    links: list[Link] = [],
    images: list[Image] = [],
    table: Table | None = None,
    metadata: dict = {},
)
```

Known block kinds are `paragraph`, `heading`, `table`, `notes`, and `comment`.

DOCX numbered paragraphs carry `metadata["numbering"]` with native `num_id`, nullable `abstract_id`, zero-based `level`, nullable OOXML `format` and parsed `template`. An optional `label` contains the logical native layout label when one unambiguous placement is available. Labels are separate from paragraph text and link offsets; repeated headers with differing labels omit `label`. See [numbering extraction](numbering-extraction.md) for source normalization and page/story limitations.

### `Link`

```python
Link(uri, text="", bbox=None, page=None, metadata={})
```

`metadata` can contain exact Unicode-character `start` and `end` offsets. PDF labels are recovered only when native geometry is unambiguous. Resolved PPTX slide links use `uri="#page=N"` with one-based N and zero-based `metadata["target_page"]`; `link.page` remains the source page. Hidden slides count in normal presentation order. Shape actions retain `metadata["shape"]=True` and do not invent text offsets.

### `Image`

```python
Image(
    name,
    data,
    media_type="application/octet-stream",
    alt="",
    bbox=None,
    page=None,
    metadata={},
)
```

`data` contains owned image bytes. `Image.to_dict()` reports `size` rather than embedding the bytes.

### `Table`

```python
Table(rows, bbox=None, page=None, metadata={}, cells=[])
```

`rows` is the compatible `list[list[str]]` text view. DOCX and PPTX tables additionally provide `cells: list[list[Cell]]`, preserving paragraphs and source merge metadata; DOCX also retains nested tables in source order. Inferred PDF tables leave `cells` empty. `to_dict()` adds the `cells` key only when this structure is available.

Simple `to_markdown()` output treats the first row as the header, pads ragged rows, escapes `|`, and converts cell newlines to `<br>`. Nested, merged, or rich Office tables use escaped HTML tables inside Markdown because pipe tables cannot represent that structure. Plain text uses the flat rows once.

### `Cell`

```python
Cell(blocks=[], col_span=1, row_span=1, covered=False, metadata={})
```

`blocks` contains owned `Block` values; a nested table is available through `block.table`. Span counts are positive, and `covered` identifies a covered source-grid slot (including DOCX vertical-merge continuations and PPTX merge coverage). `metadata["vertical_merge"]` is a DOCX state string (`"none"`, `"restart"`, or `"continue"`) or a PPTX boolean; PPTX also records a `horizontal_merge` boolean. `metadata["grid_column"]` gives the zero-based logical column, including omitted leading DOCX grid cells. WordCraft may insert a required empty paragraph after a nested table, which remains in `blocks`. These extracted values remain usable after the source document closes; changing them does not edit the source document.

## Errors

All native failures are mapped to stable Python exceptions:

| Exception | Meaning |
|---|---|
| `DocExError` | Base error and conversion failures |
| `UnsupportedFormatError` | Unsupported format, conversion pair, or native operation |
| `MalformedDocumentError` | Invalid input, encryption failure, or wrong password |
| `UnsafeArchiveError` | OOXML or native resource-limit violation |
| `DocumentClosedError` | Operation attempted after closing a document |

`ToolNotFoundError` and `ToolExecutionError` remain exported for compatibility with versions before 0.2.0; the in-process engine does not raise them. Ordinary API validation may raise `FileNotFoundError`, `IndexError`, or `ValueError` as described above.

## Auxiliary DOCX stories (unreleased)

`document.stories` is an ordered list of owned `Story(kind, blocks, metadata)` values. Kinds include `header`, `footer`, `footnote`, `endnote`, `comment`, `text_box` and note separators. Blocks reuse existing paragraph/table/link/image values, including nested cell hierarchy. `story.get_text("text" | "markdown" | "dict" | "json")` and `story.to_dict()` expose content without assigning it to an invented page.

Source block order is preserved within each part. Parts use the upstream model's part-ID order, recorded in metadata; original cross-part archive order is unavailable. Each header/footer part occurs once even if rendered on multiple pages. Links/images/table values have `page=None` in stories, and asset source paths include `story_part_id` for identity. Body note references carry `story_refs` offsets and part IDs when exposed by WordCraft.

Body `Document.to_markdown()` retains its existing projection. Comment stories additionally carry `legacy_page_projection=True` because comments remain in the compatibility page output. Callers combining both projections should choose their desired comment scope explicitly. Stories and their model values remain readable after source closure.

## Extraction diagnostics (unreleased)

DOCX/PPTX `document.metadata["diagnostics"]` contains bounded factual notices for constructs observed in the upstream model. Each has `code`, `operation="extract"`, `severity`, `message`, `count` and bounded `sources` locators. Examples distinguish omitted chart data, linearized equations, omitted ruby annotations, cached dirty fields, preview-only embedded objects, unsupported PPTX actions and approximate transformed geometry. Counts remain exact when source examples reach the 100-example total limit.

These notices describe semantic extraction only. They do not claim that rendering/export is correct, that all unsupported source markup is detectable, or that a document is safe for lossless editing. Parser, encryption, conversion and resource failures remain errors; a notice never replaces them. Native normalized JSON output stops at 64 MiB before its output buffer exceeds that budget.

## Portable Markdown

`document.export_markdown()` returns an owned `dict[str, bytes]` containing body/story Markdown, deduplicated local media and a JSON placement manifest. It reuses existing formatting and leaves `to_markdown()` compatible. See [portable Markdown and media](markdown-assets.md) for the schema, bounds and nested-table behavior.


## DOCX source paths and line layout (unreleased)

Paragraph and table metadata use `source_path`; paragraphs and their assets also use `segment`, the zero-based explicit page/column-break segment. Paths identify native model blocks: `[block]`, followed by `[row, physical_cell, block]` for each table nesting. Body values use the absence of auxiliary story keys to identify the Body story; no duplicate body tag is added. Auxiliary metadata uses existing `story_part_id` or separator story-kind/role keys. Identity belongs to this extracted snapshot; paths are not XML paths, file offsets or durable IDs across edits/reparse. Legacy aggregate page comments have no invented paragraph path.

`Page.metadata` additionally exposes native `body_bbox`, zero-based `section` and `display_number`; existing `width` and `height` remain in points. The body box describes a layout frame.

Supported Body paragraphs, including native indexed table-cell placements, have `block.metadata["layout"]`:

```python
{
    "coordinate_space": "page-points",
    "offset_space": "paragraph-utf8",
    "geometry": "native-selection",
    "lines": [
        {"page": 0, "source_range": [0, 19], "rects": [[36, 36, 130, 50]]}
    ],
}
```

The example illustrates the shape, not fixed geometry. Each half-open source range refers to the native paragraph's UTF-8 bytes, including inline-object placeholder bytes; it is **not an offset into `block.text`**. Page/column-break tokens are excluded from their adjacent explicit segment ranges. Ordered line records retain actual pages and repeated table-header occurrences. Whole automatically split paragraph text still appears once on its first assigned page; later line records do not invent normalized page text slices.

Rectangles are native logical selection extents. They can extend outside paint clipping and do not include every object, list label, drop cap or visible glyph overhang. Empty rectangles can accompany valid zero-width source evidence. Missing layout means this projection is unavailable: auxiliary stories and nested native Clip placements are not inferred. `Block.bbox`/`Table.bbox` remain `None` for DOCX; no union box is invented for multi-page/disjoint content. Existing extracted values and metadata are mutable owned snapshots and remain readable after closure; changing them does not change native document content.

Additional extraction limits are 100,000 line observations, 200,000 rectangles, 8,000,000 conservative native work units and 128 MiB conservative retained-provenance allocation allowance, including copied output representations. The allocation allowance can bind well before the count caps: the measured 200-page synthetic report charges 132,081,792 bytes, close to 128 MiB. This is not a general 200-page guarantee. A separate 16 MiB conservative native temporary-memory allowance bounds selection buffers/sort scratch. Limits fail with `UnsafeArchiveError`; malformed native geometry fails with `MalformedDocumentError`. The existing 64 MiB normalized JSON boundary still applies.
