# AGENTS.md

`pydoccraft` is a compact Python/Rust document extraction and conversion package. It exposes a PyMuPDF-style Python API through the compatible `doc_ex` import while delegating format behavior to selected upstream ArtCraft engine crates. The package is licensed under Apache-2.0 and must remain usable without external executables, runtime downloads, network access, or cloud credentials.

The active feature specification is [`.kiro/specs/rust-document-engine/`](.kiro/specs/rust-document-engine/). Keep requirements, design, tasks, and validation evidence synchronized with implementation changes.

## Non-negotiables

- **Use upstream format engines rather than reimplementing document standards.** PDF behavior belongs in PdfCraft adapters, DOCX behavior in WordCraft adapters, and PPTX behavior in DeckCraft adapters.
- **Do not link complete applications.** Never add ArtCraft desktop UI, CLI, MCP, automation server, updater, networking, printing, JavaScript, OCR-model, signing, or editor-state crates unless a reviewed requirement explicitly needs one.
- **Pin upstream Git dependencies to immutable commit revisions.** Never track `main`, a floating branch, or an unpinned tag.
- **Keep the runtime license graph permissive.** GPL, AGPL, and LGPL-only runtime dependencies are prohibited. Regenerate and review `THIRD_PARTY_LICENSES.md` whenever `Cargo.lock` changes.
- **Retain required attribution.** Update `NOTICE`, `THIRD_PARTY_LICENSES.md`, and its wheel copy `LICENSE-THIRD-PARTY` when adding or changing upstream crates, fonts, icons, CMaps, sample assets, or other attributed material.
- **Do not use ArtCraft, PdfCraft, WordCraft, or DeckCraft trademarks as this package's branding.** Names may be used only to describe upstream dependencies.
- **No subprocess document processing.** Extraction, rendering, and conversion run in-process through `doc_ex._core`.
- **No runtime downloads.** Wheels contain the native extension and all required code/assets.
- **Do not silently degrade or corrupt content.** Unsupported formats, encrypted files, unavailable conversions, and resource-limit failures must return stable explicit errors.
- **Do not claim pixel-perfect conversion.** Document the exact conversion matrix and known upstream fidelity limitations.

## Upstream revisions

Keep all crates from each repository on one revision:

| Project | Revision | Purpose |
|---|---|---|
| PdfCraft (`storytold/pdfcraft` main) | `64e0868b844e0230027a493c90d6e4bdc7af086f` | PDF inspection, text, links, images, rendering, DOCX export |
| WordCraft (`storytold/wordcraft` main) | `9d1067fc0e7f754978879bfd61351d359c9b39cb` | DOCX model, layout, rendering, PDF export |
| DeckCraft (`storytold/deckcraft` main) | `aaf6c1c38aa6c59beda8110bd014cc6c96c6ec74` | PPTX model, rendering, PDF export |
| craft-libs (`storytold/craft-libs`) | `9f75d3dc19b4b514c4f3d4847ed1ec1b0e86b4ef` | Shared PDF backend and patched Krilla dependency, one rev for all three |

PdfCraft, WordCraft and DeckCraft are pinned to immutable commits on the `main` branch of their original repositories (`storytold/*`). The pins carry no local patches and no contribution forks. Never pin a branch name; move the pin to a newer `main` commit in a dedicated upgrade.

