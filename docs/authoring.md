# Native authoring (unreleased)

The branch adds `doc_ex.new("docx" | "pdf" | "pptx")`. Editable documents own the upstream native model; extracted `Document`, `Block`, `Table` and `Cell` values remain snapshots. This page describes the implemented surface, while [the roadmap](../TODO.md) retains broader parity work. These additions are not part of the published 0.2.1 release.

## Create, save, copy and render

```python
from pathlib import Path
import doc_ex

with doc_ex.new("docx") as report:
    report.add_paragraph("Quarterly report", style="Heading 1")
    paragraph = report.add_paragraph("Prepared for ")
    paragraph.add_run("Acme", bold=True, color="navy")
    table = report.add_table([["Item", "Total"], ["Support", "$250"]])
    table[0, 0].background = "#E0F0FF"
    report.sections[0].margins = doc_ex.mm(20)
    report.sections[0].header().add_paragraph("Acme reports")
    report.save("report.docx")
    Path("report.pdf").write_bytes(report.convert("pdf"))
    Path("preview.png").write_bytes(report.render_page(0, dpi=144))
```

`to_bytes()` serializes the native format. `save(path_or_binary_stream)` writes only that format; a mismatching filename extension is an error. Path saves serialize first and replace through a temporary file in the destination directory. Binary streams must report accepting the complete write and cannot be rolled back. Parent directories must already exist. Context exit closes without saving.

`copy()` returns independent native state, including media. Child handles keep their owner alive, but access after explicit `close()` raises `DocumentClosedError`. Saving does not close the document. Rendering and conversion serialize a snapshot through the same upstream pipelines as existing read-only documents. Mutation invalidates the cached extraction snapshot.

A new PDF starts with one Letter page. A new DOCX has a default section and styles; its required blank paragraph is omitted from the user paragraph collection. An explicitly inserted empty paragraph is retained. A presentation starts with no slides and must have at least one slide before serialization.

## Units, colors and collections

Lengths are finite numbers in points (72 points per inch); `pt`, `mm`, `cm` and `inches` make conversions explicit. Page sizes accept `(width, height)` or `"A4"`, `"Letter"`, `"Legal"`. Positioned `box` values are `(x, y, width, height)` measured from the top-left of the visible page or slide. Alpha, percentages and theme colors are not accepted by solid-color setters. Colors accept `#RRGGBB`, an RGB integer tuple or the documented small name set: black, white, red, green, blue, gray/grey, yellow and navy. Getters return normalized `#RRGGBB` strings, or `None` for absent values where supported.

Paragraph, table, image, page and slide collections support iteration, negative indexing and slices returning lists of handles. A table cell is addressed as `table[row, column]`. `table.rows` is a text snapshot, so editing that list does not mutate the document. Use `table[row, column].text`.

DOCX and PPTX text accepts LF newlines and rejects carriage returns (CR/CRLF); normalize external text to LF before insertion. This prevents inconsistent writer/parser paragraph normalization. Compound mutations validate before committing. Invalid styles, colors, geometry, text or media must leave the prior document usable. Native resource limits also apply to programmatic creation; a document's serializable model is not an unlimited storage container.

## DOCX text and page layout

`document.add_paragraph(text="", style=None)` returns a paragraph. Its `text`, `style` and `alignment` properties are writable; styles must already exist in the upstream default style set. Setting paragraph text intentionally replaces run formatting. `add_run(text, bold=False, italic=False, size=None, font=None, color=None)` returns a revision-bound text range.

`paragraph.find(literal)` returns non-overlapping Unicode-character ranges. A range exposes writable `text` and `format(bold=None, italic=None, size=None, font=None, color=None)`. An edit invalidates other existing ranges; stale access fails explicitly. `paragraph.replace_text(old, new)` replaces literal matches across run boundaries atomically and returns the count, preserving surrounding formatting. This is a paragraph-scoped primitive, not a template engine or a promise to preserve arbitrary input DOCX packages.

`document.background` sets solid page color. `document.sections[0].page_size` and `.margins` control the first section. Margins accept a uniform number or `(top, right, bottom, left)`. Multiple-section creation is not implemented. Paragraph `spacing` uses `(before_points, after_points, line_multiplier)`; `page_break_before` starts a paragraph on a new page. `document.add_page_break()` inserts an explicit break and preserves leading or consecutive blank pages; its returned paragraph contains the `\f` marker. `section.header(variant="default")` and `.footer(...)` return paragraph containers; variants are `default`, `first` and `even`. First/even variants enable the corresponding native section/settings behavior. Header/footer text is saved and included in native rendering/PDF export. Reopened DOCX exposes auxiliary content separately in `document.stories`; body Markdown does not automatically repeat it.

`document.add_image(png_bytes_or_path, width=None, height=None, alt="")` returns a picture with writable `size` and `alt`. A single requested dimension preserves aspect ratio. PNG data is decoded and checked before insertion. DOCX authoring currently accepts PNG only, unlike the broader extraction surface.

DOCX tables support rectangular string rows, writable cell text/background/vertical alignment, `table.merge((row, column), (last_row, last_column))`, and horizontal `cell.split(columns=2)`. Merged coordinates address the anchor; text snapshots contain content at the anchor once and empty covered cells. Partial or intersecting merges and unsupported vertical splits fail. Cells expose `paragraphs`, `tables` and `images`, plus the same `add_paragraph`, `add_table` and `add_image` primitives as the document. Rich nested content stays in its native container; `cell.text = ...` intentionally replaces it and invalidates old descendant handles. Table `column_widths` are point values constrained by the containing page/cell and existing nested content. `borders=(color, width)` applies uniform solid borders; `None` explicitly removes them. `table.row(index)` exposes `height=(points, "at_least" | "exact")` or `None` for automatic height, `repeat_header` and `allow_split`. Repeated header rows must form a contiguous prefix. Integer/decimal/date formatting belongs in ordinary Python before table insertion.

