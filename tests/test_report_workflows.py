from pathlib import Path
import tempfile
import unittest

import doc_ex


class ReportWorkflowTests(unittest.TestCase):
    def test_blank_docx_saves_reopens_and_renders(self):
        with doc_ex.new("docx") as document:
            self.assertEqual(len(document.paragraphs), 0)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened.page_count, 1)
                self.assertEqual(reopened[0].get_text(), "")
                self.assertTrue(reopened.render_page(0, dpi=72).startswith(b"\x89PNG"))

    def test_report_uses_ordinary_primitives_and_both_outputs_keep_links(self):
        with tempfile.TemporaryDirectory() as directory, doc_ex.new("docx") as report:
            report.add_paragraph("Invoice", style="Heading 1")
            report.add_paragraph("Prepared for Acme")
            items = [("Support", 2, 125), ("Training", 1, 300)]
            rows = [["Item", "Quantity", "Total"], *[[name, str(quantity), f"${quantity * price:.2f}"] for name, quantity, price in items]]
            report.add_table(rows)
            report.add_paragraph(f"Grand total: ${sum(quantity * price for _, quantity, price in items):.2f}")
            link = report.add_paragraph().add_run("Payment terms")
            link.link = "https://example.com/terms"
            path = Path(directory) / "invoice.docx"
            report.save(path)
            for source in (path, report.convert("pdf")):
                with doc_ex.open(source) as reopened:
                    text = "\n".join(page.get_text() for page in reopened)
                    for value in ("Support", "Training", "Grand total: $550.00"):
                        self.assertEqual(text.count(value), 1)
                    self.assertIn("https://example.com/terms", [item.uri for page in reopened for item in page.get_links()])
                    self.assertTrue(reopened.render_page(0, dpi=72).startswith(b"\x89PNG"))
