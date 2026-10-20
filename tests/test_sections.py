import struct
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_rendering import png_chunks


class SectionTests(unittest.TestCase):
    def test_native_sections_have_independent_geometry_and_headers(self):
        with doc_ex.new("docx", size=(360, 480)) as document:
            first = document.sections[0]
            first.margins = 32
            first.header_distance = 18
            first.footer_distance = 18
            first.header().add_paragraph("First header")
            document.add_paragraph("First body")
            second = document.sections.add()
            second.page_size = (600, 400)
            second.margins = 40
            second.columns = (2, 18, True)
            second.header().paragraphs[0].text = "Second header"
            document.add_paragraph("Second body")
            self.assertEqual(len(document.sections), 2)
            self.assertEqual(first.page_size, (360, 480))
            self.assertEqual(second.columns, (2, 18, True))
            self.assertEqual(first.header().paragraphs[0].text, "First header")
            self.assertEqual([paragraph.text for paragraph in document.paragraphs], ["First body", "Second body"])
            for source in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(source) as reopened:
                    self.assertEqual(len(reopened), 2)
                    for index, dimensions in enumerate(((360, 480), (600, 400))):
                        png = png_chunks(reopened.render_page(index, dpi=72))
                        self.assertEqual(struct.unpack(">II", png[b"IHDR"][:8]), dimensions)
                    self.assertIn("First body", reopened[0].get_text())
                    self.assertIn("Second body", reopened[1].get_text())
            with doc_ex.open(document.convert("pdf")) as pdf:
                self.assertIn("First header", pdf[0].get_text())
                self.assertNotIn("Second header", pdf[0].get_text())
                self.assertIn("Second header", pdf[1].get_text())
                self.assertNotIn("First header", pdf[1].get_text())

    def test_absent_stories_do_not_begin_inheriting_after_append(self):
        with doc_ex.new("docx") as document:
            first = document.sections[0]
            document.add_paragraph("One")
            second = document.sections.add()
            document.add_paragraph("Two")
            first.footer().add_paragraph("Only first")
            self.assertEqual(len(second.footer().paragraphs), 0)
            second.header("first").add_paragraph("Only second")
            self.assertEqual(len(first.header("first").paragraphs), 0)
            with document.copy() as copy:
                copy.sections[0].margins = 20
                self.assertNotEqual(first.margins, copy.sections[0].margins)
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = second.page_size

    def test_invalid_geometry_and_unsupported_breaks_are_atomic(self):
        with doc_ex.new("docx") as document:
            section = document.sections[0]
            before = section.page_size, section.columns, section.margins
            for operation in (
                lambda: setattr(section, "columns", (46, 10, False)),
                lambda: setattr(section, "columns", (2, 10000, False)),
                lambda: setattr(section, "margins", 10000),
                lambda: document.sections.add(start="next_column"),
            ):
                with self.assertRaises((ValueError, doc_ex.DocExError)):
                    operation()
                self.assertEqual((section.page_size, section.columns, section.margins), before)
                self.assertEqual(len(document.sections), 1)
            document.add_paragraph("First")
            second = document.sections.add(start="continuous")
            document.add_paragraph("Second")
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                second.page_size = (400, 400)
            self.assertEqual(second.page_size, section.page_size)
            self.assertEqual(second.start, "continuous")
