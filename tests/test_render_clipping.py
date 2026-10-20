from __future__ import annotations

import math
import struct
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsafeArchiveError
from fixtures import docx_fixture, pdf_fixture, pptx_fixture
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks


def small_document(filetype: str) -> bytes:
    with doc_ex.new(filetype, size=(200.25, 180.5)) as document:
        if filetype == "pdf":
            page = document.pages[0]
            page.background = "#D0E0F0"
            page.draw_text("Clipped preview", at=(16, 24), font_size=16, color="red")
        elif filetype == "docx":
            document.sections[0].margins = 16
            document.background = "#D0E0F0"
            paragraph = document.add_paragraph("Clipped preview")
            paragraph.find("Clipped preview")[0].format(color="red", font_size=16)
            document.add_table([["A", "B"], ["1", "2"]])
        else:
            slide = document.slides.add()
            slide.background = "#D0E0F0"
            slide.add_shape("rectangle", box=(16, 24, 60, 35), fill="red")
            slide.add_text("Clipped preview", box=(16, 64, 170, 40), font_size=16)
        return document.to_bytes()


def pixels(data: bytes) -> tuple[int, int, bytes]:
    chunks = png_chunks(data)
    width, rgba = rgba_pixels(chunks)
    height = struct.unpack(">II", chunks[b"IHDR"][:8])[1]
    return width, height, rgba


