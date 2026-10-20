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

Inputs were generated before timing with PyMuPDF, python-docx, python-pptx, and openpyxl so pydoccraft did not generate its own benchmark corpus. OOXML ZIP timestamps and core-property timestamps are normalized. PyMuPDF may generate a different PDF file identifier on another run; the hash below identifies the exact bytes used here.

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

## API and performance rough edges

### 1. Rendering repeats document work

`Document.render_page()` calls the native renderer with retained source bytes, but the native path reopens the document. The public `open + render first page` operation was 2.41× slower than PyMuPDF in this run. For DOCX, repeated rendering also recomputes layout.

**Fix direction:** add a bounded native document/session owner that reuses parse and layout state across renders while preserving deterministic `close()`, stable errors, and owned extracted values. Track this under [W02 rendering correctness, bounded execution and reuse](https://github.com/narain1/pydoccraft/issues/4).

### 2. Plain-text callers pay for eager structured extraction

`doc_ex.open()` always constructs the complete normalized model before returning. A caller wanting only plain text pays for table/link/image traversal, JSON serialization/deserialization, media base64 transfer, and Python model hydration. The matched PDF text operation was 3.25× slower than PyMuPDF.

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

Run from the repository root in an isolated environment. The benchmark uses the published wheel, not `src/`:

```console
python3 -m venv /tmp/pydoccraft-benchmark
/tmp/pydoccraft-benchmark/bin/python -m pip install -r benchmarks/requirements-lock.txt
PYTHONPATH= /tmp/pydoccraft-benchmark/bin/python benchmarks/benchmark_documents.py \
  --pages 25 \
  --lines 40 \
  --spreadsheet-rows 1000 \
  --spreadsheet-columns 20 \
  --warmups 2 \
  --repeats 7 \
  --dpi 144 \
  --repository-revision "$(git rev-parse HEAD)" \
  --output /tmp/pydoccraft-benchmark-results/results.json
```

The command writes raw JSON and generated Markdown outside the repository. Do not commit generated documents, rendered images, converted outputs, profiles, virtual environments, or timing dumps.

Run results vary with hardware, power state, thermal state, operating system, Python version, library version, and document structure. Repeat on representative real documents and every supported release platform before making broad performance claims.
