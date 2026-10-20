"""Drawing normalization must not retarget an invalidated PDF page handle."""
import io
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_pdf_text_boxes import FONT, quadrant_png


class CallbackFloat(float):
    def __new__(cls, callback, value=20):
        obj = super().__new__(cls, value)
        obj.callback = callback
        return obj

    def __float__(self):
        self.callback()
        return float.__float__(self)


class CallbackColor(str):
    def __new__(cls, callback):
        obj = super().__new__(cls, "red")
        obj.callback = callback
        return obj

    def lower(self):
        self.callback()
        return "red"


class CallbackStream(io.BytesIO):
    def __init__(self, data, callback):
        super().__init__(data)
        self.callback = callback

    def read(self, *args):
        callback, self.callback = self.callback, lambda: None
        callback()
        return super().read(*args)


class CallbackIndex:
    def __init__(self, callback, value=0):
        self.callback, self.value = callback, value

    def __index__(self):
        self.callback()
        return self.value


def commands(callback):
    callback()
    yield ("move", 20, 20)
    yield ("line", 60, 60)


def cases():
    image, font = quadrant_png(), FONT.read_bytes()
    return {
        "point": lambda p, cb: p.draw_text("NEW", at=(CallbackFloat(cb), 40)),
        "box": lambda p, cb: p.draw_text("NEW", box=(CallbackFloat(cb), 40, 100, 50)),
        "size": lambda p, cb: p.draw_text("NEW", at=(20, 40), font_size=CallbackFloat(cb, 12)),
        "text_color": lambda p, cb: p.draw_text("NEW", at=(20, 40), color=CallbackColor(cb)),
        "font_point": lambda p, cb: p.draw_text("NEW", at=(20, 40), font=CallbackStream(font, cb)),
        "font_box": lambda p, cb: p.draw_text("NEW", box=(20, 40, 100, 50), font=CallbackStream(font, cb)),
        "path_iterable": lambda p, cb: p.draw_path(commands(cb)),
        "path_width": lambda p, cb: p.draw_path(commands(lambda: None), width=CallbackFloat(cb, 1)),
        "path_color": lambda p, cb: p.draw_path(commands(lambda: None), fill=CallbackColor(cb)),
        "image_stream": lambda p, cb: p.add_image(CallbackStream(image, cb), box=(20, 20, 40, 40)),
        "image_box": lambda p, cb: p.add_image(image, box=(CallbackFloat(cb), 20, 40, 40)),
        "image_crop": lambda p, cb: p.add_image(image, box=(20, 20, 40, 40), crop=(CallbackFloat(cb, 0), 0, 0, 0)),
        "image_rotation": lambda p, cb: p.add_image(image, box=(20, 20, 40, 40), rotation=CallbackIndex(cb)),
        "page_box": lambda p, cb: p.set_box("crop", (CallbackFloat(cb), 20, 100, 100)),
        "page_background": lambda p, cb: setattr(p, "background", CallbackColor(cb)),
        "page_rotation": lambda p, cb: p.rotate(CallbackIndex(cb, 90)),
    }


