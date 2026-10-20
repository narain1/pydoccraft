from io import BytesIO
import struct
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile
import zlib

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_authoring import image_fixture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def quadrants():
    colors = (b'\xff\0\0\xff', b'\0\xff\0\xff', b'\0\0\xff\xff', b'\xff\xff\0\xff')
    raw = b''.join(b'\0' + b''.join(colors[(y >= 10) * 2 + (x >= 20)] for x in range(40))
                   for y in range(20))
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 40, 20, 8, 6, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


def centers(png):
    width, pixels = rgba_pixels(png_chunks(png))
    result = []
    for color in (b'\xff\0\0\xff', b'\0\xff\0\xff', b'\0\0\xff\xff', b'\xff\xff\0\xff'):
        positions = [i // 4 for i in range(0, len(pixels), 4) if pixels[i:i + 4] == color]
        if not positions:
            raise AssertionError('missing quadrant color')
        result.append((sum(i % width for i in positions) / len(positions),
                       sum(i // width for i in positions) / len(positions), len(positions)))
    return result


class PictureTransformTests(unittest.TestCase):
    def test_crop_mirroring_pixels_roundtrip_and_cached_render(self):
        source = quadrants()
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 40
            picture = document.add_image(source, width=96, height=48, alt='Quadrants')
            original = document.render_page(0, dpi=144)
            normal = centers(original)
            for horizontal, vertical in ((True, False), (False, True)):
                picture.flip_h, picture.flip_v = horizontal, vertical
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        result = centers(reopened.render_page(0, dpi=144))
                    self.assertEqual(result[0][0] > result[1][0], horizontal)
                    self.assertEqual(result[0][1] > result[2][1], vertical)
            picture.flip_h = picture.flip_v = False
            with document.copy() as copied:
                picture.flip_h = True
                picture.flip_v = True
                picture.crop = (0.25, 0, 0, 0)
                changed = document.render_page(0, dpi=144)
                self.assertNotEqual(changed, original)
                self.assertEqual(copied.render_page(0, dpi=144), original)
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        result = centers(reopened.render_page(0, dpi=144))
                    self.assertGreater(result[0][0], result[1][0])
                    self.assertGreater(result[0][1], result[2][1])
                    self.assertLess(result[0][2], result[1][2] * 0.6)
                    self.assertAlmostEqual(result[0][1], normal[2][1], delta=2)
                with ZipFile(BytesIO(document.to_bytes())) as archive:
                    xml = ET.fromstring(archive.read('word/document.xml'))
                a = '{http://schemas.openxmlformats.org/drawingml/2006/main}'
                self.assertEqual(xml.find(f'.//{a}srcRect').attrib['l'], '25000')
                transform = xml.find(f'.//{a}xfrm')
                self.assertEqual((transform.attrib['flipH'], transform.attrib['flipV']), ('1', '1'))
                self.assertEqual((picture.size, picture.alt, picture.data), ((96, 48), 'Quadrants', source))
                picture.data = BytesIO(source)
                self.assertEqual((picture.crop, picture.flip_h, picture.flip_v), ((0.25, 0, 0, 0), True, True))
                picture.crop = None
                picture.flip_h = False
                picture.flip_v = False
                self.assertEqual(document.render_page(0, dpi=144), original)

    def test_nested_picture_transform_ownership_replacement_and_bounds(self):
        with doc_ex.new('docx') as document:
            outside = document.add_image(quadrants(), width=60)
            cell = document.add_table([['']])[0, 0].add_table([['']])[0, 0]
            picture = cell.add_image(quadrants(), width=60, alt='Nested')
            picture.crop = (0, 0.25, 0, 0)
            picture.flip_v = True
            picture.data = image_fixture()
            picture.size = (40, 30)
            self.assertEqual((picture.crop, picture.flip_v, picture.size, picture.alt),
                             ((0, 0.25, 0, 0), True, (40, 30), 'Nested'))
            self.assertEqual(outside.crop, (0, 0, 0, 0))
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    self.assertTrue(reopened.render_page(0, dpi=72).startswith(b'\x89PNG'))
            before = document.to_bytes()
            for size in ((0.99, 20), (20, 0.5), (1000, 20)):
                with self.assertRaises(ValueError):
                    picture.size = size
                self.assertEqual(document.to_bytes(), before)
            del cell.images[0]
            with self.assertRaises((ValueError, IndexError)):
                picture.flip_h = True
            self.assertEqual(outside.crop, (0, 0, 0, 0))
            document.close()
            with self.assertRaises(DocumentClosedError):
                _ = outside.crop

    def test_invalid_changes_and_minimum_frame_are_atomic(self):
        with doc_ex.new('docx') as document:
            picture = document.add_image(quadrants(), width=60)
            before = document.to_bytes()
            for value in ((-0.1, 0, 0, 0), (0.451, 0, 0, 0), (float('nan'), 0, 0, 0),
                          (float('inf'), 0, 0, 0), (0, 0), 'crop'):
                with self.assertRaises((TypeError, ValueError)):
                    picture.crop = value
                self.assertEqual(document.to_bytes(), before)
            for value in (1, None, 'yes'):
                with self.assertRaises(TypeError):
                    picture.flip_h = value
                with self.assertRaises(TypeError):
                    picture.flip_v = value
                self.assertEqual(document.to_bytes(), before)
            for width, height in ((0.99, 2), (2, 0.99)):
                with self.assertRaises(ValueError):
                    document.add_image(quadrants(), width=width, height=height)
                self.assertEqual(document.to_bytes(), before)
            picture.size = (1, 1)
            self.assertEqual(picture.size, (1, 1))
            # Numeric conversion may execute caller code; the stable picture ID must be rechecked.
            class DeleteDuringFloat(float):
                def __float__(self):
                    del document.images[0]
                    return 0.1
            with self.assertRaises((ValueError, IndexError)):
                picture.crop = (DeleteDuringFloat(), 0, 0, 0)
            self.assertEqual(len(document.images), 0)


if __name__ == '__main__':
    unittest.main()