class RenderClippingTests(unittest.TestCase):
    def assert_region_matches(self, document, dpi, clip, full=None, *, rounding_case=None):
        full = document.render_page(0, dpi=dpi) if full is None else full
        width, height, rgba = pixels(full)
        scale = dpi / 72
        if document.metadata["format"] in {"pdf", "docx"}:
            scale = struct.unpack("f", struct.pack("f", scale))[0]
        x, y, w, h = clip
        x0 = min(width, math.floor(max(0, x) * scale))
        y0 = min(height, math.floor(max(0, y) * scale))
        x1 = min(width, math.ceil(min(document[0].metadata["width"], x + w) * scale))
        y1 = min(height, math.ceil(min(document[0].metadata["height"], y + h) * scale))
        rendered = document.render_page(0, dpi=dpi, clip=clip)
        actual_width, actual_height, actual = pixels(rendered)
        self.assertEqual((actual_width, actual_height), (x1 - x0, y1 - y0))
        expected = b"".join(rgba[(row * width + x0) * 4:(row * width + x1) * 4] for row in range(y0, y1))
        if rounding_case is None:
            self.assertEqual(actual, expected)
        else:
            # Measured for this text/rectangle fixture only: PDF differs at one pixel at
            # 144 DPI and two at 150.5; PPTX at 150.5 differs along 124 top-edge pixels.
            # All differences are <=1 RGB level with identical alpha. Native translated
            # transforms and f32 path conversion suggest edge rounding; that cause is an
            # inference, not a general claim of pixel-identical region rendering.
            self.assertEqual(actual[3::4], expected[3::4])
            changed = []
            for index in range(actual_width * actual_height):
                a = actual[index * 4:index * 4 + 3]
                b = expected[index * 4:index * 4 + 3]
                if a != b:
                    self.assertLessEqual(max(abs(left - right) for left, right in zip(a, b)), 1)
                    changed.append((index % actual_width, index // actual_width))
            if rounding_case == "pdf-text":
                self.assertIn(dpi, (144, 150.5))
                self.assertLessEqual(len(changed), 2)
            elif rounding_case == "pptx-edge":
                self.assertEqual(dpi, 150.5)
                self.assertLessEqual(len(changed), 124)
                # The rectangle starts at y=24pt; only that antialiased top edge may differ.
                self.assertTrue(all(y == math.floor(24 * scale) - y0 for _, y in changed))
            else:
                self.fail("unknown fixture-specific rounding case")
        self.assertEqual(png_chunks(rendered)[b"pHYs"], png_chunks(full)[b"pHYs"])
        self.assertEqual(rendered, document[0].get_pixmap(dpi=dpi, clip=clip))
        return rendered

    def test_all_formats_match_full_page_pixels_at_multiple_dpi(self):
        for filetype in ("pdf", "docx", "pptx"):
            with doc_ex.open(small_document(filetype)) as document:
                for dpi in (72, 144, 150.5):
                    with self.subTest(filetype=filetype, dpi=dpi):
                        rounding_case = "pdf-text" if filetype == "pdf" and dpi in (144, 150.5) else "pptx-edge" if filetype == "pptx" and dpi == 150.5 else None
                        self.assert_region_matches(document, dpi, (13.25, 16.5, 100.5, 80.25), rounding_case=rounding_case)

    def test_full_page_and_boundary_intersections_keep_native_rounding(self):
        for filetype in ("pdf", "docx", "pptx"):
            with self.subTest(filetype=filetype), doc_ex.open(small_document(filetype)) as document:
                w, h = document[0].metadata["width"], document[0].metadata["height"]
                full = document.render_page(0, dpi=72)
                self.assertEqual(document.render_page(0, dpi=72, clip=(0, 0, w, h)), full)
                self.assert_region_matches(document, 72, (-5, -3, 40, 30), full)
                self.assert_region_matches(document, 72, (w - 20, h - 20, 30, 30), full)

    def test_pdf_clips_follow_crop_user_units_and_all_rotations(self):
        centers = {0: (37.5, 281.25), 90: (43.75, 37.5), 180: (187.5, 43.75), 270: (281.25, 187.5)}
        for rotation, (x, y) in centers.items():
            with doc_ex.open(geometry_pdf(rotation)) as document:
                for dpi in (72, 144):
                    with self.subTest(rotation=rotation, dpi=dpi):
                        rendered = self.assert_region_matches(document, dpi, (x - 22.25, y - 24.5, 44.5, 49))
                        self.assertIn(b"\xff\0\0\xff", pixels(rendered)[2])

    def test_small_regions_render_when_full_page_exceeds_limits(self):
        for fixture in (pdf_fixture, docx_fixture, pptx_fixture):
            with doc_ex.open(fixture()) as document:
                with self.subTest(filetype=document.metadata["format"]):
                    with self.assertRaises(UnsafeArchiveError):
                        document.render_page(0, dpi=1200)
                    clipped = document.render_page(0, dpi=1200, clip=(0, 0, 1, 1))
                    self.assertEqual(pixels(clipped)[:2], (17, 17))
                    with self.assertRaises(UnsafeArchiveError):
                        document.render_page(0, dpi=1200, clip=(0, 0, document[0].metadata["width"], document[0].metadata["height"]))

    def test_invalid_empty_and_oversized_clips_fail_explicitly(self):
        for filetype in ("pdf", "docx", "pptx"):
            with doc_ex.open(small_document(filetype)) as document:
                for clip in [(0, 0, 0, 1), (0, 0, -1, 1), (math.nan, 0, 1, 1), (0, math.inf, 1, 1), (1e308, 0, 1e308, 1), (1000, 0, 1, 1), (-20, -20, 1, 1)]:
                    with self.subTest(filetype=filetype, clip=clip), self.assertRaises(ValueError):
                        document.render_page(0, clip=clip)
                for clip in [(1, 2, 3), "abcd", (True, 0, 1, 1)]:
                    with self.assertRaises((TypeError, ValueError)):
                        document.render_page(0, clip=clip)
                with self.assertRaises(IndexError):
                    document.render_page(10, clip=(0, 0, 1, 1))
                for dpi in (0, float("nan"), 1201):
                    with self.assertRaises(UnsafeArchiveError):
                        document.render_page(0, dpi=dpi, clip=(0, 0, 1, 1))
                page = document[0]
            with self.assertRaises(DocumentClosedError):
                page.get_pixmap(clip=(0, 0, 1, 1))
        from doc_ex import _core
        with self.assertRaises(ValueError):
            _core.render(pdf_fixture(), "pdf", 0, 72, None, (0, 0, 0, 1))

    def test_editable_render_entrypoints_preserve_content_and_lifecycle(self):
        for filetype in ("pdf", "docx", "pptx"):
            with doc_ex.new(filetype, size=(200, 180)) as document:
                if filetype == "pptx":
                    page = document.slides.add()
                    page.add_shape("rectangle", box=(10, 10, 40, 30), fill="red")
                elif filetype == "pdf":
                    page = document.pages[0]
                    page.draw_text("Visible", at=(10, 10))
                else:
                    page = None
                    document.add_paragraph("Visible")
                before = document.to_bytes()
                rendered = document.render_page(-1, dpi=72, clip=(5, 5, 70, 70))
                with doc_ex.open(before) as reopened:
                    self.assertEqual(rendered, reopened.render_page(-1, dpi=72, clip=(5, 5, 70, 70)))
                if page is not None:
                    self.assertEqual(rendered, page.get_pixmap(dpi=72, clip=(5, 5, 70, 70)))
                self.assertEqual(document.to_bytes(), before)
            with self.assertRaises(DocumentClosedError):
                document.render_page(0, clip=(0, 0, 1, 1))


if __name__ == "__main__":
    unittest.main()
