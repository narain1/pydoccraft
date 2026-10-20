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

## Rendering limits

- DPI must be finite and between 1 and 1200 inclusive.
- Python rejects out-of-range page indexes.
- PdfCraft rendering uses upstream oversize rejection.
- WordCraft and DeckCraft dimensions are checked against their upstream maximum-side constants before raster allocation.
- Each raster is limited to 64 million pixels (256 MB for one RGBA buffer). Encoding and renderer intermediates can require additional memory; this is not a total process-memory budget.
- RGBA buffers must have exactly the expected length before PNG encoding.
- Rendering returns an encoded PNG held in memory.

The numeric WordCraft and DeckCraft maximum-side values are upstream constants and are not part of the `pydoccraft` API contract.

## Limits not currently enforced locally

Version 0.2.1 does not add a package-level limit for:

- total input bytes;
- PDF page count or Office page/slide count;
- aggregate extracted image bytes;
- normalized JSON payload size;
- Python object count;
- total rendering time;
- general extraction recursion beyond limits supplied by upstream engines (DOCX body table normalization additionally caps table nesting at 16 levels).

Production callers should enforce input size and execution time outside the process. A Lambda integration should check remaining invocation time before calling extraction and should reject files whose size exceeds the workload's tested envelope.

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
