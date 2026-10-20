import copy
import io
import random
import struct
import unittest
import xml.etree.ElementTree as ET
import zipfile
import zlib

import doc_ex
from doc_ex import _core, _native
from doc_ex.errors import UnsafeArchiveError
from test_authoring import image_fixture
from test_pdf_text_boxes import pixel

P = "{http://schemas.openxmlformats.org/presentationml/2006/main}"


def repeated_slides(count, *, relocate=False, encoding="utf-8", prefix="", extras=(), alternate=False):
    with doc_ex.new("pptx", size=(120, 100)) as document:
        document.slides.add().add_text("Counted once", box=(5, 5, 110, 40), font_size=12)
        original = document.to_bytes()
    result = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(original)) as source, zipfile.ZipFile(result, "w", compression=zipfile.ZIP_STORED) as target:
        for entry in source.infolist():
            name, data = entry.filename, source.read(entry.filename)
            if name == "ppt/presentation.xml":
                root = ET.fromstring(data)
                slides = root.find(f"{P}sldIdLst")
                first = copy.deepcopy(slides[0])
                slides.clear()
                for index in range(count):
                    slide = copy.deepcopy(first)
                    slide.set("id", str(index + 256))
                    slides.append(slide)  # Every entry intentionally references the same slide part.
                if alternate:
                    root.remove(slides)
                    mc = "{http://schemas.openxmlformats.org/markup-compatibility/2006}"
                    branch = ET.SubElement(ET.SubElement(root, f"{mc}AlternateContent"), f"{mc}Choice", {"Requires": ""})
                    branch.append(slides)
                declaration = '<?xml version="1.0" encoding="UTF-16"?>' if encoding.startswith("utf-16") else ''
                text = declaration + prefix + ET.tostring(root, encoding="unicode")
                data = text.encode(encoding)
                if encoding == "utf-16-le":
                    data = b"\xff\xfe" + data
                elif encoding == "utf-16-be":
                    data = b"\xfe\xff" + data
                if relocate:
                    name = "unusual/main.bin"
            elif relocate and name == "_rels/.rels":
                root = ET.fromstring(data)
                for relation in root:
                    if relation.get("Type", "").endswith("/officeDocument"):
                        relation.set("Target", "/unusual/main.bin")
                data = ET.tostring(root)
            elif relocate and name == "[Content_Types].xml":
                root = ET.fromstring(data)
                for item in root:
                    if item.get("PartName") == "/ppt/presentation.xml":
                        item.set("PartName", "/unusual/main.bin")
                data = ET.tostring(root)
            elif relocate and name == "ppt/_rels/presentation.xml.rels":
                root = ET.fromstring(data)
                for relation in root:
                    if relation.get("TargetMode") != "External" and not relation.get("Target", "").startswith("/"):
                        relation.set("Target", "../ppt/" + relation.get("Target"))
                data = ET.tostring(root)
                name = "unusual/_rels/main.bin.rels"
            target.writestr(name, data)
        for name, data in extras:
            target.writestr(name, data)
    return result.getvalue()


