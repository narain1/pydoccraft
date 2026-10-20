from __future__ import annotations

import math
import struct
import unittest
import zlib

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsafeArchiveError
from fixtures import docx_fixture, pdf_fixture, pptx_fixture, three_page_docx_fixture


def png_chunks(data: bytes) -> dict[bytes, bytes]:
    """Inspect PNG output with the standard library; no optional image dependency."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise AssertionError("render did not return a PNG")
    chunks = {}
    offset = 8
    while offset < len(data):
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        expected_crc = struct.unpack_from(">I", data, offset + 8 + length)[0]
        if zlib.crc32(kind + payload) != expected_crc:
            raise AssertionError(f"invalid {kind!r} checksum")
        chunks[kind] = chunks.get(kind, b"") + payload
        offset += length + 12
    if offset != len(data) or b"IEND" not in chunks:
        raise AssertionError("incomplete PNG")
    return chunks


def has_visible_content(chunks: dict[bytes, bytes]) -> bool:
    """Unfilter RGBA rows until nonwhite, nontransparent fixture content is found."""
    width, height = struct.unpack(">II", chunks[b"IHDR"][:8])
    data = zlib.decompress(chunks[b"IDAT"])
    stride = width * 4
    previous = bytearray(stride)
    for y in range(height):
        offset = y * (stride + 1)
        filtering = data[offset]
        row = bytearray(data[offset + 1:offset + 1 + stride])
        if filtering not in range(5):
            raise AssertionError("invalid PNG row filter")
        for x in range(stride):
            left = row[x - 4] if x >= 4 else 0
            above = previous[x]
            corner = previous[x - 4] if x >= 4 else 0
            if filtering == 1:
                predictor = left
            elif filtering == 2:
                predictor = above
            elif filtering == 3:
                predictor = (left + above) // 2
            elif filtering == 4:
                estimate = left + above - corner
                distances = (abs(estimate - left), abs(estimate - above), abs(estimate - corner))
                predictor = (left, above, corner)[distances.index(min(distances))]
            else:
                predictor = 0
            row[x] = (row[x] + predictor) & 255
        if any(row[x + 3] >= 128 and min(row[x:x + 3]) < 240 for x in range(0, stride, 4)):
            return True
        previous = row
    return False


class RenderingTests(unittest.TestCase):
    def test_all_formats_record_dimensions_and_requested_dpi(self) -> None:
        for filetype, fixture in (("pdf", pdf_fixture), ("docx", docx_fixture), ("pptx", pptx_fixture)):
            with doc_ex.open(fixture()) as document:
                page = document[0]
                for dpi in (72, 144, 150.5, 300):
                    with self.subTest(filetype=filetype, dpi=dpi):
                        data = document.render_page(0, dpi=dpi)
                        self.assertIsInstance(data, bytes)
                        chunks = png_chunks(data)
                        width, height, depth, color, *_ = struct.unpack(">IIBBBBB", chunks[b"IHDR"])
                        self.assertEqual((depth, color), (8, 6))
                        for actual, points in ((width, page.metadata["width"]), (height, page.metadata["height"])):
                            expected = points * dpi / 72
                            # Each engine has its own documented pixel rounding convention.
                            self.assertGreaterEqual(actual, max(1, math.floor(expected)))
                            self.assertLessEqual(actual, math.ceil(expected) + 1)
                        xppm, yppm, unit = struct.unpack(">IIB", chunks[b"pHYs"])
                        self.assertEqual(unit, 1)
                        self.assertEqual(xppm, yppm)
                        self.assertAlmostEqual(xppm * 0.0254, dpi, delta=0.013)
                        # Every RGBA scanline has one PNG filter byte; truncated payloads fail here.
                        self.assertEqual(len(zlib.decompress(chunks[b"IDAT"])), height * (1 + width * 4))
                        if dpi == 72:
                            self.assertTrue(has_visible_content(chunks), "fixture rendered blank")

    def test_invalid_dpi_and_oversized_requests_fail_for_all_formats(self) -> None:
        for fixture in (pdf_fixture, docx_fixture, pptx_fixture):
            with doc_ex.open(fixture()) as document:
                for dpi in (0, 0.5, -1, float("nan"), float("inf"), float("-inf"), 1200.1, 1200):
                    with self.subTest(format=document.metadata["format"], dpi=dpi):
                        # 1200 is a valid DPI, but these page sizes exceed the raster budget.
                        with self.assertRaises(UnsafeArchiveError):
                            document.render_page(0, dpi=dpi)

    def test_page_alias_negative_indexes_and_closed_document(self) -> None:
        for fixture in (pdf_fixture, docx_fixture, pptx_fixture):
            with doc_ex.open(fixture()) as document:
                page = document[-1]
                self.assertEqual(document.render_page(-1, dpi=72), page.get_pixmap(dpi=72))
                for index in (len(document), -len(document) - 1):
                    with self.assertRaises(IndexError):
                        document.render_page(index)
            with self.assertRaises(DocumentClosedError):
                page.get_pixmap()

    def test_intentionally_blank_docx_page_remains_renderable(self) -> None:
        with doc_ex.open(three_page_docx_fixture()) as document:
            self.assertEqual(document[1].get_text(), "")
            chunks = png_chunks(document.render_page(1, dpi=72))
            width, height = struct.unpack(">II", chunks[b"IHDR"][:8])
            self.assertGreater(width * height, 0)


if __name__ == "__main__":
    unittest.main()
