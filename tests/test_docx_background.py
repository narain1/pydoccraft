from io import BytesIO
from pathlib import Path
import struct
from tempfile import TemporaryDirectory
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile
import zlib

import doc_ex
from doc_ex.errors import DocumentClosedError, MalformedDocumentError, UnsafeArchiveError, UnsupportedFormatError
from test_authoring import image_fixture
from test_picture_media import media_parts, red_picture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def transparent_picture():
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    row = bytes((255, 0, 0, 255)) * 8 + bytes((0, 0, 255, 0)) * 8
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 16, 8, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress((b'\0' + row) * 8)) + chunk(b'IEND', b''))


def page_edges(png):
    chunks = png_chunks(png)
    width, height = struct.unpack('>II', chunks[b'IHDR'][:8])
    _, pixels = rgba_pixels(chunks)
    return (width, height), [tuple(pixels[((height // 2) * width + x) * 4:
                                        ((height // 2) * width + x) * 4 + 4])
                             for x in (5, width - 6)]


class DocxBackgroundTests(unittest.TestCase):
    def test_native_frame_roundtrip_rgba_color_precedence_and_mixed_pages(self):
        payload = transparent_picture()
        with doc_ex.new('docx', size=(240, 320)) as document:
            document.sections[0].margins = 20
            document.background = '#00FF00'
            document.background_image = payload
            document.add_paragraph('First page')
            document.sections.add().page_size = (320, 240)
            document.add_paragraph('Second page')
            self.assertEqual(document.background_image, payload)
            self.assertEqual(len(document.images), 0)
            raw = document.to_bytes()
            with ZipFile(BytesIO(raw)) as archive:
                xml = ET.fromstring(archive.read('word/document.xml'))
                fill = xml.find('{http://schemas.openxmlformats.org/wordprocessingml/2006/main}background/'
                                '{urn:schemas-microsoft-com:vml}background/{urn:schemas-microsoft-com:vml}fill')
                self.assertIsNotNone(fill)
                self.assertEqual(fill.attrib['type'], 'frame')
                self.assertEqual(media_parts(document), [payload])
            for kind, data in [('docx', raw), ('pdf', document.convert('pdf'))]:
                with self.subTest(kind=kind), doc_ex.open(data) as reopened:
                    self.assertEqual(len(reopened), 2)
                    for number, size in enumerate(((240, 320), (320, 240))):
                        self.assertIn(('First', 'Second')[number], reopened[number].get_text())
                        for dpi in (72, 144):
                            dimensions, colors = page_edges(reopened.render_page(number, dpi=dpi))
                            self.assertEqual(dimensions, tuple(value * dpi // 72 for value in size))
                            self.assertEqual(colors, [(255, 0, 0, 255), (0, 255, 0, 255)])
                            clipped = reopened.render_page(number, dpi=dpi,
                                                           clip=(size[0] - 32, 0, 32, 32))
                            _, pixels = rgba_pixels(png_chunks(clipped))
                            self.assertEqual(pixels, b'\x00\xff\x00\xff' * ((32 * dpi // 72) ** 2))
            before = document.render_page(0, dpi=72)
            with document.copy() as copied:
                document.background = '#0000FF'
                self.assertEqual(page_edges(document.render_page(0, dpi=72))[1],
                                 [(255, 0, 0, 255), (0, 0, 255, 255)])
                document.background_image = None
                self.assertEqual(page_edges(document.render_page(0, dpi=72))[1],
                                 [(0, 0, 255, 255)] * 2)
                self.assertEqual(document.background, '#0000FF')
                self.assertIsNone(document.background_image)
                self.assertEqual(media_parts(document), [])
                self.assertEqual(copied.render_page(0, dpi=72), before)
                self.assertEqual(copied.background_image, payload)

    def test_shared_inline_nested_note_and_field_resources_survive_replace_and_clear(self):
        blue, red = image_fixture(), red_picture(4, 4)
        with doc_ex.new('docx') as document:
            document.background_image = blue
            document.add_image(blue, width=32)
            cell = document.add_table([['']])[0, 0].add_table([['Nested']])[0, 0]
            image = cell.add_image(blue, width=24)
            anchor = document.add_paragraph('Anchor')
            note = document.notes.add(anchor.find('Anchor')[0])
            cell.add_paragraph().add_field('page_count')
            for paragraph in (anchor, note.paragraphs[0],
                              document.sections[0].header().add_paragraph()):
                paragraph.add_field('page')
            self.assertEqual(media_parts(document), [blue])
            with document.copy() as copied:
                document.background_image = red
                self.assertCountEqual(media_parts(document), [blue, red])
                self.assertEqual(image.data, blue)
                del document.images[0]
                del cell.images[0]
                self.assertEqual(media_parts(document), [red])
                self.assertEqual(media_parts(copied), [blue])
                with doc_ex.open(document.convert('pdf')) as pdf:
                    self.assertIn('Anchor', pdf[0].get_text())
                    self.assertEqual(page_edges(pdf.render_page(0, dpi=72))[1], [(255, 0, 0, 255)] * 2)
                document.background_image = None
                self.assertEqual(media_parts(document), [])
                document.add_image(blue, width=20)
                document.background_image = blue
                document.background_image = None
                self.assertEqual(media_parts(document), [blue])
                self.assertEqual(copied.background_image, blue)

    def test_sources_invalid_input_and_bounded_streams_are_transactional(self):
        payload = image_fixture()
        with TemporaryDirectory() as directory, doc_ex.new('docx') as document:
            path = Path(directory) / 'background.png'
            path.write_bytes(payload)
            for source in (path, BytesIO(payload), bytearray(payload), memoryview(payload)):
                document.background_image = source
                self.assertIsInstance(document.background_image, bytes)
                self.assertEqual(document.background_image, payload)
            before = document.to_bytes()
            for source, error in ((b'not PNG', UnsupportedFormatError),
                                  (payload[:-8], MalformedDocumentError),
                                  (b'x' * (16 * 1024 * 1024 + 1), UnsafeArchiveError)):
                with self.subTest(error=error), self.assertRaises(error):
                    document.background_image = source
                self.assertEqual(document.to_bytes(), before)
            class Endless:
                consumed = 0
                def read(self, count):
                    self.consumed += count
                    return b'x' * count
            source = Endless()
            with self.assertRaises(UnsafeArchiveError):
                document.background_image = source
            self.assertEqual(source.consumed, 16 * 1024 * 1024 + 1)
            self.assertEqual(document.to_bytes(), before)
            document.background_image = None
            self.assertIsNone(document.background_image)

    def test_stream_callbacks_preserve_edits_and_recheck_owner_lifetime(self):
        payload = image_fixture()
        class Callback(BytesIO):
            def __init__(self, action):
                super().__init__(payload)
                self.action = action
            def read(self, count):
                if self.action is not None:
                    action, self.action = self.action, None
                    action()
                return super().read(count)
        with doc_ex.new('docx') as document:
            def edit():
                document.add_paragraph('Caller edit')
                document.background = '#00FF00'
            document.background_image = Callback(edit)
            self.assertEqual(document.paragraphs[0].text, 'Caller edit')
            self.assertEqual(document.background, '#00FF00')
            self.assertEqual(document.background_image, payload)
            owned = document.background_image
            with self.assertRaises(DocumentClosedError):
                document.background_image = Callback(document.close)
            self.assertEqual(owned, payload)
            with self.assertRaises(DocumentClosedError):
                _ = document.background_image
            for value in (None, Callback(lambda: self.fail('closed owner read the stream'))):
                with self.assertRaises(DocumentClosedError):
                    document.background_image = value


if __name__ == '__main__':
    unittest.main()
