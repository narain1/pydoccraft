from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_picture_media import count_color, media_parts, red_picture

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def section_xml(document):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        root = ET.fromstring(archive.read('word/document.xml'))
        return [ET.tostring(node) for node in root.iter(W + 'sectPr')]


class StoryClearTests(unittest.TestCase):
    def test_missing_and_repeated_clear_preserve_ranges_and_warmed_cache(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Body')
            for kind in ('header', 'footer'):
                for variant in ('default', 'first', 'even'):
                    with self.subTest(kind=kind, variant=variant):
                        story = getattr(document.sections[0], kind)(variant)
                        selection = paragraph[:]
                        before = document.to_bytes()
                        png = document.render_page(0, dpi=72)
                        cached = document._cached
                        story.clear()
                        self.assertEqual(document.to_bytes(), before)
                        self.assertEqual(selection.text, 'Body')
                        self.assertIs(document._cached, cached)
                        self.assertEqual(document.render_page(0, dpi=72), png)
            story = document.sections[0].header()
            empty = story.add_paragraph('')
            selection = paragraph[:]
            document.render_page(0, dpi=72)
            cached = document._cached
            story.clear()
            self.assertEqual(len(story.paragraphs), 0)
            self.assertIsNone(document._cached)
            with self.assertRaises((ValueError, IndexError)):
                _ = empty.text
            with self.assertRaises(ValueError):
                _ = selection.text
            selection = paragraph[:]
            document.render_page(0, dpi=72)
            cached = document._cached
            before = document.to_bytes()
            story.clear()
            self.assertEqual(document.to_bytes(), before)
            self.assertIs(document._cached, cached)
            self.assertEqual(selection.text, 'Body')
            fresh = story.add_paragraph('Fresh')
            self.assertEqual([p.text for p in story.paragraphs], ['Fresh'])
            self.assertGreater(fresh._id, empty._id)

    def test_rich_clear_removes_pixels_and_children_and_preserves_body(self):
        png = red_picture(4, 3)
        for kind in ('header', 'footer'):
            for first in ('paragraph', 'table', 'image'):
                with self.subTest(kind=kind, first=first), doc_ex.new('docx', size=(400, 500)) as document:
                    section = document.sections[0]
                    section.margins = 32
                    section.header_distance = section.footer_distance = 12
                    body = document.add_paragraph('Preserved body')
                    table_outside = document.add_table([['Outside table']])
                    outside_cell = table_outside[0, 0]
                    story = getattr(section, kind)()
                    children = {}
                    order = [first] + [name for name in ('paragraph', 'table', 'image') if name != first]
                    for name in order:
                        if name == 'paragraph':
                            children[name] = story.add_paragraph('Removed paragraph')
                            children[name].add_field('page')
                        elif name == 'table':
                            children[name] = story.add_table([['Removed cell']])
                        else:
                            children[name] = story.add_image(png, width=24, alt='Removed picture')
                    cell = children['table'][0, 0]
                    row = children['table'].row(0)
                    nested = cell.add_table([['Removed nested']])
                    nested_paragraph = nested[0, 0].paragraphs[0]
                    nested_paragraph.add_field('page_count')
                    nested[0, 0].add_image(png, width=14)
                    before_sections = section_xml(document)
                    original = document.render_page(0, dpi=72)
                    self.assertGreater(count_color(original, b'\xff\x00\x00\xff'), 400)
                    with document.copy() as copied:
                        story.clear()
                        self.assertEqual((len(story.paragraphs), len(story.tables), len(story.images)), (0, 0, 0))
                        self.assertEqual(section_xml(document), before_sections)
                        self.assertEqual((body.text, outside_cell.text), ('Preserved body', 'Outside table'))
                        self.assertEqual(media_parts(document), [])
                        for getter in (lambda: children['paragraph'].text, lambda: children['table'].rows,
                                       lambda: children['image'].data, lambda: cell.text, lambda: row.height,
                                       lambda: nested_paragraph.text):
                            with self.assertRaises((ValueError, IndexError)):
                                getter()
                        self.assertEqual(copied.render_page(0, dpi=72), original)
                        for data in (document.to_bytes(), document.convert('pdf')):
                            with doc_ex.open(data) as reopened:
                                text = reopened.to_markdown()
                                self.assertNotIn('Removed', text)
                                self.assertIn('Preserved body', text)
                                self.assertIn('Outside table', text)
                                self.assertEqual(count_color(reopened.render_page(0, dpi=72), b'\xff\x00\x00\xff'), 0)
                    # Each content kind can consume the fresh synthetic placeholder.
                    if first == 'paragraph':
                        story.add_paragraph('After clear')
                    elif first == 'table':
                        story.add_table([['After clear']])
                    else:
                        story.add_image(png, width=24)
                    with ZipFile(BytesIO(document.to_bytes())) as archive:
                        part = next(name for name in archive.namelist() if name.startswith('word/' + kind) and name.endswith('.xml'))
                        blocks = list(ET.fromstring(archive.read(part)))
                        self.assertEqual(blocks[0].tag, W + ('tbl' if first == 'table' else 'p'))
                        if first == 'image':
                            self.assertIsNotNone(blocks[0].find('.//' + W + 'drawing'))

    def test_variants_copied_sections_and_outgoing_links_remain_independent(self):
        with doc_ex.new('docx', size=(400, 500)) as document:
            section = document.sections[0]
            section.margins = 36
            for variant in ('default', 'first', 'even'):
                section.header(variant).add_paragraph(variant + ' visible')
                section.footer(variant).add_paragraph(variant + ' footer')
            target = document.add_paragraph('Target')
            document.bookmarks['Target'] = target[:]
            section.header('first').paragraphs[0][:].link = '#Target'
            for label in ('Page two', 'Page three'):
                document.add_page_break()
                document.add_paragraph(label)
            second = document.sections.add()
            document.add_paragraph('Page four')
            copied = second.header('first').paragraphs[0]
            before_sections = section_xml(document)
            for variant in ('default', 'first', 'even'):
                section.header(variant).clear()
            self.assertEqual(section_xml(document), before_sections)
            self.assertEqual(copied.text, 'first visible')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(len(pdf), 4)
                for i, variant in enumerate(('first', 'even', 'default')):
                    self.assertNotIn('visible', pdf[i].get_text())
                    self.assertIn(variant + ' footer', pdf[i].get_text())
                self.assertIn('first visible', pdf[3].get_text())
            with self.assertRaises(ValueError):
                del document.bookmarks['Target']
            second.header('first').clear()
            del document.bookmarks['Target']
            self.assertEqual(target.text, 'Target')

    def test_shared_media_reclamation_and_note_lifetime_errors(self):
        png = red_picture(3, 2)
        with doc_ex.new('docx') as document:
            body = document.add_paragraph('Anchor')
            note = document.notes.add(body[:], 'Note retained')
            header = document.sections[0].header()
            footer = document.sections[0].footer()
            document.background_image = png
            body_picture = document.add_image(png, width=20)
            header.add_image(png, width=20)
            footer_picture = footer.add_image(png, width=20)
            header.clear()
            self.assertEqual(media_parts(document), [png])
            self.assertEqual(footer_picture.data, png)
            self.assertEqual(body_picture.data, png)
            footer.clear()
            del document.images[0]
            self.assertEqual(media_parts(document), [png])
            document.background_image = None
            self.assertEqual(media_parts(document), [])
            before = document.to_bytes()
            selected = body[:]
            document.render_page(0, dpi=72)
            cached = document._cached
            with self.assertRaises(UnsupportedFormatError):
                note.clear()
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(selected.text, body.text)
            self.assertIs(document._cached, cached)
            self.assertEqual(note.paragraphs[0].text, 'Note retained')
            document.close()
            for story in (header, footer, note):
                with self.assertRaises(DocumentClosedError):
                    story.clear()
