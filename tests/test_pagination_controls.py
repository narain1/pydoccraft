import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

import doc_ex
from test_rendering import has_visible_content, png_chunks


class PaginationControlTests(unittest.TestCase):
    def prepare(self, document, filler=0):
        document.sections[0].margins = 30
        document.styles["Normal"].format(font="Source Sans 3")
        style = document.styles.add("Measured lines")
        style.format(font_size=12, line_spacing=1, space_before=0, space_after=0)
        for index in range(filler):
            document.add_paragraph(f"Filler{index:02}", style=style)
        return style

    def exported_pages(self, document, labels):
        """Check rendered output as well as text retained in PDF content streams."""
        with doc_ex.open(document.to_bytes()) as native, doc_ex.open(document.convert("pdf")) as pdf:
            self.assertEqual(native.page_count, pdf.page_count)
            pages = [page.get_text() for page in pdf]
            joined = "\n".join(pages)
            for label in labels:
                self.assertEqual(joined.count(label), 1)
                self.assertEqual(native.to_markdown().count(label), 1)
            positions = [joined.index(label) for label in labels]
            self.assertEqual(positions, sorted(positions))
            for index, page in enumerate(pdf):
                for block in page.blocks:
                    if block.bbox is not None and any(label in block.text for label in labels):
                        self.assertGreaterEqual(block.bbox[1], 0)
                        self.assertLessEqual(block.bbox[3], page.metadata["height"])
                for source in (native, pdf):
                    self.assertTrue(has_visible_content(png_chunks(source.render_page(index, dpi=72))))
            return pages

    def test_keep_with_next_moves_heading_with_following_table(self):
        for keep in (False, True):
            with self.subTest(keep=keep), doc_ex.new("docx", size=(300, 300)) as document:
                style = self.prepare(document, filler=16)
                heading = document.add_paragraph("TableHeading", style=style)
                heading.format(keep_with_next=keep)
                table = document.add_table([["TableBody"]])
                table.row(0).height = (30, "exact")
                table.row(0).allow_split = False
                pages = self.exported_pages(document, ["TableHeading", "TableBody"])
                self.assertEqual(len(pages), 2)
                self.assertIn("TableBody", pages[1])
                self.assertIn("TableHeading", pages[1 if keep else 0])
                self.assertNotIn("TableHeading", pages[0 if keep else 1])

    def test_keep_together_moves_a_paragraph_that_fits_on_a_fresh_page(self):
        labels = [f"LongLine{index:02}" for index in range(6)]
        for keep in (False, True):
            with self.subTest(keep=keep), doc_ex.new("docx", size=(300, 300)) as document:
                style = self.prepare(document, filler=14)
                paragraph = document.add_paragraph("\n".join(labels), style=style)
                paragraph.format(keep_together=keep, widow_control=False)
                pages = self.exported_pages(document, labels)
                self.assertEqual(len(pages), 2)
                counts = [sum(label in page for label in labels) for page in pages]
                self.assertEqual(counts, [0, 6] if keep else [3, 3])

    def test_widow_control_prevents_single_lines_on_either_side_of_a_break(self):
        for filler, count, unprotected, protected in ((0, 18, [17, 1], [16, 2]),
                                                       (16, 4, [1, 3], [0, 4])):
            labels = [f"LongLine{index:02}" for index in range(count)]
            for enabled in (False, True):
                with self.subTest(filler=filler, widow_control=enabled), doc_ex.new("docx", size=(300, 300)) as document:
                    style = self.prepare(document, filler=filler)
                    paragraph = document.add_paragraph("\n".join(labels), style=style)
                    paragraph.format(keep_together=False, widow_control=enabled)
                    pages = self.exported_pages(document, labels)
                    self.assertEqual([sum(label in page for label in labels) for page in pages],
                                     protected if enabled else unprotected)

    def test_page_break_before_moves_short_content_and_can_be_cleared(self):
        with doc_ex.new("docx", size=(300, 300)) as document:
            style = self.prepare(document)
            document.add_paragraph("BeforeBreak", style=style)
            paragraph = document.add_paragraph("AfterBreak", style=style)
            self.assertEqual(len(self.exported_pages(document, ["BeforeBreak", "AfterBreak"])), 1)
            paragraph.format(page_break_before=True)
            pages = self.exported_pages(document, ["BeforeBreak", "AfterBreak"])
            self.assertEqual(len(pages), 2)
            self.assertIn("BeforeBreak", pages[0])
            self.assertIn("AfterBreak", pages[1])
            paragraph.format(page_break_before=None)
            self.assertEqual(len(self.exported_pages(document, ["BeforeBreak", "AfterBreak"])), 1)

    def test_one_cell_paragraph_splits_or_moves_intact_according_to_row_control(self):
        labels = [f"CellLine{index:02}" for index in range(10)]
        for split in (True, False):
            with self.subTest(allow_split=split), doc_ex.new("docx", size=(300, 300)) as document:
                style = self.prepare(document, filler=10)
                table = document.add_table([["\n".join(labels)]])
                paragraph = table[0, 0].paragraphs[0]
                paragraph.style = style
                paragraph.format(keep_together=False, widow_control=False)
                table.row(0).allow_split = split
                document.add_paragraph("AfterTable", style=style)
                pages = self.exported_pages(document, labels + ["AfterTable"])
                self.assertEqual(len(pages), 2)
                counts = [sum(label in page for label in labels) for page in pages]
                self.assertEqual(counts, [7, 3] if split else [0, 10])
                self.assertIn("AfterTable", pages[1])

    def test_splittable_cell_taller_than_a_page_keeps_all_content_visible(self):
        labels = [f"CellLine{index:02}" for index in range(24)]
        with doc_ex.new("docx", size=(300, 300)) as document:
            style = self.prepare(document)
            table = document.add_table([["\n".join(labels)]])
            paragraph = table[0, 0].paragraphs[0]
            paragraph.style = style
            paragraph.format(keep_together=False, widow_control=False)
            table.row(0).allow_split = True
            pages = self.exported_pages(document, labels)
            self.assertEqual([sum(label in page for label in labels) for page in pages], [17, 7])

    def test_oversized_unsplittable_cell_is_rejected_before_publication_and_recovers(self):
        labels = [f"CellLine{index:02}" for index in range(24)]
        with doc_ex.new("docx", size=(300, 300)) as document:
            style = self.prepare(document)
            table = document.add_table([["\n".join(labels)]])
            paragraph = table[0, 0].paragraphs[0]
            paragraph.style = style
            paragraph.format(keep_together=False, widow_control=False)
            table.row(0).allow_split = False
            for publish in (document.to_bytes, lambda: document.convert("pdf"),
                            lambda: document.render_page(0), lambda: document.page_count):
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "unsplittable table row"):
                    publish()
            with TemporaryDirectory() as directory:
                target = Path(directory) / "existing.docx"
                original = b"Existing caller-owned file"
                target.write_bytes(original)
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "unsplittable table row"):
                    document.save(target)
                self.assertEqual(target.read_bytes(), original)
            table.row(0).allow_split = True
            pages = self.exported_pages(document, labels)
            self.assertEqual([sum(label in page for label in labels) for page in pages], [17, 7])

    def test_guard_rechecks_font_margin_and_content_growth_after_cached_render(self):
        labels = [f"CellLine{index:02}" for index in range(12)]
        for change in ("font", "margin", "content"):
            with self.subTest(change=change), doc_ex.new("docx", size=(300, 300)) as document:
                style = self.prepare(document)
                table = document.add_table([["\n".join(labels)]])
                paragraph = table[0, 0].paragraphs[0]
                paragraph.style = style
                paragraph.format(keep_together=False, widow_control=False)
                table.row(0).allow_split = False
                self.assertEqual(document.page_count, 1)
                self.assertTrue(document.render_page(0))
                if change == "font":
                    style.format(font_size=28)
                elif change == "margin":
                    document.sections[0].margins = (150, 30, 30, 30)
                else:
                    paragraph.text = "\n".join(f"Expanded{index:02}" for index in range(24))
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "unsplittable table row"):
                    document.to_bytes()
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "unsplittable table row"):
                    document.render_page(0)
                table.row(0).allow_split = True
                self.assertGreater(document.page_count, 1)
                self.assertTrue(document.convert("pdf"))

    def test_guard_preserves_intentional_exact_height_clipping_and_merged_rows(self):
        with doc_ex.new("docx", size=(300, 300)) as document:
            self.prepare(document)
            table = document.add_table([["\n".join("Clipped line" for _ in range(24))]])
            table.row(0).allow_split = False
            table.row(0).height = (30, "exact")
            self.assertEqual(document.page_count, 1)
            self.assertTrue(document.convert("pdf"))
        with doc_ex.new("docx", size=(300, 300)) as document:
            self.prepare(document)
            table = document.add_table([[f"Left{row:02}", f"Right{row:02}"] for row in range(20)])
            table.merge((0, 0), (19, 0))
            table[0, 0].text = "MergedAnchor"
            for row in range(20):
                table.row(row).allow_split = False
            self.assertGreater(document.page_count, 1)
            self.exported_pages(document, ["MergedAnchor"] + [f"Right{row:02}" for row in range(20)])


if __name__ == "__main__":
    unittest.main()
