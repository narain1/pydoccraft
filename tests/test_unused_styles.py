from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsafeArchiveError
from test_picture_media import red_picture

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def styles(data):
    with ZipFile(BytesIO(data)) as archive:
        return {s.get(W + 'styleId'): ET.tostring(s)
                for s in ET.fromstring(archive.read('word/styles.xml')).findall(W + 'style')}


class UnusedStyleTests(unittest.TestCase):
    def unchanged_failure(self, document, operation, error=ValueError):
        before = document.to_bytes()
        document.page_count
        cached = document._cached
        revision = document._call('revision')
        with self.assertRaises(error):
            operation()
        self.assertEqual(document.to_bytes(), before)
        self.assertEqual(document._call('revision'), revision)
        self.assertIs(document._cached, cached)

    def test_unused_three_kinds_preserve_docx_png_pdf_and_live_handles(self):
        with doc_ex.new('docx', size=(360, 480)) as document:
            document.sections[0].margins = (48, 36, 48, 36)
            kept = document.styles.add('Retained', based_on='Normal')
            kept.format(color='navy', space_after=6)
            body = document.add_paragraph('Body retained', style=kept)
            linked = document.add_paragraph('External retained')
            linked[:].link = 'https://example.com/unused-style'
            table = document.add_table([['Table retained']])
            table[0, 0].add_table([['Nested retained']])
            picture = table[0, 0].add_image(red_picture(4, 2), width=16, height=8)
            for kind in ('header', 'footer'):
                story = getattr(document.sections[0], kind)()
                story.add_paragraph(kind + ' retained', style=kept)
            note = document.notes.add(body[:], 'Note retained')
            comment = document.comments.add(linked[:], 'Comment retained', author='Ada')
            drops = [document.styles.add('Unused ' + kind, kind=kind)
                     for kind in ('paragraph', 'character', 'table')]
            ids = [s.id for s in drops]
            before = document.to_bytes()
            original_styles = styles(before)
            before_png = [document.render_page(0, dpi=dpi) for dpi in (72, 144)]
            with doc_ex.open(document.convert('pdf')) as pdf:
                before_pdf_png = [pdf.render_page(0, dpi=dpi) for dpi in (72, 144)]
                self.assertIn('Body retained', pdf[0].get_text())
                self.assertTrue(any(link.uri == 'https://example.com/unused-style' for link in pdf[0].get_links()))
            old_range = linked[:]
            cached = document._cached
            revision = document._call('revision')
            del document.styles[drops[0].name]
            del document.styles[ids[1]]
            del document.styles[-1]
            self.assertEqual(document._call('revision'), revision + 3)
            self.assertIsNone(document._cached)
            with self.assertRaises(DocumentClosedError):
                cached.load_page(0)
            with self.assertRaisesRegex(ValueError, 'stale'):
                _ = old_range.text
            self.assertEqual(kept.name, 'Retained')
            self.assertEqual(body.style, 'Retained')
            self.assertEqual(picture.data, red_picture(4, 2))
            self.assertEqual(note.paragraphs[0].text, 'Note retained')
            self.assertEqual(comment.text, 'Comment retained')
            for identifier in ids:
                original_styles.pop(identifier)
            self.assertEqual(styles(document.to_bytes()), original_styles)
            self.assertEqual([document.render_page(0, dpi=dpi) for dpi in (72, 144)], before_png)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn('Body retained', reopened[0].get_text())
                self.assertIn('Table retained', reopened.to_markdown())
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual([pdf.render_page(0, dpi=dpi) for dpi in (72, 144)], before_pdf_png)

    def test_incoming_public_references_and_protected_styles_refuse_atomically(self):
        for scope in ('body', 'header', 'footer', 'cell', 'nested', 'footnote', 'endnote'):
            with self.subTest(scope=scope), doc_ex.new('docx') as document:
                base = document.add_paragraph('Anchor')
                target = document.styles.add('Referenced')
                if scope == 'body':
                    paragraph = document.add_paragraph('Use', style=target)
                elif scope in ('header', 'footer'):
                    paragraph = getattr(document.sections[0], scope)().add_paragraph('Use', style=target)
                elif scope in ('cell', 'nested'):
                    cell = document.add_table([['Cell']])[0, 0]
                    if scope == 'nested':
                        cell = cell.add_table([['Nested']])[0, 0]
                    paragraph = cell.add_paragraph('Use', style=target)
                elif scope in ('footnote', 'endnote'):
                    note = document.notes.add(base[:], 'Use', kind=scope)
                    paragraph = note.paragraphs[0]
                    paragraph.style = target
                self.unchanged_failure(document, lambda: document.styles.__delitem__(target.id))
                paragraph.style = 'Normal'
                del document.styles[target.id]
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Characters ')
            target = document.styles.add('Characters', kind='character')
            selected = paragraph.add_run('styled', style=target.id)
            self.unchanged_failure(document, lambda: document.styles.__delitem__(target.id))
            selected.format(style=None)
            del document.styles[target.id]
            table_style = document.styles.add('Table use', kind='table')
            table = document.add_table([['Cell']])
            table.format(style=table_style)
            self.unchanged_failure(document, lambda: document.styles.__delitem__(table_style.id))
            table.format(style=None)
            del document.styles[table_style.id]
            for relation in ('based_on', 'next', 'linked'):
                target = document.styles.add('Target ' + relation, kind='character' if relation == 'linked' else 'paragraph')
                holder = document.styles.add('Holder ' + relation)
                setattr(holder, relation, target)
                self.unchanged_failure(document, lambda: document.styles.__delitem__(target.id))
                setattr(holder, relation, None)
                del document.styles[target.id]
            for protected in ('Normal', 'DefaultParagraphFont', 'TableNormal'):
                self.unchanged_failure(document, lambda: document.styles.__delitem__(protected))

    def test_deleted_handles_never_alias_recreated_names_or_copy_histories(self):
        with doc_ex.new('docx') as document:
            document.add_paragraph('Keep')
            old = document.styles.add('Old Name')
            old_id = old.id
            del document.styles[old_id]
            recreated = document.styles.add('Old Name')
            alias = document.styles.add(old_id)
            self.assertNotEqual(recreated.id, old_id)
            self.assertNotEqual(alias.id, old_id)
            self.assertEqual(document.styles[old_id].id, alias.id)
            for attr in ('id', 'name', 'kind', 'builtin', 'based_on', 'next', 'linked', 'formatting', 'resolved_formatting'):
                with self.subTest(attr=attr):
                    with self.assertRaisesRegex(ValueError, 'deleted'):
                        getattr(old, attr)
            for operation in (lambda: old.format(bold=True), lambda: setattr(old, 'based_on', None),
                              lambda: setattr(old, 'next', None), lambda: setattr(old, 'linked', None),
                              lambda: document.add_paragraph('No', style=old),
                              lambda: document.styles.add('No', based_on=old)):
                self.unchanged_failure(document, operation)
            self.assertIn(old_id, repr(old))
            with document.copy() as copied:
                del copied.styles[recreated.id]
                again = copied.styles.add('Old Name')
                self.assertNotIn(again.id, (old_id, recreated.id))
                self.assertEqual(recreated.name, 'Old Name')
                with self.assertRaises(ValueError):
                    copied.add_paragraph('No', style=recreated)
            document.close()
            for operation in (lambda: old.name, lambda: document.styles.__delitem__(old_id)):
                with self.assertRaises(DocumentClosedError):
                    operation()

    def test_callback_deletion_recreation_cannot_retarget_any_style_consumer(self):
        consumers = ('add_style', 'body', 'header', 'cell', 'paragraph', 'table', 'base', 'next', 'linked')
        for consumer in consumers:
            with self.subTest(consumer=consumer), doc_ex.new('docx') as document:
                kind = 'table' if consumer == 'table' else 'character' if consumer == 'linked' else 'paragraph'
                reference = document.styles.add('Reference', kind=kind)
                identifier = reference.id
                paragraph = document.add_paragraph('Keep')
                table = document.add_table([['Keep cell']])
                target = document.styles.add('Target')
                state = {}
                def mutate():
                    if state:
                        return
                    del document.styles[identifier]
                    document.styles.add(identifier, kind=kind)
                    state['bytes'] = document.to_bytes()
                    state['revision'] = document._call('revision')
                class CallbackReference(type(reference)):
                    def __init__(self):
                        self._owner = document
                    @property
                    def _id(self):
                        mutate()
                        return identifier
                value = CallbackReference()
                operations = {
                    'add_style': lambda: document.styles.add('Forbidden', based_on=value),
                    'body': lambda: document.add_paragraph('Forbidden', style=value),
                    'header': lambda: document.sections[0].header().add_paragraph('Forbidden', style=value),
                    'cell': lambda: table[0, 0].add_paragraph('Forbidden', style=value),
                    'paragraph': lambda: setattr(paragraph, 'style', value),
                    'table': lambda: table.format(style=value),
                    'base': lambda: setattr(target, 'based_on', value),
                    'next': lambda: setattr(target, 'next', value),
                    'linked': lambda: setattr(target, 'linked', value),
                }
                with self.assertRaises(ValueError):
                    operations[consumer]()
                self.assertTrue(state)
                self.assertEqual(document.to_bytes(), state['bytes'])
                self.assertEqual(document._call('revision'), state['revision'])
        for consumer in ('style', 'table', 'index'):
            with self.subTest(consumer=consumer), doc_ex.new('docx') as document:
                target = document.styles.add('Target', kind='table')
                identifier = target.id
                table = document.add_table([['Keep']])
                state = {}
                def mutate():
                    if state:
                        return
                    del document.styles[identifier]
                    document.styles.add('Target', kind='table')
                    state['bytes'] = document.to_bytes()
                    state['revision'] = document._call('revision')
                class Number(float):
                    def __float__(self):
                        mutate()
                        return 5.0
                class Index:
                    def __index__(self):
                        mutate()
                        return -1
                with self.assertRaisesRegex(ValueError, 'stale|deleted'):
                    if consumer == 'style':
                        target.format(margins=(Number(5), 5, 5, 5))
                    elif consumer == 'table':
                        table.format(style=target, indent=Number(5))
                    else:
                        del document.styles[Index()]
                self.assertEqual(document.to_bytes(), state['bytes'])
                self.assertEqual(document._call('revision'), state['revision'])

    def test_lifetime_capacity_invalid_keys_and_recreation_bounds(self):
        with doc_ex.new('docx') as document:
            document.add_paragraph('Keep')
            for key, error in ((slice(None), TypeError), (1.5, TypeError), (10_000, IndexError), ('Missing', ValueError)):
                self.unchanged_failure(document, lambda: document.styles.__delitem__(key), error)
            available = 1024 - len(document.styles)
            for i in range(available):
                style = document.styles.add(f'Lifetime {i}')
                del document.styles[style.id]
            self.assertLess(len(document.styles), 1024)
            self.unchanged_failure(document, lambda: document.styles.add('Past capacity'), UnsafeArchiveError)
            with document.copy() as copied:
                self.unchanged_failure(copied, lambda: copied.styles.add('Past capacity'), UnsafeArchiveError)


if __name__ == '__main__':
    unittest.main()
