"""One combined inline-letterhead recipe, native reflow and variant parity gate."""
from collections import Counter
from io import BytesIO
import math
import struct
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from test_authoring import image_fixture
from test_cookbook import recipe
from test_picture_media import red_picture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
R = '{http://schemas.openxmlformats.org/officeDocument/2006/relationships}'
RED = b'\xff\0\0\xff'
BLUE = bytes((32, 96, 192, 255))
MARKERS = {'first': bytes((0, 128, 0, 255)), 'even': bytes((128, 0, 128, 255)),
           'odd': bytes((0, 128, 128, 255))}


class LetterheadCompositionTests(unittest.TestCase):
    def check_header(self, png, dpi, logo, variant):
        chunks = png_chunks(png)
        width, height = struct.unpack('>II', chunks[b'IHDR'][:8])
        self.assertEqual((width, height), (math.ceil(300 * dpi / 72), math.ceil(64 * dpi / 72)))
        xppm, yppm, unit = struct.unpack('>IIB', chunks[b'pHYs'])
        self.assertEqual((xppm, unit), (yppm, 1))
        self.assertAlmostEqual(xppm * 0.0254, dpi, delta=0.013)
        _, pixels = rgba_pixels(chunks)
        positions = {color: [] for color in (RED, BLUE)}
        marker_counts = Counter()
        for index in range(0, len(pixels), 4):
            color = pixels[index:index + 4]
            if color in positions:
                positions[color].append(index // 4)
            red, green, blue, alpha = color
            if alpha == 255:
                # Antialiasing mixes these header hues with white; small glyphs
                # need not contain a pixel at the exact source color.
                if red == blue and green - red > 40:
                    marker_counts['first'] += 1
                if red == blue and red - green > 40:
                    marker_counts['even'] += 1
                if green == blue and green - red > 40:
                    marker_counts['odd'] += 1
        for color in (RED, BLUE):
            points = positions[color]
            if color != logo:
                self.assertEqual(points, [])
                continue
            xs, ys = [p % width for p in points], [p // width for p in points]
            scale = dpi / 72
            # The opaque native image frame occupies exactly x24,y12,w36,h18pt.
            self.assertEqual((min(xs), min(ys), max(xs) + 1, max(ys) + 1),
                             tuple(round(v * scale) for v in (24, 12, 60, 30)))
            self.assertEqual(len(points), round(36 * scale) * round(18 * scale))
        for name, color in MARKERS.items():
            if name == variant:
                self.assertGreater(marker_counts[name], 8 * (dpi / 72) ** 2)
            else:
                self.assertEqual(marker_counts[name], 0)

    def test_first_logo_tracks_sections_through_reflow_and_numbered_parity(self):
        logos = red_picture(8, 4), image_fixture(8, 4)
        with recipe('letterhead').build_letterhead(*logos) as document:
            first, second = document.sections
            first_logo = first.header('first').images[0]
            second_logo = second.header('first').images[0]
            self.assertEqual((first_logo.data, second_logo.data), logos)
            self.assertNotEqual(first_logo.alt, second_logo.alt)
            self.assertEqual(first.header('first').paragraphs[0].text, 'Main first')
            # These scopes copied independently before the second logo/text was replaced.
            self.assertEqual(second.header('first').paragraphs[0].text, 'Appendix first')
            body = document.paragraphs[0]
            warm = document.render_page(0, dpi=72)
            for expanded in (False, True):
                with self.subTest(expanded=expanded):
                    inserted = [f'Inserted item {i:03}' for i in range(32)] if expanded else []
                    if expanded:
                        body[0:0].text = '\n'.join(inserted) + '\n'
                        self.assertNotEqual(document.render_page(0, dpi=72), warm)
                    source_bytes = document.to_bytes()
                    with ZipFile(BytesIO(source_bytes)) as archive:
                        root = ET.fromstring(archive.read('word/document.xml'))
                        sections = root.findall('.//' + W + 'sectPr')
                        self.assertEqual(len(sections), 2)
                        ids = []
                        for section in sections:
                            self.assertIsNotNone(section.find(W + 'titlePg'))
                            refs = section.findall(W + 'headerReference') + section.findall(W + 'footerReference')
                            self.assertEqual(Counter((r.tag, r.attrib[W + 'type']) for r in refs),
                                             Counter((W + kind + 'Reference', variant) for kind in ('header', 'footer')
                                                     for variant in ('default', 'first', 'even')))
                            ids.append({r.attrib[R + 'id'] for r in refs})
                        self.assertTrue(ids[0].isdisjoint(ids[1]))
                        settings = ET.fromstring(archive.read('word/settings.xml'))
                        self.assertIsNotNone(settings.find(W + 'evenAndOddHeaders'))
                    with doc_ex.open(source_bytes) as source, doc_ex.open(document.convert('pdf')) as pdf:
                        main_pages = 5 if expanded else 3
                        self.assertEqual((len(source), len(pdf)), (main_pages + 3,) * 2)
                        self.assertEqual([(p.metadata['section'], p.metadata['display_number']) for p in source],
                                         [(0, i + 2) for i in range(main_pages)] + [(1, 20), (1, 21), (1, 22)])
                        # Physical page2 is displayed page3, so it uses the odd/default slot.
                        self.assertEqual(source[1].metadata['display_number'], 3)
                        body_block = next(b for b in source[0].blocks if 'Main item 000' in b.text)
                        self.assertEqual({line['page'] for line in body_block.metadata['layout']['lines']},
                                         set(range(3 if expanded else 2)))
                        expected_labels = inserted + [f'Main item {i:03}' for i in range(24)] + [
                            'Project guide', 'Item label', 'Value label', 'Table alpha', 'Table beta',
                            'End of main section'] + [f'Appendix item {i:03}' for i in range(54)]
                        for output in (source, pdf):
                            text = '\n'.join(page.get_text() for page in output)
                            for label in expected_labels:
                                self.assertEqual(text.count(label), 1, label)
                            self.assertEqual([text.index(label) for label in expected_labels],
                                             sorted(text.index(label) for label in expected_labels))
                            links = [link for page in output for link in page.get_links()
                                     if link.uri == 'https://example.com/letterhead']
                            self.assertEqual(len(links), 1)
                            self.assertEqual(links[0].text, 'Project guide')
                        source_body = '\n'.join(page.get_text() for page in source)
                        for prefix in ('Main', 'Appendix'):
                            for variant in ('first', 'even', 'odd'):
                                self.assertNotIn(f'{prefix} {variant}', source_body)
                        self.assertEqual([table.rows for page in source for table in page.find_tables()],
                                         [[['Item label', 'Value label'], ['Table alpha', 'Table beta']]])
                        # The parity filler belongs to the preceding section and has no body text.
                        filler = main_pages - 1
                        self.assertEqual(source[filler].get_text().strip(), '')
                        self.assertEqual(pdf[filler].get_text().split(),
                                         ['Main', 'even', 'Main', 'even', 'footer', str(main_pages + 1)])
                        self.assertEqual(sum('Main first' in story.get_text() for story in source.stories), 2)
                        for page in range(len(source)):
                            section = 'Main' if page < main_pages else 'Appendix'
                            first_page = page in (0, main_pages)
                            number = source[page].metadata['display_number']
                            variant = 'first' if first_page else 'even' if number % 2 == 0 else 'odd'
                            text = pdf[page].get_text()
                            self.assertEqual(text.count(f'{section} {variant}\n'), 1)
                            self.assertIn(f'{section} {variant} footer {number}', text)
                            for other_section in ('Main', 'Appendix'):
                                for other_variant in ('first', 'even', 'odd'):
                                    if (other_section, other_variant) != (section, variant):
                                        self.assertNotIn(f'{other_section} {other_variant}', text)
                            expected_logo = RED if page == 0 else BLUE if page == main_pages else None
                            for output in (source, pdf):
                                full = output.render_page(page, dpi=72)
                                chunks = png_chunks(full)
                                self.assertEqual(struct.unpack('>II', chunks[b'IHDR'][:8]), (300, 360))
                                # Full-page control proves the logo does not recur elsewhere.
                                _, pixels = rgba_pixels(chunks)
                                counts = Counter(pixels[i:i + 4] for i in range(0, len(pixels), 4))
                                footer_ink = sum(max(pixels[i:i + 3]) < 160 for i in range(330 * 300 * 4, len(pixels), 4))
                                self.assertGreater(footer_ink, 40)
                                for color in (RED, BLUE):
                                    self.assertEqual(counts[color], 648 if color == expected_logo else 0)
                                for dpi in (72, 144, 300):
                                    self.check_header(output.render_page(page, dpi=dpi, clip=(0, 0, 300, 64)),
                                                      dpi, expected_logo, variant)
                    self.assertEqual((first_logo.data, second_logo.data), logos)
                    self.assertEqual((first_logo.size, second_logo.size), ((36, 18), (36, 18)))
