from __future__ import annotations

import base64
import json
import struct
import unittest
import zlib

import doc_ex
from fixtures import docx_fixture, package
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
P = "http://schemas.openxmlformats.org/package/2006/relationships"


def paragraph(text: str) -> str:
    return f"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"


def pixel() -> bytes:
    def chunk(name: bytes, value: bytes) -> bytes:
        return struct.pack(">I", len(value)) + name + value + struct.pack(">I", zlib.crc32(name + value))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1, 1, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(b"\0\x12\x34\x56")) + chunk(b"IEND", b"")


def stories_fixture(header: str | None = None, *, background: str = "") -> bytes:
    linked = '<w:p><w:r><w:t>λ </w:t></w:r><w:hyperlink r:id="link1"><w:r><w:t>Same</w:t></w:r></w:hyperlink><w:r><w:t> / </w:t></w:r><w:hyperlink r:id="link2"><w:r><w:t>Same</w:t></w:r></w:hyperlink></w:p>'
    image = '<w:p><w:r><w:drawing><wp:inline><wp:extent cx="127000" cy="127000"/><wp:docPr id="1" name="pixel.png" descr="Header pixel"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="pixel.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="image"/></pic:blipFill><pic:spPr><a:xfrm><a:ext cx="127000" cy="127000"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>'
    table = '<w:tbl><w:tr><w:tc>' + paragraph("Outer story") + '<w:tbl><w:tr><w:tc>' + paragraph("Inner story") + '</w:tc></w:tr></w:tbl><w:p/></w:tc></w:tr></w:tbl>'
    ns = f'xmlns:w="{W}" xmlns:r="{R}" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"'
    body = '<w:p><w:r><w:t>Body λ</w:t><w:footnoteReference w:id="7"/><w:endnoteReference w:id="9"/></w:r></w:p><w:p><w:r><w:br w:type="page"/><w:t>Second body</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="header"/><w:headerReference w:type="even" r:id="header"/><w:footerReference w:type="default" r:id="footer"/></w:sectPr>'
    relationship_types = {"header": "header", "footer": "footer", "footnote": "footnotes", "endnote": "endnotes"}
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "word/document.xml": f'<w:document {ns}>' + (f'<w:background w:color="{background}"/>' if background else '') + '<w:body>' + body + '</w:body></w:document>',
        "word/_rels/document.xml.rels": f'<Relationships xmlns="{P}">' + ''.join(f'<Relationship Id="{key}" Target="{key}s.xml" Type="{R}/{relationship_types[key]}"/>' for key in ("header", "footer", "footnote", "endnote")) + '</Relationships>',
        "word/headers.xml": f'<w:hdr {ns}>' + (header if header is not None else paragraph("Header only") + linked + image + table) + '</w:hdr>',
        "word/_rels/headers.xml.rels": f'<Relationships xmlns="{P}"><Relationship Id="link1" Target="https://example.com/first" Type="{R}/hyperlink" TargetMode="External"/><Relationship Id="link2" Target="https://example.com/second" Type="{R}/hyperlink" TargetMode="External"/><Relationship Id="image" Target="media/pixel.png" Type="{R}/image"/></Relationships>',
        "word/footers.xml": f'<w:ftr {ns}>' + paragraph("Footer only") + '</w:ftr>',
        "word/footnotes.xml": f'<w:footnotes {ns}><w:footnote w:id="-1" w:type="separator">' + paragraph("Custom separator") + '</w:footnote><w:footnote w:id="7">' + paragraph("Footnote only") + '</w:footnote></w:footnotes>',
        "word/endnotes.xml": f'<w:endnotes {ns}><w:endnote w:id="9">' + paragraph("Endnote only") + '</w:endnote></w:endnotes>',
        "word/media/pixel.png": pixel(),
        "word/media/unreferenced.png": pixel(),
    })


