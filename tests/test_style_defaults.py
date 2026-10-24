from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_styles import NS


class StyleDefaultsTests(unittest.TestCase):
    def test_next_and_linked_styles_roundtrip_with_distinct_relationship_rules(self):
        with doc_ex.new("docx") as document:
            first = document.styles.add("First paragraph")
            second = document.styles.add("Second paragraph")
            character = document.styles.add("Linked character", kind="character")
            other = document.styles.add("Other character", kind="character")
            first.next = first
            self.assertEqual(first.next.id, first.id)
            first.next = second.name
            second.next = first.id
            first.linked = character
            self.assertEqual(character.linked.id, first.id)
            self.assertEqual(first.linked.id, character.id)
            first.linked = other
            self.assertIsNone(character.linked)
            self.assertEqual(other.linked.id, first.id)
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                styles = ET.fromstring(archive.read("word/styles.xml"))
                node = styles.find(f"w:style[@w:styleId='{first.id}']", NS)
                self.assertEqual(node.find("w:next", NS).get(f"{{{NS['w']}}}val"), second.id)
                self.assertEqual(node.find("w:link", NS).get(f"{{{NS['w']}}}val"), other.id)
                partner = styles.find(f"w:style[@w:styleId='{other.id}']/w:link", NS)
                self.assertEqual(partner.get(f"{{{NS['w']}}}val"), first.id)
            with document.copy() as copied:
                copied.styles[first.id].linked = None
                self.assertIsNone(copied.styles[other.id].linked)
                self.assertEqual(first.linked.id, other.id)
            other.linked = None
            first.next = None
            self.assertIsNone(first.linked)
            self.assertIsNone(first.next)
            self.assertEqual(second.next.id, first.id)

    def test_undefined_builtin_link_is_explicit_and_repairable(self):
        with doc_ex.new("docx") as document:
            heading = document.styles["Heading1"]
            with self.assertRaisesRegex(UnsupportedFormatError, "undefined"):
                _ = heading.linked
            heading.linked = None
            self.assertIsNone(heading.linked)
            partner = document.styles.add("Heading partner", kind="character")
            document.styles["Heading2"].linked = partner
            self.assertEqual(partner.linked.id, "Heading2")
            self.assertEqual(document.styles["Heading2"].linked.id, partner.id)

    def test_native_defaults_affect_inheriting_content_and_preserve_overrides(self):
        with doc_ex.new("docx") as document:
            inherited = document.styles.add("Inherited")
            explicit = document.styles.add("Explicit", based_on=inherited)
            explicit.format(font_size=21, color="blue", bold=False)
            document.add_paragraph("INHERITED CONTENT", style=inherited)
            paragraph = document.add_paragraph("EXPLICIT CONTENT", style=explicit)
            table = document.add_table([["TABLE CONTENT"]])
            before_png = document.render_page(0, dpi=72)
            document.styles.format_defaults(font_size=16, color="red", bold=True,
                                            space_after=12, tabs=[{"position": 72, "alignment": "right"}])
            self.assertEqual(document.styles.defaults["font_size"], 16)
            self.assertTrue(inherited.resolved_formatting["bold"])
            self.assertEqual(inherited.resolved_formatting["font_size"], 16)
            self.assertEqual(explicit.resolved_formatting["font_size"], 21)
            self.assertFalse(explicit.resolved_formatting["bold"])
            self.assertEqual(paragraph.resolved_formatting["space_after"], 12)
            self.assertEqual(paragraph.resolved_formatting["tabs"][0]["position"], 72)
            self.assertEqual(table[0, 0].text, "TABLE CONTENT")
            self.assertNotEqual(document.render_page(0, dpi=72), before_png)
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                styles = ET.fromstring(archive.read("word/styles.xml"))
                size = styles.find("w:docDefaults/w:rPrDefault/w:rPr/w:sz", NS)
                self.assertEqual(size.get(f"{{{NS['w']}}}val"), "32")
                self.assertIsNotNone(styles.find("w:docDefaults/w:pPrDefault/w:pPr/w:tabs", NS))
            for source in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(source) as reopened:
                    for label in ("INHERITED CONTENT", "EXPLICIT CONTENT", "TABLE CONTENT"):
                        self.assertEqual(reopened.to_markdown().count(label), 1)
                    _, rgba = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                    colors = {rgba[i:i + 4] for i in range(0, len(rgba), 4)}
                    self.assertIn(b"\xff\x00\x00\xff", colors)
                    self.assertIn(b"\x00\x00\xff\xff", colors)
            with document.copy() as copied:
                copied.styles.format_defaults(font_size=18)
                self.assertEqual(document.styles.defaults["font_size"], 16)
            before = document.to_bytes()
            # Clearing the default size is refused (it would change on reopen); the whole patch
            # is rejected, so the bold change is not applied either.
            with self.assertRaises(UnsupportedFormatError):
                document.styles.format_defaults(font_size=None, bold=False)
            self.assertEqual(document.styles.defaults["font_size"], 16)
            self.assertEqual(document.to_bytes(), before)
            document.styles.format_defaults(bold=False)
            self.assertFalse(inherited.resolved_formatting["bold"])
            document.styles.format_defaults(bold=True)
            self.assertEqual(document.to_bytes(), before)
            document.styles.format_defaults(bold=None)
            self.assertIsNone(document.styles.defaults["bold"])
            document.styles.format_defaults(bold=False)
            self.assertFalse(document.styles.defaults["bold"])
            self.assertEqual(document.styles.defaults["font_size"], 16)

    def test_invalid_relations_defaults_and_effective_limits_are_atomic(self):
        with doc_ex.new("docx") as document, doc_ex.new("docx") as other:
            first = document.styles.add("First")
            second = document.styles.add("Second")
            partner = document.styles.add("Partner", kind="character")
            foreign = other.styles.add("Foreign")
            first.linked = partner
            second.format(tabs=[{"position": 200}])
            document.add_paragraph("Keep", style=second)
            before = document.to_bytes()
            for operation in (
                lambda: setattr(second, "linked", partner),
                lambda: setattr(partner, "next", first),
                lambda: setattr(first, "next", partner),
                lambda: setattr(first, "linked", second),
                lambda: setattr(first, "next", foreign),
                lambda: document.styles.format_defaults(row_band_size=2),
                lambda: document.styles.format_defaults(region="first_row", bold=True),
                lambda: document.styles.format_defaults(font_size=-1),
                lambda: document.styles.format_defaults(tabs=[{"position": index * 2} for index in range(64)]),
            ):
                with self.assertRaises((ValueError, TypeError)):
                    operation()
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(first.linked.id, partner.id)
                self.assertEqual(partner.linked.id, first.id)
                self.assertIsNone(second.linked)
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = first.next
            with self.assertRaises(DocumentClosedError):
                _ = document.styles.defaults


if __name__ == "__main__":
    unittest.main()
