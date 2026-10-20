# Validation evidence

## 2026-10-20 sections, slide tables and template recipes (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1. Formatting, 86 Rust tests, warnings-denied Clippy, locked release build and 107 installed-wheel Python tests passed; zero skips and exact installed artifact identity.
- Native and Python tests verify per-section PNG/PDF dimensions/order, parity and blank sections, independent header/footer copies and absent-story isolation, section-owned width validation and atomic invalid geometry. PPTX coverage verifies native merge/split/grid edits, anchor migration, covered aliases, stale handles, stored box synchronization, minimum-row expansion, explicit border clearing, scalar/aggregate-cell limits and reopened/rendered/exported output. Cell margin ordering, direct alignment/reset, mixed values and nested-width protection are tested in DOCX.
- The executable template/batch recipe verifies cross-run body/header replacement, preserved unrelated links, missing/unused/invalid-value rejection, explicit output saves and unchanged original template across independent DOCX/PDF outputs. Both reviewed PR21 link corrections are included and regression-tested.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 16,789,517 bytes; native extension: 38,903,512 bytes. SHA-256: `efbcc3186ce956b1d2558abbd6416c506905ac42493ac831ea1a11cae8b162da`. Lockfile SHA-256: `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`. All immutable upstream revisions, 263-package audited graph and attribution assets remain unchanged.
- Next-column and mixed-size continuous DOCX transitions, linked stories, unequal columns, arbitrary source-template preservation, DOCX grid CRUD and inherited/mixed PPTX border resolution remain explicitly unavailable. Minimum PPTX rows may render beyond stored box height. Remaining unchecked roadmap items are not completed by this batch; this is not a release declaration.


## 2026-10-20 PR21 independent review corrections (unreleased)

