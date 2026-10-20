# Contributing to pydoccraft

Thank you for helping improve pydoccraft. Contributions should keep the package compact, safe for untrusted documents, and usable without external executables, runtime downloads, network access, or cloud credentials.

## Before opening a change

- Use GitHub Discussions or an issue for broad design proposals.
- Search existing issues before filing a new one.
- Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md), not in a public issue.
- Keep pull requests focused. Upstream engine revision upgrades must be isolated from unrelated API or documentation work.

## Development setup

You need Python 3.11 or newer, Rust 1.95 or newer, and Maturin 1.8.1. The host may use a Rust 1.95+ container when Rust is unavailable locally.

```console
python -m venv .venv
source .venv/bin/activate
python -m pip install maturin==1.8.1
maturin develop --locked
python -m unittest discover -s tests -v
```

Read [docs/development.md](docs/development.md) for the complete build and release process. Repository invariants and adapter boundaries are documented in [AGENTS.md](AGENTS.md).

## Required checks

Run the checks relevant to your change. Before requesting final review, the full validation set should pass:

```console
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
maturin build --release --locked --out dist
python -m unittest discover -s tests -v
```

For package-facing changes, install the built wheel into a clean Python environment and run the tests against that installation rather than the source tree. Reopen converted outputs when changing conversion behavior.

## Document-engine changes

- Use the pinned upstream format engines rather than reimplementing PDF, DOCX, or PPTX standards.
- Keep Rust adapters thin and the Python/native boundary narrow.
- Do not add desktop UI, CLI, MCP, automation, updater, networking, printing, JavaScript, OCR-model, signing, or editor-state crates.
- Pin crates.io dependencies exactly and Git dependencies to immutable commit revisions.
- Keep every crate from one upstream repository on the same revision.
- Treat all documents as untrusted input and preserve existing parser/resource limits.
- Return explicit stable errors instead of silently dropping, degrading, or corrupting content.

If `Cargo.lock` changes, regenerate and review `THIRD_PARTY_LICENSES.md` and its packaged `LICENSE-THIRD-PARTY` copy. GPL-, AGPL-, and LGPL-only runtime dependencies are prohibited. Update `NOTICE` for changed upstream crates, fonts, icons, CMaps, sample assets, or other attributed material.

## Tests, documentation, and specifications

Add or update tests when behavior changes. Do not weaken assertions to accommodate an unexplained upstream difference. Generated fixtures must remain self-contained and distributable.

Update public documentation for API, format support, safety, or fidelity changes. Keep `.kiro/specs/rust-document-engine/` requirements, design, tasks, and validation evidence synchronized with implementation changes.

Do not commit generated `target/`, wheel, cache, virtual environment, native binary, or Python bytecode files.

## Pull requests

A pull request should:

1. Explain the problem and the chosen solution.
2. Link related issues where applicable.
3. Describe user-visible behavior and compatibility impact.
4. List the checks run and their results.
5. Call out changes to dependencies, licenses, attribution, safety limits, or conversion fidelity.
6. Avoid unrelated formatting or refactoring.

Maintainers may ask for a smaller change or an upstream contribution when behavior belongs in PdfCraft, WordCraft, or DeckCraft.

## Contribution license

Unless explicitly stated otherwise, contributions intentionally submitted to this repository are provided under Apache License 2.0, consistent with section 5 of that license. Do not submit material you do not have the right to contribute.
