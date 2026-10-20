# Tables, advanced Office content and composed workflows

This is the implementation handoff for [W05](https://github.com/narain1/pydoccraft/issues/7), [W12](https://github.com/narain1/pydoccraft/issues/14) and [W13](https://github.com/narain1/pydoccraft/issues/15). Source availability establishes an integration path; it does not establish tested Python authoring support. All WordCraft links below use the pinned revision `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`.

## Table boundaries

Extraction `Table` and `Cell` values own snapshots. `Table.rows` remains the flat text convenience projection; optional `Table.cells[row][cell].blocks` preserves paragraph/table order, links and images. DOCX cells retain column spans, vertical merge anchors and continuations. `find_tables()` lists page-level tables; discover nested tables through cell blocks. Changing these Python values changes that snapshot only, never the source document. The compatibility projection and structured cells are separate representations; callers should choose one when traversing content.

The future editable cell must refer to the native editable document and use its persistence/lifetime checks. Reuse the names `blocks`, `rows`, and span properties where their semantics match, but do not turn an extracted dataclass into an editing handle. In particular, rebuilding an editable document from extraction would discard unsupported content. W03 owns native lifecycle and safe persistence; W04 owns rich paragraphs/styles; W05 composes those in cells. W06 owns layout/pagination. No additional cell-specific paragraph, image or link subsystem is needed.

The public [table model](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/table.rs) already provides construction, row/column insertion/deletion, merge/split, grid positions and recursive cell blocks. Its operations can clamp or ignore invalid requests. Validate Python requests before mutation and test unchanged state on rejection; U03 tracks a possible fallible upstream interface. The [table layout implementation](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/layout/src/table.rs) already handles nested tables. Repeating headers, row splitting, RTL, vertical alignment and cross-page spans still require targeted round-trip and rendered fixtures. Do not implement another layout algorithm.

Nested/merged/rich extracted tables use escaped HTML inside Markdown because pipe syntax cannot express those structures. Simple tables retain pipe syntax. HTML-disabled Markdown consumers need their own presentation choice. Plain text uses the compatibility rows once. Source content unexpectedly present in a covered cell remains visible, even if the resulting HTML does not reproduce the source merge geometry. Tables spanning pages still use the existing first-page assignment limitation. The new extraction payload rejects body-table nesting beyond 16 levels before layout; this is not a general parser or secondary-story depth guarantee.

## Advanced Office audit

| Content | Existing pinned library surface | Integration and validation still required |
|---|---|---|
| Charts | Public [`ChartSpec`, `ChartSeries`, `ChartType`](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/chart.rs), with bounded sanitization. [`wordcraft_docx`](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/lib.rs) re-exports `chart_items` and `chart_xml`. The model supports nine common chart types. | Do not infer general workbook editing or spreadsheet recalculation. Verify cached values, relationships and unchanged embedded workbooks separately. Oversized/nonfinite inputs must produce explicit local validation or diagnostics rather than unnoticed sanitizer changes. |
| Imported chart fidelity | The [reader drawing path](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/read/chart.rs) has a placeholder for unsupported-only charts; a mixed plot can omit unsupported chart types while drawing supported types. | Add radar/stock and mixed-chart fixtures. U08 should expose a structured limitation notice for these paths; successful rendering alone must not imply complete artwork. |
| Equations | Public [`Math`, `MNode`, `parse_linear`, `to_linear`](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/math.rs) and [`parse_latex`/`to_latex`](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/math_latex.rs). Math retains imported OMML alongside its tree. Layout and DOCX writing already consume it. | Test imported-source preservation separately from edited-tree serialization, font substitution, display/inline placement and PDF output. The LaTeX parser keeps unknown commands as text and bounds input; it is not a general TeX engine. |
| Diagrams | Public [`SmartArtSpec`, `SmartArtItem`, layout/color enums](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/smart_art.rs), and re-exported `smart_art_items` from the DOCX library. Model limits bound item count, text and hierarchy. | Its editable import recognizes the data shape the writer generates. Imported arbitrary diagrams are a preservation/rendering case, not automatically editable. Test native diagram parts and cached drawings after saving and reopening. |
| Unedited graphics | [`Graphic`](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/graphic.rs) carries drawing items, optional chart/diagram models and source preservation references; other package bytes live in `Document::passthrough`. | W03 owns preservation diagnostics. W12 supplies operation-specific fixtures and reports; it does not introduce a second package lifecycle. Presence of passthrough storage alone is not proof of arbitrary round-trip fidelity. |

Python should expose an advanced operation only after its read/create/edit/save/render/export behavior is established. Keep charts, equations and diagrams independently scoped: sharing drawing placement does not make their semantic editing interchangeable. U08 owns missing format algorithms/diagnostics discovered here; local adapters own validation and Python naming. Do not link WordCraft engine/UI commands to obtain those features. These audit findings do not require an upstream revision change.

## Runnable preview recipe using the current API

This function accepts PDF, DOCX or PPTX. Install Pillow separately in the caller's environment; it is optional and is not a pydoccraft runtime dependency. The document renders in process through its ArtCraft backend. Pillow loads the returned PNG and makes a bounded preview; it does not parse documents.

```python
from io import BytesIO
from pathlib import Path

import doc_ex
from PIL import Image


def save_preview(source: str | Path | bytes, destination: str | Path,
                 page: int = 0, dpi: float = 144) -> None:
    with doc_ex.open(source) as document:
        png = document.render_page(page, dpi=dpi)
    with Image.open(BytesIO(png)) as image:
        image.load()
        preview = image.convert("RGB")
    preview.thumbnail((1200, 1200))
    preview.save(destination, format="PNG")
    preview.close()
```

The thumbnail is intentionally a screen preview and does not claim to preserve the physical page size/DPI after resizing. To retain the original requested resolution, write the PNG bytes directly in caller code. Native processing never writes the destination path. A loop over filenames is sufficient for batches; no public job framework or preview object is needed.

## Future recipes, not released APIs

- Reports and invoices: W04 paragraphs/styles + W05 tables + W06 placement, then existing DOCX-to-PDF conversion. Python formats values and repeats rows; do not create separate invoice/report methods.
- Backgrounds and watermark placement: W06 native fills/drawings; a caller loop chooses pages/sections. Avoid a parallel preset engine.
- Template substitutions: ordinary Python matching followed by W07 bounded text-range edits. Native content controls and field evaluation remain W08 capabilities, not string substitution aliases.
- Review copies: W07 comments/revisions plus W03 copying/persistence. No report-specific comment API.
- Image-only PDF assembly: W10 image/page creation with Pillow preprocessing where useful. Existing `convert()` does not currently accept PNG or claim this workflow.

Each recipe becomes runnable only when its dependencies ship and its fixture saves, reopens and renders successfully. W14 owns the common test harness; individual workstreams own semantic assertions. W05/W12/W13 remain open after this audit and initial extraction work.
