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
PYTHONPATH= /tmp/pydoccraft-test/bin/python tools/validate_installed.py --wheel-dir dist --output dist/validation.json
```

The harness verifies that the imported Python/native files match the supplied wheel, rejects source-tree imports, runs the corpus, and records test counts, platform, wheel/native sizes, hashes and pinned upstream revisions. Use a wheel directory containing exactly one artifact. The tests generate their PDF, DOCX, and PPTX fixtures locally. They require no office applications, external document binaries, cloud credentials, or runtime network access.

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

## Performance benchmarks

The canonical comparison is [PDF performance versus PyMuPDF](pdf-performance.md). It uses matched PDF text and rendering scenarios only. Historical broad capability baselines remain in [benchmarks.md](benchmarks.md) but are not used for performance claims against unrelated libraries. Benchmarks must use an installed release wheel, prepare inputs before timing, verify outputs outside timed regions, pin the comparison environment and distinguish matched operations from capability-only baselines. Raw timing output and generated documents belong outside the repository.

## CI behavior

`.github/workflows/ci.yml` runs on pull requests and pushes to `main`. It installs Rust 1.95 and Python 3.11, checks formatting, runs locked Rust tests and Clippy, builds a release wheel, installs that wheel without consulting an index, and runs the Python suite.

The tagged release workflow builds wheels for Python 3.11–3.13 on Linux, macOS, and Windows. Each matrix job runs the installed-wheel corpus and uploads validation evidence before its wheel can be published. A local Linux pass does not establish that those remote jobs have passed.

To validate the same nine jobs before creating a version tag, dispatch `release.yml` with `validation_only=true`, `publish=false`, and `release_tag` set to the exact 40-character commit SHA. Validation mode refuses branches, abbreviated SHAs and a simultaneous publication request. It builds and tests that source, uploads wheel/evidence artifacts, and skips both GitHub release creation and PyPI publication. Use the recorded commit and per-job validation results as evidence; a dispatched or successful build alone does not establish a passing corpus. The final tagged release still runs the matrix on the versioned source.

```bash
gh workflow run release.yml --ref main \
  -f release_tag=FULL_REVIEWED_COMMIT_SHA \
  -f validation_only=true -f publish=false
```

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

## Documentation website

The [GitHub Pages site](documentation-site.md) builds directly from these Markdown guides. Use the documented build and local-link check before changing site navigation or publishing configuration.
