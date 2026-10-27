"""PPTX cell breaks reuse Paragraph/TextRange with one-character public LF reads."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError

A = '{http://schemas.openxmlformats.org/drawingml/2006/main}'


def source_paragraphs(data):
    with ZipFile(BytesIO(data)) as archive:
        root = ET.fromstring(archive.read('ppt/slides/slide1.xml'))
        return root.find(f'.//{A}tc/{A}txBody').findall(f'{A}p')


class SlideCellBreakTests(unittest.TestCase):
    def test_unicode_empty_trailing_find_replace_and_structural_setter_contract(self):
        with doc_ex.new('pptx') as deck:
            cell = deck.slides.add().add_table([['A😀e\u0301Z']], box=(20, 20, 500, 200))[0, 0]
            p = cell.paragraphs[0]
            marker = p.add_break()
            self.assertEqual((marker._start, marker._end, marker.text), (5, 6, '\n'))
            marker.format(bold=True)
            self.assertEqual(marker.text, '\n')
            marker.link = 'https://example.com/explicit-break'
            self.assertEqual(marker.link, 'https://example.com/explicit-break')
            marker.link = None
            p.add_break()
            p.add_run('Tail')
            p.add_break()
            self.assertEqual(p.text, 'A😀e\u0301Z\n\nTail\n')
            self.assertEqual([(r._start, r._end, r.text) for r in p.find('\n')], [(5, 6, '\n'), (6, 7, '\n'), (11, 12, '\n')])
            self.assertEqual(p[1:4].text, '😀e\u0301')
            before, revision = deck.to_bytes(), deck._call('revision')
            for value in ('\n', '\v', p.text):
                with self.assertRaises(ValueError):
                    p.text = value
                with self.assertRaises(ValueError):
                    p[:].text = value
                with self.assertRaises(ValueError):
                    p.add_run(value)
                self.assertEqual(deck.to_bytes(), before)
                self.assertEqual(deck._call('revision'), revision)
            self.assertEqual(p.replace_text('\n', ' '), 3)
            self.assertEqual(p.text, 'A😀e\u0301Z  Tail ')
            self.assertEqual(p.find('\n'), [])
            p.text = ''
            self.assertEqual(p.add_break().text, '\n')
            self.assertEqual(p.add_break().text, '\n')
            self.assertEqual(p.text, '\n\n')
            p[:].text = ''
            self.assertEqual(p.text, '')
            self.assertEqual(len(cell.paragraphs), 1)
            p.add_break()
            cell.add_paragraph('Other')
            self.assertEqual(cell.text, '\n\nOther')
            cell.text = cell.text
            # Flattened Cell.text deliberately cannot distinguish break and paragraph LF.
            self.assertEqual(len(cell.paragraphs), 3)
            self.assertTrue(all(not paragraph.findall(f'{A}br') for paragraph in source_paragraphs(deck.to_bytes())))

    def test_merged_aliases_live_handles_copy_stale_grids_and_empty_targets(self):
        with doc_ex.new('pptx') as deck:
            slide = deck.slides.add()
            table = slide.add_table([['A', 'B'], ['C', 'Neighbor']], box=(20, 20, 500, 240))
            p, neighbor = table[0, 0].paragraphs[0], table[1, 1].paragraphs[0]
            stale = p[:]
            p.add_break()
            self.assertEqual(neighbor.text, 'Neighbor')
            self.assertEqual(p.text, 'A\n')
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = stale.text
            with deck.copy() as copy:
                copied = copy.slides[0].shapes[0][0, 0].paragraphs[0]
                copied.add_break()
                self.assertEqual(copied.text, 'A\n\n')
                self.assertEqual(p.text, 'A\n')
            table.merge((0, 0), (0, 1))
            with self.assertRaisesRegex(ValueError, 'stale'):
                p.add_break()
            anchor, alias = table[0, 0].paragraphs[0], table[0, 1].paragraphs[0]
            before = anchor.text
            self.assertEqual(alias.add_break().text, '\n')
            self.assertEqual(anchor.text, before + '\n')
            table[0, 0].split()
            self.assertEqual(len(table[0, 1].paragraphs), 0)
            with self.assertRaises(IndexError):
                table[0, 1].paragraphs[0].add_break()
            self.assertEqual(table[0, 1].add_paragraph().add_break().text, '\n')
            live = table[0, 0].paragraphs[0]
            deck.close()
            with self.assertRaises(DocumentClosedError):
                live.add_break()

    def test_invalid_kind_atomicity_scalar_hooks_and_unchanged_word_shape_input(self):
        calls = []
        class Hostile(str):
            def __str__(self):
                calls.append('str')
                raise AssertionError('must not convert subclass')
            def __eq__(self, other):
                calls.append('eq')
                raise AssertionError('must not compare subclass')
        with doc_ex.new('pptx') as deck:
            slide = deck.slides.add()
            table = slide.add_table([['Safe']], box=(20, 20, 400, 120))
            p = table[0, 0].paragraphs[0]
            before, preview = deck.to_bytes(), deck.render_page(0, dpi=72)
            cache, revision, live = deck._cached, deck._call('revision'), p[:]
            for value, error in [('page', UnsupportedFormatError), ('column', UnsupportedFormatError),
                                 ('unknown', ValueError), ('LINE', ValueError), ('', ValueError),
                                 (None, TypeError), (1, TypeError), ([], TypeError)]:
                with self.assertRaises(error):
                    p.add_break(value)
                self.assertEqual(deck.to_bytes(), before)
                self.assertEqual(deck.render_page(0, dpi=72), preview)
                self.assertIs(deck._cached, cache)
                self.assertEqual(deck._call('revision'), revision)
                self.assertEqual(live.text, 'Safe')
            self.assertEqual(p.add_break(Hostile('line')).text, '\n')
            self.assertEqual(calls, [])
            shape = slide.add_text('Shape', box=(20, 200, 300, 60))
            self.assertEqual(shape.text, 'Shape')
            with self.assertRaises(ValueError):
                shape[:].text = '\v'
            table[0, 0].text = 'Reset'
            with self.assertRaisesRegex(ValueError, 'stale'):
                p.add_break(Hostile('line'))
            self.assertEqual(calls, [])
        with doc_ex.new('docx') as word:
            p = word.add_paragraph('Word')
            self.assertEqual(p.add_break().text, '\n')
            p.add_run('Tail')
            self.assertEqual(p.text, 'Word\nTail')
