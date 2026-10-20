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

These small synthetic samples do not close P1 or establish general performance/fidelity guarantees. Representative large documents, image-heavy inputs, repeated process-level samples and other release platforms remain unmeasured. Calls retain the Python GIL; this change does not promise parallel rasterization. Post-layout page/item bounds limit retained state but cannot cap earlier native layout allocation.

Reproduce with a clean environment containing the built wheel (no additional libraries):

```console
python benchmarks/generate_office_session_inputs.py /tmp/new-office-session-inputs
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/report.docx --mode one-shot
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/report.docx --mode cached
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/slides.pptx --mode cached
python benchmarks/benchmark_office_sessions.py /tmp/new-office-session-inputs/slides.pptx --mode one-shot
```

The generator requires a new output directory. The measurement script emits JSON to standard output and uses Unix `resource` counters; current RSS is available on Linux. Keep generated inputs, outputs and timing dumps outside source control. CLI exception details are suppressed, but native backend diagnostics may appear on stderr.
