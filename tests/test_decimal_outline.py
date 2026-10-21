from io import BytesIO
from pathlib import Path
import tempfile
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_list_start import numbering_by_text, W
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


class DecimalOutlineTests(unittest.TestCase):
    def test_hierarchy_interleaved_kinds_start_restart_xml_and_pdf(self):
        with doc_ex.new("docx") as document:
            paragraphs = []
            assignments = [
                ("Ordinary", "numbered", 0, {}),
                ("Root", "decimal_outline", 0, {"start": 7}),
                ("Child", "decimal_outline", 1, {}),
                ("Grandchild", "decimal_outline", 2, {}),
                ("Another ordinary", "numbered", 0, {}),
                ("Sibling", "decimal_outline", 1, {}),
                ("Next", "decimal_outline", 0, {}),
                ("Restart", "decimal_outline", 0, {"restart": True}),
                ("Restart child", "decimal_outline", 1, {}),
            ]
            for text, kind, level, options in assignments:
                paragraph = document.add_paragraph(text)
                paragraph.set_list(kind, level=level, **options)
                self.assertEqual(paragraph.list_style, (kind, level))
                paragraphs.append(paragraph)
            values = numbering_by_text(document.to_bytes())
            expected = ["1.", "7.", "7.1.", "7.1.1.", "2.", "7.2.", "8.", "1.", "1.1."]
            self.assertEqual([values[item[0]]["label"] for item in assignments], expected)
            self.assertEqual(values["Root"]["num_id"], values["Next"]["num_id"])
            self.assertNotEqual(values["Root"]["abstract_id"], values["Ordinary"]["abstract_id"])
            self.assertNotEqual(values["Root"]["abstract_id"], values["Restart"]["abstract_id"])
            paragraphs[1].set_list("decimal_outline")
            self.assertEqual(numbering_by_text(document.to_bytes())["Root"]["label"], "7.")
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                numbering = ET.fromstring(archive.read("word/numbering.xml"))
            abstract = next(item for item in numbering.findall(f"{W}abstractNum")
                            if int(item.get(f"{W}abstractNumId")) == values["Root"]["abstract_id"])
            self.assertEqual([level.find(f"{W}lvlText").get(f"{W}val") for level in abstract.findall(f"{W}lvl")],
                             ["".join(f"%{index}." for index in range(1, depth + 2)) for depth in range(9)])
            for dpi in (72, 144):
                png = document.render_page(0, dpi=dpi)
                self.assertTrue(png.startswith(b"\x89PNG"))
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = pdf.to_markdown()
                for marker in ("7.", "7.1.", "7.1.1.", "7.2.", "8."):
                    self.assertIn(marker, text)
                for label, *_ in assignments:
                    self.assertIn(label, text)
                self.assertTrue(pdf.render_page(0, dpi=144).startswith(b"\x89PNG"))

    def test_copy_rtl_empty_items_warm_cache_and_range_lifetime(self):
        with doc_ex.new("docx") as document:
            root = document.add_paragraph("Root")
            root.set_list("decimal_outline", start=7)
            child = document.add_paragraph("Child")
            child.set_list("decimal_outline", level=1)
            empty = document.add_paragraph("")
            empty.set_list("decimal_outline", level=2)
            original = document.to_bytes()
            png = document.render_page(0, dpi=72)
            selected = root[:]
            with document.copy() as copied:
                root.set_list("decimal_outline", start=20)
                with self.assertRaises(ValueError):
                    _ = selected.text
                self.assertNotEqual(document.render_page(0, dpi=72), png)
                self.assertEqual(copied.to_bytes(), original)
                self.assertEqual(copied.render_page(0, dpi=72), png)
                copied.paragraphs[0].format(direction="rtl")
                self.assertTrue(copied.to_bytes())
                self.assertNotEqual(copied.render_page(0, dpi=72), png)
                with doc_ex.open(copied.convert("pdf")) as pdf:
                    self.assertIn("Root", pdf.to_markdown())
                    self.assertIn("7", pdf.to_markdown())
                self.assertEqual(numbering_by_text(copied.to_bytes())["Root"]["label"], "7.")
            root.set_list(None)
            self.assertIsNone(root.list_style)
            self.assertEqual(root.text, "Root")
            child.set_list("numbered")
            self.assertEqual(child.list_style, ("numbered", 0))

    def test_scope_and_section_transitions_reject_without_mutation(self):
        with doc_ex.new("docx") as document:
            body = document.add_paragraph("Anchor")
            cell = document.add_table([["Cell"]])[0, 0]
            nested = cell.add_table([["Nested"]])[0, 0]
            header = document.sections[0].header().add_paragraph("Header")
            footer_cell = document.sections[0].footer().add_table([["Footer cell"]])[0, 0]
            note = document.notes.add(body[:], "Note")
            endnote = document.notes.add(body[:], "Endnote", kind="endnote")
            for paragraph in (cell.paragraphs[0], nested.paragraphs[0], header,
                              footer_cell.paragraphs[0], note.paragraphs[0], endnote.paragraphs[0]):
                before = document.to_bytes()
                selected = paragraph[:]
                with self.assertRaisesRegex(UnsupportedFormatError, "top-level body"):
                    paragraph.set_list("decimal_outline")
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(selected.text, paragraph.text)
            body.set_list("decimal_outline")
            before = document.to_bytes()
            selected = body[:]
            selected_text = selected.text
            for change in (lambda: document.sections.add(start="continuous"),):
                with self.assertRaisesRegex(UnsupportedFormatError, "single-column"):
                    change()
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(selected.text, selected_text)
            second = document.sections.add()
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                second.start = "continuous"
            self.assertEqual(document.to_bytes(), before)
            # A separate unnumbered section may share its own page later.
            document.sections.add(start="continuous")
            self.assertTrue(document.to_bytes())
        with doc_ex.new("docx") as document:
            item = document.add_paragraph("Single-column item")
            item.set_list("decimal_outline")
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsupportedFormatError, "single-column"):
                document.sections[0].columns = (2, 20, False)
            self.assertEqual(document.to_bytes(), before)
        with doc_ex.new("docx") as document:
            earlier = document.add_paragraph("Earlier")
            document.sections.add(start="continuous")
            current = document.add_paragraph("Current")
            before = document.to_bytes()
            for paragraph in (earlier, current):
                with self.assertRaises(UnsupportedFormatError):
                    paragraph.set_list("decimal_outline")
                self.assertEqual(document.to_bytes(), before)

    def test_geometry_assignment_and_later_output_fail_safely_then_recover(self):
        with doc_ex.new("docx") as document:
            section = document.sections[0]
            section.margins = 18
            section.page_size = (144, 300)
            paragraph = document.add_paragraph("Narrow")
            selected = paragraph[:]
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsupportedFormatError, "body area"):
                paragraph.set_list("decimal_outline", level=8)
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(selected.text, "Narrow")
            section.page_size = (612, 792)
            section.margins = 72
            paragraph.set_list("decimal_outline", level=8)
            good = document.to_bytes()
            document.render_page(0, dpi=72)
            paragraph.format(indent_left=600)
            with tempfile.TemporaryDirectory() as directory:
                target = Path(directory) / "existing.docx"
                target.write_bytes(b"existing")
                for action in (document.to_bytes, lambda: document.render_page(0, dpi=72),
                               lambda: document.convert("pdf"), lambda: document.save(target)):
                    with self.assertRaisesRegex(UnsupportedFormatError, "body area"):
                        action()
                self.assertEqual(target.read_bytes(), b"existing")
            paragraph.format(indent_left=None)
            self.assertEqual(document.to_bytes(), good)
            paragraph[:].format(font_size=200)
            with self.assertRaises(UnsupportedFormatError):
                document.to_bytes()
            paragraph[:].format(font_size=None)
            self.assertTrue(document.to_bytes())
            section.margins = (72, 250, 72, 250)
            with self.assertRaises(UnsupportedFormatError):
                document.to_bytes()
            section.margins = 72
            self.assertTrue(document.to_bytes())

    def test_all_nine_levels_maximum_start_have_visible_native_pdf_ink(self):
        for direction in ("ltr", "rtl"):
            with self.subTest(direction=direction), doc_ex.new("docx") as document:
                # Fix the measured font instead of relying on installed substitutes.
                document.styles["Normal"].format(font="Source Sans 3")
                paragraphs = [document.add_paragraph("") for _ in range(9)]
                for paragraph in paragraphs:
                    paragraph.format(direction=direction)
                _, blank = rgba_pixels(png_chunks(document.render_page(0, dpi=72)))
                self.assertTrue(all(value == 255 for value in blank))
                for level, paragraph in enumerate(paragraphs):
                    paragraph.set_list("decimal_outline", level=level, start=32767 if level == 0 else None)
                    self.assertEqual(paragraph.list_style, ("decimal_outline", level))
                with doc_ex.open(document.convert("pdf")) as pdf:
                    self.assertEqual(pdf.page_count, 1)
                    for dpi in (72, 144):
                        width, source = rgba_pixels(png_chunks(document.render_page(0, dpi=dpi)))
                        pdf_width, exported = rgba_pixels(png_chunks(pdf.render_page(0, dpi=dpi)))
                        self.assertEqual(width, pdf_width)
                        extents = []
                        for pixels in (source, exported):
                            points = [(index // 4 % width, index // 4 // width)
                                      for index in range(0, len(pixels), 4)
                                      if max(pixels[index:index + 3]) < 128 and pixels[index + 3] > 0]
                            self.assertGreater(len(points), 100)
                            extents.append((len(points), min(x for x, _ in points), max(x for x, _ in points),
                                            min(y for _, y in points), max(y for _, y in points)))
                        self.assertEqual(len(source), len(exported))
                        self.assertEqual(extents[0][1:], extents[1][1:])
                        # Native source/PDF rasterizers can quantize grayscale coverage
                        # one channel value apart (CI observed 128 versus 127). Compare
                        # every channel instead of counts across that hard threshold.
                        self.assertLessEqual(max(abs(a - b) for a, b in zip(source, exported)), 1)
                        if direction == "ltr":
                            self.assertLess(extents[0][1], 100 * dpi / 72)
                        else:
                            self.assertGreater(extents[0][2], 500 * dpi / 72)
                with doc_ex.open(document.to_bytes()) as source:
                    self.assertEqual(source.page_count, 1)
                    labels = [block.metadata["numbering"]["label"] for page in source
                              for block in page.blocks if "numbering" in block.metadata]
                    self.assertEqual(labels, ["32767." + "1." * level for level in range(9)])

    def test_wrapped_item_has_one_native_marker_across_pages(self):
        with doc_ex.new("docx") as document:
            document.sections[0].margins = 18
            document.sections[0].page_size = (180, 240)
            text = "Wrapped " * 80 + "Tail"
            paragraph = document.add_paragraph(text)
            paragraph.set_list("decimal_outline", start=7)
            self.assertGreater(document.page_count, 1)
            with doc_ex.open(document.convert("pdf")) as pdf:
                output = pdf.to_markdown()
                self.assertEqual(output.count("7."), 1)
                self.assertEqual(output.count("Wrapped"), 80)
                self.assertIn("Tail", output)
                self.assertEqual(pdf.page_count, document.page_count)
            for page in range(document.page_count):
                self.assertTrue(document.render_page(page, dpi=72).startswith(b"\x89PNG"))

    def test_validation_budgets_and_reentrant_callbacks_preserve_targets(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Stable")
            before = document.to_bytes()
            for kind, options in [("legal", {}), ("multilevel", {}),
                                  ("decimal_outline", {"level": 1, "start": 7}),
                                  ("decimal_outline", {"start": True}),
                                  ("decimal_outline", {"start": 32768})]:
                with self.assertRaises((ValueError, TypeError)):
                    paragraph.set_list(kind, **options)
                self.assertEqual(document.to_bytes(), before)
            long = document.add_paragraph("x" * 25000)
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsafeArchiveError, "layout-work"):
                long.set_list("decimal_outline")
            self.assertEqual(document.to_bytes(), before)
            callback_bytes = []

            class ChangeSection:
                def __index__(self):
                    document.sections[0].columns = (2, 20, False)
                    callback_bytes.append(document.to_bytes())
                    return 7

            with self.assertRaises(UnsupportedFormatError):
                paragraph.set_list("decimal_outline", start=ChangeSection())
            self.assertEqual(document.to_bytes(), callback_bytes[0])
            self.assertIsNone(paragraph.list_style)

            class CloseOwner:
                def __index__(self):
                    document.close()
                    return 0

            with self.assertRaises(DocumentClosedError):
                paragraph.set_list("decimal_outline", level=CloseOwner())
            with self.assertRaises(DocumentClosedError):
                _ = paragraph.list_style


if __name__ == "__main__":
    unittest.main()
