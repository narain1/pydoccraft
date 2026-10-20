import struct
import unittest

import doc_ex
from test_rendering import png_chunks


class LayoutInvalidationTests(unittest.TestCase):
    def assert_current_layout(self, document, labels):
        """Cached authoring views and fresh native/PDF outputs describe one revision."""
        count = document.page_count
        with doc_ex.open(document.to_bytes()) as native, doc_ex.open(document.convert("pdf")) as pdf:
            self.assertEqual(len(native), count)
            self.assertEqual(len(pdf), count)
            text = "\n".join(page.get_text() for page in pdf)
            for label in labels:
                self.assertEqual(text.count(label), 1)
            self.assertEqual(sorted(text.index(label) for label in labels),
                             [text.index(label) for label in labels])
            for index in range(count):
                current = document.render_page(index, dpi=72)
                self.assertEqual(current, native.render_page(index, dpi=72))
                dimensions = struct.unpack(">II", png_chunks(current)[b"IHDR"][:8])
                for actual, key in zip(dimensions, ("width", "height")):
                    self.assertAlmostEqual(actual, pdf[index].metadata[key], delta=1)
        return count

    def test_font_and_margin_edits_repaginate_warmed_owner_without_changing_copy(self):
        labels = [f"Entry{index:02}" for index in range(14)]
        with doc_ex.new("docx", size=(360, 360)) as document:
            document.sections[0].margins = 24
            style = document.styles.add("Compact body", based_on="Normal")
            style.format(font_size=10, space_before=0, space_after=0, line_spacing=1)
            paragraphs = [document.add_paragraph(label, style=style) for label in labels]
            selected = paragraphs[0].find(labels[0])[0]
            baseline_count = document.page_count
            baseline_png = document.render_page(0, dpi=72)
            self.assertEqual(baseline_count, 1)
            with document.copy() as copied, doc_ex.open(document.to_bytes()) as original:
                self.assertEqual(copied.page_count, baseline_count)
                self.assertEqual(copied.render_page(0, dpi=72), baseline_png)
                original_page = original[0]
                original_text = original_page.get_text()

                style.format(font_size=32)
                with self.assertRaisesRegex(ValueError, "stale"):
                    _ = selected.text
                enlarged_count = self.assert_current_layout(document, labels)
                self.assertGreater(enlarged_count, baseline_count)
                self.assertNotEqual(document.render_page(0, dpi=72), baseline_png)

                document.sections[0].margins = (100, 24, 100, 24)
                narrowed_count = self.assert_current_layout(document, labels)
                self.assertGreater(narrowed_count, enlarged_count)
                self.assertEqual(copied.page_count, baseline_count)
                self.assertEqual(copied.render_page(0, dpi=72), baseline_png)
                self.assertEqual(original_page.get_text(), original_text)
                document.close()
                self.assertEqual(copied.render_page(0, dpi=72), baseline_png)
                self.assertEqual(original_page.get_text(), original_text)

    def test_table_and_content_edits_refresh_warmed_render_and_pdf_order(self):
        labels = ["BeforeTable"] + [f"TableRow{index:02}" for index in range(4)] + ["AfterTable"]
        with doc_ex.new("docx", size=(360, 360)) as document:
            document.sections[0].margins = 24
            document.add_paragraph(labels[0])
            table = document.add_table([[label] for label in labels[1:-1]])
            for index in range(4):
                table.row(index).height = (20, "exact")
                table.row(index).allow_split = False
            after = document.add_paragraph(labels[-1])
            cell = table[0, 0]
            baseline_count = document.page_count
            baseline_png = document.render_page(0, dpi=72)
            self.assertEqual(baseline_count, 1)

            for index in range(4):
                table.row(index).height = (90, "exact")
            expanded_count = self.assert_current_layout(document, labels)
            self.assertGreater(expanded_count, baseline_count)
            self.assertNotEqual(document.render_page(0, dpi=72), baseline_png)
            self.assertEqual(cell.text, labels[1])

            after.text = "UpdatedAfterTable"
            labels[-1] = "UpdatedAfterTable"
            document.add_page_break()
            document.add_paragraph("FinalPage")
            labels.append("FinalPage")
            self.assertGreater(self.assert_current_layout(document, labels), expanded_count)
            with doc_ex.open(document.convert("pdf")) as pdf:
                self.assertIn("FinalPage", pdf[-1].get_text())


if __name__ == "__main__":
    unittest.main()
