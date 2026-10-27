"""Callback normalization follows stable slide/shape identities across reorder."""
import io
import unittest
import xml.etree.ElementTree as ET
import zipfile

import doc_ex
from doc_ex.errors import DocumentClosedError


class Number(float):
    def __new__(cls, callback, value):
        result = super().__new__(cls, value)
        result.callback = callback
        return result

    def __float__(self):
        self.callback()
        return float.__float__(self)


class Color(str):
    def __new__(cls, callback):
        result = super().__new__(cls, "red")
        result.callback = callback
        return result

    def lower(self):
        self.callback()
        return "red"


def operations():
    return {
        "background": lambda slide, shape, cb: setattr(slide, "background", Color(cb)),
        "rotation": lambda slide, shape, cb: setattr(shape, "rotation", Number(cb, 90)),
        "fill": lambda slide, shape, cb: setattr(shape, "fill", Color(cb)),
        "box": lambda slide, shape, cb: setattr(shape, "box", (Number(cb, 30), 35, 120, 70)),
        "color": lambda slide, shape, cb: setattr(shape, "color", Color(cb)),
        "font_size": lambda slide, shape, cb: setattr(shape, "font_size", Number(cb, 22)),
        "add_run": lambda slide, shape, cb: shape.add_run(" X", color=Color(cb)),
    }


def appended_run_properties(data, slide):
    ns = {"a": "http://schemas.openxmlformats.org/drawingml/2006/main"}
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        root = ET.fromstring(archive.read(f"ppt/slides/slide{slide}.xml"))
    runs = [run for run in root.findall(".//a:r", ns) if run.findtext("a:t", namespaces=ns) == " X"]
    if len(runs) != 1:
        raise AssertionError("expected exactly one appended run")
    return runs[0].find("a:rPr", ns), ns


