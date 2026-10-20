"""Synthetic standard comments; drawing and appearance serialization stay native."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_pdf_markup import colored_pixels
from test_render_geometry import geometry_pdf, rgba_pixels
from test_rendering import png_chunks


def imported_shape(extra=b'', *, subtype='Square', color=b'[1 0 0]', border=b'<< /W 2 /S /S >>'):
    # Test-only PDF fixture: a standard dictionary with an existing indirect
    # normal appearance. Invalid profile variants must retain this appearance.
    appearance = b'1 0 0 RG 2 w 21 21 118 58 re S'
    objects = [b'<< /Type /Catalog /Pages 2 0 R >>',
               b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
               b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Annots [4 0 R] /Resources << >> >>',
               b'<< /Type /Annot /Subtype /' + subtype.encode() +
               b' /Rect [20 20 140 80] /C ' + color + b' /BS ' + border +
               b' /AP << /N 5 0 R >> /Contents (Original) /F 4 ' + extra + b' >>',
               b'<< /Type /XObject /Subtype /Form /BBox [20 20 140 80] /Resources << >> /Length ' +
               str(len(appearance)).encode() + b' >>\nstream\n' + appearance + b'\nendstream']
    output = bytearray(b'%PDF-1.7\n')
    offsets = []
    for index, value in enumerate(objects, 1):
        offsets.append(len(output))
        output.extend(f'{index} 0 obj\n'.encode() + value + b'\nendobj\n')
    xref = len(output)
    output.extend(f'xref\n0 {len(objects)+1}\n0000000000 65535 f \n'.encode())
    for offset in offsets:
        output.extend(f'{offset:010d} 00000 n \n'.encode())
    output.extend(f'trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode())
    return bytes(output)


class PdfGeometricAnnotationTests(unittest.TestCase):
    def test_visible_shapes_resize_recolor_reopen_copy_delete(self):
        for kind in ('rectangle', 'oval'):
            with self.subTest(kind=kind), doc_ex.new('pdf', size=(240, 240)) as document:
                page = document.pages[0]
                page.draw_text('Preserved body', at=(20, 200))
                baseline = document.render_page(0, dpi=72)
                mark = page.annotations.add(kind, box=(20, 20, 120, 60), color='red',
                                             text='Comment café', author='Author')
                initial = document.render_page(0, dpi=72)
                red = set(colored_pixels(initial, 0))
                self.assertGreater(len(red), 200)
                self.assertNotIn((80, 50), red)
                self.assertEqual((21, 21) in red, kind == 'rectangle')
                mark.update(box=(40, 100, 160, 60), color='blue', text='Revised ✓', author='Reviewer')
                self.assertEqual((mark.kind, mark.box, mark.text, mark.author, mark.color),
                                 (kind, (40, 100, 160, 60), 'Revised ✓', 'Reviewer', '#0000FF'))
                png = document.render_page(0, dpi=72)
                self.assertFalse(colored_pixels(png, 0))
                blue = colored_pixels(png, 2)
                self.assertGreater(len(blue), 200)
                self.assertTrue(all(40 <= x < 200 and 100 <= y < 160 for x, y in blue))
                width, pixels = rgba_pixels(png_chunks(png))
                _, original = rgba_pixels(png_chunks(baseline))
                self.assertEqual(pixels[180*width*4:], original[180*width*4:])
                with doc_ex.open(document.to_bytes()) as reopened:
                    self.assertEqual(reopened.render_page(0, dpi=72), png)
                    self.assertEqual(reopened[0].get_text().strip(), 'Preserved body')
                    with reopened.edit() as edited:
                        item = edited.pages[0].annotations[0]
                        self.assertEqual(item.kind, kind)
                        item.box = (30, 90, 140, 70)
                        self.assertEqual(item.box, (30, 90, 140, 70))
                with document.copy() as copied:
                    del page.annotations[0]
                    self.assertEqual(document.render_page(0, dpi=72), baseline)
                    self.assertEqual(copied.render_page(0, dpi=72), png)
                with self.assertRaisesRegex(ValueError, 'stale'):
                    _ = mark.box

    def test_rotated_crop_userunit_and_page_organization(self):
        for rotation in (0, 90, 180, 270):
            with self.subTest(rotation=rotation), doc_ex.open(geometry_pdf(rotation)) as source, source.edit() as document:
                baseline = document.render_page(0, dpi=72)
                page = document.pages[0]
                for row, kind in enumerate(('rectangle', 'oval')):
                    mark = page.annotations.add(kind, box=(20, 20+row*90, 120, 60), color='green')
                    mark.box = (30, 25+row*90, 140, 65)
                    for actual, expected in zip(mark.box, (30, 25+row*90, 140, 65)):
                        self.assertAlmostEqual(actual, expected, places=3)
                png = document.render_page(0, dpi=72)
                green = colored_pixels(png, 1)
                self.assertGreater(len(green), 400)
                self.assertTrue(all(30 <= x < 170 and (25 <= y < 90 or 115 <= y < 180) for x, y in green))
                document.pages.add(size=(200, 200))
                document.pages.move(0, 1)
                with self.assertRaisesRegex(ValueError, 'stale'):
                    _ = mark.kind
                self.assertEqual(document.render_page(1, dpi=72), png)
                del document.pages[0]
                fresh = document.pages[0]
                fresh.rotate(90)
                self.assertEqual([item.kind for item in fresh.annotations], ['rectangle', 'oval'])
                turned = fresh.annotations[0]
                turned.update(box=turned.box, color='blue')
                self.assertEqual(turned.color, '#0000FF')
                fresh.rotate(-90)
                del fresh.annotations[1]
                del fresh.annotations[0]
                self.assertEqual(document.render_page(0, dpi=72), baseline)

    def test_invalid_geometry_and_actions_preserve_warm_render(self):
        for kind in ('rectangle', 'oval'):
            with self.subTest(kind=kind), doc_ex.new('pdf', size=(200, 200)) as document:
                page = document.pages[0]
                mark = page.annotations.add(kind, box=(20, 20, 120, 60), text='Kept')
                before, png = document.to_bytes(), document.render_page(0, dpi=72)
                for box in ((20, 20, 2, 20), (20, 20, 20, 1), (20, 20, 2.01, 2.01), (20, 20, 2.99, 20), (-1, 20, 120, 60),
                            (20, 20, 200, 60), (20, 20, float('nan'), 60)):
                    for update in (False, True):
                        with self.subTest(box=box, update=update), self.assertRaises(ValueError):
                            if update:
                                mark.update(box=box, text='Do not commit')
                            else:
                                page.annotations.add(kind, box=box)
                        self.assertEqual(document.to_bytes(), before)
                        self.assertEqual(document.render_page(0, dpi=72), png)
                with self.assertRaises(UnsupportedFormatError):
                    mark.update(uri='https://example.com', text='Do not commit')
                self.assertEqual(mark.text, 'Kept')
                self.assertEqual(document.to_bytes(), before)
        with doc_ex.open(geometry_pdf(0)) as source, source.edit() as document:
            with self.assertRaises(ValueError):  # 2.5 view points == 2 user units.
                document.pages[0].annotations.add('oval', box=(20, 20, 2.5, 30))

    def test_three_user_unit_minimum_is_visible_at_fixture_dpi(self):
        for rotation in (0, 90, 180, 270):
            with self.subTest(rotation=rotation), doc_ex.open(geometry_pdf(rotation)) as source, source.edit() as document:
                for row, kind in enumerate(('rectangle', 'oval')):
                    # Source UserUnit1.25:3 native units become3.75 current-view points.
                    mark = document.pages[0].annotations.add(kind, box=(20, 20+row*20, 3.75, 3.75), color='green')
                    self.assertAlmostEqual(mark.box[2], 3.75, places=3)
                ink = colored_pixels(document.render_page(0, dpi=144), 1)
                for row in range(2):
                    self.assertGreater(sum(40 <= x < 48 and 40+row*40 <= y < 48+row*40 for x,y in ink), 10)

    def test_imported_profile_rejections_preserve_foreign_appearance(self):
        cases = [(dict(extra=b'/RD [1 1 1 1]'), UnsupportedFormatError),
                 (dict(extra=b'/IC [0 1 0]'), UnsupportedFormatError),
                 (dict(extra=b'/CA 0.5'), UnsupportedFormatError),
                 (dict(extra=b'/BE << /S /C >>'), UnsupportedFormatError),
                 (dict(extra=b'/Measure << >>'), UnsupportedFormatError),
                 (dict(extra=b'/PCPicture true'), UnsupportedFormatError),
                 (dict(border=b'<< /W 2 /S /D /D [1 2] >>'), UnsupportedFormatError),
                 (dict(border=b'<< /W 1 /S /S >>'), UnsupportedFormatError),
                 (dict(color=b'[1 0 0 0]'), UnsafeArchiveError)]
        for kwargs, error in cases:
            with self.subTest(kwargs=kwargs), doc_ex.open(imported_shape(**kwargs)) as source, source.edit() as document:
                mark = document.pages[0].annotations[0]
                before, png = document.to_bytes(), document.render_page(0, dpi=72)
                with self.assertRaises(error):
                    mark.update(text='Do not commit', color='blue', box=(30, 30, 100, 80))
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), png)
                self.assertEqual(mark.text, 'Original')
                del document.pages[0].annotations[0]
        with doc_ex.open(imported_shape()) as source, source.edit() as document:
            mark = document.pages[0].annotations[0]
            mark.update(color='blue', box=(30, 30, 100, 80))
            self.assertEqual(mark.kind, 'rectangle')
            self.assertTrue(colored_pixels(document.render_page(0, dpi=72), 2))

    def test_stale_or_closed_handles_fail_before_argument_callbacks(self):
        for state in ('closed', 'page_stale', 'annotation_stale'):
            with self.subTest(state=state), doc_ex.new('pdf', size=(200, 200)) as document:
                document.pages.add(size=(200, 200))
                page = document.pages[0]
                annotations = page.annotations
                mark = annotations.add('rectangle', box=(20, 20, 120, 60))
                called = []
                class RaisingFloat(float):
                    def __float__(self):
                        called.append('box')
                        raise AssertionError('box callback must not run')
                class RaisingColor(str):
                    def lower(self):
                        called.append('color')
                        raise AssertionError('color callback must not run')
                class RaisingIndex:
                    def __index__(self):
                        called.append('index')
                        raise AssertionError('index callback must not run')
                box = (RaisingFloat(), 20, 120, 60)
                color = RaisingColor('red')
                target = RaisingIndex()
                operations = [lambda: mark.update(box=box), lambda: mark.update(color=color),
                              lambda: mark.update(page=target), lambda: mark.update(box=object())]
                if state != 'annotation_stale':
                    operations.extend([lambda: annotations.add('oval', box=box),
                                       lambda: annotations.add('oval', box=(20, 20, 120, 60), color=color),
                                       lambda: annotations.add_link(box=box, uri='https://example.com'),
                                       lambda: annotations.add_link(box=(20, 20, 120, 60), page=target),
                                       lambda: annotations.add('oval', box=object())])
                if state == 'closed':
                    document.close()
                    error = DocumentClosedError
                elif state == 'page_stale':
                    document.pages.move(0, 1)
                    error = ValueError
                else:
                    del annotations[0]
                    error = ValueError
                for operation in operations:
                    with self.assertRaises(error):
                        operation()
                    self.assertEqual(called, [])

    def test_argument_callbacks_cannot_retarget_annotation_or_page(self):
        for operation in ('add', 'update', 'add_link'):
            with self.subTest(operation=operation), doc_ex.new('pdf', size=(200, 200)) as document:
                second = document.pages.add(size=(200, 200))
                second.annotations.add('oval', box=(20, 20, 120, 60), text='Second page')
                page = document.pages[0]
                mark = page.annotations.add('rectangle', box=(20, 20, 120, 60))
                class ReorderingFloat(float):
                    def __float__(self):
                        document.pages.move(0, 1)
                        return 20.0
                box = (ReorderingFloat(), 20, 120, 60)
                with self.assertRaisesRegex(ValueError, 'stale'):
                    if operation == 'add':
                        page.annotations.add('oval', box=box)
                    elif operation == 'update':
                        mark.update(box=box, text='Wrong target')
                    else:
                        page.annotations.add_link(box=box, uri='https://example.com')
                self.assertEqual(len(document.pages[0].annotations), 1)
                self.assertEqual(document.pages[0].annotations[0].text, 'Second page')
                self.assertEqual(len(document.pages[1].annotations), 1)
                self.assertEqual(document.pages[1].annotations[0].text, '')
        document = doc_ex.new('pdf')
        mark = document.pages[0].annotations.add('oval', box=(20, 20, 120, 60))
        class ClosingFloat(float):
            def __float__(self):
                document.close()
                return 20.0
        with self.assertRaises(DocumentClosedError):
            mark.box = (ClosingFloat(), 20, 120, 60)


if __name__ == '__main__':
    unittest.main()
