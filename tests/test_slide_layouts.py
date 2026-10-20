import io
import unittest
import xml.etree.ElementTree as ET
import zipfile

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

NS = {"p": "http://schemas.openxmlformats.org/presentationml/2006/main",
      "a": "http://schemas.openxmlformats.org/drawingml/2006/main"}


def xml_part(data, name):
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        return ET.fromstring(archive.read(name))


class SlideLayoutTests(unittest.TestCase):
    def test_layout_placeholders_render_and_preserve_inheritance_in_pptx_and_pdf(self):
        cases = (("title", ("center_title", "subtitle")),
                 ("title_and_content", ("title", "content")),
                 ("two_content", ("title", "content", "content")))
        with doc_ex.new("pptx", size=(720, 540)) as document:
            expected = []
            boxes = []
            for index, (layout, roles) in enumerate(cases):
                slide = document.slides.add(layout=layout)
                self.assertEqual(tuple(shape.placeholder for shape in slide.shapes), roles)
                slide.name = "Layout " + layout
                labels = []
                geometry = []
                for order, shape in enumerate(slide.shapes):
                    self.assertEqual(shape.text, "")
                    self.assertEqual(shape.rotation, 0)
                    self.assertGreater(shape.box[2], 0)
                    self.assertGreater(shape.box[3], 0)
                    label = f"Label {index + 1} {order + 1}"
                    shape.text = label
                    labels.append(label)
                    geometry.append(shape.box)
                expected.append(labels)
                boxes.append(geometry)
            native = document.to_bytes()
            ids = []
            for index in range(3):
                slide = xml_part(native, f"ppt/slides/slide{index + 1}.xml")
                for shape in slide.findall(".//p:sp", NS):
                    self.assertIsNotNone(shape.find("p:nvSpPr/p:nvPr/p:ph", NS))
                    self.assertIsNone(shape.find("p:spPr/a:xfrm", NS))
                    ids.append(shape.find("p:nvSpPr/p:cNvPr", NS).attrib["id"])
            self.assertEqual(len(ids), len(set(ids)))
            for output in (native, document.convert("pdf")):
                with doc_ex.open(output) as reopened:
                    self.assertEqual(len(reopened), 3)
                    for index, labels in enumerate(expected):
                        text = reopened[index].get_text()
                        for label in labels:
                            self.assertEqual(text.count(label), 1)
                        width, pixels = rgba_pixels(png_chunks(reopened.render_page(index, dpi=72)))
                        self.assertEqual(width, 720)
                        for x, y, w, h in boxes[index]:
                            ink = sum(min(pixels[(py * width + px) * 4:(py * width + px) * 4 + 3]) < 180
                                      for py in range(int(y), int(y + h))
                                      for px in range(int(x), int(x + w)))
                            self.assertGreater(ink, 20)

    def test_explicit_geometry_changes_preserve_other_placeholders_and_master_parts(self):
        with doc_ex.new("pptx", size=(720, 540)) as document:
            slide = document.slides.add(layout="title_and_content")
            title, content = slide.shapes
            title.text = "Title"
            content.text = "Body"
            initial = document.to_bytes()
            title_box, content_box = title.box, content.box
            title.rotation = 15
            self.assertEqual(title.box, title_box)
            self.assertEqual(content.box, content_box)
            title.box = (30, 30, 600, 80)
            self.assertEqual(title.rotation, 15)
            self.assertEqual(title.placeholder, "title")
            self.assertEqual(content.placeholder, "content")
            updated = document.to_bytes()
            root = xml_part(updated, "ppt/slides/slide1.xml")
            shapes = root.findall(".//p:sp", NS)
            self.assertIsNotNone(shapes[0].find("p:spPr/a:xfrm", NS))
            self.assertIsNone(shapes[1].find("p:spPr/a:xfrm", NS))
            with zipfile.ZipFile(io.BytesIO(initial)) as old, zipfile.ZipFile(io.BytesIO(updated)) as new:
                parts = [name for name in old.namelist() if name.startswith(("ppt/slideLayouts/", "ppt/slideMasters/", "ppt/theme/"))]
                for name in parts:
                    self.assertEqual(old.read(name), new.read(name), name)
            with self.assertRaises(AttributeError):
                title.placeholder = "content"

    def test_blank_compatibility_invalid_layout_atomicity_and_handle_identity(self):
        with doc_ex.new("pptx", size=(720, 540)) as document:
            blank = document.slides.add()
            self.assertEqual(len(blank.shapes), 0)
            ordinary = blank.add_text("Ordinary", box=(20, 20, 200, 40))
            self.assertIsNone(ordinary.placeholder)
            populated = document.slides.add(layout="title")
            title = populated.shapes[0]
            title.text = "Stable title"
            before = document.to_bytes()
            png = document.render_page(populated.number, dpi=72)
            for layout in ("unknown", "Title", "", "comparison", "title_and_vertical_text", "vertical_title_and_text"):
                with self.subTest(layout=layout), self.assertRaises((ValueError, UnsupportedFormatError)):
                    document.slides.add(layout=layout)
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(populated.number, dpi=72), png)
            document.slides.move(1, 0)
            self.assertEqual(populated.number, 0)
            self.assertEqual(title.text, "Stable title")
            self.assertEqual(title.placeholder, "center_title")
            del document.slides[0]
            with self.assertRaises((ValueError, IndexError)):
                _ = title.placeholder
        with self.assertRaises(DocumentClosedError):
            _ = ordinary.placeholder


if __name__ == "__main__":
    unittest.main()
