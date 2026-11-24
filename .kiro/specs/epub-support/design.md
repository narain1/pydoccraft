# EPUB design

## Container and package

The in-repo reader uses the existing shared ZIP preflight and locked rust libraries for ZIP and XML. The preflight bounds central directory work and checks local/central consistency before indexing. The EPUB layer compares the archive readers' entry views, opens the single declared package, then bounds each resource read and the cumulative read budget. Paths use `/` and resolve only inside the publication. Runtime operations are in-process and offline.

`epub.rs` owns the OPF metadata, manifest and spine, EPUB 3 navigation and EPUB 2 NCX, and guarded resource I/O. `epub_content.rs` maps namespace-aware XHTML onto ordered document blocks. Tables carry semantic cells with nested blocks, spans, headings and row-group metadata. Images retain source bytes, alternative text and inline position. SVG uses a restricted static profile. MathML extraction retains a namespace-complete source fragment and readable text. `epub_overlay.rs` reads each referenced SMIL timing tree once, resolves local text and audio paths, and validates target IDs when their XML members are parsed.

## Public API and output

The existing native extraction, render, and conversion operations dispatch to EPUB. Python also exposes bounded resource ranges and ordered per-chapter media cues. Cue audio points to packaged bytes and uses integer microseconds for clip timing. The timing tree preserves sequence scope and semantic roles for a future playback controller.

`epub_editor.rs` handles targeted OPF metadata updates and batch XHTML replacements. It stages changes, resolves repeated spine positions to their shared file, checks existing text anchors, and rebuilds the ZIP through the bounded sink. The chapter editor validates the complete tentative book before returning its bytes. Untouched members keep their uncompressed content, with archive header details subject to the writer.

`epub_structure.rs` stages a replacement spine and manifest additions. It retains original itemrefs, assigns unique paths and IDs to new chapters, and shares the editor’s ZIP writer. `epub_toc_edit.rs` updates EPUB 3 navigation and EPUB 2 NCX in the same transaction. Navigation groups keep their hierarchy and use each group’s earliest surviving spine target for sibling ordering. Omitted chapters stay in the manifest and archive so references from other chapters or media overlays can still resolve. New and replaced local assets are bounded, and the completed archive is parsed again before return.

`epub_output.rs` projects supported semantic blocks into the existing WordCraft model. The WordCraft web layout handles unstyled reflowable previews. The native DOCX and PDF exporters handle document pagination in the semantic profile. The bounded `epub_math.rs` adapter substitutes an equation object at each formula's original text span and lets the pinned math engine shape it.

## Creation and dependency scope

`EditableEpub` validates and escapes semantic EPUB 3 source, then calls rbook 0.7.12 with only its write feature. Writes run through a bounded sink. The public creation APIs do not accept externally supplied XHTML, OPF, or archive entry names. Authoring and metadata editing use the same output limit. Input, output, XML and resource limits bound the work of individual stages. They do not measure total process memory.

This design replaces LexePub and retains the existing rbook writer. It requires no craft-libs EPUB proposal or change to the shared backend revision. The read and creation paths are independently exercised by round-trip tests.

Container limits, repeated metadata, namespace preservation, links, table spans, and image handling are owned by the local reader. The existing writer accepts a bounded sink and publication dates at its current pin.

## Source rendition

Source rendering reads the original target resource to retain CSS selector context and author geometry. `epub_css.rs` loads and parses local styles with Raffia and resolves the supported selectors under a work budget. `epub_style.rs` maps the cascade and inherited typography onto Taffy and native WordCraft properties. `epub_layout.rs` shapes text and math, measures and lays out boxes, then paints the composed draw list through the native renderer. SVG content documents use the validated static profile and resvg with external image loaders disabled. Raster output and clip size use the shared DPI and pixel limits.

The EPUB adapter owns author stylesheet resolution and knows only archive paths. It never delegates fetching to a renderer. Raffia, Taffy and resvg are locked crates with permissive licensing. Broader selectors, CSS custom properties and layout features, packaged fonts, synthetic spreads, CSS-aware conversion and SVG embedded into XHTML remain open.
