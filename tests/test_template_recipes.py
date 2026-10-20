import importlib.util
from pathlib import Path
import tempfile
import unittest

import doc_ex


spec = importlib.util.spec_from_file_location("template_batch_recipe", Path(__file__).resolve().parents[1] / "examples/template_batch.py")
recipe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recipe)


class TemplateRecipeTests(unittest.TestCase):
    def test_independent_letters_replace_across_runs_and_explicit_stories(self):
        with recipe.make_template() as template, tempfile.TemporaryDirectory() as folder:
            original = [p.text for p in template.paragraphs]
            for index, customer in enumerate(("Acme", "Mira")):
                with recipe.fill_letter(template, {"customer": customer, "company": "Example"}) as letter:
                    path = Path(folder) / f"{index}.docx"
                    letter.save(path)
                    for source in (path, letter.convert("pdf")):
                        with doc_ex.open(source) as reopened:
                            self.assertIn(f"Dear {customer},", reopened[0].get_text())
                            self.assertNotIn("{{customer}}", reopened[0].get_text())
                            self.assertTrue(any(link.uri == "https://example.com/terms" for page in reopened for link in page.get_links()))
                    with doc_ex.open(letter.convert("pdf")) as pdf:
                        self.assertIn("Prepared by Example", pdf[0].get_text())
                self.assertEqual([p.text for p in template.paragraphs], original)
                self.assertIn("{{company}}", template.sections[0].header().paragraphs[0].text)

    def test_missing_unused_and_invalid_values_do_not_change_template(self):
        with recipe.make_template() as template:
            original = [p.text for p in template.paragraphs]
            for values in ({"customer": "Acme"}, {"customer": "Acme", "company": "Example", "extra": "unused"}, {"customer": "Acme", "company": "bad\x00value"}):
                with self.assertRaises(ValueError):
                    recipe.fill_letter(template, values)
                self.assertEqual([p.text for p in template.paragraphs], original)
