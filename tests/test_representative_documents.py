"""Combined authored documents, executed with the installed public package."""
from io import BytesIO
import math
import struct
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from test_cookbook import recipe
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


BUILDERS = recipe("representative_documents")
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


class RepresentativeDocumentTests(unittest.TestCase):
    def check_renders(self, document, size, *, table=False):
        self.assertEqual(document.page_count, 1)
        for dpi in (72, 144, 300):
            chunks = png_chunks(document.render_page(0, dpi=dpi))
            dimensions = struct.unpack(">II", chunks[b"IHDR"][:8])
            self.assertEqual(dimensions, tuple(math.ceil(n * dpi / 72 - .01) for n in size))
            xppm, yppm, unit = struct.unpack(">IIB", chunks[b"pHYs"])
            self.assertEqual((yppm, unit), (xppm, 1))
            self.assertAlmostEqual(xppm * .0254, dpi, delta=.013)
            width, pixels = rgba_pixels(chunks)
            self.assertGreater(pixels.count(b"\xf0\xf4\xf8\xff"), 100)
            if table:
                self.assertGreater(pixels.count(b"\xdd\xee\xff\xff"), 100)
                for region in ((20, 30, 340, 55), (20, 440, 340, 465)):
                    # These are bounds within the full PNG, not a render clip.
                    x0, y0, x1, y1 = (round(value * dpi / 72) for value in region)
                    dark = sum(max(pixels[(y * width + x) * 4:(y * width + x) * 4 + 3]) < 128
                               and pixels[(y * width + x) * 4 + 3] == 255
                               for y in range(y0, y1) for x in range(x0, x1))
                    self.assertGreater(dark, 10,
                                       ("missing visible header/footer text", region, dpi))
            # Large solid blue headings are positive visible-text controls.
            self.assertGreater(pixels.count(b"\x00\x00\x80\xff"), 20)

    def test_multilingual_resume_combines_stories_table_links_and_diacritics(self):
        expected = [["Skill", "Experience"], ["Python automation", "Five years"],
                    ["Document quality", "Layout and review"]]
        labels = ["Camille Müller", "Résumé professionnel",
                  "Création de rapports et coordination des équipes.", "Berufserfahrung",
                  "Präzise Dokumentation für internationale Teams.", "Python automation",
                  "Document quality", "Selected work"]
        with BUILDERS.build_resume() as document:
            source = document.to_bytes()
            with ZipFile(BytesIO(source)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
                languages = [node.get(W + "val") for node in xml.iter(W + "lang")]
                self.assertIn("fr-FR", languages)
                self.assertIn("de-DE", languages)
            for kind, data in (("docx", source), ("pdf", document.convert("pdf"))):
                with self.subTest(kind=kind), doc_ex.open(data) as opened:
                    text = opened[0].get_text()
                    for label in labels:
                        self.assertEqual(text.count(label), 1, (kind, label, text))
                    link = next(link for link in opened[0].get_links()
                                if link.uri == "https://example.com/portfolio")
                    self.assertEqual(link.text, "Selected work")
                    if kind == "docx":
                        self.assertEqual(opened[0].find_tables()[0].rows, expected)
                        self.assertEqual([s.get_text().strip() for s in opened.stories
                                          if s.kind == "header"], ["EXAMPLE STUDIO"])
                        self.assertEqual([s.get_text().strip() for s in opened.stories
                                          if s.kind == "footer"], ["Prepared for discussion"])
                    else:
                        self.assertEqual(text.count("EXAMPLE STUDIO"), 1)
                        self.assertEqual(text.count("Prepared for discussion"), 1)
                    self.check_renders(opened, (360, 500), table=True)

    def test_review_preserves_comments_separately_and_exports_clean_body(self):
        labels = ("Confirm the delivery date.", "The validation owner will confirm.")
        target = "Publish the reviewed package on Friday."
        with BUILDERS.build_review() as document:
            self.assertEqual([c.text for c in document.comments], list(labels))
            self.assertEqual([c.author for c in document.comments], ["Ada", "Ben"])
            self.assertEqual([c.resolved for c in document.comments], [False, True])
            self.assertEqual(document.comments[1].parent.range.text, target)
            with document.copy() as plain:
                del plain.comments[1]
                del plain.comments[0]
                self.assertEqual(len(document.comments), 2)
                plain_pdf = plain.convert("pdf")
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                comments = ET.fromstring(archive.read("word/comments.xml"))
                self.assertEqual([n.get(W + "author") for n in comments], ["Ada", "Ben"])
                self.assertEqual([n.get(W + "date") for n in comments],
                                 ["2026-10-21T09:00:00Z", "2026-10-21T09:30:00Z"])
            with doc_ex.open(document.to_bytes()) as source:
                stories = [s for s in source.stories if s.kind == "comment"]
                self.assertEqual([s.get_text().strip() for s in stories], list(labels))
                self.assertEqual([s.metadata["author"] for s in stories], ["Ada", "Ben"])
                self.assertEqual([s.metadata["resolved"] for s in stories], [False, True])
                self.assertEqual(stories[1].metadata["parent_id"], stories[0].metadata["comment_id"])
                for label in labels:
                    self.assertEqual(source.to_markdown().count(label), 1)
                self.assertEqual(source[0].get_text().count(target), 1)
                self.assertEqual(source[0].find_tables()[0].rows,
                                 [["Decision", "Owner"], ["Release readiness", "Mira Chen"]])
                for kind, label in (("header", "EXAMPLE STUDIO"),
                                    ("footer", "Prepared for discussion")):
                    self.assertEqual([s.get_text().strip() for s in source.stories if s.kind == kind], [label])
                self.check_renders(source, (360, 500), table=True)
            with doc_ex.open(document.convert("pdf")) as exported, doc_ex.open(plain_pdf) as clean:
                text = exported[0].get_text()
                for label in labels:
                    self.assertNotIn(label, text)
                for label in (target, "Release readiness", "Mira Chen"):
                    self.assertEqual(text.count(label), 1)
                for dpi in (72, 144, 300):
                    self.assertEqual(exported.render_page(0, dpi=dpi), clean.render_page(0, dpi=dpi))
                self.check_renders(exported, (360, 500), table=True)

    def test_intake_fields_copy_flatten_preserve_visible_appearance_and_links(self):
        with BUILDERS.build_intake_form() as document:
            self.assertEqual([f.name for f in document.fields], ["contact", "service", "approved"])
            self.assertEqual([f.value for f in document.fields], ["Mira Chen", "Document review", True])
            self.assertTrue(all(f.tooltip for f in document.fields))
            with document.copy() as changed:
                changed.fields["contact"].value = "Other contact"
                self.assertEqual(document.fields["contact"].value, "Mira Chen")
            with document.copy() as flattened:
                self.assertEqual(flattened.fields.flatten(), 3)
                self.assertEqual(len(flattened.fields), 0)
                self.assertEqual(len(document.fields), 3)
                with doc_ex.open(document.to_bytes()) as original, doc_ex.open(flattened.to_bytes()) as output:
                    for dpi in (72, 144, 300):
                        self.assertEqual(original.render_page(0, dpi=dpi), output.render_page(0, dpi=dpi))
                    for opened in (original, output):
                        self.assertTrue(any(link.uri == "https://example.com/services"
                                            for link in opened[0].get_links()))
                        text = opened[0].get_text()
                        for label in ("Project intake", "Mira Chen", "Document review", "Service guide"):
                            self.assertEqual(text.count(label), 1)
                        self.check_renders(opened, (360, 420))


if __name__ == "__main__":
    unittest.main()
