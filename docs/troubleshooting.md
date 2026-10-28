# Troubleshooting

## A scanned PDF has no text

`get_text()` reads an existing text layer; image-only scans need OCR. Render a
page with `document.render_page(0)` and use an OCR tool chosen by your
application. OCR is not included. See [scanned PDFs](formats.md#scanned-pdfs).

## A table is missing

PDF tables are inferred conservatively from geometry. Complex, irregular or
borderless tables can remain text rather than appearing in `find_tables()`. Check
`page.get_text()` and the page image, and use a purpose-built table tool if the
result is insufficient. DOCX/PPTX table extraction has different semantics.
See [PDF table detection](formats.md#pdf-table-detection).

## A Word page or line breaks differently

DOCX pagination uses the upstream layout engine and available fonts; it is not a
Microsoft Word pagination guarantee. Automatically split paragraphs cannot always
be represented as exact page text slices. Check the bundled font coverage and
render in the deployment environment. See [DOCX pagination](formats.md#docx-pagination).

## An image is missing from extraction

PDF extraction may omit grouped Form artwork or image encodings unavailable to
the upstream exporter. Office extraction returns referenced media, not every
media part. Render the page to inspect its appearance and review the
[format limitations](formats.md#pdf-images).

## I changed an extracted value but the saved document did not change

`Block`, `Table`, `Cell` and related extraction values are owned snapshots.
Changing one does not edit a source file. Use an editable document and supported
native authoring handles, then call `save()` or `to_bytes()`. Editing an arbitrary
imported Office file is not a general preservation guarantee. See
[authoring](authoring.md) and the [operation matrix](support-matrix.md).

## An editable handle became invalid

Closing its owner invalidates editable handles. Structural edits may also
invalidate positional handles; text ranges can become stale after edits.
Reacquire the handle from the owning collection after a structural change.
See [ownership and collections](authoring.md#units-colors-and-collections).

## Rendering uses much more memory than the PDF

Raster dimensions grow with the square of DPI, and decoded images, native
layout, intermediate buffers and PNG bytes can coexist. Reduce DPI or render a
region, and enforce process-level memory and time limits. The raster limit is
not a total process-memory guarantee. See [safety](safety.md#rendering-limits).

## A slide fails to render

DeckCraft may internally catch a render panic and substitute a blank slide.
The adapter checks the process-wide panic count and turns a detected panic into
an error; another thread's panic can cause a spurious failure. Do not treat an
error as an empty slide. See [rendering](formats.md#rendering).
