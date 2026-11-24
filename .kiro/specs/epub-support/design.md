# EPUB design

## Container and package

The in-repo reader uses the existing shared ZIP preflight and locked rust libraries for ZIP and XML. The preflight bounds central directory work and checks local/central consistency before indexing. The EPUB layer compares the archive readers' entry views, opens the single declared package, then bounds each resource read and the cumulative read budget. Paths use `/` and resolve only inside the publication. Runtime operations are in-process and offline.

`epub.rs` owns the OPF metadata, manifest and spine, EPUB 3 navigation and EPUB 2 NCX, and guarded resource I/O. `epub_content.rs` maps namespace-aware XHTML onto ordered document blocks. Tables carry semantic cells with nested blocks, spans, headings and row-group metadata. Images retain source bytes, alternative text and inline position. SVG uses a restricted static profile. MathML extraction retains a namespace-complete source fragment and readable text from the primary presentation branch. `epub_overlay.rs` reads each referenced SMIL timing tree once, resolves local text and audio paths, and validates target IDs when their XML members are parsed.

## Public API and output

The existing native extraction, render, and conversion operations dispatch to EPUB. Python also exposes bounded resource ranges and ordered per-chapter media cues. Cue audio points to packaged bytes and uses integer microseconds for clip timing. The timing tree preserves sequence scope and semantic roles. The Python binding uses `epub_audio.rs` to decode one cue at a time, keeping a decoder for the current track. WAVE reads seek directly to the selected PCM samples. MP3 and AAC keep codec state and the last decoded frame for adjacent cues. MP3 restarts for backward seeks and AAC warms up the preceding access units when seeking. `epub_aac.rs` bounds MP4 boxes and sample-table expansion before demux, validates the resulting local access unit ranges, and applies a continuous edit list to the presentation time. Playback yields each target's audio or text and uses blocking sinks with active-target callbacks to synchronize highlighting.

`epub_editor.rs` handles targeted OPF metadata updates and batch XHTML replacements. It stages changes, resolves repeated spine positions to their shared file, checks existing text anchors, and rebuilds the ZIP through the bounded sink. The chapter editor validates the complete tentative book before returning its bytes. Untouched members keep their uncompressed content, with archive header details subject to the writer.

`epub_structure.rs` stages a replacement spine and manifest additions. It retains original itemrefs, assigns unique paths and IDs to new chapters, and shares the editor’s ZIP writer. `epub_toc_edit.rs` updates EPUB 3 navigation and EPUB 2 NCX in the same transaction. Navigation groups keep their hierarchy and use each group’s earliest surviving spine target for sibling ordering. Omitted chapters stay in the manifest and archive so references from other chapters or media overlays can still resolve. New and replaced local assets are bounded, and the completed archive is parsed again before return.

`epub_output.rs` projects supported semantic blocks into the existing WordCraft model. The WordCraft web layout handles unstyled reflowable previews. The native DOCX and PDF exporters handle document pagination in the semantic profile. The bounded `epub_math.rs` adapter substitutes an equation object at each formula's original text span and lets the pinned math engine shape it.

## Creation and dependency scope

`EditableEpub` validates and escapes semantic EPUB 3 source, then calls rbook 0.7.12 with only its write feature. Writes run through a bounded sink. The public creation APIs do not accept externally supplied XHTML, OPF, or archive entry names. Authoring and metadata editing use the same output limit. Input, output, XML and resource limits bound the work of individual stages. They do not measure total process memory.

This design replaces LexePub and retains the existing rbook writer. It requires no craft-libs EPUB proposal or change to the shared backend revision. The read and creation paths are independently exercised by round-trip tests.

Container limits, repeated metadata, namespace preservation, links, table spans, and image handling are owned by the local reader. The existing writer accepts a bounded sink and publication dates at its current pin.

## Source rendition

Source rendering reads the original target resource to retain CSS selector context and author geometry. `epub_css.rs` loads and parses local styles with Raffia and resolves the supported selectors under a work budget. `epub_style.rs` maps the cascade and inherited typography onto Taffy and native WordCraft properties. `epub_layout.rs` shapes text and math, measures and lays out boxes, then paints the composed draw list through the native renderer. SVG content documents and embedded images use the validated static profile and resvg. Embedded SVG is rasterized with transparency in its layout box. Semantic DOCX and PDF export use PNG for the static SVG image. Raster output and clip size use the shared DPI and pixel limits.

The EPUB adapter owns author stylesheet resolution and knows only archive paths. It never delegates fetching to a renderer. Raffia, Taffy and resvg are locked crates with permissive licensing. `epub_fonts.rs` registers bounded local fonts in a chapter-owned Parley collection. It uses WordCraft's existing web-font decoding and bundled fallbacks, without adding faces to its process-wide font database. Mixed text remains shaped around image and native equation boxes. Tables using packaged fonts derive cell placements and spans from the validated semantic table grid. `epub_obfuscation.rs` recognizes the IDPF font-only declaration and applies it to preview reads after verifying the targets against the package manifest. Resource reads return the stored bytes.

