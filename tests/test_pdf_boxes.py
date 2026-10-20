from pathlib import Path
import struct
import unittest

import doc_ex
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks


FONT = Path(__file__).parent / "fixtures/fonts/Inter-Regular.ttf"


class PdfBoxTests(unittest.TestCase):
    def test_view_boxes_with_rotation_crop_and_user_units(self):
        for rotation in (0, 90, 180, 270):
            with self.subTest(rotation=rotation), doc_ex.open(geometry_pdf(rotation)) as source, source.edit() as document:
                page = document.pages[0]
                width, height = page.size
                for actual, expected in zip(page.boxes["crop"], (0, 0, width, height)):
                    self.assertAlmostEqual(actual, expected, delta=0.001)
                annotation = page.annotations.add_link(box=(20, 30, 40, 20), uri="https://example.com")
                page.set_box("crop", (10, 20, width - 20, height - 40))
                self.assertAlmostEqual(page.size[0], width - 20, delta=0.001)
                self.assertAlmostEqual(page.size[1], height - 40, delta=0.001)
                with self.assertRaisesRegex(ValueError, "stale"):
                    _ = annotation.uri
                with doc_ex.open(document.to_bytes()) as reopened:
                    dimensions = struct.unpack(">II", png_chunks(reopened.render_page(0, dpi=72))[b"IHDR"][:8])
                    self.assertEqual(dimensions, (int(width - 20), int(height - 40)))
                page.set_box("crop")
                expected = (400, 275) if rotation in (90, 270) else (275, 400)
                self.assertEqual(page.size, expected)

    def test_invalid_boxes_are_atomic_and_effective_defaults_follow_crop(self):
        with doc_ex.new("pdf", size=(300, 400)) as document:
            page = document.pages[0]
            page.set_box("trim", (30, 40, 200, 280))
            before = page.boxes
            for kind, box in (("crop", (-1, 0, 300, 400)), ("trim", (0, 0, 0, 10)), ("media", None), ("media", (290, 390, 10, 10)), ("unknown", (0, 0, 10, 10))):
                with self.subTest(kind=kind, box=box), self.assertRaises((ValueError, doc_ex.DocExError)):
                    page.set_box(kind, box)
                self.assertEqual(page.boxes, before)
            page.set_box("trim")
            self.assertEqual(page.boxes["trim"], page.boxes["crop"])
            page.background = "red"
            page.set_box("media", (-20, -20, 340, 440))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "background"):
                _ = page.background

    def test_both_text_paths_support_color_and_searchable_text(self):
        with doc_ex.new("pdf", size=(240, 120)) as document:
            page = document.pages[0]
            page.draw_text("Red", at=(10, 10), font_size=24, color="red")
            page.draw_text("Ω", at=(100, 10), font_size=24, font=FONT, color="green")
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn("Red", reopened[0].get_text())
                self.assertIn("Ω", reopened[0].get_text())
                _, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                colors = list(zip(pixels[::4], pixels[1::4], pixels[2::4]))
                self.assertTrue(any(r > 180 and g < 60 and b < 60 for r, g, b in colors))
                self.assertTrue(any(g > 80 and r < 60 and b < 60 for r, g, b in colors))
