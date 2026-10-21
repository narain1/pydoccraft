"""Original imported-size fixtures for the dedicated native dependency upgrade."""
from contextlib import ExitStack
import math
import struct
import unittest

import doc_ex
from fixtures import package
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
LABEL = "SIZEKEEP"
URI = "https://example.test/default-size"


def imported_size(size: int | None, *, styles: bool = True) -> bytes:
    """Synthetic OOXML, not runtime format logic or third-party document bytes."""
    relationships = f'<Relationship Id="link" Type="{R}/hyperlink" Target="{URI}" TargetMode="External"/>'
    if styles:
        relationships += f'<Relationship Id="styles" Type="{R}/styles" Target="styles.xml"/>'
    members = {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "_rels/.rels": f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="main" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>',
        "word/_rels/document.xml.rels": f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{relationships}</Relationships>',
        "word/document.xml": f'''<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>
<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/><w:ind w:left="0" w:right="0" w:firstLine="0"/></w:pPr>
<w:hyperlink r:id="link"><w:r><w:rPr><w:rFonts w:ascii="DejaVu Sans" w:hAnsi="DejaVu Sans"/><w:b/><w:color w:val="FF0000"/><w:kern w:val="3276"/><w:u w:val="none"/></w:rPr><w:t>{LABEL}</w:t></w:r></w:hyperlink></w:p>
<w:sectPr><w:pgSz w:w="3200" w:h="1200"/><w:pgMar w:top="240" w:bottom="240" w:left="240" w:right="240"/></w:sectPr>
</w:body></w:document>''',
    }
    if styles:
        ordinary = "" if size is None else f'<w:sz w:val="{size * 2}"/>'
        members["word/styles.xml"] = f'<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr>{ordinary}</w:rPr></w:rPrDefault></w:docDefaults><w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style></w:styles>'
    return package(members)


def ink_bounds(test: unittest.TestCase, png: bytes, dpi: int) -> tuple[int, int, int, int]:
    chunks = png_chunks(png)
    width, height = struct.unpack(">II", chunks[b"IHDR"][:8])
    test.assertEqual((width, height), (math.ceil(160 * dpi / 72), math.ceil(60 * dpi / 72)))
    decoded_width, rgba = rgba_pixels(chunks)
    test.assertEqual(decoded_width, width)
    test.assertEqual(len(rgba), width * height * 4)
    points = []
    for pos in range(0, len(rgba), 4):
        red, green, blue, alpha = rgba[pos:pos + 4]
        if red > green + 32 and red > blue + 32 and alpha > 128:
            pixel = pos // 4
            points.append((pixel % width, pixel // width))
    test.assertGreater(len(points), 20)
    xs, ys = zip(*points)
    bounds = min(xs), min(ys), max(xs) + 1, max(ys) + 1
    test.assertGreater(bounds[0], 0)
    test.assertGreater(bounds[1], 0)
    test.assertLess(bounds[2], width)
    test.assertLess(bounds[3], height)
    return bounds


class DocxDefaultSizeUpgradeTests(unittest.TestCase):
    def test_imported_omission_matches_eleven_and_preserves_explicit_and_missing_styles(self):
        fixtures = {
            "omitted": imported_size(None),
            "ten": imported_size(10),
            "eleven": imported_size(11),
            "twelve": imported_size(12),
            "no_styles": imported_size(None, styles=False),
        }
        output = {}
        with ExitStack() as stack:
            documents = {}
            pdfs = {}
            for name, data in fixtures.items():
                caller = bytearray(data)
                document = stack.enter_context(doc_ex.open(caller, filetype="docx"))
                caller[:] = b"\0" * len(caller)
                documents[name] = document
                self.assertEqual(len(document), 1)
                self.assertEqual(document[0].get_text().strip(), LABEL)
                self.assertEqual([link.uri for link in document[0].get_links()], [URI])
                pdf = stack.enter_context(doc_ex.open(document.convert("pdf")))
                pdfs[name] = pdf
                self.assertEqual(len(pdf), 1)
                self.assertEqual(pdf[0].get_text().strip(), LABEL)
                self.assertEqual([link.uri for link in pdf[0].get_links()], [URI])
            for dpi in (72, 144, 300):
                images = {}
                bounds = {}
                for name in fixtures:
                    images[name] = (
                        documents[name].render_page(0, dpi=dpi),
                        pdfs[name].render_page(0, dpi=dpi),
                    )
                    bounds[name] = tuple(ink_bounds(self, image, dpi) for image in images[name])
                    for original, exported in zip(*bounds[name]):
                        self.assertLessEqual(abs(original - exported), 2, (name, dpi, bounds[name]))
                # These unchanged controls run at all DPIs before the desired
                # implicit-policy assertion, including on the old native wheel.
                self.assertEqual(images["no_styles"], images["twelve"])
                for backend in (0, 1):
                    ten = bounds["ten"][backend]
                    eleven = bounds["eleven"][backend]
                    twelve = bounds["twelve"][backend]
                    self.assertLess(ten[2] - ten[0], eleven[2] - eleven[0])
                    self.assertLess(eleven[2] - eleven[0], twelve[2] - twelve[0])
                output[dpi] = images
            for dpi, images in output.items():
                self.assertEqual(images["omitted"], images["eleven"], f"native implicit size policy at {dpi} DPI")
