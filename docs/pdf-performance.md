# PDF performance versus PyMuPDF

This is the canonical performance comparison. It compares only pydoccraft and PyMuPDF on anonymous local PDF inputs. Lower elapsed time is better; filenames, paths, hashes, and extracted text are not recorded.

## Post-optimization results

Environment: Linux ARM64 release wheel, CPython 3.13, PyMuPDF 2.0, one warmup and five to seven measured iterations.

| ID | Pages | Scenario | pydoccraft | PyMuPDF | Result |
|---|---:|---|---:|---:|---|
| PDF-1 | 391 | All-page plain text | 505.63 ms | 245.44 ms | PyMuPDF 2.06× faster |
| PDF-2 | 65 | All-page plain text | 79.14 ms | 34.44 ms | PyMuPDF 2.30× faster |
| PDF-3 | 14 | All-page plain text | 21.13 ms | 17.27 ms | PyMuPDF 1.22× faster |
| PDF-1 | 391 | Cold open + page-1 RGBA PNG | 1,126.25 ms | 24.62 ms | PyMuPDF 45.75× faster |
| PDF-2 | 65 | Cold open + page-1 RGBA PNG | 207.79 ms | 46.33 ms | PyMuPDF 4.49× faster |
| PDF-3 | 14 | Cold open + page-1 RGBA PNG | 176.35 ms | 97.11 ms | PyMuPDF 1.82× faster |
| PDF-1 | 391 | Warm page-1 RGBA PNG | 6.45 ms | 18.42 ms | **pydoccraft 2.86× faster** |
| PDF-2 | 65 | Warm page-1 RGBA PNG | 24.30 ms | 44.67 ms | **pydoccraft 1.84× faster** |
| PDF-3 | 14 | Warm page-1 RGBA PNG | 72.00 ms | 80.72 ms | **pydoccraft 1.12× faster** |

Warm rendering excludes document-open time for both libraries. Cold rendering includes each public open path. pydoccraft's `open()` eagerly builds the complete structured model, while PyMuPDF opens lazily; cold rendering therefore measures different amounts of useful work and exposes first-page latency rather than rasterizer speed alone.

## Optimizations completed

1. **Persistent PDF render parser:** merged PR #32 retains the PdfCraft parser created during eager extraction. Warm pydoccraft rendering now beats PyMuPDF on all three anonymous inputs.
2. **Selective text API:** merged PR #34 added `doc_ex.extract_text()`, avoiding tables, images, normalized JSON, and Python object hydration when callers only need text.
3. **Single-parse valid PDF text path:** this branch removes the redundant `inspect()` parse before `PageRenderer` in `extract_text()`. `inspect()` remains an error-only fallback so malformed/password errors retain their categories.

Before the single-parse change, anonymous all-page pydoccraft text medians were 557.33, 82.04, and 21.04 ms. Afterward they were 505.63, 79.14, and 21.13 ms: approximately 1.10×, 1.04×, and 1.00×. The remaining all-page gap is inside page text interpretation rather than Python hydration.

## Remaining bottleneck

Full structured `doc_ex.open()` deliberately does more than plain-text extraction. Each PDF page currently performs:

- one PdfCraft text interpretation;
- a separate PdfCraft edit-oriented `reading_blocks` interpretation for conservative table detection;
- an unconditional `reading_images` content-stream walk;
- link-label geometry mapping;
- bounded JSON serialization and Python model hydration.

The duplicate text and image walks dominate structured-open time. They cannot be safely replaced with a local PDF heuristic without changing table/image semantics. The next material optimization belongs upstream in PdfCraft: share an authoritative page interpretation/table candidate representation and let `reading_images` return early when effective resources contain no XObjects. Any PdfCraft revision update remains a dedicated change with full corpus, license, and wheel-size validation.

## Reproduce

```console
python3.13 -m venv /tmp/pydoccraft-pdf-benchmark
/tmp/pydoccraft-pdf-benchmark/bin/python -m pip install pydoccraft PyMuPDF==2.0
PYTHONPATH= /tmp/pydoccraft-pdf-benchmark/bin/python \
  benchmarks/benchmark_pdf_vs_pymupdf.py /path/to/pdf/corpus \
  --warmups 1 --repeats 5 --dpi 144 \
  --output /tmp/pydoccraft-pdf-results.json
```

The benchmark validates text page counts, requires at least 95% multiset token overlap, and validates PNG dimensions after timing. The measured anonymous token overlaps were 96.47%, 100%, and 99.02%; timing does not establish reading-order or visual fidelity. Results vary by document structure, platform, power state, and thermal state. Use the same bytes and release wheels for both libraries; do not infer fidelity from timing alone.
