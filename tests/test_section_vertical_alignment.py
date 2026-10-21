"""Bounded native section alignment; excluded compositions remain explicit."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_authoring import image_fixture

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def modes(data):
    with ZipFile(BytesIO(data)) as archive:
        root = ET.fromstring(archive.read('word/document.xml'))
    return ['top' if section.find(W + 'vAlign') is None else
            section.find(W + 'vAlign').get(W + 'val')
            for section in root.findall('.//' + W + 'sectPr')]


def new():
    document = doc_ex.new('docx', size=(320, 420))
    document.sections[0].margins = (52, 36, 52, 36)
    return document


def red_box(data):
    width, pixels = rgba_pixels(png_chunks(data))
    ink = [i // 4 for i in range(0, len(pixels), 4)
           if pixels[i] > 120 and pixels[i] > pixels[i + 1] + 60 and pixels[i] > pixels[i + 2] + 60]
    if not ink:
        raise AssertionError('red body ink is missing')
    return (min(i % width for i in ink), min(i // width for i in ink),
            max(i % width for i in ink) + 1, max(i // width for i in ink) + 1)


class SectionVerticalAlignmentTests(unittest.TestCase):
    def test_values_copy_cache_ranges_strict_inputs_and_lifetime(self):
        document = new()
        section = document.sections[0]
        paragraph = document.add_paragraph('Body value')
        self.assertEqual(section.vertical_alignment, 'top')
        baseline = document.to_bytes()
        document.render_page(0, dpi=72)
        cached = document._cached
        revision = document._call('revision')
        retained = paragraph[:]
        class CallbackString(str):
            called = False
            def __eq__(self, other):
                self.called = True
                document.close()
                return True
        callback = CallbackString('center')
        for value in (None, 1, True, [], callback):
            with self.assertRaisesRegex(TypeError, 'must be a string'):
                section.vertical_alignment = value
            self.assertEqual(document._call('revision'), revision)
            self.assertIs(document._cached, cached)
            self.assertEqual(retained.text, 'Body value')
        self.assertFalse(callback.called)
        for value in ('both', 'Center', ' center ', ''):
            with self.assertRaisesRegex(ValueError, 'top, center, bottom or justify'):
                section.vertical_alignment = value
            self.assertIs(document._cached, cached)
            self.assertEqual(document.to_bytes(), baseline)
        with document.copy() as copied:
            for value, stored in [('center', 'center'), ('bottom', 'bottom'), ('justify', 'both')]:
                section.vertical_alignment = value
                self.assertEqual(section.vertical_alignment, value)
                self.assertEqual(modes(document.to_bytes()), [stored])
                self.assertEqual(copied.sections[0].vertical_alignment, 'top')
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = retained.text
            second = document.sections.add()
            self.assertEqual(second.vertical_alignment, 'justify')
            second.vertical_alignment = 'top'
            self.assertEqual(section.vertical_alignment, 'justify')
        document.close()
        with self.assertRaises(DocumentClosedError):
            _ = section.vertical_alignment
        for value in ('top', None):
            with self.assertRaises(DocumentClosedError):
                section.vertical_alignment = value

    def test_profile_is_preserved_by_later_mutations_and_top_recovers(self):
        with new() as document:
            paragraph = document.add_paragraph('Anchor')
            picture = document.add_image(image_fixture(4, 2), width=16, height=8)
            first = document.sections[0]
            first.vertical_alignment = 'center'
            second = document.sections.add()
            document.add_paragraph('Second section')
            self.assertEqual(second.vertical_alignment, 'center')
            document.render_page(0, dpi=72)
            cached = document._cached
            revision = document._call('revision')
            before = document.to_bytes()
            held = paragraph[:]
            actions = [lambda: setattr(second, 'columns', (2, 20, False)),
                       lambda: setattr(second, 'start', 'continuous'),
                       lambda: document.sections.add(start='odd_page'),
                       lambda: second.header('even').add_paragraph('Rejected even header'),
                       lambda: document.notes.add(paragraph[:], 'Note'),
                       lambda: picture.format(wrap='behind', anchor='page', position=(100, 100))]
            for action in actions:
                with self.assertRaisesRegex(UnsupportedFormatError, 'non-top section alignment'):
                    action()
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document._call('revision'), revision)
                self.assertIs(document._cached, cached)
                self.assertEqual(held.text, 'Anchor')
            first.vertical_alignment = 'top'
            with self.assertRaisesRegex(UnsupportedFormatError, 'non-top section alignment'):
                second.footer('even').add_paragraph('Rejected even footer')
            second.vertical_alignment = 'top'
            second.footer('even').add_paragraph('Even footer')
            with self.assertRaisesRegex(UnsupportedFormatError, 'non-top section alignment'):
                first.vertical_alignment = 'center'
        for direction in ('up', 'down'):
            for mode in ('center', 'bottom', 'justify'):
                with new() as document:
                    table = document.add_table([['Cell']])
                    nested = table[0, 0].add_table([['Nested']])
                    section = document.sections[0]
                    nested[0, 0].format(text_direction=direction)
                    before = document.to_bytes()
                    with self.assertRaisesRegex(UnsupportedFormatError, 'horizontal cells'):
                        section.vertical_alignment = mode
                    self.assertEqual(document.to_bytes(), before)
                    nested[0, 0].format(text_direction=None)
                    section.vertical_alignment = mode
                    before = document.to_bytes()
                    revision = document._call('revision')
                    for cell in (table[0, 0], nested[0, 0]):
                        with self.assertRaisesRegex(UnsupportedFormatError, 'horizontal cells'):
                            cell.format(text_direction=direction)
                        self.assertEqual(document.to_bytes(), before)
                        self.assertEqual(document._call('revision'), revision)
                    section.vertical_alignment = 'top'
                    table[0, 0].format(text_direction=direction)
                    self.assertTrue(document.to_bytes())
        for kind in ('footnote', 'endnote'):
            with new() as document:
                paragraph = document.add_paragraph('Anchor')
                document.notes.add(paragraph[:], 'Note detail', kind=kind)
                before = document.to_bytes()
                with self.assertRaisesRegex(UnsupportedFormatError, 'non-top section alignment'):
                    document.sections[0].vertical_alignment = 'bottom'
                self.assertEqual(document.to_bytes(), before)
        with new() as document:
            document.add_paragraph('Text')
            picture = document.add_image(image_fixture(4, 2), width=16, height=8)
            picture.format(wrap='behind', anchor='page', position=(100, 100))
            with self.assertRaisesRegex(UnsupportedFormatError, 'non-top section alignment'):
                document.sections[0].vertical_alignment = 'justify'
            picture.format(wrap='inline')
            document.sections[0].vertical_alignment = 'justify'
            self.assertTrue(document.to_bytes())

    def test_native_geometry_backgrounds_fields_stories_tables_and_pdf(self):
        with new() as document:
            section = document.sections[0]
            section.page_number_start = 7
            section.header_distance = 12
            section.footer_distance = 12
            section.header().add_paragraph('Header fixed')
            section.footer().add_paragraph('Footer fixed')
            paragraph = document.add_paragraph('Red marker')
            paragraph[:].format(color='red')
            document.add_paragraph('Page value ').add_field('page')
            table = document.add_table([['Table count ']])
            table[0, 0].paragraphs[0].add_field('page_count')
            table[0, 0].add_table([['Nested kept']])
            document.add_image(image_fixture(4, 2), width=16, height=8)
            document.background = '#DFF0FA'
            boxes = {}
            for value in ('top', 'center', 'bottom', 'justify'):
                section.vertical_alignment = value
                for picture in (False, True):
                    document.background_image = image_fixture(4, 2) if picture else None
                    source = document.to_bytes()
                    with doc_ex.open(source) as reopened, doc_ex.open(document.convert('pdf')) as pdf:
                        compact = ''.join(pdf[0].get_text().split())
                        self.assertIn('Pagevalue7', compact)
                        self.assertIn('Tablecount1', compact)
                        for label in ('Red marker', 'Header fixed', 'Footer fixed', 'Nested kept'):
                            self.assertEqual(pdf[0].get_text().count(label), 1)
                        for dpi in (72, 144):
                            native = document.render_page(0, dpi=dpi)
                            self.assertEqual(reopened.render_page(0, dpi=dpi), native)
                            converted = pdf.render_page(0, dpi=dpi)
                            a, b = red_box(native), red_box(converted)
                            self.assertTrue(all(abs(x-y) <= 2 for x, y in zip(a, b)), (value, picture, dpi, a, b))
                            if not picture and dpi == 72:
                                boxes[value] = a
                            for rendered in (native, converted):
                                _, pixels = rgba_pixels(png_chunks(rendered))
                                expected = bytes((32, 96, 192, 255)) if picture else bytes((223, 240, 250, 255))
                                self.assertTrue(all(abs(x-y) <= 1 for x, y in zip(pixels[:4], expected)))
            self.assertLess(boxes['top'][1], boxes['center'][1])
            self.assertLess(boxes['center'][1], boxes['bottom'][1])
            self.assertEqual(boxes['top'][1], boxes['justify'][1])

    def test_output_geometry_failure_after_typography_change_is_recoverable(self):
        with new() as document:
            paragraph = document.add_paragraph('Body value')
            section = document.sections[0]
            section.vertical_alignment = 'bottom'
            baseline = document.render_page(0, dpi=72)
            paragraph[:].format(font_size=400)
            for output in (document.to_bytes, lambda: document.render_page(0, dpi=72),
                           lambda: document.convert('pdf'), lambda: document.save(BytesIO())):
                with self.assertRaisesRegex(UnsupportedFormatError, 'vertical page bounds'):
                    output()
            paragraph[:].format(font_size=None)
            self.assertEqual(document.render_page(0, dpi=72), baseline)
            section.vertical_alignment = 'top'
            paragraph[:].format(font_size=400)
            # Top restores the pre-existing native profile; this new guard is inactive.
            self.assertTrue(document.to_bytes())
