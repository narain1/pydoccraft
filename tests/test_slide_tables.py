from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from test_rendering import png_chunks


class SlideTableTests(unittest.TestCase):
    def test_layout_values_serialize_and_render(self):
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            table = slide.add_table([["Item", "Total"], ["Support", "$250"]], box=(20, 20, 400, 120))
            table.column_widths = [260, 140]
            table.row(0).height = (40, "at_least")
            table.borders = ("navy", 1.5)
            cell = table[0, 0]
            cell.margins = (4, 8, 4, 8)
            cell.alignment = "center"
            cell.vertical_alignment = "center"
            self.assertEqual(table.column_widths, [260, 140])
            self.assertEqual(table.row(0).height, (40, "at_least"))
            self.assertEqual(cell.margins, (4, 8, 4, 8))
            self.assertEqual(cell.alignment, "center")
            self.assertEqual(table.borders, ("#000080", 1.5))
            self.assertEqual(table.box[2:], (400, 100))
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                table.row(0).height = (10, "exact")
            self.assertEqual(table.row(0).height, (40, "at_least"))
            for data in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    text = reopened[0].get_text()
                    self.assertEqual(text.count("Support"), 1)
                    self.assertEqual(text.count("$250"), 1)
                    self.assertIn(b"IDAT", png_chunks(reopened.render_page(0, dpi=72)))
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                ns = {"a": "http://schemas.openxmlformats.org/drawingml/2006/main"}
                xml = ET.fromstring(archive.read("ppt/slides/slide1.xml"))
                self.assertEqual(len(xml.findall(".//a:gridCol", ns)), 2)
            table.borders = None
            self.assertIsNone(table.borders)

    def test_merged_aliases_structure_generations_and_native_split(self):
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            table = slide.add_table([["A", "B"], ["C", "D"]], box=(20, 20, 400, 120))
            stale_cell, stale_row = table[0, 0], table.row(0)
            anchor = table.merge((0, 0), (1, 0))
            self.assertEqual(table[1, 0].text, anchor.text)
            self.assertEqual(table.rows[1][0], "")
            for getter in (lambda: stale_cell.text, lambda: stale_row.height):
                with self.assertRaisesRegex(ValueError, "stale"):
                    getter()
            table[1, 0].text = "Merged"
            self.assertEqual(anchor.text, "Merged")
            before = table.rows
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                table.merge((1, 0), (1, 1))
            self.assertEqual(table.rows, before)
            anchor.split()
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = anchor.text
            self.assertEqual(table.rows, [["Merged", "B"], ["", "D"]])
            old = table[0, 0]
            table.insert_row(1)
            with self.assertRaisesRegex(ValueError, "stale"):
                old.text = "Wrong row"
            column = table.insert_column(1)
            column[1].text = "Inserted"
            self.assertEqual(table[1, 1].text, "Inserted")
            table.delete_column(1)
            table.delete_row(1)
            self.assertEqual(table.rows, [["Merged", "B"], ["", "D"]])
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened[0].get_text().count("Merged"), 1)

    def test_cell_layout_is_shared_with_docx(self):
        with doc_ex.new("docx") as document:
            cell = document.add_table([["A"]])[0, 0]
            cell.margins = (4, 6, 8, 10)
            cell.alignment = "right"
            self.assertEqual(cell.margins, (4, 6, 8, 10))
            self.assertEqual(cell.alignment, "right")
            cell.alignment = None
            cell.margins = None
            self.assertIsNone(cell.alignment)
            with doc_ex.open(document.convert("pdf")) as pdf:
                self.assertIn("A", pdf[0].get_text())
