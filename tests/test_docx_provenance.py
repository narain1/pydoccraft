from __future__ import annotations

import json
import math
import unittest
from xml.sax.saxutils import escape

import doc_ex
from fixtures import docx_fixture, package
from test_docx_stories import stories_fixture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"


def fixture(body: str, *, columns: int = 1) -> bytes:
    return package({
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>',
        "word/document.xml": f'<w:document xmlns:w="{W}"><w:body>{body}<w:sectPr><w:pgSz w:w="7200" w:h="6000"/><w:pgMar w:top="720" w:bottom="720" w:left="720" w:right="720"/><w:cols w:num="{columns}" w:space="240"/></w:sectPr></w:body></w:document>',
    })


def paragraphs(blocks):
    for block in blocks:
        if block["table"]:
            for row in block["table"]["cells"]:
                for cell in row:
                    yield from paragraphs(cell["blocks"])
        else:
            yield block


class DocxProvenanceTests(unittest.TestCase):
    def test_native_utf8_ranges_explicit_segments_and_automatic_pages_preserve_text(self):
        long = "Automatic Ω🙂 wrapped words " * 160
        raw = "λ🙂 Before\ufffc\fAfter\x0eTail".encode()
        body = '<w:p><w:r><w:t>λ🙂 Before</w:t></w:r><w:fldSimple w:instr="PAGE"><w:r><w:t>CACHED VALUE</w:t></w:r></w:fldSimple><w:r><w:br w:type="page"/><w:t>After</w:t><w:br w:type="column"/><w:t>Tail</w:t></w:r></w:p>'
        body += f'<w:p><w:r><w:t>{escape(long)}</w:t></w:r></w:p>'
        data = fixture(body, columns=2)
        with doc_ex.open(data) as document:
            values = [page.get_text("dict") for page in document]
            blocks = [block for page in values for block in page["blocks"]]
            segments = [b for b in blocks if b["metadata"].get("source_path") == [0]]
            self.assertEqual([b["text"] for b in segments], ["λ🙂 BeforeCACHED VALUE", "After", "Tail"])
            self.assertEqual([b["metadata"]["segment"] for b in segments], [0, 1, 2])
            boundaries = [(0, raw.index(b"\f")), (raw.index(b"\f") + 1, raw.index(b"\x0e")), (raw.index(b"\x0e") + 1, len(raw))]
            for block, (lo, hi) in zip(segments, boundaries):
                layout = block["metadata"]["layout"]
                self.assertEqual(layout["offset_space"], "paragraph-utf8")
                self.assertEqual(layout["geometry"], "native-selection")
                self.assertIsNone(block["bbox"])
                for line in layout["lines"]:
                    start, stop = line["source_range"]
                    self.assertTrue(lo <= start <= stop <= hi)
                    raw[start:stop].decode()
            automatic = next(b for b in blocks if b["metadata"].get("source_path") == [1])
            self.assertEqual(automatic["text"], long)
            pages = {line["page"] for line in automatic["metadata"]["layout"]["lines"]}
            self.assertGreater(len(pages), 1)
            self.assertEqual(document.to_markdown().count("Automatic"), 160)
            self.assertEqual(json.loads(document[0].get_text("json")), json.loads(json.dumps(values[0])))
            self.assertEqual(values, [page.get_text("dict") for page in document])
        with doc_ex.open(data) as again:
            self.assertEqual(values, [page.get_text("dict") for page in again])

    def test_source_paths_assets_nested_tables_auxiliary_omission_and_owned_snapshots(self):
        with doc_ex.open(docx_fixture()) as document:
            pages = [page.get_text("dict") for page in document]
            assets = []
            tables = []
            for page in pages:
                for block in page["blocks"]:
                    if block["table"]:
                        self.assertEqual(block["metadata"]["source_path"], block["table"]["metadata"]["source_path"])
                        tables.append(block["table"])
                    if block["kind"] == "comment":
                        self.assertNotIn("source_path", block["metadata"])
                    for asset in [*block["links"], *block["images"]]:
                        assets.append(asset)
                        self.assertEqual(asset["metadata"]["source_path"], block["metadata"]["source_path"])
                        self.assertEqual(asset["metadata"]["segment"], block["metadata"]["segment"])
                for asset in [*page["links"], *page["images"]]:
                    self.assertIn("source_path", asset["metadata"])
                    self.assertIn("segment", asset["metadata"])
            self.assertTrue(assets)
            self.assertTrue(tables)
        self.assertTrue(assets[0]["metadata"]["source_path"])
        with doc_ex.open(stories_fixture()) as document:
            stories = document.stories
            body = document[0].get_text("dict")
            for story in stories:
                for block in paragraphs(story.to_dict()["blocks"]):
                    self.assertIn("source_path", block["metadata"])
                    self.assertIn("segment", block["metadata"])
                    self.assertNotIn("layout", block["metadata"])
            self.assertTrue(any("layout" in block["metadata"] for block in body["blocks"]))
            self.assertIsNone(next(s for s in stories if s.kind == "header").blocks[0].bbox)
            before = document.render_page(0, dpi=72)
            body["blocks"][0]["metadata"]["source_path"].append(999)
            self.assertEqual(document.render_page(0, dpi=72), before)
        self.assertTrue(stories[0].get_text())

    def test_native_selection_locations_match_png_and_reopened_pdf(self):
        text = "Visible source text"
        data = fixture(f'<w:p><w:r><w:t>{text}</w:t></w:r></w:p>')
        with doc_ex.open(data) as document:
            block = document[0].get_text("dict")["blocks"][0]
            metadata = document[0].get_text("dict")["metadata"]
            self.assertEqual((metadata["width"], metadata["height"]), (360, 300))
            self.assertEqual(metadata["body_bbox"], [36, 36, 324, 264])
            self.assertEqual((metadata["section"], metadata["display_number"]), (0, 1))
            lines = block["metadata"]["layout"]["lines"]
            self.assertEqual(lines[0]["source_range"], [0, len(text.encode())])
            for dpi in (72, 144):
                width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=dpi)))
                scale = dpi / 72
                for line in lines:
                    for x0, y0, x1, y1 in line["rects"]:
                        dark = sum(all(c < 100 for c in pixels[(y * width + x) * 4:(y * width + x) * 4 + 3])
                                   for y in range(math.floor(y0 * scale), math.ceil(y1 * scale))
                                   for x in range(math.floor(x0 * scale), math.ceil(x1 * scale)))
                        self.assertGreater(dark, 10)
            with doc_ex.open(document.convert("pdf")) as pdf:
                boxes = pdf[0].search_for(text)
                self.assertTrue(boxes)
                for box in boxes:
                    self.assertTrue(any(box[0] < r[2] and box[2] > r[0] and box[1] < r[3] and box[3] > r[1]
                                        for line in lines for r in line["rects"]))
        self.assertEqual(block["text"], text)

    def test_repeated_table_header_and_turned_cells_keep_paths_and_occurrences(self):
        rows = []
        for row in range(9):
            header = '<w:tblHeader/>' if row == 0 else ''
            cells = []
            for col in range(2):
                direction = '<w:textDirection w:val="tbRl"/>' if row == 1 and col == 0 else ''
                empty = '<w:p/>' if row == 0 and col == 0 else ''
                cells.append(f'<w:tc><w:tcPr><w:tcW w:w="2880" w:type="dxa"/>{direction}</w:tcPr><w:p><w:r><w:t>R{row}C{col}</w:t></w:r></w:p>{empty}</w:tc>')
            rows.append(f'<w:tr><w:trPr>{header}<w:trHeight w:val="1300" w:hRule="atLeast"/><w:cantSplit/></w:trPr>{"".join(cells)}</w:tr>')
        data = fixture('<w:tbl><w:tblPr><w:tblW w:w="5760" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="2880"/><w:gridCol w:w="2880"/></w:tblGrid>' + ''.join(rows) + '</w:tbl><w:p/>')
        with doc_ex.open(data) as document:
            table = document[0].find_tables()[0]
            self.assertEqual(table.metadata["source_path"], [0])
            first = table.cells[0][0].blocks[0]
            self.assertEqual(first.metadata["source_path"], [0, 0, 0, 0])
            self.assertGreater(len({line["page"] for line in first.metadata["layout"]["lines"]}), 1)
            empty = table.cells[0][0].blocks[1]
            self.assertEqual(empty.text, "")
            self.assertTrue(empty.metadata["layout"]["lines"])
            for line in empty.metadata["layout"]["lines"]:
                self.assertEqual(line["source_range"], [0, 0])
                self.assertEqual(line["rects"], [])
            self.assertFalse(any(block.metadata.get("source_path") == [1]
                                 for page in document for block in page.blocks))
            turned = table.cells[1][0].blocks[0]
            rect = turned.metadata["layout"]["lines"][0]["rects"][0]
            self.assertGreater(rect[3] - rect[1], rect[2] - rect[0])
            self.assertIsNone(table.bbox)
            self.assertEqual(document.to_markdown().count("R0C0"), 1)


if __name__ == "__main__":
    unittest.main()
