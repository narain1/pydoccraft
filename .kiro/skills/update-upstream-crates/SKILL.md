---
name: update-upstream-crates
description: >
  Upgrade pydoccraft's pinned ArtCraft engine crates (PdfCraft, WordCraft,
  DeckCraft, and the craft-libs rev they share) to the latest upstream main.
  Use when the user asks to update, bump, upgrade or refresh the upstream
  packages, engine crates, ArtCraft dependencies or their pinned revisions, or to
  check whether newer upstream commits exist.
license: Apache-2.0
---

# Update the upstream ArtCraft crates

An upstream upgrade is **one dedicated change**. Do not mix it with refactors, features or
performance work (AGENTS.md). Work on its own branch cut from `main`.

The crates come from the **original repositories only**, never from contribution forks:

| Project | Repository |
|---|---|
| PdfCraft | `https://github.com/storytold/pdfcraft` |
| WordCraft | `https://github.com/storytold/wordcraft` |
| DeckCraft | `https://github.com/storytold/deckcraft` |
| craft-libs | `https://github.com/storytold/craft-libs` |

The target is the current `main` commit of each. Cargo still needs an **immutable commit**,
so resolve `main` to a full SHA and pin that SHA. Never write a branch name or a floating tag.
Read `AGENTS.md` ("Upstream revisions") first for the current pins and the list of behaviors
the adapters provide themselves.

Use a scratch directory outside the repository, e.g. `~/.cache/dx-upstream`. `/tmp` can be
wiped mid-session. Use `bash -c '...'` for loops: zsh does not word-split unquoted variables.
Never run two git operations in the same clone from parallel tool calls.

## 1. Resolve the targets

```bash
for r in pdfcraft wordcraft deckcraft craft-libs; do
  echo "$r $(git ls-remote https://github.com/storytold/$r refs/heads/main | cut -f1)"
done
```

Clone each upstream once and fetch `origin/main`. Then find the craft-libs rev their `main`
branches pin (`git show origin/main:Cargo.toml | grep craft-libs`). WordCraft and DeckCraft
must agree on the `krilla`/`craft-pdf` rev, and that rev goes in this repository's
`[patch.crates-io]`. DeckCraft may also pull other craft-libs crates at other revs; Cargo
resolves those itself.

## 2. Re-pin in this repository

Branch `deps/upstream-releases` from `main`, then:

1. Replace every old revision in `Cargo.toml` (the repository URL too). All crates of one
   repository move together. Count occurrences to confirm, and confirm no `narain1` remains.
2. **Refresh `vendor/pdfcraft/`.** It is a byte-identical copy of the pinned PdfCraft `vendor/`
   tree (`hayro`, `hayro-cmap`, `hayro-interpret`, `hayro-jbig2`, `hayro-syntax`, `lopdf`).
   Verify that at the old pin, then replace those directories from the new commit with
   `git archive <sha> vendor | tar -x`. Do not vendor `winit`, `egui-wgpu` or `vello_common`:
   they belong to the desktop app we must not link.
3. **Diff the upstream `[patch]` sections** between old and new. Root-workspace patches do not
   propagate, so copy new entries that our graph needs into this `Cargo.toml`.
4. `cargo update -w`, then `cargo metadata --locked` must succeed.
5. Compare `Cargo.lock` before and after: package count, added or removed packages, and any
   non-fork version change. Anything new needs license review.
6. License audit:
   ```bash
   cargo metadata --locked --format-version 1 > meta.json
   python3 tools/audit_licenses.py meta.json            # "stale" until regenerated
   python3 tools/audit_licenses.py meta.json --write
   ```
   The diff of `THIRD_PARTY_LICENSES.md` and `LICENSE-THIRD-PARTY` should be only version rows.
   Confirm none of the prohibited crates are linked (engine, ui, automation, mcp, ocr, print,
   sign, js, updater, platform, a11y).
7. Update the pin table in `AGENTS.md` and `docs/dependencies-and-licenses.md`. Read SHAs from
   git, never type them from memory.

## 3. Audit what the upstream no longer provides

There are no local patches, so anything we relied on a patch for must be accounted for. For
each public item our adapters call, ask whether `main` still has it. The compiler finds the
missing symbols, but it cannot tell you what changed silently. Also run the tests.

For every missing capability, choose in this order and write the choice down:

1. **Reimplement it in the adapter with public upstream APIs.** Allowed by AGENTS.md as a
   "minimal adapter workaround" when documented. Example: an underlay is added normally and its
   stream moved to the front of `/Contents`.
