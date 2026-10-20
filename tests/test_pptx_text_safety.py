from __future__ import annotations

import unittest

import doc_ex


class PptxTextSafetyTests(unittest.TestCase):
    def test_carriage_returns_are_rejected_atomically_and_lf_roundtrips(self) -> None:
        with doc_ex.new("pptx") as document:
            slide = document.slides.add()
            text = slide.add_text("a\nb", box=(10, 10, 200, 80))
            table = slide.add_table([["cell"]], box=(10, 100, 200, 80))
            slide.notes = "first\nsecond"
            for invalid in ("a\rb", "a\r\nb"):
                changes = (
                    lambda: slide.add_text(invalid, box=(10, 10, 200, 80)),
                    lambda: setattr(text, "text", invalid),
                    lambda: slide.add_table([[invalid]], box=(10, 100, 200, 80)),
                    lambda: setattr(table.cell(0, 0), "text", invalid),
                    lambda: setattr(slide, "notes", invalid),
                    lambda: setattr(slide, "name", invalid),
                )
                for change in changes:
                    with self.subTest(text=repr(invalid)), self.assertRaisesRegex(ValueError, "LF newlines"):
                        change()
                    self.assertEqual(text.text, "a\nb")
                    self.assertEqual(table.cell(0, 0).text, "cell")
                    self.assertEqual(slide.notes, "first\nsecond")
                    self.assertEqual(len(slide.shapes), 2)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn("a\nb", reopened[0].get_text())
                self.assertNotIn("a\n\nb", reopened[0].get_text())


if __name__ == "__main__":
    unittest.main()