class SlideLimitTests(unittest.TestCase):
    def test_repeated_source_references_are_counted_before_native_truncation(self):
        accepted = repeated_slides(10_000)
        with doc_ex.open(accepted) as document:
            self.assertEqual(len(document), 10_000)
            self.assertEqual(document[-1].get_text().count("Counted once"), 1)
            self.assertTrue(document[-1].get_pixmap(dpi=72).startswith(b"\x89PNG"))
        rejected = repeated_slides(10_001)
        for operation in (lambda: doc_ex.open(rejected),
                          lambda: doc_ex.convert(rejected, "pdf"),
                          # Rendering normally follows open(); exercise the native
                          # render entry directly to prove it shares the preflight.
                          lambda: _native(lambda: _core.render(rejected, "pptx", 0, 72, None))):
            with self.assertRaisesRegex(UnsafeArchiveError, "source exceeds 10000 slide entries"):
                operation()

    def test_relocated_native_utf16_and_leading_text_roots_cannot_bypass_limit(self):
        cases = (("utf-8", " \n\t"), ("utf-8-sig", ""),
                 ("utf-16-le", ""), ("utf-16-be", ""),
                 ("utf-8", "native accepts leading text\n"))
        for encoding, prefix in cases:
            with self.subTest(encoding=encoding, prefix=prefix):
                with doc_ex.open(repeated_slides(1, relocate=True, encoding=encoding, prefix=prefix)) as document:
                    self.assertEqual(len(document), 1)
                    self.assertIn("Counted once", document[0].get_text())
                rejected = repeated_slides(10_001, relocate=True, encoding=encoding, prefix=prefix)
                with self.assertRaisesRegex(UnsafeArchiveError, "source exceeds 10000 slide entries"):
                    doc_ex.open(rejected)

    def test_native_markup_compatibility_promotion_cannot_bypass_source_count(self):
        with doc_ex.open(repeated_slides(1, alternate=True)) as document:
            self.assertEqual(len(document), 1)
            self.assertIn("Counted once", document[0].get_text())
        with self.assertRaisesRegex(UnsafeArchiveError, "source exceeds 10000 slide entries"):
            doc_ex.open(repeated_slides(10_001, alternate=True))

    def test_large_binary_and_png_media_pass_but_unused_presentation_is_bounded(self):
        padding = b"\xff" * (2 * 1024 * 1024)
        kind = b"caAs"
        png_padding = random.Random(17).randbytes(len(padding))
        chunk = struct.pack(">I", len(png_padding)) + kind + png_padding + struct.pack(">I", zlib.crc32(kind + png_padding))
        small_png = image_fixture()
        large_png = small_png[:-12] + chunk + small_png[-12:]
        with doc_ex.new("pptx", size=(120, 100)) as authored:
            authored.slides.add().add_image(large_png, box=(10, 10, 80, 40))
            with doc_ex.open(authored.to_bytes()) as document:
                self.assertEqual(pixel(document.render_page(0, dpi=72), 40, 30), b"\x20\x60\xc0\xff")
        data = repeated_slides(1, extras=(("ppt/media/blob.bin", padding), ("ppt/media/large.png", large_png)))
        with doc_ex.open(data) as document:
            self.assertEqual(len(document), 1)
            self.assertEqual(document[0].get_images(), [])
            self.assertTrue(document.render_page(0, dpi=72))
            with doc_ex.open(document.convert("pdf")) as pdf:
                self.assertIn("Counted once", pdf[0].get_text())
        unused = b"<presentation><sldIdLst>" + b"<sldId/>" * 10_001 + b"</sldIdLst></presentation>"
        with self.assertRaisesRegex(UnsafeArchiveError, "source exceeds 10000 slide entries"):
            doc_ex.open(repeated_slides(1, extras=(("unused/presentation.bin", unused),)))

    def test_presentation_prolog_and_token_budgets_return_explicit_errors(self):
        for prefix, expected in ((" " * (64 * 1024 + 1), "prolog exceeds 64 KiB"),
                                 ("", "token exceeds 1 MiB")):
            data = repeated_slides(1, prefix=prefix)
            if not prefix:
                result = io.BytesIO()
                with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(result, "w") as target:
                    for entry in source.infolist():
                        value = source.read(entry.filename)
                        if entry.filename == "ppt/presentation.xml":
                            root = ET.fromstring(value)
                            ET.SubElement(root, f"{P}extLst").text = "x" * (1024 * 1024 + 1)
                            value = ET.tostring(root)
                        target.writestr(entry.filename, value)
                data = result.getvalue()
            with self.subTest(expected=expected), self.assertRaisesRegex(UnsafeArchiveError, expected):
                doc_ex.open(data)


if __name__ == "__main__":
    unittest.main()
