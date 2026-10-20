#!/usr/bin/env python3
"""Anonymous PDF-only pydoccraft versus PyMuPDF benchmark."""

from __future__ import annotations

import argparse
import gc
import json
import statistics
import struct
import time
from collections import Counter
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Callable

import doc_ex
import pymupdf


@dataclass(frozen=True)
class Result:
    identifier: str
    pages: int
    mib: float
    scenario: str
    library: str
    median_ms: float
    minimum_ms: float
    maximum_ms: float


def measure(function: Callable[[], object], validate: Callable[[object], tuple], warmups: int, repeats: int) -> tuple[float, float, float]:
    expected = None
    for _ in range(warmups):
        expected = validate(function())
    samples = []
    for _ in range(repeats):
        gc.collect()
        started = time.perf_counter_ns()
        value = function()
        samples.append((time.perf_counter_ns() - started) / 1_000_000)
        actual = validate(value)
        if expected is not None and actual != expected:
            raise RuntimeError("benchmark result changed between iterations")
        expected = actual
    return statistics.median(samples), min(samples), max(samples)


def text_summary(value: str, expected_pages: int) -> tuple[int, int]:
    pages = value.count("\f") + 1
    if pages != expected_pages:
        raise RuntimeError("text result does not match the source page count")
    return pages, len(value)


def png_summary(value: bytes, expected: tuple[int, int] | None = None) -> tuple[int, int]:
    if not value.startswith(b"\x89PNG\r\n\x1a\n") or len(value) < 24:
        raise RuntimeError("renderer did not return PNG")
    dimensions = struct.unpack(">II", value[16:24])
    if not all(dimensions) or (expected is not None and dimensions != expected):
        raise RuntimeError("render dimensions do not match the comparison")
    return dimensions


def pydoc_text(data: bytes) -> str:
    return doc_ex.extract_text(data, filetype="pdf")


def pymupdf_text(data: bytes) -> str:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        return "\f".join(page.get_text("text").rstrip() for page in document)


def pydoc_cold_render(data: bytes, dpi: int) -> bytes:
    with doc_ex.open(data, filetype="pdf") as document:
        return document.render_page(0, dpi=dpi)


def pymupdf_cold_render(data: bytes, dpi: int) -> bytes:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        return document[0].get_pixmap(dpi=dpi, alpha=True).tobytes("png")


def add(results: list[Result], identifier: str, pages: int, size: int, scenario: str, library: str, timing: tuple[float, float, float]) -> None:
    results.append(Result(identifier, pages, round(size / 1048576, 2), scenario, library, *timing))


def input_paths(directory: Path, output: Path | None = None) -> list[Path]:
    paths = sorted((path for path in directory.iterdir()
                    if path.is_file() and path.suffix.lower() == ".pdf"),
                   key=lambda path: path.stat().st_size)
    if output is not None:
        if output.suffix.lower() != ".json":
            raise ValueError("benchmark output must use .json")
        if any(output.resolve() == path.resolve()
               or (output.exists() and output.samefile(path)) for path in paths):
            raise ValueError("benchmark output must not replace an input")
    return paths


def run(directory: Path, warmups: int, repeats: int, dpi: int, output: Path | None = None) -> list[Result]:
    paths = input_paths(directory, output)
    if not paths:
        raise RuntimeError("no PDF inputs")
    results = []
    for number, path in enumerate(paths, 1):
        identifier = f"PDF-{number}"
        data = path.read_bytes()
        with pymupdf.open(stream=data, filetype="pdf") as probe:
            pages = probe.page_count
            if pages < 1:
                raise RuntimeError("PDF input contains no pages")
            # Preflight outside timing. Both libraries must render the same pixel size.
            dimensions = png_summary(probe[0].get_pixmap(dpi=dpi, alpha=True).tobytes("png"))
        validate_text = lambda value: text_summary(value, pages)
        validate_png = lambda value: png_summary(value, dimensions)

        ours_text = pydoc_text(data)
        theirs_text = pymupdf_text(data)
        ours_tokens = Counter(ours_text.split())
        theirs_tokens = Counter(theirs_text.split())
        overlap = sum((ours_tokens & theirs_tokens).values()) / max(
            sum(ours_tokens.values()), sum(theirs_tokens.values()), 1
        )
        if overlap < 0.95:
            raise RuntimeError("libraries returned less than 95% token overlap")

        add(results, identifier, pages, len(data), "all-page plain text", "pydoccraft", measure(lambda: pydoc_text(data), validate_text, warmups, repeats))
        add(results, identifier, pages, len(data), "all-page plain text", "PyMuPDF", measure(lambda: pymupdf_text(data), validate_text, warmups, repeats))
        add(results, identifier, pages, len(data), "cold open + page-1 RGBA PNG", "pydoccraft", measure(lambda: pydoc_cold_render(data, dpi), validate_png, warmups, repeats))
        add(results, identifier, pages, len(data), "cold open + page-1 RGBA PNG", "PyMuPDF", measure(lambda: pymupdf_cold_render(data, dpi), validate_png, warmups, repeats))

        with doc_ex.open(data, filetype="pdf") as ours, pymupdf.open(stream=data, filetype="pdf") as theirs:
            add(results, identifier, pages, len(data), "warm page-1 RGBA PNG", "pydoccraft", measure(lambda: ours.render_page(0, dpi=dpi), validate_png, warmups, repeats))
            add(results, identifier, pages, len(data), "warm page-1 RGBA PNG", "PyMuPDF", measure(lambda: theirs[0].get_pixmap(dpi=dpi, alpha=True).tobytes("png"), validate_png, warmups, repeats))
    return results


def render(results: list[Result], warmups: int, repeats: int, dpi: int) -> str:
    lines = [
        "# Anonymous pydoccraft versus PyMuPDF PDF benchmark",
        "",
        "No filenames, paths, hashes, or extracted text are emitted.",
        "",
        "| ID | Pages | MiB | Scenario | Library | Median ms | Min–max ms |",
        "|---|---:|---:|---|---|---:|---:|",
    ]
    for result in results:
        lines.append(f"| {result.identifier} | {result.pages} | {result.mib:.2f} | {result.scenario} | {result.library} | {result.median_ms:.2f} | {result.minimum_ms:.2f}–{result.maximum_ms:.2f} |")
    lines.extend(["", f"Configuration: {warmups} warmup(s), {repeats} measured iterations, {dpi} DPI.", ""])
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--warmups", type=int, default=1)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--dpi", type=int, default=144)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.warmups < 0 or args.repeats < 1 or args.dpi < 1:
        parser.error("invalid benchmark counts or DPI")
    try:
        results = run(args.directory, args.warmups, args.repeats, args.dpi, args.output)
        report = render(results, args.warmups, args.repeats, args.dpi)
        print(report)
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps({"results": [asdict(result) for result in results]}, indent=2) + "\n")
        return 0
    except Exception as error:
        raise SystemExit(f"PDF benchmark failed ({type(error).__name__}); document details omitted") from None


if __name__ == "__main__":
    raise SystemExit(main())
