# Native authoring

Version 0.3.0 adds `doc_ex.new("docx" | "pdf" | "pptx")`. Editable documents own the upstream native model; extracted `Document`, `Block`, `Table` and `Cell` values remain snapshots. This page describes the implemented surface, while [the roadmap](../TODO.md) retains broader parity work. These additions are not part of earlier releases such as 0.2.1.

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

Owned Word/PDF metadata and DOCX picture/table/cell `.formatting` snapshots also have precise output type hints. Unset metadata keys are absent; Word's `custom` mapping is always present and contains strings, booleans, integers or floats. Direct formatting values may be `None`; cell margin snapshots remain lists, while picture position/distance remain tuples. These hints describe existing snapshot values and do not change runtime input validation or make edits to the returned dictionary affect the document. Cell margins already accept lists; table/cell border hints also accept their exact snapshot dictionaries. Static compatibility does not bypass runtime combination rules: an inline picture snapshot includes derived position/distance fields that cannot be submitted as floating parameters with inline wrapping. Supported floating-picture and table/cell snapshots can be passed to their existing format method within the normal scope guards. Reset a picture with `picture.format(wrap="inline")`. Paragraph/style/default output hints also describe their existing native values. Paragraph direct formatting guarantees the alignment key; Word-only keys should be read with `snapshot.get(...)` because DOCX-only properties are absent from PPTX cell snapshots. A PPTX cell paragraph reports alignment, six nullable direct indentation/spacing values and nullable direct tab stops; their supported direct snapshot replay is also typed. Assigning its returned tab list separately to an input-only ParagraphFormatting TypedDict can still encounter TypedDict list invariance; the full-snapshot overload avoids that unrelated assignment. Successful resolved paragraph snapshots are Word-only and contain concrete paragraph defaults except nullable borders/background. Style snapshots guarantee shared character keys, with paragraph/table keys present only for their native kind; `Style.kind` is not a static type discriminator. Resolved style hints remain conservatively nullable because table inheritance can retain unset values. Table regions have no nested regions or paragraph-only properties. Whole-table style margin values remain lists. Document defaults contain the character/paragraph keys and no table-only fields. Language and strike retain their native nullable/boolean/string types. These are private output schemas, not new exported classes or a promise that every snapshot can be unpacked as a formatting input.

Paragraph, table and style reprs show identity and owner closure without reading document content, including stale handles. DOCX body, story and cell handles use opaque owner-local identities, independent of native block positions. Private `_id` values and numbers shown by paragraph/table reprs have changed for newly created DOCX objects; they are diagnostics, not collection indexes or persistent cross-document identifiers. Copies preserve their own identities while remaining independent owners. A repr is not a validity check: retrieve the desired property to apply normal lifetime checks. Other authoring annotations remain incomplete. The development consumer check is deliberately narrower than a full-package strict-typing guarantee:

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

PDF page lookup/deletion/movement and slide deletion/movement reject index callbacks that change their captured selection before the outer operation proceeds. The callback’s own changes remain visible. PDF selection uses page generation and count; page-box and rotation changes also invalidate it. Slide mutation uses the ordered native IDs. Metadata or content changes that leave those snapshots valid are allowed. Existing stable-ID slide lookup continues to refer to the originally selected slide after reordering.

DOCX and whole PPTX text bodies accept LF newlines and reject carriage returns (CR/CRLF); normalize external text to LF before insertion. Individual PPTX cell-paragraph text setters reject LF and VT: use paragraph.add_break() for a line break or append another paragraph through the cell. This prevents inconsistent writer/parser paragraph normalization. Compound mutations validate before committing. Invalid styles, colors, geometry, text or media must leave the prior document usable. Native resource limits also apply to programmatic creation; a document's serializable model is not an unlimited storage container.

## DOCX table deletion

Delete a whole owned table through its existing collection: `del word.tables[index]`, `del header.tables[index]`, `del footer.tables[index]` or `del cell.tables[index]`. Integer and negative indexes are supported; deletion of slices is not. Nested tables and merged-cell aliases use the same canonical container. Headers and footers must be independently owned. Rich notes, comment parts and imported document editing are outside this profile.

Deletion removes the table and its descendants. Their old table, cell, row, paragraph and picture handles become invalid; surviving handles in that container and other stories keep their identities. Removing a nested table does not invalidate its parent grid. Copies remain independent. Existing required final blanks stay hidden and reusable, while authored empty paragraphs stay editable. Text, cells and uniquely referenced media are reclaimed; media still used elsewhere, including a page background, is retained.

Bookmarks, note references, owned comments and section boundaries cannot be removed incidentally. Clear those references using their existing APIs first; comments require leaf replies to be deleted before their root. A missing owned comment pair reports `ValueError` during candidate validation, before the general protected-anchor error. Rejected deletion leaves the document, counters and cached output unchanged. Later siblings containing valid comments retain fresh range lookup after their native paths shift.

Index conversion captures the intended table identity. A callback deleting that table cannot redirect the operation to a replacement; its own changes remain. Stale cell grids and closed owners fail before conversion callbacks. Ordinary output and geometry guards still apply. This is whole-table removal, not general block movement, copying or cross-container adoption.

## DOCX table movement

Use `word.tables.move(index, destination)`, `header.tables.move(...)`, `footer.tables.move(...)` or `cell.tables.move(...)` to reorder an intact table in its current container. Both indexes select tables and support negative values; destination is the final table ordinal. Intervening paragraphs keep their relative order, but can move to the other side of the selected table. Moving table A past table B in `[A, paragraph, B]` gives `[paragraph, B, A]`; reversing table ordinals alone does not promise to restore every intervening block position.

The supported containers are the body within one section, independently owned headers/footers, and ordinary nested or merged cells. Held table, row, cell, paragraph, nested-table and picture handles retain their identities, including displaced siblings and ancestor grids. Actual movement invalidates existing text ranges and rendered snapshots once; use fresh ranges afterward. A same-index move preserves bytes, ranges, revision and cached output after scope and reference validation. Copies stay independent and required blank paragraphs are preserved by identity.

A moved subtree cannot contain bookmarks, note references, comments or section boundaries, including on a no-op. Bookmarks and comments can be removed explicitly through their existing APIs first. Clearing note content retains its reference, so that table still cannot move. Valid anchors in retained siblings stay attached and comments resolve fresh ranges after the shift. Owned table PAGE fields remain unsupported; total and section page counts retain their existing native evaluation. Moving across containers or sections, importing content, and rich-note/PPTX table movement remain outside this profile.

Index-conversion callbacks cannot redirect a pending move after changing or closing the owner. Stale cell grids reject before those callbacks. Immediate ownership, source-work, registry, media and revision checks remain active; rejection preserves the prior state except for the callback's own completed mutation. Ordinary serialization/render geometry guards apply after movement; a successful move does not certify every later layout.

## DOCX comments

Owned documents expose one `document.comments` collection for roots and direct replies, in creation order. It uses existing paragraph ranges:

```python
paragraph = document.add_paragraph("Please review this whole paragraph.")
comment = document.comments.add(paragraph[:], "Check the wording", author="Ada")
reply = document.comments.add(comment, "Updated", author="Ben")
comment.range.format(bold=True)
reply.resolved = True
reply.text = "Updated and checked"
del document.comments[-1]  # Delete replies before their root.
del document.comments[0]
```

