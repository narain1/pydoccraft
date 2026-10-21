"""Owned DOCX table deletion uses existing collections and preserves surviving identities."""
from io import BytesIO
from xml.etree import ElementTree as ET
from zipfile import ZipFile
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import count_color, media_parts, red_picture
from test_story_clear import section_xml

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def container(document, scope):
    return document if scope == 'body' else getattr(document.sections[0], scope)()


def native_blocks(document, scope):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        name = 'document' if scope == 'body' else scope + '1'
        root = ET.fromstring(archive.read('word/' + name + '.xml'))
        return root.find(W + 'body') if scope == 'body' else root


class TableDeletionTests(unittest.TestCase):
    def test_body_and_stories_delete_subtrees_keep_handles_copy_and_visible_output(self):
        for scope in ('body', 'header', 'footer'):
            with self.subTest(scope=scope), doc_ex.new('docx') as document:
                document.sections[0].margins = (180, 36, 180, 36)
                document.add_paragraph('BodyKeep')
                owner = container(document, scope)
                removed = owner.add_table([['TableDrop']])
                removed_cell = removed[0, 0]
                removed_row = removed.row(0)
                removed_cell.background = 'blue'
                removed_paragraph = removed_cell.paragraphs[0]
                removed_paragraph.add_field('page')
                deep = removed_cell.add_table([['DeepDrop']])
                deep_paragraph = deep[0, 0].paragraphs[0]
                picture = deep[0, 0].add_image(red_picture(2, 2), width=12)
                kept = owner.add_table([['TableKeep']])
                kept_cell = kept[0, 0]
                kept_paragraph = kept_cell.paragraphs[0]
                kept_nested = kept_cell.add_table([['DeepKeep']])
                kept_deep = kept_nested[0, 0].paragraphs[0]
                kept_image = kept_nested[0, 0].add_image(red_picture(2, 2), width=12)
                before = document.render_page(0, dpi=72)
                self.assertGreater(count_color(before, b'\0\0\xff\xff'), 100)
                assets = media_parts(document)
                with document.copy() as copied:
                    selected = kept_paragraph[:]
                    del owner.tables[0]
                    self.assertEqual(len(owner.tables), 1)
                    for getter in (lambda: removed.rows, lambda: removed_cell.text, lambda: removed_row.height,
                                   lambda: removed_paragraph.text, lambda: deep.rows,
                                   lambda: deep_paragraph.text, lambda: picture.data):
                        with self.assertRaises((IndexError, ValueError)):
                            getter()
                    with self.assertRaises(ValueError):
                        _ = selected.text
                    self.assertEqual(kept_paragraph.text, 'TableKeep')
                    self.assertIn('DeepKeep', kept_cell.text)
                    self.assertEqual(kept_deep.text, 'DeepKeep')
                    self.assertEqual(kept_image.data, red_picture(2, 2))
                    self.assertEqual(media_parts(document), assets)
                    self.assertEqual(copied.render_page(0, dpi=72), before)
                    kept_deep.text = 'DeepUpdated'
                    for dpi in (72, 144):
                        png = document.render_page(0, dpi=dpi)
                        self.assertEqual(count_color(png, b'\0\0\xff\xff'), 0)
                        self.assertGreater(count_color(png, b'\xff\0\0\xff'), 100)
                    for data in (document.to_bytes(), document.convert('pdf')):
                        with doc_ex.open(data) as reopened:
                            text = reopened.to_markdown() + ' '.join(story.get_text() for story in reopened.stories)
                            self.assertNotIn('TableDrop', text)
                            self.assertNotIn('DeepDrop', text)
                            self.assertIn('TableKeep', text)
                            self.assertIn('DeepUpdated', text)
                            self.assertGreater(count_color(reopened.render_page(0, dpi=72), b'\xff\0\0\xff'), 100)

    def test_nested_merged_aliases_preserve_parent_grid_and_later_comments(self):
        for scope in ('body', 'header', 'footer'):
            for end in ((0, 1), (1, 0)):
                with self.subTest(scope=scope, end=end), doc_ex.new('docx') as document:
                    document.sections[0].margins = (180, 36, 180, 36)
                    document.add_paragraph('BodyKeep')
                    outer = container(document, scope).add_table([['Anchor', 'Right'], ['Below', 'Sibling']])
                    anchor = outer.merge((0, 0), end)
                    covered = outer[end]
                    anchor.text = 'Anchor'
                    removed = anchor.add_table([['Drop']])
                    kept = anchor.add_table([['Keep']])
                    text = kept[0, 0].paragraphs[0]
                    paragraph = anchor.add_paragraph('Later 😀')
                    comment = document.comments.add(paragraph[:], 'Review') if scope == 'body' else None
                    row = outer.row(0)
                    before_rows = len(outer.rows)
                    del covered.tables[0]
                    self.assertEqual(len(outer.rows), before_rows)
                    self.assertEqual(len(anchor.tables), 1)
                    self.assertEqual(text.text, 'Keep')
                    self.assertEqual(covered.tables[0].rows, kept.rows)
                    row.allow_split = False
                    if comment is not None:
                        self.assertEqual(comment.range.text, 'Later 😀')
                        comment.range.text = 'Still anchored 😀'
                    else:
                        self.assertEqual(paragraph.text, 'Later 😀')
                    with self.assertRaises(IndexError):
                        _ = removed.rows
                    document.to_bytes()

    def test_required_blank_reuse_and_authored_empty_identity(self):
        for scope in ('body', 'header', 'footer'):
            for authored in (False, True):
                with self.subTest(scope=scope, authored=authored), doc_ex.new('docx') as document:
                    owner = container(document, scope)
                    target = owner.add_table([['Drop']])
                    empty = owner.add_paragraph('') if authored else None
                    del owner.tables[-1]
                    self.assertEqual(len(owner.tables), 0)
                    self.assertEqual(len(owner.paragraphs), int(authored))
                    if empty is not None:
                        self.assertEqual(empty.text, '')
                    count = len(native_blocks(document, scope))
                    fresh = owner.add_paragraph('Fresh')
                    self.assertEqual(len(native_blocks(document, scope)), count + int(authored))
                    self.assertGreater(fresh._id, target._id)
                    self.assertEqual(fresh.text, 'Fresh')
                    another = owner.add_table([['Another']])
                    del owner.tables[0]
                    self.assertEqual(fresh.text, 'Fresh')
                    with self.assertRaises(IndexError):
                        _ = another.rows

    def test_reference_guards_rollback_and_explicit_comment_removal(self):
        for kind in ('bookmark', 'note', 'comment'):
            with self.subTest(kind=kind), doc_ex.new('docx') as document:
                table = document.add_table([['Protected']])
                paragraph = table[0, 0].paragraphs[0]
                if kind == 'bookmark':
                    document.bookmarks['Keep'] = paragraph[:]
                elif kind == 'note':
                    document.notes.add(paragraph[:0], 'Detail')
                else:
                    root = document.comments.add(paragraph[:], 'Review')
                    document.comments.add(root, 'Reply')
                document.add_paragraph('TailKeep')
                before = document.to_bytes()
                png = document.render_page(0, dpi=72)
                cached = document._cached
                held = document.paragraphs[-1][:]
                error = ValueError if kind == 'comment' else UnsupportedFormatError
                with self.assertRaises(error):
                    del document.tables[0]
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(held.text, 'TailKeep')
                self.assertIs(document._cached, cached)
                self.assertEqual(document.render_page(0, dpi=72), png)
                if kind == 'comment':
                    del document.comments[-1]
                    del document.comments[0]
                    del document.tables[0]
                    self.assertEqual(len(document.tables), 0)
        with doc_ex.new('docx') as document:
            first = document.add_table([['Drop']])
            document.sections.add(start='next_page')
            document.add_paragraph('Second section')
            before = section_xml(document)
            del document.tables[0]
            self.assertEqual(section_xml(document), before)
            self.assertEqual(len(document.sections), 2)
            with self.assertRaises(IndexError):
                _ = first.rows

    def test_shared_background_media_reclaims_only_final_reference(self):
        with doc_ex.new('docx') as document:
            image = red_picture(2, 2)
            document.background_image = image
            table = document.add_table([['Drop']])
            picture = table[0, 0].add_image(image, width=12)
            del document.tables[0]
            self.assertEqual(media_parts(document), [image])
            self.assertEqual(document.background_image, image)
            with self.assertRaises(IndexError):
                _ = picture.data
            document.background_image = None
            self.assertEqual(media_parts(document), [])
            table = document.add_table([['Drop unique']])
            table[0, 0].add_image(image, width=12)
            del document.tables[0]
            self.assertEqual(media_parts(document), [])

    def test_index_callbacks_stale_cells_slices_and_closed_handles(self):
        with doc_ex.new('docx') as document:
            first = document.add_table([['First']])
            second = document.add_table([['Second']])
            third = document.add_table([['Third']])
            class Earlier:
                def __index__(self):
                    del document.tables[0]
                    return 1
            del document.tables[Earlier()]
            self.assertEqual(document.tables[0].rows, third.rows)
            for deleted in (first, second):
                with self.assertRaises(IndexError):
                    _ = deleted.rows
            class Replace:
                def __index__(self):
                    del document.tables[0]
                    document.add_table([['Replacement']])
                    return 0
            with self.assertRaises(IndexError):
                del document.tables[Replace()]
            self.assertEqual(document.tables[0].rows, [['Replacement']])
            with self.assertRaises(TypeError):
                del document.tables[:]
            with self.assertRaises(IndexError):
                del document.tables[4]
            table = document.tables[0]
            cell = table[0, 0]
            nested = cell.add_table([['Nested']])
            collection = cell.tables
            table.insert_row(0)
            class MustNotRun:
                def __index__(self):
                    raise AssertionError('stale cell must fail first')
            with self.assertRaises(ValueError):
                del collection[MustNotRun()]
            with self.assertRaises(IndexError):
                _ = nested.rows
        document = doc_ex.new('docx')
        document.add_table([['Close']])
        class Close:
            def __index__(self):
                document.close()
                return 0
        with self.assertRaises(DocumentClosedError):
            del document.tables[Close()]
        with self.assertRaises(DocumentClosedError):
            len(document.tables)
