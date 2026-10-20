# pydoccraft

[![CI](https://github.com/narain1/pydoccraft/actions/workflows/ci.yml/badge.svg)](https://github.com/narain1/pydoccraft/actions/workflows/ci.yml)

`pydoccraft` 0.2.1 is an Apache-2.0 Python/Rust package for eager, in-process structured extraction, rendering, and semantic conversion of PDF, DOCX, and PPTX files. It exposes the compatible `doc_ex` PyMuPDF-style API and uses pinned PdfCraft, WordCraft, and DeckCraft library crates—without external executables, runtime downloads, networking, cloud credentials, or OCR.

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

PDF table detection is conservative. PDF links usually retain destination and geometry but not label text. DOCX computed pagination cannot split one automatically wrapped paragraph into exact normalized page slices. PPTX exposes URL links but not every PowerPoint action. Conversion uses upstream semantic exporters and is not pixel-perfect.

## Documentation

- [Python API](docs/api.md)
- [Formats, rendering, and conversion](docs/formats.md)
- [Safety and resource behavior](docs/safety.md)
- [Dependencies, licensing, and attribution](docs/dependencies-and-licenses.md)
- [Development and release validation](docs/development.md)

## Development

```console
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
maturin build --release --locked --out dist
python -m unittest discover -s tests -v
```

Release validation must install the built wheel into a clean environment and run the suite against that installed package. Tests generate their own fixtures and require no external document tools.

## License

`pydoccraft` is licensed under Apache-2.0. Distributions include [`LICENSE`](LICENSE), [`NOTICE`](NOTICE), and the wheel-compatible [`LICENSE-THIRD-PARTY`](LICENSE-THIRD-PARTY) copy of the audited [`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md) inventory. Upstream dependency names identify their source and are not `pydoccraft` branding.

The source repository is [`narain1/pydoccraft`](https://github.com/narain1/pydoccraft). PyPI trusted publishing is configured for project `pydoccraft`, repository `pydoccraft`, workflow `release.yml`, and environment `pypi`.
