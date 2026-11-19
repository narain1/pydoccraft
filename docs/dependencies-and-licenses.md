# Dependencies, licensing, and attribution

`pydoccraft` is licensed under the Apache License 2.0. Distributed source and wheels must include [`LICENSE`](../LICENSE), [`NOTICE`](../NOTICE), and [`LICENSE-THIRD-PARTY`](../LICENSE-THIRD-PARTY), the wheel-compatible copy of [`THIRD_PARTY_LICENSES.md`](../THIRD_PARTY_LICENSES.md).

This document describes project policy and distribution mechanics, not legal advice.

## Upstream engines

The package links selected library crates rather than complete ArtCraft applications:

| Project | Pinned revision | Responsibility |
|---|---|---|
| [PdfCraft](https://github.com/storytold/pdfcraft) | `03996b10258288e723c4e07334c9cd1bae1f68a8` | PDF inspection, reading-order text, links, images, rendering, DOCX export primitives |
| [WordCraft](https://github.com/storytold/wordcraft) | `bbe3547dd11bb5f613f3aa893b12f593910644de` | DOCX model, layout, rendering, PDF export |
| [DeckCraft](https://github.com/storytold/deckcraft) | `f7a7fd7f5ffda19758584908730216222e387aa6` | PPTX model, rendering, PDF export |
| [craft-libs](https://github.com/storytold/craft-libs) | `92d392cff132eaf53e13afff77f38b5c11b73aa0` | Shared PDF backend and patched Krilla dependency |

PdfCraft, WordCraft and DeckCraft are pinned to immutable commits on the `main` branch of their original repositories (`storytold/*`). The pins carry no local patches and no contribution forks. Never pin a branch name; move the pin to a newer `main` commit in a dedicated upgrade.

**Upstream revisions are held back on purpose.** PdfCraft `03996b10` predates a CropBox extraction change that omits off-page text previously extracted. WordCraft `bbe3547d` predates a separate craft-libs proofing revision, and DeckCraft `f7a7fd7f` is the newest first-parent commit sharing their PDF backend. Both use craft-libs `92d392cf` for `craft-pdf` and Krilla. Newer upstream heads diverge on shared revisions; revisit these holds after the extraction contract and dependency convergence are resolved.

On 2026-11-15, the checked upstream heads were PdfCraft `e191b0ea69b7dcc4e554c1c4e51b6214bec1c590`, WordCraft `cc90520b9d8fb0d1e6be2c2688e6f915319e850a`, DeckCraft `bb2c2254ee60761b2b4dab2053fe33d7190b6804`, and craft-libs `58964adec11197594d9b6b79a94ae6232221211a`. Their PDF dependencies pin craft-libs at `e2dc681a`, `6e03f65b`, and `63196e66` respectively. Current WordCraft additionally pins its proofing library separately, and current DeckCraft pins other shared crates separately. Linking these heads would therefore violate the single-revision rule; root workspace patches cannot make the upstream Git revisions converge without changing the engines. PdfCraft also requires its own Vello patches at newer revisions, and its CropBox text filtering needs a deliberate compatibility decision before adoption.

Behavior the earlier reviewed forks provided is handled in the adapters or is unsupported. Treat these as the current contract:

- **Underlay placement** (`behind=True`): `author_pdf::add_content_with_placement` adds the item with PdfCraft's public API, then moves its stream to the front of the page's `/Contents`.
- **Render panics:** DeckCraft draws a blank slide and counts the panic. `pptx::try_render_slide` and `pptx::export_pdf` turn that count into an explicit error. It is process-wide, so a panic on another thread can fail a call spuriously, but a panic can never pass silently.
- **DOCX core `language`:** rejected as an unknown metadata key. WordCraft has no `dc:language`.
- **Clearing the DOCX default font size:** supported. WordCraft writes the resolver's 11 pt fallback when the default is unset (storytold/wordcraft#4092), so the effective size survives a save and reopen. The reader still reads a file that omits the size as 10 pt, so an imported DOCX without a default size renders at 10 pt (an 11 pt document before this change); that reader/resolver mismatch is open upstream.
- **PDF export of very thick shape strokes:** WordCraft's PDF output uses square joins, so corners differ from the raster by up to the stroke width. Recorded in `docx/stroke_regression_tests.rs`.
- **Known upstream defect:** DeckCraft's PDF export builds paragraph text without line-break characters, so searchable text is corrupted for PPTX paragraphs containing explicit breaks (`paragraph.add_break()`). Native PDF export of such documents is not supported until upstream fixes it.
- Slide copy and retained extended document properties are not available and have no production caller.

Git dependencies use immutable `rev` values. Do not replace them with branches or floating tags.

## Deliberately excluded components

The runtime does not link ArtCraft desktop applications or their CLI, MCP, automation, UI, updater, networking, OCR, JavaScript, printing, or signing packages. Lower-level rendering and font dependencies required by the selected crates remain part of the native graph.

`pdfcraft-engine` is deliberately excluded because its un-gated dependency graph includes unrelated application capabilities. PDF→DOCX instead maps public PdfCraft extraction values into the smaller `pdfcraft-export` writer. Native PDF authoring uses the pinned `pdfcraft-create`, `pdfcraft-edit`, `pdfcraft-organize`, `pdfcraft-annot`, `pdfcraft-content` and `pdfcraft-fonts` libraries. `tools/audit_licenses.py` checks normal/build dependency traversal, reviewed SPDX choices and prohibited application packages against locked Cargo metadata.

## Package license

Project source uses Apache-2.0 as declared in:

- `Cargo.toml`;
- `pyproject.toml`;
- `LICENSE`;
- wheel metadata generated by Maturin.

Apache-2.0 redistribution requires preservation of applicable notices and identification of modified upstream-derived files where required. The package does not use ArtCraft, PdfCraft, WordCraft, or DeckCraft names or logos as its own branding.

## Locked dependency inventory

`THIRD_PARTY_LICENSES.md` is generated from the committed `Cargo.lock` and reviewed against the final normal/build graph. The current recorded audit covered 300 packages and found no undeclared or GPL-, AGPL-, or LGPL-only runtime dependency.

An SPDX `OR` expression is acceptable when at least one available license is allowed and the project complies with that choice. Bundled assets—such as fonts and CMaps—must be reviewed separately from crate metadata because asset licenses may not be represented by Cargo's package license field.

## NOTICE and assets

`NOTICE` must be reviewed whenever the dependency graph changes. Current categories include:

- upstream ArtCraft project attribution;
- PdfCraft/PDFium/Foxit standard-font notices;
- Adobe CMap notices;
- bundled font licenses, including applicable SIL Open Font License and Ubuntu font terms;
- trademark disclaimers.

`NOTICE` identifies the linked PdfCraft, WordCraft, DeckCraft, and craft-libs revisions and records the applicable bundled-font, CMap, hyphenation-data, and ICC-profile notices. Re-run this review whenever the locked graph or upstream revisions change; a passing crate-license scan alone does not establish asset compliance.

## Upgrading an upstream engine

Use a dedicated dependency upgrade:

1. Update every direct crate from that repository to one immutable revision.
2. Update root `[patch.crates-io]` entries required by the upstream workspace; dependency-workspace patches do not propagate.
3. Regenerate `Cargo.lock`.
4. Run the full extraction, rendering, and conversion fixture suite.
5. Test representative production documents.
6. Regenerate `THIRD_PARTY_LICENSES.md` from the final locked graph.
7. Compare upstream `LICENSE`, `NOTICE`, `ATTRIBUTION`, and bundled assets.
8. Update this document and root `NOTICE`.
9. Measure wheel and native-extension size changes.
10. Record commands and results in `.kiro/specs/rust-document-engine/validation.md`.

Do not combine an upstream revision change with unrelated API behavior.
