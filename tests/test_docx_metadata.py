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
            "keywords": "one;two", "language": "en-GB", "description": "First line\nSecond line", "category": "Review",
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
            for name in ("title", "subject", "keywords", "language", "description", "category", "created", "modified"):
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
                {"title": "Changed", "unknown_property": "en"},
                {"title": "Changed", "language": 42}, {"language": " en"},
                {"language": "en "}, {"language": "bad\x00value"},
                {"language": "Ω" * 32769}, {"title": " leading"},
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

    def test_core_language_is_metadata_only_and_preserves_source_and_visible_output(self):
        from test_authoring import image_fixture
        from test_render_geometry import rgba_pixels
        from test_rendering import png_chunks

        with doc_ex.new("docx", size=(240, 220)) as document:
            document.sections[0].margins = 24
            paragraph = document.add_paragraph()
            paragraph.add_run("Visible language label", language="fr-FR")
            paragraph[:].link = "https://example.test/core-language"
            document.add_table([["Retained table"]])
            document.add_image(image_fixture(4, 2), width=20, height=10)
            before = document.to_bytes()
            with ZipFile(BytesIO(before)) as archive:
                original_parts = {name: archive.read(name) for name in archive.namelist()}
            dpis = (72, 144, 300)
            source_pixels = {dpi: document.render_page(0, dpi=dpi) for dpi in dpis}
            with doc_ex.open(document.convert("pdf")) as pdf:
                pdf_pixels = {dpi: pdf.render_page(0, dpi=dpi) for dpi in dpis}
                pdf_text = pdf[0].get_text()
                pdf_links = [(link.text, link.uri) for link in pdf[0].get_links()]
            self.assertIn("Visible language label", pdf_text)
            self.assertEqual([uri for _, uri in pdf_links], ["https://example.test/core-language"])
            for png in source_pixels.values():
                _, rgba = rgba_pixels(png_chunks(png))
                self.assertGreater(sum(rgba[i:i + 3] != b"\xff\xff\xff"
                                       for i in range(0, len(rgba), 4)), 50)
            selected = paragraph[:]
            document.update_metadata(language="en-GB")
            self.assertEqual(document.metadata["language"], "en-GB")
            with self.assertRaises(ValueError):
                _ = selected.text
            with document.copy() as copied:
                snapshot = document.metadata
                snapshot["language"] = "Detached"
                document.update_metadata(title="Unrelated")
                self.assertEqual(document.metadata["language"], "en-GB")
                document.update_metadata(title=None, language="de-DE<&>Ω")
                self.assertEqual(copied.metadata["language"], "en-GB")
            data = document.to_bytes()
            with ZipFile(BytesIO(data)) as archive:
                current_parts = {name: archive.read(name) for name in archive.namelist()}
            self.assertEqual(current_parts.keys(), original_parts.keys())
            for name in original_parts:
                if name != "docProps/core.xml":
                    self.assertEqual(current_parts[name], original_parts[name], name)
            self.assertEqual(property_parts(data)[0]["language"], "de-DE<&>Ω")
            with doc_ex.open(data) as reopened:
                self.assertEqual(reopened.metadata["language"], "de-DE<&>Ω")
                self.assertEqual(reopened.to_markdown().count("Retained table"), 1)
                for dpi in dpis:
                    self.assertEqual(reopened.render_page(0, dpi=dpi), source_pixels[dpi])
                    self.assertEqual(document.render_page(0, dpi=dpi), source_pixels[dpi])
            with doc_ex.open(document.convert("pdf")) as reopened:
                self.assertEqual(reopened[0].get_text(), pdf_text)
                self.assertEqual([(link.text, link.uri) for link in reopened[0].get_links()], pdf_links)
                for dpi in dpis:
                    self.assertEqual(reopened.render_page(0, dpi=dpi), pdf_pixels[dpi])
            for clear in (None, ""):
                document.update_metadata(language="en-GB")
                document.update_metadata(language=clear)
                self.assertNotIn("language", document.metadata)
                self.assertNotIn("language", property_parts(document.to_bytes())[0])
                self.assertEqual(document.to_bytes(), before)
            document.close()
            with self.assertRaises(DocumentClosedError):
                document.update_metadata(language="en-GB")

    def test_language_utf8_and_aggregate_bounds_are_atomic_and_reusable(self):
        # Varied ASCII avoids making this metadata boundary an archive-ratio test.
        ascii_value = "".join(f"{index:04x}" for index in range(16384))
        exact = "Ω" + ascii_value[:65534]
        self.assertEqual(len(exact.encode("utf-8")), 65536)
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Retained boundary body")
            document.update_metadata(language=exact)
            self.assertEqual(document.metadata["language"], exact)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened.metadata["language"], exact)
            selected = paragraph[:]
            before = document.to_bytes()
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                document.update_metadata(title="Must not commit", language=exact + "x")
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(selected.text, "Retained boundary body")
            document.update_metadata(language=None)
            value = ascii_value[:60000]
            document.update_metadata(title=value, author=value, subject=value, description=value)
            selected = paragraph[:]
            before = document.to_bytes()
            with self.assertRaises((ValueError, doc_ex.DocExError)):
                document.update_metadata(language=value)
            self.assertEqual(document.to_bytes(), before)
            self.assertEqual(selected.text, "Retained boundary body")
            document.update_metadata(description=None, language=value)
            self.assertEqual(document.metadata["language"], value)
            self.assertNotIn("description", document.metadata)
            with doc_ex.open(document.to_bytes()) as reopened:
                self.assertEqual(reopened.metadata["language"], value)

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
