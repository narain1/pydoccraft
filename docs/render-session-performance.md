# Render-session reuse evidence

The bounded P1 criterion is complete: PDF retains its native render parser, DOCX retains its parsed model and layout, and PPTX retains its imported model. Existing lifetime/error and mixed-page full/clip tests combine with 66 fresh-process synthetic samples: [36 Office samples](office-performance.md#larger-workloads-and-repeated-processes) and the 30 PDF samples below. This establishes reuse and within-format output parity on the tested workloads. It does not promise universal speedups, a representative real-world corpus, a process-memory ceiling or verification on every release platform; QA7 and P4 retain those separate gaps.

## PDF workload and method

Measured on 2026-10-21 UTC, Linux x86_64 / Python 3.11.2 / Rust 1.95 / AMD Ryzen 9 9955HX, using the exact paragraph-movement wheel from runtime `43fd15cc870f633947ab6ff420bb4591bc5dc9aa` (SHA-256 `ffb815e29b7429ba7e6d9b8b74e6dbc1be9b68888edcb59a24665c3a8b5fa1b9`). [Reviewed JSON evidence](pdf-session-performance-results.json) contains every sample, source/tool hashes, native counters, timing components, memory readings and ordered all-page PNG digest. The earlier Office result file retains its historical tool hashes.

Five native-exported PDFs cover 20-page report/deck baselines, a 200-page report, and 24-page report/deck files with distinct 512×512 images. Three fresh sequential processes per input/mode render the first page, five warm repeats and every page at 144 DPI; mode order alternates between repeats. Local validation/build work was paused during measurement. Both modes use the same wheel and eager structured extraction. The one-shot mode closes the retained session before rendering, then uses the existing one-shot path on every request. Input hashing warms the filesystem cache before timing; these are not disk-cold or historical-wheel measurements.

| PDF origin | Pages | Source bytes | Cached all pages ms | One-shot all pages ms | Ratio |
|---|---:|---:|---:|---:|---:|
| 20-page report | 20 | 125,926 | 271.2 | 268.4 | 0.99× |
| 20-page deck | 20 | 3,014,860 | 269.2 | 286.2 | 1.06× |
| 200-page report | 200 | 1,172,232 | 2,750.8 | 3,349.5 | 1.22× |
| 24-page image report | 24 | 19,032,934 | 391.9 | 471.4 | 1.20× |
| 24-page image deck | 24 | 14,970,673 | 433.8 | 520.7 | 1.20× |

Times are medians of three full-document totals; ratios divide one-shot by cached time. The 20-page report is slightly slower with retention (0.99×). These results show why parse reuse must not be advertised as a universal speedup. They do not compare PyMuPDF, rasterizer-only performance or cross-format conversion fidelity.

| PDF origin | Cached peak RSS MiB | One-shot peak RSS MiB |
|---|---:|---:|
| 20-page report | 48.7–55.7 | 48.5–49.1 |
| 20-page deck | 81.5–82.3 | 81.9–82.3 |
| 200-page report | 58.0–58.5 | 59.3–61.7 |
| 24-page image report | 137.8–140.8 | 137.2–139.4 |
| 24-page image deck | 142.6–143.8 | 142.4–143.7 |

Memory ranges are minimum–maximum high-water RSS across three processes, measured before close. Retention does not always reduce peak RSS; the report baseline and image workloads demonstrate that limitation. Current/peak RSS use different OS counters and include native/global caches and allocator retention. They do not measure only the document, prove reclamation, or cap earlier native allocation. Close/lifetime behavior is verified separately by the installed corpus.

Every first-page and ordered all-page PNG digest matches between modes and across repetitions. The retained PdfCraft `PageRenderer` starts with one parser build, `pages` text requests and zero rasterizations. It ends with the same one parser build, `2*pages + 6` requests and `pages + 6` rasterizations. One-shot mode has no retained final session. These counters describe this renderer; they do not count separate eager structured-extraction walks or prove that every PDF subsystem parses only once.

## Reproduce

The shared development runner supports PDF, DOCX and PPTX. The previous `benchmark_office_sessions.py` entry point remains available and preserves its Office counter field; named counters distinguish native PDF and Office meanings. No package API or runtime dependency changes.

```console
python benchmarks/generate_office_session_inputs.py /tmp/pdf-baseline --pdf
python benchmarks/generate_office_session_inputs.py /tmp/pdf-images --pages 24 --image-pixels 512 --unique-images --image-points 144 --pdf
python benchmarks/benchmark_render_sessions.py /tmp/pdf-baseline/report.pdf --mode cached --dpi 144 --rounds 5
python benchmarks/benchmark_render_sessions.py /tmp/pdf-baseline/report.pdf --mode one-shot --dpi 144 --rounds 5
```

Generate the 200-page report with `--pages 200` and convert its `report.docx` through the existing `document.convert("pdf")` API. This evidence does not include a 200-slide PDF export. Repeat both modes three times for each of the five PDFs, alternating order. Require equal source hash, actual page count, format, DPI and ordered all-page digest before interpreting timing. Inputs and ad hoc timing dumps remain outside source control; committed JSON is reviewed synthetic evidence. General real-input/platform and allocation-bound work remains open.
