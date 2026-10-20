"""Owned PDF field tooltips preserve field values, geometry and visible content."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import geometry_pdf


KINDS = (("text", (), "Customer"), ("checkbox", (), True),
         ("choice", ("Canada", "France"), "France"))
TOOLTIP = "  Customer — 顧客\nHelp Ω  "


class FieldTooltipTests(unittest.TestCase):
    def test_metadata_preserves_all_kinds_rotated_geometry_pixels_and_followup_edits(self):
        for rotation in (0, 90, 180, 270):
            for kind, options, value in KINDS:
                with self.subTest(rotation=rotation, kind=kind), doc_ex.new("pdf") as document:
                    document.pages.insert(1, geometry_pdf(rotation))
                    field = document.pages[1].fields.add("target", (20, 100, 180, 40), kind=kind, options=options)
                    field.value = value
                    self.assertEqual(field.tooltip, "")
                    metadata = field.name, field.kind, field.value, field.options, field.box, field.page.number
                    before = document.render_page(1, dpi=144)
                    field.tooltip = TOOLTIP
                    self.assertEqual(field.tooltip, TOOLTIP)
                    self.assertEqual((field.name, field.kind, field.value, field.options, field.box, field.page.number), metadata)
                    self.assertEqual(document.render_page(1, dpi=144), before)
                    with doc_ex.open(document.to_bytes()) as reopened:
                        self.assertEqual(reopened.render_page(1, dpi=144), before)
                        with self.assertRaises(UnsupportedFormatError):
                            reopened.edit()
                    with document.copy() as copied:
                        changed = copied.fields["target"]
                        self.assertEqual(changed.tooltip, TOOLTIP)
                        changed.tooltip = "Copy only"
                        self.assertEqual(field.tooltip, TOOLTIP)
                        changed.box = (20, 150, 190, 40)
                        changed.value = value
                        self.assertEqual(changed.tooltip, "Copy only")
                        # Both controls follow the same native flatten placement, including
                        # its existing rotated/UserUnit rounding; tooltip has no visual role.
                        with copied.copy() as control:
                            control.fields["target"].tooltip = ""
                            self.assertEqual(copied.fields.flatten(), 1)
                            self.assertEqual(control.fields.flatten(), 1)
                            self.assertEqual(copied.render_page(1, dpi=144), control.render_page(1, dpi=144))
                    field.tooltip = ""
                    self.assertEqual(field.tooltip, "")
                    self.assertEqual(document.render_page(1, dpi=144), before)
                    field.tooltip = TOOLTIP
                    document.pages.move(1, 0)
                    with self.assertRaises(ValueError):
                        _ = field.tooltip
                    fresh = document.fields["target"]
                    self.assertEqual(fresh.tooltip, TOOLTIP)
                    self.assertEqual(fresh.page.number, 0)
                    self.assertEqual(document.render_page(0, dpi=144), before)

    def test_utf8_limits_bad_values_and_warm_cache_are_atomic(self):
        with doc_ex.new("pdf") as document:
            field = document.pages[0].fields.add("target", (20, 20, 180, 30))
            limit = "é" * 2048
            field.tooltip = limit
            self.assertEqual(field.tooltip, limit)
            before, pixels = document.to_bytes(), document.render_page(0, dpi=72)
            cached = document._cached
            for value, error in ((None, TypeError), (True, TypeError), (123, TypeError),
                                 (b"bytes", TypeError), ("bad\0text", ValueError),
                                 ("\ud800", ValueError), (limit + "x", ValueError),
                                 ("a" * 4097, ValueError)):
                with self.subTest(value_type=type(value), size=len(value) if isinstance(value, (str, bytes)) else None), self.assertRaises(error):
                    field.tooltip = value
                self.assertIs(document._cached, cached)
                self.assertEqual(field.tooltip, limit)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), pixels)
            field.tooltip = ""
            self.assertEqual(field.tooltip, "")

    def test_subclasses_cannot_run_callbacks_or_hide_size(self):
        class Sneaky(str):
            def __len__(self):
                raise AssertionError("custom length called")
            def __str__(self):
                raise AssertionError("custom string conversion called")
            def encode(self, *args, **kwargs):
                raise AssertionError("custom encoding called")
        with doc_ex.new("pdf") as document:
            field = document.pages[0].fields.add("target", (20, 20, 180, 30))
            field.tooltip = Sneaky(TOOLTIP)
            self.assertEqual(field.tooltip, TOOLTIP)
            with self.assertRaises(ValueError):
                field.tooltip = Sneaky("a" * 1_000_000)
            self.assertEqual(field.tooltip, TOOLTIP)

    def test_closed_stale_deleted_and_copied_handles_keep_identity(self):
        for state in ("closed", "reordered", "deleted"):
            with self.subTest(state=state), doc_ex.new("pdf") as document:
                document.pages.add()
                field = document.pages[0].fields.add("target", (20, 20, 180, 30))
                field.tooltip = "Original"
                if state == "closed":
                    document.close()
                    error = DocumentClosedError
                elif state == "reordered":
                    document.pages.move(0, 1)
                    error = ValueError
                else:
                    del document.fields["target"]
                    document.pages[0].fields.add("target", (20, 20, 180, 30)).tooltip = "Replacement"
                    error = ValueError
                for invalid in (None, "bad\0text"):
                    with self.assertRaises(error):
                        field.tooltip = invalid
                with self.assertRaises(error):
                    _ = field.tooltip
                if state != "closed":
                    self.assertEqual(document.fields["target"].tooltip, "Replacement" if state == "deleted" else "Original")


if __name__ == "__main__":
    unittest.main()
