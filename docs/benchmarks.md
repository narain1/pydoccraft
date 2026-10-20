# Performance benchmarks

This page records a reproducible synthetic baseline for the published `pydoccraft` 0.2.1 wheel. It is not a claim that one library is universally faster, and it does not measure the unreleased authoring work currently on `main`.

## Headline result

On the matched PDF operations in this run, **PyMuPDF was faster**:

- plain-text extraction: **3.25× faster** by median elapsed time;
- first-page RGBA PNG rendering at 144 DPI: **2.41× faster** by median elapsed time.

Those are slowdowns to address, not pydoccraft speedups. pydoccraft performs eager normalized extraction when opening a document, including Python object hydration, and its render call currently reopens the source. PyMuPDF opens PDFs lazily and exposes a narrower PDF-specific path in these scenarios.

pydoccraft's distinctive measurements are its in-process Office extraction and conversion operations. Core PyMuPDF and openpyxl do not provide equivalent PDF↔DOCX or PPTX→PDF operations, so those numbers are capability baselines rather than winner/loser comparisons.

## Results

Run at 2026-10-20 16:44 UTC. Times are medians of seven measured iterations after two warmups; lower is better.

| Category | Operation | Library | Median | IQR | Min–max |
|---|---|---:|---:|---:|---:|
| PDF | Open + extract all plain text | pydoccraft | 78.42 ms | 76.59–78.82 ms | 75.01–86.56 ms |
| PDF | Open + extract all plain text | PyMuPDF | 24.10 ms | 24.08–24.34 ms | 24.03–24.46 ms |
| PDF | Open + render first page to RGBA PNG at 144 DPI | pydoccraft | 96.90 ms | 96.15–98.84 ms | 95.08–100.67 ms |
| PDF | Open + render first page to RGBA PNG at 144 DPI | PyMuPDF | 40.22 ms | 40.11–41.55 ms | 39.98–42.43 ms |
| PDF | Open + rewrite PDF bytes | PyMuPDF | 11.97 ms | 11.92–12.10 ms | 11.86–12.72 ms |
| DOCX | Open + extract all plain text | pydoccraft | 20.16 ms | 19.34–20.53 ms | 18.61–22.00 ms |
| PPTX | Open + extract all plain text | pydoccraft | 5.07 ms | 5.06–5.44 ms | 4.73–5.94 ms |
| Conversion | PDF → DOCX | pydoccraft | 9.84 ms | 9.65–10.02 ms | 9.24–11.12 ms |
| Conversion | DOCX → PDF | pydoccraft | 22.67 ms | 22.36–22.75 ms | 22.07–23.33 ms |
| Conversion | PPTX → PDF | pydoccraft | 154.19 ms | 148.39–156.44 ms | 147.22–160.07 ms |
| XLSX | Open + iterate all 20,000 cell values | openpyxl | 75.12 ms | 71.21–75.26 ms | 69.97–78.58 ms |
| XLSX | Create workbook + write XLSX bytes | openpyxl | 52.90 ms | 52.48–53.09 ms | 52.21–53.49 ms |

The 25-page PDF text rates were approximately 319 pages/s for pydoccraft and 1,039 pages/s for PyMuPDF. Throughput is derived from this small in-memory corpus and should not be extrapolated to arbitrary documents.

## Anonymized local-file results

A second run used five local files from `.test_files`: three PDFs, one DOCX, and one PPTX. The files are not committed or distributed. Results intentionally omit filenames, paths, hashes, and extracted text; anonymous IDs are assigned by format and ascending byte size.

Configuration: one warmup and five measured iterations on the same Apple M5 Pro host, with first-page RGBA rendering at 144 DPI.

