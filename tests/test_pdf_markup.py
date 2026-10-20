import unittest

import doc_ex
from doc_ex.errors import UnsupportedFormatError
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks


KINDS = ("underline", "strikeout", "squiggly")


def colored_pixels(png, channel):
    width, pixels = rgba_pixels(png_chunks(png))
    return [(offset // 4 % width, offset // 4 // width)
            for offset in range(0, len(pixels), 4)
            if pixels[offset + channel] > max(pixels[offset + other]
                for other in range(3) if other != channel) + 30]


class PdfMarkupTests(unittest.TestCase):
    def test_metadata_move_recolor_reopen_delete_and_stale_handles(self):
        for kind in KINDS:
            with self.subTest(kind=kind), doc_ex.new("pdf", size=(220, 220)) as document:
                page = document.pages[0]
                page.draw_text("Preserved body", at=(20, 160))
                mark = page.annotations.add(kind, box=(20, 20, 120, 20),
                                            text="Comment café", author="Author", color="red")
                initial = document.render_page(0, dpi=72)
                self.assertTrue(colored_pixels(initial, 0))
                mark.update(text="Revised ✓", author="Reviewer", color="blue", box=(30, 50, 120, 20))
                self.assertEqual((mark.kind, mark.text, mark.author, mark.color, mark.box),
                                 (kind, "Revised ✓", "Reviewer", "#0000FF", (30, 50, 120, 20)))
                updated = document.render_page(0, dpi=72)
                self.assertNotEqual(initial, updated)
                ink = colored_pixels(updated, 2)
                self.assertTrue(ink)
                self.assertTrue(all(30 <= x < 150 and 50 <= y < 70 for x, y in ink))
                self.assertFalse(colored_pixels(updated, 0))
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual(reopened[0].get_text().strip(), "Preserved body")
                    self.assertEqual(reopened.render_page(0, dpi=72), updated)
                    with reopened.edit() as edited:
                        self.assertEqual(edited.pages[0].annotations[0].kind, kind)
                        self.assertEqual(edited.pages[0].annotations[0].text, "Revised ✓")
                del page.annotations[0]
                self.assertEqual(len(page.annotations), 0)
                self.assertFalse(colored_pixels(document.render_page(0, dpi=72), 2))
                with self.assertRaisesRegex(ValueError, "stale"):
                    _ = mark.text

    def test_native_markup_pixels_follow_rotated_cropped_user_unit_views(self):
        for rotation in (0, 90, 180, 270):
            with self.subTest(rotation=rotation), doc_ex.open(geometry_pdf(rotation)) as original, original.edit() as document:
                page = document.pages[0]
                handles = [page.annotations.add(kind, box=(20, 20 + row * 50, 120, 20), color="#00FF00")
                           for row, kind in enumerate(KINDS)]
                pixels = colored_pixels(document.render_page(0, dpi=72), 1)
                for row, kind in enumerate(KINDS):
                    y = 20 + row * 50
                    self.assertEqual(handles[row].kind, kind)
                    for actual, expected in zip(handles[row].box, (20, y, 120, 20)):
                        self.assertAlmostEqual(actual, expected, places=3)
                    ink = [(x, py) for x, py in pixels if 20 <= x < 140 and y <= py < y + 20]
                    self.assertGreater(len(ink), 60)
                    top, bottom = min(py for _, py in ink), max(py for _, py in ink)
                    if kind == "strikeout":
                        self.assertGreaterEqual(top, y + 10)
                        self.assertLessEqual(bottom, y + 13)
                    else:
                        self.assertGreaterEqual(top, y + 16)
                        self.assertLess(bottom, y + 20)
                page.rotate(90)
                with self.assertRaisesRegex(ValueError, "stale"):
                    _ = handles[0].kind

    def test_invalid_markup_changes_preserve_bytes_and_warm_render(self):
        for kind in KINDS:
            with self.subTest(kind=kind), doc_ex.new("pdf", size=(200, 200)) as document:
                mark = document.pages[0].annotations.add(kind, box=(20, 20, 120, 20), text="Before")
                before = document.to_bytes()
                png = document.render_page(0, dpi=72)
                for changes in ({"text": "Do not commit", "box": (20, 20, 130, 20)},
                                {"text": "Do not commit", "uri": "https://example.com"}):
                    with self.assertRaises(UnsupportedFormatError):
                        mark.update(**changes)
                    self.assertEqual(document.to_bytes(), before)
                    self.assertEqual(document.render_page(0, dpi=72), png)
                    self.assertEqual(mark.text, "Before")


if __name__ == "__main__":
    unittest.main()
