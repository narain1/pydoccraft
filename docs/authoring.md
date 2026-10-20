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

A new PDF starts with one Letter page unless `size` is supplied. `pdf.pages.add()` defaults to Letter independently of the first page; pass `size` explicitly for another size. A new DOCX has a default section and styles; its required blank paragraph is omitted from the user paragraph collection. An explicitly inserted empty paragraph is retained. A presentation starts with no slides and must have at least one slide before serialization.

## Typing and inspection

The package ships `py.typed`. Literal `new("docx")`, `new("pdf")` and `new("pptx")` calls retain the matching document type through context managers and copies. Common creation methods return typed handles; collections and table slices retain their element types. `get_text("dict")` returns a dictionary, while text/Markdown/JSON modes return strings. A dynamic mode string retains the union. These hints describe successful results; existing format, ownership and value checks still apply.

Paragraph, table and style reprs show identity and owner closure without reading document content, including stale handles. A repr is not a validity check: retrieve the desired property to apply normal lifetime checks. Other authoring annotations remain incomplete. The development consumer check is deliberately narrower than a full-package strict-typing guarantee:

```bash
python -m pip install -r tools/typing-requirements.txt
python -m mypy --strict --follow-imports=silent --python-executable "$(command -v python)" tools/check_public_typing.py
```

Run this after installing the wheel built from the checkout. The checker is development tooling and is not a runtime dependency.

## Units, colors and collections

Lengths are finite numbers in points (72 points per inch); `pt`, `mm`, `cm` and `inches` make conversions explicit. Page sizes accept `(width, height)` or `"A4"`, `"Letter"`, `"Legal"`. Positioned `box` values are `(x, y, width, height)` measured from the top-left of the visible page or slide. Alpha, percentages and theme colors are not accepted by solid-color setters. Colors accept `#RRGGBB`, an RGB integer tuple or the documented small name set: black, white, red, green, blue, gray/grey, yellow and navy. Getters return normalized `#RRGGBB` strings, or `None` for absent values where supported.

Paragraph, table, image, page and slide collections support iteration, negative indexing and slices returning lists of handles. A table cell is addressed as `table[row, column]`. `table.rows` is a text snapshot, so editing that list does not mutate the document. Use `table[row, column].text`.

Editable child handles keep their document alive, but closing the document invalidates their access. Extracted values and returned bytes/dictionaries/lists own their data. A collection slice is a list of live handles: changing the list does not change membership; changing a handle edits its owner. Whole-document `copy()` creates an independent owner; detached editable child copies and general cross-document reparenting are not supported. Recreate content through ordinary primitives, or use the specifically documented PDF page-import and bookmark-move operations. Passing a style from another owner fails explicitly.

Iteration captures membership and handles when iteration first advances. Later insertions are omitted from that iteration. Stable DOCX identities and slide/shape identities continue referring to the same objects after unrelated edits; deleting an object makes its old handle unusable. PDF page, annotation and outline positions have generation checks: structural changes invalidate captured handles, including ones still waiting in an iterator, so they cannot silently point to a different object. Table grid edits similarly invalidate positional row/cell handles. Retrieve those handles again after structural edits. PDF field iteration explicitly rejects structural changes. Mutations invalidate derived layout/render snapshots; retained extracted values remain independent snapshots.

DOCX and whole PPTX text bodies accept LF newlines and reject carriage returns (CR/CRLF); normalize external text to LF before insertion. Individual PPTX cell-paragraph edits reject LF: append another paragraph through the cell instead. This prevents inconsistent writer/parser paragraph normalization. Compound mutations validate before committing. Invalid styles, colors, geometry, text or media must leave the prior document usable. Native resource limits also apply to programmatic creation; a document's serializable model is not an unlimited storage container.

## DOCX text and page layout

`document.add_paragraph(text="", style=None)` returns a paragraph. Its `text`, `style` and `alignment` properties are writable; styles can be built-in or created through `document.styles`. Setting paragraph text intentionally replaces run formatting. `add_run(text, **formatting)` returns a revision-bound text range. `paragraph.set_list("bullet" | "numbered", level=0, restart=False, start=None)` controls native list membership; `None` clears it. Ordinary items continue the nearest matching sequence in the same story; restart creates a separate sequence. For a top-level numbered list, `start=7` creates a fresh independent native sequence starting at that integer (1–32767); it does not reassign paragraphs already in another sequence. Later assignments continue the nearest matching sequence as usual. Starts on bullets, cleared membership or nonzero levels are invalid. Supplied starts in headers/footers, including their nested cells, are unsupported because native measurement advances their counters before visible placement. `paragraph.list_style` retains the current kind/level view; extracted numbering metadata can inspect serialized native labels.

`paragraph.find(literal)` returns non-overlapping Unicode-character ranges. A range exposes writable `text` and `format(bold=None, italic=None, font_size=None, font=None, color=None)`. An edit invalidates other existing ranges; stale access fails explicitly. `paragraph.replace_text(old, new)` replaces literal matches across run boundaries atomically and returns the count, preserving surrounding formatting. A range's `link` property reads/sets/clears an HTTP, HTTPS or mailto URL while preserving surrounding formatting. Mixed targets, stale ranges and unsafe URLs fail explicitly. This is a paragraph-scoped primitive, not a template engine or a promise to preserve arbitrary input DOCX packages.

`paragraph[start:stop]` selects the same range by Python Unicode-character offsets, including negative indexes and clamped bounds. Only a contiguous slice with step one is accepted. An empty slice inserts through `.text`; assigning an empty string to a nonempty range removes its text. Deleting all text retains the paragraph. Formatting/link assignment on an empty range fails until text is inserted. Body, cell and auxiliary-story paragraphs share these rules. Selection checks the document revision, and semantic note/image markers remain protected by existing mutation guards. Integer character access, stepped selection, DOCX cross-paragraph ranges and comments are not added.

```python
paragraph[:0].text = "Prefix "
paragraph[7:12].format(bold=True)
paragraph[-3:].text = ""
```

`document.background` sets solid page color. `document.sections[index].page_size` and `.margins` control that section. Margins accept a uniform number or `(top, right, bottom, left)`. `sections.add(start="next_page")` appends a stable section; other supported starts are `continuous`, `odd_page` and `even_page`. Intentional parity/blank pages use upstream layout. The section retains its identity when later sections are appended. Paragraph `spacing` uses `(before_points, after_points, line_multiplier)`; `page_break_before` controls paragraph placement. `document.add_page_break()` inserts the upstream explicit break marker (`\f`), preserving initial and consecutive blank pages. `section.header(variant="default")` and `.footer(...)` return content containers; variants are `default`, `first` and `even`. Inserting into first/even variants enables the corresponding native section/settings behavior; reading does not mutate the document. Header/footer text is saved and included in native rendering/PDF export. Reopened DOCX exposes auxiliary content separately in `document.stories`; body Markdown does not automatically repeat it.

`paragraph.add_field("page" | "page_count" | "section_page_count")` appends a native decimal field and returns the same TextRange used for runs and slices. The range selects the native object marker (`range.text` is U+FFFC), not an evaluated number. Format it through `range.format(...)`, or remove it by assigning `range.text = ""`. WordCraft evaluates the field for PNG/PDF output. Saved DOCX contains an empty dirty cached result: source extraction therefore does not fabricate a current page number. For example:

```python
footer = document.sections[0].footer().add_paragraph("Page ")
footer.add_field("page")
footer.add_run(" of ")
footer.add_field("page_count")
document.sections[0].page_number_start = 3
```

