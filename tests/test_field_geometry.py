import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks


def dark_pixels(png, box):
    width, pixels = rgba_pixels(png_chunks(png))
    x, y, w, h = box
    return sum(all(value < 80 for value in pixels[(row * width + col) * 4:(row * width + col) * 4 + 3])
               for row in range(y, y + h) for col in range(x, x + w))


class FieldGeometryTests(unittest.TestCase):
    def test_move_resize_all_kinds_preserves_handles_and_visible_content(self):
        for kind, options, value in (("text", (), "Customer"),
                                     ("checkbox", (), True),
                                     ("choice", ("Canada", "France"), "France")):
            with self.subTest(kind=kind), doc_ex.new("pdf", size=(300, 300)) as document:
                field = document.pages[0].fields.add("field", (20, 20, 180, 30), kind=kind, options=options)
                field.value = value
                before = document.render_page(0, dpi=72)
                self.assertGreater(dark_pixels(before, (20, 20, 180, 30)), 10)
                field.box = [60, 130, 210, 50]
                self.assertEqual(field.box, (60, 130, 210, 50))
                self.assertEqual((field.name, field.kind, field.value, field.options), ("field", kind, value, options))
                after = document.render_page(0, dpi=72)
                self.assertNotEqual(before, after)
                self.assertEqual(dark_pixels(after, (20, 20, 180, 30)), 0)
                self.assertGreater(dark_pixels(after, (60, 130, 210, 50)), 10)
                with document.copy() as copied:
                    copied.fields["field"].box = (20, 210, 200, 40)
                    self.assertEqual(field.box, (60, 130, 210, 50))
                    self.assertEqual(copied.fields["field"].value, value)
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual(reopened.render_page(0, dpi=72), after)
                document.fields.flatten()
                self.assertEqual(document.render_page(0, dpi=72), after)
                with doc_ex.open(document.to_bytes()) as flattened:
                    if isinstance(value, str):
                        self.assertIn(value, flattened[0].get_text())
                with self.assertRaises(ValueError):
                    field.box = (20, 20, 180, 30)

    def test_bad_fit_coordinates_and_types_leave_warm_caches_unchanged(self):
        with doc_ex.new("pdf", size=(300, 300)) as document:
            document.pages[0].fields.add("text", (20, 20, 180, 30)).value = "Customer"
            document.pages[0].fields.add("choice", (20, 80, 250, 30), kind="choice", options=("A", "Long unselected option")).value = "A"
            before, png = document.to_bytes(), document.render_page(0, dpi=72)
            for name in ("text", "choice"):
                field = document.fields[name]
                original = field.box
                for box, error in (((0, 0, 12, 30), ValueError),
                                   ((0, 0, 180, 5), ValueError),
                                   ((-1, 0, 180, 30), ValueError),
                                   ((200, 0, 180, 30), ValueError),
                                   ((0, 0, -5, 30), ValueError),
                                   ((0, 0, float("nan"), 30), ValueError),
                                   ((0, 0, float("inf"), 30), ValueError),
                                   ((0, 0, True, 30), TypeError),
                                   ((0, 0, 30), TypeError),
                                   (None, TypeError)):
                    with self.subTest(name=name, box=box), self.assertRaises(error):
                        field.box = box
                    self.assertEqual(field.box, original)
                    self.assertEqual(document.to_bytes(), before)
                    self.assertEqual(document.render_page(0, dpi=72), png)

    def test_rotated_cropped_userunit_pages_use_display_coordinates(self):
        for rotation in (0, 90, 180, 270):
            for kind, options, value in (("text", (), "Moved"),
                                         ("checkbox", (), True),
                                         ("choice", ("Canada", "France"), "France")):
                with self.subTest(rotation=rotation, kind=kind), doc_ex.new("pdf") as document:
                    document.pages.insert(1, geometry_pdf(rotation))
                    field = document.pages[1].fields.add("moved", (10, 100, 180, 30), kind=kind, options=options)
                    field.value = value
                    field.box = (20, 150, 190, 40)
                    self.assertEqual(field.value, value)
                    for actual, expected in zip(field.box, (20, 150, 190, 40)):
                        self.assertAlmostEqual(actual, expected, delta=.001)
                    with doc_ex.open(document.to_bytes()) as saved:
                        png = saved.render_page(1, dpi=72)
                        self.assertEqual(dark_pixels(png, (10, 100, 180, 30)), 0)
                        self.assertGreater(dark_pixels(png, (20, 150, 190, 40)), 10)
                    document.fields.flatten()
                    with doc_ex.open(document.to_bytes()) as flattened:
                        png = flattened.render_page(1, dpi=72)
                        self.assertEqual(dark_pixels(png, (10, 100, 180, 30)), 0)
                        self.assertGreater(dark_pixels(png, (20, 150, 190, 40)), 10)
                        if isinstance(value, str):
                            block = next(block for block in flattened[1].blocks if value in block.text)
                            x0, y0, x1, y1 = block.bbox
                            self.assertGreaterEqual(x0, 20)
                            self.assertGreaterEqual(y0, 150)
                            self.assertLessEqual(x1, 210)
                            self.assertLessEqual(y1, 190)

    def test_move_after_page_rotation_preserves_existing_text_orientation(self):
        with doc_ex.new("pdf", size=(300, 400)) as document:
            old = document.pages[0].fields.add("target", (20, 20, 180, 30))
            old.value = "Customer"
            document.pages[0].rotate(90)
            with self.assertRaises(ValueError):
                old.box = (100, 60, 40, 180)
            field = document.fields["target"]
            original_box = field.box
            before, png = document.to_bytes(), document.render_page(0, dpi=72)
            # The widget's text remains rotated relative to the displayed page.
            # A horizontally wide box has insufficient text width in that orientation.
            with self.assertRaises(ValueError):
                field.box = (100, 60, 180, 40)
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(document.render_page(0, dpi=72), png)
            field.box = (100, 60, 40, 180)
            after = document.render_page(0, dpi=72)
            self.assertGreater(dark_pixels(after, (100, 60, 40, 180)), 10)
            self.assertEqual(dark_pixels(after, tuple(round(v) for v in original_box)), 0)
            document.fields.flatten()
            self.assertEqual(document.render_page(0, dpi=72), after)
            with doc_ex.open(document.to_bytes()) as flattened:
                block = next(b for b in flattened[0].blocks if "Customer" in b.text)
                x0, y0, x1, y1 = block.bbox
                self.assertGreater(y1 - y0, 2 * (x1 - x0))
                self.assertGreaterEqual(x0, 100)
                self.assertGreaterEqual(y0, 60)
                self.assertLessEqual(x1, 140)
                self.assertLessEqual(y1, 240)

    def test_callback_structural_edits_do_not_revive_stale_handles(self):
        for action in ("reorder", "recreate", "close"):
            with self.subTest(action=action), doc_ex.new("pdf") as document:
                document.pages.add()
                field = document.pages[0].fields.add("target", (20, 20, 180, 30))
                field.value = "Original"
                original_box = field.box

                class Coordinate(float):
                    def __float__(self):
                        if action == "reorder":
                            document.pages.move(0, 1)
                        elif action == "recreate":
                            del document.fields["target"]
                            document.pages[0].fields.add("target", (20, 20, 180, 30)).value = "Replacement"
                        else:
                            document.close()
                        return 60.0

                expected_error = DocumentClosedError if action == "close" else ValueError
                with self.assertRaises(expected_error):
                    field.box = (Coordinate(60), 120, 200, 40)
                if action != "close":
                    self.assertEqual(document.fields["target"].box, original_box)
                    self.assertEqual(document.fields["target"].value, "Original" if action == "reorder" else "Replacement")
                    self.assertEqual(document.fields["target"].page.number, 1 if action == "reorder" else 0)


if __name__ == "__main__":
    unittest.main()
