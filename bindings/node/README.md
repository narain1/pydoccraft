# Node-API experiment (not a release)

This Linux-only, synchronous, in-process preview depends on the **in-progress**
Python-free `pydoccraft-core` API. It is not a published package, a stable API,
or a replacement for the Python API. No sessions, EPUB authoring, password
argument, clipping, async execution, or TypeScript definitions are exposed. Calls block the
Node event loop. Document processing never invokes a subprocess or downloads code.

`detect(Buffer)` returns `pdf`, `docx`, `pptx`, or `epub`. Detection is not a
complete validity check. `convert(Buffer, source, target)` returns a new Buffer;
the preview allows PDF→DOCX and DOCX→PDF only. PPTX conversion is disabled
entirely because upstream PDF export can corrupt searchable text for explicit
paragraph breaks, and the core has no reliable guard. Conversion fidelity is
not pixel-perfect. No password can be
passed, so encrypted documents may fail. Errors are ordinary JavaScript `Error`s
whose messages start with the core error category; these are preview behavior.
Inputs must be Node Buffers backed by non-shared ArrayBuffers; shared backing
storage is rejected to prevent concurrent mutation during native processing.
The core rejects inputs larger than 128 MiB, but conversion and output copying
can require substantially more memory; this is not a process memory limit.

`extract(Buffer, filetype)` returns the core's normalized payload as a JSON
string, including EPUB extraction. `render(Buffer, filetype, page, dpi)` returns
a PNG Buffer; page is a zero-based non-negative integer. EPUB rendering is not
supported. Both are stateless and apply the core's input, payload and rendering
limits. They take exactly the listed arguments and do not accept a password.

With Node.js and Rust 1.95+ installed, from the repository root:

```sh
cargo build --locked --manifest-path bindings/node/Cargo.toml
cp bindings/node/target/debug/libdoccraft_node_preview.so bindings/node/doccraft.node
node --test bindings/node/test/*.test.js
```

The addon uses Node-API directly without npm. It directly depends on the same
pinned serde_json already used by the core. Its independent
Cargo root repeats the parent workspace's pinned patches because dependency
patches do not propagate. `Cargo.lock` was derived from the root lockfile; its
only package additions are this local binding, and Python-only dependencies are
absent. Reconcile patches and locked versions whenever the parent changes.
An offline locked metadata review found 289 normal/build packages, all accepted by
the repository's license-expression and prohibited-package checks. The 288
third-party name/version/license rows occur in the regenerated root inventory;
the remaining row is the local Apache-2.0 `pydoccraft-core`. No packaged binary
is provided, and this review is not a production distribution approval.
The build assumes Linux shared-library linking; other platforms are untested.
See [ATTRIBUTION.md](ATTRIBUTION.md) for the preview's attribution and audit status.
