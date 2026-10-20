from __future__ import annotations

import unittest

import doc_ex


class PageBreakTests(unittest.TestCase):
    def test_explicit_breaks_preserve_leading_and_consecutive_blank_pages(self):
        for before in (False, True):
            for count in (1, 2):
                with self.subTest(before=before, count=count), doc_ex.new("docx") as document:
                    if before:
                        document.add_paragraph("Before")
                    for _ in range(count):
                        marker = document.add_page_break()
                        self.assertEqual(marker.text, "\f")
                        self.assertFalse(marker.page_break_before)
                    document.add_paragraph("After")
                    for data in (document.to_bytes(), document.convert("pdf")):
                        with doc_ex.open(data) as reopened:
                            self.assertEqual(len(reopened), count + 1)
                            for index, page in enumerate(reopened):
                                text = page.get_text()
                                self.assertEqual("Before" in text, before and index == 0)
                                self.assertEqual("After" in text, index == count)
                                if (not before or index > 0) and index < count:
                                    self.assertEqual(text.strip(), "")


if __name__ == "__main__":
    unittest.main()
