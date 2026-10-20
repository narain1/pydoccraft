from pathlib import Path
import unittest

import doc_ex
from doc_ex.errors import UnsupportedFormatError
from test_authoring import image_fixture


def resource_pdf(name="PCHelv", *, inherited=False, opacity=False):
    if opacity:
        resources = b"<< /ExtGState << /PCGS100 << /Type /ExtGState /ca 0.5 /CA 0.5 >> >> >>"
        content = b"q /PCGS100 gs 1 0 0 rg 10 140 40 40 re f Q"
    else:
        resources = f"<< /Font << /{name} << /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >> >> >>".encode()
        content = f"BT /{name} 20 Tf 1 0 0 1 10 150 Tm (MMMM WWWW) Tj ET".encode()
    resources = b" /Resources " + resources
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1" + (resources if inherited else b"") + b" >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R" + (b"" if inherited else resources) + b" >>",
        f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream",
    ]
    output = bytearray(b"%PDF-1.7\n")
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


class PdfResourcePreservationTests(unittest.TestCase):
    def test_imported_font_collision_fails_without_changing_pixels_or_content(self):
        font = Path(__file__).parent / "fixtures/fonts/Inter-Regular.ttf"
        for inherited in (False, True):
            with self.subTest(inherited=inherited), doc_ex.open(resource_pdf(inherited=inherited)) as source, source.edit() as document:
                page = document.pages[0]
                before = document.to_bytes()
                png = document.render_page(0, dpi=72)
                for operation in (
                    lambda: page.draw_path([("move", 10, 100), ("line", 30, 100)]),
                    lambda: setattr(page, "background", "white"),
                    lambda: page.draw_text("Added", at=(10, 100), font=font),
                ):
                    with self.assertRaisesRegex(UnsupportedFormatError, "resource"):
                        operation()
                    self.assertEqual(document.to_bytes(), before)
                    self.assertEqual(document.render_page(0, dpi=72), png)
                    self.assertIn("MMMM WWWW", page.get_text())
                # Unaffected native writers can still add content on this page.
                clip = source.render_page(0, dpi=72, clip=(0, 0, 200, 70))
                page.draw_text("Added", at=(10, 100))
                page.add_image(image_fixture(), box=(10, 140, 20, 20))
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual(reopened.render_page(0, dpi=72, clip=(0, 0, 200, 70)), clip)
                    self.assertEqual(reopened.to_markdown().count("Added"), 1)
                    self.assertEqual(source.render_page(0, dpi=72), png)

    def test_added_text_collision_and_inherited_opacity_collision_are_atomic(self):
        for source_bytes, operation in (
            (resource_pdf("PCFHelvetica"), lambda page: page.draw_text("Added", at=(10, 100))),
            (resource_pdf(inherited=True, opacity=True), lambda page: setattr(page, "background", "white")),
        ):
            with doc_ex.open(source_bytes) as source, source.edit() as document:
                before = document.to_bytes()
                png = document.render_page(0, dpi=72)
                with self.assertRaisesRegex(UnsupportedFormatError, "resource"):
                    operation(document.pages[0])
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), png)


if __name__ == "__main__":
    unittest.main()
