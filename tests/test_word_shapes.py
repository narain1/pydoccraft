"""Owned closed-shape acceptance; no line/textbox/rotation fidelity claim."""
from io import BytesIO
from functools import lru_cache
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks
from test_picture_media import red_picture

A = '{http://schemas.openxmlformats.org/drawingml/2006/main}'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def document():
    result = doc_ex.new('docx', size=(300, 300))
    result.sections[0].margins = 30
    return result


@lru_cache(maxsize=2)
def colored_bounds(data):
    # Decode each PNG once for both colors; retain only two small bounds maps.
    width, pixels = rgba_pixels(png_chunks(data))
    colors = {b'\xff\x00\x00\xff': None, b'\x00\x00\xff\xff': None}
    for offset in range(0, len(pixels), 4):
        color = pixels[offset:offset+4]
        if color in colors:
            index = offset // 4
            x, y = index % width, index // width
            previous = colors[color]
            colors[color] = (x, y, x+1, y+1) if previous is None else (min(x, previous[0]), min(y, previous[1]), max(x+1, previous[2]), max(y+1, previous[3]))
    return colors


def bounds(data, color):
    return colored_bounds(data)[color]


class WordShapeTests(unittest.TestCase):
    def test_source_order_exact_handles_copy_deletion_and_closed_owner(self):
        d = document()
        first = d.add_shape('rectangle', width=40, height=20, fill='red')
        self.assertEqual(len(d.paragraphs), 0)
        self.assertEqual(len(d.images), 0)
        paragraph = d.add_paragraph('Retained paragraph')
        second = d.add_shape('ellipse', width=50, height=30, fill='blue', stroke='black', stroke_width=2)
        self.assertEqual([s.kind for s in d.shapes], ['rectangle', 'ellipse'])
        self.assertEqual([s.size for s in d.shapes[::-1]], [(50, 30), (40, 20)])
        self.assertEqual(d.shapes[-1].fill, '#0000FF')
        self.assertEqual(second.stroke, ('#000000', 2))
        copy = d.copy()
        copy.shapes[0].fill = 'green'
        self.assertEqual(first.fill, '#FF0000')
        del d.shapes[0]
        self.assertEqual(paragraph.text, 'Retained paragraph')
        self.assertEqual(second.size, (50, 30))
        self.assertEqual(len(d.shapes), 1)
        # Like picture deletion, a native empty anchor paragraph remains.
        self.assertEqual([p.text for p in d.paragraphs], ['', 'Retained paragraph'])
        replacement = d.add_shape('rectangle', width=40, height=20)
        self.assertNotEqual(first._id, replacement._id)
        for operation in (lambda: first.kind, lambda: first.size, lambda: first.fill,
                          lambda: first.stroke, lambda: first.formatting,
                          lambda: first.format(), lambda: setattr(first, 'size', (20, 20)),
                          lambda: setattr(first, 'fill', 'blue'), lambda: setattr(first, 'stroke', None)):
            with self.assertRaises(IndexError):
                operation()
        with self.assertRaises(TypeError):
            del d.shapes[:]
        d.close()
        for operation in (lambda: len(d.shapes), lambda: replacement.kind,
                          lambda: d.add_shape('ellipse', width=10, height=10)):
            with self.assertRaises(DocumentClosedError):
                operation()
        self.assertEqual(copy.shapes[0].fill, '#008000')
        self.assertEqual(copy.shapes[1].size, (50, 30))

    def test_inline_and_native_placement_docx_png_pdf_at_three_dpis(self):
        for kind in ('rectangle', 'ellipse'):
            with self.subTest(kind=kind):
                d = document()
                d.add_paragraph('BodyKeep')
                shape = d.add_shape(kind, width=60, height=30, fill='red', stroke='blue', stroke_width=2)
                d.add_paragraph('TailKeep')
                self.assertEqual(shape.formatting, dict(wrap='inline', anchor='paragraph', position=(0, 0), distance=(0, 0, 0, 0)))
                inline = d.render_page(0, dpi=72)
                self.assertIsNotNone(bounds(inline, b'\xff\x00\x00\xff'))
                for anchor, expected in (('page', (80, 100)), ('margin', (110, 130))):
                    shape.format(wrap='behind', anchor=anchor, position=(80, 100), distance=(2, 4, 6, 4))
                    snapshot = shape.formatting
                    snapshot['position'] = (-10, -10)
                    self.assertEqual(shape.formatting['position'], (80, 100))
                    data = d.to_bytes()
                    with ZipFile(BytesIO(data)) as archive:
                        xml = ET.fromstring(archive.read('word/document.xml'))
                        self.assertEqual([g.get('prst') for g in xml.iter(A+'prstGeom')], [kind if kind == 'ellipse' else 'rect'])
                    with doc_ex.open(data) as source, doc_ex.open(d.convert('pdf')) as pdf:
                        for opened in (source, pdf):
                            text = opened[0].get_text()
                            self.assertEqual(text.count('BodyKeep'), 1)
                            self.assertEqual(text.count('TailKeep'), 1)
                            self.assertNotIn('\ufffc', text)
                        for dpi in (72, 144, 300):
                            scale = dpi / 72
                            native = source.render_page(0, dpi=dpi)
                            converted = pdf.render_page(0, dpi=dpi)
                            self.assertEqual(native, d.render_page(0, dpi=dpi))
                            for png in (native, converted):
                                red = bounds(png, b'\xff\x00\x00\xff')
                                blue = bounds(png, b'\x00\x00\xff\xff')
                                self.assertIsNotNone(red)
                                self.assertIsNotNone(blue)
                                x, y = expected
                                for actual, wanted in zip(blue, ((x-1)*scale, (y-1)*scale, (x+61)*scale, (y+31)*scale)):
                                    self.assertAlmostEqual(actual, wanted, delta=2)
                            for actual, wanted in zip(bounds(native, b'\x00\x00\xff\xff'), bounds(converted, b'\x00\x00\xff\xff')):
                                self.assertAlmostEqual(actual, wanted, delta=2)
                shape.fill = None
                shape.stroke = None
                self.assertIsNone(bounds(d.render_page(0, dpi=72), b'\xff\x00\x00\xff'))
                self.assertIsNone(bounds(d.render_page(0, dpi=72), b'\x00\x00\xff\xff'))

    def test_strict_values_failed_mutations_cache_revision_and_full_stroke(self):
        d = document()
        shape = d.add_shape('rectangle', width=40, height=20, fill='red', stroke='blue', stroke_width=2)
        shape.format(wrap='behind', anchor='page', position=(80, 100))
        saved = d.to_bytes()
        png = d.render_page(0, dpi=72)
        cache = d._cached
        revision = d._call('revision')
        operations = [lambda: setattr(shape, 'size', 'a4'), lambda: setattr(shape, 'size', (True, 20)),
                      lambda: setattr(shape, 'size', (40.01, 20)), lambda: setattr(shape, 'size', (4001, 20)),
                      lambda: setattr(shape, 'size', (0, 20)), lambda: setattr(shape, 'fill', (0, 0, 256)),
                      lambda: setattr(shape, 'stroke', ('blue', 200)), lambda: setattr(shape, 'stroke', ('blue', 20.05)),
                      lambda: setattr(shape, 'size', (1, 1)), lambda: setattr(shape, 'stroke', ('blue', 0.2)),
                      lambda: setattr(shape, 'stroke', (None, 1)), lambda: shape.format(wrap='front'),
                      lambda: shape.format(position=(0, 100)), lambda: shape.format(rotation=90),
                      lambda: shape.format(distance=(1, 2, 3, 4)), lambda: shape.format(position=(float('nan'), 1)),
                      lambda: setattr(d.sections[0], 'page_size', (100, 300)),
                      lambda: d.add_shape('line', width=40, height=20),
                      lambda: d.add_shape('textbox', width=40, height=20),
                      lambda: d.add_shape('rectangle', width=40, height=20, stroke_width=200)]
        for operation in operations:
            with self.subTest(operation=operation):
                with self.assertRaises((TypeError, ValueError, UnsupportedFormatError)):
                    operation()
                self.assertEqual(d._call('revision'), revision)
                self.assertIs(d._cached, cache)
                self.assertEqual(d.to_bytes(), saved)
                self.assertEqual(d.render_page(0, dpi=72), png)
        shape.format(position=(1, 100))  # Exact half-stroke containment.
        self.assertEqual(shape.formatting['position'], (1, 100))
        shape.size = (50, 20)
        self.assertEqual(shape.size, (50, 20))
        self.assertNotEqual(d.to_bytes(), saved)

    def test_conversion_callbacks_cannot_retarget_or_delete_changed_owner(self):
        d = document()
        shape = d.add_shape('rectangle', width=40, height=20, fill='red')
        class Once(float):
            calls = 0
            def __float__(self):
                self.calls += 1
                return float.__float__(self)
        once = Once(2)
        d.add_shape('ellipse', width=20, height=20, stroke='blue', stroke_width=once)
        self.assertEqual(once.calls, 1)
        class Change(float):
            def __float__(self):
                d.add_paragraph('Callback retained')
                return float.__float__(self)
        for operation in (lambda: setattr(shape, 'size', (Change(40), 20)),
                          lambda: shape.format(position=(Change(80), 100)),
                          lambda: d.add_shape('ellipse', width=Change(20), height=20)):
            count = len(d.shapes)
            with self.assertRaisesRegex(ValueError, 'changed'):
                operation()
            self.assertEqual(len(d.shapes), count)
            self.assertEqual(shape.size, (40, 20))
        class Delete(float):
            def __float__(self):
                del d.shapes[0]
                d.add_shape('rectangle', width=40, height=20, fill='blue')
                return float.__float__(self)
        with self.assertRaises(IndexError):
            shape.stroke = ('red', Delete(2))
        self.assertEqual(d.shapes[-1].fill, '#0000FF')
        class ChangedIndex:
            def __index__(self):
                d.add_paragraph('Index callback retained')
                return 0
        count = len(d.shapes)
        with self.assertRaisesRegex(ValueError, 'changed'):
            del d.shapes[ChangedIndex()]
        self.assertEqual(len(d.shapes), count)
        fresh = d.shapes[0]
        class Close(float):
            def __float__(self):
                d.close()
                return float.__float__(self)
        with self.assertRaises(DocumentClosedError):
            fresh.size = (Close(20), 20)

    def test_composition_tables_styles_links_notes_images_and_source_identity(self):
        d = document()
        d.add_paragraph('OrdinaryStart')
        first = d.add_paragraph('Linked body')
        first[:].link = 'https://example.test/shape-body'
        d.bookmarks['retained'] = first[:]
        note_anchor = d.add_paragraph('NoteAnchor')
        d.notes.add(note_anchor[:], 'NoteKeep')
        shape = d.add_shape('rectangle', width=40, height=20, fill='blue')
        image = d.add_image(red_picture(4, 2), width=24, height=12, alt='ImageKeep')
        table = d.add_table([['CellKeep']])
        last = d.add_paragraph('LastKeep')
        d.add_paragraph('TailKeep')
        d.paragraphs.move(-2, 0)
        self.assertEqual(shape.size, (40, 20))
        style = d.styles.add('Unused shape style')
        del d.styles[style.name]
        self.assertEqual(shape.fill, '#0000FF')
        table[0, 0].text = 'CellChanged'
        self.assertEqual(shape.size, (40, 20))
        self.assertEqual(image.alt, 'ImageKeep')
        self.assertEqual(last.text, 'LastKeep')
        data = d.to_bytes()
        with doc_ex.open(data) as source, doc_ex.open(d.convert('pdf')) as pdf:
            for opened in (source, pdf):
                text = '\n'.join(page.get_text() for page in opened)
                for label in ('Linked body', 'CellChanged', 'LastKeep'):
                    self.assertEqual(text.count(label), 1, (label, text))
                self.assertTrue(any(link.uri == 'https://example.test/shape-body' for page in opened for link in page.get_links()))
            self.assertEqual('\n'.join(story.get_text() for story in source.stories).count('NoteKeep'), 1)
            self.assertEqual('\n'.join(page.get_text() for page in pdf).count('NoteKeep'), 1)
            # Existing extraction schema reports omitted shape geometry, not a fake image.
            notice, = [item for item in source.metadata['diagnostics'] if item['code'] == 'shape_geometry_omitted']
            self.assertEqual(notice['count'], 1)
            self.assertEqual(len(source[0].get_images()), 1)
        self.assertEqual(bounds(d.render_page(0, dpi=72), b'\x00\x00\xff\xff'), bounds(doc_ex.open(data).render_page(0, dpi=72), b'\x00\x00\xff\xff'))
        del d.tables[0]
        self.assertEqual(shape.fill, '#0000FF')
        self.assertEqual(image.alt, 'ImageKeep')

    def test_paragraph_anchor_wrap_mirroring_columns_and_section_rejection(self):
        d = document()
        d.sections[0].margins = (30, 50, 30, 40)
        d.sections[0].page_number_start = 2
        d.mirror_margins = True
        shape = d.add_shape('ellipse', width=40, height=20, fill='red')
        for wrap in ('square', 'top_bottom', 'behind'):
            shape.format(wrap=wrap, anchor='paragraph', position=(10, 40), distance=2)
            self.assertEqual(shape.formatting['anchor'], 'paragraph')
            self.assertEqual(shape.formatting['wrap'], wrap)
            with doc_ex.open(d.to_bytes()) as source, doc_ex.open(d.convert('pdf')) as pdf:
                expected = bounds(source.render_page(0, dpi=72), b'\xff\x00\x00\xff')
                self.assertIsNotNone(expected)
                for actual, wanted in zip(bounds(pdf.render_page(0, dpi=72), b'\xff\x00\x00\xff'), expected):
                    self.assertAlmostEqual(actual, wanted, delta=2)
        d.sections[0].columns = (2, 12, False)
        self.assertEqual(shape.size, (40, 20))
        saved = d.to_bytes()
        with self.assertRaises(UnsupportedFormatError):
            d.sections[0].vertical_alignment = 'center'
        self.assertEqual(d.to_bytes(), saved)
        d.sections[0].columns = (1, 12, False)
        shape.format(wrap='inline')
        d.sections[0].vertical_alignment = 'center'
        with doc_ex.open(d.to_bytes()) as source, doc_ex.open(d.convert('pdf')) as pdf:
            expected = bounds(source.render_page(0, dpi=72), b'\xff\x00\x00\xff')
            self.assertIsNotNone(expected)
            self.assertGreater(expected[1], 100)
            for actual, wanted in zip(bounds(pdf.render_page(0, dpi=72), b'\xff\x00\x00\xff'), expected):
                self.assertAlmostEqual(actual, wanted, delta=2)
        d.sections[0].vertical_alignment = 'top'
        d.sections.add()
        second = d.add_shape('rectangle', width=40, height=20, fill='blue')
        self.assertEqual([s.kind for s in d.shapes], ['ellipse', 'rectangle'])
        self.assertEqual(second.size, (40, 20))
