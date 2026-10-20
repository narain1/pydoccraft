"""Synthetic authored line numbering; native pagination owns labels and ordering."""
from io import BytesIO
import re
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'


def label_ink(png, dpi=72):
    width, pixels = rgba_pixels(png_chunks(png))
    # Only the left page margin: all body text starts at72pt in these fixtures.
    scale = dpi / 72
    return sum(max(pixels[(y * width + x) * 4:(y * width + x) * 4 + 3]) < 220
               for y in range(int(65 * scale), int(250 * scale))
               for x in range(int(25 * scale), int(65 * scale)))


def source_numbering(data):
    with ZipFile(BytesIO(data)) as archive:
        root = ET.fromstring(archive.read('word/document.xml'))
    return [None if node.find(f'{{{W}}}lnNumType') is None else
            {key.split('}', 1)[1]: value for key, value in node.find(f'{{{W}}}lnNumType').attrib.items()}
            for node in root.findall(f'.//{{{W}}}sectPr')]


class LineNumberTests(unittest.TestCase):
    def test_line_labels_and_rich_story_geometry_share_output_validation(self):
        from test_picture_media import count_color, red_picture
        with doc_ex.new('docx') as document:
            section = document.sections[0]
            header = section.header().add_table([['Report header']])
            header.row(0).allow_split = False
            section.footer().add_image(red_picture(2, 2), width=20)
            document.add_paragraph('Body alpha')
            document.add_paragraph('Body bravo')
            section.line_numbers = (1, 1, 0, 'each_page')
            baseline = document.render_page(0, dpi=72)
            self.assertGreater(label_ink(baseline), 20)
            self.assertGreater(count_color(baseline, bytes((255, 0, 0, 255))), 300)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened.render_page(0, dpi=72), baseline)
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertIn('Report header', pdf[0].get_text())
                self.assertGreater(label_ink(pdf.render_page(0, dpi=72)), 20)
            section.line_numbers = (1, 1, 1000, 'each_page')
            with self.assertRaisesRegex(UnsupportedFormatError, 'line-number extent'):
                document.to_bytes()
            section.line_numbers = (1, 1, 0, 'each_page')
            header[0, 0].text = 'Overflow ' * 2000
            with self.assertRaisesRegex(UnsupportedFormatError, 'physical page'):
                document.to_bytes()
            section.line_numbers = None
            with self.assertRaisesRegex(UnsupportedFormatError, 'physical page'):
                document.to_bytes()
            header[0, 0].text = 'Report header'
            section.line_numbers = (1, 1, 0, 'each_page')
            self.assertEqual(document.render_page(0, dpi=72), baseline)

    def test_source_png_pdf_copy_and_clear_use_native_labels(self):
        with doc_ex.new('docx') as document:
            section = document.sections[0]
            self.assertIsNone(section.line_numbers)
            for text in ('Alpha', 'Bravo', 'Charlie'):
                document.add_paragraph(text)
            baseline = document.render_page(0, dpi=72)
            self.assertEqual(label_ink(baseline), 0)
            section.line_numbers = (1, 1, 0, 'each_page')
            self.assertEqual(section.line_numbers, (1, 1, 0.0, 'each_page'))
            numbered = document.render_page(0, dpi=72)
            self.assertGreater(label_ink(numbered), 20)
            data = document.to_bytes()
            self.assertEqual(source_numbering(data), [{'countBy': '1', 'start': '0', 'restart': 'newPage'}])
            with doc_ex.open(data) as reopened:
                self.assertEqual(reopened.render_page(0, dpi=72), numbered)
                self.assertEqual(reopened[0].get_text(), 'Alpha\nBravo\nCharlie')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(pdf.page_count, 1)
                self.assertEqual(pdf[0].get_text().split(), ['1', 'Alpha', '2', 'Bravo', '3', 'Charlie'])
                self.assertGreater(label_ink(pdf.render_page(0, dpi=72)), 20)
            with document.copy() as copied:
                section.line_numbers = None
                self.assertEqual(document.render_page(0, dpi=72), baseline)
                self.assertEqual(source_numbering(document.to_bytes()), [None])
                self.assertEqual(copied.sections[0].line_numbers, (1, 1, 0.0, 'each_page'))
                self.assertEqual(copied.render_page(0, dpi=72), numbered)

    def test_restart_intervals_sections_and_fields_remain_native(self):
        for restart in ('each_page', 'each_section', 'continuous'):
            with self.subTest(restart=restart), doc_ex.new('docx') as document:
                first = document.sections[0]
                first.line_numbers = (1, 7, 12.5, restart)
                paragraph = document.add_paragraph('First page=')
                paragraph.add_field('page')
                second = document.sections.add()
                document.add_paragraph('Second')
                self.assertEqual(second.line_numbers, first.line_numbers)
                with doc_ex.open(document.convert('pdf')) as pdf:
                    self.assertEqual(pdf.page_count, 2)
                    self.assertEqual(pdf[0].get_text().split()[0], '7')
                    self.assertEqual(pdf[1].get_text().split()[0], '8' if restart == 'continuous' else '7')
                    self.assertIn('page=1', re.sub(r'=\s+', '=', pdf[0].get_text()))
                self.assertEqual(source_numbering(document.to_bytes())[0]['distance'], '250')
        with doc_ex.new('docx') as document:
            document.sections[0].line_numbers = (2, 5, 0, 'each_section')
            for text in ('Alpha', 'Bravo', 'Charlie'):
                document.add_paragraph(text)
            with doc_ex.open(document.convert('pdf')) as pdf:
                tokens = pdf[0].get_text().split()
                self.assertEqual([token for token in tokens if token.isdecimal()], ['6'])
            continuous = document.sections.add(start='continuous')
            continuous.line_numbers = (1, 20, 0, 'each_section')
            document.add_paragraph('Delta')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(pdf.page_count, 1)
                self.assertIn('20', pdf[0].get_text().split())

    def test_automatic_multipage_restart_and_continuation(self):
        for restart in ('each_page', 'continuous'):
            with self.subTest(restart=restart), doc_ex.new('docx', size=(300, 260)) as document:
                document.sections[0].line_numbers = (1, 1, 0, restart)
                for index in range(15):
                    document.add_paragraph(f'Paragraph{index}')
                with doc_ex.open(document.convert('pdf')) as pdf:
                    self.assertGreater(pdf.page_count, 1)
                    groups = [[int(token) for token in page.get_text().split() if token.isdecimal()] for page in pdf]
                    self.assertEqual(sum(map(len, groups)), 15)
                    if restart == 'each_page':
                        for group in groups:
                            self.assertEqual(group, list(range(1, len(group) + 1)))
                    else:
                        # Native PDF reading order puts the wider two-digit '10' before
                        # same-page one-digit labels. Native layout tests separately
                        # assert strict y/order; extraction must retain each page's values.
                        self.assertEqual([value for group in groups for value in sorted(group)], list(range(1, 16)))

    def test_invalid_configuration_column_edits_and_work_limits_are_atomic(self):
        with doc_ex.new('docx') as document:
            section = document.sections[0]
            paragraph = document.add_paragraph('Kept')
            section.line_numbers = (1, 1, 0, 'each_page')
            before, png = document.to_bytes(), document.render_page(0, dpi=72)
            for value, error in [(True, TypeError), ((1, 1, 0), TypeError),
                ((True, 1, 0, 'each_page'), TypeError), ((1, False, 0, 'each_page'), TypeError),
                ((1.0, 1, 0, 'each_page'), TypeError), ((0, 1, 0, 'each_page'), ValueError),
                ((101, 1, 0, 'each_page'), ValueError), ((1, 32769, 0, 'each_page'), ValueError),
                ((1, 1, -1, 'each_page'), ValueError), ((1, 1, float('inf'), 'each_page'), ValueError),
                ((1, 1, 0, 'page'), ValueError)]:
                with self.subTest(value=value), self.assertRaises(error):
                    section.line_numbers = value
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), png)
            with self.assertRaises(UnsupportedFormatError):
                section.columns = (2, 36, False)
            self.assertEqual(section.columns, (1, 36.0, False))
            with self.assertRaises(UnsafeArchiveError):
                paragraph.text = 'x' * 25000
            self.assertEqual(paragraph.text, 'Kept')
            self.assertEqual(document.to_bytes(), before)
            section.line_numbers = None
            paragraph.text = 'x' * 25000
            document.add_table([['Candidate table measurement']])
            before = document.to_bytes()
            with self.assertRaises(UnsafeArchiveError):
                section.line_numbers = (100, 1, 0, 'each_page')
            self.assertEqual(document.to_bytes(), before)
            self.assertIsNone(section.line_numbers)

    def test_actual_extent_guard_allows_default_margins_and_rejects_offpage_output(self):
        with doc_ex.new('docx') as document:
            section = document.sections[0]
            document.add_paragraph('Geometry')
            section.line_numbers = (1, 32768, 0, 'each_page')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertIn('32768', pdf[0].get_text().split())
            before = document.to_bytes()
            section.line_numbers = (1, 32768, 1000, 'each_page')
            for operation in (document.to_bytes, lambda: document.render_page(0), lambda: document.convert('pdf')):
                with self.assertRaisesRegex(UnsupportedFormatError, 'physical page'):
                    operation()
            section.line_numbers = (1, 32768, 0, 'each_page')
            self.assertEqual(document.to_bytes(), before)

    def test_each_page_reenable_after_unnumbered_pages_is_explicitly_unsupported(self):
        with doc_ex.new('docx', size=(400, 350)) as document:
            document.sections[0].line_numbers = (1, 1, 0, 'each_page')
            document.add_paragraph('First')
            middle = document.sections.add()
            middle.line_numbers = None
            middle.columns = (2, 36, False)
            for index in range(24):
                document.add_paragraph(f'Middle{index}')
            final = document.sections.add(start='continuous')
            final.columns = (1, 36, False)
            document.add_paragraph('Last')
            before = document.to_bytes()
            with self.assertRaisesRegex(UnsupportedFormatError, 'each_page.*continuous'):
                final.line_numbers = (1, 1, 0, 'each_page')
            self.assertEqual(document.to_bytes(), before)
            final.line_numbers = (1, 1, 0, 'each_section')
            with doc_ex.open(document.convert('pdf')) as pdf:
                self.assertGreaterEqual(pdf.page_count, 3)
                self.assertEqual([token for token in pdf[-1].get_text().split() if token.isdecimal()], ['1'])

    def test_closed_owner_callback_does_not_mutate_a_replacement_target(self):
        document = doc_ex.new('docx')
        section = document.sections[0]
        class ClosingIndex:
            def __index__(self):
                document.close()
                return 1
        with self.assertRaises(DocumentClosedError):
            section.line_numbers = (ClosingIndex(), 1, 0, 'each_page')
        with self.assertRaises(DocumentClosedError):
            _ = section.line_numbers


if __name__ == '__main__':
    unittest.main()
