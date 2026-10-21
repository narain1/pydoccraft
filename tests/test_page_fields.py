from io import BytesIO
import re
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import count_color


def counters(paragraph, label):
    paragraph.add_run(label + ' page=')
    for index, kind in enumerate(('page', 'page_count', 'section_page_count')):
        if index:
            paragraph.add_run((' / pages=', ' / section=')[index - 1])
        paragraph.add_field(kind).format(color='#FF0000')
    return paragraph


def pdf_pages(document):
    with doc_ex.open(document.convert('pdf')) as pdf:
        # PdfCraft may classify aligned counters as table columns and emit tabs.
        # Compare their values while keeping the engine's reading order.
        return [re.sub(r"=\s+", "=", ' '.join(page.get_text().split())) for page in pdf]


class PageFieldTests(unittest.TestCase):
    def test_native_fields_across_scopes_sections_and_repagination(self):
        with doc_ex.new('docx', size=(400, 420)) as document:
            first = document.sections[0]
            first.margins = 40
            first.header_distance = first.footer_distance = 15
            first.page_number_start = 7
            counters(first.header().add_paragraph(), 'Header')
            counters(first.footer().add_paragraph(), 'Footer')
            body = counters(document.add_paragraph(), 'Body')
            cell = document.add_table([['']])[0, 0].paragraphs[0]
            with self.assertRaises(UnsupportedFormatError):
                cell.add_field('page')
            cell.add_run('Cell pages=')
            cell.add_field('page_count').format(color='#FF0000')
            cell.add_run(' / section=')
            cell.add_field('section_page_count').format(color='#FF0000')
            anchor = document.add_paragraph('References')
            note = document.notes.add(anchor.find('References')[0])
            counters(note.paragraphs[0], 'Note')
            endnote = document.notes.add(anchor.find('References')[0], kind='endnote')
            counters(endnote.paragraphs[0], 'Endnote')
            document.add_page_break()
            counters(document.add_paragraph(), 'Second')
            second = document.sections.add()
            self.assertIsNone(second.page_number_start)
            counters(document.add_paragraph(), 'Third')
            self.assertIn('Third page=9 / pages=3 / section=1', pdf_pages(document)[2])
            second.page_number_start = 20
            pages = pdf_pages(document)
            for label in ('Body', 'Note', 'Header', 'Footer'):
                self.assertIn(label + ' page=7 / pages=3 / section=2', pages[0])
            self.assertIn('Cell pages=3 / section=2', pages[0])
            self.assertIn('Second page=8 / pages=3 / section=2', pages[1])
            self.assertIn('Third page=20 / pages=3 / section=1', pages[2])
            self.assertIn('Endnote page=20 / pages=3 / section=1', pages[2])
            before = document.render_page(0, dpi=144)
            self.assertGreater(count_color(before, b'\xff\0\0\xff'), 10)
            with document.copy() as copied:
                document.add_page_break()
                counters(document.add_paragraph(), 'Fourth')
                pages = pdf_pages(document)
                self.assertEqual(len(pages), 4)
                self.assertIn('Body page=7 / pages=4 / section=2', pages[0])
                self.assertIn('Fourth page=21 / pages=4 / section=2', pages[3])
                self.assertNotEqual(document.render_page(0, dpi=144), before)
                self.assertEqual(copied.render_page(0, dpi=144), before)
            raw = document.to_bytes()
            with ZipFile(BytesIO(raw)) as archive:
                xml = ET.fromstring(archive.read('word/document.xml'))
                w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
                instructions = [node.text.strip() for node in xml.iter(w + 'instrText')]
                self.assertEqual(set(instructions), {'PAGE \\* Arabic', 'NUMPAGES \\* Arabic', 'SECTIONPAGES \\* Arabic'})
                self.assertEqual([node.attrib[w + 'start'] for node in xml.iter(w + 'pgNumType')], ['7', '20'])
            with doc_ex.open(raw) as reopened:
                self.assertIn('Body page= / pages= / section=', reopened[0].get_text())
                self.assertNotIn('Body page=7', reopened[0].get_text())
                self.assertEqual(reopened.render_page(0, dpi=144), document.render_page(0, dpi=144))
            second.page_number_start = None
            self.assertIn('Third page=9 / pages=4 / section=2', pdf_pages(document)[2])

    def test_endnote_display_scope_and_copied_headers_are_checked_atomically(self):
        with doc_ex.new('docx', size=(360, 420)) as document:
            document.add_paragraph('First section')
            continuous = document.sections.add(start='continuous')
            anchor = document.add_paragraph('Anchor in continuous section')
            note = document.notes.add(anchor.find('Anchor')[0], kind='endnote')
            document.sections.add()
            document.add_paragraph('Final section')
            counters(note.paragraphs[0], 'Placed')
            pages = pdf_pages(document)
            self.assertIn('Placed page=2 / pages=2 / section=1', pages[1])
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                continuous.configure_notes(kind='endnote', placement='section_end')
            self.assertEqual(document.to_bytes(), before)
        with doc_ex.new('docx') as document:
            header = document.sections[0].header().add_paragraph('Total ')
            header.add_field('section_page_count')
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                document.sections.add(start='continuous')
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(len(document.sections), 1)

    def test_intermediate_endnote_collectors_reject_unsafe_fields_and_later_edits(self):
        for trigger in ('insert', 'placement', 'start'):
            with self.subTest(trigger=trigger), doc_ex.new('docx', size=(360, 420)) as document:
                document.sections[0].margins = 40
                anchor = document.add_paragraph('First anchor')
                note = document.notes.add(anchor.find('anchor')[0], kind='endnote')
                collector = document.sections.add(start='next_page' if trigger == 'start' else 'continuous')
                collector.configure_notes(kind='endnote', placement='document_end' if trigger == 'placement' else 'section_end')
                document.add_paragraph('Collector begins')
                document.add_page_break()
                document.add_paragraph('Collector second page')
                last = document.sections.add()
                last.configure_notes(kind='endnote', placement='document_end')
                document.add_paragraph('Final section')
                paragraph = note.paragraphs[0]
                paragraph.add_run('Deferred count=')
                field = None if trigger == 'insert' else paragraph.add_field('section_page_count')
                before, png = document.to_bytes(), document.render_page(0, dpi=72)
                with self.assertRaises(UnsupportedFormatError):
                    if trigger == 'insert':
                        paragraph.add_field('section_page_count')
                    elif trigger == 'placement':
                        collector.configure_notes(kind='endnote', placement='section_end')
                    else:
                        collector.start = 'continuous'
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), png)
                if field is not None:
                    self.assertEqual(field.text, '\ufffc')

    def test_earlier_safe_collector_takes_precedence_over_later_continuous_collector(self):
        with doc_ex.new('docx', size=(360, 420)) as document:
            document.sections[0].margins = 40
            anchor = document.add_paragraph('First anchor')
            note = document.notes.add(anchor.find('anchor')[0], kind='endnote')
            safe = document.sections.add()
            safe.configure_notes(kind='endnote', placement='section_end')
            document.add_paragraph('Safe collector')
            later = document.sections.add(start='continuous')
            later.configure_notes(kind='endnote', placement='section_end')
            document.add_paragraph('Later collector')
            final = document.sections.add()
            final.configure_notes(kind='endnote', placement='document_end')
            document.add_paragraph('Final section')
            note.paragraphs[0].add_run('Safe count=')
            field = note.paragraphs[0].add_field('section_page_count')
            self.assertIn('Safe count=1', pdf_pages(document)[1])
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                safe.configure_notes(kind='endnote', placement='document_end')
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(field.text, '\ufffc')

    def test_markers_ranges_failure_rollback_and_owner_lifetime(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('e\u0301🦀 ')
            field = paragraph.add_field('page')
            self.assertEqual(field.text, '\ufffc')
            field.format(bold=True, font_size=20)
            self.assertEqual(field.text, '\ufffc')
            raw = document.to_bytes()
            for kind in ('date', 'PAGE', '', 'section'):
                with self.assertRaises(ValueError):
                    paragraph.add_field(kind)
                self.assertEqual(document.to_bytes(), raw)
                self.assertEqual(field.text, '\ufffc')
            with self.assertRaises(ValueError):
                paragraph.add_run('\ufffc')
            field.text = 'replaced'
            self.assertEqual(paragraph.text, 'e\u0301🦀 replaced')
            stale = paragraph.add_field('page_count')
            paragraph.add_run('!')
            with self.assertRaises(ValueError):
                _ = stale.text
            paragraph[-2:-1].text = ''
            self.assertNotIn('\ufffc', paragraph.text)
            section = document.sections[0]
            before = document.to_bytes()
            for value in (True, False, 0, -1, 32768, 1.5, '1'):
                with self.assertRaises((TypeError, ValueError)):
                    section.page_number_start = value
                self.assertEqual(document.to_bytes(), before)
            section.page_number_start = 32767
            paragraph.add_field('page')
            self.assertIn('32767', pdf_pages(document)[0])
            class ClosesOwner:
                def __index__(self):
                    document.close()
                    return 1
            with self.assertRaises(DocumentClosedError):
                section.page_number_start = ClosesOwner()
            with self.assertRaises(DocumentClosedError):
                paragraph.add_field('page')

    def test_continuous_sections_keep_supported_fields_and_reject_unsafe_counts(self):
        with doc_ex.new('docx', size=(360, 420)) as document:
            first = document.sections[0]
            first.page_number_start = 3
            counters(document.add_paragraph(), 'First')
            continuous = document.sections.add(start='continuous')
            body = document.add_paragraph('Continuous=')
            body.add_field('page')
            body.add_run(' total=')
            body.add_field('page_count')
            anchor = document.add_paragraph('Note anchor')
            for paragraph in (body, continuous.header().add_paragraph('Header'),
                              continuous.footer().add_paragraph('Footer'),
                              document.add_table([['Cell']])[0, 0].paragraphs[0],
                              document.notes.add(anchor.find('anchor')[0]).paragraphs[0],
                              document.notes.add(document.paragraphs[0].find('First')[0],
                                                 kind='endnote').paragraphs[0]):
                before = document.to_bytes()
                with self.assertRaises(UnsupportedFormatError):
                    paragraph.add_field('section_page_count')
                self.assertEqual(document.to_bytes(), before)
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                continuous.page_number_start = 10
            self.assertEqual(document.to_bytes(), before)
            document.add_page_break()
            last = document.add_paragraph('Next=')
            last.add_field('page')
            pages = pdf_pages(document)
            self.assertIn('Continuous=3 total=2', pages[0])
            self.assertIn('Next=4', pages[1])
            # A later scope change must not make an already-authored field unsafe.
            separate = document.sections.add()
            counters(document.add_paragraph(), 'Separate')
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                separate.start = 'continuous'
            self.assertEqual(document.to_bytes(), before)


if __name__ == '__main__':
    unittest.main()