Headers and footers also expose `tables`, `images`, `add_table(rows, *, columns=None)` and `add_image(source, *, width=None, height=None, alt="")`. These reuse the same table, cell, paragraph and picture handles as body content. For example, `section.header().add_table([["Company", "Report"]])` creates a native table; its cells can contain nested tables, paragraphs and PNG pictures. The collections contain direct children; `document.images` includes inline pictures across all authored stories. Deleting `story.images[index]` retains its empty paragraph anchor in `story.paragraphs`, and later additions preserve that anchor. First/even/default variants and copies keep independent content.

Header/footer content occupies a repeated native box and cannot flow onto another page. Table and picture frames extending outside the physical page fail before serialization, save, rendering or conversion, including after later content/font edits. Intentional exact-height cell clipping within a page remains supported. These checks use native placed geometry and do not guarantee that all text ink fits. Inactive first/even variants have no measured cell rectangles until a matching page exists; nested table/image insertion and geometry-dependent section changes can fail atomically with an explicit unsupported error. Ordinary top-level insertion is still available. Note stories retain their paragraph-only profile; rich note content and header/footer linkage, clearing and arbitrary block movement remain open.

`section.page_number_start` is `None` for continuation or an integer from 1 to 32767 for an explicit restart. New sections continue numbering until explicitly restarted. A continuous section cannot restart numbering; setting a restart there, or changing a restarted section to continuous, fails atomically because native layout ignores that combination. The section-page-count field is unavailable in continuous-section scopes because native counting omits a shared first page; ordinary page and total-page fields remain supported there. Up to 10,000 fields across all authored stories are supported. Arbitrary field instructions and nondecimal numbering are unavailable in this profile: native implicit section-format inheritance differs by story, and some numeral formats clamp larger values. Page/count fields do not constitute a general field-calculation or TOC API.

`section.line_numbers = (1, 1, 0, "each_page")` enables native decimal body-line labels; `None` disables them. The tuple is `(count_by, start, distance_points, restart)`: count_by is 1–100, start is 1–32768, and distance is 0–1584pt. Zero distance uses the native 18pt default; saved geometry has twip precision. Restart is `each_page`, `each_section` or `continuous`, matching note-numbering names. Newly appended sections inherit this setting. Intervals select divisible displayed values: start 5 with count_by 2 first displays 6.

Labels count ordinary top-level body lines, including some empty/break lines; tables, auxiliary stories and natively suppressed paragraphs are excluded. DOCX semantic text excludes the decorations, while native PNG/PDF output includes them. PDF extraction may treat aligned labels/body text as a table or reorder labels with different widths; this API does not replace native reading order.

Only single-column numbered sections are supported: the pinned renderer places multi-column labels at overlapping left-margin positions. After earlier numbering, `each_page` cannot resume in a continuous section unless the preceding section also uses `each_page`; the native counter can otherwise retain a value from an older page. Use a new page, `each_section` or `continuous` for that transition. Invalid combinations and later incompatible edits fail atomically. Numbered body text has a conservative 100,000-unit label-work budget: four units per UTF-8 byte plus one per paragraph and inline object, including suppressed paragraphs. Output also checks actual native label bounds and rejects labels outside the physical page or wrapped by formatting. These are label-work/logical-geometry limits, not total process-memory or glyph-ink guarantees.

`document.add_image(source, width=None, height=None, alt="")` and `cell.add_image(...)` accept PNG bytes, a path or a binary stream and return a picture with writable `size`, `alt`, `data`, `crop`, `flip_h` and `flip_v`. A single requested dimension preserves aspect ratio on insertion. Reading `picture.data` returns owned PNG bytes; assigning another source replaces the payload while preserving its existing size and alt text. Change `size` explicitly when the replacement should use another aspect ratio. PNG format, dimensions and decoded size are checked before mutation. DOCX authoring currently accepts PNG only, unlike the broader extraction surface. Image frame dimensions must each be 1–1584 points and fit the native container; sub-point frames are rejected because the native inline layout clamps them to one point.

`picture.crop=(left, top, right, bottom)` uses finite fractions in 0..0.45; `None` resets the crop. `flip_h` and `flip_v` require booleans. The uncropped display frame remains `size`, original PNG bytes remain in `data`, and replacing the source preserves these transforms. These properties use native drawing transforms in saved DOCX, PNG and PDF output. Rotation is not exposed for DOCX pictures: a native quarter-turn fixture loses edge strips, so a local image-resampling workaround is not used.

Use `del document.images[index]` or `del cell.images[index]` to remove one picture placement. Its paragraph remains as an empty editable anchor, preserving unrelated paragraph identities and formatting. Deleted handles fail explicitly. Shared media is deduplicated by the native document API; removing/replacing a placement keeps bytes referenced elsewhere and discards media only when no authored story references it. Table/cell content deletion also releases unreferenced media. Up to 1,000 placements and 64 MiB of unique referenced encoded media are allowed, with the existing per-image decode limits. The cleanup profile covers authored inline images across body, nested tables and auxiliary stories; arbitrary imported graphics, OLE and page pictures need a broader upstream reference API before they can share this path.

DOCX tables support rectangular string rows, writable cell text/background/vertical alignment, `table.merge((row, column), (last_row, last_column))`, and horizontal `cell.split(columns=2)`. Merged coordinates address the anchor; text snapshots contain content at the anchor once and empty covered cells. Partial or intersecting merges and unsupported vertical splits fail. Cells expose `paragraphs`, `tables` and `images`, plus the same `add_paragraph`, `add_table` and `add_image` primitives as the document. Rich nested content stays in its native container; `cell.text = ...` intentionally replaces it and invalidates old descendant handles. Table `column_widths` are point values constrained by the containing page/cell and existing nested content. `borders=(color, width)` applies uniform solid borders; `None` explicitly removes them. `table.row(index)` exposes `height=(points, "at_least" | "exact")` or `None` for automatic height, `repeat_header` and `allow_split`. Repeated header rows must form a contiguous prefix. Integer/decimal/date formatting belongs in ordinary Python before table insertion.

### Named styles and formatting

```python
with doc_ex.new("docx") as document:
    base = document.styles.add("Report body", based_on="Normal")
    base.format(font_size=11, color="navy", space_after=8)
    heading = document.styles.add("Report heading", based_on=base)
    heading.format(bold=True, font_size=18, keep_with_next=True)
    paragraph = document.add_paragraph("Revenue", style=heading)
    paragraph.add_run(" this quarter")  # inherits the paragraph style
    selected = paragraph.find("quarter")[0]
    selected.format(italic=True, underline="single")
    selected.format(italic=None)  # restore inherited italics
```

`document.styles` iterates paragraph/character/table styles in native order and supports integer/slice access or lookup by ID/name. `styles.add(name, kind="paragraph", based_on=None)` returns a style with immutable `id`, `name`, `kind`, `builtin`, writable `based_on`, `next`, `linked`, and `format(**properties)`. IDs and display names are distinct. Passing a Style handle from another document is rejected. Table styles use the same collection and formatting methods, with conditional regions described below. Theme changes and style deletion/import remain open.

`style.formatting` returns direct values; `None` indicates inheritance. `style.resolved_formatting` asks the upstream resolver for effective properties. `based_on` returns another Style or `None`. Duplicate/ambiguous names, wrong-kind or missing bases, inheritance cycles and chains exceeding 32 styles fail before mutation. A document accepts at most 1,024 named styles.

Formatting follows one rule across `add_run`, range/paragraph `format`, and style `format`: omitted properties stay unchanged, `None` removes a direct override, and `False` explicitly disables a boolean property. New runs with omitted bold/italic inherit. `font_size` always means points. A successful range edit refreshes that range; other captured ranges may become stale after any document mutation.

