# Python API

`pydoccraft` exposes an eager, PyMuPDF-style API. Opening a document reads the source into memory, invokes the native Rust engine, and returns Python objects that own their extracted text and image bytes.

This page describes the extraction API. The unreleased branch also has [native authoring](authoring.md) through `new`, typed content objects and explicit save/copy operations. The [design crosswalk](authoring-api-design.md) and [feature backlog](../TODO.md) describe broader parity work; unchecked items are not implemented promises.

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

`path_or_bytes` accepts `str`, `pathlib.Path`, `bytes`, `bytearray`, or `memoryview`. Paths are expanded with `Path.expanduser()` and missing paths raise `FileNotFoundError`.

Recognized explicit `filetype` values are:

| Canonical format | Accepted values |
|---|---|
| PDF | `pdf`, `.pdf` |
| Word | `docx`, `.docx`, `word` |
| PowerPoint | `pptx`, `.pptx`, `powerpoint` |

For paths, the suffix is checked against the detected content. For bytes, the native detector identifies PDF and OOXML content. A mismatched explicit type raises `UnsupportedFormatError`.

Passwords are supported for encrypted PDF and DOCX input. Encrypted PPTX is rejected by the upstream DeckCraft importer.

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
| `document.render_page(index, dpi=144)` | Rendered page/slide as PNG `bytes` |
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

### `Link`

```python
Link(uri, text="", bbox=None, page=None, metadata={})
```

`metadata` can contain exact `start` and `end` character offsets for DOCX and PPTX links. PDF URI links currently have geometry but generally no recovered label.

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

`rows` is the compatible `list[list[str]]` text view. DOCX tables additionally provide `cells: list[list[Cell]]`, preserving paragraphs and nested tables in source order. PDF/PPTX currently leave `cells` empty. `to_dict()` adds the `cells` key only when this structure is available.

Simple `to_markdown()` output treats the first row as the header, pads ragged rows, escapes `|`, and converts cell newlines to `<br>`. Nested, merged, or rich DOCX tables use escaped HTML tables inside Markdown because pipe tables cannot represent that structure. Plain text uses the flat rows once.

### `Cell`

```python
Cell(blocks=[], col_span=1, row_span=1, covered=False, metadata={})
```

`blocks` contains owned `Block` values; a nested table is available through `block.table`. Span counts are positive, and `covered` identifies a vertical merge continuation. `metadata["vertical_merge"]` records the source merge state; `metadata["grid_column"]` gives its zero-based logical column, including omitted leading grid cells. WordCraft may insert a required empty paragraph after a nested table, which remains in `blocks`. These extracted values remain usable after the source document closes; changing them does not edit the source document.

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
