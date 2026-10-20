#!/usr/bin/env python3
"""Benchmark local documents while emitting no filenames, paths, hashes, or text."""

from __future__ import annotations

import argparse
import gc
import io
import json
import statistics
import struct
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Callable

import doc_ex
import openpyxl
import pymupdf


@dataclass(frozen=True)
class Item:
    identifier: str
    filetype: str
    data: bytes
    size_bytes: int
    units: int


@dataclass(frozen=True)
class Result:
    identifier: str
    filetype: str
    size_bytes: int
    units: int
    operation: str
    library: str
    median_ms: float
    p25_ms: float
    p75_ms: float
    minimum_ms: float
    maximum_ms: float
    repeats: int
    comparable_group: str | None = None


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    weight = position - lower
    return ordered[lower] * (1 - weight) + ordered[upper] * weight


def summarize_png(value: tuple[int, bytes]) -> tuple[int, int, int]:
    pages, data = value
    if not data.startswith(b"\x89PNG\r\n\x1a\n") or len(data) < 24:
        raise RuntimeError("render did not return a complete PNG header")
    width, height = struct.unpack(">II", data[16:24])
    if not width or not height:
        raise RuntimeError("render returned empty dimensions")
    return pages, width, height


def summarize_document(value: tuple[int, int]) -> tuple[int, int]:
    pages, characters = value
    if pages < 1 or characters < 1:
        raise RuntimeError("document extraction returned no pages or text")
    return value


def summarize_conversion(value: tuple[bytes, str]) -> tuple[int, int]:
    data, filetype = value
    with doc_ex.open(data, filetype=filetype) as document:
        characters = sum(len(page.get_text("text")) for page in document)
        if document.page_count < 1 or characters < 1:
            raise RuntimeError("converted output did not reopen with text")
        return document.page_count, characters


def summarize_xlsx(value: tuple[int, int]) -> tuple[int, int]:
    rows, cells = value
    if rows < 1 or cells < 1:
        raise RuntimeError("XLSX operation returned no cells")
    return value


def pydoc_extract(data: bytes, filetype: str) -> tuple[int, int]:
    with doc_ex.open(data, filetype=filetype) as document:
        return document.page_count, sum(len(page.get_text("text")) for page in document)


def pymupdf_extract(data: bytes) -> tuple[int, int]:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        return document.page_count, sum(len(page.get_text("text")) for page in document)


def pydoc_render(data: bytes, filetype: str, dpi: int) -> tuple[int, bytes]:
    with doc_ex.open(data, filetype=filetype) as document:
        return document.page_count, document.render_page(0, dpi=dpi)


def pymupdf_render(data: bytes, dpi: int) -> tuple[int, bytes]:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        return document.page_count, document[0].get_pixmap(dpi=dpi, alpha=True).tobytes("png")


def convert(data: bytes, source: str) -> tuple[bytes, str]:
    target = "docx" if source == "pdf" else "pdf"
    return doc_ex.convert(data, target, filetype=source), target


def rewrite_pdf(data: bytes) -> tuple[bytes, str]:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        return document.tobytes(garbage=3, deflate=True), "pdf"


def openpyxl_extract(data: bytes) -> tuple[int, int]:
    workbook = openpyxl.load_workbook(io.BytesIO(data), read_only=True, data_only=True)
    try:
        rows = cells = 0
        for worksheet in workbook.worksheets:
            for row in worksheet.iter_rows(values_only=True):
                rows += 1
                cells += sum(value is not None for value in row)
        return rows, cells
    finally:
        workbook.close()


def measure(
    item: Item,
    operation: str,
    library: str,
    function: Callable[[], object],
    summarize: Callable[[object], tuple],
    warmups: int,
    repeats: int,
    comparable_group: str | None = None,
) -> Result:
    expected = None
    for _ in range(warmups):
        expected = summarize(function())
    samples = []
    for _ in range(repeats):
        gc.collect()
        started = time.perf_counter_ns()
        value = function()
        elapsed = (time.perf_counter_ns() - started) / 1_000_000
        observed = summarize(value)
        if expected is not None and observed != expected:
            raise RuntimeError(f"unstable semantic result for {item.identifier} {operation}")
        expected = observed
        samples.append(elapsed)
    return Result(
        identifier=item.identifier,
        filetype=item.filetype,
        size_bytes=item.size_bytes,
        units=item.units,
        operation=operation,
        library=library,
        median_ms=statistics.median(samples),
        p25_ms=percentile(samples, 0.25),
        p75_ms=percentile(samples, 0.75),
        minimum_ms=min(samples),
        maximum_ms=max(samples),
        repeats=repeats,
        comparable_group=comparable_group,
    )


