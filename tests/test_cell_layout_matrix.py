"""Combined native cell layout acceptance; no format-writing or layout fallback."""
from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

NS = {'w': 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}
W = '{' + NS['w'] + '}'
VERTICALS = ('top', 'center', 'bottom')
ALIGNMENTS = ('left', 'center', 'right')
DIRECTIONS = ('horizontal', 'down', 'up')
MARGINS = (8, 10, 12, 14)


def fill_color(row, column):
    return bytes((200 + row * 10, 210 + column * 10, 240, 255))


def build_matrix(direction, *, rtl=False, paragraph_rtl=False, empty=False, verticals=VERTICALS):
    document = doc_ex.new('docx', size=(384, 480 if len(verticals) > 1 else 240))
    document.sections[0].margins = 30
    document.styles.format_defaults(font_size=12, space_before=0, space_after=0)
    labels = [['' if empty else f'MARK{r}{c}' for c in range(3)] for r in range(len(verticals))]
    table = document.add_table(labels)
    table.format(layout='fixed', width=324, alignment='center', rtl=rtl)
    table.borders = ('navy', 1)
    for row, vertical in enumerate(verticals):
        table.row(row).height = (100, 'exact')
        for column, alignment in enumerate(ALIGNMENTS):
            cell = table[row, column]
            cell.format(margins=MARGINS, alignment=alignment, vertical_alignment=vertical,
                        text_direction=direction, background='#' + fill_color(row, column)[:3].hex())
            cell.paragraphs[0].format(direction='rtl' if paragraph_rtl else 'ltr')
            if not empty:
                cell.paragraphs[0][:].format(color='red')
    return document, table, labels


def source_table(data):
    with ZipFile(BytesIO(data)) as archive:
        return ET.fromstring(archive.read('word/document.xml')).find('.//w:tbl', NS)