| ID | Type | MiB | Pages/slides | Operation | Library | Median | IQR |
|---|---|---:|---:|---|---|---:|---:|
| DOCX-1 | DOCX | 0.03 | 2 | Extract text | pydoccraft | 5.18 ms | 5.13–5.30 ms |
| DOCX-1 | DOCX | 0.03 | 2 | Render page 1 | pydoccraft | 16.49 ms | 16.21–16.52 ms |
| DOCX-1 | DOCX | 0.03 | 2 | Convert to PDF | pydoccraft | 6.65 ms | 6.60–6.73 ms |
| PDF-1 | PDF | 1.58 | 391 | Extract text | pydoccraft | 1,278.19 ms | 1,274.49–1,284.31 ms |
| PDF-1 | PDF | 1.58 | 391 | Extract text | PyMuPDF | 406.03 ms | 405.80–406.68 ms |
| PDF-1 | PDF | 1.58 | 391 | Render page 1 | pydoccraft | 1,364.63 ms | 1,361.99–1,368.14 ms |
| PDF-1 | PDF | 1.58 | 391 | Render page 1 | PyMuPDF | 22.81 ms | 22.60–23.00 ms |
| PDF-1 | PDF | 1.58 | 391 | Convert to DOCX | pydoccraft | 759.18 ms | 744.07–769.48 ms |
| PDF-2 | PDF | 2.24 | 65 | Extract text | pydoccraft | 204.70 ms | 204.40–206.73 ms |
| PDF-2 | PDF | 2.24 | 65 | Extract text | PyMuPDF | 44.77 ms | 44.58–44.89 ms |
| PDF-2 | PDF | 2.24 | 65 | Render page 1 | pydoccraft | 232.06 ms | 230.72–233.87 ms |
| PDF-2 | PDF | 2.24 | 65 | Render page 1 | PyMuPDF | 42.86 ms | 42.57–42.90 ms |
| PDF-2 | PDF | 2.24 | 65 | Convert to DOCX | pydoccraft | 107.61 ms | 107.06–107.98 ms |
| PDF-3 | PDF | 13.24 | 14 | Extract text | pydoccraft | 104.47 ms | 104.16–104.68 ms |
| PDF-3 | PDF | 13.24 | 14 | Extract text | PyMuPDF | 25.09 ms | 24.96–25.10 ms |
| PDF-3 | PDF | 13.24 | 14 | Render page 1 | pydoccraft | 174.28 ms | 173.49–174.67 ms |
| PDF-3 | PDF | 13.24 | 14 | Render page 1 | PyMuPDF | 90.51 ms | 90.16–91.85 ms |
| PDF-3 | PDF | 13.24 | 14 | Convert to DOCX | pydoccraft | 69.38 ms | 68.31–70.04 ms |
| PPTX-1 | PPTX | 8.47 | 14 | Extract text | pydoccraft | 41.93 ms | 41.44–42.35 ms |
| PPTX-1 | PPTX | 8.47 | 14 | Render page 1 | pydoccraft | 72.73 ms | 72.17–72.87 ms |
| PPTX-1 | PPTX | 8.47 | 14 | Convert to PDF | pydoccraft | 168.11 ms | 167.72–170.92 ms |

Matched PDF extraction favored PyMuPDF by 3.15×, 4.57×, and 4.16×. Matched first-page rendering favored PyMuPDF by 59.83×, 5.41×, and 1.93×. The 391-page case makes the main rough edge especially clear: pydoccraft eagerly extracts the complete document before rendering page 1, so first-page latency scales with total document work.

PyMuPDF PDF rewrite medians, retained in the machine-readable local run but omitted from the compact table above because there is no pydoccraft equivalent, were 204.05 ms, 38.47 ms, and 10.01 ms for PDF-1 through PDF-3.

## What is comparable

The two paired PDF scenarios produce the same class of caller-visible result:

1. open the same PDF bytes and collect plain text from every page;
2. open the same PDF bytes and encode page 1 as PNG at 144 DPI.

Both rendering paths include PNG encoding. Generation, fixture validation, and checksums are outside timed regions.

The implementations still do different amounts of work. `doc_ex.open()` eagerly detects, parses, normalizes, JSON-decodes, base64-decodes media, and hydrates owned Python values for the full document. PyMuPDF's PDF open is lazy. In the render scenario, pydoccraft therefore extracts all 25 pages before rendering page 1, while PyMuPDF does not. This is representative of the public APIs today but is not a renderer-microbenchmark.

## What is not comparable

- **openpyxl is an XLSX library.** pydoccraft does not support XLSX, and openpyxl does not parse, render, or convert PDF/DOCX/PPTX. Its rows are included only as an independently labeled spreadsheet baseline.
- **Conversion has no matched competitor here.** Core PyMuPDF and openpyxl do not expose the same PDF→DOCX, DOCX→PDF, or PPTX→PDF semantic conversions.
- **PDF rewrite is PyMuPDF-only.** Published pydoccraft 0.2.1 has no equivalent identity-save operation.
- Timing does not establish extraction or conversion fidelity. The harness performs structural checks, but it does not compare visual similarity or complete semantic equivalence.

## Corpus and correctness checks

Inputs were generated before timing with PyMuPDF, python-docx, python-pptx, and openpyxl so pydoccraft did not generate its own benchmark corpus. The corrected harness normalizes OOXML ZIP timestamps and core-property timestamps after serialization. The recorded run preceded the post-serialization correction: openpyxl overwrote its modified timestamp during save, so its historical XLSX hash identifies that run rather than a reproducible file. The correction changes fixture metadata and hashes without changing cell values or timed operations. PyMuPDF may generate a different PDF file identifier on another run; the hash below identifies the exact bytes used here.

