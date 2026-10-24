"""Existing formatter reset: native inheritance, output, ownership and rollback."""
from contextlib import ExitStack
import unittest

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_authoring import image_fixture

URI = "https://example.test/default-reset"

ROLE_COLORS = {
    "body": (255, 0, 0), "header": (0, 0, 128), "cell": (0, 128, 0),
    "style": (128, 0, 128), "direct": (255, 128, 0),
}


def owner(size):
    d = doc_ex.new("docx")
    section = d.sections[0]
    section.page_size = (240, 220)
    section.margins = (32, 12, 12, 12)
    section.header_distance = 10
    d.styles.format_defaults(font="DejaVu Sans", font_size=size, color="red",
                             bold=True, space_before=0, space_after=0, line_spacing=1)
    style = d.styles.add("Inherited")
    body = d.add_paragraph("BODYKEEP", style=style)
    body[:].link = URI
    # The native Hyperlink style supplies its own color. Keep font size inherited
    # while making this carrier's ink independently identifiable.
    body[:].format(color="red")
    section.header().add_paragraph("HEADERKEEP")[:].format(color="navy")
    d.add_table([["CELLKEEP"]])[0, 0].paragraphs[0][:].format(color="green")
    fixed = d.styles.add("Fixed", based_on=style)
    fixed.format(font_size=18, color="#800080")
    d.add_paragraph("STYLEKEEP", style=fixed)
    d.add_paragraph().add_run("DIRECTKEEP", font_size=15, color="#FF8000")
    d.add_image(image_fixture(4, 2), width=20, height=10)
    return d



class DefaultSizeResetTests(unittest.TestCase):
    """Clearing the document default font size is refused.

    WordCraft's reader substitutes 10 pt for an omitted default size while its model resolves the
    same omission as 11 pt, so a cleared size would change every inheriting run after a save and
    reopen. The refusal must be explicit and must leave the document untouched.
    """

    def test_clear_is_refused_atomically_and_other_defaults_still_work(self):
        with ExitStack() as stack:
            d = stack.enter_context(owner(10))
            copied = stack.enter_context(d.copy())
            before = (d.to_bytes(), d.styles.defaults, d.render_page(0, dpi=144))
            for patch in (
                {"font_size": None},
                {"font_size": None, "bold": True},
                {"font_size": None, "color": (255, 0, 0)},
            ):
                with self.assertRaises(UnsupportedFormatError) as caught:
                    d.styles.format_defaults(**patch)
                self.assertIn("clearing the default font size", str(caught.exception))
                self.assertEqual((d.to_bytes(), d.styles.defaults, d.render_page(0, dpi=144)), before)
            self.assertEqual(d.styles.defaults["font_size"], 10)
            self.assertEqual(copied.styles.defaults["font_size"], 10)
            # Setting a size, or changing other defaults, is unaffected.
            d.styles.format_defaults(font_size=12)
            self.assertEqual(d.styles.defaults["font_size"], 12)
            d.styles.format_defaults(bold=True)
            self.assertTrue(d.styles.defaults["bold"])
            self.assertEqual(copied.styles.defaults["font_size"], 10)

    def test_refusal_does_not_run_value_callbacks_after_the_owner_changes(self):
        class EditedChannel(int):
            def __format__(self, spec):
                d.add_paragraph("CALLBACKKEEP")
                return int.__format__(self, spec)
        with owner(10) as d:
            before = d.to_bytes(), d.styles.defaults
            with self.assertRaises((UnsupportedFormatError, DocumentClosedError)):
                d.styles.format_defaults(font_size=None, color=(EditedChannel(255), 0, 0))
            self.assertEqual(d.styles.defaults, before[1])
        with doc_ex.new("docx") as d:
            d.styles.format_defaults(font_size=10)
            with d.copy() as copy:
                with self.assertRaises((UnsupportedFormatError, DocumentClosedError)):
                    d.styles.format_defaults(font_size=None)
                self.assertEqual(copy.styles.defaults["font_size"], 10)
                self.assertEqual(d.styles.defaults["font_size"], 10)
