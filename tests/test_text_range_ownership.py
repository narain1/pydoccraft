"""Range formatting checks lifetime/revision before caller normalization."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError


class Color(str):
    def __new__(cls, callback):
        value = super().__new__(cls, "red")
        value.callback = callback
        return value

    def lower(self):
        self.callback()
        return "red"


def make(kind):
    document = doc_ex.new("docx" if kind == "word" else "pptx")
    if kind == "word":
        paragraph = document.add_paragraph("ALPHA")
    else:
        slide = document.slides.add()
        document.slides.add()
        if kind == "shape":
            paragraph = slide.add_text("ALPHA", box=(20, 20, 180, 60))
        else:
            paragraph = slide.add_table([["ALPHA"]], box=(20, 20, 180, 60))[0, 0].paragraphs[0]
    return document, paragraph


class TextRangeOwnershipTests(unittest.TestCase):
    def test_closed_and_stale_ranges_do_not_run_callbacks(self):
        for kind in ("word", "shape", "cell"):
            for state in ("closed", "stale"):
                with self.subTest(kind=kind, state=state):
                    document, paragraph = make(kind)
                    with document:
                        selected = paragraph[:1]
                        if state == "closed":
                            document.close()
                            error = DocumentClosedError
                        else:
                            paragraph.add_run("!")
                            error = ValueError
                        def forbidden():
                            self.fail("invalid range ran a formatting callback")
                        with self.assertRaises(error):
                            selected.format(color=Color(forbidden))

    def test_mutating_callbacks_remain_stale_and_keep_only_their_changes(self):
        for kind in ("word", "shape", "cell"):
            for action in ("append", "reorder", "close"):
                if kind == "word" and action == "reorder":
                    continue
                with self.subTest(kind=kind, action=action):
                    document, paragraph = make(kind)
                    with document:
                        selected = paragraph[:1]
                        expected, calls = [], []
                        def callback():
                            calls.append(1)
                            if action == "close":
                                document.close()
                                return
                            if action == "append":
                                paragraph.add_run("!")
                            else:
                                document.slides.move(0, -1)
                            expected.append((document.to_bytes(), document.render_page(0, dpi=72)))
                        with self.assertRaises(DocumentClosedError if action == "close" else ValueError):
                            selected.format(color=Color(callback))
                        self.assertEqual(calls, [1])
                        if action != "close":
                            self.assertEqual(document.to_bytes(), expected[0][0])
                            self.assertEqual(document.render_page(0, dpi=72), expected[0][1])

    def test_live_callback_failure_preserves_error_and_document(self):
        for kind in ("word", "shape", "cell"):
            with self.subTest(kind=kind):
                document, paragraph = make(kind)
                with document:
                    selected = paragraph[:1]
                    before, pixels = document.to_bytes(), document.render_page(0, dpi=72)
                    def failure():
                        raise RuntimeError("caller color failed")
                    with self.assertRaisesRegex(RuntimeError, "caller color failed"):
                        selected.format(color=Color(failure))
                    self.assertEqual(document.to_bytes(), before)
                    self.assertEqual(document.render_page(0, dpi=72), pixels)
                    self.assertEqual(selected.text, "A")

    def test_valid_format_refreshes_only_its_range_and_preserves_copies(self):
        for kind in ("word", "shape", "cell"):
            with self.subTest(kind=kind):
                document, paragraph = make(kind)
                with document:
                    selected, other = paragraph[:1], paragraph[1:2]
                    before, copies = document.to_bytes(), []
                    selected.format(color=Color(lambda: copies.append(document.copy())))
                    try:
                        self.assertEqual(selected.text, "A")
                        selected.format(bold=True)
                        self.assertEqual(selected.text, "A")
                        with self.assertRaisesRegex(ValueError, "stale"):
                            _ = other.text
                        self.assertEqual(len(copies), 1)
                        self.assertEqual(copies[0].to_bytes(), before)
                        self.assertNotEqual(document.to_bytes(), before)
                        with doc_ex.open(document.to_bytes()) as reopened:
                            self.assertIn("ALPHA", reopened[0].get_text())
                            self.assertTrue(reopened.render_page(0, dpi=72))
                    finally:
                        for copied in copies:
                            copied.close()


if __name__ == "__main__":
    unittest.main()
