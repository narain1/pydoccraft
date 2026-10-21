from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_note_conversion import pdf_text_pages, red_rows
from test_picture_media import count_color, media_parts, red_picture

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def note_xml(document, kind):
    filename = 'word/footnotes.xml' if kind == 'footnote' else 'word/endnotes.xml'
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        root = ET.fromstring(archive.read(filename))
        return [node for node in root if node.get(W + 'type') is None]


class NoteClearTests(unittest.TestCase):
    def test_clear_keeps_reference_and_number_with_visible_content_removed(self):
        for kind in ('footnote', 'endnote'):
            with self.subTest(kind=kind), doc_ex.new('docx', size=(300, 360)) as document:
                document.sections[0].margins = 24
                document.sections[0].configure_notes(kind=kind, start=3, numbering='decimal')
                anchor = document.add_paragraph('Citation')
                note = document.notes.add(anchor[:], 'RemovedDetail', kind=kind)
                old = note.paragraphs[0]
                old[:].format(color='red')
                other = document.notes.add(anchor[:], 'RetainedDetail', kind=kind)
                other_paragraph = other.paragraphs[0]
                note.add_paragraph('RemovedSecond')[:].format(color='red')
                document.add_page_break()
                document.add_paragraph('FinalBody')
                before_pngs = [document.render_page(i, dpi=144) for i in range(document.page_count)]
                self.assertTrue(any(red_rows(png) for png in before_pngs))
                self.assertIn('RemovedDetail', ' '.join(pdf_text_pages(document)))
                body_text = anchor.text
                identifiers = [node.get(W + 'id') for node in note_xml(document, kind)]
                before_options = document.sections[0].note_options(kind=kind)
                note.clear()
                self.assertEqual(len(note.paragraphs), 0)
                self.assertEqual(note.kind, kind)
                self.assertEqual(len(document.notes), 2)
                self.assertEqual(anchor.text, body_text)
                self.assertEqual(other_paragraph.text, 'RetainedDetail')
                with self.assertRaises((ValueError, IndexError)):
                    _ = old.text
                after = note_xml(document, kind)
                self.assertEqual([node.get(W + 'id') for node in after], identifiers)
                cleared = after[0]
                self.assertEqual(len(list(cleared.iter(W + kind + 'Ref'))), 1)
                self.assertEqual(list(cleared.iter(W + 't')), [])
                self.assertEqual(cleared.find('.//' + W + 'pStyle').get(W + 'val'),
                                 'FootnoteText' if kind == 'footnote' else 'EndnoteText')
                self.assertEqual(document.sections[0].note_options(kind=kind), before_options)
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        text = ' '.join(page.get_text() for page in reopened) + ' ' + ' '.join(story.get_text() for story in reopened.stories)
                        self.assertNotIn('RemovedDetail', text)
                        self.assertNotIn('RemovedSecond', text)
                        self.assertIn('RetainedDetail', text)
                        self.assertFalse(any(red_rows(reopened.render_page(i, dpi=144))
                                             for i in range(reopened.page_count)))
                output = ' '.join(pdf_text_pages(document))
                self.assertRegex(output, r'4\s*RetainedDetail')
                self.assertRegex(output, r'\b3\b')

    def test_long_note_clear_rebuilds_layout_and_recovers_one_paragraph(self):
        for kind in ('footnote', 'endnote'):
            with self.subTest(kind=kind), doc_ex.new('docx', size=(300, 360)) as document:
                document.sections[0].margins = 24
                anchor = document.add_paragraph('Citation')
                note = document.notes.add(anchor[:], 'LongStart', kind=kind)
                for i in range(45):
                    note.add_paragraph(f'Continuation{i:02} complete text.')
                count = document.page_count
                self.assertGreater(count, 1)
                for page in range(count):
                    document.render_page(page, dpi=72)
                self.assertIn('Continuation44', ' '.join(pdf_text_pages(document)))
                with document.copy() as copied:
                    note.clear()
                    self.assertIsNone(document._cached)
                    self.assertLess(document.page_count, count)
                    self.assertEqual(document.page_count, 1)
                    self.assertEqual(copied.page_count, count)
                    self.assertIn('Continuation44', ' '.join(pdf_text_pages(copied)))
                    self.assertNotIn('Continuation', ' '.join(pdf_text_pages(document)))
                    fresh = note.add_paragraph('RestoredText')
                    self.assertEqual(len(note.paragraphs), 1)
                    self.assertEqual(len(note_xml(document, kind)[0].findall(W + 'p')), 1)
                    self.assertIn('RestoredText', ' '.join(pdf_text_pages(document)))
                    self.assertEqual(fresh.style, 'Footnote Text' if kind == 'footnote' else 'Endnote Text')

    def test_empty_clear_conversion_and_noop_preserve_cache_and_ranges(self):
        with doc_ex.new('docx') as document:
            anchor = document.add_paragraph('Anchor')
            note = document.notes.add(anchor[:], '')
            empty = note.paragraphs[0]
            selection = anchor[:]
            document.render_page(0, dpi=72)
            note.clear()
            with self.assertRaises((ValueError, IndexError)):
                _ = empty.text
            with self.assertRaises(ValueError):
                _ = selection.text
            for kind in ('footnote', 'endnote', 'footnote'):
                document.notes.convert(kind=kind)
                selection = anchor[:]
                png = document.render_page(0, dpi=72)
                cached = document._cached
                raw = document.to_bytes()
                note.clear()
                self.assertEqual(selection.text, anchor.text)
                self.assertIs(document._cached, cached)
                self.assertEqual(document.render_page(0, dpi=72), png)
                self.assertEqual(document.to_bytes(), raw)
                with document.copy() as copied:
                    other = copied.notes[0]
                    other.clear()
                    self.assertEqual(copied.to_bytes(), raw)
                    other.add_paragraph('Only copy')
                    self.assertEqual(len(note.paragraphs), 0)
                    self.assertEqual(len(other.paragraphs), 1)
            note.add_paragraph('')
            selected = anchor[:]
            note.clear()
            with self.assertRaises(ValueError):
                _ = selected.text

    def test_fields_links_nested_body_anchor_and_shared_media_survive_clear(self):
        with doc_ex.new('docx') as document:
            target = document.add_paragraph('Destination')
            document.bookmarks['target'] = target[:]
            outer = document.add_table([['Outer']])
            nested = outer[0, 0].add_table([['Citation']])
            anchor = nested[0, 0].paragraphs[0]
            note = document.notes.add(anchor[:], 'LinkedNote')
            paragraph = note.paragraphs[0]
            paragraph[:].link = '#target'
            paragraph.add_field('page')
            note.add_paragraph('External').add_run('Label').link = 'https://example.test/clear'
            picture = document.add_image(red_picture(2, 2), width=20)
            document.background_image = picture.data
            original = anchor.text
            visible_cell = nested[0, 0].text
            media = media_parts(document)
            note.clear()
            self.assertEqual(anchor.text, original)
            self.assertEqual(document.bookmarks['target'].text, 'Destination')
            self.assertEqual(media_parts(document), media)
            self.assertEqual(picture.data, media[0])
            self.assertNotIn('LinkedNote', ' '.join(pdf_text_pages(document)))
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(sum(len(page.get_links()) for page in pdf), 0)
            self.assertEqual(len(note.paragraphs), 0)
            self.assertEqual(nested[0, 0].text, visible_cell)

    def test_note_style_reset_and_closed_owner_keep_rich_profile_explicit(self):
        document = doc_ex.new('docx')
        anchor = document.add_paragraph('Anchor')
        note = document.notes.add(anchor[:], 'Custom')
        document.styles['Footnote Text'].format(color='blue', font_size=20)
        document.styles['Endnote Text'].format(color='red', font_size=20)
        custom = document.styles.add('Custom Note', based_on='Footnote Text')
        custom.format(color='red', font_size=22)
        note.paragraphs[0].style = custom
        note.clear()
        for kind, color in (('footnote', b'\0\0\xff\xff'),
                            ('endnote', b'\xff\0\0\xff')):
            document.notes.convert(kind=kind)
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    self.assertGreater(count_color(reopened.render_page(0, dpi=144), color), 5)
            cached = document._cached
            note.clear()
            self.assertIs(document._cached, cached)
        document.notes.convert(kind='footnote')
        fresh = note.add_paragraph('Default note')
        self.assertEqual(fresh.style, 'Footnote Text')
        self.assertNotEqual(document.styles[fresh.style].resolved_formatting['color'], '#FF0000')
        for operation in (lambda: note.images, lambda: note.tables,
                          lambda: note.add_table([['unsupported']]),
                          lambda: note.add_image(red_picture(1, 1))):
            with self.assertRaises(UnsupportedFormatError):
                operation()
        document.close()
        for operation in (note.clear, lambda: note.paragraphs[0],
                          lambda: note.add_paragraph('Closed')):
            with self.assertRaises(DocumentClosedError):
                operation()