**WordCraft is held back on purpose.** Its `main` has moved craft-libs on from `9f75d3dc` (to `17744430`, then `1e6f1278`, then `b926b5c0`) while DeckCraft `main` still pins `9f75d3dc`. All three engines must share one craft-libs backend, so WordCraft stays at `9d1067fc`, the newest commit that still uses `9f75d3dc` (it includes storytold/wordcraft#4092). Moving WordCraft further needs DeckCraft to move its craft-libs rev first; otherwise the lockfile holds two `craft-pdf` copies. Check this on every upgrade.

Behavior the earlier reviewed forks provided is handled in the adapters or is unsupported. Treat these as the current contract:

- **Underlay placement** (`behind=True`): `author_pdf::add_content_with_placement` adds the item with PdfCraft's public API, then moves its stream to the front of the page's `/Contents`.
- **Render panics:** DeckCraft draws a blank slide and counts the panic. `pptx::try_render_slide` and `pptx::export_pdf` turn that count into an explicit error. It is process-wide, so a panic on another thread can fail a call spuriously, but a panic can never pass silently.
- **DOCX core `language`:** rejected as an unknown metadata key. WordCraft has no `dc:language`.
- **Clearing the DOCX default font size:** supported. WordCraft writes the resolver's 11 pt fallback when the default is unset (storytold/wordcraft#4092), so the effective size survives a save and reopen. The reader still reads a file that omits the size as 10 pt, so an imported DOCX without a default size renders at 10 pt (an 11 pt document before this change); that reader/resolver mismatch is open upstream.
- **PDF export of very thick shape strokes:** WordCraft's PDF output uses square joins, so corners differ from the raster by up to the stroke width. Recorded in `docx/stroke_regression_tests.rs`.
- **Known upstream defect:** DeckCraft's PDF export builds paragraph text without line-break characters, so searchable text is corrupted for PPTX paragraphs containing explicit breaks (`paragraph.add_break()`). Native PDF export of such documents is not supported until upstream fixes it.
- Slide copy and retained extended document properties are not available and have no production caller.

Use the `update-upstream-crates` skill (`.kiro/skills/update-upstream-crates/SKILL.md`) for upgrades.

An upstream upgrade is a dedicated change: update every crate from that repository together, regenerate `Cargo.lock`, rerun the license audit, run the complete corpus, measure wheel size, and record results in the spec validation file.

## Architecture

```text
src/doc_ex/
  __init__.py       Python facade, format detection, native error mapping
  _document.py      Document/Page compatibility API and Markdown rendering
  models.py         Public Block/Table/Link/Image values
  errors.py         Stable public exceptions
rust/src/
  lib.rs            PyO3 boundary and operation dispatch
  model.rs          Normalized serializable payload
  pdf.rs            Thin PdfCraft adapter
  docx.rs           Thin WordCraft adapter
  pptx.rs           Thin DeckCraft adapter
  convert.rs        Upstream-backed conversion dispatch
```

The native boundary stays narrow:

```python
_core.extract(data, filetype, password=None) -> JSON string
_core.convert(data, source_format, target_format, password=None) -> bytes
_core.render(data, filetype, page, dpi=144, password=None) -> PNG bytes
```

Python owns compatibility objects and Markdown formatting. Rust owns parsing, model traversal, rendering, and conversion. Do not expose upstream Rust types directly to Python.

## Public API contract

Preserve these interfaces unless the specification explicitly changes them:

```python
doc_ex.open(path_or_bytes, filetype=None, password=None) -> Document
doc_ex.convert(path_or_bytes, output_format, filetype=None, password=None) -> bytes

len(document)
document.page_count
document.load_page(index)
document[index]
document.to_markdown()
document.render_page(index, dpi=144)
document.convert(output_format)
document.close()

page.number
page.get_text("text" | "markdown" | "dict" | "json")
page.get_links()
page.get_images(full=False)
page.find_tables()
page.get_pixmap(dpi=144)
```

Page indexes are zero-based. Negative document/page indexes follow normal Python behavior. Returned model values own their data and remain usable after the source bytes or temporary caller objects disappear.

## Normalization rules

- Preserve source order for pages, blocks, paragraphs, runs, tables, notes, and comments.
- Keep text links attached to their exact labels and character offsets.
- Keep images attached to the page/block that references them; do not emit unreferenced OOXML media.
- Preserve semantic tables. Nested tables must not be flattened ambiguously.
- DOCX headings use resolved style inheritance, not style-name guesses.
- DOCX page mapping uses WordCraft layout plus explicit page breaks. State limitations when an automatically split paragraph cannot be represented exactly per page.
- PPTX returns one page per slide and recursively walks grouped shapes.
- PDF uses PdfCraft reading order and coordinate conversion. PDF table extraction may retain only a small geometry normalizer because PdfCraft does not expose its table detector publicly.
- Never duplicate table text in plain text, Markdown, or converted output.
- Rendering returns PNG bytes and validates page index, DPI, and output dimensions.

## Dependency discipline

- Add only a crate needed by a current acceptance criterion.
- Use exact versions for crates.io dependencies and immutable `rev` values for Git dependencies.
- Prefer lower-level public crates over `*-engine` umbrella crates when the umbrella pulls prohibited or unrelated systems.
- Root workspace patches required by upstream repositories must be copied into this root `Cargo.toml`; dependency-workspace patches do not propagate.
- Keep `Cargo.lock` committed.
- Do not edit vendored/upstream source to make integration easier. Prefer a public upstream API contribution; otherwise isolate a minimal adapter workaround and document it.

## Safety

Documents are untrusted input.

- Preserve upstream parser limits and add stricter boundary limits where needed.
- Bound input size, page/slide count, image dimensions, decoded image bytes, JSON payload size, recursion, and archive expansion.
- Reject OOXML traversal paths, absolute relationship targets, excessive members, oversized members, excessive aggregate expansion, and suspicious compression ratios.
- Avoid panics on document data. Return `EngineError` and map it to a stable Python exception.
- Native code must not perform network requests or write caller-controlled paths.
- Do not deserialize unbounded structures merely to simplify adapters.

## Change discipline

- Keep adapters thin. If a format algorithm already exists upstream, call it.
- Do not add speculative abstractions or unrelated conversion pairs.
- Do not mix an upstream revision upgrade with unrelated API work.
- Do not weaken tests to accommodate upstream differences; document intentional changes and update golden expectations deliberately.
- Do not retain generated `target/`, wheel, cache, virtualenv, or Python bytecode files as source changes.

## Verification

The host may not have Rust installed. Use a Rust 1.95+ Docker image when necessary. Validation must use the committed lockfile.

```bash
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
maturin build --release --locked
python -m unittest discover -s tests -v
```

Before declaring a release candidate complete:

1. Install the built wheel into a clean Python 3.11 environment.
2. Run all tests against the installed wheel, not the source tree.
3. Reopen every converted output.
4. Verify PDF, DOCX, and PPTX extraction and PNG rendering.
5. Audit the locked runtime license graph and regenerate `THIRD_PARTY_LICENSES.md`.
6. Confirm no prohibited ArtCraft engine/UI/CLI/MCP/automation/OCR/JS/print/signing packages are linked.
7. Record wheel size, uncompressed extension size, test counts, platform, upstream revisions, and limitations in `.kiro/specs/rust-document-engine/validation.md`.

A successful build alone is not sufficient evidence of extraction or conversion correctness.
