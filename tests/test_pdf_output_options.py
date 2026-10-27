"""PDF object representation options reuse the native full writer and shared save path."""
import io
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_pdf_text_boxes import FONT, quadrant_png, pixel
from test_render_geometry import geometry_pdf


def classic_xref(data):
    offset = int(data.rsplit(b"startxref\n", 1)[1].splitlines()[0])
    return data[offset:].startswith(b"xref\n")


def rich_document():
    document = doc_ex.new("pdf", size=(300, 400))
    page = document.pages[0]
    page.draw_text("Café Ω stable", at=(20, 20), font=FONT, font_size=16)
    page.add_image(quadrant_png(), box=(20, 60, 60, 60))
    page.draw_path([("move", 120, 60), ("line", 180, 60), ("line", 180, 100)],
                   fill="blue", close=True)
    page.annotations.add_link(box=(20, 20, 160, 25), uri="https://example.com")
    page.annotations.add("ellipse", box=(200, 60, 60, 50), color="red", text="Comment")
    document.bookmarks.add("Start", 0)
    document.update_metadata(title="Stable output", author="Fixture")
    page.fields.add("name", (20, 160, 180, 30)).value = "Alice"
    page.fields.add("approved", (20, 210, 24, 24), kind="checkbox").value = True
    page.fields.add("country", (20, 260, 180, 30), kind="choice", options=["Canada", "France"]).value = "France"
    return document