2. **Rebuild the guarantee from what upstream does expose.** Example: DeckCraft renders a
   panicked slide as a blank image and bumps `panics_caught()`. A wrapper compares the counter
   before and after, so blank output becomes an explicit error.
3. **Refuse explicitly.** A key or operation that cannot be done faithfully must fail with a
   stable error, never be dropped silently. Pin the refusal with a test and document it.
   Example: DOCX core `language` has no `dc:language` upstream. When upstream later fixes the
   cause (the default-size clear was refused until storytold/wordcraft#4092 merged), lift the
   refusal, restore the original tests, and update every document that states it.
4. **Remove the test** only when the code it exercised has no production caller and no public
   API (an upstream contribution's own regression test). Say so in the commit.
5. **Record a known upstream defect** in docs and a test that asserts the actual behavior, when
   the effect is cosmetic and bounded.
6. **Stop and ask** when the defect corrupts user content (for example text lost or misordered
   in converted output) or breaks a documented public feature. This is the user's decision.

Do not edit vendored or upstream source to make integration easier.

## 4. Fix the adapters

`cargo build --locked`. Compile errors are real upstream API changes. Decide each deliberately
and keep behavior identical unless the change is the point: a plain color became a `Fill` enum,
a single band size became a row and a column pair, an export `bytes: Vec<u8>` became
`Arc<[u8]>`, a struct gained a field. Do not turn feature work into the upgrade; for a new
optional field use the value that preserves today's behavior.

## 5. Triage test failures against `main`, not against zero

Local macOS runs need libpython and have **pre-existing** failures that the upgrade did not
cause. Establish the baseline first:

```bash
export DYLD_FALLBACK_LIBRARY_PATH=$(python3.13 -c 'import sysconfig;print(sysconfig.get_config_var("LIBDIR"))')
git worktree add --detach ~/.cache/dx-upstream/main main
CARGO_TARGET_DIR=~/.cache/dx-upstream/target-base cargo test --locked   # in that worktree
```

Save failing test names for both trees and `comm -13` them. Only the **new** failures are the
upgrade's. Use the interpreter the package targets (Python 3.13) explicitly; a stray `python3`
may be 3.9.

For every new failure, in this order:

1. Read the test and its comments. Many pin a *known upstream defect* ("native reports only its
   owned last page", "vertical wave is empty", "approximated as double").
2. **Probe, don't guess.** Add a temporary `eprintln!("PROBE ...")`, run with `--nocapture`,
   print old values on the `main` worktree and new ones here. Diff struct fields to find the
   one that changed.
3. Find the upstream commit: `git log --oneline <old>..<new> -S'<symbol>'` or `--grep`.
4. Classify: stale expectation of a fixed bug (update it, keep intent, rename if the name states
   the old behavior), deliberate change reaching a public feature (update test **and** docs),
   adapter assumption broke (fix the adapter), or real regression (step 3.6: ask).
5. **Never loosen a tolerance or delete a test to make it green.** A measured gap beyond a
   fidelity contract is a product decision. Quantify it (values over the limit, maximum
   difference), check it against `main`, and bring it to the user.
6. Remove every probe before committing: `grep -rn PROBE rust/`.

When resolving merges or deleting test modules, grep the test build for `is never used`: a
function that lost its `#[test]` attribute compiles but never runs.

## 6. Release-candidate verification

Per AGENTS.md "Verification", on the committed lockfile:

1. `cargo fmt --all -- --check`, `cargo test --locked` (compare to the baseline), `cargo clippy
   --locked --all-targets -- -D warnings` (compare to `main`; newer local Rust adds lints).
2. `maturin build --release --locked`; install the wheel into a clean Python 3.11 environment.
3. Run the Python tests **against the installed wheel**, not the source tree. Features that
   changed (refused keys, removed behavior) have Python tests and docs that must change with
   them. Reopen every converted output and verify PDF, DOCX and PPTX extraction and PNG
   rendering on `.test_files/`.
4. Record wheel size, uncompressed extension size, test counts, platform, the pinned revisions
   and limitations in `.kiro/specs/rust-document-engine/validation.md`.

A successful build alone is not evidence of extraction or conversion correctness.

## Reporting

Say which commits were pinned and why those, which capabilities disappeared and how each was
handled (adapter, refused, removed, or recorded), which API and behavior changes were adapted,
what is still failing against `main`, and which decisions are the user's. Leave generated
artifacts (`target/`, wheels, caches) out of the commit.
