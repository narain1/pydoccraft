from __future__ import annotations

import io
import unittest
import zipfile
from xml.etree import ElementTree as ET

import doc_ex
from fixtures import docx_fixture, package, pptx_fixture
from test_docx_stories import stories_fixture
from test_structure import structure_fixture

M = "http://schemas.openxmlformats.org/officeDocument/2006/math"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"


def docx_projections() -> str:
    # Ruby belongs inside a run; OMML and a simple field are paragraph children.
    return '<w:p><m:oMath xmlns:m="' + M + '"><m:r><m:t>x+y</m:t></m:r></m:oMath><w:r><w:ruby><w:rubyPr><w:hps w:val="12"/><w:hpsBaseText w:val="20"/><w:hpsRaise w:val="20"/></w:rubyPr><w:rt><w:r><w:t>phonetic</w:t></w:r></w:rt><w:rubyBase><w:r><w:t>Base</w:t></w:r></w:rubyBase></w:ruby></w:r><w:fldSimple w:instr="DATE" w:dirty="true"><w:r><w:t>Cached date</w:t></w:r></w:fldSimple></w:p>'


def pptx_projections() -> bytes:
    with zipfile.ZipFile(io.BytesIO(pptx_fixture())) as archive:
        members = {name: archive.read(name) for name in archive.namelist()}
    root = ET.fromstring(members["ppt/slides/slide1.xml"])
    shape = root.find(f".//{{{P}}}sp")
    shape.find(f"{{{P}}}spPr/{{{A}}}xfrm").set("rot", "5400000")
    paragraph = shape.find(f"{{{P}}}txBody/{{{A}}}p")
    field = ET.SubElement(paragraph, f"{{{A}}}fld", {"id": "{AABBCCDD-0000-0000-0000-000000000001}", "type": "slidenum"})
    ET.SubElement(field, f"{{{A}}}t").text = "7"
    hyperlink = shape.find(f".//{{{A}}}hlinkClick")
    hyperlink.attrib.pop(f"{{{R}}}id", None)
    hyperlink.set("action", "ppaction://hlinkshowjump?jump=nextslide")
    members["ppt/slides/slide1.xml"] = ET.tostring(root)
    return package(members)


class DiagnosticTests(unittest.TestCase):
    def test_plain_documents_do_not_get_speculative_warnings(self) -> None:
        for data in (docx_fixture(), pptx_fixture()):
            with doc_ex.open(data) as document:
                self.assertEqual(document.metadata["diagnostics"], [])

    def test_docx_equation_ruby_and_dirty_field_report_exact_projections(self) -> None:
        with doc_ex.open(structure_fixture(docx_projections())) as document:
            diagnostics = {item["code"]: item for item in document.metadata["diagnostics"]}
            self.assertEqual(set(diagnostics), {"equation_linearized", "ruby_annotation_omitted", "dirty_field_cached_result"})
            self.assertIn("x+y", document[0].get_text())
            self.assertIn("Base", document[0].get_text())
            self.assertIn("Cached date", document[0].get_text())
            self.assertNotIn("phonetic", document[0].get_text())
            for item in diagnostics.values():
                self.assertEqual(item["operation"], "extract")
                self.assertEqual(item["count"], 1)
                self.assertEqual(item["sources"][0]["story"], "body")
                self.assertEqual(item["sources"][0]["source_path"], [0])
                self.assertNotIn("xml", item)

    def test_auxiliary_story_locations_and_global_source_budget(self) -> None:
        math = '<w:p><m:oMath xmlns:m="' + M + '"><m:r><m:t>x</m:t></m:r></m:oMath></w:p>'
        with doc_ex.open(stories_fixture(math * 125)) as document:
            diagnostics = document.metadata["diagnostics"]
            self.assertEqual(len(diagnostics), 1)
            item = diagnostics[0]
            self.assertEqual((item["code"], item["count"]), ("equation_linearized", 125))
            self.assertEqual(len(item["sources"]), 100)
            header = next(story for story in document.stories if story.kind == "header")
            self.assertTrue(all(source["story_part_id"] == header.metadata["part_id"] for source in item["sources"]))
            self.assertEqual(item["sources"][0]["source_path"], [0])
            self.assertEqual(item["sources"][-1]["source_path"], [99])

    def test_pptx_fields_non_url_links_and_transformed_geometry_are_detected(self) -> None:
        with doc_ex.open(pptx_projections()) as document:
            diagnostics = {item["code"]: item for item in document.metadata["diagnostics"]}
            self.assertEqual(set(diagnostics), {"field_cached_result", "non_url_link_omitted", "transformed_bbox_approximation"})
            for item in diagnostics.values():
                self.assertEqual(item["count"], 1)
                self.assertEqual(item["sources"][0]["slide"], 0)
                self.assertEqual(item["sources"][0]["shape_id"], 2)
            self.assertIn("7", document[0].get_text())
            self.assertNotIn("https://example.com/one", [link.uri for link in document[0].get_links()])


if __name__ == "__main__":
    unittest.main()
