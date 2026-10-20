import gc
import unittest
import weakref

import doc_ex
from doc_ex.errors import DocumentClosedError


class CollectionOwnershipTests(unittest.TestCase):
    def test_pdf_iterators_do_not_rebind_positions_after_structure_changes(self):
        with doc_ex.new("pdf", size=(200, 200)) as document:
            document.pages.add(size=(250, 250))
            document.pages.add(size=(300, 300))
            iterator = iter(document.pages)
            self.assertEqual(next(iterator).size, (200, 200))
            document.pages.move(2, 1)
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = next(iterator).size

            page = document.pages[0]
            for name in ("first", "second", "third"):
                page.annotations.add("note", box=(10, 10, 20, 20), text=name)
                document.bookmarks.add(name, 0)
            annotations = iter(page.annotations)
            bookmarks = iter(document.bookmarks)
            self.assertEqual(next(annotations).text, "first")
            self.assertEqual(next(bookmarks).title, "first")
            del page.annotations[0]
            del document.bookmarks[0]
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = next(annotations).text
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = next(bookmarks).title
            self.assertEqual(page.annotations[0].text, "second")
            self.assertEqual(document.bookmarks[0].title, "second")

    def test_stable_handles_snapshot_membership_and_survive_owner_variable(self):
        document = doc_ex.new("docx")
        owner = weakref.ref(document)
        first = document.add_paragraph("first")
        second = document.add_paragraph("second")
        iterator = iter(document.paragraphs)
        self.assertEqual(next(iterator).text, "first")
        document.add_paragraph("later")
        second.text = "updated"
        self.assertEqual([paragraph.text for paragraph in iterator], ["updated"])
        snapshot = document.paragraphs[::-1]
        snapshot.pop()
        self.assertEqual(len(document.paragraphs), 3)
        with document.copy() as copied:
            copied.paragraphs[0].text = "independent"
            self.assertEqual(first.text, "first")
        del document
        gc.collect()
        self.assertEqual(first.text, "first")
        owner().close()
        with self.assertRaises(DocumentClosedError):
            _ = first.text

    def test_slide_identity_survives_reordering_but_close_invalidates_iteration(self):
        with doc_ex.new("pptx") as document:
            for name in ("first", "second", "third"):
                document.slides.add().name = name
            iterator = iter(document.slides)
            self.assertEqual(next(iterator).name, "first")
            document.slides.move(2, 1)
            self.assertEqual(next(iterator).name, "second")
            self.assertEqual(next(iterator).name, "third")
            iterator = iter(document.slides)
            next(iterator)
        with self.assertRaises(DocumentClosedError):
            next(iterator)


if __name__ == "__main__":
    unittest.main()
