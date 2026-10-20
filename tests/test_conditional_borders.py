"""Pinned native conditional-border limits; no new border API.

The raw-model omission is preserved separately in the Rust
upstream_conditional_vertical_borders_are_serialized_but_dropped regression.
"""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_styles import NS


AFFECTED = (
    ("first_row", "inside_vertical"), ("last_row", "inside_vertical"),
    ("odd_rows", "inside_vertical"), ("even_rows", "inside_vertical"),
    ("first_column", "right"), ("last_column", "left"),
    ("top_left", "right"), ("bottom_left", "right"),
    ("top_right", "left"), ("bottom_right", "left"),
    ("odd_columns", "left"), ("odd_columns", "right"),
    ("even_columns", "left"), ("even_columns", "right"),
)


def colored_pixels(document):
    counts = []
    for data in (document.to_bytes(), document.convert("pdf")):
        with doc_ex.open(data) as reopened:
            _, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=144)))
            counts.append({
                color: sum(pixels[i:i + 4] == rgba for i in range(0, len(pixels), 4))
                for color, rgba in (("red", b"\xff\x00\x00\xff"),
                                    ("blue", b"\x00\x00\xff\xff"),
                                    ("green", b"\x00\x80\x00\xff"))
            })
    return counts


class ConditionalBorderTests(unittest.TestCase):
    def test_lost_conditional_vertical_edges_reject_atomically(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Kept paragraph")
            style = document.styles.add("Checked borders", kind="table")
            table = document.add_table([["A", "B"], ["C", "D"]])
            table.format(style=style)
            selected = paragraph[:]
            before = document.to_bytes()
            rendered = document.render_page(0, dpi=72)
            for region, side in AFFECTED:
                for border in ({"color": "red", "width": 4}, {"style": "none"}):
                    with self.subTest(region=region, side=side, border=border):
                        with self.assertRaisesRegex(UnsupportedFormatError, "conditional table styles"):
                            style.format(region=region, background="green", borders={side: border})
                        self.assertEqual(document.to_bytes(), before)
                        self.assertEqual(selected.text, "Kept paragraph")
                        self.assertEqual(table[0, 0].text, "A")
                        self.assertIsNone(style.formatting["regions"][region]["background"])
            self.assertEqual(document.render_page(0, dpi=72), rendered)
            for region, side in AFFECTED:
                style.format(region=region, borders={side: None})
                style.format(region=region, borders=None)
                self.assertIsNone(style.formatting["regions"][region]["borders"])
            document.close()
            with self.assertRaises(DocumentClosedError):
                style.format(region="first_row", borders={"inside_vertical": {"style": "none"}})

    def test_safe_conditional_edges_keep_inheritance_reset_and_visible_output(self):
        with doc_ex.new("docx") as document:
            base = document.styles.add("Exterior borders", kind="table")
            base.format(region="first_row", borders={"top": {"color": "red", "width": 4}})
            base.format(region="first_column", borders={"left": {"color": "blue", "width": 4}})
            base.format(region="last_column", borders={"right": {"color": "green", "width": 4}})
            child = document.styles.add("Inherited borders", kind="table", based_on=base)
            table = document.add_table([["", "", ""], ["", "", ""]])
            table.format(style=child, look={"header_row": True, "first_column": True, "last_column": True})
            table.column_widths = (80, 80, 80)
            for row in range(2):
                table.row(row).height = (30, "exact")
            for counts in colored_pixels(document):
                self.assertTrue(all(counts[color] > 100 for color in ("red", "blue", "green")))
            child.format(region="first_row", borders={"top": {"style": "none"}})
            for counts in colored_pixels(document):
                self.assertEqual(counts["red"], 0)
                self.assertGreater(counts["blue"], 100)
            child.format(region="first_row", borders={"top": None})
            self.assertEqual(child.resolved_formatting["regions"]["first_row"]["borders"]["top"]["color"], "#FF0000")
            self.assertIsNone(child.formatting["regions"]["first_row"]["borders"])
            for counts in colored_pixels(document):
                self.assertGreater(counts["red"], 100)
            with document.copy() as copied:
                copied.styles[base.id].format(region="first_row", borders=None)
                self.assertIsNone(copied.styles[child.id].resolved_formatting["regions"]["first_row"]["borders"])
                self.assertEqual(base.formatting["regions"]["first_row"]["borders"]["top"]["color"], "#FF0000")
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                styles = ET.fromstring(archive.read("word/styles.xml"))
                node = styles.find(f"w:style[@w:styleId='{base.id}']/w:tblStylePr[@w:type='firstRow']/w:tcPr/w:tcBorders/w:top", NS)
                self.assertEqual(node.get(f"{{{NS['w']}}}color"), "FF0000")

    def test_direct_and_whole_table_edges_keep_existing_mapping_and_tuple(self):
        with doc_ex.new("docx") as document:
            style = document.styles.add("Grid", kind="table")
            style.format(borders={"inside_vertical": {"color": "red", "width": 4}})
            table = document.add_table([["", "", ""], ["", "", ""]])
            table.format(style=style)
            table.column_widths = (80, 80, 80)
            for row in range(2):
                table.row(row).height = (30, "exact")
            for counts in colored_pixels(document):
                self.assertGreater(counts["red"], 100)
            table.format(borders={"inside_vertical": {"color": "blue", "width": 4}})
            for counts in colored_pixels(document):
                self.assertEqual(counts["red"], 0)
                self.assertGreater(counts["blue"], 100)
            for row in range(2):
                for column in range(3):
                    table[row, column].format(borders={"left": {"style": "none"}, "right": {"style": "none"}})
            for counts in colored_pixels(document):
                self.assertEqual(counts["blue"], 0)
            for row in range(2):
                for column in range(3):
                    table[row, column].format(borders=None)
            table.format(borders=None)
            table.borders = ("green", 2)
            self.assertEqual(table.borders, ("#008000", 2))
            for counts in colored_pixels(document):
                self.assertEqual(counts["red"], 0)
                self.assertGreater(counts["green"], 100)
            table.borders = None
            self.assertIsNone(table.borders)
            table.format(borders=None)
            for counts in colored_pixels(document):
                self.assertGreater(counts["red"], 100)


if __name__ == "__main__":
    unittest.main()
