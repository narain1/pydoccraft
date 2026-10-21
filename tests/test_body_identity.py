"""Owned DOCX identities remain stable independently of native body positions."""
from io import BytesIO
import struct
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import count_color, red_picture
from test_rendering import png_chunks

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


class BodyIdentityTests(unittest.TestCase):
    def test_interleaved_owned_objects_enumerate_in_source_order_and_share_targets(self):
        with doc_ex.new('docx', size=(360, 480)) as document:
            document.sections[0].margins = 30
            first = document.add_paragraph('First body')
            header = document.sections[0].header()
            head = header.add_paragraph('Header')
            table = document.add_table([['Cell']])
            cell = table[0, 0].paragraphs[0]
            picture = document.add_image(red_picture(2, 2), width=12)
            empty = document.add_paragraph('')
            last = document.add_paragraph('Last body')
            handles = (first, head, table, cell, picture, empty, last)
            ids = [item._id for item in handles]
            self.assertEqual(len(set(ids)), len(ids))
            self.assertEqual(ids, sorted(ids))
            self.assertEqual([p.text for p in document.paragraphs], ['First body', '', 'Last body'])
            self.assertEqual([t.rows for t in document.tables], [[['Cell']]])
            self.assertEqual([p.alt for p in document.images], [''])
            document.bookmarks['First'] = first[:]
            self.assertEqual(document.bookmarks['First']._paragraph._id, first._id)
            source = document.to_bytes()
            warmed = document.render_page(0, dpi=144)
            selected = last[:]
            for _ in range(3):
                self.assertEqual([p._id for p in document.paragraphs], [first._id, empty._id, last._id])
                self.assertEqual(document.tables[0]._id, table._id)
                self.assertEqual(document.images[0]._id, picture._id)
                self.assertEqual(document.bookmarks['First']._paragraph._id, first._id)
            self.assertEqual(selected.text, 'Last body')
            self.assertEqual(document.to_bytes(), source)
            self.assertEqual(document.render_page(0, dpi=144), warmed)
            self.assertGreater(count_color(warmed, b'\xff\0\0\xff'), 100)
            for data in (source, document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    text = reopened.to_markdown()
                    self.assertLess(text.index('First body'), text.index('Cell'))
                    self.assertLess(text.index('Cell'), text.index('Last body'))
                    self.assertGreater(count_color(reopened.render_page(0, dpi=144), b'\xff\0\0\xff'), 100)

    def test_empty_sections_and_required_tail_keep_physical_boundaries_hidden(self):
        with doc_ex.new('docx', size=(300, 360)) as document:
            document.sections[0].margins = 24
            first = document.add_table([['First section']])
            original_header = document.sections[0].header().add_paragraph('Head')
            document.sections.add()
            final = document.sections.add()
            final.page_size = (420, 480)
            final.header().paragraphs[0].text = 'Last head'
            last = document.add_paragraph('Last section')
            self.assertEqual([p.text for p in document.paragraphs], ['Last section'])
            self.assertEqual(original_header.text, 'Head')
            self.assertEqual(first.rows, [['First section']])
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                body = ET.fromstring(archive.read('word/document.xml')).find(W+'body')
                self.assertEqual([node.tag for node in body], [W+'tbl', W+'p', W+'p', W+'p', W+'sectPr'])
                self.assertIsNotNone(body[1].find(W+'pPr/'+W+'sectPr'))
                self.assertIsNotNone(body[2].find(W+'pPr/'+W+'sectPr'))
                self.assertEqual(list(body[1].iter(W+'t')), [])
                self.assertEqual(list(body[2].iter(W+'t')), [])
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    self.assertEqual(len(reopened), 3)
                    self.assertIn('First section', reopened[0].get_text())
                    self.assertNotIn('Last section', reopened[1].get_text())
                    self.assertIn('Last section', reopened[2].get_text())
                    for index, expected in enumerate(((300, 360), (300, 360), (420, 480))):
                        chunks = png_chunks(reopened.render_page(index, dpi=72))
                        self.assertEqual(struct.unpack('>II', chunks[b'IHDR'][:8]), expected)
            with document.copy() as copied:
                document.close()
                with self.assertRaises(DocumentClosedError):
                    _ = last.text
                self.assertEqual(copied.paragraphs[0]._id, last._id)
                self.assertEqual(copied.paragraphs[0].text, 'Last section')
                self.assertEqual(copied.tables[0]._id, first._id)

    def test_removed_picture_and_invalidated_children_do_not_retarget_body_handles(self):
        with doc_ex.new('docx') as document:
            first = document.add_paragraph('First')
            table = document.add_table([['Cell']])
            child = table[0, 0].paragraphs[0]
            header = document.sections[0].header()
            head = header.add_paragraph('Old header')
            picture = document.add_image(red_picture(2, 2), width=12)
            last = document.add_paragraph('Last')
            with document.copy() as copied:
                stale_range = first[:]
                del document.images[0]
                self.assertEqual([p._id for p in document.paragraphs], [first._id, picture._id, last._id])
                anchor = document.paragraphs[1]
                self.assertEqual(anchor.text, '')
                anchor.text = 'Picture anchor'
                with self.assertRaises((ValueError, IndexError)):
                    _ = picture.data
                with self.assertRaises(ValueError):
                    _ = stale_range.text
                table[0, 0].text = 'New cell'
                header.clear()
                for stale in (child, head):
                    with self.assertRaises((ValueError, IndexError)):
                        _ = stale.text
                fresh_child = table[0, 0].paragraphs[0]
                fresh_head = header.add_paragraph('Fresh header')
                self.assertGreater(fresh_child._id, last._id)
                self.assertGreater(fresh_head._id, fresh_child._id)
                self.assertEqual((first.text, last.text), ('First', 'Last'))
                self.assertEqual(copied.tables[0][0, 0].text, 'Cell')
                self.assertEqual(copied.sections[0].header().paragraphs[0].text, 'Old header')
                self.assertEqual(copied.images[0].data, red_picture(2, 2))
                before = document.to_bytes()
                with self.assertRaises(UnsupportedFormatError):
                    del document.paragraphs[0]
                with self.assertRaises(UnsupportedFormatError):
                    del table[0, 0].paragraphs[0]
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual((first.text, last.text), ('First', 'Last'))

    def test_failed_body_creation_preserves_existing_range_cache_and_next_identity(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Kept')
            selected = paragraph[:]
            source = document.to_bytes()
            warm = document.render_page(0, dpi=72)
            for operation in (
                lambda: document.add_paragraph('\0'),
                lambda: document.add_table([['\0']]),
                lambda: document.add_image(red_picture(2, 2), width=0.5),
            ):
                with self.assertRaises((ValueError, doc_ex.DocExError)):
                    operation()
                self.assertEqual(selected.text, 'Kept')
                self.assertEqual(document.to_bytes(), source)
                self.assertEqual(document.render_page(0, dpi=72), warm)
            fresh = document.add_paragraph('Fresh')
            self.assertEqual(fresh._id, paragraph._id + 1)
            self.assertEqual([p.text for p in document.paragraphs], ['Kept', 'Fresh'])
            self.assertIn(f'id={paragraph._id}', repr(paragraph))
            self.assertLess(len(repr(paragraph)), 160)


if __name__ == '__main__':
    unittest.main()
