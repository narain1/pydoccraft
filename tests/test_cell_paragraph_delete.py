"""Cell paragraph deletion retains native containers and surviving owned handles."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import count_color, media_parts, red_picture

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def cell_xml(document, scope='body'):
    name = 'document' if scope == 'body' else scope + '1'
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        return ET.fromstring(archive.read('word/' + name + '.xml')).find('.//' + W + 'tc')


def owned_table(document, scope, rows):
    if scope == 'body':
        return document.add_table(rows)
    document.add_paragraph('Body kept')
    return getattr(document.sections[0], scope)().add_table(rows)


class CellParagraphDeletionTests(unittest.TestCase):
    def test_owned_scopes_keep_nested_handles_media_copies_and_visible_output(self):
        for scope in ('body', 'header', 'footer'):
            with self.subTest(scope=scope), doc_ex.new('docx') as document:
                document.sections[0].margins = (180, 36, 180, 36)
                table = owned_table(document, scope, [['REMOVE FIRST', 'Sibling kept']])
                cell = table[0, 0]
                first = cell.paragraphs[0]
                first[:].format(color='blue')
                sibling = table[0, 1].paragraphs[0]
                nested = cell.add_table([['Deep kept']])
                deep = nested[0, 0].paragraphs[0]
                picture = nested[0, 0].add_image(red_picture(2, 2), width=12)
                middle = cell.add_paragraph('REMOVE MIDDLE')
                middle[:].format(color='blue')
                last = cell.add_paragraph('Last kept')
                assets = media_parts(document)
                before = document.render_page(0, dpi=144)
                self.assertGreater(count_color(before, b'\0\0\xff\xff'), 25)
                with document.copy() as copied:
                    selected = last[:]
                    del cell.paragraphs[0]
                    del cell.paragraphs[0]
                    self.assertEqual([p.text for p in cell.paragraphs], ['Last kept'])
                    for removed in (first, middle):
                        with self.assertRaises((IndexError, ValueError)):
                            _ = removed.text
                    with self.assertRaises(ValueError):
                        _ = selected.text
                    self.assertEqual((deep.text, sibling.text), ('Deep kept', 'Sibling kept'))
                    self.assertEqual(picture.data, red_picture(2, 2))
                    self.assertEqual(media_parts(document), assets)
                    table.row(0).allow_split = False
                    deep.text = 'Deep updated'
                    self.assertEqual(copied.render_page(0, dpi=144), before)
                    for dpi in (72, 144):
                        png = document.render_page(0, dpi=dpi)
                        self.assertEqual(count_color(png, b'\0\0\xff\xff'), 0)
                        self.assertGreater(count_color(png, b'\xff\0\0\xff'), 50)
                    for data in (document.to_bytes(), document.convert('pdf')):
                        with doc_ex.open(data) as reopened:
                            text = reopened.to_markdown() + ' '.join(story.get_text() for story in reopened.stories)
                            self.assertNotIn('REMOVE', text)
                            for label in ('Deep updated', 'Sibling kept', 'Last kept'):
                                self.assertEqual(text.count(label), 1)
                            self.assertGreater(count_color(reopened.render_page(0, dpi=144), b'\xff\0\0\xff'), 100)
                    del cell.paragraphs[-1]
                    self.assertEqual(len(cell.paragraphs), 0)
                    self.assertEqual(deep.text, 'Deep updated')
                    cell.add_paragraph('Fresh tail')
                    self.assertEqual([p.text for p in cell.paragraphs], ['Fresh tail'])

    def test_merged_aliases_and_nested_deletion_preserve_other_cells_and_grid_handles(self):
        for end in ((0, 1), (1, 0)):
            with self.subTest(end=end), doc_ex.new('docx') as document:
                table = document.add_table([['REMOVE', 'Right kept'], ['Below kept', 'Sibling kept']])
                anchor = table.merge((0, 0), end)
                covered = table[end]
                sibling = table[1, 1].paragraphs[0]
                first = anchor.paragraphs[0]
                keep = anchor.add_paragraph('Kept in merge')
                nested = anchor.add_table([['REMOVE NESTED', 'Nested sibling']])
                nested_cell = nested[0, 0]
                nested_drop = nested_cell.paragraphs[0]
                deep = nested_cell.add_table([['Deep kept']])
                deep_para = deep[0, 0].paragraphs[0]
                nested_keep = nested_cell.add_paragraph('Nested tail')
                self.assertEqual(covered.paragraphs[0]._id, first._id)
                del covered.paragraphs[0]
                del nested_cell.paragraphs[0]
                self.assertEqual([p._id for p in anchor.paragraphs], [p._id for p in covered.paragraphs])
                self.assertEqual((keep.text, nested_keep.text, deep_para.text, sibling.text), ('Kept in merge', 'Nested tail', 'Deep kept', 'Sibling kept'))
                self.assertEqual(nested[0, 1].text, 'Nested sibling')
                for removed in (first, nested_drop):
                    with self.assertRaises((IndexError, ValueError)):
                        _ = removed.text
                anchor.background = 'yellow'
                nested_cell.add_paragraph('Append after nested delete')
                for data in (document.to_bytes(), document.convert('pdf')):
                    with doc_ex.open(data) as reopened:
                        text = reopened.to_markdown()
                        self.assertNotIn('REMOVE', text)
                        for label in ('Kept in merge', 'Nested tail', 'Deep kept', 'Sibling kept', 'Nested sibling'):
                            self.assertEqual(text.count(label), 1)

    def test_authored_empty_paragraphs_stay_visible_until_deleted_and_required_blank_is_reused(self):
        for scope in ('body', 'header', 'footer'):
            with self.subTest(scope=scope), doc_ex.new('docx') as document:
                table = owned_table(document, scope, [['']])
                cell = table[0, 0]
                first = cell.paragraphs[0]
                second = cell.add_paragraph('')
                del cell.paragraphs[0]
                self.assertEqual([p._id for p in cell.paragraphs], [second._id])
                self.assertEqual(second.text, '')
                self.assertEqual(len(cell_xml(document, scope).findall(W+'p')), 1)
                del cell.paragraphs[0]
                self.assertEqual(len(cell.paragraphs), 0)
                self.assertEqual(len(cell_xml(document, scope).findall(W+'p')), 1)
                for removed in (first, second):
                    with self.assertRaises((IndexError, ValueError)):
                        _ = removed.text
                fresh = cell.add_paragraph('Fresh')
                self.assertGreater(fresh._id, second._id)
                nested = cell.add_table([['Nested kept']])
                deep = nested[0, 0].paragraphs[0]
                tail = cell.add_paragraph('Tail')
                del cell.paragraphs[-1]
                tags = [node.tag for node in cell_xml(document, scope) if node.tag != W+'tcPr']
                self.assertEqual(tags, [W+'p', W+'tbl', W+'p'])
                del cell.paragraphs[0]
                self.assertEqual(len(cell.paragraphs), 0)
                self.assertEqual(deep.text, 'Nested kept')
                tags = [node.tag for node in cell_xml(document, scope) if node.tag != W+'tcPr']
                self.assertEqual(tags, [W+'tbl', W+'p'])
                replacement = cell.add_paragraph('Replacement')
                self.assertGreater(replacement._id, tail._id)
                self.assertEqual([p.text for p in cell.paragraphs], ['Replacement'])
                self.assertEqual(len(cell_xml(document, scope).findall(W+'p')), 1)

    def test_protected_anchors_links_fields_and_invalid_inputs_fail_atomically(self):
        with doc_ex.new('docx') as document:
            cell = document.add_table([['Target']])[0, 0]
            target = cell.paragraphs[0]
            document.bookmarks['target'] = target[:]
            citation = cell.add_paragraph('Citation')
            note = document.notes.add(citation[:], 'Retained note')
            outgoing = cell.add_paragraph('Jump target')
            outgoing[:].link = '#target'
            outgoing.add_field('page')
            outgoing.add_field('title')
            for index, error in ((0, ValueError), (1, UnsupportedFormatError)):
                before = document.to_bytes()
                selected = outgoing[:]
                document.render_page(0, dpi=72)
                cache = document._cached
                with self.assertRaises(error):
                    del cell.paragraphs[index]
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(selected.text, outgoing.text)
                self.assertIs(document._cached, cache)
            for bad in (3, -4, 10**200, 1.5, '0', slice(None)):
                before = document.to_bytes()
                with self.assertRaises((TypeError, IndexError, OverflowError)):
                    del cell.paragraphs[bad]
                self.assertEqual(document.to_bytes(), before)
            with self.assertRaises(UnsupportedFormatError):
                cell.paragraphs.move(0, 1)
            del cell.paragraphs[-1]
            self.assertEqual(document.bookmarks['target'].text, 'Target')
            self.assertEqual(note.paragraphs[0].text, 'Retained note')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(sum(len(page.get_links()) for page in pdf), 0)
            with self.assertRaises(UnsupportedFormatError):
                del cell.paragraphs[0]
        with doc_ex.new('pptx') as document:
            cell = document.slides.add().add_table([['Kept']], box=(10, 10, 200, 80))[0, 0]
            with self.assertRaises(TypeError):
                del cell.paragraphs[:]
            self.assertEqual(cell.text, 'Kept')

    def test_callbacks_capture_ids_and_stale_grid_rejects_before_index_conversion(self):
        with doc_ex.new('docx') as document:
            table = document.add_table([['A', 'Neighbor']])
            cell = table[0, 0]
            a = cell.paragraphs[0]
            b, c = [cell.add_paragraph(text) for text in ('B', 'C')]
            iterator = iter(cell.paragraphs)
            self.assertEqual(next(iterator).text, 'A')
            class Shift:
                def __index__(self):
                    del cell.paragraphs[0]
                    return 1
            del cell.paragraphs[Shift()]
            self.assertEqual([p.text for p in cell.paragraphs], ['C'])
            with self.assertRaises((IndexError, ValueError)):
                _ = next(iterator).text
            self.assertEqual(next(iterator).text, c.text)
            class Replace:
                def __index__(self):
                    cell.text = 'Replacement'
                    return 0
            with self.assertRaises((IndexError, ValueError)):
                del cell.paragraphs[Replace()]
            self.assertEqual(cell.text, 'Replacement')
            collection = cell.paragraphs
            table.merge((0, 0), (0, 1))
            class MustNotRun:
                def __index__(self):
                    raise AssertionError('stale cell must be checked first')
            with self.assertRaises(ValueError):
                del collection[MustNotRun()]
            cell = table[0, 0]
            class MergeDuringIndex:
                def __index__(self):
                    table.insert_row(1)
                    return 0
            with self.assertRaises((IndexError, ValueError)):
                del cell.paragraphs[MergeDuringIndex()]
            self.assertIn('Replacement', table[0, 0].text)
            current = table[0, 0]
            class Close:
                def __index__(self):
                    document.close()
                    return 99999
            with self.assertRaises(DocumentClosedError):
                del current.paragraphs[Close()]
            for handle in (a, b, c):
                with self.assertRaises(DocumentClosedError):
                    _ = handle.text


if __name__ == '__main__':
    unittest.main()
