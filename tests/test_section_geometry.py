from io import BytesIO
import math
from pathlib import Path
import struct
from tempfile import TemporaryDirectory
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_authoring import image_fixture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def colored_bounds(png, channel):
    width, pixels = rgba_pixels(png_chunks(png))
    points = [(index % width, index // width)
              for index in range(len(pixels) // 4)
              if pixels[4 * index + channel] > 200
              and pixels[4 * index + (channel + 1) % 3] < 100
              and pixels[4 * index + (channel + 2) % 3] < 100]
    if not points:
        raise AssertionError("expected visible colored text")
    return (min(x for x, _ in points), min(y for _, y in points),
            max(x for x, _ in points), max(y for _, y in points))


def separator_ink(png):
    width, pixels = rgba_pixels(png_chunks(png))
    # At 144 DPI the gap center is x=340; sample below all text lines.
    return sum(max(pixels[(y * width + x) * 4:(y * width + x) * 4 + 3]) < 230
               for y in range(100, 500) for x in range(339, 342))


class SectionGeometryTests(unittest.TestCase):
    def test_unequal_columns_gutter_break_order_and_native_pixels(self):
        with doc_ex.new("docx", size=(440, 300)) as document:
            section = document.sections[0]
            section.margins = 20
            section.gutter = 20
            section.columns = (2, 20, True)
            section.column_widths = (120, 240)
            paragraph = document.add_paragraph("")
            paragraph.add_run("LEFT", color="red", font_size=18)
            paragraph.add_break("column")
            paragraph.add_run("RIGHT", color="#00FF00", font_size=18)
            paragraph.add_break("column")
            paragraph.add_run("NEXT", color="blue", font_size=18)
            self.assertEqual(section.columns, (2, 20, True))
            self.assertEqual(section.column_widths, (120, 240))
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            w = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
            props = xml.find(f".//{w}sectPr")
            self.assertEqual(props.find(f"{w}pgMar").attrib[f"{w}gutter"], "400")
            columns = props.find(f"{w}cols")
            self.assertEqual(columns.attrib[f"{w}equalWidth"], "0")
            self.assertEqual(columns.attrib[f"{w}sep"], "1")
            self.assertEqual([(int(col.attrib[f"{w}w"]), int(col.attrib[f"{w}space"]))
                              for col in columns], [(2400, 400), (4800, 0)])
            for output in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(output) as reopened:
                    self.assertEqual(reopened.page_count, 2)
                    self.assertIn("LEFT", reopened[0].get_text())
                    self.assertIn("RIGHT", reopened[0].get_text())
                    self.assertNotIn("NEXT", reopened[0].get_text())
                    self.assertIn("NEXT", reopened[1].get_text())
                    png = reopened.render_page(0, dpi=144)
                    self.assertEqual(struct.unpack(">II", png_chunks(png)[b"IHDR"][:8]), (880, 600))
                    left, right = colored_bounds(png, 0), colored_bounds(png, 1)
                    self.assertGreaterEqual(left[0], 80)
                    self.assertLess(left[0], 96)
                    self.assertLess(left[2], 320)
                    self.assertGreaterEqual(right[0], 360)
                    self.assertLess(right[0], 376)
                    self.assertLess(right[2], 840)
                    self.assertGreater(separator_ink(png), 300)
                    following = colored_bounds(reopened.render_page(1, dpi=144), 2)
                    self.assertGreaterEqual(following[0], 80)
                    self.assertLess(following[0], 96)
            section.columns = (2, 20, False)
            section.column_widths = (120, 240)
            for output in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(output) as reopened:
                    self.assertEqual(separator_ink(reopened.render_page(0, dpi=144)), 0)

    def test_gutter_repositions_warmed_render_and_copy_retains_its_geometry(self):
        with doc_ex.new("docx", size=(440, 300)) as document:
            section = document.sections[0]
            section.margins = 20
            self.assertEqual(section.gutter, 0)
            self.assertIsNone(section.column_widths)
            document.add_paragraph("").add_run("Marker", color="red", font_size=18)
            before = document.render_page(0, dpi=144)
            with document.copy() as copied:
                section.gutter = 20
                after = document.render_page(0, dpi=144)
                self.assertAlmostEqual(colored_bounds(after, 0)[0] - colored_bounds(before, 0)[0], 40, delta=1)
                self.assertEqual(copied.render_page(0, dpi=144), before)
                self.assertEqual(copied.sections[0].gutter, 0)
                section.columns = (2, 20, False)
                section.column_widths = (120, 240)
                section.column_widths = None
                self.assertIsNone(section.column_widths)
                self.assertEqual(section.columns, (2, 20, False))
                section.column_widths = (120, 240)
                section.columns = (1, 0, False)
                self.assertIsNone(section.column_widths)
                document.close()
                with self.assertRaises(DocumentClosedError):
                    _ = section.gutter
                with self.assertRaises(DocumentClosedError):
                    _ = section.column_widths

    def test_invalid_geometry_and_narrow_column_child_overflow_roll_back(self):
        with doc_ex.new("docx", size=(440, 300)) as document:
            section = document.sections[0]
            section.margins = 20
            section.columns = (2, 20, False)
            section.column_widths = (120, 260)
            table = document.add_table([["Outer"]])
            nested = table[0, 0].add_table([["Nested"]])
            picture = document.add_image(image_fixture(), width=100)
            self.assertAlmostEqual(sum(table.column_widths), 120)
            before = document.to_bytes()
            for name, value in [
                ("gutter", -1), ("gutter", 1), ("gutter", True), ("gutter", math.inf),
                ("column_widths", []), ("column_widths", [120]), ("column_widths", [17, 260]),
                ("column_widths", [120, 261]), ("column_widths", [True, 260]),
                ("column_widths", [math.nan, 260]), ("column_widths", [80, 260]),
            ]:
                with self.subTest(property=name, value=value):
                    with self.assertRaises((TypeError, ValueError, doc_ex.DocExError)):
                        setattr(section, name, value)
                    self.assertEqual(document.to_bytes(), before)
            for operation in (lambda: setattr(table, "column_widths", [121]),
                              lambda: setattr(nested, "column_widths", [120]),
                              lambda: setattr(picture, "size", (121, 20))):
                with self.assertRaises((ValueError, doc_ex.DocExError)):
                    operation()
                self.assertEqual(document.to_bytes(), before)

    def test_spanning_tables_fit_narrowest_column_and_keep_rows_once(self):
        with doc_ex.new("docx", size=(440, 200)) as document:
            section = document.sections[0]
            section.margins = 20
            section.columns = (2, 20, False)
            section.column_widths = (260, 120)
            labels = [f"Entry{index:02d}" for index in range(20)]
            table = document.add_table([[label] for label in labels])
            self.assertEqual(table.column_widths, [120])
            for index in range(len(labels)):
                table.row(index).height = (20, "at_least")
            for output in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(output) as reopened:
                    self.assertGreaterEqual(reopened.page_count, 2)
                    text = reopened.to_markdown()
                    for label in labels:
                        self.assertEqual(text.count(label), 1)
                    for index in range(reopened.page_count):
                        self.assertTrue(reopened.render_page(index, dpi=72).startswith(b"\x89PNG"))

    def test_autofit_relocation_fails_before_publishing_output_and_recovers(self):
        with TemporaryDirectory() as directory, doc_ex.new("docx", size=(440, 300)) as document:
            section = document.sections[0]
            section.margins = 20
            section.columns = (2, 20, False)
            section.column_widths = (260, 120)
            before = document.add_paragraph("")
            before.add_break("column")
            content = "WideContent" * 5
            table = document.add_table([[content]])
            self.assertEqual(table.formatting["layout"], "fixed")
            self.assertEqual(table.formatting["width"], 120)
            table.format(layout="autofit", width=None)
            document.render_page(0, dpi=72)
            destination = Path(directory) / "kept.docx"
            document.save(destination)
            saved = destination.read_bytes()
            before.text = "Moved before table"
            for operation in (document.to_bytes, lambda: document.render_page(0, dpi=72),
                              lambda: document.convert("pdf"), lambda: document.save(destination)):
                with self.assertRaises((ValueError, doc_ex.DocExError)):
                    operation()
                self.assertEqual(destination.read_bytes(), saved)
            table.column_widths = [120]
            document.save(destination)
            with doc_ex.open(destination) as reopened:
                self.assertIn(content, reopened.to_markdown())
                self.assertIn("Moved before table", reopened.to_markdown())
                self.assertTrue(reopened.render_page(0, dpi=72).startswith(b"\x89PNG"))


if __name__ == "__main__":
    unittest.main()
