from __future__ import annotations

import hashlib
import io
import struct
from pathlib import Path
import tempfile
import unittest
import warnings
import wave
import xml.etree.ElementTree as ET
import zipfile

import doc_ex
from fixtures import PNG, docx_fixture
from test_pdf_underlays import solid
from test_rendering import png_chunks
from test_render_geometry import rgba_pixels


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


def narrated_epub(*, first_clip_end: str = "00:00:01.125", first_target: str = "../one.xhtml#caf%C3%A9") -> tuple[bytes, bytes]:
    source = epub_fixture(chapter_one='<html xmlns="http://www.w3.org/1999/xhtml"><body id="chapter"><p id="café">Original narration</p><p id="aside">Text cue</p></body></html>')
    with zipfile.ZipFile(io.BytesIO(source)) as archive:
        package = archive.read("OEBPS/content.opf")
    package = package.replace(b'<item id="one" ', b'<item id="one" media-overlay="sync" ')
    package = package.replace(b'<item id="two" ', b'<item id="two" media-overlay="sync" ')
    package = package.replace(b"</manifest>", b'<item id="sync" href="Sync/read.smil" media-type="application/smil+xml"/><item id="voice" href="Media/voice.wav" media-type="audio/wav"/></manifest>')
    overlay = f"""<smil xmlns="http://www.w3.org/ns/SMIL" xmlns:epub="http://www.idpf.org/2007/ops" version="3.0">
    <body><seq epub:textref="../one.xhtml#chapter" epub:type="chapter">
      <par id="start"><text src="{first_target}"/><audio src="../Media/voice.wav" clipBegin="250ms" clipEnd="{first_clip_end}"/></par>
      <par id="pause" epub:type="aside"><text src="../one.xhtml#aside"/></par>
    </seq><par id="next"><audio src="../Media/voice.wav" clipBegin="1.125s"/><text src="../two.xhtml"/></par></body>
    </smil>"""
    audio = io.BytesIO()
    with wave.open(audio, "wb") as wav:
        wav.setparams((1, 1, 8000, 0, "NONE", "not compressed"))
        wav.writeframes(bytes(range(256)) * 64)
    recorded = audio.getvalue()
    return rewrite_epub(source, {
        "OEBPS/content.opf": package,
        "OEBPS/Sync/read.smil": overlay,
        "OEBPS/Media/voice.wav": recorded,
    }), recorded


