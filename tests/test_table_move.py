"""Same-parent DOCX table movement preserves held subtree identities."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import media_parts, red_picture
from test_note_conversion import red_rows
from test_table_delete import container


class TableMovementTests(unittest.TestCase):
    def test_body_and_stories_move_visible_content_media_fields_and_held_handles(self):
        for scope in ('body', 'header', 'footer'):
            with self.subTest(scope=scope), doc_ex.new('docx') as document:
                document.sections[0].margins = (180, 36, 180, 36)
                document.add_paragraph('Body kept')
                owner = container(document, scope)
                first = owner.add_table([['First table']])
                owner.add_paragraph('Between kept')
                last = owner.add_table([['Moved table']])
                cell, row = last[0, 0], last.row(0)
                cell.background = 'red'
                paragraph = cell.paragraphs[0]
                paragraph[:].link = 'https://example.com/moved'
                paragraph.add_field('page_count')
                nested = cell.add_table([['Nested kept']])
                deep = nested[0, 0].paragraphs[0]
                picture = nested[0, 0].add_image(red_picture(2, 2), width=8)
                before = document.render_page(0, dpi=72)
                assets = media_parts(document)
                with document.copy() as copied:
                    held = paragraph[:]
                    owner.tables.move(-1, 0)
                    self.assertEqual([t[0, 0].paragraphs[0].text for t in owner.tables],
                                     ['Moved table\ufffc', 'First table'])
                    with self.assertRaises(ValueError):
                        _ = held.text
                    self.assertEqual(first[0, 0].text, 'First table')
                    self.assertEqual(paragraph[:11].link, 'https://example.com/moved')
                    self.assertEqual(deep.text, 'Nested kept')
                    self.assertEqual(picture.data, red_picture(2, 2))
                    self.assertEqual(cell.background, '#FF0000')
                    _ = row.height
                    self.assertEqual(media_parts(document), assets)
                    self.assertEqual(copied.render_page(0, dpi=72), before)
                    self.assertLess(min(red_rows(document.render_page(0, dpi=72))), min(red_rows(before)))
                    deep.text = 'Nested updated'
                    for data in (document.to_bytes(), document.convert('pdf')):
                        with doc_ex.open(data) as reopened:
                            text = reopened.to_markdown() + ' '.join(s.get_text() for s in reopened.stories)
                            for label in ('Moved table', 'First table', 'Between kept', 'Nested updated'):
                                self.assertEqual(text.count(label), 1)
                            self.assertLess(text.index('Moved table'), text.index('First table'))
                            for dpi in (72, 144):
                                self.assertTrue(red_rows(reopened.render_page(0, dpi=dpi)))

    def test_nested_merged_aliases_keep_generations_empty_paragraphs_and_comment_ranges(self):
        for scope in ('body', 'header', 'footer'):
            for end in ((0, 1), (1, 0)):
                with self.subTest(scope=scope, end=end), doc_ex.new('docx') as document:
                    outer = container(document, scope).add_table([['Anchor', 'Right'], ['Below', 'Sibling']])
                    anchor = outer.merge((0, 0), end)
                    covered = outer[end]
                    first = anchor.add_table([['First']])
                    deep = first[0, 0].add_table([['Deep']])
                    held = deep[0, 0].paragraphs[0]
                    last = anchor.add_table([['Later comment']])
                    blank = anchor.add_paragraph('')
                    paragraph = last[0, 0].paragraphs[0]
                    comment = document.comments.add(paragraph[:], 'Review') if scope == 'body' else None
                    reply = document.comments.add(comment, 'Reply') if comment else None
                    covered.tables.move(0, -1)
                    self.assertEqual([t[0, 0].paragraphs[0].text for t in anchor.tables],
                                     [('\ufffcLater comment\ufffc' if comment else 'Later comment'), 'First'])
                    self.assertEqual((held.text, blank.text), ('Deep', ''))
                    held.text = 'Deep edited'
                    self.assertEqual(deep[0, 0].text, 'Deep edited')
                    if reply:
                        self.assertEqual(reply.range.text, 'Later comment')
                    fresh = covered.add_paragraph('Fresh')
                    self.assertEqual(fresh.text, 'Fresh')
                    self.assertEqual(blank.text, '')
                    with doc_ex.open(document.to_bytes()) as reopened:
                        self.assertIn('Deep edited', reopened.to_markdown() + ' '.join(s.get_text() for s in reopened.stories))

    def test_noop_keeps_cache_ranges_and_invalid_selections_or_sections_roll_back(self):
        with doc_ex.new('docx') as document:
            first = document.add_table([['First']])
            document.add_table([['Last']])
            held = first[0, 0].paragraphs[0][:]
            document.render_page(0, dpi=72)
            cache, revision, before = document._cached, document._call('revision'), document.to_bytes()
            document.tables.move(0, -2)
            self.assertIs(document._cached, cache)
            self.assertEqual(document._call('revision'), revision)
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(held.text, 'First')
            for pair in ((2, 0), (-3, 0), (0, 2), (slice(None), 0), (0, 1.5)):
                with self.assertRaises((TypeError, IndexError)):
                    document.tables.move(*pair)
                self.assertEqual(document.to_bytes(), before)
            document.sections.add()
            document.add_table([['Second section']])
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsupportedFormatError, 'section'):
                document.tables.move(0, -1)
            self.assertEqual(document.to_bytes(), before)
        with self.assertRaises(DocumentClosedError):
            document.tables.move(0, 0)

    def test_callbacks_cannot_move_changed_owner_and_stale_cells_reject_before_callback(self):
        class Index:
            def __init__(self, action, value):
                self.action, self.value = action, value
            def __index__(self):
                self.action()
                return self.value
        for position in (0, 1):
            for action in ('append', 'delete', 'merge', 'close'):
                with self.subTest(position=position, action=action), doc_ex.new('docx') as document:
                    outer = document.add_table([['Anchor', 'Sibling']])
                    cell = outer[0, 0]
                    cell.add_table([['First']])
                    cell.add_table([['Last']])
                    methods = {'append': lambda: cell.add_table([['Callback']]),
                               'delete': lambda: cell.tables.__delitem__(0),
                               'merge': lambda: outer.merge((0, 0), (0, 1)),
                               'close': document.close}
                    args = [0, 1]
                    args[position] = Index(methods[action], args[position])
                    with self.assertRaises((ValueError, DocumentClosedError)):
                        cell.tables.move(*args)
                    if action == 'append':
                        self.assertEqual([t[0, 0].text for t in cell.tables], ['First', 'Last', 'Callback'])
                    elif action == 'delete':
                        self.assertEqual([t[0, 0].text for t in cell.tables], ['Last'])
                    elif action == 'merge':
                        self.assertEqual([t[0, 0].text for t in outer[0, 0].tables], ['First', 'Last'])
        with doc_ex.new('docx') as document:
            outer = document.add_table([['Anchor', 'Sibling']])
            cell = outer[0, 0]
            cell.add_table([['First']])
            tables = cell.tables
            outer.merge((0, 0), (0, 1))
            calls = []
            with self.assertRaises(ValueError):
                tables.move(Index(lambda: calls.append(True), 0), 0)
            self.assertEqual(calls, [])

    def test_source_anchors_refuse_even_noop_and_removable_anchors_can_be_cleared(self):
        for kind in ('bookmark', 'comment', 'note'):
            with self.subTest(kind=kind), doc_ex.new('docx') as document:
                first = document.add_table([['First']])
                document.add_table([['Last']])
                paragraph = first[0, 0].paragraphs[0]
                if kind == 'bookmark':
                    document.bookmarks['keep'] = paragraph[:]
                elif kind == 'comment':
                    comment = document.comments.add(paragraph[:], 'Review')
                else:
                    note = document.notes.add(paragraph[:])
                before, revision = document.to_bytes(), document._call('revision')
                for destination in (0, 1):
                    with self.assertRaisesRegex(UnsupportedFormatError, 'anchors'):
                        document.tables.move(0, destination)
                    self.assertEqual((document.to_bytes(), document._call('revision')), (before, revision))
                if kind == 'bookmark':
                    del document.bookmarks['keep']
                elif kind == 'comment':
                    del document.comments[0]
                else:
                    note.clear()  # Clearing note content retains its protected reference.
                    with self.assertRaisesRegex(UnsupportedFormatError, 'anchors'):
                        document.tables.move(0, 1)
                    self.assertEqual(len(document.notes), 1)
                    continue
                document.tables.move(0, 1)
                self.assertEqual(document.tables[1][0, 0].text, 'First')

    def test_supported_count_fields_follow_moved_table_across_page_break(self):
        for kind in ('page_count', 'section_page_count'):
            with self.subTest(kind=kind), doc_ex.new('docx') as document:
                first = document.add_table([['Count value ']])
                first[0, 0].paragraphs[0].add_field(kind)
                document.add_page_break()
                document.add_table([['Later table']])
                document.tables.move(0, 1)
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        self.assertEqual(reopened.page_count, 2)
                        self.assertIn('Count value', reopened[1].get_text())
                        self.assertNotIn('Count value', reopened[0].get_text())
                        for dpi in (72, 144):
                            self.assertGreater(len(reopened.render_page(1, dpi=dpi)), 1000)
                with doc_ex.open(document.convert('pdf')) as pdf:
                    self.assertRegex(' '.join(pdf[1].get_text().split()), r'Count value\s*2')
