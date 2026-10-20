# DOCX numbering extraction

DOCX paragraph and heading blocks expose parsed list definitions and, where unambiguous, the label produced by native layout. These are ordinary owned values in `block.metadata["numbering"]`; they do not introduce another list or numbering API.

```python
with doc_ex.open("report.docx") as document:
    for page in document:
        for block in page.blocks:
            numbering = block.metadata.get("numbering")
            if numbering is not None:
                print(numbering.get("label"), block.text)
```

The same metadata appears on paragraph blocks inside table cells and in `document.stories`. Walk those existing collections when inspecting nested or auxiliary content.

| Key | Meaning |
| --- | --- |
| `num_id` | Native numbering-instance identifier from the resolved paragraph reference. |
| `abstract_id` | Referenced abstract definition identifier, or `None` when the numbering instance is missing. An identifier can still refer to a missing abstract definition. |
| `level` | Zero-based level in the resolved reference. Normal authored lists use levels 0–8; imported references are reported without a guessed correction. |
| `format` | Effective native level format's OOXML name, such as `decimal`, `lowerLetter` or `bullet`; `None` if the level cannot be resolved. This is not the authoring `set_list` kind. |
| `template` | Effective parsed level text, such as `%1.` or `Item %1)`; `None` if the level cannot be resolved. Native level overrides take precedence. |
| `label` | Optional native layout `Label.text`, such as `7.` or `•`, for this source segment. Its absence means no unique laid-out label is asserted. |

Style inheritance and native table context determine the effective numbering reference. A resolved reference of zero explicitly removes inherited numbering, so the block has no `numbering` entry. Missing definitions retain the known instance identifier and level, with unresolved definition fields set to `None`.

## Text and placement

List labels remain separate from `block.text`, links, character offsets, plain text and Markdown. Extraction does not prefix markers or run another counter algorithm. Empty numbered paragraphs remain present as semantic blocks even when their text is empty. PDF conversion and raster rendering continue to draw the native labels.

Native layout identifies each paragraph by its story and source path. Body paragraphs, nested cells, headers, footers and notes therefore cannot collide merely because they share a numeric path. Explicit page/column breaks split the source paragraph into segments; only the segment containing the native label line receives `label`. Automatic wrapping does not duplicate the label. Existing extraction still assigns an automatically split paragraph's source text to its first computed page rather than reconstructing exact per-page text fragments.

Repeated placements with identical labels yield one owned source label. If placements disagree, `label` is omitted while definition metadata remains. In the pinned engine, a numbered header repeated across two pages can render as `2.` and `3.`. Neither is asserted as the single label of the owned header paragraph. Unplaced auxiliary paragraphs can retain definition metadata without a label.

## Native normalization and limits

These values describe the parsed native model, not an untouched copy of source XML. The pinned DOCX reader truncates imported `lvlText` to 64 Unicode characters before exposing the model. Extraction reports that parsed template. It cannot recover the discarded suffix. Native symbol-font mapping can also make a private-use bullet template produce the Unicode label `•`.

`label` is native logical marker text, not a reconstruction of glyph order or capitalization. For example, an RTL label can have metadata `1.` while its PDF glyph order reads `.1`. A reported format or template does not claim full rendering fidelity for every imported numbering format; native format fallbacks remain upstream limitations.

Extraction bounds numbering definitions, effective-level records and observed paths at 100,000, scans at most 1,000,000 native layout items, and rejects model template/label strings over 64 KiB. Aggregate template bytes, observed label bytes and conservatively accounted output metadata each have a 16 MiB budget. The output accounting allows for JSON escaping and the existing duplicated table projection. Native definition lookups are cached by instance and level, including missing references, with at most 100,000 cached results and a conservative 10,000,000-step cumulative scan budget. This bounds the adapter's definition lookup work; it does not bound the earlier native layout algorithm. Exceeding a bound raises the existing resource-limit exception; partial metadata is not returned successfully.

## Acceptance evidence

`tests/test_numbering_extraction.py` covers nine levels, continuation/restart, empty items, owned values, exact link offsets, inherited styles, explicit numbering removal, imported start overrides, custom templates, nested cells, page/column breaks, automatic pagination, body/header/note identity, ambiguous repeated headers, missing definitions, native template normalization, symbol bullets and RTL label text. Supported labels are checked against reopened native PDF output.

Native tests in `rust/src/docx.rs` additionally cover conflicting placements, unresolved levels, oversized model templates and aggregate metadata rejection. This slice does not add starting-number or custom-level authoring, cross-document numbering remapping, or an alternative list renderer. Those remain separate TXT5/DOC6 work.
