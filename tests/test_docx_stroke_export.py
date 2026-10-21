"""Imported native-generated fixture, independent of the future shape authoring API."""
import hashlib
import math
from pathlib import Path
import unittest

import doc_ex
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

FIXTURE = Path(__file__).with_name("data") / "wordcraft-rectangle-stroke100.docx"


class DocxStrokeExportTests(unittest.TestCase):
    def test_imported_native_rectangle_round_corners_render_and_convert(self):
        data = FIXTURE.read_bytes()
        self.assertEqual(hashlib.sha256(data).hexdigest(), "e70b198cbdcb030f3cede47396ec6c5907464eca85f4b3a48c3757b9f84c31e4")
        with doc_ex.open(data) as document, doc_ex.open(FIXTURE) as reopened:
            self.assertEqual(document.page_count, 1)
            pdf_bytes = document.convert("pdf")
            with doc_ex.open(pdf_bytes) as pdf:
                self.assertEqual(pdf.page_count, 1)
                for dpi in (72, 144, 300):
                    native = document.render_page(0, dpi=dpi)
                    self.assertEqual(native, reopened.render_page(0, dpi=dpi))
                    self.assertEqual(native, doc_ex._core.render(data, "docx", 0, dpi, None))
                    for png in (native, pdf.render_page(0, dpi=dpi)):
                        width, pixels = rgba_pixels(png_chunks(png))
                        side = math.ceil(440 * dpi / 72)
                        self.assertEqual(width, side)
                        self.assertEqual(len(pixels), side * side * 4)
                        for point, expected in ((110, True), (95, False)):
                            center = round(point * dpi / 72)
                            for y in range(center - 1, center + 2):
                                for x in range(center - 1, center + 2):
                                    i = (y * width + x) * 4
                                    r, g, b, a = pixels[i:i+4]
                                    self.assertEqual(b-r > 16 and b-g > 16 and a > 16, expected, (dpi, point, x, y))
