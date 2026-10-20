from collections import Counter
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex._authoring import Paragraph, TextRange
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"


class SlideCellParagraphTests(unittest.TestCase):
    def test_rich_paragraphs_ranges_links_saved_source_and_pdf_pixels(self):
        with doc_ex.new("pptx", size=(640, 360)) as deck:
            slide = deck.slides.add()
            destination = deck.slides.add()
            destination.add_text("Destination", box=(20, 20, 500, 50))
            table = slide.add_table([["λ same same", "Neighbor"]], box=(20, 20, 600, 120), font_size=18)
            first = table[0, 0].paragraphs[0]
            self.assertIsInstance(first, Paragraph)
            self.assertEqual([item.text for item in first.find("same")], ["same", "same"])
            first.find("same")[1].text = "other"
            self.assertEqual(first.replace_text("other", "same"), 1)
            first.add_run(" RED", font_size=24, color="red", bold=True)
            second = table[0, 0].add_paragraph()
            self.assertEqual(second.text, "")
            second.text = "Second"
            added = second.add_run(" BLUE", font_size=24, color="blue", italic=True)
            self.assertIsInstance(added, TextRange)
            second.alignment = "right"
            self.assertEqual(second.formatting, {"alignment": "right"})
            first.format(alignment="left")
            self.assertEqual(first.alignment, "left")
            with self.assertRaisesRegex(ValueError, "mixed"):
                _ = table[0, 0].alignment
            label = first.find("same")[0]
            label.format(underline="double", highlight=(255, 255, 0))
            label.link = "https://example.com/cell"
            label.link_tooltip = "Cell details"
            self.assertEqual(label.link_tooltip, "Cell details")
            label = second.find("Second")[0]
            label.link = destination
            label.link_tooltip = "Next slide"
            self.assertEqual(label.link.number, 1)
            deck.slides.move(1, 0)
            self.assertEqual(second.find("Second")[0].link.number, 0)
            source = deck.to_bytes()
            expected = "λ same same RED\nSecond BLUE"
            self.assertEqual(table.rows, [[expected, "Neighbor"]])
            with ZipFile(BytesIO(source)) as archive:
                root = ET.fromstring(archive.read("ppt/slides/slide2.xml"))
                body = root.find(f".//{A}tc/{A}txBody")
                self.assertEqual(len(body.findall(f"{A}p")), 2)
                self.assertEqual(body.findall(f"{A}p")[1].find(f"{A}pPr").get("algn"), "r")
                self.assertEqual(len(body.findall(f".//{A}hlinkClick")), 2)
            for kind, data in (("PPTX", source), ("PDF", deck.convert("pdf"))):
                with self.subTest(kind=kind), doc_ex.open(data) as reopened:
                    self.assertEqual(reopened.page_count, 2)
                    text = reopened[1].get_text()
                    for label in ("RED", "Second", "BLUE", "Neighbor"):
                        self.assertEqual(text.count(label), 1)
                    links = reopened[1].get_links()
                    self.assertTrue(any(link.uri == "https://example.com/cell" and link.text == "same" for link in links))
                    self.assertTrue(any(link.metadata.get("target_page") == 0 for link in links))
                    if kind == "PPTX":
                        self.assertEqual(reopened[1].find_tables()[0].rows, [[expected, "Neighbor"]])
                        link = next(link for link in links if link.uri == "https://example.com/cell")
                        self.assertEqual((link.metadata["start"], link.metadata["end"]), (2, 6))
                    _, pixels = rgba_pixels(png_chunks(reopened.render_page(1, dpi=72)))
                    counts = Counter(pixels[index:index + 4] for index in range(0, len(pixels), 4))
                    self.assertGreater(counts[b"\xff\0\0\xff"], 10)
                    self.assertGreater(counts[b"\0\0\xff\xff"], 10)
                    self.assertGreater(counts[b"\xff\xff\0\xff"], 10)

    def test_append_copy_aliases_grid_and_body_generations(self):
        with doc_ex.new("pptx") as deck:
            slide = deck.slides.add()
            table = slide.add_table([["A", "B"], ["C", "D"]], box=(20, 20, 500, 180))
            first = table[0, 0].paragraphs[0]
            old_range = first[:]
            appended = table[0, 0].add_paragraph("Tail")
            self.assertEqual(first.text, "A")
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = old_range.text
            table[1, 1].text = "Other cell reset"
            self.assertEqual(appended.text, "Tail")
            with deck.copy() as copied:
                copied_table = copied.slides[0].shapes[0]
                copied_table[0, 0].paragraphs[1].text = "Copy only"
                self.assertEqual(appended.text, "Tail")
                self.assertEqual(copied_table[0, 0].text, "A\nCopy only")
            table[0, 0].text = "Reset"
            for paragraph in (first, appended):
                with self.assertRaisesRegex(ValueError, "stale"):
                    _ = paragraph.text
            before_merge = table[0, 0].paragraphs[0]
            table.merge((0, 0), (0, 1))
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = before_merge.text
            anchor = table[0, 0].paragraphs[0]
            alias = table[0, 1].paragraphs[0]
            alias.text = "Alias edit"
            self.assertEqual(anchor.text, "Alias edit")
            table[0, 1].add_paragraph("Alias append")
            self.assertEqual(anchor.text, "Alias edit")
            table[0, 1].text = "Alias reset"
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = anchor.text
            paragraph = table[0, 0].paragraphs[0]
            table[0, 0].split()
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = paragraph.text
            paragraph = table[1, 1].paragraphs[0]
            table.insert_row(1)
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = paragraph.text
            paragraph = table[0, 0].paragraphs[0]
            deck.close()
            with self.assertRaises(DocumentClosedError):
                _ = paragraph.text

    def test_unsupported_word_operations_and_invalid_mutations_are_explicit_atomic(self):
        with doc_ex.new("pptx") as deck:
            table = deck.slides.add().add_table([["Safe"]], box=(20, 20, 400, 120))
            cell = table[0, 0]
            paragraph = cell.paragraphs[0]
            before = deck.to_bytes()
            preview = deck.render_page(0, dpi=72)
            operations = [lambda: paragraph.style, lambda: setattr(paragraph, "style", "Normal"),
                          lambda: paragraph.list_style, lambda: paragraph.set_list("bullet"),
                          lambda: paragraph.page_break_before, lambda: setattr(paragraph, "page_break_before", True),
                          lambda: paragraph.spacing, lambda: setattr(paragraph, "spacing", (0, 0, 1)),
                          lambda: paragraph.resolved_formatting, lambda: paragraph.add_field("page"),
                          lambda: paragraph.add_break(), lambda: paragraph.format(space_after=12),
                          lambda: cell.add_paragraph("No", style="Normal"),
                          lambda: len(cell.tables), lambda: len(cell.images)]
            for operation in operations:
                with self.subTest(operation=operation), self.assertRaises(UnsupportedFormatError):
                    operation()
                self.assertEqual(deck.to_bytes(), before)
            for operation in (lambda: setattr(paragraph, "text", "A\nB"),
                              lambda: cell.add_paragraph("A\nB"), lambda: paragraph.add_run("A\nB"),
                              lambda: paragraph.replace_text("Safe", "A\nB"),
                              lambda: paragraph[:].format(font_size=-1),
                              lambda: setattr(paragraph, "alignment", "distributed")):
                with self.assertRaises(ValueError):
                    operation()
                self.assertEqual(deck.to_bytes(), before)
                self.assertEqual(deck.render_page(0, dpi=72), preview)
            with self.assertRaises(UnsafeArchiveError):
                paragraph.add_run("x" * (16 * 1024 * 1024))
            self.assertEqual(deck.to_bytes(), before)
            self.assertEqual(paragraph.text, "Safe")

    def test_callbacks_recheck_stable_slide_and_cell_ownership(self):
        with doc_ex.new("pptx") as deck:
            first = deck.slides.add()
            deck.slides.add()
            table = first.add_table([["Original"]], box=(20, 20, 400, 120))
            cell = table[0, 0]
            paragraph = cell.paragraphs[0]

            class Reorder(str):
                def lower(self):
                    deck.slides.move(0, 1)
                    return super().lower()

            paragraph.add_run(" appended", color=Reorder("red"))
            self.assertEqual(first.number, 1)
            self.assertEqual(paragraph.text, "Original appended")

            class Reset(str):
                def lower(self):
                    cell.text = "Callback reset"
                    return super().lower()

            with self.assertRaisesRegex(ValueError, "stale"):
                paragraph.add_run(" must not appear", color=Reset("red"))
            self.assertEqual(cell.text, "Callback reset")
            paragraph = cell.paragraphs[0]

            class ResetIndex:
                def __index__(self):
                    cell.text = "Callback selection reset"
                    return 0

            with self.assertRaisesRegex(ValueError, "stale|changed"):
                paragraph[ResetIndex():]
            self.assertEqual(cell.text, "Callback selection reset")

    def test_inserted_split_and_reset_empty_cells_keep_exact_paragraph_ordinals(self):
        with doc_ex.new("pptx") as deck:
            table = deck.slides.add().add_table([["A", "B"]], box=(20, 20, 400, 80))
            table.insert_row(1)
            self.assertEqual([p.text for p in table[1, 0].paragraphs], [""])
            self.assertEqual(table[1, 0].add_paragraph("New row").text, "New row")
            self.assertEqual([p.text for p in table[1, 0].paragraphs], ["", "New row"])
            table.insert_column(2)
            self.assertEqual(table[0, 2].add_paragraph("New column").text, "New column")
            self.assertEqual([p.text for p in table[0, 2].paragraphs], ["", "New column"])
            table.merge((0, 0), (0, 1))
            table[0, 0].split()
            self.assertEqual(len(table[0, 1].paragraphs), 0)
            paragraph = table[0, 1].add_paragraph("First after split")
            self.assertEqual(paragraph.text, "First after split")
            self.assertEqual([p.text for p in table[0, 1].paragraphs], ["First after split"])
            table[0, 1].text = ""
            self.assertEqual(table[0, 1].add_paragraph("After reset").text, "After reset")
            self.assertEqual([p.text for p in table[0, 1].paragraphs], ["", "After reset"])
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = paragraph.text

    def test_empty_paragraphs_and_native_row_growth_without_nominal_geometry_rewrite(self):
        with doc_ex.new("pptx", size=(400, 300)) as deck:
            table = deck.slides.add().add_table([[""]], box=(20, 20, 300, 20), font_size=18)
            initial_box = table.box
            first = table[0, 0].paragraphs[0]
            first[:].text = "First line"
            second = table[0, 0].add_paragraph("Second line")
            second[:].format(color="red", font_size=24)
            empty = table[0, 0].add_paragraph()
            self.assertEqual([p.text for p in table[0, 0].paragraphs], ["First line", "Second line", ""])
            self.assertEqual(empty.text, "")
            self.assertEqual(table.box, initial_box)
            for data in (deck.to_bytes(), deck.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    self.assertEqual(reopened[0].get_text().count("Second line"), 1)
                    width, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                    # Native row fitting draws the second paragraph below the nominal 20pt row.
                    below = pixels[40 * width * 4:]
                    self.assertGreater(sum(below[i:i + 4] == b"\xff\0\0\xff"
                                           for i in range(0, len(below), 4)), 10)


if __name__ == "__main__":
    unittest.main()
