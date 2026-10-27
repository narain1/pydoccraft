"""Owned paragraph moves preserve native content and a compact collection contract."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_body_paragraph_delete import body_xml, section_xml
from test_authoring import image_fixture
from test_picture_media import media_parts
from test_note_conversion import red_rows

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def paragraphs(document):
    return [p.text for p in document.paragraphs]


class ParagraphMoveTests(unittest.TestCase):
    def test_body_move_reorders_native_content_and_reuses_required_tail(self):
        with doc_ex.new('docx', size=(400, 600)) as document:
            document.sections[0].margins = 30
            first = document.add_paragraph('First ordinary')
            table = document.add_table([['Cell retained']])
            cell = table[0, 0]
            nested = cell.add_table([['Nested retained']])
            deep = nested[0, 0].paragraphs[0]
            image = image_fixture(2, 2)
            picture = nested[0, 0].add_image(image, width=10)
            last = document.add_paragraph('Last red')
            last[:].format(color='red', bold=True)
            last[:].link = 'https://example.com/moved'
            before_rows = red_rows(document.render_page(0, dpi=144))
            self.assertTrue(before_rows)
            held = first[:]
            document.paragraphs.move(-1, 0)
            self.assertEqual(paragraphs(document), ['Last red', 'First ordinary'])
            self.assertEqual((first.text, last.text, deep.text),
                             ('First ordinary', 'Last red', 'Nested retained'))
            self.assertEqual(last[:].link, 'https://example.com/moved')
            self.assertEqual(picture.data, image)
            self.assertEqual(media_parts(document), [image])
            with self.assertRaises(ValueError):
                _ = held.text
            after_rows = red_rows(document.render_page(0, dpi=144))
            self.assertTrue(after_rows)
            self.assertLess(min(after_rows), min(before_rows))
            body = body_xml(document)
            self.assertEqual([b.tag for b in body], [W+'p', W+'p', W+'tbl', W+'p', W+'sectPr'])
            self.assertFalse(''.join(body[-2].itertext()).strip())
            fresh = document.add_paragraph('Inserted later')
            self.assertEqual(len(body_xml(document)), len(body))
            fresh.add_field('page')
            document.paragraphs.move(-1, 1)
            self.assertEqual(paragraphs(document), ['Last red', 'Inserted later\ufffc', 'First ordinary'])
            moved_xml = body_xml(document)[1]
            self.assertEqual(''.join(n.text or '' for n in moved_xml.iter(W+'t')), 'Inserted later')
            self.assertEqual([n.text.strip() for n in moved_xml.iter(W+'instrText')],
                             ['PAGE \\* Arabic'])
            with doc_ex.open(document.convert('pdf')) as pdf:
                plain = ' '.join(pdf[0].get_text().split())
                self.assertRegex(plain, r'Inserted later\s*1')
                self.assertLess(plain.index('Last red'), plain.index('Inserted later'))
                self.assertLess(plain.index('Inserted later'), plain.index('First ordinary'))
            deep.text = 'Nested edited'
            cell.add_paragraph('Cell appended')
            for data in (document.to_bytes(), document.convert('pdf')):
                with doc_ex.open(data) as opened:
                    text = opened.to_markdown()
                    for label in ('Last red', 'Inserted later', 'First ordinary', 'Nested edited', 'Cell appended'):
                        self.assertEqual(text.count(label), 1)
                    self.assertLess(text.index('Last red'), text.index('Inserted later'))
                    self.assertLess(text.index('Inserted later'), text.index('First ordinary'))
                    self.assertTrue(red_rows(opened.render_page(0, dpi=144)))
            self.assertEqual(table[0, 0].paragraphs[0].text, 'Cell retained')

    def test_all_auxiliary_containers_move_paragraphs_and_keep_native_markers(self):
        for kind in ('header', 'footer', 'footnote', 'endnote'):
            with self.subTest(kind=kind), doc_ex.new('docx') as document:
                body = document.add_paragraph('Body anchor')
                if kind in ('header', 'footer'):
                    story = getattr(document.sections[0], kind)()
                    first = story.add_paragraph('First auxiliary')
                else:
                    story = document.notes.add(body[:], 'First auxiliary', kind=kind)
                    first = story.paragraphs[0]
                table = story.add_table([['Auxiliary table']]) if kind in ('header', 'footer') else None
                last = story.add_paragraph('Last auxiliary')
                last[:].format(color='red')
                old_sections = section_xml(document)
                story.paragraphs.move(-1, 0)
                self.assertEqual([p.text for p in story.paragraphs], ['Last auxiliary', 'First auxiliary'])
                self.assertEqual((first.text, last.text), ('First auxiliary', 'Last auxiliary'))
                if table is not None:
                    self.assertEqual(table[0, 0].text, 'Auxiliary table')
                story.add_paragraph('Reused blank')
                story.paragraphs.move(-1, 1)
                self.assertEqual([p.text for p in story.paragraphs],
                                 ['Last auxiliary', 'Reused blank', 'First auxiliary'])
                self.assertEqual(section_xml(document), old_sections)
                with ZipFile(BytesIO(document.to_bytes())) as archive:
                    name = next(n for n in archive.namelist()
                                if n.startswith('word/' + kind) and n.endswith('.xml'))
                    tree = ET.fromstring(archive.read(name))
                    text = ' '.join(n.text or '' for n in tree.iter(W+'t'))
                    for label in ('Last auxiliary', 'Reused blank', 'First auxiliary'):
                        self.assertEqual(text.count(label), 1)
                    self.assertLess(text.index('Last auxiliary'), text.index('Reused blank'))
                    self.assertLess(text.index('Reused blank'), text.index('First auxiliary'))
                    if kind in ('footnote', 'endnote'):
                        self.assertEqual(len(list(tree.iter(W + kind + 'Ref'))), 1)
                with doc_ex.open(document.convert('pdf')) as pdf:
                    text = pdf.to_markdown()
                    for label in ('Last auxiliary', 'Reused blank', 'First auxiliary'):
                        self.assertEqual(text.count(label), 1)
                    self.assertTrue(red_rows(pdf.render_page(0, dpi=144)))

    def test_section_anchor_and_unsupported_scope_failures_preserve_state(self):
        for start in ('next_page', 'continuous', 'odd_page', 'even_page'):
            with self.subTest(start=start), doc_ex.new('docx') as document:
                first = document.add_paragraph('First')
                second = document.add_paragraph('Second')
                boundary = document.add_paragraph('Boundary')
                document.sections.add(start=start)
                document.add_paragraph('Final section')
                before = section_xml(document)
                document.paragraphs.move(1, 0)
                self.assertEqual(section_xml(document), before)
                for source, target in ((0, 2), (2, 0), (0, -1)):
                    raw = document.to_bytes()
                    document.render_page(0, dpi=72)
                    cache = document._cached
                    held = first[:]
                    with self.assertRaises(UnsupportedFormatError):
                        document.paragraphs.move(source, target)
                    self.assertEqual(document.to_bytes(), raw)
                    self.assertIs(document._cached, cache)
                    self.assertEqual(held.text, 'First')
                self.assertEqual((first.text, second.text, boundary.text), ('First', 'Second', 'Boundary'))
        with doc_ex.new('docx') as document:
            anchored = document.add_paragraph('Reference')
            other = document.add_paragraph('Other')
            document.notes.add(anchored[:], 'Retained note')
            raw = document.to_bytes()
            with self.assertRaisesRegex(UnsupportedFormatError, 'anchors'):
                document.paragraphs.move(0, 1)
            self.assertEqual(document.to_bytes(), raw)
            document.paragraphs.move(1, 0)
            self.assertEqual(paragraphs(document)[0], other.text)
            table = document.add_table([['Cell']])
            raw = document.to_bytes()
            revision = document._call("revision")
            document.render_page(0, dpi=72)
            cache = document._cached
            table[0, 0].paragraphs.move(0, 0)
            self.assertEqual(document.to_bytes(), raw)
            self.assertEqual(document._call("revision"), revision)
            self.assertIs(document._cached, cache)
        with doc_ex.new('pptx') as slides:
            cell = slides.slides.add().add_table([['Text']], box=(20, 20, 200, 60))[0, 0]
            paragraph = cell.paragraphs[0]
            held = paragraph[:]
            raw, revision = slides.to_bytes(), slides._call('revision')
            slides.render_page(0, dpi=72)
            cache = slides._cached
            cell.paragraphs.move(0, 0)
            self.assertEqual(slides.to_bytes(), raw)
            self.assertEqual(slides._call('revision'), revision)
            self.assertIs(slides._cached, cache)
            self.assertEqual(held.text, 'Text')
            self.assertEqual(paragraph.text, 'Text')

    def test_noop_iteration_copy_negative_indexes_and_lifetimes(self):
        with doc_ex.new('docx') as document:
            first = document.add_paragraph('First')
            document.render_page(0, dpi=72)
            cache = document._cached
            held = first[:]
            raw = document.to_bytes()
            document.paragraphs.move(0, -1)
            self.assertEqual(document.to_bytes(), raw)
            self.assertIs(document._cached, cache)
            self.assertEqual(held.text, 'First')
            second = document.add_paragraph('Second')
            third = document.add_paragraph('Third')
            iterator = iter(document.paragraphs)
            self.assertIsInstance(next(iterator), type(first))
            with document.copy() as copied:
                document.paragraphs.move(-1, 0)
                self.assertEqual([p.text for p in iterator], ['Second', 'Third'])
                self.assertEqual(paragraphs(document), ['Third', 'First', 'Second'])
                self.assertEqual(paragraphs(copied), ['First', 'Second', 'Third'])
                self.assertEqual((first.text, second.text, third.text), ('First', 'Second', 'Third'))
            for pair in ((10, 0), (0, -10), (slice(None), 0), (0, '1')):
                raw = document.to_bytes()
                with self.assertRaises((TypeError, IndexError)):
                    document.paragraphs.move(*pair)
                self.assertEqual(document.to_bytes(), raw)
            del document.paragraphs[0]
            with self.assertRaises((ValueError, IndexError)):
                _ = third.text
            collection = document.paragraphs
        with self.assertRaises(DocumentClosedError):
            collection.move(0, 0)

    def test_index_callbacks_cannot_retarget_or_apply_a_pending_move(self):
        class Index:
            def __init__(self, callback, result):
                self.callback, self.result = callback, result
            def __index__(self):
                self.callback()
                return self.result

        for argument in (0, 1):
            for operation in ('append', 'delete', 'edit', 'close'):
                with self.subTest(argument=argument, operation=operation), doc_ex.new('docx') as document:
                    first = document.add_paragraph('First')
                    document.add_paragraph('Second')
                    callbacks = {'append': lambda: document.add_paragraph('Callback'),
                                 'delete': lambda: document.paragraphs.__delitem__(0),
                                 'edit': lambda: setattr(first, 'text', 'Changed'),
                                 'close': document.close}
                    args = [0, 1]
                    args[argument] = Index(callbacks[operation], args[argument])
                    with self.assertRaises(DocumentClosedError if operation == 'close' else ValueError):
                        document.paragraphs.move(*args)
                    if operation != 'close':
                        expected = {'append': ['First', 'Second', 'Callback'],
                                    'delete': ['Second'], 'edit': ['Changed', 'Second']}[operation]
                        self.assertEqual(paragraphs(document), expected)
        with doc_ex.new('docx') as document:
            document.add_paragraph('First')
            document.add_paragraph('Second')
            document.paragraphs.move(Index(lambda: None, 0), Index(lambda: None, 1))
            self.assertEqual(paragraphs(document), ['Second', 'First'])
