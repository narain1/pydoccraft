#!/usr/bin/env python3
"""Reproducible document parsing, rendering, writing, and conversion benchmarks."""

from __future__ import annotations

import argparse
import gc
import hashlib
import importlib.metadata
import io
import json
import platform
import statistics
import time
import zipfile
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable

import doc_ex
import openpyxl
import pymupdf
from docx import Document as DocxDocument
from docx.enum.text import WD_BREAK
from pptx import Presentation
from pptx.util import Inches, Pt


@dataclass(frozen=True)
class Corpus:
    pdf: bytes
    docx: bytes
    pptx: bytes
    xlsx: bytes


@dataclass(frozen=True)
class Scenario:
    category: str
    operation: str
    library: str
    function: Callable[[], object]
    comparable_group: str | None = None
    note: str = ""


@dataclass(frozen=True)
class Result:
    category: str
    operation: str
    library: str
    median_ms: float
    p25_ms: float
    p75_ms: float
    minimum_ms: float
    maximum_ms: float
    iterations: int
    comparable_group: str | None
    note: str


def normalize_ooxml(data: bytes) -> bytes:
    """Remove variable ZIP timestamps so generated OOXML bytes are reproducible."""
    source = zipfile.ZipFile(io.BytesIO(data))
    output = io.BytesIO()
    with source, zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for name in sorted(source.namelist()):
            info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = source.getinfo(name).external_attr
            archive.writestr(info, source.read(name))
    return output.getvalue()


def generate_pdf(pages: int, lines: int) -> bytes:
    document = pymupdf.open()
    try:
        for page_number in range(pages):
            page = document.new_page(width=612, height=792)
            for line_number in range(lines):
                text = (
                    f"Page {page_number + 1:03d} line {line_number + 1:03d}: "
                    "synthetic benchmark text with numbers 0123456789."
                )
                page.insert_text((54, 54 + line_number * 14), text, fontsize=10)
            page.insert_link(
                {
                    "kind": pymupdf.LINK_URI,
                    "from": pymupdf.Rect(54, 730, 180, 748),
                    "uri": f"https://example.com/page/{page_number + 1}",
                }
            )
        return document.tobytes(garbage=3, deflate=True)
    finally:
        document.close()


def generate_docx(pages: int, paragraphs: int) -> bytes:
    document = DocxDocument()
    timestamp = datetime(2020, 1, 1, tzinfo=timezone.utc)
    document.core_properties.created = timestamp
    document.core_properties.modified = timestamp
    for page_number in range(pages):
        document.add_heading(f"Benchmark page {page_number + 1}", level=1)
        for paragraph_number in range(paragraphs):
            document.add_paragraph(
                f"Paragraph {paragraph_number + 1}: synthetic DOCX benchmark text "
                "with deterministic content and Unicode café naïve."
            )
        table = document.add_table(rows=5, cols=4)
        for row_number, row in enumerate(table.rows):
            for column_number, cell in enumerate(row.cells):
                cell.text = f"R{row_number + 1}C{column_number + 1} P{page_number + 1}"
        if page_number + 1 < pages:
            document.add_paragraph().add_run().add_break(WD_BREAK.PAGE)
    output = io.BytesIO()
    document.save(output)
    return normalize_ooxml(output.getvalue())


def generate_pptx(slides: int) -> bytes:
    presentation = Presentation()
    timestamp = datetime(2020, 1, 1, tzinfo=timezone.utc)
    presentation.core_properties.created = timestamp
    presentation.core_properties.modified = timestamp
    presentation.slide_width = Inches(10)
    presentation.slide_height = Inches(7.5)
    blank = presentation.slide_layouts[6]
    for slide_number in range(slides):
        slide = presentation.slides.add_slide(blank)
        textbox = slide.shapes.add_textbox(Inches(0.6), Inches(0.5), Inches(8.8), Inches(2.0))
        frame = textbox.text_frame
        frame.text = f"Benchmark slide {slide_number + 1}"
        for paragraph_number in range(8):
            paragraph = frame.add_paragraph()
            paragraph.text = (
                f"Paragraph {paragraph_number + 1}: synthetic presentation text 0123456789"
            )
            paragraph.font.size = Pt(16)
        table = slide.shapes.add_table(5, 4, Inches(0.8), Inches(3.0), Inches(8.4), Inches(3.2)).table
        for row_number in range(5):
            for column_number in range(4):
                table.cell(row_number, column_number).text = (
                    f"R{row_number + 1}C{column_number + 1} S{slide_number + 1}"
                )
    # python-pptx starts with no slides for its default template.
    output = io.BytesIO()
    presentation.save(output)
    return normalize_ooxml(output.getvalue())


