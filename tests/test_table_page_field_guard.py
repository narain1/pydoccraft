"""Owned table PAGE refusal preserves supported fields and caller state."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_cell_paragraph_delete import owned_table


class TablePageFieldGuardTests(unittest.TestCase):
    def test_table_page_refusal_is_atomic_with_supported_counts_and_direct_controls(self):
        for scope in ('body', 'header', 'footer'):
            for merged in (None, (0, 1), (1, 0)):
                for nested in (False, True):
                    with self.subTest(scope=scope, merged=merged, nested=nested), doc_ex.new('docx') as document:
                        document.sections[0].margins = (180, 36, 180, 36)
                        document.add_paragraph('First page')
                        document.add_page_break()
                        direct = document.add_paragraph('Direct page ')
                        direct.add_field('page')
                        table = owned_table(document, scope, [['Keep', 'Other'], ['Below', 'Last']])
                        cell = table[0, 0] if merged is None else table.merge((0, 0), merged)
                        if merged is not None:
                            cell = table[merged]
                        if nested:
                            cell = cell.add_table([['Keep']])[0, 0]
                        paragraph = cell.paragraphs[0]
                        before = document.to_bytes()
                        selected = paragraph[:4]
                        image = document.render_page(0, dpi=72)
                        cache = document._cached
                        with document.copy() as copied:
                            with self.assertRaisesRegex(UnsupportedFormatError, 'page fields in table cells'):
                                paragraph.add_field('page')
                            self.assertEqual(document.to_bytes(), before)
                            self.assertEqual(copied.to_bytes(), before)
                            self.assertEqual(selected.text, 'Keep')
                            self.assertIs(document._cached, cache)
                            self.assertEqual(document.render_page(0, dpi=72), image)
                        stream = BytesIO()
                        document.save(stream)
                        self.assertEqual(stream.getvalue(), before)
                        counts = cell.add_paragraph('Counts ')
                        counts.add_field('page_count')
                        counts.add_run(' / ')
                        counts.add_field('section_page_count')
                        raw = document.to_bytes()
                        with ZipFile(BytesIO(raw)) as archive:
                            instructions = []
                            for name in archive.namelist():
                                if name.startswith('word/') and name.endswith('.xml'):
                                    root = ET.fromstring(archive.read(name))
                                    for native_cell in root.iter('{http://schemas.openxmlformats.org/wordprocessingml/2006/main}tc'):
                                        instructions.extend(n.text.strip() for n in native_cell.iter('{http://schemas.openxmlformats.org/wordprocessingml/2006/main}instrText'))
                            self.assertIn('NUMPAGES \\* Arabic', instructions)
                            self.assertIn('SECTIONPAGES \\* Arabic', instructions)
                            self.assertNotIn('PAGE \\* Arabic', instructions)
                        if merged is None and not nested:
                            with doc_ex.open(document.convert('pdf')) as pdf:
                                self.assertEqual(len(pdf), 2)
                                self.assertIn('Direct page 2', pdf[1].get_text())
                                text = ' '.join(page.get_text() for page in pdf)
                                self.assertIn('Counts 2 / 2', ' '.join(text.split()))
                                for dpi in (72, 144):
                                    self.assertGreater(len(pdf.render_page(1, dpi=dpi)), 1000)
                                    self.assertGreater(len(document.render_page(1, dpi=dpi)), 1000)
                        with doc_ex.open(raw) as reopened:
                            self.assertIn('Counts', reopened.to_markdown() + ' '.join(s.get_text() for s in reopened.stories))
                        document.close()
                        with self.assertRaises(DocumentClosedError):
                            paragraph.add_field('page')


if __name__ == '__main__':
    unittest.main()