- Reviewed runtime head `00c7f237aed55ca51b7346ad7858625778208bbf` against merged main `76881093b7966b45389e9c4d811ecdb9c185aa47`. [CI run 39450961572](https://github.com/narain1/pydoccraft/actions/runs/39450961572) passed formatting, 76 Rust tests, warnings-denied Clippy, the locked 263-package license audit, the release build and 99 installed-wheel Python tests with zero skips.
- Independent review corrected two PPTX link defects: explicit range assignment could not replace/clear mixed links, and invalid XML characters in URLs were silently removed on save. Reads still reject mixed targets; assignment preserves a uniform tooltip and clears mixed tooltips. Existing native text validation now rejects U+FFFE/U+FFFF before mutation. Regressions verify stale/invalid ranges, failed-edit atomicity and saved link targets.
- CI platform: Linux x86_64, Rust 1.95.0 and Python 3.11.17. Wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`: 16,704,307 bytes; uncompressed extension: 38,704,080 bytes. Wheel SHA-256: `88067a0e3af05a1cdfebef285ce53a183c423c78aaeff81014aeb379a007cda8`. The installed-wheel harness verified artifact identity.
- Lockfile SHA-256 remains `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`; all four pinned upstream revisions, dependencies and attribution assets are unchanged. Broader roadmap and format limitations remain open.

## 2026-10-20 styles, slide ranges and portable Markdown (unreleased)

- Offline Linux x86_64, Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1. Formatting, 75 Rust tests, warnings-denied Clippy and the locked release build passed. Installed-wheel validation passed 97 Python tests with zero skips and matching installed package members.
- New evidence covers style graph/name validation, native inheritance/reset/false semantics, direct/resolved snapshots, omitted run-property inheritance, table-style spacing, every supported DOCX underline/highlight enum, supplied formatting persistence and DOCX PNG/PDF output. PPTX tests cover Unicode/run-boundary and paragraph replacement, exact URL labels, tooltip persistence, native internal PDF destinations after reorder, referenced-target deletion rejection, copied owners, failed-edit rollback and property/fragmentation limits.
- Markdown export tests cover actual PDF/DOCX/PPTX extraction, separate story files, nested HTML cells, content deduplication with repeated placements, safe names/escaped image attributes, cross-page inventory suppression, closed lifetimes, missing media, cyclic/deep/ragged tables and byte/node budgets. Existing extraction/conversion/render suites remain passing.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 16,703,209 bytes. Native extension: 38,700,424 bytes. Wheel SHA-256: `e6380ff66348ce6616b6001177cd80cde6292ff4346e87cb27953646c7598331`. Lockfile SHA-256: `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`. All four immutable upstream revisions and the reviewed 263-package license graph remain unchanged; no new code or attribution assets were added.
- Table/theme/next-linked styles, general Office source editing, rich PPTX table-cell ranges, full internal-link extraction and broader unchecked roadmap criteria remain open. PPTX run shading is explicitly unsupported; highlighting is distinct. Markdown is a bounded extraction package, not layout reproduction or an HTML sanitizer. This is not a release declaration.


## 2026-10-20 text/vector content and diagnostics (unreleased)

- Validated offline on Linux x86_64 with Rust 1.95.0, CPython 3.11.2 and Maturin 1.8.1; all four upstream revisions remain unchanged.
- Formatting, 63 Rust tests, warnings-denied Clippy and the locked release build passed. Installed-wheel validation passed 85 Python tests with zero skips and verified imported package members against the artifact.
- Coverage includes DOCX lists/links, explicit initial/consecutive blank pages, native report DOCX/PDF order and single table text, supplied-font searchable Unicode and visible glyphs, vector pixels/invalid-edit rollback, metadata/bookmark persistence, nonmutating absent-story lookup, stable originating child collections, bounded inputs/JSON and structured extraction diagnostics. Existing PDF/DOCX/PPTX extraction/render/conversion checks remain passing.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 16,600,451 bytes. Native extension: 38,425,016 bytes. Wheel SHA-256: `390dbe8015a307f7512d94741070c50592fbe3ae43aed37acb5aaecc21cb8a84`. Lockfile SHA-256: `07d283b629c33e1994e4bd2b9e3e8f16876ee5efda73c46428581e20869fcdf8`.
- `pdfcraft-content` and `pdfcraft-fonts` become direct dependencies at the same pinned revision; both were already transitive. The regenerated/reviewed normal/build inventory remains 263 packages, with no prohibited application crates or non-permissive-only license. NOTICE and the adjacent complete OFL license cover the unmodified Inter regression fixture from pinned PdfCraft; the runtime asset set is unchanged.
- Diagnostics cover detectable native model constructs, not unknown markup discarded by upstream readers. Full conversion diagnostics, arbitrary Office source editing, complex-script conformance, advanced styles/sections/fields/review, attachment resource controls and unchecked roadmap criteria remain open. This is feature validation, not a release declaration.


## 2026-10-20 PR19 independent review corrections (unreleased)

- Reviewed runtime commit `c00e2dee4e1659668ebfb0a2939370020de083d3` against merged main `2340edd867f40cf43b33a5017c190ca01b1d769d`. [CI run 39447521531](https://github.com/narain1/pydoccraft/actions/runs/39447521531) passed formatting, all 45 Rust tests, warnings-denied Clippy, the locked license audit (263 normal/build packages), release wheel build and all 71 installed-wheel Python tests with no skips.
- Review found and corrected dropped reference-only paragraph anchors, lost leading/consecutive blank pages from explicit page breaks, and repeated paragraph-prefix scans while normalizing references, links and formatting. Regressions cover note-only/text-box-only anchors, native DOCX and PDF blank-page positions, and long Unicode prefixes with many objects, links and formatting runs. The normalization changes use public native ordered objects/run ranges and retain exact character offsets.
- CI platform: Linux x86_64, Python 3.11.17, Rust 1.95.0; wheel `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`. Wheel size: 16,420,760 bytes; uncompressed native extension: 38,038,256 bytes. Wheel SHA-256: `56ffc0203f45b3d4985d8481b02d1c5e776331084ddb8de0f887e6dc709ba8a4`. Installed package identity was checked against that artifact.
- Lockfile SHA-256 remains `1b2bcaa846d1ffe42cc844fce299750cf9c288e1aab00a3e8aafc40b7a14b3c9`; all four pinned upstream revisions and the runtime dependency graph are unchanged. These corrections do not complete other open roadmap criteria or establish cross-platform visual fidelity.

## 2026-10-20 DOCX layout controls and auxiliary stories (unreleased)

- Same offline Rust 1.95.0/Linux x86_64/Python 3.11.2 environment and unchanged locked upstream revisions as the authoring baseline.
- Formatting, 41 full Rust tests and warnings-denied Clippy passed. Clean installed-wheel validation passed 68 Python tests with no skips, including the separately reviewed PPTX CR/CRLF regression.
- New coverage verifies DOCX rich auxiliary header/footer/note/comment stories, source reference connections, owned values after close, nested-story grid/depth limits, and page-color pixels in both native PNG and PDF-rendered output. Table column widths/borders/row properties round-trip; explicit breaks yield three correctly ordered PDF pages.
- Wheel: 16,419,239 bytes; uncompressed native extension: 38,033,224 bytes. Wheel SHA-256: `40664a866303fab98964c30c095a654d065924132507031853fbf75356406e34`. Lockfile SHA-256 remains `1b2bcaa846d1ffe42cc844fce299750cf9c288e1aab00a3e8aafc40b7a14b3c9`.
- Story order is native part-ID order, not an invented original archive ordering. Auxiliary objects have no guessed page assignment. Legacy comments remain in page output for compatibility and are flagged when also exposed as stories. General Office source editing, multiple-section mutation, advanced fills and other unchecked roadmap criteria remain open.


## 2026-10-20 native authoring baseline (unreleased)

- Built offline in the Rust 1.95.0 Debian container on Linux x86_64; Python 3.11.2, Maturin 1.8.1. All builds used `--locked`; upstream revisions remain unchanged from AGENTS.md.
- `cargo fmt --all -- --check`, `cargo test --locked` (37 tests), and `cargo clippy --locked --all-targets -- -D warnings` passed.
- Clean installed-wheel validation passed all 61 Python tests with no skips. Package files were verified against the wheel; DOCX/PDF/PPTX authored outputs and conversions were reopened. Tests cover failed-edit atomicity, native ownership/copies, stale handles, rich/nested cells, header/footer PDF text, page/slide organization, shape/background/media retention, PDF annotation CRUD, exact PDF link labels and rotated/cropped/UserUnit raster geometry.
- Wheel: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`; 16,382,442 bytes. Native extension: 37,941,224 bytes. Wheel SHA-256: `9e96852313dd58f2aeafba2f1da412b0c9f787f0bb55fa8a87c7c70e89487bdc`. Lockfile SHA-256: `1b2bcaa846d1ffe42cc844fce299750cf9c288e1aab00a3e8aafc40b7a14b3c9`.
- Reviewed normal/build graph: 263 packages; only `pdfcraft-organize` is newly added to the graph. `pdfcraft-annot` was already transitive and is now direct. Both use the existing immutable PdfCraft revision. Regenerated/reviewed both license inventories and NOTICE; no prohibited application crate or non-permissive-only license was found. CI now checks the generated inventory.
- The initial PPTX invalid-character test was corrected after source audit: U+FFFC is valid literal DeckCraft text, unlike WordCraft's object placeholder. A native save/reopen test verifies it; NUL/U+FFFE/U+FFFF remain rejected. Annotation geometry assertions allow upstream f32 rounding within 0.0001 point, with failed resize preserving geometry.
- Supported scope and explicit limits are in `docs/authoring.md`. Existing source DOCX/PPTX edits are refused; source PDF edits refuse encrypted/form/signed/certified profiles. PDF text remains Helvetica/WinAnsi; Office image authoring currently accepts PNG. Annotation creation supports notes/highlights/links and does not implement redaction. Header/footer authoring/export is verified, but DOCX story extraction remains separate work. Advanced roadmap coverage is not marked complete by this baseline.


## 2026-10-20 PR preparation against updated main

Rebased the completed roadmap foundation and feature commits onto `f0baff5` (community guidelines and package URL metadata). Repeated formatting, locked Rust tests (5 passed), Clippy, the locked release build, and all 36 Python tests in a new clean CPython 3.11 environment. Container networking remained disconnected. The runtime dependency graph and upstream revisions are unchanged.

The resulting Linux x86_64 wheel is 14,411,717 bytes, with a 33,068,744-byte native extension. SHA-256: `d216dea47b1882808df2984cc8e0a8cd7f2f7ad317bac487a3e4179b69385cd8`. The installed-wheel identity check passed. This supersedes the pre-rebase artifact below for PR review; the broader authoring workstreams remain open.

## 2026-10-20 first agent implementation batch (unreleased)

Three agents covered the W01/W03/W04/W07/W08, W02/W06/W10/W11, and W09/W05/W12/W13 portfolios; the integrator owned W14. Implementation in this working tree is limited to rendering metadata/bounds, recursive DOCX extraction, roadmap validation and the installed-wheel harness. Dependent authoring portfolios have pinned-source readiness audits, not completed runtime features. No new `new`, `edit`, or save API is advertised.

Validation used Rust 1.95.0 in `rust:1.95-bookworm`, CPython 3.11.2, Maturin 1.8.1 and Linux x86_64. The committed `Cargo.lock` and all four upstream revisions remain unchanged. The final installed-wheel corpus ran with the container disconnected from its Docker network.

| Check | Result |
|---|---|
| Formatting, `cargo test --locked`, Clippy with warnings denied | Passed; 5 Rust tests |
| Release wheel build with committed lockfile | Passed |
| Clean installed-wheel suite | 36 passed, zero skipped: 28 runtime/API tests and 8 roadmap-validator tests |
| DPI metadata and dimensions | PDF/DOCX/PPTX at 72, 144, 150.5 and 300 DPI; decoded PNG data and representative visible output checked |
| Render errors/lifetimes | Invalid DPI, oversize requests, indexes and closed documents checked; intentional blank DOCX page remains valid |
| Recursive tables | Nested content, source order, exact links, images, merged cells including omitted leading grid cells, cross-page fallback deduplication, owned lifetime and 16/17-level boundary checked |
| Conversion reopen | Existing three advertised pairs plus nested DOCX-to-PDF no-duplication assertion passed |
| Artifact identity | Harness compares installed package/native members with wheel; deliberately mismatched wheel rejected before tests/evidence output |
| Coverage ledger | 139 IDs, 14 workstreams, unique owners, valid roles/upstream references, acyclic dependencies |
| Attribution | Wheel LICENSE/NOTICE/LICENSE-THIRD-PARTY match source; third-party wheel copy matches reviewed inventory |
| Repository checks | Workflow YAML parses; `git diff --check` passes |

Artifact: `pydoccraft-0.2.1-cp311-cp311-manylinux_2_35_x86_64.whl`, 14,411,386 bytes; native extension 33,068,744 bytes. Wheel SHA-256: `22d11eb7e734f41339b576fe2445dfa17db77228cca4348bd5d4d6814028e5d5`. Lockfile SHA-256: `36084d8fdc39107d12a67bcb28e8454d919e8ee888828992e90a75c2ea81e13d`. Machine-readable evidence is generated at `dist/validation.json` and remains a build artifact. The previous size baseline was AArch64, so these sizes are not a same-platform regression comparison.

Review corrected a PyO3 module-name collision, cross-page table asset duplication and omitted-grid merge positioning. WordCraft adds a required empty paragraph after a nested table; the structural regression deliberately retains and asserts that normalization instead of dropping empty paragraphs.

R3 is complete for this tested working-tree artifact. R2/R4 remain open for rotated/landscape/nonstandard geometry coverage and the upstream DeckCraft silent blank-on-panic contract. E1 remains partial for broader PPTX/PDF semantics and pagination coverage. The 16-level table guard applies to DOCX body extraction normalization, not all parser/stories/operations. No dependency upgrade, new runtime library or upstream issue was introduced. Remote CI/release matrices, cross-platform visual baselines and broad authoring acceptance remain unverified; this is not a release declaration.

GitHub progress was read back after updating all 14 issues: each remains open, 139 coverage IDs remain unique, and only R1/R3 are checked. Project W01/W02/W09/W14 are In Progress; dependent authoring work remains Todo with source-audit findings attached. Issue notes explicitly identify these changes as local, uncommitted and unreleased. Local documentation checks covered 73 file links and parsed the readiness-document Python snippets.

## 2026-10-20 GitHub roadmap issue publication

- Published and read back 14 canonical roadmap issues in `narain1/pydoccraft`, numbers 3–16. The [workstream map](../../../docs/roadmap-workstreams.md) links each issue; its JSON records the issue URLs.
- Verified issue bodies against the generated scope/checklists: all 139 coverage IDs are owned exactly once, preserving 138 open goals and existing rendering capability R1. Added priorities, backend labels, qualified upstream-candidate labels, related-work links and prerequisite baselines.
- Used native GitHub blocking relationships only for whole-issue API-contract prerequisites. Existing rendering and extraction work remain independent; partial prerequisites are explicit body links rather than broad blockers.
- Local checks passed: 61 local links/anchors across 14 Markdown files, draft Python example syntax, exhaustive unique coverage, acyclic workstream dependencies and whitespace checks. No runtime tests or native builds were rerun for these planning changes.
- After user authorization, created [pydoccraft Roadmap, project 8](https://github.com/users/narain1/projects/8) and linked it to the repository. The project retains GitHub's private visibility default.
- Added all 14 issue workstreams exactly once and read back every item's status, priority, work kind, ownership and upstream-candidate fields. Verified the repository link, project README, Backlog table, Roadmap timeline and filtered Upstream work view. Start/target dates remain unset; no delivery commitments were invented.

## 2026-10-19 overlap, API surface and upstream source review

- Three explicitly requested agents reviewed duplicated scope, public API breadth and dependencies, then inspected the pinned WordCraft/PdfCraft/DeckCraft repositories through GitHub APIs.
- Consolidated all 139 coverage IDs into 14 canonical workstreams, each ID with one owner. Higher-level workflows, wrappers and data conveniences were reduced to recipes or acceptance checks; existing API contracts remain unchanged.
- Recorded source evidence and ownership in [upstream-roadmap.md](../../../docs/upstream-roadmap.md). Writers and many model/edit/background operations already exist. Narrow verified candidates include PdfCraft's private table detector, DeckCraft's sized blank-image fallback on caught render panics, and WordCraft formatting-revision algorithms located in engine code.
- This is source inspection and planning, not new runtime evidence. No native behavior/dependencies/upstream revisions changed and no issue was filed in an upstream repository.

## 2026-10-19 Open XML feature-parity planning (documentation only)

- Reviewed Microsoft's `dotnet/Open-XML-SDK` repository README, `WordprocessingDocument.cs`, WordprocessingML schema inventory, and official feature references for text/styles, tables/cells, backgrounds, sections, fields, content controls, revisions and validation. Reviewed Pillow's Image API for ergonomic conventions. Source links and review scope are recorded in [authoring-api-design.md](../../../docs/authoring-api-design.md).
- Expanded the feature roadmap with grouped acceptance TODOs and milestones for a Pythonic authoring/editing API. Added draft creation, table/background, edit-copy, PDF and slide workflows; all are explicitly marked unimplemented.
- Recorded the sole-backend policy: WordCraft/PdfCraft/DeckCraft public library APIs. Open XML SDK is a feature reference, not a runtime or required validation dependency. No upstream feature availability was inferred from Open XML support.
- Added explicit per-operation coverage requirements for read/create/edit/preserve/render/export and blockers for missing upstream APIs. Rendering existing files remains implemented; new authoring capabilities have not been implemented or runtime-validated.
- Updated requirements to distinguish permitted programmatic library editing from prohibited desktop editor-state dependencies. No source code, dependency, lockfile, license, attribution asset or upstream revision changed in this planning extension.
- Documentation validation passed: 38 local links/anchors, balanced code fences across nine Markdown files, syntax checks for six Python examples, 139 unique roadmap IDs (138 open, one implemented), and whitespace checks. Draft authoring examples were not executed; no native builds or runtime tests were rerun for these documentation changes.

## 2026-10-19 feature roadmap review (documentation only)

- Reviewed the Python facade, native dispatch and all three rendering adapters, conversion dispatch, existing tests, and format documentation.
- Confirmed source implementations of `Document.render_page(index, dpi=144)` and `Page.get_pixmap(dpi=144)` for PDF, DOCX, and PPTX, including finite 1–1200 DPI validation and `dpi / 72` scaling.
- Existing tests assert PNG signatures, default/72 DPI behavior, negative indexes, and zero-DPI rejection. They do not establish multi-DPI dimensions, visual fidelity, or consistent PNG physical-resolution metadata; these remain roadmap R2–R4.
- Confirmed there is no public authoring/mutation/save API. Added DOCX/PDF creation TODOs C1–C5 and prioritized extraction/performance work in [TODO.md](../../../TODO.md).
- Compared capabilities with linked official PyMuPDF documentation. PDF rendering/creation already exist there; Office rendering is also documented. The roadmap distinguishes parity work from candidate differentiation and makes no market-wide uniqueness claim.
- This change adds documentation only. No native build or runtime tests were rerun; historical installed-wheel results below remain separate evidence. No dependencies, lockfile, licenses, or upstream revisions changed.
- Documentation checks passed: 22 local links (including referenced headings), balanced code fences across seven Markdown files, 19 unique roadmap IDs (one implemented and 18 open), and `git diff --check`.

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