def generate_xlsx(rows: int, columns: int, *, normalize: bool = True) -> bytes:
    workbook = openpyxl.Workbook(write_only=True)
    timestamp = datetime(2020, 1, 1, tzinfo=timezone.utc)
    workbook.properties.created = timestamp
    workbook.properties.modified = timestamp
    worksheet = workbook.create_sheet("Benchmark")
    for row_number in range(rows):
        worksheet.append(
            [f"R{row_number + 1}C{column_number + 1}" for column_number in range(columns)]
        )
    output = io.BytesIO()
    workbook.save(output)
    data = output.getvalue()
    return normalize_ooxml(data) if normalize else data


def make_corpus(pages: int, lines: int, spreadsheet_rows: int, spreadsheet_columns: int) -> Corpus:
    return Corpus(
        pdf=generate_pdf(pages, lines),
        docx=generate_docx(pages, max(8, lines // 3)),
        pptx=generate_pptx(pages),
        xlsx=generate_xlsx(spreadsheet_rows, spreadsheet_columns),
    )


def pydoccraft_extract(data: bytes, filetype: str) -> tuple[int, int]:
    with doc_ex.open(data, filetype=filetype) as document:
        text = "\n".join(page.get_text("text") for page in document)
        return document.page_count, len(text)


def pymupdf_extract(data: bytes) -> tuple[int, int]:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        text = "\n".join(page.get_text("text") for page in document)
        return document.page_count, len(text)


def pydoccraft_render(data: bytes, dpi: float) -> tuple[int, bytes]:
    with doc_ex.open(data, filetype="pdf") as document:
        output = document.render_page(0, dpi=dpi)
        return document.page_count, output


def pymupdf_render(data: bytes, dpi: float) -> tuple[int, bytes]:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        # PyMuPDF 2.0 requires integer DPI; the CLI accepts only integral values for this path.
        integral_dpi = round(dpi)
        if integral_dpi != dpi:
            raise ValueError("PyMuPDF comparison requires an integral DPI")
        output = document[0].get_pixmap(dpi=integral_dpi, alpha=True).tobytes("png")
        return document.page_count, output


def pymupdf_rewrite(data: bytes) -> bytes:
    with pymupdf.open(stream=data, filetype="pdf") as document:
        return document.tobytes(garbage=3, deflate=True)


def openpyxl_parse(data: bytes) -> tuple[int, int, int]:
    workbook = openpyxl.load_workbook(io.BytesIO(data), read_only=True, data_only=True)
    try:
        worksheet = workbook["Benchmark"]
        cells = 0
        characters = 0
        for row in worksheet.iter_rows(values_only=True):
            cells += len(row)
            characters += sum(len(str(value)) for value in row if value is not None)
        return worksheet.max_row, cells, characters
    finally:
        workbook.close()


def openpyxl_write(rows: int, columns: int) -> int:
    # Serialization is intentionally performed even though ZIP metadata may vary in size.
    generate_xlsx(rows, columns, normalize=False)
    return rows * columns


def converted(data: bytes, source: str, target: str) -> bytes:
    return doc_ex.convert(data, target, filetype=source)


def guard(value: object) -> tuple:
    # ZIP/PDF metadata can make byte lengths vary without changing benchmark semantics.
    if isinstance(value, bytes):
        return (value[:8],)
    if isinstance(value, tuple):
        normalized = []
        for item in value:
            normalized.append(item[:8] if isinstance(item, bytes) else item)
        return tuple(normalized)
    return (value,)


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    weight = position - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def measure(scenario: Scenario, warmups: int, repeats: int) -> Result:
    expected = None
    for _ in range(warmups):
        expected = guard(scenario.function())
    samples = []
    for _ in range(repeats):
        gc.collect()
        started = time.perf_counter_ns()
        value = scenario.function()
        elapsed = time.perf_counter_ns() - started
        observed = guard(value)
        if expected is not None and observed != expected:
            raise RuntimeError(
                f"unstable result for {scenario.library} {scenario.operation}: "
                f"{observed!r} != {expected!r}"
            )
        expected = observed
        samples.append(elapsed / 1_000_000)
    return Result(
        category=scenario.category,
        operation=scenario.operation,
        library=scenario.library,
        median_ms=statistics.median(samples),
        p25_ms=percentile(samples, 0.25),
        p75_ms=percentile(samples, 0.75),
        minimum_ms=min(samples),
        maximum_ms=max(samples),
        iterations=repeats,
        comparable_group=scenario.comparable_group,
        note=scenario.note,
    )


def verify_corpus(corpus: Corpus, pages: int, xlsx_cells: int, dpi: float) -> dict[str, object]:
    pydoc_pdf = pydoccraft_extract(corpus.pdf, "pdf")
    mupdf_pdf = pymupdf_extract(corpus.pdf)
    if pydoc_pdf[0] != pages or mupdf_pdf[0] != pages:
        raise RuntimeError(f"PDF page count mismatch: {pydoc_pdf}, {mupdf_pdf}")
    if pydoc_pdf[1] == 0 or mupdf_pdf[1] == 0:
        raise RuntimeError("PDF extraction returned no text")

    pydoc_png = pydoccraft_render(corpus.pdf, dpi)[1]
    mupdf_png = pymupdf_render(corpus.pdf, dpi)[1]
    if not pydoc_png.startswith(b"\x89PNG\r\n\x1a\n") or not mupdf_png.startswith(b"\x89PNG\r\n\x1a\n"):
        raise RuntimeError("renderer did not return PNG")

    docx_pdf = converted(corpus.docx, "docx", "pdf")
    pptx_pdf = converted(corpus.pptx, "pptx", "pdf")
    pdf_docx = converted(corpus.pdf, "pdf", "docx")
    for data, filetype in ((docx_pdf, "pdf"), (pptx_pdf, "pdf"), (pdf_docx, "docx")):
        with doc_ex.open(data, filetype=filetype) as document:
            if document.page_count < 1:
                raise RuntimeError(f"converted {filetype} has no pages")
            if not any(page.get_text("text") for page in document):
                raise RuntimeError(f"converted {filetype} has no text")

    xlsx = openpyxl_parse(corpus.xlsx)
    if xlsx[1] != xlsx_cells:
        raise RuntimeError(f"XLSX cell mismatch: {xlsx[1]} != {xlsx_cells}")

    return {
        "pdf_text_characters": {"pydoccraft": pydoc_pdf[1], "pymupdf": mupdf_pdf[1]},
        "png_bytes": {"pydoccraft": len(pydoc_png), "pymupdf": len(mupdf_png)},
        "conversion_bytes": {
            "pdf_to_docx": len(pdf_docx),
            "docx_to_pdf": len(docx_pdf),
            "pptx_to_pdf": len(pptx_pdf),
        },
        "xlsx_cells": xlsx[1],
    }


def environment(corpus: Corpus, repository_revision: str) -> dict[str, object]:
    return {
        "timestamp_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "processor": platform.processor(),
        "python": platform.python_version(),
        "repository_revision": repository_revision,
        "packages": {
            name: importlib.metadata.version(name)
            for name in ("pydoccraft", "PyMuPDF", "openpyxl", "python-docx", "python-pptx")
        },
        "pydoccraft_import": str(Path(doc_ex.__file__).resolve()),
        "inputs": {
            name: {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
            for name, data in asdict(corpus).items()
        },
    }


def markdown_report(payload: dict[str, object]) -> str:
    config = payload["configuration"]
    env = payload["environment"]
    lines = [
        "# Generated document benchmark results",
        "",
        f"Run: `{env['timestamp_utc']}` on `{env['platform']}` / `{env['machine']}` with Python `{env['python']}`.",
        "",
        "| Category | Operation | Library | Median (ms) | IQR (ms) | Min–max (ms) |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for result in payload["results"]:
        lines.append(
            f"| {result['category']} | {result['operation']} | {result['library']} | "
            f"{result['median_ms']:.2f} | {result['p25_ms']:.2f}–{result['p75_ms']:.2f} | "
            f"{result['minimum_ms']:.2f}–{result['maximum_ms']:.2f} |"
        )
    lines.extend(["", "## Comparable PDF ratios", ""])
    for comparison in payload["comparisons"]:
        lines.append(
            f"- **{comparison['group']}**: {comparison['faster']} was "
            f"**{comparison['ratio']:.2f}× faster** by median elapsed time."
        )
    lines.extend(
        [
            "",
            "## Configuration",
            "",
            f"- Pages/slides: {config['pages']}; PDF lines/page: {config['lines_per_page']}.",
            f"- XLSX: {config['spreadsheet_rows']} rows × {config['spreadsheet_columns']} columns.",
            f"- Warmups: {config['warmups']}; measured iterations: {config['repeats']}; render DPI: {config['dpi']}.",
            "- Input generation, validation, and checksums were outside timed regions.",
            "- Conversions and XLSX operations are capability baselines, not cross-library comparisons.",
            "",
        ]
    )
    return "\n".join(lines)


def parse_arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pages", type=int, default=25)
    parser.add_argument("--lines", type=int, default=40)
    parser.add_argument("--spreadsheet-rows", type=int, default=1000)
    parser.add_argument("--spreadsheet-columns", type=int, default=20)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--dpi", type=float, default=144)
    parser.add_argument("--repository-revision", default="unknown")
    parser.add_argument("--output", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_arguments()
    for name in ("pages", "lines", "spreadsheet_rows", "spreadsheet_columns", "repeats"):
        if getattr(args, name) < 1:
            raise SystemExit(f"--{name.replace('_', '-')} must be positive")
    if args.warmups < 0:
        raise SystemExit("--warmups must not be negative")

    corpus = make_corpus(args.pages, args.lines, args.spreadsheet_rows, args.spreadsheet_columns)
    checks = verify_corpus(
        corpus,
        args.pages,
        args.spreadsheet_rows * args.spreadsheet_columns,
        args.dpi,
    )
    scenarios = [
        Scenario("PDF", "open + extract all plain text", "pydoccraft", lambda: pydoccraft_extract(corpus.pdf, "pdf"), "pdf-text"),
        Scenario("PDF", "open + extract all plain text", "PyMuPDF", lambda: pymupdf_extract(corpus.pdf), "pdf-text"),
        Scenario("PDF", f"open + render first page to PNG at {args.dpi:g} DPI", "pydoccraft", lambda: pydoccraft_render(corpus.pdf, args.dpi), "pdf-render"),
        Scenario("PDF", f"open + render first page to PNG at {args.dpi:g} DPI", "PyMuPDF", lambda: pymupdf_render(corpus.pdf, args.dpi), "pdf-render"),
        Scenario("PDF", "open + rewrite PDF bytes", "PyMuPDF", lambda: pymupdf_rewrite(corpus.pdf), note="No equivalent pydoccraft identity-save operation."),
        Scenario("DOCX", "open + extract all plain text", "pydoccraft", lambda: pydoccraft_extract(corpus.docx, "docx")),
        Scenario("PPTX", "open + extract all plain text", "pydoccraft", lambda: pydoccraft_extract(corpus.pptx, "pptx")),
        Scenario("Conversion", "PDF to DOCX", "pydoccraft", lambda: converted(corpus.pdf, "pdf", "docx")),
        Scenario("Conversion", "DOCX to PDF", "pydoccraft", lambda: converted(corpus.docx, "docx", "pdf")),
        Scenario("Conversion", "PPTX to PDF", "pydoccraft", lambda: converted(corpus.pptx, "pptx", "pdf")),
        Scenario("XLSX", "open + iterate all cell values", "openpyxl", lambda: openpyxl_parse(corpus.xlsx), note="Separate XLSX baseline; pydoccraft does not support XLSX."),
        Scenario("XLSX", "create workbook + write XLSX bytes", "openpyxl", lambda: openpyxl_write(args.spreadsheet_rows, args.spreadsheet_columns), note="Separate XLSX baseline; not a document conversion comparison."),
    ]
    results = [measure(scenario, args.warmups, args.repeats) for scenario in scenarios]

    comparisons = []
    groups = sorted({result.comparable_group for result in results if result.comparable_group})
    for group in groups:
        members = [result for result in results if result.comparable_group == group]
        fastest = min(members, key=lambda result: result.median_ms)
        slowest = max(members, key=lambda result: result.median_ms)
        comparisons.append(
            {
                "group": group,
                "faster": fastest.library,
                "slower": slowest.library,
                "ratio": slowest.median_ms / fastest.median_ms,
            }
        )

    payload = {
        "environment": environment(corpus, args.repository_revision),
        "configuration": {
            "pages": args.pages,
            "lines_per_page": args.lines,
            "spreadsheet_rows": args.spreadsheet_rows,
            "spreadsheet_columns": args.spreadsheet_columns,
            "warmups": args.warmups,
            "repeats": args.repeats,
            "dpi": args.dpi,
        },
        "checks": checks,
        "results": [asdict(result) for result in results],
        "comparisons": comparisons,
    }
    rendered = markdown_report(payload)
    print(rendered)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(payload, indent=2) + "\n")
        args.output.with_suffix(".md").write_text(rendered + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
