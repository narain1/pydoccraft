from pathlib import Path
import struct
import unittest
import zlib

import doc_ex
from doc_ex.errors import UnsupportedFormatError, UnsafeArchiveError
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks

FONT = Path(__file__).parent / "fixtures/fonts/Inter-Regular.ttf"


def quadrant_png():
    colors = ((255, 0, 0, 255), (0, 255, 0, 255), (0, 0, 255, 255), (255, 255, 0, 255))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = []
    for y in range(20):
        rows.append(b"\0" + b"".join(bytes(colors[(y >= 10) * 2 + (x >= 10)]) for x in range(20)))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 20, 20, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b"")


def pixel(png, x, y):
    width, rgba = rgba_pixels(png_chunks(png))
    return rgba[(y * width + x) * 4:(y * width + x + 1) * 4]


class PdfTextBoxTests(unittest.TestCase):
    def test_native_wrapping_alignment_and_exactly_once_searchable_text(self):
        with doc_ex.new("pdf", size=(300, 300)) as document:
            page = document.pages[0]
            text = "One two three four five six seven eight nine ten"
            page.draw_text(text, box=(20, 20, 110, 100), font_size=14, color="blue")
            for index, align in enumerate(("left", "center", "right")):
                page.draw_text("Aligned", box=(150, 20 + index * 60, 120, 40), align=align)
            with doc_ex.open(document.to_bytes()) as reopened:
                actual = reopened[0].get_text()
                for word in text.split():
                    self.assertEqual(actual.split().count(word), 1)
                aligned = [block for block in reopened[0].blocks if block.text.strip() == "Aligned"]
                self.assertEqual(len(aligned), 3)
                self.assertLess(aligned[0].bbox[0], aligned[1].bbox[0])
                self.assertLess(aligned[1].bbox[0], aligned[2].bbox[0])
                for block in reopened[0].blocks:
                    self.assertGreaterEqual(block.bbox[0], 19)
                    self.assertLessEqual(block.bbox[2], 271)
                _, rgba = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                self.assertIn(b"\x00\x00\xff\xff", {rgba[i:i + 4] for i in range(0, len(rgba), 4)})

    def test_supplied_font_explicit_unicode_lines_and_atomic_overflow(self):
        with doc_ex.new("pdf", size=(300, 300)) as document:
            page = document.pages[0]
            page.draw_text("Café Ω\nSecond line", box=(30, 30, 230, 100), font=FONT, align="right", font_size=20)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn("Café Ω", reopened[0].get_text())
                self.assertIn("Second line", reopened[0].get_text())
                self.assertTrue(reopened.render_page(0))
            before = document.to_bytes()
            for arguments in (
                {"box": (20, 20, 20, 40), "font": FONT},
                {"box": (20, 20, 200, 1), "font": FONT},
                {"box": (20, 20, 200, 100), "font": FONT, "align": "justify"},
                {"box": (20, 20, 10, 50)},
                {"box": (20, 20, 200, 1)},
                {"box": (290, 20, 30, 50)},
                {"box": (20, 20, -1, 50)},
                {"box": (20, 20, 200, 50), "at": (0, 0)},
                {"at": (20, 20), "align": "right"},
                {},
            ):
                with self.subTest(arguments=arguments), self.assertRaises((ValueError, UnsupportedFormatError)):
                    page.draw_text("A sufficiently wide line", **arguments)
                self.assertEqual(document.to_bytes(), before)
            with self.assertRaises(UnsafeArchiveError):
                page.draw_text("word " * 10000, box=(10, 150, 270, 100))
            self.assertEqual(document.to_bytes(), before)

    def test_box_and_image_in_current_rotated_cropped_view(self):
        for rotation in (0, 90, 180, 270):
            with self.subTest(rotation=rotation), doc_ex.open(geometry_pdf(rotation)) as original, original.edit() as document:
                page = document.pages[0]
                page.draw_text("View label", box=(30, 80, 130, 45), font_size=16)
                page.add_image(quadrant_png(), box=(40, 140, 60, 60), flip_h=True)
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual(reopened[0].get_text().count("View label"), 1)
                    png = reopened.render_page(0, dpi=72)
                    self.assertEqual(pixel(png, 50, 150), b"\x00\xff\x00\xff")
                    self.assertEqual(pixel(png, 90, 190), b"\x00\x00\xff\xff")

    def test_image_crop_rotation_flips_and_visual_only_crop(self):
        colors = [b"\xff\x00\x00\xff", b"\x00\xff\x00\xff", b"\x00\x00\xff\xff", b"\xff\xff\x00\xff"]
        cases = (({}, [0, 1, 2, 3]), ({"flip_h": True}, [1, 0, 3, 2]),
                 ({"flip_v": True}, [2, 3, 0, 1]), ({"rotation": 90}, [1, 3, 0, 2]),
                 ({"rotation": 180}, [3, 2, 1, 0]), ({"rotation": 270}, [2, 0, 3, 1]))
        for options, order in cases:
            with self.subTest(options=options), doc_ex.new("pdf", size=(200, 200)) as document:
                document.pages[0].add_image(quadrant_png(), box=(40, 40, 100, 100), **options)
                png = document.render_page(0, dpi=72)
                self.assertEqual([pixel(png, x, y) for x, y in ((60, 60), (120, 60), (60, 120), (120, 120))],
                                 [colors[index] for index in order])
                self.assertEqual(pixel(png, 30, 60), b"\xff\xff\xff\xff")
        with doc_ex.new("pdf", size=(200, 200)) as document:
            document.pages[0].add_image(quadrant_png(), box=(40, 40, 100, 100), crop=(0.4, 0, 0, 0))
            self.assertEqual(pixel(document.render_page(0, dpi=72), 65, 60), colors[1])
            with doc_ex.open(document.to_bytes()) as reopened:
                image = reopened[0].get_images()[0]
                self.assertEqual(struct.unpack(">II", png_chunks(image.data)[b"IHDR"][:8]), (20, 20))

    def test_invalid_image_transforms_leave_document_unchanged(self):
        with doc_ex.new("pdf", size=(200, 200)) as document:
            page = document.pages[0]
            before = document.to_bytes()
            for options in ({"crop": (0.46, 0, 0, 0)}, {"crop": (-0.1, 0, 0, 0)},
                            {"crop": (float("nan"), 0, 0, 0)}, {"crop": (0, 0)},
                            {"rotation": 45}, {"rotation": True}, {"flip_h": 1}):
                with self.subTest(options=options), self.assertRaises((ValueError, TypeError)):
                    page.add_image(quadrant_png(), box=(20, 20, 100, 100), **options)
                self.assertEqual(document.to_bytes(), before)


if __name__ == "__main__":
    unittest.main()
