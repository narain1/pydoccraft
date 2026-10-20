"""Native comment locks preserve appearances and inhibit documented edits only."""
import unittest

import doc_ex
from doc_ex.errors import DocExError, DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_pdf_geometric_annotations import imported_shape
from test_render_geometry import geometry_pdf
from test_pdf_markup import colored_pixels


KINDS = ('note', 'highlight', 'underline', 'strikeout', 'squiggly', 'rectangle', 'ellipse')


class AnnotationLockTests(unittest.TestCase):
    def test_all_kinds_preserve_pixels_and_metadata_in_rotated_crop_userunit_views(self):
        for rotation in (0, 90, 180, 270):
            for kind in KINDS:
                with self.subTest(rotation=rotation, kind=kind), doc_ex.open(geometry_pdf(rotation)) as source, source.edit() as document:
                    baseline = document.render_page(0, dpi=144)
                    mark = document.pages[0].annotations.add(kind, box=(30, 80, 25, 25) if kind == 'note' else (30, 80, 100, 24), text='Original café', author='Author', color='red')
                    metadata = mark.kind, mark.box, mark.text, mark.author, mark.color
                    pixels = document.render_page(0, dpi=144)
                    self.assertNotEqual(pixels, baseline)
                    self.assertTrue(colored_pixels(pixels, 0))
                    self.assertIs(mark.locked, False)
                    mark.locked = True
                    self.assertIs(mark.locked, True)
                    self.assertEqual((mark.kind, mark.box, mark.text, mark.author, mark.color), metadata)
                    self.assertEqual(document.render_page(0, dpi=144), pixels)
                    with doc_ex.open(document.to_bytes()) as reopened, reopened.edit() as edited:
                        other = edited.pages[0].annotations[0]
                        self.assertTrue(other.locked)
                        self.assertEqual(edited.render_page(0, dpi=144), pixels)
                        other.locked = False
                        self.assertFalse(other.locked)
                        self.assertEqual(edited.render_page(0, dpi=144), pixels)
                    with document.copy() as copied:
                        copied.pages[0].annotations[0].locked = False
                        self.assertTrue(mark.locked)
                        self.assertFalse(copied.pages[0].annotations[0].locked)
                    mark.locked = False
                    mark.locked = False
                    self.assertEqual(document.render_page(0, dpi=144), pixels)

    def test_locked_text_is_editable_but_geometry_style_author_delete_are_atomic(self):
        for kind in KINDS:
            with self.subTest(kind=kind), doc_ex.new('pdf', size=(240, 240)) as document:
                page = document.pages[0]
                mark = page.annotations.add(kind, box=(30, 40, 20, 20) if kind == 'note' else (30, 40, 100, 30), text='Original', author='Author', color='red')
                mark.locked = True
                mark.text = 'Changed comment ✓'
                self.assertTrue(mark.locked)
                self.assertEqual(mark.text, 'Changed comment ✓')
                pixels = document.render_page(0, dpi=144)
                before, cached = document.to_bytes(), document._cached
                x, y, width, height = mark.box
                for patch in ({'box': (x+10, y+10, width, height)}, {'color': 'blue'}, {'author': 'Other'},
                              {'text': 'Rollback', 'color': 'blue'}, {'text': 'Rollback', 'author': 'Other'}):
                    with self.subTest(patch=patch), self.assertRaisesRegex(DocExError, 'locked'):
                        mark.update(**patch)
                    self.assertEqual(document.to_bytes(), before)
                    self.assertIs(document._cached, cached)
                with self.assertRaisesRegex(DocExError, 'locked'):
                    del page.annotations[0]
                self.assertIs(document._cached, cached)
                self.assertEqual(document.render_page(0, dpi=144), pixels)
                mark.locked = False
                mark.update(box=(x+10, y+10, width, height), color='blue', author='Other')
                self.assertEqual(mark.author, 'Other')
                self.assertTrue(colored_pixels(document.render_page(0, dpi=144), 2))
                del page.annotations[0]
                self.assertEqual(len(page.annotations), 0)

    def test_strict_bool_lifetime_and_page_identity(self):
        class Dangerous:
            def __bool__(self):
                raise AssertionError('truth callback executed')
        with doc_ex.new('pdf') as document:
            mark = document.pages[0].annotations.add('rectangle', box=(20, 20, 100, 40))
            pixels = document.render_page(0, dpi=72)
            before, cached = document.to_bytes(), document._cached
            for value in (None, 0, 1, 'true', [], Dangerous()):
                with self.assertRaises(TypeError):
                    mark.locked = value
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cached)
            self.assertEqual(document.render_page(0, dpi=72), pixels)
            mark.locked = True
            document.pages.add()
            document.pages.move(0, 1)
            with self.assertRaisesRegex(ValueError, 'stale'):
                mark.locked = Dangerous()
            fresh = document.pages[1].annotations[0]
            self.assertTrue(fresh.locked)
            fresh.locked = False
            del document.pages[1].annotations[0]
            with self.assertRaisesRegex(ValueError, 'stale'):
                fresh.locked = Dangerous()
            replacement = document.pages[1].annotations.add('rectangle', box=(20, 20, 100, 40))
            self.assertFalse(replacement.locked)
            document.close()
            for value in (True, Dangerous()):
                with self.assertRaises(DocumentClosedError):
                    replacement.locked = value
            with self.assertRaises(DocumentClosedError):
                _ = replacement.locked

    def test_imported_profiles_link_exclusion_and_warm_cache_rollback(self):
        with doc_ex.open(imported_shape()) as source, source.edit() as document:
            mark = document.pages[0].annotations[0]
            pixels = document.render_page(0, dpi=72)
            mark.locked = True
            self.assertTrue(mark.locked)
            self.assertEqual(document.render_page(0, dpi=72), pixels)
            mark.text = 'Still editable'
            mark.locked = False
            self.assertEqual(document.render_page(0, dpi=72), pixels)
        for extra, error in ((b'/F 0', UnsupportedFormatError), (b'/F (4)', UnsupportedFormatError),
                             (b'/IRT 4 0 R', UnsupportedFormatError),
                             (b'/Foreign [ [ 1 2 3 ] ]', UnsupportedFormatError),
                             (b'/CA 0.5', UnsupportedFormatError),
                             (b'/Contents (' + b'x'*131075 + b')', UnsafeArchiveError)):
            with self.subTest(extra=extra[:30]), doc_ex.open(imported_shape(extra)) as source, source.edit() as document:
                mark = document.pages[0].annotations[0]
                pixels = document.render_page(0, dpi=72)
                before, cached = document.to_bytes(), document._cached
                with self.assertRaises(error):
                    mark.locked = True
                self.assertEqual(document.to_bytes(), before)
                self.assertIs(document._cached, cached)
                self.assertEqual(document.render_page(0, dpi=72), pixels)
        with doc_ex.new('pdf') as document:
            mark = document.pages[0].annotations.add_link(box=(20, 20, 100, 30), uri='https://example.com')
            before = document.to_bytes()
            with self.assertRaises(UnsupportedFormatError):
                _ = mark.locked
            with self.assertRaises(UnsupportedFormatError):
                mark.locked = True
            self.assertEqual(document.to_bytes(), before)
            mark.uri = 'https://example.org'
            self.assertEqual(mark.uri, 'https://example.org')


if __name__ == '__main__':
    unittest.main()