A root accepts one nonempty whole body paragraph, including an ordinary, merged or nested body-table cell. The paragraph must have no other inline objects or semantic anchors. Cell ancestors must use horizontal text and must not have exact-height rows. Headers, footers, notes, empty/point anchors, interior or cross-paragraph ranges, overlapping roots and nested replies are unsupported. There is at most one root per paragraph. Native interior comment markers change shaping for ligatures, Arabic and combining text, so this API deliberately does not promise arbitrary range comments; see [the upstream findings](upstream-roadmap.md#owned-docx-comment-anchors).

Each comment has mutable `text`, `author`, `date` and `resolved` properties. Comment text is plain text, may be empty and accepts LF line breaks; rich comment paragraphs, tables and media are not exposed. Author defaults to `"Unknown"` and must be nonempty, trimmed, single-line text. Date defaults to `""` or accepts a valid UTC `YYYY-MM-DDTHH:MM:SSZ` value. `parent` is `None` for a root and a root handle for a reply. `range` is a read-only property returning the existing editable, revision-bound TextRange strictly inside the root markers, with normal Unicode-character offsets; replies return the same root range. Native marker characters remain visible through the underlying paragraph text, as with other semantic objects. Use `comment.range` to edit the anchored text without selecting its markers.

Root endpoints remain guarded after every edit and before output. Replacing the inner text with nonempty text and changing its formatting are supported; appending outside the markers, emptying the anchor, rotating its cell or applying exact row height fails atomically. Removing an anchored paragraph produces `ValueError` for the missing exact marker pair before the general protected-anchor guard. Delete the comment through its collection first. A root with replies cannot be deleted until its replies are removed. Stale/deleted handles cannot retarget a reused native part; copies are independent. The collection supports iteration, integer/negative indexing, slices as handle lists, and integer deletion. It does not adopt comments from read-only opened documents.

Native DOCX output stores comments in separate parts rather than inserting their text into the body. Reopened extraction retains the existing compatibility projection: comments also appear once in legacy page Markdown and as detached `document.stories` snapshots marked `legacy_page_projection`. Do not append those story texts to legacy page output again. PNG previews shade anchors, without comment balloons; setting `resolved=True` stores the state but does not hide that shading. Default PDF conversion omits comment markup and comment text. There is no new display-mode switch or tracked-change API.

Limits are 1,000 comments per document, 200 direct replies per root, 1 MiB UTF-8 per comment text, 1,024 UTF-8 bytes per author and 256 KiB of native comment metadata. Private comment paragraphs and root marker bytes count toward the existing 100,000-block and 16 MiB document-text budgets. Validation bounds source traversal at 2,000,000 charged items, precharging parts, comments, blocks, private runs, rows, cells and inline objects before their bounded scans. Comment identities are monotonic within an owner, and native parts remain below the reader's 50,000-part cutoff. Resource, identity and revision exhaustion fail before committing. This is a bounded owned-comment profile; broader REV1/TXT8/REV5 criteria remain open.

## DOCX text and page layout

`document.add_paragraph(text="", style=None)` returns a paragraph. Its `text`, `style` and `alignment` properties are writable; styles can be built-in or created through `document.styles`. Setting paragraph text intentionally replaces run formatting. `add_run(text, **formatting)` returns a revision-bound text range. `paragraph.set_list("bullet" | "numbered" | "decimal_outline", level=0, restart=False, start=None)` controls native list membership; `None` clears it. Ordinary items continue the nearest matching sequence in the same story; restart creates a separate sequence. For a top-level numbered or decimal-outline list, `start=7` creates a fresh independent native sequence starting at that integer (1–32767); it does not reassign paragraphs already in another sequence. Later assignments continue the nearest matching sequence as usual. Starts on bullets, cleared membership or nonzero levels are invalid. Supplied starts in headers/footers, including their nested cells, are unsupported because native measurement advances their counters before visible placement. `paragraph.list_style` retains the current kind/level view; extracted numbering metadata can inspect serialized native labels.

`decimal_outline` uses native decimal hierarchy labels such as `1.`, `1.1.`, and `1.1.1.` at levels 0–8. It is available only on top-level horizontal body paragraphs in a single-column section that is neither continuous nor immediately followed by a continuous section. Cells, headers, footers and notes reject this new kind. Ordinary numbered and decimal-outline sequences remain distinct even though their level-zero labels look alike; starts and restarts retain independent native identities.

Assignment checks native layout before committing. Later font, indent, text or margin changes can make save/render/export fail when the label or first-line logical extents no longer fit the native body area; restore fitting geometry to recover. These are logical advance bounds, not arbitrary-font ink guarantees. Targeted outline text has a 100,000-unit layout-work cap (four per UTF-8 byte plus one per paragraph and inline object), and definition lookup and observed geometry scans have separate 2,000,000-unit caps. Native layout allocation precedes the observed-result checks. Custom markers/definitions and broader list layout remain open.

`paragraph.find(literal)` returns non-overlapping Unicode-character ranges. A range exposes writable `text` and `format(bold=None, italic=None, font_size=None, font=None, color=None)`. An edit invalidates other existing ranges; stale access fails explicitly. Across Word paragraphs and slide shape/cell ranges, `range.format(...)` checks owner lifetime and range revision before running argument-normalization callbacks. A callback that mutates the document still makes the range stale; native validation rejects the formatting without undoing the callback's own changes. `paragraph.replace_text(old, new)` replaces literal matches across run boundaries atomically and returns the count, preserving surrounding formatting. A range's `link` property reads/sets/clears an HTTP, HTTPS or mailto URL while preserving surrounding formatting. Mixed targets, stale ranges and unsafe URLs fail explicitly. This is a paragraph-scoped primitive, not a template engine or a promise to preserve arbitrary input DOCX packages.

`paragraph[start:stop]` selects the same range by Python Unicode-character offsets, including negative indexes and clamped bounds. Only a contiguous slice with step one is accepted. An empty slice inserts through `.text`; assigning an empty string to a nonempty range removes its text. Deleting all text retains the paragraph. Formatting/link assignment on an empty range fails until text is inserted. Body, cell and auxiliary-story paragraphs share these rules. Selection checks the document revision, and semantic note/image markers remain protected by existing mutation guards. Integer character access, stepped selection and DOCX cross-paragraph ranges are not added. The [bounded comment API](#docx-comments) accepts a whole paragraph slice.

```python
paragraph[:0].text = "Prefix "
paragraph[7:12].format(bold=True)
paragraph[-3:].text = ""
```

DOCX paragraphs accept `paragraph.format(direction="rtl" | "ltr" | None)`. Direction follows native style/default inheritance; `None` clears the direct override and `"ltr"` explicitly selects left-to-right. Native alignment, indents, tabs and list labels use the paragraph start edge, so logical left alignment appears on the right in RTL. Setting direction preserves the existing alignment and source text. The same paragraph formatting works in cells, headers, footers and notes, and in paragraph/whole-table styles. It does not reverse table columns (`table.format(rtl=...)`) or select vertical cell orientation (`text_direction`). Conditional table-style regions still reject paragraph direction.

`paragraph.add_run(..., language="fa-IR")` and `range.format(language=...)` set native run-language metadata; `None` restores inheritance. Character/paragraph styles, defaults and conditional table-style character regions accept the same key. Tags contain nonempty ASCII alphanumeric components separated by single hyphens, with at most 63 bytes and preserved spelling/case. This is a bounded literal-tag profile, not a complete BCP47 validator, font selector or spelling API. The native reader drops longer values, so authoring rejects them. Direct/effective snapshots expose both keys. The mixed Persian/Latin/digit fixture has native shaping and PNG/PDF placement evidence; Unicode source/range preservation does not certify every script or glyph. Complex-script font controls, CJK typography and substitution diagnostics remain open; these DOCX keys do not imply PPTX/PDF formatting support.

`document.background` sets solid page color. `document.sections[index].page_size` and `.margins` control that section. Margins accept a uniform number or `(top, right, bottom, left)`. `sections.add(start="next_page")` appends a stable section; other supported starts are `continuous`, `odd_page` and `even_page`. Intentional parity/blank pages use upstream layout. The section retains its identity when later sections are appended. Paragraph `spacing` uses `(before_points, after_points, line_multiplier)`; `page_break_before` controls paragraph placement. `document.add_page_break()` inserts the upstream explicit break marker (`\f`), preserving initial and consecutive blank pages. `section.header(variant="default")` and `.footer(...)` return content containers; variants are `default`, `first` and `even`. Inserting into first/even variants enables the corresponding native section/settings behavior; reading does not mutate the document. Header/footer text is saved and included in native rendering/PDF export. Reopened DOCX exposes auxiliary content separately in `document.stories`; body Markdown does not automatically repeat it.

`paragraph.add_field("page" | "page_count" | "section_page_count")` appends a native decimal field and returns the same TextRange used for runs and slices. The range selects the native object marker (`range.text` is U+FFFC), not an evaluated number. Format it through `range.format(...)`, or remove it by assigning `range.text = ""`. WordCraft evaluates the field for PNG/PDF output. Saved DOCX contains an empty dirty cached result: source extraction therefore does not fabricate a current page number. For example:

`page` is unavailable in owned table cells, including nested and merged cells in every authored story. The pinned native table layout can keep the preceding paragraph's page-number context after pagination, so a cell on page 2 can display 1. The guard rejects this scope at insertion, every staged edit and serialization; it does not rewrite field results or guess when the value happens to be correct. Use a direct body, header, footer or note paragraph for `page`. Table `page_count` and `section_page_count` remain available within their existing section restrictions, as do supported title/author fields. This is an owned-authoring guard; rendering read-only imported documents continues to use native behavior and may exhibit the same limitation.

```python
footer = document.sections[0].footer().add_paragraph("Page ")
footer.add_field("page")
footer.add_run(" of ")
footer.add_field("page_count")
document.sections[0].page_number_start = 3
```

Headers and footers also expose `tables`, `images`, `add_table(rows, *, columns=None)` and `add_image(source, *, width=None, height=None, alt="")`. These reuse the same table, cell, paragraph and picture handles as body content. For example, `section.header().add_table([["Company", "Report"]])` creates a native table; its cells can contain nested tables, paragraphs and PNG pictures. The collections contain direct children; `document.images` includes inline pictures across all authored stories. Deleting `story.images[index]` retains its empty paragraph anchor in `story.paragraphs`, and later additions preserve that anchor. First/even/default variants and copies keep independent content.

`story.clear()` removes the content of an independently owned header/footer, including nested tables and pictures. It retains the explicit part reference, section distances, first/even flags and page settings; it does not link the story to the preceding section. Native layout can repaginate when the story height changes. Removed child handles become invalid, while unrelated paragraph/table/picture handles remain usable. Existing text ranges follow normal document revision rules and become stale after a real clear. Retrieve fresh children after adding new content.

Clearing a missing or already-cleared story does nothing: serialized bytes, existing ranges and cached renders stay valid. An explicitly authored empty paragraph counts as content and is removed once. Shared images/backgrounds survive until their last reference is removed; copied sections remain independent. Outgoing internal links are removed with their text, while protected note/bookmark/comment anchors still prevent unsafe deletion. `Note.clear()` clears content while retaining the note reference; see the note lifecycle below. This operation does not provide general block movement or imported DOCX editing.

`del story.paragraphs[index]` removes one direct paragraph from an owned header, footer or note, using normal integer and negative indexes. Surviving paragraph, table, cell and picture handles keep their identities even when their native positions shift. The deleted paragraph handle becomes invalid and existing text ranges become stale normally. A required empty paragraph after a table is retained by the native model, hidden from the collection and reused by a later addition. Deleting the last note paragraph preserves its reference and styled number mark, as `note.clear()` does.

`del document.paragraphs[index]` also removes an ordinary direct owned body paragraph. Surviving nested table/cell/picture handles keep their identities and source order. Section-ending paragraphs cannot be removed: all section objects, properties and boundaries remain intact. If the last ordinary paragraph of the final section is removed, the native model retains one hidden blank paragraph for that section; a later addition reuses it. Explicitly authored empty paragraphs remain real deletable objects. Removing content can repaginate the document and invalidates existing ranges and cached renders normally.

`del cell.paragraphs[index]` removes an ordinary paragraph from an owned DOCX body/header/footer cell, including nested cells and covered coordinates of a merged cell. The collection excludes picture paragraphs and hidden required blanks. Surviving paragraphs, nested tables, pictures and neighboring cells retain their handles and table generations. Deleting the last paragraph, or the paragraph after a final nested table, leaves exactly one native required blank hidden from the collection; `cell.add_paragraph(...)` reuses it. Other authored empty paragraphs remain visible and keep their identities. Grid edits still invalidate held cell/paragraph handles normally. The same collection also supports same-cell ordinary paragraph movement with the rules below.

Slice deletion remains unsupported in both formats. PPTX cell-paragraph deletion is described below. Protected semantic anchors, aliased header/footer parts and section-boundary paragraphs fail atomically. A bookmark target with incoming links raises the existing missing-target ValueError before the protected-anchor check; remove or retarget links first, and use the bookmark mapping to remove the protected pair explicitly. Outgoing links and supported fields can disappear with their paragraph without removing their targets or unrelated shared resources. Callback index normalization keeps the captured logical target; caller changes can remain, but the outer operation cannot silently select a replacement. This does not add general block moves, source-document adoption, section deletion or semantic note deletion.

`paragraphs.move(index, destination)` reorders one ordinary DOCX paragraph within the same body section, header, footer, note or canonical body/header/footer table cell (including nested cells and merged-cell aliases). Both arguments are paragraph-collection indexes, including negative indexes; `destination` is the final paragraph index, not an insertion gap. Other native blocks keep their relative order. For `[paragraph A, table, paragraph B]`, moving paragraph B to index 0 produces `[B, A, table, required blank]`. The required native blank stays hidden and is reused by a later addition. The method returns `None`.

To insert new text earlier, append through the existing creator, then move it:

```python
report.add_paragraph("New first paragraph")
report.paragraphs.move(-1, 0)
```

Movement preserves paragraph/run properties and ordinary links/fields, existing object identities, descendant table/cell handles, table generations and media. Fresh collection reads use the new order; an existing iterator retains its captured handle order. A real move invalidates revision-bound ranges and cached renders; moving a supported paragraph to itself leaves bytes, ranges and caches unchanged. Copies remain independent. Both integer arguments are normalized under a captured document revision; a callback that mutates or closes the document prevents the pending move, without undoing the callback's own change.

Section-bearing source or destination paragraphs, cross-section/cross-container movement, aliased header/footer parts, PPTX paragraphs outside table cells, and moving paragraphs containing note/bookmark/comment anchors are explicitly unsupported. Ordinary paragraphs may move around protected anchors without moving those anchors relative to one another. Note content paragraphs can move; their body reference remains unchanged. Rich note tables/pictures and arbitrary imported DOCX editing are not enabled. Cell movement preserves the grid, neighboring cells, nested child handles and authored empty paragraphs. Only a native-created required tail is hidden; append consumes it. Moving a singleton to itself leaves bytes, revision, cached rendering and existing ranges unchanged. Paragraph-only indexes do not address every possible gap between tables or other blocks. Broader block adoption and semantic movement remain on the roadmap.


Header/footer content occupies a repeated native box and cannot flow onto another page. Table and picture frames extending outside the physical page fail before serialization, save, rendering or conversion, including after later content/font edits. Intentional exact-height cell clipping within a page remains supported. These checks use native placed geometry and do not guarantee that all text ink fits. Inactive first/even variants have no measured cell rectangles until a matching page exists; nested table/image insertion and geometry-dependent section changes can fail atomically with an explicit unsupported error. Ordinary top-level insertion is still available. Note stories retain their paragraph-only profile; rich note content and header/footer linkage and arbitrary block movement remain open.

`paragraph.add_field("title")` and `paragraph.add_field("author")` display the document's current core metadata through the same native field/range machinery. Update the values with `document.update_metadata(title="Report", author="Writer")`; setting a value to `None` or an empty string makes its field display empty. Body, table-cell, header/footer, footnote and endnote paragraphs use the same method. Copies retain independent metadata. These are native TITLE/AUTHOR fields with empty dirty cached results: PNG/PDF output evaluates them, while DOCX source extraction retains the empty cache and reports its existing `dirty_field_cached_result` notice. This does not refresh nonempty imported caches or add a general field evaluator.

Metadata fields are indivisible native text objects. They must fit the available native line box and physical page; turned or explicitly clipped table cells are outside this profile. Overflow is checked when saving, serializing, previewing or exporting, so a metadata/style/layout change may succeed and subsequently make output fail. Shorten the metadata or change the formatting/layout to recover. No output is published to a destination path on that failure. The check covers native line/advance boxes, not glyph-outline containment.

Metadata used by these fields must be a single line without tabs, carriage returns or Unicode line/paragraph separators. Repeated field values plus source text are conservatively limited to 1 MiB per field-containing paragraph and 16 MiB per document on every edit, including later metadata changes and copied stories. All field kinds share the existing 10,000-field cap. At output, actual repeated field placements also consume a 16 MiB display-byte limit and a two-million-unit item/line/cluster scan budget; repeated headers, footers and table headings receive no deduplication discount. These placement checks run after native layout and cannot cap allocations already performed by that layout.

`section.page_number_start` is `None` for continuation or an integer from 1 to 32767 for an explicit restart. New sections continue numbering until explicitly restarted. A continuous section cannot restart numbering; setting a restart there, or changing a restarted section to continuous, fails atomically because native layout ignores that combination. The section-page-count field is unavailable in continuous-section scopes because native counting omits a shared first page; ordinary page and total-page fields remain supported there. Up to 10,000 fields across all authored stories are supported. Arbitrary field instructions and nondecimal numbering are unavailable in this profile: native implicit section-format inheritance differs by story, and some numeral formats clamp larger values. Page/count fields do not constitute a general field-calculation or TOC API.

`section.line_numbers = (1, 1, 0, "each_page")` enables native decimal body-line labels; `None` disables them. The tuple is `(count_by, start, distance_points, restart)`: count_by is 1–100, start is 1–32768, and distance is 0–1584pt. Zero distance uses the native 18pt default; saved geometry has twip precision. Restart is `each_page`, `each_section` or `continuous`, matching note-numbering names. Newly appended sections inherit this setting. Intervals select divisible displayed values: start 5 with count_by 2 first displays 6.

Labels count ordinary top-level body lines, including some empty/break lines; tables, auxiliary stories and natively suppressed paragraphs are excluded. DOCX semantic text excludes the decorations, while native PNG/PDF output includes them. PDF extraction may treat aligned labels/body text as a table or reorder labels with different widths; this API does not replace native reading order.

Only single-column numbered sections are supported: the pinned renderer places multi-column labels at overlapping left-margin positions. After earlier numbering, `each_page` cannot resume in a continuous section unless the preceding section also uses `each_page`; the native counter can otherwise retain a value from an older page. Use a new page, `each_section` or `continuous` for that transition. Invalid combinations and later incompatible edits fail atomically. Numbered body text has a conservative 100,000-unit label-work budget: four units per UTF-8 byte plus one per paragraph and inline object, including suppressed paragraphs. Output also checks actual native label bounds and rejects labels outside the physical page or wrapped by formatting. These are label-work/logical-geometry limits, not total process-memory or glyph-ink guarantees.

`document.add_image(source, width=None, height=None, alt="")` and `cell.add_image(...)` accept PNG bytes, a path or a binary stream and return a picture with writable `size`, `alt`, `data`, `crop`, `flip_h` and `flip_v`. A single requested dimension preserves aspect ratio on insertion. Reading `picture.data` returns owned PNG bytes; assigning another source replaces the payload while preserving its existing size and alt text. Change `size` explicitly when the replacement should use another aspect ratio. PNG format, dimensions and decoded size are checked before mutation. DOCX authoring currently accepts PNG only, unlike the broader extraction surface. Image frame dimensions must each be 1–1584 points and fit the native container; sub-point frames are rejected because the native inline layout clamps them to one point.

`picture.crop=(left, top, right, bottom)` uses finite fractions in 0..0.45; `None` resets the crop. `flip_h` and `flip_v` require booleans. The uncropped display frame remains `size`, original PNG bytes remain in `data`, and replacing the source preserves these transforms. These properties use native drawing transforms in saved DOCX, PNG and PDF output. Rotation is not exposed for DOCX pictures: a native quarter-turn fixture loses edge strips, so a local image-resampling workaround is not used.

Direct body pictures also reuse `picture.format(...)` and the owned `picture.formatting` snapshot for native placement:

```python
picture = document.add_image(logo_png, width=80, height=40)
picture.format(wrap="behind", anchor="page", position=(90, 120), distance=4)
# Existing source bytes, size, crop and flips remain attached to this placement.
picture.format(wrap="square", distance=(2, 4, 6, 4))
picture.format(wrap="inline")  # restore ordinary flow and clear placement offsets
```

`wrap` is `inline`, `square`, `top_bottom` or `behind`; `anchor` is `page`, `margin` or `paragraph`. Page/margin offsets use that coordinate frame on the anchor paragraph's laid-out page. They do not pin a picture to a permanent physical page number or repeat it across pages. Paragraph offsets follow the anchor paragraph, with horizontal position relative to its column and vertical position relative to its start above before-spacing. `position=(x,y)` uses points in −31680..31680, in 0.05-point increments. `distance` is a scalar or `(top,right,bottom,left)` points in 0..1584, also in 0.05-point increments; left and right must match because the native model stores one horizontal distance. A snapshot returns canonical tuples. Set a floating wrap before or together with placement values; `inline` resets anchor/position/distance while preserving media, crop and flips.

Formatting preflights actual native image rectangles and rejects pictures outside the physical page atomically. Saving/rendering/exporting checks again after later text or page edits; restore fitting geometry to recover. The existing image width bound still applies to its native section/column. Floating cell/header/footer/note pictures, front layering, contour wrapping, arbitrary alignment, z-order, overlap policy, rotation and opacity remain unavailable. The pinned writer fixes `layoutInCell=1` and `allowOverlap=1`; those are not configurable guarantees. Front placement is intentionally rejected: native PNG and exported PDF paint later paragraph text over an earlier `InFrontOfText` image. See the [upstream fidelity evidence](upstream-roadmap.md#docx-floating-picture-placement).

Use `del document.images[index]` or `del cell.images[index]` to remove one picture placement. Its paragraph remains as an empty editable anchor, preserving unrelated paragraph identities and formatting. Deleted handles fail explicitly. Shared media is deduplicated by the native document API; removing/replacing a placement keeps bytes referenced elsewhere and discards media only when no authored story references it. Table/cell content deletion also releases unreferenced media. Up to 1,000 placements and 64 MiB of unique referenced encoded media are allowed, with the existing per-image decode limits. The cleanup profile covers authored images across body, nested tables and auxiliary stories; arbitrary imported graphics, OLE and page pictures need a broader upstream reference API before they can share this path.

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

`document.styles` iterates paragraph/character/table styles in native order and supports integer/slice access or lookup by ID/name. `styles.add(name, kind="paragraph", based_on=None)` returns a style with immutable `id`, `name`, `kind`, `builtin`, writable `based_on`, `next`, `linked`, and `format(**properties)`. IDs and display names are distinct. Passing a Style handle from another document is rejected. Table styles use the same collection and formatting methods, with conditional regions described below. Theme changes and style deletion/import remain open. Theme palette/font metadata alone does not establish live theme references: the pinned DOCX reader resolves ordinary theme-bound properties to fixed colors/font names. Existing style/default formatting is the supported way to update inheriting fixed formatting; [STY3 requires native theme identity and resolution](upstream-roadmap.md#owned-docx-theme-identity).

`style.formatting` returns direct values; `None` indicates inheritance. `style.resolved_formatting` asks the upstream resolver for effective properties. `based_on` returns another Style or `None`. Duplicate/ambiguous names, wrong-kind or missing bases, inheritance cycles and chains exceeding 32 styles fail before mutation. A document accepts at most 1,024 named styles.

Formatting follows one rule across `add_run`, range/paragraph `format`, and style `format`: omitted properties stay unchanged, `None` removes a direct override, and `False` explicitly disables a boolean property. New runs with omitted bold/italic inherit. `font_size` always means points. A successful range edit refreshes that range; other captured ranges may become stale after any document mutation.

Character keys are `bold`, `italic`, `font_size`, `font`, `language`, `letter_spacing`, `color`, `underline`, `strike`, `script`, `highlight`, `background`, and a character `style` ID/name. `color="auto"` uses native automatic color; `background` is RGB shading. Underline accepts bool or `none`, `single`, `words`, `double`, `thick`, `dotted`, `dash`, `dot_dash`, `dot_dot_dash`, `wave`, `double_wave`. Strike accepts bool or `none`, `single`, `double`; script accepts `baseline`, `superscript`, `subscript`. Highlight uses WordCraft's fixed palette (`yellow`, `bright_green`, `turquoise`, `pink`, `blue`, `red`, `dark_blue`, `teal`, `green`, `violet`, `dark_red`, `dark_yellow`, `gray50`, `gray25`, `black`, `white`, `none`). Unsupported names fail explicitly.

DOCX `paragraph.add_run(..., letter_spacing=0.3)`, `range.format(letter_spacing=-0.05)` and the existing style/default character-format paths adjust native shaped-cluster advances in points. Paragraph/character styles and whole-table/conditional character styles share this property; paragraph before/after/line spacing remains separate. Values are finite -100 through 100 points in 0.05-point increments (one native DOCX twip). Ordinary floating-point arithmetic such as `0.1 + 0.2` is accepted and normalized to 0.3; finer precision such as 0.301 fails. Direct/effective snapshots return canonical point values. Omission leaves the property unchanged, 0 explicitly cancels inherited spacing and `None` restores inheritance.

Positive spacing expands and negative spacing condenses; large negative values intentionally overlap or reverse visible glyph positions. Native shaping owns cluster boundaries: this is not per-code-point repositioning, and source Unicode remains unchanged. PDF reading-order/space inference can differ from source text when glyphs overlap or spread apart, even though tested source/PDF ink agrees. Existing navigation/geometry bounds still apply. PPTX shape/cell ranges and `add_run` support the same bounded `letter_spacing` key; see the slide profile below. No kerning, horizontal scaling, hidden-text or general typography API is implied.

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

PDF extraction exposes the native display label in `page.metadata["label"]`; `page.number` remains the zero-based physical index. Full editable copies preserve labels. Native page movement keeps label ranges at document positions rather than attaching labels to moved content; deletion adjusts the ranges and continues numbering. Page subsets and imports do not copy the source document's label tree. No label writer or alias is exposed. Invalid/oversized native label trees can fall back to physical numbering; label readback is not a complete preservation report.

The import-safe [label assembly reproducer](../tools/reproduce_pdf_page_label_assembly.py) uses distinct page widths to show these behaviors with Roman and prefixed decimal labels. Formatting/range adjustment remains in the pinned [native label implementation](https://github.com/storytold/pdfcraft/blob/645016addcfb9078c557f0189559ab47cdcee057/crates/organize/src/labels.rs); the script only constructs synthetic test input.

### PDF annotations

`page.annotations` is an ordered collection. `add("note" | "highlight" | "rectangle" | "ellipse", box=..., text="", author="", color="#FFD200")` and `add_link(box=..., uri=... | page=...)` return checked annotation handles. A link needs exactly one target; page targets are zero-based. Handles expose `kind`, `box`, `text`, `author`, `color`, `uri` and `target_page`; `update(...)` applies a compound mutation atomically. `del page.annotations[index]` invalidates previously retrieved annotation handles on that page, including those retrieved through another page wrapper.

Notes use the upstream fixed 20×20 PDF-user-unit icon size. Notes and highlights can be moved but not resized; links, rectangles and ellipses can be resized. Unsupported type-specific edits fail before changing the document. Readback can include other native annotation kinds, but mutation is limited to the documented profile. A highlight or comment is not secure redaction. Extracted internal links use `#page=N` with a one-based fragment and zero-based `metadata.target_page`.


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

`slide.add_shape("rectangle" | "ellipse" | "line", box=..., fill=None, stroke=None, stroke_width=1)` returns a shape with writable `rotation`, `fill` and `stroke=(color, width)` properties. `None` removes a fill or stroke. Line boxes can have one zero dimension. `slide.shapes` lists typed handles in stacking order, supports `move(index, destination)` and deletion, and preserves surviving identities. A PNG `slide.background_image` uses the native picture fill; reading solid `background` for an image fill raises an explicit unsupported error. Setting a solid background replaces the image fill and removes unreferenced owned media. Assign `slide.background = None` to remove the explicit solid or picture override and restore native layout/master/theme inheritance. The getter reports the explicit color only: `None` means no override, not white or transparent, even if an inherited image or gradient is rendered. Reset preserves shared picture/other-slide media and reclaims an unreferenced owned background image. It uses normal revision/cache invalidation, including repeated resets. Imported master/layout/theme editing and general inherited-media reclamation remain outside the authored profile.

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

`gutter` adds a binding inset in points, reducing usable width; it is on the left by default and on the inside when document-wide mirrored margins are enabled. It accepts 0–1584 points subject to the page's remaining content area. Set `column_widths` to a tuple of explicit point widths matching the current column count; widths are 18–1584 points each, share the gap in `columns`, and must fit the usable width together with those gaps. Unused trailing space is allowed. The native layout positions and flows text; widths are never automatically scaled. `column_widths=None` restores equal columns. Assigning `columns` also clears explicit widths. Page dimensions determine orientation; top gutters and independent per-column gaps are not exposed.

```python
section = document.sections[0]
section.gutter = doc_ex.mm(5)
section.columns = (2, 18, True)
section.column_widths = (180, 240)
```

A newly appended section copies its predecessor's setup and receives independent header/footer parts, including independent empty parts for missing variants. Editing an earlier header later cannot leak into the new section. This initial API deliberately has no linked-to-previous switch. Body tables and images must fit the owning section's narrowest column, while header/footer content uses that section's full content width. This conservative limit protects tables continuing from a wider column: the pinned layout measures a table once and does not reflow its grid for narrower subsequent columns. New body tables in unequal-column sections start with fixed width equal to the narrowest column. Existing tables are not silently reformatted: percentage/autofit tables can be rejected even when they initially fit a wider column. Serialization rechecks unequal-section table geometry because later text/font edits can move a table between columns. Existing nested cell limits also apply. Changing page size, margins, gutter or columns validates existing children atomically.

`next_column` is rejected because the pinned layout path treats that transition as a new page. Continuous transitions with differing page sizes are rejected before mutation rather than promising geometry the upstream layout cannot represent. Odd/even starts, leading blank sections and mixed page dimensions have native PNG/PDF regression coverage.

### PPTX character spacing

Existing `shape.add_run(..., letter_spacing=0.3)`, `shape[:].format(letter_spacing=-0.05)` and corresponding cell paragraph operations use the same -100..100 point, 0.05-point profile as DOCX. Although DrawingML supports hundredths of a point, this initial shared profile rejects finer values such as 0.01. `None` removes the direct property and restores native paragraph/placeholder inheritance; explicit 0 cancels inherited spacing. Appending without the key retains the existing adjacent-run formatting behavior. No named Word style/default API or direct/effective run-property getter is added for slides.

DeckCraft owns grapheme tracking, wrapping and autofit. Autofit scales spacing with font size, and table rows keep their existing native growth behavior. Negative spacing can deliberately overlap glyphs; source text stays unchanged while geometric PDF text inference may differ. Native PDF export rasterizes slides at 200 DPI and adds an invisible text layer. The acceptance fixtures verify both visible output and text-layer glyph displacements, not a general arbitrary-DPI pixel identity guarantee. The existing revision, cell-generation, resource and link checks remain in place.

### Rich PPTX cell paragraphs

PPTX cells reuse `cell.paragraphs`, `cell.add_paragraph(text)` and ordinary `Paragraph`/`TextRange` operations. Edit a paragraph's `.text`, `.alignment` or `.format(alignment=...)`; append styled text with `.add_run`, select with `[start:stop]`, or use `.find`/`.replace_text`. Range `.format`, `.link` and `.link_tooltip` follow the existing slide rules, including same-presentation Slide targets. `.formatting` reports direct alignment and the six nullable spacing/indentation values described below. Named Word styles, lists, fields, page/column breaks, tight or exact line spacing, effective Word style inspection, nested tables and cell images remain explicitly unsupported.

`del cell.paragraphs[index]` removes one PPTX cell paragraph, with normal negative indexes and merged-cell aliases. Surviving paragraph formatting and links remain intact. All previously held paragraph handles in that canonical cell become stale; retrieve fresh handles from `cell.paragraphs`. The Cell and table handles, and paragraph handles in other cells, remain usable. Existing ranges follow ordinary document revision invalidation. A callback during index conversion that changes the presentation rejects the outer deletion.

`cell.paragraphs.move(index, destination)` moves one intact paragraph to its final index within that same canonical PPTX cell. Negative indexes and merged aliases follow ordinary collection rules; it returns `None`. Paragraph/run/default/end formatting, links and authored empty paragraphs move together. A changed order makes all captured paragraph handles in that cell stale, including captured iterator handles; retrieve fresh handles afterward. Cell/table handles and paragraph handles in other cells remain live, while revision-bound ranges become stale. This differs from DOCX movement, which retains stable paragraph identities.

A validated move to the same index preserves bytes, revision, cached renders and existing handles/ranges. Source and destination are checked under one captured revision; index callbacks that mutate or close the owner prevent the pending movement without undoing the callback. Stale grid handles reject before index callbacks. Reordering does not add or delete a native required blank or change live content/media budgets. Copies remain independent. Cross-cell adoption and general slide-shape paragraph collections are not enabled. Existing native layout/overflow behavior remains unchanged; this method does not add a fit-to-cell guarantee.

Deleting the last owned paragraph leaves an empty collection. The native writer emits its required blank when saving; reopening therefore exposes a blank paragraph in the serialized content. `cell.add_paragraph("New")` initializes the empty owned collection without a leading separator. No placeholder API is needed. Copies are independent and old paragraph handles never revive after append. Slice deletion and general shape-paragraph collections remain unavailable.

```python
cell = table[0, 0]
paragraph = cell.paragraphs[0]
paragraph.text = "Delivery: "
paragraph.add_run("Complete", bold=True, color="green")
terms = cell.add_paragraph("Read the terms")
terms.find("terms")[0].link = "https://example.com/terms"
cell.paragraphs.move(-1, 0)
```

Each handle addresses one paragraph. LF is rejected in paragraph text/range replacement; use `cell.add_paragraph` for a new paragraph. Setting paragraph text to an empty string retains the paragraph. Adding a paragraph preserves earlier paragraph handles; ranges still become stale after unrelated document changes. Replacing `cell.text` invalidates that cell's old paragraph handles, and structural grid operations invalidate descendants. Covered coordinates share their canonical merged cell. Ordinary newly created/inserted cells contain an empty paragraph; splitting can expose a cell with no paragraphs, where `add_paragraph` creates the first one directly. Slide reordering and independent document copies retain their existing ownership rules.

`paragraph.add_break("line")` (or `.add_break()`) appends a native line break without creating another paragraph and returns the existing revision-bound `TextRange`. It inherits adjacent character appearance, clearing navigation on the inserted break; its returned range can be formatted, linked, replaced or deleted normally. Page/column breaks remain unsupported in PPTX. An empty paragraph accepts a break; an empty paragraph collection still requires `cell.add_paragraph()` first.

Owned PPTX cell paragraph/range text exposes each native break as one LF character, matching DOCX. Character positions remain one-to-one, including non-BMP and combining text. `paragraph.find("\n")` selects breaks and `paragraph.replace_text("\n", " ")` replaces them with ordinary text. Generic paragraph/range setters and `add_run` still reject raw LF and VT; reading a string containing a break does not make that string valid replacement input. Whole `cell.text` and `table.rows` are flattened readable views: paragraph separators and explicit line breaks both appear as LF, so setting `cell.text = cell.text` does not preserve that structural distinction. Native stored markers remain unchanged; general shape and note text behavior is unchanged.

Appending a break keeps paragraph/cell/table handles live and invalidates prior revision-bound ranges. Merged aliases address the same canonical cell. Stale or closed owners and invalid kinds reject atomically, preserving content, counters and cached output. Source text, run and property limits are checked before insertion; this adds no new fit-to-cell or overflow guarantee.

PPTX cell paragraphs also accept direct spacing and indentation through `paragraph.format(...)`:

```python
paragraph.format(indent_left=24, indent_right=12, indent_first=-12,
                 space_before=6, space_after=8, line_spacing=1.5)
settings = paragraph.formatting
paragraph.format(**settings)
paragraph.format(space_before=None, line_spacing=None)  # Restore native inheritance.
```

Indentation and before/after spacing use points. Left/right indentation accepts `0..4032`, first-line indentation accepts `-4032..4032`, and before/after spacing accepts `0..1584`. `line_spacing` is a multiplier in `1..100`; `None` clears any of these direct overrides. Snapshots preserve explicit zero versus `None` and do not resolve inheritance. The native writer rounds indentation to 1/12700 point, before/after spacing to 0.01 point and line multipliers to 1/100000; direct snapshots retain the supplied value until serialization. Native body settings normally suppress spacing before the first paragraph and after the last paragraph. Negative first-line indentation is hanging indentation; the native renderer clamps a first-line origin outside the cell's left text origin.

Multipliers below one are not admitted: native output can overlap lines and normalized PDF extraction can discard repeated overlapping text even though the native PDF text operators retain it. Native point-valued line spacing and percentage before/after spacing are not projected into misleading units. General shape-paragraph formatting, bullets, exact line-height controls and resolved snapshots are outside this cell-only extension. `paragraph.spacing` remains unavailable for PPTX. Formatting keeps paragraph/cell handles live, invalidates revision-bound ranges, and applies compound edits atomically; invalid values and callbacks that change or close the owner cannot apply a pending patch.

DeckCraft fits ordinary horizontal unmerged rows to content, which can draw below the stored table box. Merged-row and vertical cells keep native stored-height behavior. This API does not promise automatic font shrinking, page containment or a rewritten nominal table size; inspect rendered output for overflowing content.

### PPTX cell tab stops

Use the existing paragraph formatting method with ordinary tab characters in text:

```python
paragraph.text = "Item\t12.50"
paragraph.format(tabs=[{"position": 144, "alignment": "decimal"}])
paragraph.format(**paragraph.formatting)
paragraph.format(tabs=[])    # Explicitly use no custom stops.
paragraph.format(tabs=None)  # Restore native inheritance.
```

Positions are points, finite and within `-2147483647..2147483647`. Supply at most 32 stops in strictly increasing order; positions must remain distinct after the native writer rounds them to 1/12700 point. The snapshot retains the supplied direct values. Alignment is `left` (the default), `center`, `right` or `decimal`; a decimal-aligned segment without a decimal separator aligns its end. Leaders are unsupported: omit `leader` or use `"none"`. Horizontal cells are the currently constructible owned public profile: there is no public PPTX cell orientation or paragraph-direction setter. This change does not add imported editing, vertical-cell or bidi formatting controls, or claim a new native vertical-state refusal.

`None` inherits and `[]` overrides an inherited custom list. The native layout can skip zero/negative stops and uses its 72-point default pitch after the last applicable custom stop. Large positions can wrap text onto additional lines; these scalar bounds do not promise that content fits a cell or page. The existing finite-cell overflow behavior and render/resource limits still apply. Literal HT remains one character in paragraph/range text; existing explicit line breaks and their LF selection semantics are unchanged.

Source PPTX text and links retain their exact labels and offsets. PDF export retains meaningful label order in its native text operators and positions the labels through the native layout. PdfCraft's normalized PDF reading order can treat wide tab gaps as columns, including with existing default tabs; normalized PDF paragraph order is not promised to equal the source paragraph order. This behavior is distinct from text being omitted during export.

Tab sources inherited from list styles, masters, layouts and presentation defaults receive the same 32-stop validation and bounded traversal/property accounting before copying, mutation or serialization. Compound invalid edits and reentrant scalar conversions leave the pending patch unapplied. Paragraph/cell handles remain live after formatting; existing revision-bound ranges become stale.

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

`note.clear()` removes all authored note paragraphs and their formatting while retaining the note's reference, kind, ordering and section numbering/placement options. The native writer keeps its own number mark; an empty note still occupies native layout space. The synthetic empty paragraph uses the native FootnoteText/EndnoteText style and is hidden from `note.paragraphs`. `note.add_paragraph(...)` reuses it with a fresh child identifier, without an extra blank paragraph. Clearing an explicitly authored empty paragraph is a real mutation; repeated clears, including after note-kind conversion, preserve existing ranges and cached renders.

A real clear invalidates removed paragraph handles and all existing document ranges under the usual revision contract. The Note handle and unrelated paragraph/table/picture handles remain usable; copies are independent. Supported fields and outgoing links disappear with their content, but protected note/bookmark/comment anchors cannot be removed through this operation. Clearing content does not delete the note/reference, enable imported note editing or enable rich note table/picture insertion.

`document.notes.convert(kind="footnote" | "endnote")` converts all owned notes to the selected kind and returns the number changed. It retains note/paragraph handles, reference identities, content and custom style names; native standard note text/reference styles switch to their destination counterparts. Destination sections supply numbering and placement through the existing note options. Copies remain independent. An empty/already-matching collection leaves ranges and cached renders valid; an actual conversion invalidates existing ranges normally.

Conversion is atomic and uses native WordCraft semantics. It rejects a shared standard-style run if changing a reference style would also restyle ordinary text, another semantic object or an unconverted note marker. Apply a custom character style or separate formatting through existing ranges first; no local run splitting is performed. Runs containing only converted note markers remain supported. Existing field/navigation/geometry guards still apply, including unsafe section-page counts after moving an endnote. A two-million-unit paragraph/object/run-comparison budget bounds the native reference/run scan, in addition to existing document limits. This does not enable note deletion or fragment adoption.

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

`document.fields` indexes fields by immutable name and iterates checked field handles; `page.fields` filters that collection and creates a widget or same-page radio group. Fields expose `name`, `kind`, `value`, `options`, `page`, `box` and immutable owned `boxes`. Text values are strings, checkbox values are booleans, and choice values are one declared option or `None` to clear selection. `del document.fields[name]` removes the whole field and all its widgets. Structural field edits and page organization invalidate existing field handles; retrieve them again by name. Value and box changes retain handles. Assign `field.box=(x, y, width, height)` to move or resize its existing widget on the same page in displayed points. The box must fit the current page view and leave room for the fixed appearance, current value and all choice options. Invalid geometry or overflow fails atomically. Native widget identity, field values and options remain intact. Whole-document copies are independent.

This creation profile supports single-widget text/checkbox/choice fields and bounded same-page radio groups in PDFs created by this owner or its copies. Imported PDFs cannot acquire fields; imported AcroForms remain explicitly unsupported for editing. Text/choice appearances use fixed 12-point Helvetica with WinAnsi characters and conservative fit validation. Newlines, missing glyphs, overflow, duplicate names/options and unsupported kinds fail atomically. Names cannot contain periods, controls or surrounding whitespace. Limits are independently 1,000 logical fields and 1,000 total widgets, 256 UTF-8 bytes per name, 256 choice options, 4 KiB per option/value and 4 MiB aggregate field text. Radio groups have the stricter profile below.

`document.fields.flatten()` turns all supported field appearances into page content and returns the number of logical fields flattened (a radio group counts once). It separately verifies the native drawn-widget count and that no terminal fields/widgets remain before committing. The native writer may retain an empty AcroForm dictionary, so editable reopening can still reject a flattened file; ordinary extraction and rendering remain supported. Subsetting a PDF with active fields is unavailable; flatten or delete them first. Arbitrary multi-page/mutable widget groups, multiline text, autosizing, custom fonts, scripts, signatures and imported-field inspection remain open work.

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

Cell `vertical_alignment` and paragraph `alignment` act on different axes. With horizontal text, vertical alignment positions the block down the cell. With `text_direction="down"` or `"up"`, native layout quarter-turns the lines: paragraph alignment runs along that writing direction, while vertical alignment moves the lines across the cell. A right-to-left table mirrors columns and logical start/end padding; paragraph `direction="rtl"` independently changes the inline start edge. Empty cells preserve their formatting and visible geometry. The [combined cell-layout corpus](table-corpus.md#cell-padding-and-alignment-matrix) checks these interactions in saved DOCX and native/PDF renders at 72/144/300 DPI. These quarter turns do not establish full East Asian vertical-script fidelity. Inter-cell spacing remains unavailable and is distinct from cell margins.

Border mappings address `top`, `right`, `bottom`, `left`, and table-only `inside_horizontal`/`inside_vertical`. A line mapping contains native `style`, `color` and `width` values; `color=None` retains the native automatic color. Supported styles are `none`, `single`, `dotted`, `dashed`, `thick`, and horizontal `double`/`wave`; widths use eighth-point steps from 0.25–12 points, with at least 1.5pt for `thick`. Native-substituted styles and unsupported vertical double/wave edges fail explicitly. A side set to `None` inherits, while `style="none"` suppresses that side. `format(borders=None)` clears direct borders; the pre-existing `table.borders = None` convenience explicitly hides them. Adjacent conflicting borders follow pinned native rendering, which does not implement full Word shared-edge conflict resolution. Conditional table styles also reject interior vertical edges that the pinned native layout drops: row-region `inside_vertical`; `right` in first-column and left-corner regions; `left` in last-column and right-corner regions; and `left`/`right` in column bands. This includes explicit `style="none"` suppression. Side/group `None` resets remain available, and whole-table/direct-table/direct-cell borders are unaffected. Conditional styles can later be applied to wider tables, so this restriction also applies when a current one-column table would place the edge outside.

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

All regions accept supported character properties, cell `background`, paragraph `alignment`, `vertical_alignment` and the existing border mapping subject to the conditional interior-vertical restrictions above. Whole-table defaults additionally accept supported paragraph properties, `margins` and `row_band_size`. Conditional paragraph spacing/indentation/keep properties are unsupported. `None` clears a direct override and `False` explicitly disables boolean formatting. Table-style backgrounds are cell fills, distinct from run shading.

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

`document.styles.defaults` returns direct native document character/paragraph defaults. `styles.format_defaults(...)` accepts the existing character and paragraph properties, including tabs; omission preserves a property, `None` clears a direct default, including ordinary font size, and `False` explicitly disables a boolean. Resetting the ordinary document default font size removes its direct value and restores native inheritance, with an 11pt fallback when no ordinary size is inherited. This differs from initializing a new document with 12pt defaults. Explicit complex-script, named-style and direct run sizes remain intact. Background means paragraph shading here. Table-region, table-only and style-assignment properties are unsupported. Named styles and direct formatting keep their normal precedence over defaults; resolved style/paragraph snapshots use upstream resolution. Default changes validate affected table geometry and effective tab limits before committing, invalidate cached rendering and preserve independent copies. Arbitrary default-style selection through the native default-kind flag remains unavailable because the pinned resolver's fallback is `Normal`.

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

Both `page.draw_text(..., behind=True)` and `page.add_image(..., behind=True)` prepend typed content beneath everything already on the page. `behind` must be a boolean and defaults to `False`, preserving overlay behavior. Built-in text supports the same point/box wrapping and alignment; images retain the PNG/JPEG and transform contract above. For example, `page.draw_text("DRAFT", at=(40, 100), font_size=36, color="gray", behind=True)` places a text underlay.

Adding underlay A then B paints B, A, original. An opaque original page background or image may hide the result entirely; hidden source text can still be extracted. This does not add a PDF background-reset operation. Caller-supplied font text cannot use `behind=True` and refuses before reading the font source. `draw_path` remains overlay-only. Existing resource-name collisions still fail atomically, leaving content and cached output unchanged; broader raw-content layering is unavailable.

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
    title="Quarterly report", author="Reporting team", language="en-GB", revision=2,
    modified="2026-10-20T12:00:00Z",
    custom={"Approved": False, "Quarter": 4, "Ratio": 1.25},
)
report.update_metadata(custom={"Approved": True, "Ratio": None})
```

Core keys are `title`, `author`, `subject`, `keywords`, `description`, `category`, `last_modified_by`, `created`, `modified` and `revision`. Strings use XML-safe text without surrounding whitespace because the native reader trims it. Empty strings or `None` clear core strings; revision is an unsigned 32-bit integer, with zero/`None` clearing it. Created/modified accept valid UTC `YYYY-MM-DDTHH:MM:SSZ` timestamps, years 0001–9999. Saving leaves caller timestamps and revision unchanged. The dictionary omits cleared core values and always includes `custom`.

Custom values support strings, booleans, signed 64-bit integers and finite floats, preserving their native types. A custom dictionary patches entries: omitted names stay unchanged, per-entry `None` removes one, and `custom=None` clears all. Names use native ASCII case-insensitive identity while retaining their original spelling; ambiguous batch names fail. Custom string whitespace is preserved. Nested objects, arbitrary XML, vectors and custom date wrappers are unavailable. Names are 1–255 UTF-8 bytes with no control characters or surrounding whitespace; values are limited to 64 KiB, with at most 1,024 properties. Incoming JSON, stored core values and stored custom values each have a 256 KiB serialized budget. Compound updates fail atomically, retaining existing ranges and cached output on rejection.

Version 0.3.0 accepted a document-level `language` core key. Version 0.3.1 removed it: WordCraft's core properties have no `dc:language`, so `update_metadata(language=...)` raises `ValueError("unknown metadata key: language")` and leaves the document unchanged instead of dropping the value when the document is saved. Run, paragraph, style and default languages (`language=` on `add_run`, `format` and `styles.format_defaults`) are separate and unaffected. Core strings are limited to 64 KiB of UTF-8 within the shared core budget.

Ordinary DOCX extraction exposes the supported core properties. Imported custom-property completeness and extended/application property editing remain DOC7 gaps. Metadata editing uses native CoreProps/CustomProp and the normal document writer; it does not add arbitrary source DOCX editing.

### PDF rectangle and ellipse comments

`page.annotations.add("rectangle" | "ellipse", box=..., text="", author="", color=...)` creates a native solid, unfilled, opaque outline. It reuses annotation handles, writable `box`, color/comment metadata, atomic `update(...)`, deletion and copies. Comment text is metadata, not visible FreeText. Page text/images remain unchanged. Coordinate boxes use current displayed-page points, including rotation, CropBox and UserUnit.

The native outline is two PDF user units wide; both shape dimensions must be at least three PDF user units. For UserUnit 2, the minimum displayed width/height is six points. Smaller paths can disappear in the pinned renderer, so they are rejected instead of silently enlarged. No arbitrary-low-DPI visibility guarantee is implied. Only this fixed stroke profile is supported; fill and decorative borders remain unavailable. Native opacity is supported through the existing comment property described below.

Imported shape editing requires a bounded canonical dictionary: direct four-number rectangle and RGB color, a solid two-unit border, optional finite alpha between zero and one and native printable flags, bounded plain metadata, and at most an indirect normal appearance stream with no rollover/down states. Insets, cloud/dash/measurement styling, replies/actions, complex or unknown extensions fail explicitly before native target appearance rebuilding. Metadata-only edits retain the appearance; geometry/color edits regenerate the standard native appearance and do not preserve arbitrary custom artwork. Deletion remains available when editing is rejected.

Generated appearance work is preflighted and actual staged stream bytes are charged, including both streams when a compound resize/recolor creates an intermediate appearance. These checks bound the target mutation; existing global imported annotation summaries/initial inspection can allocate before their output caps and need a separate upstream bounded-inventory API. The [upstream register](upstream-roadmap.md#bounded-pdf-annotation-inventory) records that limitation. Initial closed/stale checks and final checks after argument normalization prevent callback-driven edits from targeting another page.

### PDF text-markup annotations

`page.annotations.add(kind, box=..., text="", author="", color=...)` also accepts `underline`, `strikeout` and `squiggly`, alongside `note` and `highlight`. Existing annotation properties and `update` move the markup, change text/author/color, and preserve searchable page text. Deletion remains `del page.annotations[index]`. Boxes use current displayed-page points; native quadrilaterals preserve rotation, CropBox and UserUnit geometry. Each new mark uses one quadrilateral. Movement is supported; resizing and multi-quad authoring remain unavailable.

Native appearance generation is preflighted before potentially allocating it, including when updating imported highlights. Direct quadrilateral arrays are limited to 1,024 quads with finite coordinates within ±1,000,000 PDF user units (the editable page-box coordinate limit), plus a conservative work estimate within the remaining document-input budget; generated appearance bytes are charged after successful staging. Squiggly's native 10,000-segment cap coarsens the whole line rather than truncating its extent. Unsupported/malformed/over-budget updates leave bytes and cached renders unchanged. Deletion reads the subtype without rebuilding appearances, so an over-budget appearance does not prevent removal. This profile does not establish complete imported-form, reply-thread or arbitrary annotation appearance support.


### DOCX named ranges and internal links

`word.bookmarks` maps names to existing `TextRange` values. Assign a fresh range to create or move a target; lookup returns a fresh range, and iteration yields names in document order. PDF `bookmarks` remain outline entries with their existing interface.

```python
word = doc_ex.new("docx")
target = word.add_paragraph("Summary")
word.bookmarks["summary"] = target[:]
label = word.add_paragraph("Read summary")
label[:].link = "#summary"
selected = word.bookmarks["summary"]
assert selected.text == "Summary"
```

Names are case-sensitive ASCII identifiers matching `[A-Za-z][A-Za-z0-9_]{0,39}`, with at most 1,000 targets. Targets cover a whole body paragraph, including supported horizontal nested table cells, or an empty point at its start/end. Interior ranges and interior points are rejected: the pinned native engine splits shaping at bookmark markers, changing ligatures and Arabic joining. Auxiliary-story, cross-paragraph, foreign-document and clipped/turned targets are unavailable. A selection cannot contain note/comment/other-bookmark or evaluated-field anchors. Moving a target may select its own existing marker pair, including movement between a whole paragraph and its endpoints, without changing links to its name.

Native markers remain U+FFFC characters in authored paragraph text and count in range offsets; native extraction/rendering hide them. Any mutation invalidates previous ranges. Use fresh lookup after grid changes or edits; names resolve current native paths, not saved offsets. Whole-document copies retain independent targets and links. Generic range/table deletion cannot destroy markers: clear all links to a target, then use `del word.bookmarks["summary"]` before removing its content. Links from headers, footers and notes count too. Replacing the entire target text is supported. Adding text outside its markers, or inserting text into an endpoint point that would make a marker interior, fails atomically; relocate or delete the bookmark first. Paragraph formatting remains available. Every staged mutation and output checks marker endpoints, discounting bookmark markers but retaining all other object offsets. There is no implicit rename or cross-reference field evaluation.

Adding the first bookmark activates stricter checks for all existing links. Internal-link assignment and bookmark creation/movement check native layout before committing. Serialization rechecks after later formatting/layout edits; unsupported output leaves an existing save destination intact, and resetting the unsupported formatting allows saving again. The current native PDF exporter loses vertical-cell links and can place clipped-cell annotations outside their clip. Linked clipped/turned/rotated/figure contexts are therefore rejected; linked figures are outside the audited profile, not asserted intrinsically defective. Targets require actual native carets within the physical page, without paragraph-start fallback.

At most 10,000 nonempty linked native glyph batches per page are allowed when navigation is active, counting repeated stories and external links too. This conservatively avoids the native annotation cap; coalescing can make the limit stricter than the final annotation count. Native layout/display generation happens before observed-result item/depth checks, so these checks are not a preallocation memory guarantee. Broader transformed graphics, tooltips, imported editing, heading-outline export and REF/PAGEREF evaluation remain open.


Bookmark validation and export preflight share a conservative 2,000,000-unit lookup/scan budget: indexed target pieces, native line/cluster lookup bounds, destination-page item scans, source-run scans for turned placements and observed display visits. Each internal linked glyph batch also pays its target lookup estimate because native export resolves that destination again; native annotation merging can only reduce the actual number. A long target paragraph with many links can therefore be rejected even below the per-page annotation cap. Reduce or split the target paragraph or reduce linked batches to recover. This bounds the additional observed lookup work; native layout/display allocation and the exporter's initial document-wide bookmark inventory remain separate.


### PDF object-stream output

PDF `to_bytes(*, object_streams=True)` and `save(destination, *, object_streams=True)` accept a strict boolean. The default preserves current native output. With `False`, the native writer writes objects directly and uses a classic cross-reference table, useful for inspection and consumers requiring that representation. Other formats retain their existing serialization signatures.

```python
pdf.save("classic-xref.pdf", object_streams=False)
data = pdf.to_bytes(object_streams=False)
```

This controls object packing only: image/content/font streams can remain compressed. It is not an optimizer, incremental save, PDF-version downgrade or PDF/A/PDF/UA profile, and it does not promise smaller files. The native writer can raise an older header version when object streams are enabled; disabling them does not remove newer source features. Both modes retain existing imported-document restrictions, owned-field checks and the same 256 MiB returned-output cap. The upstream writer allocates its complete byte vector before this cap, so it is not a peak-memory limit. Saving does not change model state or render caches; path replacement stays atomic and stream writes retain their documented short-write errors.


### PDF text search geometry

`page.search_for(text)` returns owned `(x0, y0, x1, y1)` tuples for native PDF text matches. It works on read pages and editable PDF pages through their current read snapshot. Coordinates use displayed page points with a top-left origin; rotation, CropBox and UserUnit are already applied. Multiline matches return one rectangle per native line, flattened in match order. The page must remain attached to a live owner, but returned tuples remain usable after closing it. DOCX/PPTX read pages raise UnsupportedFormatError; Office authored paragraph `find` remains a separate text-editing operation.

Native matching is literal, case-insensitive, whitespace-normalized and nonoverlapping. Turkish I variants and Greek sigma variants fold together; this is not full Unicode casefold (`Straße` does not match `STRASSE`). Distinct occurrences inside a ligature can share the same rectangle, and a character inside an ActualText replacement can return the complete replacement box. Search follows native text reading order, which can cross column boundaries and differ from normalized table/Markdown order. It does not return offsets into `get_text()`.

The rectangles describe glyph em/advance extents. Clipped, invisible and off-page text can match, including an existing OCR text layer; no OCR is run. Boxes are not clipped to the page or promised to tightly enclose visible ink. Regex, fuzzy matching and extra matching flags are not exposed.

Queries are limited to 4,096 UTF-8 bytes. Later search work is bounded to one million glyphs and conservative normalized scalar slots, four MiB of source glyph text, one million matched glyph visits, and 10,000 matches/rectangles. Excess work fails explicitly instead of truncating. These bounds apply after native text interpretation and do not promise a total parser/process-memory ceiling. Empty/ASCII-whitespace queries avoid interpretation and an editable snapshot; other Unicode whitespace is handled natively. Repeated ordinary searches reuse the parsed session but reinterpret text, without rasterizing or changing document content.


PDF page handles become stale after page organization. Drawing and page geometry changes check this again after reading caller streams or normalizing numeric/color arguments, so a callback that moves or deletes pages cannot redirect the requested operation. The callback's own changes remain; reacquire the page from `pdf.pages` before retrying. Callback exceptions propagate, and already-closed/stale handles fail before processing arguments.


Presentation slide and shape handles retain stable identities across reordering. Callback-capable argument normalization resolves the current slide position again before changing the target; a deleted shape/slide or closed presentation fails explicitly. Caller callback changes remain effective even when they invalidate the requested operation.


### Owned PDF field help text

`field.tooltip` reads or sets Unicode help metadata on an owned text, checkbox, choice or radio field. It defaults to `""`; assigning `""` clears it, while `None` is invalid. Whitespace and newlines are preserved. Tooltips do not change painted field values or add text to the page, and visible output remains unchanged within the tested owned profile.

Tooltips are limited to 4,096 UTF-8 bytes and reject NUL or malformed Unicode. They count toward the existing four MiB aggregate field-text budget. Native metadata updates regenerate the field appearance, so existing mutation and output limits still apply. Copies retain independent metadata; ordinary field structural invalidation rules remain. This does not enable imported forms, arbitrary widget editing or viewer-independent tooltip display.

### PDF comment editing locks

`annotation.locked` is a strict boolean property for notes, highlights, underlines, strikeouts, squiggles, rectangles and ellipses. Set it to `True` to inhibit native movement, resizing, color/opacity changes, author changes and deletion; set it to `False` to enable those edits again. Comment text remains editable while locked. Compound updates that also request a blocked change fail atomically with `DocExError`. Locking changes the native flag without redrawing the appearance or changing comment metadata, geometry or handle identity.

Links and other annotation kinds do not support this property. The setter accepts only canonical native-style comment dictionaries with bounded metadata/geometry and indirect page annotation entries. Imported replies, unknown nested properties, noncanonical flags/styles and unsupported appearance structures fail explicitly before mutation. Existing native annotation inventory and initial-inspection allocation limitations still apply; this target guard is not a general imported-PDF safety guarantee.

This is an annotation editing flag, not encryption or access control. Page/document operations are not prevented, and other software can clear the flag. Existing copies remain independent; page organization and annotation deletion keep their usual stale-handle rules. Values other than actual `True`/`False` raise `TypeError` without invoking truth-value callbacks.

### Owned PDF read-only fields

`field.read_only` is a strict boolean property for owned text, checkbox, choice and radio fields. It starts as `False`. Setting it to `True` disables native filling, including valid `field.value = ...` assignments through this Python API. Assigning the existing value or clearing a choice also fails with `DocExError` while read-only. Set `field.read_only = False` before filling again. Invalid types and values retain their existing earlier validation errors.

Read-only fields still allow tooltip changes, supported single-widget box changes, copies, deletion and flattening. Their flag survives supported page organization and serialization; normal stale-handle rules still apply. This property is distinct from a widget's Locked flag and is not encryption or access control. Imported AcroForms, arbitrary mutable/multi-page widget groups and additional field flags remain unsupported.

The native property setter regenerates appearances even when only this flag changes. Existing conservative appearance reservations and actual generated-byte accounting apply; raw appearance references may change, while tested values, metadata and visible output remain unchanged. Repeated assignments can allocate and invalidate cached snapshots. Non-boolean values raise `TypeError` without invoking truth-value callbacks.


### Field creation and caller callbacks

`page.fields.add(...)` validates the page before evaluating option iterators or coordinate conversions and again after normalization. If caller code closes the document or reorganizes its pages, the outer creation fails instead of adding a field at a reused page index. Caller exceptions propagate; callback side effects are not rolled back. Normal creation, including through a negative-index page handle, and independent document copies retain their existing behavior.


### Repeated PDF field writes

Owned field creation and updates retain cumulative authoring budgets. Text and choice assignments regenerate native appearances even when the value is unchanged or empty; those generated bytes and each successful fill count toward the document's existing budgets. Checkbox values reuse their state appearances but still consume a mutation item. Creation accounts both the native initial appearance and its final configured appearance.

Copies retain cumulative accounting, and deleting a field does not refund earlier generated streams that the native document still holds. A budget failure leaves the attempted outer mutation uncommitted. The appearance reservation and raw-byte charge are not a total process-memory limit; native parsing, cloning, compression and other intermediates have separate costs. Imported forms remain unsupported. Radio-specific cumulative costs are described below.


### Owned radio groups

```python
with doc_ex.new("pdf") as document:
    group = document.pages[0].fields.add(
        "delivery", kind="radio", options=("Mail", "Collect — 顧客"),
        boxes=((40, 40, 24, 24), (80, 40, 24, 24)),
    )
    group.value = "Collect — 顧客"
    group.tooltip = "Choose a delivery method"
    print(group.boxes)  # immutable owned tuples, one per option
    document.save("delivery.pdf")
```

Use `boxes` instead of singular `box` for 2–32 ordered, distinct options on one page. The group exposes the existing value/options/page/tooltip/read_only properties. Boxes use current top-left view points and must fit the page, span at least 12 native user units per dimension and stay within ±1,000,000 native coordinates. The appearance is the native fixed black circle with white background and black rectangular border; option strings are export identifiers, not visible captions. Add visible captions separately with existing drawing operations. Arbitrary widget styling and group reshaping are unavailable: `group.box` access or assignment raises `UnsupportedFormatError`; inspect `group.boxes` instead. Single-widget fields also expose `boxes`.

Options allow literal Unicode and `#`; native name helpers encode/decode them without aliasing a literal `#41` to `A`. Empty/whitespace-only strings, controls, literal `Off`, duplicates, invalid Unicode and options over 4,096 UTF-8 bytes fail. The four MiB aggregate uses canonical encoded native option/value lengths, which can be larger than literal UTF-8. Initial value is `None`; native NO_TOGGLE_TO_OFF prevents clearing an already selected group. Native read_only refuses programmatic same/different/None assignments. Unlocking, metadata updates, whole-group deletion, copies and flattening remain supported. These flags are not security controls.

Creation of N widgets performs N initial pairs plus N(N+1)/2 member redraws. The adapter reserves 64 KiB per generated pair before staging, charges that conservative allowance for each already-replaced initial pair and actual raw bytes for each final redraw, and consumes N(N+3)/2 mutation work units. Metadata redraws and value-state updates consume one work unit per member; complete value updates reuse existing AP streams. The existing 128 MiB cumulative history and 10,000 work-unit limits can be reached well before the structural 1,000-widget ceiling. Copy preserves history; deleting or flattening fields does not refund retained streams. This is bounded work/storage accounting, not a process-memory guarantee or an orphan-stream cleanup algorithm.


### PDF comment opacity

`annotation.opacity` and `annotation.update(opacity=...)` set native opacity from 0 (transparent) through 1 (opaque) for notes, highlights, underlines, strikeouts, squiggles, rectangles and ellipses. Creation defaults to 1; set the property after adding a comment. Color and opacity in one update use one native appearance regeneration. Highlights retain their native multiply blend mode. Opacity does not remove content or perform redaction.

The getter/setter use the same bounded canonical target profile as editing locks. Finite fractional `/CA` values are now supported within that profile; unrelated imported metadata/style restrictions remain. Links reject opacity. Nonfinite/out-of-range inputs fail instead of clamping; booleans, strings and `None` are invalid property values. `update(opacity=None)` leaves opacity unchanged, consistent with other optional update arguments. Locked comments refuse opacity changes. Native rendering, saved appearance streams and copies retain the value; style/opacity and geometry edits can replace custom imported appearances as documented above. Native note appearance regeneration draws the Comment icon as a bubble and other accepted icon names as a folded-page icon; imported note artwork is not preserved by these redraws.

Comment style redraws charge each generated normal appearance plus a conservative 512-byte resource allowance for each generation, including intermediate rectangle/ellipse resize appearances. Canonical note recoloring now uses the same bounded target guard and 64 KiB appearance-work reservation as note opacity, including subsequent color-only changes after fractional opacity. Repeated identical assignments still regenerate native appearances and consume the cumulative mutation budget. Initial and final handle checks prevent numeric-conversion callbacks from retargeting deleted or moved comments. This is a bounded target operation, not a new general imported inventory guarantee.


### Mirrored DOCX margins

`document.mirror_margins = True` enables the native document-wide setting; its default is `False`. It accepts an exact Python boolean. Each section keeps the existing `(top, right, bottom, left)` margin tuple: left plus gutter becomes the inside margin and right becomes outside. Inside is left on odd **displayed page numbers** and right on even displayed numbers. Existing section page-number restarts therefore affect the side selection; physical page indexes alone do not determine it. `False` restores ordinary left/right placement without changing the saved margin values.

```python
with doc_ex.new("docx", size=(300, 300)) as document:
    section = document.sections[0]
    section.margins = (48, 36, 36, 72)
    section.gutter = 8
    document.mirror_margins = True
    document.add_paragraph("Inside and outside margins follow native page numbering.")
    document.save("mirrored.docx")
```

Body text, supported tables, inline header/footer content and supported margin/paragraph-anchored pictures follow native placement. Page-relative floating pictures retain their page coordinates. Native extraction body boxes and source-selection rectangles use the resulting geometry. Header variants remain a separate setting: for a next-page section restart, native parity filler pages depend on even/odd headers, not on `mirror_margins` alone. A first physical page displaying 2 has even mirrored margins even when it has no preceding filler.

The setter stages the native candidate and reuses existing bounded geometry checks before committing. An outside margin too narrow for a line-number label, or a floating picture shifted outside its physical page, raises `UnsupportedFormatError` atomically; prior bytes, revision, ranges and render cache remain usable. Existing output guards still apply after later content/layout mutations. Single-column line numbers and ordinary equal-column flow have combined acceptance; multi-column line numbering retains its existing explicit rejection. Successful assignments, including assigning the current value, follow the existing document-setting convention: they advance the revision and invalidate previous text ranges/render cache. Copies remain independent.

This owned horizontal-page profile adds no settings manager, section-specific mirror override, booklet printing, binding-direction switch, top gutter, imported editing or web-layout behavior. It does not expand the page-border, continuous-section, field or floating-artwork profiles. LAY10 remains open for its other settings.

### DOCX page-body vertical alignment

`section.vertical_alignment` is `"top"` by default. Set it to `"center"`, `"bottom"` or `"justify"` to use native page-body placement; `"top"` restores ordinary flow. Center and bottom position the body group within available vertical space. Justify distributes gaps between native blocks and leaves a single block at the top. This is distinct from paragraph horizontal alignment and cell vertical alignment. Appended sections inherit the property, and copies remain independent.

```python
with doc_ex.new("docx") as document:
    document.add_paragraph("A short title page")
    document.sections[0].vertical_alignment = "center"
    document.save("title.docx")
```

While any section is non-top, every section must use one column and a next-page start. Cells must use horizontal text; up/down cell text, notes, active even/odd header/footer selection and floating pictures are unavailable in that profile. These rules also apply to later edits; restore **all** sections to top before adding an excluded feature. Inactive even-story placeholders created by section copying are harmless. Ordinary body and nested tables, default/first headers and footers, inline pictures and page color/PNG backgrounds remain available.

The pinned native engine can shift page-anchored pictures and uneven columns incorrectly, and note serialization changes the available alignment space. These limits are explicit errors in owned non-top documents, not local layout corrections. Imported reading/rendering retains the upstream behavior. See [the recorded native limits](upstream-roadmap.md#section-vertical-alignment).

Source-profile failures preserve document bytes, handles, revision and cached output. Successful typography/content edits can still produce invalid vertical geometry; ordinary serialization, save, rendering and PDF conversion then raise `UnsupportedFormatError`. Reduce the content/formatting, enlarge the page or restore top. The property does not promise pixel-perfect roundtrips or complete layout-settings support.


### Delete an unused DOCX style

```python
with doc_ex.new("docx") as document:
    draft = document.styles.add("Draft note", based_on="Normal")
    draft_id = draft.id
    del document.styles[draft_id]       # a name or integer index also works
    replacement = document.styles.add("Draft note")
    assert replacement.id != draft_id
```

Deletion removes one unused custom paragraph, character or table definition. Builtin/default styles are protected. Incoming references anywhere in owned content, another style, defaults or numbering prevent deletion, including unrendered stored stories and conditional regions. Clear or change those references with the existing formatting/relationship APIs first. Slices, automatic restyling, batch pruning and imported style editing are unavailable. Unsupported opaque or tracked-history carriers fail explicitly.

A deleted Style handle remains invalid even if its name or ID becomes another style's display name. Existing name lookup still selects the current style. Copies retain independent identity history. A successful deletion invalidates revision-bound ranges and cached rendering; unrelated paragraph/table/picture/style handles remain valid. Conversion callbacks cannot redirect the outer style operation.

A document has a lifetime budget of 1024 named style IDs including builtins: deleted IDs stay reserved, so deleting a style does not replenish that budget. Copies inherit the reservation history; it is not serialized into DOCX. Capacity/work limits raise the existing UnsafeArchiveError. These safeguards support unused-only deletion; STY6 style import and general cleanup remain open.


## Owned DOCX rectangles and ellipses

```python
word = doc_ex.new("docx", size=(300, 300))
word.sections[0].margins = 30
shape = word.add_shape("rectangle", width=60, height=30,
                       fill="red", stroke="blue", stroke_width=2)
shape.format(wrap="behind", anchor="page", position=(80, 100))
shape.size = (80, 40)
shape.stroke = ("navy", 1)
shape.fill = None
for shape in word.shapes:
    print(shape.kind, shape.size, shape.formatting)
del word.shapes[0]
```

`WordDocument.add_shape("rectangle" | "ellipse", *, width, height, fill=None, stroke=None, stroke_width=1)` appends an inline native closed shape and returns an owned handle. `kind` is read-only; `size`, `fill` and `stroke` are writable. Colors accept the existing names/hex/RGB forms and read back as hex strings; `None` clears fill or stroke. Clearing both makes an intentionally invisible shape. Width and height use 1–4000 points in 0.05-point increments, subject to native column and physical-page bounds. Outlines use 0.25–100 points and cannot exceed the smaller shape dimension. Values outside that profile fail explicitly; they are never clamped.

The existing Picture placement vocabulary is shared: `format(wrap=..., anchor=..., position=..., distance=...)` and its owned `formatting` snapshot. Floating wraps are `square`, `top_bottom` and `behind`; `page`/`margin` anchor both axes together, while `paragraph` follows the native column/paragraph pair. Horizontal distances must match. Native layout supplies the physical placement, including mirrored margins and columns. The complete outline must fit the physical page; every later edit and output rechecks geometry, placement cardinality and unchanged requested dimensions. Shape editing can therefore reject a later paragraph/page/style change that would clip a shape.

`word.shapes` supports ordinary iteration, negative indexing, slices and single-item deletion. Its IDs survive surrounding paragraph/table edits; copies have independent owners. Deletion retains the native empty paragraph and any section boundary, like picture deletion. The deleted shape handle stays invalid even after new shapes are added. No shape movement, separate manager, second box/position representation or public handle class is added.

This profile covers direct owned body shapes in horizontal sections. Lines, text boxes, rotated shapes, front layering, groups, effects, imported editing and header/footer/cell/note shapes remain unavailable. Existing non-top section-alignment restrictions still apply. `layoutInCell` and `allowOverlap` remain upstream writer behavior, not configurable guarantees. Shapes do not become fake extracted text or images: semantic extraction retains its existing `shape_geometry_omitted` document diagnostic, while rendering uses native geometry. MED4 remains open for the excluded functionality; see [upstream shape evidence](upstream-roadmap.md#docx-closed-shapes-and-text-boxes).


### Imported PPTX line breaks and PDF text

The selected DeckCraft contribution preserves searchable text after existing explicit line breaks when an imported PPTX is converted to PDF. Native slide and notes/handout master projections now use the logical paragraph text, retaining break offsets used by layout. That dedicated dependency correction alone added no authoring method or Python getter change. The separate [owned cell paragraph adapter](#rich-pptx-cell-paragraphs) now supports `add_break("line")` with one-character LF reads; PPTX page/column breaks remain unsupported. The imported regression checks text, links, retained image ink and representative PNG/PDF output; it is not a claim of general Office or pixel-perfect fidelity.
### Reusing paragraph formatting

For an owned DOCX paragraph, reuse its full direct formatting with `paragraph.format(**paragraph.formatting)`. To make the currently resolved values explicit, use `paragraph.format(**paragraph.resolved_formatting)`. Direct values retain `None` for inheritance; resolved values make a local copy of the effective formatting, so later style changes can behave differently. Both forms retain the existing validation and mutation rules. The same direct-snapshot call works for the supported PPTX cell paragraph properties; resolved paragraph formatting remains DOCX-only.

Type checkers accept these complete snapshots and the existing inline patch dictionaries. This adds no method or new runtime formatting value. Some separately annotated nonnullable border dictionaries still have standalone border-input typing limitations; explicit nullable input dictionaries follow the existing input contract.

### Imported DOCX default font sizes

When a DOCX styles part exists but supplies no inherited font size, the selected native engine now preserves that absence and uses its existing 11-point fallback. Earlier builds inserted 10 points while reading. Explicit sizes are unchanged, and documents without a ZIP styles part retain the engine's initialized 12-point defaults. These are native application policies, not a universal Word or OOXML default.

This dependency correction applies to existing import/render/PDF conversion methods. Owned `document.styles.format_defaults(font_size=None)` removes the direct ordinary font size and restores native inheritance (spec132). With no inherited ordinary size, text resolves to 11 points; this does not recreate a new document’s initial 12-point value. Explicit style/run overrides remain intact, and `styles.defaults["font_size"]` reads `None`. Setting an explicit size and the other supported clears retain their existing behavior.

### Conservative PDF table detection

PDF tables are inferred from native reading geometry. If discarded native text overlaps a candidate table's replacement region, extraction retains the original text blocks instead of adopting that candidate. A visually table-like region can therefore remain paragraphs; the check prevents the demonstrated omitted-line case without promising arbitrary PDF table recognition. DOCX/PPTX semantic tables and the public table API are unchanged.

## EPUB creation (feature branch)

```python
import doc_ex

with doc_ex.new("epub") as book:
    book.set_metadata(
        title="Example book",
        identifier="urn:example:book",
        language="en",
        authors=["Example Author"],
    )
    chapter = book.add_chapter("Introduction")
    chapter.add_heading("Welcome", level=1)
    chapter.add_paragraph("A reflowable paragraph.")
    chapter.add_link("Project", "https://example.com")
    chapter.add_image(png_bytes, media_type="image/png", alt="Diagram")
    epub_bytes = book.to_bytes()
```

EPUB creation is append-only and semantic. `EpubDocument` supports the shared `to_bytes`, path/stream `save`, `copy`, context manager and idempotent `close` lifecycle. Required metadata is title, identifier and language. Links are inert ASCII-case-insensitive HTTP(S), mailto or safe fragment targets; remote resources are never fetched. Supported image resources are signature-checked PNG, JPEG and GIF bytes, limited to 16 MiB each and 64 MiB total. SVG authoring is rejected because active content cannot be guaranteed inert.

Callers cannot inject raw XHTML, OPF, CSS, SVG, scripts or archive paths. Existing EPUB editing, fixed-layout content, rendering, conversion, DRM and media overlays are unsupported.