| Input | Shape | Bytes | SHA-256 |
|---|---|---:|---|
| PDF | 25 pages × 40 text lines, one URI link/page | 246,444 | `183286bbd741d31637391e54314830572263f777cf3303b83e66348e0de6bc54` |
| DOCX | 25 explicit pages, paragraphs and 5×4 tables | 39,192 | `7529fcc691f1f6aea329f90df4a9969fcafc358ee7933eb6e1b510bfad4e4acb` |
| PPTX | 25 slides, text and 5×4 tables | 59,476 | `1bf619856c92bd797dc24b22ab91ff60e85f9f4defd41681936845c3a9a20684` |
| XLSX | 1,000 rows × 20 string cells | 71,663 | `2be89e05967696711878232a07bf47a695799ad320f595ccfe0e86d72f6b2156` |

The harness verified page counts, nonempty text, PNG signatures, all 20,000 XLSX values, and successful pydoccraft reopen/text extraction for every conversion. Output sizes were 5,367 bytes for PDF→DOCX, 162,387 bytes for DOCX→PDF, and 4,904,284 bytes for PPTX→PDF.

pydoccraft extracted 68,999 PDF text characters and PyMuPDF extracted 69,024. Inspection indicates a normalization difference of one character per page, consistent with trailing line/page separators. The benchmark intentionally does not force the outputs to match byte-for-byte because the public text-normalization contracts differ.

## Environment

- Hardware: Apple M5 Pro (`arm64`)
- Operating system: macOS 26.6.2
- Python: CPython 3.13.12
- Base repository revision: `7a1559ca775c16e52927159cd1fec315bba085e8` (`v0.2.1-45-g7a1559c`); the benchmark itself was an uncommitted addition during the run
- pydoccraft: 0.2.1 installed from its macOS ARM64 wheel
- pydoccraft wheel SHA-256: `6a23842cabce0c0463f24bb57aa8de31f396deb00d21f6c7c720a75db04b477d`
- PyMuPDF: 2.0
- openpyxl: 3.1.5
- Fixture writers: python-docx 1.2.0 and python-pptx 1.0.2

`benchmarks/requirements-lock.txt` records the complete Python environment, including transitive packages.

## Root-cause stage profile

A separate diagnostic run decomposed the anonymous PDF path. These timings are medians of five iterations and are not additional public-API comparisons; `_core` calls are private implementation probes.

| ID | Pages | Detect | Native full extraction | JSON decode | Python hydration | Public open | Direct native page-1 render | Public open + render | PyMuPDF open + render |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| PDF-1 | 391 | <0.01 ms | 1,281.93 ms | 23.01 ms | 7.56 ms | 1,307.87 ms | 70.63 ms | 1,381.57 ms | 21.78 ms |
| PDF-2 | 65 | <0.01 ms | 193.95 ms | 7.02 ms | 4.16 ms | 202.44 ms | 26.67 ms | 229.85 ms | 42.89 ms |
| PDF-3 | 14 | <0.01 ms | 76.41 ms | 17.42 ms | 11.29 ms | 104.78 ms | 70.30 ms | 174.73 ms | 90.26 ms |

The evidence identifies four costs:

1. **Whole-document extraction is the primary bottleneck.** Native extraction accounts for approximately 98%, 96%, and 73% of public open latency in these three cases. `doc_ex.open()` renders text and traverses tables, links, and images for every page even when the caller only wants page count or a first-page image.
2. **Public render performs two independent pipelines.** `open()` first performs full extraction; `render_page()` then sends the retained bytes through a fresh native inspect and `PageRenderer`. Public first-page time closely tracks `public open + direct native render`.
3. **The renderer itself is not uniformly slow.** The private direct-native probe was 1.61× faster than PyMuPDF's public open/render path on PDF-2 and 1.28× faster on PDF-3, while PyMuPDF was 3.24× faster on PDF-1. The 59.83× public-path gap therefore cannot be attributed to rasterization alone.
4. **JSON and Python hydration are secondary but visible.** They add roughly 12–29 ms here and become proportionally significant for the 13.24 MiB/14-page document. Image-heavy inputs may add base64 expansion and decoding beyond what this stage profile isolates.

The highest-value feature improvement is a bounded private native document session: inspect/open once, preserve reusable parse/render state, extract pages or text profiles on demand, and drop native state deterministically on `close()`. Existing hydrated values must remain owned, and all resource limits and stable errors must remain intact. A smaller interim improvement is a bounded direct-render or selected-page extraction entry point that avoids mandatory whole-document normalization.

## API and performance rough edges

### 1. Rendering repeats document work

`Document.render_page()` calls the native renderer with retained source bytes, but the native path reopens the document. The public `open + render first page` operation was 2.41× slower than PyMuPDF in this run. For DOCX, repeated rendering also recomputes layout.

