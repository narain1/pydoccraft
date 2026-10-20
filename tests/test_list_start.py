from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_numbering_extraction import all_blocks


W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def numbering_by_text(source):
    with doc_ex.open(source) as document:
        return {block.text: block.metadata.get("numbering") for block in all_blocks(document)}


class ListStartTests(unittest.TestCase):
    def test_start_membership_continuation_restart_and_native_xml(self):
        with doc_ex.new("docx") as document:
            first, selected, assigned = [document.add_paragraph(text)
                                         for text in ("First", "Selected", "Already assigned")]
            for paragraph in (first, selected, assigned):
                paragraph.set_list("numbered")
            selected.find("Selected")[0].format(bold=True)
            before = numbering_by_text(document.to_bytes())
            selected.set_list("numbered", start=7)
            changed = numbering_by_text(document.to_bytes())
            self.assertEqual(changed["Already assigned"]["num_id"], before["Already assigned"]["num_id"])
            self.assertEqual(changed["Already assigned"]["label"], "2.")
            self.assertEqual(changed["Selected"]["label"], "7.")
            self.assertNotEqual(changed["Selected"]["abstract_id"], changed["First"]["abstract_id"])
            future = document.add_paragraph("Future")
            future.set_list("numbered")
            self.assertEqual(numbering_by_text(document.to_bytes())["Future"]["label"], "3.")
            future.set_list("numbered", restart=True, start=20)
            document.add_paragraph("Continued").set_list("numbered")
            document.add_paragraph("Restart one").set_list("numbered", restart=True)
            document.add_paragraph("Maximum start").set_list("numbered", start=32767)
            document.add_paragraph("Beyond start bound").set_list("numbered")
            source = document.to_bytes()
            values = numbering_by_text(source)
            self.assertEqual([values[text]["label"] for text in
                              ("First", "Selected", "Already assigned", "Future", "Continued", "Restart one", "Maximum start", "Beyond start bound")],
                             ["1.", "7.", "2.", "20.", "21.", "1.", "32767.", "32768."])
            self.assertEqual(selected.list_style, ("numbered", 0))
            with ZipFile(BytesIO(source)) as archive:
                numbering = ET.fromstring(archive.read("word/numbering.xml"))
                main = ET.fromstring(archive.read("word/document.xml"))
            starts = {int(node.get(f"{W}numId")): int(node.find(f"{W}lvlOverride/{W}startOverride").get(f"{W}val"))
                      for node in numbering.findall(f"{W}num") if node.find(f"{W}lvlOverride/{W}startOverride") is not None}
            self.assertEqual(starts[values["Selected"]["num_id"]], 7)
            self.assertEqual(starts[values["Future"]["num_id"]], 20)
            selected_run = next(run for run in main.iter(f"{W}r") if "".join(run.itertext()) == "Selected")
            self.assertIsNotNone(selected_run.find(f"{W}rPr/{W}b"))
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = pdf.to_markdown()
                for expected in ("7.", "20.", "21.", "32767.", "32768."):
                    self.assertIn(expected, text)
                for label in values:
                    self.assertEqual(text.count(label), 1)

    def test_warm_render_copy_and_clear_preserve_content_and_independent_sequences(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Same paragraph")
            paragraph.set_list("numbered")
            original = document.to_bytes()
            original_png = document.render_page(0, dpi=72)
            with document.copy() as copied:
                paragraph.set_list("numbered", start=7)
                self.assertEqual(paragraph.text, "Same paragraph")
                self.assertNotEqual(document.render_page(0, dpi=72), original_png)
                self.assertEqual(copied.to_bytes(), original)
                self.assertEqual(copied.render_page(0, dpi=72), original_png)
                copied.paragraphs[0].set_list("numbered", start=20)
                self.assertEqual(numbering_by_text(document.to_bytes())["Same paragraph"]["label"], "7.")
                self.assertEqual(numbering_by_text(copied.to_bytes())["Same paragraph"]["label"], "20.")
            paragraph.set_list(None)
            self.assertIsNone(paragraph.list_style)
            self.assertIsNone(numbering_by_text(document.to_bytes())["Same paragraph"])
            self.assertEqual(paragraph.text, "Same paragraph")

    def test_cell_and_note_starts_render_but_header_footer_starts_fail_atomically(self):
        with doc_ex.new("docx") as document:
            body = document.add_paragraph("Body reference")
            body.set_list("numbered", start=7)
            table = document.add_table([["Cell item"]])
            table[0, 0].paragraphs[0].set_list("numbered", start=20)
            nested = table[0, 0].add_table([["Nested item"]])
            nested[0, 0].paragraphs[0].set_list("numbered", start=30)
            footnote = document.notes.add(body.find("reference")[0], "Footnote item")
            footnote.paragraphs[0].set_list("numbered", start=40)
            endnote = document.notes.add(body.find("Body")[0], "Endnote item", kind="endnote")
            endnote.paragraphs[0].set_list("numbered", start=50)
            header = document.sections[0].header().add_paragraph("Header item")
            footer = document.sections[0].footer().add_paragraph("Footer item")
            for paragraph in (header, footer):
                paragraph.set_list("numbered")
                before = document.to_bytes()
                selected = paragraph[:]
                with self.assertRaisesRegex(UnsupportedFormatError, "headers/footers"):
                    paragraph.set_list("numbered", start=11)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(selected.text, paragraph.text)
            with doc_ex.open(document.to_bytes()) as source:
                blocks = all_blocks(source) + [block for story in source.stories for block in story.blocks]
                for text, label in (("Cell item", "20."), ("Nested item", "30."), ("Footnote item", "40."), ("Endnote item", "50.")):
                    self.assertEqual(next(block for block in blocks if block.text == text).metadata["numbering"]["label"], label)
                with doc_ex.open(source.convert("pdf")) as pdf:
                    text = pdf.to_markdown()
                    for label in ("7.", "20.", "30.", "40.", "50."):
                        self.assertIn(label, text)
                    for label in ("Cell item", "Nested item", "Footnote item", "Endnote item"):
                        self.assertEqual(text.count(label), 1)

    def test_invalid_starts_and_callback_deletions_cannot_retarget_paragraphs(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Stable")
            paragraph.set_list("numbered", start=7)
            selected = paragraph[:]
            before = document.to_bytes()
            for value in (True, False, 0, -1, 32768, 2**100, 1.0, "7"):
                with self.subTest(value=value), self.assertRaises((TypeError, ValueError, OverflowError)):
                    paragraph.set_list("numbered", start=value)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(selected.text, "Stable")
            for options in ({"kind": None}, {"kind": "bullet"}, {"kind": "numbered", "level": 1}):
                with self.assertRaises(ValueError):
                    paragraph.set_list(start=7, **options)
                self.assertEqual(document.to_bytes(), before)
            # The private native boundary also rejects bool, rather than accepting it as1.
            with self.assertRaises(TypeError):
                document._native.set_paragraph_list(paragraph._id, "numbered", 0, False, True)
            self.assertEqual(document.to_bytes(), before)
            table = document.add_table([["Deleted"], ["Survivor"]])
            target = table[0, 0].paragraphs[0]
            callback_state = []

            class DeleteTarget:
                def __index__(self):
                    table.delete_row(0)
                    callback_state.append(document.to_bytes())
                    return 7

            with self.assertRaises(IndexError):
                target.set_list("numbered", start=DeleteTarget())
            self.assertEqual(document.to_bytes(), callback_state[0])
            self.assertEqual(table[0, 0].text, "Survivor")
            self.assertIsNone(table[0, 0].paragraphs[0].list_style)

            class CloseOwner:
                def __index__(self):
                    document.close()
                    return 7

            with self.assertRaises(DocumentClosedError):
                paragraph.set_list("numbered", start=CloseOwner())
            with self.assertRaises(DocumentClosedError):
                paragraph.set_list("numbered", start=7)


if __name__ == "__main__":
    unittest.main()