`epub_svg.rs` applies the shared EPUB cascade to SVG prolog stylesheets and embedded declarations in source order. It sends only supported presentation properties and same-document paint references to resvg. SVG images carry their archive resolution base independently of the original markup and public image name. The renderer supplies PNG or JPEG resources through a bounded in-memory resolver. The SVG font database is scoped to the rendition and registers packaged fonts under their CSS family names. External resolvers for file and data URLs are disabled.

Broader selectors, CSS custom properties, font Unicode ranges and variable-axis selection, additional SVG features and semantic fixed-layout conversion remain open.


## Presentation MathML

`epub_math.rs` projects bounded Presentation MathML to the pinned WordCraft equation model for layout and export. The equation envelope carries root size and foreground, and OMML runs carry inherited size, foreground and variant. Presentation and syntax nodes share a projection budget. Each tensor pair contributes one native script level to the depth guard, including levels generated for prescripts. Variable fences and separators use explicit math runs when the native delimiter cannot hold the full sequence.

The EPUB stylesheet cascade resolves declarations on the source chapter before detaching an equation. The preview and export paths attach a bounded snapshot of the supported declarations and the enclosing font size and foreground. The equation projector applies those declarations over same-element presentation hints and evaluates relative sizes in the equation's inherited context. Fixed-layout previews use the chapter viewport. The semantic export path applies these styles within equations. Public extraction retains the original MathML markup.

WordCraft represents zeroed `mpadded` metrics with its visible phantom node. Horizontal `mspace` uses a sized math-space run and follows native script scaling. The emitted OMML retains that run. MathML tables with spanning cells, custom rule thickness, arbitrary vertical spaces and padding shifts, and structures beyond the documented profile remain open.


## Resource removal and media overlay authoring

Chapter edits derive the EPUB 3 `mathml` and `svg` declarations from element namespaces in the new XHTML. Replacement splices only the affected manifest attributes into the original OPF. Insertion uses the same scanner when adding new manifest entries. EPUB 2 retains its existing package vocabulary.

The structure editor rewrites the spine and navigation first, then stages narration association and duration edits. It creates one SMIL document per edited chapter with ordered sequences and text and audio cues. The Python facade sends a bounded depth-first list, and the native writer validates its parent-child transitions and emits `seq` and `par` elements. Sequences point to the chapter or a fragment, and their roles apply to descendant cues during playback. Durations use checked microsecond arithmetic. The package total sums each associated overlay once. Retained overlays use declared duration metadata or explicit clip ranges. Text-only cues contribute zero recorded duration.

Resource removal checks the staged archive after additions, replacements, and overlay changes. It validates retained package ID references, targets in XHTML, SVG, SMIL and NCX, and stylesheet dependencies. The scan follows archive-local stylesheet links and imports regardless of media conditions. It includes recognized off-spine resources and excludes the removed members, allowing jointly removed assets to reference each other. Unprovable scripted dependencies fail explicitly. Untouched ZIP members are raw-copied. The reader reopens and validates the entire result before returning bytes.

An IDPF-obfuscated font replacement validates the plain input within the shared font budget and applies the original package's obfuscation key before writing the member. After a successful staged reference check, removing a font splices its declaration out of the encryption document. Removing the last declared font removes the encryption document from the archive as well.


## Media overlay playback

`epub_audio.rs` maps SMIL time bounds to half-open sample-frame intervals. Streaming advances that integer cursor using PCM frames returned by the decoder. It reuses the track and its decoded frame cache between reads and supports an open-ended interval through the end of the resource. The MPEG decoder verifies a supported Xing/Info tag against packet headers before applying its encoder delay, decoder delay and end padding. AAC uses the MP4 presentation edit.

`epub_opus.rs` checks Ogg page boundaries, packet sizes, and stream continuity before demuxing with checksum verification. It indexes packet times at 48 kHz from their Opus frame headers and validates page granule positions against those times. The decoder applies the identification packet's output gain and uses pre-skip and the final granule to bound the presentation interval. This profile supports one logical stream with mono or stereo output.

The Python iterator seeks by chapter cue index before applying role filters. It decodes each selected target as a group of chunks. Playback calls the target callback once before handing that group's audio or text to the sinks and clears the target when the group finishes or a sink fails.


## Fixed-layout spreads and visual export

The reader preserves package progression and spread settings and spine-level placement and spread overrides. EPUB spread preview pairs fixed-layout entries in progression order. Centered entries, reflowable chapters and entries where the active spread policy disables facing pages use an individual viewport. Non-linear entries can be previewed individually. The first automatic placement is the trailing slot. A missing companion is a white slot with the dimensions of the present chapter.

XHTML and SVG render into the shared pixel-region format. Spread composition places their original-sized rasters edge to edge, aligns their top edges, and paints only regions that intersect the requested clip. Fixed-layout visual PDF export renders each requested chapter as a full-page image and passes its original viewport to the pinned PDF writer. The export bounds aggregate pixel and encoded-image counts.