Character keys are `bold`, `italic`, `font_size`, `font`, `color`, `underline`, `strike`, `script`, `highlight`, `background`, and a character `style` ID/name. `color="auto"` uses native automatic color; `background` is RGB shading. Underline accepts bool or `none`, `single`, `words`, `double`, `thick`, `dotted`, `dash`, `dot_dash`, `dot_dot_dash`, `wave`, `double_wave`. Strike accepts bool or `none`, `single`, `double`; script accepts `baseline`, `superscript`, `subscript`. Highlight uses WordCraft's fixed palette (`yellow`, `bright_green`, `turquoise`, `pink`, `blue`, `red`, `dark_blue`, `teal`, `green`, `violet`, `dark_red`, `dark_yellow`, `gray50`, `gray25`, `black`, `white`, `none`). Unsupported names fail explicitly.

`paragraph.format` accepts `alignment`, `indent_left`, `indent_right`, `indent_first`, `space_before`, `space_after`, `line_spacing`, `keep_with_next`, `keep_together`, `widow_control`, `page_break_before`, and `background`. Lengths use points and line spacing is a multiplier. Table-cell spacing inspection includes native table-style inheritance. Paragraph-style `format` accepts both character and paragraph properties, with `background` meaning paragraph shading; character-style background means run shading. Style bases are changed through `based_on`, not a formatting `style` field. Patches are flat, allowlisted and bounded to 16 KiB.

The [cookbook](cookbook.md) composes these APIs into six executable workflows without adding package-specific recipe objects.

## DOCX image backgrounds

`document.background_image = png_bytes_or_source` uses a native stretched PNG over every DOCX page, including differently sized sections. The getter returns owned original PNG bytes; `None` clears only the image. `document.background` remains an independent color beneath it: transparent PNG pixels reveal that color, and changing the color preserves the image. This fill stretches to the page rectangle and can change the image's aspect ratio. It does not provide fit/crop modes, tiling, offsets, arbitrary opacity or section-local placement.

The background shares native media deduplication with body and nested/story images. Replacing or clearing it retains any resources still used by those images; inline `document.images` collections do not include the background. Input is bounded to 16 MiB, 8,192 pixels per side, 16 million decoded pixels and a 64 MiB decode buffer, with a separate 64 MiB aggregate referenced encoded-media budget. Original bytes and independent copies remain valid after closing the owner. Native PNG/PDF output retains color/transparency and supports existing DPI/clipping calls.

```python
with doc_ex.new("docx") as report:
    report.background = "#FAFAF7"
    report.background_image = "page-art.png"
    report.add_paragraph("Quarterly report")
    report.save("report.docx")
```