def load_items(directory: Path) -> list[Item]:
    supported = {".pdf": "pdf", ".docx": "docx", ".pptx": "pptx", ".xlsx": "xlsx"}
    files = [path for path in directory.iterdir() if path.is_file() and path.suffix.lower() in supported]
    files.sort(key=lambda path: (supported[path.suffix.lower()], path.stat().st_size))
    counters: dict[str, int] = {}
    items = []
    for path in files:
        filetype = supported[path.suffix.lower()]
        counters[filetype] = counters.get(filetype, 0) + 1
        identifier = f"{filetype.upper()}-{counters[filetype]}"
        data = path.read_bytes()
        if filetype == "xlsx":
            units = openpyxl_extract(data)[0]
        else:
            units = pydoc_extract(data, filetype)[0]
        items.append(Item(identifier, filetype, data, len(data), units))
    if not items:
        raise RuntimeError("no supported local benchmark documents")
    return items


def report(results: list[Result], dpi: int, warmups: int, repeats: int) -> str:
    lines = [
        "# Anonymized local-corpus benchmark",
        "",
        "No filenames, paths, hashes, or extracted text are included.",
        "",
        "| ID | Type | MiB | Pages/slides/rows | Operation | Library | Median ms | IQR ms |",
        "|---|---|---:|---:|---|---|---:|---:|",
    ]
    for result in results:
        lines.append(
            f"| {result.identifier} | {result.filetype.upper()} | {result.size_bytes / 1048576:.2f} | "
            f"{result.units} | {result.operation} | {result.library} | {result.median_ms:.2f} | "
            f"{result.p25_ms:.2f}–{result.p75_ms:.2f} |"
        )
    lines.extend(["", "## Matched PDF ratios", ""])
    groups = sorted({result.comparable_group for result in results if result.comparable_group})
    for group in groups:
        members = [result for result in results if result.comparable_group == group]
        fast = min(members, key=lambda result: result.median_ms)
        slow = max(members, key=lambda result: result.median_ms)
        lines.append(f"- **{group}**: {fast.library} was {slow.median_ms / fast.median_ms:.2f}× faster.")
    lines.extend(
        [
            "",
            f"Configuration: {warmups} warmup(s), {repeats} measured iterations, first-page render at {dpi} DPI.",
            "Validation occurs after the timer and generated outputs are not retained.",
            "",
        ]
    )
    return "\n".join(lines)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--dpi", type=int, default=144)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.warmups < 0 or args.repeats < 1 or args.dpi < 1:
        parser.error("warmups must be nonnegative; repeats and DPI must be positive")
    if args.output is not None and args.output.suffix.lower() == ".md":
        parser.error("--output must not use .md: JSON and Markdown require distinct paths")
    return args


def run(args: argparse.Namespace) -> int:
    items = load_items(args.directory)
    results = []
    for item in items:
        if item.filetype == "xlsx":
            results.append(measure(item, "parse values", "openpyxl", lambda item=item: openpyxl_extract(item.data), summarize_xlsx, args.warmups, args.repeats))
            continue
        results.append(measure(item, "extract text", "pydoccraft", lambda item=item: pydoc_extract(item.data, item.filetype), summarize_document, args.warmups, args.repeats, f"{item.identifier}-extract" if item.filetype == "pdf" else None))
        if item.filetype == "pdf":
            results.append(measure(item, "extract text", "PyMuPDF", lambda item=item: pymupdf_extract(item.data), summarize_document, args.warmups, args.repeats, f"{item.identifier}-extract"))
        results.append(measure(item, f"render page 1 RGBA PNG ({args.dpi} DPI)", "pydoccraft", lambda item=item: pydoc_render(item.data, item.filetype, args.dpi), summarize_png, args.warmups, args.repeats, f"{item.identifier}-render" if item.filetype == "pdf" else None))
        if item.filetype == "pdf":
            results.append(measure(item, f"render page 1 RGBA PNG ({args.dpi} DPI)", "PyMuPDF", lambda item=item: pymupdf_render(item.data, args.dpi), summarize_png, args.warmups, args.repeats, f"{item.identifier}-render"))
            results.append(measure(item, "rewrite PDF", "PyMuPDF", lambda item=item: rewrite_pdf(item.data), summarize_conversion, args.warmups, args.repeats))
        results.append(measure(item, "convert to DOCX" if item.filetype == "pdf" else "convert to PDF", "pydoccraft", lambda item=item: convert(item.data, item.filetype), summarize_conversion, args.warmups, args.repeats))
    rendered = report(results, args.dpi, args.warmups, args.repeats)
    print(rendered)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps({"results": [asdict(result) for result in results]}, indent=2) + "\n")
        args.output.with_suffix(".md").write_text(rendered + "\n")
    return 0


def main() -> int:
    args = parse_arguments()
    try:
        return run(args)
    except Exception as error:
        # Exceptions from readers and filesystem operations can contain local paths or text.
        # Keep the public CLI failure as anonymous as its successful report.
        raise SystemExit(
            f"local benchmark failed ({type(error).__name__}); document details omitted"
        ) from None


if __name__ == "__main__":
    raise SystemExit(main())
