# Attribution and distribution status

This unpublished preview links pydoccraft-core and its third-party dependencies;
the project-level `NOTICE`, `THIRD_PARTY_LICENSES.md`, and `LICENSE-THIRD-PARTY`
describe attribution and bundled assets. The preview adds a direct dependency on serde_json 1.0.149,
already present in the core's locked graph, and does not add new assets.

The independently locked graph was reviewed with `cargo metadata --offline
--locked --format-version 1` and `tools.audit_licenses.inventory`: 289 normal/build
packages passed its license-expression allowlist and prohibited-name checks.
Of these, 288 are third-party rows present in the regenerated root inventory;
the other is the local Apache-2.0 core. The root inventory is a superset, not an
exact preview dependency list. Original upstream Git sources and revisions were
checked against NOTICE. No new bundled assets were identified in this preview.

This source preview is not production distribution approval. Before shipping,
review the actual packaged binary, platform-specific dependencies, required
license texts, notices and asset attribution. Repeat the graph review when either
lockfile or the patches change.
