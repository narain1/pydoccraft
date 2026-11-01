import unittest

from doc_ex import Block, Document, Image, Link, Page
from doc_ex import _core
from doc_ex._document import _format_join


class FormatterNativeTests(unittest.TestCase):
    def test_native_preflight_before_conversion(self):
        class Hostile(str):
            def __len__(self):
                raise AssertionError("subclass callback")

        class HostileList(list):
            def __iter__(self):
                raise AssertionError("subclass callback")

        self.assertIsNone(_core._format_join([Hostile("x")], "\n", False))
        self.assertIsNone(_core._format_join(HostileList(["x"]), "\n", False))
        self.assertIsNone(_core._format_join(["x"] * 100_001, "", False))
        self.assertIsNone(_core._format_join(["x" * 1_000_001], "", False))
        self.assertIsNone(_core._format_join([], "x" * 1_000_001, False))

    def test_unicode_and_fallback_keep_python_semantics(self):
        for text in ("\u001c hi \u001f", "\u2003é😀\u0085", "\ud800x\udfff", "x" * 1_000_001):
            with self.subTest(text=text[:20]):
                page = Page(0, [Block("paragraph", text)])
                self.assertEqual(page.get_text(), text.strip())
                self.assertEqual(page.get_text("markdown"), text.strip())

        class HostileTruth(str):
            def __bool__(self):
                raise AssertionError("unexpected truth test")

        self.assertEqual(_format_join([HostileTruth("ok")], "\n"), "ok")

    def test_mutation_links_and_callback(self):
        link = Link("target", "😀", metadata={"start": 1, "end": 2})
        image = Image("a.png", b"x", "image/png", "alt")
        block = Block("paragraph", "é😀", links=[link], images=[image])
        page = Page(0, [block])
        doc = Document([page], {}, b"", "docx", None)
        self.assertIn("é[😀](target)", doc.to_markdown())
        block.text = "changed"
        self.assertIn("changed", doc.to_markdown())
        calls = []
        self.assertIn("![alt](reference)", page._markdown(image_reference=lambda img, path: calls.append(path) or "reference"))
        self.assertEqual(calls, [("blocks", 0, "images", 0)])


if __name__ == "__main__":
    unittest.main()
