import unittest

import doc_ex


class LiteralTextOperandTests(unittest.TestCase):
    def test_find_and_replace_use_base_string_values_without_owner_callbacks(self):
        for kind in ("docx", "shape", "cell"):
            with self.subTest(kind=kind), doc_ex.new("docx" if kind == "docx" else "pptx") as document:
                if kind == "docx":
                    content = document.add_paragraph("α same same")
                else:
                    slide = document.slides.add()
                    if kind == "shape":
                        content = slide.add_text("α same same", box=(20, 20, 400, 80))
                    else:
                        cell = slide.add_table([["α same same"]], box=(20, 20, 400, 80))[0, 0]
                        content = cell.paragraphs[0]

                class Literal(str):
                    calls = 0

                    def __str__(self):
                        raise AssertionError("literal conversion must not dispatch user hooks")

                    def __bool__(self):
                        raise AssertionError("literal truth must not dispatch user hooks")

                    def __eq__(self, other):
                        raise AssertionError("literal equality must not dispatch user hooks")

                    def __len__(self):
                        self.calls += 1
                        if self.calls == 2:
                            # Previously ran inside the staged replacement, whose
                            # commit silently overwrote this caller-owned reset.
                            if kind == "cell":
                                cell.text = "CALLBACK RESET"
                            else:
                                content.text = "CALLBACK RESET"
                        return super().__len__()

                search = Literal("same")
                self.assertEqual([item.text for item in content.find(search)], ["same", "same"])
                self.assertEqual(search.calls, 0)
                replacement = Literal("β")
                self.assertEqual(content.replace_text(search, replacement), 2)
                self.assertEqual(content.text, "α β β")
                self.assertEqual((search.calls, replacement.calls), (0, 0))
                before = document.to_bytes()
                for invalid in (None, 1, b"text", Literal("")):
                    with self.assertRaises(ValueError):
                        content.find(invalid)
                    with self.assertRaises(ValueError):
                        content.replace_text(invalid, "replacement")
                for invalid in (None, 1, b"replacement"):
                    with self.assertRaises(TypeError):
                        content.replace_text("β", invalid)
                self.assertEqual(document.to_bytes(), before)


if __name__ == "__main__":
    unittest.main()
