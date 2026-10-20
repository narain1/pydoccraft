"""Owned field read-only state uses native fill refusal, not a security boundary."""
import unittest

import doc_ex
from doc_ex.errors import DocExError, DocumentClosedError, UnsupportedFormatError
from test_render_geometry import geometry_pdf


KINDS = (("text", (), "Customer", "Other", ""),
         ("checkbox", (), True, False, False),
         ("choice", ("Canada", "France"), "France", "Canada", None))


class FieldReadOnlyTests(unittest.TestCase):
    def test_all_kinds_preserve_four_view_pixels_metadata_copy_and_allowed_edits(self):
        for rotation in (0, 90, 180, 270):
            for kind, options, value, changed, _ in KINDS:
                with self.subTest(rotation=rotation, kind=kind), doc_ex.new("pdf") as document:
                    document.pages.insert(1, geometry_pdf(rotation))
                    baseline = document.render_page(1, dpi=144)
                    field = document.pages[1].fields.add("target", (20, 100, 180, 40), kind=kind, options=options)
                    field.value = value
                    field.tooltip = "Help — 顧客"
                    self.assertIs(field.read_only, False)
                    metadata = field.name, field.kind, field.value, field.options, field.box, field.page.number, field.tooltip
                    pixels = document.render_page(1, dpi=144)
                    self.assertNotEqual(pixels, baseline)
                    field.read_only = True
                    self.assertIs(field.read_only, True)
                    self.assertEqual((field.name, field.kind, field.value, field.options, field.box, field.page.number, field.tooltip), metadata)
                    self.assertEqual(document.render_page(1, dpi=144), pixels)
                    field.read_only = True
                    self.assertEqual(document.render_page(1, dpi=144), pixels)
                    with doc_ex.open(document.to_bytes()) as reopened:
                        self.assertEqual(reopened.render_page(1, dpi=144), pixels)
                        with self.assertRaises(UnsupportedFormatError):
                            reopened.edit()
                    with self.assertRaises(UnsupportedFormatError):
                        document.copy(pages=[1])
                    with document.copy() as copied:
                        item = copied.fields["target"]
                        self.assertTrue(item.read_only)
                        item.tooltip = "Changed help"
                        item.box = (20, 150, 190, 40)
                        self.assertTrue(item.read_only)
                        with copied.copy() as control:
                            control.fields["target"].read_only = False
                            self.assertEqual(copied.fields.flatten(), 1)
                            self.assertEqual(control.fields.flatten(), 1)
                            self.assertEqual(copied.render_page(1, dpi=144), control.render_page(1, dpi=144))
                        self.assertTrue(field.read_only)
                        self.assertEqual(field.value, value)
                    with document.copy() as copied:
                        copied.fields["target"].read_only = False
                        copied.fields["target"].value = changed
                        self.assertEqual(copied.fields["target"].value, changed)
                        self.assertEqual(field.value, value)
                    field.read_only = False
                    self.assertFalse(field.read_only)
                    self.assertEqual(document.render_page(1, dpi=144), pixels)
                    field.value = changed
                    self.assertEqual(field.value, changed)

    def test_programmatic_same_different_and_clear_values_fail_atomically(self):
        for kind, options, value, changed, clear in KINDS:
            with self.subTest(kind=kind), doc_ex.new("pdf") as document:
                field = document.pages[0].fields.add("target", (20, 20, 180, 40), kind=kind, options=options)
                field.value = value
                field.read_only = True
                pixels = document.render_page(0, dpi=144)
                before, cached = document.to_bytes(), document._cached
                for attempt in (value, changed, clear):
                    with self.assertRaisesRegex(DocExError, "read-only"):
                        field.value = attempt
                    self.assertEqual(field.value, value)
                    self.assertTrue(field.read_only)
                    self.assertEqual(document.to_bytes(), before)
                    self.assertIs(document._cached, cached)
                invalid = True if kind == "text" else "not an option"
                with self.assertRaises(ValueError):
                    field.value = invalid
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cached)
                self.assertEqual(document.render_page(0, dpi=144), pixels)
                field.read_only = False
                field.value = clear
                self.assertEqual(field.value, clear)
                field.value = changed
                self.assertEqual(field.value, changed)

    def test_strict_bool_and_closed_stale_removed_identity(self):
        class Dangerous:
            def __bool__(self):
                raise AssertionError("truth conversion executed")
        with doc_ex.new("pdf") as document:
            field = document.pages[0].fields.add("target", (20, 20, 180, 40))
            pixels = document.render_page(0, dpi=72)
            before, cached = document.to_bytes(), document._cached
            for value in (None, 0, 1, "true", [], Dangerous()):
                with self.assertRaises(TypeError):
                    field.read_only = value
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cached)
            self.assertEqual(document.render_page(0, dpi=72), pixels)
            field.read_only = True
            document.pages.add()
            document.pages.move(0, 1)
            with self.assertRaisesRegex(ValueError, "stale"):
                field.read_only = Dangerous()
            fresh = document.fields["target"]
            self.assertTrue(fresh.read_only)
            fresh.page.rotate(90)
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = fresh.read_only
            rotated = document.fields["target"]
            self.assertTrue(rotated.read_only)
            del document.fields["target"]
            with self.assertRaisesRegex(ValueError, "stale"):
                rotated.read_only = Dangerous()
            replacement = document.pages[1].fields.add("target", (20, 20, 180, 40))
            self.assertFalse(replacement.read_only)
            document.close()
            with self.assertRaises(DocumentClosedError):
                replacement.read_only = Dangerous()
            with self.assertRaises(DocumentClosedError):
                _ = replacement.read_only

    def test_read_only_deletion_and_page_deletion_remain_available(self):
        for kind, options, value, _, _ in KINDS:
            with self.subTest(kind=kind), doc_ex.new("pdf") as document:
                document.pages.add()
                field = document.pages[1].fields.add("target", (20, 20, 180, 40), kind=kind, options=options)
                field.value = value
                field.read_only = True
                with document.copy() as copied:
                    del copied.fields["target"]
                    self.assertEqual(len(copied.fields), 0)
                    self.assertTrue(field.read_only)
                del document.pages[1]
                self.assertEqual(len(document.fields), 0)
                with self.assertRaisesRegex(ValueError, "stale"):
                    _ = field.read_only


if __name__ == "__main__":
    unittest.main()
