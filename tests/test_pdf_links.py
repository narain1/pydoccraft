from __future__ import annotations

import unittest

import doc_ex


def linked_pdf(lines: list[tuple[int, int, str]], links: list[tuple[tuple[float, float, float, float], str]]) -> bytes:
    content = b"BT /F1 12 Tf " + b" ".join(
        f"1 0 0 1 {x} {y} Tm <{text.encode('cp1252').hex()}> Tj".encode()
        for x, y, text in lines
    ) + b" ET"
    annotations = " ".join(f"{6 + index} 0 R" for index in range(len(links)))
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [{annotations}] >>".encode(),
        f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ]
    for rect, uri in links:
        coordinates = " ".join(str(value) for value in rect)
        action = "/S /GoTo /D [3 0 R /Fit]" if uri == "#page=1" else f"/S /URI /URI <{uri.encode().hex()}>"
        objects.append(f"<< /Type /Annot /Subtype /Link /Rect [{coordinates}] /A << {action} >> >>".encode())
    output = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
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


class PdfLinkTests(unittest.TestCase):
    def test_repeated_labels_have_geometry_specific_unicode_character_offsets(self) -> None:
        data = linked_pdf(
            [(40, 250, "café same and same"), (40, 210, "same alone")],
            [((65.5, 247, 96, 263), "https://example.com/first"),
             ((121.5, 247, 152, 263), "https://example.com/second")],
        )
        with doc_ex.open(data) as document:
            page = document[0]
            links = page.get_links()
            self.assertEqual([(link.text, link.metadata.get("start"), link.metadata.get("end")) for link in links], [("same", 5, 9), ("same", 14, 18)])
            for index, link in enumerate(links):
                block = page.blocks[link.metadata["block"]]
                self.assertEqual(block.text[link.metadata["start"]:link.metadata["end"]], link.text)
                self.assertEqual(link.metadata["annotation_index"], index)
                self.assertEqual(link.metadata["label_status"], "resolved")
                self.assertIn(link.uri, [value.uri for value in block.links])
            markdown = page.get_text("markdown")
            self.assertIn("café [same](https://example.com/first) and [same](https://example.com/second)", markdown)
            self.assertIn("same alone", markdown)
            self.assertNotIn("**Links:**", markdown)

    def test_multiline_empty_and_overlapping_annotations_remain_unresolved(self) -> None:
        cases = [
            ([(40, 180, "First line"), (40, 160, "Second line")],
             [((38, 157, 120, 194), "https://example.com/multiline")]),
            ([(40, 250, "same")],
             [((38, 247, 75, 263), "https://example.com/one"),
              ((38, 247, 75, 263), "https://example.com/two")]),
            ([(40, 250, "same")],
             [((200, 100, 250, 120), "https://example.com/empty")]),
            ([(40, 180, "First line"), (40, 160, "Second line")],
             [((38, 157, 120, 194), "https://example.com/multiline"),
              ((38, 177, 100, 194), "https://example.com/overlap")]),
        ]
        for lines, annotations in cases:
            with self.subTest(annotations=annotations), doc_ex.open(linked_pdf(lines, annotations)) as document:
                page = document[0]
                self.assertEqual([link.uri for link in page.get_links()], [uri for _, uri in annotations])
                for link in page.get_links():
                    self.assertEqual(link.text, "")
                    self.assertNotIn("start", link.metadata)
                    self.assertNotIn("end", link.metadata)
                    self.assertEqual(link.metadata["label_status"], "unresolved_geometry")
                self.assertTrue(all(not block.links for block in page.blocks))

    def test_internal_page_destinations_keep_uri_fragment_and_zero_based_target(self) -> None:
        data = linked_pdf([(40, 250, "same")], [((38, 247, 75, 263), "#page=1")])
        with doc_ex.open(data) as document:
            link = document[0].get_links()[0]
            self.assertEqual((link.text, link.uri), ("same", "#page=1"))
            self.assertEqual(link.metadata["target_kind"], "page")
            self.assertEqual(link.metadata["target_page"], 0)
            self.assertIn("[same](#page=1)", document[0].get_text("markdown"))

    def test_link_inside_detected_table_does_not_duplicate_cell_text(self) -> None:
        data = linked_pdf(
            [(40, 250, "Name"), (180, 250, "Score"), (40, 230, "Alice"), (180, 230, "91")],
            [((38, 247, 75, 263), "https://example.com/table")],
        )
        with doc_ex.open(data) as document:
            page = document[0]
            self.assertEqual(len(page.find_tables()), 1)
            self.assertEqual(page.get_text().count("Name"), 1)
            self.assertEqual(page.get_text("markdown").count("Name"), 1)
            self.assertEqual(page.get_links()[0].text, "")
            self.assertEqual(page.get_links()[0].uri, "https://example.com/table")


if __name__ == "__main__":
    unittest.main()
