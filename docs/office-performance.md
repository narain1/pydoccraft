# Office render-session measurements

An open DOCX/PPTX now retains its parsed native model; DOCX also retains completed pagination. Repeated `render_page`/`get_pixmap` calls reuse this state. Public methods and PNG return values are unchanged. Close releases owned state and source/password references, while extracted values remain usable.

Original synthetic fixtures contain 20 pages/slides each, with text, semantic tables and an image per page. Measured on Linux x86_64, CPython 3.11.2, Rust 1.95 release wheel on 2026-10-20, at 144 DPI. Each mode runs in a fresh process; five warm same-page repetitions follow the first render, then every page is rendered once. Results compare the same build with retained state enabled versus explicitly closed native state followed by the existing one-shot render path. They do not compare a historical wheel.

| Format/mode | Open ms | First render ms | Warm median ms | All 20 pages ms | Peak RSS MiB |
|---|---:|---:|---:|---:|---:|
| DOCX one-shot |17.20|24.06|23.88|478.02|61.66|
| DOCX retained |16.93|13.14|11.85|235.37|61.49|
| PPTX one-shot |6.27|10.19|7.79|158.41|54.86|
| PPTX retained |6.87|6.54|4.42|88.86|52.29|

The first-page PNG hash matches between modes for each format. Private counters show one DOCX parse/layout and one PPTX import across 26 render requests. Tests separately verify mixed-page/region byte parity at 72/144/300 DPI.

RSS before/after close was unchanged in these runs: DOCX retained 51.19 MiB, one-shot 50.58 MiB; PPTX retained 41.50 MiB, one-shot 43.12 MiB. Retaining a model/layout can increase steady-state memory. Native allocator retention, fonts and DeckCraft's private thread-local renderer/cache affect RSS; its public API has no per-document cache-clear hook. Native weak-reference tests prove session-owned models are released. Peak RSS is a process high-water mark and cannot demonstrate reclamation. Current `/proc` RSS and `getrusage` peak counters have different sampling/accounting behavior.

These original small synthetic samples do not close P1 or establish general performance/fidelity guarantees. The later section records larger Office workloads and repeated processes; independent PDF memory evidence, broader real-world inputs and other release platforms remain separate work. Calls retain the Python GIL; this change does not promise parallel rasterization. Post-layout page/item bounds limit retained state but cannot cap earlier native layout allocation.

Reproduce with a clean environment containing the built wheel (no additional libraries):

```console
python benchmarks/generate_office_session_inputs.py /tmp/new-office-session-inputs
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/report.docx --mode one-shot
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/report.docx --mode cached
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/slides.pptx --mode cached
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/slides.pptx --mode one-shot
```

The generator requires a new output directory. The measurement script emits JSON to standard output and uses Unix `resource` counters; current RSS is available on Linux. Keep generated inputs, outputs and ad hoc timing dumps outside source control; the reviewed result set linked below is an explicit evidence artifact. CLI exception details are suppressed, but native backend diagnostics may appear on stderr.