Native tiling and text watermark rendering have reproduced PNG/PDF differences; see the [upstream findings](upstream-roadmap.md#docx-background-and-watermark-fidelity). They remain outside this profile.

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

PDF `copy(pages=...)` and page insertion accept up to 10,000 selected indexes, preserving order, duplicates and negative indexes. Excess selections raise `UnsafeArchiveError` after one lookahead item, before native subset/import. If index conversion, source reading or selection iteration changes the destination page organization, the outer operation raises a stale-selection `ValueError`; closing the destination raises `DocumentClosedError`. Retrieve current pages before retrying. Caller callback changes remain applied, while the rejected outer operation performs no additional subset/import.

`doc_ex.open(source).edit()` creates an independent editable PDF using original bytes, not extracted text. The conservative initial edit profile refuses encrypted documents, AcroForms, signatures and certification permissions. `edit()` for existing DOCX/PPTX remains unsupported until preservation diagnostics can prevent lossy saves. A new authored Office document can be copied and edited throughout its lifetime.

Without a supplied font, PDF text uses the upstream Helvetica/WinAnsi path and rejects unsupported characters or text that cannot fit. `page.draw_text(text, at=(x, y), font_size=12, font=font_bytes_or_path, color="black")` accepts an explicitly supplied embeddable font and uses upstream shaping, glyph mapping and embedding. Font input is bounded to 16 MiB and subject to PdfCraft's declared-license/embedding checks. This route handles explicit newline rows without automatic wrapping, rejects missing glyphs/overflow, and has tested Latin extended, Greek, Cyrillic, combining accents and ligature words. It does not assert broad complex-script conformance. PDF image insertion accepts bounded PNG/JPEG inputs. Page subsets/imports do not promise preservation of all document-level outlines, tags, forms or metadata. Background readback is supported for backgrounds managed by this editable owner; unknown imported content is not guessed from its appearance.

### PDF annotations

`page.annotations` is an ordered collection. `add("note" | "highlight", box=..., text="", author="", color="#FFD200")` and `add_link(box=..., uri=... | page=...)` return checked annotation handles. A link needs exactly one target; page targets are zero-based. Handles expose `kind`, `box`, `text`, `author`, `color`, `uri` and `target_page`; `update(...)` applies a compound mutation atomically. `del page.annotations[index]` invalidates previously retrieved annotation handles on that page, including those retrieved through another page wrapper.

Notes use the upstream fixed 20×20 PDF-user-unit icon size. Notes and highlights can be moved but not resized; links can be resized. Unsupported type-specific edits fail before changing the document. Readback can include other native annotation kinds, but mutation is limited to the documented profile. A highlight or comment is not secure redaction. Extracted internal links use `#page=N` with a one-based fragment and zero-based `metadata.target_page`.

### PDF paths, metadata and bookmarks

`page.draw_path(commands, stroke="black", fill=None, width=1, close=False)` accepts ordinary tuples: `("move", x, y)`, `("line", x, y)` and `("curve", x1, y1, x2, y2, x, y)`. Coordinates are top-left points; native typed content operators serialize the drawing. The current profile uses opaque RGB, round caps/joins and nonzero winding fill. Paths and strokes must fit inside the page; commands and serialized input are bounded.

`pdf.metadata` is a snapshot. `pdf.update_metadata(title=..., author=..., subject=..., keywords=...)` updates supported document-info fields, with `None` or an empty string clearing a field. Unknown keys and invalid batches fail atomically.

`pdf.bookmarks` is an ordered hierarchy. `bookmarks.add(title, page, index=None)` returns an entry with `title`, `page`, `expanded` and `children`. Child collections support the same insertion, deletion and `move(index, destination, to=another_collection)` operations. Structural outline edits invalidate unrelated bookmark handles; retrieve them again. A child collection refreshes its originating parent after its own insertion, deletion or same-parent move, so it remains usable. Targets track native page organization. Native write depth is limited to 32, and imported outlines deeper than the upstream listing limit are not advertised as fully inspectable. Attachments require a separately bounded upstream API.

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

Slides expose `number`, `size`, writable `name`, `notes`, `hidden` and solid `background`, plus existing PNG/text readback. Shapes have writable `box`; text shapes also have `text`, `font_size`, `font`, `color`, `bold` and `italic`. Uniform formatting getters return `None` for mixed/unresolved inherited values. Tables reuse cell access/background/vertical alignment and add a placement box. PPTX table merging/splitting and grid layout are described below. Image insertion accepts fully validated PNG input.

`slides.add(layout="blank")` also accepts the native `title`, `title_and_content` and `two_content` layouts. Native placeholders remain editable text shapes; `shape.placeholder` reports their role (`title`, `center_title`, `subtitle` or `content` for this profile), or `None` for an ordinary shape. Iterate existing `slide.shapes` to select a role; there is no second placeholder collection. Setting placeholder text retains the native layout/master references and inherited styles. `box` and `rotation` read native resolved geometry; changing either materializes that transform on the shape while preserving its placeholder identity and other inheritance.

```python
slide = deck.slides.add(layout="title_and_content")
for shape in slide.shapes:
    if shape.placeholder == "title":
        shape.text = "Quarterly results"
    elif shape.placeholder == "content":
        shape.text = "Revenue increased\nCosts remained stable"
```

Other known native layout names raise `UnsupportedFormatError`; unknown names raise `ValueError`. Layout/master/theme import, changing an existing slide's layout and cross-presentation copying remain unavailable. Native source support alone does not establish rendering fidelity for vertical, picture or comparison layouts.

`slide.add_shape("rectangle" | "ellipse" | "line", box=..., fill=None, stroke=None, stroke_width=1)` returns a shape with writable `rotation`, `fill` and `stroke=(color, width)` properties. `None` removes a fill or stroke. Line boxes can have one zero dimension. `slide.shapes` lists typed handles in stacking order, supports `move(index, destination)` and deletion, and preserves surviving identities. A PNG `slide.background_image` uses the native picture fill; reading solid `background` for an image fill raises an explicit unsupported error. Setting a solid background replaces the image fill and removes unreferenced owned media.

An image placement returned by `slide.add_image(...)` or the existing shape collection exposes `data`, `alt`, `crop`, `flip_h` and `flip_v`. `data` returns owned PNG bytes; assigning PNG bytes, a path or a binary stream replaces only that placement's source. Its identity, alt text, box, rotation, crop, flips, link and stacking position remain unchanged. Shared media still used by another picture or slide background stays live. Delete a placement through `del slide.shapes[index]` as usual.

`crop=(left, top, right, bottom)` uses fractions from 0 through 0.45; `None` resets all four to zero. Original source bytes remain intact. Native PPTX serialization rounds fractions to 1/100000, so readback can differ by at most 0.000005. `flip_h` and `flip_v` require booleans. The native renderer crops in local shape coordinates, then applies flips and the existing clockwise `rotation`. These are editable placement transforms, with no image-processing dependency. PNG inputs are bounded to 16 MiB before native validation; the presentation retains at most 64 MiB of distinct referenced encoded media. These encoded-byte limits do not claim a total peak-memory bound.

`deck.slides.move(index, destination)` and `del deck.slides[index]` preserve the identities of remaining slide and shape handles. Handles to removed content fail explicitly. These operations are limited to the new authored profile, which has no imported animations, custom shows, navigation or unknown relationships. General slide cloning and source-presentation editing need additional upstream support and preservation checks.

## Limits and evidence

Do not infer complete Open XML parity from successful serialization. Advanced numbering/formatting properties, arbitrary source Office edits, general field evaluation, review operations, charts and diagram fidelity remain roadmap items. No desktop application crates, external document tools, runtime downloads or fallback backends are used. See [validation evidence](../.kiro/specs/rust-document-engine/validation.md) and the operation-specific [readiness audits](content-readiness.md).

## A report recipe

Reports are ordinary Python composition. Format values before inserting them; no separate report schema is needed:

```python
from pathlib import Path
import doc_ex

items = [("Support", 2, 125), ("Training", 1, 300)]
with doc_ex.new("docx") as report:
    report.add_paragraph("Invoice", style="Heading 1")
    rows = [["Item", "Quantity", "Total"]]
    rows += [[name, str(qty), f"${qty * price:.2f}"] for name, qty, price in items]
    report.add_table(rows)
    report.add_paragraph(f"Grand total: ${sum(qty * price for _, qty, price in items):.2f}")
    link = report.add_paragraph().add_run("Payment terms")
    link.link = "https://example.com/terms"
    report.save("invoice.docx")
    Path("invoice.pdf").write_bytes(report.convert("pdf"))
```

The installed-wheel recipe test reopens both outputs, checks table rows appear once, verifies the link and renders both. DOCX reflows through WordCraft; the PDF is fixed output from its native layout, with the same documented font/layout limitations.

### PPTX rich text and navigation

Text-bearing shapes reuse `add_run(text, **formatting)`, `find(text)`, contiguous `shape[start:stop]` selection and `replace_text(old, new)`; matches may cross runs and LF-separated paragraphs. Ranges expose `text`, `format`, `link` and `link_tooltip`. Slices use Python Unicode-character offsets, with each LF paragraph separator counting as one character. They share DOCX slice normalization and return the existing slide range type, including internal-slide links and tooltips. Unsupported nontext shapes fail without synthesizing a text body; table indexing keeps its existing row/column meaning. Range offsets remain checked and revision-bound. Replacement preserves surrounding formatting and uses the first selected character's appearance. Appended runs inherit appearance but do not inherit an adjacent hyperlink.

PPTX character patches support bold/italic, font_size, font, color, underline, strike, script and RGB `highlight`. Underline accepts the shared readable DOCX names plus `thick_dotted`, `thick_dash`, `long_dash`, `thick_long_dash`, `thick_dot_dash`, `thick_dot_dot_dash`, and `thick_wave`. Native XML spellings are not public API. Run background shading and named DOCX styles have no equivalent in this profile and fail explicitly. Rich PPTX table-cell ranges remain separate work.

A shape or text range's `link` accepts an HTTP/HTTPS/mailto URL, a Slide from the same presentation, or `None`. `link_tooltip` requires an existing target. Reading a range with mixed targets/tooltips fails explicitly; assigning `link` replaces or clears the whole range. Reassignment retains a uniform tooltip and clears mixed tooltips. URLs containing characters the XML writer cannot preserve are rejected before mutation. URL and internal slide links survive native serialization and PDF export. Reordering follows stable native slide IDs; deleting a slide still referenced by surviving shapes, text, cells, notes or groups is rejected. PPTX extraction resolves native fixed slide targets to `#page=N` and zero-based `target_page` metadata. Context-dependent, executable and unresolved actions remain outside that fixed-target profile; native reader omissions are documented in the [format guide](formats.md#pptx).

Text edits preflight run/paragraph fragmentation (100,000 each), visible text and duplicated string properties (16 MiB each), URL targets (8 KiB) and tooltips (4 KiB). Unsupported executable actions are never constructed. Whole-shape text and font setters also respect these limits. Existing output and format fidelity limits still apply.

### Multiple DOCX sections

Each section exposes `start`, `page_size`, `margins`, `gutter`, `header_distance`, `footer_distance` and `columns=(count, gap_points, separator_bool)`. Section creation appends flowing content; subsequent document insertions belong to the final section. Section deletion/reorder and insertion into an earlier section are not yet exposed. Columns are limited to 1–45, with at least 18 points of usable width per column. There are at most 128 sections.

`gutter` adds a left binding inset in points, reducing usable width. It accepts 0–1584 points subject to the page's remaining content area. Set `column_widths` to a tuple of explicit point widths matching the current column count; widths are 18–1584 points each, share the gap in `columns`, and must fit the usable width together with those gaps. Unused trailing space is allowed. The native layout positions and flows text; widths are never automatically scaled. `column_widths=None` restores equal columns. Assigning `columns` also clears explicit widths. Page dimensions determine orientation; mirrored/top gutters and independent per-column gaps are not exposed.

```python
section = document.sections[0]
section.gutter = doc_ex.mm(5)
section.columns = (2, 18, True)
section.column_widths = (180, 240)
```

A newly appended section copies its predecessor's setup and receives independent header/footer parts, including independent empty parts for missing variants. Editing an earlier header later cannot leak into the new section. This initial API deliberately has no linked-to-previous switch. Body tables and images must fit the owning section's narrowest column, while header/footer content uses that section's full content width. This conservative limit protects tables continuing from a wider column: the pinned layout measures a table once and does not reflow its grid for narrower subsequent columns. New body tables in unequal-column sections start with fixed width equal to the narrowest column. Existing tables are not silently reformatted: percentage/autofit tables can be rejected even when they initially fit a wider column. Serialization rechecks unequal-section table geometry because later text/font edits can move a table between columns. Existing nested cell limits also apply. Changing page size, margins, gutter or columns validates existing children atomically.

`next_column` is rejected because the pinned layout path treats that transition as a new page. Continuous transitions with differing page sizes are rejected before mutation rather than promising geometry the upstream layout cannot represent. Odd/even starts, leading blank sections and mixed page dimensions have native PNG/PDF regression coverage.

### Rich PPTX cell paragraphs

PPTX cells reuse `cell.paragraphs`, `cell.add_paragraph(text)` and ordinary `Paragraph`/`TextRange` operations. Edit a paragraph's `.text`, `.alignment` or `.format(alignment=...)`; append styled text with `.add_run`, select with `[start:stop]`, or use `.find`/`.replace_text`. Range `.format`, `.link` and `.link_tooltip` follow the existing slide rules, including same-presentation Slide targets. `.formatting` reports direct alignment only. Named Word styles, lists, fields, breaks, paragraph spacing/indentation, effective Word style inspection, nested tables and cell images remain explicitly unsupported.

```python
cell = table[0, 0]
paragraph = cell.paragraphs[0]
paragraph.text = "Delivery: "
paragraph.add_run("Complete", bold=True, color="green")
terms = cell.add_paragraph("Read the terms")
terms.find("terms")[0].link = "https://example.com/terms"
```

Each handle addresses one paragraph. LF is rejected in paragraph text/range replacement; use `cell.add_paragraph` for a new paragraph. Setting paragraph text to an empty string retains the paragraph. Adding a paragraph preserves earlier paragraph handles; ranges still become stale after unrelated document changes. Replacing `cell.text` invalidates that cell's old paragraph handles, and structural grid operations invalidate descendants. Covered coordinates share their canonical merged cell. Ordinary newly created/inserted cells contain an empty paragraph; splitting can expose a cell with no paragraphs, where `add_paragraph` creates the first one directly. Slide reordering and independent document copies retain their existing ownership rules.

DeckCraft fits ordinary horizontal unmerged rows to content, which can draw below the stored table box. Merged-row and vertical cells keep native stored-height behavior. This API does not promise automatic font shrinking, page containment or a rewritten nominal table size; inspect rendered output for overflowing content.

### Shared cell layout and PPTX grid editing

Both DOCX and PPTX cells expose `margins=(top, right, bottom, left)` in points; a scalar sets all sides, and `None` clears the direct override. Getters resolve native default margins. `cell.alignment` applies to direct cell paragraphs only, leaving nested tables alone. It returns a direct `left`/`center`/`right`/`justify` override or `None`; mixed paragraph overrides raise an explicit error. Setting `None` restores paragraph inheritance. This is separate from cell `vertical_alignment`. DOCX paragraph alignment resolves its native formatting; PPTX cell paragraph alignment reports its direct override or `None`.

PPTX tables reuse `column_widths`, `borders`, `row(index)`, `merge(first,last)` and `cell.split()`. Covered coordinates address the merged anchor, while `table.rows` emits its text only once. Partial overlap with an existing merge is rejected. A split restores the existing merged grid and retains combined content at the former anchor; it does not recreate the original separate texts. Explicit `columns` is supported only when it matches an existing horizontal merge. DOCX's default split remains two horizontal columns.

`table.insert_row(index)` returns the new row handle; `insert_column(index)` returns the new column's cell handles. `delete_row(index)` and `delete_column(index)` reject removing the last row/column. Insert indexes must be within the grid or immediately after it; negative indexes count from its existing end. These grid CRUD methods support DOCX and PPTX. Table/shape handles survive structural edits, while previous row/cell handles fail as stale. Descendant paragraph handles in both formats, and DOCX table/image handles, also become invalid after structural edits; handles outside the changed table remain valid. DOCX edits preserve native spans, header-prefix rules and widths; edits that would leave an orphan vertical merge, exceed the containing width or remove positive text width inside explicit cell margins fail atomically. Column resizing enforces the same explicit-margin constraint. Retrieve fresh cells after merge/split/insert/delete. Scalar layout/text changes retain those handles.

PPTX `row.height=(points, "at_least")` is a minimum. Exact and automatic-height requests are rejected in this profile. Native layout may expand rendered text below the stored table box; the box is the stored grid extent. Width/height changes synchronize that extent, and resizing a table shape scales its grid. Explicit cell margins must still leave positive text width. Uniform border setters create explicit solid overrides; `borders=None` writes no-fill lines. Inspecting inherited or mixed borders raises an explicit unsupported/mixed error because a public resolved table-style accessor is unavailable upstream.

PPTX grids allow at most 1,000 rows/columns; DOCX grids allow at most 32,767 rows and 63 columns. Both formats enforce 100,000 aggregate cells, including empty cells; PPTX also counts covered grid slots. Native mutations remain atomic and use upstream grid algorithms.

## Literal templates and batch generation recipes

[examples/template_batch.py](../examples/template_batch.py) constructs an owned DOCX template, replaces caller-chosen literals across styled runs in explicitly selected body/header scopes, validates missing/unused values and expected counts, and writes independent DOCX/PDF letters in a normal Python loop. It preserves unrelated formatting and links, closes failed copies and leaves the source template unchanged. No new template syntax, evaluation, batching API or document schema is introduced.

This recipe accepts an already authored editable template. It does not enable arbitrary source DOCX editing, `.dotx` parsing or repeating rich rows. Those preservation/CRUD requirements remain open. Flowing PDF reports and tables use the earlier DOCX report recipe plus native `convert("pdf")`; fixed-position PDF drawing remains a separate geometry-oriented path.

### PDF page boxes

`page.boxes` returns owned `(x, y, width, height)` tuples for `media`, `crop`, `bleed`, `trim` and `art`, in current top-left view coordinates with rotation and UserUnit applied. `page.set_box(kind, box)` delegates geometry to PdfCraft. Non-media boxes must fit inside the media box; `set_box(kind, None)` resets a non-media box. Changing media recomputes effective intersections and can clip existing boxes, but rejects an empty intersection. Geometry changes invalidate other page/annotation/bookmark handles and cached rendering. The originating page remains usable. Background readback becomes unknown and raises `UnsupportedFormatError` until a new managed background is set.

Known native content-truncation warnings raise the existing resource-limit exception (`UnsafeArchiveError`) during PDF extraction/rendering. Editable import rejects incomplete outline inspection instead of preserving a truncated outline. These checks do not claim that every possible upstream omission is diagnosed.

### Footnotes and endnotes

```python
with doc_ex.new("docx") as document:
    paragraph = document.add_paragraph("See the source for details.")
    note = document.notes.add(paragraph.find("source")[0], "Supporting detail.")
    note.paragraphs[0].add_run(" Additional context.", italic=True)
    document.sections[0].configure_notes(numbering="lower_roman", start=1)
    document.save("notes.docx")
```

`document.notes` is an ordered collection of native note stories. `notes.add(range, text, kind="footnote" | "endnote")` attaches a reference at the end of a same-document body range without removing its text. Body table paragraphs are supported; headers, footers and other notes cannot contain note anchors. Each note exposes `kind`, `paragraphs` and `add_paragraph`, reusing ordinary paragraph formatting and range editing. Whole-document copies own independent notes. The supplied range is refreshed; other existing ranges become stale after insertion.

Editable paragraph text exposes the native object replacement character U+FFFC at note anchors; it occupies one Python character in range offsets. Generic paragraph, range, cell and table edits that would destroy or duplicate a semantic reference are rejected atomically. Replace text around the reference instead. Dedicated note deletion and arbitrary copying of anchored fragments remain unavailable; no note part is silently orphaned.

`section.note_options(kind="footnote")` returns resolved native options. `configure_notes` accepts `placement`, `numbering`, `start` and `restart`; omitted values stay unchanged and `None` restores inheritance. Footnote placement is `page_bottom` or `beneath_text`; endnotes use `section_end` or `document_end`. Numbering accepts `decimal`, `upper_roman`, `lower_roman`, `upper_letter`, `lower_letter` and `chicago`. Start is 1–32767; restart is `continuous`, `each_section`, or footnote-only `each_page`. WordCraft owns numbering, pagination and native DOCX/PDF rendering. Extracted note stories remain separate from body text.

### PDF form creation

```python
with doc_ex.new("pdf") as document:
    page = document.pages[0]
    field = page.fields.add("customer", (40, 40, 200, 30))
    field.value = "Alice"
    page.fields.add("approved", (40, 90, 24, 24), kind="checkbox").value = True
    page.fields.add("country", (40, 140, 200, 30), kind="choice",
                    options=["Canada", "France"]).value = "France"
    document.fields["customer"].value = "Bob"
    document.save("form.pdf")
```

`document.fields` indexes fields by immutable name and iterates checked field handles; `page.fields` filters that collection and creates a widget on the page. Fields expose `name`, `kind`, `value`, `options`, `page` and `box`. Text values are strings, checkbox values are booleans, and choice values are one declared option or `None` to clear selection. `del document.fields[name]` removes the field and its widget. Structural field edits and page organization invalidate existing field handles; retrieve them again by name. Value and box changes retain handles. Assign `field.box=(x, y, width, height)` to move or resize its existing widget on the same page in displayed points. The box must fit the current page view and leave room for the fixed appearance, current value and all choice options. Invalid geometry or overflow fails atomically. Native widget identity, field values and options remain intact. Whole-document copies are independent.

This creation profile supports one widget per field in PDFs created by this owner or its copies. Imported PDFs cannot acquire fields; imported AcroForms remain explicitly unsupported for editing. Text/choice appearances use fixed 12-point Helvetica with WinAnsi characters and conservative fit validation. Newlines, missing glyphs, overflow, duplicate names/options and unsupported kinds fail atomically. Names cannot contain periods, controls or surrounding whitespace. Limits are 1,000 fields/widgets, 256 UTF-8 bytes per name, 256 distinct nonempty options, 4 KiB per option/value and 4 MiB aggregate field text.

`document.fields.flatten()` turns all supported field appearances into page content and returns the number flattened. It verifies that no terminal fields/widgets remain before committing. The native writer may retain an empty AcroForm dictionary, so editable reopening can still reject a flattened file; ordinary extraction and rendering remain supported. Subsetting a PDF with active fields is unavailable; flatten or delete them first. Radio groups, multiline text, autosizing, custom fonts, scripts, signatures and arbitrary imported-field inspection remain open work.

Active PDF form appearances cannot be preserved by the current PDF-to-DOCX exporter, so conversion rejects them explicitly. Flatten supported owned fields first to export their visible text. Field deletion invalidates annotation handles on that page because native annotation indices can shift. Flattening an empty field collection leaves handles unchanged. Rotated/UserUnit flattening tests permit at most four pixels differing by one 8-bit channel level; the measured fixture differed in one pixel, consistent with native coordinate rounding. Unrotated form fixtures are PNG-byte-identical.

### Table views and direct formatting

`add_table(rows=3, columns=4)` creates a rectangular blank grid through the same native constructor as string rows; this works for documents, DOCX cells and slides (slides still require `box`). Empty/ragged grids are rejected. Counts are bounded before allocation, then checked against native format limits. Set a DOCX header prefix with `table.row(index).repeat_header = True`.

`table[row, :]` and `table[:, column]` return ordered lists of cell handles; two slices return a list of row lists. Normal negative indexes, reverse slices and empty slices apply. These are snapshots of handles, so changing a returned list does not alter the grid. Covered merged coordinates still address the same anchor. Structural edits make previously returned handles stale; cell text/layout edits retain them.

DOCX `table.format(...)` and `cell.format(...)` apply atomic typed patches; `.formatting` returns an owned snapshot of direct values. Omitted properties remain unchanged. Nullable values clear direct overrides. Existing convenience setters remain available.

```python
with doc_ex.new("docx") as document:
    table = document.add_table([["Region", "Revenue"], ["West", "$100"]])
    table.format(alignment="center", width=360, layout="fixed",
                 title="Quarterly totals", description="Revenue by region")
    table[0, 0].format(background="yellow", margins=6,
                       borders={"bottom": {"style": "double", "color": "blue", "width": 1}})
    widths = table.fit()
    document.save("table.docx")
```

Table keys are `style`, `look`, `alignment` (`left`, `center`, `right`), `indent`, `width`, `width_percent`, `layout` (`fixed`, `autofit`), `rtl`, `title`, `description` and `borders`. Width is in points; percentage is in 0.02-percent increments through 100. Setting one non-null width preference clears the other; supplying both non-null fails. Title/description are alternative-text metadata, not visible numbered captions, and are limited to 10,000 characters each with an aggregate budget. Reopened DOCX returns them in `Table.metadata` without inserting them into cell text or Markdown. They do not establish accessible table structure in exported PDF.

Cell keys are `background`, `margins`, `alignment`, `vertical_alignment`, `text_direction` (`horizontal`, `down`, `up`), `no_wrap` and `borders`. Paragraph alignment remains distinct from vertical alignment. Margin tuples retain top/right/bottom/left ordering. Snapshots distinguish inherited nullable properties from direct settings; boolean native layout switches remain explicit. Mixed paragraph alignment raises the same explicit error as the cell alignment getter.

Border mappings address `top`, `right`, `bottom`, `left`, and table-only `inside_horizontal`/`inside_vertical`. A line mapping contains native `style`, `color` and `width` values; `color=None` retains the native automatic color. Supported styles are `none`, `single`, `dotted`, `dashed`, `thick`, and horizontal `double`/`wave`; widths use eighth-point steps from 0.25–12 points, with at least 1.5pt for `thick`. Native-substituted styles and unsupported vertical double/wave edges fail explicitly. A side set to `None` inherits, while `style="none"` suppresses that side. `format(borders=None)` clears direct borders; the pre-existing `table.borders = None` convenience explicitly hides them. Adjacent conflicting borders follow pinned native rendering, which does not implement full Word shared-edge conflict resolution.

`table.fit()` measures content through WordCraft and applies its proposed column widths. It clears absolute/percentage preferences and selects native autofit layout. At most 4,000 directly measured cell paragraphs are allowed. Minimum widths that exceed the container, explicit margins that consume a cell, or squeezed nested content fail atomically. The same native geometry checks apply when alignment activates an indent, section page size/margins/columns change, or grid edits resize nested percentage tables. Rejected edits retain existing handles and content. It does not add a second text-measurement algorithm. Direct table-wide fill, patterned shading and cell spacing remain unavailable; use cell backgrounds for solid fills. The new formatting/fit methods are DOCX-only; existing shared PPTX table properties remain supported.

### PDF imports with outlines

`target.pages.insert(index, source, pages=None, outline_title=None)` retains its existing page/link import behavior by default. Supply `outline_title="Appendix"` to copy native source outlines beneath that container. A source without outlines adds no container. Duplicate selected source pages map destinations to the first imported instance. Omitted targets become destinationless outline entries or link annotations; links are not silently reassigned to an unrelated page.

Outline imports reject incomplete/deep/oversized source trees, unsupported external/chained actions and unrepresentable bookmark properties. The extra container level counts toward the depth limit; titles are bounded to 4,096 UTF-8 bytes and must be nonblank without NUL. Sources with forms/encryption remain unsupported; already-owned destination fields are preserved. Page import invalidates page/annotation/bookmark handles as before. Subset copies still do not preserve outlines and remain separate work.

### Conditional DOCX table styles

```python
with doc_ex.new("docx") as document:
    style = document.styles.add("Revenue table", kind="table")
    style.format(margins=6, background="white", row_band_size=1)
    style.format(region="first_row", background="navy", color="white", bold=True)
    style.format(region="odd_rows", background="#EAF0F8")
    table = document.add_table([["Region", "Revenue"], ["West", "$100"]])
    table.format(style=style, look={"header_row": True, "banded_rows": True})
    table.row(0).repeat_header = True
    document.save("styled-table.docx")
```

Table styles reuse `document.styles`, same-kind `based_on` inheritance and `Style.format()`. A table accepts a same-document style handle, ID or name; `style=None` clears its assignment. Style edits and assignment validate affected native table layout, including nested content, before publication. Failed changes preserve the model and existing handles.

Omit `region` (or use `whole_table`) for whole-table defaults. Conditional regions are `first_row`, `last_row`, `first_column`, `last_column`, `odd_rows`, `even_rows`, `odd_columns`, `even_columns`, `top_left`, `top_right`, `bottom_left` and `bottom_right`. Native precedence applies whole-table defaults, column bands, row bands, edge columns, edge rows and corners, followed by direct cell formatting. Corner names describe logical cells; RTL reverses their displayed horizontal positions. Row band size is 1–1,000; column bands currently have width one.

All regions accept supported character properties, cell `background`, paragraph `alignment`, `vertical_alignment` and the existing six-side border mapping. Whole-table defaults additionally accept supported paragraph properties, `margins` and `row_band_size`. Conditional paragraph spacing/indentation/keep properties are unsupported. `None` clears a direct override and `False` explicitly disables boolean formatting. Table-style backgrounds are cell fills, distinct from run shading.

`style.formatting` and `style.resolved_formatting` include whole-table properties and a `regions` mapping. Resolution follows the native base-style chain; a region snapshot is not a final cell appearance after all intersecting conditions and direct overrides. `table.formatting` adds the canonical style ID and six concrete `look` flags. Look keys are `header_row`, `total_row`, `first_column`, `last_column`, `banded_rows` and `banded_columns`; omitted flags stay unchanged and `look=None` resets native defaults. Header styling is independent of `row.repeat_header`, which controls pagination. Pinned upstream border-conflict and patterned-fill limitations still apply.

### Tabs and explicit breaks

```python
with doc_ex.new("docx") as document:
    paragraph = document.add_paragraph("Total\t123.45")
    paragraph.format(tabs=[{"position": 240, "alignment": "right", "leader": "dot"}])
    paragraph.add_break("line")
    paragraph.add_run("Supporting detail")
    paragraph.add_break("page")
    paragraph.add_run("Next page")
    document.save("tabs.docx")
```

Paragraph and whole-table/paragraph style formatting accept `tabs`, an ordered list of mappings with a required point `position` and optional `alignment`/`leader`. Alignments are `left` (default), `center`, `right` and `clear`; leaders are `none` (default), `dot`, `hyphen`, `underscore` and `middle_dot`. Positions range from -1,584 to 1,584 in exact 0.05-point increments; distinct direct positions must be at least 0.5 points apart. Direct and effective lists are bounded to 64 stops to prevent native serialization truncation. Decimal and bar tabs are unsupported at this pin: native decimal layout aligns the segment’s right edge to the stop, and bar layout does not draw a bar.

`tabs=None` and `tabs=[]` both clear the direct list and restore inheritance. To suppress an inherited stop, include its position with `alignment="clear"` and no leader. Native style resolution merges/removes stops. `paragraph.formatting` returns direct values and `paragraph.resolved_formatting` returns native effective paragraph properties, including table-style context. Whole-table styles support tabs; conditional table regions reject them.

`paragraph.add_break("line" | "page" | "column")` appends a native break and returns a revision-bound `TextRange`, sharing existing range invalidation. The default is `line`. Page/column breaks currently require a top-level body paragraph; line breaks also work in cells and auxiliary stories. Raw editable paragraph text contains LF, form-feed or U+000E respectively, each occupying one Python character; generic text setters still reject raw page/column control characters. Native layout determines page/column transitions. Existing `document.add_page_break()` remains available.

DOCX extraction separates explicit page/column-break segments and assigns them using upstream placed-line byte ranges, including when earlier content wraps automatically. It does not assume that each column break advances a page. Automatically wrapped text inside a segment still belongs to its first computed page; exact automatic paragraph slicing remains a documented limitation.

### Style relationships and document defaults

```python
with doc_ex.new("docx") as document:
    document.styles.format_defaults(font_size=11, space_after=8)
    body = document.styles.add("Report body", based_on="Normal")
    heading = document.styles.add("Report heading", based_on=body)
    heading.format(font_size=18, bold=True, keep_with_next=True)
    heading.next = body
    character = document.styles.add("Report heading character", kind="character")
    heading.linked = character
    document.add_paragraph("Overview", style=heading)
    document.add_paragraph("Report text.", style=body)
    document.save("report.docx")
```

`style.next` stores a paragraph-to-paragraph relationship and returns an existing Style handle or `None`. It accepts a same-document handle, ID, name or `None`; self-reference and next-style cycles are valid because they do not participate in inheritance. Paragraph creation still uses its explicitly selected/default style; automatic next-style application on Enter is outside this document API.

`style.linked` joins a paragraph style and a character style reciprocally. Setting a free partner clears the caller's previous reciprocal relationship atomically. Clearing either side clears both sides. Selecting a partner already linked to another style fails without disconnecting that pair. Wrong-kind, unknown, foreign-owner and oversized references fail before publication. Native DOCX serialization preserves both relationships; `based_on` retains its separate same-kind, acyclic inheritance rules.

`document.styles.defaults` returns direct native document character/paragraph defaults. `styles.format_defaults(...)` accepts the existing character and paragraph properties, including tabs; omission preserves a property, `None` clears a direct default except font size, and `False` explicitly disables a boolean. Resetting the document default font size is explicitly unsupported: the pinned native reader changes an omitted size from the resolver’s 11pt fallback to 10pt on reopen. Set an explicit size instead; clearing named-style sizes remains supported. Background means paragraph shading here. Table-region, table-only and style-assignment properties are unsupported. Named styles and direct formatting keep their normal precedence over defaults; resolved style/paragraph snapshots use upstream resolution. Default changes validate affected table geometry and effective tab limits before committing, invalidate cached rendering and preserve independent copies. Arbitrary default-style selection through the native default-kind flag remains unavailable because the pinned resolver's fallback is `Normal`.

Some native built-in headings refer to absent companion character styles. Reading such a `linked` property raises `UnsupportedFormatError`; clear or replace the link to repair it. No companion style is fabricated.

PDF drawing preserves existing page resource bindings, including inherited resources. If a pinned native writer would replace a font, image or graphics-state resource used by existing content, the operation raises `UnsupportedFormatError` atomically. This can affect text, paths, images and backgrounds on imported PDFs; unaffected drawing operations remain usable. Complex directly embedded resource dictionaries also fail conservatively when unchanged bindings cannot be established within the bounded comparison profile. Collision-safe resource allocation is tracked upstream.

DOCX flow controls are verified against native pagination: `keep_with_next` keeps a heading with a following table, `keep_together` moves a fitting paragraph, `widow_control` avoids a single line on either side of a page break, and `page_break_before` supports clearing. This does not promise Word-identical pagination.

A splittable table row can continue its cell paragraphs across pages. A non-splitting row that fits on a fresh page moves intact. The pinned paginator can place an oversized non-splitting row beyond the physical page; authored documents now reject that case before serialization, save, preview or export. Reduce the content/font size, enlarge the page or enable row splitting to recover. The guard also covers exact-height and repeated-header rows without vertical merges. Explicit exact-height clipping within a page remains intentional. Vertical-merge geometry needs separate upstream handling and is outside this narrow overflow check. Documents containing guarded rows incur a native layout pass before saving; traversal limits apply after layout, not before its allocations. Failed saves preserve an existing destination file.

PDF text boxes and image transforms extend the existing drawing methods:

```python
with doc_ex.new("pdf", size=(600, 800)) as document:
    page = document.pages[0]
    page.draw_text("A paragraph that wraps within its area.",
                   box=(40, 40, 260, 100), font_size=16, align="left")
    page.add_image("photo.png", box=(40, 180, 200, 150),
                   crop=(0.1, 0, 0.1, 0), rotation=90, flip_h=True)
    document.save("layout.pdf")
```

`draw_text` requires exactly one of `at=(x, y)` or `box=(x, y, width, height)` in current displayed-page points. Box alignment is `left`, `center`, `right` or `justify`; built-in fonts wrap through native layout. With caller-supplied font bytes, boxes support explicit newline-separated lines and left/center/right alignment. Automatic supplied-font wrapping and justification are unavailable. Native logical line widths and vertical metrics must fit; overflow fails atomically. These checks do not clip glyph outlines, so italic/other ink overhang is not a strict clipping guarantee. Native text wrapping has a bounded work budget in addition to text-byte limits, including the existing `at` path.

Image crop is `(left, top, right, bottom)` with each fraction between 0 and 0.45. Rotation is counterclockwise in 0/90/180/270-degree steps; `flip_h` and `flip_v` are booleans. Native transforms keep the embedded source pixels and clip transformed paint to the destination rectangle. Cropping is visual: it does not remove source pixels from the PDF. All geometry uses the current rotated/cropped view, including PDF UserUnit. Existing resource-preservation checks apply to text boxes and transformed images.

Paragraph borders use the existing formatting vocabulary:

```python
paragraph.format(borders={
    "top": {"style": "double", "color": "navy", "width": 1.5},
    "left": {"style": "single", "color": "blue", "width": 1},
})
```

Paragraphs, paragraph styles and document defaults accept `top`, `right`, `bottom` and `left`. Table styles retain their table/cell-border meaning, including inside edges. Omitted direct sides are unchanged; a side set to `None` clears that direct edge, and `borders=None` clears the whole direct group. Native paragraph inheritance replaces the whole border group: setting a direct left edge does not also retain an inherited top edge. Use `formatting` and `resolved_formatting` to inspect that distinction.

DOCX borders accept `none`, `single`, `dotted`, `dashed`, `thick`, and horizontal `double`/`wave`. Widths are 0.25–12pt in eighth-point increments; `thick` needs at least 1.5pt. Vertical double/wave, triple, dot-dash and narrower widths fail explicitly because the pinned renderer omits, overlaps, substitutes or clamps them. This correction applies consistently to DOCX paragraph, table, cell and conditional-style formatting; PPTX behavior is separate. Paragraph between/bar/inside edges remain unsupported. Document PNG rendering disables the native editor-only header/footer dimming, preserving source colors consistently with PDF output; supported border colors and geometry are verified in both outputs.

Supplied-font PDF drawing rejects pages with 256 or more `PCFull`-prefixed font bindings, including inherited bindings. At this pin, native bounded lookup can miss the full font program on such pages and embed it repeatedly across lines without the expected reuse. The guard runs before embedding and leaves the document unchanged; ordinary repeated font calls remain supported.

Supplied static TrueType fonts additionally require every shaped glyph outline to fit inside a text box. Native unhinted outlines and exact curve bounds include shaping offsets; overflow fails before committing. Boxed CFF fonts are explicitly unsupported because their public native evaluator lacks an aggregate work budget. The existing `at` path is unchanged. Built-in fonts still use native logical metrics and may have small ink overhangs; no path promises clipping. Outline checks cap distinct glyphs, component visits/depth, points, path operations and scratch memory.

### DOCX document properties

`document.metadata` returns an owned dictionary. `update_metadata(**values)` patches core properties and an optional nested `custom` dictionary:

```python
report.update_metadata(
    title="Quarterly report", author="Reporting team", revision=2,
    modified="2026-10-20T12:00:00Z",
    custom={"Approved": False, "Quarter": 4, "Ratio": 1.25},
)
report.update_metadata(custom={"Approved": True, "Ratio": None})
```

Core keys are `title`, `author`, `subject`, `keywords`, `description`, `category`, `last_modified_by`, `created`, `modified` and `revision`. Strings use XML-safe text without surrounding whitespace because the native reader trims it. Empty strings or `None` clear core strings; revision is an unsigned 32-bit integer, with zero/`None` clearing it. Created/modified accept valid UTC `YYYY-MM-DDTHH:MM:SSZ` timestamps, years 0001–9999. Saving leaves caller timestamps and revision unchanged. The dictionary omits cleared core values and always includes `custom`.

Custom values support strings, booleans, signed 64-bit integers and finite floats, preserving their native types. A custom dictionary patches entries: omitted names stay unchanged, per-entry `None` removes one, and `custom=None` clears all. Names use native ASCII case-insensitive identity while retaining their original spelling; ambiguous batch names fail. Custom string whitespace is preserved. Nested objects, arbitrary XML, vectors and custom date wrappers are unavailable. Names are 1–255 UTF-8 bytes with no control characters or surrounding whitespace; values are limited to 64 KiB, with at most 1,024 properties. Incoming JSON, stored core values and stored custom values each have a 256 KiB serialized budget. Compound updates fail atomically, retaining existing ranges and cached output on rejection.

Ordinary DOCX extraction exposes the supported core properties. Imported custom-property inventory, core language and extended/application property editing remain DOC7 gaps. Metadata editing uses native CoreProps/CustomProp and the normal document writer; it does not add arbitrary source DOCX editing.

### PDF text-markup annotations

`page.annotations.add(kind, box=..., text="", author="", color=...)` also accepts `underline`, `strikeout` and `squiggly`, alongside `note` and `highlight`. Existing annotation properties and `update` move the markup, change text/author/color, and preserve searchable page text. Deletion remains `del page.annotations[index]`. Boxes use current displayed-page points; native quadrilaterals preserve rotation, CropBox and UserUnit geometry. Each new mark uses one quadrilateral. Movement is supported; resizing and multi-quad authoring remain unavailable.

Native appearance generation is preflighted before potentially allocating it, including when updating imported highlights. Direct quadrilateral arrays are limited to 1,024 quads with finite coordinates within ±1,000,000 PDF user units (the editable page-box coordinate limit), plus a conservative work estimate within the remaining document-input budget; generated appearance bytes are charged after successful staging. Squiggly's native 10,000-segment cap coarsens the whole line rather than truncating its extent. Unsupported/malformed/over-budget updates leave bytes and cached renders unchanged. Deletion reads the subtype without rebuilding appearances, so an over-budget appearance does not prevent removal. This profile does not establish complete imported-form, reply-thread or arbitrary annotation appearance support.
