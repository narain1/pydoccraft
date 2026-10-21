# Existing acceptance gates

The first 2026-10-21 source/evidence audit closes **API7, QA6 and QA8** as current reporting and representative-corpus criteria. It compares the original reviewed roadmap (`ece6c85`) with the final spec123 source/artifact, not just later backlog summaries, bringing the checklist to 51 checked and 88 open criteria out of 139. The later spec126 audit below covers QA2 separately. These decisions do not promote an operation support cell or certify full format parity.

The exact [spec123 validation](../.kiro/specs/rust-document-engine/validation.md#123-typed-pdf-underlays--combined-installed-artifact) records 452 native tests and two clean installed 568-test passes, with zero failures/errors/skips, immutable dependency pins, source hashes and artifact sizes. This audit reuses that evidence; it does not claim another wheel build or runtime test run. Parent delivery branch source preserves the same reviewed runtime.

Evidence navigation: [operation matrix](support-matrix.md), [formats](formats.md), [authoring](authoring.md), [upstream register](upstream-roadmap.md), [cookbook](cookbook.md), [historical API sketches](authoring-api-design.md), [support-ledger checks](../tests/test_support_matrix.py), [public API/error fixtures](../tests/test_api.py), [input-bound fixtures](../tests/test_input_bounds.py), [structure fixtures](../tests/test_structure.py), [story fixtures](../tests/test_docx_stories.py), [ownership fixtures](../tests/test_collection_ownership.py), [authoring fixtures](../tests/test_authoring.py), [save fixtures](../tests/test_pdf_output_options.py), [slide-limit fixtures](../tests/test_slide_limits.py), [native byte/archive guards](../rust/src/limits.rs).

These are acceptance decisions for the exact reporting/test mechanisms. New operations must keep satisfying applicable tests. API1/QA1 completeness, R4/P4/U09 allocation work, QA7 platform/font evidence and all remaining behavioral clauses stay open. No public capability-query framework or second completion database is added.

## API7: one support inventory and useful errors

Original criterion: use the same development ledger as API1/QA1, publish its operation-specific support matrix, defer a public capability-query framework, identify unavailable runtime operations, and retain existing exceptions.

| Clause | Existing concrete evidence | Remaining clause within this criterion |
|---|---|---|
| One development ledger / published operation matrix | `docs/support-ledger.json` is the sole operation source; `tools/support_matrix.py` generates `docs/support-matrix.md`.21 profiles ×7 operation cells, all139 roadmap joins; ownership/checklists are derived, not independently stored. `SupportMatrixTests.test_no_second_ownership_or_completion_database`, `test_duplicate_profiles_and_missing_operation_cells`, `test_generated_join_tracks_roadmap_without_storing_ownership` verify this design. | None for current inventory mechanism. Broader per-feature dispositions stay API1/QA1. |
| Traceable support, blockers and explicit unassessed scope | Test/native/immutable-revision/artifact/source-fingerprint joins are enforced by `test_supported_cell_needs_tests_native_scope_and_artifact`, `test_selector_and_source_digest_drift_are_not_silently_refreshed`, `test_native_pin_sdk_snapshot_and_artifact_section_are_verified`, `test_upstream_blocker_needs_real_register_and_native_citation`. Sparse rows explicitly say Unassessed, not unsupported or complete. | No promotion of unassessed operation cells. |
| No public capability framework | No runtime capability/query manager exists; the check/generator uses Python stdlib and source data offline. Current public authoring/extraction paths remain the ordinary handles. | None. |
| Explicit compatible errors | `src/doc_ex/errors.py` retains existing exception identities including compatibility classes. `src/doc_ex/__init__.py::_raise_native` maps unsupported/malformed/unsafe_archive/resource_limit/closed consistently. `ApiTests.test_detection_and_stable_errors`, `test_malformed_xml_zip_relationships_and_media_fail_safely`, and authored unsupported-operation fixtures exercise these paths. Representative precise messages include DOCX default-size reset, table-cell PAGE, supplied-font PDF underlay refusal and unsupported PPTX ranges. | These are current declared profiles; no guarantee every upstream failure is richly structured or E5/PKG8 completeness. |

The current roadmap links to the generated inventory rather than repeating a volatile profile count. Complete member and operation dispositions remain under API1/QA1. No additional capability registry is introduced.

## QA8: publish honest support matrices

Original criterion: document supported operations, upstream blockers and partial fidelity per format/feature; separate runnable cookbook examples from proposed sketches; do not claim full Open XML compatibility before verified scope.

| Clause | Existing concrete evidence | Remaining clause within this criterion |
|---|---|---|
| Supported operations / blockers / partial fidelity | Generated operation matrix, `docs/formats.md`, `docs/authoring.md`, `docs/upstream-roadmap.md` U01–U11 and reference crosswalk distinguish partial/unsupported/blocked/unassessed and authored/imported scope. Every supported ledger cell requires exact tests/native/artifact evidence. Unseeded139criterion joins remain explicitly unassessed. | None as honest reporting. This is not the missing complete QA1 ledger audit. |
| Runnable vs proposed API | `docs/cookbook.md` links executable examples and states the current owned/unreleased profile. `docs/authoring-api-design.md` begins “historical design direction” and labels sketches PROPOSED. Seven `CookbookTests` execute concrete imported functions/CLIs, reopen DOCX/PDF/PPTX outputs and verify text/table/link/render outcomes. | Representative document coverage is assessed separately under QA2 below. |
| No blanket compatibility claim | Formats/cookbook/API direction/operation matrix explicitly disclaim full parity, arbitrary imported preservation and pixel-perfect conversion. Published GitHub Pages is independently recorded in validation website gate; local final123 site has29pages/1,775 local links. | Future features must keep updating these documents; current-main publication is still the ordinary delivery gate. |

The formats guide links to the generated current matrix without a stale profile count. Historical validation counts and earlier QA8-open decisions remain unchanged; this dated acceptance audit supersedes them without rewriting their evidence.

## QA6: error and hostile-input corpus

Original criterion: test malformed source data, invalid edits, stale handles, excessive nesting/media and save failures; keep subprocess/network access absent and do not weaken parser limits.

| Required category | Exact existing acceptance / source |
|---|---|
| Malformed source | `ApiTests.test_detection_and_stable_errors`; `test_malformed_xml_zip_relationships_and_media_fail_safely` checks brokenXML, absent/missing package relationships and malformedPDF. Its malformed image fixture intentionally preserves opaque bytes during extraction, not successful image decoding. |
| Unsafe archive / byte bounds | `ApiTests.test_archive_path_compression_and_member_count_safety` checks traversal, suspicious compression and10,001 ZIP members. `InputBoundsTests.test_input_limit_covers_bytes_streams_paths_and_memoryview_byte_size` checks byte semantics before parser entry. Native `limits::tests::{bounds_input_and_serialized_bytes_including_escapes,rejects_unsafe_members,checks_compression_ratio_without_truncation}` covers exact/+1, aggregate overflow and compression arithmetic. |
| Excessive nesting | `StructureTests.test_docx_nested_payload_depth_boundary` accepts16 and rejects17 nested tables; `DocxStoriesTests.test_header_nesting_and_grid_limits_precede_layout` rejects17 nested header tables and huge gridBefore before layout. Native `ooxml::tests::presentation_only_token_prolog_and_depth_budgets_are_explicit` checks presentation XML depth/token/prolog budgets. |
| Media / source work limits | `SlideLimitTests.test_large_binary_and_png_media_pass_but_unused_presentation_is_bounded`, its repeated/relocated/compatibility source-reference cases, and native `author_docx::tests::aggregate_media_budget_is_transactional_and_removed_payloads_release_capacity` test encoded-media/work accounting and atomic exact/+1 refusal. The synthetic aggregate-media proof never serializes or renders its deliberate nonPNG payloads. |
| Invalid edits / stale handles | `AuthoringTests.test_invalid_docx_edits_are_atomic`, `test_numeric_color_validation_and_failed_pptx_edits`, `test_pdf_organization_invalidates_handles_and_is_atomic`; `CollectionOwnershipTests.test_pdf_iterators_do_not_rebind_positions_after_structure_changes` and `test_slide_identity_survives_reordering_but_close_invalidates_iteration`; current focused callback/resource suites provide more precise independent cases. |
| Failed saves / no partially replaced path | `AuthoringTests.test_safe_path_stream_save_and_close`, `PdfOutputOptionsTests.test_save_path_stream_atomicity_permissions_and_default` inject os.replace failure, preserve previous contents/mode, remove temporaries and detect short writes. Caller-owned streams are explicitly not transactional; no false general multi-file transaction claim. |
| In-process / offline runtime / unchanged limits | Native boundary/imports and selected lower-level dependency graph have no subprocess/network document processor. Source scan of `src`, `rust/src`, Cargo manifest finds no subprocess/Command/socket/HTTP-client dispatch. The264-package locked normal/build license audit and selected lower-level crates remain unchanged. Existing parser guards are active; native engine corrections117/114 improve refusal rather than removing limits. This is a source/architecture audit, not a dynamic network sandbox proof. |

All named public/native cases are present in the exact final123452-native / two568-installed runs (zero failures/errors/skips). Their178-file build inventory is retained. The later sole documentation profile-count migration20→21 is separate; all27 doc tests then passed. This acceptance audit adds no runtime, fixture, fuzz run, external-validator claim or runtime test count. QA6 completion means the enumerated representative corpus exists and passes; R4/P4/U09 allocation/work guarantees, QA7 platform/performance and broader hostile-input research remain open.

## QA2: representative authored documents

Original criterion: generate letters, resumes, invoices, branded reports, forms, reviews and multilingual documents, including combined tables, backgrounds and headers.

| Required category | Executable example and acceptance fixture |
|---|---|
| Letters | `examples/letter.py` and `CookbookTests.test_letter_story_order_links_and_preview`; `template_batch.py` additionally checks two independently filled letters. |
| Resumes | `representative_documents.build_resume` and `RepresentativeDocumentTests.test_multilingual_resume_combines_stories_table_links_and_diacritics`. |
| Invoices | `examples/invoice.py` and `CookbookTests.test_invoice_decimal_values_semantic_table_and_fill`. |
| Branded reports | `examples/branded_report.py` and `CookbookTests.test_report_two_pages_repeated_stories_brand_and_table`; `LetterheadCompositionTests.test_first_logo_tracks_sections_through_reflow_and_numbered_parity` adds combined section/reflow/logo controls. |
| Forms | `representative_documents.build_intake_form` and `RepresentativeDocumentTests.test_intake_fields_copy_flatten_preserve_visible_appearance_and_links`. |
| Reviews | `representative_documents.build_review` and `RepresentativeDocumentTests.test_review_preserves_comments_separately_and_exports_clean_body`. |
| Multilingual documents | The resume contains exact English, French and German Latin text, diacritics and native proofing-language metadata; the same resume fixture checks source and converted output. This does not establish arbitrary-script coverage. |

The three new builders compose existing public operations without another API family. Resume/review documents combine page backgrounds, visible headers/footers, shaded semantic tables, headings, links or comments. The form combines instructions, graphics, a link and owned text/checkbox/choice fields, with copy/flatten controls. The tests reopen outputs, verify exact text and metadata, and check physical dimensions, DPI and visible content at 72/144/300 DPI. Comments remain separate source stories and are deliberately omitted from PDF output; no tracked-revision or imported-editing support is implied.

The [spec126 validation](../.kiro/specs/rust-document-engine/validation.md#126-representative-combined-document-corpus) records retained artifacts and the final exact installed-wheel corpus. Existing runtime, historical support profiles and fixture fingerprints remain unchanged. This closes QA2 and brings the prepared checklist to 52 checked and 87 open of 139 criteria. It fulfills the enumerated representative corpus, not exhaustive combinations, independent viewer conformance, cross-platform fidelity or every authoring operation.

## STY2: style inheritance — acceptance review pending final artifact

The independent spec138 audit compares the unchanged original criterion at `ece6c85`: support base styles, next/linked styles, document defaults and effective formatting inspection through upstream resolution; reject inheritance cycles and distinguish absent properties from explicit false/reset values. The reviewed132 default-size reset closes the last previously recorded behavioral gap. Closure remains conditional on the final combined installed-artifact result; this audit alone does not mark the criterion complete.

| Original clause | Concrete implementation and acceptance evidence |
|---|---|
| Base styles and effective inspection | `Style.based_on`, `formatting` and `resolved_formatting` use native `StyleSheet.resolve_char/resolve_para`, with the table-style resolver for table kinds. `StyleTests.test_named_styles_inheritance_resets_and_native_output` verifies inherited and explicit values, serialized basedOn, exact text and reopened DOCX/PDF. `test_character_style_and_rich_formatting_clear` covers character-style inheritance and native clear serialization. |
| Next and linked styles | `StyleDefaultsTests.test_next_and_linked_styles_roundtrip_with_distinct_relationship_rules` checks native serialized next/link IDs, reciprocal partner replacement, clear behavior and independent copies. Native `style_relations_are_reciprocal_bounded_and_roundtrip` checks full model/readback. Next-style cycles are valid follow-on hints and are not inheritance cycles. |
| Document defaults and explicit overrides | `StyleDefaultsTests.test_native_defaults_affect_inheriting_content_and_preserve_overrides` checks inherited body/table output, unaffected explicit style values, default serialization and real DOCX/PDF pixels. Native `document_defaults_roundtrip_inherit_and_clear_without_overwriting_direct_values` and `supported_default_clears_preserve_effective_native_readback` check direct and resolved native values after serialization. |
| Reject inheritance cycles | `StyleTests.test_invalid_style_changes_are_atomic_and_handles_are_owned` attempts a parent→child cycle and wrong-kind/foreign-owner changes. Native `style_chains_names_limits_and_invalid_patches_are_atomic` and `validate_style_chains` cover actual cycles, missing bases and the 32-level bound before commit. |
| Absent versus false/reset | The style/default workflows above explicitly distinguish omitted/inherited, False and None. `DefaultSizeResetTests.test_reset_inherits_eleven_preserves_overrides_parts_copy_and_visible_outputs` now adds 10→None/effective11 with unchanged explicit overrides, source/PDF text and 72/144/300-DPI pixels. Its invalid-combination workflow and both `default_size_reset_tests` native cases cover full-model preservation, accounting and rollback. |

The implementation stays within existing style handles and formatting dictionaries. Missing native built-in linked companions still raise an explicit error and can be cleared or replaced, as `test_undefined_builtin_link_is_explicit_and_repairable` proves; no style is fabricated. The resolver retains its native Normal fallback, and arbitrary default-kind selection is not added. Imported editing, theme identity, font contexts and style import/cleanup remain separate DOC2/STY3/STY5/STY6 criteria. This decision does not promote historical operation cells, claim arbitrary Open XML style parity or add a new completion database.

## Criteria deliberately not promoted

- **API1/QA1**: representative198 reference members/44families and21operation profiles do not complete dispositions/status/native/test joins for every feature; unassessed joins cannot be relabeled verified.
- **API3**: remaining property annotations, some pretyped border dictionaries and IDE coverage are explicitly incomplete; spec133 covers full Word paragraph snapshot replay. Existing common factories/getters/reprs are substantial partial evidence.
- **QA3**: broad save/reopen/copy and exact-link/nested-table evidence exists, but “for each operation” is not yet exhaustively mapped; leave open withQA1.
- **QA4/QA7**: representative DPI/reopen/memory/size gates exist, but platform/font divergence is the separate active120 workstream. Do not claim cross-platform visual baselines from this Linux artifact.
- **QA5**: repeated reopen and independent same-engine test execution do not establish all independent viewer/schema/conformance evidence.
- **TXT1**: ordinary owned body/story/cell paragraph movement/deletion and rich ranges are strong evidence, but defer until root/reviewer explicitly reconcile the original DOCX text clause with broader ordered/protected/container coverage. No speculative narrowing or overlap with124PPTX line-break work.
- **R4/P2/P4**: structured failures/native allocation entrypoints, alpha/background semantics and bounded selective processing remain documented gaps.
- **C5/FORM2/DOC2/DOC5/DOC6/DOC8**: repeating rich rows/imported Office/template variants/cross-container remapping/duplication remain open; independent whole-document copy does not prove fragment adoption.
- **DOC7**: core language and typed extended-property access remain absent despite supported core/custom scalars.
- **E2/E3/E5/LAY8**: exact normalized/source offsets, broader scope geometry/revisions and substitution/omission diagnostics remain incomplete.
- **STY2/STY3/STY5/STY6/TXT4/TXT5/TXT6/TXT8**: explicit default-size reset, live theme identity, document-scoped fonts, style import/retarget, decimal/bar tabs, custom list/script coverage and arbitrary ranges remain native-backed gaps.
- **TBL4/TBL6/TBL7/TBL8/TBL10**: complete vertical split reversibility, merged-row overflow, fill/border conflicts, inter-cell spacing/vertical script and captions/accessibility are not supplied by current ordinary-cell/table profiles.
- **BG2/BG3/BG4/BG7/BG8/LAY3/LAY6/LAY7/LAY10/MED2–MED8**: placement/fills/rotation/linkage/field-context/semantic-note deletion/hyphenation/group/chart/math/caption source contracts are incomplete; new typedPDF underlays do not establish Office watermark parity.
- **REF2–REF8/FORM3–FORM5/REV1–REV6/PKG1–PKG8/PDF2/PDF3/PDF6–PDF8/SLIDE1–SLIDE6**: their explicit remaining clauses in the reviewed roadmap remain; no nearby successful bounded profile is promoted into full import/preservation/evaluation/appearance/standards support.
