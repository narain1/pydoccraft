import io
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile

import doc_ex
from doc_ex._authoring import SlidePicture
from doc_ex.errors import DocumentClosedError, MalformedDocumentError, UnsafeArchiveError, UnsupportedFormatError
from test_pdf_text_boxes import quadrant_png, pixel

NS = {"p": "http://schemas.openxmlformats.org/presentationml/2006/main",
      "a": "http://schemas.openxmlformats.org/drawingml/2006/main"}
COLORS = (b"\xff\0\0\xff", b"\0\xff\0\xff", b"\0\0\xff\xff", b"\xff\xff\0\xff")


def media_parts(data):
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return {name: archive.read(name) for name in archive.namelist()
                if name.startswith("ppt/media/")}


class SlidePictureTests(unittest.TestCase):
    def test_native_crop_flips_clockwise_rotation_and_export_pixels(self):
        cases = ((0, False, False, (0, 0, 0, 0), (0, 1, 2, 3)),
                 (90, False, False, (0, 0, 0, 0), (2, 0, 3, 1)),
                 (180, False, False, (0, 0, 0, 0), (3, 2, 1, 0)),
                 (270, False, False, (0, 0, 0, 0), (1, 3, 0, 2)),
                 (0, True, False, (0, 0, 0, 0), (1, 0, 3, 2)),
                 (0, False, True, (0, 0, 0, 0), (2, 3, 0, 1)),
                 (90, True, False, (.45, 0, 0, 0), (3, 1, 3, 1)),
                 (0, False, False, (.45, .45, 0, 0), (3, 3, 3, 3)))
        image = quadrant_png()
        for rotation, horizontal, vertical, crop, expected in cases:
            with self.subTest(rotation=rotation, horizontal=horizontal, vertical=vertical, crop=crop), doc_ex.new("pptx", size=(160, 160)) as document:
                slide = document.slides.add()
                picture = slide.add_image(image, box=(40, 40, 80, 80))
                self.assertIsInstance(picture, SlidePicture)
                self.assertIsInstance(slide.shapes[0], SlidePicture)
                picture.crop, picture.flip_h, picture.flip_v, picture.rotation = crop, horizontal, vertical, rotation
                self.assertEqual(picture.data, image)
                self.assertEqual(picture.crop, crop)
                self.assertEqual((picture.flip_h, picture.flip_v), (horizontal, vertical))
                pptx = document.to_bytes()
                with zipfile.ZipFile(io.BytesIO(pptx)) as archive:
                    root = ET.fromstring(archive.read("ppt/slides/slide1.xml"))
                    rect = root.find(".//p:pic/p:blipFill/a:srcRect", NS)
                    if any(crop):
                        self.assertEqual(tuple(int(rect.attrib.get(key, 0)) for key in ("l", "t", "r", "b")), tuple(round(value * 100000) for value in crop))
                for output in (pptx, document.convert("pdf")):
                    with doc_ex.open(output) as reopened:
                        png = reopened.render_page(0, dpi=72)
                        for (x, y), color in zip(((60, 60), (100, 60), (60, 100), (100, 100)), expected):
                            self.assertEqual(pixel(png, x, y), COLORS[color])

    def test_replacement_ownership_metadata_shared_media_and_handle_identity(self):
        image = quadrant_png()
        # Reuse a valid second PNG with a different palette, without external imaging tools.
        from test_authoring import image_fixture
        PNG = image_fixture()
        with tempfile.TemporaryDirectory() as directory, doc_ex.new("pptx", size=(160, 160)) as document:
            path = Path(directory) / "source.png"
            path.write_bytes(image)
            slide = document.slides.add()
            slide.background_image = image
            picture = slide.add_image(path, box=(40, 40, 80, 80), alt="Original")
            sibling = slide.add_image(io.BytesIO(image), box=(5, 5, 20, 20))
            self.assertEqual(len(media_parts(document.to_bytes())), 1)
            picture.crop = [.1, .2, .3, .4]
            picture.flip_h, picture.flip_v, picture.rotation = True, True, 90
            picture.link, picture.link_tooltip = "https://example.com/picture", "Picture link"
            before = document.render_page(0, dpi=72)
            picture.data = io.BytesIO(PNG)
            picture.alt = "New café ✓"
            self.assertEqual(picture.data, PNG)
            self.assertEqual(picture.alt, "New café ✓")
            self.assertEqual(sibling.data, image)
            self.assertEqual(slide.background_image, image)
            self.assertEqual(picture.box, (40, 40, 80, 80))
            self.assertEqual(picture.crop, (.1, .2, .3, .4))
            self.assertEqual((picture.flip_h, picture.flip_v, picture.rotation), (True, True, 90))
            self.assertEqual((picture.link, picture.link_tooltip), ("https://example.com/picture", "Picture link"))
            self.assertNotEqual(document.render_page(0, dpi=72), before)
            self.assertEqual(len(media_parts(document.to_bytes())), 2)
            with document.copy() as copied:
                picture.data = path
                self.assertEqual(copied.slides[0].shapes[0].data, PNG)
            sibling.data = PNG
            picture.data = PNG
            self.assertEqual(len(media_parts(document.to_bytes())), 2)
            slide.background = "white"
            self.assertEqual(len(media_parts(document.to_bytes())), 1)
            picture.crop = None
            self.assertEqual(picture.crop, (0, 0, 0, 0))
            slide.shapes.move(0, 1)
            self.assertEqual(picture.alt, "New café ✓")
            document.slides.add()
            document.slides.move(0, 1)
            self.assertEqual(slide.number, 1)
            self.assertEqual(picture.data, PNG)
            del slide.shapes[1]
            with self.assertRaises((ValueError, IndexError)):
                _ = picture.data
        with self.assertRaises(DocumentClosedError):
            _ = sibling.crop

    def test_callback_inputs_resolve_original_slide_and_picture_after_reordering(self):
        from test_authoring import image_fixture

        class CallbackStream(io.BytesIO):
            def __init__(self, data, callback):
                super().__init__(data)
                self.callback = callback

            def read(self, size=-1):
                callback, self.callback = self.callback, None
                if callback is not None:
                    callback()
                return super().read(size)

        class CallbackNumber(float):
            def __float__(self):
                callback, self.callback = self.callback, None
                if callback is not None:
                    callback()
                return super().__float__()

        original, replacement = quadrant_png(), image_fixture()
        for operation in ("replace_stream", "add_stream", "add_coordinate"):
            with self.subTest(operation=operation), doc_ex.new("pptx", size=(160, 160)) as document:
                target, other = document.slides.add(), document.slides.add()
                picture = target.add_image(original, box=(40, 40, 80, 80), alt="Target")
                other_picture = other.add_image(original, box=(5, 5, 20, 20), alt="Other")
                reorder = lambda: document.slides.move(target.number, 1)
                if operation == "replace_stream":
                    picture.data = CallbackStream(replacement, reorder)
                    result = picture
                    self.assertEqual(len(target.shapes), 1)
                elif operation == "add_stream":
                    result = target.add_image(CallbackStream(replacement, reorder), box=(10, 10, 20, 20))
                else:
                    x = CallbackNumber(10)
                    x.callback = reorder
                    result = target.add_image(replacement, box=(x, 10, 20, 20))
                self.assertEqual(target.number, 1)
                self.assertEqual(other.number, 0)
                self.assertEqual(result.data, replacement)
                self.assertEqual(other_picture.data, original)
                self.assertEqual(len(other.shapes), 1)
                self.assertEqual(picture.alt, "Target")
                if operation != "replace_stream":
                    self.assertEqual(len(target.shapes), 2)
                    self.assertEqual(picture.data, original)
                    self.assertEqual(target.shapes[1].data, replacement)
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual(len(reopened), 2)
                    self.assertTrue(reopened.render_page(target.number, dpi=72))

    def test_invalid_mutations_preserve_serialization_and_warm_render(self):
        with doc_ex.new("pptx", size=(160, 160)) as document:
            slide = document.slides.add()
            picture = slide.add_image(quadrant_png(), box=(40, 40, 80, 80))
            before, png = document.to_bytes(), document.render_page(0, dpi=72)
            invalid = (("crop", (-.01, 0, 0, 0), ValueError),
                       ("crop", (0, .46, 0, 0), ValueError),
                       ("crop", (0, 0, float("nan"), 0), ValueError),
                       ("crop", (0, 0, 0, float("inf")), ValueError),
                       ("crop", (0, 0), TypeError), ("crop", "invalid", TypeError),
                       ("flip_h", 1, TypeError), ("flip_v", "false", TypeError),
                       ("alt", "bad\0text", ValueError),
                       ("data", b"not a PNG", UnsupportedFormatError),
                       ("data", quadrant_png()[:-12], MalformedDocumentError),
                       ("data", b"x" * (16 * 1024 * 1024 + 1), UnsafeArchiveError))
            for attribute, value, error in invalid:
                with self.subTest(attribute=attribute, size=len(value) if hasattr(value, "__len__") else value), self.assertRaises(error):
                    setattr(picture, attribute, value)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), png)
            with self.assertRaises(UnsafeArchiveError):
                slide.add_image(b"x" * (16 * 1024 * 1024 + 1), box=(0, 0, 10, 10))
            self.assertEqual(document.to_bytes(), before)


if __name__ == "__main__":
    unittest.main()
