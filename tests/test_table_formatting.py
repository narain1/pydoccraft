from io import BytesIO
import unittest
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocExError


class TableFormattingTests(unittest.TestCase):
    def test_direct_formatting_native_fit_and_metadata_roundtrip(self):
        with doc_ex.new('docx') as document:
            table = document.add_table([['Short', 'Longer measured content'], ['A', 'B']])
            table.format(alignment='center', width=360, layout='fixed', title='Quarterly totals', description='Amounts by region')
            self.assertEqual(table.formatting['alignment'], 'center')
            self.assertEqual(table.formatting['width'], 360)
            self.assertEqual(table.formatting['title'], 'Quarterly totals')
            table.format(width=None, width_percent=80, rtl=False)
            self.assertIsNone(table.formatting['width'])
            self.assertEqual(table.formatting['width_percent'], 80)
            cell = table[0, 0]
            cell.format(background=(255, 255, 0), margins=(4, 5, 6, 7), alignment='right',
                        vertical_alignment='center', text_direction='horizontal', no_wrap=False,
                        borders={'bottom': {'style': 'double', 'color': 'blue', 'width': 1}})
            self.assertEqual(cell.background, '#FFFF00')
            self.assertEqual(cell.margins, (4, 5, 6, 7))
            self.assertEqual(cell.alignment, 'right')
            self.assertEqual(cell.formatting['borders']['bottom']['style'], 'double')
            cell.format(borders={'bottom': None}, vertical_alignment=None)
            self.assertIsNone(cell.formatting['borders'])
            self.assertIsNone(cell.formatting['vertical_alignment'])
            cell.format(borders={'bottom': {'style': 'single'}})
            snapshot = cell.formatting
            self.assertIsNone(snapshot['borders']['bottom']['color'])
            cell.format(**snapshot)
            self.assertEqual(cell.formatting, snapshot)
            table.format(width_percent=None, layout='autofit')
            widths = table.fit()
            self.assertEqual(len(widths), 2)
            self.assertTrue(all(width > 0 for width in widths))
            self.assertEqual(table.column_widths, widths)
            self.assertEqual(cell.text, 'Short')
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                xml = archive.read('word/document.xml').decode()
                self.assertIn('Quarterly totals', xml)
                self.assertIn('Amounts by region', xml)
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    self.assertIn('Longer measured content', reopened.to_markdown())
                    self.assertNotIn('Amounts by region', reopened.to_markdown())
                    self.assertTrue(reopened.render_page(0, dpi=72).startswith(b'\x89PNG'))

    def test_invalid_format_patches_are_atomic(self):
        with doc_ex.new('docx') as document:
            table = document.add_table([['Keep', 'Stable']])
            cell = table[0, 0]
            initial = table.formatting
            initial_cell = cell.formatting
            for patch in ({'width': 200, 'width_percent': 50}, {'alignment': 'justify'},
                          {'width_percent': 101}, {'title': 'x' * 10001}, {'unknown': 1},
                          {'borders': {'diagonal': {'style': 'single'}}},
                          {'borders': {'top': {'style': 'unsupported'}}}):
                with self.subTest(patch=next(iter(patch))), self.assertRaises((ValueError, TypeError, DocExError)):
                    table.format(**patch)
                self.assertEqual(table.formatting, initial)
                self.assertEqual(cell.text, 'Keep')
            for patch in ({'text_direction': 'sideways'}, {'margins': (1, 1000, 1, 1000)},
                          {'borders': {'inside_horizontal': {'style': 'single'}}}, {'no_wrap': 'yes'}):
                with self.subTest(patch=patch), self.assertRaises((ValueError, TypeError, DocExError)):
                    cell.format(**patch)
                self.assertEqual(cell.formatting, initial_cell)
                self.assertEqual(cell.text, 'Keep')


if __name__ == '__main__':
    unittest.main()
