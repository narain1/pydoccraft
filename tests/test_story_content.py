from io import BytesIO
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_authoring import image_fixture
from test_picture_media import count_color, media_parts, red_picture


W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


class StoryContentTests(unittest.TestCase):
    def test_table_and_picture_first_preserve_order_rich_cells_and_native_output(self):
        red = red_picture(4, 3)
        with doc_ex.new('docx', size=(400, 500)) as document:
            section = document.sections[0]
            section.margins = 32
            section.header_distance = section.footer_distance = 12
            document.add_paragraph('Body content')
            for kind in ('header', 'footer'):
                story = getattr(section, kind)()
                before = document.to_bytes()
                self.assertEqual((len(story.tables), len(story.images), len(story.paragraphs)), (0, 0, 0))
                self.assertEqual(document.to_bytes(), before)
                picture = story.add_image(red, width=24, alt=kind + ' picture') if kind == 'footer' else None
                table = story.add_table([[kind + ' table']])
                nested = table[0, 0].add_table([['Nested ' + kind]])
                nested[0, 0].add_image(red, width=18)
                if picture is None:
                    picture = story.add_image(red, width=24, alt=kind + ' picture')
                paragraph = story.add_paragraph(kind + ' page ')
                paragraph.add_field('page')
                story.add_paragraph('')
                story.add_paragraph('After explicit empty')
                self.assertEqual(len(story.tables), 1)
                self.assertEqual(len(story.images), 1)
                self.assertEqual(len(story.paragraphs), 3)
                self.assertEqual(story.paragraphs[1].text, '')
                self.assertEqual(picture.data, red)
                self.assertEqual(table[0, 0].text.splitlines()[0], kind + ' table')
            self.assertEqual(len(document.images), 4)
            self.assertGreater(count_color(document.render_page(0, dpi=72), bytes((255, 0, 0, 255))), 500)
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                for name in archive.namelist():
                    if name.startswith(('word/header', 'word/footer')) and name.endswith('.xml'):
                        children = list(ET.fromstring(archive.read(name)))
                        table_index, image_index = (0, 1) if 'header' in name else (1, 0)
                        self.assertEqual(children[table_index].tag, W + 'tbl')
                        self.assertIsNotNone(children[image_index].find('.//' + W + 'drawing'))
                        self.assertIsNotNone(children[2].find('.//' + W + 'instrText'))
            with doc_ex.open(document.convert('pdf')) as pdf:
                text = pdf[0].get_text()
                for label in ('Body content', 'header table', 'footer table', 'Nested header', 'Nested footer'):
                    self.assertIn(label, text)
                self.assertGreater(count_color(pdf.render_page(0, dpi=72), bytes((255, 0, 0, 255))), 500)

    def test_variants_and_copied_sections_keep_independent_handles_and_placeholders(self):
        red = red_picture(4, 3)
        with doc_ex.new('docx', size=(360, 480)) as document:
            first = document.sections[0]
            first.margins = 32
            first.header_distance = first.footer_distance = 12
            for variant in ('default', 'first', 'even'):
                story = first.header(variant)
                story.add_table([[variant + ' heading']])
                story.add_image(red, width=20)
            footer_table = first.footer('first').add_table([['Footer original']])
            document.add_paragraph('Page one')
            document.add_page_break()
            document.add_paragraph('Page two')
            document.add_page_break()
            document.add_paragraph('Page three')
            second = document.sections.add()
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsupportedFormatError, 'geometry'):
                second.page_size = (420, 520)
            self.assertEqual(document.to_bytes(), before)
            second.header('first').tables[0][0, 0].text = 'Independent first'
            second.footer('first').add_paragraph('After copied table')
            document.add_paragraph('Page four')
            self.assertEqual(first.header('first').tables[0][0, 0].text, 'first heading')
            self.assertEqual(footer_table[0, 0].text, 'Footer original')
            self.assertEqual(len(first.footer('first').paragraphs), 0)
            self.assertEqual(len(second.footer('first').tables), 1)
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(len(pdf), 4)
                for i, expected in enumerate(('first heading', 'even heading', 'default heading', 'Independent first')):
                    self.assertIn(expected, pdf[i].get_text())
                    self.assertGreater(count_color(pdf.render_page(i, dpi=72), bytes((255, 0, 0, 255))), 200)
                self.assertNotIn('After copied table', pdf[0].get_text())
                self.assertIn('After copied table', pdf[3].get_text())
            with document.copy() as copied:
                copied.sections[0].header('first').tables[0][0, 0].text = 'Copy only'
                self.assertEqual(first.header('first').tables[0][0, 0].text, 'first heading')

    def test_export_colors_match_pdf_full_region_sessions_and_mixed_page_sizes(self):
        from doc_ex import _core, _native
        red, blue = red_picture(4, 3), image_fixture(4, 3)
        with doc_ex.new('docx', size=(360, 480)) as document:
            first = document.sections[0]
            first.margins = 32
            first.header_distance = first.footer_distance = 12
            for story in (first.header(), first.footer()):
                story.add_table([['Shared story table']])
                story.add_image(red, width=24)
            document.add_image(blue, width=24)
            document.add_paragraph('First body')
            second = document.sections.add()
            second.page_size = (420, 520)
            document.add_image(blue, width=24)
            document.add_paragraph('Second body')
            source = document.to_bytes()
            with doc_ex.open(source) as opened, doc_ex.open(document.convert('pdf')) as pdf:
                for index, (width, height) in enumerate(((360, 480), (420, 520))):
                    for clip in (None, (0, 0, width, height / 2), (0, height / 2, width, height / 2)):
                        retained = opened.render_page(index, dpi=72, clip=clip)
                        one_shot = bytes(_native(lambda: _core.render(source, 'docx', index, 72, None, clip)))
                        self.assertEqual(retained, one_shot)
                        # Exact saturated source red is retained in both headers and footers.
                        self.assertGreater(count_color(retained, bytes((255, 0, 0, 255))), 300)
                        self.assertEqual(count_color(retained, bytes((255, 0, 0, 255))),
                                         count_color(pdf.render_page(index, dpi=72, clip=clip), bytes((255, 0, 0, 255))))
                    # Body pixels keep their prior source colour independently of the header fix.
                    self.assertGreater(count_color(opened.render_page(index, dpi=72), bytes((32, 96, 192, 255))), 300)
                    self.assertEqual(count_color(opened.render_page(index, dpi=72), bytes((32, 96, 192, 255))),
                                     count_color(pdf.render_page(index, dpi=72), bytes((32, 96, 192, 255))))

    def test_media_replacement_deletion_and_cache_keep_other_references_alive(self):
        red, blue = red_picture(4, 4), image_fixture()
        with doc_ex.new('docx') as document:
            document.add_paragraph('Body')
            document.background_image = red
            story = document.sections[0].header()
            original = story.add_image(red, width=24, alt='Stable')
            cell = story.add_table([['Cell']])[0, 0]
            nested = cell.add_image(red, width=16)
            before = document.render_page(0, dpi=72)
            with document.copy() as copied:
                original.data = BytesIO(blue)
                self.assertNotEqual(document.render_page(0, dpi=72), before)
                self.assertEqual(original.alt, 'Stable')
                self.assertEqual(original.size, (24, 24))
                self.assertEqual(nested.data, red)
                self.assertEqual(copied.render_page(0, dpi=72), before)
                del story.images[0]
                with self.assertRaises(IndexError):
                    _ = original.data
                self.assertEqual(media_parts(document), [red])
                self.assertEqual([p.text for p in story.paragraphs], [''])
                story.add_paragraph('After retained anchor')
                self.assertEqual([p.text for p in story.paragraphs], ['', 'After retained anchor'])
                del cell.images[0]
                self.assertEqual(media_parts(document), [red])
                document.background_image = None
                self.assertEqual(media_parts(document), [])
                self.assertEqual(copied.sections[0].header().images[0].data, red)
                self.assertEqual(len(story.tables), 1)
                self.assertEqual(len(story.images), 0)

    def test_inactive_variant_measurement_and_invalid_operations_are_atomic(self):
        payload = red_picture(2, 2)
        with doc_ex.new('docx') as document:
            document.add_paragraph('One page')
            section = document.sections[0]
            unused = section.footer()
            before = document.to_bytes()
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                unused.add_image(b'not PNG')
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(len(unused.images), 0)
            even = section.header('even')
            table = even.add_table([['Hidden until an even page']])
            before = document.to_bytes()
            for operation in (lambda: table[0, 0].add_image(payload, width=10),
                              lambda: table[0, 0].add_table([['nested']])):
                with self.assertRaisesRegex(UnsupportedFormatError, 'geometry'):
                    operation()
                self.assertEqual(document.to_bytes(), before)
            document.add_page_break()
            document.add_paragraph('Even page')
            table[0, 0].add_image(payload, width=10)
            self.assertEqual(len(table[0, 0].images), 1)
            anchor = document.add_paragraph('Anchor')
            note = document.notes.add(anchor.find('Anchor')[0])
            before = document.to_bytes()
            for operation in (lambda: note.tables, lambda: note.images,
                              lambda: note.add_table([['No']]), lambda: note.add_image(payload)):
                with self.assertRaises(UnsupportedFormatError):
                    operation()
                self.assertEqual(document.to_bytes(), before)
            document.close()
            for operation in (lambda: note.tables, lambda: unused.images,
                              lambda: unused.add_table([['No']]), lambda: unused.add_image(payload)):
                with self.assertRaises(DocumentClosedError):
                    operation()
        closed = doc_ex.new('docx')
        story = closed.sections[0].header()
        class ClosingStream:
            sent = False
            def read(self, size):
                if self.sent:
                    return b''
                self.sent = True
                closed.close()
                return payload
        with self.assertRaises(DocumentClosedError):
            story.add_image(ClosingStream(), width=12)
        closed = doc_ex.new('docx')
        story = closed.sections[0].header()
        class ClosingRows:
            def __iter__(self):
                closed.close()
                yield ['Caller closes owner']
        with self.assertRaises(DocumentClosedError):
            story.add_table(ClosingRows())

    def test_physical_overflow_rejects_output_and_recovers_after_later_edits(self):
        with TemporaryDirectory() as directory, doc_ex.new('docx', size=(240, 320)) as document:
            document.add_paragraph('Body')
            header = document.sections[0].header()
            table = header.add_table([['Short']])
            table[0, 0].text = 'Overflow ' * 500
            path = Path(directory) / 'existing.docx'
            path.write_bytes(b'keep existing')
            for operation in (document.to_bytes, lambda: document.save(path),
                              lambda: document.render_page(0), lambda: document.convert('pdf')):
                with self.assertRaisesRegex(UnsupportedFormatError, 'physical page'):
                    operation()
                self.assertEqual(path.read_bytes(), b'keep existing')
            table[0, 0].text = 'Short'
            document.to_bytes()
            paragraph = table[0, 0].paragraphs[0]
            paragraph[:].format(font_size=160)
            with self.assertRaisesRegex(UnsupportedFormatError, 'physical page'):
                document.to_bytes()
            paragraph[:].format(font_size=12)
            document.to_bytes()
            picture = document.sections[0].footer().add_image(red_picture(2, 2), width=20, height=400)
            with self.assertRaisesRegex(UnsupportedFormatError, 'physical page'):
                document.to_bytes()
            picture.size = (20, 20)
            document.to_bytes()
            table.row(0).height = (20, 'exact')
            table[0, 0].add_image(red_picture(2, 2), width=10, height=400)
            # Intentional native clipping is allowed when the clip frame stays on-page.
            document.to_bytes()
            red = bytes((255, 0, 0, 255))
            visible = count_color(document.render_page(0, dpi=72), red)
            self.assertGreater(visible, 400)  # Footer plus a visible clipped strip.
            self.assertLess(visible, 1000)  # The 400-point source is not painted in full.
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(count_color(pdf.render_page(0, dpi=72), red), visible)
