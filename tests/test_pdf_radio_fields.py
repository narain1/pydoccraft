"""Same-page owned radio groups use the native writer and immutable widget geometry."""
import unittest

import doc_ex
from doc_ex.errors import DocExError, DocumentClosedError, UnsupportedFormatError
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks


BOXES = ((20, 100, 24, 24), (60, 100, 24, 24))


def add(page, name="group", options=("A", "B"), boxes=BOXES):
    return page.fields.add(name, kind="radio", options=options, boxes=boxes)


class PdfRadioFieldTests(unittest.TestCase):
    def assertBoxes(self, actual, expected):
        self.assertEqual(len(actual), len(expected))
        for first, second in zip(actual, expected):
            for a, b in zip(first, second):
                self.assertAlmostEqual(a, b, delta=0.0001)  # Existing native f32 view conversion.

    def test_four_views_selection_pixels_source_copy_flatten_and_metadata(self):
        for rotation in (0, 90, 180, 270):
            with self.subTest(rotation=rotation), doc_ex.new("pdf") as document:
                document.pages.insert(1, geometry_pdf(rotation))
                field = add(document.pages[1], options=("A", "café 顧客"))
                self.assertEqual(field.kind, "radio")
                self.assertEqual(field.options, ("A", "café 顧客"))
                self.assertBoxes(field.boxes, BOXES)
                self.assertIsNone(field.value)
                self.assertEqual(field.page.number, 1)
                self.assertEqual(len(document.fields), 1)
                self.assertEqual(len(document.pages[1].fields), 1)
                field.value = None
                off = document.render_page(1, dpi=144)
                field.value = "café 顧客"
                on = document.render_page(1, dpi=144)
                self.assertNotEqual(on, off)
                width, pixels = rgba_pixels(png_chunks(on))
                _, off_pixels = rgba_pixels(png_chunks(off))
                index = ((112 * 2 * width + 72 * 2) * 4)
                self.assertLess(pixels[index], 30)
                self.assertGreater(off_pixels[index], 240)
                field.tooltip = "Help — 顧客"
                field.read_only = True
                self.assertEqual(document.render_page(1, dpi=144), on)
                self.assertBoxes(field.boxes, BOXES)
                with doc_ex.open(document.to_bytes()) as source:
                    self.assertEqual(source.render_page(1, dpi=144), on)
                    with self.assertRaises(UnsupportedFormatError):
                        source.edit()
                with self.assertRaises(UnsupportedFormatError):
                    document.copy(pages=[1])
                with document.copy() as copied:
                    item = copied.fields["group"]
                    self.assertTrue(item.read_only)
                    self.assertEqual(item.tooltip, "Help — 顧客")
                    self.assertEqual(item.options, field.options)
                    self.assertBoxes(item.boxes, BOXES)
                    self.assertEqual(copied.fields.flatten(), 1)
                    self.assertEqual(len(copied.fields), 0)
                    self.assertEqual(copied.render_page(1, dpi=144), on)
                self.assertEqual(field.value, "café 顧客")
                field.read_only = False
                field.value = "A"
                self.assertNotEqual(document.render_page(1, dpi=144), on)

    def test_literal_hash_aliases_native_no_toggle_and_read_only_are_atomic(self):
        options = ("A", "#41", "#4Fff", "Off#", "顧客")
        boxes = tuple((20 + 35 * i, 20, 24, 24) for i in range(len(options)))
        with doc_ex.new("pdf", size=(250, 150)) as document:
            field = add(document.pages[0], options=options, boxes=boxes)
            self.assertEqual(field.options, options)
            for selected, value in enumerate(options):
                field.value = value
                self.assertEqual(field.value, value)
                width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=72)))
                for i in range(len(options)):
                    pixel = pixels[(32 * width + 32 + 35 * i) * 4]
                    self.assertLess(pixel, 30) if i == selected else self.assertGreater(pixel, 240)
            field.read_only = True
            snapshot, cached = document.to_bytes(), document._cached
            for value in (options[-1], "A", None):
                with self.assertRaisesRegex(DocExError, "read-only"):
                    field.value = value
                self.assertEqual(document.to_bytes(), snapshot)
                self.assertIs(document._cached, cached)
            field.read_only = False
            snapshot = document.to_bytes()
            with self.assertRaises(DocExError):
                field.value = None
            self.assertEqual(document.to_bytes(), snapshot)
            for value in ("missing", "Off", True, 1):
                with self.assertRaises((TypeError, ValueError)):
                    field.value = value
                self.assertEqual(document.to_bytes(), snapshot)
            with self.assertRaises(UnsupportedFormatError):
                _ = field.box
            with self.assertRaises(UnsupportedFormatError):
                field.box = (10, 10, 30, 30)
            self.assertEqual(document.to_bytes(), snapshot)

    def test_invalid_creation_limits_aliases_and_owned_boxes(self):
        with doc_ex.new("pdf") as document:
            page = document.pages[0]
            baseline = document.to_bytes()
            invalid = [
                {"options": ("A", "A")}, {"options": ("Off", "B")},
                {"options": (" ", "B")}, {"options": ("A\0", "B")},
                {"options": ("x" * 4097, "B")}, {"options": ("顧" * 1366, "B")},
                {"options": ("\ud800", "B")}, {"options": ("A",)},
                {"boxes": ((10, 10, 11, 24), BOXES[1])},
                {"boxes": ((-1, 10, 24, 24), BOXES[1])},
                {"boxes": ((10, 10, float("nan"), 24), BOXES[1])},
                {"boxes": (BOXES[0],)},
                {"boxes": (BOXES[0],) * 33, "options": tuple(str(i) for i in range(33))},
            ]
            for kwargs in invalid:
                with self.subTest(kwargs=kwargs), self.assertRaises((TypeError, ValueError)):
                    add(page, **kwargs)
                self.assertEqual(document.to_bytes(), baseline)
                self.assertEqual(len(document.fields), 0)
            with self.assertRaises(ValueError):
                page.fields.add("g", BOXES[0], kind="radio", boxes=BOXES, options=("A", "B"))
            with self.assertRaises(ValueError):
                page.fields.add("g", BOXES[0], boxes=BOXES)
            class Discriminator(str):
                def __eq__(self, other):
                    raise AssertionError("custom equality must not select native kind")
            ordinary = page.fields.add("ordinary", (20, 20, 100, 30), kind=Discriminator("text"))
            self.assertEqual(ordinary.boxes, (ordinary.box,))
            encoded = add(page, name="encoded", options=("#" * 4096, "Off#"))
            encoded.value = "#" * 4096
            self.assertEqual(encoded.value, "#" * 4096)
            self.assertEqual(encoded.options, ("#" * 4096, "Off#"))
            source_boxes = [list(box) for box in BOXES]
            group = add(page, boxes=source_boxes)
            owned = group.boxes
            source_boxes[0][0] = 99
            self.assertBoxes(owned, BOXES)
            self.assertIsInstance(owned, tuple)
            self.assertTrue(all(isinstance(box, tuple) for box in owned))
            document.close()
            self.assertBoxes(owned, BOXES)

    def test_page_organization_delete_and_closed_or_stale_handles(self):
        with doc_ex.new("pdf", size=(300, 300)) as document:
            document.pages.add()
            field = add(document.pages[0])
            field.value = "B"
            selected = document.render_page(0, dpi=72)
            document.pages.move(0, 1)
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = field.boxes
            field = document.fields["group"]
            self.assertEqual(field.page.number, 1)
            self.assertEqual(document.render_page(1, dpi=72), selected)
            field.page.rotate(90)
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = field.value
            fresh = document.fields["group"]
            before = document.render_page(1, dpi=72)
            fresh.tooltip = "Rotated"
            self.assertEqual(document.render_page(1, dpi=72), before)
            with document.copy() as copied:
                del copied.pages[1]
                self.assertEqual(len(copied.fields), 0)
            del document.fields["group"]
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = fresh.boxes
            replacement = add(document.pages[1])
            replacement.value = "A"
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = replacement.boxes

    def test_creation_callback_does_not_retarget_or_add_after_owner_changes(self):
        for mechanism in ("coordinate", "options", "boxes"):
            for action in ("move", "delete", "close", "raise"):
                with self.subTest(mechanism=mechanism, action=action), doc_ex.new("pdf") as document:
                    document.pages.add()
                    for i in range(2):
                        document.pages[i].draw_text(f"PAGE {i}", at=(20, 200))
                    page = document.pages[0]
                    observed = {}
                    def callback():
                        if action == "move":
                            document.pages.move(0, 1)
                        elif action == "delete":
                            del document.pages[0]
                        elif action == "close":
                            document.close()
                        if action != "close":
                            observed["bytes"] = document.to_bytes()
                            observed["pngs"] = [document.render_page(i, dpi=72) for i in range(document.page_count)]
                        if action == "raise":
                            raise RuntimeError("caller failure")
                    class Coordinate(float):
                        def __float__(self):
                            callback()
                            return 20.0
                    def options():
                        callback()
                        yield "A"
                        yield "B"
                    def boxes():
                        callback()
                        yield from BOXES
                    kwargs = {"options": ("A", "B"), "boxes": BOXES}
                    if mechanism == "coordinate":
                        kwargs["boxes"] = ((Coordinate(), 100, 24, 24), BOXES[1])
                    elif mechanism == "options":
                        kwargs["options"] = options()
                    else:
                        kwargs["boxes"] = boxes()
                    expected = RuntimeError if action == "raise" else DocumentClosedError if action == "close" else ValueError
                    with self.assertRaises(expected):
                        add(page, **kwargs)
                    if action != "close":
                        self.assertEqual(len(document.fields), 0)
                        self.assertEqual(document.to_bytes(), observed["bytes"])
                        self.assertEqual([document.render_page(i, dpi=72) for i in range(document.page_count)], observed["pngs"])
        with doc_ex.new("pdf") as document:
            page = document.pages[0]
            document.close()
            def forbidden():
                raise AssertionError("callback on closed owner")
                yield
            with self.assertRaises(DocumentClosedError):
                add(page, options=forbidden(), boxes=forbidden())


if __name__ == "__main__":
    unittest.main()
