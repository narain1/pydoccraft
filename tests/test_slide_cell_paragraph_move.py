"""PPTX cell paragraphs use the existing collection's final-index movement."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_authoring import image_fixture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

A = '{http://schemas.openxmlformats.org/drawingml/2006/main}'


def cell_paragraph_xml(data):
    with ZipFile(BytesIO(data)) as archive:
        root = ET.fromstring(archive.read('ppt/slides/slide1.xml'))
        relationships = ET.fromstring(archive.read('ppt/slides/_rels/slide1.xml.rels'))
        targets = {r.get('Id'): (r.get('Type'), r.get('TargetMode'), r.get('Target')) for r in relationships}
        paragraphs = root.find(f'.//{A}tc/{A}txBody').findall(f'{A}p')
        # The native writer allocates relationship IDs in new source order.
        # Compare their exact semantic targets, not incidental rId ordinals.
        relation_id = '{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id'
        for paragraph in paragraphs:
            for node in paragraph.iter():
                if relation_id in node.attrib:
                    node.set(relation_id, repr(targets[node.get(relation_id)]))
        return [ET.tostring(p) for p in paragraphs]


class SlideCellParagraphMoveTests(unittest.TestCase):
    def assert_ink_order(self, data, order, dpi):
        width, pixels = rgba_pixels(png_chunks(data))
        self.assertEqual((width, len(pixels) // (width * 4)), (576 * dpi // 72, 360 * dpi // 72))
        sums, counts = [0, 0, 0], [0, 0, 0]
        for i in range(0, width * (270 * dpi // 72) * 4, 4):
            pixel = pixels[i:i + 4]
            for color in range(3):
                if pixel[color] > 80 and all(pixel[color] > pixel[other] + 60 for other in range(3) if other != color):
                    sums[color] += i // (width * 4)
                    counts[color] += 1
        self.assertTrue(all(count > 10 for count in counts), counts)
        means = [total / count for total, count in zip(sums, counts)]
        for first, second in zip(order, order[1:]):
            self.assertLess(means[first] + 3, means[second])
        color = b'\x20\x60\xc0\xff'
        count = sum(pixels[(y * width + x) * 4:(y * width + x) * 4 + 4] == color
                    for y in range(305 * dpi // 72, 315 * dpi // 72)
                    for x in range(505 * dpi // 72, 535 * dpi // 72))
        self.assertGreater(count, 200)

    def test_native_properties_links_media_and_actual_png_pdf_order(self):
        labels = ['First office', 'Middle café', 'Last tail']
        for source, destination in ((0, -1), (-1, 0)):
            with self.subTest(source=source), doc_ex.new('pptx', size=(576, 360)) as deck:
                slide, target = deck.slides.add(), deck.slides.add()
                slide.notes = 'Retained source notes'
                target.notes = 'Retained target notes'
                image = image_fixture()
                for item in (slide, target):
                    item.add_image(image, box=(500, 300, 40, 20))
                slide.background = '#f8f8f8'
                table = slide.add_table([[labels[0], 'Neighbor']], box=(20, 20, 536, 240), font_size=20)
                table[0, 0].background = table[0, 1].background = 'white'
                cell = table[0, 0]
                cell.add_paragraph('')
                for label in labels[1:]:
                    cell.add_paragraph(label)
                sequence = [labels[0], '', labels[1], labels[2]]
                for paragraph in cell.paragraphs:
                    if not paragraph.text:
                        continue
                    i = labels.index(paragraph.text)
                    paragraph.format(alignment=('left', 'center', 'right')[i])
                    paragraph[:].format(color=('red', 'green', 'blue')[i], bold=True)
                    paragraph[:].link = target if i == 2 else f'https://example.com/{i}'
                original = cell_paragraph_xml(deck.to_bytes())
                order = list(range(4))
                selected = order.pop(source)
                order.insert(destination % 4, selected)
                neighbor = table[0, 1].paragraphs[0]
                old_range = neighbor[:]
                old = list(cell.paragraphs)
                baseline = deck.render_page(0, dpi=72)
                with deck.copy() as copied:
                    cell.paragraphs.move(source, destination)
                    self.assertEqual(cell_paragraph_xml(deck.to_bytes()), [original[i] for i in order])
                    self.assertEqual([p.text for p in cell.paragraphs], [sequence[i] for i in order])
                    self.assertEqual(neighbor.text, 'Neighbor')
                    for paragraph in old:
                        with self.assertRaisesRegex(ValueError, 'stale'):
                            _ = paragraph.text
                    with self.assertRaisesRegex(ValueError, 'stale'):
                        _ = old_range.text
                    self.assertEqual(copied.render_page(0, dpi=72), baseline)
                    self.assertEqual(slide.notes, 'Retained source notes')
                    self.assertEqual(target.notes, 'Retained target notes')
                    with ZipFile(BytesIO(deck.to_bytes())) as archive:
                        self.assertEqual([archive.read(name) for name in archive.namelist() if name.startswith('ppt/media/')], [image])
                    for data in (deck.to_bytes(), deck.convert('pdf')):
                        with doc_ex.open(data) as reopened:
                            self.assertEqual(reopened.page_count, 2)
                            text = reopened[0].get_text()
                            visible = [labels.index(sequence[i]) for i in order if sequence[i]]
                            positions = [text.index(labels[i]) for i in visible]
                            self.assertEqual(positions, sorted(positions))
                            for label in labels + ['Neighbor']:
                                self.assertEqual(text.count(label), 1)
                            self.assertEqual(len(reopened[0].get_links()), 3)
                            for dpi in (72, 144):
                                self.assert_ink_order(reopened.render_page(0, dpi=dpi), visible, dpi)

    def test_merged_aliases_empty_moves_copy_and_held_iteration(self):
        for end in ((0, 1), (1, 0), (1, 1)):
            with self.subTest(end=end), doc_ex.new('pptx') as deck:
                table = deck.slides.add().add_table([['A', 'B'], ['C', 'D']], box=(20, 20, 500, 240))
                table.merge((0, 0), end)
                alias, anchor = table[end], table[0, 0]
                alias.text = 'First\n\nLast e\u0301 😀 سلام'
                held = anchor.paragraphs[0]
                iterator = iter(alias.paragraphs)
                next(iterator)
                with deck.copy() as copied:
                    alias.paragraphs.move(-1, 0)
                    self.assertEqual([p.text for p in anchor.paragraphs], ['Last e\u0301 😀 سلام', 'First', ''])
                    self.assertEqual(copied.slides[0].shapes[0][0, 0].text, 'First\n\nLast e\u0301 😀 سلام')
                    with self.assertRaisesRegex(ValueError, 'stale'):
                        _ = held.text
                    with self.assertRaisesRegex(ValueError, 'stale'):
                        _ = next(iterator).text
                    anchor.paragraphs.move(-1, 1)
                    self.assertEqual([p.text for p in alias.paragraphs], ['Last e\u0301 😀 سلام', '', 'First'])
                    alias.paragraphs.move(0, -1)
                    self.assertEqual([p.text for p in anchor.paragraphs], ['', 'First', 'Last e\u0301 😀 سلام'])
        with doc_ex.new('pptx') as deck:
            cell = deck.slides.add().add_table([['']], box=(10, 10, 200, 80))[0, 0]
            old = cell.paragraphs[0]
            cell.paragraphs.move(0, -1)
            self.assertEqual(old.text, '')
            del cell.paragraphs[0]
            before = deck.to_bytes()
            with self.assertRaises(IndexError):
                cell.paragraphs.move(0, 0)
            self.assertEqual(deck.to_bytes(), before)
            cell.add_paragraph('Fresh')
            cell.paragraphs.move(-1, 0)
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = old.text
            self.assertEqual(cell.text, 'Fresh')

    def test_noop_and_invalid_inputs_preserve_warm_cache_ranges_and_revision(self):
        with doc_ex.new('pptx') as deck:
            cell = deck.slides.add().add_table([['A\nB\nC']], box=(20, 20, 400, 160))[0, 0]
            before = deck.to_bytes()
            rendered = deck.render_page(0, dpi=72)
            cache = deck._cached
            revision = deck._call('revision')
            live = cell.paragraphs[1][:]
            cell.paragraphs.move(1, -2)
            for a, b, error in ((slice(None), 0, TypeError), (0, '1', TypeError), (0, 1.5, TypeError),
                                 (9, 0, IndexError), (0, -9, IndexError), (0, 10**100, IndexError)):
                with self.assertRaises(error):
                    cell.paragraphs.move(a, b)
                self.assertEqual(deck.to_bytes(), before)
                self.assertEqual(deck.render_page(0, dpi=72), rendered)
                self.assertIs(deck._cached, cache)
                self.assertEqual(deck._call('revision'), revision)
                self.assertEqual(live.text, 'B')
            cell.paragraphs.move(0, 2)
            self.assertIsNone(deck._cached)
            self.assertEqual(deck._call('revision'), revision + 1)
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = live.text

    def test_stale_grid_and_mutating_closing_callbacks_reject_before_move(self):
        for argument in ('source', 'destination'):
            for close in (False, True):
                with self.subTest(argument=argument, close=close), doc_ex.new('pptx') as deck:
                    table = deck.slides.add().add_table([['A\nB\nC']], box=(20, 20, 400, 160))
                    cell = table[0, 0]

                    class Change:
                        def __index__(self):
                            if close:
                                deck.close()
                            else:
                                cell.text = 'Callback retained'
                            return 0

                    args = (Change(), 1) if argument == 'source' else (0, Change())
                    with self.assertRaises(DocumentClosedError if close else ValueError):
                        cell.paragraphs.move(*args)
                    if not close:
                        self.assertEqual(cell.text, 'Callback retained')
        with doc_ex.new('pptx') as deck:
            table = deck.slides.add().add_table([['A\nB']], box=(20, 20, 400, 160))
            captured = table[0, 0].paragraphs
            table.insert_row(0)

            class MustNotRun:
                def __index__(self):
                    raise AssertionError('stale grid must reject before either callback')

            with self.assertRaisesRegex(ValueError, 'stale'):
                captured.move(MustNotRun(), MustNotRun())
            current = table[1, 0].paragraphs
            deck.close()
            with self.assertRaises(DocumentClosedError):
                current.move(MustNotRun(), MustNotRun())


if __name__ == '__main__':
    unittest.main()
