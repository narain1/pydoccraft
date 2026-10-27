from io import BytesIO
import struct
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_picture_media import red_picture
from test_authoring import image_fixture

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
RED = bytes((255, 0, 0, 255))
BLUE = bytes((32, 96, 192, 255))


def settings(data):
    with ZipFile(BytesIO(data)) as archive:
        return ET.fromstring(archive.read('word/settings.xml')).find(W + 'mirrorMargins') is not None


def color_bounds(data, color):
    width, pixels = rgba_pixels(png_chunks(data))
    found = [i // 4 for i in range(0, len(pixels), 4) if pixels[i:i + 4] == color]
    if not found:
        return None
    return (min(i % width for i in found), min(i // width for i in found),
            max(i % width for i in found) + 1, max(i // width for i in found) + 1)


def setup(document, *, right=24):
    section = document.sections[0]
    section.margins = (60, right, 36, 72)
    section.gutter = 8
    section.header_distance = 12
    section.footer_distance = 12
    section.page_number_start = 2
    return section


class MirroredMarginTests(unittest.TestCase):
    def test_exact_bool_lifetime_copy_reset_and_revision_cache_contract(self):
        document = doc_ex.new('docx', size=(300, 300))
        setup(document)
        paragraph = document.add_paragraph('Retained body')
        document.bookmarks['body'] = paragraph[:]
        picture = document.sections[0].header().add_image(red_picture(4, 2), width=24, height=12)
        self.assertIs(document.mirror_margins, False)
        data = document.to_bytes()
        baseline = document.render_page(0, dpi=72)
        cached = document._cached
        revision = document._call('revision')
        retained = paragraph[:]
        retained_text = retained.text
        class Truthy:
            called = False
            def __bool__(self):
                self.called = True
                return True
        truthy = Truthy()
        for value in (None, 0, 1, 'true', [], truthy):
            with self.subTest(value=value):
                with self.assertRaisesRegex(TypeError, 'must be a bool'):
                    document.mirror_margins = value
                self.assertEqual(document._call('revision'), revision)
                self.assertIs(document._cached, cached)
                self.assertEqual(retained.text, retained_text)
        self.assertFalse(truthy.called)
        with document.copy() as copied:
            document.mirror_margins = True
            self.assertIs(copied.mirror_margins, False)
            self.assertIs(document.mirror_margins, True)
            self.assertEqual(document._call('revision'), revision + 1)
            self.assertNotEqual(document.render_page(0, dpi=72), baseline)
            self.assertEqual(picture.data, red_picture(4, 2))
            self.assertEqual(document.bookmarks['body'].text, 'Retained body')
            self.assertTrue(settings(document.to_bytes()))
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = retained.text
            current = paragraph[:]
            document.mirror_margins = True
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = current.text
            document.mirror_margins = False
            self.assertEqual(document.to_bytes(), data)
            self.assertEqual(document.render_page(0, dpi=72), baseline)
            copied.mirror_margins = True
            self.assertIs(document.mirror_margins, False)
        document.close()
        with self.assertRaises(DocumentClosedError):
            _ = document.mirror_margins
        for value in (True, 1):
            with self.assertRaises(DocumentClosedError):
                document.mirror_margins = value

    def test_sections_header_variants_gutter_table_and_pdf_follow_displayed_parity(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            first = setup(document)
            for variant in ('default', 'first', 'even'):
                story = first.header(variant)
                story.add_image(red_picture(4, 2), width=24, height=12, alt='Main logo')
                story.add_paragraph(f'Main {variant}')
                first.footer(variant).add_paragraph(f'Main {variant} footer')
            first_body = document.add_paragraph('Main alpha')
            first_body[:].link = 'https://example.com/margins'
            document.add_table([['Table keep', 'Value keep']])
            document.add_page_break()
            document.add_paragraph('Main bravo')
            second = document.sections.add()
            second.margins = (60, 30, 36, 60)
            second.gutter = 6
            second.page_number_start = 20
            for variant in ('default', 'first', 'even'):
                second.header(variant).images[0].data = image_fixture(4, 2)
                second.header(variant).paragraphs[-1].text = f'Appendix {variant}'
                second.footer(variant).paragraphs[0].text = f'Appendix {variant} footer'
            document.add_paragraph('Appendix alpha')
            document.add_page_break()
            document.add_paragraph('Appendix bravo')
            first.header().add_table([['Main header table']])
            for enabled in (False, True):
                document.mirror_margins = enabled
                data = document.to_bytes()
                self.assertEqual(settings(data), enabled)
                with doc_ex.open(data) as source, doc_ex.open(document.convert('pdf')) as pdf:
                    self.assertEqual(len(source), 5)
                    self.assertEqual(len(pdf), 5)
                    self.assertEqual([(p.metadata['section'], p.metadata['display_number']) for p in source],
                                     [(0, 2), (0, 3), (0, 4), (1, 20), (1, 21)])
                    for output in (source, pdf):
                        text = output.to_markdown()
                        for label in ('Main alpha', 'Main bravo', 'Appendix alpha', 'Appendix bravo', 'Table keep', 'Value keep'):
                            self.assertEqual(text.count(label), 1)
                        links = [link for page in output for link in page.get_links()]
                        self.assertEqual([link.uri for link in links if link.uri], ['https://example.com/margins'])
                    self.assertEqual(source[2].get_text(), '')
                    self.assertIn('Main even', pdf[2].get_text())
                    self.assertEqual(source[0].find_tables()[0].rows, [['Table keep', 'Value keep']])
                    for index, number in enumerate((2, 3, 4, 20, 21)):
                        inside, outside = (80, 24) if index < 3 else (66, 30)
                        x = outside if enabled and number % 2 == 0 else inside
                        color = RED if index < 3 else BLUE
                        expected_variant = 'first' if index in (0, 3) else 'even' if number % 2 == 0 else 'default'
                        prefix = 'Main' if index < 3 else 'Appendix'
                        self.assertIn(f'{prefix} {expected_variant}', pdf[index].get_text())
                        for output in (source, pdf):
                            for dpi in (72, 144, 300):
                                png = output.render_page(index, dpi=dpi, clip=(0, 0, 300, 60))
                                scale = dpi / 72
                                dims = struct.unpack('>II', png_chunks(png)[b'IHDR'][:8])
                                self.assertEqual(dims, (round(300 * scale), round(60 * scale)))
                                box = color_bounds(png, color)
                                self.assertIsNotNone(box)
                                for actual, wanted in zip(box, (x * scale, 12 * scale, (x + 24) * scale, 24 * scale)):
                                    self.assertLessEqual(abs(actual - wanted), 2)
                                self.assertIsNone(color_bounds(png, BLUE if color == RED else RED))
            self.assertEqual(first.header().images[0].data, red_picture(4, 2))
            self.assertEqual(second.header().images[0].data, image_fixture(4, 2))

    def test_line_numbers_and_columns_keep_native_output_after_natural_growth(self):
        for columns in (1, 2):
            with self.subTest(columns=columns), doc_ex.new('docx', size=(300, 300)) as document:
                section = setup(document, right=36)
                section.columns = (columns, 12, False)
                document.styles.format_defaults(font_size=10, space_after=0, line_spacing=1)
                paragraph = document.add_paragraph('\n'.join(f'Flow {i:03}' for i in range(45)))
                if columns == 1:
                    section.line_numbers = (1, 1, 12, 'each_page')
                document.mirror_margins = True
                for growth in (False, True):
                    if growth:
                        paragraph[len(paragraph.text):].text = '\n' + '\n'.join(f'Extra {i:03}' for i in range(40))
                    with doc_ex.open(document.to_bytes()) as source, doc_ex.open(document.convert('pdf')) as pdf:
                        self.assertGreater(len(source), 1)
                        if growth:
                            self.assertGreater(len(source), previous_pages)
                        previous_pages = len(source)
                        for output in (source, pdf):
                            text = output.to_markdown()
                            for label in (f'Flow {i:03}' for i in range(45)):
                                self.assertEqual(text.count(label), 1)
                            if growth:
                                for label in (f'Extra {i:03}' for i in range(40)):
                                    self.assertEqual(text.count(label), 1)
                            for index in range(len(output)):
                                image = output.render_page(index, dpi=72)
                                self.assertEqual(struct.unpack('>II', png_chunks(image)[b'IHDR'][:8]), (300, 300))
                                outside = 36 if source[index].metadata['display_number'] % 2 == 0 else 80
                                width, pixels = rgba_pixels(png_chunks(image))
                                label_ink = sum(max(pixels[(y * width + x) * 4:(y * width + x) * 4 + 3]) < 160
                                                for y in range(300) for x in range(outside - 2))
                                if columns == 1:
                                    self.assertGreater(label_ink, 10)
                                else:
                                    self.assertEqual(label_ink, 0)
                        for page in source:
                            base = 36 if page.metadata['display_number'] % 2 == 0 else 80
                            self.assertAlmostEqual(page.metadata['body_bbox'][0], base, places=3)
                        if columns == 1:
                            for page in pdf:
                                self.assertIn('1', page.get_text().split())

    def test_line_number_and_floating_overflow_toggle_rolls_back_bytes_ranges_and_cache(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            section = setup(document, right=10)
            paragraph = document.add_paragraph('Kept')
            section.line_numbers = (1, 1, 20, 'each_page')
            def rejected(message):
                data = document.to_bytes()
                png = document.render_page(0, dpi=72)
                cached = document._cached
                revision = document._call('revision')
                selected = paragraph[:]
                with self.assertRaisesRegex(UnsupportedFormatError, message):
                    document.mirror_margins = True
                self.assertIs(document.mirror_margins, False)
                self.assertEqual(document._call('revision'), revision)
                self.assertEqual(document.to_bytes(), data)
                self.assertIs(document._cached, cached)
                self.assertEqual(selected.text, 'Kept')
                self.assertEqual(document.render_page(0, dpi=72), png)
            rejected('line-number extent')
            section.line_numbers = None
            section.margins = (60, 24, 36, 72)
            picture = document.add_image(red_picture(4, 2), width=24, height=12)
            picture.format(wrap='behind', anchor='margin', position=(-30, 90))
            rejected('floating picture exceeds')
            picture.format(position=(4, 90))
            document.mirror_margins = True
            with doc_ex.open(document.to_bytes()) as source, doc_ex.open(document.convert('pdf')) as pdf:
                for output in (source, pdf):
                    box = color_bounds(output.render_page(0, dpi=72), RED)
                    self.assertEqual(box, (28, 150, 52, 162))
            section.line_numbers = (1, 1, 0, 'each_page')
            with self.assertRaisesRegex(UnsupportedFormatError, 'single-column'):
                section.columns = (2, 12, False)
