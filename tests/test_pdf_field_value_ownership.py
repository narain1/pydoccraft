"""Field values normalize real string content before their final identity check."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError


class PdfFieldValueOwnershipTests(unittest.TestCase):
    def test_string_subclass_callbacks_cannot_replace_target_or_coerce_payload(self):
        with doc_ex.new("pdf", size=(300, 200)) as document:
            field = document.pages[0].fields.add("same", (20, 20, 180, 30))
            original_box = field.box
            calls = []
            class Text(str):
                def __len__(self):
                    calls.append("len")
                    return 0
                def __str__(self):
                    calls.append("str")
                    return "different"
                def encode(self, *args, **kwargs):
                    calls.append("encode")
                    del document.fields["same"]
                    document.pages[0].fields.add("same", (20, 70, 180, 30)).value = "replacement"
                    return b"different"
            field.value = Text("café")
            self.assertEqual(calls, [])
            self.assertEqual(field.value, "café")
            self.assertEqual(field.box, original_box)
            self.assertEqual(len(document.fields), 1)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn("café", reopened[0].get_text())
            with document.copy() as copied:
                self.assertEqual(copied.fields["same"].value, "café")
                self.assertEqual(copied.render_page(0, dpi=72), document.render_page(0, dpi=72))
                self.assertEqual(copied.to_bytes(), document.to_bytes())

    def test_real_length_utf8_invalid_values_and_supported_bool_none_are_atomic(self):
        class Text(str):
            def __len__(self):
                raise AssertionError("custom length")
            def encode(self, *args, **kwargs):
                raise AssertionError("custom encode")
        with doc_ex.new("pdf", size=(300, 200)) as document:
            field = document.pages[0].fields.add("text", (20, 20, 180, 30))
            field.value = "original"
            pixels = document.render_page(0, dpi=72)
            before, cached = document.to_bytes(), document._cached
            for value in (Text("x" * 4097), Text("é" * 2049), Text("\ud800"), "bad\0value", True, None, 1, object()):
                with self.subTest(value_type=type(value)), self.assertRaises((TypeError, ValueError)):
                    field.value = value
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cached)
                self.assertEqual(document.render_page(0, dpi=72), pixels)
            with self.assertRaises(UnsupportedFormatError):
                field.value = Text("顧客")
            self.assertEqual(document.to_bytes(), before)
            choice = document.pages[0].fields.add("choice", (20, 60, 180, 30), kind="choice", options=("café", "B"))
            choice.value = Text("café")
            self.assertEqual(choice.value, "café")
            choice.value = None
            self.assertIsNone(choice.value)
            checkbox = document.pages[0].fields.add("checkbox", (20, 100, 24, 24), kind="checkbox")
            for value in (True, False):
                checkbox.value = value
                self.assertIs(checkbox.value, value)

    def test_initial_closed_stale_precedence_and_replacement_output_preserved(self):
        class Forbidden(str):
            def __len__(self):
                raise AssertionError("invalid handle callback")
            def encode(self, *args, **kwargs):
                raise AssertionError("invalid handle callback")
        with doc_ex.new("pdf", size=(300, 200)) as document:
            field = document.pages[0].fields.add("same", (20, 20, 180, 30))
            del document.fields["same"]
            fresh = document.pages[0].fields.add("same", (20, 70, 180, 30))
            fresh.value = "replacement"
            pixels = document.render_page(0, dpi=72)
            before, cached = document.to_bytes(), document._cached
            for value in (Forbidden("stale"), Forbidden("\ud800"), object()):
                with self.assertRaisesRegex(ValueError, "stale"):
                    field.value = value
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cached)
            self.assertEqual(document.render_page(0, dpi=72), pixels)
            self.assertEqual(fresh.value, "replacement")
            document.close()
            for value in (Forbidden("closed"), object()):
                with self.assertRaises(DocumentClosedError):
                    fresh.value = value


if __name__ == "__main__":
    unittest.main()