## PDF pages and assembly

```python
with doc_ex.new("pdf", size=(300, 400)) as pdf:
    page = pdf.pages[0]
    page.background = "#FAFAF7"
    page.draw_text("Invoice", at=(30, 40), font_size=18)
    page.add_image("logo.png", box=(30, 80, 64, 32))
    pdf.pages.add(size="A4").draw_text("Appendix", at=(36, 36))
    pdf.save("invoice.pdf")
    with pdf.copy(pages=[1, 0, 1]) as selection:
        selection.save("selection.pdf")
```

`del pdf.pages[index]`, `pdf.pages.move(index, destination)` and `page.rotate(degrees)` use upstream organization operations. Move destinations are final indexes. `pdf.pages.insert(index, source, pages=None, password=None)` imports selected PDF pages with upstream resource remapping and returns new page handles; repeated source indexes are allowed. Removing the final page is rejected. Structural organization invalidates existing page handles explicitly; retrieve them from `pdf.pages` again. A rotated page handle refreshes itself while other old handles become stale.

`doc_ex.open(source).edit()` creates an independent editable PDF using original bytes, not extracted text. The conservative initial edit profile refuses encrypted documents, AcroForms, signatures and certification permissions. `edit()` for existing DOCX/PPTX remains unsupported until preservation diagnostics can prevent lossy saves. A new authored Office document can be copied and edited throughout its lifetime.

PDF text currently uses the upstream Helvetica/WinAnsi path and rejects unsupported characters or text that cannot fit. It does not yet provide general Unicode font embedding. PDF image insertion accepts bounded PNG/JPEG inputs. Page subsets/imports do not promise preservation of all document-level outlines, tags, forms or metadata. Background readback is supported for backgrounds managed by this editable owner; unknown imported content is not guessed from its appearance.

### PDF annotations

`page.annotations` is an ordered collection. `add("note" | "highlight", box=..., text="", author="", color="#FFD200")` and `add_link(box=..., uri=... | page=...)` return checked annotation handles. A link needs exactly one target; page targets are zero-based. Handles expose `kind`, `box`, `text`, `author`, `color`, `uri` and `target_page`; `update(...)` applies a compound mutation atomically. `del page.annotations[index]` invalidates previously retrieved annotation handles on that page, including those retrieved through another page wrapper.

Notes use the upstream fixed 20×20 PDF-user-unit icon size. Notes and highlights can be moved but not resized; links can be resized. Unsupported type-specific edits fail before changing the document. Readback can include other native annotation kinds, but mutation is limited to the documented profile. A highlight or comment is not secure redaction. Extracted internal links use `#page=N` with a one-based fragment and zero-based `metadata.target_page`.

## PPTX slides and typed shapes

```python
with doc_ex.new("pptx", size=(720, 540)) as deck:
    slide = deck.slides.add()
    slide.name = "Overview"
    slide.notes = "Presenter notes"
    slide.background = "#E0F0FF"
    title = slide.add_text("Quarterly results", box=(36, 36, 600, 70))
    title.font_size = 28
    title.bold = True
    title.color = "navy"
    table = slide.add_table([["Name", "Score"], ["Alice", "99"]], box=(36, 140, 600, 180))
    table[1, 1].background = "#FFFFFF"
    slide.add_image("logo.png", box=(36, 400, 64, 32), alt="Company logo")
    deck.save("results.pptx")
```

Slides expose `number`, `size`, writable `name`, `notes`, `hidden` and solid `background`, plus existing PNG/text readback. Shapes have writable `box`; text shapes also have `text`, `font_size`, `font`, `color`, `bold` and `italic`. Uniform formatting getters return `None` for mixed/unresolved inherited values. Tables reuse cell access/background/vertical alignment and add a placement box. PPTX cell merges/splits are not exposed yet. Image insertion accepts fully validated PNG input.

`slide.add_shape("rectangle" | "ellipse" | "line", box=..., fill=None, stroke=None, stroke_width=1)` returns a shape with writable `rotation`, `fill` and `stroke=(color, width)` properties. `None` removes a fill or stroke. Line boxes can have one zero dimension. `slide.shapes` lists typed handles in stacking order, supports `move(index, destination)` and deletion, and preserves surviving identities. A PNG `slide.background_image` uses the native picture fill; reading solid `background` for an image fill raises an explicit unsupported error. Setting a solid background replaces the image fill and removes unreferenced owned media.

`deck.slides.move(index, destination)` and `del deck.slides[index]` preserve the identities of remaining slide and shape handles. Handles to removed content fail explicitly. These operations are limited to the new authored profile, which has no imported animations, custom shows, navigation or unknown relationships. General slide cloning and source-presentation editing need additional upstream support and preservation checks.

## Limits and evidence

Do not infer complete Open XML parity from successful serialization. Lists, many advanced formatting properties, arbitrary source Office edits, field evaluation, review operations, charts and diagram fidelity remain roadmap items. No desktop application crates, external document tools, runtime downloads or fallback backends are used. See [validation evidence](../.kiro/specs/rust-document-engine/validation.md) and the operation-specific [readiness audits](content-readiness.md).
