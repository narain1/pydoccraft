from __future__ import annotations

import io
import struct
from pathlib import Path
import tempfile
import unittest
import warnings
import zipfile

import doc_ex
from fixtures import PNG, docx_fixture


def epub_fixture(*, encrypted: bool = False, version: str = "3.0", chapter_one: str | None = None) -> bytes:
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        archive.writestr(
            "META-INF/container.xml",
            '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>',
        )
        if encrypted:
            archive.writestr("META-INF/encryption.xml", "<encryption/>")
        archive.writestr(
            "OEBPS/content.opf",
            f'<?xml version="1.0"?><package version="{version}" xmlns="http://www.idpf.org/2007/opf" unique-identifier="book-id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="book-id">urn:test:book</dc:identifier><dc:title>Native EPUB fixture</dc:title><dc:creator>Test Author</dc:creator><dc:language>en</dc:language><dc:publisher>pydoccraft tests</dc:publisher></metadata><manifest><item id="one" href="one.xhtml" media-type="application/xhtml+xml"/><item id="two" href="two.xhtml" media-type="application/xhtml+xml"/><item id="image" href="image.png" media-type="image/png"/></manifest><spine><itemref idref="one"/><itemref idref="two"/></spine></package>',
        )
        archive.writestr(
            "OEBPS/one.xhtml",
            chapter_one or '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head><body><h1>First chapter</h1><p>Alpha café. <a href="https://example.com">Example</a></p><p><img src="image.png" alt="Red pixel" /></p></body></html>',
        )
        archive.writestr("OEBPS/image.png", PNG)
        archive.writestr(
            "OEBPS/two.xhtml",
            '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Two</title></head><body><h1>Second chapter</h1><p>Beta naïve.</p></body></html>',
        )
    return output.getvalue()


def rewrite_epub(data: bytes, updates: dict[str, bytes | str]) -> bytes:
    members: dict[str, bytes] = {}
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for name in archive.namelist():
            members[name] = archive.read(name)
    members.update({name: value.encode() if isinstance(value, str) else value for name, value in updates.items()})
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, value in members.items():
            archive.writestr(name, value)
    return output.getvalue()


