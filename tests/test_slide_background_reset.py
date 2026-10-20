"""Removing an explicit owned slide background restores native inheritance."""
import io
import unittest
import xml.etree.ElementTree as ET
import zipfile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_pdf_text_boxes import quadrant_png, pixel
from test_slide_pictures import media_parts
from test_slide_shape_ownership import Color


class SlideBackgroundResetTests(unittest.TestCase):
    def test_reset_restores_native_background_source_pixels_pdf_and_copy(self):
        with doc_ex.new('pptx', size=(160, 160)) as document:
            slide = document.slides.add()
            shape = slide.add_text('Retained', box=(30, 50, 100, 40))
            shape.link = 'https://example.test/retained'
            baseline = slide.get_pixmap(dpi=72)
            self.assertIsNone(slide.background)
            for override in ('solid', 'picture'):
                with self.subTest(override=override):
                    if override == 'solid':
                        slide.background = 'blue'
                    else:
                        slide.background_image = quadrant_png()
                        with self.assertRaises(UnsupportedFormatError):
                            _ = slide.background
                    self.assertNotEqual(slide.get_pixmap(dpi=72), baseline)
                    selection = shape[:]
                    with document.copy() as copied:
                        cached = document._cached
                        slide.background = None
                        self.assertIsNone(slide.background)
                        self.assertIsNone(slide.background_image)
                        self.assertIsNone(document._cached)
                        self.assertIsNotNone(cached)
                        with self.assertRaisesRegex(ValueError, 'stale'):
                            _ = selection.text
                        self.assertEqual(shape.text, 'Retained')
                        self.assertEqual(shape.link, 'https://example.test/retained')
                        self.assertEqual(slide.get_pixmap(dpi=72), baseline)
                        self.assertNotEqual(copied.render_page(0, dpi=72), baseline)
                    data = document.to_bytes()
                    with zipfile.ZipFile(io.BytesIO(data)) as archive:
                        root = ET.fromstring(archive.read('ppt/slides/slide1.xml'))
                        self.assertIsNone(root.find('{http://schemas.openxmlformats.org/presentationml/2006/main}cSld/{http://schemas.openxmlformats.org/presentationml/2006/main}bg'))
                    with doc_ex.open(data) as reopened:
                        self.assertEqual(reopened.render_page(0, dpi=72), baseline)
                    with doc_ex.open(document.convert('pdf')) as reopened:
                        self.assertEqual(reopened.page_count, 1)
                        self.assertIn('Retained', reopened[0].get_text())
                        self.assertEqual(pixel(reopened[0].get_pixmap(dpi=72), 10, 10), b'\xff\xff\xff\xff')
                        self.assertTrue(any(link.uri == 'https://example.test/retained' for link in reopened[0].get_links()))
                    self.assertEqual(media_parts(data), {})
            slide.background = None  # No cache/revision-preserving no-op promise.
            self.assertEqual(slide.get_pixmap(dpi=72), baseline)

    def test_shared_media_retained_then_reclaimed_after_last_reference(self):
        image = quadrant_png()
        with doc_ex.new('pptx', size=(160, 160)) as document:
            first, second = document.slides.add(), document.slides.add()
            first.background_image = image
            second.background_image = image
            picture = first.add_image(image, box=(40, 40, 80, 80), alt='Shared')
            second_before = second.get_pixmap(dpi=72)
            with document.copy() as copied:
                first.background = None
                self.assertEqual(picture.data, image)
                self.assertEqual(picture.alt, 'Shared')
                self.assertEqual(second.get_pixmap(dpi=72), second_before)
                self.assertEqual(copied.slides[0].background_image, image)
                self.assertEqual(len(media_parts(document.to_bytes())), 1)
                second.background = None
                self.assertEqual(len(media_parts(document.to_bytes())), 1)
                self.assertEqual(pixel(first.get_pixmap(dpi=72), 60, 60), b'\xff\0\0\xff')
                del first.shapes[0]
                self.assertEqual(media_parts(document.to_bytes()), {})
            first.background_image = image
            first.background = None
            self.assertEqual(media_parts(document.to_bytes()), {})

    def test_stable_identity_callbacks_invalid_input_closed_deleted_and_copies(self):
        with doc_ex.new('pptx') as document:
            first, second = document.slides.add(), document.slides.add()
            first.background, second.background = 'blue', 'green'
            first.background = Color(lambda: document.slides.move(0, 1))
            self.assertEqual((first.number, second.number), (1, 0))
            self.assertEqual((first.background, second.background), ('#FF0000', '#008000'))
            first.background = None
            self.assertIsNone(first.background)
            self.assertEqual(second.background, '#008000')
            before = document.to_bytes()
            for invalid in (False, 1, [], 'invalid', (0, 0, 999)):
                with self.assertRaises((TypeError, ValueError)):
                    first.background = invalid
                self.assertEqual(document.to_bytes(), before)
            def deleted():
                del document.slides[first.number]
            with self.assertRaises(IndexError):
                first.background = Color(deleted)
            self.assertEqual(second.background, '#008000')
            def forbidden():
                raise AssertionError('callback on invalid owner')
            with self.assertRaises(IndexError):
                first.background = Color(forbidden)
            with self.assertRaises(IndexError):
                first.background = None
            document.close()
            with self.assertRaises(DocumentClosedError):
                second.background = Color(forbidden)
            with self.assertRaises(DocumentClosedError):
                second.background = None


if __name__ == '__main__':
    unittest.main()
