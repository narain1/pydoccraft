from io import BytesIO
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex._authoring import TextRange
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_picture_media import media_parts, red_picture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


class WordBookmarkTests(unittest.TestCase):
    def test_mapping_unicode_move_point_delete_and_native_roundtrip(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Before ')
            paragraph.add_run('Target e\u0301 😀', bold=True, color='red')
            paragraph.add_run(' after')
            before_text = paragraph.text
            before_png = document.render_page(0, dpi=72)
            document.bookmarks['Target'] = paragraph.find('Target e\u0301 😀')[0]
            self.assertEqual(document.bookmarks['Target'].text, 'Target e\u0301 😀')
            self.assertEqual(list(document.bookmarks), ['Target'])
            self.assertEqual(len(document.bookmarks), 1)
            self.assertEqual(document.render_page(0, dpi=72), before_png)
            old = document.bookmarks['Target']
            document.bookmarks['Point'] = paragraph[0:0]
            self.assertEqual(document.bookmarks['Point'].text, '')
            self.assertEqual(list(document.bookmarks), ['Point', 'Target'])
            with self.assertRaises(ValueError):
                _ = old.text
            document.bookmarks['Target'] = paragraph.find('Before')[0]
            self.assertEqual(document.bookmarks['Target'].text, 'Before')
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = ET.fromstring(archive.read('word/document.xml'))
                starts = xml.findall('.//' + W + 'bookmarkStart')
                ends = xml.findall('.//' + W + 'bookmarkEnd')
                self.assertEqual(len(starts), 2)
                self.assertEqual({n.get(W + 'id') for n in starts}, {n.get(W + 'id') for n in ends})
                run = next(r for r in xml.iter(W + 'r') if 'Target' in ''.join(r.itertext()))
                self.assertIsNotNone(run.find(W + 'rPr/' + W + 'b'))
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    text = reopened.to_markdown()
                    self.assertNotIn('\ufffc', text)
                    self.assertEqual(text.count('Target'), 1)
            del document.bookmarks['Point']
            del document.bookmarks['Target']
            self.assertEqual(paragraph.text, before_text)
            self.assertEqual(document.render_page(0, dpi=72), before_png)
            with self.assertRaises(KeyError):
                _ = document.bookmarks['Target']

    def test_same_paragraph_relocation_and_fresh_self_assignment(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Before Middle After')
            document.bookmarks['Moving'] = paragraph.find('Middle')[0]
            for text in ('Before', 'After', 'Middle'):
                document.bookmarks['Moving'] = paragraph.find(text)[0]
                self.assertEqual(document.bookmarks['Moving'].text, text)
                document.bookmarks['Moving'] = document.bookmarks['Moving']
                self.assertEqual(document.bookmarks['Moving'].text, text)
            selected = document.bookmarks['Moving']
            document.bookmarks['Moving'] = paragraph[selected._start - 1:selected._end]
            self.assertEqual(document.bookmarks['Moving'].text, 'Middle')
            selected = document.bookmarks['Moving']
            document.bookmarks['Moving'] = paragraph[selected._start:selected._end + 1]
            self.assertEqual(document.bookmarks['Moving'].text, 'Middle')
            document.bookmarks['Moving'] = paragraph[:]
            self.assertEqual(document.bookmarks['Moving'].text, 'Before Middle After')
            document.bookmarks['Moving'] = paragraph.find('Middle')[0]
            self.assertEqual(document.bookmarks['Moving'].text, 'Middle')
            document.bookmarks['Other'] = paragraph.find('After')[0]
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                document.bookmarks['Moving'] = paragraph[:]
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(document.bookmarks['Moving'].text, 'Middle')
            fields = document.add_paragraph('Page ')
            fields.add_field('page')
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                document.bookmarks['Field'] = fields[:]
            self.assertEqual(document.to_bytes(), before)

    def test_internal_label_destination_repagination_and_references_across_stories(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            source = document.add_paragraph('Jump ')
            source.add_run('label', bold=True)
            target = document.add_paragraph('Before\n' * 40)
            target.add_run('Destination', color='red')
            document.bookmarks['Destination'] = target.find('Destination')[0]
            source[:].link = '#Destination'
            header = document.sections[0].header().add_paragraph('HeaderLink')
            footer = document.sections[0].footer().add_paragraph('FooterLink')
            anchor = document.add_paragraph('Note reference')
            note = document.notes.add(anchor[:], 'NoteLink').paragraphs[0]
            endnote = document.notes.add(anchor.find('reference')[0], 'EndnoteLink', kind='endnote').paragraphs[0]
            for paragraph in (header, footer, note, endnote):
                paragraph[:].link = '#Destination'
            with doc_ex.open(document.to_bytes()) as reopened:
                source_links = [link for page in reopened for link in page.get_links()]
                jump = [link for link in source_links if link.text == 'Jump label']
                self.assertEqual(''.join(link.text for link in jump), 'Jump label')
                self.assertTrue(all(link.uri == '#Destination' for link in jump))
                self.assertEqual([(link.metadata['start'], link.metadata['end']) for link in jump], [(0, 10)])
            destinations = []
            for margin in (30, 70):
                document.sections[0].margins = margin
                with doc_ex.open(document.convert('pdf')) as pdf:
                    destination = next(i for i, page in enumerate(pdf) if 'Destination' in page.get_text())
                    self.assertGreater(destination, 0)
                    destinations.append(destination)
                    links = [link for page in pdf for link in page.get_links()]
                    for text in ('Jump label', 'HeaderLink', 'FooterLink', 'NoteLink', 'EndnoteLink'):
                        matching = [link for link in links if link.text == text]
                        self.assertTrue(matching, text)
                        self.assertTrue(all(link.metadata['target_page'] == destination for link in matching))
                    self.assertTrue(pdf.render_page(destination, dpi=72).startswith(b'\x89PNG'))
            self.assertNotEqual(destinations[0], destinations[1])
            with document.copy() as copied:
                copied.bookmarks['Destination'].text = 'Copy destination'
                self.assertEqual(document.bookmarks['Destination'].text, 'Destination')
                with doc_ex.open(copied.convert('pdf')) as pdf:
                    self.assertIn('Copy destination', pdf.to_markdown())
                    self.assertTrue(any(page.get_links() for page in pdf))
            before = document.to_bytes()
            with self.assertRaises(ValueError):
                del document.bookmarks['Destination']
            self.assertEqual(document.to_bytes(), before)
            # Clear references in every story before deleting the target.
            for paragraph in (source, header, footer, note, endnote):
                paragraph[:].link = None
            del document.bookmarks['Destination']
            self.assertNotIn('\ufffc', target.text)

    def test_horizontal_nested_cell_link_box_and_grid_generation(self):
        with doc_ex.new('docx', size=(400, 500)) as document:
            document.sections[0].margins = 30
            outer = document.add_table([['']])
            nested = outer[0, 0].add_table([['CellLink'], ['CellTarget']])
            target = nested[1, 0].paragraphs[0]
            document.bookmarks['Cell'] = target[:]
            label = nested[0, 0].paragraphs[0]
            label[:].format(color='red')
            label[:].link = '#Cell'
            with doc_ex.open(document.convert('pdf')) as pdf:
                link = next(link for link in pdf[0].get_links() if link.text == 'CellLink')
                self.assertEqual(link.metadata['target_page'], 0)
                self.assertIsNotNone(link.bbox)
                width, pixels = rgba_pixels(png_chunks(pdf.render_page(0, dpi=144)))
                red = [(i // 4 % width / 2, i // 4 // width / 2) for i in range(0, len(pixels), 4)
                       if pixels[i] > 180 and pixels[i + 1] < 100 and pixels[i + 2] < 100]
                self.assertTrue(red)
                x0, y0, x1, y1 = link.bbox
                self.assertGreater(sum(x0 - 1 <= x <= x1 + 1 and y0 - 1 <= y <= y1 + 1 for x, y in red), len(red) * .95)
            old = document.bookmarks['Cell']
            nested.insert_row(0)
            with self.assertRaises((ValueError, IndexError)):
                _ = old.text
            self.assertEqual(document.bookmarks['Cell'].text, 'CellTarget')
            snapshot = document.to_bytes()
            with self.assertRaises(ValueError):
                nested.delete_row(2)
            self.assertEqual(document.to_bytes(), snapshot)
            nested[1, 0].paragraphs[0][:].link = None
            snapshot = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                nested.delete_row(2)
            self.assertEqual(document.to_bytes(), snapshot)
            with document.copy() as copied:
                selected = copied.bookmarks['Cell']
                selected.text = 'Changed target'
                self.assertEqual(document.bookmarks['Cell'].text, 'CellTarget')
                document.close()
                self.assertEqual(copied.bookmarks['Cell'].text, 'Changed target')
                self.assertTrue(copied.render_page(0, dpi=72).startswith(b'\x89PNG'))

    def test_media_fields_notes_and_bookmark_operations_preserve_references(self):
        png = red_picture(3, 2)
        with doc_ex.new('docx') as document:
            body = document.add_paragraph('Anchor and page ')
            body.add_field('page')
            document.background_image = png
            picture = document.add_image(png, width=20)
            note = document.notes.add(body.find('Anchor')[0], 'Note')
            note.paragraphs[0].add_field('page_count')
            document.sections[0].header().add_image(png, width=20)
            document.bookmarks['Anchor'] = body.find('Anchor')[0]
            nested = document.add_table([['']])[0, 0].add_table([['Target']])
            nested[0, 0].add_image(png, width=16)
            document.bookmarks['Nested'] = nested[0, 0].paragraphs[0].find('Target')[0]
            self.assertEqual(len(media_parts(document)), 1)
            before = document.render_page(0, dpi=72)
            picture.data = red_picture(4, 3)
            del document.images[0]
            document.background_image = None
            del document.bookmarks['Anchor']
            document.bookmarks['Nested'] = body.find('Anchor')[0]
            self.assertEqual(len(media_parts(document)), 1)
            self.assertNotEqual(document.render_page(0, dpi=72), before)
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertIn('Anchor', pdf.to_markdown())
                self.assertIn('Target', pdf.to_markdown())
                self.assertIn('Note', pdf.to_markdown())

    def test_bounds_scope_wrapped_links_rollback_and_existing_path_save(self):
        with doc_ex.new('docx') as document, doc_ex.new('docx') as other:
            paragraph = document.add_paragraph('Keep target')
            before = document.to_bytes()
            for name in ('', '1name', '_private', 'two names', 'x' * 41, '日本', '#name'):
                with self.subTest(name=name), self.assertRaises(ValueError):
                    document.bookmarks[name] = paragraph[:]
                self.assertEqual(document.to_bytes(), before)
            document.bookmarks['x' * 40] = paragraph[:]
            with self.assertRaises(ValueError):
                document.bookmarks['Foreign'] = other.add_paragraph('Foreign')[:]
            header = document.sections[0].header().add_paragraph('Header target')
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                document.bookmarks['Header'] = header[:]
            self.assertEqual(document.to_bytes(), before)
            with self.assertRaises(ValueError):
                paragraph.find('Keep')[0].link = '#Missing'
            table = document.add_table([['Visible\nClipped']])
            table.row(0).height = (20, 'exact')
            clipped = table[0, 0].paragraphs[0]
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                document.bookmarks['Clipped'] = clipped.find('Visible')[0]
            with self.assertRaises(UnsupportedFormatError):
                clipped.find('Visible')[0].link = '#' + 'x' * 40
            self.assertEqual(document.to_bytes(), before)
            table.row(0).height = None
            clipped.find('Visible')[0].link = '#' + 'x' * 40
            table[0, 0].format(text_direction='down')
            with TemporaryDirectory() as directory:
                path = Path(directory) / 'existing.docx'
                path.write_bytes(b'keep existing file')
                with self.assertRaises(UnsupportedFormatError):
                    document.save(path)
                self.assertEqual(path.read_bytes(), b'keep existing file')
            table[0, 0].format(text_direction='horizontal')
            self.assertTrue(document.to_bytes().startswith(b'PK'))
            table.row(0).height = (20, 'exact')
            with self.assertRaises(UnsupportedFormatError):
                document.convert('pdf')
            table.row(0).height = None
            self.assertTrue(document.to_bytes().startswith(b'PK'))
            foreign = other.add_paragraph('Other target')
            unsafe = other.add_table([['Vertical link']])[0, 0]
            unsafe.paragraphs[0][:].link = 'https://example.com'
            unsafe.format(text_direction='down')
            existing = other.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                other.bookmarks['Other'] = foreign[:]
            self.assertEqual(other.to_bytes(), existing)

    def test_callback_stale_and_closed_selections_do_not_apply_extra_edits(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Target')
            initial = paragraph[:]
            class ChangingSelection(TextRange):
                def __getattribute__(self, name):
                    if name == '_start':
                        paragraph.add_run(' callback')
                    return super().__getattribute__(name)
            changing = ChangingSelection(paragraph, initial._start, initial._end, initial._revision)
            with self.assertRaises(ValueError):
                document.bookmarks['Wrong'] = changing
            self.assertEqual(list(document.bookmarks), [])
            self.assertEqual(paragraph.text, 'Target callback')
            initial = paragraph[:]
            class ClosingSelection(TextRange):
                def __getattribute__(self, name):
                    if name == '_start':
                        document.close()
                    return super().__getattribute__(name)
            closing = ClosingSelection(paragraph, initial._start, initial._end, initial._revision)
            with self.assertRaises(DocumentClosedError):
                document.bookmarks['Closed'] = closing
            with self.assertRaises(DocumentClosedError):
                list(document.bookmarks)

    def test_repeated_long_target_lookup_work_rejects_atomically_and_recovers(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            target = document.add_paragraph('Before\n' * 700 + 'Target')
            document.bookmarks['Late'] = target.find('Target')[0]
            label = document.add_paragraph('Jump\n' * 30)
            selected = label[:]
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsafeArchiveError, 'lookup and geometry scans'):
                selected.link = '#Late'
            self.assertEqual(document.to_bytes(), before)
            self.assertIsNone(selected.link)
            label[:4].link = '#Late'
            self.assertTrue(document.to_bytes().startswith(b'PK'))
