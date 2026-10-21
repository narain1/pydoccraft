from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_note_clear import note_xml
from test_note_conversion import pdf_text_pages, red_rows
from test_picture_media import count_color, media_parts, red_picture

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


class StoryParagraphDeletionTests(unittest.TestCase):
    def test_header_footer_delete_moves_native_blocks_keeps_nested_handles_and_pixels(self):
        for kind in ('header', 'footer'):
            with self.subTest(kind=kind), doc_ex.new('docx') as document:
                body = document.add_paragraph('Body kept')
                story = getattr(document.sections[0], kind)()
                first = story.add_paragraph('REMOVE FIRST')
                first[:].format(color='red')
                table = story.add_table([['Outer kept']])
                cell = table[0, 0]
                outer = cell.paragraphs[0]
                nested = cell.add_table([['Nested kept']])
                deep = nested[0, 0].paragraphs[0]
                pic = nested[0, 0].add_image(red_picture(2, 2), width=10)
                direct_pic = story.add_image(red_picture(2, 2), width=12)
                middle = story.add_paragraph('REMOVE MIDDLE')
                middle[:].format(color='red')
                last = story.add_paragraph('Last kept')
                raw_media = media_parts(document)
                before = document.render_page(0, dpi=144)
                with document.copy() as copied:
                    held_range = last[:]
                    del story.paragraphs[0]
                    del story.paragraphs[0]
                    self.assertEqual([p.text for p in story.paragraphs], ['Last kept'])
                    for deleted in (first, middle):
                        with self.assertRaises((IndexError, ValueError)):
                            _ = deleted.text
                    with self.assertRaises(ValueError):
                        _ = held_range.text
                    self.assertEqual((outer.text, deep.text, cell.text),
                                     ('Outer kept', 'Nested kept', 'Outer kept\nNested kept'))
                    self.assertEqual(pic.data, direct_pic.data)
                    self.assertEqual(media_parts(document), raw_media)
                    cell.add_paragraph('After nested')
                    self.assertEqual([p.text for p in cell.paragraphs], ['Outer kept', 'After nested'])
                    deep.text = 'Nested updated'
                    self.assertEqual(nested[0, 0].text, 'Nested updated')
                    self.assertEqual(body.text, 'Body kept')
                    self.assertEqual(copied.render_page(0, dpi=144), before)
                    after = document.render_page(0, dpi=144)
                    self.assertNotEqual(after, before)
                    self.assertGreater(count_color(after, b'\xff\0\0\xff'), 100)
                    for data in (document.to_bytes(), document.convert('pdf')):
                        with doc_ex.open(data) as reopened:
                            text = reopened.to_markdown() + ' '.join(s.get_text() for s in reopened.stories)
                            self.assertGreater(count_color(reopened.render_page(0, dpi=144), b'\xff\0\0\xff'), 100)
                            self.assertNotIn('REMOVE', text)
                            for expected in ('Nested updated', 'Last kept', 'Body kept'):
                                self.assertIn(expected, text)
                    del story.images[0]
                    self.assertEqual(pic.data, raw_media[0])
                    del nested[0, 0].images[0]
                    self.assertEqual(media_parts(document), [])

    def test_native_required_trailing_paragraph_is_hidden_and_reused(self):
        with doc_ex.new('docx') as document:
            document.add_paragraph('Body')
            header = document.sections[0].header()
            first = header.add_paragraph('First')
            table = header.add_table([['Table kept']])
            tail = header.add_paragraph('Tail')
            del header.paragraphs[-1]
            self.assertEqual([p.text for p in header.paragraphs], ['First'])
            self.assertEqual(table[0, 0].text, 'Table kept')
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                root = ET.fromstring(archive.read('word/header1.xml'))
                self.assertEqual([node.tag for node in root], [W+'p', W+'tbl', W+'p'])
                self.assertEqual(list(root[-1].iter(W+'t')), [])
            fresh = header.add_paragraph('Fresh')
            self.assertGreater(fresh._id, tail._id)
            with self.assertRaises((IndexError, ValueError)):
                _ = tail.text
            self.assertEqual(first.text, 'First')
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                root = ET.fromstring(archive.read('word/header1.xml'))
                self.assertEqual([node.tag for node in root], [W+'p', W+'tbl', W+'p'])
            empty = document.sections[0].footer().add_paragraph('')
            del document.sections[0].footer().paragraphs[0]
            with self.assertRaises((IndexError, ValueError)):
                _ = empty.text
            selected = first[:]
            document.sections[0].footer().clear()
            self.assertEqual(selected.text, 'First')

    def test_note_first_middle_last_deletion_keeps_marker_kind_and_source_order(self):
        for kind in ('footnote', 'endnote'):
            with self.subTest(kind=kind), doc_ex.new('docx', size=(300, 360)) as document:
                document.sections[0].margins = 24
                anchor = document.add_paragraph('Citation')
                note = document.notes.add(anchor[:], 'First', kind=kind)
                note.add_paragraph('Remove middle')[:].format(color='red')
                survivor = note.add_paragraph('Last survivor')
                before = anchor.text
                original_ids = [node.get(W+'id') for node in note_xml(document, kind)]
                self.assertTrue(red_rows(document.render_page(0, dpi=144)))
                del note.paragraphs[1]
                self.assertEqual([p.text for p in note.paragraphs], ['First', 'Last survivor'])
                self.assertFalse(red_rows(document.render_page(0, dpi=144)))
                del note.paragraphs[0]
                self.assertEqual(survivor.text, 'Last survivor')
                self.assertIn('Last survivor', ' '.join(pdf_text_pages(document)))
                self.assertEqual(len(list(note_xml(document, kind)[0].iter(W+kind+'Ref'))), 1)
                del note.paragraphs[-1]
                self.assertEqual(len(note.paragraphs), 0)
                self.assertEqual(note.kind, kind)
                self.assertEqual(anchor.text, before)
                self.assertEqual([node.get(W+'id') for node in note_xml(document, kind)], original_ids)
                with self.assertRaises((IndexError, ValueError)):
                    _ = survivor.text
                current = anchor[:]
                document.render_page(0, dpi=144)
                cached = document._cached
                note.clear()
                self.assertIs(document._cached, cached)
                self.assertEqual(current.text, before)
                document.notes.convert(kind='endnote' if kind == 'footnote' else 'footnote')
                note.clear()
                fresh = note.add_paragraph('Recovered')
                self.assertGreater(fresh._id, survivor._id)
                self.assertEqual(len(note_xml(document, note.kind)[0].findall(W+'p')), 1)
                self.assertIn('Recovered', ' '.join(pdf_text_pages(document)))

    def test_index_callbacks_snapshot_stable_ids_and_captured_iterators_do_not_retarget(self):
        with doc_ex.new('docx') as document:
            story = document.sections[0].header()
            a, b, c = [story.add_paragraph(text) for text in ('A', 'B', 'C')]
            iterator = iter(story.paragraphs)
            self.assertEqual(next(iterator).text, 'A')
            class Shift:
                def __index__(self):
                    del story.paragraphs[0]
                    return 1
            del story.paragraphs[Shift()]
            self.assertEqual([p.text for p in story.paragraphs], ['C'])
            stale = next(iterator)
            with self.assertRaises((IndexError, ValueError)):
                _ = stale.text
            self.assertEqual(next(iterator).text, c.text)
            class Replace:
                def __index__(self):
                    story.clear()
                    story.add_paragraph('Replacement')
                    return 0
            with self.assertRaises((IndexError, ValueError)):
                del story.paragraphs[Replace()]
            self.assertEqual([p.text for p in story.paragraphs], ['Replacement'])
            class Close:
                def __index__(self):
                    document.close()
                    return 999999
            with self.assertRaises(DocumentClosedError):
                del story.paragraphs[Close()]
            for handle in (a, b, c):
                with self.assertRaises(DocumentClosedError):
                    _ = handle.text

    def test_unsupported_scopes_invalid_indexes_and_fields_links_are_atomic(self):
        with doc_ex.new('docx') as document:
            body = document.add_paragraph('Target')
            document.bookmarks['target'] = body[:]
            table = document.add_table([['Cell']])
            document.bookmarks['cell_target'] = table[0, 0].paragraphs[0][:]
            story = document.sections[0].header()
            paragraph = story.add_paragraph('Remove linked')
            paragraph[:].link = '#target'
            paragraph.add_field('page')
            before = document.to_bytes()
            current = paragraph[:]
            document.render_page(0, dpi=72)
            cached = document._cached
            # Body deletion is supported now; its linked target remains protected
            # by the existing dangling-link check before anchor removal validation.
            with self.assertRaisesRegex(ValueError, 'missing bookmark'):
                del document.paragraphs[0]
            with self.assertRaises(UnsupportedFormatError):
                del table[0, 0].paragraphs[0]
            for bad in (2, -2, 10**200, 1.5, '0', slice(None)):
                with self.assertRaises((TypeError, IndexError, OverflowError)):
                    del story.paragraphs[bad]
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(current.text, paragraph.text)
                self.assertIs(document._cached, cached)
            del story.paragraphs[0]
            self.assertEqual(document.bookmarks['target'].text, 'Target')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(sum(len(page.get_links()) for page in pdf), 0)
            document.close()
            with self.assertRaises(DocumentClosedError):
                del story.paragraphs['bad']
        with doc_ex.new('pptx') as deck:
            table = deck.slides.add().add_table([['Cell']], box=(10, 10, 200, 80))
            before = deck.to_bytes()
            with self.assertRaises(TypeError):
                del table[0, 0].paragraphs[:]
            self.assertEqual(deck.to_bytes(), before)
