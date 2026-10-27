from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


NS = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
VAL = "{" + NS["w"] + "}val"


def xml_parts(document):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        return {name: ET.fromstring(archive.read(name)) for name in archive.namelist()
                if name.startswith("word/") and name.endswith(".xml")}


def colored_extent(png):
    width, pixels = rgba_pixels(png_chunks(png))
    points = [(i // 4 % width, i // 4 // width) for i in range(0, len(pixels), 4)
              if pixels[i] > 180 and pixels[i + 1] < 100 and pixels[i + 2] < 100]
    if not points:
        raise AssertionError("expected visibly red text")
    return (min(x for x, _ in points), max(x for x, _ in points),
            min(y for _, y in points), max(y for _, y in points), len(points))


class DirectionLanguageTests(unittest.TestCase):
    def test_direction_inheritance_reset_and_visible_native_pdf_alignment(self):
        with doc_ex.new("docx") as document:
            section = document.sections[0]
            section.page_size = (300, 300)
            section.margins = 30
            document.styles.format_defaults(direction="rtl", language="fa-IR")
            base = document.styles.add("Directional text", based_on="Normal")
            base.format(direction="rtl", language="fa-IR", color="red", font_size=18)
            child = document.styles.add("Inherited direction", based_on=base)
            paragraph = document.add_paragraph("سلام Word 250", style=child)
            self.assertIsNone(paragraph.formatting["direction"])
            self.assertEqual(paragraph.resolved_formatting["direction"], "rtl")
            self.assertIsNone(child.formatting["direction"])
            self.assertEqual(child.resolved_formatting["language"], "fa-IR")
            rtl_extent = colored_extent(document.render_page(0, dpi=144))
            # Logical left alignment is the start edge, physically right in RTL.
            self.assertGreater(rtl_extent[0], 200)
            self.assertLessEqual(abs(rtl_extent[1] - 540), 6)
            with doc_ex.open(document.convert("pdf")) as pdf:
                pdf_extent = colored_extent(pdf.render_page(0, dpi=144))
                for a, b in zip(rtl_extent[:4], pdf_extent[:4]):
                    self.assertLessEqual(abs(a - b), 2)
                self.assertIn("Word", pdf.to_markdown())
                self.assertIn("250", pdf.to_markdown())
            paragraph.format(direction="ltr")
            self.assertEqual(paragraph.formatting["direction"], "ltr")
            self.assertEqual(paragraph.resolved_formatting["direction"], "ltr")
            ltr_extent = colored_extent(document.render_page(0, dpi=144))
            self.assertLess(ltr_extent[0], 65)
            self.assertNotEqual(ltr_extent, rtl_extent)
            parts = xml_parts(document)
            self.assertEqual(parts["word/document.xml"].find(".//w:pPr/w:bidi", NS).get(VAL), "0")
            paragraph.format(direction=None)
            self.assertIsNone(paragraph.formatting["direction"])
            self.assertEqual(paragraph.resolved_formatting["direction"], "rtl")
            self.assertEqual(colored_extent(document.render_page(0, dpi=144)), rtl_extent)
            with doc_ex.open(document.to_bytes()) as source:
                self.assertIn("سلام Word 250", source.to_markdown())
            document.styles.format_defaults(language=None, direction=None)
            parts = xml_parts(document)
            self.assertIsNone(parts["word/styles.xml"].find("w:docDefaults/w:rPrDefault/w:rPr/w:lang", NS))
            self.assertIsNone(parts["word/styles.xml"].find("w:docDefaults/w:pPrDefault/w:pPr/w:bidi", NS))
            self.assertIsNone(document.styles.defaults["language"])
            self.assertEqual(paragraph.resolved_formatting["direction"], "rtl")

    def test_language_character_style_clear_unicode_and_owned_ranges(self):
        with doc_ex.new("docx") as document:
            style = document.styles.add("Hebrew proofing", kind="character")
            style.format(language="he-IL")
            paragraph = document.add_paragraph("Prefix ")
            selected = paragraph.add_run("שלום e\u0301 😀", style=style.id, language="en-GB")
            old = paragraph[:]
            selected.format(language=None, italic=True)
            self.assertEqual(selected.text, "שלום e\u0301 😀")
            with self.assertRaises(ValueError):
                _ = old.text
            parts = xml_parts(document)
            run = next(r for r in parts["word/document.xml"].findall(".//w:r", NS)
                       if "שלום" in "".join(r.itertext()))
            # The direct language is gone. WordCraft's writer may still tag a right-to-left
            # segment with the *style's* complex-script language (w:bidi), never the cleared one.
            lang = run.find("w:rPr/w:lang", NS)
            if lang is not None:
                self.assertIsNone(lang.get(f"{{{NS['w']}}}val"))
                self.assertIsNone(lang.get(f"{{{NS['w']}}}eastAsia"))
                self.assertEqual(lang.get(f"{{{NS['w']}}}bidi"), "he-IL")
            self.assertEqual(run.find("w:rPr/w:rStyle", NS).get(VAL), style.id)
            self.assertEqual(style.resolved_formatting["language"], "he-IL")
            with document.copy() as copied:
                copied.styles[style.id].format(language="ar-SA")
                self.assertEqual(style.formatting["language"], "he-IL")
                self.assertEqual(copied.styles[style.id].formatting["language"], "ar-SA")
                document.close()
                with self.assertRaises(DocumentClosedError):
                    selected.format(language="fr-FR")
                self.assertIn("שלום", copied.paragraphs[0].text)
                self.assertTrue(copied.render_page(0, dpi=72).startswith(b"\x89PNG"))

    def test_body_nested_cell_header_footer_and_note_scope_roundtrip(self):
        with doc_ex.new("docx") as document:
            body = document.add_paragraph("BodyAnchor")
            nested = document.add_table([[""]])[0, 0].add_table([[""]])[0, 0].paragraphs[0]
            header = document.sections[0].header().add_paragraph("")
            footer = document.sections[0].footer().add_paragraph("")
            footnote = document.notes.add(body.find("Body")[0], "").paragraphs[0]
            endnote = document.notes.add(body.find("Anchor")[0], "", kind="endnote").paragraphs[0]
            paragraphs = [nested, header, footer, footnote, endnote]
            for index, paragraph in enumerate(paragraphs):
                paragraph.format(direction="rtl")
                paragraph.add_run(f"Scope{index} سلام", language="fa-IR", color="red")
                self.assertEqual(paragraph.resolved_formatting["direction"], "rtl")
            parts = xml_parts(document)
            for index in range(5):
                node = next(p for xml in parts.values() for p in xml.findall(".//w:p", NS)
                            if f"Scope{index}" in "".join(p.itertext()))
                self.assertIsNotNone(node.find("w:pPr/w:bidi", NS))
                self.assertEqual(node.find(".//w:rPr/w:lang", NS).get(VAL), "fa-IR")
            for data in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    text = reopened.to_markdown() + "\n" + "\n".join(s.get_text() for s in reopened.stories)
                    for index in range(5):
                        self.assertIn(f"Scope{index}", text)
                    total_pixels = sum(colored_extent(reopened.render_page(i, dpi=72))[4]
                                       for i in range(reopened.page_count))
                    self.assertGreater(total_pixels, 100)

    def test_table_direction_and_conditional_language_leave_column_order(self):
        with doc_ex.new("docx") as document:
            style = document.styles.add("Regional language", kind="table")
            style.format(direction="rtl", language="ar-SA")
            style.format(region="first_row", language="he-IL")
            table = document.add_table([["First", "Second"], ["Third", "Fourth"]])
            table.format(style=style, look={"header_row": True})
            first = table[0, 0].paragraphs[0]
            first.format(direction="ltr")
            self.assertEqual(first.resolved_formatting["direction"], "ltr")
            self.assertEqual(table[0, 1].paragraphs[0].resolved_formatting["direction"], "rtl")
            self.assertFalse(table.formatting["rtl"])
            self.assertEqual(style.resolved_formatting["regions"]["first_row"]["language"], "he-IL")
            first.format(direction=None)
            self.assertEqual(first.resolved_formatting["direction"], "rtl")
            snapshot = document.to_bytes()
            with self.assertRaises(ValueError):
                style.format(region="first_row", direction="rtl")
            self.assertEqual(document.to_bytes(), snapshot)
            for data in (snapshot, document.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    text = reopened.to_markdown()
                    self.assertLess(text.index("First"), text.index("Second"))
                    self.assertLess(text.index("Third"), text.index("Fourth"))
                    self.assertTrue(reopened.render_page(0, dpi=72).startswith(b"\x89PNG"))

    def test_invalid_tags_directions_and_callback_changes_are_atomic(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Keep")
            selected = paragraph.add_run(" bounded", language="a" * 63)
            before = document.to_bytes()
            for value in ("a" * 64, "", " en", "en ", "en_US", "en--US", "-en", "en-", "日本", True, 12):
                with self.subTest(language=value), self.assertRaises((ValueError, TypeError)):
                    selected.format(language=value, bold=True)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(selected.text, " bounded")
            for value in ("auto", "RTL", True, 0):
                with self.subTest(direction=value), self.assertRaises((ValueError, TypeError)):
                    paragraph.format(direction=value, alignment="right")
                self.assertEqual(document.to_bytes(), before)
            class MutatingTag(str):
                def __len__(self):
                    paragraph.add_run(" callback")
                    return super().__len__()
            with self.assertRaises(ValueError):
                selected.format(language=MutatingTag("en-US"))
            self.assertEqual(paragraph.text, "Keep bounded callback")
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                self.assertNotIn(b'en-US', archive.read("word/document.xml"))
            class ClosingDirection(str):
                def __len__(self):
                    document.close()
                    return super().__len__()
            with self.assertRaises(DocumentClosedError):
                paragraph.format(direction=ClosingDirection("rtl"))
