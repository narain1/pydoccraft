# Cookbook

These six runnable examples use the current authored-document profile. Install a wheel built from this checkout, then run an example from the same checkout. These authoring additions are unreleased and are absent from the published 0.2.1 wheel; see [development instructions](development.md). Each command takes an output directory and an optional `--dpi` (default 96); use a new directory because named outputs may be replaced. No external document applications, downloaded assets or optional Python packages are required.

| Workflow | Source and entry point | Outputs |
|---|---|---|
| Letter | [letter.py](../examples/letter.py), `create_letter(output_dir, dpi=96)` | DOCX, PDF and PNG |
| Invoice | [invoice.py](../examples/invoice.py), `create_invoice(output_dir, dpi=96)` | DOCX, PDF and PNG; Python `Decimal` formats amounts |
| Branded report | [branded_report.py](../examples/branded_report.py), `create_report(output_dir, dpi=96)` | Two-page DOCX/PDF, one PNG per page |
| Template letters | [template_batch.py](../examples/template_batch.py), `write_letters(output_dir, dpi=96)` | Two independently filled DOCX/PDF letters and previews |
| PDF assembly | [pdf_assembly.py](../examples/pdf_assembly.py), `assemble_pdfs(output_dir, dpi=96)` | Two source PDFs, assembled PDF, selected subset and previews |
| Slide preview | [slide_preview.py](../examples/slide_preview.py), `create_slide_preview(output_dir, dpi=96)` | Two-slide PPTX/PDF, slide PNGs and a region preview |

```bash
python examples/letter.py output/letter --dpi 144
python examples/invoice.py output/invoice
python examples/branded_report.py output/report
python examples/template_batch.py output/letters
python examples/pdf_assembly.py output/assembly
python examples/slide_preview.py output/slides --dpi 144
```

Each file also exposes the ordinary Python function listed above and can be imported without generating files. The examples compose the public API; they do not add report, invoice, template or preview classes to the package. Template filling copies an authored in-memory document and replaces two explicit literal placeholders in named story scopes. It does not edit arbitrary imported DOCX files or bind content controls.

## Shared conventions

- `new("pdf")` starts with one page; `new("pptx")` starts with zero slides. The assembly example inserts real pages before deleting the default blank page, since a PDF must retain at least one page.
- Lengths are points; use `mm`, `cm` or `inches` explicitly. Boxes are `(x, y, width, height)` from the displayed top-left corner. `font_size` is in points. DPI belongs to raster output, not document layout.
- `section.header()` and `section.footer()` return story containers. Tables use `table[row, column]` for edits; `table.rows` is a text snapshot. Formatting and links reuse paragraphs and ranges.
- Structural page organization invalidates positional page handles. Reacquire them from `pdf.pages`; consume or refresh revision-bound text ranges before unrelated edits. Copies own independent data.
- Context-manager exit closes without saving. `save()` writes the document's native format, `convert("pdf")` returns bytes, and `render_page()` returns PNG bytes. The examples write those bytes explicitly with `Path.write_bytes`.
- Unsupported operations and invalid input fail explicitly. Example functions do not provide a multi-file transaction: an exception can leave earlier completed outputs in the caller's directory.

The same preview call works for PDF, DOCX layout pages and PPTX slides:

```python
from pathlib import Path
import doc_ex

with doc_ex.open("document.docx") as document:
    Path("page.png").write_bytes(document.render_page(0, dpi=144))
```

For caller-side thumbnails, load the PNG in optional Pillow and call `thumbnail`; see the [image interoperability recipe](content-readiness.md#runnable-preview-recipe-using-the-current-api). Resizing does not preserve the original physical page dimensions by itself.

## Evidence and scope

[Cookbook tests](../tests/test_cookbook.py) execute the actual example functions and CLI entry points against the installed wheel, reopen advertised formats and verify semantic order, links, page counts, DPI and visible content. Existing [template tests](../tests/test_template_recipes.py) own placeholder validation and copy independence, while the [table corpus](table-corpus.md) owns long/merged/nested/RTL/image table combinations. See the [validation record](../.kiro/specs/rust-document-engine/validation.md) for exact artifacts and test counts.

These workflows exercise the current names, defaults, units, explicit failures and handle lifetimes with zero new public library methods. They complete the current-profile API10 usability gate; broader parity, imported Office editing, advanced preservation and the full representative document corpus remain separate open criteria. Conversion is not pixel-perfect; use the [format limits](formats.md) and [authoring guide](authoring.md) when adapting a recipe.
