"""Same-cell paragraph movement preserves native content, identities and required blanks."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_cell_paragraph_delete import cell_xml, owned_table, W
from test_picture_media import count_color, media_parts, red_picture
from test_note_conversion import red_rows


class CellParagraphMoveTests(unittest.TestCase):
    def test_owned_scopes_move_fields_links_and_nested_handles_with_rendered_order(self):
        for scope in ('body', 'header', 'footer'):
            with self.subTest(scope=scope), doc_ex.new('docx') as document:
                document.sections[0].margins = (180, 36, 180, 36)
                table = owned_table(document, scope, [['First ordinary', 'Sibling kept']])
                cell = table[0, 0]
                first = cell.paragraphs[0]
                nested = cell.add_table([['Deep kept']])
                deep = nested[0, 0].paragraphs[0]
                image = red_picture(2, 2)
                picture = nested[0, 0].add_image(image, width=12)
                last = cell.add_paragraph('Moved red')
                last[:].format(color='red')
                last[:].link = 'https://example.com/moved'
                last.add_field('page')
                before = document.render_page(0, dpi=144)
                assets = media_parts(document)
                with document.copy() as copied:
                    held = first[:]
                    cell.paragraphs.move(-1, 0)
                    self.assertEqual([p.text for p in cell.paragraphs], ['Moved red\ufffc', 'First ordinary'])
                    with self.assertRaises(ValueError):
                        _ = held.text
                    self.assertEqual((first.text, deep.text), ('First ordinary', 'Deep kept'))
                    self.assertEqual(last[:9].link, 'https://example.com/moved')
                    self.assertEqual(picture.data, image)
                    self.assertEqual(media_parts(document), assets)
                    self.assertEqual(copied.render_page(0, dpi=144), before)
                    self.assertLess(min(red_rows(document.render_page(0, dpi=144))), min(red_rows(before)))
                    native = cell_xml(document, scope)
                    self.assertEqual([b.tag for b in native if b.tag != W+'tcPr'], [W+'p', W+'p', W+'tbl', W+'p'])
                    self.assertEqual([n.text.strip() for n in native[1].iter(W+'instrText')], ['PAGE \\* Arabic'])
                    count = len(native)
                    fresh = cell.add_paragraph('Fresh tail')
                    self.assertEqual(len(cell_xml(document, scope)), count)
                    cell.paragraphs.move(-1, 1)
                    self.assertEqual([p.text for p in cell.paragraphs], ['Moved red\ufffc', 'Fresh tail', 'First ordinary'])
                    self.assertEqual(fresh.text, 'Fresh tail')
                    deep.text = 'Deep updated'
                    for dpi in (72, 144):
                        self.assertGreater(count_color(document.render_page(0, dpi=dpi), b'\xff\0\0\xff'), 50)
                    for data in (document.to_bytes(), document.convert('pdf')):
                        with doc_ex.open(data) as reopened:
                            text = reopened.to_markdown() + ' '.join(story.get_text() for story in reopened.stories)
                            for label in ('Moved red', 'Fresh tail', 'First ordinary', 'Deep updated', 'Sibling kept'):
                                self.assertEqual(text.count(label), 1)
                            self.assertLess(text.index('Moved red'), text.index('Fresh tail'))
                            self.assertLess(text.index('Fresh tail'), text.index('First ordinary'))
                            self.assertGreater(count_color(reopened.render_page(0, dpi=144), b'\xff\0\0\xff'), 100)
                    with doc_ex.open(document.convert('pdf')) as pdf:
                        self.assertRegex(' '.join(pdf[0].get_text().split()), r'Moved red\s*1')

    def test_merged_aliases_nested_cells_and_authored_empty_keep_independent_identity(self):
        for end in ((0, 1), (1, 0)):
            with self.subTest(end=end), doc_ex.new('docx') as document:
                table = document.add_table([['First', 'Other'], ['Below', 'Sibling']])
                anchor = table.merge((0, 0), end)
                anchor.text = 'First'
                covered = table[end]
                first = anchor.paragraphs[0]
                empty = anchor.add_paragraph('')
                nested = anchor.add_table([['Nested first']])
                nested_cell = nested[0, 0]
                nested_first = nested_cell.paragraphs[0]
                nested_last = nested_cell.add_paragraph('Nested last')
                sibling = table[1, 1].paragraphs[0]
                covered.paragraphs.move(-1, 0)
                self.assertEqual([p.text for p in anchor.paragraphs], ['', 'First'])
                self.assertEqual((first.text, empty.text, sibling.text), ('First', '', 'Sibling'))
                nested_cell.paragraphs.move(-1, 0)
                self.assertEqual([p.text for p in nested_cell.paragraphs], ['Nested last', 'Nested first'])
                nested_first.text = 'Nested edited'
                self.assertEqual(nested_last.text, 'Nested last')
                fresh = covered.add_paragraph('Fresh')
                self.assertEqual([p.text for p in anchor.paragraphs], ['', 'First', 'Fresh'])
                self.assertEqual(fresh.text, 'Fresh')
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertIn('Nested edited', reopened.to_markdown())

    def test_noop_preserves_bytes_revision_cache_ranges_and_rejects_unsupported(self):
        with doc_ex.new('docx') as document:
            cell = document.add_table([['Only']])[0, 0]
            paragraph = cell.paragraphs[0]
            held = paragraph[:]
            document.render_page(0, dpi=72)
            cache = document._cached
            before, revision = document.to_bytes(), document._call('revision')
            cell.paragraphs.move(0, -1)
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(document._call('revision'), revision)
            self.assertIs(document._cached, cache)
            self.assertEqual(held.text, 'Only')
            cell.add_paragraph('Other')
            document.bookmarks['anchor'] = paragraph[:]
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsupportedFormatError, 'anchors'):
                cell.paragraphs.move(0, 1)
            self.assertEqual(document.to_bytes(), before)
            for pair in ((2, 0), (-3, 0), (0, 2), (slice(None), 0), (0, 1.5)):
                with self.assertRaises((IndexError, TypeError)):
                    cell.paragraphs.move(*pair)
                self.assertEqual(document.to_bytes(), before)
            del cell.paragraphs[1]
            with self.assertRaises((IndexError, ValueError)):
                _ = cell.paragraphs[1]
        with self.assertRaises(DocumentClosedError):
            cell.paragraphs.move(0, 0)
        with doc_ex.new('pptx') as presentation:
            cell = presentation.slides.add().add_table([['Text']], box=(20, 20, 200, 60))[0, 0]
            with self.assertRaises(UnsupportedFormatError):
                cell.paragraphs.move(0, 0)

    def test_callbacks_cannot_apply_pending_move_to_changed_cell_or_owner(self):
        class Index:
            def __init__(self, callback, value):
                self.callback, self.value = callback, value
            def __index__(self):
                self.callback()
                return self.value
        for position in (0, 1):
            for change in ('append', 'delete', 'close', 'merge'):
                with self.subTest(position=position, change=change), doc_ex.new('docx') as document:
                    table = document.add_table([['First', 'Sibling']])
                    cell = table[0, 0]
                    cell.add_paragraph('Last')
                    actions = {'append': lambda: cell.add_paragraph('Callback'),
                               'delete': lambda: cell.paragraphs.__delitem__(0),
                               'close': document.close,
                               'merge': lambda: table.merge((0, 0), (0, 1))}
                    args = [0, 1]
                    args[position] = Index(actions[change], args[position])
                    with self.assertRaises((ValueError, DocumentClosedError)):
                        cell.paragraphs.move(*args)
                    if change == 'append':
                        self.assertEqual([p.text for p in cell.paragraphs], ['First', 'Last', 'Callback'])
                    elif change == 'delete':
                        self.assertEqual([p.text for p in cell.paragraphs], ['Last'])
                    elif change == 'merge':
                        self.assertEqual([p.text for p in table[0, 0].paragraphs], ['First', 'Last', 'Sibling'])
