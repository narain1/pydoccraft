from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_styles import NS


class ParagraphBorderTests(unittest.TestCase):
    def test_native_group_inheritance_direct_patch_and_reset(self):
        with doc_ex.new("docx") as document:
            document.styles.format_defaults(borders={"bottom": {"color": "green", "width": 2}})
            base = document.styles.add("Border base")
            child = document.styles.add("Border child", based_on=base)
            self.assertEqual(child.resolved_formatting["borders"]["bottom"]["color"], "#008000")
            base.format(borders={"top": {"color": "blue", "width": 2}})
            self.assertIsNone(child.resolved_formatting["borders"]["bottom"])
            self.assertEqual(child.resolved_formatting["borders"]["top"]["color"], "#0000FF")
            paragraph = document.add_paragraph("Inherited border", style=child)
            paragraph.format(borders={"left": {"color": "red", "width": 2}})
            self.assertIsNone(paragraph.resolved_formatting["borders"]["top"])
            paragraph.format(borders={"bottom": {"color": "#800080", "width": 2}})
            self.assertEqual(paragraph.formatting["borders"]["left"]["color"], "#FF0000")
            paragraph.format(borders={"left": None})
            self.assertIsNone(paragraph.formatting["borders"]["left"])
            self.assertEqual(paragraph.formatting["borders"]["bottom"]["color"], "#800080")
            paragraph.format(borders=None)
            self.assertIsNone(paragraph.formatting["borders"])
            self.assertEqual(paragraph.resolved_formatting["borders"]["top"]["color"], "#0000FF")
            before = document.to_bytes()
            paragraph.format(borders={})
            self.assertEqual(document.to_bytes(), before)
            with document.copy() as copied:
                copied.styles[base.id].format(borders=None)
                self.assertEqual(copied.paragraphs[0].resolved_formatting["borders"]["bottom"]["color"], "#008000")
                self.assertEqual(paragraph.resolved_formatting["borders"]["top"]["color"], "#0000FF")

    def test_paragraph_borders_roundtrip_and_render_in_body_cells_and_stories(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Bordered body")
            paragraph.format(borders={"top": {"style": "double", "color": "red", "width": 2},
                                      "right": {"style": "dashed", "color": "blue", "width": 2}})
            cell = document.add_table([["Bordered cell"]])[0, 0].paragraphs[0]
            cell.format(borders={"bottom": {"color": "green", "width": 2}})
            document.sections[0].header().add_paragraph("Bordered header").format(borders={"top": {"color": "red", "width": 2}})
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                root = ET.fromstring(archive.read("word/document.xml"))
                nodes = root.findall(".//w:pPr/w:pBdr", NS)
                self.assertEqual(len(nodes), 2)
                self.assertEqual(nodes[0].find("w:top", NS).get(f"{{{NS['w']}}}val"), "double")
                self.assertEqual(nodes[1].find("w:bottom", NS).get(f"{{{NS['w']}}}color"), "008000")
            for source in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(source) as reopened:
                    text = reopened.to_markdown() + "\n" + "\n".join(block.text for story in reopened.stories for block in story.blocks)
                    for label in ("Bordered body", "Bordered cell", "Bordered header"):
                        self.assertEqual(text.count(label), 1)
                    _, rgba = rgba_pixels(png_chunks(reopened.render_page(0, dpi=144)))
                    pixels = list(zip(rgba[0::4], rgba[1::4], rgba[2::4], rgba[3::4]))
                    for channel in range(3):
                        self.assertTrue(any(p[channel] > 80 and all(p[channel] > p[other] + 50 for other in range(3) if other != channel) for p in pixels))

    def test_table_border_meaning_and_unsupported_edges_remain_explicit(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Kept paragraph")
            table = document.add_table([["Kept cell"]])
            style = document.styles.add("Kept table style", kind="table")
            style.format(borders={"inside_horizontal": {"style": "double", "width": 1}})
            self.assertEqual(style.formatting["borders"]["inside_horizontal"]["style"], "double")
            before = document.to_bytes()
            for target in (paragraph, table, table[0, 0], style):
                for border in ({"top": {"style": "triple"}}, {"top": {"style": "dot_dash"}},
                               {"top": {"width": 0.125}}, {"top": {"style": "thick", "width": 1}},
                               {"left": {"style": "double"}}, {"right": {"style": "wave"}}):
                    with self.subTest(target=type(target).__name__, border=border), self.assertRaises((ValueError, UnsupportedFormatError)):
                        target.format(borders=border)
                    self.assertEqual(document.to_bytes(), before)
            for side in ("between", "bar", "inside_vertical"):
                with self.assertRaises((ValueError, TypeError, UnsupportedFormatError)):
                    paragraph.format(borders={side: {"width": 1}})
                self.assertEqual(document.to_bytes(), before)
            with self.assertRaises((ValueError, UnsupportedFormatError)):
                document.styles.format_defaults(borders={"left": {"style": "wave"}})
            self.assertEqual(document.to_bytes(), before)


if __name__ == "__main__":
    unittest.main()
