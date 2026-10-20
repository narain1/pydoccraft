"""Measure one Office render mode per process using an installed pydoccraft wheel.

Run cached and one-shot modes in separate fresh processes on the same input.
Outputs timings/native RSS only; never writes or displays document content.
"""
from __future__ import annotations

import argparse
import gc
import hashlib
import json
import math
from pathlib import Path
import platform
import resource
import statistics
import sys
import time

import doc_ex


def memory():
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if sys.platform != "darwin":
        peak *= 1024  # Linux reports KiB; macOS reports bytes.
    resident = None
    if sys.platform.startswith("linux"):
        for line in Path("/proc/self/status").read_text().splitlines():
            if line.startswith("VmRSS:"):
                resident = int(line.split()[1]) * 1024
                break
    return {"rss_bytes": resident, "peak_rss_bytes": peak}


def timed(call):
    started = time.perf_counter()
    value = call()
    return value, (time.perf_counter() - started) * 1000


def run(source: Path, mode: str, dpi: float, rounds: int):
    before = memory()
    document, opened_ms = timed(lambda: doc_ex.open(source))
    try:
        if document.metadata["format"] not in ("docx", "pptx"):
            raise ValueError("this benchmark requires DOCX or PPTX")
        if mode == "one-shot":
            # Match the earlier facade: retain extracted Python values/source, but
            # discard native parsing/layout state before every render.
            document._render_session.close()
            document._render_session = None
        after_open = memory()
        first, cold_ms = timed(lambda: document.render_page(0, dpi))
        digest = hashlib.sha256(first).hexdigest()
        del first
        warm = []
        for _ in range(rounds):
            output, elapsed = timed(lambda: document.render_page(0, dpi))
            if hashlib.sha256(output).hexdigest() != digest:
                raise RuntimeError("same-page raster changed between identical requests")
            warm.append(elapsed)
            del output
        mixed = []
        # Include every page without retaining all PNG outputs in memory.
        for page in range(document.page_count):
            output, elapsed = timed(lambda: document.render_page(page, dpi))
            mixed.append(elapsed)
            del output
        before_close = memory()
        stats = document._render_session._stats() if document._render_session is not None else None
        result = {
            "mode": mode, "format": document.metadata["format"], "pages": document.page_count,
            "dpi": dpi, "rounds": rounds, "open_ms": opened_ms, "cold_render_ms": cold_ms,
            "warm_median_ms": statistics.median(warm), "all_pages_total_ms": sum(mixed),
            "all_pages_median_ms": statistics.median(mixed), "first_png_sha256": digest,
            "session_stats_parse_layout_attempts": stats, "before_open": before,
            "after_open": after_open, "before_close": before_close,
        }
    finally:
        document.close()
    gc.collect()
    result["after_close"] = memory()
    result["platform"] = platform.platform()
    result["python"] = platform.python_version()
    result["package_version"] = doc_ex.__version__
    result["limitations"] = "Synthetic or caller-selected input only. RSS includes native/global caches and allocator retention; peak RSS cannot decrease or prove close reclamation. Calls hold the Python GIL. No cross-format fidelity or throughput guarantee."
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--mode", choices=("cached", "one-shot"), required=True)
    parser.add_argument("--dpi", type=float, default=144)
    parser.add_argument("--rounds", type=int, default=5)
    args = parser.parse_args()
    if not math.isfinite(args.dpi) or not 1 <= args.dpi <= 1200:
        parser.error("dpi must be finite and between 1 and 1200")
    if not 1 <= args.rounds <= 100:
        parser.error("rounds must be between 1 and 100")
    try:
        result = run(args.source, args.mode, args.dpi, args.rounds)
    except Exception as error:
        # Upstream diagnostics may still appear on stderr; suppress CLI exception details.
        print(f"benchmark failed ({type(error).__name__})", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
