from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import count_color, red_picture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def red_rows(png):
    width, pixels = rgba_pixels(png_chunks(png))
    return {offset // (width * 4) for offset in range(0, len(pixels), 4)
            if pixels[offset:offset + 4] == b'\xff\0\0\xff'}


def pdf_text_pages(document):
    with doc_ex.open(document.convert('pdf')) as pdf:
        return [' '.join(page.get_text().split()) for page in pdf]


class NoteConversionTests(unittest.TestCase):
    def test_conversion_moves_numbered_visible_notes_and_preserves_cache_on_noop(self):
        with doc_ex.new('docx', size=(300, 360)) as document:
            paragraph = document.add_paragraph('Citation')
            untouched = paragraph[:]
            self.assertEqual(document.notes.convert(kind='endnote'), 0)
            self.assertEqual(untouched.text, 'Citation')
            section = document.sections[0]
            section.margins = 24
            section.configure_notes(kind='footnote', start=3, numbering='decimal')
            section.configure_notes(kind='endnote', start=7, numbering='decimal')
            note = document.notes.add(paragraph[:], 'DetailFirst')
            note_paragraph = note.paragraphs[0]
            note_paragraph[:].format(color='red')
            document.add_page_break()
            document.add_paragraph('FinalBody')
            original = document.render_page(0, dpi=144)
            rows = red_rows(original)
            self.assertTrue(rows)
            self.assertGreater(min(rows), 500)
            self.assertRegex(pdf_text_pages(document)[0], r'3\s*DetailFirst')
            cache = document._cached
            current = note_paragraph[:]
            raw = document.to_bytes()
            self.assertEqual(document.notes.convert(kind='footnote'), 0)
            self.assertIs(document._cached, cache)
            self.assertEqual(document.to_bytes(), raw)
            self.assertEqual(current.text, 'DetailFirst')
            with document.copy() as copied:
                self.assertEqual(document.notes.convert(kind='endnote'), 1)
                self.assertIsNone(document._cached)
                self.assertEqual(note.kind, 'endnote')
                self.assertEqual(note_paragraph.text, 'DetailFirst')
                with self.assertRaises(ValueError):
                    _ = current.text
                self.assertEqual(copied.notes[0].kind, 'footnote')
                self.assertEqual(copied.render_page(0, dpi=144), original)
                self.assertEqual(len(document.notes), 1)
                self.assertRegex(pdf_text_pages(document)[1], r'7\s*DetailFirst')
                self.assertEqual(red_rows(document.render_page(0, dpi=144)), set())
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        rows = red_rows(reopened.render_page(1, dpi=144))
                        self.assertTrue(rows)
                        self.assertLess(max(rows), 200)
                        self.assertEqual(red_rows(reopened.render_page(0, dpi=144)), set())
                self.assertEqual(document.notes.convert(kind='endnote'), 0)
                self.assertEqual(document.notes.convert(kind='footnote'), 1)
                self.assertEqual(note.kind, 'footnote')
                self.assertEqual(document.render_page(0, dpi=144), original)

    def test_nested_references_styles_media_links_and_copy_keep_their_handles(self):
        with doc_ex.new('docx') as document:
            body = document.add_paragraph('Before citation after.')
            first = document.notes.add(body.find('citation')[0], 'Custom detail')
            first_paragraph = first.paragraphs[0]
            special = document.styles.add('Special Note', based_on='Footnote Text')
            special.format(italic=True)
            first_paragraph.style = special
            first_paragraph.find('Custom')[0].format(bold=True)
            outer = document.add_table([['Outer']])
            nested = outer[0, 0].add_table([['Nested citation']])
            anchor = nested[0, 0].paragraphs[0]
            second = document.notes.add(anchor.find('citation')[0], 'Standard detail')
            second_paragraph = second.paragraphs[0]
            existing_end = document.notes.add(body.find('after')[0], 'Existing end', kind='endnote')
            destination = document.add_paragraph('Destination')
            document.bookmarks['dest'] = destination[:]
            second_paragraph.find('Standard')[0].link = '#dest'
            first_paragraph.find('detail')[0].link = 'https://example.test/note'
            picture = document.add_image(red_picture(2, 2), width=20)
            document.background_image = picture.data
            original_body = body.text
            with document.copy() as copied:
                self.assertEqual(document.notes.convert(kind='endnote'), 2)
                self.assertEqual([note.kind for note in document.notes], ['endnote'] * 3)
                self.assertEqual(first_paragraph.style, 'Special Note')
                self.assertEqual(second_paragraph.style, 'Endnote Text')
                self.assertEqual(body.text, original_body)
                self.assertEqual(anchor.text, 'Nested citation\ufffc')
                self.assertEqual(picture.data, document.background_image)
                self.assertEqual(existing_end.paragraphs[0].text, 'Existing end')
                second.add_paragraph('Later addition')
                self.assertEqual(second.paragraphs[-1].style, 'Endnote Text')
                self.assertEqual(copied.notes[1].kind, 'footnote')
                self.assertEqual(document.notes.convert(kind='footnote'), 3)
                self.assertEqual(first_paragraph.style, 'Special Note')
                self.assertEqual(second_paragraph.style, 'Footnote Text')
            data = document.to_bytes()
            with ZipFile(BytesIO(data)) as archive:
                source = ET.fromstring(archive.read('word/document.xml'))
                self.assertEqual(len(source.findall('.//' + W + 'footnoteReference')), 3)
                self.assertEqual(len(source.findall('.//' + W + 'endnoteReference')), 0)
                footnotes = ET.fromstring(archive.read('word/footnotes.xml'))
                self.assertTrue(footnotes.findall('.//' + W + 'b'))
            with doc_ex.open(data) as reopened:
                stories = '\n'.join(story.get_text() for story in reopened.stories)
                for label in ('Custom detail', 'Standard detail', 'Existing end', 'Later addition'):
                    self.assertEqual(stories.count(label), 1)
                    self.assertNotIn(label, reopened.to_markdown())
            with doc_ex.open(document.convert('pdf')) as pdf:
                text = '\n'.join(page.get_text() for page in pdf)
                for label in ('Custom detail', 'Standard detail', 'Existing end', 'Later addition'):
                    self.assertEqual(text.count(label), 1)
                links = [link for page in pdf for link in page.get_links()]
                self.assertTrue(any(link.uri == 'https://example.test/note' for link in links))

    def test_long_notes_keep_every_paragraph_and_visible_continuation(self):
        with doc_ex.new('docx', size=(300, 360)) as document:
            document.sections[0].margins = 24
            anchor = document.add_paragraph('Citation')
            note = document.notes.add(anchor[:], 'DetailFirst')
            note.paragraphs[0][:].format(color='red')
            document.add_page_break()
            document.add_paragraph('FinalBody')
            for index in range(45):
                note.add_paragraph(f'Continuation{index:02} complete text.')[:].format(color='red')
            for kind in ('endnote', 'footnote'):
                self.assertEqual(document.notes.convert(kind=kind), 1)
                pdf_bytes = document.convert('pdf')
                with doc_ex.open(pdf_bytes) as pdf:
                    text = '\n'.join(page.get_text() for page in pdf)
                    ink_pages = [page.number for page in pdf
                                 if count_color(page.get_pixmap(dpi=144), b'\xff\0\0\xff') > 10]
                    self.assertGreater(len(ink_pages), 1)
                    self.assertEqual(ink_pages[0], 1 if kind == 'endnote' else 0)
                    for index in range(45):
                        self.assertEqual(text.count(f'Continuation{index:02}'), 1)
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual([page.number for page in reopened
                                      if count_color(page.get_pixmap(dpi=144), b'\xff\0\0\xff') > 10], ink_pages)
                    text = '\n'.join(story.get_text() for story in reopened.stories)
                    for index in range(45):
                        self.assertEqual(text.count(f'Continuation{index:02}'), 1)

    def test_unsafe_field_destination_rolls_back_all_notes_and_can_recover(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Anchor')
            first = document.notes.add(paragraph[:], 'Count=')
            first.paragraphs[0].add_field('section_page_count')
            second = document.notes.add(paragraph.find('Anchor')[0], 'Other detail')
            continuous = document.sections.add(start='continuous')
            document.add_paragraph('Continuous destination')
            continuous.configure_notes(kind='endnote', placement='section_end')
            original = document.render_page(0, dpi=144)
            before = document.to_bytes()
            current = second.paragraphs[0][:]
            cache = document._cached
            with self.assertRaises(UnsupportedFormatError):
                document.notes.convert(kind='endnote')
            self.assertEqual(document.to_bytes(), before)
            self.assertIs(document._cached, cache)
            self.assertEqual(document.render_page(0, dpi=144), original)
            self.assertEqual(current.text, 'Other detail')
            self.assertEqual([note.kind for note in document.notes], ['footnote', 'footnote'])
            continuous.start = 'next_page'
            self.assertEqual(document.notes.convert(kind='endnote'), 2)
            self.assertEqual(first.kind, 'endnote')
            self.assertEqual(second.kind, 'endnote')
            self.assertIn('Count=', '\n'.join(pdf_text_pages(document)))

    def test_shared_standard_style_runs_are_guarded_with_custom_style_recovery(self):
        for contents in ('text', 'mixed_notes', 'field', 'all_converted'):
            with self.subTest(contents=contents), doc_ex.new('docx') as document:
                paragraph = document.add_paragraph('Ordinary' if contents == 'text' else '')
                foot = document.notes.add(paragraph[:], 'Foot detail')
                if contents in ('mixed_notes', 'all_converted'):
                    second = document.notes.add(paragraph[0:0], 'Other detail',
                                                kind='endnote' if contents == 'mixed_notes' else 'footnote')
                elif contents == 'field':
                    paragraph.add_field('page')
                document.styles['Footnote Reference'].format(color='red', font_size=16)
                document.styles['Endnote Reference'].format(color='blue', font_size=16)
                paragraph[:].format(style=document.styles['Footnote Reference'].id)
                expected_text = paragraph.text
                if contents == 'all_converted':
                    self.assertEqual(document.notes.convert(kind='endnote'), 2)
                    self.assertEqual((foot.kind, second.kind), ('endnote', 'endnote'))
                    continue
                before = document.to_bytes()
                render = document.render_page(0, dpi=144)
                selection = paragraph[:]
                cache = document._cached
                with self.assertRaises(UnsupportedFormatError):
                    document.notes.convert(kind='endnote')
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cache)
                self.assertEqual(selection.text, expected_text)
                self.assertEqual(document.render_page(0, dpi=144), render)
                custom = document.styles.add('Keep reference appearance', kind='character',
                                             based_on='Footnote Reference')
                paragraph[:].format(style=custom.id)
                self.assertEqual(document.notes.convert(kind='endnote'), 1)
                self.assertEqual(paragraph.text, expected_text)
                self.assertEqual(foot.kind, 'endnote')
                self.assertGreater(count_color(document.render_page(0, dpi=144), b'\xff\0\0\xff'), 0)
                self.assertEqual(count_color(document.render_page(0, dpi=144), b'\0\0\xff\xff'), 0)
                with ZipFile(BytesIO(document.to_bytes())) as archive:
                    source = ET.fromstring(archive.read('word/document.xml'))
                    self.assertTrue(any(item.get(W + 'val') == custom.id
                                        for item in source.findall('.//' + W + 'rStyle')))
                    self.assertEqual(len(source.findall('.//' + W + 'endnoteReference')),
                                     2 if contents == 'mixed_notes' else 1)
                    if contents == 'field':
                        self.assertIn(' PAGE ', archive.read('word/document.xml').decode())

    def test_kind_validation_is_bounded_callback_free_and_closed_first(self):
        class Hostile(str):
            def __str__(self):
                raise AssertionError('user str callback')
            def __len__(self):
                raise AssertionError('user len callback')
            def __hash__(self):
                raise AssertionError('user hash callback')
            def __eq__(self, other):
                raise AssertionError('user equality callback')
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Anchor')
            document.notes.add(paragraph[:], 'Detail')
            document.render_page(0, dpi=144)
            before = document.to_bytes()
            cache = document._cached
            current = paragraph[:]
            for invalid in (None, 1, True, b'endnote', [], '', 'Footnote', 'swap', 'endnote\0', 'x' * (1024 * 1024)):
                with self.assertRaises((TypeError, ValueError)):
                    document.notes.convert(kind=invalid)
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cache)
                self.assertEqual(current.text, 'Anchor\ufffc')
            self.assertEqual(document.notes.convert(kind=Hostile('endnote')), 1)
            notes = document.notes
        with self.assertRaises(DocumentClosedError):
            notes.convert(kind=None)


if __name__ == '__main__':
    unittest.main()
