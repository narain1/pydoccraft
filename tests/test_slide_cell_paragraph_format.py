"""Direct PPTX cell paragraph spacing uses existing format/snapshot vocabulary."""
from io import BytesIO
import math
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

A = '{http://schemas.openxmlformats.org/drawingml/2006/main}'
KEYS = ('indent_left', 'indent_right', 'indent_first', 'space_before', 'space_after', 'line_spacing')


def properties(data, index=0):
    with ZipFile(BytesIO(data)) as archive:
        root = ET.fromstring(archive.read('ppt/slides/slide1.xml'))
        return root.find(f'.//{A}tc/{A}txBody').findall(f'{A}p')[index].find(f'{A}pPr')


class SlideCellParagraphFormatTests(unittest.TestCase):
    def test_fitting_wrap_spacing_links_copy_and_actual_source_pdf_output(self):
        for end in (None, (0, 1), (1, 0), (1, 1)):
            with self.subTest(merge=end), doc_ex.new('pptx', size=(576, 576)) as deck:
                slide = deck.slides.add()
                slide.notes = 'Retained notes'
                table = slide.add_table([['First office', 'Neighbor'], ['Lower', 'Outside']],
                                        box=(24, 24, 528, 480), font_size=18)
                if end:
                    table.merge((0, 0), end)
                cell = table[end] if end else table[0, 0]
                cell.background = 'white'
                cell.margins = 0
                cell.text = 'First office\n' + 'M' * 48 + '\nLast tail'
                for p, color in zip(cell.paragraphs, ('red', 'green', 'blue')):
                    p[:].format(color=color, bold=True, underline='none')
                    p.format(alignment='left', indent_left=0, indent_right=0, indent_first=0,
                             space_before=0, space_after=0, line_spacing=1)
                cell.paragraphs[0][:].link = 'https://example.com/retained'
                paragraph = cell.paragraphs[1]
                held_first = cell.paragraphs[0]
                neighbor = table[0, 1].paragraphs[0] if end is None else None
                live = paragraph[:]
                original = deck.to_bytes()
                baseline = deck.render_page(0, dpi=72)
                with deck.copy() as copy:
                    paragraph.format(indent_left=24, indent_right=12, indent_first=-12,
                                     space_before=10, space_after=9, line_spacing=1.5)
                    self.assertEqual(paragraph.text, 'M' * 48)
                    self.assertEqual(held_first.text, 'First office')
                    if neighbor is not None:
                        self.assertEqual(neighbor.text, 'Neighbor')
                    self.assertEqual(table[0, 0].paragraphs[1].formatting, paragraph.formatting)
                    with self.assertRaisesRegex(ValueError, 'stale'):
                        _ = live.text
                    self.assertEqual(copy.to_bytes(), original)
                    self.assertEqual(copy.render_page(0, dpi=72), baseline)
                    self.assertNotEqual(deck.render_page(0, dpi=72), baseline)
                saved = deck.to_bytes()
                p = properties(saved, 1)
                self.assertEqual((p.get('marL'), p.get('marR'), p.get('indent')), ('304800', '152400', '-152400'))
                for name, value in (('spcBef', '1000'), ('spcAft', '900')):
                    self.assertEqual(p.find(f'{A}{name}/{A}spcPts').get('val'), value)
                self.assertEqual(p.find(f'{A}lnSpc/{A}spcPct').get('val'), '150000')
                pdf = deck.convert('pdf')
                with doc_ex.open(saved) as source, doc_ex.open(pdf) as converted:
                    for document in (source, converted):
                        self.assertEqual(document.page_count, 1)
                        text = document[0].get_text()
                        self.assertEqual(text.count('M'), 48)
                        self.assertEqual(text.count('First office'), 1)
                        self.assertEqual(text.count('Last tail'), 1)
                        self.assertTrue(any(link.uri == 'https://example.com/retained'
                                            for link in document[0].get_links()))
                        for dpi in (72, 144):
                            width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=dpi)))
                            self.assertEqual((width, len(pixels)), (576 * dpi // 72, (576 * dpi // 72) ** 2 * 4))
                            self.assertGreater(sum(pixels[i + 1] > pixels[i] + 40 and pixels[i + 1] > pixels[i + 2] + 40
                                                   for i in range(0, len(pixels), 4)), 100)
                    a = rgba_pixels(png_chunks(source.render_page(0, dpi=200)))
                    b = rgba_pixels(png_chunks(converted.render_page(0, dpi=200)))
                    self.assertEqual((a[0], len(a[1])), (b[0], len(b[1])))
                    self.assertTrue(all(abs(x - y) <= 1 for x, y in zip(a[1], b[1])))
                self.assertEqual(slide.notes, 'Retained notes')

    def test_direct_none_zero_quantization_endpoints_and_snapshot_replay(self):
        with doc_ex.new('pptx') as deck:
            paragraph = deck.slides.add().add_table([['Safe']], box=(20, 20, 400, 120))[0, 0].paragraphs[0]
            original = paragraph.formatting
            self.assertEqual({key: original[key] for key in KEYS}, dict.fromkeys(KEYS))
            paragraph.format(indent_left=1.23456, indent_right=2.34567, indent_first=-3.45678,
                             space_before=4.56789, space_after=5.67891, line_spacing=1.23456789)
            direct = paragraph.formatting
            self.assertEqual(direct['space_after'], 5.67891)
            before = deck.to_bytes()
            paragraph.format(**direct)
            self.assertEqual(deck.to_bytes(), before)
            p = properties(before)
            self.assertEqual((p.get('marL'), p.get('marR'), p.get('indent')), ('15679', '29790', '-43901'))
            self.assertEqual(p.find(f'{A}spcBef/{A}spcPts').get('val'), '457')
            self.assertEqual(p.find(f'{A}spcAft/{A}spcPts').get('val'), '568')
            self.assertEqual(p.find(f'{A}lnSpc/{A}spcPct').get('val'), '123457')
            paragraph.format(indent_left=0, indent_right=0, indent_first=0, space_before=0, space_after=0, line_spacing=1)
            self.assertEqual([paragraph.formatting[k] for k in KEYS], [0, 0, 0, 0, 0, 1])
            # Endpoints prove storage, not that extreme offsets fit a finite cell.
            paragraph.format(indent_left=4032, indent_right=4032, indent_first=-4032,
                             space_before=1584, space_after=1584, line_spacing=100)
            self.assertEqual([paragraph.formatting[k] for k in KEYS], [4032, 4032, -4032, 1584, 1584, 100])
            p = properties(deck.to_bytes())
            self.assertEqual(p.get('marL'), str(4032 * 12700))
            self.assertEqual(p.find(f'{A}lnSpc/{A}spcPct').get('val'), '10000000')
            paragraph.format(indent_first=4032)
            self.assertEqual(paragraph.formatting['indent_first'], 4032)
            paragraph.format(**dict.fromkeys(KEYS))
            self.assertEqual(paragraph.formatting, original)
            p = properties(deck.to_bytes())
            self.assertIsNone(p.get('marL'))
            self.assertIsNone(p.find(f'{A}lnSpc'))

    def test_invalid_compounds_noop_cache_and_existing_other_paragraph_contracts(self):
        with doc_ex.new('pptx') as deck:
            slide = deck.slides.add()
            paragraph = slide.add_table([['Safe']], box=(20, 20, 400, 120))[0, 0].paragraphs[0]
            before, preview = deck.to_bytes(), deck.render_page(0, dpi=72)
            cache, revision, live = deck._cached, deck._call('revision'), paragraph[:]
            invalid = [('line_spacing', value) for value in (0, -1, .06, math.nextafter(1, 0), 100.00001, True, '1.5', [], float('nan'), float('inf'))]
            invalid += [('indent_left', -.00001), ('indent_right', 4032.00001), ('indent_first', -4032.00001),
                        ('indent_first', 4032.00001), ('space_before', -.01), ('space_after', 1584.01)]
            for key, value in invalid:
                with self.subTest(key=key, value=value), self.assertRaises((ValueError, TypeError)):
                    paragraph.format(alignment='center', **{key: value})
                self.assertEqual(deck.to_bytes(), before)
                self.assertEqual(deck._call('revision'), revision)
                self.assertIs(deck._cached, cache)
                self.assertEqual(live.text, 'Safe')
            for patch in ({'keep_with_next': True}, {'tabs': []}, {'direction': 'rtl'}):
                with self.assertRaises(UnsupportedFormatError):
                    paragraph.format(space_after=8, **patch)
            paragraph.format()
            self.assertEqual(deck.to_bytes(), before)
            self.assertEqual(deck.render_page(0, dpi=72), preview)
            self.assertIs(deck._cached, cache)
            self.assertEqual(deck._call('revision'), revision)
            self.assertEqual(live.text, 'Safe')
            with self.assertRaises(UnsupportedFormatError):
                _ = paragraph.spacing
            # Shape-wide text formatting keeps its existing, separate vocabulary.
            shape = slide.add_text('Other', box=(20, 200, 300, 80))
            shape[:].format(font_size=20)
            self.assertEqual(shape.text, 'Other')
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Word')
            paragraph.format(space_before=4, space_after=8, line_spacing=1.5)
            self.assertEqual(paragraph.formatting['space_after'], 8)

    def test_callback_edits_closure_stale_targets_and_live_paragraph_handles(self):
        calls = []
        class Callback(str):
            def __new__(cls, callback):
                result = super().__new__(cls, 'center')
                result.callback = callback
                return result
            def __len__(self):
                calls.append('callback')
                self.callback()
                return super().__len__()
        for action in ('append', 'reset', 'grid', 'move', 'close'):
            with self.subTest(action=action), doc_ex.new('pptx') as deck:
                slide = deck.slides.add()
                deck.slides.add()
                table = slide.add_table([['Safe']], box=(20, 20, 400, 120))
                cell, paragraph = table[0, 0], table[0, 0].paragraphs[0]
                callbacks = {'append': lambda: cell.add_paragraph('Callback'),
                             'reset': lambda: setattr(cell, 'text', 'Callback'),
                             'grid': lambda: table.insert_row(1),
                             'move': lambda: deck.slides.move(0, 1), 'close': deck.close}
                calls.clear()
                with self.assertRaises(DocumentClosedError if action == 'close' else ValueError):
                    paragraph.format(alignment=Callback(callbacks[action]),
                                     space_after=Callback(lambda: self.fail('second callback must not run')))
                self.assertEqual(calls, ['callback'])
                if action != 'close':
                    self.assertIsNone(table[0, 0].paragraphs[0].formatting['space_after'])
        with doc_ex.new('pptx') as deck:
            table = deck.slides.add().add_table([['Safe']], box=(20, 20, 400, 120))
            paragraph = table[0, 0].paragraphs[0]
            live = paragraph[:]
            paragraph.format(space_after=8)
            self.assertEqual(paragraph.text, 'Safe')
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = live.text
            table.insert_row(1)
            calls.clear()
            with self.assertRaisesRegex(ValueError, 'stale'):
                paragraph.format(alignment=Callback(lambda: self.fail('stale must reject before callback')))
            self.assertEqual(calls, [])
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = paragraph.formatting
