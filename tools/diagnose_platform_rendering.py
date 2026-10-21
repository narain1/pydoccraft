"""Temporary platform investigation; recorded results are not acceptance gates."""
from __future__ import annotations

import hashlib
import io
import json
from pathlib import Path
import platform
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tests"))

import doc_ex
from doc_ex import _core
from test_metadata_fields import values
from test_picture_media import count_color
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

OUTPUT = ROOT / "dist/platform-diagnostics"
OUTPUT.mkdir(parents=True, exist_ok=True)
ORIGINAL_NEW = doc_ex.new


def factory(font):
    def create(*args, **kwargs):
        document = ORIGINAL_NEW(*args, **kwargs)
        if document.filetype == "docx" and font is not None:
            document.styles["Normal"].format(font=font)
        return document
    return create


def capture(document, name):
    directory = OUTPUT / name
    directory.mkdir()
    raw = document.to_bytes()
    converted = document.convert("pdf")
    (directory / "source.docx").write_bytes(raw)
    (directory / "converted.pdf").write_bytes(converted)
    records = []
    with doc_ex.open(converted) as pdf, doc_ex.open(raw) as source:
        (directory / "source-text.json").write_text(source[0].get_text("json"), encoding="utf-8")
        (directory / "pdf-text.json").write_text(pdf[0].get_text("json"), encoding="utf-8")
        for dpi in (72, 144):
            native = document.render_page(0, dpi=dpi)
            exported = pdf.render_page(0, dpi=dpi)
            (directory / f"native-{dpi}.png").write_bytes(native)
            (directory / f"pdf-{dpi}.png").write_bytes(exported)
            width, a = rgba_pixels(png_chunks(native))
            pdf_width, b = rgba_pixels(png_chunks(exported))
            records.append({
                "dpi": dpi, "native_width": width, "pdf_width": pdf_width,
                "native_bytes": len(a), "pdf_bytes": len(b),
                "comparable_raster": width == pdf_width and len(a) == len(b),
                "max_channel_delta": (max(abs(x-y) for x, y in zip(a, b))
                                      if width == pdf_width and len(a) == len(b) else None),
                "native_red": count_color(native, b"\xff\0\0\xff"),
                "native_blue": count_color(native, b"\0\0\xff\xff"),
                "pdf_red": count_color(exported, b"\xff\0\0\xff"),
                "pdf_blue": count_color(exported, b"\0\0\xff\xff"),
            })
    return records


def investigate(font):
    name = font or "platform-default"
    new = factory(font)
    output = {}
    for direction in ("ltr", "rtl"):
        with new("docx") as document:
            paragraphs = [document.add_paragraph("") for _ in range(9)]
            for paragraph in paragraphs:
                paragraph.format(direction=direction)
            for level, paragraph in enumerate(paragraphs):
                paragraph.set_list("decimal_outline", level=level, start=32767 if level == 0 else None)
            output[direction] = capture(document, f"{name}-outline-{direction}")
    with new("docx") as document:
        document.update_metadata(title="Report Ω", author="Writer é")
        values(document.add_paragraph(), "Body")
        values(document.add_table([[""]])[0, 0].paragraphs[0], "Cell")
        values(document.sections[0].header().add_paragraph(), "Header")
        values(document.sections[0].footer().add_paragraph(), "Footer")
        anchor = document.add_paragraph("Anchor")
        values(document.notes.add(anchor[:]).paragraphs[0], "Footnote")
        values(document.notes.add(anchor[:], kind="endnote").paragraphs[0], "Endnote")
        output["metadata"] = capture(document, f"{name}-metadata")
    # Exact historical assertions execute twice: original default, then a named
    # requested named-font control (selected face bytes are unverified). The latter is diagnostic, not replacement evidence.
    tests = ("test_decimal_outline", "test_metadata_fields", "test_pagination_controls",
             "test_letterhead_composition", "test_picture_placement", "test_word_bookmarks")
    suite = unittest.defaultTestLoader.loadTestsFromNames(tests)
    stream = io.StringIO()
    with patch.object(doc_ex, "new", new):
        result = unittest.TextTestRunner(stream=stream, verbosity=2).run(suite)
    (OUTPUT / f"{name}-tests.log").write_text(stream.getvalue(), encoding="utf-8")
    output["tests"] = {"run": result.testsRun, "failures": len(result.failures),
                       "errors": len(result.errors), "skipped": len(result.skipped)}
    return output


report = {"platform": platform.platform(), "python": sys.version,
          "extension_sha256": hashlib.sha256(Path(_core.__file__).read_bytes()).hexdigest(),
          "modes": {}}
for font in (None, "Source Sans 3"):
    try:
        report["modes"][font or "platform-default"] = investigate(font)
    except Exception:
        import traceback
        report["modes"][font or "platform-default"] = {"error": traceback.format_exc()}
report["files"] = {str(p.relative_to(OUTPUT)): hashlib.sha256(p.read_bytes()).hexdigest()
                   for p in sorted(OUTPUT.rglob("*")) if p.is_file()}
(OUTPUT / "report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
print(json.dumps(report["modes"], indent=2))
