# Design: EPUB support

## Engine selection

Use `lexepub = 0.1.2` from crates.io, locked by checksum in `Cargo.lock`. The crate is Apache-2.0 and supports in-memory EPUB extraction. Default features are disabled; its CLI, C FFI, WASM, and Embassy features are not linked. The GPL-3.0 `epub` crate is prohibited.

`rust/src/epub.rs` is a thin adapter. Existing ZIP preflight runs before LexePub. A bounded strict XML preflight validates the container, selected OPF package, and chapter XHTML before LexePub's metadata/AST traversal. The adapter verifies that every declared chapter was returned and maps headings, paragraphs, links, referenced images, and otherwise readable block text into the existing normalized payload.

## Compatibility mapping

An EPUB is reflowable rather than page-based. The compatibility API maps one spine chapter to one zero-based `Page`. Each chapter contains source-ordered heading/paragraph blocks, inline link offsets, and referenced images exposed by the upstream AST. Recognized readable list, quote, preformatted, caption, definition, and table-cell containers map to paragraph blocks without duplicating nested text. Document metadata records `format=epub`, EPUB version, title, authors, language and other available publication fields.

This chapter-as-page mapping is explicit and must not be described as physical pagination. Fixed-layout EPUB remains unsupported.

## Boundary

The existing boundary remains unchanged:

```python
_core.extract(data, "epub", password=None) -> JSON string
```

Passwords are rejected because encrypted/DRM EPUB is unsupported. Rendering and conversion dispatch return stable unsupported errors.

## Safety

Run the shared ZIP preflight before upstream parsing. Require the EPUB mimetype marker, strict bounded container/OPF/XHTML XML, and reject DTDs, unsafe package paths, and encryption metadata. Cap chapters at 10,000 and normalized JSON at 64 MiB. LexePub 0.1.2 still eagerly materializes and clones its CSS/chapter AST before returning; a hard intermediate-allocation ceiling requires an upstream bounded traversal API and is not claimed here.

## Creation backend

Use `rbook = 0.7.12` with default features disabled and only `write` enabled. It is Apache-2.0, accepts in-memory resources, generates the package/manifest/spine/navigation/mimetype/ZIP structure, and writes to `Vec<u8>`. A private `EditableEpub` stores only bounded semantic metadata, chapters, safe text fragments, inert links and image resources. Python owns path/stream I/O through the existing `EditableDocument` lifecycle.

The adapter rejects XML-illegal text, escapes text/attributes, accepts only signature-checked PNG/JPEG/GIF images, and supplies bounded semantic XHTML bodies to rbook. It tracks 64 MiB content and aggregate-media budgets, a 16 MiB per-image budget, and a conservative 128 MiB output budget before writing, then checks actual output size. Callers cannot provide raw XHTML, OPF, CSS, SVG, scripts, archive paths or arbitrary resource names. Because rbook writes to a `Vec<u8>`, a sink-enforced allocation ceiling remains an upstream requirement; the conservative preflight prevents profile-compliant state from exceeding the public bound.

## Deferred work

The completed profile adds semantic extraction and append-only creation. Navigation beyond extracted hyperlinks, CSS, SVG authoring, media overlays, fixed-layout pagination/rendering, conversion, and editing existing EPUB files require separate acceptance criteria.