def frames_and_ink(png, row_count):
    width, pixels = rgba_pixels(png_chunks(png))
    stride = width * 4
    ink = [(i % width, i // width) for i, (red, green, blue) in
           enumerate(zip(pixels[0::4], pixels[1::4], pixels[2::4]))
           if red > 150 and green < 100 and blue < 100]
    frames, glyphs, assigned = {}, {}, 0
    for row in range(row_count):
        for column in range(3):
            color = fill_color(row, column)
            first, last = pixels.find(color), pixels.rfind(color)
            if first < 0 or first % 4 or last % 4:
                raise AssertionError('missing exact native cell fill')
            top, bottom = first // stride, last // stride
            scanline = pixels[top * stride:(top + 1) * stride]
            left, right = scanline.find(color) // 4, scanline.rfind(color) // 4
            frame = (left, top, right, bottom)
            local = [(x, y) for x, y in ink if left <= x <= right and top <= y <= bottom]
            assigned += len(local)
            frames[row, column] = frame
            glyphs[row, column] = None if not local else (
                min(x for x, _ in local), min(y for _, y in local),
                max(x for x, _ in local), max(y for _, y in local))
    if assigned != len(ink):
        raise AssertionError('text ink escaped the visible cell frames')
    return frames, glyphs


class CellLayoutMatrixTests(unittest.TestCase):
    def assert_source(self, data, labels, direction, rtl, paragraph_rtl):
        table = source_table(data)
        bidi = table.find('w:tblPr/w:bidiVisual', NS)
        self.assertEqual(bidi is not None and bidi.get(W + 'val', '1') != '0', rtl)
        for row, node in enumerate(table.findall('w:tr', NS)):
            self.assertEqual(node.find('w:trPr/w:trHeight', NS).get(W + 'val'), '2000')
            for column, cell in enumerate(node.findall('w:tc', NS)):
                self.assertEqual(''.join(cell.itertext()), labels[row][column])
                properties = cell.find('w:tcPr', NS)
                self.assertEqual(properties.find('w:textDirection', NS).get(W + 'val'),
                                 dict(horizontal='lrTb', down='tbRl', up='btLr')[direction])
                self.assertEqual(properties.find('w:vAlign', NS).get(W + 'val'), VERTICALS[row])
                margins = {side.tag.removeprefix(W): int(side.get(W + 'w'))
                           for side in properties.find('w:tcMar', NS)}
                self.assertEqual(margins, dict(top=160, right=200, bottom=240, left=280))
                self.assertEqual(cell.find('w:p/w:pPr/w:jc', NS).get(W + 'val'), ALIGNMENTS[column])
                self.assertEqual(cell.find('w:p/w:pPr/w:bidi', NS).get(W + 'val', '1'), '1' if paragraph_rtl else '0')

    def assert_frames(self, frames, glyphs, direction, rtl, paragraph_rtl, dpi):
        scale = dpi / 72
        relative = {}
        for (row, column), frame in frames.items():
            ink = glyphs[row, column]
            self.assertIsNotNone(ink)
            left, top, right, bottom = frame
            # Solid fill lies inside the two half-width 1pt border strokes;
            # antialiasing can consume up to two additional edge pixels.
            self.assertAlmostEqual(right - left + 1, 107 * scale, delta=2)
            # Pinned native exact height includes bottom padding:100+12=112pt.
            # The visible interior excludes its1pt horizontal border strokes.
            self.assertAlmostEqual(bottom - top + 1, 111 * scale, delta=2)
            if row:
                self.assertAlmostEqual(top - frames[row - 1, column][1], 112 * scale, delta=2)
            physical_left, physical_right = (10, 14) if rtl else (14, 10)
            for actual, lower in ((ink[0] - left, physical_left - 2),
                                  (right - ink[2], physical_right - 2),
                                  (ink[1] - top, 8 - 2), (bottom - ink[3], 12 - 2)):
                self.assertGreaterEqual(actual / scale, lower)
            width, height = ink[2] - ink[0], ink[3] - ink[1]
            self.assertGreater(width if direction == 'horizontal' else height,
                               3 * (height if direction == 'horizontal' else width))
            relative[row, column] = ((ink[0] - left) / scale, (ink[1] - top) / scale)
        # Table direction mirrors physical columns without changing source identities.
        columns = [frames[0, column][0] for column in range(3)]
        self.assertEqual(columns, sorted(columns, reverse=rtl))
        # Paragraph alignment runs along the writing direction, independently of
        # vertical alignment, which moves across a quarter-turned cell.
        along = 0 if direction == 'horizontal' else 1
        ascending = (direction != 'up') != paragraph_rtl
        for row in range(3):
            values = [relative[row, column][along] for column in range(3)]
            for first, second in zip(values, values[1:]):
                self.assertGreater((second - first) if ascending else (first - second), 10)
        across = 1 if direction == 'horizontal' else 0
        for column in range(3):
            values = [relative[row, column][across] for row in range(3)]
            for first, second in zip(values, values[1:]):
                self.assertGreater((first - second) if direction == 'down' else (second - first), 20)

    def test_padding_alignment_orientation_and_rtl_matrix_at_three_dpis(self):
        for direction in DIRECTIONS:
            for rtl in (False, True):
                with self.subTest(direction=direction, rtl=rtl):
                    document, table, labels = build_matrix(direction, rtl=rtl, paragraph_rtl=rtl)
                    with document:
                        data, pdf_data = document.to_bytes(), document.convert('pdf')
                        self.assert_source(data, labels, direction, rtl, rtl)
                        self.assertEqual(table.rows, labels)
                        with doc_ex.open(data) as native, doc_ex.open(pdf_data) as pdf:
                            self.assertEqual((native.page_count, pdf.page_count), (1, 1))
                            self.assertEqual(native[0].find_tables()[0].rows, labels)
                            for label in sum(labels, []):
                                self.assertEqual(native.to_markdown().count(label), 1)
                                self.assertEqual(pdf.to_markdown().count(label), 1)
                            for dpi in (72, 144, 300):
                                native_frames, native_ink = frames_and_ink(native.render_page(0, dpi=dpi), 3)
                                pdf_frames, pdf_ink = frames_and_ink(pdf.render_page(0, dpi=dpi), 3)
                                self.assert_frames(native_frames, native_ink, direction, rtl, rtl, dpi)
                                self.assert_frames(pdf_frames, pdf_ink, direction, rtl, rtl, dpi)
                                for key in native_frames:
                                    for a, b in zip(native_frames[key] + native_ink[key], pdf_frames[key] + pdf_ink[key]):
                                        self.assertLessEqual(abs(a - b), 2)

    def test_paragraph_direction_is_independent_of_rtl_column_order(self):
        document, table, labels = build_matrix('horizontal', rtl=True, verticals=('top',))
        with document:
            before = {dpi: frames_and_ink(document.render_page(0, dpi=dpi), 1) for dpi in (72, 144, 300)}
            for column in range(3):
                table[0, column].paragraphs[0].format(direction='rtl')
            self.assertEqual(table.rows, labels)
            self.assertTrue(table.formatting['rtl'])
            with doc_ex.open(document.to_bytes()) as native, doc_ex.open(document.convert('pdf')) as pdf:
                for dpi in (72, 144, 300):
                    prior_frames, prior_ink = before[dpi]
                    for output in (native, pdf):
                        frames, ink = frames_and_ink(output.render_page(0, dpi=dpi), 1)
                        self.assertEqual(frames, prior_frames)
                        self.assertGreater(ink[0, 0][0] - prior_ink[0, 0][0], 20 * dpi / 72)
                        self.assertLess(ink[0, 2][0] - prior_ink[0, 2][0], -20 * dpi / 72)
                        self.assertLessEqual(abs(ink[0, 1][0] - prior_ink[0, 1][0]), 1)

    def test_empty_cells_keep_geometry_and_formatting_without_spacing_alias(self):
        document, table, labels = build_matrix('horizontal', rtl=True, empty=True)
        with document:
            for row in range(3):
                for column, direction in enumerate(DIRECTIONS):
                    table[row, column].format(text_direction=direction)
            data = document.to_bytes()
            self.assertEqual(table.rows, labels)
            xml = source_table(data)
            self.assertEqual(len(xml.findall('.//w:tc', NS)), 9)
            self.assertEqual(len(xml.findall('.//w:tc/w:p', NS)), 9)
            self.assertEqual(''.join(xml.itertext()), '')
            before = {}
            with doc_ex.open(data) as native, doc_ex.open(document.convert('pdf')) as pdf:
                self.assertEqual(native[0].find_tables()[0].rows, labels)
                self.assertFalse(pdf.to_markdown().strip())
                for dpi in (72, 144, 300):
                    frames, ink = frames_and_ink(native.render_page(0, dpi=dpi), 3)
                    pdf_frames, pdf_ink = frames_and_ink(pdf.render_page(0, dpi=dpi), 3)
                    self.assertTrue(all(value is None for value in ink.values()))
                    self.assertTrue(all(value is None for value in pdf_ink.values()))
                    self.assertEqual(frames, pdf_frames)
                    before[dpi] = frames
            with document.copy() as copied:
                for row in range(3):
                    for column in range(3):
                        cell = copied.tables[0][row, column]
                        self.assertEqual(cell.margins, MARGINS)
                        self.assertEqual(cell.formatting['text_direction'], DIRECTIONS[column])
                        self.assertEqual(cell.vertical_alignment, VERTICALS[row])
                        cell.paragraphs[0].add_run(f'ADDED{row}{column}', color='red')
                for dpi in (72, 144, 300):
                    frames, ink = frames_and_ink(copied.render_page(0, dpi=dpi), 3)
                    self.assertEqual(frames, before[dpi])
                    self.assertTrue(all(value is not None for value in ink.values()))
                self.assertEqual(table.rows, labels)
            before_bytes = document.to_bytes()
            with self.assertRaises(ValueError):
                table.format(cell_spacing=4)
            self.assertEqual(document.to_bytes(), before_bytes)
            cell = table[0, 0]
            cell.format(margins=None, alignment=None, vertical_alignment=None, text_direction=None)
            for key in ('margins', 'alignment', 'vertical_alignment', 'text_direction'):
                self.assertIsNone(cell.formatting[key])
            first = source_table(document.to_bytes()).find('w:tr/w:tc', NS)
            for path in ('w:tcPr/w:tcMar', 'w:tcPr/w:vAlign', 'w:tcPr/w:textDirection', 'w:p/w:pPr/w:jc'):
                self.assertIsNone(first.find(path, NS))
            self.assertEqual(table.rows, labels)


if __name__ == '__main__':
    unittest.main()
