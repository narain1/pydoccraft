from __future__ import annotations

import math
import struct
import unittest
import zlib

import doc_ex
from test_rendering import png_chunks


def geometry_pdf(rotation: int) -> bytes:
    """Original fixture with offset boxes, user units, and asymmetric colored marks."""
    content = b"1 0 0 rg 50 60 20 30 re f 0 0 1 rg 150 230 30 40 re f"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        (
            f"<< /Type /Page /Parent 2 0 R /MediaBox [10 20 230 340] "
            f"/CropBox [30 40 210 300] /UserUnit 1.25 /Rotate {rotation} "
            f"/Resources << >> /Contents 4 0 R >>"
        ).encode(),
        f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream",
    ]
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n".encode() + value + b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode())
    output.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(output)


def rgba_pixels(chunks: dict[bytes, bytes]) -> tuple[int, bytes]:
    width, height, depth, color, *_ = struct.unpack(">IIBBBBB", chunks[b"IHDR"])
    if (depth, color) != (8, 6):
        raise AssertionError("expected 8-bit RGBA rendering")
    filtered = zlib.decompress(chunks[b"IDAT"])
    stride = width * 4
    pixels = bytearray(height * stride)
    for row in range(height):
        kind = filtered[row * (stride + 1)]
        if kind not in range(5):
            raise AssertionError("invalid PNG filter")
        for column in range(stride):
            index = row * stride + column
            left = pixels[index - 4] if column >= 4 else 0
            above = pixels[index - stride] if row else 0
            corner = pixels[index - stride - 4] if row and column >= 4 else 0
            if kind == 4:
                estimate = left + above - corner
                distances = (abs(estimate - left), abs(estimate - above), abs(estimate - corner))
                predictor = (left, above, corner)[distances.index(min(distances))]
            else:
                predictor = (0, left, above, (left + above) // 2)[kind]
            pixels[index] = (filtered[row * (stride + 1) + column + 1] + predictor) & 255
    return width, bytes(pixels)


class RenderGeometryTests(unittest.TestCase):
    def test_crop_user_units_and_rotations_at_multiple_resolutions(self) -> None:
        for rotation in (0, 90, 180, 270):
            with doc_ex.open(geometry_pdf(rotation)) as document:
                expected = (325, 225) if rotation in (90, 270) else (225, 325)
                self.assertEqual((document[0].metadata["width"], document[0].metadata["height"]), expected)
                for dpi in (1, 72, 144, 150.5, 300):
                    with self.subTest(rotation=rotation, dpi=dpi):
                        chunks = png_chunks(document.render_page(0, dpi=dpi))
                        size = struct.unpack(">II", chunks[b"IHDR"][:8])
                        self.assertEqual(size, tuple(max(1, math.ceil(side * dpi / 72 - 0.01)) for side in expected))
                        xppm, yppm, unit = struct.unpack(">IIB", chunks[b"pHYs"])
                        self.assertEqual((xppm, unit), (yppm, 1))
                        self.assertAlmostEqual(xppm * 0.0254, dpi, delta=0.013)

    def test_rotation_and_crop_place_colored_content_at_expected_positions(self) -> None:
        # Centers in displayed point coordinates, independently calculated for this fixture.
        centers = {
            0: ((37.5, 281.25), (168.75, 62.5)),
            90: ((43.75, 37.5), (262.5, 168.75)),
            180: ((187.5, 43.75), (56.25, 262.5)),
            270: ((281.25, 187.5), (62.5, 56.25)),
        }
        for rotation, points in centers.items():
            with self.subTest(rotation=rotation), doc_ex.open(geometry_pdf(rotation)) as document:
                width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=72)))
                for (x, y), expected in zip(points, (b"\xff\0\0\xff", b"\0\0\xff\xff")):
                    offset = (int(y) * width + int(x)) * 4
                    self.assertEqual(pixels[offset:offset + 4], expected)


if __name__ == "__main__":
    unittest.main()
