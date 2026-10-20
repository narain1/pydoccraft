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
- PyPI trusted publishing must be configured for the `pydoccraft` project from `narain1/pydoccraft` and the `pypi` GitHub environment before the first release tag.
- Known upstream fidelity limitations remain those documented in `README.md` and `design.md`; conversion is not claimed to be pixel-perfect.

## 2026-10-20 pydoccraft 0.2.1 packaging validation

- Renamed the PyPI distribution to `pydoccraft`; the compatible Python import remains `doc_ex`.
- Package, crate, lockfile, and Python versions agree on `0.2.1`.
- Replaced invalid Git patches to PdfCraft's excluded `vendor/` directory with exact vendored Apache/MIT snapshots of Hayro and lopdf from PdfCraft revision `645016addcfb9078c557f0189559ab47cdcee057`.
- Built `pydoccraft-0.2.1-cp311-cp311-manylinux_2_17_aarch64.manylinux2014_aarch64.whl` with Rust 1.95 and Maturin 1.8.1.
- Installed the wheel in a clean Python 3.11 container; `importlib.metadata.version("pydoccraft")` and `doc_ex.__version__` both returned `0.2.1`.
- Installed-wheel test suite passed: 13 tests.
- Wheel size: 13,198,306 bytes.
- Wheel metadata reports Apache-2.0 and includes `LICENSE`, `NOTICE`, and `LICENSE-THIRD-PARTY`; the latter matches `THIRD_PARTY_LICENSES.md` byte-for-byte.
- `pydoccraft` was not yet published when checked. First publication requires a matching pending PyPI Trusted Publisher for repository `narain1/pydoccraft`, workflow `release.yml`, and environment `pypi`.

## 2026-10-20 final v0.2.1 release validation

- Repository renamed to `narain1/pydoccraft`; local origin, `main`, GitHub environment, existing v0.2.0 tag, and release were verified after the rename.
- Rust 1.95 clean-target validation passed: formatting, 3 Rust tests, and Clippy with warnings denied.
- Final CPython 3.11 Linux AArch64 wheel built with the committed lockfile and installed in a clean container.
- Installed-wheel Python suite passed: 13 tests covering extraction, rendering, lifecycle, safety, and reopenable PDF→DOCX, DOCX→PDF, and PPTX→PDF conversions.
- Final wheel size: 13,198,994 bytes; native extension remains 28,648,840 bytes uncompressed.
- Wheel metadata reports `pydoccraft` 0.2.1 under Apache-2.0.
- Wheel includes `LICENSE`, the reviewed `NOTICE`, and `LICENSE-THIRD-PARTY`; the latter matches `THIRD_PARTY_LICENSES.md` byte-for-byte.
- Final lockfile contains 246 unique package names and no prohibited engine, UI, CLI, MCP, automation, OCR, JavaScript, print, signing, or WinPrint package.
- Documentation links, workflow YAML, and `git diff --check` passed.
- PyPI trusted publishing was reported configured for project `pydoccraft`, owner `narain1`, repository `pydoccraft`, workflow `release.yml`, and environment `pypi`.
