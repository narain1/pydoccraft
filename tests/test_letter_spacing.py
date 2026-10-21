"""Native character spacing: owned formatting, native clusters, and output fidelity."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_direction_language import colored_extent
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def parts(document):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        return {name: ET.fromstring(archive.read(name)) for name in archive.namelist()
                if name.startswith("word/") and name.endswith(".xml")}


def spacing_values(document, part="word/document.xml"):
    return [node.get(W + "val") for node in parts(document)[part].iter(W + "spacing")
            if node.get(W + "val") is not None]


class LetterSpacingTests(unittest.TestCase):
    def assert_pixel_parity(self, document, dpi=144, *, red_text_only=False):
        with doc_ex.open(document.convert("pdf")) as pdf:
            self.assertEqual(document.page_count, pdf.page_count)
            for page in range(document.page_count):
                source_png = document.render_page(page, dpi=dpi)
                pdf_png = pdf.render_page(page, dpi=dpi)
                a_width, a = rgba_pixels(png_chunks(source_png))
                b_width, b = rgba_pixels(png_chunks(pdf_png))
                self.assertEqual((a_width, len(a)), (b_width, len(b)))
                if red_text_only:
                    # Native grid/separator strokes differ even at zero spacing.
                    # Explicit cell padding keeps glyphs off those strokes; compare
                    # every chromatic pixel without claiming structural-stroke parity.
                    self.assertEqual(colored_extent(source_png)[:4], colored_extent(pdf_png)[:4])
                    offsets = [i for i in range(0, len(a), 4)
                               if a[i] > a[i + 1] or b[i] > b[i + 1]]
                    self.assertGreater(len(offsets), 100)
                    self.assertLessEqual(max(abs(a[i + channel] - b[i + channel])
                                             for i in offsets for channel in range(4)), 1)
                else:
                    self.assertLessEqual(max(abs(x - y) for x, y in zip(a, b)), 1)

    def test_visible_expanded_condensed_unicode_and_native_pdf_output(self):
        for text, direction in [("e\u0301 office", "ltr"), ("سلام Word", "rtl")]:
            with self.subTest(direction=direction), doc_ex.new("docx") as document:
                document.sections[0].page_size = (400, 250)
                document.sections[0].margins = 30
                document.styles.format_defaults(color="red", font_size=24)
                paragraph = document.add_paragraph(text)
                paragraph.format(direction=direction)
                baseline = colored_extent(document.render_page(0, dpi=144))
                paragraph[:].format(letter_spacing=2)
                expanded = colored_extent(document.render_page(0, dpi=144))
                self.assertGreater(expanded[1] - expanded[0], baseline[1] - baseline[0])
                self.assert_pixel_parity(document)
                paragraph[:].format(letter_spacing=-1)
                condensed = colored_extent(document.render_page(0, dpi=144))
                self.assertLess(condensed[1] - condensed[0], baseline[1] - baseline[0])
                self.assert_pixel_parity(document, dpi=72)
                self.assertEqual(paragraph.text, text)
                with doc_ex.open(document.to_bytes()) as source:
                    self.assertEqual(source[0].get_text(), text)
                paragraph[:].format(letter_spacing=None)
                self.assertEqual(colored_extent(document.render_page(0, dpi=144)), baseline)

    def test_twip_precision_default_style_direct_zero_and_reset(self):
        with doc_ex.new("docx") as document:
            document.styles.format_defaults(letter_spacing=0.1 + 0.2)
            self.assertEqual(document.styles.defaults["letter_spacing"], 0.3)
            base = document.styles.add("Expanded", based_on="Normal")
            base.format(letter_spacing=2)
            child = document.styles.add("Inherited expanded", based_on=base)
            character = document.styles.add("Condensed", kind="character")
            character.format(letter_spacing=-0.1 - 0.2)
            self.assertEqual(character.formatting["letter_spacing"], -0.3)
            self.assertIsNone(child.formatting["letter_spacing"])
            self.assertEqual(child.resolved_formatting["letter_spacing"], 2)
            paragraph = document.add_paragraph("", style=child)
            selected = paragraph.add_run("Styled", style=character.id, letter_spacing=0)
            self.assertEqual(spacing_values(document), ["0"])
            selected.format(letter_spacing=None)
            self.assertEqual(spacing_values(document), [])
            self.assertEqual(character.resolved_formatting["letter_spacing"], -0.3)
            base.format(letter_spacing=0)
            self.assertEqual(child.resolved_formatting["letter_spacing"], 0)
            base.format(letter_spacing=None)
            self.assertEqual(child.resolved_formatting["letter_spacing"], 0.3)
            document.styles.format_defaults(letter_spacing=None)
            self.assertIsNone(document.styles.defaults["letter_spacing"])
            self.assertEqual(child.resolved_formatting["letter_spacing"], 0)
            for value in (-100, -0.05, 0.05, 100):
                character.format(letter_spacing=value)
                self.assertEqual(character.formatting["letter_spacing"], value)
                xml = parts(document)["word/styles.xml"]
                style = next(s for s in xml.findall(W + "style") if s.get(W + "styleId") == character.id)
                self.assertEqual(style.find(f"{W}rPr/{W}spacing").get(W + "val"), str(round(value * 20)))

    def test_header_footer_notes_nested_cell_and_conditional_style_scopes(self):
        with doc_ex.new("docx") as document:
            anchor = document.add_paragraph("BodyAnchor")
            header = document.sections[0].header().add_paragraph("")
            footer = document.sections[0].footer().add_paragraph("")
            nested_cell = document.add_table([[""]])[0, 0].add_table([[""]])[0, 0]
            # A zero-bottom-margin descender can intersect the native table rule.
            # Keep the border and every text assertion, with explicit glyph clearance.
            top, right, _, left = nested_cell.margins
            nested_cell.margins = (top, right, 3, left)
            nested = nested_cell.paragraphs[0]
            foot = document.notes.add(anchor.find("Body")[0], "").paragraphs[0]
            end = document.notes.add(anchor.find("Anchor")[0], "", kind="endnote").paragraphs[0]
            for index, paragraph in enumerate([header, footer, nested, foot, end]):
                paragraph.add_run(f"Scope{index}", letter_spacing=-0.05 if index % 2 else 0.3, color="red")
            xmls = parts(document)
            for index in range(5):
                paragraph = next(p for xml in xmls.values() for p in xml.iter(W + "p")
                                 if f"Scope{index}" in "".join(p.itertext()))
                self.assertEqual(paragraph.find(f".//{W}rPr/{W}spacing").get(W + "val"), "-1" if index % 2 else "6")
            table_style = document.styles.add("Regional spacing", kind="table")
            table_style.format(letter_spacing=1, color="red")
            table_style.format(region="first_row", letter_spacing=3)
            table = document.add_table([["Header"], ["Body"]])
            table.format(style=table_style, look={"header_row": True})
            self.assertEqual(table_style.resolved_formatting["regions"]["first_row"]["letter_spacing"], 3)
            widths = table.column_widths
            selected = table[0, 0].paragraphs[0][:]
            selected.format(letter_spacing=0)
            selected.format(letter_spacing=None)
            self.assertEqual(table.column_widths, widths)
            self.assertEqual([table[row, 0].text for row in range(2)], ["Header", "Body"])
            self.assert_pixel_parity(document, dpi=72, red_text_only=True)
            # A fractional origin also exercised the previous descender/rule overlap.
            top, right, bottom, left = document.sections[0].margins
            document.sections[0].margins = (top + 0.3, right, bottom, left)
            self.assert_pixel_parity(document, dpi=72, red_text_only=True)
            with doc_ex.open(document.to_bytes()) as source:
                text = source.to_markdown() + "\n" + "\n".join(s.get_text() for s in source.stories)
                for index in range(5):
                    self.assertIn(f"Scope{index}", text)

    def test_owned_ranges_copy_and_invalid_patches_are_atomic(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Owned")
            old = paragraph[:]
            selected = paragraph[:]
            selected.format(letter_spacing=0.3)
            with self.assertRaises(ValueError):
                _ = old.text
            self.assertEqual(selected.text, "Owned")
            frozen = document.to_bytes()
            cached = document.render_page(0, dpi=72)
            for invalid in (True, "1", float("nan"), float("inf"), -100.05, 100.05, 0.301, -0.001):
                with self.subTest(invalid=invalid):
                    with self.assertRaises((ValueError, TypeError)):
                        selected.format(bold=True, letter_spacing=invalid)
                    self.assertEqual(selected.text, "Owned")
                    self.assertEqual(document.to_bytes(), frozen)
                    self.assertEqual(document.render_page(0, dpi=72), cached)
            with document.copy() as copied:
                copied.paragraphs[0][:].format(letter_spacing=-1)
                self.assertEqual(spacing_values(document), ["6", "6"])
                self.assertEqual(spacing_values(copied), ["-20", "-20"])
                document.close()
                with self.assertRaises(DocumentClosedError):
                    selected.format(letter_spacing=None)
                self.assertEqual(copied.paragraphs[0].text, "Owned")
                self.assert_pixel_parity(copied, dpi=72)

    def test_callback_staleness_and_navigation_guards_remain_active(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Keep")
            selected = paragraph[:]
            class MutatingSpacing(str):
                def __len__(self):
                    paragraph.add_run(" caller")
                    return super().__len__()
            with self.assertRaisesRegex(ValueError, "stale"):
                selected.format(letter_spacing=MutatingSpacing("1"), bold=True)
            self.assertEqual(paragraph.text, "Keep caller")
            self.assertEqual(spacing_values(document), [])
            selected = paragraph[:]
            class ClosingSpacing(str):
                def __len__(self):
                    document.close()
                    return super().__len__()
            with self.assertRaises(DocumentClosedError):
                selected.format(letter_spacing=ClosingSpacing("1"))
        with doc_ex.new("docx") as document:
            source = document.add_paragraph("Jump to target")
            document.add_page_break()
            target = document.add_paragraph("Destination")
            document.bookmarks["target"] = target[:]
            source[:].link = "#target"
            for spacing in (-1, 0.3):
                source[:].format(letter_spacing=spacing)
                target[:].format(letter_spacing=spacing)
                with doc_ex.open(document.convert("pdf")) as pdf:
                    links = pdf[0].get_links()
                    self.assertTrue(any(link.metadata.get("target_page") == 1 for link in links))
                self.assertIn("Destination", document.bookmarks["target"].text)

    def test_shared_character_formatting_rejects_unsupported_pptx(self):
        with doc_ex.new("pptx") as presentation:
            slide = presentation.slides.add()
            shape = slide.add_text("Text", box=(20, 20, 200, 40))
            cell = slide.add_table([["Cell"]], box=(20, 80, 200, 80))[0, 0].paragraphs[0]
            for paragraph in (shape, cell):
                selected = paragraph[:]
                before = presentation.to_bytes()
                for spacing in (None, 0, -0.05, 1):
                    with self.assertRaisesRegex(ValueError, "unsupported PPTX formatting property: letter_spacing"):
                        selected.format(bold=True, letter_spacing=spacing)
                    with self.assertRaisesRegex(ValueError, "unsupported PPTX formatting property: letter_spacing"):
                        paragraph.add_run("Extra", letter_spacing=spacing)
                    self.assertEqual(presentation.to_bytes(), before)
                    self.assertEqual(selected.text, paragraph.text)


if __name__ == "__main__":
    unittest.main()