## Larger workloads and repeated processes
Measured on 2026-10-21 UTC using the exact paragraph-movement release artifact (`43fd15cc870f633947ab6ff420bb4591bc5dc9aa`), Linux x86_64, Python 3.11.2, Rust 1.95 and an AMD Ryzen 9 9955HX. Wheel SHA-256 is `ffb815e29b7429ba7e6d9b8b74e6dbc1be9b68888edcb59a24665c3a8b5fa1b9`. These are synthetic native-authored inputs. [The reviewed JSON evidence](office-performance-results.json) records all 36 process samples, source hashes/sizes, tool hashes, page counts, ordered all-page PNG digests, timings, session counters and memory observations.
Each input/mode ran in three fresh sequential processes, with five warm same-page repetitions followed by every page at 144 DPI. Mode order alternated between repetitions. Local Cargo builds and validation suites were paused for measurement. Both modes use the same build and eager extraction; the one-shot mode explicitly closes the retained native session before rendering. Input hashing precedes timing and warms the filesystem cache, so first-render figures are not disk-cold measurements. No historical-wheel or cross-library comparison is implied.
The original 20-page baseline uses one shared 64-pixel image. The larger report/deck has 200 pages/slides with text and tables. The image-heavy pair has 24 distinct 512×512 PNGs displayed at 144 points, totaling about 21.6 MB per source file. All six reopened page counts matched the requested counts. Generated inputs remain outside source control; the committed JSON is reviewed evidence, not a fixture payload.
| Workload | Format | Cached all pages ms | One-shot all pages ms | Ratio |
|---|---|---:|---:|---:|
| 20-page baseline | DOCX | 238.4 | 465.6 | 1.95× |
| 20-page baseline | PPTX | 85.8 | 156.2 | 1.82× |
| 200-page report/deck | DOCX | 2,152.7 | 24,807.3 | 11.52× |
| 200-page report/deck | PPTX | 696.0 | 6,475.9 | 9.30× |
| 24-page distinct images | DOCX | 374.6 | 875.7 | 2.34× |
| 24-page distinct images | PPTX | 162.7 | 405.4 | 2.49× |

Values above are medians of three full-document totals; individual samples are retained in JSON. Peak process RSS ranges below are the minimum–maximum of three samples, in MiB, measured before close.
| Workload | Format | Cached peak RSS MiB | One-shot peak RSS MiB |
|---|---|---:|---:|
| 20-page baseline | DOCX | 61.5–62.2 | 61.3–62.1 |
| 20-page baseline | PPTX | 52.4–52.9 | 53.7–54.5 |
| 200-page report/deck | DOCX | 140.9–141.1 | 143.1–145.6 |
| 200-page report/deck | PPTX | 93.3–94.4 | 113.1–113.8 |
| 24-page distinct images | DOCX | 270.1–271.2 | 269.9–271.2 |
| 24-page distinct images | PPTX | 159.4–159.5 | 205.1–206.6 |

Every first-page and ordered all-page PNG digest matched between modes and across repetitions. Cached DOCX reported one parse and one layout; cached PPTX reported one import and no DOCX layout, with exactly `pages + 6` render requests. This verifies reuse and within-format byte parity on these inputs; it does not establish source-versus-PDF visual fidelity or pixel-perfect Office compatibility.
Memory includes native/global caches, decoded media and allocator retention. Current RSS and high-water RSS use different OS counters; neither unchanged current RSS nor a nondecreasing high-water mark disproves the separately tested release of session-owned models. Close does not promise returning all memory to the OS. The image-heavy DOCX runs reach about 271 MiB in both modes. These measurements are observations, not allocation limits or universal speed/memory guarantees.
This supplies repeated Linux large-document and image-heavy Office evidence. P1 remains open for an independently reproducible PDF memory/full-document counterpart; broader real-world corpora and other supported platforms remain QA7 work. Post-layout limits still cannot bound earlier native allocations, and calls retain the Python GIL.
Reproduce the two larger inputs with a clean installed wheel, then run both existing benchmark modes three times each, alternating mode order:

```console
python benchmarks/generate_office_session_inputs.py /tmp/office-long --pages 200
python benchmarks/generate_office_session_inputs.py /tmp/office-images --pages 24 --image-pixels 512 --unique-images --image-points 144
python benchmarks/benchmark_office_sessions.py /tmp/office-long/report.docx --mode cached --dpi 144 --rounds 5
python benchmarks/benchmark_office_sessions.py /tmp/office-long/report.docx --mode one-shot --dpi 144 --rounds 5
```

Repeat for `slides.pptx` and the image-heavy directory. Compare source hash, format, DPI, actual page count and all-page digest before interpreting timing ratios. Generation rejects out-of-budget configurations before creating the output directory and never overwrites an existing directory.
