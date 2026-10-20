from __future__ import annotations

import io
import math
import struct
import tempfile
import unittest
import zipfile
import zlib
from pathlib import Path
from unittest.mock import patch
from xml.etree import ElementTree as ET

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from fixtures import docx_fixture, pdf_fixture
from test_rendering import png_chunks


def image_fixture(width=32, height=16):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    raw = (b"\x00" + b"\x20\x60\xC0\xFF" * width) * height
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


class AuthoringTests(unittest.TestCase):
    def test_docx_create_style_table_and_conversion(self):
        with doc_ex.new("docx") as document:
            self.assertEqual(len(document.paragraphs), 0)
            document.sections[0].page_size = "A4"
            self.assertAlmostEqual(document.sections[0].page_size[0], doc_ex.mm(210), delta=0.1)
            document.background = "#F4F6FA"
            self.assertEqual(document.background, "#F4F6FA")
            heading = document.add_paragraph("Quarterly report", style="Heading 1")
            heading.alignment = "center"
            paragraph = document.add_paragraph("Prepared for ")
            run = paragraph.add_run("Café", bold=True, size=14, color="navy")
            self.assertEqual(run.text, "Café")
            table = document.add_table([["Item", "Total"], ["Support", "$200"]])
            table[-1, -1].text = "$250"
            self.assertEqual(table.rows, [["Item", "Total"], ["Support", "$250"]])
            self.assertEqual(document.paragraphs[-1].text, "Prepared for Café")
            data = document.to_bytes()
            with doc_ex.open(data) as reopened:
                self.assertEqual(reopened[0].blocks[0].kind, "heading")
                self.assertEqual(reopened[0].find_tables()[0].rows, table.rows)
                self.assertEqual(reopened.to_markdown().count("Support"), 1)
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = "\n".join(page.get_text() for page in pdf)
                self.assertIn("Quarterly report", text)
                self.assertEqual(text.count("$250"), 1)
            self.assertIn(b"pHYs", png_chunks(document.render_page(0, dpi=72)))
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
                ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
                runs = xml.findall(".//w:r", ns)
                cafe = next(item for item in runs if "Café" in "".join(item.itertext()))
                self.assertIsNotNone(cafe.find("w:rPr/w:b", ns))
                self.assertEqual(cafe.find("w:rPr/w:color", ns).get("{" + ns["w"] + "}val"), "000080")

    def test_docx_copy_ranges_and_explicit_empty_paragraphs(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("λ ")
            paragraph.add_run("{{cus", bold=True)
            paragraph.add_run("tomer}}", italic=True)
            stale = paragraph.find("customer")[0]
            self.assertEqual(paragraph.replace_text("{{customer}}", "Acme"), 1)
            self.assertEqual(paragraph.text, "λ Acme")
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = stale.text
            replacement = paragraph.find("Acme")[0]
            replacement.format(color="#112233")
            replacement.text = "New client"
            self.assertEqual(paragraph.text, "λ New client")
            document.add_paragraph("")
            document.add_paragraph("Last")
            self.assertEqual([p.text for p in document.paragraphs], ["λ New client", "", "Last"])
            copy = document.copy()
            copy.paragraphs[0].text = "Copy only"
            self.assertEqual(paragraph.text, "λ New client")
        with self.assertRaises(DocumentClosedError):
            _ = paragraph.text
        with copy:
            self.assertEqual(copy.paragraphs[0].text, "Copy only")
            self.assertIn("Copy only", copy.to_markdown())

    def test_invalid_docx_edits_are_atomic(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Original")
            for invalid in ("bad\x00", "bad\ufffc"):
                with self.assertRaises(ValueError):
                    paragraph.text = invalid
                self.assertEqual(paragraph.text, "Original")
            with self.assertRaises(ValueError):
                document.add_paragraph("Must not appear", style="Unknown style")
            self.assertEqual([p.text for p in document.paragraphs], ["Original"])
            with self.assertRaises(ValueError):
                document.add_table([["a", "b"], ["c"]])
            with self.assertRaises(TypeError):
                document.add_table([[10]])
            self.assertEqual(len(document.tables), 0)
            with self.assertRaises(ValueError):
                document.sections[0].page_size = (10, 10)
            self.assertEqual(document.sections[0].page_size, (612, 792))

    def test_docx_table_first_has_no_leading_blank_paragraph(self):
        with doc_ex.new("docx") as document:
            table = document.add_table([["Only"]])
            self.assertEqual(len(document.paragraphs), 0)
            paragraph = document.add_paragraph("After")
            table[0, 0].text = "Before"
            self.assertEqual(paragraph.text, "After")
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened[0].get_text(), "Before\nAfter")

    def test_docx_margins_images_merges_and_header_stories(self):
        with doc_ex.new("docx") as document:
            section = document.sections[0]
            section.margins = doc_ex.mm(20)
            self.assertTrue(all(abs(value - doc_ex.mm(20)) < .1 for value in section.margins))
            header = section.header().add_paragraph("Header ")
            header.add_run("brand", bold=True)
            section.footer().add_paragraph("Footer")
            self.assertEqual(section.header().paragraphs[-1].text, "Header brand")
            image = document.add_image(image_fixture(), width=64, alt="Logo")
            self.assertEqual(image.size, (64, 32))
            image.size = (32, 16)
            image.alt = "New logo"
            self.assertEqual(document.images[0].alt, "New logo")
            table = document.add_table([["A", "B"], ["C", "D"]])
            table[0, 0].background = "#E0F0FF"
            table[0, 0].vertical_alignment = "center"
            self.assertEqual(table[0, 0].background, "#E0F0FF")
            self.assertEqual(table[0, 0].vertical_alignment, "center")
            merged = table.merge((0, 0), (0, 1))
            self.assertEqual(table[0, 1].text, merged.text)
            self.assertEqual(table.rows[0][1], "")
            merged.split(columns=2)
            self.assertEqual(len(table.rows[0]), 2)
            with document.copy() as copy:
                copy.sections[0].header().paragraphs[-1].text = "Copy header"
                self.assertEqual(header.text, "Header brand")
            with doc_ex.open(document.convert("pdf")) as rendered:
                text = "\n".join(page.get_text() for page in rendered)
                self.assertIn("Header brand", text)
                self.assertIn("Footer", text)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened[0].get_images()[0].alt, "New logo")

    def test_pptx_slide_identity_metadata_and_cell_style(self):
        with doc_ex.new("pptx") as document:
            first = document.slides.add()
            first.name = "Overview"
            first.notes = "Presenter notes"
            first.hidden = True
            shape = first.add_text("First", box=(20, 20, 200, 50))
            table = first.add_table([["Cell"]], box=(20, 80, 200, 50))
            table[0, 0].background = "#E0F0FF"
            table[0, 0].vertical_alignment = "center"
            second = document.slides.add()
            second.add_text("Second", box=(20, 20, 200, 50))
            document.slides.move(0, 1)
            self.assertEqual(first.number, 1)
            self.assertEqual((first.name, first.notes, first.hidden), ("Overview", "Presenter notes", True))
            shape.text = "First updated"
            table[0, 0].text = "Updated cell"
            self.assertEqual(table[0, 0].background, "#E0F0FF")
            self.assertEqual(table[0, 0].vertical_alignment, "center")
            self.assertIn("First updated", document.slides[1].get_text())
            self.assertEqual(table.rows, [["Updated cell"]])
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn("Presenter notes", reopened[1].get_text())
            del document.slides[0]
            self.assertEqual(first.number, 0)
            with self.assertRaises((ValueError, IndexError)):
                second.get_text()
            del document.slides[0]
            with self.assertRaises((ValueError, IndexError)):
                _ = shape.text

    def test_docx_rich_cell_content_and_stale_children(self):
        with doc_ex.new("docx") as document:
            cell = document.add_table([["Original"]])[0, 0]
            paragraph = cell.add_paragraph("Rich ")
            paragraph.add_run("bold", bold=True)
            nested = cell.add_table([["Nested", "Value"]])
            picture = cell.add_image(image_fixture(), width=32, alt="Cell image")
            self.assertEqual(cell.paragraphs[-1].text, "Rich bold")
            self.assertEqual(cell.tables[0].rows, [["Nested", "Value"]])
            self.assertEqual(cell.images[0].alt, "Cell image")
            with doc_ex.open(document.to_bytes()) as reopened:
                markdown = reopened.to_markdown()
                self.assertEqual(markdown.count("Nested"), 1)
                self.assertEqual(markdown.count("Cell image"), 1)
                self.assertIn("Rich bold", reopened[0].get_text())
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = "\n".join(page.get_text() for page in pdf)
                self.assertEqual(text.count("Nested"), 1)
            cell.text = "Replacement"
            for operation in (lambda: paragraph.text, lambda: nested.rows, lambda: picture.alt):
                with self.assertRaises((ValueError, IndexError)):
                    operation()
            self.assertEqual(cell.paragraphs[0].text, "Replacement")

    def test_pptx_shapes_stacking_background_and_deletion(self):
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            slide.background_image = image_fixture()
            self.assertEqual(slide.background_image, image_fixture())
            with self.assertRaises(UnsupportedFormatError):
                _ = slide.background
            rectangle = slide.add_shape("rectangle", box=(20, 20, 100, 60), fill="red", stroke="blue", stroke_width=2)
            ellipse = slide.add_shape("ellipse", box=(60, 40, 100, 60), fill="green")
            line = slide.add_shape("line", box=(20, 150, 200, 0), stroke="navy")
            rectangle.rotation = 30
            rectangle.fill = "#E0F0FF"
            rectangle.stroke = ("black", 3)
            self.assertEqual((rectangle.rotation, rectangle.fill, rectangle.stroke), (30, "#E0F0FF", ("#000000", 3)))
            slide.shapes.move(0, 2)
            self.assertEqual(slide.shapes[-1].fill, "#E0F0FF")
            self.assertEqual(line.box, (20, 150, 200, 0))
            del slide.shapes[0]
            with self.assertRaises((ValueError, IndexError)):
                _ = ellipse.box
            self.assertEqual(len(slide.shapes), 2)
            self.assertIn(b"pHYs", png_chunks(slide.get_pixmap(dpi=72)))
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened.page_count, 1)
            slide.background = "white"
            self.assertIsNone(slide.background_image)
            with zipfile.ZipFile(io.BytesIO(document.to_bytes())) as archive:
                self.assertFalse(any(name.startswith("ppt/media/") for name in archive.namelist()))

    def test_pdf_annotations_links_and_stale_handles(self):
        with doc_ex.new("pdf", size=(300, 400)) as document:
            page = document.pages[0]
            page.draw_text("Read more", at=(30, 40))
            uri = page.annotations.add_link(box=(30, 40, 80, 20), uri="https://example.com")
            note = page.annotations.add("note", box=(30, 90, 20, 20), text="Review λ", author="Reviewer")
            highlight = page.annotations.add("highlight", box=(30, 40, 80, 20), text="Marked")
            self.assertEqual((uri.uri, note.text, note.author, highlight.kind), ("https://example.com", "Review λ", "Reviewer", "highlight"))
            note.text = "Updated λ"
            note.color = "blue"
            self.assertEqual(note.color, "#0000FF")
            note.box = (50, 90, 20, 20)
            with self.assertRaises(UnsupportedFormatError):
                highlight.box = (30, 40, 90, 20)
            for actual, expected in zip(highlight.box, (30, 40, 80, 20)):
                self.assertAlmostEqual(actual, expected, places=4)
            uri.target_page = 0
            self.assertEqual(uri.target_page, 0)
            self.assertIsNone(uri.uri)
            with doc_ex.open(document.to_bytes()) as reopened:
                link = reopened[0].get_links()[0]
                self.assertEqual((link.uri, link.metadata["target_page"]), ("#page=1", 0))
                with reopened.edit() as edited:
                    self.assertEqual([a.kind for a in edited.pages[0].annotations], ["link", "note", "highlight"])
                    self.assertEqual(edited.pages[0].annotations[1].text, "Updated λ")
            del document.pages[0].annotations[1]
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = uri.uri
            self.assertEqual(len(page.annotations), 2)
            page.rotate(90)
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = highlight.text

    def test_docx_table_and_paragraph_layout_controls(self):
        with doc_ex.new("docx") as document:
            heading = document.add_paragraph("First page")
            heading.spacing = (6, 12, 1.5)
            self.assertEqual(heading.spacing, (6, 12, 1.5))
            table = document.add_table([["Header", "Value"], ["Row", "Data"]])
            table.column_widths = [180, 220]
            table.borders = ("navy", 0.5)
            table.row(0).repeat_header = True
            table.row(1).height = (30, "at_least")
            table.row(1).allow_split = False
            self.assertEqual(table.column_widths, [180, 220])
            self.assertEqual(table.borders, ("#000080", 0.5))
            self.assertEqual(table.row(1).height, (30, "at_least"))
            self.assertFalse(table.row(1).allow_split)
            self.assertTrue(table.row(0).repeat_header)
            with self.assertRaises(ValueError):
                table.column_widths = [1000, 1000]
            self.assertEqual(table.column_widths, [180, 220])
            document.add_page_break()
            document.add_paragraph("Second page")
            third = document.add_paragraph("Third page")
            third.page_break_before = True
            self.assertTrue(third.page_break_before)
            with doc_ex.open(document.convert("pdf")) as pdf:
                self.assertEqual(pdf.page_count, 3)
                self.assertIn("First page", pdf[0].get_text())
                self.assertIn("Second page", pdf[1].get_text())
                self.assertIn("Third page", pdf[2].get_text())
            table.borders = None
            self.assertIsNone(table.borders)

    def test_safe_path_stream_save_and_close(self):
        with tempfile.TemporaryDirectory() as directory, doc_ex.new("docx") as document:
            document.add_paragraph("Persisted")
            path = Path(directory) / "report.docx"
            path.write_bytes(b"previous")
            with patch("doc_ex._authoring.os.replace", side_effect=OSError("simulated disk failure")):
                with self.assertRaises(OSError):
                    document.save(path)
            self.assertEqual(path.read_bytes(), b"previous")
            self.assertEqual(list(Path(directory).iterdir()), [path])
            document.save(path)
            with doc_ex.open(path) as reopened:
                self.assertEqual(reopened[0].get_text(), "Persisted")
            output = io.BytesIO()
            self.assertIsNone(document.save(output))
            with doc_ex.open(output.getvalue()) as reopened:
                self.assertEqual(reopened[0].get_text(), "Persisted")
            with self.assertRaises(ValueError):
                document.save(Path(directory) / "wrong.pdf")
            class ShortWriter:
                def write(self, data):
                    return 1
            with self.assertRaises(OSError):
                document.save(ShortWriter())
        document.close()
        self.assertTrue(document.closed)
        for operation in (document.to_bytes, document.copy):
            with self.assertRaises(DocumentClosedError):
                operation()

    def test_pdf_creation_pages_images_and_copy(self):
        with doc_ex.new("pdf", size=(300, 400)) as document:
            self.assertEqual(len(document.pages), 1)
            first = document.pages[0]
            first.background = "#FAFAF7"
            first.draw_text("Invoice", at=(30, 40), font_size=18)
            first.add_image(image_fixture(), box=(30, 80, 64, 32))
            second = document.pages.add(size=(400, 300))
            second.draw_text("Second", at=(20, 30))
            self.assertEqual(first.background, "#FAFAF7")
            self.assertIn("Invoice", first.get_text())
            self.assertIn(b"pHYs", png_chunks(first.get_pixmap(dpi=72)))
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened.page_count, 2)
                self.assertEqual(len(reopened[0].get_images()), 1)
            with document.copy(pages=[1, 0, 1]) as subset:
                self.assertEqual(subset.page_count, 3)
                self.assertIn("Second", subset.pages[0].get_text())
                self.assertIn("Invoice", subset.pages[1].get_text())
                subset.pages[0].draw_text("Clone", at=(20, 60))
                self.assertNotIn("Clone", second.get_text())

    def test_pdf_organization_invalidates_handles_and_is_atomic(self):
        with doc_ex.new("pdf", size=(300, 400)) as document:
            first = document.pages[0]
            first.draw_text("First", at=(30, 30))
            document.pages.add(size=(400, 300)).draw_text("Second", at=(30, 30))
            with self.assertRaises(IndexError):
                document.pages.move(0, 2)
            self.assertIn("First", first.get_text())
            document.pages.move(0, 1)
            with self.assertRaisesRegex(ValueError, "stale"):
                first.get_text()
            self.assertIn("Second", document.pages[0].get_text())
            page = document.pages[-1]
            page.rotate(90)
            self.assertEqual(page.size, (400, 300))
            self.assertIn("First", page.get_text())
            del document.pages[0]
            self.assertEqual(document.page_count, 1)
            with self.assertRaises(ValueError):
                del document.pages[0]
            self.assertEqual(document.page_count, 1)

    def test_pdf_edit_and_import_own_original_data(self):
        with doc_ex.open(pdf_fixture()) as source:
            edited = source.edit()
        with edited:
            edited.pages[0].draw_text("Added", at=(36, 48))
            self.assertIn("Page one", edited.pages[0].get_text())
            self.assertIn("Added", edited.pages[0].get_text())
            with doc_ex.new("pdf") as destination:
                imported = destination.pages.insert(1, edited, pages=[1, 0])
                self.assertEqual(destination.page_count, 3)
                self.assertIn("Added", imported[1].get_text())
                with self.assertRaises(IndexError):
                    destination.pages.insert(0, edited, pages=[99])
                self.assertEqual(destination.page_count, 3)
        with doc_ex.open(docx_fixture()) as source:
            with self.assertRaises(UnsupportedFormatError):
                source.edit()

    def test_pptx_creation_typed_shapes_tables_images_and_conversion(self):
        with doc_ex.new("pptx", size=(720, 540)) as document:
            self.assertEqual(document.page_count, 0)
            with self.assertRaises(ValueError):
                document.to_bytes()
            slide = document.slides.add()
            slide.background = "#E0F0FF"
            self.assertEqual(slide.background, "#E0F0FF")
            text = slide.add_text("Café report", box=(36, 36, 600, 80), font_size=24)
            text.color = "navy"
            text.bold = True
            text.font_size = 28
            text.text = "Updated report"
            self.assertEqual((text.text, text.color, text.font_size, text.bold), ("Updated report", "#000080", 28, True))
            table = slide.add_table([["Name", "Score"], ["Alice", "91"]], box=(36, 140, 600, 180))
            table[1, 1].text = "99"
            table.box = (36, 150, 600, 180)
            self.assertEqual(table.rows[1], ["Alice", "99"])
            picture = slide.add_image(image_fixture(), box=(36, 400, 64, 32), alt="Blue sample")
            self.assertEqual(picture.box, (36, 400, 64, 32))
            self.assertIn(b"pHYs", png_chunks(slide.get_pixmap(dpi=72)))
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened.page_count, 1)
                self.assertIn("Updated report", reopened[0].get_text())
                self.assertEqual(reopened[0].find_tables()[0].rows[1], ["Alice", "99"])
                self.assertEqual(reopened[0].get_images()[0].alt, "Blue sample")
            with doc_ex.open(document.convert("pdf")) as pdf:
                self.assertEqual(pdf.page_count, 1)
                self.assertIn("Updated report", pdf[0].get_text())
            copy = document.copy()
        with self.assertRaises(DocumentClosedError):
            _ = text.text
        with copy:
            copy.slides.add().add_text("Independent", box=(20, 20, 200, 50))
            self.assertEqual(copy.page_count, 2)

    def test_numeric_color_validation_and_failed_pptx_edits(self):
        self.assertAlmostEqual(doc_ex.mm(25.4), 72)
        self.assertAlmostEqual(doc_ex.cm(2.54), 72)
        self.assertEqual(doc_ex.inches(1), 72)
        for value in (math.nan, math.inf):
            with self.assertRaises(ValueError):
                doc_ex.pt(value)
        with self.assertRaises(TypeError):
            doc_ex.pt("12")
        with self.assertRaises(UnsupportedFormatError):
            doc_ex.new("doc")
        with self.assertRaises(TypeError):
            doc_ex.new("pdf", size=())
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            shape = slide.add_text("Retained", box=(0, 0, 300, 50))
            for bad in ("bad\x00", "bad\ufffe", "bad\uffff"):
                with self.assertRaises(ValueError):
                    shape.text = bad
                self.assertEqual(shape.text, "Retained")
            with self.assertRaises(ValueError):
                shape.box = (0, 0, -1, 20)
            self.assertEqual(shape.box, (0, 0, 300, 50))
            with self.assertRaises(ValueError):
                slide.background = (1, 2, 999)
            self.assertIn("Retained", slide.get_text())


if __name__ == "__main__":
    unittest.main()
