# Node-API experiment (not a release)

This Linux-only, synchronous, in-process preview depends on the **in-progress**
Python-free `pydoccraft-core` API. It is not a published package, a stable API,
or a replacement for the Python API. No async execution or TypeScript
definitions are exposed. Calls block the
Node event loop. Document processing never invokes a subprocess or downloads code.

`detect(Buffer)` returns `pdf`, `docx`, `pptx`, or `epub`. Detection is not a
complete validity check. `convert(Buffer, source, target, password = null)` returns a new Buffer;
the preview allows PDF→DOCX and DOCX→PDF only. PPTX conversion is disabled
entirely because upstream PDF export can corrupt searchable text for explicit
paragraph breaks, and the core has no reliable guard. Conversion fidelity is
not pixel-perfect. Passwords are optional strings and are passed to the core;
password support depends on the format and upstream engine. Errors are ordinary JavaScript `Error`s
whose messages start with the core error category; these are preview behavior.
Inputs must be Node Buffers backed by non-shared ArrayBuffers; shared backing
storage is rejected to prevent concurrent mutation during native processing.
The core rejects inputs larger than 128 MiB, but conversion and output copying
can require substantially more memory; this is not a process memory limit.

`extract(Buffer, filetype, password = null)` returns the core's normalized payload as a JSON
string, including EPUB extraction. `render(Buffer, filetype, page, dpi, password = null, clip = null)` returns
a PNG Buffer; page is a zero-based non-negative integer. EPUB rendering is not
supported. Both are stateless and apply the core's input, payload and rendering
limits. A clip is `[x, y, width, height]` in displayed page points and is subject
to the core's validation. Pass `null` for the password to supply a clip without one.

`open(Buffer, filetype, password = null)` opens a PDF, DOCX or PPTX and returns a
session with `extract()`, `render(page, dpi, clip = null)`, `convert(target)`,
and `close()`. EPUB sessions are unsupported. A native session retains parsed
state and an owned extraction snapshot; returned JSON strings remain usable after
close. Session conversion is unsupported by the core and throws `unsupported:`;
use the stateless `convert` operation instead. `close()` is idempotent; other
calls after close throw `closed:` errors. A finalizer releases sessions that are
not explicitly closed. Closing does not guarantee secure erasure of memory.

`createEpub()` creates a new, in-memory editable EPUB; it cannot edit an existing
EPUB. Its synchronous methods are:

```js
const book = api.createEpub();
book.setMetadata('Title', 'urn:example:book', 'en', ['Author']);
const chapter = book.addChapter('Introduction'); // zero-based index
book.addHeading(chapter, 'Heading', 1); // levels 1–6
book.addParagraph(chapter, 'Text');
book.addLink(chapter, 'Project', 'https://example.com');
book.addImage(chapter, pngBuffer, 'image/png', 'Description');
const bytes = book.save(); // new EPUB Buffer, not a filesystem operation
const independentCopy = book.copy();
book.close(); // idempotent; also finalized when unreachable
```

`chapterCount()` reports the number of chapters. Saving requires a nonempty
title, identifier, language and at least one chapter. Text and link targets are
validated by the core; links are restricted to HTTP(S), mailto and safe
fragments. Images accept PNG, JPEG and GIF; SVG is rejected. Image bytes must
have non-shared backing storage. Because reopening currently corrupts some
XML-sensitive metadata, this preview rejects `&`, `<` and `>` in metadata and
chapter titles. Body text supports these characters. Core authoring limits
include 10,000 chapters/images, 64 MiB of body text, 16 MiB per image, 64 MiB
of images, and 128 MiB output; this binding additionally limits aggregate author
names to 1 MiB. `copy()` duplicates the bounded document and may use substantial
memory. Failed validation does not intentionally modify the document.

`createPdf(width = 612, height = 792)` creates a PDF with one blank page;
`openPdf(buffer, password = null)` opens an existing PDF for editing. Both
return an editor with `pageCount()`, `pageSize(page)` (a `[width, height]`
array), `addPage(width, height)` (returns the new zero-based index),
`addText(page, text, x, y, size = 12)`, `save()`, `copy()` and `close()`.
Page indexes may be negative; coordinates and sizes are in points, with text
positioned from the top left. Text uses the core's built-in font, black color,
and wraps to the page's right edge. Other authoring features are not exposed.
The core validates dimensions, text, item counts and input/output sizes. Editing
encrypted PDFs and imported PDFs with forms, signatures or certification is
unsupported. Saving returns a Buffer; it does not write a file. Copies are
independent and may use substantial memory. Close is idempotent; finalizers
release unreachable editors. Core closed and page-index errors currently have
unprefixed messages, unlike some other preview errors.

With Node.js and Rust 1.95+ installed, from the repository root:

```sh
cargo build --locked --manifest-path bindings/node/Cargo.toml
cp bindings/node/target/debug/libdoccraft_node_preview.so bindings/node/doccraft.node
node --test bindings/node/test/*.test.js
```

The addon uses Node-API directly without npm. It directly depends on the same
pinned serde_json already used by the core. Its independent
Cargo root repeats the parent workspace's pinned patches because dependency
patches do not propagate. `Cargo.lock` was derived from the root lockfile; its
only package additions are this local binding, and Python-only dependencies are
absent. Reconcile patches and locked versions whenever the parent changes.
An offline locked metadata review found 289 normal/build packages, all accepted by
the repository's license-expression and prohibited-package checks. The 288
third-party name/version/license rows occur in the regenerated root inventory;
the remaining row is the local Apache-2.0 `pydoccraft-core`. No packaged binary
is provided, and this review is not a production distribution approval.
The build assumes Linux shared-library linking; other platforms are untested.
See [ATTRIBUTION.md](ATTRIBUTION.md) for the preview's attribution and audit status.
