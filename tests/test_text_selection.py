from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError


class TextSelectionTests(unittest.TestCase):
    def test_unicode_slices_follow_python_bounds_without_mutating_document(self):
        text = "  A😀e\u0301\tB\nC  "
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph(text)
            existing = paragraph.find("😀")[0]
            before = document.to_bytes()
            for selection in (slice(None), slice(3, 6), slice(5, 6), slice(-4, -1),
                              slice(-100, 100), slice(999, 1000), slice(5, 2),
                              slice(None, 0), slice(True, 4, True)):
                with self.subTest(selection=selection):
                    selected = paragraph[selection]
                    self.assertIs(type(selected), type(existing))
                    self.assertEqual(selected.text, text[selection])
            self.assertEqual(existing.text, "😀")
            self.assertEqual(document.to_bytes(), before)
            with doc_ex.open(before) as reopened:
                self.assertEqual(reopened[0].blocks[0].text, text)

    def test_empty_insertion_reuses_range_format_link_and_full_deletion(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("")
            paragraph.add_run("Left", bold=True)
            paragraph.add_run(" Right", italic=True)
            stale = paragraph[:4]
            insertion = paragraph[4:4]
            self.assertEqual(insertion.text, "")
            insertion.text = " 😀e\u0301 "
            self.assertEqual(insertion.text, " 😀e\u0301 ")
            insertion.format(underline="single", color="blue")
            insertion.link = "https://example.com/selection"
            self.assertEqual(insertion.link, "https://example.com/selection")
            with self.assertRaises(ValueError):
                _ = stale.text
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                root = ET.fromstring(archive.read("word/document.xml"))
            w = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
            runs = {"".join(run.itertext()): run for run in root.iter(f"{w}r")}
            self.assertIsNotNone(runs["Left"].find(f"{w}rPr/{w}b"))
            self.assertIsNotNone(runs[" Right"].find(f"{w}rPr/{w}i"))
            with doc_ex.open(document.to_bytes()) as reopened:
                links = reopened[0].get_links()
                self.assertEqual([(link.text, link.uri) for link in links],
                                 [(" 😀e\u0301 ", "https://example.com/selection")])
            paragraph[:4].text = ""
            self.assertEqual(paragraph.text, " 😀e\u0301  Right")
            paragraph[:].text = ""
            self.assertEqual(paragraph.text, "")
            self.assertEqual(len(document.paragraphs), 1)
            paragraph[50:0].text = "Restored"
            self.assertEqual(paragraph.text, "Restored")
            with doc_ex.open(document.convert("pdf")) as reopened:
                self.assertEqual(reopened.to_markdown().count("Restored"), 1)

    def test_same_selection_api_in_body_cell_header_and_note_stories(self):
        with doc_ex.new("docx") as document:
            body = document.add_paragraph("Body label")
            table = document.add_table([["Cell label"]])
            cell = table[0, 0].paragraphs[0]
            header = document.sections[0].header().add_paragraph("Header label")
            note = document.notes.add(body[:4], "Note label")
            note_paragraph = note.paragraphs[0]
            for paragraph in (body, cell, header, note_paragraph):
                paragraph[:0].text = "Prefix "
                paragraph[-5:].format(bold=True)
                self.assertTrue(paragraph.text.startswith("Prefix "))
            with doc_ex.open(document.convert("pdf")) as reopened:
                text = reopened.to_markdown()
                for label in ("Cell label", "Header label", "Note label"):
                    self.assertEqual(text.count(label), 1)
            selected = cell[:6]
            with document.copy() as copied:
                copied.tables[0][0, 0].paragraphs[0][:6].text = "Copied"
                self.assertEqual(selected.text, "Prefix")
                self.assertEqual(copied.tables[0][0, 0].paragraphs[0].text, "Copied Cell label")
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = selected.text
            with self.assertRaises(DocumentClosedError):
                _ = header[:]

    def test_invalid_slices_do_not_edit_and_selection_detects_intervening_change(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Stable text")
            existing = paragraph[:6]
            before = document.to_bytes()
            for selection in (0, -1, "text", (0, 2), slice(None, None, 0),
                              slice(None, None, -1), slice(None, None, 2),
                              slice(1.5, 3), slice(0, "3"), slice(None, None, 1.0)):
                with self.subTest(selection=selection):
                    with self.assertRaises((TypeError, ValueError)):
                        _ = paragraph[selection]
                    self.assertEqual(existing.text, "Stable")
                    self.assertEqual(document.to_bytes(), before)
            empty = paragraph[3:1]
            with self.assertRaises(ValueError):
                empty.format(bold=True)
            with self.assertRaises(ValueError):
                empty.link = "https://example.com/"
            self.assertEqual(document.to_bytes(), before)

            class MutatingIndex:
                def __index__(self):
                    paragraph.text = "Changed text"
                    return 0

            with self.assertRaisesRegex(ValueError, "text changed during selection"):
                _ = paragraph[MutatingIndex():3]
            self.assertEqual(paragraph.text, "Changed text")

    def test_selection_cannot_remove_semantic_note_anchors(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Before marker after")
            note = document.notes.add(paragraph[7:13], "Preserved note")
            offset = paragraph.text.index("\ufffc")
            marker = paragraph[offset:offset + 1]
            original = document.to_bytes()
            for selected in (marker, paragraph[:]):
                with self.assertRaises((ValueError, doc_ex.DocExError)):
                    selected.text = ""
                self.assertEqual(document.to_bytes(), original)
                self.assertEqual(marker.text, "\ufffc")
                self.assertEqual(note.paragraphs[0].text, "Preserved note")
            paragraph[:7].text = "Changed "
            self.assertEqual(note.paragraphs[0].text, "Preserved note")
            with self.assertRaises(ValueError):
                _ = marker.text
            with doc_ex.open(document.convert("pdf")) as reopened:
                self.assertEqual(reopened.to_markdown().count("Preserved note"), 1)

    def test_slide_selection_crosses_paragraphs_and_preserves_placeholder_links(self):
        with doc_ex.new("pptx", size=(720, 540)) as document:
            slide = document.slides.add(layout="title_and_content")
            destination = document.slides.add()
            title, shape = slide.shapes
            title.text = "Selection title"
            text = "A😀e\u0301\nSecond tail"
            shape.text = text
            box, role = shape.box, shape.placeholder
            original = document.to_bytes()
            existing = shape.find("Second")[0]
            for key in (slice(None), slice(2, 8), slice(-8, -2), slice(-999, 999),
                        slice(999, 1000), slice(5, 2), slice(True, 4, True)):
                selected = shape[key]
                self.assertIs(type(selected), type(existing))
                self.assertEqual(selected.text, text[key])
            self.assertEqual(existing.text, "Second")
            self.assertEqual(document.to_bytes(), original)
            selected = shape[1:10]
            selected.format(bold=True, color="blue")
            selected.link = destination
            selected.link_tooltip = "Go to destination"
            self.assertEqual(selected.link.number, destination.number)
            self.assertEqual(selected.link_tooltip, "Go to destination")
            self.assertEqual(selected.text, text[1:10])
            with self.assertRaises(ValueError):
                _ = existing.text
            linked = document.to_bytes()
            p = "{http://schemas.openxmlformats.org/presentationml/2006/main}"
            a = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
            with ZipFile(BytesIO(original)) as before, ZipFile(BytesIO(linked)) as after:
                for name in before.namelist():
                    if name.startswith(("ppt/slideLayouts/", "ppt/slideMasters/", "ppt/theme/")):
                        self.assertEqual(before.read(name), after.read(name))
                root = ET.fromstring(after.read("ppt/slides/slide1.xml"))
            content = root.findall(f".//{p}sp")[1]
            self.assertIsNone(content.find(f"{p}spPr/{a}xfrm"))
            labels = [node for node in content.iter(f"{a}hlinkClick")]
            self.assertGreaterEqual(len(labels), 2)
            self.assertTrue(all(node.attrib.get("tooltip") == "Go to destination" for node in labels))
            self.assertEqual((shape.box, shape.placeholder), (box, role))
            selected.text = "Joined\nBody"
            self.assertEqual(shape.text, text[:1] + "Joined\nBody" + text[10:])
            shape[:0].text = "Prefix "
            shape[-5:].text = ""
            self.assertEqual(shape.text, "Prefix AJoined\nBodyd")
            self.assertEqual((shape.box, shape.placeholder), (box, role))
            for output in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(output) as reopened:
                    self.assertIn("Prefix AJoined", reopened[0].get_text())
                    self.assertIn("Bodyd", reopened[0].get_text())

    def test_slide_selection_rejects_nontext_and_invalid_handles_without_retargeting(self):
        from test_authoring import image_fixture
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            other = document.slides.add()
            shape = slide.add_text("Stable 😀 text", box=(10, 10, 240, 60))
            table = slide.add_table([["Cell"]], box=(10, 100, 200, 80))
            picture = slide.add_image(image_fixture(), box=(300, 10, 40, 40))
            rectangle = slide.add_shape("rectangle", box=(300, 100, 40, 40))
            selected = shape[:6]
            before = document.to_bytes()
            for invalid in (0, (0, 1), slice(None, None, -1), slice(None, None, 0),
                            slice(None, None, 2), slice(0.5, 2)):
                with self.assertRaises((TypeError, ValueError)):
                    _ = shape[invalid]
            for nontext in (picture, rectangle):
                with self.assertRaises(ValueError):
                    _ = nontext[:]
            self.assertEqual(table[0, 0].text, "Cell")
            self.assertEqual(table[:, 0][0].text, "Cell")
            self.assertEqual(selected.text, "Stable")
            self.assertEqual(document.to_bytes(), before)
            document.slides.move(0, 1)
            self.assertEqual(shape[:6].text, "Stable")
            with self.assertRaises(ValueError):
                _ = selected.text
            shape[:].text = ""
            shape[99:0].text = "Restored"
            self.assertEqual(shape.text, "Restored")
            old = shape[:]
            del slide.shapes[0]
            with self.assertRaises(ValueError):
                _ = old.text
            with self.assertRaises(ValueError):
                _ = shape[:]
            survivor = other.add_text("Survivor", box=(10, 10, 100, 30))
            surviving_range = survivor[:]
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = survivor[:]
            with self.assertRaises(DocumentClosedError):
                _ = surviving_range.text


if __name__ == "__main__":
    unittest.main()
