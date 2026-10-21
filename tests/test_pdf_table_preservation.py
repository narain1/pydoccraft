"""Original standalone PDF regression for conservative table detection."""
import struct
import unittest

import doc_ex


def fixture(extra, rotation):
    content = "".join(
        f"BT /F1 12 Tf 1 0 0 1 {x} {y} Tm ({text}) Tj ET\n"
        for text, x, y in [
            ("LEFTTOP", 50, 340), ("RIGHTTOP", 300, 340),
            ("LEFTBOTTOM", 50, 180), ("RIGHTBOTTOM", 300, 180),
        ]
    )
    if extra is not None:
        x, y = extra
        content += f"BT /F1 12 Tf 1 0 0 1 {x} {y} Tm (EMERALD) Tj ET\n"
    objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 480 420] /CropBox [10 20 470 400] /Rotate {rotation} /UserUnit 1.25 /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        f"<< /Length {len(content)} >>\nstream\n{content}endstream",
    ]
    output = bytearray(b"%PDF-1.7\n")
    offsets = [0]
    for number, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f"{number} 0 obj\n{value}\nendobj\n".encode())
    xref = len(output)
    output.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        output.extend(f"{offset:010} 00000 n \n".encode())
    output.extend(f"trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(output)


class PdfTablePreservationTests(unittest.TestCase):
    def test_ambiguous_table_keeps_every_word_and_genuine_tables_stay_semantic(self):
        for rotation in (0, 90, 180, 270):
            for extra, table_count in ((None, 1), ((50, 260), 0), ((20, 260), 0), ((50, 100), 1)):
                with self.subTest(rotation=rotation, extra=extra):
                    with doc_ex.open(fixture(extra, rotation), filetype="pdf") as document:
                        self.assertEqual(len(document), 1)
                        page = document[0]
                        text = page.get_text("text")
                        markdown = page.get_text("markdown")
                        for label in ("LEFTTOP", "RIGHTTOP", "LEFTBOTTOM", "RIGHTBOTTOM"):
                            self.assertEqual(text.count(label), 1)
                            self.assertEqual(markdown.count(label), 1)
                        self.assertEqual(text.count("EMERALD"), int(extra is not None))
                        self.assertEqual(markdown.count("EMERALD"), int(extra is not None))
                        tables = page.find_tables()
                        self.assertEqual(len(tables), table_count)
                        if tables:
                            self.assertEqual(tables[0].rows, [["LEFTTOP", "RIGHTTOP"], ["LEFTBOTTOM", "RIGHTBOTTOM"]])
                        png = document.render_page(0, dpi=72)
                        expected = (575, 475) if rotation % 180 == 0 else (475, 575)
                        self.assertEqual(struct.unpack(">II", png[16:24]), expected)
                        self.assertEqual(page.get_text("text"), text)
