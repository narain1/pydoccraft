"""Combined table fixtures: native semantics, actual pixels and reopened PDF."""
import unittest

import doc_ex
from test_authoring import image_fixture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def build_case(document, name):
    document.sections[0].margins = 24
    document.styles.format_defaults(font_size=11, space_after=0)
    document.background = '#F4F8FC'
    document.add_paragraph(name + 'Report', style='Heading 1')
    repeated = []
    if name == 'Invoice':
        labels = ['ItemCode', 'AmountCode', 'ServiceAlpha', 'USD100', 'ServiceBeta', 'USD200']
        table = document.add_table([labels[i:i + 2] for i in range(0, len(labels), 2)])
    elif name == 'Merged':
        labels = ['MergedHeading', 'LeftField', 'RightField']
        table = document.add_table([[labels[0], ''], labels[1:]])
        table.merge((0, 0), (0, 1))
    elif name == 'Nested':
        labels = ['OuterCell', 'PeerCell', 'NestedFirst', 'NestedSecond', 'AfterNested']
        table = document.add_table([labels[:2]])
        nested = table[0, 0].add_table([[labels[2]], [labels[3]]])
        nested.format(layout='fixed', width=130)
        nested[1, 0].background = '#00FF00'
        table[0, 0].add_paragraph(labels[4])
    elif name == 'Repeated':
        repeated = ['RepeatHeading', 'RepeatValue']
        labels = [f'RowEntry{n:02}' for n in range(24)]
        table = document.add_table([repeated] + [[label, str(n)] for n, label in enumerate(labels)])
        table.row(0).repeat_header = True
        for index in range(25):
            table.row(index).height = (24, 'at_least')
            table.row(index).allow_split = False
    elif name == 'RTL':
        labels = ['LogicalFirst', 'LogicalSecond', 'FirstData', 'SecondData']
        table = document.add_table([labels[:2], labels[2:]])
        table.format(rtl=True)
    elif name == 'Pictures':
        labels = ['PictureCell', 'PicturePeer', 'AfterPicture']
        table = document.add_table([labels[:2]])
        table[0, 0].add_image(image_fixture(), width=64, alt='Blue rectangle')
        table[0, 0].add_paragraph(labels[2])
    else:
        raise AssertionError(name)
    table.format(layout='fixed', width=312)
    table.borders = ('navy', 1)
    table[0, 0].background = '#FF0000'
    if name != 'Merged':
        table[0, 1].background = '#0000FF'
    return table, labels, repeated


def colored_xs(png, color):
    width, pixels = rgba_pixels(png_chunks(png))
    return [index % width for index in range(len(pixels) // 4)
            if pixels[index * 4:index * 4 + 3] == color]


class TableCorpusTests(unittest.TestCase):
    def test_table_fixture_pack_preserves_semantics_and_visible_export(self):
        for name in ('Invoice', 'Merged', 'Nested', 'Repeated', 'RTL', 'Pictures'):
            with self.subTest(fixture=name), doc_ex.new('docx', size=(360, 420)) as document:
                table, labels, repeated = build_case(document, name)
                with doc_ex.open(document.to_bytes()) as native, doc_ex.open(document.convert('pdf')) as pdf:
                    self.assertEqual(native.page_count, pdf.page_count)
                    self.assertGreaterEqual(native.page_count, 2 if name == 'Repeated' else 1)
                    self.assertTrue(any(page.find_tables() for page in native))
                    native_text = native.to_markdown()
                    pdf_pages = [page.get_text() for page in pdf]
                    pdf_text = '\n'.join(pdf_pages)
                    for label in labels:
                        self.assertEqual(native_text.count(label), 1, (name, label, native_text))
                        self.assertEqual(pdf_text.count(label), 1, (name, label, pdf_text))
                    for label in repeated:
                        self.assertEqual(native_text.count(label), 1)
                        self.assertTrue(all(page.count(label) == 1 for page in pdf_pages))
                    for source in (native, pdf):
                        for page_index in range(source.page_count):
                            png = source.render_page(page_index, dpi=72)
                            self.assertGreater(len(colored_xs(png, b'\xff\x00\x00')), 20)
                        png = source.render_page(0, dpi=72)
                        if name == 'RTL':
                            red = colored_xs(png, b'\xff\x00\x00')
                            blue = colored_xs(png, b'\x00\x00\xff')
                            self.assertGreater(min(red), max(blue))
                        if name == 'Nested':
                            self.assertGreater(len(colored_xs(png, b'\x00\xff\x00')), 20)
                        if name == 'Pictures':
                            self.assertGreater(len(colored_xs(png, b'\x20\x60\xc0')), 100)
                    if name == 'Merged':
                        self.assertEqual(native[0].find_tables()[0].cells[0][0].col_span, 2)
                    if name == 'RTL':
                        self.assertEqual(native[0].find_tables()[0].rows, table.rows)


if __name__ == '__main__':
    unittest.main()
