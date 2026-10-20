from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_styles import NS


class TableStyleTests(unittest.TestCase):
    def test_inheritance_regions_and_independent_look_flags(self):
        with doc_ex.new("docx") as document:
            base = document.styles.add("Report table", kind="table")
            base.format(background="white", margins=(2, 4, 6, 8), row_band_size=2)
            base.format(region="first_row", background="navy", bold=True, color="white",
                        alignment="center", vertical_alignment="center",
                        borders={"bottom": {"style": "double", "width": 1}})
            derived = document.styles.add("Detail table", kind="table", based_on=base)
            self.assertEqual(derived.kind, "table")
            self.assertEqual(document.styles[derived.name].id, derived.id)
            self.assertEqual(len(derived.formatting["regions"]), 12)
            self.assertIsNone(derived.formatting["regions"]["first_row"]["bold"])
            self.assertTrue(derived.resolved_formatting["regions"]["first_row"]["bold"])
            self.assertEqual(derived.resolved_formatting["margins"], [2, 4, 6, 8])
            derived.format(region="first_row", bold=False)
            self.assertFalse(derived.resolved_formatting["regions"]["first_row"]["bold"])
            derived.format(region="first_row", bold=None)
            self.assertTrue(derived.resolved_formatting["regions"]["first_row"]["bold"])
            table = document.add_table([["Heading", "Amount"], ["West", "100"]])
            table.format(style=derived, look={"header_row": True, "banded_rows": False})
            self.assertEqual(table.formatting["style"], derived.id)
            self.assertFalse(table.row(0).repeat_header)
            table.row(0).repeat_header = True
            table.format(look={"header_row": False})
            self.assertTrue(table.row(0).repeat_header)
            self.assertFalse(table.formatting["look"]["banded_rows"])
            table.format(style=derived.name, look={"header_row": True})
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
                self.assertEqual(xml.find(".//w:tblStyle", NS).get(f"{{{NS['w']}}}val"), derived.id)
                self.assertIsNotNone(xml.find(".//w:tblHeader", NS))
                styles = ET.fromstring(archive.read("word/styles.xml"))
                node = styles.find(f"w:style[@w:styleId='{base.id}']", NS)
                self.assertEqual(node.get(f"{{{NS['w']}}}type"), "table")
                self.assertIsNotNone(node.find("w:tblStylePr[@w:type='firstRow']", NS))
            table.format(style=None, look=None)
            self.assertIsNone(table.formatting["style"])

    def test_style_and_cell_fills_survive_native_render_and_pdf_export(self):
        with doc_ex.new("docx") as document:
            style = document.styles.add("Colored rows", kind="table")
            style.format(background="yellow", margins=6)
            style.format(region="first_row", background="red")
            style.format(region="odd_rows", background="blue")
            table = document.add_table([["HeaderA", "HeaderB"], ["BodyA", "BodyB"], ["LastA", "LastB"]])
            table.format(style=style, look={"header_row": True, "banded_rows": True})
            table[0, 0].format(background="green")
            for data in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    text = reopened.to_markdown()
                    for label in ("HeaderA", "HeaderB", "BodyA", "BodyB", "LastA", "LastB"):
                        self.assertEqual(text.count(label), 1)
                    _, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                    colors = {pixels[i:i + 4] for i in range(0, len(pixels), 4)}
                    for color in (b"\xff\x00\x00\xff", b"\x00\x80\x00\xff", b"\x00\x00\xff\xff"):
                        self.assertIn(color, colors)
            table[0, 0].format(background=None)
            self.assertIsNone(table[0, 0].formatting["background"])

    def test_style_changes_preserve_nested_geometry_and_ownership(self):
        with doc_ex.new("docx") as document, doc_ex.new("docx") as other:
            base = document.styles.add("Base table", kind="table")
            derived = document.styles.add("Derived table", kind="table", based_on=base)
            table = document.add_table([["", "Outer"]])
            table.format(style=derived)
            nested = table[0, 0].add_table([["Nested"]])
            impossible = document.styles.add("Large margins", kind="table")
            impossible.format(margins=150)
            before = document.to_bytes()
            for operation in (
                lambda: base.format(margins=(0, 150, 0, 150)),
                lambda: setattr(derived, "based_on", impossible),
                lambda: table.format(style=impossible),
                lambda: table.format(style="Normal"),
                lambda: other.add_table([["Other"]]).format(style=base),
            ):
                with self.assertRaises((ValueError, TypeError)):
                    operation()
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(nested[0, 0].text, "Nested")
            with document.copy() as copied:
                copied.styles[base.id].format(background="blue")
                self.assertIsNone(base.formatting["background"])
            base.format(margins=0)
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = derived.formatting

    def test_unsupported_regions_and_patches_fail_atomically(self):
        with doc_ex.new("docx") as document:
            style = document.styles.add("Checked table", kind="table")
            table = document.add_table([["Keep"]])
            table.format(style=style)
            before = document.to_bytes()
            for patch in ({"region": "unknown", "bold": True},
                          {"region": "first_row", "space_after": 8},
                          {"region": "odd_rows", "margins": 4},
                          {"row_band_size": True}, {"row_band_size": 0},
                          {"row_band_size": 1001}, {"row_band_size": 1.5},
                          {"borders": {"diagonal": {"style": "single"}}}):
                with self.subTest(patch=patch), self.assertRaises((ValueError, TypeError)):
                    style.format(**patch)
                self.assertEqual(document.to_bytes(), before)
            for look in ({"header_row": 1}, {"unknown": True}):
                with self.assertRaises((ValueError, TypeError)):
                    table.format(look=look)
                self.assertEqual(document.to_bytes(), before)
            with self.assertRaises(ValueError):
                document.styles["Normal"].format(region="first_row", bold=True)
            with self.assertRaises(ValueError):
                document.add_paragraph("Invalid", style=style)


if __name__ == "__main__":
    unittest.main()
