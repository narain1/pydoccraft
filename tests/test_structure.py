from __future__ import annotations

import io
import json
import unittest
import zipfile
from xml.etree import ElementTree

import doc_ex
from doc_ex import Block, Cell, Image, Link, Page, Table
from fixtures import docx_fixture, nested_table_and_break_before_docx_fixture, package


def structure_fixture(body: str) -> bytes:
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "word/document.xml": '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>' + body + '</w:body></w:document>',
        "word/_rels/document.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="link" Target="https://example.com/?x=1&amp;y=2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" TargetMode="External"/></Relationships>',
    })


def paragraph(text: str) -> str:
    return f"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"


class StructureTests(unittest.TestCase):
    def test_nested_docx_owned_hierarchy_and_existing_rows(self) -> None:
        with doc_ex.open(nested_table_and_break_before_docx_fixture()) as document:
            page = document[0]
            table = page.find_tables()[0]
            self.assertEqual(table.rows, [["Outer\nInner"]])
            # WordCraft adds the required trailing cell paragraph when the source
            # ends with a nested table. Retain that canonical empty block.
            self.assertEqual([block.kind for block in table.cells[0][0].blocks], ["paragraph", "table", "paragraph"])
            self.assertEqual(table.cells[0][0].blocks[2].text, "")
            inner = table.cells[0][0].blocks[1].table
            self.assertEqual(inner.cells[0][0].blocks[0].text, "Inner")
            self.assertEqual(page.get_text(), "Outer\nInner")
            markdown = page.get_text("markdown")
            self.assertEqual(markdown.count("<table>"), 2)
            for text in ["Outer", "Inner"]:
                self.assertEqual(markdown.count(text), 1)
            payload = json.loads(page.get_text("json"))
            self.assertEqual(payload["blocks"][0]["table"]["cells"][0][0]["blocks"][1]["table"]["rows"], [["Inner"]])
            with doc_ex.open(document.convert("pdf")) as converted:
                text = "\n".join(page.get_text() for page in converted)
                self.assertEqual(text.count("Outer"), 1)
                self.assertEqual(text.count("Inner"), 1)
        self.assertEqual(inner.rows, [["Inner"]])
        self.assertEqual(table.cells[0][0].blocks[0].text, "Outer")

    def test_nested_source_order_and_link_offsets(self) -> None:
        nested = '<w:tbl><w:tr><w:tc><w:p><w:r><w:t>λ </w:t></w:r><w:hyperlink r:id="link"><w:r><w:t>nested label</w:t></w:r></w:hyperlink></w:p></w:tc></w:tr></w:tbl>'
        body = '<w:tbl><w:tr><w:tc>' + paragraph("Before &lt;tag&gt;") + nested + paragraph("After") + '</w:tc></w:tr></w:tbl>'
        with doc_ex.open(structure_fixture(body)) as document:
            page = document[0]
            blocks = page.find_tables()[0].cells[0][0].blocks
            self.assertEqual([block.text for block in blocks], ["Before <tag>", "", "After"])
            link = blocks[1].table.cells[0][0].blocks[0].links[0]
            self.assertEqual((link.text, link.metadata["start"], link.metadata["end"]), ("nested label", 2, 14))
            markdown = page.get_text("markdown")
            self.assertEqual(markdown.count("nested label"), 1)
            self.assertNotIn("**Links:**", markdown)
            self.assertIn("Before &lt;tag&gt;", markdown)
            self.assertIn('href="https://example.com/?x=1&amp;y=2"', markdown)
            self.assertLess(markdown.index("Before"), markdown.index("nested label"))
            self.assertLess(markdown.index("nested label"), markdown.index("After"))

    def test_docx_merge_geometry_preserves_source_cells(self) -> None:
        body = '<w:tbl><w:tblGrid><w:gridCol w="1440"/><w:gridCol w="1440"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge w:val="restart"/></w:tcPr>' + paragraph("Merged") + '</w:tc></w:tr><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>'
        with doc_ex.open(structure_fixture(body)) as document:
            table = document[0].find_tables()[0]
            self.assertEqual((table.cells[0][0].row_span, table.cells[0][0].col_span), (2, 2))
            self.assertTrue(table.cells[1][0].covered)
            self.assertEqual(table.rows, [["Merged"], [""]])
            self.assertEqual(table.to_markdown().count("<td "), 1)
            self.assertIn('colspan="2" rowspan="2"', table.to_markdown())

    def test_legacy_table_payload_and_pipe_markdown_unchanged(self) -> None:
        table = Table([["Name", "Score"], ["A", "9"]])
        self.assertEqual(table.to_markdown(), "| Name | Score |\n| --- | --- |\n| A | 9 |")
        self.assertNotIn("cells", table.to_dict())
        self.assertEqual(doc_ex._table(table.to_dict()).cells, [])

    def test_rich_cell_image_is_not_repeated_in_page_appendix(self) -> None:
        image = Image("images/pixel.png", b"pixels", alt='a "quoted" image')
        link = Link("https://example.com", "Label", metadata={"start": 0, "end": 5})
        table = Table([["Label"]], cells=[[Cell([Block("paragraph", "Label", links=[link], images=[image])])]])
        page = Page(0, [Block("table", table=table)], images=[image], links=[link], tables=[table])
        markdown = page.get_text("markdown")
        self.assertEqual(markdown.count("images/pixel.png"), 1)
        self.assertEqual(markdown.count("Label"), 1)
        self.assertIn("a &quot;quoted&quot; image", markdown)

    def test_docx_image_placement_remains_in_cell(self) -> None:
        with zipfile.ZipFile(io.BytesIO(docx_fixture())) as archive:
            members = {name: archive.read(name) for name in archive.namelist()}
        root = ElementTree.fromstring(members["word/document.xml"])
        namespace = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
        for prefix, uri in [*namespace.items(), ("r", "http://schemas.openxmlformats.org/officeDocument/2006/relationships"), ("a", "http://schemas.openxmlformats.org/drawingml/2006/main"), ("wp", "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"), ("pic", "http://schemas.openxmlformats.org/drawingml/2006/picture")]:
            ElementTree.register_namespace(prefix, uri)
        drawing = root.find(".//w:drawing", namespace)
        parent = next(element for element in root.iter() if drawing in list(element))
        parent.remove(drawing)
        cell = root.find(".//w:tc", namespace)
        paragraph_node = ElementTree.SubElement(cell, "{" + namespace["w"] + "}p")
        run = ElementTree.SubElement(paragraph_node, "{" + namespace["w"] + "}r")
        run.append(drawing)
        members["word/document.xml"] = ElementTree.tostring(root)
        with doc_ex.open(package(members)) as document:
            table = document[0].find_tables()[0]
            images = [image for block in table.cells[0][0].blocks for image in block.images]
            self.assertEqual(len(images), 1)
            self.assertTrue(images[0].data.startswith(b"\x89PNG"))
            markdown = document[0].get_text("markdown")
            self.assertEqual(markdown.count(images[0].name), 1)

    def test_repeated_cell_labels_keep_distinct_links(self) -> None:
        first = Link("https://example.com/one", "Same", metadata={"start": 0, "end": 4})
        second = Link("https://example.com/two", "Same", metadata={"start": 5, "end": 9})
        block = Block("paragraph", "Same Same", links=[first, second])
        table = Table([[block.text]], cells=[[Cell([block])]])
        page = Page(0, [Block("table", table=table)], links=[first, second])
        markdown = page.get_text("markdown")
        self.assertEqual(markdown.count("Same"), 2)
        self.assertEqual(markdown.count(first.uri), 1)
        self.assertEqual(markdown.count(second.uri), 1)
        self.assertNotIn("**Links:**", markdown)

    def test_docx_merge_with_omitted_leading_grid_cell(self) -> None:
        body = '<w:tbl><w:tblGrid><w:gridCol w="1440"/><w:gridCol w="1440"/></w:tblGrid><w:tr><w:tc>' + paragraph("First") + '</w:tc><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr>' + paragraph("Merged") + '</w:tc></w:tr><w:tr><w:trPr><w:gridBefore w:val="1"/></w:trPr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr></w:tbl>'
        with doc_ex.open(structure_fixture(body)) as document:
            table = document[0].find_tables()[0]
            self.assertEqual(table.cells[0][1].row_span, 2)
            self.assertTrue(table.cells[1][0].covered)
            self.assertEqual(table.cells[1][0].metadata["grid_column"], 1)
            self.assertIn("<tr><td></td></tr>", table.to_markdown())

    def test_docx_nested_payload_depth_boundary(self) -> None:
        body = paragraph("Deep content")
        for _ in range(16):
            body = "<w:tbl><w:tr><w:tc>" + body + "</w:tc></w:tr></w:tbl>"
        with doc_ex.open(structure_fixture(body)) as document:
            self.assertEqual(document[0].get_text().count("Deep content"), 1)
        body = "<w:tbl><w:tr><w:tc>" + body + "</w:tc></w:tr></w:tbl>"
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "nesting exceeds 16"):
            doc_ex.open(structure_fixture(body))

    def test_multipage_table_link_document_fallback_not_duplicated(self) -> None:
        first = '<w:tr><w:trPr><w:trHeight w:val="12900" w:hRule="exact"/></w:trPr><w:tc>' + paragraph("First row") + '</w:tc></w:tr>'
        second = '<w:tr><w:tc><w:p><w:hyperlink r:id="link"><w:r><w:t>Later label</w:t></w:r></w:hyperlink></w:p></w:tc></w:tr>'
        # A 645pt first row in an explicit 648pt body leaves less than one line.
        section = '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:bottom="1440" w:left="1440" w:right="1440"/></w:sectPr>'
        with doc_ex.open(structure_fixture('<w:tbl>' + first + second + '</w:tbl>' + section)) as document:
            links = [link for page in document for link in page.get_links()]
            self.assertEqual(len(links), 1)
            self.assertGreater(links[0].page, 0)
            self.assertIn("Later label", document[links[0].page].get_text("markdown"))
            self.assertEqual(document.to_markdown().count("Later label"), 1)

    def test_crosspage_table_image_suppression_keeps_distinct_placement(self) -> None:
        table_image = Image("images/shared.png", b"pixels", page=1, metadata={"source_path": [0, 1, 0, 0], "segment": 0, "source_offset": 0})
        other_image = Image("images/shared.png", b"pixels", page=1, metadata={"source_path": [1], "segment": 0, "source_offset": 0})
        table = Table([[""]], cells=[[Cell([Block("paragraph", images=[table_image])])]])
        first = Page(0, [Block("table", table=table)], tables=[table])
        second = Page(1, images=[table_image, other_image])
        document = doc_ex.Document([first, second], {}, b"", "docx", None)
        self.assertEqual(document.to_markdown().count("images/shared.png"), 2)
        self.assertIn("images/shared.png", second.get_text("markdown"))


if __name__ == "__main__":
    unittest.main()
