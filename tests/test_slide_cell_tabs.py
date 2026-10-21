"""Native tab stops through the existing owned PPTX cell paragraph format path."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

A = '{http://schemas.openxmlformats.org/drawingml/2006/main}'


def stops(data):
    with ZipFile(BytesIO(data)) as archive:
        paragraph = ET.fromstring(archive.read('ppt/slides/slide1.xml')).find(f'.//{A}tc/{A}txBody/{A}p')
        return paragraph.find(f'{A}pPr/{A}tabLst')


class SlideCellTabTests(unittest.TestCase):
    def test_native_alignments_source_pdf_labels_links_pixels_and_copy(self):
        for alignment in ('left', 'center', 'right', 'decimal'):
            with self.subTest(alignment=alignment), doc_ex.new('pptx', size=(576, 360)) as deck:
                slide = deck.slides.add()
                slide.notes = 'Retained notes'
                table = slide.add_table([['AA\t123.45', 'Neighbor'], ['Lower', 'Outside']],
                                        box=(24, 24, 528, 240), font_size=18)
                table.merge((0, 0), (1, 1))
                cell = table[1, 1]
                cell.text = 'AA\t123.45\nBB\t78.90\nCC\t987'
                cell.background = 'white'
                cell.margins = 0
                for paragraph, color in zip(cell.paragraphs, ('red', 'green', 'blue')):
                    paragraph[:].format(color=color, bold=True, underline='none')
                    paragraph.format(alignment='left', indent_left=0, indent_right=0, indent_first=0,
                                     space_before=0, space_after=0, line_spacing=1,
                                     tabs=[{'position': 140, 'alignment': alignment}])
                cell.paragraphs[0][:].link = 'https://example.com/tab'
                saved = deck.to_bytes()
                with deck.copy() as copy:
                    self.assertEqual(copy.to_bytes(), saved)
                pdf = deck.convert('pdf')
                with doc_ex.open(saved) as source, doc_ex.open(pdf) as converted:
                    self.assertIn('AA\t123.45', source[0].get_text())
                    for document in (source, converted):
                        for token in ('AA', '123.45', 'BB', '78.90', 'CC', '987'):
                            self.assertEqual(document[0].get_text().count(token), 1)
                        self.assertTrue(any(link.uri == 'https://example.com/tab' for link in document[0].get_links()))
                        for dpi in (72, 144, 200):
                            width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=dpi)))
                            self.assertEqual(width, 576 * dpi // 72)
                            self.assertEqual(len(pixels), width * (360 * dpi // 72) * 4)
                            for channel in range(3):
                                self.assertGreater(sum(pixels[i+channel] > pixels[i+(channel+1)%3]+40 and
                                                       pixels[i+channel] > pixels[i+(channel+2)%3]+40
                                                       for i in range(0, len(pixels), 4)), 100)
                    a = rgba_pixels(png_chunks(source.render_page(0, dpi=200)))
                    b = rgba_pixels(png_chunks(converted.render_page(0, dpi=200)))
                    self.assertEqual((a[0], len(a[1])), (b[0], len(b[1])))
                    self.assertTrue(all(abs(x-y) <= 1 for x,y in zip(a[1], b[1])))
                self.assertEqual(slide.notes, 'Retained notes')

    def test_none_empty_native_quantization_endpoints_snapshot_and_break(self):
        with doc_ex.new('pptx') as deck:
            table = deck.slides.add().add_table([['A\tB', 'Neighbor']], box=(20, 20, 600, 160))
            paragraph, neighbor = table[0, 0].paragraphs[0], table[0, 1].paragraphs[0]
            self.assertIsNone(paragraph.formatting['tabs'])
            live = paragraph[:]
            for position in (-2147483647, 0, 1/12700, 1.23456, 2147483647):
                paragraph.format(tabs=[{'position': position, 'alignment': 'decimal', 'leader': 'none'}])
                snapshot = paragraph.formatting
                self.assertEqual(snapshot['tabs'], [{'position': position, 'alignment': 'decimal', 'leader': 'none'}])
                saved = deck.to_bytes()
                self.assertEqual(stops(saved)[0].get('pos'), str(round(position * 12700)))
                paragraph.format(**snapshot)
                self.assertEqual(deck.to_bytes(), saved)
                self.assertEqual(neighbor.text, 'Neighbor')
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = live.text
            paragraph.format(tabs=[])
            self.assertEqual(paragraph.formatting['tabs'], [])
            self.assertEqual(len(stops(deck.to_bytes())), 0)
            paragraph.format(tabs=None)
            self.assertIsNone(paragraph.formatting['tabs'])
            self.assertIsNone(stops(deck.to_bytes()))
            paragraph.format(tabs=[{'position': 72}])
            inserted = paragraph.add_break()
            self.assertEqual(inserted.text, '\n')
            paragraph.add_run('C\tD')
            self.assertEqual(paragraph.text, 'A\tB\nC\tD')
            paragraph.find('\t')[0].text = ' '
            self.assertEqual(paragraph.text, 'A B\nC\tD')
            self.assertEqual(paragraph.formatting['tabs'][0]['position'], 72)

    def test_invalid_compounds_cardinality_order_and_noop_are_atomic(self):
        with doc_ex.new('pptx') as deck:
            paragraph = deck.slides.add().add_table([['Safe']], box=(20, 20, 400, 120))[0, 0].paragraphs[0]
            paragraph.format(tabs=[{'position': i} for i in range(32)])
            saved, preview = deck.to_bytes(), deck.render_page(0, dpi=72)
            revision, cache, live = deck._call('revision'), deck._cached, paragraph[:]
            invalid = [[{'position': i} for i in range(33)], [{'position': 2}, {'position': 1}],
                       [{'position': 1e-8}, {'position': 2e-8}], [{}], [{'position': 1, 'extra': 2}]]
            invalid += [[{'position': x}] for x in (2147483648, -2147483648, True, float('nan'), float('inf'), object())]
            for value in invalid:
                with self.subTest(value=value), self.assertRaises((TypeError, ValueError)):
                    paragraph.format(space_after=8, tabs=value)
                self.assertEqual(deck.to_bytes(), saved)
                self.assertEqual(deck._call('revision'), revision)
                self.assertIs(deck._cached, cache)
                self.assertEqual(live.text, 'Safe')
            for value in ({'position': 72, 'alignment': 'bar'}, {'position': 72, 'leader': 'dot'}):
                with self.assertRaises(UnsupportedFormatError):
                    paragraph.format(space_after=8, tabs=[value])
                self.assertEqual(deck.to_bytes(), saved)
                self.assertEqual(deck._call('revision'), revision)
            paragraph.format()
            self.assertEqual(deck.render_page(0, dpi=72), preview)
            self.assertIs(deck._cached, cache)

    def test_per_item_callbacks_stale_closed_and_builtin_container_access(self):
        class HostileList(list):
            def __iter__(self): raise AssertionError('custom iteration')
            def __len__(self): raise AssertionError('custom length')
            def __getitem__(self, key): raise AssertionError('custom indexing')
        class HostileDict(dict):
            def items(self): raise AssertionError('custom items')
            def __len__(self): raise AssertionError('custom length')
        class HostileString(str):
            def __str__(self): raise AssertionError('custom string')
            def __len__(self): raise AssertionError('custom length')
        with doc_ex.new('pptx') as deck:
            table = deck.slides.add().add_table([['Safe', 'Neighbor']], box=(20, 20, 600, 120))
            paragraph = table[0, 0].paragraphs[0]
            paragraph.format(tabs=HostileList([HostileDict(position=72, alignment=HostileString('left'))]))
            self.assertEqual(paragraph.formatting['tabs'][0]['position'], 72)
            calls = []
            class MutatingFloat(float):
                def __float__(self):
                    calls.append('first')
                    table[0, 1].text = 'Changed'
                    return 72.0
            class LaterFloat(float):
                def __float__(self):
                    calls.append('later')
                    return 144.0
            with self.assertRaisesRegex(ValueError, 'stale'):
                paragraph.format(tabs=[{'position': MutatingFloat(72)}, {'position': LaterFloat(144)}])
            self.assertEqual(calls, ['first'])
            self.assertEqual(paragraph.formatting['tabs'][0]['position'], 72)
            self.assertEqual(table[0, 1].text, 'Changed')
            stale = table[0, 0].paragraphs[0]
            table[0, 0].text = 'Replacement'
            with self.assertRaisesRegex(ValueError, 'stale'):
                stale.format(tabs=[{'position': LaterFloat(144)}])
            self.assertEqual(calls, ['first'])
        with self.assertRaises(DocumentClosedError):
            paragraph.format(tabs=[{'position': LaterFloat(144)}])
        self.assertEqual(calls, ['first'])
