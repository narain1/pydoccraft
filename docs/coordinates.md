# Coordinates, boxes and offsets

Geometry describes an extraction or editing operation, not a universal layout
model. Check the format-specific contract before using a value.

## Points and rectangles

One point is 1/72 inch. PDF extracted block and table bounding boxes use
`(x0, y0, x1, y1)` in displayed page coordinates. Their width and height are
`x1 - x0` and `y1 - y0`. Positioned authoring boxes and the `clip` argument to
rendering instead use `(x, y, width, height)` measured from the displayed
top-left. A clip `(36, 36, 144, 72)` covers a 2-by-1-inch region starting
half an inch from the top-left; at 144 DPI its ideal raster size is 288 by 144
pixels. Native rounding and page clipping can affect the final dimensions.

PDF display coordinates account for the inspected page view, including crop and
rotation. Do not use them as raw PDF user-space coordinates. Page, image and
block geometry can be absent; DOCX block/table bounding boxes are not invented
for multi-page or disjoint content. See [formats](formats.md) and the
[Python API](api.md).

## Text offsets

Link `metadata["start"]` and `metadata["end"]` refer to character offsets in
the associated normalized block text when available. The range is end-exclusive:
for `A café`, `[2:6]` selects `café`. Offsets may be absent when an exact label
cannot be recovered. These are not PDF byte offsets or source-file offsets.

DOCX line-layout metadata can instead describe source ranges in UTF-8 bytes.
For `café`, the final `é` occupies one Python character but two UTF-8 bytes;
do not use a byte range directly to slice a Python string. Source paths and
ranges identify a particular extraction snapshot, not durable IDs across edits
or reparsing. See [DOCX source paths and line layout](api.md#docx-source-paths-and-line-layout).