class DocxStoriesTests(unittest.TestCase):
    def test_auxiliary_stories_are_owned_and_do_not_repeat_on_pages(self) -> None:
        with doc_ex.open(stories_fixture()) as document:
            self.assertEqual(len(document), 2)
            stories = document.stories
            self.assertEqual({story.kind for story in stories}, {"header", "footer", "footnote", "endnote", "footnote_separator"})
            self.assertEqual(sum(story.kind == "header" for story in stories), 1)
            body = document.to_markdown()
            for label in ("Header only", "Footer only", "Footnote only", "Endnote only", "Outer story", "Inner story"):
                self.assertNotIn(label, body)
            header = next(story for story in stories if story.kind == "header")
            self.assertIn("Header only", header.get_text())
            self.assertEqual(header.get_text("markdown").count("Outer story"), 1)
            self.assertEqual(header.get_text("markdown").count("Inner story"), 1)
            self.assertFalse(any(page.get_images() for page in document))
            self.assertFalse(any(page.get_links() for page in document))
        self.assertIn("Header only", header.get_text())

    def test_exact_story_assets_and_note_reference_connections(self) -> None:
        payload = json.loads(doc_ex._core.extract(stories_fixture(), "docx"))
        stories = payload["stories"]
        header = next(story for story in stories if story["kind"] == "header")
        links = [link for block in header["blocks"] for link in block["links"]]
        self.assertEqual([(link["text"], link["metadata"]["start"], link["metadata"]["end"]) for link in links], [("Same", 2, 6), ("Same", 9, 13)])
        self.assertEqual([link["uri"] for link in links], ["https://example.com/first", "https://example.com/second"])
        images = [image for block in header["blocks"] for image in block["images"]]
        self.assertEqual(len(images), 1)
        self.assertEqual(base64.b64decode(images[0]["data"]), pixel())
        for asset in [*links, *images]:
            self.assertIsNone(asset["page"])
            self.assertEqual(asset["metadata"]["story_part_id"], header["metadata"]["part_id"])
            self.assertIn("source_path", asset["metadata"])
        table = next(block["table"] for block in header["blocks"] if block["kind"] == "table")
        self.assertIsNone(table["page"])
        self.assertIsNone(table["cells"][0][0]["blocks"][1]["table"]["page"])
        refs = payload["pages"][0]["blocks"][0]["metadata"]["story_refs"]
        by_id = {story["metadata"]["part_id"]: story["kind"] for story in stories if "part_id" in story["metadata"]}
        self.assertEqual([(by_id[ref["part_id"]], ref["offset"]) for ref in refs], [("footnote", 6), ("endnote", 6)])

    def test_comment_compatibility_projection_is_explicit(self) -> None:
        with doc_ex.open(docx_fixture()) as document:
            comments = [story for story in document.stories if story.kind == "comment"]
            self.assertEqual(len(comments), 2)
            self.assertTrue(all(story.metadata["legacy_page_projection"] for story in comments))
            self.assertEqual(document.to_markdown().count("Review this"), 1)

    def test_header_nesting_and_grid_limits_precede_layout(self) -> None:
        nested = paragraph("Deep")
        for _ in range(17):
            nested = '<w:tbl><w:tr><w:tc>' + nested + '<w:p/></w:tc></w:tr></w:tbl>'
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "16 levels"):
            doc_ex.open(stories_fixture(nested))
        grid = '<w:tbl><w:tr><w:trPr><w:gridBefore w:val="4294967295"/></w:trPr><w:tc><w:p/></w:tc></w:tr></w:tbl>'
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "63 columns"):
            doc_ex.open(stories_fixture(grid))

    def test_document_background_survives_png_and_pdf_rendering(self) -> None:
        with doc_ex.open(stories_fixture(background="123456")) as document:
            for source in (document, doc_ex.open(document.convert("pdf"))):
                try:
                    _, pixels = rgba_pixels(png_chunks(source.render_page(0, dpi=36)))
                    self.assertEqual(pixels[:4], b"\x12\x34\x56\xff")
                finally:
                    if source is not document:
                        source.close()


if __name__ == "__main__":
    unittest.main()
