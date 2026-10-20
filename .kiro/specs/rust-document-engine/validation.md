# Validation evidence

## 2026-10-19 repository and CI initialization

This is pre-release validation for the initial GitHub publication, not a declaration that task 10 or every release-candidate platform check is complete.

### Environment

- Host: macOS on Apple Silicon
- Validation container: Linux AArch64, Rust 1.95, CPython 3.13
- Build backend: Maturin 1.8.1
- Locked upstream revisions: PdfCraft `645016addcfb9078c557f0189559ab47cdcee057`, WordCraft `f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`, DeckCraft `22db9261f9d1a214c296fd390173130c45d76d66`, craft-libs `c3487320b2d1de657b860b3c8abb44e1f2b32bd8`

### Results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo test --locked` | Passed: 3 tests |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| Maturin release wheel build with `--locked` | Passed |
| Tests against the installed wheel | Passed: 13 tests |
| Conversion outputs reopened by tests | Passed for PDF→DOCX, DOCX→PDF, and PPTX→PDF |
| PDF, DOCX, and PPTX extraction and PNG rendering | Passed in the installed-wheel suite |
| GitHub Actions workflow lint | Passed with `actionlint` |
| Prohibited package scan | No PdfCraft engine/JavaScript, WinPrint, UI, MCP, automation, or OCR package linked |
| Locked license inventory | 262 normal/build packages; no undeclared or GPL-/AGPL-/LGPL-only license |

The Linux AArch64 CPython 3.13 wheel measured 13,192,166 bytes. The uncompressed native extension measured 28,648,840 bytes.

### Dependency and conversion correction

The pre-existing manifest referenced the PdfCraft application engine while the lockfile referenced the lower-level export crate. The manifest now pins `pdfcraft-export` at the existing PdfCraft revision, and PDF→DOCX maps public PdfCraft extraction values into `pdfcraft_export::Page` before calling the upstream DOCX writer. This removes the application engine and its prohibited JavaScript/printing graph. `Cargo.lock` and `THIRD_PARTY_LICENSES.md` were regenerated and reviewed.

### Remaining release checks

- GitHub CI must confirm the clean Python 3.11 installed-wheel path after the initial push.
- Tagged release jobs must confirm Python 3.11–3.13 wheels on GitHub-hosted Linux, macOS, and Windows runners.
- PyPI trusted publishing must be configured for `narain1/doc-ex` and the `pypi` GitHub environment before the first release tag.
- Known upstream fidelity limitations remain those documented in `README.md` and `design.md`; conversion is not claimed to be pixel-perfect.
