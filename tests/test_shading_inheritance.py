from collections import Counter
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_styles import NS


COLORS = {
    "red": b"\xff\x00\x00\xff",
    "green": b"\x00\x80\x00\xff",
    "blue": b"\x00\x00\xff\xff",
    "yellow": b"\xff\xff\x00\xff",
    "gray": b"\x80\x80\x80\xff",
}


class ShadingInheritanceTests(unittest.TestCase):
    def prepare(self, document):
        document.sections[0].margins = 20
        document.background = "gray"

    def assert_output_colors(self, document, present, absent=()):
        native = document.to_bytes()
        for kind, data in (("DOCX", native), ("PDF", document.convert("pdf"))):
            with self.subTest(output=kind), doc_ex.open(data) as reopened:
                self.assertEqual(reopened.page_count, 1)
                _, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                counts = Counter(pixels[i:i + 4] for i in range(0, len(pixels), 4))
                for color in ("gray", *present):
                    self.assertGreater(counts[COLORS[color]], 100, f"missing {color} fill in {kind}")
                for color in absent:
                    self.assertEqual(counts[COLORS[color]], 0, f"unexpected {color} fill in {kind}")
        with ZipFile(BytesIO(native)) as archive:
            return (ET.fromstring(archive.read("word/document.xml")),
                    ET.fromstring(archive.read("word/styles.xml")))

    def test_run_shading_and_highlight_inherit_and_reset_independently(self):
        with doc_ex.new("docx", size=(300, 200)) as document:
            self.prepare(document)
            style = document.styles.add("Marked text", kind="character")
            style.format(background="red", highlight="yellow", font_size=20)
            paragraph = document.add_paragraph("")
            selected = paragraph.add_run("MMMMMMMMMM", style=style.id)

            def run_properties(xml):
                return next(run for run in xml.findall(".//w:r", NS)
                            if "MMMMMMMMMM" in "".join(run.itertext())).find("w:rPr", NS)

            xml, styles = self.assert_output_colors(document, ("yellow",), ("red", "green"))
            self.assertIsNone(run_properties(xml).find("w:shd", NS))
            self.assertIsNone(run_properties(xml).find("w:highlight", NS))
            definition = styles.find(f"w:style[@w:styleId='{style.id}']/w:rPr", NS)
            self.assertEqual(definition.find("w:shd", NS).get(f"{{{NS['w']}}}fill"), "FF0000")
            self.assertEqual(definition.find("w:highlight", NS).get(f"{{{NS['w']}}}val"), "yellow")

            selected.format(background="green")
            self.assert_output_colors(document, ("yellow",), ("red", "green"))
            selected.format(highlight="none")
            xml, _ = self.assert_output_colors(document, ("green",), ("red", "yellow"))
            self.assertEqual(run_properties(xml).find("w:highlight", NS).get(f"{{{NS['w']}}}val"), "none")
            self.assertEqual(run_properties(xml).find("w:shd", NS).get(f"{{{NS['w']}}}fill"), "008000")

            selected.format(background=None)
            xml, _ = self.assert_output_colors(document, ("red",), ("green", "yellow"))
            self.assertIsNone(run_properties(xml).find("w:shd", NS))
            self.assertEqual(run_properties(xml).find("w:highlight", NS).get(f"{{{NS['w']}}}val"), "none")
            selected.format(highlight=None)
            xml, _ = self.assert_output_colors(document, ("yellow",), ("red", "green"))
            self.assertIsNone(run_properties(xml).find("w:highlight", NS))
            style.format(highlight=None)
            self.assert_output_colors(document, ("red",), ("green", "yellow"))
            self.assertEqual(document.background, "#808080")

    def test_paragraph_and_cell_shading_return_to_inherited_style_after_clear(self):
        for scope in ("paragraph", "cell"):
            with self.subTest(scope=scope), doc_ex.new("docx", size=(300, 200)) as document:
                self.prepare(document)
                kind = "paragraph" if scope == "paragraph" else "table"
                base = document.styles.add("Base fill", kind=kind)
                base.format(background="red")
                derived = document.styles.add("Derived fill", kind=kind, based_on=base)
                if scope == "paragraph":
                    target = document.add_paragraph("Paragraph shading", style=derived)
                    direct_path = ".//w:body/w:p/w:pPr/w:shd"
                else:
                    table = document.add_table([["Cell shading"]])
                    table.format(style=derived)
                    target = table[0, 0]
                    direct_path = ".//w:tc/w:tcPr/w:shd"
                self.assert_output_colors(document, ("red",), ("green", "blue"))
                target.format(background="green")
                xml, _ = self.assert_output_colors(document, ("green",), ("red", "blue"))
                self.assertEqual(xml.find(direct_path, NS).get(f"{{{NS['w']}}}fill"), "008000")
                target.format(background=None)
                self.assertIsNone(target.formatting["background"])
                xml, _ = self.assert_output_colors(document, ("red",), ("green", "blue"))
                self.assertIsNone(xml.find(direct_path, NS))
                derived.format(background="blue")
                self.assert_output_colors(document, ("blue",), ("red", "green"))
                derived.format(background=None)
                xml, styles = self.assert_output_colors(document, ("red",), ("green", "blue"))
                definition = styles.find(f"w:style[@w:styleId='{derived.id}']", NS)
                self.assertEqual(definition.find("w:basedOn", NS).get(f"{{{NS['w']}}}val"), base.id)
                self.assertIsNone(definition.find(".//w:shd", NS))

    def test_default_paragraph_shading_is_separate_from_page_background(self):
        with doc_ex.new("docx", size=(300, 200)) as document:
            self.prepare(document)
            document.styles.format_defaults(background="red")
            paragraph = document.add_paragraph("Default shading")
            paragraph.format(background="green")
            self.assert_output_colors(document, ("green",), ("red",))
            paragraph.format(background=None)
            _, styles = self.assert_output_colors(document, ("red",), ("green",))
            shading = styles.find("w:docDefaults/w:pPrDefault/w:pPr/w:shd", NS)
            self.assertEqual(shading.get(f"{{{NS['w']}}}fill"), "FF0000")
            document.styles.format_defaults(background=None)
            _, styles = self.assert_output_colors(document, (), ("red", "green", "yellow"))
            self.assertIsNone(styles.find("w:docDefaults/w:pPrDefault/w:pPr/w:shd", NS))
            self.assertEqual(document.background, "#808080")


if __name__ == "__main__":
    unittest.main()
