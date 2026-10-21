"""Imported native-generated breaks; independent of any new authoring method."""
import hashlib
import math
from pathlib import Path
import unittest

import doc_ex
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

FIXTURE = Path(__file__).with_name("data") / "deckcraft-table-line-breaks.pptx"


class PptxBreakExportTests(unittest.TestCase):
    def assert_ink(self, width, pixels, dpi):
        height = len(pixels) // (width * 4)
        self.assertEqual((width, height), (math.ceil(576 * dpi / 72), math.ceil(360 * dpi / 72)))
        self.assertEqual(len(pixels), width * height * 4)
        counts, sums = [0, 0, 0], [0, 0, 0]
        for i in range(0, width * (height * 4 // 5) * 4, 4):
            rgb = pixels[i:i+3]
            for c in range(3):
                if rgb[c] > 80 and rgb[c] > rgb[(c+1) % 3]+60 and rgb[c] > rgb[(c+2) % 3]+60:
                    counts[c] += 1
                    sums[c] += i // (width * 4)
        self.assertTrue(all(count > 10 for count in counts), counts)
        means = [total / count for total, count in zip(sums, counts)]
        self.assertLess(means[0] + 3, means[1])
        self.assertLess(means[1] + 3, means[2])
        scale = dpi / 72
        picture = 0
        for y in range(int(305 * scale), int(315 * scale)):
            for x in range(int(505 * scale), int(535 * scale)):
                i = (y * width + x) * 4
                picture += all(abs(a-b) <= 1 for a, b in zip(pixels[i:i+4], (32, 96, 192, 255)))
        self.assertGreater(picture, 200)

    def test_imported_breaks_keep_pdf_text_links_images_and_session_pixels(self):
        data = FIXTURE.read_bytes()
        self.assertEqual(hashlib.sha256(data).hexdigest(), "1e2f2506ca7eeec142f50731e14a5f3a6b0db9d2994094fcfb9f7918ba421fe5")
        with doc_ex.open(data) as source, doc_ex.open(FIXTURE) as reopened:
            self.assertEqual(source.page_count, 2)
            self.assertEqual(source[0].find_tables()[0].rows, [["ALPHA\nBETA\nGAMMA"]])
            self.assertEqual(len(source[0].get_images()), 1)
            links = source[0].get_links()
            url = next(link for link in links if link.uri == "https://example.com/line-break")
            self.assertEqual((url.text, url.metadata["start"], url.metadata["end"]), ("ALPHA", 0, 5))
            target = next(link for link in links if link.metadata.get("target_page") == 1)
            self.assertEqual((target.text, target.metadata["start"], target.metadata["end"]), ("GAMMA", 11, 16))
            pdf_bytes = source.convert("pdf")
            self.assertEqual(pdf_bytes, reopened.convert("pdf"))
            with doc_ex.open(pdf_bytes) as pdf:
                self.assertEqual(pdf.page_count, 2)
                text = pdf[0].get_text()
                positions = []
                for label in ("ALPHA", "BETA", "GAMMA"):
                    self.assertEqual(text.count(label), 1, text)
                    positions.append(text.index(label))
                self.assertEqual(positions, sorted(positions))
                pdf_links = pdf[0].get_links()
                self.assertEqual(len(pdf_links), 2)
                self.assertTrue(any(link.uri == url.uri for link in pdf_links))
                self.assertTrue(any(link.page == 1 or link.uri == "#page=2" for link in pdf_links))
                for dpi in (72, 144, 200):
                    png = source.render_page(0, dpi=dpi)
                    self.assertEqual(png, reopened.render_page(0, dpi=dpi))
                    self.assertEqual(png, doc_ex._core.render(data, "pptx", 0, dpi, None))
                    native_width, native_pixels = rgba_pixels(png_chunks(png))
                    pdf_width, pdf_pixels = rgba_pixels(png_chunks(pdf.render_page(0, dpi=dpi)))
                    self.assert_ink(native_width, native_pixels, dpi)
                    self.assert_ink(pdf_width, pdf_pixels, dpi)
                    if dpi == 200:
                        self.assertEqual(len(native_pixels), len(pdf_pixels))
                        self.assertTrue(all(abs(a-b) <= 1 for a, b in zip(native_pixels, pdf_pixels)))
