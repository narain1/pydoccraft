# Experimental Go binding

This is a source preview, not a published or self-contained Go distribution.
The repository's [license](../../LICENSE), [notice](../../NOTICE), and
[third-party inventory](../../THIRD_PARTY_LICENSES.md) apply. A packaged
distribution must carry reviewed attribution and native libraries before release.

This initial Linux/cgo binding calls the Python-free Rust core in-process. It
exposes byte-based detection, JSON extraction, PNG rendering (including clipping),
conversion, reusable render sessions, and bounded EPUB authoring, but not the
Python compatibility API.
The extraction JSON follows the Rust `DocumentPayload` schema and is not a typed
Go API. Extraction supports PDF, DOCX, PPTX and EPUB; rendering supports PDF,
DOCX and PPTX, not EPUB. Page indexes are nonnegative and zero-based; clip values
are in displayed page points. Detection recognizes PDF,
DOCX, PPTX and EPUB; conversion is limited to PDF→DOCX and DOCX→PDF.
The native engines' fidelity limitations and resource limits still apply; this
is not pixel-perfect conversion. In particular, native PPTX PDF export does not
support paragraphs with explicit line breaks because upstream corrupts their
searchable text, so PPTX conversion is disabled in this preview. Detection is
identification, not a full validity check.

Build from a checkout with Rust 1.95+, Go 1.20+, cgo and a C linker installed:

```sh
sh bindings/go/build.sh
cd bindings/go
CGO_LDFLAGS="-L$PWD/native" LD_LIBRARY_PATH="$PWD/native${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" go test ./...
```

To keep the Cargo artifacts, native library and temporary files off the checkout
filesystem, use absolute paths on a filesystem with enough free space:

```sh
export CARGO_TARGET_DIR=/path/to/large/target
export PYDOCCRAFT_GO_LIB_DIR=/path/to/large/native
export TMPDIR=/path/to/large/tmp
mkdir -p "$TMPDIR"
sh bindings/go/build.sh
cd bindings/go
CGO_LDFLAGS="-L$PYDOCCRAFT_GO_LIB_DIR" \
  LD_LIBRARY_PATH="$PYDOCCRAFT_GO_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" go test ./...
```

`PYDOCCRAFT_GO_LIB_DIR` defaults to `bindings/go/native` relative to the
repository root. `TMPDIR` defaults to the native output directory. Set
`CGO_LDFLAGS` when building Go programs, and configure the dynamic loader at
runtime; the library is not embedded in the Go binary.

The build uses the root manifest, its patches, and the committed lockfile with
`--locked --no-default-features`; it creates no separate Cargo dependency graph.
Dependencies must already be available for an offline build. The generated
shared library is not committed. Consumers must build and distribute it alongside
the Go binary and configure the platform's dynamic loader; there are no runtime
downloads or subprocess document processing. This build script currently targets
Linux only. It uses Cargo's debug profile and respects `CARGO_TARGET_DIR`.
Use a dedicated target directory if it contains multiple `serde_json` rlibs;
the build fails rather than selecting an incompatible artifact.

Validation: on Linux x86-64, a Rust 1.95 container and Debian Go 1.24.4 built
the binding with Cargo offline and ran all ten Go tests, including with the Go
race detector, with `GOPROXY=off`. An earlier stateless build also passed with
`PATH=/nonexistent`, and `ldd` showed no Python library dependency. These checks
cover the included DOCX fixture, PDF conversion, a PDF search fixture and an
authored EPUB reopen, not
every supported format or platform. The Go race detector does not instrument Rust.

`Error.Code` values are 1 invalid argument, 2 unsupported, 3 malformed, 4 unsafe
archive, 5 resource limit, 6 encrypted, 7 conversion failure, 8 other engine
error, 9 caught Rust panic, 10 closed, 11 page index, and 12 rendering failure.
Existing engines may report an out-of-range page using another code. Diagnostics
are not stable. Panics are caught at
the ABI boundary when unwinding is enabled; process aborts, allocation failures,
and invalid C pointers cannot be recovered. The C API requires valid input memory
and exactly-once freeing of each unmodified returned buffer. Go copies results
before freeing native memory. Neither API should be used with untrusted callers
that can forge pointers.

`OpenSession` returns a session and separately owned JSON payload. Close every
session explicitly; close is idempotent and waits for in-flight session calls.
Do not copy a Session. Render and PDF-only SearchFor use zero-based page indexes;
search rectangles are `[x0, y0, x1, y1]` in displayed page points. Sessions do
not support EPUB. Returned payloads and images remain usable after Close.

`NewEPUB` creates a new editable book; it cannot import or edit existing EPUBs.
Set title, identifier and language and add at least one chapter before `Save`.
Chapters accept headings, plain paragraphs, links and PNG/JPEG/GIF images. Images
are checked by signature, not decoded or guaranteed valid; SVG is rejected.
Links are limited to HTTP(S), mailto and safe fragments. Chapters use zero-based
indexes. `Save` returns bytes, not a file, and `Copy` creates an independent book.
Close each book explicitly and do not copy its Go struct. Calls serialize with
close. Native limits include 10,000 chapters and images, 64 MiB each for content
and total image data, 16 MiB per image and 128 MiB output; this Go binding also
limits aggregate metadata to 1 MiB. The API does not fetch links or image data.
