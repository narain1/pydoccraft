# Rust-first API and language bindings plan

Status: proposed; no API migration or new language binding is implemented by this plan.

## Goal and scope

Make Rust the single implementation of the complete supported document API, with a native Rust library and thin Python, Node.js/npm and Go bindings. Preserve the existing Python API and its observable behavior throughout migration. This includes extraction, rendering, conversion, authoring and editing, not just the original three native operations. Unsupported operations remain explicitly unsupported; this project does not grant permission to add new format capabilities or change upstream engines.

Today `rust/src/lib.rs` mixes PyO3 entry points and engine dispatch. `src/doc_ex/__init__.py`, `_document.py`, `models.py` and `_authoring.py` contain public behavior, validation, formatting, object ownership and lifecycle logic. A Rust `rlib` exists but is not yet a standalone public API (`publish = false` and PyO3 is a direct dependency). Moving wrappers alone would not deliver this goal.

## Target architecture

- A Python-independent Rust core owns typed document and page APIs, owned extraction models, errors, detection, validation, limits, formatting, authoring, save/serialization and handle lifecycle. It calls the existing upstream-backed adapters and never exposes upstream types as its public contract. A standalone Rust consumer builds and runs without Python or PyO3.
- A separate PyO3 binding maps Python inputs, exceptions, paths/streams and language conventions to the core. Existing imports, signatures, output, negative indexing, callbacks and error behavior remain compatible. Path and stream I/O can stay in the binding; semantic validation and bounded processing belong in core.
- A Node.js native binding (N-API) maps Buffers, typed values, errors and lifecycle to the same core. Package prebuilt native binaries per supported platform; do not rely on executable tools or runtime downloads.
- A narrow versioned C ABI over the core enables Go cgo bindings. Use opaque handles, explicit buffer lengths and free functions, stable error codes, panic containment and documented thread/lifetime rules. Never pass Rust layouts or Go pointers retained beyond a call across the ABI. Define a support policy for cgo and distributed native libraries before release.
- Keep language-specific ergonomic wrappers thin: argument conversion, local I/O, native resource cleanup and language-standard exceptions are allowed, but document semantics and limits are not duplicated. Rust is the source of truth, not a JSON round-trip through Python.

## Workstreams and gates

1. **Inventory and freeze behavior.** Enumerate every exported function, object, property, method, overload, callback and exception in the Python facade and type stubs, including authoring children and format-specific operations. Map each to implementation, limits, ownership/invalidation semantics and existing tests. Capture golden fixtures for text, Markdown, HTML, assets, tables, metadata, selection, rendering, conversions, copy/save, collections and failure cases. Review unresolved compatibility questions before moving code. Reconcile with ongoing work and update the inventory when the public API changes.
2. **Separate core from PyO3.** Introduce a workspace or equivalent crate boundary, a typed Rust error model and public owned models. Move Python-only exception conversion and PyO3 declarations to the binding crate. Maintain locked dependency pins and existing package/wheel names. Prove a clean Rust-only consumer can compile and run without linking Python; compare behavior against unchanged Python tests.
3. **Migrate read APIs.** Move format selection, extraction models, document/page operations, text/Markdown/HTML and media export into core in small vertical slices. Preserve budgets before allocation, source ordering, image/link ownership, session reuse, indexing and close semantics. Migrate Python callers slice by slice and test outputs and errors against the frozen corpus, including malformed and hostile inputs.
4. **Migrate authoring and editing.** Inventory and transfer all PDF/DOCX/PPTX creation, mutable objects, child collections, formatting, properties, save/copy, callbacks and stale-handle invalidation. Define safe Rust ownership and mutation APIs without leaking backend models. Preserve Python's callback reentrancy, atomic validation and failure semantics with regression tests. Continue using public upstream format APIs, not replacement format implementations.
5. **Rust API delivery.** Document stable public types, units, indexing, resource ownership, concurrency, supported operations and versioning. Add integration tests from an external Rust consumer and decide crate publication/name separately. Do not declare parity until every inventoried operation has a mapping and evidence.
6. **Python binding parity.** Replace semantic Python implementations with thin adapters incrementally; preserve compatibility including binary streams, paths, exceptions and values that outlive their document. Run the complete existing suite against an installed wheel and compare the shared corpus, not only unit tests of native calls.
7. **Node and Go bindings.** Define idiomatic signatures and an explicit per-operation support matrix based on the inventory. Implement bindings against the same typed core/ABI, including error mapping, close and stale handle behavior, memory release, limits and offline distribution. Test independently installed npm packages and Go consumers against the shared corpus. Avoid claiming full API support until all inventory rows pass in each language.
8. **Release and ongoing parity.** Add CI for Rust, installed Python wheels, installed npm packages and Go consumers on each supported OS/architecture. Test build and offline execution without Python for Rust/Node/Go, without network credentials or external document executables. Run concurrency, repeated-open/close, malformed-input, allocation/limit and cross-language output/error tests. Audit the locked runtime license graph, attribution and artifact sizes whenever dependencies change. Record versions, platforms, test counts, supported matrix and known upstream fidelity limits in the spec validation record.

## Cross-language contract and safety

Specify immutable snapshots versus live handles, document-close and mutation invalidation, thread safety and callback/reentrancy before exposing each operation. Values that currently own their data must remain usable after the source closes. Define index conversion, Unicode and character offsets, units, numeric range checks, PNG bytes, passwords, error categories and mapping in each binding. Never convert an unsupported feature into silent success. Enforce input, archive, recursion, page, image and output budgets in core for every entry point; bindings may add stricter checks. No runtime downloads, network requests or subprocess document processing.

## Acceptance criteria

- The reviewed inventory covers the entire current public API, with a Rust implementation and Python/Node/Go binding mapping and passing evidence for every supported row; unsupported rows have explicit errors and reasons.
- Existing Python users retain their documented behavior and the full installed-wheel suite passes without weakening tests. Rust, installed npm and Go consumers pass the same representative extraction, render, conversion, authoring, lifecycle, error and hostile-input cases.
- A Rust-only application and Node/Go consumers work without a Python installation. Artifacts are self-contained for supported platforms and work offline.
- Core behavior and safety checks are not implemented separately in each binding; native resources and ABI allocations are released safely, including failures and repeated close.
- `cargo fmt --all -- --check`, locked Rust tests and warnings-denied Clippy pass; all language artifacts are built and tested, conversion outputs reopened, licenses reviewed, and validation evidence recorded. Existing fidelity and upstream limitations remain documented.

This is a sequence of reviewable changes, not a promise that all bindings or every platform will ship in the first implementation PR. New public semantics and any compatibility break require a reviewed specification change.
