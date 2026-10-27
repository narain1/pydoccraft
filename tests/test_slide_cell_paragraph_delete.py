"""Owned PPTX paragraph deletion through the existing cell collection."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_authoring import image_fixture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"


class SlideCellParagraphDeleteTests(unittest.TestCase):
    def test_first_middle_last_preserve_format_links_and_native_pdf_output(self):
        labels = ('First office', 'Middle café', 'Last tail')
        colors = ('red', 'green', 'blue')
        alignments = ('left', 'center', 'right')
        for index in (0, 1, -1):
            with self.subTest(index=index), doc_ex.new('pptx', size=(576, 360)) as deck:
                slide = deck.slides.add()
                destination = deck.slides.add()
                destination.add_text('Destination', box=(20, 20, 400, 40))
                image = image_fixture()
                slide.add_image(image, box=(450, 290, 40, 20))
                destination.add_image(image, box=(450, 290, 40, 20))
                table = slide.add_table([[labels[0], 'Neighbor']], box=(20, 20, 536, 240), font_size=20)
                cell = table[0, 0]
                cell.add_paragraph(labels[1])
                cell.add_paragraph(labels[2])
                for i, paragraph in enumerate(cell.paragraphs):
                    paragraph.format(alignment=alignments[i])
                    paragraph[:].format(color=colors[i], bold=True)
                    paragraph[:].link = destination if i == 2 else f'https://example.com/{i}'
                neighbor = table[0, 1].paragraphs[0]
                stale = cell.paragraphs[0]
                old_range = neighbor[:]
                del cell.paragraphs[index]
                expected = list(labels)
                removed = expected.pop(index)
                self.assertEqual([p.text for p in cell.paragraphs], expected)
                self.assertEqual(neighbor.text, 'Neighbor')
                with self.assertRaisesRegex(ValueError, 'stale'):
                    _ = stale.text
                with self.assertRaisesRegex(ValueError, 'stale'):
                    _ = old_range.text
                source = deck.to_bytes()
                self.assertEqual(source, deck.to_bytes())
                with ZipFile(BytesIO(source)) as archive:
                    self.assertEqual([archive.read(name) for name in archive.namelist()
                                      if name.startswith('ppt/media/')], [image])
                    root = ET.fromstring(archive.read('ppt/slides/slide1.xml'))
                    paragraphs = root.find(f'.//{A}tc/{A}txBody').findall(f'{A}p')
                    expected_alignments = ['l', 'ctr', 'r']
                    expected_alignments.pop(index)
                    self.assertEqual([p.find(f'{A}pPr').get('algn') for p in paragraphs], expected_alignments)
                for kind, data in (('pptx', source), ('pdf', deck.convert('pdf'))):
                    with self.subTest(kind=kind), doc_ex.open(data) as reopened:
                        self.assertEqual(reopened.page_count, 2)
                        text = reopened[0].get_text()
                        self.assertNotIn(removed, text)
                        for label in expected + ['Neighbor']:
                            self.assertEqual(text.count(label), 1)
                        self.assertEqual(len(reopened[0].get_links()), 2)
                        for dpi in (72, 144):
                            width, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=dpi)))
                            self.assertEqual((width, len(pixels) // (width * 4)),
                                             (576 * dpi // 72, 360 * dpi // 72))
                            self.assertGreater(sum(pixels[i] < 128 for i in range(0, len(pixels), 4)), 20)
                            self.assertGreater(sum(pixels[i:i + 4] == b'\x20\x60\xc0\xff'
                                                   for i in range(0, len(pixels), 4)), 10)

    def test_merged_alias_generations_empty_append_and_copy_independence(self):
        for end in ((0, 1), (1, 0), (1, 1)):
            with self.subTest(end=end), doc_ex.new('pptx') as deck:
                slide = deck.slides.add()
                deck.slides.add()
                table = slide.add_table([['A', 'B'], ['C', 'D']], box=(20, 20, 500, 240))
                table.merge((0, 0), end)
                alias = table[end]
                anchor = table[0, 0]
                alias.text = 'First\nSecond\nThird'
                held = anchor.paragraphs[2]
                grid_cell = table[0, 0]
                with deck.copy() as copied:
                    del alias.paragraphs[1]
                    self.assertEqual(anchor.text, 'First\nThird')
                    self.assertEqual(grid_cell.text, anchor.text)
                    self.assertEqual(copied.slides[0].shapes[0][0, 0].text, 'First\nSecond\nThird')
                    with self.assertRaisesRegex(ValueError, 'stale'):
                        _ = held.text
                    deck.slides.move(0, 1)
                    self.assertEqual(slide.number, 1)
                    while alias.paragraphs:
                        del alias.paragraphs[-1]
                    self.assertEqual(anchor.text, '')
                    self.assertEqual(len(anchor.paragraphs), 0)
                    self.assertEqual(alias.add_paragraph('Fresh').text, 'Fresh')
                    self.assertEqual([p.text for p in anchor.paragraphs], ['Fresh'])
                    with self.assertRaisesRegex(ValueError, 'stale'):
                        _ = held.text

    def test_invalid_selection_and_callbacks_do_not_delete_another_paragraph(self):
        with doc_ex.new('pptx') as deck:
            table = deck.slides.add().add_table([['A\nB\nC']], box=(20, 20, 400, 160))
            cell = table[0, 0]
            before = deck.to_bytes()
            preview = deck.render_page(0, dpi=72)
            live = cell.paragraphs[1][:]
            for index, error in ((slice(None), TypeError), ('0', TypeError), (9, IndexError), (-9, IndexError)):
                with self.assertRaises(error):
                    del cell.paragraphs[index]
                self.assertEqual(deck.to_bytes(), before)
                self.assertEqual(deck.render_page(0, dpi=72), preview)
                self.assertEqual(live.text, 'B')

            captured = cell.paragraphs
            table.insert_row(0)

            class MustNotRun:
                def __index__(self):
                    raise AssertionError('stale cell must reject before caller code')

            with self.assertRaisesRegex(ValueError, 'stale'):
                del captured[MustNotRun()]
            cell = table[1, 0]

            class MutatingIndex:
                def __index__(self):
                    cell.text = 'Callback edit'
                    return 0

            with self.assertRaisesRegex(ValueError, 'stale'):
                del cell.paragraphs[MutatingIndex()]
            self.assertEqual(cell.text, 'Callback edit')

            class ClosingIndex:
                def __index__(self):
                    deck.close()
                    return 0

            with self.assertRaises(DocumentClosedError):
                del cell.paragraphs[ClosingIndex()]
            with self.assertRaises(DocumentClosedError):
                del cell.paragraphs['bad']

    def test_final_authored_empty_has_native_required_blank_and_no_handle_revival(self):
        with doc_ex.new('pptx', size=(576, 360)) as deck:
            cell = deck.slides.add().add_table([['']], box=(20, 20, 500, 140))[0, 0]
            cell.background = 'yellow'
            old = cell.paragraphs[0]
            before = deck.render_page(0, dpi=144)
            del cell.paragraphs[0]
            self.assertEqual(len(cell.paragraphs), 0)
            self.assertEqual(deck.render_page(0, dpi=144), before)
            with self.assertRaises(IndexError):
                del cell.paragraphs[0]
            source = deck.to_bytes()
            with ZipFile(BytesIO(source)) as archive:
                root = ET.fromstring(archive.read('ppt/slides/slide1.xml'))
                self.assertEqual(len(root.find(f'.//{A}tc/{A}txBody').findall(f'{A}p')), 1)
            for data in (source, deck.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    self.assertEqual(reopened.page_count, 1)
                    self.assertEqual(reopened[0].get_text().strip(), '')
                    reopened.render_page(0, dpi=144)
            with deck.copy() as copied:
                copied.slides[0].shapes[0][0, 0].add_paragraph('Independent')
                self.assertEqual(len(cell.paragraphs), 0)
            self.assertEqual(cell.add_paragraph('New').text, 'New')
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = old.text


if __name__ == '__main__':
    unittest.main()
