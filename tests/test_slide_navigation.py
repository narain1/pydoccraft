import copy
import io
import unittest
import xml.etree.ElementTree as ET
import zipfile

import doc_ex

P = "{http://schemas.openxmlformats.org/presentationml/2006/main}"
A = "{http://schemas.openxmlformats.org/drawingml/2006/main}"
R = "{http://schemas.openxmlformats.org/officeDocument/2006/relationships}"


def rewrite_slide(data, update):
    result = io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as source, zipfile.ZipFile(result, "w") as destination:
        for part in source.infolist():
            value = source.read(part.filename)
            if part.filename == "ppt/slides/slide1.xml":
                root = ET.fromstring(value)
                update(root)
                value = ET.tostring(root, encoding="utf-8", xml_declaration=True)
            destination.writestr(part, value)
    return result.getvalue()


def navigation_fixture():
    with doc_ex.new("pptx", size=(400, 300)) as document:
        source = document.slides.add()
        target = document.slides.add()
        target.hidden = True
        shape = source.add_text("Fixture label", box=(10, 20, 200, 30))
        shape[:].link = target
        shape.link = target
        return document.to_bytes()


class SlideNavigationTests(unittest.TestCase):
    def test_reordered_hidden_targets_unicode_offsets_urls_and_self_links(self):
        with doc_ex.new("pptx", size=(600, 400)) as document:
            source, hidden, other = (document.slides.add() for _ in range(3))
            hidden.hidden = True
            shape = source.add_text("λ Same / Same 😀", box=(20, 20, 500, 50))
            shape[2:6].link = hidden
            shape[9:13].link = "https://example.com/unchanged"
            shape[14:15].link = source
            shape.link = other
            multiline = source.add_text("A😀e\u0301\nSecond tail", box=(20, 100, 500, 100))
            multiline[1:10].link = hidden
            table = source.add_table([["Cell text"]], box=(20, 240, 300, 40))
            table.link = hidden
            document.slides.move(hidden.number, 0)
            document.slides.move(other.number, 1)
            self.assertEqual((source.number, hidden.number, other.number), (2, 0, 1))
            data = document.to_bytes()
        with doc_ex.open(data) as reopened:
            self.assertEqual(len(reopened), 3)
            self.assertTrue(reopened[0].metadata["hidden"])
            page = reopened[2]
            links = page.get_links()
            self.assertEqual([(link.text, link.uri) for link in links[:4]],
                             [("Same", "#page=1"), ("Same", "https://example.com/unchanged"),
                              ("😀", "#page=3"), ("λ Same / Same 😀", "#page=2")])
            self.assertEqual([(link.metadata["start"], link.metadata["end"]) for link in links[:3]], [(2, 6), (9, 13), (14, 15)])
            for link in links:
                self.assertEqual(link.page, 2)
                if link.uri.startswith("#page="):
                    self.assertEqual(link.metadata["target_page"], int(link.uri[6:]) - 1)
                else:
                    self.assertNotIn("target_page", link.metadata)
            multiline_links = []
            for block in page.blocks:
                for link in block.links:
                    if "start" in link.metadata:
                        self.assertEqual(block.text[link.metadata["start"]:link.metadata["end"]], link.text)
                        if block.text in ("A😀e\u0301", "Second tail"):
                            multiline_links.append(link)
            self.assertEqual("".join(link.text for link in multiline_links), "😀e\u0301Secon")
            self.assertTrue(all(link.uri == "#page=1" for link in multiline_links))
            text = page.get_text()
            for label in ("λ Same / Same 😀", "A😀e\u0301", "Second tail", "Cell text"):
                self.assertEqual(text.count(label), 1)
            self.assertNotIn("non_url_link_omitted", [item["code"] for item in reopened.metadata["diagnostics"]])
            owned = links[0]
        self.assertEqual(owned.metadata["target_page"], 0)

    def test_actionless_internal_relationships_and_group_click_source_order(self):
        data = navigation_fixture()

        def group_shape(root):
            tree = root.find(f"{P}cSld/{P}spTree")
            shape = tree.find(f"{P}sp")
            tree.remove(shape)
            group = ET.SubElement(tree, f"{P}grpSp")
            nonvisual = ET.SubElement(group, f"{P}nvGrpSpPr")
            common = ET.SubElement(nonvisual, f"{P}cNvPr", {"id": "999", "name": "Linked group"})
            common.append(copy.deepcopy(shape.find(f"{P}nvSpPr/{P}cNvPr/{A}hlinkClick")))
            ET.SubElement(nonvisual, f"{P}cNvGrpSpPr")
            ET.SubElement(nonvisual, f"{P}nvPr")
            properties = ET.SubElement(group, f"{P}grpSpPr")
            transform = ET.SubElement(properties, f"{A}xfrm")
            for name, attributes in (("off", {"x": "127000", "y": "254000"}),
                                     ("ext", {"cx": "5080000", "cy": "3810000"}),
                                     ("chOff", {"x": "0", "y": "0"}),
                                     ("chExt", {"cx": "5080000", "cy": "3810000"})):
                ET.SubElement(transform, f"{A}{name}", attributes)
            group.append(shape)
            for link in root.iter(f"{A}hlinkClick"):
                link.attrib.pop("action", None)

        with doc_ex.open(rewrite_slide(data, group_shape)) as document:
            page = document[0]
            links = page.get_links()
            self.assertEqual([(link.text, link.uri) for link in links],
                             [("", "#page=2"), ("Fixture label", "#page=2"), ("Fixture label", "#page=2")])
            self.assertTrue(all(link.metadata["target_page"] == 1 for link in links))
            self.assertEqual(links[1].bbox, (20, 40, 220, 70))
            self.assertEqual(page.get_text().count("Fixture label"), 1)
            self.assertEqual(document.metadata["diagnostics"], [])

    def test_native_large_shape_id_remapping_keeps_distinct_slide_targets(self):
        def large_id(root):
            root.find(f"{P}cSld/{P}spTree/{P}sp/{P}nvSpPr/{P}cNvPr").set("id", str(2**32 - 1))
        data = rewrite_slide(navigation_fixture(), large_id)
        with doc_ex.open(data) as document:
            self.assertEqual(len(document), 2)
            self.assertEqual(len(document[0].get_links()), 2)
            self.assertTrue(all(link.uri == "#page=2" and link.metadata["target_page"] == 1
                                for link in document[0].get_links()))

    def test_contextual_actions_are_diagnosed_and_native_dropped_actions_are_documented(self):
        data = navigation_fixture()
        cases = (("ppaction://hlinkshowjump?jump=nextslide", False, 2),
                 ("ppaction://hlinkshowjump?jump=lastslideviewed", False, 2),
                 ("ppaction://unknown-action", False, 0),
                 ("ppaction://hlinksldjump", True, 0))
        for action, missing_relationship, count in cases:
            def change(root):
                for link in root.iter(f"{A}hlinkClick"):
                    link.set("action", action)
                    if missing_relationship:
                        link.set(f"{R}id", "missingRelationship")
            with self.subTest(action=action, missing_relationship=missing_relationship), doc_ex.open(rewrite_slide(data, change)) as document:
                self.assertEqual(document[0].get_links(), [])
                self.assertEqual(document[0].get_text().count("Fixture label"), 1)
                diagnostics = document.metadata["diagnostics"]
                if count:
                    self.assertEqual(diagnostics[0]["code"], "non_url_link_omitted")
                    self.assertEqual(diagnostics[0]["count"], count)
                else:
                    # Pinned native reader drops these actions before model extraction;
                    # absence of per-link diagnostics is an explicit upstream limitation.
                    self.assertEqual(diagnostics, [])


if __name__ == "__main__":
    unittest.main()