**Fix direction:** add a bounded native document/session owner that reuses parse and layout state across renders while preserving deterministic `close()`, stable errors, and owned extracted values. Track this under [W02 rendering correctness, bounded execution and reuse](https://github.com/narain1/pydoccraft/issues/4).

### 2. Plain-text callers pay for eager structured extraction

`doc_ex.open()` always constructs the complete normalized model before returning. A caller wanting only plain text pays for eager table/link traversal, JSON serialization/deserialization, and Python model hydration on this text-and-link corpus. Image-bearing documents additionally pay media transfer costs, but that overhead was not measured by the synthetic PDF scenario. The matched PDF text operation was 3.25× slower than PyMuPDF.

**Fix direction:** investigate an upstream-backed bounded text-only or selective extraction path rather than weakening the default owned structured model. Keep explicit limits and stable errors. Track extraction shape and selective processing under [W09 extraction structure and portable output](https://github.com/narain1/pydoccraft/issues/11).

### 3. `get_pixmap()` returns bytes, not a Pixmap-like object

The familiar method name suggests PyMuPDF's `Pixmap`, but pydoccraft deliberately returns encoded PNG bytes. Callers cannot inspect width, height, stride, channels, or alpha without decoding the PNG. `render_page()` is accurately named, while `get_pixmap()` is only API-shape compatibility.

**Fix direction:** preserve the existing bytes return contract, but consider a separate metadata/result API if real use cases justify it. Do not silently change `get_pixmap()` and break callers. Track API consistency under [W01 compact Python API](https://github.com/narain1/pydoccraft/issues/3).

### 4. PDF plain-text separators differ

The same synthetic PDF produced 25 more characters in PyMuPDF—one per page—than in pydoccraft. Human-readable content was present in both, but exact downstream checksums differ.

**Fix direction:** specify pydoccraft's trailing newline/page-separator policy and add exact normalization tests. If source mapping is added, offsets must refer to the documented normalized representation. Track this under [W09](https://github.com/narain1/pydoccraft/issues/11).

### 5. Conversion timing lacks structured diagnostics

Conversions reopen successfully, but elapsed time and valid output do not tell callers where fonts, layout, links, or unsupported constructs were approximated.

**Fix direction:** return or expose structured conversion notices without turning unsupported or unsafe behavior into silent success. Track lifecycle and diagnostics under [W03](https://github.com/narain1/pydoccraft/issues/5).

## Reproduce

Run from the repository root in an isolated Python 3.13 environment (the recorded lock was resolved with Python 3.13). The benchmark uses the published wheel, not `src/`:

```console
python3.13 -m venv /tmp/pydoccraft-benchmark
/tmp/pydoccraft-benchmark/bin/python -m pip install -r benchmarks/requirements-lock.txt
PYTHONPATH= /tmp/pydoccraft-benchmark/bin/python benchmarks/benchmark_documents.py \
  --pages 25 \
  --lines 40 \
  --spreadsheet-rows 1000 \
  --spreadsheet-columns 20 \
  --warmups 2 \
  --repeats 7 \
  --dpi 144 \
  --harness-revision "$(git rev-parse HEAD)" \
  --output /tmp/pydoccraft-benchmark-results/results.json
```

The command writes raw JSON and generated Markdown outside the repository. The `--output` path must not end in `.md`, which is reserved for the companion report. Run focused harness checks with `/tmp/pydoccraft-benchmark/bin/python -m unittest discover -s benchmarks -v`. Do not commit generated documents, rendered images, converted outputs, profiles, virtual environments, or timing dumps.

Run results vary with hardware, power state, thermal state, operating system, Python version, library version, and document structure. Ratios are observed-run estimates: scenarios were measured in a fixed sequence, so future harness work should interleave matched libraries and add repeated process-level runs before treating small differences as stable. Repeat on representative real documents and every supported release platform before making broad performance claims.


To measure your own files without including their filenames, paths, hashes or extracted text in reports:

```console
PYTHONPATH= /tmp/pydoccraft-benchmark/bin/python benchmarks/benchmark_test_files.py /path/to/local/corpus \
  --warmups 1 --repeats 5 --dpi 144 \
  --output /tmp/pydoccraft-benchmark-results/local.json
```

The corpus script validates CLI counts and keeps JSON/Markdown output paths distinct. Its Python failure messages omit reader/filesystem exception details; successful reports contain anonymous IDs, format, file size, page/slide/row counts and timings. File sizes/counts are still document metadata, so review reports before sharing them. Third-party backend diagnostics are not a promised redaction boundary. Serialized byte length is excluded from repeatability guards because metadata can change it without changing content.
