"""Owned slide/table character spacing through the existing range formatter."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_direction_language import colored_extent
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"


def runs(document, page=0):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        xml = ET.fromstring(archive.read(f"ppt/slides/slide{page + 1}.xml"))
    return [(r.findtext(A + "t"), None if r.find(A + "rPr") is None
             else r.find(A + "rPr").get("spc")) for r in xml.iter(A + "r")]


class SlideLetterSpacingTests(unittest.TestCase):
    def test_visible_unicode_expansion_condensation_source_and_pdf(self):
        for text in ("office", "e\u0301 Word", "سلام"):
            with self.subTest(text=text), doc_ex.new("pptx", size=(648, 252)) as deck:
                shape = deck.slides.add().add_text(text, box=(50, 50, 540, 120), font_size=24)
                shape.color = "red"
                baseline = colored_extent(deck.render_page(0, dpi=200))
                for spacing, expanded in ((2, True), (-1, False)):
                    shape[:].format(letter_spacing=spacing)
                    raw = deck.to_bytes()
                    source_png = deck.render_page(0, dpi=200)
                    extent = colored_extent(source_png)
                    if expanded:
                        self.assertGreater(extent[1] - extent[0], baseline[1] - baseline[0])
                    else:
                        self.assertLess(extent[1] - extent[0], baseline[1] - baseline[0])
                    with doc_ex.open(raw) as source, doc_ex.open(deck.convert("pdf")) as pdf:
                        self.assertEqual(source[0].get_text(), text)
                        self.assertEqual(source.render_page(0, dpi=200), source_png)
                        self.assertEqual(pdf.page_count, 1)
                        a_width, a = rgba_pixels(png_chunks(source_png))
                        b_width, b = rgba_pixels(png_chunks(pdf.render_page(0, dpi=200)))
                        self.assertEqual((a_width, len(a)), (b_width, len(b)))
                        # Native PDF rasterizes slides at 200 DPI; this fixture uses
                        # that resolution rather than assuming arbitrary-DPI equality.
                        self.assertLessEqual(max(abs(x - y) for x, y in zip(a, b)), 1)
                shape[:].format(letter_spacing=None)
                self.assertEqual(colored_extent(deck.render_page(0, dpi=200)), baseline)

    def test_shape_cell_precision_reset_append_and_saved_properties(self):
        with doc_ex.new("pptx") as deck:
            slide = deck.slides.add()
            shape = slide.add_text("Shape", box=(20, 20, 300, 60))
            cell = slide.add_table([["Cell"]], box=(20, 100, 300, 100))[0, 0].paragraphs[0]
            for paragraph in (shape, cell):
                for spacing in (-100, -0.1 - 0.2, -0.05, 0, 0.1 + 0.2, 100):
                    paragraph[:].format(letter_spacing=spacing)
                    expected = str(round(spacing * 100))
                    self.assertIn((paragraph.text, expected), runs(deck))
                paragraph[:].format(letter_spacing=None)
                self.assertIn((paragraph.text, None), runs(deck))
                paragraph.add_run(" spaced", letter_spacing=0.3)
                paragraph.add_run(" inherited")
                self.assertTrue(any("spaced inherited" in text and spacing == "30"
                                    for text, spacing in runs(deck)))
                paragraph.add_run(" reset", letter_spacing=None)
                self.assertIn((" reset", None), runs(deck))
                paragraph.add_run(" zero", letter_spacing=0)
                self.assertIn((" zero", "0"), runs(deck))
            with doc_ex.open(deck.to_bytes()) as source:
                self.assertEqual(source[0].get_text().count("spaced inherited reset zero"), 2)

    def test_invalid_patches_are_atomic_and_copy_is_independent(self):
        with doc_ex.new("pptx") as deck:
            slide = deck.slides.add()
            shape = slide.add_text("Shape", box=(20, 20, 300, 60))
            cell = slide.add_table([["Cell"]], box=(20, 100, 300, 100))[0, 0].paragraphs[0]
            for paragraph in (shape, cell):
                selected = paragraph[:]
                selected.format(letter_spacing=0.3)
                before = deck.to_bytes()
                cached = deck.render_page(0, dpi=72)
                for invalid in (True, "1", float("nan"), float("inf"), -100.05, 100.05, 0.01, 0.301):
                    with self.subTest(invalid=invalid), self.assertRaises((ValueError, TypeError)):
                        selected.format(bold=True, letter_spacing=invalid)
                    self.assertEqual(deck.to_bytes(), before)
                    self.assertEqual(deck.render_page(0, dpi=72), cached)
                    self.assertEqual(selected.text, paragraph.text)
            with deck.copy() as copied:
                copied.slides[0].shapes[0][:].format(letter_spacing=-1)
                self.assertIn(("Shape", "30"), runs(deck))
                self.assertIn(("Shape", "-100"), runs(copied))
                deck.close()
                with self.assertRaises(DocumentClosedError):
                    shape[:].format(letter_spacing=0)
                self.assertEqual(copied.slides[0].shapes[0].text, "Shape")

    def test_links_merge_aliases_and_range_revision_protection(self):
        with doc_ex.new("pptx") as deck:
            source = deck.slides.add()
            target = deck.slides.add()
            target.add_text("Destination", box=(20, 20, 400, 60))
            shape = source.add_text("URL target", box=(20, 20, 500, 60))
            shape.find("URL")[0].link = "https://example.com/spacing"
            shape.find("target")[0].link = target
            table = source.add_table([["Cell", "Tail"]], box=(20, 100, 500, 100))
            table.merge((0, 0), (0, 1))
            alias = table[0, 1].paragraphs[0]
            alias[:].format(letter_spacing=0.3)
            self.assertEqual(table[0, 0].paragraphs[0].text, "Cell")
            shape[:].format(letter_spacing=0.3)
            deck.slides.move(1, 0)
            with doc_ex.open(deck.to_bytes()) as read:
                links = read[1].get_links()
                self.assertTrue(any(link.uri == "https://example.com/spacing" and link.text == "URL" for link in links))
                self.assertTrue(any(link.metadata.get("target_page") == 0 and link.text == "target" for link in links))
            with doc_ex.open(deck.convert("pdf")) as pdf:
                self.assertTrue(any(link.metadata.get("target_page") == 0 for link in pdf[1].get_links()))
            selected = alias[:]
            table[0, 0].text = "Replaced"
            with self.assertRaisesRegex(ValueError, "stale"):
                selected.format(letter_spacing=1)
            self.assertEqual(table[0, 0].text, "Replaced")

    def test_range_callback_mutation_close_and_early_stale_checks(self):
        for in_cell in (False, True):
            with self.subTest(in_cell=in_cell), doc_ex.new("pptx") as deck:
                slide = deck.slides.add()
                deck.slides.add()
                paragraph = (slide.add_table([["Keep"]], box=(20, 20, 300, 100))[0, 0].paragraphs[0]
                             if in_cell else slide.add_text("Keep", box=(20, 20, 300, 100)))
                selected = paragraph[:]
                class Moving(str):
                    def __len__(self):
                        deck.slides.move(0, 1)
                        return super().__len__()
                with self.assertRaisesRegex(ValueError, "stale"):
                    selected.format(letter_spacing=Moving("1"), bold=True)
                self.assertEqual(paragraph.text, "Keep")
                self.assertTrue(all(spacing is None for _, spacing in runs(deck, 1)))
                calls = []
                class Raising(str):
                    def __len__(self):
                        calls.append(True)
                        raise AssertionError("already-stale range invoked callback")
                with self.assertRaisesRegex(ValueError, "stale"):
                    selected.format(letter_spacing=Raising("1"))
                self.assertEqual(calls, [])
                selected = paragraph[:]
                class Closing(str):
                    def __len__(self):
                        deck.close()
                        return super().__len__()
                with self.assertRaises(DocumentClosedError):
                    selected.format(letter_spacing=Closing("1"))


if __name__ == "__main__":
    unittest.main()
