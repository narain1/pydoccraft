from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from zipfile import ZipFile, ZIP_STORED

import doc_ex


W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def rewrite(source, edit):
    with ZipFile(BytesIO(source)) as archive:
        parts = {name: archive.read(name) for name in archive.namelist()}
    edit(parts)
    output = BytesIO()
    with ZipFile(output, "w", compression=ZIP_STORED) as archive:
        for name, data in parts.items():
            archive.writestr(name, data)
    return output.getvalue()


def all_blocks(document):
    def walk(blocks):
        for block in blocks:
            if block.table is None:
                yield block
            else:
                for row in block.table.cells:
                    for cell in row:
                        yield from walk(cell.blocks)
    return [block for page in document for block in walk(page.blocks)]


class NumberingExtractionTests(unittest.TestCase):
    def test_native_multilevel_continuation_restart_empty_item_and_owned_metadata(self):
        with doc_ex.new("docx") as document:
            first = document.add_paragraph("Root 😀 label")
            first.set_list("numbered")
            first.find("😀 label")[0].link = "https://example.com/list"
            for level in range(1, 9):
                document.add_paragraph(f"Depth {level}").set_list("numbered", level=level)
            document.add_paragraph("Continue root").set_list("numbered")
            document.add_paragraph("Restart root").set_list("numbered", restart=True)
            document.add_paragraph("").set_list("numbered")
            source, pdf = document.to_bytes(), document.convert("pdf")
        with doc_ex.open(source) as reopened:
            blocks = all_blocks(reopened)
            numbered = [block for block in blocks if "numbering" in block.metadata]
            self.assertEqual(len(numbered), 12)
            values = [block.metadata["numbering"] for block in numbered]
            self.assertEqual([value["level"] for value in values], [*range(9), 0, 0, 0])
            self.assertEqual([value["label"] for value in values],
                             ["1.", "a.", "i.", "1.", "a.", "i.", "1.", "a.", "i.", "2.", "1.", "2."])
            self.assertEqual(len({value["num_id"] for value in values[:10]}), 1)
            self.assertNotEqual(values[0]["num_id"], values[10]["num_id"])
            self.assertEqual(values[10]["num_id"], values[11]["num_id"])
            self.assertEqual(numbered[-1].text, "")
            self.assertEqual(numbered[0].text, "Root 😀 label")
            link = numbered[0].links[0]
            self.assertEqual((link.text, link.metadata["start"], link.metadata["end"]), ("😀 label", 5, 12))
            self.assertEqual(reopened.to_markdown().count("Root"), 1)
            self.assertEqual(reopened.to_markdown().count("Continue root"), 1)
            self.assertNotIn("1. Root", reopened.to_markdown())
        # Owned source metadata remains valid after closing the source document.
        self.assertEqual(values[9]["label"], "2.")
        with doc_ex.open(pdf) as reopened:
            text = "\n".join(page.get_text() for page in reopened)
            self.assertIn("Continue root", text)
            self.assertIn("Restart root", text)
            self.assertGreaterEqual(text.count("2."), 2)

    def test_imported_inheritance_start_override_custom_template_and_nested_cells(self):
        with doc_ex.new("docx") as document:
            document.add_paragraph("First").set_list("numbered")
            document.add_paragraph("Inherited")
            document.add_paragraph("Removed")
            cell = document.add_table([["Cell"]])[0, 0]
            cell.add_table([["Nested"]])
            source = document.to_bytes()

        def edit(parts):
            main = ET.fromstring(parts["word/document.xml"])
            first_num = main.find(f".//{W}numId").get(f"{W}val")
            for paragraph in main.iter(f"{W}p"):
                text = "".join(node.text or "" for node in paragraph.iter(f"{W}t"))
                if text not in ("Inherited", "Removed", "Cell", "Nested"):
                    continue
                props = paragraph.find(f"{W}pPr")
                if props is None:
                    props = ET.Element(f"{W}pPr")
                    paragraph.insert(0, props)
                ET.SubElement(props, f"{W}pStyle", {f"{W}val": "ImportedList"})
                if text == "Removed":
                    num = ET.SubElement(props, f"{W}numPr")
                    ET.SubElement(num, f"{W}numId", {f"{W}val": "0"})
            styles = ET.fromstring(parts["word/styles.xml"])
            style = ET.SubElement(styles, f"{W}style", {f"{W}type": "paragraph", f"{W}styleId": "ImportedList"})
            ET.SubElement(style, f"{W}name", {f"{W}val": "Imported list"})
            num = ET.SubElement(ET.SubElement(style, f"{W}pPr"), f"{W}numPr")
            ET.SubElement(num, f"{W}ilvl", {f"{W}val": "0"})
            ET.SubElement(num, f"{W}numId", {f"{W}val": first_num})
            numbering = ET.fromstring(parts["word/numbering.xml"])
            level = numbering.find(f"{W}abstractNum/{W}lvl")
            level.find(f"{W}start").set(f"{W}val", "5")
            level.find(f"{W}lvlText").set(f"{W}val", "Item %1)")
            instance = next(value for value in numbering.findall(f"{W}num") if value.get(f"{W}numId") == first_num)
            override = ET.SubElement(instance, f"{W}lvlOverride", {f"{W}ilvl": "0"})
            ET.SubElement(override, f"{W}startOverride", {f"{W}val": "7"})
            for name, root in (("word/document.xml", main), ("word/styles.xml", styles), ("word/numbering.xml", numbering)):
                parts[name] = ET.tostring(root)

        with doc_ex.open(rewrite(source, edit)) as document:
            blocks = {block.text: block for block in all_blocks(document) if block.text}
            for text, label in zip(("First", "Inherited", "Cell", "Nested"), ("Item 7)", "Item 8)", "Item 9)", "Item 10)")):
                self.assertEqual(blocks[text].metadata["numbering"]["label"], label)
                self.assertEqual(blocks[text].metadata["numbering"]["template"], "Item %1)")
                self.assertEqual(document.to_markdown().count(text), 1)
            self.assertNotIn("numbering", blocks["Removed"].metadata)
            self.assertEqual(blocks["Inherited"].style, "ImportedList")
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = pdf.to_markdown()
                for label in ("Item 7)", "Item 8)", "Item 9)", "Item 10)"):
                    self.assertEqual(text.count(label), 1)

    def test_explicit_break_segments_and_automatic_continuation_do_not_duplicate_labels(self):
        with doc_ex.new("docx") as document:
            section = document.sections[0]
            section.page_size = (300, 240)
            section.margins = (20, 20, 20, 20)
            paragraph = document.add_paragraph("")
            paragraph.set_list("numbered")
            paragraph.add_break("page")
            paragraph.add_run("After page")
            paragraph.add_break("column")
            paragraph.add_run("After column")
            document.add_paragraph("Long text " * 300).set_list("numbered")
            source, pdf = document.to_bytes(), document.convert("pdf")
        with doc_ex.open(source) as document:
            self.assertGreater(len(document), 3)
            blocks = all_blocks(document)
            explicit = [block for block in blocks if block.text in ("", "After page", "After column")]
            self.assertEqual([block.text for block in explicit], ["", "After page", "After column"])
            self.assertNotIn("label", explicit[0].metadata["numbering"])
            self.assertEqual(explicit[1].metadata["numbering"]["label"], "1.")
            self.assertNotIn("label", explicit[2].metadata["numbering"])
            labels = [(page.number, block.metadata["numbering"]["label"]) for page in document for block in page.blocks
                      if "label" in block.metadata.get("numbering", {})]
            self.assertEqual([label for _, label in labels], ["1.", "2."])
            self.assertEqual(document.to_markdown().count("After page"), 1)
            self.assertEqual(document.to_markdown().count("After column"), 1)
        with doc_ex.open(pdf) as document:
            for page, label in labels:
                self.assertIn(label, document[page].get_text())

    def test_story_paths_do_not_collide_and_varying_repeated_header_has_no_guessed_marker(self):
        with doc_ex.new("docx") as document:
            paragraph = document.add_paragraph("Body item")
            paragraph.set_list("numbered")
            header = document.sections[0].header().add_paragraph("Header item")
            header.set_list("numbered")
            note = document.notes.add(paragraph.find("Body")[0], "Note item")
            note.paragraphs[0].set_list("numbered")
            document.add_page_break()
            document.add_paragraph("Second page")
            source = document.to_bytes()
        with doc_ex.open(source) as document:
            body = next(block for block in all_blocks(document) if block.text.startswith("Body"))
            stories = {story.kind: story for story in document.stories if story.kind in ("header", "footnote")}
            header = next(block for block in stories["header"].blocks if block.text == "Header item")
            note = next(block for block in stories["footnote"].blocks if "Note item" in block.text)
            values = [block.metadata["numbering"] for block in (body, header, note)]
            self.assertEqual(len({value["num_id"] for value in values}), 3)
            self.assertEqual(values[0]["label"], "1.")
            self.assertEqual(values[2]["label"], "1.")
            self.assertNotIn("label", values[1])
            self.assertEqual(header.metadata["source_path"], [0])
            self.assertEqual(note.metadata["source_path"], [0])
            self.assertNotEqual(header.metadata["story_part_id"], note.metadata["story_part_id"])
            with doc_ex.open(document.convert("pdf")) as pdf:
                self.assertEqual(sum(page.get_text().count("Header item") for page in pdf), 2)
                # The pinned native counter assigns different labels to this repeated story.
                # One owned header block must not claim either label as globally applicable.
                self.assertIn("2. Header item", pdf[0].get_text())
                self.assertIn("3. Header item", pdf[1].get_text())
                self.assertEqual(pdf.to_markdown().count("Note item"), 1)

    def test_missing_definition_and_native_imported_template_normalization_are_explicit(self):
        with doc_ex.new("docx") as document:
            document.add_paragraph("Missing instance").set_list("numbered")
            source = document.to_bytes()

        def missing(parts):
            root = ET.fromstring(parts["word/document.xml"])
            root.find(f".//{W}numId").set(f"{W}val", "999999")
            parts["word/document.xml"] = ET.tostring(root)

        with doc_ex.open(rewrite(source, missing)) as document:
            value = document[0].blocks[0].metadata["numbering"]
            self.assertEqual(value, {"num_id": 999999, "abstract_id": None, "level": 0,
                                     "format": None, "template": None})
            self.assertEqual(document[0].get_text(), "Missing instance")

        def oversized(parts):
            root = ET.fromstring(parts["word/numbering.xml"])
            root.find(f"{W}abstractNum/{W}lvl/{W}lvlText").set(f"{W}val", "x" * (65536 + 1))
            parts["word/numbering.xml"] = ET.tostring(root)

        # The pinned reader truncates lvlText to64 Unicode characters before exposing its
        # model. Extraction reports that parsed template, not the unavailable original XML.
        with doc_ex.open(rewrite(source, oversized)) as document:
            value = document[0].blocks[0].metadata["numbering"]
            self.assertEqual(value["template"], "x" * 64)
            self.assertEqual(value["label"], "x" * 64)

    def test_native_symbol_mapping_and_logical_rtl_label_are_kept_separate_from_text(self):
        with doc_ex.new("docx") as document:
            document.add_paragraph("Symbol bullet").set_list("bullet")
            document.add_paragraph("RTL marker").set_list("numbered")
            source = document.to_bytes()

        def edit(parts):
            numbering = ET.fromstring(parts["word/numbering.xml"])
            level = numbering.find(f"{W}abstractNum/{W}lvl")
            level.find(f"{W}lvlText").set(f"{W}val", "\uf0b7")
            props = level.find(f"{W}rPr")
            if props is None:
                props = ET.SubElement(level, f"{W}rPr")
            fonts = props.find(f"{W}rFonts")
            if fonts is None:
                fonts = ET.SubElement(props, f"{W}rFonts")
            for name in ("ascii", "hAnsi", "eastAsia", "cs"):
                fonts.set(f"{W}{name}", "Symbol")
            main = ET.fromstring(parts["word/document.xml"])
            paragraph = main.findall(f"{W}body/{W}p")[1]
            ET.SubElement(paragraph.find(f"{W}pPr"), f"{W}bidi", {f"{W}val": "1"})
            parts["word/numbering.xml"] = ET.tostring(numbering)
            parts["word/document.xml"] = ET.tostring(main)

        with doc_ex.open(rewrite(source, edit)) as document:
            bullet, rtl = all_blocks(document)
            self.assertEqual(bullet.metadata["numbering"]["template"], "\uf0b7")
            self.assertEqual(bullet.metadata["numbering"]["label"], "•")
            self.assertEqual(rtl.metadata["numbering"]["label"], "1.")
            self.assertEqual([bullet.text, rtl.text], ["Symbol bullet", "RTL marker"])
            self.assertNotIn("•", document.to_markdown())
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = pdf[0].get_text()
                self.assertIn("•", text)
                self.assertIn(".1", text)


if __name__ == "__main__":
    unittest.main()
