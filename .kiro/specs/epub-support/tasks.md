# EPUB implementation plan

- [x] Detect EPUB 2 and 3 and validate archive structure and sizes before reading resources.
- [x] Replace the eager LexePub path with bounded in-repo container, OPF, navigation and XHTML extraction.
- [x] Verify repeated metadata, path resolution, manifest and spine order, links, tables and media through the public API.
- [x] Expose bounded member reads and targeted metadata editing for existing unsigned EPUBs.
- [x] Extend reflowable EPUB 3 creation with publication dates, lists, tables, preformatted text and static media. Enforce the ZIP output limit at the writer sink.
- [x] Add semantic reflow preview and DOCX/PDF conversion with explicit errors for unsupported features.
- [x] Update locked dependencies, license inventories, and format and authoring documentation.
- [ ] Validate the release wheel matrix on Linux, macOS, and Windows.
- [ ] Extend layout fidelity to CSS and fixed-layout renditions, add MathML layout and media overlay support, and expand imported-book editing and conversion.
