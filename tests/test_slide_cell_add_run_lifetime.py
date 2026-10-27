"""Cell add_run validates lifetime before and after formatting callbacks."""
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError


class SlideCellAddRunLifetimeTests(unittest.TestCase):
    def test_closed_deleted_and_stale_targets_do_not_invoke_format_callbacks(self):
        for invalidation in ("closed", "slide", "table", "grid", "body"):
            with self.subTest(invalidation=invalidation), doc_ex.new("pptx") as deck:
                slide = deck.slides.add()
                table = slide.add_table([["Keep", "Neighbor"]], box=(20, 20, 300, 100))
                paragraph = table[0, 0].paragraphs[0]
                if invalidation == "closed":
                    deck.close()
                elif invalidation == "slide":
                    del deck.slides[0]
                elif invalidation == "table":
                    del slide.shapes[0]
                elif invalidation == "grid":
                    table.merge((0, 0), (0, 1))
                else:
                    table[0, 0].text = "Reset"
                calls = []
                class Font(str):
                    def __len__(self):
                        calls.append(True)
                        raise AssertionError("invalid target invoked caller")
                with self.assertRaises(DocumentClosedError if invalidation == "closed" else (ValueError, IndexError)):
                    paragraph.add_run(" nope", font=Font("Inter"))
                self.assertEqual(calls, [])

    def test_live_reordering_follows_original_slide_and_cell(self):
        with doc_ex.new("pptx") as deck:
            first = deck.slides.add()
            second = deck.slides.add()
            table = first.add_table([["First"]], box=(20, 20, 300, 100))
            other = second.add_table([["Second"]], box=(20, 20, 300, 100))
            paragraph = table[0, 0].paragraphs[0]
            calls = []
            class Font(str):
                def __len__(self):
                    calls.append(True)
                    deck.slides.move(0, 1)
                    return super().__len__()
            selected = paragraph.add_run(" appended", font=Font("Inter"))
            self.assertEqual(calls, [True])
            self.assertEqual(first.number, 1)
            self.assertEqual(selected.text, " appended")
            self.assertEqual(table[0, 0].text, "First appended")
            self.assertEqual(other[0, 0].text, "Second")
            with doc_ex.open(deck.to_bytes()) as read:
                self.assertEqual(read[0].get_text(), "Second")
                self.assertEqual(read[1].get_text(), "First appended")

    def test_live_deletion_reset_close_and_exception_do_not_append(self):
        for action in ("delete", "reset", "close", "raise"):
            with self.subTest(action=action), doc_ex.new("pptx") as deck:
                slide = deck.slides.add()
                table = slide.add_table([["Keep"]], box=(20, 20, 300, 100))
                paragraph = table[0, 0].paragraphs[0]
                before = deck.to_bytes()
                cached = deck.render_page(0, dpi=72)
                callback_bytes = []
                class Font(str):
                    def __len__(self):
                        if action == "delete":
                            del slide.shapes[0]
                        elif action == "reset":
                            table[0, 0].text = "Reset"
                        elif action == "close":
                            deck.close()
                        else:
                            raise RuntimeError("caller exception")
                        if action != "close":
                            callback_bytes.append(deck.to_bytes())
                        return super().__len__()
                error = DocumentClosedError if action == "close" else RuntimeError if action == "raise" else (ValueError, IndexError)
                with self.assertRaises(error):
                    paragraph.add_run(" nope", font=Font("Inter"))
                if action in {"delete", "reset"}:
                    self.assertEqual(deck.to_bytes(), callback_bytes[0])
                elif action == "raise":
                    self.assertEqual(table[0, 0].text, "Keep")
                    self.assertEqual(deck.to_bytes(), before)
                    self.assertEqual(deck.render_page(0, dpi=72), cached)


if __name__ == "__main__":
    unittest.main()
