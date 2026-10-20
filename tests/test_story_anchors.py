from __future__ import annotations

import io
import unittest
import zipfile

import doc_ex
from fixtures import package
from test_docx_stories import stories_fixture
from test_structure import structure_fixture


class StoryAnchorTests(unittest.TestCase):
    def test_text_box_only_paragraph_keeps_story_connection(self) -> None:
        body = '<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml"><v:shape id="TextBox" style="width:120pt;height:40pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>Box content</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>'
        with doc_ex.open(structure_fixture(body)) as document:
            box = next(story for story in document.stories if story.kind == "text_box")
            self.assertEqual(box.get_text(), "Box content")
            block = document[0].blocks[0]
            self.assertEqual(block.text, "")
            self.assertEqual(block.metadata["story_refs"], [{"part_id": box.metadata["part_id"], "kind": "text_box", "offset": 0}])
            self.assertNotIn("Box content", document.to_markdown())

    def test_reference_only_paragraph_keeps_note_connections(self) -> None:
        with zipfile.ZipFile(io.BytesIO(stories_fixture())) as archive:
            members = {name: archive.read(name) for name in archive.namelist()}
        members["word/document.xml"] = members["word/document.xml"].replace(b"<w:t>Body \xce\xbb</w:t>", b"")
        with doc_ex.open(package(members)) as document:
            block = document[0].blocks[0]
            self.assertEqual(block.text, "")
            refs = block.metadata["story_refs"]
            by_id = {story.metadata["part_id"]: story.kind for story in document.stories if "part_id" in story.metadata}
            self.assertEqual([(by_id[ref["part_id"]], ref["offset"]) for ref in refs], [("footnote", 0), ("endnote", 0)])
            self.assertNotIn("Footnote only", document.to_markdown())


if __name__ == "__main__":
    unittest.main()
