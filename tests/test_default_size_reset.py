"""Existing formatter reset: native inheritance, output, ownership and rollback."""
from contextlib import ExitStack
from io import BytesIO
import math
import struct
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_authoring import image_fixture
from test_docx_default_size_upgrade import imported_size
from test_picture_media import media_parts
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_styles import NS

URI = "https://example.test/default-reset"
LABELS = ("BODYKEEP", "HEADERKEEP", "CELLKEEP", "STYLEKEEP", "DIRECTKEEP")
ROLE_COLORS = {
    "body": (255, 0, 0), "header": (0, 0, 128), "cell": (0, 128, 0),
    "style": (128, 0, 128), "direct": (255, 128, 0),
}


def owner(size):
    d = doc_ex.new("docx")
    section = d.sections[0]
    section.page_size = (240, 220)
    section.margins = (32, 12, 12, 12)
    section.header_distance = 10
    d.styles.format_defaults(font="DejaVu Sans", font_size=size, color="red",
                             bold=True, space_before=0, space_after=0, line_spacing=1)
    style = d.styles.add("Inherited")
    body = d.add_paragraph("BODYKEEP", style=style)
    body[:].link = URI
    # The native Hyperlink style supplies its own color. Keep font size inherited
    # while making this carrier's ink independently identifiable.
    body[:].format(color="red")
    section.header().add_paragraph("HEADERKEEP")[:].format(color="navy")
    d.add_table([["CELLKEEP"]])[0, 0].paragraphs[0][:].format(color="green")
    fixed = d.styles.add("Fixed", based_on=style)
    fixed.format(font_size=18, color="#800080")
    d.add_paragraph("STYLEKEEP", style=fixed)
    d.add_paragraph().add_run("DIRECTKEEP", font_size=15, color="#FF8000")
    d.add_image(image_fixture(4, 2), width=20, height=10)
    return d