class PdfDrawingOwnershipTests(unittest.TestCase):
    def document(self):
        document = doc_ex.new("pdf", size=(180, 180))
        document.pages.add(size=(180, 180))
        document.pages[0].draw_text("FIRST", at=(20, 90))
        document.pages[-1].draw_text("SECOND", at=(20, 90))
        return document

    def test_move_and_delete_callbacks_do_not_draw_on_a_different_page(self):
        for name, draw in cases().items():
            for action in ("move", "delete"):
                with self.subTest(name=name, action=action), self.document() as document:
                    page = document.pages[0]
                    document.render_page(0, dpi=72)  # Warm the old snapshot.
                    expected = {}
                    def callback():
                        if action == "move":
                            document.pages.move(0, -1)
                        else:
                            del document.pages[0]
                        expected["bytes"] = document.to_bytes()
                        expected["pixels"] = document.render_page(0, dpi=72)
                    with self.assertRaisesRegex(ValueError, "stale"):
                        draw(page, callback)
                    self.assertEqual(document.to_bytes(), expected["bytes"])
                    self.assertEqual(document.render_page(0, dpi=72), expected["pixels"])
                    with doc_ex.open(document.to_bytes()) as reopened:
                        self.assertEqual(reopened[0].get_text().strip(), "SECOND")

    def test_close_callbacks_and_already_invalid_handles(self):
        for name, draw in cases().items():
            with self.subTest(name=name):
                document = self.document()
                page = document.pages[0]
                with self.assertRaises(DocumentClosedError):
                    draw(page, document.close)
            for invalid in ("closed", "stale"):
                with self.subTest(name=name, invalid=invalid), self.document() as document:
                    page = document.pages[0]
                    if invalid == "closed":
                        document.close()
                        error = DocumentClosedError
                    else:
                        document.pages.move(0, 1)
                        error = ValueError
                    def forbidden():
                        self.fail("normalization ran on an already-invalid handle")
                    with self.assertRaises(error):
                        draw(page, forbidden)

    def test_callback_errors_are_preserved_without_drawing(self):
        for name, draw in cases().items():
            with self.subTest(name=name), self.document() as document:
                before, pixels = document.to_bytes(), document.render_page(0, dpi=72)
                def fail():
                    raise RuntimeError("caller normalization failed")
                with self.assertRaisesRegex(RuntimeError, "caller normalization failed"):
                    draw(document.pages[-1], fail)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), pixels)

    def test_callback_failure_after_organization_keeps_caller_error(self):
        for name, draw in cases().items():
            with self.subTest(name=name), self.document() as document:
                page = document.pages[0]
                expected = []
                def fail_after_move():
                    document.pages.move(0, 1)
                    expected.append(document.to_bytes())
                    raise RuntimeError("callback moved pages then failed")
                with self.assertRaisesRegex(RuntimeError, "callback moved pages then failed"):
                    draw(page, fail_after_move)
                self.assertEqual(document.to_bytes(), expected[0])

    def test_copy_callback_keeps_live_handle_and_copy_independent(self):
        for name, draw in cases().items():
            with self.subTest(name=name), self.document() as document:
                before = document.to_bytes()
                copies = []
                draw(document.pages[-1], lambda: copies.append(document.copy()))
                try:
                    self.assertEqual(len(copies), 1)
                    self.assertEqual(copies[0].to_bytes(), before)
                    self.assertNotEqual(document.to_bytes(), before)
                    with doc_ex.open(document.to_bytes()) as reopened:
                        self.assertEqual(reopened[0].get_text().strip(), "FIRST")
                finally:
                    for copied in copies:
                        copied.close()

    def test_rotation_preserves_integer_coercion_and_geometry_handle_refresh(self):
        with self.document() as document:
            page = document.pages[-1]
            page.rotate(CallbackIndex(lambda: None, -90))
            self.assertEqual(page.number, 1)
            page.set_box("crop", (10, 10, 100, 100))
            self.assertEqual(page.size, (100, 100))
            before = document.to_bytes()
            for value, error in ((90.0, TypeError), (True, ValueError), (1 << 80, OverflowError)):
                with self.subTest(value=value), self.assertRaises(error):
                    page.rotate(value)
                self.assertEqual(document.to_bytes(), before)

    def test_native_bool_coercion_does_not_invoke_arbitrary_callbacks(self):
        class Flag:
            def __bool__(self):
                raise AssertionError("native conversion invoked arbitrary bool callback")
        with self.document() as document:
            before = document.to_bytes()
            with self.assertRaises(TypeError):
                document.pages[0].draw_path(commands(lambda: None), close=Flag())
            self.assertEqual(document.to_bytes(), before)


if __name__ == "__main__":
    unittest.main()
