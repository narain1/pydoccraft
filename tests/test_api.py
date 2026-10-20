from __future__ import annotations

import io
import tempfile
import unittest
import zipfile
from pathlib import Path

import doc_ex
from doc_ex import Block, Link, Page, Table
from doc_ex.errors import (
    DocumentClosedError,
    MalformedDocumentError,
    UnsafeArchiveError,
    UnsupportedFormatError,
)
from fixtures import (
    docx_fixture,
    inherited_resources_pdf_fixture,
    inline_break_docx_fixture,
    nested_table_and_break_before_docx_fixture,
    package,
    pdf_fixture,
    pptx_fixture,
    three_page_docx_fixture,
)


class ApiTests(unittest.TestCase):
    def test_pdf_path_bytes_links_images_and_lifecycle(self) -> None:
        data = pdf_fixture()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "fixture.pdf"
            path.write_bytes(data)
            with doc_ex.open(path) as document:
                self.assertEqual(document.page_count, 2)
                self.assertEqual(document[-1].number, 1)
                self.assertIn("Page one", document[0].get_text())
                self.assertEqual(document[0].get_links()[0].uri, "https://example.com")
                self.assertEqual(
                    document[0].find_tables()[0].rows,
                    [["Name", "Score"], ["Alice", "91"]],
                )
                source_text = document[0].get_text()
                source_markdown = document[0].get_text("markdown")
                for value in ("Name", "Score", "Alice", "91"):
                    self.assertEqual(source_text.count(value), 1)
                    self.assertEqual(source_markdown.count(value), 1)
                image = document[1].get_images()[0]
                self.assertTrue(image.data.startswith(b"\x89PNG\r\n\x1a\n"))
                self.assertEqual(image.metadata["width"], 1)
                self.assertEqual(image.metadata["height"], 1)
                self.assertIsInstance(document[0].get_text("dict"), dict)
                self.assertIsInstance(document[0].get_text("json"), str)
        document = doc_ex.open(data)
        page = document[0]
        document.close()
        document.close()
        self.assertIn("Page one", page.get_text())
        with self.assertRaises(DocumentClosedError):
            _ = document.page_count

    def test_pdf_render_page_and_pixmap(self) -> None:
        with doc_ex.open(pdf_fixture()) as document:
            pixmap = document[0].get_pixmap()
            self.assertTrue(pixmap.startswith(b"\x89PNG\r\n\x1a\n"))
            self.assertEqual(document.render_page(-1, dpi=72)[:8], b"\x89PNG\r\n\x1a\n")
            with self.assertRaises(IndexError):
                document.render_page(2)
            with self.assertRaises(UnsafeArchiveError):
                document.render_page(0, dpi=0)

    def test_pdf_inherited_resources_and_image_metadata(self) -> None:
        with doc_ex.open(inherited_resources_pdf_fixture()) as document:
            image = document[0].get_images()[0]
            self.assertTrue(image.data.startswith(b"\x89PNG\r\n\x1a\n"))
            self.assertEqual(image.metadata["width"], 1)
            self.assertEqual(image.metadata["height"], 1)
            self.assertIsNotNone(image.bbox)

    def test_docx_structure_markdown_and_owned_data(self) -> None:
        data = docx_fixture()
        document = doc_ex.open(data)
        self.assertEqual(document.page_count, 2)
        self.assertEqual(document[0].blocks[0].kind, "heading")
        self.assertEqual(document[0].blocks[0].style, "CustomHeading")
        self.assertEqual(document[0].blocks[0].metadata["outline_level"], 1)
        self.assertEqual(document[0].blocks[0].metadata["style_name"], "Audit heading")
        self.assertEqual(
            [(link.text, link.uri, link.metadata["start"], link.metadata["end"]) for link in document[0].get_links()],
            [
                ("Example link", "https://example.com", 0, 12),
                ("Example link", "https://example.org", 17, 29),
            ],
        )
        self.assertEqual(document[0].find_tables()[0].rows[1], ["Alice", "91"])
        image = document[0].get_images()[0]
        self.assertEqual(image.data[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(image.alt, "Red pixel")
        self.assertAlmostEqual(image.metadata["width"], 0.75)
        self.assertAlmostEqual(image.metadata["height"], 0.75)
        comments = [block for block in document[0].blocks if block.kind == "comment"]
        self.assertEqual([comment.text for comment in comments], ["Review this", "Reply here"])
        self.assertEqual(comments[1].metadata["parent_id"], 0)
        self.assertFalse(any(block.kind == "comment" for block in document[1].blocks))
        markdown = document.to_markdown()
        self.assertIn("# Example heading", markdown)
        self.assertIn("[Example link](https://example.com)", markdown)
        self.assertIn("| Name | Score |", markdown)
        self.assertIn("Review this", markdown)
        self.assertIn("![Red pixel](images/image1.png)", markdown)
        self.assertNotIn("unreferenced.png", markdown)
        self.assertIn("Second page", document[1].get_text())

    def test_docx_inline_page_break_splits_one_paragraph(self) -> None:
        with doc_ex.open(inline_break_docx_fixture()) as document:
            self.assertEqual(document.page_count, 2)
            self.assertEqual(document[0].get_text(), "A")
            self.assertEqual(document[1].get_text(), "B")

    def test_docx_nested_table_break_before_and_render(self) -> None:
        with doc_ex.open(nested_table_and_break_before_docx_fixture()) as document:
            self.assertEqual(document.page_count, 2)
            self.assertEqual(document[0].find_tables()[0].rows, [["Outer\nInner"]])
            self.assertEqual(document[1].get_text(), "After break")
            self.assertTrue(document[0].get_pixmap(dpi=72).startswith(b"\x89PNG\r\n\x1a\n"))
            self.assertTrue(document.render_page(-1)[:8] == b"\x89PNG\r\n\x1a\n")
            with self.assertRaises(UnsafeArchiveError):
                document.render_page(0, dpi=0)

    def test_pptx_boundaries_notes_comments_links_and_images(self) -> None:
        with doc_ex.open(pptx_fixture(), filetype="pptx") as document:
            self.assertEqual(document.page_count, 1)
            page = document[0]
            paragraphs = [block.text for block in page.blocks if block.kind == "paragraph"]
            self.assertEqual(paragraphs, ["Repeat middle Repeat\nline two", "Second paragraph"])
            self.assertEqual(page.find_tables()[0].rows, [["Key\ndetail", "Value"], ["A", "B"]])
            self.assertEqual(
                [(link.text, link.uri, link.metadata["start"], link.metadata["end"]) for link in page.get_links()],
                [
                    ("Repeat", "https://example.com/one", 0, 6),
                    ("Repeat", "https://example.com/two", 14, 20),
                ],
            )
            image = page.get_images()[0]
            self.assertTrue(image.data.startswith(b"\x89PNG"))
            self.assertEqual(image.alt, "Red pixel")
            self.assertEqual(image.metadata["width"], 1)
            self.assertEqual(image.metadata["height"], 1)
            self.assertIsNotNone(image.bbox)
            self.assertEqual(image.page, 0)
            self.assertTrue(page.get_pixmap(dpi=72).startswith(b"\x89PNG\r\n\x1a\n"))
            self.assertTrue(document.render_page(-1)[:8] == b"\x89PNG\r\n\x1a\n")
            with self.assertRaises(UnsafeArchiveError):
                document.render_page(0, dpi=0)
            markdown = document.to_markdown()
            self.assertIn("[Repeat](https://example.com/one)", markdown)
            self.assertIn("[Repeat](https://example.com/two)", markdown)
            self.assertNotIn("https://example.com/unused", markdown)
            self.assertIn("**Notes:**", markdown)
            self.assertIn("Speaker\nnotes", markdown)
            self.assertIn("Next note", markdown)
            self.assertIn("**Comment:**", markdown)
            self.assertIn("Check slide", markdown)

    def test_table_text_and_repeated_exact_link_offsets(self) -> None:
        table = Table([["Only", "Table"], ["A", "B"]])
        page = Page(0, [Block("table", table=table)], tables=[table])
        self.assertEqual(page.get_text(), "Only\tTable\nA\tB")

        first = Link("https://one", "same", metadata={"start": 0, "end": 4})
        second = Link("https://two", "same", metadata={"start": 9, "end": 13})
        block = Block("paragraph", "same and same", links=[first, second])
        page = Page(0, [block], links=[first, second])
        self.assertEqual(
            page.get_text("markdown"),
            "[same](https://one) and [same](https://two)",
        )

    def test_conversion_reopen_no_table_duplication_and_empty_pages(self) -> None:
        docx = doc_ex.convert(pdf_fixture(), "docx", filetype="pdf")
        with doc_ex.open(docx) as converted:
            converted_text = "\n".join(page.get_text() for page in converted)
            self.assertIn("Page one", converted_text)
            self.assertIn("Second page image", converted_text)
            self.assertEqual(converted_text.count("Name"), 1)
            self.assertEqual(converted_text.count("Alice"), 1)

        pdf = doc_ex.convert(docx_fixture(), "pdf", filetype="docx")
        with doc_ex.open(pdf) as converted:
            self.assertGreaterEqual(converted.page_count, 2)
            self.assertIn("Example heading", converted[0].get_text())

        blank_pdf = doc_ex.convert(three_page_docx_fixture(), "pdf", filetype="docx")
        with doc_ex.open(blank_pdf) as converted:
            self.assertEqual(converted.page_count, 3)
            self.assertEqual(converted[0].get_text(), "A")
            self.assertEqual(converted[1].get_text(), "")
            self.assertEqual(converted[2].get_text(), "B")

        with doc_ex.open(pdf_fixture()) as source:
            reopened = doc_ex.open(source.convert("docx"))
            self.assertIn("Second page image", "\n".join(page.get_text() for page in reopened))
        pptx_pdf = doc_ex.convert(pptx_fixture(), "pdf", filetype="pptx")
        self.assertTrue(pptx_pdf.startswith(b"%PDF-"))
        with doc_ex.open(pptx_pdf) as converted:
            self.assertEqual(converted.page_count, 1)
            converted_text = converted[0].get_text()
            self.assertIn("Repeat middle Repeat", converted_text)
            self.assertIn("Second paragraph", converted_text)

        with doc_ex.open(pptx_fixture(), filetype="pptx") as source:
            reopened = doc_ex.open(source.convert("pdf"))
            self.assertIn("Repeat middle Repeat", reopened[0].get_text())
        with self.assertRaises(UnsupportedFormatError):
            doc_ex.convert(pptx_fixture(), "docx", filetype="pptx")

    def test_docx_unicode_conversion_reopens(self) -> None:
        pdf = doc_ex.convert(
            inline_break_docx_fixture("café", "naïve"), "pdf", filetype="docx"
        )
        self.assertTrue(pdf.startswith(b"%PDF-"))
        with doc_ex.open(pdf) as converted:
            text = "\n".join(page.get_text() for page in converted)
            self.assertIn("café", text)
            self.assertIn("naïve", text)

    def test_detection_and_stable_errors(self) -> None:
        self.assertEqual(doc_ex.open(docx_fixture()).metadata["format"], "docx")
        self.assertEqual(doc_ex.open(pptx_fixture()).metadata["format"], "pptx")
        with self.assertRaises(UnsupportedFormatError):
            doc_ex.open(pdf_fixture(), filetype="docx")
        with self.assertRaises(UnsupportedFormatError):
            doc_ex.open(b"not a document")
        with self.assertRaises(MalformedDocumentError):
            doc_ex.open(b"%PDF-broken")

    def test_malformed_xml_zip_relationships_and_media_fail_safely(self) -> None:
        docx_types = '<Types>wordprocessingml.document.main+xml</Types>'
        pptx_types = '<Types>presentationml.presentation.main+xml</Types>'
        cases = [
            package({"[Content_Types].xml": docx_types, "word/document.xml": "<w:document><"}),
            package({
                "[Content_Types].xml": pptx_types,
                "ppt/presentation.xml": '<p:presentation xmlns:p="p" xmlns:r="r"><p:sldId r:id="missing"/></p:presentation>',
                "ppt/_rels/presentation.xml.rels": "<Relationships/>",
            }),
            package({"[Content_Types].xml": docx_types}),
        ]
        for data in cases:
            with self.subTest(size=len(data)):
                with self.assertRaises(MalformedDocumentError):
                    doc_ex.open(data)

        malformed_media = package({
            "[Content_Types].xml": docx_types,
            "word/document.xml": '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body><w:p><w:r><w:drawing><wp:inline><wp:extent cx="9525" cy="9525"/><wp:docPr id="1" name="bad.png"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="bad.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="image"/></pic:blipFill><pic:spPr><a:xfrm><a:ext cx="9525" cy="9525"/></a:xfrm><a:prstGeom prst="rect"/></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p></w:body></w:document>',
            "word/_rels/document.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="image" Target="media/bad.png" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image"/></Relationships>',
            "word/media/bad.png": b"not decoded or executed",
        })
        with doc_ex.open(malformed_media) as document:
            self.assertEqual(document[0].get_images()[0].data, b"not decoded or executed")

    def test_archive_path_compression_and_member_count_safety(self) -> None:
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as archive:
            archive.writestr("../escape", b"x")
            archive.writestr("[Content_Types].xml", "wordprocessingml.document.main+xml")
        with self.assertRaises(UnsafeArchiveError):
            doc_ex.open(output.getvalue())

        output = io.BytesIO()
        with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("[Content_Types].xml", "wordprocessingml.document.main+xml")
            archive.writestr("bomb", b"0" * 2_000_000)
        with self.assertRaises(UnsafeArchiveError):
            doc_ex.open(output.getvalue())

        output = io.BytesIO()
        with zipfile.ZipFile(output, "w", zipfile.ZIP_STORED) as archive:
            archive.writestr("[Content_Types].xml", "wordprocessingml.document.main+xml")
            for index in range(10_000):
                archive.writestr(f"empty/{index}", b"")
        with self.assertRaises(UnsafeArchiveError):
            doc_ex.open(output.getvalue())


if __name__ == "__main__":
    unittest.main()