class PdfOutputOptionsTests(unittest.TestCase):
    def test_rich_both_modes_roundtrip_and_owner_cache_are_unchanged(self):
        with rich_document() as document:
            default = document.to_bytes()
            self.assertEqual(default, document.to_bytes(object_streams=True))
            self.assertEqual(default, bytes(document._native.to_bytes()))
            self.assertEqual(document.to_bytes(object_streams=False),
                             bytes(document._native.to_bytes(object_streams=False)))
            baseline = document.render_page(0, dpi=72)
            cached = document._cached
            field = document.fields["name"]
            bookmark = document.bookmarks[0]
            comment = next(item for item in document.pages[0].annotations if item.kind == "ellipse")
            for packed in (False, True, False):
                with self.subTest(object_streams=packed):
                    data = document.to_bytes(object_streams=packed)
                    self.assertEqual(data, document.to_bytes(object_streams=packed))
                    self.assertEqual(classic_xref(data), not packed)
                    with doc_ex.open(data) as reopened:
                        self.assertIn("Café Ω stable", reopened[0].get_text())
                        self.assertEqual(reopened.metadata["title"], "Stable output")
                        self.assertEqual(reopened.metadata["author"], "Fixture")
                        self.assertEqual(len(reopened[0].get_images()), 1)
                        self.assertEqual(reopened[0].get_links()[0].uri, "https://example.com")
                        self.assertEqual(reopened.render_page(0, dpi=72), baseline)
                    self.assertEqual(pixel(baseline, 30, 70), bytes((255, 0, 0, 255)))
                    self.assertIs(document._cached, cached)
                    self.assertEqual(field.value, "Alice")
                    self.assertEqual(bookmark.title, "Start")
                    self.assertEqual(comment.text, "Comment")
                    with document.copy() as copied:
                        self.assertEqual(copied.to_bytes(object_streams=packed), data)
            self.assertEqual(document.to_bytes(), default)
            self.assertEqual(document.fields.flatten(), 3)
            for packed in (False, True):
                with doc_ex.open(document.to_bytes(object_streams=packed)) as flattened:
                    self.assertIn("Alice", flattened[0].get_text())
                    self.assertIn("France", flattened[0].get_text())
                    self.assertEqual(flattened.render_page(0, dpi=72), baseline)

    def test_conservative_imports_subsets_and_geometry_both_modes(self):
        for rotation in (0, 90, 180, 270):
            with self.subTest(rotation=rotation), doc_ex.open(geometry_pdf(rotation)) as source, source.edit() as edited:
                edited.pages[0].draw_text("Retained", at=(30, 50))
                baseline = edited.render_page(0, dpi=144)
                for packed in (False, True):
                    output = edited.to_bytes(object_streams=packed)
                    with doc_ex.open(output) as reopened, reopened.edit() as imported:
                        self.assertEqual(reopened.render_page(0, dpi=144), baseline)
                        self.assertIn("Retained", reopened[0].get_text())
                        with imported.copy(pages=[0]) as subset:
                            for mode in (False, True):
                                with doc_ex.open(subset.to_bytes(object_streams=mode)) as result:
                                    self.assertEqual(result.render_page(0, dpi=144), baseline)

    def test_save_path_stream_atomicity_permissions_and_default(self):
        with doc_ex.new("pdf") as document, tempfile.TemporaryDirectory() as directory:
            document.pages[0].draw_text("Saved", at=(20, 30))
            path = Path(directory) / "result.pdf"
            for packed in (False, True):
                expected = document.to_bytes(object_streams=packed)
                path.write_bytes(b"previous")
                path.chmod(0o640)
                # Windows exposes writable/read-only state, not POSIX permission bits.
                previous_mode = stat.S_IMODE(path.stat().st_mode)
                if os.name == "posix":
                    self.assertEqual(previous_mode, 0o640)
                with patch("doc_ex._authoring.os.replace", side_effect=OSError("replace failure")):
                    with self.assertRaisesRegex(OSError, "replace failure"):
                        document.save(path, object_streams=packed)
                self.assertEqual(path.read_bytes(), b"previous")
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), previous_mode)
                self.assertEqual(list(Path(directory).iterdir()), [path])
                document.save(path, object_streams=packed)
                self.assertEqual(path.read_bytes(), expected)
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), previous_mode)
                stream = io.BytesIO()
                self.assertIsNone(document.save(stream, object_streams=packed))
                self.assertEqual(stream.getvalue(), expected)
                with self.assertRaises(ValueError):
                    document.save(Path(directory) / "wrong.docx", object_streams=packed)
                class ShortWriter:
                    def write(self, data):
                        return len(data) - 1
                with self.assertRaisesRegex(OSError, "complete document"):
                    document.save(ShortWriter(), object_streams=packed)
            default = io.BytesIO()
            document.save(default)
            self.assertEqual(default.getvalue(), document.to_bytes(object_streams=True))

    def test_callback_order_and_closed_type_precedence(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "callback.pdf"
            path.write_bytes(b"unchanged")
            for packed in (False, True):
                document = doc_ex.new("pdf")
                class ClosingPath:
                    def __fspath__(self):
                        document.close()
                        return str(path)
                with self.assertRaises(DocumentClosedError):
                    document.save(ClosingPath(), object_streams=packed)
                self.assertEqual(path.read_bytes(), b"unchanged")
                with doc_ex.new("pdf") as document:
                    class EditingPath:
                        def __fspath__(self):
                            document.update_metadata(title="After callback")
                            return str(path)
                    document.save(EditingPath(), object_streams=packed)
                    self.assertEqual(path.read_bytes(), document.to_bytes(object_streams=packed))
                    with doc_ex.open(path) as reopened:
                        self.assertEqual(reopened.metadata["title"], "After callback")
                    path.write_bytes(b"unchanged")
                with doc_ex.new("pdf") as document:
                    writes = []
                    class ClosingWriteProperty:
                        @property
                        def write(self):
                            document.close()
                            return lambda data: writes.append(data)
                    with self.assertRaises(DocumentClosedError):
                        document.save(ClosingWriteProperty(), object_streams=packed)
                    self.assertEqual(writes, [])
                with doc_ex.new("pdf") as document:
                    expected = document.to_bytes(object_streams=packed)
                    class EditingWriter:
                        def write(self, data):
                            writes.append(data)
                            document.update_metadata(title="After serialization")
                            return len(data)
                    document.save(EditingWriter(), object_streams=packed)
                    self.assertEqual(writes, [expected])
                    self.assertNotEqual(document.to_bytes(object_streams=packed), expected)
            calls = []
            class BoolBomb:
                def __bool__(self):
                    raise AssertionError("object_streams must not coerce truthiness")
            class UntouchedDestination:
                @property
                def write(self):
                    calls.append("write")
                    raise AssertionError("closed/invalid options must not access destination")
            with doc_ex.new("pdf") as document:
                for value in (None, 0, 1, "false", [], object(), BoolBomb()):
                    with self.subTest(value=value), self.assertRaises(TypeError):
                        document.to_bytes(object_streams=value)
                    with self.assertRaises(TypeError):
                        document.save(UntouchedDestination(), object_streams=value)
                with self.assertRaises(TypeError):
                    document.to_bytes(False)
            for value in (True, False, object()):
                with self.assertRaises(DocumentClosedError):
                    document.to_bytes(object_streams=value)
                with self.assertRaises(DocumentClosedError):
                    document.save(UntouchedDestination(), object_streams=value)
            self.assertEqual(calls, [])

    def test_other_format_signatures_and_save_path_remain_unchanged(self):
        for kind in ("docx", "pptx"):
            with self.subTest(kind=kind), doc_ex.new(kind) as document:
                if kind == "docx":
                    document.add_paragraph("Unchanged")
                else:
                    document.slides.add().add_text("Unchanged", box=(20, 20, 200, 50))
                expected = document.to_bytes()
                stream = io.BytesIO()
                document.save(stream)
                self.assertEqual(stream.getvalue(), expected)
                with self.assertRaises(TypeError):
                    document.to_bytes(object_streams=False)
                with self.assertRaises(TypeError):
                    document.save(io.BytesIO(), object_streams=False)


if __name__ == "__main__":
    unittest.main()
