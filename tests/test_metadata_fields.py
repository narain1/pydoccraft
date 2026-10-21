from io import BytesIO
from pathlib import Path
import tempfile
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import count_color


def values(paragraph, label):
    paragraph.add_run(label + ' ')
    paragraph.add_field('title').format(color='#FF0000')
    paragraph.add_run(' ')
    paragraph.add_field('author').format(color='#0000FF')


def pdf_text(document):
    with doc_ex.open(document.convert('pdf')) as pdf:
        return ' '.join(page.get_text() for page in pdf)


class MetadataFieldTests(unittest.TestCase):
    def test_current_metadata_in_all_stories_with_empty_stored_results(self):
        with doc_ex.new('docx') as document:
            # A named bold face gives the color fixture solid interior ink at72DPI.
            document.styles['Normal'].format(font='Source Sans 3', bold=True)
            document.update_metadata(title='Report Ω', author='Writer é')
            values(document.add_paragraph(), 'Body')
            values(document.add_table([['']])[0, 0].paragraphs[0], 'Cell')
            values(document.sections[0].header().add_paragraph(), 'Header')
            values(document.sections[0].footer().add_paragraph(), 'Footer')
            anchor = document.add_paragraph('Anchor')
            values(document.notes.add(anchor[:]).paragraphs[0], 'Footnote')
            values(document.notes.add(anchor[:], kind='endnote').paragraphs[0], 'Endnote')
            for dpi in (72, 144):
                image = document.render_page(0, dpi=dpi)
                self.assertGreater(count_color(image, b'\xff\0\0\xff'), 10)
                self.assertGreater(count_color(image, b'\0\0\xff\xff'), 10)
                with doc_ex.open(document.convert('pdf')) as exported:
                    preview = exported.render_page(0, dpi=dpi)
                    self.assertGreater(count_color(preview, b'\xff\0\0\xff'), 10)
                    self.assertGreater(count_color(preview, b'\0\0\xff\xff'), 10)
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual(reopened.render_page(0, dpi=dpi), image)
                    self.assertNotIn('Report Ω', reopened.to_markdown())
                    self.assertIn('dirty_field_cached_result', {item['code'] for item in reopened.metadata['diagnostics']})
            text = pdf_text(document)
            self.assertEqual(text.count('Report Ω'), 6)
            self.assertEqual(text.count('Writer é'), 6)
            for label in ('Body', 'Cell', 'Header', 'Footer', 'Footnote', 'Endnote'):
                self.assertIn(label, text)
            raw = document.to_bytes()
            self.assertEqual(document.to_bytes(), raw)
            with ZipFile(BytesIO(raw)) as archive:
                w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
                instructions = []
                for name in archive.namelist():
                    if name.startswith('word/') and name.endswith('.xml'):
                        xml = ET.fromstring(archive.read(name))
                        instructions.extend(node.text.strip() for node in xml.iter(w + 'instrText'))
                self.assertEqual(instructions.count('TITLE'), 6)
                self.assertEqual(instructions.count('AUTHOR'), 6)
            before = document.render_page(0, dpi=144)
            with document.copy() as copied:
                document.update_metadata(title='Changed Ω', author='Other é')
                changed = pdf_text(document)
                self.assertEqual(changed.count('Changed Ω'), 6)
                self.assertEqual(changed.count('Other é'), 6)
                self.assertNotIn('Report Ω', changed)
                self.assertNotEqual(document.render_page(0, dpi=144), before)
                self.assertEqual(copied.render_page(0, dpi=144), before)
                document.update_metadata(title=None, author=None)
                cleared = pdf_text(document)
                self.assertNotIn('Changed Ω', cleared)
                self.assertNotIn('Other é', cleared)
                document.update_metadata(title='Report Ω', author='Writer é')
                self.assertEqual(document.render_page(0, dpi=144), before)

    def test_metadata_ranges_follow_existing_lifetime_and_replacement_rules(self):
        document = doc_ex.new('docx')
        document.update_metadata(title='Report')
        paragraph = document.add_paragraph('Title: ')
        field = paragraph.add_field('title')
        self.assertEqual(field.text, '\ufffc')
        field.format(bold=True)
        before = document.to_bytes()
        for kind in ('date', 'filename', 'docproperty', 'TITLE', 'unknown'):
            with self.assertRaises(ValueError):
                paragraph.add_field(kind)
            self.assertEqual(document.to_bytes(), before)
        document.update_metadata(title='Changed')
        with self.assertRaisesRegex(ValueError, 'stale'):
            _ = field.text
        paragraph[-1:].text = 'Literal'
        self.assertEqual(paragraph.text, 'Title: Literal')
        self.assertNotIn('Changed', pdf_text(document))
        self.assertIn('Title: Literal', pdf_text(document))
        document.close()
        with self.assertRaises(DocumentClosedError):
            paragraph.add_field('title')

    def test_metadata_field_geometry_failures_keep_destination_and_recover(self):
        with doc_ex.new('docx', size=(300, 300)) as document:
            document.sections[0].margins = 30
            document.update_metadata(title='Short')
            paragraph = document.add_paragraph()
            paragraph.add_field('title')
            good = document.to_bytes()
            document.update_metadata(title=('Long title ' * 80).strip())
            with tempfile.TemporaryDirectory() as folder:
                destination = Path(folder) / 'existing.docx'
                destination.write_bytes(good)
                for output in (document.to_bytes, lambda: document.render_page(0, dpi=72),
                               lambda: document.convert('pdf'), lambda: document.save(destination)):
                    with self.assertRaisesRegex(UnsupportedFormatError, 'metadata field'):
                        output()
                    self.assertEqual(destination.read_bytes(), good)
            document.update_metadata(title='Short')
            self.assertEqual(document.to_bytes(), good)
            before = document.to_bytes()
            for text in ('A\nB', 'A\tB', 'A\x85B', 'A\u2028B', 'A\u2029B'):
                with self.assertRaisesRegex(UnsupportedFormatError, 'single.line'):
                    document.update_metadata(title=text)
                self.assertEqual(document.to_bytes(), before)
            paragraph[:].format(font_size=200)
            with self.assertRaisesRegex(UnsupportedFormatError, 'metadata field'):
                document.to_bytes()
            paragraph[:].format(font_size=None)
            self.assertEqual(pdf_text(document).count('Short'), 1)
            table = document.add_table([['']])
            table[0, 0].paragraphs[0].add_field('title')
            widths = table.column_widths
            table.column_widths = [20]
            with self.assertRaisesRegex(UnsupportedFormatError, 'metadata field'):
                document.to_bytes()
            table.column_widths = widths
            table[0, 0].format(text_direction='down')
            with self.assertRaisesRegex(UnsupportedFormatError, 'metadata field'):
                document.to_bytes()
            table[0, 0].format(text_direction=None)
            self.assertIn('Short', pdf_text(document))
            table.row(0).height = (12, 'exact')
            with self.assertRaisesRegex(UnsupportedFormatError, 'metadata field'):
                document.to_bytes()
            table.row(0).height = None
            self.assertEqual(pdf_text(document).count('Short'), 2)
