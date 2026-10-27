"""Repeated owned-field writes retain value, appearance and copy semantics."""
import unittest

import doc_ex


class PdfFieldAccountingTests(unittest.TestCase):
    def test_repeated_same_and_empty_writes_preserve_pixels_and_independent_copies(self):
        for kind, options, selected, empty, changed in (
            ("text", (), "Alice", "", "Bob"),
            ("choice", ("A", "B"), "A", None, "B"),
            ("checkbox", (), True, False, False),
        ):
            with self.subTest(kind=kind), doc_ex.new("pdf", size=(300, 300)) as document:
                document.pages[0].draw_text("Original body", at=(20, 200))
                field = document.pages[0].fields.add("field", (20, 20, 180, 30), kind=kind, options=options)
                field.tooltip = "Help"
                for value in (selected, empty, selected):
                    field.value = value
                    baseline = document.render_page(0, dpi=72)
                    for _ in range(16):
                        field.value = value
                    self.assertEqual(field.value, value)
                    self.assertEqual(field.tooltip, "Help")
                    self.assertEqual(field.options, options)
                    self.assertEqual(document.render_page(0, dpi=72), baseline)
                    with doc_ex.open(document.to_bytes()) as reopened:
                        self.assertEqual(reopened.render_page(0, dpi=72), baseline)
                        self.assertIn("Original body", reopened[0].get_text())
                with document.copy() as copied:
                    copied.fields["field"].value = changed
                    self.assertEqual(field.value, selected)
                    self.assertEqual(document.render_page(0, dpi=72), baseline)
                    before_flatten = copied.render_page(0, dpi=72)
                    self.assertEqual(copied.fields.flatten(), 1)
                    self.assertEqual(copied.render_page(0, dpi=72), before_flatten)
                    self.assertEqual(len(copied.fields), 0)
                    self.assertEqual(len(document.fields), 1)
                # Deleting fields does not reset the document's cumulative history;
                # another canonical creation remains usable within the budget.
                del document.fields["field"]
                fresh = document.pages[0].fields.add("field", (20, 20, 180, 30), kind=kind, options=options)
                fresh.value = selected
                with self.assertRaises(ValueError):
                    _ = field.value
                self.assertEqual(fresh.value, selected)


if __name__ == "__main__":
    unittest.main()