class EpubTests(unittest.TestCase):
    def test_spine_edit_updates_navigation_and_keeps_removed_resources_reachable(self) -> None:
        source, original_audio = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(
            b"</manifest>",
            b'<item id="nav" href="Nav/book.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest>',
        ).replace(b'<itemref idref="one"/>', b'<itemref id="retained-id" idref="one" linear="no"/>')
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/Nav/book.xhtml": """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
            <head><title>Original navigation</title></head><body>
            <nav epub:type="toc" id="author-toc"><ol>
              <li><span>Part &amp; section</span><ol><li><a href="../one.xhtml#caf%C3%A9">Narrated</a></li></ol></li>
              <li><a href="../two.xhtml">Original last</a></li>
            </ol></nav>
            <nav epub:type="page-list"><ol><li><a href="../two.xhtml">2</a></li></ol></nav>
            </body></html>""",
        })
        new_page = doc_ex.EpubChapterSource(
            'Inserted "Étude"',
            """<html xmlns="http://www.w3.org/1999/xhtml"><head>
            <title>Inserted Fixed Chapter</title><meta name="viewport" content="width=120,height=80"/>
            <link rel="stylesheet" href="../OEBPS/Styles/new.css"/></head>
            <body><h1>New Chapter</h1><a href="../OEBPS/two.xhtml">Original backlink</a>
            <img src="../OEBPS/Art/gold.png" alt="Gold"/></body></html>""",
            path="Extras/new étude.xhtml",
            fixed_layout=True,
        )
        css = b"body { background-color: #f2e0a0; margin: 0; } img { position: absolute; top: 50px; left: 80px; width: 20px; height: 20px; }"
        art = solid((220, 170, 0))
        with doc_ex.open(source) as original:
            self.assertEqual(original.edit_epub_structure([0, 1]), source)
            self.assertEqual(original.edit_epub_structure(), source)
            revised = original.edit_epub_structure([1, new_page, 0], resources={
                "OEBPS/Styles/new.css": doc_ex.EpubResource("text/css", css),
                "OEBPS/Art/gold.png": doc_ex.EpubResource("image/png", art),
            })
            self.assertEqual(len(original), 2)
            cues = original.epub_media_cues(0)
        with doc_ex.open(revised) as published:
            self.assertEqual(len(published), 3)
            self.assertIn("Second chapter", published[0].get_text())
            self.assertEqual(published[1].metadata["chapter_path"], "Extras/new étude.xhtml")
            self.assertIn("New Chapter", published[1].get_text())
            self.assertEqual(published[1].metadata["viewport"], [90, 60])
            self.assertEqual(published.read_epub_resource("OEBPS/Styles/new.css"), css)
            self.assertEqual(published.read_epub_resource("OEBPS/Art/gold.png"), art)
            rendered = png_chunks(published.render_page(1, dpi=96))
            self.assertEqual(struct.unpack_from(">II", rendered[b"IHDR"]), (120, 80))
            width, rgba = rgba_pixels(rendered)
            offset = (62 * width + 90) * 4
            self.assertEqual(rgba[offset:offset + 4], bytes((220, 170, 0, 255)))
            self.assertFalse(published[2].metadata["linear"])
            self.assertEqual(published.epub_media_cues(2), cues)
            items = published.metadata["navigation"][0]["items"]
            self.assertEqual([item["label"] for item in items], ["Original last", 'Inserted "Étude"', "Part & section"])
            self.assertEqual(items[2]["children"][0]["fragment"], "café")
            modified = next(meta["value"] for meta in published.metadata["package_metadata"] if meta["property"] == "dcterms:modified")
            self.assertRegex(modified, r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
            removed = published.edit_epub_structure([1, 2], resources={
                "OEBPS/Styles/new.css": doc_ex.EpubResource("text/css", css + b"\n/* revised */"),
            })
        with zipfile.ZipFile(io.BytesIO(source)) as before, zipfile.ZipFile(io.BytesIO(revised)) as after:
            for path in set(before.namelist()) - {"OEBPS/content.opf", "OEBPS/Nav/book.xhtml"}:
                self.assertEqual(before.read(path), after.read(path), path)
            self.assertIn(b'id="retained-id"', after.read("OEBPS/content.opf"))
        with doc_ex.open(removed) as published:
            self.assertEqual(len(published), 2)
            self.assertEqual(published.epub_media_cues(1), cues)
            self.assertEqual(published.read_epub_resource("OEBPS/Media/voice.wav"), original_audio)
            self.assertIn(b"Second chapter", published.read_epub_resource("OEBPS/two.xhtml"))
            self.assertEqual(published[0].get_links()[0].metadata["target_path"], "OEBPS/two.xhtml")
            self.assertEqual([nav["kind"] for nav in published.metadata["navigation"]], ["toc"])
            self.assertTrue(published.read_epub_resource("OEBPS/Styles/new.css").endswith(b"/* revised */"))

    def test_spine_edit_reorders_epub2_ncx_and_creates_missing_navigation(self) -> None:
        source = epub_fixture(version="2.0")
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"<spine>", b'<spine toc="ncx">').replace(
            b"</manifest>", b'<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest>',
        )
        ncx = """<n:ncx xmlns:n="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
        <n:head><n:meta name="dtb:depth" content="1"/></n:head><n:docTitle><n:text>Book</n:text></n:docTitle>
        <n:navMap><n:navPoint id="first" playOrder="1"><n:navLabel><n:text>First</n:text></n:navLabel>
        <n:content src="one.xhtml"/></n:navPoint><n:navPoint id="last" playOrder="2">
        <n:navLabel><n:text>Last</n:text></n:navLabel><n:content src="two.xhtml"/></n:navPoint></n:navMap></n:ncx>"""
        chapter = doc_ex.EpubChapterSource("Added", '<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Fresh content</p></body></html>')
        data = rewrite_epub(source, {"OEBPS/content.opf": package, "OEBPS/toc.ncx": ncx})
        with doc_ex.open(data) as original, doc_ex.open(original.edit_epub_structure([1, chapter, 0])) as edited:
            self.assertEqual(edited.metadata["epub_version"], "2.0")
            self.assertEqual([item["label"] for item in edited.metadata["navigation"][0]["items"]], ["Last", "Added", "First"])
            ncx_nodes = ET.fromstring(edited.read_epub_resource("OEBPS/toc.ncx")).findall(".//{*}navPoint")
            self.assertEqual([node.attrib["playOrder"] for node in ncx_nodes], ["1", "2", "3"])
            self.assertEqual(ncx_nodes[0].attrib["id"], "last")
        for version in ("2.0", "3.0"):
            with self.subTest(version=version), doc_ex.open(epub_fixture(version=version)) as original:
                data = original.edit_epub_structure([chapter, 0])
                with doc_ex.open(data) as edited:
                    self.assertEqual(len(edited), 2)
                    self.assertEqual(edited[0].get_text(), "Fresh content")
                    self.assertEqual(edited.metadata["navigation"][0]["items"][0]["label"], "Added")

    def test_spine_edit_promotes_nested_navigation_with_inherited_namespaces(self) -> None:
        source = epub_fixture()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(
            b"</manifest>",
            b'<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest>',
        )
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/nav.xhtml": """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
            <head><title>Navigation</title></head><body><nav epub:type="toc"><ol>
            <li xmlns:local="http://www.w3.org/1999/xhtml"><a href="one.xhtml">Removed</a>
            <ol><local:li><local:a href="two.xhtml">Surviving</local:a></local:li></ol></li>
            </ol></nav></body></html>""",
        })
        with doc_ex.open(source) as original, doc_ex.open(original.edit_epub_structure([1])) as edited:
            items = edited.metadata["navigation"][0]["items"]
            self.assertEqual([item["label"] for item in items], ["Surviving"])
            nav = ET.fromstring(edited.read_epub_resource("OEBPS/nav.xhtml"))
            self.assertEqual(len(nav.findall(".//{*}li")), 1)

    def test_spine_and_resource_changes_fail_atomically(self) -> None:
        source = epub_fixture()
        valid = '<html xmlns="http://www.w3.org/1999/xhtml"><body><p>New</p></body></html>'
        with doc_ex.open(source) as document:
            for order in ([], [0, -2], [doc_ex.EpubChapterSource("Collision", valid, "OEBPS/one.xhtml")]):
                with self.subTest(order=order), self.assertRaises(ValueError):
                    document.edit_epub_structure(order)
            with self.assertRaises(doc_ex.MalformedDocumentError):
                document.edit_epub_structure([1, doc_ex.EpubChapterSource("Bad", "<html>"), 0])
            with self.assertRaises(ValueError):
                document.edit_epub_structure(resources={"OEBPS/image.png": doc_ex.EpubResource("audio/mpeg", b"wrong type")})
            with self.assertRaises(ValueError):
                document.edit_epub_structure(resources={"META-INF/container.xml": doc_ex.EpubResource("text/css", b"bad")})
            with self.assertRaises(doc_ex.UnsafeArchiveError):
                document.edit_epub_structure(resources={"../new.css": doc_ex.EpubResource("text/css", b"a {}")})
            self.assertEqual(document.edit_epub_structure(), source)
        signed = rewrite_epub(source, {"META-INF/signatures.xml": "<signatures/>"})
        with doc_ex.open(signed) as document, self.assertRaises(doc_ex.UnsupportedFormatError):
            document.edit_epub_structure([1, 0])

    def test_fixed_layout_uses_viewport_and_local_stylesheet_cascade(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
        <meta name="viewport" content="width=320, height=240"/>
        <link rel="stylesheet" href="Styles/book.css"/>
        </head><body>
        <section><div class="panel" id="first" data-active="YES">
        <p>Sample <span>coloured</span> text</p><p class="hidden">Hidden filler</p>
        </div><div class="overlay"/><img src="image.png" alt="Gold swatch"/></section>
        </body></html>"""
        source = epub_fixture(chapter_one=chapter)
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</metadata>", b'<meta property="rendition:layout">pre-paginated</meta></metadata>')
        package = package.replace(b'<itemref idref="two"/>', b'<itemref idref="two" properties="rendition:layout-reflowable"/>')
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/Styles/book.css": """@import "theme/colors.css";
                html { background: #eee; }
                body { margin: 0; }
                section { position: relative; }
                .panel { position: absolute; left: 40px; top: 24px; width: 160px; height: 100px; padding: 6px; }
                #first { background-color: blue; }
                .panel p { margin: 0; font-size: 20px; }
                .panel span { color: #008000; font-weight: bold; }
                .panel .hidden { display: none; }
                .overlay { position: absolute; left: 156px; top: 86px; width: 62px; height: 80px; z-index: 1; background-color: #0000dd; }
                img { position: absolute; left: 245px; top: 130px; width: 30px; height: 24px; }
                @media screen and (max-width: 325px) { #first { border: 4px solid black; } }
            """,
            "OEBPS/Styles/theme/colors.css": "section > div.panel[data-active='yes' i] { background-color: #dd4444 !important; }",
            "OEBPS/image.png": solid((220, 170, 0)),
        })
        with doc_ex.open(source) as document:
            self.assertEqual(document[0].metadata["viewport"], [240, 180])
            self.assertTrue(document[0].metadata["fixed_layout"])
            self.assertFalse(document[1].metadata.get("fixed_layout", False))
            self.assertIn("Second chapter", document[1].get_text())
            self.assertTrue(document.render_page(1, dpi=72).startswith(b"\x89PNG"))
            for dpi, scale in ((96, 1), (192, 2)):
                chunks = png_chunks(document.render_page(0, dpi=dpi))
                self.assertEqual(struct.unpack_from(">II", chunks[b"IHDR"]), (320 * scale, 240 * scale))
                width, pixels = rgba_pixels(chunks)

                def pixel(x: int, y: int) -> bytes:
                    offset = ((y * scale) * width + x * scale) * 4
                    return pixels[offset:offset + 4]

                self.assertEqual(pixel(12, 12), bytes((238, 238, 238, 255)))
                self.assertEqual(pixel(42, 26), bytes((0, 0, 0, 255)))
                self.assertEqual(pixel(55, 110), bytes((221, 68, 68, 255)))
                self.assertEqual(pixel(165, 105), bytes((0, 0, 221, 255)))
                self.assertEqual(pixel(260, 140), bytes((220, 170, 0, 255)))
                self.assertIn(bytes((0, 128, 0, 255)), [pixels[i:i + 4] for i in range(0, len(pixels), 4)])
            clip = png_chunks(document.render_page(0, dpi=96, clip=(37.5, 67.5, 30.0, 15.0)))
            self.assertEqual(struct.unpack_from(">II", clip[b"IHDR"]), (40, 20))
            _, pixels = rgba_pixels(clip)
            self.assertEqual(pixels[:4], bytes((221, 68, 68, 255)))

    def test_reflow_css_flex_layout_and_archive_only_imports(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
        <style>body { margin: 0; } .row { display: flex; gap: 10px; }
        .row div { width: 70px; height: 28px; }
        .row div:first-child { background: #bb2233; }
        .row div + div { background: #2244bb; } </style></head>
        <body><section class="row"><div>Left</div><div>Right</div></section></body></html>"""
        source = epub_fixture(chapter_one=chapter)
        with doc_ex.open(source) as document:
            width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=96)))
            offset = (22 * width + 50) * 4
            self.assertEqual(pixels[offset:offset + 4], bytes((187, 34, 51, 255)))
            offset = (22 * width + 136) * 4
            self.assertEqual(pixels[offset:offset + 4], bytes((34, 68, 187, 255)))
        grid = chapter.replace("display: flex; gap: 10px", "display: grid; grid-template-columns: 70px 70px; gap: 18px")
        with doc_ex.open(epub_fixture(chapter_one=grid)) as document:
            width, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=96)))
            offset = (22 * width + 84) * 4
            self.assertEqual(pixels[offset:offset + 4], bytes((255, 255, 255, 255)))
            offset = (22 * width + 92) * 4
            self.assertEqual(pixels[offset:offset + 4], bytes((34, 68, 187, 255)))
        remote = chapter.replace("body { margin", '@import "https://example.com/style.css"; body { margin')
        with doc_ex.open(epub_fixture(chapter_one=remote)) as document:
            self.assertIn("Left", document[0].get_text())
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                document.render_page(0)
        cycle = chapter.replace("body { margin", '@import "cycle.css"; body { margin')
        with doc_ex.open(rewrite_epub(epub_fixture(chapter_one=cycle), {"OEBPS/cycle.css": '@import "./cycle.css";'})) as document:
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "cyclic"):
                document.render_page(0)

    def test_epub_packaged_font_alias_shapes_and_stays_local_to_publication(self) -> None:
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
        <meta name="viewport" content="width=280,height=120"/>
        <link rel="stylesheet" href="Style/main.css"/>
        </head><body><p>Wide W and narrow i</p></body></html>"""
        data = epub_fixture(chapter_one=xhtml)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</metadata>", b'<meta property="rendition:layout">pre-paginated</meta></metadata>')
        package = package.replace(b'<itemref idref="two"/>', b'<itemref idref="two" properties="rendition:layout-reflowable"/>')
        package = package.replace(b"</manifest>", b'<item id="typeface" href="Fonts/type%20face.ttf" media-type="font/ttf"/></manifest>')
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        declarations = b'body {margin: 0} p {margin: 0; color: #ca2300; font-family: "Book Type"; font-size: 36px; line-height: 1.1;}'
        definition = b"""@font-face {
            font-family: "Book Type";
            src: local("Missing install"), url("../../Fonts/type%20face.ttf") format("truetype");
        }"""
        common = {"OEBPS/content.opf": package, "OEBPS/Style/main.css": b'@import "Nested/face.css"; ' + declarations}
        baseline = rewrite_epub(data, {**common, "OEBPS/Style/Nested/face.css": b""})
        textbook = rewrite_epub(data, {
            **common, "OEBPS/Style/Nested/face.css": definition,
            "OEBPS/Fonts/type face.ttf": font,
        })
        with doc_ex.open(baseline) as against, doc_ex.open(textbook) as book:
            without = against.render_page(0, dpi=96)
            with_font = book.render_page(0, dpi=96)
            self.assertNotEqual(with_font, without)
            self.assertEqual(against.render_page(0, dpi=96), without)
            self.assertEqual(book.render_page(0, dpi=96), with_font)
            width, pixels = rgba_pixels(png_chunks(with_font))
            self.assertEqual(width, 280)
            self.assertTrue(any(pixels[i:i+4] == bytes((202, 35, 0, 255)) for i in range(0, len(pixels), 4)))
            self.assertIn("Wide W and narrow i", book[0].get_text())
        bad_font = rewrite_epub(textbook, {"OEBPS/Fonts/type face.ttf": b"not a font"})
        with doc_ex.open(bad_font) as book, self.assertRaises(doc_ex.MalformedDocumentError):
            book.render_page(0, dpi=96)

    def test_epub_packaged_fonts_preserve_inline_svg_and_math(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML" xmlns:s="http://www.w3.org/2000/svg"><head>
        <style>@font-face {font-family: 'Local Type'; src: url('font.ttf')}
        body {margin: 0} p {margin: 0; font-family: 'Local Type'; font-size: 24px; color: #1122cc}</style>
        </head><body><p>Font <m:math><m:mfrac><m:mi>x</m:mi><m:mn>7</m:mn></m:mfrac></m:math>
        <s:svg width="25" height="25"><s:rect width="25" height="25" fill="#dd5544"/></s:svg> after</p></body></html>"""
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        data = rewrite_epub(epub_fixture(chapter_one=chapter), {"OEBPS/font.ttf": font})
        with doc_ex.open(data) as document:
            self.assertIn("Font", document[0].get_text())
            self.assertEqual(len(document[0].get_images()), 1)
            _, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=96)))
            colors = {pixels[offset:offset+4] for offset in range(0, len(pixels), 4)}
            self.assertIn(bytes((221, 85, 68, 255)), colors)
            self.assertIn(bytes((17, 34, 204, 255)), colors)

    def test_epub_packaged_fonts_render_table_cells_and_spans(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
        <style>@font-face {font-family: 'Local Table'; src: url('font.ttf')}
        body {margin: 0} table {width: 160px; font-family: 'Local Table'; font-size: 18px}
        td {padding: 3px} td.first {background: #1122cc; color: white}
        td.last {background: #dd5544; color: white}</style></head><body>
        <table><caption>Title</caption>
        <tr><td class="first" rowspan="2">Left</td><td>Top</td></tr>
        <tr><td class="last">Bottom</td></tr><tr><td colspan="2">Both columns</td></tr>
        </table></body></html>"""
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        data = rewrite_epub(epub_fixture(chapter_one=chapter), {"OEBPS/font.ttf": font})
        with doc_ex.open(data) as document:
            _, pixels = rgba_pixels(png_chunks(document.render_page(0, dpi=96)))
            colors = {pixels[offset:offset+4] for offset in range(0, len(pixels), 4)}
            self.assertIn(bytes((17, 34, 204, 255)), colors)
            self.assertIn(bytes((221, 85, 68, 255)), colors)
            self.assertIn("Both columns", document[0].get_text())

    def test_standard_font_obfuscation_is_local_and_survives_edits(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
        <style>@font-face {font-family: 'Wrapped'; src: url('Typeface.ttf')}
        p {font-family: Wrapped; font-size: 28px;}</style>
        </head><body><p>Obfuscated font</p></body></html>"""
        data = epub_fixture(chapter_one=chapter)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</manifest>", b'<item id="font" href="Typeface.ttf" media-type="font/ttf"/></manifest>')
        package = package.replace(b'urn:test:book</dc:identifier>', b'urn:test:  book\n</dc:identifier>')
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        key = hashlib.sha1(b"urn:test:book").digest()
        wrapped = bytes(value ^ key[index % len(key)] for index, value in enumerate(font[:1040])) + font[1040:]
        encryption = """<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container" xmlns:x="http://www.w3.org/2001/04/xmlenc#">
        <x:EncryptedData><x:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
        <x:CipherData><x:CipherReference URI="OEBPS/Typeface.ttf"/></x:CipherData></x:EncryptedData>
        </encryption>"""
        original = rewrite_epub(data, {"OEBPS/content.opf": package, "OEBPS/Typeface.ttf": font})
        encoded = rewrite_epub(original, {"META-INF/encryption.xml": encryption, "OEBPS/Typeface.ttf": wrapped})
        with doc_ex.open(original) as clear, doc_ex.open(encoded) as book:
            preview = clear.render_page(0, dpi=96)
            self.assertEqual(book.render_page(0, dpi=96), preview)
            self.assertEqual(book.read_epub_resource("OEBPS/Typeface.ttf"), wrapped)
            revision = book.edit_epub_structure([1, 0])
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                book.edit_epub_structure(resources={"OEBPS/Typeface.ttf": doc_ex.EpubResource("font/ttf", font)})
        with doc_ex.open(revision) as edited:
            self.assertEqual(edited.render_page(1, dpi=96), preview)
        for disallowed in (
            encryption.replace("OEBPS/Typeface.ttf", "OEBPS/one.xhtml"),
            encryption.replace("http://www.idpf.org/2008/embedding", "http://www.w3.org/2001/04/xmlenc#aes128-cbc"),
        ):
            with self.subTest(encryption=disallowed), self.assertRaises(doc_ex.UnsupportedFormatError):
                doc_ex.open(rewrite_epub(encoded, {"META-INF/encryption.xml": disallowed}))

    def test_embedded_and_linked_svg_render_in_chapter_viewport(self) -> None:
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:s="http://www.w3.org/2000/svg"><head>
        <meta name="viewport" content="width=180,height=100"/>
        <style>
        body {margin: 0; background: #ffeebb}
        .art {position: absolute; left: 10px; top: 20px; width: 60px; height: 40px;}
        img {position: absolute; left: 100px; top: 20px; width: 60px; height: 40px;}
        </style></head><body>
        <s:svg class="art" viewBox="0 0 30 20"><s:title>Red rectangle</s:title>
        <s:rect x="5" y="5" width="20" height="10" fill="#dd2211"/></s:svg>
        <img src="graphics/tile.svg" alt="Blue rectangle"/>
        </body></html>"""
        data = epub_fixture(chapter_one=xhtml)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b'<itemref idref="one"/>', b'<itemref idref="one" properties="rendition:layout-pre-paginated"/>')
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="60" height="40"><rect x="10" y="10" width="40" height="20" fill="#2244dd"/></svg>'
        book = rewrite_epub(data, {"OEBPS/content.opf": package, "OEBPS/graphics/tile.svg": svg})
        with doc_ex.open(book) as loaded:
            self.assertEqual([image.alt for image in loaded[0].get_images()], ["Red rectangle", "Blue rectangle"])
            for dpi, scale in ((96, 1), (192, 2)):
                width, pixels = rgba_pixels(png_chunks(loaded.render_page(0, dpi=dpi)))
                self.assertEqual(width, 180 * scale)
                for x, y, color in ((40, 40, (221, 34, 17)), (130, 40, (34, 68, 221)), (12, 22, (255, 238, 187))):
                    offset = ((y * scale) * width + x * scale) * 4
                    self.assertEqual(pixels[offset:offset + 4], bytes((*color, 255)))
            cropped = png_chunks(loaded.render_page(0, dpi=96, clip=(75, 15, 45, 30)))
            self.assertEqual(struct.unpack_from(">II", cropped[b"IHDR"]), (60, 40))

    def test_semantic_output_reopens_with_embedded_and_linked_svg(self) -> None:
        inline = '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><title>Inline art</title><rect width="40" height="30" fill="green"/></svg>'
        chapter = f'<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Art {inline} here <img src="art.svg" alt="Linked art"/></p></body></html>'
        data = rewrite_epub(epub_fixture(chapter_one=chapter), {"OEBPS/art.svg": inline.replace('fill="green"', 'fill="purple"')})
        with doc_ex.open(data) as book:
            self.assertTrue(book.render_page(0, dpi=144).startswith(b"\x89PNG"))
            for kind in ("docx", "pdf"):
                with self.subTest(kind=kind), doc_ex.open(book.convert(kind), filetype=kind) as converted:
                    self.assertIn("Art", converted[0].get_text())
                    self.assertEqual(len(converted[0].get_images()), 2)
                    self.assertTrue(converted.render_page(0, dpi=96).startswith(b"\x89PNG"))

    def test_svg_spine_cascades_local_styles_and_renders_packaged_resources(self) -> None:
        source = epub_fixture()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(
            b'href="one.xhtml" media-type="application/xhtml+xml"',
            b'href="Graphics/cover.svg" media-type="image/svg+xml"',
        ).replace(b'<itemref idref="one"/>', b'<itemref idref="one" properties="rendition:layout-pre-paginated"/>')
        svg = """<?xml-stylesheet type="text/css" href="../Styles/main.css"?>
        <?xml-stylesheet href="ignored.css" media="print"?>
        <svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
             width="240px" height="160px" viewBox="0 0 120 80">
          <title>Styled vector page</title>
          <style type="text/css">.swatch { fill: #884499 } .cascade { fill: #cc1144 }</style>
          <rect class="swatch" width="20" height="20"/>
          <rect class="cascade" x="20" width="20" height="20" style="fill:#33bb22"/>
          <image xlink:href="../Images/raster%20tile.png" x="40" width="20" height="20"/>
          <g class="label"><text x="3" y="55">Packed type 123</text></g>
        </svg>"""
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        book = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/Graphics/cover.svg": svg,
            "OEBPS/Styles/main.css": """@import 'Nested/palette.css';
                .swatch { fill: #ffcc22 }
                @font-face { font-family: 'Vector Face'; src: url('../Fonts/type.ttf'); }
                .label { font-family: 'vector FACE'; font-size: 18px; fill: #113399; }""",
            "OEBPS/Styles/Nested/palette.css": ".cascade { fill: #ee6600 !important }",
            "OEBPS/Fonts/type.ttf": font,
            "OEBPS/Images/raster tile.png": solid((11, 104, 163)),
        })
        with doc_ex.open(book) as document:
            image, = document[0].get_images()
            self.assertEqual((image.alt, image.data), ("Styled vector page", svg.encode()))
            self.assertEqual(document[0].metadata["viewport"], [90, 60])
            data = document.render_page(0, dpi=96)
            width, pixels = rgba_pixels(png_chunks(data))
            self.assertEqual(width, 120)
            for x, color in ((8, (136, 68, 153)), (28, (238, 102, 0)), (48, (11, 104, 163))):
                offset = (8 * width + x) * 4
                self.assertEqual(pixels[offset:offset+4], bytes((*color, 255)))
            self.assertTrue(bytes((17, 51, 153, 255)) in pixels[width * 26 * 4:])
        with doc_ex.open(rewrite_epub(book, {"OEBPS/Fonts/type.ttf": b"invalid"})) as broken:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                broken.render_page(0, dpi=96)

    def test_svg_resource_bases_survive_reflow_preview_and_conversion(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" width="25" height="20">
            <style>.picture { opacity: 1 }</style>
            <image class="picture" href="tile.png" width="25" height="20"/>
          </svg>"""
        chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><body>
            <p>Two swatches {svg}<img src="graphics/tile.svg" alt="Blue"/></p></body></html>"""
        data = rewrite_epub(epub_fixture(chapter_one=chapter), {
            "OEBPS/graphics/tile.svg": svg,
            "OEBPS/tile.png": solid((215, 40, 26)),
            "OEBPS/graphics/tile.png": solid((27, 54, 223)),
        })

        def assert_placements(preview: bytes) -> None:
            _, pixels = rgba_pixels(png_chunks(preview))
            self.assertTrue(bytes((215, 40, 26, 255)) in pixels, "red inline raster missing")
            self.assertTrue(bytes((27, 54, 223, 255)) in pixels, "blue linked raster missing")

        with doc_ex.open(data) as book:
            self.assertEqual(len(book[0].get_images()), 2)
            assert_placements(book.render_page(0, dpi=96))
            for kind in ("docx", "pdf"):
                with self.subTest(kind=kind), doc_ex.open(book.convert(kind), filetype=kind) as converted:
                    self.assertIn("Two swatches", converted[0].get_text())
                    self.assertEqual(len(converted[0].get_images()), 2)
                    assert_placements(converted.render_page(0, dpi=96))

    def test_svg_resource_profile_rejects_external_uris_and_bounds_placements(self) -> None:
        def book_with_svg(body: str, prolog: str = "") -> bytes:
            chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><body>
                <p><img src="tile.svg" alt="Vector image"/></p></body></html>"""
            svg = f'{prolog}<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 25 20">{body}</svg>'
            return rewrite_epub(epub_fixture(chapter_one=chapter), {
                "OEBPS/tile.svg": svg,
                "OEBPS/tile.png": solid((215, 40, 26)),
            })

        for href in ("https://example.com/tile.png", "//example.com/tile.png", "file:///tile.png"):
            with self.subTest(href=href), self.assertRaises(doc_ex.UnsupportedFormatError):
                doc_ex.open(book_with_svg(f'<image href="{href}"/>'))
        for href in ("../../tile.png", "%2E%2E/tile.png", "/tile.png"):
            with self.subTest(href=href), self.assertRaises(doc_ex.UnsafeArchiveError):
                doc_ex.open(book_with_svg(f'<image href="{href}"/>'))
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            doc_ex.open(book_with_svg("", '<?xml-stylesheet href="https://example.com/style.css"?>'))
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            doc_ex.open(book_with_svg("", '<?custom href="style.css"?>'))
        with doc_ex.open(book_with_svg('<rect width="20" height="20" style="fill:url(https://example.com/paint)"/>')) as book:
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                book.render_page(0, dpi=96)
        with doc_ex.open(book_with_svg('<image href="tile.png" width="10" height="10"/>' * 257)) as book:
            with self.assertRaises(doc_ex.UnsafeArchiveError):
                book.render_page(0, dpi=96)

    def test_fixed_svg_chapter_renders_viewbox_and_clips(self) -> None:
        source = epub_fixture()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(
            b'href="one.xhtml" media-type="application/xhtml+xml"',
            b'href="cover.svg" media-type="image/svg+xml"',
        ).replace(b'<itemref idref="one"/>', b'<itemref idref="one" properties="rendition:layout-pre-paginated"/>')
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/cover.svg": '<svg xmlns="http://www.w3.org/2000/svg" viewBox="30 25 160 90"><rect x="50" y="35" width="100" height="50" fill="#55bb77"/></svg>',
        })
        with doc_ex.open(source) as document:
            self.assertEqual(document[0].metadata["viewport"], [120, 67.5])
            rendered = png_chunks(document.render_page(0, dpi=96))
            self.assertEqual(struct.unpack_from(">II", rendered[b"IHDR"]), (160, 90))
            w, pixels = rgba_pixels(rendered)
            self.assertEqual(pixels[:4], bytes((255, 255, 255, 255)))
            offset = (20 * w + 25) * 4
            self.assertEqual(pixels[offset:offset + 4], bytes((85, 187, 119, 255)))
            cropped = png_chunks(document.render_page(0, dpi=96, clip=(18.75, 15.0, 15.0, 15.0)))
            self.assertEqual(struct.unpack_from(">II", cropped[b"IHDR"]), (20, 20))
            self.assertEqual(rgba_pixels(cropped)[1][:4], bytes((85, 187, 119, 255)))

    def test_inert_xhtml_doctype_survives_extraction_render_and_edit(self) -> None:
        book = epub_fixture(chapter_one="""<?xml version="1.0"?>
        <!-- ordinary XHTML doctype -->
        <!DOCTYPE html>
        <html xmlns="http://www.w3.org/1999/xhtml"><body><p>Text &amp; symbols.</p></body></html>""")
        with doc_ex.open(book) as original:
            self.assertEqual(original[0].get_text(), "Text & symbols.")
            self.assertTrue(original.render_page(0, dpi=72).startswith(b"\x89PNG"))
            revised = original.edit_epub_structure([1, 0])
        with doc_ex.open(revised) as edited:
            self.assertEqual(edited[1].get_text(), "Text & symbols.")
            self.assertIn(b"<!DOCTYPE html>", edited.read_epub_resource("OEBPS/one.xhtml"))
        bad = (
            '<!DOCTYPE html SYSTEM "custom.dtd">',
            '<!DOCTYPE html [<!ENTITY local "custom">]>',
            '<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">',
        )
        for doctype in bad:
            with self.subTest(doctype=doctype), self.assertRaises(doc_ex.MalformedDocumentError):
                doc_ex.open(epub_fixture(chapter_one=doctype + '<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Text</p></body></html>'))

    def test_media_overlay_playback_clips_pcm_and_synchronizes_callbacks(self) -> None:
        source, _ = narrated_epub()
        with doc_ex.open(source) as document:
            first, aside = list(document.epub_narration(0))
            self.assertIsNotNone(first.audio)
            assert first.audio is not None
            self.assertEqual((first.audio.sample_rate, first.audio.channels, first.audio.frame_count), (8000, 1, 7000))
            self.assertEqual((first.fragment, first.roles), ("café", ("chapter",)))
            self.assertEqual((aside.text, aside.roles), ("Text cue", ("chapter", "aside")))
            with wave.open(io.BytesIO(first.audio.wave), "rb") as pcm:
                self.assertEqual((pcm.getnframes(), pcm.getsampwidth()), (7000, 2))
                self.assertEqual(pcm.readframes(7000), b"".join(
                    struct.pack("<h", ((index % 256) - 128) * 256) for index in range(2000, 9000)
                ))
            events: list[tuple[str, object]] = []
            document.play_epub_narration(
                0,
                play_audio=lambda audio: events.append(("audio", audio.frame_count)),
                speak_text=lambda text: events.append(("speech", text)),
                on_cue=lambda cue: events.append(("target", cue and cue.fragment)),
            )
            self.assertEqual(events, [
                ("target", "café"), ("audio", 7000), ("target", None),
                ("target", "aside"), ("speech", "Text cue"), ("target", None),
            ])
            self.assertEqual(len(list(document.epub_narration(0, skip_roles={"aside"}))), 1)
            self.assertEqual(list(document.epub_narration(0, skip_roles={"chapter"})), [])
            [tail] = document.epub_narration(-1)
            assert tail.audio is not None
            self.assertEqual(tail.audio.frame_count, 16384 - 9000)

    def test_media_overlay_decodes_mpeg_and_reuses_track_for_overlapping_clips(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/mpeg")
            smil = archive.read("OEBPS/Sync/read.smil")
        smil = smil.replace(
            b'<text src="../one.xhtml#aside"/>',
            b'<text src="../one.xhtml#aside"/><audio src="../Media/voice.wav" clipBegin="500ms" clipEnd="1.5s"/>',
        )
        # A locally synthesized 500 Hz tone, 8000 samples/sec and one channel.
        mp3 = (Path(__file__).parent / "fixtures" / "epub-narration-tone.mp3").read_bytes()
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/Media/voice.wav": mp3,
            "OEBPS/Sync/read.smil": smil,
        })
        with doc_ex.open(source) as document:
            a, b = [cue.audio for cue in document.epub_narration(0)]
            assert a is not None and b is not None
            self.assertEqual((a.sample_rate, a.channels, a.frame_count, b.frame_count), (8000, 1, 7000, 8000))
            with wave.open(io.BytesIO(a.wave), "rb") as pcm:
                samples = pcm.readframes(7000)
            with wave.open(io.BytesIO(b.wave), "rb") as pcm:
                self.assertEqual(pcm.readframes(5000), samples[4000:])
            values = struct.unpack("<7000h", samples)
            self.assertGreater(max(values), 1000)
            crossings = sum(left <= 0 < right for left, right in zip(values, values[1:]))
            self.assertAlmostEqual(crossings, 438, delta=2)

    def test_media_overlay_decodes_aac_on_mp4_presentation_timeline(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/mp4")
        # The synthetic track has a 1024-sample AAC priming edit and runs for 2 seconds.
        track = (Path(__file__).parent / "fixtures" / "epub-narration-tone.m4a").read_bytes()
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/Media/voice.wav": track,
        })
        with doc_ex.open(source) as document:
            intro = next(document.epub_narration(0)).audio
            tail = next(document.epub_narration(1)).audio
            assert intro is not None and tail is not None
            self.assertEqual((intro.sample_rate, intro.channels, intro.frame_count), (8000, 1, 7000))
            self.assertEqual(tail.frame_count, 7000)
            with wave.open(io.BytesIO(tail.wave), "rb") as pcm:
                values = struct.unpack("<7000h", pcm.readframes(7000))
                self.assertGreater(max(values), 1000)
                crossings = sum(left <= 0 < right for left, right in zip(values, values[1:]))
                self.assertAlmostEqual(crossings, 438, delta=2)

    def test_media_overlay_playback_errors_and_clears_active_cue(self) -> None:
        source, _ = narrated_epub(first_clip_end="4s")
        with doc_ex.open(source) as document, self.assertRaises(doc_ex.MalformedDocumentError):
            next(document.epub_narration(0))
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/flac")
        with doc_ex.open(rewrite_epub(source, {"OEBPS/content.opf": package})) as document:
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                list(document.epub_narration(0))
        cues: list[doc_ex.EpubNarrationCue | None] = []

        def fail(audio: doc_ex.EpubAudioClip) -> None:
            raise RuntimeError("could not play audio")

        with doc_ex.open(source) as document:
            with self.assertRaisesRegex(RuntimeError, "could not play audio"):
                document.play_epub_narration(0, play_audio=fail, speak_text=lambda _: None, on_cue=cues.append)
        self.assertEqual([cue and cue.fragment for cue in cues], ["café", None])

    def test_media_overlay_resolves_cues_and_audio_in_spine_order(self) -> None:
        data, audio = narrated_epub()
        with doc_ex.open(data) as document:
            overlays = document.metadata["media_overlays"]
            self.assertEqual(list(overlays), ["sync"])
            self.assertEqual(overlays["sync"]["body"]["children"][0]["epub_type"], "chapter")
            first, pause = document.epub_media_cues(0)
            self.assertEqual(first["text"], {"path": "OEBPS/one.xhtml", "fragment": "café"})
            self.assertEqual(first["audio"]["clip_begin_us"], 250_000)
            self.assertEqual(first["audio"]["clip_end_us"], 1_125_000)
            self.assertIsNone(pause["audio"])
            self.assertEqual(pause["epub_type"], "aside")
            next_cue, = document.epub_media_cues(-1)
            self.assertEqual(next_cue["id"], "next")
            self.assertIsNone(next_cue["audio"]["clip_end_us"])
            voice = next_cue["audio"]["path"]
            self.assertEqual(document.read_epub_resource(voice), audio)
            self.assertEqual(document.read_epub_resource(voice, offset=29, length=90), audio[29:119])
            self.assertEqual(document.read_epub_resource(voice, offset=len(audio) - 5), audio[-5:])
            self.assertEqual(document.read_epub_resource(voice, offset=len(audio), length=1), b"")
            with self.assertRaises(ValueError):
                document.read_epub_resource(voice, offset=len(audio) + 1)
            expected = document.epub_media_cues(0)
            overlay_path = overlays["sync"]["path"]
            smil = document.read_epub_resource(overlay_path)
        https = rewrite_epub(data, {overlay_path: smil.replace(b"http://www.w3.org/ns/SMIL", b"https://www.w3.org/ns/SMIL")})
        with doc_ex.open(https) as document:
            self.assertEqual(document.epub_media_cues(0), expected)

    def test_mathml_preview_and_exports_retain_equation_structure(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML"><body>
        <p style="font-size: 16px">Before <m:math><m:mfrac><m:msup><m:mi>x</m:mi><m:mn>2</m:mn></m:msup><m:msqrt><m:mi>q</m:mi></m:msqrt></m:mfrac></m:math> After</p>
        <p><m:math display="block"><m:mfenced><m:mtable>
        <m:mtr><m:mtd><m:mn>1</m:mn></m:mtd><m:mtd><m:mn>2</m:mn></m:mtd></m:mtr>
        <m:mtr><m:mtd><m:mn>3</m:mn></m:mtd><m:mtd><m:mn>4</m:mn></m:mtd></m:mtr>
        </m:mtable></m:mfenced></m:math></p></body></html>"""
        plain = '<html xmlns="http://www.w3.org/1999/xhtml"><body><p style="font-size: 16px">Before x2q After</p><p>1234</p></body></html>'
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as document, doc_ex.open(epub_fixture(chapter_one=plain)) as flat:
            preview = document.render_page(0, dpi=72)
            _, height = struct.unpack_from(">II", preview, 16)
            _, flat_height = struct.unpack_from(">II", flat.render_page(0, dpi=72), 16)
            self.assertTrue(preview.startswith(b"\x89PNG"))
            self.assertGreater(height, flat_height)
            docx = document.convert("docx")
            with zipfile.ZipFile(io.BytesIO(docx)) as archive:
                word = archive.read("word/document.xml")
                for tag in (b"<m:f>", b"<m:sSup>", b"<m:rad>", b"<m:m>", b"<m:oMathPara>"):
                    self.assertIn(tag, word)
            with doc_ex.open(docx) as converted:
                self.assertIn("Before", converted[0].get_text())
                self.assertIn("After", converted[0].get_text())
            with doc_ex.open(document.convert("pdf")) as converted:
                self.assertIn("Before", converted[0].get_text())
                self.assertIn("After", converted[0].get_text())

    def test_invalid_media_overlay_references_and_timing_are_explicit(self) -> None:
        for options, error in (
            ({"first_target": "../one.xhtml#missing"}, doc_ex.MalformedDocumentError),
            ({"first_target": "https://example.com/foreign.xhtml"}, doc_ex.UnsupportedFormatError),
            ({"first_clip_end": "100ms"}, doc_ex.MalformedDocumentError),
            ({"first_clip_end": "1e2s"}, doc_ex.MalformedDocumentError),
        ):
            with self.subTest(options=options), self.assertRaises(error):
                doc_ex.open(narrated_epub(**options)[0])

    def test_batch_chapter_replacement_preserves_package_and_overlay(self) -> None:
        data, _ = narrated_epub()
        first = '<html xmlns="http://www.w3.org/1999/xhtml"><body id="chapter"><p id="café">Revised narration</p><p id="aside">Short aside</p></body></html>'
        last = '<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Last chapter</p></body></html>'
        with doc_ex.open(data) as original:
            revised = original.edit_epub_chapters({0: first, -1: last})
            self.assertIn("Original narration", original[0].get_text())
            with doc_ex.open(revised) as edited:
                self.assertEqual(edited.page_count, 2)
                self.assertIn("Revised narration", edited[0].get_text())
                self.assertEqual(edited[-1].get_text(), "Last chapter")
                self.assertEqual(edited.epub_media_cues(0), original.epub_media_cues(0))
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                original.edit_epub_chapters({0: first.replace('id="café"', "")})
            with self.assertRaises(doc_ex.MalformedDocumentError):
                original.edit_epub_chapters({0: first, 1: "<broken"})
            with self.assertRaises(IndexError):
                original.edit_epub_chapters({2: last})
            with self.assertRaisesRegex(ValueError, "conflicting replacements"):
                original.edit_epub_chapters({0: first, -2: last})
            self.assertEqual(original.edit_epub_chapters({}), data)
        with zipfile.ZipFile(io.BytesIO(data)) as before, zipfile.ZipFile(io.BytesIO(revised)) as after:
            self.assertEqual(before.namelist(), after.namelist())
            self.assertEqual(after.infolist()[0].compress_type, zipfile.ZIP_STORED)
            for path in set(before.namelist()) - {"mimetype", "OEBPS/one.xhtml", "OEBPS/two.xhtml"}:
                self.assertEqual(after.read(path), before.read(path), path)
        with doc_ex.open(rewrite_epub(data, {"META-INF/signatures.xml": "<signatures/>"})) as signed:
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                signed.edit_epub_chapters({0: first})

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
