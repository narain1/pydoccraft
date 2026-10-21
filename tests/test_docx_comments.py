"""Owned whole-paragraph comments preserve native text and keep review scope explicit."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import red_picture, media_parts

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def comments_xml(document):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        return ET.fromstring(archive.read('word/comments.xml'))


class OwnedCommentTests(unittest.TestCase):
    def test_crud_unicode_runs_replies_and_exact_separate_story_projection(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Whole 😀 e\u0301 office سلام')
            paragraph[:5].format(bold=True, color='blue')
            selected = paragraph[:]
            comment = document.comments.add(selected, 'ReviewOnly 😀', author='Ada', date='2026-10-20T12:00:00Z')
            with self.assertRaises(ValueError):
                _ = selected.text
            self.assertEqual(comment.range.text, 'Whole 😀 e\u0301 office سلام')
            self.assertIsNone(comment.parent)
            reply = document.comments.add(comment, '', author='Ben')
            self.assertEqual(reply.parent.range.text, comment.range.text)
            self.assertEqual(reply.range.text, comment.range.text)
            reply.text = 'ReplyOnly\nSecond line'
            reply.resolved = True
            comment.author = 'Other'
            comment.date = ''
            comment.resolved = True
            self.assertEqual([c.text for c in document.comments], ['ReviewOnly 😀', 'ReplyOnly\nSecond line'])
            self.assertEqual((comment.author, comment.date, comment.resolved), ('Other', '', True))
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                body = ''.join(ET.fromstring(archive.read('word/document.xml')).itertext())
                self.assertNotIn('ReviewOnly', body)
                self.assertNotIn('ReplyOnly', body)
            with doc_ex.open(document.to_bytes()) as opened:
                stories = [story for story in opened.stories if story.kind == 'comment']
                self.assertEqual(len(stories), 2)
                self.assertEqual(opened.to_markdown().count('ReviewOnly'), 1)
                self.assertEqual(opened.to_markdown().count('ReplyOnly'), 1)
                self.assertTrue(all(story.metadata['legacy_page_projection'] for story in stories))
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                del document.comments[0]
            self.assertEqual(document.to_bytes(), before)
            del document.comments[-1]
            del document.comments[0]
            self.assertEqual(paragraph.text, 'Whole 😀 e\u0301 office سلام')
            self.assertEqual(len(document.comments), 0)
            for stale in (comment, reply):
                with self.assertRaises(IndexError):
                    _ = stale.text

    def test_default_preview_shading_and_pdf_omit_comment_text_and_markup(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Visible whole target')
            plain = {dpi: document.render_page(0, dpi=dpi) for dpi in (72, 144)}
            unmarked_pdf = document.convert('pdf')
            comment = document.comments.add(paragraph[:], 'ReviewOnly')
            reply = document.comments.add(comment, 'ReplyOnly')
            shaded = {dpi: document.render_page(0, dpi=dpi) for dpi in (72, 144)}
            for dpi in plain:
                self.assertNotEqual(shaded[dpi], plain[dpi])
            comment.resolved = reply.resolved = True
            for dpi in shaded:
                self.assertEqual(document.render_page(0, dpi=dpi), shaded[dpi])
            with doc_ex.open(document.to_bytes()) as reopened:
                for dpi in shaded:
                    self.assertEqual(reopened.render_page(0, dpi=dpi), shaded[dpi])
            with doc_ex.open(document.convert('pdf')) as exported, doc_ex.open(unmarked_pdf) as baseline:
                self.assertNotIn('ReviewOnly', exported.to_markdown())
                self.assertNotIn('ReplyOnly', exported.to_markdown())
                for dpi in shaded:
                    self.assertEqual(exported.render_page(0, dpi=dpi), baseline.render_page(0, dpi=dpi))
            del document.comments[1]
            del document.comments[0]
            for dpi in plain:
                self.assertEqual(document.render_page(0, dpi=dpi), plain[dpi])

    def test_cells_nested_merged_and_navigation_media_keep_handles_and_output(self):
        for scope in ('ordinary', 'nested', 'merged'):
            with self.subTest(scope=scope), doc_ex.new('docx') as document:
                target = document.add_paragraph('Link target')
                document.bookmarks['Target'] = target[:]
                jump = document.add_paragraph('Jump')
                jump[:].link = '#Target'
                image = document.add_image(red_picture(2, 2), width=12)
                table = document.add_table([['Cell 😀 office', 'Sibling']])
                if scope == 'nested':
                    cell = table[0, 0].add_table([['Cell 😀 office']])[0, 0]
                elif scope == 'merged':
                    cell = table.merge((0, 0), (0, 1))
                else:
                    cell = table[0, 0]
                paragraph = cell.paragraphs[0]
                before_pdf = document.convert('pdf')
                assets = media_parts(document)
                comment = document.comments.add(paragraph[:], 'ReviewOnly')
                with doc_ex.open(document.convert('pdf')) as exported, doc_ex.open(before_pdf) as baseline:
                    for dpi in (72, 144):
                        self.assertEqual(exported.render_page(0, dpi=dpi), baseline.render_page(0, dpi=dpi))
                    self.assertTrue(any(link.page == 0 for link in exported[0].get_links()))
                before = document.to_bytes()
                held_range = comment.range
                cached = document.render_page(0, dpi=72)
                for operation in (lambda: cell.format(text_direction='down'),
                                  lambda: setattr(table.row(0), 'height', (30, 'exact'))):
                    with self.assertRaises(UnsupportedFormatError):
                        operation()
                    self.assertEqual(document.to_bytes(), before)
                    self.assertEqual(held_range.text, comment.range.text)
                    self.assertEqual(document.render_page(0, dpi=72), cached)
                with self.assertRaisesRegex(ValueError, 'comment root is missing its exact marker pair'):
                    del cell.paragraphs[0]
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(held_range.text, comment.range.text)
                self.assertEqual(document.render_page(0, dpi=72), cached)
                del document.comments[0]
                self.assertEqual(media_parts(document), assets)
                self.assertEqual(image.data, red_picture(2, 2))
                self.assertEqual(document.bookmarks['Target'].text, 'Link target')
                self.assertIn('Cell', paragraph.text)
                with self.assertRaises(IndexError):
                    _ = comment.range

    def test_later_edits_scope_rejections_and_rollback_preserve_ranges(self):
        with doc_ex.new('docx') as document, doc_ex.new('docx') as foreign:
            paragraph = document.add_paragraph('Target office')
            empty = document.add_paragraph('')
            header = document.sections[0].header().add_paragraph('Header')
            foreign_p = foreign.add_paragraph('Elsewhere')
            for reference, exception in ((paragraph[1:4], UnsupportedFormatError), (empty[:], UnsupportedFormatError),
                                         (header[:], UnsupportedFormatError), (foreign_p[:], ValueError)):
                before = document.to_bytes()
                with self.assertRaises(exception):
                    document.comments.add(reference, 'Review')
                self.assertEqual(document.to_bytes(), before)
            comment = document.comments.add(paragraph[:], 'Review')
            selected = comment.range
            before = document.to_bytes()
            for operation in (lambda: paragraph.add_run('Outside'), lambda: setattr(paragraph[:0], 'text', 'Outside'),
                              lambda: setattr(comment.range, 'text', ''), lambda: document.comments.add(comment.range, 'Overlap'),
                              lambda: setattr(comment, 'author', ''), lambda: setattr(comment, 'date', '2026-02-30T12:00:00Z'),
                              lambda: setattr(comment, 'resolved', 1)):
                with self.assertRaises((UnsupportedFormatError, ValueError, TypeError)):
                    operation()
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(selected.text, 'Target office')
            reply = document.comments.add(comment, 'Reply')
            with self.assertRaises(UnsupportedFormatError):
                document.comments.add(reply, 'Nested')
            comment.range.text = 'Updated 😀'
            self.assertEqual(reply.range.text, 'Updated 😀')
            comment.range.format(bold=True)
            self.assertEqual(comment.range.text, 'Updated 😀')
            document.add_paragraph('Unrelated later paragraph')
            self.assertEqual(comment.range.text, 'Updated 😀')

    def test_copy_deleted_identity_part_reuse_closed_and_index_callbacks(self):
        document = doc_ex.new('docx')
        paragraph = document.add_paragraph('Target')
        comment = document.comments.add(paragraph[:], 'Original')
        with document.copy() as copied:
            del document.comments[0]
            document.sections[0].header().add_paragraph('Header reuse')
            new = document.comments.add(paragraph[:], 'New')
            self.assertEqual(copied.comments[0].text, 'Original')
            self.assertEqual(new.text, 'New')
            with self.assertRaises(IndexError):
                comment.text = 'Stale edit'
            with self.assertRaises(ValueError):
                document.comments.add(copied.comments[0], 'Foreign')
            class MutateIndex:
                def __index__(self):
                    del document.comments[0]
                    document.comments.add(paragraph[:], 'Replacement')
                    return 0
            with self.assertRaises(IndexError):
                del document.comments[MutateIndex()]
            self.assertEqual(document.comments[0].text, 'Replacement')
        held = document.comments[0]
        class CloseIndex:
            def __index__(self):
                document.close()
                return 0
        with self.assertRaises(DocumentClosedError):
            del document.comments[CloseIndex()]
        for operation in (lambda: held.text, lambda: held.parent, lambda: held.range,
                          lambda: setattr(held, 'author', 'After close'), lambda: len(document.comments)):
            with self.assertRaises(DocumentClosedError):
                operation()

    def test_text_author_limits_and_canonical_durable_ids_survive_deletion(self):
        with doc_ex.new('docx') as document:
            p = document.add_paragraph('First')
            q = document.add_paragraph('Second')
            first = document.comments.add(p[:], '', author='A' * 1024)
            second = document.comments.add(q[:], 'x' * (1024 * 1024))
            before = document.to_bytes()
            for operation in (lambda: setattr(first, 'author', 'A' * 1025),
                              lambda: setattr(second, 'text', 'x' * (1024 * 1024 + 1)),
                              lambda: setattr(first, 'author', ' line '),
                              lambda: setattr(first, 'text', '\ufffc')):
                with self.assertRaises(ValueError):
                    operation()
                self.assertEqual(document.to_bytes(), before)
            def durable_ids():
                with ZipFile(BytesIO(document.to_bytes())) as archive:
                    return [element.attrib for element in ET.fromstring(archive.read('word/commentsIds.xml'))]
            original = list(durable_ids()[-1].values())[-1]
            del document.comments[0]
            self.assertEqual(list(durable_ids()[0].values())[-1], original)
            self.assertEqual(second.author, 'Unknown')
            self.assertEqual(document.comments[0].text, 'x' * (1024 * 1024))
