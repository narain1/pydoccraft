# Implementation sequence and ownership

Status: historical implementation plan; see [the authoring guide](authoring.md) for implemented APIs. The existing extraction,
conversion and PNG rendering contracts remain the baseline. This document makes the
[API direction](authoring-api-design.md) actionable; the
[workstream ledger](roadmap-workstreams.json) remains the only feature ownership list.
Source feasibility is distinct from an installed-wheel acceptance result.

The original planning batch added sequencing only. Subsequent authoring and native rendering/extraction implementations are tracked by their owners and [validation evidence](../.kiro/specs/rust-document-engine/validation.md).

Run `python tools/check_roadmap.py` to check that every TODO ID has one owner, every
role and upstream reference is valid, and prerequisites contain no cycle. The check
does not establish implementation, fidelity, or completion. Regression tests exercise
missing and duplicated coverage, invalid references, cycles, and misclassified work.

## Dependency waves

W01 owns naming and ownership decisions. Each format workstream implements its own
operations and acceptance tests; W14 supplies shared fixtures, build checks and
release evidence. Shared source files need a single integration owner even when
several workstreams are active. A prerequisite means the relevant accepted baseline,
not completion of every advanced feature on a large issue.

| Wave | Canonical owner | Next bounded deliverable | Handoff / gate |
|---|---|---|---|
| 0 | W01 / #3 | Coverage validator and lifecycle/ownership decisions below | Review decisions before freezing any new public signature; member-level parity inventory remains open |
| 0 | W02 / #4 | Multi-DPI dimensions/metadata/error tests and bounded render requests | Existing rendering APIs; DeckCraft per-call failure reporting remains U09 |
| 0 | W09 / #11 | Recursive table extraction preserving `Table.rows` and source order | Share semantic definitions with W05; do not introduce editable extraction objects |
| 0 and continuous | W14 / #16 | Reproducible locked build, corpus and installed-wheel harness | Every implementation owner contributes tests; W14 records aggregate evidence |
| 1 | W03 / #5 | New DOCX lifecycle, serialization and independent copying | Start with native-generated content; arbitrary-source editing waits for preservation diagnostics |
| 2 | W04 / #6 | Paragraph/run creation and plain-text serialization, then style/list properties | Shares the first thin slice with W03; W04 owns text semantics, W03 owns lifecycle |
| 2 | W10 / #12 | Fixed-page PDF creation and reopen/render proof | Common lifecycle from W03; inspect already-linked creation/edit primitives before adding crates |
| 2 | W11 / #13 | Blank PPTX/model validation/export feasibility and first slide | Common lifecycle from W03; writer exists; themes and opaque parts require preservation fixtures |
| 2 | W12 / #14 | Chart/math/diagram operation inventory and fixture selection | W03 preservation/diagnostic contract; identify concrete upstream gaps without speculative facade methods |
| 3 | W05 / #7 | Tables with string cells, then nested blocks/merges/style | Uses W04 paragraphs; W06 supplies section geometry for pagination acceptance |
| 3 | W06 / #8 | Section geometry, native page color, images and placement | W04 text baseline; backgrounds/underlays use these primitives |
| 3 | W07 / #9 | Revision-bound literal text ranges, links and bookmarks | W04 text baseline; W06 geometry for navigation export; review resolution gated by U06 |
| 4 | W08 / #10 | Supported field/control inventory and explicit stale-result behavior | W06 layout + W07 anchors; calculation and convergence stay upstream under U07 |
| 4, incrementally | W13 / #15 | Executable letter/report/table/template/assembly recipes | Relevant verified primitives only; Pillow rendering recipe can precede authoring |

The workstreams cover all 14 issues once. Their remaining coverage IDs stay open
after an initial slice lands. W10 and W11 do not wait for complete DOCX parity.
W02 and W09 can advance against existing extraction independently of authoring.
W12 is an audit until a supported operation can pass the same acceptance gate.

## Lifecycle and native dispatch decision

Introduce one private PyO3 owner for an editable native document when the first
authoring slice is implemented. Keep WordCraft `Document` inside Rust; do not ship
its serde representation to Python. A Python `EditableDocument` holds this owner
and supplies file/stream I/O and readable methods. There is no global handle registry,
public session class, generic command dictionary or second document schema.

