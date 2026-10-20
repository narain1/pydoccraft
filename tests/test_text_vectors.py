from pathlib import Path
import unittest

import doc_ex
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


FONT = Path(__file__).parent / "fixtures/fonts/Inter-Regular.ttf"


class TextVectorTests(unittest.TestCase):
    def test_pdf_metadata_and_bookmark_hierarchy_roundtrip(self):
        with doc_ex.new("pdf") as document:
            document.pages.add()
            document.update_metadata(title="Report λ", author="Writer")
            self.assertEqual(document.metadata["title"], "Report λ")
            with self.assertRaises(ValueError):
                document.update_metadata(title="Must not apply", unknown="invalid")
            self.assertEqual(document.metadata["title"], "Report λ")
            first = document.bookmarks.add("Chapter", 0)
            independent = document.bookmarks[0]
            child = first.children.add("Detail", 1)
            self.assertEqual(child.page, 1)
            self.assertEqual(first.title, "Chapter")
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = independent.title
            document.bookmarks[0].expanded = False
            document.bookmarks.add("Appendix", 1)
            moved = document.bookmarks[0].children.move(0, 1, to=document.bookmarks)
            moved.title = "Moved detail"
            self.assertEqual([entry.title for entry in document.bookmarks], ["Chapter", "Moved detail", "Appendix"])
            del document.bookmarks[2]
            with self.assertRaisesRegex(ValueError, "stale"):
                _ = moved.title
            document.update_metadata(author=None)
            with doc_ex.open(document.to_bytes()) as reopened, reopened.edit() as edited:
                self.assertEqual(edited.metadata["title"], "Report λ")
                self.assertFalse(edited.metadata.get("author"))
                self.assertEqual([(entry.title, entry.page) for entry in edited.bookmarks], [("Chapter", 0), ("Moved detail", 1)])
                edited.pages.move(0, 1)
                self.assertEqual([entry.page for entry in edited.bookmarks], [1, 0])

    def test_bookmark_child_collection_survives_its_own_edits(self):
        with doc_ex.new("pdf") as document:
            parent = document.bookmarks.add("Parent", 0)
            children = parent.children
            children.add("One", 0)
            children.add("Two", 0)
            children.move(0, 1)
            self.assertEqual([entry.title for entry in children], ["Two", "One"])
            del children[0]
            children.add("Three", 0)
            self.assertEqual([entry.title for entry in children], ["One", "Three"])
            self.assertEqual(parent.title, "Parent")

    def test_reading_absent_headers_and_footers_does_not_mutate(self):
        from io import BytesIO
        from zipfile import ZipFile

        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Body ")
            text = paragraph.add_run("range")
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                before = {name: archive.read(name) for name in archive.namelist()}
            section = document.sections[0]
            for kind in (section.header, section.footer):
                for variant in ("default", "first", "even"):
                    self.assertEqual(len(kind(variant).paragraphs), 0)
            self.assertEqual(text.text, "range")
            with ZipFile(BytesIO(document.to_bytes())) as archive:
                self.assertEqual({name: archive.read(name) for name in archive.namelist()}, before)
            section.header("first").add_paragraph("First header")
            self.assertEqual(section.header("first").paragraphs[0].text, "First header")

    def test_pdf_supplied_font_preserves_unicode_and_has_visible_glyphs(self):
        content = "Aé ΩЖ € office e\u0301"
        with doc_ex.new("pdf", size=(300, 200)) as document:
            page = document.pages[0]
            page.draw_text(content, at=(20, 20), font_size=16, font=FONT)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertIn(content, reopened[0].get_text())
                _, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=72)))
                self.assertTrue(any(channel < 80 for channel in pixels[::4]))
            original = page.get_text()
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                page.draw_text("Invalid font", at=(20, 60), font=b"not a font")
            self.assertEqual(page.get_text(), original)

    def test_pdf_vector_fill_and_failed_path_are_atomic(self):
        with doc_ex.new("pdf", size=(100, 100)) as document:
            page = document.pages[0]
            page.draw_path([("move", 20, 20), ("line", 80, 20), ("line", 80, 80), ("line", 20, 80)], fill="red", stroke=None, close=True)
            _, pixels = rgba_pixels(png_chunks(page.get_pixmap(dpi=72)))
            center = (50 * 100 + 50) * 4
            self.assertEqual(pixels[center:center + 4], b"\xff\x00\x00\xff")
            with self.assertRaises(ValueError):
                page.draw_path([("line", 10, 10)])
            _, after = rgba_pixels(png_chunks(page.get_pixmap(dpi=72)))
            self.assertEqual(after, pixels)

    def test_docx_lists_and_revision_bound_links(self):
        with doc_ex.new("docx") as document:
            first = document.add_paragraph("First")
            first.set_list("numbered")
            second = document.add_paragraph("Second")
            second.set_list("numbered")
            third = document.add_paragraph("New sequence")
            third.set_list("numbered", restart=True)
            self.assertEqual(first.list_style, ("numbered", 0))
            self.assertEqual(second.list_style, ("numbered", 0))
            second.set_list("bullet", level=1)
            self.assertEqual(second.list_style, ("bullet", 1))
            second.set_list()
            self.assertIsNone(second.list_style)
            paragraph = document.add_paragraph("λ ")
            link = paragraph.add_run("Read more", bold=True)
            link.link = "https://example.com"
            self.assertEqual(link.link, "https://example.com")
            link.format(color="navy")
            with doc_ex.open(document.to_bytes()) as reopened:
                values = [item for page in reopened for item in page.get_links()]
                self.assertEqual([(item.text, item.uri) for item in values], [("Read more", "https://example.com")])
                self.assertEqual((values[0].metadata["start"], values[0].metadata["end"]), (2, 11))
            with self.assertRaises(ValueError):
                link.link = "https://bad url"
            self.assertEqual(link.link, "https://example.com")
            link.link = None
            self.assertIsNone(link.link)
            with doc_ex.open(document.convert("pdf")) as reopened:
                self.assertIn("New sequence", reopened[0].get_text())
