from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_rendering import has_visible_content, png_chunks
from test_styles import NS


class TabsAndBreaksTests(unittest.TestCase):
    def test_native_tabs_inherit_override_clear_and_roundtrip(self):
        with doc_ex.new("docx") as document:
            style = document.styles.add("Tab layout", based_on="Normal")
            style.format(tabs=[{"position": 144, "alignment": "right", "leader": "dot"}])
            paragraph = document.add_paragraph("café\t123.45", style=style)
            self.assertIsNone(paragraph.formatting["tabs"])
            self.assertEqual(paragraph.resolved_formatting["tabs"], [
                {"position": 144, "alignment": "right", "leader": "dot"}])
            paragraph.format(tabs=[{"position": 144, "alignment": "clear"},
                                   {"position": 180, "alignment": "center", "leader": "underscore"}])
            self.assertEqual([tab["position"] for tab in paragraph.resolved_formatting["tabs"]], [180])
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
                values = xml.findall(".//w:tabs/w:tab", NS)
                self.assertEqual([node.get(f"{{{NS['w']}}}val") for node in values], ["clear", "center"])
                self.assertEqual(values[1].get(f"{{{NS['w']}}}leader"), "underscore")
            for data in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    self.assertIn("café", reopened.to_markdown())
                    self.assertIn("123.45", reopened.to_markdown())
                    self.assertTrue(has_visible_content(png_chunks(reopened.render_page(0, dpi=72))))
            for reset in ([], None):
                paragraph.format(tabs=reset)
                self.assertIsNone(paragraph.formatting["tabs"])
                self.assertEqual(paragraph.resolved_formatting["tabs"], style.resolved_formatting["tabs"])
            table_style = document.styles.add("Cell tabs", kind="table")
            table_style.format(tabs=[{"position": 72}])
            table = document.add_table([["A\tB"]])
            table.format(style=table_style)
            self.assertEqual(table[0, 0].paragraphs[0].resolved_formatting["tabs"][0]["position"], 72)

    def test_page_column_and_line_breaks_use_native_layout(self):
        with doc_ex.new("docx", size=(400, 300)) as document:
            document.sections[0].margins = 30
            document.sections[0].columns = (2, 20, False)
            paragraph = document.add_paragraph("LEFT")
            marker = paragraph.add_break("column")
            self.assertEqual(marker.text, "\x0e")
            paragraph.add_run("RIGHT")
            with self.assertRaises(ValueError):
                _ = marker.text
            paragraph.add_break("column")
            paragraph.add_run("NEXT PAGE")
            line = paragraph.add_break()
            self.assertEqual(line.text, "\n")
            paragraph.add_run("NEXT LINE")
            for source in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(source) as reopened:
                    self.assertEqual(reopened.page_count, 2)
                    self.assertIn("LEFT", reopened[0].get_text())
                    self.assertIn("RIGHT", reopened[0].get_text())
                    self.assertIn("NEXT PAGE", reopened[1].get_text())
                    self.assertIn("NEXT LINE", reopened[1].get_text())
                    for page in reopened:
                        self.assertTrue(has_visible_content(png_chunks(page.get_pixmap(dpi=72))))
            with doc_ex.open(document.convert("pdf")) as reopened:
                left = next(block for block in reopened[0].blocks if block.text == "LEFT")
                right = next(block for block in reopened[0].blocks if block.text == "RIGHT")
                self.assertLess(left.bbox[0], 100)
                self.assertGreater(right.bbox[0], 190)
            paragraph.add_break("page")
            paragraph.add_run("FINAL PAGE")
            self.assertEqual(document.page_count, 3)
            with document.copy() as copied:
                copied.paragraphs[0].add_break("line")
                self.assertFalse(paragraph.text.endswith("\n"))

    def test_invalid_tabs_and_break_locations_leave_state_unchanged(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Keep")
            cell = document.add_table([["Cell"]])[0, 0].paragraphs[0]
            header = document.sections[0].header().add_paragraph("Header")
            style = document.styles.add("Table style", kind="table")
            before = document.to_bytes()
            for tabs in ([{"position": 72, "alignment": "bar"}],
                         [{"position": 72, "alignment": "decimal"}],
                         [{"position": 72, "alignment": "clear", "leader": "dot"}],
                         [{"position": float("nan")}], [{"position": True}],
                         [{"position": 72.01}], [{"position": 1584.05}],
                         [{"position": 72}, {"position": 72.25}],
                         [{"position": index} for index in range(65)],
                         [{"position": 72, "unknown": 1}]):
                with self.subTest(tabs=tabs), self.assertRaises((ValueError, TypeError)):
                    paragraph.format(tabs=tabs)
                self.assertEqual(document.to_bytes(), before)
            for content in (cell, header):
                for kind in ("page", "column"):
                    with self.assertRaises((ValueError, doc_ex.UnsupportedFormatError)):
                        content.add_break(kind)
                    self.assertEqual(document.to_bytes(), before)
            with self.assertRaises(ValueError):
                style.format(region="first_row", tabs=[{"position": 72}])
            with self.assertRaises(ValueError):
                paragraph.add_break("section")
            self.assertEqual(document.to_bytes(), before)
            cell.add_break("line")
            self.assertEqual(cell.text, "Cell\n")
            document.close()
            with self.assertRaises(DocumentClosedError):
                paragraph.add_break()

    def test_explicit_segments_follow_native_pages_after_automatic_wrapping(self):
        with doc_ex.new("docx", size=(400, 300)) as document:
            document.sections[0].margins = 30
            document.sections[0].columns = (2, 20, False)
            paragraph = document.add_paragraph("filler " * 200)
            paragraph.add_break("page")
            paragraph.add_run("AFTER PAGE").link = "https://example.com/page"
            paragraph.add_break("column")
            paragraph.add_run("AFTER COLUMN").link = "https://example.com/column"
            with doc_ex.open(document.to_bytes()) as word, doc_ex.open(document.convert("pdf")) as pdf:
                self.assertEqual(word.page_count, pdf.page_count)
                self.assertGreaterEqual(word.page_count, 3)
                for label, uri in (("AFTER PAGE", "https://example.com/page"),
                                   ("AFTER COLUMN", "https://example.com/column")):
                    expected = next(page.number for page in pdf if label in page.get_text())
                    self.assertEqual([page.number for page in word if label in page.get_text()], [expected])
                    link = next(link for page in word for link in page.get_links() if link.uri == uri)
                    self.assertEqual((link.page, link.text), (expected, label))
                    block = next(block for block in word[expected].blocks if label in block.text)
                    self.assertEqual(block.text[link.metadata["start"]:link.metadata["end"]], label)

    def test_leading_and_consecutive_breaks_keep_intentional_blank_pages(self):
        with doc_ex.new("docx", size=(400, 300)) as document:
            document.sections[0].margins = 30
            document.sections[0].columns = (2, 20, False)
            paragraph = document.add_paragraph("")
            paragraph.add_break("column")
            paragraph.add_break("column")
            paragraph.add_run("SECOND PAGE")
            paragraph.add_break("page")
            paragraph.add_break("page")
            paragraph.add_run("FOURTH PAGE")
            for source in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(source) as reopened:
                    self.assertEqual(reopened.page_count, 4)
                    self.assertEqual([page.get_text().strip() for page in reopened],
                                     ["", "SECOND PAGE", "", "FOURTH PAGE"])

    def test_effective_tab_limit_is_checked_across_inheritance(self):
        with doc_ex.new("docx") as document:
            style = document.styles.add("Many stops")
            style.format(tabs=[{"position": index * 2} for index in range(64)])
            paragraph = document.add_paragraph("Keep", style=style)
            before = document.to_bytes()
            with self.assertRaises(ValueError):
                paragraph.format(tabs=[{"position": 200}])
            self.assertEqual(document.to_bytes(), before)
            paragraph.format(tabs=[{"position": 0, "alignment": "clear"}, {"position": 200}])
            self.assertEqual(len(paragraph.resolved_formatting["tabs"]), 64)


if __name__ == "__main__":
    unittest.main()
