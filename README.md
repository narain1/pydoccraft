# pydoccraft

[![CI](https://github.com/narain1/pydoccraft/actions/workflows/ci.yml/badge.svg)](https://github.com/narain1/pydoccraft/actions/workflows/ci.yml)

`pydoccraft` 0.3.1 is an Apache-2.0 Python/Rust package for eager, in-process structured extraction, rendering, and semantic conversion of PDF, DOCX, PPTX, and EPUB files. EPUB support (chapter-ordered semantic extraction and native reflowable creation, without physical pagination) is unreleased. It exposes the compatible `doc_ex` PyMuPDF-style API and uses pinned PdfCraft, WordCraft, and DeckCraft library crates—without external executables, runtime downloads, networking, cloud credentials, or OCR.

## Install

```console
python -m pip install pydoccraft
```

The wheel contains the private `doc_ex._core` native extension. No office suite, Pandoc, or Craft CLI is required.

## Quick start

```python
from pathlib import Path
import doc_ex

with doc_ex.open("report.pdf") as document:
    print(document.page_count)
    print(document[0].get_text())
    print(document[0].get_text("markdown"))
    print(document[0].get_links())
    print(document[0].find_tables())
    Path("page.png").write_bytes(document[0].get_pixmap(dpi=144))
    Path("report.docx").write_bytes(document.convert("docx"))

Path("report.pdf").write_bytes(doc_ex.convert("report.docx", "pdf"))
Path("slides.pdf").write_bytes(doc_ex.convert("slides.pptx", "pdf"))
```

`Document` supports context management, iteration, integer and negative indexing, `load_page`, `page_count`, metadata, Markdown, PNG rendering, conversion, and idempotent close. `Page.number` is zero-based. Returned `Block`, `Table`, `Link`, and `Image` objects own their extracted data.

## Supported operations

| Operation | PDF | DOCX | PPTX |
|---|---:|---:|---:|
| Structured text, links, images, and tables | Yes | Yes | Yes |
| PNG page/slide rendering | Yes | Yes | Yes |
| Convert to PDF | — | Yes | Yes |
| Convert to DOCX | Yes | — | No |
| Password input | Yes | Yes | Encrypted PPTX unsupported |
| OCR | No | — | — |
| Create a new document from scratch | Basic | Basic | Basic |

PDF table detection is conservative. PDF links retain destinations and geometry; unambiguous single-line and contiguous multiline labels have exact character offsets. Ambiguous regions remain unresolved. DOCX computed pagination cannot split one automatically wrapped paragraph into exact normalized page slices. PPTX exposes URL links but not every PowerPoint action. Conversion uses upstream semantic exporters and is not pixel-perfect.

## Render a page at a chosen DPI

The same API works for PDF pages, DOCX layout pages, and PPTX slides:

```python
from pathlib import Path
import doc_ex

with doc_ex.open("report.docx") as document:  # also .pdf or .pptx
    Path("page-1.png").write_bytes(document.render_page(0, dpi=300))
    # Equivalent: document[0].get_pixmap(dpi=300)
    Path("detail.png").write_bytes(document.render_page(0, dpi=300, clip=(36, 36, 144, 72)))
```

Indexes are zero-based; negative indexes are supported. DPI defaults to 144 and must be finite and between 1 and 1200, subject to output-size limits. Both methods return PNG bytes. Optional `clip=(x, y, width, height)` selects a region in displayed-page points. DOCX pagination follows WordCraft layout and may differ from Microsoft Word. See [rendering details](docs/formats.md#rendering).

Version 0.3.0 adds native authoring through `doc_ex.new("docx" | "pdf" | "pptx")`, including text, tables, images, backgrounds, explicit saving and independent copies. See [the authoring API](docs/authoring.md) for examples, the supported edit profile and current limits. Earlier releases, such as 0.2.1, do not include these additions.

The expanded [roadmap](TODO.md) covers Open XML feature parity, styles, sections, templates, review features and safe editing. The [design and reference crosswalk](docs/authoring-api-design.md) keeps ArtCraft libraries as the only document backend. Unchecked coverage remains incomplete even where an initial slice is implemented.

The coverage checklist is consolidated into [14 connected workstreams](docs/roadmap-workstreams.md), with [upstream contribution candidates](docs/upstream-roadmap.md) distinguished from existing library support. Reports, watermarks, templates and batches should compose a small core API wherever possible.

Track the work in the [GitHub Roadmap project](https://github.com/users/narain1/projects/8).

## Documentation

Browse the [documentation website](https://narain1.github.io/pydoccraft/) for guides, examples and the roadmap. It follows `main`; unreleased features remain labeled separately from published packages.

- [Python API](docs/api.md)
- [Native authoring](docs/authoring.md)
- [Six runnable cookbook workflows](docs/cookbook.md)
- [Formats, rendering, and conversion](docs/formats.md)
- [Safety and resource behavior](docs/safety.md)
- [Dependencies, licensing, and attribution](docs/dependencies-and-licenses.md)
- [PDF performance versus PyMuPDF](docs/pdf-performance.md)
- [Development and release validation](docs/development.md)
- [Feature roadmap and TODOs](TODO.md)
- [Proposed Python authoring API and Open XML crosswalk](docs/authoring-api-design.md)
- [Canonical workstreams and dependencies](docs/roadmap-workstreams.md)
- [Upstream ownership and pinned-source evidence](docs/upstream-roadmap.md)

## Development

```console
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
maturin build --release --locked --out dist
python -m unittest discover -s tests -v
```

Release validation must install the built wheel into a clean environment and run the suite against that installed package. Tests generate their own fixtures and require no external document tools.

## Community

- Read [CONTRIBUTING.md](CONTRIBUTING.md) before proposing changes.
- Ask usage questions through [GitHub Discussions](https://github.com/narain1/pydoccraft/discussions) and report reproducible defects through [GitHub Issues](https://github.com/narain1/pydoccraft/issues).
- Report vulnerabilities privately according to [SECURITY.md](SECURITY.md); never attach malicious or confidential documents to public issues.
- Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md), with project decisions described in [GOVERNANCE.md](GOVERNANCE.md) and support expectations in [SUPPORT.md](SUPPORT.md).

## License

`pydoccraft` is licensed under Apache-2.0. Distributions include [`LICENSE`](LICENSE), [`NOTICE`](NOTICE), and the wheel-compatible [`LICENSE-THIRD-PARTY`](LICENSE-THIRD-PARTY) copy of the audited [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) inventory. Upstream dependency names identify their source and are not `pydoccraft` branding.

The source repository is [`narain1/pydoccraft`](https://github.com/narain1/pydoccraft). PyPI trusted publishing is configured for project `pydoccraft`, repository `pydoccraft`, workflow `release.yml`, and environment `pypi`.
