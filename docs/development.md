# Development and release validation

## Prerequisites

- Python 3.11 or newer;
- Rust 1.95 with `rustfmt` and Clippy;
- Maturin 1.8.1;
- network access during dependency resolution and wheel builds for pinned Git and crates.io dependencies.

Document extraction at runtime requires no network access.

## Local setup

```bash
python -m venv .venv
source .venv/bin/activate
python -m pip install maturin==1.8.1
maturin develop --locked
python -m unittest discover -s tests -v
```

Keep virtual environments, `target/`, wheels, and Python caches untracked.

## Required checks

Run the committed lockfile throughout validation:

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
maturin build --release --locked --out dist
```

A source-tree Python test is useful during development but is not release evidence. Install the built wheel into a clean environment and run the suite against that installation:

```bash
python -m venv /tmp/pydoccraft-test
/tmp/pydoccraft-test/bin/python -m pip install --no-index --find-links dist pydoccraft
PYTHONPATH= /tmp/pydoccraft-test/bin/python -m unittest discover -s tests -v
```

The tests generate their PDF, DOCX, and PPTX fixtures locally. They require no office applications, external document binaries, cloud credentials, or runtime network access.

## What the suite must cover

- PDF, DOCX, and PPTX detection from paths and bytes;
- zero-based and negative page indexing;
- text, dictionaries, JSON, and Markdown;
- links and exact offsets where supported;
- semantic DOCX/PPTX tables and conservative PDF tables;
- referenced images and owned bytes;
- DOCX comments/replies and PPTX notes/comments;
- PNG rendering for all formats;
- PDF→DOCX, DOCX→PDF, and PPTX→PDF outputs reopened by `pydoccraft`;
- malformed PDF/XML/relationships/media behavior;
- OOXML path, member-count, size, aggregate-expansion, and compression-ratio limits;
- document close behavior and stable Python exceptions.

## Current validation baseline

The recorded 2026-10-19 pre-release run used Linux AArch64, Rust 1.95, and CPython 3.13:

| Check | Recorded result |
|---|---|
| Rust formatting | Passed |
| Rust tests | 3 passed |
| Clippy with warnings denied | Passed |
| Installed-wheel Python suite | 13 passed |
| Conversion reopen checks | PDF→DOCX, DOCX→PDF, PPTX→PDF passed |
| Extraction and PNG rendering | PDF, DOCX, PPTX passed |
| Locked license inventory | 262 normal/build packages; no GPL-/AGPL-/LGPL-only dependency |
| Wheel size | 13,198,994 bytes |
| Native extension | 28,648,840 bytes uncompressed |

This is a platform-specific baseline, not a release declaration. See `.kiro/specs/rust-document-engine/validation.md` for the full evidence and remaining checks.

## CI behavior

`.github/workflows/ci.yml` runs on pull requests and pushes to `main`. It installs Rust 1.95 and Python 3.11, checks formatting, runs locked Rust tests and Clippy, builds a release wheel, installs that wheel without consulting an index, and runs the Python suite.

The tagged release workflow builds wheels for Python 3.11–3.13 on Linux, macOS, and Windows. At present those matrix jobs perform an import smoke test; the complete suite runs in CI's Python 3.11 job.

## Release checklist

1. Set matching versions in `Cargo.toml`, `pyproject.toml`, and `src/doc_ex/__init__.py`.
2. Ensure requirements, design, tasks, and validation evidence are current.
3. Run all required checks against `Cargo.lock`.
4. Install and test the wheel in a clean Python 3.11 environment.
5. Reopen every conversion output.
6. Test representative real documents, including malformed and image-heavy inputs.
7. Audit the locked dependency graph and regenerate `THIRD_PARTY_LICENSES.md`.
8. Review `NOTICE` and all bundled asset attributions.
9. Scan the lockfile for prohibited UI/CLI/MCP/automation/OCR/JavaScript/print/signing packages.
10. Record wheel and native-extension sizes.
11. Confirm GitHub's Python 3.11 CI passes.
12. Confirm tagged Python 3.11–3.13 wheels build on all three operating systems.
13. Configure PyPI trusted publishing before creating the first release tag.
14. Tag exactly `v<project.version>`.

## Upstream changes

Upstream engine upgrades deserve isolated pull requests. Keep each project's crates on one revision, preserve root patches, rerun the whole corpus, compare size and licenses, and document intentional output changes. Never track an upstream branch.
