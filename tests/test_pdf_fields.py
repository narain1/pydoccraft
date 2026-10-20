import unittest

import doc_ex
from doc_ex.errors import DocExError, DocumentClosedError, UnsupportedFormatError


class PdfFieldTests(unittest.TestCase):
    def test_owned_forms_render_copy_and_flatten(self):
        with doc_ex.new('pdf') as document:
            page = document.pages[0]
            self.assertEqual(document.fields.flatten(), 0)
            self.assertEqual(page.number, 0)
            field = page.fields.add('customer', (40, 40, 200, 30))
            field.value = 'Alice'
            self.assertEqual(field.value, 'Alice')
            self.assertEqual(field.kind, 'text')
            checkbox = page.fields.add('approved', (40, 90, 24, 24), kind='checkbox')
            checkbox.value = True
            with self.assertRaises(ValueError):
                _ = field.value
            choice = page.fields.add('country', (40, 140, 200, 30), kind='choice', options=['Canada', 'France'])
            self.assertIsNone(choice.value)
            self.assertEqual(choice.options, ('Canada', 'France'))
            choice.value = 'France'
            self.assertEqual(len(document.fields), 3)
            self.assertEqual(len(page.fields), 3)
            for actual, expected in zip(document.fields['customer'].box, (40, 40, 200, 30)):
                self.assertAlmostEqual(actual, expected, delta=0.001)
            self.assertEqual(document.fields['country'].page.number, 0)
            with document.copy() as copied:
                copied.fields['customer'].value = 'Bob'
                self.assertEqual(document.fields['customer'].value, 'Alice')
                copied.fields['country'].value = None
                self.assertIsNone(copied.fields['country'].value)
            with doc_ex.open(document.to_bytes()) as reopened:
                image = reopened.render_page(0, dpi=72)
                self.assertTrue(image.startswith(b'\x89PNG'))
                with self.assertRaises(UnsupportedFormatError):
                    reopened.edit()
            with self.assertRaises(UnsupportedFormatError):
                document.convert('docx')
            with self.assertRaises(UnsupportedFormatError):
                doc_ex.convert(document.to_bytes(), 'docx')
            old = document.fields['customer']
            self.assertEqual(document.fields.flatten(), 3)
            self.assertEqual(len(document.fields), 0)
            with self.assertRaises(ValueError):
                _ = old.value
            with doc_ex.open(document.to_bytes()) as flattened:
                self.assertEqual(flattened.render_page(0, dpi=72), image)
                self.assertIn('Alice', flattened.to_markdown())
                self.assertIn('France', flattened.to_markdown())
            with doc_ex.open(document.convert('docx')) as converted:
                self.assertIn('Alice', converted.to_markdown())
                self.assertIn('France', converted.to_markdown())
        with self.assertRaises(DocumentClosedError):
            _ = old.value

    def test_invalid_values_are_atomic_and_names_do_not_revive(self):
        with doc_ex.new('pdf') as document:
            page = document.pages[0]
            field = page.fields.add('name', (30, 30, 100, 30))
            field.value = 'Valid'
            before = document.to_bytes()
            for value in (True, None, 'Unsupported \u03bb', 'A' * 100, 'a\nb'):
                with self.subTest(value=value), self.assertRaises((ValueError, TypeError, DocExError)):
                    field.value = value
                self.assertEqual(field.value, 'Valid')
                self.assertEqual(document.to_bytes(), before)
            for name in ('name', '', ' padded ', 'has.dot', 'bad\x00'):
                with self.subTest(name=name), self.assertRaises((ValueError, DocExError)):
                    page.fields.add(name, (30, 80, 100, 30))
            with self.assertRaises((ValueError, DocExError)):
                page.fields.add('choice', (30, 80, 100, 30), kind='choice', options=['Same', 'Same'])
            with self.assertRaises(ValueError):
                page.fields.add('huge', (30, 80, 100, 30), kind='choice', options=['é' * 2049])
            self.assertEqual(field.value, 'Valid')
            iterator = iter(document.fields)
            self.assertEqual(next(iterator).name, 'name')
            del document.fields['name']
            page.fields.add('name', (30, 30, 100, 30))
            with self.assertRaises(ValueError):
                _ = field.value
            self.assertEqual(document.fields['name'].value, '')
            with self.assertRaises(KeyError):
                _ = document.fields['missing']

    def test_field_deletion_invalidates_shifted_annotation_handles(self):
        with doc_ex.new('pdf') as document:
            page = document.pages[0]
            page.fields.add('first', (20, 20, 100, 30))
            first = page.annotations.add('note', box=(20, 80, 20, 20), text='First comment')
            second = page.annotations.add('note', box=(20, 120, 20, 20), text='Second comment')
            del document.fields['first']
            for old in (first, second):
                with self.assertRaises(ValueError):
                    _ = old.text
                with self.assertRaises(ValueError):
                    old.text = 'Must not overwrite a shifted comment'
            self.assertEqual([item.text for item in page.annotations], ['First comment', 'Second comment'])

    def test_fields_follow_page_organization(self):
        with doc_ex.new('pdf') as document:
            document.pages.add()
            field = document.pages[0].fields.add('first', (20, 20, 100, 30))
            field.value = 'First'
            document.pages.move(0, 1)
            with self.assertRaises(ValueError):
                _ = field.value
            self.assertEqual(document.fields['first'].page.number, 1)
            self.assertEqual(len(document.pages[0].fields), 0)
            with self.assertRaises((ValueError, DocExError)):
                document.copy(pages=[1])
            del document.pages[1]
            self.assertEqual(len(document.fields), 0)


if __name__ == '__main__':
    unittest.main()
