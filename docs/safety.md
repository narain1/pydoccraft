# Safety and resource behavior

`pydoccraft` parses untrusted binary documents in-process. It performs no runtime downloads, network requests, subprocess execution, or cloud access, but callers must still apply application-level upload and execution limits.

## Execution model

1. Python reads the complete source into memory.
2. The bytes cross the PyO3 boundary.
3. PdfCraft, WordCraft, or DeckCraft parses the document.
4. Rust serializes normalized extraction data as JSON.
5. Embedded image bytes cross that JSON boundary as base64 and are decoded into Python `Image.data`.

This eager design makes returned values independent of source files, but peak memory can exceed the input size substantially, especially for image-heavy documents.

## Locally enforced OOXML limits

DOCX and PPTX ZIP packages are preflighted before upstream import:

| Limit | Value |
|---|---:|
| Archive members | 10,000 |
| One uncompressed member | 128 MiB |
| Aggregate uncompressed size | 512 MiB |
| Compression ratio | 1,000:1 |

Absolute archive paths, `..` traversal components using either slash style, expansion arithmetic overflow, suspicious zero-byte compressed entries, malformed members, and excessive compression ratios are rejected.

These limits apply to the local OOXML preflight. WordCraft and DeckCraft also apply their own parser limits.

PPTX additionally checks source slide references before import. Every `sldId` element beneath a presentation root counts toward a 10,000-reference ceiling, including section lists, alternate-content branches and unused presentation parts. This conservative bound can admit fewer than 10,000 actual slides when references are duplicated; it prevents native truncation without interpreting relationships or choosing markup branches.

The check scans eligible members regardless of their path or extension, so relocated presentation parts are covered. It holds one member at a time, capped at the native XML ceiling of 96 MiB; UTF-16 BOM decoding can additionally use up to 144 MiB. Borrowed XML events avoid allocating a second token buffer or element tree. Presentation prologs are capped at 64 KiB, individual tokens at 1 MiB and nesting at 256 levels. Binary/non-presentation parts do not fail merely because they are not XML. Other-format and type-detection preflight retains the ZIP-only path. These are bounded intermediate allocations, not a whole-process memory guarantee.

## Rendering limits

- DPI must be finite and between 1 and 1200 inclusive.
- Python rejects out-of-range page indexes.
- PdfCraft rendering uses upstream oversize rejection.
- WordCraft and DeckCraft dimensions are checked against their upstream maximum-side constants before raster allocation.
- Each raster is limited to 64 million pixels (256 MB for one RGBA buffer). Encoding and renderer intermediates can require additional memory; this is not a total process-memory budget.
- RGBA buffers must have exactly the expected length before PNG encoding.
- Rendering returns an encoded PNG held in memory.

The numeric WordCraft and DeckCraft maximum-side values are upstream constants and are not part of the `pydoccraft` API contract.

## Input, extraction and remaining limits

Public native entry points reject inputs over 128 MiB, and normalized JSON serialization stops at 64 MiB. Python path/stream reads enforce the input bound before loading the complete source. Selected text requests cap selections at 10,000 pages and UTF-8 output at 64 MiB. PDF copy/import uses the same selection bound: iteration consumes at most 10,001 items, and the excess item is not converted to an index. Caller callbacks that close or reorganize the destination are rejected before the outer subset/import runs; callback side effects themselves are not rolled back. These output bounds do not prevent earlier upstream parsing/layout allocations.

DOCX structured extraction bounds auxiliary parts at 10,000, blocks/objects at 100,000, source text at 32 MiB and referenced images at 64 MiB. Table nesting is limited to 16 levels. Native DOCX extraction/render sessions reject layouts over 10,000 pages or one million placed items after pagination; the checks bound retained state, not peak layout allocation. [Numbering metadata](numbering-extraction.md) has additional path, placement and byte bounds. PPTX parsed slide indexing is limited to 10,000 entries and semantic tables to 100,000 total cells, with at most 1,000 rows/columns per table.

Imported DOCX custom-property projection allows at most 1,024 parsed entries, 64 KiB per value and 256 KiB aggregate raw names/kinds/values. Unsupported or malformed scalar variants and ASCII-case-insensitive duplicate names are omitted with bounded diagnostics. These checks follow native parsing; the native reader can discard malformed parts or cap source traversal before the adapter sees them. Table alternative-text bytes count toward the existing 32 MiB extraction text budget. Authored page/count fields are limited to 10,000 across all stories.

There is no universal process-memory, Python-object-count or execution-time budget. Eager PDF page traversal, native Office pagination and format-specific parsing can allocate before normalized output limits apply. Image accounting and recursion guards differ by format; the JSON ceiling is not a pre-decode media bound. Callers should enforce upload and execution limits appropriate to their workload, including remaining invocation time in serverless applications.

## Image behavior

Image extraction can temporarily retain multiple representations: source bytes, upstream decoded/exported bytes, base64 JSON, and Python bytes. This is the largest predictable source of memory amplification.

PDF image export may skip unsupported image encodings or grouped Form artwork. DOCX and PPTX return referenced media rather than every file present in the package. Malformed image bytes can be preserved as `Image.data` without being decoded during extraction; rendering may reject content that plain extraction accepted.

## Passwords and encryption

- PDF and DOCX accept a password through `doc_ex.open(..., password=...)` and `doc_ex.convert(..., password=...)`.
- Missing and incorrect passwords are surfaced as `MalformedDocumentError`.
- Encrypted PPTX is detected and rejected; PPTX password decryption is not implemented.
- Passwords remain in process memory for the duration of the call. Do not log them.

## Error handling

Native panics are not an accepted error path. Format adapters convert upstream errors into stable native categories, which Python maps to public exceptions. Resource-limit errors currently map to `UnsafeArchiveError`, including render limits that are not archive-specific.

Region rendering validates finite positive clip dimensions, page intersection, quantized pixel size and returned raster dimensions. Invalid clip arguments raise `ValueError`; oversized valid regions retain `UnsafeArchiveError`. PDF tile requests reject native side clamping and imprecise large offsets. Office regions use native transforms instead of rendering and cropping a full-page buffer. Existing input/parser/layout limits still apply, and DeckCraft's upstream blank-image panic fallback remains unresolved.

Applications should treat all extraction failures as file-local. Do not retry malformed or unsupported files indefinitely.

## Deployment guidance

- Run the extension as a non-root user.
- Use a read-only filesystem when possible; parsing does not require writes.
- Apply request body and upload limits before loading bytes.
- Constrain process memory and execution time.
- Keep `Cargo.lock` and upstream Git revisions pinned.
- Upgrade upstream engines only with corpus tests and a new license audit.
- Do not expose raw exception details directly to untrusted clients; they may include parser context.

## Input and normalized output boundaries (unreleased)

All public native detect/extract/render/convert entry points reject inputs over 128 MiB. Python path and binary-stream reads stop at that limit; bytearray/memoryview inputs are checked by byte size before copying. Caller-owned streams are not closed. The native JSON writer stops at 64 MiB during serialization, including escaped/UTF-8 bytes, instead of allocating the whole oversized result first. Allocation failure maps to a resource-limit error.

This is not a streaming document parser or a total process-memory guarantee: upstream parsing/layout and owned models have separate limits and allocations. DOCX auxiliary traversal bounds parts, blocks/objects, text, referenced media and nested grids. Source examples in diagnostic notices are bounded independently from total occurrence counts. All existing archive and raster guards remain in force.
