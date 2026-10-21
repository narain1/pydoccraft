from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_transforms import quadrants
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

WP = '{http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing}'
A = '{http://schemas.openxmlformats.org/drawingml/2006/main}'


def bounds(png):
    width, pixels = rgba_pixels(png_chunks(png))
    colors = {b'\xff\0\0\xff', b'\0\xff\0\xff', b'\0\0\xff\xff', b'\xff\xff\0\xff'}
    indices = [i // 4 for i in range(0, len(pixels), 4) if pixels[i:i+4] in colors]
    return min(i % width for i in indices), min(i // width for i in indices), max(i % width for i in indices), max(i // width for i in indices)


class PicturePlacementTests(unittest.TestCase):
    def test_metadata_fields_and_pictures_keep_both_output_guards(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            document.update_metadata(title='Short')
            document.add_paragraph().add_field('title')
            picture = document.add_image(quadrants(), width=80, height=40)
            picture.format(wrap='behind', anchor='page', position=(160, 180))
            good = document.to_bytes()
            png = document.render_page(0, dpi=72)
            cached = document._cached
            revision = document._call('revision')
            formatting = picture.formatting
            with self.assertRaisesRegex(UnsupportedFormatError, 'floating picture'):
                picture.format(position=(250, 180))
            self.assertEqual(document._call('revision'), revision)
            self.assertEqual(picture.formatting, formatting)
            self.assertIs(document._cached, cached)
            self.assertEqual(document.to_bytes(), good)
            self.assertEqual(document.render_page(0, dpi=72), png)

            document.update_metadata(title=('Long title ' * 80).strip())
            with self.assertRaisesRegex(UnsupportedFormatError, 'metadata field'):
                document.to_bytes()
            revision = document._call('revision')
            with self.assertRaisesRegex(UnsupportedFormatError, 'metadata field'):
                picture.format(position=(150, 180))
            self.assertEqual(document._call('revision'), revision)
            self.assertEqual(picture.formatting, formatting)
            self.assertIsNone(document._cached)
            document.update_metadata(title='Short')
            self.assertEqual(document.to_bytes(), good)

            document.sections[0].page_size = (300, 200)
            for output in (document.to_bytes, lambda: document.convert('pdf')):
                with self.assertRaisesRegex(UnsupportedFormatError, 'floating picture'):
                    output()
            document.sections[0].page_size = (300, 300)
            self.assertEqual(document.to_bytes(), good)
            for filetype, data in (('docx', good), ('pdf', document.convert('pdf'))):
                with doc_ex.open(data, filetype=filetype) as reopened:
                    if filetype == 'pdf':
                        self.assertEqual(reopened[0].get_text().count('Short'), 1)
                    for dpi in (72, 144):
                        scale = dpi / 72
                        observed = bounds(reopened.render_page(0, dpi=dpi))
                        expected = (160*scale, 180*scale, 240*scale-1, 220*scale-1)
                        for got, want in zip(observed, expected):
                            self.assertAlmostEqual(got, want, delta=1)

    def test_page_margin_paragraph_roundtrip_png_pdf_and_owned_snapshot(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            picture = document.add_image(quadrants(), width=80, height=40, alt='Placement')
            self.assertEqual(picture.formatting, dict(wrap='inline', anchor='paragraph', position=(0, 0), distance=(0, 0, 0, 0)))
            for anchor, x, y in (('page', 90, 120), ('margin', 120, 150), ('paragraph', 120, 150)):
                picture.format(wrap='behind', anchor=anchor, position=(90, 120), distance=(2, 4, 6, 4))
                expected = dict(wrap='behind', anchor=anchor, position=(90, 120), distance=(2, 4, 6, 4))
                self.assertEqual(picture.formatting, expected)
                snapshot = picture.formatting
                snapshot['position'] = (0, 0)
                self.assertEqual(picture.formatting, expected)
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        for dpi in (72, 144, 300):
                            observed = bounds(reopened.render_page(0, dpi=dpi))
                            scale = dpi / 72
                            for got, want in zip(observed, (x*scale, y*scale, (x+80)*scale-1, (y+40)*scale-1)):
                                self.assertAlmostEqual(got, want, delta=1)
                with ZipFile(BytesIO(document.to_bytes())) as archive:
                    xml = ET.fromstring(archive.read('word/document.xml'))
                node = xml.find(f'.//{WP}anchor')
                self.assertEqual(node.attrib['behindDoc'], '1')
                self.assertEqual(node.attrib['distL'], node.attrib['distR'])
                self.assertEqual(node.find(f'{WP}positionV/{WP}posOffset').text, '1524000')
                self.assertEqual((picture.data, picture.alt), (quadrants(), 'Placement'))

    def test_wrapping_reset_copy_transforms_and_anchor_page_ownership(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            document.add_paragraph('First page')
            document.add_page_break()
            picture = document.add_image(quadrants(), width=80, height=40)
            picture.flip_h = True
            picture.crop = (0.25, 0, 0, 0)
            text = document.add_paragraph('Words flowing beside the image. ' * 20)
            for wrap in ('square', 'top_bottom', 'behind'):
                picture.format(wrap=wrap, anchor='page', position=(100, 60), distance=5)
                self.assertEqual(picture.formatting['wrap'], wrap)
                with ZipFile(BytesIO(document.to_bytes())) as archive:
                    xml = ET.fromstring(archive.read('word/document.xml'))
                node = xml.find(f'.//{WP}anchor')
                self.assertIsNotNone(node)
                self.assertEqual(bounds(document.render_page(1, dpi=72)), (100, 60, 179, 99))
                with doc_ex.open(document.convert('pdf')) as reopened:
                    self.assertIn('First page', reopened[0].get_text())
                    self.assertEqual(bounds(reopened.render_page(1, dpi=72)), (100, 60, 179, 99))
            with document.copy() as copied:
                picture.format(wrap='inline')
                self.assertEqual(copied.images[0].formatting['wrap'], 'behind')
                self.assertEqual(picture.formatting['wrap'], 'inline')
                self.assertEqual((picture.flip_h, picture.crop), (True, (0.25, 0, 0, 0)))
                self.assertEqual(text.text, 'Words flowing beside the image. ' * 20)
                with ZipFile(BytesIO(document.to_bytes())) as archive:
                    xml = ET.fromstring(archive.read('word/document.xml'))
                self.assertIsNotNone(xml.find(f'.//{WP}inline'))
                self.assertIsNone(xml.find(f'.//{WP}anchor'))

    def test_invalid_scope_and_out_of_page_changes_are_atomic(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            picture = document.add_image(quadrants(), width=80, height=40)
            cell_picture = document.add_table([['']])[0, 0].add_image(quadrants(), width=20)
            header_picture = document.sections[0].header().add_image(quadrants(), width=20)
            before = document.to_bytes()
            for invalid in (dict(wrap='front'), dict(wrap='tight'), dict(wrap='through'),
                            dict(wrap='behind', distance=(1, 2, 3, 4)),
                            dict(wrap='behind', anchor='page', position=(250, 30)),
                            dict(wrap='behind', anchor='page', position=(-1, 30))):
                with self.assertRaises(UnsupportedFormatError):
                    picture.format(**invalid)
                self.assertEqual(document.to_bytes(), before)
            for invalid in (dict(wrap='bad'), dict(anchor='page'), dict(wrap='inline', distance=0),
                            dict(wrap='behind', position=(0.001, 20)), dict(wrap='behind', distance=-1),
                            dict(wrap='behind', distance=1584.05), dict(wrap='behind', position=(31680.05, 0)),
                            dict(wrap='behind', position=(True, 0)), dict(wrap='behind', position=(float('nan'), 0)),
                            dict(wrap='behind', position=(0,)), dict(unknown=1)):
                with self.assertRaises((ValueError, TypeError)):
                    picture.format(**invalid)
                self.assertEqual(document.to_bytes(), before)
            for nested in (cell_picture, header_picture):
                with self.assertRaises(UnsupportedFormatError):
                    nested.format(wrap='behind', anchor='page', position=(30, 30))
                self.assertEqual(document.to_bytes(), before)
            picture.format(wrap='behind', anchor='page', position=(200, 100))
            document.sections[0].page_size = (300, 130)
            with self.assertRaises(UnsupportedFormatError):
                document.to_bytes()
            document.sections[0].page_size = (300, 300)
            self.assertEqual(bounds(document.render_page(0, dpi=72))[2:], (279, 139))

    def test_behind_picture_retains_text_on_both_sides_of_anchor(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            document.add_paragraph('BLACK LABEL BEFORE')
            picture = document.add_image(quadrants(), width=200, height=100)
            document.add_paragraph('BLACK LABEL AFTER')
            picture.format(wrap='behind', anchor='page', position=(30, 30))
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    text = reopened[0].get_text()
                    self.assertEqual(text.count('BLACK LABEL BEFORE'), 1)
                    self.assertEqual(text.count('BLACK LABEL AFTER'), 1)
                    width, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                    dark = [i // 4 for i in range(0, len(pixels), 4) if pixels[i:i+4] == b'\0\0\0\xff']
                    self.assertGreater(len(dark), 20)
                    self.assertTrue(all(30 <= i % width < 230 and 30 <= i // width < 130 for i in dark))

    def test_callbacks_stale_close_and_held_handles(self):
        with doc_ex.new('docx') as document:
            text = document.add_paragraph('Stable')
            picture = document.add_image(quadrants(), width=40)
            table = document.add_table([['Stable table']])
            revision = document._call('revision')
            picture.format()
            self.assertEqual(document._call('revision'), revision)
            class MutatingFloat(float):
                def __float__(self):
                    text.text = 'Callback edit'
                    return 10.0
            with self.assertRaisesRegex(ValueError, 'changed'):
                picture.format(wrap='behind', position=(MutatingFloat(), 0))
            self.assertEqual(text.text, 'Callback edit')
            self.assertEqual(picture.formatting['wrap'], 'inline')
            picture.format(wrap='behind', anchor='page', position=(60.05, 70.15))
            self.assertEqual(picture.formatting['position'], (60.05, 70.15))
            self.assertEqual(table[0, 0].text, 'Stable table')
            picture.data = quadrants()
            self.assertEqual(picture.formatting['position'], (60.05, 70.15))
            class DeletingFloat(float):
                def __float__(self):
                    del document.images[0]
                    return 10.0
            with self.assertRaises((IndexError, ValueError)):
                picture.format(position=(DeletingFloat(), 0))
            with self.assertRaises((IndexError, ValueError)):
                picture.format(wrap='behind')
            document.close()
            with self.assertRaises(DocumentClosedError):
                picture.format(wrap='behind', position=(object(), 0))


if __name__ == '__main__':
    unittest.main()
