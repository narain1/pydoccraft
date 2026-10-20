# Portable Markdown and media

`Document.export_markdown()` returns a `dict[str, bytes]` containing a complete in-memory export. It does not write files, download media, or launch another application.

```python
import io
import zipfile
import doc_ex

with doc_ex.open("report.docx") as document:
    files = document.export_markdown()

archive = io.BytesIO()
with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as output:
    for name, data in files.items():
        output.writestr(name, data)
# archive.getvalue() is a portable ZIP; files remain usable after document.close().
```

The mapping contains:

- `document.md`: UTF-8 body Markdown with local image references. It uses the existing Markdown formatter; `to_markdown()` remains unchanged.
- `media/<sha256>.<extension>`: original referenced image bytes. Identical bytes share one asset while each emitted occurrence retains its position in Markdown and the manifest. Source filenames never become output paths. PNG, JPEG, GIF, WebP, TIFF and BMP media types have fixed extensions; other types use `.bin` without conversion.
- `stories/0001.md`, and so on: auxiliary stories in their extracted order, when available. These files use `../media/` references and are separate from the body; headers and notes are not repeated at inferred page positions.
- `manifest.json`: UTF-8 JSON, schema version `1`, listing asset paths, SHA-256 digests, lengths and media types, each emitted placement, story files, and limitations. Placement `path` arrays identify the original normalized page/story, block, recursive table cell and image indexes. `source_page` may differ from the owning table's page when layout splits that table.

All mapping keys are generated relative paths with no traversal segments. Exported image reference paths are relative to their Markdown file. The manifest preserves original image names as JSON data only. Markdown image alternative text and HTML image attributes are escaped. Source text and hyperlinks keep the existing formatter behavior: this export is not an HTML sanitizer, and callers presenting untrusted documents in a browser must apply their own rendering policy. Hyperlinks are never fetched.

Nested tables, merged cells and rich table cells use the existing HTML table fallback. Readers must support inline HTML to display that structure. The exporter emits table content once; it does not repeat flattened cell strings. Unsupported image encodings may not display in a particular Markdown reader. This is an extraction export, not a page-layout reproduction. Auxiliary stories retain their own content; legacy comment blocks can also occur in body extraction.

The aggregate returned bytes are limited to 128 MiB. A conservative UTF-8/escaping estimate also bounds formatting before large joins, with at most 100,000 content nodes and 32 nested tables. These are operation limits, not a total process-memory guarantee. Limit failures raise `UnsafeArchiveError`, missing referenced image bytes and cyclic tables raise `MalformedDocumentError`, and exporting a closed document raises `DocumentClosedError`. No partial mapping is returned on failure. Existing extraction fidelity diagnostics remain available on `document.metadata`; the manifest describes export behavior rather than replacing those diagnostics.