class SlideShapeOwnershipTests(unittest.TestCase):
    def document(self, same_position=True):
        document = doc_ex.new("pptx", size=(240, 180))
        first, second = document.slides.add(), document.slides.add()
        a = first.add_text("ALPHA", box=(20, 30, 100, 60), font_size=16)
        b = second.add_text("BRAVO", box=(20 if same_position else 120, 30, 100, 60), font_size=16)
        a.fill, b.fill = "blue", "green"
        return document, first, a, second, b

    def test_reordering_updates_same_stable_target_with_exact_control_output(self):
        for name, operation in operations().items():
            for same_position in (True, False):
                with self.subTest(name=name, same_position=same_position):
                    document, slide, shape, other_slide, other_shape = self.document(same_position)
                    with document, document.copy() as control:
                        original_ids = (slide._id, shape._id, other_slide._id, other_shape._id)
                        self.assertNotEqual(shape._id, other_shape._id)
                        original_other = document.render_page(1, dpi=72)
                        original_target = document.render_page(0, dpi=72)
                        control_slide, control_shape = control.slides[0], control.slides[0].shapes[0]
                        control.slides.move(0, -1)
                        operation(control_slide, control_shape, lambda: None)
                        old_range = shape[:1]
                        result = operation(slide, shape, lambda: document.slides.move(0, -1))
                        self.assertEqual((slide._id, shape._id, other_slide._id, other_shape._id), original_ids)
                        self.assertEqual((slide.number, other_slide.number), (1, 0))
                        self.assertEqual(document.slides[1].shapes[0]._id, shape._id)
                        self.assertEqual(document.slides[0].shapes[0]._id, other_shape._id)
                        self.assertEqual(document.render_page(0, dpi=72), original_other)
                        self.assertNotEqual(document.render_page(1, dpi=72), original_target)
                        self.assertEqual(document.to_bytes(), control.to_bytes())
                        for index in (0, 1):
                            self.assertEqual(document.render_page(index, dpi=72), control.render_page(index, dpi=72))
                        if name == "add_run":
                            self.assertEqual(result.text, " X")
                            props, ns = appended_run_properties(document.to_bytes(), 2)
                            self.assertEqual(props.find("a:solidFill/a:srgbClr", ns).attrib["val"].upper(), "FF0000")
                            with self.assertRaisesRegex(ValueError, "stale"):
                                _ = old_range.text
                        for output, expected in ((document.to_bytes(), control.to_bytes()),
                                                 (document.convert("pdf"), control.convert("pdf"))):
                            with doc_ex.open(output) as reopened, doc_ex.open(expected) as baseline:
                                self.assertEqual(reopened.page_count, 2)
                                self.assertIn("BRAVO", reopened[0].get_text())
                                self.assertIn("ALPHA", reopened[1].get_text())
                                self.assertEqual(reopened.render_page(1, dpi=72), baseline.render_page(1, dpi=72))

    def test_deleted_or_closed_targets_fail_before_callbacks(self):
        for name, operation in operations().items():
            for invalid in ("closed", "slide", "shape"):
                if name == "background" and invalid == "shape":
                    continue
                with self.subTest(name=name, invalid=invalid):
                    document, slide, shape, _, _ = self.document()
                    with document:
                        if invalid == "closed":
                            document.close()
                            error = DocumentClosedError
                        elif invalid == "slide":
                            del document.slides[0]
                            error = IndexError
                        else:
                            del slide.shapes[0]
                            error = ValueError
                        def forbidden():
                            self.fail("callback invoked on an already-invalid target")
                        with self.assertRaises(error):
                            operation(slide, shape, forbidden)

    def test_callbacks_delete_target_without_extra_mutation(self):
        for name, operation in operations().items():
            for delete in ("slide", "shape"):
                if name == "background" and delete == "shape":
                    continue
                with self.subTest(name=name, delete=delete):
                    document, slide, shape, _, _ = self.document()
                    with document:
                        expected = []
                        def callback():
                            if delete == "slide":
                                del document.slides[0]
                            else:
                                del slide.shapes[0]
                            expected.append(document.to_bytes())
                        with self.assertRaises(IndexError if delete == "slide" else ValueError):
                            operation(slide, shape, callback)
                        self.assertEqual(document.to_bytes(), expected[0])

    def test_close_and_raising_callbacks(self):
        for name, operation in operations().items():
            with self.subTest(name=name):
                document, slide, shape, _, _ = self.document()
                with self.assertRaises(DocumentClosedError):
                    operation(slide, shape, document.close)
                document, slide, shape, _, _ = self.document()
                with document:
                    before, pixels = document.to_bytes(), document.render_page(0, dpi=72)
                    def failure():
                        raise RuntimeError("caller failure")
                    with self.assertRaisesRegex(RuntimeError, "caller failure"):
                        operation(slide, shape, failure)
                    self.assertEqual(document.to_bytes(), before)
                    self.assertEqual(document.render_page(0, dpi=72), pixels)

    def test_copy_callback_keeps_owner_and_range_identity(self):
        for name, operation in operations().items():
            with self.subTest(name=name):
                document, slide, shape, _, _ = self.document()
                with document:
                    before, copies = document.to_bytes(), []
                    result = operation(slide, shape, lambda: copies.append(document.copy()))
                    try:
                        self.assertEqual(len(copies), 1)
                        self.assertEqual(copies[0].to_bytes(), before)
                        self.assertNotEqual(document.to_bytes(), before)
                        if name == "add_run":
                            self.assertEqual(result.text, " X")
                            result.format(bold=True)
                            self.assertEqual(result.text, " X")
                            props, _ = appended_run_properties(document.to_bytes(), 1)
                            self.assertIn(props.attrib["b"], ("1", "true"))
                            self.assertEqual(copies[0].slides[0].shapes[0].text, "ALPHA")
                    finally:
                        for copied in copies:
                            copied.close()


if __name__ == "__main__":
    unittest.main()