class EpubTests(unittest.TestCase):
    def test_xml_sensitive_metadata_roundtrips(self) -> None:
        with doc_ex.new("epub") as book:
            book.set_metadata(title="A & <B>", identifier="urn:a&b", language="en", authors=["A & B", "C <D>"])
            book.add_chapter("Chapter").add_paragraph("Text")
            data = book.to_bytes()
        with doc_ex.open(data) as document:
            self.assertEqual(document.metadata["title"], "A & <B>")
            self.assertEqual(document.metadata["identifiers"], ["urn:a&b"])
            self.assertEqual(document.metadata["authors"], ["A & B", "C <D>"])

    def test_multiple_rootfiles_are_rejected_before_metadata_parsing(self) -> None:
        data = rewrite_epub(epub_fixture(), {
            "META-INF/container.xml": '<container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/><rootfile full-path="other" media-type="other/type"/></rootfiles></container>'
        })
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            doc_ex.open(data)

    def test_duplicate_package_members_are_rejected_before_engine_parsing(self) -> None:
        output = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(epub_fixture())) as source:
            with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
                for member in source.infolist():
                    archive.writestr(member.filename, source.read(member))
                with warnings.catch_warnings():
                    warnings.simplefilter("ignore", UserWarning)
                    archive.writestr("OEBPS/content.opf", "<!DOCTYPE package><package/>")
        with self.assertRaises(doc_ex.UnsafeArchiveError):
            doc_ex.open(output.getvalue())

    def test_raw_duplicate_member_count_is_bounded(self) -> None:
        output = io.BytesIO()
        with warnings.catch_warnings():
            warnings.simplefilter("ignore", UserWarning)
            with zipfile.ZipFile(output, "w") as archive:
                for _ in range(10_001):
                    archive.writestr("same", "")
        with self.assertRaises(doc_ex.UnsafeArchiveError):
            doc_ex.open(output.getvalue(), filetype="epub")

    def test_forged_directory_count_is_rejected(self) -> None:
        data = bytearray(epub_fixture())
        end = data.rfind(b"PK\x05\x06")
        self.assertGreaterEqual(end, 0)
        struct.pack_into("<HH", data, end + 8, 1, 1)
        with self.assertRaises(doc_ex.UnsafeArchiveError):
            doc_ex.open(bytes(data), filetype="epub")

    def test_local_and_central_names_must_agree(self) -> None:
        data = bytearray(epub_fixture())
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            member = archive.getinfo("OEBPS/content.opf")
        offset = member.header_offset + 30
        data[offset] = ord("X")
        with self.assertRaises(doc_ex.UnsafeArchiveError):
            doc_ex.open(bytes(data), filetype="epub")

    def test_data_descriptor_members_remain_supported(self) -> None:
        class Sink(io.RawIOBase):
            def __init__(self):
                self.data = bytearray()

            def writable(self):
                return True

            def write(self, value):
                self.data.extend(value)
                return len(value)

        sink = Sink()
        with zipfile.ZipFile(io.BytesIO(epub_fixture())) as source:
            with zipfile.ZipFile(sink, "w", zipfile.ZIP_DEFLATED) as archive:
                for member in source.infolist():
                    archive.writestr(member.filename, source.read(member))
        with doc_ex.open(bytes(sink.data)) as document:
            self.assertEqual(document.metadata["title"], "Native EPUB fixture")
            self.assertEqual(document.page_count, 2)

    def test_zip64_is_explicitly_unsupported_by_bounded_preflight(self) -> None:
        output = io.BytesIO()
        original_limit = zipfile.ZIP64_LIMIT
        try:
            zipfile.ZIP64_LIMIT = 1
            with zipfile.ZipFile(output, "w", allowZip64=True) as archive:
                archive.writestr("mimetype", "application/epub+zip")
        finally:
            zipfile.ZIP64_LIMIT = original_limit
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            doc_ex.open(output.getvalue(), filetype="epub")

    def test_zip_comment_with_ambiguous_signature_is_explicitly_unsupported(self) -> None:
        data = io.BytesIO(epub_fixture())
        with zipfile.ZipFile(data, "a") as archive:
            archive.comment = b"A harmless PK\x05\x06 marker"
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            doc_ex.open(data.getvalue())

    def test_bytes_path_metadata_order_and_owned_values(self) -> None:
        data = epub_fixture()
        with doc_ex.open(data) as document:
            self.assertEqual(document.metadata["format"], "epub")
            self.assertEqual(document.metadata["title"], "Native EPUB fixture")
            self.assertEqual(document.metadata["authors"], ["Test Author"])
            self.assertEqual(document.metadata["languages"], ["en"])
            self.assertEqual(document.page_count, 2)
            self.assertIn("First chapter", document[0].get_text())
            self.assertIn("Second chapter", document[1].get_text())
            self.assertEqual(document[0].blocks[0].kind, "heading")
            self.assertEqual(document[0].blocks[0].level, 1)
            self.assertEqual(document[0].get_links()[0].uri, "https://example.com")
            self.assertEqual(document[0].get_links()[0].text, "Example")
            self.assertEqual(document[0].get_images()[0].data, PNG)
            self.assertEqual(document[0].get_images()[0].alt, "Red pixel")
            page = document[0]
        self.assertIn("Alpha café", page.get_text())

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "book.epub"
            path.write_bytes(data)
            self.assertEqual(doc_ex.open(path).page_count, 2)

    def test_native_creation_roundtrips_metadata_structure_links_images_and_copy(self) -> None:
        with doc_ex.new("epub") as book:
            book.set_metadata(
                title="Created book",
                identifier="urn:test:created",
                language="en",
                authors=["Author One"],
            )
            chapter = book.add_chapter("Introduction")
            chapter.add_heading("Welcome & hello", level=1)
            chapter.add_paragraph("Created paragraph with café.")
            chapter.add_link("Project", "https://example.com")
            chapter.add_image(PNG, media_type="image/png", alt="Red pixel")
            clone = book.copy()
            clone.add_chapter("Appendix").add_paragraph("Only in copy")
            data = book.to_bytes()
            self.assertEqual(book.chapter_count, 1)
            self.assertEqual(clone.chapter_count, 2)
            clone.close()

        with doc_ex.open(data, filetype="epub") as reopened:
            self.assertEqual(reopened.metadata["title"], "Created book")
            self.assertEqual(reopened.metadata["authors"], ["Author One"])
            self.assertEqual(reopened.page_count, 1)
            self.assertEqual(reopened[0].blocks[0].kind, "heading")
            self.assertIn("Welcome & hello", reopened[0].get_text())
            self.assertEqual(reopened[0].get_links()[0].uri, "https://example.com")
            self.assertEqual(reopened[0].get_images()[0].data, PNG)

        with doc_ex.new("epub") as invalid:
            with self.assertRaises(ValueError):
                invalid.to_bytes()
            invalid.set_metadata(title="Book", identifier="urn:test", language="en")
            with self.assertRaises(ValueError):
                invalid.to_bytes()
            chapter = invalid.add_chapter("One")
            with self.assertRaises(ValueError):
                chapter.add_heading("bad", level=7)
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                chapter.add_link("unsafe", "javascript:alert(1)")
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                chapter.add_image(PNG, media_type="image/unknown")

    def test_epub2_detection_ooxml_collision_and_malformed_xhtml(self) -> None:
        with doc_ex.open(epub_fixture(version="2.0"), filetype="ebook") as document:
            self.assertTrue(str(document.metadata["epub_version"]).startswith("2"))
            self.assertEqual(document.page_count, 2)

        source = docx_fixture()
        members = {}
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            for name in archive.namelist():
                members[name] = archive.read(name)
        members["mimetype"] = b"application/epub+zip"
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
            for name, value in members.items():
                archive.writestr(name, value)
        self.assertEqual(doc_ex.open(output.getvalue()).metadata["format"], "docx")

        malformed = '<html xmlns="http://www.w3.org/1999/xhtml"><body><p>broken</body></html>'
        with self.assertRaises(doc_ex.MalformedDocumentError):
            doc_ex.open(epub_fixture(chapter_one=malformed))
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            doc_ex.open(epub_fixture(), password="secret")

        unsafe = io.BytesIO()
        with zipfile.ZipFile(unsafe, "w") as archive:
            archive.writestr("mimetype", "application/epub+zip")
            archive.writestr("../escape", b"x")
        with self.assertRaises(doc_ex.UnsafeArchiveError):
            doc_ex.open(unsafe.getvalue())

    def test_native_creation_save_stream_close_and_atomic_validation(self) -> None:
        book = doc_ex.new("ebook")
        book.set_metadata(title="Saved", identifier="urn:saved", authors=["A"])
        book.add_chapter("One").add_paragraph("Saved content")
        stream = io.BytesIO()
        book.save(stream)
        self.assertFalse(stream.closed)
        self.assertIn("Saved content", doc_ex.open(stream.getvalue())[0].get_text())
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "saved.epub"
            book.save(path)
            self.assertEqual(doc_ex.open(path).metadata["title"], "Saved")
            with self.assertRaises(ValueError):
                book.save(Path(directory) / "wrong.pdf")
        book.close()
        book.close()
        with self.assertRaises(doc_ex.DocumentClosedError):
            _ = book.chapter_count

    def test_semantic_fallback_offsets_uri_and_detection_edges(self) -> None:
        chapter = (
            '<html xmlns="http://www.w3.org/1999/xhtml"><body>'
            '<h1>Title</h1><ul><li>List café <a href="HTTPS://example.com">Again</a></li></ul>'
            '<blockquote>Quoted body</blockquote></body></html>'
        )
        data = rewrite_epub(
            epub_fixture(chapter_one=chapter),
            {"[Content_Types].xml": "wordprocessingml.document.main+xml"},
        )
        with doc_ex.open(data) as document:
            self.assertEqual(document.metadata["format"], "epub")
            text = document[0].get_text()
            self.assertIn("List café Again", text)
            self.assertIn("Quoted body", text)
            link = document[0].get_links()[0]
            block = next(block for block in document[0].blocks if link in block.links)
            self.assertEqual(block.text[link.metadata["start"]:link.metadata["end"]], "Again")
            self.assertEqual(link.metadata["target_kind"], "external")

    def test_strict_package_and_authoring_safety_are_atomic(self) -> None:
        malformed_container = rewrite_epub(
            epub_fixture(),
            {"META-INF/container.xml": b'<container><rootfiles><rootfile full-path="\xff"/></rootfiles></container>'},
        )
        with self.assertRaises(doc_ex.MalformedDocumentError):
            doc_ex.open(malformed_container)
        malformed_package = rewrite_epub(
            epub_fixture(),
            {"OEBPS/content.opf": '<package><manifest></package>'},
        )
        with self.assertRaises(doc_ex.MalformedDocumentError):
            doc_ex.open(malformed_package)
        malformed_unreferenced_xhtml = rewrite_epub(
            epub_fixture(),
            {"OEBPS/unused.xhtml": '<html xmlns="http://www.w3.org/1999/xhtml"><body><p>broken</body></html>'},
        )
        with self.assertRaises(doc_ex.MalformedDocumentError):
            doc_ex.open(malformed_unreferenced_xhtml)

        with doc_ex.new("epub") as book:
            book.set_metadata(title="Safe", identifier="urn:safe", language="en")
            chapter = book.add_chapter("One")
            chapter.add_paragraph("before")
            before = book.to_bytes()
            for operation in (
                lambda: chapter.add_paragraph("bad\x01text"),
                lambda: chapter.add_link("bad\x02label", "https://example.com"),
                lambda: chapter.add_image(b'<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>', media_type="image/svg+xml"),
                lambda: chapter.add_image(
                    b'<svg xmlns="http://www.w3.org/2000/svg"><rect fill="url(https://example.com/paint)"/></svg>',
                    media_type="image/svg+xml",
                ),
                lambda: chapter.add_image(b"not png", media_type="image/png"),
            ):
                with self.assertRaises((ValueError, doc_ex.UnsupportedFormatError)):
                    operation()
                self.assertEqual(book.to_bytes(), before)
            chapter.add_link("Uppercase", "HTTPS://example.com")

    def test_reflowable_preview_and_conversions_reopen(self) -> None:
        from test_authoring import image_fixture

        source = rewrite_epub(epub_fixture(), {"OEBPS/image.png": image_fixture()})
        with doc_ex.open(source) as document:
            self.assertTrue(document.render_page(0, dpi=72).startswith(b"\x89PNG"))
            self.assertTrue(document.render_page(1, dpi=72, clip=(15, 15, 120, 50)).startswith(b"\x89PNG"))
            for output in ("pdf", "docx"):
                with self.subTest(output=output), doc_ex.open(document.convert(output)) as converted:
                    self.assertIn("First chapter", converted[0].get_text())
                    self.assertIn("Second chapter", converted[-1].get_text())
                    self.assertTrue(converted[0].get_images())
                    self.assertIn("https://example.com", [link.uri for link in converted[0].get_links()])

    def test_namespace_paths_navigation_and_spine_occurrences(self) -> None:
        data = epub_fixture(chapter_one='<h:html xmlns:h="http://www.w3.org/1999/xhtml"><h:body><h:p>Greek λ<![CDATA[ & café]]><h:br/><h:a href="../Text/Two%20%C3%A9.xhtml#tag">go</h:a></h:p><h:img src="../Media/caf%C3%A9.png"/></h:body></h:html>')
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</metadata>", b'<other:title xmlns:other="urn:other">wrong</other:title><dc:title xmlns:dc="http://purl.org/dc/elements/1.1/">Second &amp; title</dc:title></metadata>')
        package = package.replace(b'href="two.xhtml"', b'href="../Text/Two%20%C3%A9.xhtml"').replace(b'<item id="image" href="image.png"', b'<item id="image" href="../Media/caf%C3%A9.png"')
        package = package.replace(b"</manifest>", b'<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest>')
        package = package.replace(b'<itemref idref="two"/>', b'<itemref idref="two" linear="no"/><itemref idref="one"/>')
        data = rewrite_epub(data, {
            "OEBPS/content.opf": package,
            "Media/café.png": PNG,
            "Text/Two é.xhtml": '<html xmlns="http://www.w3.org/1999/xhtml"><body><p id="tag">Target</p></body></html>',
            "OEBPS/nav.xhtml": '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><span>Part</span><ol><li><a href="../Text/Two%20%C3%A9.xhtml#tag">Target</a></li></ol></li></ol></nav></body></html>',
        })
        with doc_ex.open(data) as document:
            self.assertEqual(document.page_count, 3)
            self.assertEqual(document.metadata["dublin_core"]["title"], ["Native EPUB fixture", "Second & title"])
            self.assertEqual(document.metadata["navigation"][0]["items"][0]["children"][0]["path"], "Text/Two é.xhtml")
            self.assertIn("Greek λ & café\ngo", document[0].get_text())
            self.assertEqual(document[0].get_text(), document[2].get_text())
            self.assertEqual(document[0].get_links()[0].metadata["target_path"], "Text/Two é.xhtml")
            self.assertEqual(document[0].get_images()[0].data, PNG)
            self.assertEqual(document.read_epub_resource("Media/café.png"), PNG)
            with self.assertRaises(doc_ex.UnsafeArchiveError):
                document.read_epub_resource("../Media/café.png")

    def test_nested_tables_linked_images_and_inherited_svg_namespaces(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:s="http://www.w3.org/2000/svg"
          xmlns:m="http://www.w3.org/1998/Math/MathML"><body>
          <table><caption>Caption</caption><thead><tr><th id="h" scope="col">Title</th><th>B</th></tr></thead>
          <tbody><tr><td rowspan="0" headers="h">Row</td><td><table><tr><td>Nested</td></tr></table></td></tr>
          <tr><td><a href="https://example.com/img"><img src="image.png" alt="Linked"/></a></td></tr></tbody></table>
          <p><s:svg width="10" height="10"><s:rect width="10" height="10"/></s:svg><m:math><m:mi>x</m:mi></m:math></p>
          </body></html>"""
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as document:
            page = document[0]
            table = page.find_tables()[0]
            self.assertEqual(table.cells[1][0].row_span, 2)
            self.assertEqual(table.cells[1][0].metadata["html_headers"], "h")
            self.assertEqual(page.get_text().count("Nested"), 1)
            self.assertIn('<th colspan="1" rowspan="1" id="h" scope="col">', page.get_text("markdown"))
            self.assertEqual(page.get_images()[0].metadata["href"], "https://example.com/img")
            import xml.etree.ElementTree as ET
            root = ET.fromstring(page.get_images()[1].data)
            self.assertEqual(root.tag, "{http://www.w3.org/2000/svg}svg")
            self.assertIn("<m:mi>x", page.blocks[-1].metadata["mathml"][0]["source"])

    def test_richer_authoring_roundtrip(self) -> None:
        svg = b'<svg xmlns="http://www.w3.org/2000/svg" width="5" height="5"><rect width="5" height="5"/></svg>'
        with doc_ex.new("epub") as book:
            book.set_metadata(title="a" * 1100, identifier="urn:extended")
            book.set_publication_date("2026-11-23")
            chapter = book.add_chapter("Chapter & table")
            chapter.add_table([["Header", "Name"], ["Row", "Data"]], header=True)
            chapter.add_list(["First", "Second"], ordered=True)
            chapter.add_preformatted("  indented\rline")
            chapter.add_image(svg, media_type="image/svg+xml", alt="tab\t and newline\n")
            with doc_ex.open(book.to_bytes()) as read:
                self.assertEqual(read.metadata["title"], "a" * 1100)
                self.assertTrue(read.metadata["publication_date"].startswith("2026-11-23"))
                self.assertEqual(read[0].find_tables()[0].rows[1], ["Row", "Data"])
                self.assertIn("  indented\rline", read[0].get_text())
                self.assertEqual(read[0].get_images()[0].alt, "tab\t and newline\n")
                self.assertEqual(read[0].get_images()[0].data, svg)

    def test_epub2_ncx_and_existing_metadata_edit_preserve_members(self) -> None:
        source = epub_fixture(version="2.0")
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</manifest><spine>", b'<item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="toc">')
        ncx = '<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="a"><navLabel><text>One &amp; first</text></navLabel><content src="one.xhtml#first"/></navPoint></navMap></ncx>'
        source = rewrite_epub(source, {"OEBPS/content.opf": package, "OEBPS/toc.ncx": ncx})
        with doc_ex.open(source) as document:
            self.assertEqual(document.metadata["navigation"][0]["items"][0]["label"], "One & first")
            changed = document.edit_epub_metadata(title="A & new title", description="New description")
            self.assertEqual(document.metadata["title"], "Native EPUB fixture")
        with doc_ex.open(changed) as edited:
            self.assertEqual(edited.metadata["title"], "A & new title")
            self.assertEqual(edited.metadata["description"], "New description")
            self.assertEqual(edited.metadata["identifiers"], ["urn:test:book"])
            self.assertIn("First chapter", edited[0].get_text())
        with zipfile.ZipFile(io.BytesIO(source)) as old, zipfile.ZipFile(io.BytesIO(changed)) as new:
            self.assertEqual(new.infolist()[0].filename, "mimetype")
            self.assertEqual(new.infolist()[0].compress_type, zipfile.ZIP_STORED)
            for name in set(old.namelist()) - {"OEBPS/content.opf"}:
                self.assertEqual(old.read(name), new.read(name), name)

    def test_fixed_layout_and_encryption_remain_explicit(self) -> None:
        with zipfile.ZipFile(io.BytesIO(epub_fixture())) as archive:
            package = archive.read("OEBPS/content.opf")
        fixed = rewrite_epub(epub_fixture(), {
            "OEBPS/content.opf": package.replace(b'<itemref idref="one"/>', b'<itemref idref="one" properties="rendition:layout-pre-paginated"/>')
        })
        with doc_ex.open(fixed) as document:
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                document.render_page(0)
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                document.convert("pdf")
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            doc_ex.open(epub_fixture(encrypted=True))

        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as archive:
            archive.writestr("mimetype", "application/epub+zip")
        with self.assertRaises(doc_ex.MalformedDocumentError):
            doc_ex.open(output.getvalue())


if __name__ == "__main__":
    unittest.main()
