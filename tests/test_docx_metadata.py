from io import BytesIO
import math
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import DocumentClosedError


def property_parts(data):
    with ZipFile(BytesIO(data)) as archive:
        core = ET.fromstring(archive.read("docProps/core.xml"))
        custom = (ET.fromstring(archive.read("docProps/custom.xml"))
                  if "docProps/custom.xml" in archive.namelist() else [])
    return ({element.tag.rsplit("}", 1)[-1]: element.text for element in core},
            {element.attrib["name"]: (element[0].tag.rsplit("}", 1)[-1], element[0].text or "")
             for element in custom})


class DocxMetadataTests(unittest.TestCase):
    def test_core_and_scalar_custom_properties_serialize_without_changing_content(self):
        values = {
            "title": "Report <&> Ω", "author": "Example Author", "subject": "Status",
            "keywords": "one;two", "description": "First line\nSecond line", "category": "Review",
            "last_modified_by": "Editor", "created": "2024-02-29T01:02:03Z",
            "modified": "2026-10-20T23:59:59Z", "revision": 7,
            "custom": {"Text": "  <&> Ω  ", "Empty": "", "Approved": False, "Count": -42,
                       "Ratio": 1.25, "Maximum": 2**63 - 1, "Minimum": -(2**63)},
        }
        with doc_ex.new("docx") as document:
            document.add_paragraph("Visible report body")
            original_render = document.render_page(0, dpi=72)
            document.update_metadata(**values)
            self.assertEqual(document.metadata, values)
            raw = document.to_bytes()
            self.assertEqual(document.to_bytes(), raw)
            core, custom = property_parts(raw)
            for public, native in (("author", "creator"), ("last_modified_by", "lastModifiedBy")):
                self.assertEqual(core[native], values[public])
            for name in ("title", "subject", "keywords", "description", "category", "created", "modified"):
                self.assertEqual(core[name], values[name])
            self.assertEqual(core["revision"], "7")
            self.assertEqual(custom["Text"], ("lpwstr", "  <&> Ω  "))
            self.assertEqual(custom["Empty"], ("lpwstr", ""))
            self.assertEqual(custom["Approved"], ("bool", "false"))
            self.assertEqual(custom["Count"], ("i8", "-42"))
            self.assertEqual(custom["Ratio"], ("r8", "1.25"))
            self.assertEqual(document.render_page(0, dpi=72), original_render)
            with doc_ex.open(raw) as reopened:
                for key, value in values.items():
                    if key != "custom":
                        self.assertEqual(reopened.metadata[key], value)
            for output in (raw, document.convert("pdf")):
                with doc_ex.open(output) as reopened:
                    self.assertEqual(reopened.page_count, 1)
                    self.assertEqual(reopened.to_markdown().count("Visible report body"), 1)
                    self.assertTrue(reopened.render_page(0, dpi=72).startswith(b"\x89PNG"))

    def test_snapshots_copy_case_updates_and_clear_have_independent_ownership(self):
        with doc_ex.new("docx") as document:
            self.assertEqual(document.metadata, {"custom": {}})
            document.update_metadata(title="Original", author="Author", revision=1,
                                     created="2000-02-29T00:00:00Z", custom={"Flag": False, "Empty": ""})
            snapshot = document.metadata
            snapshot["custom"]["Flag"] = "Detached"
            snapshot["title"] = "Detached"
            self.assertEqual(document.metadata["custom"]["Flag"], False)
            with document.copy() as copied:
                document.update_metadata(title=None, author="", revision=None, created=None,
                                         custom={"flag": True, "Empty": None})
                self.assertEqual(document.metadata, {"custom": {"Flag": True}})
                self.assertEqual(copied.metadata["title"], "Original")
                self.assertEqual(copied.metadata["custom"], {"Flag": False, "Empty": ""})
                document.update_metadata(custom=None)
                self.assertEqual(document.metadata, {"custom": {}})
                self.assertEqual(property_parts(document.to_bytes()), ({}, {}))
                with doc_ex.open(document.to_bytes()) as reopened:
                    for key in ("title", "author", "revision", "created"):
                        self.assertNotIn(key, reopened.metadata)
                document.close()
                with self.assertRaises(DocumentClosedError):
                    _ = document.metadata
                with self.assertRaises(DocumentClosedError):
                    document.update_metadata(title="Closed")
                self.assertEqual(copied.metadata["title"], "Original")

    def test_invalid_batches_leave_metadata_bytes_and_existing_ranges_unchanged(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Stable text")
            document.update_metadata(title="Kept", custom={"Kept": 1})
            # Successful metadata mutation invalidates ranges conservatively;
            # rejected batches must not advance the owner's revision again.
            selected = paragraph.find("Stable")[0]
            before = document.to_bytes()
            bad_updates = [
                {"title": "Changed", "language": "en"}, {"title": " leading"},
                {"title": "trailing "}, {"title": "bad\rline"}, {"title": "bad\x00value"},
                {"revision": True}, {"revision": -1}, {"revision": 2**32}, {"revision": 1.0},
                {"created": "2023-02-29T00:00:00Z"}, {"modified": "2024-01-01T24:00:00Z"},
                {"created": "2024-01-01"}, {"created": "2024-01-01T00:00:00+01:00"},
                {"custom": {"Key": 1, "KEY": 2}}, {"custom": {1: "not a string key"}},
                {"custom": {"Kept": None, "Nested": []}}, {"custom": []},
                {"custom": {"Number": 2**63}}, {"custom": {"Number": math.inf}},
                {"custom": {"Number": math.nan}}, {"custom": {"": "empty name"}},
                {"custom": {"bad\nname": "value"}}, {"custom": {"x" * 256: 1}},
                {"title": "x" * 65537}, {"custom": {"Large": "x" * 65537}},
                {"custom": {"Unicode": "Ω" * 32769}},
                {"custom": {f"Escaped{index}": '"' * 60000 for index in range(3)}},
                {"custom": {f"Unicode{index}": "Ω" * 32768 for index in range(4)}},
            ]
            for update in bad_updates:
                with self.subTest(update=str(update)[:100]):
                    with self.assertRaises((TypeError, ValueError, doc_ex.DocExError)):
                        document.update_metadata(**update)
                    self.assertEqual(document.to_bytes(), before)
                    self.assertEqual(selected.text, "Stable")

    def test_property_budgets_can_be_reused_after_clear(self):
        with doc_ex.new("docx") as document:
            value = "x" * 60000
            for key in ("title", "author", "subject", "description"):
                document.update_metadata(**{key: value})
            before = document.to_bytes()
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                document.update_metadata(keywords=value)
            self.assertEqual(document.to_bytes(), before)
            document.update_metadata(description=None, keywords=value)
            document.update_metadata(custom={f"Property{index}": index for index in range(1024)})
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                document.update_metadata(custom={"Extra": 1})
            document.update_metadata(custom={"Property0": None, "Extra": 1})
            self.assertEqual(len(document.metadata["custom"]), 1024)
            document.update_metadata(custom=None)
            document.update_metadata(custom={"Recovered": True})
            self.assertEqual(document.metadata["custom"], {"Recovered": True})


if __name__ == "__main__":
    unittest.main()
