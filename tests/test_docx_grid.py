import unittest

import doc_ex


class DocxGridTests(unittest.TestCase):
    def test_grid_edits_preserve_content_and_invalidate_only_table_children(self):
        with doc_ex.new("docx") as document:
            outside = document.add_paragraph("Outside")
            table = document.add_table([["A", "B"], ["C", "D"]])
            table.column_widths = [100, 100]
            old_cell, old_row = table[0, 0], table.row(0)
            old_paragraph = table[0, 0].paragraphs[0]
            table.insert_row(1)
            self.assertEqual(table.rows, [["A", "B"], ["", ""], ["C", "D"]])
            self.assertEqual(outside.text, "Outside")
            for read in (lambda: old_cell.text, lambda: old_row.height):
                with self.assertRaisesRegex(ValueError, "stale"):
                    read()
            with self.assertRaisesRegex(IndexError, "unknown document child"):
                _ = old_paragraph.text
            cells = table.insert_column(1)
            cells[1].text = "Inserted"
            self.assertEqual(table.rows[1][1], "Inserted")
            table.delete_column(1)
            table.delete_row(1)
            self.assertEqual(table.rows, [["A", "B"], ["C", "D"]])
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened[0].find_tables()[0].rows, table.rows)
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = "\n".join(page.get_text() for page in pdf)
                for item in ("Outside", "A", "B", "C", "D"):
                    self.assertIn(item, text)

    def test_merge_generations_and_failed_insert_leave_handles_valid(self):
        with doc_ex.new("docx") as document:
            table = document.add_table([["A", "B"]])
            cell = table[0, 0]
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                table.insert_column(1)  # Existing columns already fill the available width.
            self.assertEqual(cell.text, "A")
            table.column_widths = [100, 100]
            merged = table.merge((0, 0), (0, 1))
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = cell.text
            merged.split()
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = merged.text
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                table.delete_row(0)
            self.assertEqual(len(table.rows), 1)

    def test_grid_changes_preserve_positive_width_inside_explicit_margins(self):
        with doc_ex.new("docx") as document:
            table = document.add_table([["A", "B"]])
            table.column_widths = [100, 100]
            cell = table.merge((0, 0), (0, 1))
            cell.margins = (0, 75, 0, 75)
            paragraph = cell.paragraphs[0]
            for operation in (
                lambda: table.delete_column(1),
                lambda: cell.split(),
                lambda: setattr(table, "column_widths", [50, 50]),
            ):
                with self.assertRaisesRegex(ValueError, "explicit cell margins"):
                    operation()
                self.assertEqual(table.column_widths, [100, 100])
                self.assertEqual(cell.margins, (0, 75, 0, 75))
                self.assertEqual(cell.text, "A\nB")
                self.assertEqual(paragraph.text, "A")
            cell.margins = (0, 10, 0, 10)
            table.delete_column(1)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened[0].find_tables()[0].rows, [["A\nB"]])
