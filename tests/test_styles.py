from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_rendering import png_chunks


NS = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}


class StyleTests(unittest.TestCase):
    def test_named_styles_inheritance_resets_and_native_output(self):
        with doc_ex.new("docx") as document:
            base = document.styles.add("Report base", based_on="Normal")
            base.format(font_size=14, bold=True, color="navy", space_after=9)
            derived = document.styles.add("Report detail", based_on=base)
            self.assertEqual(document.styles[derived.name].id, derived.id)
            self.assertEqual(document.styles[derived.id].based_on.id, base.id)
            self.assertIsNone(derived.formatting["bold"])
            self.assertTrue(derived.resolved_formatting["bold"])
            self.assertEqual(derived.resolved_formatting["font_size"], 14)
            derived.format(bold=False, font_size=18)
            self.assertFalse(derived.resolved_formatting["bold"])
            derived.format(font_size=None)
            self.assertEqual(derived.resolved_formatting["font_size"], 14)
            derived.format(bold=None)
            paragraph = document.add_paragraph("Inherited ", style=derived)
            paragraph.add_run("content")
            paragraph.format(alignment="center", indent_left=12, space_before=6, keep_with_next=True)
            self.assertEqual(paragraph.alignment, "center")
            self.assertEqual(paragraph.spacing[:2], (6, 9))
            paragraph.format(space_before=None)
            self.assertEqual(paragraph.spacing[0], base.resolved_formatting["space_before"])
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
                run = next(r for r in xml.findall(".//w:r", NS) if "content" in "".join(r.itertext()))
                self.assertIsNone(run.find("w:rPr/w:b", NS), "omitted bold must inherit")
                styles = ET.fromstring(archive.read("word/styles.xml"))
                self.assertIsNotNone(styles.find(f"w:style[@w:styleId='{derived.id}']/w:basedOn", NS))
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn("Inherited content", reopened[0].get_text())
            with doc_ex.open(document.convert("pdf")) as reopened:
                self.assertIn("Inherited content", reopened[0].get_text())
                self.assertIn(b"IDAT", png_chunks(reopened.render_page(0, dpi=72)))

    def test_character_style_and_rich_formatting_clear(self):
        with doc_ex.new("docx") as document:
            emphasis = document.styles.add("Report emphasis", kind="character")
            emphasis.format(italic=True, underline="double", color="blue")
            paragraph = document.add_paragraph("Before ")
            selected = paragraph.add_run("λ report", style=emphasis.id, bold=True)
            selected.format(bold=None, underline=False, script="superscript", highlight="yellow", background="navy")
            selected.format(script=None, background=None)
            self.assertEqual(selected.text, "λ report")
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
                run = next(r for r in xml.findall(".//w:r", NS) if "λ report" in "".join(r.itertext()))
                self.assertIsNone(run.find("w:rPr/w:b", NS))
                self.assertEqual(run.find("w:rPr/w:rStyle", NS).get(f"{{{NS['w']}}}val"), emphasis.id)
                self.assertEqual(run.find("w:rPr/w:u", NS).get(f"{{{NS['w']}}}val"), "none")
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn("λ report", reopened[0].get_text())

    def test_invalid_style_changes_are_atomic_and_handles_are_owned(self):
        with doc_ex.new("docx") as document, doc_ex.new("docx") as other:
            parent = document.styles.add("Parent", based_on="Normal")
            child = document.styles.add("Child", based_on=parent)
            char = document.styles.add("Character", kind="character")
            count = len(document.styles)
            before = parent.formatting
            for operation in (
                lambda: document.styles.add("Parent"),
                lambda: parent.format(bold=True, unknown="no"),
                lambda: setattr(parent, "based_on", child),
                lambda: setattr(parent, "based_on", char),
                lambda: other.add_paragraph("No", style=parent),
            ):
                with self.assertRaises((ValueError, TypeError)):
                    operation()
                self.assertEqual(len(document.styles), count)
                self.assertEqual(parent.formatting, before)
            with document.copy() as copied:
                copied.styles[parent.id].format(bold=True)
                self.assertEqual(parent.formatting, before)
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = child.name

    def test_table_cell_spacing_uses_native_table_style(self):
        with doc_ex.new("docx") as document:
            cell = document.add_table([["Value"]])[0, 0]
            self.assertEqual(cell.paragraphs[0].spacing, (0, 0, 1))
