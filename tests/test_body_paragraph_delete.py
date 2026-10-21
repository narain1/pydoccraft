"""Body paragraph removal preserves section ownership and surviving identities."""
from io import BytesIO
import struct
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_note_clear import note_xml
from test_picture_media import count_color, media_parts, red_picture
from test_rendering import png_chunks

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def body_xml(document):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        return ET.fromstring(archive.read('word/document.xml')).find(W+'body')


def section_xml(document):
    return [ET.tostring(value) for value in body_xml(document).iter(W+'sectPr')]


class BodyParagraphDeletionTests(unittest.TestCase):
    def test_first_middle_last_deletion_preserves_nested_handles_and_visible_output(self):
        with doc_ex.new('docx', size=(400, 600)) as document:
            document.sections[0].margins = 30
            first = document.add_paragraph('REMOVE FIRST')
            first[:].format(color='blue')
            table = document.add_table([['Cell kept']])
            cell = table[0, 0]
            cell_para = cell.paragraphs[0]
            nested = cell.add_table([['Deep kept']])
            deep = nested[0, 0].paragraphs[0]
            picture = nested[0, 0].add_image(red_picture(2, 2), width=12)
            middle = document.add_paragraph('REMOVE MIDDLE')
            middle[:].format(color='blue')
            last = document.add_paragraph('Last kept')
            head = document.sections[0].header().add_paragraph('Header kept')
            before = document.render_page(0, dpi=144)
            self.assertGreater(count_color(before, b'\0\0\xff\xff'), 25)
            assets = media_parts(document)
            with document.copy() as copied:
                selected = last[:]
                del document.paragraphs[0]
                del document.paragraphs[0]
                self.assertEqual([p.text for p in document.paragraphs], ['Last kept'])
                for removed in (first, middle):
                    with self.assertRaises((IndexError, ValueError)):
                        _ = removed.text
                with self.assertRaises(ValueError):
                    _ = selected.text
                self.assertEqual((cell_para.text, deep.text, head.text), ('Cell kept', 'Deep kept', 'Header kept'))
                self.assertEqual(picture.data, red_picture(2, 2))
                self.assertEqual(media_parts(document), assets)
                # Cell and row handles survive the enclosing root path shift.
                cell.add_paragraph('After nested')
                table.row(0).allow_split = False
                deep.text = 'Deep updated'
                after = document.render_page(0, dpi=144)
                self.assertEqual(count_color(after, b'\0\0\xff\xff'), 0)
                self.assertGreater(count_color(after, b'\xff\0\0\xff'), 100)
                self.assertEqual(copied.render_page(0, dpi=144), before)
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        text = reopened.to_markdown()
                        self.assertNotIn('REMOVE', text)
                        for expected in ('Cell kept', 'Deep updated', 'After nested', 'Last kept'):
                            self.assertEqual(text.count(expected), 1)
                        self.assertGreater(count_color(reopened.render_page(0, dpi=144), b'\xff\0\0\xff'), 100)
                del document.paragraphs[-1]
                self.assertEqual(len(document.paragraphs), 0)
                root = body_xml(document)
                self.assertEqual([node.tag for node in root], [W+'tbl', W+'p', W+'sectPr'])
                self.assertEqual(list(root[1].iter(W+'t')), [])
                fresh = document.add_paragraph('Fresh tail')
                self.assertGreater(fresh._id, last._id)
                self.assertEqual([node.tag for node in body_xml(document)], [W+'tbl', W+'p', W+'sectPr'])
                self.assertEqual(deep.text, 'Deep updated')

    def test_final_section_blank_preserves_breaks_properties_and_fields(self):
        # Native section_side uses the restarted number's parity: an even
        # section starting at7 displays8; restarted odd sections need no filler.
        for start, pages, displayed in (('next_page', 2, 7), ('continuous', 1, None), ('odd_page', 2, 7), ('even_page', 2, 8)):
            with self.subTest(start=start), doc_ex.new('docx', size=(300, 360)) as document:
                first = document.sections[0]
                first.margins = 24
                first.header_distance = 10
                first.header().add_paragraph('First head')
                boundary = document.add_paragraph('Boundary body')
                final = document.sections.add(start=start)
                final.header().paragraphs[0].text = 'Final head'
                if start != 'continuous':
                    final.page_number_start = 7
                number = final.footer().add_paragraph('Number:')
                number.add_field('page')
                tail = document.add_paragraph('Removed tail')
                expected = section_xml(document)
                before = document.to_bytes()
                with doc_ex.open(document.convert('pdf')) as baseline:
                    self.assertEqual(len(baseline), pages)
                    if displayed is not None:
                        self.assertRegex(baseline[-1].get_text(), rf'Number:\s*{displayed}')
                with self.assertRaises(UnsupportedFormatError):
                    del document.paragraphs[0]
                self.assertEqual(document.to_bytes(), before)
                del document.paragraphs[-1]
                self.assertEqual(section_xml(document), expected)
                self.assertEqual([p.text for p in document.paragraphs], ['Boundary body'])
                root = body_xml(document)
                self.assertEqual([node.tag for node in root], [W+'p', W+'p', W+'sectPr'])
                self.assertIsNotNone(root[0].find(W+'pPr/'+W+'sectPr'))
                self.assertIsNone(root[1].find(W+'pPr/'+W+'sectPr'))
                self.assertEqual(list(root[1].iter(W+'t')), [])
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        self.assertEqual(len(reopened), pages)
                        self.assertNotIn('Removed tail', reopened.to_markdown())
                        for index in range(pages):
                            chunks = png_chunks(reopened.render_page(index, dpi=72))
                            self.assertEqual(struct.unpack('>II', chunks[b'IHDR'][:8]), (300, 360))
                if start != 'continuous':
                    with doc_ex.open(document.convert('pdf')) as pdf:
                        self.assertRegex(pdf[-1].get_text(), rf'Number:\s*{displayed}')
                        self.assertIn('Final head', pdf[-1].get_text())
                fresh = document.add_paragraph('Fresh tail')
                self.assertGreater(fresh._id, tail._id)
                self.assertEqual(len(body_xml(document)), 3)
                self.assertEqual(section_xml(document), expected)
                self.assertEqual(boundary.text, 'Boundary body')
        with doc_ex.new('docx', size=(300, 360)) as document:
            document.sections[0].margins = 24
            document.add_table([['Initial table']])
            document.sections.add()
            final = document.sections.add()
            final.page_size = (420, 480)
            paragraph = document.add_paragraph('Only final content')
            values = section_xml(document)
            del document.paragraphs[0]
            self.assertEqual(section_xml(document), values)
            self.assertEqual(len(document.sections), 3)
            self.assertEqual(len(document.paragraphs), 0)
            with self.assertRaises((IndexError, ValueError)):
                _ = paragraph.text
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as reopened:
                    self.assertEqual(len(reopened), 3)
                    self.assertEqual(struct.unpack('>II', png_chunks(reopened.render_page(2, dpi=72))[b'IHDR'][:8]), (420, 480))

    def test_only_and_explicit_empty_paragraphs_reclaim_a_hidden_reusable_body(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Only')
            expected = section_xml(document)
            del document.paragraphs[0]
            self.assertEqual(len(document.paragraphs), 0)
            self.assertEqual(section_xml(document), expected)
            self.assertEqual([node.tag for node in body_xml(document)], [W+'p', W+'sectPr'])
            self.assertEqual(document.page_count, 1)
            empty = document.add_paragraph('')
            self.assertGreater(empty._id, paragraph._id)
            selected = empty[:]
            del document.paragraphs[0]
            with self.assertRaises(ValueError):
                _ = selected.text
            picture = document.add_image(red_picture(2, 2), width=12)
            self.assertEqual(len(list(body_xml(document).iter(W+'drawing'))), 1)
            del document.images[0]
            anchor = document.paragraphs[0]
            self.assertEqual(anchor._id, picture._id)
            del document.paragraphs[0]
            self.assertEqual(media_parts(document), [])
            self.assertEqual(len(document.paragraphs), 0)
            table = document.add_table([['Recovered']])
            self.assertEqual([node.tag for node in body_xml(document)], [W+'tbl', W+'p', W+'sectPr'])
            self.assertEqual(table[0, 0].text, 'Recovered')

    def test_protected_references_fail_atomically_but_preceding_content_can_disappear(self):
        with doc_ex.new('docx', size=(400, 600)) as document:
            document.sections[0].margins = 30
            before = document.add_paragraph('Remove before')
            target = document.add_paragraph('Target')
            document.bookmarks['target'] = target[:]
            citation = document.add_paragraph('Citation')
            note = document.notes.add(citation[:], 'Retained detail')
            ids = [node.get(W+'id') for node in note_xml(document, 'footnote')]
            outgoing = document.add_paragraph('Jump target')
            outgoing[:].link = '#target'
            outgoing.add_field('page')
            del document.paragraphs[0]
            self.assertEqual(document.bookmarks['target'].text, 'Target')
            self.assertEqual(note.paragraphs[0].text, 'Retained detail')
            self.assertEqual([node.get(W+'id') for node in note_xml(document, 'footnote')], ids)
            for index in (0, 1):
                source = document.to_bytes()
                selected = outgoing[:]
                warm = document.render_page(0, dpi=72)
                if index == 0:
                    # Dangling-link validation precedes protected-anchor validation.
                    with self.assertRaisesRegex(ValueError, 'missing bookmark'):
                        del document.paragraphs[index]
                else:
                    with self.assertRaises(UnsupportedFormatError):
                        del document.paragraphs[index]
                self.assertEqual(document.to_bytes(), source)
                self.assertEqual(selected.text, outgoing.text)
                self.assertEqual(document.render_page(0, dpi=72), warm)
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(sum(len(page.get_links()) for page in pdf), 1)
                self.assertIn('Retained detail', pdf.to_markdown())
            del document.paragraphs[-1]
            self.assertEqual(document.bookmarks['target'].text, 'Target')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(sum(len(page.get_links()) for page in pdf), 0)
            with self.assertRaises((IndexError, ValueError)):
                _ = before.text

    def test_integer_callbacks_keep_snapshot_targets_and_section_creation_is_not_undone(self):
        with doc_ex.new('docx') as document:
            a, b, c = [document.add_paragraph(text) for text in ('A', 'B', 'C')]
            iterator = iter(document.paragraphs)
            self.assertEqual(next(iterator).text, 'A')
            class Shift:
                def __index__(self):
                    del document.paragraphs[0]
                    return 1
            del document.paragraphs[Shift()]
            self.assertEqual([p.text for p in document.paragraphs], ['C'])
            with self.assertRaises((IndexError, ValueError)):
                _ = next(iterator).text
            self.assertEqual(next(iterator).text, c.text)
            class MakeBoundary:
                def __index__(self):
                    document.sections.add()
                    return 0
            with self.assertRaises(UnsupportedFormatError):
                del document.paragraphs[MakeBoundary()]
            self.assertEqual(len(document.sections), 2)
            self.assertEqual(c.text, 'C')
            tail = document.add_paragraph('Tail')
            class RemoveSelected:
                def __index__(self):
                    del document.paragraphs[-1]
                    return 1
            with self.assertRaises((IndexError, ValueError)):
                del document.paragraphs[RemoveSelected()]
            self.assertEqual([p.text for p in document.paragraphs], ['C'])
            replacement = document.add_paragraph('Replacement')
            self.assertGreater(replacement._id, tail._id)
            with document.copy() as copied:
                class Copy:
                    def __index__(self):
                        self.copy = document.copy()
                        self.copy.close()
                        return -1
                del document.paragraphs[Copy()]
                self.assertEqual(copied.paragraphs[-1].text, 'Replacement')
                self.assertEqual([p.text for p in document.paragraphs], ['C'])
            class Close:
                def __index__(self):
                    document.close()
                    return 999999
            with self.assertRaises(DocumentClosedError):
                del document.paragraphs[Close()]
            for value in (a, b, c):
                with self.assertRaises(DocumentClosedError):
                    _ = value.text


if __name__ == '__main__':
    unittest.main()