def pixels(test, png, dpi):
    chunks = png_chunks(png)
    w, h = struct.unpack(">II", chunks[b"IHDR"][:8])
    test.assertEqual((w, h), (math.ceil(240 * dpi / 72), math.ceil(220 * dpi / 72)))
    width, rgba = rgba_pixels(chunks)
    test.assertEqual(width, w)
    test.assertEqual(len(rgba), w * h * 4)
    result = {}
    positions = {rgb: [] for rgb in ROLE_COLORS.values()}
    for offset in range(0, len(rgba), 4):
        rgb = tuple(rgba[offset:offset + 3])
        if rgb in positions and rgba[offset + 3] == 255:
            pos = offset // 4
            positions[rgb].append((pos % w, pos // w))
    for role, rgb in ROLE_COLORS.items():
        points = positions[rgb]
        test.assertGreater(len(points), 20, (dpi, role))
        xs, ys = zip(*points)
        result[role] = (min(xs), min(ys), max(xs) + 1, max(ys) + 1)
        test.assertGreater(min(xs), 0)
        test.assertGreater(min(ys), 0)
        test.assertLess(max(xs) + 1, w)
        test.assertLess(max(ys) + 1, h)
    test.assertGreater(sum(rgba[i:i + 4] == b"\x20\x60\xc0\xff"
                           for i in range(0, len(rgba), 4)), 20)
    return result


class DefaultSizeResetTests(unittest.TestCase):
    def test_reset_inherits_eleven_preserves_overrides_parts_copy_and_visible_outputs(self):
        with ExitStack() as stack:
            d = stack.enter_context(owner(10))
            control = stack.enter_context(owner(11))
            copied = stack.enter_context(d.copy())
            defaults_before = d.styles.defaults
            media_before = media_parts(d)
            warm = d.render_page(0, dpi=144)
            original_read = stack.enter_context(doc_ex.open(d.to_bytes()))
            warm_blocks = original_read[0].get_text("dict")["blocks"]
            d.styles.format_defaults(font_size=None)
            self.assertIsNone(d.styles.defaults["font_size"])
            self.assertEqual(defaults_before["font_size"], 10)
            self.assertEqual(d.styles["Inherited"].resolved_formatting["font_size"], 11)
            self.assertEqual(d.styles["Fixed"].resolved_formatting["font_size"], 18)
            self.assertEqual(copied.styles.defaults["font_size"], 10)
            self.assertEqual(copied.render_page(0, dpi=144), warm)
            self.assertNotEqual(d.render_page(0, dpi=144), warm)
            self.assertTrue(warm_blocks)
            self.assertEqual(media_parts(d), media_before)
            with ZipFile(BytesIO(d.to_bytes())) as archive:
                styles = ET.fromstring(archive.read("word/styles.xml"))
                # WordCraft writes the resolver's fallback (11 pt) for an unset default, so the file
                # states the size the document resolves instead of leaving it to the reader's 10 pt.
                size = styles.find("w:docDefaults/w:rPrDefault/w:rPr/w:sz", NS)
                self.assertEqual(size.get(f"{{{NS['w']}}}val"), "22")
                body = ET.fromstring(archive.read("word/document.xml"))
                direct = [run for run in body.findall(".//w:r", NS)
                          if run.findtext("w:t", namespaces=NS) == "DIRECTKEEP"]
                self.assertEqual(len(direct), 1)
                self.assertEqual(direct[0].find("w:rPr/w:sz", NS).get(f"{{{NS['w']}}}val"), "30")
            reopened = stack.enter_context(doc_ex.open(d.to_bytes()))
            pdf = stack.enter_context(doc_ex.open(d.convert("pdf")))
            control_read = stack.enter_context(doc_ex.open(control.to_bytes()))
            for owned in (d, control):
                self.assertEqual(owned.page_count, 1)
                for label in LABELS:
                    self.assertEqual(owned.to_markdown().count(label), 0 if label == "HEADERKEEP" else 1)
                self.assertEqual(sum(p.text.count("HEADERKEEP")
                                     for p in owned.sections[0].header().paragraphs), 1)
            for source in (control_read, reopened, pdf):
                self.assertEqual(len(source), 1)
                text = source[0].get_text()
                for label in LABELS:
                    self.assertEqual(text.count(label), 0 if label == "HEADERKEEP" and source is not pdf else 1)
                if source is not pdf:
                    headers = [story for story in source.stories if story.kind == "header"]
                    self.assertEqual(len(headers), 1)
                    self.assertEqual(headers[0].get_text().count("HEADERKEEP"), 1)
                self.assertEqual([link.uri for link in source[0].get_links()], [URI])
                self.assertEqual(len(source[0].get_images()), 1)
            for dpi in (72, 144, 300):
                source_png = d.render_page(0, dpi=dpi)
                self.assertEqual(source_png, control.render_page(0, dpi=dpi))
                self.assertEqual(source_png, reopened.render_page(0, dpi=dpi))
                a = pixels(self, source_png, dpi)
                b = pixels(self, pdf.render_page(0, dpi=dpi), dpi)
                for color in a:
                    for source, exported in zip(a[color], b[color]):
                        self.assertLessEqual(abs(source - exported), 2, (dpi, color, a[color], b[color]))
            # Reset removes a direct value; it does not recreate initial12pt defaults.
            no_styles = stack.enter_context(doc_ex.open(imported_size(None, styles=False)))
            twelve = stack.enter_context(doc_ex.open(imported_size(12)))
            eleven = stack.enter_context(doc_ex.open(imported_size(11)))
            self.assertEqual(no_styles.render_page(0, dpi=72), twelve.render_page(0, dpi=72))
            self.assertNotEqual(no_styles.render_page(0, dpi=72), eleven.render_page(0, dpi=72))
            before = d.to_bytes()
            d.styles.format_defaults(font_size=None)
            self.assertEqual(d.to_bytes(), before)
            copied.styles.format_defaults(font_size=12)
            self.assertIsNone(d.styles.defaults["font_size"])

    def test_invalid_combination_warm_rollback_and_existing_normalization_callbacks(self):
        with owner(10) as d:
            before = d.to_bytes(), d.styles.defaults, d.render_page(0, dpi=72)
            for patch in (
                {"font_size": None, "bold": "wrong"},
                {"font_size": None, "tabs": [{"position": 10000}]},
                {"font_size": True},
                {"font_size": float("inf")},
            ):
                with self.assertRaises((ValueError, TypeError)):
                    d.styles.format_defaults(**patch)
                self.assertEqual((d.to_bytes(), d.styles.defaults, d.render_page(0, dpi=72)), before)
            class EditedChannel(int):
                def __format__(self, spec):
                    d.add_paragraph("CALLBACKKEEP")
                    return int.__format__(self, spec)
            d.styles.format_defaults(font_size=None, color=(EditedChannel(255), 0, 0))
            self.assertIsNone(d.styles.defaults["font_size"])
            self.assertEqual(d.to_markdown().count("CALLBACKKEEP"), 1)
        with doc_ex.new("docx") as d:
            d.styles.format_defaults(font_size=10)
            with d.copy() as copy:
                class ClosedChannel(int):
                    def __format__(self, spec):
                        d.close()
                        return int.__format__(self, spec)
                with self.assertRaises(DocumentClosedError):
                    d.styles.format_defaults(font_size=None, color=(ClosedChannel(255), 0, 0))
                self.assertEqual(copy.styles.defaults["font_size"], 10)
                with self.assertRaises(DocumentClosedError):
                    d.styles.format_defaults(font_size=None)
