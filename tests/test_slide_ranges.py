from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from test_rendering import png_chunks


class SlideRangeTests(unittest.TestCase):
    def test_unicode_rich_ranges_links_and_literal_replacement(self):
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            shape = slide.add_text("λ ", box=(20, 20, 600, 200))
            shape.add_run("{{na", bold=True)
            shape.add_run("me}}\nSecond", italic=True)
            stale = shape.find("{{name}}")[0]
            self.assertEqual(shape.replace_text("{{name}}", "Acme"), 1)
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = stale.text
            self.assertEqual(shape.text, "λ Acme\nSecond")
            selected = shape.find("Acme")[0]
            selected.format(bold=False, underline="double", font_size=22, color="navy", highlight="yellow")
            selected.link = "https://example.com/report"
            selected.link_tooltip = "Read report"
            self.assertEqual(selected.link_tooltip, "Read report")
            self.assertEqual(selected.link, "https://example.com/report")
            with self.assertRaises(ValueError):
                selected.link = "file:///tmp/no"
            self.assertEqual(selected.text, "Acme")
            for data in (document.to_bytes(), document.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    self.assertIn("Acme", reopened[0].get_text())
                    self.assertTrue(any(link.uri == "https://example.com/report" for link in reopened[0].get_links()))
                    self.assertIn(b"IDAT", png_chunks(reopened.render_page(0, dpi=72)))
            self.assertEqual(shape.replace_text("Acme\nSecond", "Joined"), 1)
            self.assertEqual(shape.text, "λ Joined")

    def test_slide_links_survive_reorder_and_block_referenced_deletion(self):
        with doc_ex.new("pptx") as document, doc_ex.new("pptx") as other:
            first = document.slides.add()
            target = document.slides.add()
            target.add_text("Destination", box=(20, 20, 300, 80))
            shape = first.add_shape("rectangle", box=(20, 20, 100, 50), fill="blue")
            shape.link = target
            shape.link_tooltip = "Go to details"
            foreign = other.slides.add()
            with self.assertRaises(ValueError):
                shape.link = foreign
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                del document.slides[target.number]
            self.assertEqual(len(document.slides), 2)
            document.slides.move(1, 0)
            self.assertEqual(shape.link.number, 0)
            self.assertEqual(shape.link_tooltip, "Go to details")
            with doc_ex.open(document.convert("pdf")) as pdf:
                links = pdf[1].get_links()
                self.assertTrue(any(link.metadata.get("target_page") == 0 for link in links))
            shape.link = None
            del document.slides[0]
            self.assertEqual(len(document.slides), 1)
            self.assertIsNone(shape.link)

    def test_text_links_and_reset_do_not_leak_into_appended_runs(self):
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            shape = slide.add_text("Linked", box=(20, 20, 500, 100))
            selected = shape.find("Linked")[0]
            selected.link = "https://example.com"
            selected.format(italic=True, underline=True)
            selected.format(italic=None, underline=None)
            appended = shape.add_run(" plain")
            self.assertIsNone(appended.link)
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = ET.fromstring(archive.read("ppt/slides/slide1.xml"))
                ns = {"a": "http://schemas.openxmlformats.org/drawingml/2006/main"}
                run = next(r for r in xml.findall(".//a:r", ns) if " plain" in "".join(r.itertext()))
                self.assertIsNone(run.find("a:rPr/a:hlinkClick", ns))
            selected = shape.find("Linked")[0]
            selected.link = None
            self.assertIsNone(selected.link)
            with self.assertRaises(ValueError):
                selected.link_tooltip = "No target"

    def test_mixed_link_ranges_can_be_reassigned_or_cleared(self):
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            shape = slide.add_text("AB", box=(20, 20, 300, 100))
            shape.find("A")[0].link = "https://a.example"
            shape.find("B")[0].link = "https://b.example"
            selected = shape.find("AB")[0]
            with self.assertRaisesRegex(ValueError, "mixed"):
                _ = selected.link
            selected.link = "https://all.example"
            self.assertEqual(selected.link, "https://all.example")
            self.assertEqual(selected.link_tooltip, "")
            selected.link_tooltip = "Uniform tooltip"
            selected.link = "https://replacement.example"
            self.assertEqual(selected.link_tooltip, "Uniform tooltip")
            stale = shape.find("AB")[0]
            shape.find("A")[0].link = None
            with self.assertRaisesRegex(ValueError, "stale"):
                stale.link = None
            selected = shape.find("AB")[0]
            selected.link = None
            self.assertIsNone(selected.link)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertFalse(reopened[0].get_links())
