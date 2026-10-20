import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError


class TableViewTests(unittest.TestCase):
    def test_count_construction_and_ordered_slices_in_both_formats(self):
        for kind in ('docx', 'pptx'):
            with self.subTest(kind=kind), doc_ex.new(kind) as document:
                if kind == 'docx':
                    table = document.add_table(rows=3, columns=3)
                else:
                    table = document.slides.add().add_table(rows=3, columns=3, box=(20, 20, 600, 300))
                self.assertEqual(table.rows, [['', '', ''], ['', '', ''], ['', '', '']])
                for row, cells in enumerate(table[:, :]):
                    for column, cell in enumerate(cells):
                        cell.text = f'{row},{column}'
                self.assertEqual([cell.text for cell in table[-1, ::-1]], ['2,2', '2,1', '2,0'])
                self.assertEqual([cell.text for cell in table[::-1, -1]], ['2,2', '1,2', '0,2'])
                self.assertEqual([[cell.text for cell in row] for row in table[1:, 1:]], [['1,1', '1,2'], ['2,1', '2,2']])
                self.assertEqual(table[0:0, :], [])
                self.assertEqual(table[:, 0:0], [[], [], []])
                self.assertEqual(table[0, 0:0], [])
                with self.assertRaises(ValueError):
                    _ = table[::0, :]
                old = table[:, 0]
                table.insert_row(1)
                with self.assertRaises(ValueError):
                    _ = old[0].text
                table.merge((0, 0), (0, 1))
                aliases = table[0, :2]
                aliases[0].text = 'Merged'
                self.assertEqual(aliases[1].text, 'Merged')
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertIn('Merged', reopened.to_markdown())
                if kind == 'docx':
                    nested = table[1, 0].add_table(rows=1, columns=2)
                    nested[0, 1].text = 'Nested'
                    self.assertEqual(nested.rows, [['', 'Nested']])
                else:
                    with self.assertRaises(UnsupportedFormatError):
                        table.format(alignment='center')
            with self.assertRaises(DocumentClosedError):
                _ = table[:, 0]

    def test_invalid_counts_fail_before_mutation(self):
        with doc_ex.new('docx') as document:
            for rows, columns in ((0, 1), (1, 0), (-1, 2), (10000, 1000), (True, 2), (2, False), (1.5, 2), ([['a']], 2)):
                with self.subTest(rows=rows, columns=columns), self.assertRaises((ValueError, TypeError)):
                    document.add_table(rows=rows, columns=columns)
                self.assertEqual(len(document.tables), 0)
            with self.assertRaises(ValueError):
                document.add_table(rows=1, columns=64)
            self.assertEqual(len(document.tables), 0)
            for rows in ([], [[]], [['a'], ['b', 'c']]):
                with self.assertRaises(ValueError):
                    document.add_table(rows)
                self.assertEqual(len(document.tables), 0)


if __name__ == '__main__':
    unittest.main()
