from __future__ import annotations

import hashlib
import json
from pathlib import PurePosixPath
import unittest
from unittest.mock import patch

import doc_ex
from doc_ex import Block, Cell, Document, Image, Page, Story, Table
from fixtures import docx_fixture, pdf_fixture, pptx_fixture
from test_docx_stories import stories_fixture


def document(pages, *, stories=None):
    return Document(pages, {}, b"", "docx", None, stories=stories)


class MarkdownAssetsTests(unittest.TestCase):
    def test_content_dedup_keeps_recursive_and_repeated_placements(self):
        first = Image("../escape.png", b"pixels", "image/png", "one", page=0)
        second = Image("same.png", b"pixels", "image/png", "two", page=0)
        nested = Table([["inside"]], cells=[[Cell([Block("paragraph", "inside", images=[second])])]])
        table = Table([["inside"]], cells=[[Cell([Block("table", table=nested)])]])
        page = Page(0, [Block("paragraph", "before", images=[first]), Block("table", table=table)], images=[first, second])
        # The two fallback-only occurrences have identical metadata but remain two placements.
        later = Page(1, images=[first, first])
        with document([page, later]) as source:
            legacy = source.to_markdown()
            exported = source.export_markdown()
            self.assertEqual(source.to_markdown(), legacy)
        manifest = json.loads(exported["manifest.json"])
        self.assertEqual(len(manifest["assets"]), 1)
        self.assertEqual(len(manifest["placements"]), 4)
        asset = manifest["assets"][0]["path"]
        self.assertEqual(exported[asset], b"pixels")
        markdown = exported["document.md"].decode()
        self.assertEqual(markdown.count(asset), 4)
        self.assertEqual(markdown.count("inside"), 1)
        self.assertEqual(markdown.count("<table>"), 2)
        self.assertIn('src="' + asset + '"', markdown)
        self.assertIn("cells", manifest["placements"][1]["path"])
        self.assertEqual(len({tuple(item["path"]) for item in manifest["placements"]}), 4)
        with self.assertRaises(doc_ex.DocumentClosedError):
            source.export_markdown()
        self.assertEqual(exported[asset], b"pixels")

    def test_crosspage_inventory_is_not_a_second_table_placement(self):
        shared = Image("same.png", b"pixels", "image/png", page=1, metadata={"source_path": [0, 0], "source_offset": 0})
        other = Image("same.png", b"pixels", "image/png", page=1, metadata={"source_path": [1], "source_offset": 0})
        table = Table([[""]], cells=[[Cell([Block("paragraph", images=[shared])])]])
        source = document([Page(0, [Block("table", table=table)]), Page(1, images=[shared, other])])
        manifest = json.loads(source.export_markdown()["manifest.json"])
        self.assertEqual(len(manifest["placements"]), 2)
        self.assertEqual(manifest["placements"][1]["path"], ["pages", 1, "images", 1])

    def test_extracted_formats_and_separate_stories_resolve_assets(self):
        for fixture in (pdf_fixture, docx_fixture, pptx_fixture, stories_fixture):
            with self.subTest(format=fixture.__name__), doc_ex.open(fixture()) as source:
                exported = source.export_markdown()
                self.assertEqual(exported, source.export_markdown())
                manifest = json.loads(exported["manifest.json"])
                self.assertTrue(manifest["assets"])
                for asset in manifest["assets"]:
                    self.assertEqual(hashlib.sha256(exported[asset["path"]]).hexdigest(), asset["sha256"])
                    self.assertEqual(len(exported[asset["path"]]), asset["size"])
                for placement in manifest["placements"]:
                    relative = ("../" if placement["file"].startswith("stories/") else "") + placement["asset"]
                    self.assertIn(relative, exported[placement["file"]].decode())
                if fixture is stories_fixture:
                    self.assertNotIn(b"Header only", exported["document.md"])
                    header = next(item for item in manifest["stories"] if item["kind"] == "header")
                    self.assertIn(b"Header only", exported[header["file"]])
                    self.assertIn(b"../media/", exported[header["file"]])

    def test_names_are_generated_and_alt_text_cannot_break_image_reference(self):
        image = Image("https://evil/../../x.svg", b"opaque", "../../svg", 'x](/escape)\n[alt\\')
        source = document([Page(0, [Block("paragraph", images=[image])])], stories=[Story("../../header", [Block("paragraph", images=[image])])])
        exported = source.export_markdown()
        for name in exported:
            self.assertFalse(PurePosixPath(name).is_absolute())
            self.assertNotIn("..", PurePosixPath(name).parts)
        media = next(name for name in exported if name.startswith("media/"))
        self.assertTrue(media.endswith(".bin"))
        self.assertIn(b"x\\](/escape) \\[alt\\\\", exported["document.md"])
        self.assertIn("stories/0001.md", exported)

    def test_missing_media_cycles_depth_and_package_budget_fail_explicitly(self):
        source = document([Page(0, images=[Image("missing.png", b"")])])
        with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "no data"):
            source.export_markdown()
        cyclic = Table([[""]])
        cyclic.cells = [[Cell([Block("table", table=cyclic)])]]
        with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "cyclic"):
            document([Page(0, [Block("table", table=cyclic)])]).export_markdown()
        block = Block("paragraph", "leaf")
        for _ in range(34):
            block = Block("table", table=Table([[""]], cells=[[Cell([block])]]))
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "nesting"):
            document([Page(0, [block])]).export_markdown()
        ragged = Table([[""] * 1000] + [[] for _ in range(1000)])
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "expanded table grid"):
            document([Page(0, [Block("table", table=ragged)])]).export_markdown()
        with patch("doc_ex._document._MAX_EXPORT_BYTES", 4096):
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "128 MiB"):
                document([Page(0, images=[Image("a.png", b"x" * 4000)])]).export_markdown()
        with patch("doc_ex._document._MAX_EXPORT_NODES", 2):
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "content nodes"):
                document([Page(0, [Block("paragraph"), Block("paragraph")])]).export_markdown()
