# EPUB requirements

## Extraction

1. Detect EPUB 2 and 3 containers and read them in-process. Preserve spine order, repeated entries, and non-linear entries. Expose one compatibility page per entry.
2. Preserve ordered Dublin Core values, OPF meta refinements, manifest records, and hierarchical EPUB 3 navigation or EPUB 2 NCX. Resolve UTF-8 resource paths using archive separators on every host.
3. Preserve semantic headings, paragraphs, preformatted text, inline link labels and character offsets, nested tables, captions, spans, row groups and headers. Preserve referenced raster or validated static SVG images, their alternative text and link metadata, and MathML text and source markup.
4. Return owned extraction values. Support bounded byte retrieval for a particular archive resource while the document is open.
5. Resolve EPUB 3 media-overlay associations, sequence order, roles, text anchors and local audio clip boundaries. Expose a chapter's ordered cues and bounded reads of the corresponding audio bytes.

## Safety and errors

1. Preflight member counts, paths, archive structure, lengths, expansion, compression ratios, and local/central consistency. Limit individual and cumulative reads, XML sizes and structure, and normalized output.
2. Parse UTF-8 XML without custom DTDs. Reject unsupported package selection, archive traversal, remote resource loading, and encrypted content. Never execute active content.
3. Report malformed source and unsupported operations through stable errors. Reject authoring mutations atomically when they would violate limits or input validation.

## Creation and editing

1. Create reflowable EPUB 3 with title, identifier, language, ordered authors and chapters, and optional publication date. Support headings, paragraphs, lists, tables, preformatted text, inert links, common raster images, and validated static SVG.
2. Use the existing editable-document bytes, path/stream save, copy, context manager, and close lifecycle. Reopen created bytes through the extraction path.
3. Limit individual images to 16 MiB, cumulative media and chapter content to 64 MiB each, and encoded EPUB output to 128 MiB.
4. Update selected Dublin Core title and description fields in an existing unsigned book. Preserve the uncompressed bytes of untouched chapter and resource members and the unrelated OPF fields. Reject signed and encrypted packages.
5. Replace multiple XHTML chapter resources by spine position in one transaction. Preserve the original publication's manifest, navigation and spine, and keep IDs targeted by the extracted links, navigation and overlays. Reject edits that fail validation and leave the original usable.
6. Add, remove and reorder spine entries in one transaction. Update EPUB 3 navigation and EPUB 2 NCX, preserving unrelated package information and referenced resources. Add or replace supported local resources using decoded archive paths and validate the resulting book.

## Preview and conversion

1. Preview supported reflowable chapters as PNG with the common DPI and clipping API. Export the semantic profile to DOCX and PDF, starting each chapter on a new page.
2. Retain supported native text, tables, raster images and Presentation MathML. Render supported equation structures through the native equation backend and export native DOCX equations. Return an explicit error when a structure cannot be projected safely.
3. Render fixed-layout XHTML at its numeric viewport and static SVG at its viewBox dimensions while honoring package and spine layout declarations. Clip previews at requested DPI.
4. Apply the supported author CSS cascade for styled XHTML. Read inline, embedded, linked and imported styles using archive-relative resolution. Exercise inherited typography, positioning, colors, box styling, and block, flex and grid layout with pixel-level acceptance tests.
5. Shape XHTML text with archive-local CSS font faces in a chapter-scoped collection, including static TrueType, OpenType, WOFF and WOFF2, and IDPF-obfuscated fonts. Retain inline image and equation placement and table spans. Render validated static SVG content in XHTML and rasterize it in semantic exports.
6. Render local SVG CSS from prolog stylesheet instructions and embedded styles. Resolve CSS imports, packaged fonts and referenced PNG or JPEG images inside the archive. Exercise the SVG rendition in spine pages, XHTML images and semantic export while preserving the original SVG payload.
7. Decode local PCM WAVE, MPEG Layer III, and AAC-LC MP4 overlay audio to bounded PCM clips with the SMIL interval and target. Provide readable narration for text-only cues and a sequential playback API with synchronized target callbacks.
8. Full source CSS and fixed-layout fidelity, richer MathML layout, remaining overlay codecs and playback controls, asset deletion and authoring new media overlay associations remain open. DRM, script execution and unlisted conversion pairs remain outside the scope.

## Verification

Run the locked Rust, license, and installed-wheel checks. Generate local fixtures to exercise EPUB 2 and 3, semantic links and tables, archive bounds, authoring and targeted edits, preview, and reopened conversion outputs. Keep all engine and shared backend Git pins immutable and compatible.
