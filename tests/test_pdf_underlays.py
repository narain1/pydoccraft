"""Typed native PDF placement through the existing text/image methods."""
import base64
import io
import struct
import unittest
import zlib

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_pdf_drawing_ownership import CallbackFloat, CallbackStream
from test_pdf_resource_preservation import resource_pdf
from test_pdf_text_boxes import FONT, quadrant_png
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks

# Same original synthetic 2x1 JPEG as the existing native embedding regression.
JPEG = base64.b64decode("/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/2wBDAQkJCQwLDBgNDRgyIRwhMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjIyMjL/wAARCAABAAIDASIAAhEBAxEB/8QAHwAAAQUBAQEBAQEAAAAAAAAAAAECAwQFBgcICQoL/8QAtRAAAgEDAwIEAwUFBAQAAAF9AQIDAAQRBRIhMUEGE1FhByJxFDKBkaEII0KxwRVS0fAkM2JyggkKFhcYGRolJicoKSo0NTY3ODk6Q0RFRkdISUpTVFVWV1hZWmNkZWZnaGlqc3R1dnd4eXqDhIWGh4iJipKTlJWWl5iZmqKjpKWmp6ipqrKztLW2t7i5usLDxMXGx8jJytLT1NXW19jZ2uHi4+Tl5ufo6erx8vP09fb3+Pn6/8QAHwEAAwEBAQEBAQEBAQAAAAAAAAECAwQFBgcICQoL/8QAtREAAgECBAQDBAcFBAQAAQJ3AAECAxEEBSExBhJBUQdhcRMiMoEIFEKRobHBCSMzUvAVYnLRChYkNOEl8RcYGRomJygpKjU2Nzg5OkNERUZHSElKU1RVVldYWVpjZGVmZ2hpanN0dXZ3eHl6goOEhYaHiImKkpOUlZaXmJmaoqOkpaanqKmqsrO0tba3uLm6wsPExcbHyMnK0tPU1dbX2Nna4uPk5ebn6Onq8vP09fb3+Pn6/9oADAMBAAIRAxEAPwDiaKKK8Q/XD//Z")


def solid(rgb):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\0" + bytes(rgb) + b"\xff")) + chunk(b"IEND", b""))


def decoded(png):
    chunks = png_chunks(png)
    width, height = struct.unpack(">II", chunks[b"IHDR"][:8])
    actual_width, pixels = rgba_pixels(chunks)
    if actual_width != width or len(pixels) != width * height * 4:
        raise AssertionError("invalid decoded pixel extent")
    return width, height, pixels



