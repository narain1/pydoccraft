"""Existing field creation must not reuse an ordinal captured before callbacks."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError


class Coordinate(float):
    def __new__(cls, callback):
        value = super().__new__(cls, 20)
        value.callback = callback
        return value

    def __float__(self):
        self.callback()
        return 20.0


class Options:
    def __init__(self, callback):
        self.callback = callback

    def __iter__(self):
        self.callback()
        yield "A"
        yield "B"


class PdfFieldAddOwnershipTests(unittest.TestCase):
    def document(self):
        document = doc_ex.new("pdf", size=(300, 300))
        document.pages.add(size=(300, 300))
        document.pages[0].draw_text("FIRST", at=(20, 200))
        document.pages[1].draw_text("SECOND", at=(20, 200))
        return document

    def add(self, fields, mechanism, callback):
        if mechanism == "coordinate":
            return fields.add("new", (Coordinate(callback), 20, 180, 30))
        return fields.add("new", (20, 20, 180, 30), kind="choice", options=Options(callback))

    def test_callback_page_edits_or_close_reject_without_outer_mutation(self):
        for mechanism in ("coordinate", "options"):
            for action in ("move", "delete", "close"):
                with self.subTest(mechanism=mechanism, action=action), self.document() as document:
                    fields = document.pages[0].fields
                    snapshots = []
                    for page in range(len(document.pages)):
                        document.render_page(page, dpi=72)
                    def callback():
                        if action == "move":
                            document.pages.move(0, 1)
                        elif action == "delete":
                            del document.pages[0]
                        else:
                            document.close()
                        if action != "close":
                            snapshots.append((document.to_bytes(),
                                              [document.render_page(page, dpi=72)
                                               for page in range(len(document.pages))]))
                    error = DocumentClosedError if action == "close" else ValueError
                    with self.assertRaises(error):
                        self.add(fields, mechanism, callback)
                    if action != "close":
                        self.assertEqual(document.to_bytes(), snapshots[0][0])
                        self.assertEqual([document.render_page(page, dpi=72)
                                          for page in range(len(document.pages))], snapshots[0][1])
                        self.assertEqual(len(document.fields), 0)
                        with doc_ex.open(document.to_bytes()) as reopened:
                            self.assertIn("SECOND", reopened[0].get_text())
                            if action == "move":
                                self.assertIn("FIRST", reopened[1].get_text())

    def test_already_invalid_handles_do_not_evaluate_inputs(self):
        for mechanism in ("coordinate", "options"):
            for action in ("move", "delete", "close"):
                with self.subTest(mechanism=mechanism, action=action), self.document() as document:
                    fields = document.pages[0].fields
                    if action == "move":
                        document.pages.move(0, 1)
                    elif action == "delete":
                        del document.pages[0]
                    else:
                        document.close()
                    called = []
                    def callback():
                        called.append(True)
                        raise AssertionError("invalid handle evaluated input")
                    error = DocumentClosedError if action == "close" else ValueError
                    with self.assertRaises(error):
                        self.add(fields, mechanism, callback)
                    self.assertEqual(called, [])

    def test_callback_errors_and_independent_copies_preserve_target(self):
        for mechanism in ("coordinate", "options"):
            with self.subTest(mechanism=mechanism), self.document() as document:
                fields = document.pages[-1].fields
                before = document.to_bytes()
                pixels = document.render_page(1, dpi=72)
                def fail():
                    raise RuntimeError("caller failed")
                with self.assertRaisesRegex(RuntimeError, "caller failed"):
                    self.add(fields, mechanism, fail)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(1, dpi=72), pixels)
                copies = []
                def copy():
                    copied = document.copy()
                    copied.pages.move(0, 1)
                    copies.append(copied)
                field = self.add(fields, mechanism, copy)
                try:
                    self.assertEqual(field.page.number, 1)
                    self.assertEqual(len(copies[0].fields), 0)
                    self.assertEqual(field.kind, "choice" if mechanism == "options" else "text")
                    field.value = "A"
                    self.assertEqual(document.fields["new"].value, "A")
                    with doc_ex.open(document.to_bytes()) as reopened:
                        self.assertIn("SECOND", reopened[1].get_text())
                finally:
                    for copied in copies:
                        copied.close()


if __name__ == "__main__":
    unittest.main()
