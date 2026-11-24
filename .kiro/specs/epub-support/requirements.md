# EPUB requirements

## Extraction

1. Detect EPUB 2 and 3 containers and read them in-process. Preserve spine order, repeated entries, and non-linear entries. Expose one compatibility page per entry.
2. Preserve ordered Dublin Core values, OPF meta refinements, manifest records, and hierarchical EPUB 3 navigation or EPUB 2 NCX. Resolve UTF-8 resource paths using archive separators on every host.
3. Preserve semantic headings, paragraphs, preformatted text, inline link labels and character offsets, nested tables, captions, spans, row groups and headers. Preserve referenced raster or validated static SVG images, their alternative text and link metadata, and MathML text and source markup.
4. Return owned extraction values. Support bounded byte retrieval for a particular archive resource while the document is open.

## Safety and errors

1. Preflight member counts, paths, archive structure, lengths, expansion, compression ratios, and local/central consistency. Limit individual and cumulative reads, XML sizes and structure, and normalized output.
2. Parse UTF-8 XML without custom DTDs. Reject unsupported package selection, archive traversal, remote resource loading, and encrypted content. Never execute active content.
3. Report malformed source and unsupported operations through stable errors. Reject authoring mutations atomically when they would violate limits or input validation.

## Creation and metadata editing

1. Create reflowable EPUB 3 with title, identifier, language, ordered authors and chapters, and optional publication date. Support headings, paragraphs, lists, tables, preformatted text, inert links, common raster images, and validated static SVG.
2. Use the existing editable-document bytes, path/stream save, copy, context manager, and close lifecycle. Reopen created bytes through the extraction path.
3. Limit individual images to 16 MiB, cumulative media and chapter content to 64 MiB each, and encoded EPUB output to 128 MiB.
4. Update selected Dublin Core title and description fields in an existing unsigned book. Preserve the uncompressed bytes of untouched chapter and resource members and the unrelated OPF fields. Reject signed and encrypted packages.

## Preview and conversion

1. Preview supported reflowable chapters as PNG with the common DPI and clipping API. Export the semantic profile to DOCX and PDF, starting each chapter on a new page.
2. Retain supported native text, tables and raster images. Return an explicit unsupported error when a semantic feature cannot be projected safely. Source CSS and fixed-layout fidelity, DRM, script execution, media overlay playback, arbitrary imported chapter edits, and other conversion pairs remain outside the supported profile.

## Verification

Run the locked Rust, license, and installed-wheel checks. Generate local fixtures to exercise EPUB 2 and 3, semantic links and tables, archive bounds, authoring and targeted edits, preview, and reopened conversion outputs. Keep all engine and shared backend Git pins immutable and compatible.
