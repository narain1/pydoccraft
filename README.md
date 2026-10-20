# doc-ex

[![CI](https://github.com/narain1/doc-ex/actions/workflows/ci.yml/badge.svg)](https://github.com/narain1/doc-ex/actions/workflows/ci.yml)

`doc-ex` 0.2.0 is a Python 3.11+ mixed Python/Rust package for eager, in-process structured extraction from PDF, DOCX, and PPTX. It exposes a small PyMuPDF-style API and semantic PDF↔DOCX conversion. It performs no downloads, networking, subprocess execution, font discovery, or OCR.

## Install

```console
python -m pip install doc_ex-0.2.0-*.whl
```

The platform wheel contains the private `doc_ex._core` native extension; no office suite or external executable is required.

## API

```python
from pathlib import Path
import doc_ex

with doc_ex.open("report.pdf") as document:
    print(document.page_count)
    print(document[0].get_text("text"))
    print(document[0].get_text("markdown"))
    print(document[0].get_text("dict"))
    print(document[0].get_links())
    print(document[0].get_images(full=True))
    print(document[0].find_tables())
    png_bytes = document[0].get_pixmap(dpi=144)
    docx_bytes = document.convert("docx")

pdf_bytes = doc_ex.convert(Path("report.docx").read_bytes(), "pdf", filetype="docx")
```

`Document` supports context management, iteration, integer and negative indexing, `load_page`, `page_count`, `metadata`, `to_markdown`, `convert`, and idempotent `close`. `Page.number` is zero-based. Returned `Block`, `Table`, `Link`, and `Image` values own their data and remain independent of source and temporary files.

DOCX uses WordCraft layout for computed pages, explicit `PAGE_BREAK` characters, and inherited `page_break_before` styles. Normalized blocks are emitted once on the first computed page they occupy because WordCraft does not expose exact per-page text slices for automatically wrapped paragraphs. PPTX returns one page per slide in presentation order. DOCX→PDF uses WordCraft layout and export, including Unicode text and empty explicit pages. PDF table detection is conservative and only reports stable positioned rows. Scan-only PDFs are not OCRed. Password-protected PDFs and DOCX files require the matching password.

## Safety limits

OOXML archives are rejected above 10,000 members, 128 MiB per member, 512 MiB aggregate uncompressed data, or a 1,000:1 suspicious compression ratio. Absolute and parent-traversing relationship targets are rejected. Parsing is memory-only after Python reads the source.

## Runtime licenses

The package itself is Apache-2.0. Native runtime crates are pinned in `Cargo.toml`; PdfCraft v0.9.0 is pinned to revision `645016addcfb9078c557f0189559ab47cdcee057` and WordCraft v0.8.0 is pinned to revision `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`. Distributions must include `LICENSE` and `NOTICE`; the latter retains PdfCraft, PDFium/Foxit font, and Adobe CMap attribution notices. `THIRD_PARTY_LICENSES.md` must be regenerated and reviewed under task 9 before release.

## Test

```console
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
maturin build --release --locked
python -m unittest discover -s tests -v
```

Tests generate all PDF, DOCX, and PPTX fixtures locally and require no network, cloud credentials, or external document binaries.

## Releases

Every pull request and push to `main` runs Rust formatting, locked tests, Clippy, a release wheel build, and the Python suite against the installed wheel. Tags matching the package version (for example, `v0.2.0`) build and smoke-test Python 3.11–3.13 wheels on Linux, macOS, and Windows, then attach them to a GitHub release.

The release workflow also uses PyPI trusted publishing through the `pypi` GitHub environment. Before the first tag, configure `narain1/doc-ex` and `.github/workflows/release.yml` as a trusted publisher for the `doc-ex` project on PyPI.
