# Design: EPUB support

## Engine selection

Use `lexepub = 0.1.2` from crates.io, locked by checksum in `Cargo.lock`. The crate is Apache-2.0 and supports in-memory EPUB extraction. Default features are disabled; its CLI, C FFI, WASM, and Embassy features are not linked. The GPL-3.0 `epub` crate is prohibited.

`rust/src/epub.rs` is a thin adapter. Existing ZIP preflight runs before LexePub. The adapter uses LexePub metadata and text-only spine traversal, verifies that every declared chapter was returned, and maps values into the existing normalized payload. It does not parse OPF/XHTML itself.

## Compatibility mapping

An EPUB is reflowable rather than page-based. The compatibility API maps one spine chapter to one zero-based `Page`. Each chapter initially contains one paragraph block with upstream-extracted plain text. Document metadata records `format=epub`, EPUB version, title, authors, language and other available publication fields.

This chapter-as-page mapping is explicit and must not be described as physical pagination. Fixed-layout EPUB remains unsupported.

## Boundary

The existing boundary remains unchanged:

```python
_core.extract(data, "epub", password=None) -> JSON string
```

Passwords are rejected because encrypted/DRM EPUB is unsupported. Rendering and conversion dispatch return stable unsupported errors.

## Safety

Run the shared ZIP preflight before upstream parsing. Require the EPUB mimetype marker and reject encryption metadata. Cap chapters at 10,000 and rely on the existing 64 MiB bounded JSON serializer. Treat upstream chapter-count mismatch as malformed/unsupported content rather than silently returning partial extraction.

## Deferred work

Navigation links, images, semantic XHTML blocks/headings, CSS, media overlays, fixed-layout rendering, conversion, and authoring require separate upstream/API audits and acceptance criteria.