The initial private methods should cover construction, paragraph insertion, native
serialization, independent copying and close. Add typed private operations only
when the corresponding public operation passes acceptance. A later internal enum
can dispatch WordCraft/PdfCraft/DeckCraft owners once another format is implemented;
an unused universal model is unnecessary now. Existing `_core.extract`, `convert`
and `render` signatures remain intact. Initially serialize native authoring state
before calling the existing render/conversion path; W02 can optimize measured
repeated work later without changing public methods.

The Python facade owns filesystem paths. `to_bytes()` always serializes the native
format; `save()` delegates to it, checks filename/format conflicts and writes only
after successful serialization. For path destinations use a same-directory
temporary file and atomic replacement, cleaning up on failure; do not promise
atomicity for a caller-owned stream. A partial/failed stream write is an I/O error,
not a successful save. Context exit closes but never saves implicitly. Native code
does not accept output paths. Existing `convert()` remains explicit about format
changes and unsupported pairs.

`copy()` clones the native document into an independent owner. The pinned WordCraft
model uses shared immutable blocks with copy-on-write mutation, so independence
does not require a custom deep-copy algorithm. Validate input before mutation;
where one user operation makes several native edits, stage it on a native clone
and publish only after success. Bound the work before constructing or staging large
content. Release the Python GIL only after the native ownership/locking rules have
been established and tested; concurrent mutation of the same document must never
race rendering or serialization.

Editable children retain their owning document and use checked private identifiers,
never borrowed Rust pointers. The first append-only paragraph slice can use checked
indexes; deletion/reordering requires identity remapping before those operations
ship. Closing the document invalidates editable children with a stable error;
owned extracted values continue to work as before. A successful edit increments
the owner's revision and invalidates cached layout/page handles. Text ranges bind
to that revision and fail on incompatible edits instead of silently targeting a
different substring. `copy()` produces new child ownership; children from one
document cannot be inserted into another without the explicit content-copy path
owned by W04.

## Smallest authoring slice

Implement and verify `new("docx")`, `add_paragraph(text)`, `to_bytes()`, `copy()`,
`save()`, `close()` and context management together. `add_paragraph` returns its
editable paragraph, initially with text access only. Empty documents retain an
upstream-required paragraph; the first added paragraph consumes the untouched
initial placeholder so ordinary creation does not introduce an extra blank line.
An explicitly added empty paragraph is user content and must not later disappear.
Define embedded newline as a manual line break, not multiple paragraphs; callers
use repeated `add_paragraph` calls for separate paragraphs. Reject prohibited XML
characters and the reserved object replacement character rather than allowing
upstream normalization to silently remove user input.

The pinned WordCraft revision is
`f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22`; all needed crates already appear in
the root `Cargo.toml`. No new runtime dependency or upstream revision is required
by these inspected primitives:

| Operation | Existing public symbol | Integration constraint |
|---|---|---|
| Blank model | `wordcraft_doc::Document::new()` | Supplies a Letter section, built-in styles and an empty Normal paragraph |
| Paragraph value | `wordcraft_doc::Paragraph::with_text(&str, CharProps)` | Filters reserved U+FFFC; reject it in our boundary before construction |
| Append block | `Document::insert_block(StoryRef::Body, &Path::top(index), Block::Para(paragraph))` | Validate index/count bounds; use the actual native model |
| Independent copy | `Document::clone()` | Exercise mutation independence, not just equality of serialized bytes |
| DOCX serialization | `wordcraft_docx::write(&document) -> Result<Vec<u8>, DocxError>` | Map failure before filesystem I/O; apply serialized-output limits |
| Reopen | `wordcraft_docx::read_with_password(bytes, password)` through existing adapter | Reuse OOXML preflight; creation does not imply arbitrary-source preservation |

