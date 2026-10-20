"""Execute the cookbook sources against the installed package, without Pillow."""
from collections import Counter
import importlib.util
import math
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

import doc_ex
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


EXAMPLES = Path(__file__).resolve().parents[1] / "examples"


def recipe(name):
    spec = importlib.util.spec_from_file_location(f"cookbook_{name}", EXAMPLES / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CookbookTests(unittest.TestCase):
    def check_png(self, data, size, dpi, *, ink=True, colors=()):
        chunks = png_chunks(data)
        dimensions = struct.unpack(">II", chunks[b"IHDR"][:8])
        self.assertEqual(dimensions, tuple(max(1, math.ceil(side * dpi / 72 - 0.01)) for side in size))
        xppm, yppm, unit = struct.unpack(">IIB", chunks[b"pHYs"])
        self.assertEqual((xppm, unit), (yppm, 1))
        self.assertAlmostEqual(xppm * 0.0254, dpi, delta=0.013)
        _, pixels = rgba_pixels(chunks)
        counts = Counter(pixels[index:index + 4] for index in range(0, len(pixels), 4))
        if ink:
            self.assertGreater(sum(count for pixel, count in counts.items()
                                   if min(pixel[:3]) < 100 and pixel[3] == 255), 20)
        for color in colors:
            self.assertGreater(counts[color], 20, f"missing visible color {color!r}")
        return counts

    def check_output(self, path, labels, *, colors=()):
        """Reopen and render every page of each advertised document output."""
        with doc_ex.open(path) as document:
            self.assertEqual(document.page_count, len(labels))
            texts = [page.get_text() for page in document]
            for index, label in enumerate(labels):
                self.assertEqual(texts[index].count(label), 1)
                self.assertEqual(sum(text.count(label) for text in texts), 1)
                page = document[index]
                size = (page.metadata["width"], page.metadata["height"])
                self.check_png(document.render_page(index, dpi=72), size, 72,
                               colors=colors[index] if colors else ())
            return texts

    def test_letter_story_order_links_and_preview(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = recipe("letter").create_letter(directory, dpi=96)
            body = ["20 October 2026", "Mira Chen", "Dear Mira,", "Your project is ready for review.",
                    "Please send your comments by 30 October 2026.", "Project guide", "Alex Morgan"]
            for kind in ("docx", "pdf"):
                text = self.check_output(paths[kind], ["Your project is ready for review."])[0]
                positions = [text.index(value) for value in body]
                self.assertEqual(positions, sorted(positions))
                for value in body:
                    self.assertEqual(text.count(value), 1)
                with doc_ex.open(paths[kind]) as reopened:
                    links = reopened[0].get_links()
                    self.assertTrue(any(link.uri == "https://example.com/project-guide"
                                        and link.text == "Project guide" for link in links))
                    size = (reopened[0].metadata["width"], reopened[0].metadata["height"])
                    self.assertAlmostEqual(size[0], doc_ex.mm(210), delta=0.1)
                    self.assertAlmostEqual(size[1], doc_ex.mm(297), delta=0.1)
                    if kind == "docx":
                        stories = reopened.stories
                        self.check_png(paths["png"].read_bytes(), size, 96)
                    else:
                        self.assertIn("EXAMPLE STUDIO | Client services", text)
                        self.assertIn("Example Studio | example.com", text)
            # Extracted stories remain usable after their source closes.
            self.assertTrue(any(story.kind == "header" and "EXAMPLE STUDIO" in story.get_text()
                                for story in stories))
            self.assertTrue(any(story.kind == "footer" and "example.com" in story.get_text()
                                for story in stories))

    def test_invoice_decimal_values_semantic_table_and_fill(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = recipe("invoice").create_invoice(directory, dpi=96)
            expected = [["Service", "Quantity", "Amount"], ["Support", "2", "$250.00"],
                        ["Training", "1", "$300.00"]]
            for kind in ("docx", "pdf"):
                text = self.check_output(paths[kind], ["Grand total: $550.00"],
                                         colors=[(b"\xdd\xee\xff\xff",)])[0]
                for value in ("Support", "Training", "$250.00", "$300.00", "$550.00"):
                    self.assertEqual(text.count(value), 1)
                with doc_ex.open(paths[kind]) as reopened:
                    self.assertTrue(any(link.uri == "https://example.com/payment-terms"
                                        for link in reopened[0].get_links()))
                    if kind == "docx":
                        self.assertEqual(reopened[0].find_tables()[0].rows, expected)
                        page = reopened[0]
                        self.check_png(paths["png"].read_bytes(),
                                       (page.metadata["width"], page.metadata["height"]), 96,
                                       colors=(b"\xdd\xee\xff\xff",))

    def test_report_two_pages_repeated_stories_brand_and_table(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = recipe("branded_report").create_report(directory, dpi=96)
            labels = ["Quarterly overview", "Next quarter"]
            colors = [(b"\xf0\xf4\xf8\xff", b"\x00\x00\xff\xff", b"\xdd\xee\xff\xff"),
                      (b"\xf0\xf4\xf8\xff", b"\x00\x00\xff\xff")]
            for kind in ("docx", "pdf"):
                texts = self.check_output(paths[kind], labels, colors=colors)
                self.assertEqual("\n".join(texts).count("Completed projects"), 1)
                with doc_ex.open(paths[kind]) as reopened:
                    if kind == "docx":
                        stories = reopened.stories
                        self.assertEqual(reopened[0].find_tables()[0].rows,
                                         [["Measure", "Result"], ["Completed projects", "12"],
                                          ["Client satisfaction", "96%"]])
                        for index, page in enumerate(reopened):
                            self.check_png(paths[f"page_{index + 1}"].read_bytes(),
                                           (page.metadata["width"], page.metadata["height"]),
                                           96, colors=colors[index])
                    else:
                        for text in texts:
                            self.assertEqual(text.count("EXAMPLE STUDIO"), 1)
                            self.assertEqual(text.count("Internal report | October 2026"), 1)
            self.assertEqual(sum(story.get_text().count("EXAMPLE STUDIO") for story in stories), 1)
            self.assertEqual(sum(story.get_text().count("Internal report | October 2026") for story in stories), 1)

    def test_pdf_assembly_order_split_source_independence_and_links(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = recipe("pdf_assembly").assemble_pdfs(directory, dpi=96)
            red, blue = b"\xff\x00\x00\xff", b"\x00\x00\xff\xff"
            for key, labels, colors in (
                ("source-a", ["Source A"], [(red,)]), ("source-b", ["Source B"], [(blue,)]),
                ("assembled", ["Source B", "Source A"], [(blue,), (red,)]),
                ("subset", ["Source A"], [(red,)]),
            ):
                self.check_output(paths[key], labels, colors=colors)
            with doc_ex.open(paths["assembled"]) as assembled, doc_ex.open(paths["source-a"]) as source_a, \
                    doc_ex.open(paths["source-b"]) as source_b, doc_ex.open(paths["subset"]) as subset:
                # Exact output pixels prove imported pages and the source copies retained their content.
                self.assertEqual(assembled.render_page(0, dpi=72), source_b.render_page(0, dpi=72))
                self.assertEqual(assembled.render_page(1, dpi=72), source_a.render_page(0, dpi=72))
                self.assertEqual(subset.render_page(0, dpi=72), source_a.render_page(0, dpi=72))
                for page in (source_a[0], assembled[1], subset[0]):
                    self.assertTrue(any(link.uri == "https://example.com/source-a" for link in page.get_links()))
                for index, color in enumerate((blue, red), start=1):
                    self.check_png(paths[f"page_{index}"].read_bytes(), (360, 240), 96, colors=(color,))

    def test_slide_preview_saved_deck_pdf_and_region_at_requested_dpi(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = recipe("slide_preview").create_slide_preview(directory, dpi=144)
            red, blue = b"\xff\x00\x00\xff", b"\x00\x00\xff\xff"
            for kind in ("pptx", "pdf"):
                texts = self.check_output(paths[kind], ["Project overview", "Next steps"],
                                         colors=[(red, b"\xf0\xf4\xf8\xff"), (blue, b"\xff\xf4\xd6\xff")])
                self.assertEqual(texts[0].count("Design"), 1)
            for index, color in enumerate((red, blue), start=1):
                self.check_png(paths[f"page_{index}"].read_bytes(), (480, 270), 144, colors=(color,))
            self.check_png(paths["clip"].read_bytes(), (60, 40), 144, ink=False, colors=(red,))

    def test_existing_template_recipe_writes_two_independent_renderable_sets(self):
        with tempfile.TemporaryDirectory() as directory:
            outputs = recipe("template_batch").write_letters(directory, dpi=96)
            self.assertEqual(len(outputs), 2)
            for paths, customer in zip(outputs, ("Acme", "Mira")):
                for kind in ("docx", "pdf"):
                    text = self.check_output(paths[kind], [f"Dear {customer},"])[0]
                    self.assertNotIn("{{", text)
                    self.assertNotIn("Dear " + ("Mira" if customer == "Acme" else "Acme"), text)
                    with doc_ex.open(paths[kind]) as reopened:
                        if kind == "docx":
                            for story in reopened.stories:
                                self.assertNotIn("{{", story.get_text())
                            page = reopened[0]
                            self.check_png(paths["png"].read_bytes(),
                                           (page.metadata["width"], page.metadata["height"]), 96)
                        else:
                            self.assertIn("Prepared by Example Ltd", text)

    def test_examples_are_import_safe_and_cli_output_directory_is_explicit(self):
        names = ("letter", "invoice", "branded_report", "pdf_assembly", "slide_preview", "template_batch")
        with tempfile.TemporaryDirectory() as directory:
            working = Path(directory)
            for name in names:
                with self.subTest(recipe=name):
                    # Run the module under a non-main name in an otherwise empty directory.
                    process = subprocess.run([sys.executable, "-c",
                                              "import runpy,sys; runpy.run_path(sys.argv[1], run_name='recipe_import')",
                                              str(EXAMPLES / f"{name}.py")], cwd=working,
                                             capture_output=True, text=True, timeout=60)
                    self.assertEqual(process.returncode, 0, process.stderr)
                    self.assertFalse(list(working.iterdir()))
            for name in names:
                with self.subTest(cli=name):
                    target = working / name / "output"
                    process = subprocess.run([sys.executable, str(EXAMPLES / f"{name}.py"),
                                              str(target), "--dpi", "72"], cwd=working,
                                             capture_output=True, text=True, timeout=60)
                    self.assertEqual(process.returncode, 0, process.stderr)
                    self.assertTrue(list(target.glob("*.pdf")))
                    png = next(target.glob("*.png")).read_bytes()
                    ppm, _, unit = struct.unpack(">IIB", png_chunks(png)[b"pHYs"])
                    self.assertEqual(unit, 1)
                    self.assertAlmostEqual(ppm * 0.0254, 72, delta=0.013)
                    for output in target.iterdir():
                        if output.suffix in (".docx", ".pptx", ".pdf"):
                            with doc_ex.open(output) as reopened:
                                self.assertGreater(reopened.page_count, 0)


if __name__ == "__main__":
    unittest.main()
