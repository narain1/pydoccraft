"""Native comment alpha uses existing handles and retains page content."""
import unittest

import doc_ex
from doc_ex.errors import DocExError, DocumentClosedError, UnsupportedFormatError
from test_annotation_lock import KINDS
from test_pdf_geometric_annotations import imported_shape
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def pixels(png):
    return rgba_pixels(png_chunks(png))[1]


class AnnotationOpacityTests(unittest.TestCase):
    def test_all_kinds_native_alpha_roundtrip_and_copy(self):
        for kind in KINDS:
            for background in ('white', '#80C080'):
                with self.subTest(kind=kind, background=background), doc_ex.new('pdf', size=(160, 160)) as doc:
                    page = doc.pages[0]
                    page.background = background
                    page.draw_text('Retained text', at=(15, 130))
                    baseline = pixels(doc.render_page(0, dpi=72))
                    mark = page.annotations.add(kind, box=(20, 30, 20, 20) if kind == 'note' else (20, 30, 100, 30), color='red', text='Note', author='Author')
                    self.assertEqual(mark.opacity, 1)
                    original = mark.kind, mark.box, mark.text, mark.author, mark.color
                    distances = []
                    for alpha in (0, .25, .5, 1):
                        mark.opacity = alpha
                        self.assertEqual(mark.opacity, alpha)
                        current = pixels(doc.render_page(0, dpi=72))
                        distance = sum(abs(a-b) for a, b in zip(current, baseline))
                        distances.append(distance)
                        if alpha == 0:
                            self.assertEqual(current, baseline)
                        self.assertEqual((mark.kind, mark.box, mark.text, mark.author, mark.color), original)
                    self.assertEqual(sorted(distances), distances)
                    self.assertEqual(len(set(distances)), 4)
                    mark.update(opacity=.5, color='blue')
                    png = doc.render_page(0, dpi=144)
                    for packed in (False, True):
                        with doc_ex.open(doc.to_bytes(object_streams=packed)) as reopened:
                            self.assertEqual(reopened.render_page(0, dpi=144), png)
                            self.assertIn('Retained text', reopened[0].get_text())
                            with reopened.edit() as edited:
                                other = edited.pages[0].annotations[0]
                                self.assertEqual(other.opacity, .5)
                                other.opacity = 1
                                self.assertEqual(other.opacity, 1)
                    with doc.copy() as copied:
                        copied.pages[0].annotations[0].opacity = .25
                        self.assertEqual(mark.opacity, .5)
                        self.assertNotEqual(copied.render_page(0, dpi=144), png)

    def test_invalid_values_locked_combined_updates_and_link(self):
        with doc_ex.new('pdf', size=(180, 180)) as doc:
            mark = doc.pages[0].annotations.add('rectangle', box=(20, 20, 80, 40))
            mark.update(box=(30, 40, 90, 50), color='red', opacity=.25)
            self.assertEqual((mark.box, mark.color, mark.opacity), ((30, 40, 90, 50), '#FF0000', .25))
            doc.render_page(0, dpi=72)
            before, cached = doc.to_bytes(), doc._cached
            for value in (None, True, '0.5', b'.5', -0.1, 1.1, float('nan'), float('inf'), object()):
                with self.subTest(value=value), self.assertRaises((TypeError, ValueError)):
                    mark.opacity = value
                self.assertEqual(doc.to_bytes(), before)
                self.assertIs(doc._cached, cached)
            mark.locked = True
            before = doc.to_bytes()
            with self.assertRaisesRegex(DocExError, 'locked'):
                mark.update(opacity=.5, text='Must rollback')
            self.assertEqual(doc.to_bytes(), before)
            self.assertEqual(mark.opacity, .25)
            mark.locked = False
            mark.opacity = .5
            link = doc.pages[0].annotations.add_link(box=(10, 110, 100, 20), uri='https://example.com')
            before = doc.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                _ = link.opacity
            with self.assertRaises(UnsupportedFormatError):
                link.opacity = .5
            self.assertEqual(doc.to_bytes(), before)

    def test_canonical_imports_accept_fractional_scalar_only(self):
        with doc_ex.open(imported_shape(b'/CA 0.5')) as source, source.edit() as doc:
            mark = doc.pages[0].annotations[0]
            self.assertEqual(mark.opacity, .5)
            mark.locked = True
            self.assertTrue(mark.locked)
            mark.locked = False
            mark.opacity = .25
            self.assertEqual(mark.opacity, .25)
            mark.opacity = 1
            self.assertEqual(mark.opacity, 1)
        for extra in (b'/CA 1.5', b'/CA -0.5', b'/CA (0.5)', b'/CA 5 0 R', b'/CA 0.5 /Foreign [ [ 1 ] ]'):
            with self.subTest(extra=extra), doc_ex.open(imported_shape(extra)) as source, source.edit() as doc:
                mark = doc.pages[0].annotations[0]
                before = doc.to_bytes()
                with self.assertRaises(UnsupportedFormatError):
                    _ = mark.opacity
                with self.assertRaises(UnsupportedFormatError):
                    mark.opacity = .5
                self.assertEqual(doc.to_bytes(), before)

    def test_float_callbacks_cannot_retarget_stale_handles(self):
        class Callback:
            def __init__(self, action):
                self.action = action
            def __float__(self):
                self.action()
                return .5
        for action in ('delete', 'move', 'close'):
            with self.subTest(action=action), doc_ex.new('pdf') as doc:
                page = doc.pages[0]
                mark = page.annotations.add('note', box=(20, 20, 20, 20))
                def mutate():
                    if action == 'close':
                        doc.close()
                    elif action == 'move':
                        doc.pages.add()
                        doc.pages.move(0, 1)
                    else:
                        del page.annotations[0]
                        page.annotations.add('note', box=(60, 60, 20, 20))
                with self.assertRaises((ValueError, DocumentClosedError)):
                    mark.opacity = Callback(mutate)
                if action != 'close':
                    fresh = doc.pages[1 if action == 'move' else 0].annotations[0]
                    self.assertEqual(fresh.opacity, 1)
                with self.assertRaises((ValueError, DocumentClosedError)):
                    mark.opacity = Callback(lambda: self.fail('stale callback executed'))
                with self.assertRaises((ValueError, DocumentClosedError)):
                    _ = mark.opacity


if __name__ == '__main__':
    unittest.main()