Evidence: [document construction and model](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/lib.rs#L658),
[paragraph construction](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/para.rs#L802),
[public block insertion](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/edit.rs#L287),
[writer entry points](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/write/mod.rs#L126),
and [DOCX public exports/read contract](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/docx/src/lib.rs#L25).

The acceptance slice must create blank and populated documents, save/reopen, assert
paragraph order and Unicode text, render at chosen DPI, convert to PDF and reopen
that result. Include independent-copy mutation, failed-save destination preservation,
close/child behavior, explicit empty paragraphs, reserved characters and oversized
input. Run against the installed wheel and record font/layout limitations. Text
roundtrip alone does not verify glyph rendering. W03 and W04 remain partial after
this slice; tables, styles, source editing and broad DOCX parity are separate gates.

## Text, ranges and fields after the first slice

These workstreams can reuse the same owner, mutation validation and error mapping;
they do not need independent document/session frameworks.

| Owner | Public pinned symbols | Next viable slice and remaining gate |
|---|---|---|
| W04 | `Document::insert_text`, `format_range`, `format_paragraphs`; `CharProps`, `ParaProps`; `Paragraph::styled` | Append a styled run and change paragraph alignment on a newly created document; test Unicode offsets and save/reopen/render before broader style/list coverage |
| W04 | `Document::copy_range`, `body_fragment`, `insert_fragment`; `edit::Fragment` | Start with same-document paragraph copying. A fragment carries blocks and text-box parts; existence alone does not prove complete cross-document media/style/list/comment remapping. Gate general copying on reference-preservation fixtures |
| W07 | `Pos`, `Path`, `StoryRef`; `Document::delete_range`, `insert_text`, `insert_object`; `InlineObject::BookmarkStart`, `BookmarkEnd`, `CommentStart`, `CommentEnd` | First support literal replacement inside a verified paragraph of newly created content, including matches spanning runs. Validate boundaries before mutation and invalidate stale ranges. Comments/bookmarks require balanced anchors and ID tests |
| W07 | Native revision models plus application-engine `fmt_revisions` implementation | Keep acceptance/rejection of formatting revisions blocked on U06: move/re-export pure routines into a permitted lower-level crate. Do not duplicate that code or link the application engine |
| W08 | `InlineObject::Field`, `FieldStart`, `FieldEnd`; `Document::field_ranges`, `has_unbalanced_field_ranges`; `wordcraft_layout::fields::field_text` / `field_text_on`, `FieldCtx` | Audit PAGE/NUMPAGES first with layout fixtures. The public evaluation helper handles selected field types and otherwise retains cached text; this is not a general refresh/save contract. U07 owns complete status/convergence semantics |
| W08 | `ContentControl::new`, `ControlKind`; `Document::control_ranges`, `control_at`, `control_mut`, `control_text`, `has_unbalanced_controls` | Audit native text controls and preservation on a fixed fixture before any fill method. Native controls exist, but data binding, lock enforcement and writer/render support require independent tests; do not call all controls absent |

Evidence: [range editing and copying](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/edit.rs#L436),
[inline objects](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/para.rs#L371),
[field ranges](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/fields.rs#L92),
[field evaluation](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/layout/src/fields.rs#L126),
[content controls](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/doc/src/control.rs#L485),
and [engine revision routines](https://github.com/storytold/wordcraft/blob/f2e15cf8f899a79899a2d9595ca4ce7df1e5ea22/crates/engine/src/cmd/fmt_revisions.rs#L265).

## Explicit blockers and ownership boundaries

- Arbitrary existing-document `edit()` is not safe merely because `read` and `write`
  exist. The inspected reader is lenient and skips unknown markup; U01 owns strict
  preservation diagnostics. Native-created documents can ship first without a
  misleading promise about source editing.
- The upstream document uses byte offsets. Python text positions must be Unicode
  character offsets with explicit checked conversion; W07 owns that integration.
  Never reinterpret Python indexes directly as native byte offsets.
- W05 owns editable tables; W09 owns owned extraction values. Both use semantic
  row/cell spans, but no mutable shared object is required between the two APIs.
- W06 owns drawing/section primitives; W13 composes watermarks, branded reports and
  template loops. Recipes do not justify additional method families.
- W07 advanced revision acceptance/rejection needs public lower-level operations
  rather than engine Session code (U06). W08 field calculation stays upstream (U07).
- W10 must separately verify Unicode/font support, true redaction and conformance.
  W11 must separately verify preservation and render failure handling (U09/U11).
  These cannot inherit a verified status from blank-document construction.
- No upstream source is edited or issue filed by this plan. Contribution candidates
  remain in the [upstream register](upstream-roadmap.md), with dedicated revision,
  license and corpus review required for any later upgrade.