def stored_pixels(png):
    """Decode the tiny RGB/RGBA stored fixture without assuming render-output RGBA."""
    if len(png) > 65536:
        raise AssertionError("stored fixture input is too large")
    chunks = png_chunks(png)
    width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", chunks[b"IHDR"])
    if (depth != 8 or color not in (2, 6) or not 1 <= width <= 20 or not 1 <= height <= 20
            or (compression, filtering, interlace) != (0, 0, 0)):
        raise AssertionError("unexpected stored fixture profile")
    channels = 3 if color == 2 else 4
    stride = width * channels
    expected = (stride + 1) * height
    decoder = zlib.decompressobj()
    data = decoder.decompress(chunks[b"IDAT"], expected + 1)
    if len(data) != expected or decoder.unconsumed_tail or decoder.unused_data or not decoder.eof:
        raise AssertionError("unexpected stored fixture length")
    pixels = bytearray(width * height * channels)
    for y in range(height):
        method = data[y * (stride + 1)]
        if method not in range(5):
            raise AssertionError("unexpected PNG row filter")
        for x in range(stride):
            pos = y * stride + x
            left = pixels[pos - channels] if x >= channels else 0
            above = pixels[pos - stride] if y else 0
            corner = pixels[pos - stride - channels] if y and x >= channels else 0
            if method == 4:
                estimate = left + above - corner
                distances = (abs(estimate - left), abs(estimate - above), abs(estimate - corner))
                predictor = (left, above, corner)[distances.index(min(distances))]
            else:
                predictor = (0, left, above, (left + above) // 2)[method]
            pixels[pos] = (data[y * (stride + 1) + 1 + x] + predictor) & 255
    rgba = bytes(pixels) if channels == 4 else b"".join(bytes(pixels[i:i+3]) + b"\xff" for i in range(0, len(pixels), 3))
    return width, height, rgba


def at(image, x, y, dpi=72):
    width, _, pixels = image
    scale = dpi / 72
    start = (int(y * scale) * width + int(x * scale)) * 4
    return pixels[start:start + 4]


class PdfUnderlayTests(unittest.TestCase):
    def test_owned_layering_text_images_copy_reopen_and_hidden_source(self):
        red, blue = solid((255, 0, 0)), solid((0, 0, 255))
        with doc_ex.new("pdf", size=(160, 160)) as document:
            page = document.pages[0]
            page.draw_text("Original", at=(5, 140), font_size=10)
            page.annotations.add_link(box=(5, 140, 60, 15), uri="https://example.com/kept")
            page.add_image(blue, box=(60, 60, 40, 40))
            page.draw_path([("move", 90, 60), ("line", 110, 60), ("line", 110, 90), ("line", 90, 90)],
                           stroke=None, fill="green", close=True)
            original = document.to_bytes()
            page.add_image(red, box=(40, 40, 80, 80), behind=True)
            page.draw_text("Point", at=(5, 5), font_size=10, behind=True)
            page.draw_text("Box", box=(5, 22, 145, 17), align="center", font_size=10, behind=True)
            page.add_image(JPEG, box=(5, 100, 20, 10), behind=True)
            before = document.to_bytes()
            with document.copy() as copy:
                copy.pages[0].add_image(red, box=(60, 60, 40, 40))
                self.assertEqual(at(decoded(copy.render_page(0, dpi=72)), 70, 70), b"\xff\0\0\xff")
                self.assertEqual(document.to_bytes(), before)
            for streams in (False, True):
                with doc_ex.open(document.to_bytes(object_streams=streams)) as reopened:
                    text = reopened[0].get_text()
                    for label in ("Original", "Point", "Box"):
                        self.assertEqual(text.split().count(label), 1)
                    self.assertEqual([link.uri for link in reopened[0].get_links()], ["https://example.com/kept"])
                    images = reopened[0].get_images()
                    self.assertEqual(len(images), 3)
                    self.assertEqual([image.data for image in images if image.media_type == "image/jpeg"], [JPEG])
                    self.assertEqual(sum(image.metadata["width"] == 1 and image.metadata["height"] == 1 for image in images), 2)
                    for dpi in (72, 144, 300):
                        image = decoded(reopened.render_page(0, dpi=dpi))
                        self.assertEqual(image[:2], (round(160 * dpi / 72),) * 2)
                        self.assertEqual(at(image, 50, 50, dpi), b"\xff\0\0\xff")
                        self.assertEqual(at(image, 70, 70, dpi), b"\0\0\xff\xff")
                        self.assertEqual(at(image, 100, 70, dpi), b"\0\x80\0\xff")
                        self.assertNotEqual(at(image, 15, 105, dpi), b"\xff" * 4)
                        self.assertEqual(at(image, 30, 100, dpi), b"\xff" * 4)
                        width, _, rgba = image
                        for y0, y1 in ((5, 20), (22, 39), (140, 156)):
                            ink = sum(all(value < 100 for value in rgba[(y * width + x) * 4:(y * width + x) * 4 + 3])
                                      for y in range(int(y0 * dpi / 72), int(y1 * dpi / 72))
                                      for x in range(int(5 * dpi / 72), int(150 * dpi / 72)))
                            self.assertGreater(ink, 10)
            with doc_ex.open(original) as source:
                self.assertNotIn("Point", source[0].get_text())
                self.assertEqual(at(decoded(source.render_page(0, dpi=72)), 50, 50), b"\xff" * 4)
        with doc_ex.new("pdf", size=(120, 120)) as document:
            page = document.pages[0]
            page.background = "white"
            blank = document.render_page(0, dpi=72)
            page.draw_text("Hidden", at=(5, 5), behind=True)
            page.add_image(red, box=(30, 30, 60, 60), behind=True)
            page.add_image(blue, box=(30, 30, 60, 60), behind=True)
            self.assertEqual(document.render_page(0, dpi=72), blank)
            self.assertEqual(page.get_text().count("Hidden"), 1)
        with doc_ex.new("pdf", size=(120, 120)) as revealed:
            page = revealed.pages[0]
            page.draw_text("Visible", at=(5, 5), behind=True)
            page.add_image(red, box=(30, 30, 60, 60), behind=True)
            page.add_image(blue, box=(30, 30, 60, 60), behind=True)
            self.assertEqual(at(decoded(revealed.render_page(0, dpi=72)), 60, 60), b"\xff\0\0\xff")

    def test_imported_view_and_representative_image_transform_profiles(self):
        # Four page views crossed with these four profiles, not every transform combination.
        profiles = ({}, {"rotation": 90, "flip_v": True},
                    {"rotation": 180, "flip_h": True, "crop": (0.1, 0.2, 0.15, 0.05)},
                    {"rotation": 270, "flip_h": True, "flip_v": True, "crop": (0, 0, 0.45, 0)})
        for rotation in (0, 90, 180, 270):
            with doc_ex.open(geometry_pdf(rotation)) as source:
                original_pixels = source.render_page(0, dpi=72)
                for options in profiles:
                    with self.subTest(rotation=rotation, options=options), source.edit() as under:
                        page = under.pages[0]
                        page.add_image(solid((128, 0, 128)), box=(45, 100, 10, 10))
                        with under.copy() as over:
                            page.add_image(quadrant_png(), box=(20, 80, 80, 80), behind=True, **options)
                            over.pages[0].add_image(quadrant_png(), box=(20, 80, 80, 80), **options)
                            # Full72DPI views retain exact extent; bounded image ROI gets all3DPIs.
                            self.assertEqual(decoded(under.render_page(0, dpi=72))[:2], tuple(int(v) for v in page.size))
                            for dpi in (72, 144, 300):
                                lower = decoded(under.render_page(0, dpi=dpi, clip=(20, 80, 80, 80)))
                                upper = decoded(over.render_page(0, dpi=dpi, clip=(20, 80, 80, 80)))
                                self.assertEqual(lower[:2], upper[:2])
                                width, height, pixels = lower
                                self.assertEqual(at(lower, 30, 25, dpi), b"\x80\0\x80\xff")
                                colors = set()
                                for y in range(height):
                                    for x in range(width):
                                        pos = (y * width + x) * 4
                                        # Exclude only the actual opaque cover plus its one-pixel AA edge.
                                        if not (24 * dpi / 72 <= x <= 36 * dpi / 72 and 19 * dpi / 72 <= y <= 31 * dpi / 72):
                                            self.assertEqual(pixels[pos:pos+4], upper[2][pos:pos+4])
                                        colors.add(pixels[pos:pos+4])
                                self.assertIn(b"\xff\0\0\xff", colors)
                                self.assertIn(b"\xff\xff\0\xff", colors)
                            with doc_ex.open(under.to_bytes()) as reopened:
                                self.assertEqual(reopened.render_page(0, dpi=72), under.render_page(0, dpi=72))
                                self.assertEqual(len(reopened[0].get_images()), 2)
                                image = next(v for v in reopened[0].get_images() if v.metadata["width"] == 20)
                                self.assertEqual(stored_pixels(image.data), stored_pixels(quadrant_png()))
                    self.assertEqual(source.render_page(0, dpi=72), original_pixels)

    def test_strict_boolean_font_read_and_callback_lifetimes(self):
        class Flag:
            def __bool__(self):
                raise AssertionError("arbitrary bool conversion")
        class Unread:
            def read(self, *args):
                raise AssertionError("source read before refusal")
        with doc_ex.new("pdf", size=(160, 160)) as document:
            page = document.pages[0]
            before = document.to_bytes()
            for value in (0, 1, None, "yes", Flag()):
                for operation in (lambda: page.draw_text("No", at=(5, 5), font=Unread(), behind=value),
                                  lambda: page.add_image(Unread(), box=(5, 5, 10, 10), behind=value)):
                    with self.assertRaisesRegex(TypeError, "behind"):
                        operation()
                    self.assertEqual(document.to_bytes(), before)
            for geometry in ({"at": (5, 5)}, {"box": (5, 5, 100, 30)}):
                with self.assertRaisesRegex(UnsupportedFormatError, "supplied fonts"):
                    page.draw_text("No", font=Unread(), behind=True, **geometry)
                self.assertEqual(document.to_bytes(), before)
            page.draw_text("Default font", at=(5, 5), font=FONT, behind=False)
            page.draw_path([("move", 5, 40), ("line", 100, 40)])
        draws = (
            lambda p, cb: p.draw_text("Outer", at=(CallbackFloat(cb, 5), 5), behind=True),
            lambda p, cb: p.draw_text("Outer", box=(CallbackFloat(cb, 5), 5, 100, 30), behind=True),
            lambda p, cb: p.add_image(CallbackStream(quadrant_png(), cb), box=(5, 40, 30, 30), behind=True),
        )
        for draw in draws:
            for action in ("close", "move", "stale", "edit"):
                with self.subTest(draw=draw, action=action), doc_ex.new("pdf", size=(160, 160)) as document:
                    document.pages.add(size=(160, 160))
                    page = document.pages[0]
                    expected = []
                    def callback():
                        if action == "close": document.close()
                        elif action == "move": document.pages.move(0, 1); expected.append(document.to_bytes())
                        elif action == "stale": self.fail("callback on stale page")
                        else: page.draw_text("Caller", at=(5, 120))
                    if action == "stale": document.pages.move(0, 1)
                    if action in ("close", "move", "stale"):
                        with self.assertRaises(DocumentClosedError if action == "close" else ValueError):
                            draw(page, callback)
                        if expected: self.assertEqual(document.to_bytes(), expected[0])
                    else:
                        draw(page, callback)
                        self.assertEqual(document.pages[0].get_text().count("Caller"), 1)

    def test_default_overlay_equivalence_and_failed_underlay_atomicity(self):
        with doc_ex.new("pdf", size=(160, 160)) as default, default.copy() as explicit:
            for document, options in ((default, {}), (explicit, {"behind": False})):
                page = document.pages[0]
                page.draw_text("Point", at=(5, 5), **options)
                page.draw_text("Box", box=(5, 30, 100, 30), **options)
                page.add_image(quadrant_png(), box=(30, 70, 80, 80), **options)
            self.assertEqual(default.to_bytes(), explicit.to_bytes())
            page = default.pages[0]
            before, png = default.to_bytes(), default.render_page(0, dpi=72)
            cache = default._cached
            for operation in (
                lambda: page.draw_text("Wide unbreakableword", box=(5, 5, 1, 10), behind=True),
                lambda: page.draw_text("No", at=(float("nan"), 5), behind=True),
                lambda: page.draw_text("word " * 10000, box=(5, 5, 100, 100), behind=True),
                lambda: page.add_image(b"invalid", box=(5, 5, 20, 20), behind=True),
                lambda: page.add_image(quadrant_png(), box=(150, 5, 20, 20), behind=True),
                lambda: page.add_image(quadrant_png(), box=(5, 5, 20, 20), crop=(0.5, 0, 0, 0), behind=True),
            ):
                with self.assertRaises((ValueError, UnsupportedFormatError, UnsafeArchiveError)):
                    operation()
                self.assertEqual(default.to_bytes(), before)
                self.assertIs(default._cached, cache)
                self.assertEqual(default.render_page(0, dpi=72), png)
        for inherited in (False, True):
            with doc_ex.open(resource_pdf("PCFHelvetica", inherited=inherited)) as original, original.edit() as document:
                page = document.pages[0]
                before, png = document.to_bytes(), document.render_page(0, dpi=72)
                cache = document._cached
                for geometry in ({"at": (10, 100)}, {"box": (10, 100, 100, 40)}):
                    with self.assertRaisesRegex(UnsupportedFormatError, "resource"):
                        page.draw_text("Fail", behind=True, **geometry)
                    self.assertEqual(document.to_bytes(), before)
                    self.assertIs(document._cached, cache)
                    self.assertEqual(document.render_page(0, dpi=72), png)
                page.add_image(quadrant_png(), box=(10, 100, 40, 40), behind=True)
                self.assertIn("MMMM WWWW", page.get_text())


if __name__ == "__main__":
    unittest.main()
