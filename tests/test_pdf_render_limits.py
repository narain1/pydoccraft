"""Exact native output-pixel preflight; never allocate the oversized raster."""
import struct
import unittest

import doc_ex
from doc_ex import _core, _native
from doc_ex.errors import DocumentClosedError, UnsafeArchiveError
from test_rendering import png_chunks


def bounded_page(width=4000.25, height=3999.75, *, unit=2, rotation=0):
    """Original empty PDF page with offset CropBox and independent MediaBox."""
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>",
               b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
               (f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width+20} {height+40}] "
                f"/CropBox [10 20 {width+10} {height+20}] /UserUnit {unit} /Rotate {rotation} "
                "/Resources << >> /Contents 4 0 R >>").encode(),
               b"<< /Length 0 >>\nstream\n\nendstream"]
    out = bytearray(b"%PDF-1.7\n")
    offsets = []
    for number, value in enumerate(objects, 1):
        offsets.append(len(out))
        out.extend(f"{number} 0 obj\n".encode() + value + b"\nendobj\n")
    xref = len(out)
    out.extend(b"xref\n0 5\n0000000000 65535 f \n")
    for offset in offsets:
        out.extend(f"{offset:010} 00000 n \n".encode())
    out.extend(f"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(out)


def dimensions(png):
    return struct.unpack(">II", png_chunks(png)[b"IHDR"][:8])


class PdfRenderLimitTests(unittest.TestCase):
    def test_rounded_pixel_area_rejects_before_native_request_in_all_views(self):
        for rotation in (0, 90, 180, 270):
            raw = bounded_page(rotation=rotation)
            with self.subTest(rotation=rotation), doc_ex.open(raw) as document:
                expected = (8000.5, 7999.5) if rotation % 180 == 0 else (7999.5, 8000.5)
                self.assertEqual((document[0].metadata["width"], document[0].metadata["height"]), expected)
                # Warm a small output, then reject the full 64,008,000-pixel request
                # before entering native rasterization. No near-limit buffer is built.
                small = document.render_page(0, dpi=1)
                self.assertEqual(dimensions(small), (112, 112))
                session = document._render_session
                before = session._stats()
                for render in (lambda: document.render_page(0, 72), lambda: document[0].get_pixmap(72),
                               lambda: _native(lambda: session.render(0, 72))):
                    with self.assertRaisesRegex(UnsafeArchiveError, "pixel limit"):
                        render()
                    self.assertEqual(session._stats(), before)
                with self.assertRaisesRegex(UnsafeArchiveError, "pixel limit"):
                    _native(lambda: _core.render(raw, "pdf", 0, 72))
                self.assertEqual(document.render_page(0, dpi=1), small)
                self.assertEqual(dimensions(document.render_page(0, dpi=72, clip=(0, 0, 1, 1))), (1, 1))
                self.assertEqual(dimensions(document.render_page(0, dpi=1200, clip=(0, 0, 1, 1))), (17, 17))
                self.assertEqual(document[0].get_text(), "")
            with self.assertRaises(DocumentClosedError):
                document.render_page(0, 72)

    def test_native_side_boundary_renders_only_a_thin_safe_strip(self):
        with doc_ex.open(bounded_page(8192, 1, unit=1)) as document:
            self.assertEqual(dimensions(document.render_page(0, dpi=72)), (8192, 1))
            self.assertEqual(dimensions(document.render_page(0, dpi=72, clip=(0, 0, 4096, 1))), (4096, 1))
            before = document._render_session._stats()
            with self.assertRaises(UnsafeArchiveError):
                document.render_page(0, dpi=72, clip=(0, 0, 4097, 1))
            self.assertEqual(document._render_session._stats(), before)
        with doc_ex.open(bounded_page(8193, 1, unit=1)) as document:
            before = document._render_session._stats()
            with self.assertRaises(UnsafeArchiveError):
                document.render_page(0, dpi=72)
            self.assertEqual(document._render_session._stats(), before)
            self.assertEqual(dimensions(document.render_page(0, dpi=72, clip=(0, 0, 1, 1))), (1, 1))


if __name__ == "__main__":
    unittest.main()
