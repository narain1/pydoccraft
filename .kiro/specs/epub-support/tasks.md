# EPUB implementation plan

- [x] Detect EPUB 2 and 3 and validate archive structure and sizes before reading resources.
- [x] Replace the eager LexePub path with bounded in-repo container, OPF, navigation and XHTML extraction.
- [x] Verify repeated metadata, path resolution, manifest and spine order, links, tables and media through the public API.
- [x] Expose bounded member reads and targeted metadata editing for existing unsigned EPUBs.
- [x] Extend reflowable EPUB 3 creation with publication dates, lists, tables, preformatted text and static media. Enforce the ZIP output limit at the writer sink.
- [x] Add semantic reflow preview and DOCX/PDF conversion with explicit errors for unsupported features.
- [x] Update locked dependencies, license inventories, and format and authoring documentation.
- [ ] Validate the release wheel matrix on Linux, macOS, and Windows.
- [x] Replace existing XHTML chapters transactionally and preserve targeted navigation and overlay anchors.
- [x] Read SMIL timing trees and expose chapter cue order, time ranges, target IDs and bounded audio reads.
- [x] Project supported Presentation MathML into native equations for PNG, DOCX and PDF.
- [x] Apply local author styles for the supported cascade and layout properties. Render fixed-layout XHTML and static SVG chapters with viewport bounds and clipping.
- [ ] Extend source CSS and fixed-layout fidelity, including archive-local fonts, embedded SVG, synthetic spreads and conversion.
- [ ] Expand MathML layout to remaining elements, attributes and styling.
- [ ] Decode and synchronize media overlay playback, including text-only narration cues.
- [ ] Add structural chapter operations and edits to packaged resources.
