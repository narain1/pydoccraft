# Documents, in Python.

Extract structured content, render pages at your chosen DPI, and convert PDF, DOCX and PPTX files. One Python interface, with native engines running in your process.

<div class="hero-links">
<a class="button" href="../docs/api.md">Explore the API →</a>
<a class="button secondary" href="../docs/cookbook.md">Build with examples</a>
</div>

## Start with a page

```console
python -m pip install pydoccraft
```

```python
from pathlib import Path
import doc_ex

with doc_ex.open("report.docx") as document:  # PDF and PPTX work too
    print(document[0].get_text())
    Path("page.png").write_bytes(document.render_page(0, dpi=300))
    Path("report.pdf").write_bytes(document.convert("pdf"))
```

Rendering returns PNG bytes. Page indexes start at zero; negative indexes work as in Python. DPI defaults to 144 and accepts finite values from 1 to 1200 within the output-size limits. See [rendering and conversion](../docs/formats.md#rendering) for details.

## Choose a workflow

| I want to… | Start here |
|---|---|
| Extract text, links, tables and images | [Python API](../docs/api.md) |
| Export a page as semantic HTML (unreleased) | [HTML example](../docs/api.md#export-a-page-as-html-unreleased) |
| Render a page or slide, or convert a document | [Formats and rendering](../docs/formats.md) |
| Create a report with paragraphs, tables and backgrounds | [Native authoring — development API](../docs/authoring.md) |
| Compose reports, templates, watermarks and batches | [Six runnable cookbook workflows](../docs/cookbook.md) |
| Export Markdown with its referenced images | [Markdown and assets](../docs/markdown-assets.md) |
| Check a specific operation before adopting it | [Operation support matrix](../docs/support-matrix.md) |

## Know what is supported

| Operation | PDF | DOCX | PPTX |
|---|---|---|---|
| Extract structured content | Supported | Supported | Supported |
| Render a page or slide to PNG | Supported | Supported | Supported |
| Convert to PDF | Already PDF | Supported | Supported |
| Convert to DOCX | Supported | Already DOCX | Unsupported |

This is semantic conversion, not pixel-perfect reproduction. PDF table detection is conservative. DOCX pagination follows the upstream layout engine and may differ from Microsoft Word; automatically split paragraphs cannot always be mapped exactly into extracted page slices. PowerPoint actions and complex content have additional limitations. See the [complete format notes](../docs/formats.md) and [operation evidence](../docs/support-matrix.md).

Authoring is documented separately and first ships in version 0.3.0. A supported creation workflow does not imply safe editing or preservation of arbitrary imported files. Check the [authoring guide](../docs/authoring.md), [editing readiness](../docs/format-authoring-readiness.md) and [release notes](https://github.com/narain1/pydoccraft/releases) for the version you install.

## A small interface, native document engines

`pydoccraft` is Apache-2.0 licensed and uses pinned PdfCraft, WordCraft and DeckCraft libraries as upstream engines. Wheels contain the native extension and required assets. Document operations require no external executable, runtime download, network connection or cloud credentials. OCR is not included.

Read the [resource limits and error behavior](../docs/safety.md) before processing untrusted files, and the [dependency and attribution guide](../docs/dependencies-and-licenses.md) for distribution details.

## Follow the work

The [feature roadmap](../TODO.md) is grouped into [14 connected workstreams](../docs/roadmap-workstreams.md). Unchecked features remain incomplete, even when a narrower part already works. The [upstream roadmap](../docs/upstream-roadmap.md) identifies work that belongs in the native libraries.

[Track the GitHub Project](https://github.com/users/narain1/projects/8) · [Report an issue](https://github.com/narain1/pydoccraft/issues) · [Contribute](../CONTRIBUTING.md)
