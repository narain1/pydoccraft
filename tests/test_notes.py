import unittest

import doc_ex
from doc_ex.errors import DocExError, DocumentClosedError


class NoteTests(unittest.TestCase):
    def test_native_note_stories_unicode_and_copy(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('A café citation and conclusion.')
            reference = paragraph.find('café')[0]
            old = paragraph.find('conclusion')[0]
            note = document.notes.add(reference, 'Footnote detail.')
            self.assertEqual(reference.text, 'café')
            self.assertIn('café\ufffc citation', paragraph.text)
            with self.assertRaises(ValueError):
                _ = old.text
            self.assertEqual(note.kind, 'footnote')
            note.paragraphs[0].add_run(' More detail.', italic=True)
            note.add_paragraph('Second paragraph.')
            endnote = document.notes.add(paragraph.find('conclusion')[0], 'Endnote detail.', kind='endnote')
            self.assertEqual(endnote.kind, 'endnote')
            self.assertEqual([item.kind for item in document.notes], ['footnote', 'endnote'])
            with document.copy() as copied:
                copied.notes[0].paragraphs[0].text = 'Independent note.'
                self.assertIn('Footnote detail.', note.paragraphs[0].text)
                self.assertEqual(copied.notes[0].paragraphs[0].text, 'Independent note.')
            with doc_ex.open(document.to_bytes()) as reopened:
                stories = '\n'.join(story.get_text("markdown") for story in reopened.stories)
                self.assertIn('Footnote detail.', stories)
                self.assertIn('Endnote detail.', stories)
                self.assertNotIn('Footnote detail.', reopened.to_markdown())
                self.assertTrue(reopened.render_page(0, dpi=72).startswith(b'\x89PNG'))
            with doc_ex.open(document.convert('pdf')) as pdf:
                text = pdf.to_markdown()
                self.assertIn('Footnote detail.', text)
                self.assertIn('Endnote detail.', text)
        with self.assertRaises(DocumentClosedError):
            _ = note.kind

    def test_note_anchor_guards_and_section_options(self):
        with doc_ex.new('docx') as document, doc_ex.new('docx') as other:
            paragraph = document.add_paragraph('Before marker after')
            reference = paragraph.find('marker')[0]
            with self.assertRaises(ValueError):
                other.notes.add(reference, 'Wrong owner')
            note = document.notes.add(reference, 'Preserved note')
            original = paragraph.text
            with self.assertRaises((ValueError, DocExError)):
                paragraph.text = 'Would remove anchor'
            with self.assertRaises((ValueError, DocExError)):
                paragraph.find('\ufffc')[0].text = ''
            self.assertEqual(paragraph.text, original)
            paragraph.replace_text('Before', 'Changed before')
            self.assertEqual(note.paragraphs[0].text, 'Preserved note')
            with self.assertRaises((ValueError, DocExError)):
                document.notes.add(note.paragraphs[0].find('note')[0], 'Nested note')
            self.assertEqual(len(document.notes), 1)
            section = document.sections[0]
            section.configure_notes(numbering='lower_roman', start=3, restart='each_section')
            self.assertEqual(section.note_options()['numbering'], 'lower_roman')
            self.assertEqual(section.note_options()['start'], 3)
            before = section.note_options()
            with self.assertRaises((ValueError, DocExError)):
                section.configure_notes(start=32768)
            self.assertEqual(section.note_options(), before)
            with self.assertRaises((ValueError, DocExError)):
                section.configure_notes(kind='endnote', restart='each_page')
            section.configure_notes(start=None)
            self.assertEqual(section.note_options()['start'], 1)
            table = document.add_table([['Table citation'], ['Other row']])
            table_note = document.notes.add(table[0, 0].paragraphs[0].find('citation')[0], 'Table note')
            with self.assertRaises((ValueError, DocExError)):
                table.delete_row(0)
            self.assertEqual(table_note.paragraphs[0].text, 'Table note')
            self.assertEqual(len(table.rows), 2)


if __name__ == '__main__':
    unittest.main()
