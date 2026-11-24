# EPUB design

## Container and package

The in-repo reader uses the existing shared ZIP preflight and locked rust libraries for ZIP and XML. The preflight bounds central directory work and checks local/central consistency before indexing. The EPUB layer compares the archive readers' entry views, opens the single declared package, then bounds each resource read and the cumulative read budget. Paths use `/` and resolve only inside the publication. Runtime operations are in-process and offline.

`epub.rs` owns the OPF metadata, manifest and spine, EPUB 3 navigation and EPUB 2 NCX, and guarded resource I/O. `epub_content.rs` maps namespace-aware XHTML onto ordered document blocks. Tables carry semantic cells with nested blocks, spans, headings and row-group metadata. Images retain source bytes, alternative text and inline position. SVG uses a restricted static profile. MathML is stored as a namespace-complete source fragment plus text, with no execution or rendering.

## Public API and output

The existing native extraction, render, and conversion operations dispatch to EPUB. Python also exposes `Document.read_epub_resource(path)` and `Document.edit_epub_metadata(title=..., description=...)`. The latter edits only the relevant OPF ranges and rebuilds the ZIP within the output budget. Its original chapter and resource bytes are preserved. Untouched ZIP headers and compression layout can change.

`epub_output.rs` projects supported semantic blocks into the existing WordCraft model. A WordCraft web layout renders an individual chapter. The native DOCX and PDF exporters handle document pagination. Sources with unsupported layout or content features return explicit errors. The adapter applies no EPUB CSS and performs no physical page mapping during extraction.

## Creation and dependency scope

`EditableEpub` validates and escapes semantic EPUB 3 source, then calls rbook 0.7.12 with only its write feature. Writes run through a bounded sink. The public creation APIs do not accept externally supplied XHTML, OPF, or archive entry names. Authoring and metadata editing use the same output limit. Input, output, XML and resource limits bound the work of individual stages. They do not measure total process memory.

This design replaces LexePub and retains the existing rbook writer. It requires no craft-libs EPUB proposal or change to the shared backend revision. The read and creation paths are independently exercised by round-trip tests.

Container limits, repeated metadata, namespace preservation, links, table spans, and image handling are owned by the local reader. The existing writer accepts a bounded sink and publication dates at its current pin. MathML is retained for downstream use. Its layout and the EPUB CSS cascade still need implementations before source-fidelity rendering can be offered.
