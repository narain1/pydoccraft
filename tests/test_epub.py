from __future__ import annotations

import hashlib
import io
import re
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


def font_with_extra_advance(font: bytes) -> bytes:
    # Distinguish two copies of the same face without adding a binary fixture.
    wide = bytearray(font)
    count = struct.unpack_from(">H", wide, 4)[0]
    offsets = {tag: offset for tag, _, offset, _ in
               struct.iter_unpack(">4sIII", wide[12:12 + 16 * count])}
    metrics = struct.unpack_from(">H", wide, offsets[b"hhea"] + 34)[0]
    for i in range(metrics):
        pos = offsets[b"hmtx"] + i * 4
        advance = struct.unpack_from(">H", wide, pos)[0]
        struct.pack_into(">H", wide, pos, min(65535, advance + 300))
    return bytes(wide)


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


def fixed_spread_fixture(
    entries: list[tuple[str, str, int, int]],
    *,
    direction: str = "ltr",
    policy: str = "both",
) -> bytes:
    source = epub_fixture()
    with zipfile.ZipFile(io.BytesIO(source)) as archive:
        package = archive.read("OEBPS/content.opf")
    package = package.replace(
        b"</metadata>",
        (f'<meta property="rendition:layout">pre-paginated</meta>'
         f'<meta property="rendition:spread">{policy}</meta></metadata>').encode(),
    )
    manifest = []
    spine = []
    updates = {}
    for index, (properties, color, width, height) in enumerate(entries):
        path = f"s{index}.xhtml"
        manifest.append(f'<item id="s{index}" href="{path}" media-type="application/xhtml+xml"/>')
        linear = ' linear="no"' if "nonlinear" in properties else ""
        properties = properties.replace("nonlinear", "").strip()
        spine.append(f'<itemref idref="s{index}" properties="{properties}"{linear}/>')
        updates["OEBPS/" + path] = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head>
          <title>Chapter {index + 1}</title>
          <meta name="viewPort" content="width={width}px; height = {height}.0px width=4000, height=4000"/>
          <meta name="viewport" content="width=90,height=90"/>
          <style>html {{ background: {color} }}</style></head><body/></html>"""
    package = package.replace(b"</manifest>", ("".join(manifest) + "</manifest>").encode())
    package = package.replace(
        b'<spine><itemref idref="one"/><itemref idref="two"/></spine>',
        (f'<spine page-progression-direction="{direction}">' + "".join(spine) + "</spine>").encode(),
    )
    return rewrite_epub(source, {**updates, "OEBPS/content.opf": package})


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

    def test_spine_properties_change_placement_without_rewriting_navigation(self) -> None:
        source = fixed_spread_fixture([
            ("page-spread-left rendition:page-spread-left rendition:orientation-landscape", "#c04020", 72, 96),
            ("page-spread-right", "#1040a0", 80, 96),
            ("", "#508030", 90, 96),
        ])
        package_path = "OEBPS/content.opf"
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read(package_path)
        package = package.replace(
            b'<itemref idref="s0"', b'<itemref id="kept-spine-id" idref="s0"', 1,
        )
        package = package.replace(
            b"</manifest>",
            b'<item id="nav" href="Nav.xhtml" media-type="application/xhtml+xml" properties="nav"/></manifest>',
        )
        navigation = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
          <head><title>Contents</title></head><body><nav epub:type="toc"><ol>
            <li><a href="s0.xhtml">First</a></li><li><a href="s1.xhtml">Second</a></li>
            <li><a href="s2.xhtml">Third</a></li></ol></nav></body></html>"""
        source = rewrite_epub(source, {package_path: package, "OEBPS/Nav.xhtml": navigation})
        with doc_ex.open(source) as book:
            self.assertEqual(struct.unpack_from(">II", png_chunks(book.render_epub_spread(0, dpi=96))[b"IHDR"]),
                             (152, 96))
            nav = book.metadata["navigation"]
            edited = book.edit_epub_structure([0, 1, 2], spine_properties={
                0: doc_ex.EpubSpineProperties(page_spread="center", spread="both"),
                -1: doc_ex.EpubSpineProperties(linear=False),
            })
            self.assertTrue(book[2].metadata["linear"])
        with doc_ex.open(edited) as book:
            self.assertEqual(book.metadata["navigation"], nav)
            self.assertEqual(book[0].metadata["page_spread"], "center")
            self.assertEqual(book[0].metadata["rendition_spread"], "both")
            self.assertFalse(book[2].metadata["linear"])
            self.assertEqual(struct.unpack_from(">II", png_chunks(book.render_epub_spread(0, dpi=96))[b"IHDR"]),
                             (72, 96))
            self.assertEqual(edited, book.edit_epub_structure(spine_properties={
                0: doc_ex.EpubSpineProperties(page_spread="center", spread="both"),
            }))
            inherited = book.edit_epub_structure(spine_properties={
                0: doc_ex.EpubSpineProperties(layout="inherit", page_spread="inherit", spread="inherit"),
                -1: doc_ex.EpubSpineProperties(linear=True),
            })
        with zipfile.ZipFile(io.BytesIO(source)) as before, zipfile.ZipFile(io.BytesIO(edited)) as after:
            for path in set(before.namelist()) - {package_path}:
                self.assertEqual(before.read(path), after.read(path), path)
            raw = after.read(package_path)
            self.assertIn(b'id="kept-spine-id"', raw)
            self.assertIn(b"rendition:orientation-landscape", raw)
            self.assertNotIn(b"page-spread-left", raw)
        with doc_ex.open(inherited) as book:
            self.assertTrue(book[0].metadata["fixed_layout"])
            self.assertNotIn("page_spread", book[0].metadata)
            self.assertNotIn("rendition_spread", book[0].metadata)
            self.assertTrue(book[2].metadata["linear"])

    def test_spine_layout_and_reordering_validate_staged_content(self) -> None:
        source = epub_fixture()
        xhtml = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Changed</title>{}</head>
          <body><p>Changed chapter</p></body></html>'''
        with doc_ex.open(source) as book:
            fixed = book.edit_epub_structure([1, 0], replacements={
                0: xhtml.format('<meta name="viewport" content="width=120,height=80"/>'),
            }, spine_properties={
                -2: doc_ex.EpubSpineProperties(layout="pre-paginated", spread="none"),
            })
        with doc_ex.open(fixed) as book:
            self.assertEqual(book[0].metadata["chapter_path"], "OEBPS/two.xhtml")
            self.assertEqual(book[1].metadata["viewport"], [90, 60])
            pdf = book.export_epub_fixed_pdf(pages=[1], dpi=96)
            with doc_ex.open(pdf) as rendered:
                self.assertEqual([rendered[0].metadata[key] for key in ("width", "height")], [90, 60])
            reflowable = book.edit_epub_structure(replacements={-1: xhtml.format("")}, spine_properties={
                -1: doc_ex.EpubSpineProperties(layout="inherit", spread="inherit"),
            })
        with doc_ex.open(reflowable) as book:
            self.assertNotIn("fixed_layout", book[1].metadata)
            self.assertNotIn("viewport", book[1].metadata)
            self.assertEqual(book[1].get_text(), "Changed chapter")
            with doc_ex.open(book.convert("docx")) as result:
                self.assertIn("Changed chapter", result[-1].get_text())

        repeated = fixed_spread_fixture([("", "red", 72, 96)])
        with zipfile.ZipFile(io.BytesIO(repeated)) as archive:
            package = archive.read("OEBPS/content.opf")
        repeated = rewrite_epub(repeated, {"OEBPS/content.opf": package.replace(
            b"</spine>", b'<itemref idref="s0"/></spine>',
        )})
        reflow = doc_ex.EpubSpineProperties(layout="reflowable")
        with doc_ex.open(repeated) as book:
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "requires a numeric viewport"):
                book.edit_epub_structure(replacements={0: xhtml.format("")}, spine_properties={0: reflow})
            revised = book.edit_epub_structure(replacements={0: xhtml.format("")},
                                               spine_properties={0: reflow, 1: reflow})
        with doc_ex.open(revised) as book:
            self.assertEqual([page.get_text() for page in book], ["Changed chapter"] * 2)
            self.assertTrue(all("fixed_layout" not in page.metadata for page in book))

    def test_spine_property_validation_is_atomic_and_epub2_keeps_linear_editing(self) -> None:
        properties = doc_ex.EpubSpineProperties
        source = epub_fixture()
        with doc_ex.open(source) as book:
            with self.assertRaisesRegex(ValueError, "numeric viewport"):
                book.edit_epub_structure(spine_properties={0: properties(layout="pre-paginated")})
            with self.assertRaisesRegex(ValueError, "omitted chapter"):
                book.edit_epub_structure([1], spine_properties={0: properties(linear=False)})
            with self.assertRaisesRegex(ValueError, "one linear chapter"):
                book.edit_epub_structure(spine_properties={
                    0: properties(linear=False), 1: properties(linear=False),
                })
            with self.assertRaisesRegex(ValueError, "duplicate EPUB spine property edit"):
                book.edit_epub_structure(spine_properties={
                    0: properties(page_spread="left"), -2: properties(page_spread="right"),
                })
            with self.assertRaises(IndexError):
                book.edit_epub_structure(spine_properties={2: properties()})
            with self.assertRaisesRegex(TypeError, "EpubSpineProperties"):
                book.edit_epub_structure(spine_properties={0: "invalid"})
            self.assertTrue(book[0].metadata["linear"])
            self.assertEqual(book.edit_epub_structure(), source)
        with doc_ex.open(epub_fixture(version="2.0")) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "requires EPUB 3"):
                book.edit_epub_structure(spine_properties={0: properties(page_spread="center")})
            edited = book.edit_epub_structure(spine_properties={0: properties(linear=False)})
        with doc_ex.open(edited) as book:
            self.assertFalse(book[0].metadata["linear"])
            self.assertTrue(book[1].metadata["linear"])

    def test_new_auxiliary_chapter_requires_reachable_staged_links(self) -> None:
        source = epub_fixture()
        prop = doc_ex.EpubSpineProperties(linear=False)
        html = '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head><body>{}</body></html>'
        apply = {1: prop}
        with doc_ex.open(source) as book:
            with self.assertRaisesRegex(ValueError, 'two.xhtml.*no reachable local link'):
                book.edit_epub_structure(spine_properties=apply)
            first = html.format('<a href="two.xhtml#caf%C3%A9">Show the note</a>')
            with self.assertRaisesRegex(ValueError, 'two.xhtml.*no reachable local link'):
                book.edit_epub_structure(replacements={0: first}, spine_properties=apply)
            second = html.format('<p xml:id="café">The note</p>')
            revised = book.edit_epub_structure(
                replacements={0: first, 1: second}, spine_properties=apply,
            )
            self.assertTrue(book[1].metadata["linear"])
            self.assertEqual(book.edit_epub_structure(), source)
        with doc_ex.open(revised) as book:
            self.assertFalse(book[1].metadata["linear"])
            self.assertEqual(book[0].get_links()[0].metadata["target_fragment"], "café")
            self.assertEqual(book[1].get_text(), "The note")

        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        fallback = package.replace(
            b"</manifest>",
            b'<item id="foreign" href="foreign.bin" media-type="application/vnd.example.chapter" fallback="two"/></manifest>',
        ).replace(b'<itemref idref="two"/>', b'<itemref idref="foreign"/>')
        with doc_ex.open(rewrite_epub(source, {
            "OEBPS/content.opf": fallback,
            "OEBPS/foreign.bin": b"Foreign source",
            "OEBPS/one.xhtml": html.format('<a href="foreign.bin">Open fallback chapter</a>'),
        })) as book:
            revised = book.edit_epub_structure(spine_properties=apply)
        with doc_ex.open(revised) as book:
            self.assertFalse(book[1].metadata["linear"])
            self.assertEqual(book[1].metadata["chapter_path"], "OEBPS/two.xhtml")

        guided = package.replace(
            b"</package>", b'<guide><reference type="notes" href="two.xhtml"/></guide></package>',
        )
        with doc_ex.open(rewrite_epub(source, {"OEBPS/content.opf": guided})) as book:
            revised = book.edit_epub_structure(spine_properties=apply)
        with doc_ex.open(revised) as book:
            self.assertFalse(book[1].metadata["linear"])

    def test_auxiliary_access_resolves_transitive_links_and_frame_sources(self) -> None:
        source = epub_fixture()
        package_path = "OEBPS/content.opf"
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read(package_path)
        package = package.replace(
            b"</manifest>",
            b'<item id="three" href="three.xhtml" media-type="application/xhtml+xml"/></manifest>',
        ).replace(b"</spine>", b'<itemref idref="three"/></spine>')
        html = '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title></head><body>{}</body></html>'
        source = rewrite_epub(source, {
            package_path: package,
            "OEBPS/two.xhtml": html.format('<a href="three.xhtml#note">Next</a>'),
            "OEBPS/three.xhtml": html.format('<p id="note">Note</p><a href="two.xhtml">Back</a>'),
        })
        prop = doc_ex.EpubSpineProperties(linear=False)
        with doc_ex.open(source) as book:
            with self.assertRaisesRegex(ValueError, "no reachable local link"):
                book.edit_epub_structure(spine_properties={1: prop, 2: prop})
            with self.assertRaisesRegex(ValueError, "no reachable local link"):
                book.edit_epub_structure(
                    replacements={
                        0: html.format('<iframe src="two.xhtml" title="Embedded"/>'),
                        2: html.format('<p id="note">Note</p>'),
                    },
                    spine_properties={1: prop, 2: prop},
                )
            revised = book.edit_epub_structure(
                replacements={0: html.format('<iframe src="Index/frame.xhtml" title="Index"/>')},
                resources={"OEBPS/Index/frame.xhtml": doc_ex.EpubResource(
                    "application/xhtml+xml", html.format('<a href="../two.xhtml">View the notes</a>').encode(),
                )},
                spine_properties={1: prop, 2: prop},
            )
        with doc_ex.open(revised) as book:
            self.assertEqual([page.metadata["linear"] for page in book], [True, False, False])
            self.assertEqual(book[2].get_links()[0].metadata["target_path"], "OEBPS/two.xhtml")

    def test_resource_removal_checks_the_combined_edited_archive(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
          <link rel="stylesheet" href="Styles/chapter.css"/></head><body><p>Retained chapter</p></body></html>"""
        source = epub_fixture(chapter_one=chapter)
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</manifest>", b"""
          <item id="styles" href="Styles/chapter.css" media-type="text/css"/>
          <item id="print" href="Styles/print.css" media-type="text/css"/>
          <item id="old" href="Art/caf%C3%A9%20leaf.png" media-type="image/png"/>
          <item id="spare" href="Art/spare.png" media-type="image/png"/>
        </manifest>""")
        old = "OEBPS/Art/café leaf.png"
        spare = "OEBPS/Art/spare.png"
        css = b"""@import "print.css" print;
            body { --art: url("../Art/caf%C3%A9%20leaf.png"); background-image: var(--art) }"""
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/Styles/chapter.css": css,
            "OEBPS/Styles/print.css": b"""body { background-image: image-set("../Art/spare.png" 1x); }""",
            old: solid((11, 25, 91)),
            spare: solid((13, 71, 35)),
        })
        with doc_ex.open(source) as book:
            for paths in ((old,), (spare,), ("OEBPS/Styles/print.css",), ("OEBPS/Styles/chapter.css",)):
                with self.subTest(paths=paths), self.assertRaisesRegex(ValueError, "still referenced"):
                    book.edit_epub_structure(remove_resources=paths)
            for path in ("mimetype", "META-INF/container.xml", "OEBPS/content.opf", "OEBPS/absent.png"):
                with self.subTest(path=path), self.assertRaises(ValueError):
                    book.edit_epub_structure(remove_resources=[path])
            with self.assertRaisesRegex(ValueError, "replace and remove"):
                book.edit_epub_structure(
                    resources={spare: doc_ex.EpubResource("image/png", solid((0, 0, 0)))},
                    remove_resources=[spare],
                )
            revised = book.edit_epub_structure(
                resources={"OEBPS/Styles/chapter.css": doc_ex.EpubResource(
                    "text/css", b"""/* url('../Art/spare.png') */ p::after { content: 'Art/spare.png'; }""",
                )},
                remove_resources=[old, spare, "OEBPS/Styles/print.css"],
            )
            self.assertEqual(book.read_epub_resource("OEBPS/Styles/chapter.css"), css)
        with doc_ex.open(revised) as edited, zipfile.ZipFile(io.BytesIO(revised)) as archive:
            self.assertIn("Retained chapter", edited[0].get_text())
            self.assertEqual(archive.read("OEBPS/one.xhtml"), chapter.encode())
            self.assertTrue({old, spare, "OEBPS/Styles/print.css"}.isdisjoint(archive.namelist()))
            self.assertTrue({"old", "spare", "print"}.isdisjoint(item["id"] for item in edited.metadata["manifest"]))
            self.assertTrue(edited.render_page(0, dpi=72).startswith(b"\x89PNG"))

    def test_resource_removal_preserves_references_outside_the_spine(self) -> None:
        source = epub_fixture()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</manifest>", b"""
          <item id="spare" href="Art/retain.png" media-type="image/png"/>
          <item id="vector" href="vector.svg" media-type="image/svg+xml"/>
          <item id="alt" href="Styles/alternate.css" media-type="text/css"/>
          <item id="hidden" href="Art/indirect.png" media-type="image/png"/>
        </manifest>""")
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/Art/retain.png": solid((24, 25, 26)),
            "OEBPS/Styles/alternate.css": '@import "extensionless"; rect { fill: red; }',
            "OEBPS/Styles/extensionless": 'rect { fill: url("../Art/indirect.png") }',
            "OEBPS/Art/indirect.png": solid((27, 28, 29)),
            "OEBPS/vector.svg": """<?xml-stylesheet href="Styles/alternate.css" type="text/css" alternate="yes" title="Alt"?>
              <svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10"/></svg>""",
            "OEBPS/extras.xhtml": """<html xmlns="http://www.w3.org/1999/xhtml"><body>
              <p><img alt="Responsive" srcset="data:image/png;base64,AA== 1x, Art/retain.png 2x"/></p></body></html>""",
        })
        with doc_ex.open(source) as book:
            for path in ("OEBPS/Art/retain.png", "OEBPS/Styles/alternate.css", "OEBPS/Art/indirect.png"):
                with self.subTest(path=path), self.assertRaisesRegex(ValueError, "still referenced"):
                    book.edit_epub_structure(remove_resources=[path])
            with self.assertRaisesRegex(ValueError, "still referenced"):
                book.edit_epub_structure([0], remove_resources=["OEBPS/image.png"])
            edited = book.edit_epub_structure([1], remove_resources=["OEBPS/one.xhtml", "OEBPS/image.png"])
        with doc_ex.open(edited) as book:
            self.assertIn("Second chapter", book[0].get_text())
            self.assertEqual(len(book), 1)

    def test_attach_media_overlay_to_inserted_chapter_and_keep_existing_timing(self) -> None:
        source, old_audio = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b"</metadata>", b"""<meta property="media:duration" refines="#sync">0:00:01.798</meta>
                   <meta property="media:duration">0:00:01.798</meta></metadata>""",
            )
        source = rewrite_epub(source, {"OEBPS/content.opf": package})
        chapter = doc_ex.EpubChapterSource("Added", """<html xmlns="http://www.w3.org/1999/xhtml"><body>
          <p id="café">Audio</p><p id="aside">Speak this</p></body></html>""", "Extra/new café.xhtml")
        track = (Path(__file__).parent / "fixtures" / "epub-narration-tone.mp3").read_bytes()
        cues = [
            doc_ex.EpubOverlayCue("café", "Audio/lecture 1.mp3", 250_000, 750_000, "chapter"),
            doc_ex.EpubOverlayCue("aside", epub_type="aside"),
        ]
        audio = {"Audio/lecture 1.mp3": doc_ex.EpubResource("audio/mpeg", track)}
        with doc_ex.open(source) as original:
            old = original.epub_media_cues(0)
            with self.assertRaises(doc_ex.MalformedDocumentError):
                original.edit_epub_structure([0, chapter, 1], resources=audio, overlays={
                    chapter.path: [doc_ex.EpubOverlayCue("missing")],
                })
            with self.assertRaisesRegex(ValueError, "explicit later clip end"):
                original.edit_epub_structure([0, chapter, 1], resources=audio, overlays={
                    chapter.path: [doc_ex.EpubOverlayCue("café", "Audio/lecture 1.mp3")],
                })
            revised = original.edit_epub_structure([0, chapter, 1], resources=audio, overlays={chapter.path: cues})
            self.assertEqual(original.epub_media_cues(0), old)
            self.assertEqual(len(original), 2)
        meta = "{http://www.idpf.org/2007/opf}meta"
        with doc_ex.open(revised) as edited, zipfile.ZipFile(io.BytesIO(revised)) as archive:
            self.assertEqual(edited.epub_media_cues(0), old)
            self.assertEqual(edited.read_epub_resource("OEBPS/Media/voice.wav"), old_audio)
            overlay_id = edited[1].metadata["media_overlay"]
            smil = edited.metadata["media_overlays"][overlay_id]["path"]
            self.assertIn(b"new%20caf%C3%A9.xhtml#caf%C3%A9", archive.read(smil))
            playback = list(edited.epub_narration(1))
            self.assertEqual([cue.fragment for cue in playback], ["café", "aside"])
            self.assertEqual([cue.roles for cue in playback], [("chapter",), ("aside",)])
            self.assertEqual(playback[0].audio.frame_count, 4000)
            self.assertEqual(playback[1].text, "Speak this")
            xml = ET.fromstring(archive.read("OEBPS/content.opf"))
            durations = {node.attrib.get("refines"): node.text for node in xml.iter(meta)
                         if node.attrib.get("property") == "media:duration"}
            self.assertEqual(durations, {"#sync": "0:00:01.798", "#" + overlay_id: "0:00:00.500000",
                                         None: "0:00:02.298000"})

    def test_author_nested_media_overlays_with_inherited_roles(self) -> None:
        source = epub_fixture(chapter_one="""<html xmlns="http://www.w3.org/1999/xhtml"><body>
        <section id="chapter"><p id="café">Listen</p><aside id="aside"><p id="note">Sidebar</p></aside></section>
        <p id="end">Finish</p></body></html>""")
        audio_path = "OEBPS/Media/voice.wav"
        _, wav = narrated_epub()
        sequence = doc_ex.EpubOverlaySequence([
            doc_ex.EpubOverlayCue("café", audio_path, 10_000, 510_000, "sentence"),
            doc_ex.EpubOverlaySequence([
                doc_ex.EpubOverlayCue("note"),
            ], "aside", "sidebar"),
        ], "chapter", "chapter")
        with doc_ex.open(source) as original:
            revised = original.edit_epub_structure(
                resources={audio_path: doc_ex.EpubResource("audio/wav", wav)},
                overlays={"OEBPS/one.xhtml": [sequence, doc_ex.EpubOverlayCue("end")]},
            )
            self.assertEqual(original.epub_media_cues(0), [])
        with doc_ex.open(revised) as document, zipfile.ZipFile(io.BytesIO(revised)) as archive:
            overlay_id = document[0].metadata["media_overlay"]
            overlay = document.metadata["media_overlays"][overlay_id]
            group, last = overlay["body"]["children"]
            self.assertEqual((group["kind"], group["id"], group["text"]["fragment"]),
                             ("seq", "seq-1", "chapter"))
            inner = group["children"][1]
            self.assertEqual((inner["kind"], inner["id"], inner["text"]["fragment"], last["kind"]),
                             ("seq", "seq-2", "aside", "par"))
            self.assertIn(b"#caf%C3%A9", archive.read(overlay["path"]))
            first, aside, last = document.epub_narration(0)
            self.assertEqual([(cue.id, cue.fragment, cue.roles) for cue in (first, aside, last)], [
                ("cue-1", "café", ("chapter", "sentence")),
                ("cue-2", "note", ("chapter", "sidebar")),
                ("cue-3", "end", ()),
            ])
            assert first.audio is not None
            self.assertEqual(first.audio.frame_count, 4000)
            self.assertEqual([cue.fragment for cue in document.epub_narration(0, skip_roles={"sidebar"})],
                             ["café", "end"])
            opf = ET.fromstring(archive.read("OEBPS/content.opf"))
            durations = {node.get("refines"): node.text for node in opf.iter("{http://www.idpf.org/2007/opf}meta")
                         if node.get("property") == "media:duration"}
            self.assertEqual(durations, {"#" + overlay_id: "0:00:00.500000", None: "0:00:00.500000"})

    def test_authored_overlay_rejects_empty_missing_and_unbounded_sequences(self) -> None:
        path = "OEBPS/one.xhtml"
        cue = doc_ex.EpubOverlayCue()
        children: list[doc_ex.EpubOverlayCue | doc_ex.EpubOverlaySequence] = []
        cycle = doc_ex.EpubOverlaySequence(children)
        children.append(cycle)
        bad_groups = (
            (doc_ex.EpubOverlaySequence([]), ValueError),
            (doc_ex.EpubOverlaySequence([cue], "missing"), doc_ex.MalformedDocumentError),
            (cycle, doc_ex.UnsafeArchiveError),
            (doc_ex.EpubOverlaySequence(cue for _ in range(10_000)), doc_ex.UnsafeArchiveError),
        )
        with doc_ex.open(epub_fixture()) as document:
            for sequence, error in bad_groups:
                with self.subTest(error=error), self.assertRaises(error):
                    document.edit_epub_structure(overlays={path: [sequence]})
            self.assertEqual(document.epub_media_cues(0), [])

    def test_detach_shared_overlay_and_remove_its_package_members_atomically(self) -> None:
        source, audio = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b"</metadata>", b"""<meta property="media:duration" refines="#sync">0:00:01.798</meta>
                   <meta property="media:duration">0:00:01.798</meta></metadata>""",
            )
        source = rewrite_epub(source, {"OEBPS/content.opf": package})
        removed = ["OEBPS/Sync/read.smil", "OEBPS/Media/voice.wav"]
        with doc_ex.open(source) as original:
            with self.assertRaisesRegex(ValueError, "still referenced"):
                original.edit_epub_structure(remove_resources=[removed[0]])
            with self.assertRaisesRegex(ValueError, "still referenced"):
                original.edit_epub_structure(remove_resources=[removed[1]])
            partial = original.edit_epub_structure(overlays={"OEBPS/one.xhtml": None})
            kept = original.epub_media_cues(1)
            revised = original.edit_epub_structure(
                overlays={"OEBPS/one.xhtml": None, "OEBPS/two.xhtml": None},
                remove_resources=removed,
            )
            self.assertEqual(original.read_epub_resource(removed[1]), audio)
        with doc_ex.open(partial) as edited, zipfile.ZipFile(io.BytesIO(partial)) as archive:
            self.assertEqual(edited.epub_media_cues(0), [])
            self.assertEqual(edited.epub_media_cues(1), kept)
            self.assertEqual(edited.read_epub_resource(removed[1]), audio)
            root = ET.fromstring(archive.read("OEBPS/content.opf"))
            durations = {node.get("refines"): node.text for node in root.iter("{http://www.idpf.org/2007/opf}meta")
                         if node.get("property") == "media:duration"}
            self.assertEqual(durations, {"#sync": "0:00:00.923000", None: "0:00:00.923000"})
        with doc_ex.open(revised) as edited, zipfile.ZipFile(io.BytesIO(revised)) as archive:
            self.assertIsNone(edited.metadata.get("media_overlays"))
            self.assertEqual(edited.epub_media_cues(0), [])
            self.assertTrue(set(removed).isdisjoint(archive.namelist()))
            self.assertNotIn(b"media:duration", archive.read("OEBPS/content.opf"))

    def test_replace_chapter_in_shared_overlay_preserves_nested_roles_and_staged_timing(self) -> None:
        source, original_audio = narrated_epub()
        path = "OEBPS/Media/voice.wav"
        retired = "OEBPS/Media/retired.wav"
        smil_path = "OEBPS/Sync/read.smil"
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            smil = archive.read(smil_path)
            package = archive.read("OEBPS/content.opf").replace(
                b"</manifest>", b'<item id="retired" href="Media/retired.wav" media-type="audio/wav"/></manifest>',
            )
        smil = smil.replace(b"../Media/voice.wav", b"../Media/retired.wav", 1)
        smil = smil.replace(b' clipEnd="00:00:01.125"', b"")
        smil = smil.replace(b"<body>", b'<body epub:textref="../one.xhtml#chapter">')
        smil = smil.replace(
            b'</seq><par id="next">',
            b'<seq id="nested" epub:textref="../two.xhtml" epub:type="subchapter"><par id="next">',
        )
        smil = smil.replace(b"</par></body>", b"</par></seq></seq></body>")
        replacement = io.BytesIO()
        with wave.open(replacement, "wb") as pcm:
            pcm.setparams((1, 1, 8000, 0, "NONE", "not compressed"))
            pcm.writeframes(bytes(20000))
        with doc_ex.open(rewrite_epub(source, {
            smil_path: smil, "OEBPS/content.opf": package, retired: original_audio,
        })) as original:
            self.assertEqual(original.epub_media_cues(1)[0]["id"], "next")
            edited = original.edit_epub_structure(
                [1, 0],
                remove_resources=[retired],
                resources={path: doc_ex.EpubResource("audio/wav", replacement.getvalue())},
                overlays={"OEBPS/one.xhtml": [doc_ex.EpubOverlayCue("café", path, 0, 20_000)]},
            )
            self.assertEqual(original.read_epub_resource(smil_path), smil)

        with doc_ex.open(edited) as book, zipfile.ZipFile(io.BytesIO(edited)) as archive:
            self.assertEqual(book[0].metadata["chapter_path"], "OEBPS/two.xhtml")
            self.assertEqual([cue["id"] for cue in book.epub_media_cues(0)], ["next"])
            self.assertIsNone(book.epub_media_cues(0)[0]["audio"]["clip_end_us"])
            self.assertEqual(book.epub_media_cues(1)[0]["text"]["fragment"], "café")
            kept = next(book.epub_narration(0))
            new = next(book.epub_narration(1))
            assert kept.audio is not None and new.audio is not None
            self.assertEqual(kept.audio.frame_count, 11000)
            self.assertEqual(new.audio.frame_count, 160)
            revised = book.metadata["media_overlays"]["sync"]["body"]
            self.assertEqual(revised["text"], {"path": "OEBPS/two.xhtml", "fragment": None})
            parent, = revised["children"]
            nested, = parent["children"]
            self.assertEqual((parent["epub_type"], nested["id"], nested["epub_type"]),
                             ("chapter", "nested", "subchapter"))
            self.assertEqual(parent["text"]["path"], "OEBPS/two.xhtml")
            self.assertEqual(book.read_epub_resource(path), replacement.getvalue())
            self.assertNotIn(retired, archive.namelist())
            root = ET.fromstring(archive.read("OEBPS/content.opf"))
            durations = {node.get("refines"): node.text for node in root.iter("{http://www.idpf.org/2007/opf}meta")
                         if node.get("property") == "media:duration"}
            self.assertEqual(durations.pop("#sync"), "0:00:01.375000")
            self.assertEqual(durations.pop(None), "0:00:01.395000")
            self.assertEqual(list(durations.values()), ["0:00:00.020000"])

        # A chapter can be associated with a shared overlay while having
        # no timed cues in it. A staged audio replacement still changes timing.
        original_smil = smil[0:smil.index(b"<body")] + (
            b"<body>" + smil[smil.index(b'<par id="next"'):smil.index(b"</par></seq></seq></body>")]
            + b"</par></body></smil>"
        )
        declared = package.replace(b"</metadata>", (
            b'<meta property="media:duration" refines="#sync">0:00:00.923</meta>'
            b'<meta property="media:duration">0:00:00.923</meta></metadata>'
        ))
        with doc_ex.open(rewrite_epub(source, {
            smil_path: original_smil, "OEBPS/content.opf": declared, retired: original_audio,
        })) as original:
            edited = original.edit_epub_structure(
                resources={path: doc_ex.EpubResource("audio/wav", replacement.getvalue())},
                overlays={"OEBPS/one.xhtml": None},
            )
        with doc_ex.open(edited) as book, zipfile.ZipFile(io.BytesIO(edited)) as archive:
            self.assertEqual(book.epub_media_cues(0), [])
            self.assertEqual(book.read_epub_resource(smil_path), original_smil)
            root = ET.fromstring(archive.read("OEBPS/content.opf"))
            durations = {node.get("refines"): node.text for node in root.iter("{http://www.idpf.org/2007/opf}meta")
                         if node.get("property") == "media:duration"}
            self.assertEqual(durations, {"#sync": "0:00:01.375000", None: "0:00:01.375000"})

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

    def test_spine_edit_synchronizes_epub3_navigation_and_compatibility_ncx(self) -> None:
        with doc_ex.new("epub") as book:
            book.set_metadata(title="Hybrid navigation", identifier="urn:test:hybrid-nav")
            book.add_chapter("First").add_paragraph("First chapter")
            book.add_chapter("Second").add_paragraph("Second chapter")
            book.add_chapter("Third").add_paragraph("Third chapter")
            source = book.to_bytes()

        chapter = doc_ex.EpubChapterSource(
            "Inserted", '<html xmlns="http://www.w3.org/1999/xhtml"><head>'
            '<title>Inserted</title></head><body><p>New chapter</p></body></html>',
        )
        with doc_ex.open(source) as original:
            old_ncx = original.read_epub_resource("OEBPS/toc.ncx")
            self.assertEqual(original.edit_epub_structure([0, 1, 2]), source)
            result = original.edit_epub_structure([2, chapter, 0])
            self.assertEqual(original.read_epub_resource("OEBPS/toc.ncx"), old_ncx)

        with doc_ex.open(result) as edited:
            html_items = edited.metadata["navigation"][0]["items"]
            self.assertEqual([item["label"] for item in html_items], ["Third", "Inserted", "First"])
            ncx = ET.fromstring(edited.read_epub_resource("OEBPS/toc.ncx"))
            entries = ncx.findall("./{*}navMap/{*}navPoint")
            self.assertEqual([entry.findtext("./{*}navLabel/{*}text") for entry in entries],
                             ["Third", "Inserted", "First"])
            self.assertEqual([entry.attrib["playOrder"] for entry in entries], ["1", "2", "3"])
            self.assertEqual([f'OEBPS/{entry.find("./{*}content").get("src")}' for entry in entries],
                             [page.metadata["chapter_path"] for page in edited])

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

    def test_fixed_spreads_pair_in_ltr_order_with_blank_and_centered_pages(self) -> None:
        data = fixed_spread_fixture([
            ("", "#aa2211", 120, 80),
            ("", "#1155cc", 120, 80),
            ("", "#33aacc", 100, 74),
            ("rendition:page-spread-center", "#ffd411", 180, 90),
            ("page-spread-left", "#119933", 120, 80),
            ("rendition:page-spread-left", "#661133", 120, 80),
            ("rendition:page-spread-right", "#225588", 120, 80),
        ])
        with doc_ex.open(data) as book:
            self.assertEqual(book.metadata["page_progression_direction"], "ltr")
            self.assertEqual(book.metadata["rendition_spread"], "both")
            self.assertEqual(book[0].metadata["viewport"], [90, 60])
            cover = png_chunks(book.render_epub_spread(0, dpi=96))
            self.assertEqual(struct.unpack_from(">II", cover[b"IHDR"]), (240, 80))
            _, pixels = rgba_pixels(cover)
            self.assertEqual(pixels[:4], b"\xff\xff\xff\xff")
            self.assertEqual(pixels[120*4:121*4], bytes((170, 34, 17, 255)))
            paired = book.render_epub_spread(1, dpi=96)
            self.assertEqual(paired, book.render_epub_spread(2, dpi=96))
            self.assertEqual(paired, book.render_epub_spread(1, dpi=96, orientation="portrait"))
            w, pixels = rgba_pixels(png_chunks(paired))
            self.assertEqual(w, 220)
            self.assertEqual(pixels[:4], bytes((17, 85, 204, 255)))
            self.assertEqual(pixels[120*4:121*4], bytes((51, 170, 204, 255)))
            start = (76*w+120)*4
            self.assertEqual(pixels[start:start+4], b"\xff\xff\xff\xff")
            cropped = png_chunks(book.render_epub_spread(1, dpi=96, clip=(85.5, 9.75, 12, 10.5)))
            self.assertEqual(struct.unpack_from(">II", cropped[b"IHDR"]), (16, 14))
            _, clip_pixels = rgba_pixels(cropped)
            expected = b"".join(pixels[(y*w+114)*4:(y*w+130)*4] for y in range(13, 27))
            self.assertEqual(clip_pixels, expected)
            self.assertEqual(book.render_epub_spread(3, dpi=96), book.render_page(3, dpi=96))
            _, solo = rgba_pixels(png_chunks(book.render_epub_spread(4, dpi=96)))
            self.assertEqual(solo[120*4:121*4], b"\xff\xff\xff\xff")
            self.assertEqual(book.render_epub_spread(5, dpi=96), book.render_epub_spread(-1, dpi=96))

    def test_fixed_spreads_apply_rtl_orientation_and_linear_overrides(self) -> None:
        data = fixed_spread_fixture([
            ("page-spread-right", "#550000", 120, 80),
            ("nonlinear", "#ffff00", 120, 80),
            ("rendition:page-spread-left", "#000077", 120, 80),
            ("rendition:spread-none page-spread-right", "#aa33aa", 120, 80),
            ("rendition:spread-both page-spread-right", "#335577", 120, 80),
            ("rendition:spread-both page-spread-left", "#557799", 120, 80),
        ], direction="rtl", policy="landscape")
        with doc_ex.open(data) as book:
            self.assertEqual(book.metadata["page_progression_direction"], "rtl")
            w, pixels = rgba_pixels(png_chunks(book.render_epub_spread(0, dpi=96)))
            self.assertEqual(w, 240)
            self.assertEqual(pixels[:4], bytes((0, 0, 119, 255)))
            self.assertEqual(pixels[120*4:121*4], bytes((85, 0, 0, 255)))
            self.assertEqual(book.render_epub_spread(1, dpi=96), book.render_page(1, dpi=96))
            self.assertEqual(book.render_epub_spread(0, dpi=96, orientation="portrait"), book.render_page(0, dpi=96))
            self.assertEqual(book.render_epub_spread(3, dpi=96), book.render_page(3, dpi=96))
            self.assertEqual(book.render_epub_spread(4, dpi=96, orientation="portrait"), book.render_epub_spread(5, dpi=96))
            with self.assertRaises(ValueError):
                book.render_epub_spread(0, orientation="frames")
            with self.assertRaises(IndexError):
                book.render_epub_spread(-7)

    def test_spread_clip_can_render_across_gutter_without_full_page_allocations(self) -> None:
        data = fixed_spread_fixture([
            ("page-spread-left", "#aa2211", 6_000, 180),
            ("page-spread-right", "#1155cc", 6_000, 180),
        ])
        with doc_ex.open(data) as book:
            with self.assertRaises(doc_ex.UnsafeArchiveError):
                book.render_epub_spread(1, dpi=600)
            clipped = png_chunks(book.render_epub_spread(1, dpi=600, clip=(4494, 18, 12, 6)))
            self.assertEqual(struct.unpack_from(">II", clipped[b"IHDR"]), (100, 50))
            _, pixels = rgba_pixels(clipped)
            self.assertEqual(pixels[49*4:50*4], bytes((170, 34, 17, 255)))
            self.assertEqual(pixels[50*4:51*4], bytes((17, 85, 204, 255)))

    def test_visual_pdf_preserves_fixed_xhtml_and_svg_viewports_and_selection(self) -> None:
        data = fixed_spread_fixture([
            ("page-spread-left", "#13613c", 160, 100),
            ("page-spread-right", "#cc2300", 110, 86),
            ("rendition:layout-reflowable", "black", 150, 80),
        ])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b'href="s1.xhtml" media-type="application/xhtml+xml"',
                                  b'href="s1.svg" media-type="image/svg+xml"')
        data = rewrite_epub(data, {
            "OEBPS/content.opf": package,
            "OEBPS/s1.svg": """<svg xmlns="http://www.w3.org/2000/svg" viewBox="15 13 110 86">
                 <rect x="15" y="13" width="110" height="86" fill="#cc2300"/></svg>""",
        })
        with doc_ex.open(data) as book:
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                book.export_epub_fixed_pdf()
            with self.assertRaises(ValueError):
                book.export_epub_fixed_pdf(pages=[])
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                book.convert("pdf")
            spread = png_chunks(book.render_epub_spread(0, dpi=96))
            self.assertEqual(struct.unpack_from(">II", spread[b"IHDR"]), (270, 100))
            w, pixels = rgba_pixels(spread)
            self.assertEqual(pixels[159*4:160*4], bytes((19, 97, 60, 255)))
            self.assertEqual(pixels[160*4:161*4], bytes((204, 35, 0, 255)))
            pdf = book.export_epub_fixed_pdf(dpi=96, pages=[-2, 0])
        with doc_ex.open(pdf) as result:
            self.assertEqual(len(result), 2)
            for index, (dimensions, rgb) in enumerate((((110, 86), (204, 35, 0)), ((160, 100), (19, 97, 60)))):
                with self.subTest(index=index):
                    raster = png_chunks(result.render_page(index, dpi=96))
                    self.assertEqual(struct.unpack_from(">II", raster[b"IHDR"]), dimensions)
                    _, pixels = rgba_pixels(raster)
                    offset = (20 * dimensions[0] + 20) * 4
                    self.assertEqual(pixels[offset:offset+4], bytes((*rgb, 255)))
                    self.assertEqual(len(result[index].get_images()), 1)
                    self.assertEqual(result[index].get_text().strip(), "")

    def test_fixed_pdf_preserves_interior_samples_at_fractional_dpi(self) -> None:
        data = fixed_spread_fixture([("", "#125369", 169, 105)] * 2)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s1.xhtml" media-type="application/xhtml+xml"',
                b'href="s1.svg" media-type="image/svg+xml"',
            )
        stripes = "".join(f'<rect x="{4 + i}" y="7" width="1.4" height="105" fill="#04b"/>'
                          for i in range(1, 169, 3))
        stripes += "".join(f'<rect x="4" y="{7 + i}" width="169" height="1.4" fill="#c30" opacity=".4"/>'
                           for i in range(1, 105, 3))
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="4 7 169 105">{stripes}</svg>'
        data = rewrite_epub(data, {"OEBPS/content.opf": package, "OEBPS/s1.svg": svg})
        viewport = (126.75, 78.75)
        nearly_whole = (100.005 * 96) / 169
        with doc_ex.open(data) as book:
            for dpi, dimensions, interior in (
                (47.25, (84, 52), (83, 51)),
                (60, (106, 66), (105, 65)),
                (96, (169, 105), (169, 105)),
                (nearly_whole, (100, 63), (100, 62)),
            ):
                pdf = book.export_epub_fixed_pdf(dpi=dpi, searchable=dpi == 47.25)
                with doc_ex.open(pdf) as result:
                    for index in range(2):
                        with self.subTest(dpi=dpi, page=index):
                            rendered = png_chunks(result.render_page(index, dpi=dpi))
                            preview = png_chunks(book.render_page(index, dpi=dpi))
                            self.assertEqual((result[index].metadata["width"], result[index].metadata["height"]),
                                             viewport)
                            self.assertEqual(struct.unpack_from(">II", preview[b"IHDR"]), dimensions)
                            self.assertEqual(struct.unpack_from(">II", rendered[b"IHDR"]), dimensions)
                            image, = result[index].get_images()
                            self.assertEqual((image.metadata["width"], image.metadata["height"]), dimensions)
                            pixels = rgba_pixels(preview)[1]
                            pdf_pixels = rgba_pixels(rendered)[1]
                            stride = dimensions[0] * 4
                            for row in range(interior[1]):
                                start = row * stride
                                end = start + interior[0] * 4
                                self.assertEqual(pixels[start:end], pdf_pixels[start:end], f"row {row}")

    def test_fixed_layout_pdf_preserves_nested_navigation_after_page_selection(self) -> None:
        source = fixed_spread_fixture([("", "white", 160, 100)] * 3)
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b'href="s1.xhtml" media-type="application/xhtml+xml"',
                                  b'href="s1.svg" media-type="image/svg+xml"')
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
        <title>First</title><meta name="viewport" content="width=160,height=100"/>
        <style>body { margin: 0 } h1 { position: absolute; left: 16px; top: 32px;
        margin: 0; color: #124578; font-size: 18px }</style>
        </head><body><h1 id="café">Chapter</h1></body></html>"""
        svg = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 20 160 100">
        <g transform="translate(10 20)"><rect id="plate" x="24" y="16" width="40"
        height="32" fill="#bb1256"/></g></svg>"""
        navigation = """<html xmlns="http://www.w3.org/1999/xhtml"
            xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head>
            <body><nav epub:type="toc landmarks"><ol>
              <li><span>Part one</span><ol>
                <li><a href="s0.xhtml#caf%C3%A9">Café</a></li>
                <li><a href="s2.xhtml">Omitted</a></li>
                <li><a href="s1.svg#plate">Plate</a></li>
              </ol></li>
              <li><a href="s1.svg">Whole plate</a></li>
            </ol></nav></body></html>"""
        ncx_items = """<navPoint id="part"><navLabel><text>Part one</text></navLabel>
            <content src="s0.xhtml#caf%C3%A9"/>
            <navPoint id="cafe"><navLabel><text>Café</text></navLabel><content src="s0.xhtml#caf%C3%A9"/></navPoint>
            <navPoint id="omit"><navLabel><text>Omitted</text></navLabel><content src="s2.xhtml"/></navPoint>
            <navPoint id="plate"><navLabel><text>Plate</text></navLabel><content src="s1.svg#plate"/></navPoint>
            </navPoint>
            <navPoint id="whole"><navLabel><text>Whole plate</text></navLabel><content src="s1.svg"/></navPoint>"""
        for version in ("2", "3"):
            with self.subTest(version=version):
                if version == "3":
                    opf = package.replace(
                        b"</manifest>",
                        b'<item id="nav" href="toc.xhtml" properties="nav" media-type="application/xhtml+xml"/></manifest>',
                    )
                    files = {"OEBPS/toc.xhtml": navigation}
                else:
                    opf = package.replace(b'version="3.0"', b'version="2.0"').replace(
                        b'<spine page-progression-direction="ltr">', b'<spine toc="ncx">',
                    ).replace(
                        b"</manifest>", b'<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest>',
                    )
                    files = {"OEBPS/toc.ncx": f'<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>{ncx_items}</navMap></ncx>'}
                data = rewrite_epub(source, {
                    **files, "OEBPS/content.opf": opf, "OEBPS/s0.xhtml": xhtml, "OEBPS/s1.svg": svg,
                })
                with doc_ex.open(data) as book:
                    selection = [1, 0, 1]
                    pdf = book.export_epub_fixed_pdf(dpi=96, pages=selection)
                    previews = [rgba_pixels(png_chunks(book.render_page(index, dpi=96))) for index in selection]
                    omitted_pdf = book.export_epub_fixed_pdf(dpi=96, pages=[1])
                with doc_ex.open(pdf) as result, result.edit() as editor:
                    bookmarks = editor.bookmarks
                    self.assertEqual([(entry.title, entry.page) for entry in bookmarks],
                                     [("Part one", 1), ("Whole plate", 0)])
                    self.assertEqual([(entry.title, entry.page) for entry in bookmarks[0].children],
                                     [("Café", 1), ("Plate", 0)])
                    for index, preview in enumerate(previews):
                        self.assertEqual(rgba_pixels(png_chunks(result.render_page(index, dpi=96))), preview)
                    readable_pdf = editor.to_bytes(object_streams=False)
                with doc_ex.open(omitted_pdf) as result, result.edit() as editor:
                    self.assertEqual([(entry.title, entry.page) for entry in editor.bookmarks],
                                     [("Part one", 0), ("Whole plate", 0)])
                    self.assertEqual([(entry.title, entry.page) for entry in editor.bookmarks[0].children],
                                     [("Plate", 0)])
                self.assertRegex(readable_pdf, rb'/XYZ\s+12(?:\.0+)?\s+51(?:\.0+)?\s+null')
                self.assertRegex(readable_pdf, rb'/XYZ\s+18(?:\.0+)?\s+63(?:\.0+)?\s+null')

    def test_logical_css_box_cascade_matches_physical_layout_in_ltr_and_rtl(self) -> None:
        markup = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Logical boxes</title>
          <meta name="viewport" content="width=240,height=150"/><style>
            html,body{margin:0} #panel{position:absolute;left:112px;top:17px;width:110px;height:100px;
                                        background:#eaead1;font-size:12px}
            #first,#child{position:absolute;display:block;background:#c94b39}
            #child{direction:rtl;background:#215caa;font-size:7px}
            #target{position:absolute;left:12px;top:120px;font-size:13px}
            STYLES
          </style></head><body>
            <a id="first" href="#target">First</a>
            <div id="panel"><a id="child" href="#target">Next</a></div>
            <p id="target">Visible target</p>
          </body></html>"""
        logical = """
            #first { left:15px!important; inset-inline-start:16px!important;
                     top:15px; inset-block-start:13px; inline-size:66px; width:58px;
                     min-inline-size:61px; block-size:30px; max-block-size:24px;
                     --space: 7px 3px; padding-inline:var(--space); padding-block:4px 2px;
                     border:1px solid #474747; border-block-start:2px solid rgb(252, 208, 17);
                     border-inline-end-color:#9444b4 }
            #panel {padding-inline-start:2em}
            #child { inset-inline-start:14px; inset-block:16px auto;
                     inline-size:28px; block-size:24px; padding-inline-start:inherit;
                     padding-inline-end:4px; margin-inline-start:5px; margin-inline-end:2px;
                     border-inline:solid 2px #272952;
                     border-inline-color:rgb(252, 208, 17) #9444b4 }
        """
        physical = """
            #first { left:16px;top:13px;width:58px;min-width:61px;height:30px;max-height:24px;
                     padding:4px 3px 2px 7px;border:1px solid #474747;
                     border-top:2px solid rgb(252, 208, 17);border-right-color:#9444b4 }
            #panel {padding-left:24px}
            #child {right:14px;top:16px;width:28px;height:24px;padding-right:24px;padding-left:4px;
                     margin-right:5px;margin-left:2px;border-right:solid 2px rgb(252, 208, 17);
                     border-left:solid 2px #9444b4}
        """
        source = fixed_spread_fixture([("", "white", 240, 150)])
        results = []
        for css in (logical, physical):
            with doc_ex.open(source) as book:
                revised = book.edit_epub_structure(replacements={0: markup.replace("STYLES", css)})
            with doc_ex.open(revised) as book:
                paint = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                pdf = book.export_epub_fixed_pdf(dpi=96, searchable=True)
            with doc_ex.open(pdf) as output:
                self.assertEqual(paint, rgba_pixels(png_chunks(output.render_page(0, dpi=96))))
                self.assertEqual(len(output[0].search_for("Visible target")), 1)
                links = output[0].get_links()
                self.assertEqual(len(links), 2)
                self.assertTrue(all(link.metadata["target_page"] == 0 for link in links))
                results.append((paint, [link.bbox for link in links]))
        self.assertEqual(results[0], results[1])
        _, pixels = results[0][0]
        for color in ("c94b39ff", "215caaff", "fcd011ff", "9444b4ff"):
            self.assertIn(bytes.fromhex(color), pixels)

    def test_logical_css_var_failure_resets_alias_and_vertical_writing_fails(self) -> None:
        markup = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Logical flow</title>
          <style>body{margin:0} #outer{width:64px;height:40px;background:#c94b39}
            #inside{width:20px;height:12px;background:#24b781} STYLES
          </style></head><body><div id="outer" DIR><div id="inside"/></div></body></html>"""
        preview = []
        for css in (
            "#outer {padding-left:9px; padding-inline-start:var(--unresolved); margin-block:initial}",
            "#outer {padding-left:0; margin-top:0; margin-bottom:0}",
        ):
            with doc_ex.open(epub_fixture(chapter_one=markup.replace("STYLES", css).replace("DIR", ""))) as book:
                preview.append(rgba_pixels(png_chunks(book.render_page(0, dpi=96))))
        self.assertEqual(preview[0], preview[1])
        for ignored, expected in (
            ("inline-size: banana; padding-inline: banana 2px", "padding-left:8px"),
            ("--gap: banana; padding-inline-start:var(--gap)", "padding-left:0"),
        ):
            control = markup.replace("STYLES", f"#outer{{{expected}}}").replace("DIR", "")
            candidate = markup.replace("STYLES", f"#outer{{padding-left:8px;{ignored}}}").replace("DIR", "")
            with doc_ex.open(epub_fixture(chapter_one=control)) as a, doc_ex.open(epub_fixture(chapter_one=candidate)) as b:
                self.assertEqual(a.render_page(0, dpi=96), b.render_page(0, dpi=96))
        data = epub_fixture(chapter_one=markup.replace(
            "STYLES", "#outer {writing-mode:vertical-rl; inline-size:30px}",
        ).replace("DIR", ""))
        with doc_ex.open(data) as book, self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "horizontal writing mode"):
            book.render_page(0, dpi=96)
        dashed = markup.replace("STYLES", "#outer{border-inline:2px dashed red}").replace("DIR", "")
        with doc_ex.open(epub_fixture(chapter_one=dashed)) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "logical CSS border style"):
                book.render_page(0, dpi=96)

    def test_auto_direction_uses_first_strong_xhtml_text_for_geometry_and_pdf_links(self) -> None:
        source = fixed_spread_fixture([("", "white", 240, 190)])
        markup = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Direction</title>
          <meta name="viewport" content="width=240,height=190"/><style>
            body{margin:0} article,bdi.own{position:absolute; display:block; inline-size:92px;
                block-size:34px; inset-inline-start:16px;font-size:11px; margin:0}
            #rtl{inset-block-start:8px;background:#c94b39}
            #ltr{inset-block-start:46px;background:#24b781}
            #neutral{inset-block-start:84px;background:#215caa}
            #own{inset-block-start:122px;background:#f6d844}
            #target{position:absolute;left:16px;top:156px;margin:0;font-size:12px}
          </style></head><body>
            <article id="rtl" DIR_A><span dir="ltr" hidden="hidden">English</span>
              <bdi hidden="hidden">English</bdi><style>.latin{color:black}</style>
              ١٢٣ — שלום <a href="#target">Back</a></article>
            <article id="ltr" DIR_B><bdi hidden="hidden">שלום</bdi>123 — 東京</article>
            <div dir="rtl"><article id="neutral" DIR_C>١٢٣ —</article></div>
            <bdi id="own" class="own" DIR_D>مرحبا</bdi><p id="target">Target</p>
          </body></html>"""
        variants = (
            ('dir="AuTo"', 'dir="auto"', 'dir="auto"', ""),
            ('dir="rtl"', 'dir="ltr"', 'dir="ltr"', 'dir="rtl"'),
        )
        seen = []
        for directions in variants:
            chapter = markup
            for name, attr in zip(("DIR_A", "DIR_B", "DIR_C", "DIR_D"), directions):
                chapter = chapter.replace(name, attr)
            with doc_ex.open(source) as original:
                updated = original.edit_epub_structure(replacements={0: chapter})
            with doc_ex.open(updated) as book:
                paint = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                pdf = book.export_epub_fixed_pdf(dpi=96, searchable=True)
            with doc_ex.open(pdf) as output:
                self.assertEqual(paint, rgba_pixels(png_chunks(output.render_page(0, dpi=96))))
                (link,) = output[0].get_links()
                self.assertEqual(link.metadata["target_page"], 0)
                self.assertGreater(link.bbox[0], 90)
                seen.append((paint, link.bbox))
        self.assertEqual(seen[0], seen[1])
        width, pixels = seen[0][0]
        for x, y, expected in (
            (218, 33, "c94b39ff"), (24, 70, "24b781ff"),
            (24, 108, "215caaff"), (218, 146, "f6d844ff"),
        ):
            offset = (y * width + x) * 4
            self.assertEqual(pixels[offset:offset + 4], bytes.fromhex(expected))

    def test_reflowable_auto_direction_without_author_css(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Direction</title></head>
          <body><p dir="DIR">123 — שלום Mixed</p></body></html>"""
        previews = []
        for direction in ("auto", "rtl", "ltr"):
            with doc_ex.open(epub_fixture(chapter_one=template.replace("DIR", direction))) as book:
                self.assertEqual(book[0].get_text(), "123 — שלום Mixed")
                previews.append(rgba_pixels(png_chunks(book.render_page(0, dpi=96))))
        self.assertEqual(previews[0], previews[1])
        self.assertNotEqual(previews[0], previews[2])
        inherited = template.replace("<body>", '<body dir="rtl">').replace("123 — שלום Mixed", "English שלום")
        control = inherited.replace('dir="DIR"', 'dir="rtl"')
        candidate = inherited.replace('dir="DIR"', 'dir="auto" style="direction:var(--missing)"')
        with doc_ex.open(epub_fixture(chapter_one=control)) as a, doc_ex.open(epub_fixture(chapter_one=candidate)) as b:
            self.assertEqual(a.render_page(0, dpi=96), b.render_page(0, dpi=96))

    def test_inline_bidi_isolation_matches_controls_and_preserves_pdf_links(self) -> None:
        source = fixed_spread_fixture([("", "white", 260, 150)])
        markup = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Inline bidi</title>
          <meta name="viewport" content="width=260,height=150"/>
          <style>html,body{margin:0} p{margin:0;font-size:13px;font-family:sans-serif,serif}
          #top{position:absolute;left:12px;top:10px;width:215px}
          #end{position:absolute;left:12px;top:110px;width:215px}</style></head>
          <body><p id="top" dir="ltr">Left FIRST Right <br/> Code SECOND End
            <br/> After THIRD Tail <br/> Start FOURTH Last.</p>
            <p id="end"><a href="#inside">Go to target</a></p>
          </body></html>"""
        automatic = (
            '<a href="https://example.org/bidi"><bdi>שלום!</bdi></a>',
            '<bdo dir="rtl">AbCd</bdo>',
            '<span style="direction:rtl;unicode-bidi:embed">שלום!</span>',
            '<bdi id="inside">שלום!<br/>שלום!</bdi>',
        )
        literal = (
            '<a href="https://example.org/bidi">&#x2067;שלום!&#x2069;</a>',
            '&#x2068;&#x202E;AbCd&#x202C;&#x2069;',
            '&#x202B;שלום!&#x202C;',
            '&#x2067;<span id="inside">שלום!</span>&#x2069;<br/>&#x2067;שלום!&#x2069;',
        )
        pictures = []
        for authored in (automatic, literal):
            chapter = markup
            for key, element in zip(("FIRST", "SECOND", "THIRD", "FOURTH"), authored):
                chapter = chapter.replace(key, element)
            with doc_ex.open(source) as before:
                updated = before.edit_epub_structure(replacements={0: chapter})
            with doc_ex.open(updated) as book:
                preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                pdf = book.export_epub_fixed_pdf(dpi=96, searchable=True)
                if authored == automatic:
                    text = book[0].get_text()
                    self.assertIn("שלום!", text)
                    self.assertFalse(any(mark in text for mark in "\u2067\u2069\u202e"))
                    link = next(link for link in book[0].get_links() if link.uri == "https://example.org/bidi")
                    self.assertEqual(link.text, "שלום!")
                    paragraph = next(block for block in book[0].blocks if "Left" in block.text)
                    self.assertEqual(paragraph.text[link.metadata["start"]:link.metadata["end"]], "שלום!")
            with doc_ex.open(pdf) as output, output.edit() as editor:
                self.assertEqual(preview, rgba_pixels(png_chunks(output.render_page(0, dpi=96))))
                self.assertFalse(any(mark in output[0].get_text() for mark in "\u2067\u2069\u202e"))
                self.assertTrue(output[0].search_for("Right"))
                external, local = output[0].get_links()
                self.assertEqual(external.uri, "https://example.org/bidi")
                self.assertEqual(local.metadata["target_page"], 0)
                readable = editor.to_bytes(object_streams=False)
                destination = re.search(rb"/XYZ\s+([0-9.]+)\s+([0-9.]+)\s+null", readable)
                self.assertIsNotNone(destination)
                assert destination is not None
                self.assertGreater(float(destination[1]), 10)
                self.assertLess(float(destination[2]), 112.5)
                pictures.append((preview, external.bbox, destination.groups()))
        self.assertEqual(pictures[0], pictures[1])

    def test_inline_bidi_trailing_anchor_uses_the_logical_end_of_an_rtl_isolate(self) -> None:
        source = fixed_spread_fixture([("", "white", 260, 150)])
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>RTL anchor</title>
          <meta name="viewport" content="width=260,height=150"/>
          <style>html,body{margin:0} p{margin:0;font-size:14px;font-family:sans-serif,serif}
            #edge,#middle,#links{position:absolute;left:12px;width:215px}
            #edge{top:10px} #middle{top:45px} #links{top:100px}</style></head>
          <body>
            <p id="edge" dir="rtl"><bdi dir="rtl"><a href="https://example.org/edge">שלום</a><span id="edge-end"/></bdi></p>
            <p id="middle" dir="ltr">Start <bdi dir="rtl"><a href="https://example.org/middle">שלום</a><span id="middle-end"/></bdi> End</p>
            <p id="links"><a href="#edge-end">First target</a> <a href="#middle-end">Second target</a></p>
          </body></html>"""
        with doc_ex.open(source) as before:
            updated = before.edit_epub_structure(replacements={0: chapter})
        with doc_ex.open(updated) as book:
            pdf = book.export_epub_fixed_pdf(dpi=96)
        with doc_ex.open(pdf) as output, output.edit() as editor:
            links = output[0].get_links()
            boxes = {
                link.uri.rsplit("/", 1)[-1]: link.bbox
                for link in links if link.uri.startswith("https://example.org/")
            }
            self.assertEqual(set(boxes), {"edge", "middle"})
            self.assertEqual(sum(link.metadata.get("target_page") == 0 for link in links), 2)
            destinations = re.findall(
                rb"/XYZ\s+([0-9.]+)\s+([0-9.]+)\s+null",
                editor.to_bytes(object_streams=False),
            )
        self.assertEqual(len(destinations), 2)
        destinations.sort(key=lambda pair: -float(pair[1]))
        for key, (x, _) in zip(("edge", "middle"), destinations):
            left, _, right, _ = boxes[key]
            self.assertGreater(right - left, 10)
            self.assertAlmostEqual(float(x), left, delta=0.05)

    def test_reflow_inline_bidi_preserves_whitespace_and_image_offsets(self) -> None:
        markup = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Bidi image</title>
          <style>body{margin:0} p{font-family:sans-serif,serif}</style></head><body>
          <p dir="ltr">A SCOPE Z</p><p>End</p></body></html>"""
        scoped = '<bdi dir="rtl"> שלום <img src="image.png" alt="Pixel"/>!</bdi>'
        literal = '&#x2067;שלום <img src="image.png" alt="Pixel"/>!&#x2069;'
        previews = []
        for value in (scoped, literal):
            data = rewrite_epub(epub_fixture(chapter_one=markup.replace("SCOPE", value)), {
                "OEBPS/image.png": solid((170, 80, 35)),
            })
            with doc_ex.open(data) as book:
                previews.append(rgba_pixels(png_chunks(book.render_page(0, dpi=96))))
                if value == scoped:
                    self.assertIn("A  שלום ! Z", book[0].get_text())
                    (picture,) = book[0].get_images()
                    self.assertEqual(picture.alt, "Pixel")
                    self.assertEqual(picture.metadata["inline_offset"], len("A  שלום "))
        self.assertEqual(previews[0], previews[1])

    def test_fixed_layout_embedded_xhtml_preserves_frame_pixels_text_and_links(self) -> None:
        source = fixed_spread_fixture([("", "white", 220, 154)])
        host = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Host</title>
          <meta name="viewport" content="width=220,height=154"/>
          <style>html,body {margin:0} body {background:#c22020}
            h1 {position:absolute;left:32px;top:8px;font-size:12px;margin:0}
            iframe {display:block;position:absolute;left:20px;top:32px;width:160px;height:100px}
          </style></head><body>
            <h1 id="home">Host</h1><iframe src="Frames/card.xhtml" title="Card" width="110" height="60"/>
          </body></html>"""
        card = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Card</title>
          <style>body {margin:0}
            .banner {background:#0044bb;height:18px}
            @media (min-width:140px) {.banner {background:#2357c6}}
            object {display:block;position:absolute;top:24px;left:5px;width:120px;height:52px}
          </style></head><body><div class="banner">Embedded</div>
            <object type="application/xhtml+xml" data="Nested/notice.xhtml" title="Notice"/>
          </body></html>"""
        notice = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Notice</title>
          <style>body {margin:0;background:#309858}
            p {margin:0} .outside {position:absolute;left:0;top:58px;width:120px;height:18px;background:#e150ba}
          </style></head><body><p><a href="../../s0.xhtml#home">NESTEDTEXT</a></p>
            <div class="outside">Clipped text</div>
          </body></html>"""
        with doc_ex.open(source) as initial:
            edited = initial.edit_epub_structure(
                replacements={0: host},
                resources={
                    "OEBPS/Frames/card.xhtml": doc_ex.EpubResource("application/xhtml+xml", card.encode()),
                    "OEBPS/Frames/Nested/notice.xhtml": doc_ex.EpubResource("application/xhtml+xml", notice.encode()),
                },
            )
        with doc_ex.open(edited) as book:
            frame, = book[0].get_images()
            self.assertEqual((frame.alt, frame.media_type, frame.data), ("Card", "application/xhtml+xml", card.encode()))
            preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
            fixed = book.export_epub_fixed_pdf(dpi=96, searchable=True)
        width, pixels = preview

        def rgb(x: int, y: int) -> bytes:
            start = (y * width + x) * 4
            return pixels[start:start + 3]

        self.assertEqual(rgb(172, 36), bytes.fromhex("2357c6"))
        self.assertEqual(rgb(130, 94), bytes.fromhex("309858"))
        self.assertEqual(rgb(171, 107), bytes.fromhex("c22020"))
        self.assertEqual(rgb(130, 115), bytes.fromhex("c22020"))
        with doc_ex.open(fixed) as pdf, pdf.edit() as editor:
            self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))), preview)
            self.assertEqual(len(pdf[0].search_for("NESTEDTEXT")), 1)
            self.assertFalse(pdf[0].search_for("Clipped text"))
            link, = pdf[0].get_links()
            self.assertEqual(link.metadata["target_page"], 0)
            self.assertGreaterEqual(link.bbox[0], 18.75)
            self.assertGreaterEqual(link.bbox[1], 42)
            self.assertLessEqual(link.bbox[2], 108.75)
            self.assertLessEqual(link.bbox[3], 81)
            self.assertRegex(editor.to_bytes(object_streams=False), rb"/XYZ\s+24(?:\.0+)?\s+109\.5\s+null")

    def test_embedded_xhtml_rasterizes_in_semantic_exports_and_checks_bounds(self) -> None:
        html = '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Frame</title></head><body>{}</body></html>'
        resource = "application/xhtml+xml"
        source = epub_fixture()
        with doc_ex.open(source) as initial:
            documents = {
                "OEBPS/card.xhtml": doc_ex.EpubResource(resource, html.format(
                    '<div style="width:60px;height:36px;background:#2b59aa"/>'
                ).encode()),
            }
            output = initial.edit_epub_structure(
                replacements={0: html.format('<p>Before <iframe src="card.xhtml" title="Inset" width="60" height="36"/> after</p>')},
                resources=documents,
            )
            for markup, message in (
                ('<iframe src="https://example.org/"/>', "remote embedded EPUB XHTML"),
                ('<iframe src="card.xhtml" srcdoc="No"/>', "iframe srcdoc"),
                ('<iframe src="card.xhtml#section"/>', "fragment scrolling"),
                ('<iframe src="image.png"/>', "local XHTML manifest resource"),
            ):
                with self.subTest(message=message), self.assertRaisesRegex(doc_ex.UnsupportedFormatError, message):
                    initial.edit_epub_structure(replacements={0: html.format(markup)}, resources=documents)
            loops = {
                "OEBPS/card.xhtml": doc_ex.EpubResource(resource, html.format('<iframe src="child.xhtml"/>').encode()),
                "OEBPS/child.xhtml": doc_ex.EpubResource(resource, html.format('<iframe src="card.xhtml"/>').encode()),
            }
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "cyclic embedded EPUB XHTML"):
                initial.edit_epub_structure(replacements={0: html.format('<iframe src="card.xhtml"/>')}, resources=loops)
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "frame count"):
                initial.edit_epub_structure(
                    replacements={0: html.format('<iframe src="card.xhtml"/>' * 33)}, resources=documents,
                )
            oversized = initial.edit_epub_structure(
                replacements={0: html.format('<iframe src="card.xhtml" width="6000" height="4800"/>')},
                resources=documents,
            )
        with doc_ex.open(oversized) as book:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "image pixel budget"):
                book.render_page(0, dpi=96, clip=(0, 0, 100, 100))
        with doc_ex.open(output) as book:
            _, raw = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
            self.assertIn(bytes.fromhex("2b59aaff"), raw)
            for target in ("docx", "pdf"):
                with self.subTest(target=target), doc_ex.open(book.convert(target)) as exported:
                    self.assertIn("Before", exported[0].get_text())
                    self.assertIn("after", exported[0].get_text())
                    image, = exported[0].get_images()
                    self.assertEqual(image.media_type, "image/png")

    def test_fixed_layout_pdf_links_follow_wrapped_text_images_and_destinations(self) -> None:
        source = fixed_spread_fixture([("", "white", 200, 180)] * 3)
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        for family in ("sans-serif", "Sample"):
            with self.subTest(family=family):
                fonts = "@font-face {font-family: Sample; src: url(Inter.ttf)}" if family == "Sample" else ""
                first = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>First</title>
                <meta name="viewport" content="width=200,height=180"/><style>
                {fonts} body {{margin:0; font: 18px {family}}}
                #wrap {{position:absolute; left:16px; top:12px; width:90px; height:31px; overflow:hidden}}
                #pic {{position:absolute; left:132px; top:8px}}
                #block {{display:block; position:absolute; left:16px; top:90px; width:48px; height:24px}}
                #jumps {{position:absolute; left:16px; top:125px}}
                </style></head><body>
                <div id="wrap"><a href="https://example.org/text">Alpha Bravo Charlie Delta Echo Foxtrot</a></div>
                <div id="pic"><a href="mailto:author@example.org"><img src="image.png" alt="Mail"/></a></div>
                <a id="block" href="s1.xhtml#caf%C3%A9">Next</a>
                <div id="jumps"><a href="#wrap">Same</a> <a href="s2.xhtml">Omitted</a>
                <a href="s1.xhtml#missing">Missing</a> <a href="javascript:void(0)">Inert</a></div>
                </body></html>"""
                second = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Second</title>
                <meta name="viewport" content="width=200,height=180"/><style>
                {fonts} body {{margin:0; font: 16px {family}}}
                p {{position:absolute; left:24px; top:56px; margin:0}}
                </style></head><body><p>Prefix <span id="café">Café</span> suffix</p></body></html>"""
                data = rewrite_epub(source, {
                    "OEBPS/s0.xhtml": first, "OEBPS/s1.xhtml": second, "OEBPS/Inter.ttf": font,
                    "OEBPS/image.png": solid((212, 32, 32)),
                })
                with doc_ex.open(data) as book:
                    pdf = book.export_epub_fixed_pdf(dpi=96, pages=[1, 0, 0])
                    preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                with doc_ex.open(pdf) as result, result.edit() as editor:
                    for index in (1, 2):
                        links = result[index].get_links()
                        wrapped = [link.bbox for link in links if link.uri == "https://example.org/text"]
                        self.assertEqual(len(wrapped), 2, wrapped)
                        self.assertLess(wrapped[0][1], wrapped[1][1])
                        for left, top, right, bottom in wrapped:
                            self.assertGreaterEqual(left, 12)
                            self.assertGreaterEqual(top, 9)
                            self.assertLessEqual(right, 79.5)
                            self.assertLessEqual(bottom, 32.25)
                        mail, = [link for link in links if link.uri == "mailto:author@example.org"]
                        self.assertGreaterEqual(mail.bbox[0], 99)
                        block, same = [link for link in links if link.metadata.get("target_kind") == "page"]
                        self.assertEqual(block.metadata["target_page"], 0)
                        self.assertEqual(tuple(block.bbox), (12, 67.5, 48, 85.5))
                        self.assertEqual(same.metadata["target_page"], index)
                        self.assertEqual(len(links), 5)
                        self.assertEqual(rgba_pixels(png_chunks(result.render_page(index, dpi=96))), preview)
                    pdf_objects = editor.to_bytes(object_streams=False)
                    self.assertRegex(pdf_objects, rb"/XYZ\s+12(?:\.0+)?\s+126(?:\.0+)?\s+null")
                    self.assertRegex(pdf_objects, rb"/XYZ\s+(?:[4-9]\d)(?:\.\d+)?\s+9[0-5](?:\.\d+)?\s+null")

    def test_fixed_layout_pdf_preserves_link_order_around_inline_equations(self) -> None:
        source = fixed_spread_fixture([("", "white", 360, 120)])
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        for family in ("serif", "Local"):
            with self.subTest(family=family):
                chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Linked equation</title>
                <meta name="viewport" content="width=360,height=120"/>
                <style>@font-face {{font-family:Local;src:url(font.ttf)}}
                body {{margin:0;font-family:{family};font-size:20px}}</style></head>
                <body><p>Open <a href="https://example.org/formula">formula
                <math xmlns="http://www.w3.org/1998/Math/MathML"><mfrac><mi>x</mi><mi>y</mi></mfrac></math>
                details</a> and <a href="#target">jump</a>.</p><p>Label <span id="target">target</span></p>
                </body></html>"""
                control = chapter.replace(
                    '<math xmlns="http://www.w3.org/1998/Math/MathML">',
                    '</a><math xmlns="http://www.w3.org/1998/Math/MathML">',
                ).replace("</math>", '</math><a href="https://example.org/formula">')

                def areas(markup: str) -> list[tuple[float, float, float, float]]:
                    data = rewrite_epub(source, {"OEBPS/s0.xhtml": markup, "OEBPS/font.ttf": font})
                    with doc_ex.open(data) as book:
                        pdf = book.export_epub_fixed_pdf(dpi=96)
                    with doc_ex.open(pdf) as reopened:
                        links = reopened[0].get_links()
                    self.assertEqual(links[-1].metadata.get("target_page"), 0)
                    self.assertTrue(all(link.uri == "https://example.org/formula" for link in links[:-1]))
                    return [link.bbox for link in links[:-1] if link.bbox is not None]

                left, right = areas(control)
                center = (left[2] + right[0]) / 2
                self.assertGreater(right[0] - left[2], 2)
                self.assertTrue(any(area[0] < center < area[2] for area in areas(chapter)))

    def test_fixed_layout_pdf_links_use_transformed_svg_content_geometry(self) -> None:
        data = fixed_spread_fixture([("", "white", 160, 100)] * 2)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b'href="s0.xhtml" media-type="application/xhtml+xml"',
                                  b'href="s0.svg" media-type="image/svg+xml"')
        chapter = """<svg xmlns="http://www.w3.org/2000/svg"
                xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="10 20 160 100">
                <g transform="translate(10 20)">
                  <a href="#destination" xlink:href="s1.xhtml">
                    <rect x="24" y="16" width="40" height="32" fill="#bb1256"/>
                  </a>
                  <clipPath id="edge"><rect x="144" y="12" width="40" height="40"/></clipPath>
                  <a id="pydoccraft-pdf-link-0" xlink:href="https://example.org/plate" clip-path="url(#edge)">
                    <rect x="140" y="10" width="40" height="24" fill="#124796"/>
                  </a>
                  <a href="s1.xhtml"><rect x="20" y="60" width="20" height="20"/></a>
                  <a href="javascript:void(0)"><rect x="50" y="60" width="20" height="20"/></a>
                  <a href="https://example.org/invisible" style="display:none">
                    <rect x="20" y="10" width="20" height="20"/>
                  </a>
                  <rect id="destination" x="80" y="60" width="20" height="20" fill="#339933"/>
                </g>
                </svg>"""
        data = rewrite_epub(data, {"OEBPS/content.opf": package, "OEBPS/s0.svg": chapter})
        with doc_ex.open(data) as book:
            pdf = book.export_epub_fixed_pdf(dpi=96, pages=[0])
            preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_chapters({0: chapter.replace('id="destination"', '')})
            revised = book.edit_epub_chapters({0: chapter.replace('x="80" y="60"', 'x="92" y="60"')})
        with doc_ex.open(revised) as book:
            updated = book.export_epub_fixed_pdf(dpi=96, pages=[0])
        with doc_ex.open(updated) as result, result.edit() as editor:
            self.assertRegex(editor.to_bytes(object_streams=False),
                             rb"/XYZ\s+69(?:\.0+)?\s+30(?:\.0+)?\s+null")
        with doc_ex.open(pdf) as result, result.edit() as editor:
            same_page, external = result[0].get_links()
            self.assertEqual(same_page.metadata["target_page"], 0)
            self.assertEqual(tuple(same_page.bbox), (18, 12, 48, 36))
            self.assertEqual(external.uri, "https://example.org/plate")
            self.assertEqual(tuple(external.bbox), (108, 9, 120, 25.5))
            self.assertEqual(rgba_pixels(png_chunks(result.render_page(0, dpi=96))), preview)
            self.assertRegex(editor.to_bytes(object_streams=False),
                             rb"/XYZ\s+60(?:\.0+)?\s+30(?:\.0+)?\s+null")

    def test_fixed_layout_pdf_recovers_graphics_links_and_destinations_inside_svg_use(self) -> None:
        fixture = fixed_spread_fixture([("", "white", 184, 124)])
        with zipfile.ZipFile(io.BytesIO(fixture)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )
        svg = """<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
            width="184" height="124" viewBox="0 0 184 124">
          <style>.first { --tint: #d05030; } .second { --tint: #6070d0; }
                 .panel rect { fill: var(--tint, black); }</style>
          <defs>
            <clipPath id="triangle"><path d="M 20,2 L 36,2 L 20,18 Z"/></clipPath>
            <g id="panel" class="panel">
              <a href="#caf%C3%A9" xlink:href="https://example.org/wrong">
                <rect x="2" y="2" width="12" height="12"/>
              </a>
              <a xlink:href="https://example.org/wedge" clip-path="url(#triangle)">
                <rect x="18" y="0" width="24" height="22"/>
              </a>
              <a href="https://example.org/clear" style="opacity:0">
                <rect x="38" y="0" width="10" height="10"/>
              </a>
              <a href="https://example.org/transparent">
                <rect x="2" y="22" width="10" height="8" fill-opacity="0"/>
              </a>
              <g transform="translate(37 22)">
                <rect id="café" x="3" y="2" width="9" height="9"/>
              </g>
            </g>
            <use id="nested" href="#panel" x="4" y="3"/>
            <symbol id="symbol" viewBox="0 0 52 36"><use href="#nested"/></symbol>
          </defs>
          <use class="first" href="#nested" x="90" y="5"/>
          <use class="second" href="#symbol" x="10" y="58" width="78" height="54"/>
        </svg>"""
        xhtml = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Reused graphics</title>
          <meta name="viewport" content="width=184,height=124"/></head>
          <body style="margin:0">{svg}</body></html>"""
        variants = (
            ("OEBPS/s0.svg", svg, {"OEBPS/content.opf": package}),
            ("OEBPS/s0.xhtml", xhtml, {}),
        )
        for path, chapter, updates in variants:
            with self.subTest(path=path), doc_ex.open(rewrite_epub(fixture, {**updates, path: chapter})) as book:
                preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                output = book.export_epub_fixed_pdf(dpi=96, pages=[0, 0], searchable=True)
                self.assertEqual(book.read_epub_resource(path), chapter.encode())
                with doc_ex.open(output) as pdf, pdf.edit() as editor:
                    for i in range(2):
                        jump, wedge, repeat_jump, repeat_wedge = pdf[i].get_links()
                        self.assertEqual([jump.metadata["target_page"], repeat_jump.metadata["target_page"]], [i, i])
                        self.assertEqual(wedge.uri, "https://example.org/wedge")
                        self.assertEqual(repeat_wedge.uri, wedge.uri)
                        for link, bounds in (
                            (jump, (72.0, 7.5, 81.0, 16.5)),
                            (repeat_jump, (14.25, 49.125, 27.75, 62.625)),
                            (wedge, (85.5, 7.5, 97.5, 19.5)),
                        ):
                            for actual, expected in zip(link.bbox, bounds):
                                self.assertAlmostEqual(actual, expected, delta=0.001)
                        for actual, expected in zip(repeat_wedge.bbox, (34.5, 49.125, 52.5, 67.125)):
                            self.assertAlmostEqual(actual, expected, delta=0.75)
                        self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(i, dpi=96))), preview)
                    raw = editor.to_bytes(object_streams=False)
                    self.assertEqual(len(re.findall(rb"/XYZ\s+100\.5\s+69(?:\.0+)?\s+null", raw)), 4)

    def test_fixed_layout_pdf_links_use_positioned_svg_text_spans(self) -> None:
        data = fixed_spread_fixture([("", "white", 320, 160)])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )
        svg = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 160">
            <style>.active {fill:#92004c} .quiet { visibility:hidden }</style>
            <defs><g id="callout"><text font-size="18"><a href="https://example.org/reused">Repeated</a></text></g></defs>
            <g transform="translate(8 6)" font-family="Liberation Serif" font-size="18">
              <text x="12" y="38">Before <a href="https://example.org/text" x="86">
                <tspan class="active">See</tspan><tspan> details</tspan></a><tspan x="220" fill="#000001"> After</tspan></text>
              <text x="16" y="78"><a href="#caf%C3%A9">Jump</a>
                <tspan x="130" id="café">the fragment</tspan></text>
              <text x="16" y="118"><a class="quiet" href="https://example.org/hidden">Invisible</a>
                <a x="130" fill-opacity="0" href="https://example.org/clear">Transparent</a></text>
              <text x="16" y="142"><a href="#pydoccraft-pdf-link-2">Missing</a></text>
            </g><use href="#callout" x="178" y="118"/></svg>"""
        data = rewrite_epub(data, {"OEBPS/content.opf": package, "OEBPS/s0.svg": svg})
        with doc_ex.open(data) as book:
            preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
            output = book.export_epub_fixed_pdf(dpi=96, pages=[0, 0], searchable=True)
        with doc_ex.open(output) as pdf, pdf.edit() as editor:
            for i in range(2):
                external, jump, instance = pdf[i].get_links()
                self.assertEqual(external.uri, "https://example.org/text")
                self.assertAlmostEqual(external.bbox[0], 70.5, delta=1)
                self.assertLess(external.bbox[2], 160)
                self.assertEqual(jump.metadata["target_page"], i)
                self.assertLess(jump.bbox[2], 90)
                self.assertEqual(instance.uri, "https://example.org/reused")
                self.assertAlmostEqual(instance.bbox[0], 133.5, delta=1)
                self.assertIn("the fragment", pdf[i].get_text())
                self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(i, dpi=96))), preview)
            self.assertRegex(editor.to_bytes(object_streams=False), rb"/XYZ\s+103\.5\s+[0-9.]+\s+null")

    def test_fixed_layout_pdf_links_inside_inline_svg_use_xhtml_placement(self) -> None:
        data = fixed_spread_fixture([("", "white", 360, 180)])
        packaged = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        svg = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 48" width="120" height="48">
            <a href="https://example.org/plot"><rect x="8" y="5" width="24" height="16" fill="#04b"/></a>
            <text x="42" y="32" font-size="14"><a href="#detail">Read</a></text></svg>"""
        for family in ("serif", "Local"):
            with self.subTest(font=family):
                chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Inline SVG</title>
                    <meta name="viewport" content="width=360,height=180"/>
                    <style>@font-face {{font-family:Local;src:url(font.ttf)}}
                    body {{margin:0;font-family:{family};font-size:16px}}
                    p {{position:absolute;left:16px;top:10px;margin:0}}
                    #clip {{position:absolute;left:48px;top:76px;width:22px;height:48px;overflow:hidden}}
                    #target {{position:absolute;left:152px;top:132px}}
                    img {{position:absolute;left:224px;top:10px}}</style></head>
                    <body><p>Label {svg} after</p><div id="clip">{svg}</div>
                    <img src="picture.svg" alt="Inert image"/>
                    <svg id="target" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 40" width="120" height="40">
                    <text x="12" y="24"><tspan id="detail">Detail</tspan></text></svg></body></html>"""
                source = rewrite_epub(data, {"OEBPS/s0.xhtml": chapter, "OEBPS/font.ttf": packaged,
                                                 "OEBPS/picture.svg": svg})
                with doc_ex.open(source) as book:
                    output = book.export_epub_fixed_pdf(dpi=96)
                    preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                with doc_ex.open(output) as pdf, pdf.edit() as editor:
                    links = pdf[0].get_links()
                    plot = [link for link in links if link.uri == "https://example.org/plot"]
                    self.assertEqual(len(plot), 2, links)
                    inline, clipped = sorted(plot, key=lambda link: link.bbox[1])
                    self.assertGreater(inline.bbox[0], 18)
                    self.assertLess(inline.bbox[1], 45)
                    self.assertEqual(tuple(clipped.bbox), (42, 60.75, 52.5, 72.75))
                    jump, = [link for link in links if link.metadata.get("target_kind") == "page"]
                    self.assertEqual(jump.metadata["target_page"], 0)
                    self.assertEqual(len(links), 3)
                    self.assertRegex(editor.to_bytes(object_streams=False), rb"/XYZ\s+123(?:\.0+)?\s+[0-9.]+\s+null")
                    self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))), preview)

    def test_fixed_layout_pdf_svg_coverage_follows_clip_rules_and_masks(self) -> None:
        data = fixed_spread_fixture([("", "white", 360, 236)] * 2)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )
        outline = "M0 0H208V114H0Z M59 4H143V59H59Z"
        svg = f"""<svg xmlns="http://www.w3.org/2000/svg" width="360" height="236" viewBox="0 0 360 236">
          <defs>
            <clipPath id="odd"><path clip-rule="evenodd" d="{outline}"/></clipPath>
            <clipPath id="winding"><path clip-rule="nonzero" d="{outline}"/></clipPath>
            <clipPath id="edge"><circle cx="4" cy="28" r="11"/></clipPath>
            <mask id="lum" maskUnits="userSpaceOnUse" x="0" y="62" width="208" height="52">
              <rect x="0" y="62" width="208" height="52" fill="white"/>
              <circle cx="128" cy="93" r="27" fill="black"/>
            </mask>
            <mask id="alpha" mask-type="alpha" maskContentUnits="objectBoundingBox"
                  x="0" y="0" width="1" height="1">
              <rect width=".54" height="1" fill="black" opacity=".6"/>
            </mask>
          </defs>
          <g font-family="monospace" font-size="16">
            <g transform="translate(20 14)" clip-path="url(#odd)">
              <text x="8" y="30">KEPT</text>
              <text x="70" y="30"><a href="https://example.org/hidden">HOLE</a></text>
              <a href="https://example.org/obscured"><rect x="79" y="45" width="40" height="8"/></a>
              <g mask="url(#lum)">
                <text x="6" y="99">OPEN SPACE</text><text x="102" y="99">BLACK</text>
              </g>
            </g>
            <g transform="translate(20 118)" clip-path="url(#winding)">
              <text x="70" y="30">FILLED</text>
            </g>
            <g transform="translate(222 22) rotate(5)" mask="url(#alpha)">
              <text x="4" y="33">ALPHA</text><text x="63" y="33">ZERO</text>
            </g>
            <g transform="translate(230 79)" clip-path="url(#edge)">
              <text x="5" y="39" font-size="24"><a href="https://example.org/edge">M</a></text>
            </g>
            <g transform="translate(240 144)">
              <a href="https://example.org/shape" clip-path="url(#edge)">
                <rect x="0" y="13" width="40" height="30" fill="#d44"/>
              </a>
            </g>
          </g></svg>"""
        xhtml = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>SVG coverage</title>
          <meta name="viewport" content="width=360,height=236"/>
          <style>html,body {{margin:0}} svg {{display:block}}</style></head><body>{svg}</body></html>"""
        data = rewrite_epub(data, {"OEBPS/content.opf": package, "OEBPS/s0.svg": svg, "OEBPS/s1.xhtml": xhtml})
        with doc_ex.open(data) as book:
            rendered = [rgba_pixels(png_chunks(book.render_page(i, dpi=96))) for i in range(2)]
            output = book.export_epub_fixed_pdf(dpi=96, searchable=True)
        with doc_ex.open(output) as pdf:
            for i in range(2):
                with self.subTest(page=i):
                    text = pdf[i].get_text()
                    for label in ("KEPT", "OPEN SPACE", "FILLED", "ALPHA", "M"):
                        self.assertIn(label, text)
                        self.assertTrue(pdf[i].search_for(label))
                    for label in ("HOLE", "BLACK", "ZERO"):
                        self.assertNotIn(label, text)
                    edge, shape = pdf[i].get_links()
                    self.assertEqual(edge.uri, "https://example.org/edge")
                    self.assertEqual(shape.uri, "https://example.org/shape")
                    hit, = pdf[i].search_for("M")
                    self.assertLessEqual(hit[2], 184)
                    self.assertLessEqual(edge.bbox[2], 184)
                    self.assertLessEqual(shape.bbox[2], 191.25)
                    self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(i, dpi=96))), rendered[i])

    def test_fixed_layout_pdf_svg_coverage_budget_does_not_limit_visual_exports(self) -> None:
        data = fixed_spread_fixture([("", "white", 9000, 45)])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )
        svg = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 9000 45">
          <defs><clipPath id="window"><rect width="9000" height="44"/></clipPath></defs>
          <g clip-path="url(#window)"><rect width="9000" height="44" fill="#f5f5ff"/>
            <text x="20" y="30">Large layer</text></g></svg>"""
        data = rewrite_epub(data, {"OEBPS/content.opf": package, "OEBPS/s0.svg": svg})
        with doc_ex.open(data) as book:
            visual = book.export_epub_fixed_pdf(dpi=96)
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "coverage exceeds its tile budget"):
                book.export_epub_fixed_pdf(dpi=96, searchable=True)
        with doc_ex.open(visual) as pdf:
            self.assertEqual(pdf[0].metadata["width"], 6750)
            self.assertEqual(pdf[0].get_text(), "")

    def test_fixed_layout_pdf_searchable_text_preserves_rendering_and_selection(self) -> None:
        source = fixed_spread_fixture([("", "white", 320, 200)] * 3)
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            opf = archive.read("OEBPS/content.opf")
        opf = opf.replace(b'href="s2.xhtml" media-type="application/xhtml+xml"',
                          b'href="s2.svg" media-type="image/svg+xml"')

        def xhtml(family: str, word: str) -> str:
            return f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Text</title>
                <meta name="viewport" content="width=320,height=200"/>
                <style>
                @font-face {{font-family:Book;src:url(Inter.ttf)}}
                body {{margin:0; font-family:{family}; font-size:20px}}
                #content {{position:absolute; top:20px; left:24px}}
                #clip {{position:absolute; left:24px; top:80px; height:28px; overflow:hidden}}
                #omitted {{position:absolute; top:60px}}
                </style></head><body>
                <div id="content"><a href="https://example.org/words">{word} café office Ω</a>
                <span style="display:none">DISPLAYNONE</span>
                <span style="visibility:hidden">INVISIBLE</span></div>
                <div id="clip">Visible here<span id="omitted">OUTSIDECLIP</span></div>
                <p style="position:absolute; left:24px; top:135px; margin:0">
                <math xmlns="http://www.w3.org/1998/Math/MathML">
                  <mfrac><mtext>Numerator</mtext><mtext>Denominator</mtext></mfrac>
                </math></p>
                <div style="position:absolute;left:174px;top:68px">
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 140 24">
                    <text x="1" y="18" font-family="Source Sans 3" font-size="18">Vector heading</text>
                  </svg>
                </div>
                <p style="position:absolute;left:158px;top:106px;margin:0">Art
                  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 24">
                    <text x="1" y="18" font-family="Source Sans 3" font-size="17">Inline vector</text>
                  </svg></p>
                </body></html>"""

        svg = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="10 20 320 200">
                <style>@font-face {font-family:Book;src:url(Inter.ttf)}</style>
                <g transform="translate(10 20)">
                  <text x="32" y="64" font-size="24" font-family="Book">SVG résumé</text>
                  <text x="32" y="120" font-size="24" font-family="Book" visibility="hidden">SVGHIDDEN</text>
                </g></svg>"""
        data = rewrite_epub(source, {
            "OEBPS/content.opf": opf, "OEBPS/s0.xhtml": xhtml("serif", "Standard"),
            "OEBPS/s1.xhtml": xhtml("Book", "Packaged"), "OEBPS/s2.svg": svg,
            "OEBPS/Inter.ttf": font,
        })
        order = [2, 1, 0, 1]
        with doc_ex.open(data) as book:
            visual = book.export_epub_fixed_pdf(dpi=96, pages=order)
            searchable = book.export_epub_fixed_pdf(dpi=96, pages=order, searchable=True)
            with self.assertRaisesRegex(TypeError, "searchable must be a bool"):
                book.export_epub_fixed_pdf(searchable=1)
        with doc_ex.open(visual) as images, doc_ex.open(searchable) as pdf:
            self.assertEqual(len(pdf), len(order))
            for index, word in enumerate(("SVG résumé", "Packaged", "Standard", "Packaged")):
                with self.subTest(index=index):
                    page = pdf[index]
                    text = page.get_text()
                    self.assertEqual(text.count(word), 1, text)
                    self.assertEqual(images[index].get_text().strip(), "")
                    hit, = page.search_for(word)
                    self.assertGreater(hit[0], 15)
                    self.assertLess(hit[0], 29)
                    self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(index, dpi=96))),
                                     rgba_pixels(png_chunks(images.render_page(index, dpi=96))))
                    if index == 0:
                        self.assertNotIn("SVGHIDDEN", text)
                    else:
                        self.assertIn("café office Ω", text)
                        self.assertIn("Visible here", text)
                        for hidden in ("DISPLAYNONE", "INVISIBLE", "OUTSIDECLIP"):
                            self.assertNotIn(hidden, text)
                        self.assertIn("Numerator", text)
                        self.assertIn("Denominator", text)
                        self.assertIn("Vector heading", text)
                        self.assertIn("Inline vector", text)
                        self.assertEqual(page.get_links()[0].uri, "https://example.org/words")

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

    def test_epub_unicode_range_uses_composite_faces_and_fallback_glyphs(self) -> None:
        inter = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        wide = font_with_extra_advance(inter)

        def preview(css: str, content: str) -> bytes:
            source = epub_fixture(chapter_one=f"""<html xmlns="http://www.w3.org/1999/xhtml"><head>
              <style>body, p {{margin: 0; font-size: 42px; line-height: 64px}} {css}</style></head>
              <body><p>{content}</p></body></html>""")
            source = rewrite_epub(source, {"OEBPS/inter.ttf": inter, "OEBPS/wide.ttf": wide})
            with doc_ex.open(source) as book:
                self.assertEqual(book[0].get_text(), "AAAABBBBX")
                return book.render_page(0, dpi=96)

        first = "@font-face {font-family: Composite; src: url(inter.ttf); unicode-range: U+004?, U+20}"
        second = "@font-face {font-family: Composite; src: url(wide.ttf); unicode-range: U+0042-0042}"
        controls = """@font-face {font-family: Standard; src: url(inter.ttf)}
          @font-face {font-family: Wide; src: url(wide.ttf)}
          .standard {font-family: Standard} .wide {font-family: Wide} .fallback {font-family: 'Source Sans 3'}"""
        reference = preview(controls, '<span class="standard">AAAA</span><span class="wide">BBBB</span><span class="fallback">X</span>')
        composite = preview(first + second + " p {font-family: Composite}", "AAAABBBBX")
        self.assertEqual(composite, reference)
        self.assertNotEqual(composite, preview(second + first + " p {font-family: Composite}", "AAAABBBBX"))

        wide = wide[:4] + struct.pack(">H", 4096) + wide[6:]
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "font table count"):
            preview(first + second + " p {font-family: Composite}", "AAAABBBBX")

    def test_epub_css_variables_resolve_inheritance_cycles_and_shorthand_order(self) -> None:
        def preview(main: str, local: str) -> bytes:
            chapter = f'''<html xmlns="http://www.w3.org/1999/xhtml"><head>
                <link rel="stylesheet" href="main.css"/></head>
              <body><div class="box" style="{local}"><p>Typography</p>
              <p class="cycle">Fallback</p><p class="case">Case sensitive</p>
              <p class="invalid">Inherited color</p></div><p class="case">Outside</p></body></html>'''
            source = rewrite_epub(epub_fixture(chapter_one=chapter), {
                "OEBPS/main.css": ("@import url(tokens.css);" + main).encode(),
                "OEBPS/tokens.css": b":root {--Ink:#2446a8;--ink:#95164a;--alias:var(--Ink);--gap:6px 12px}",
            })
            with doc_ex.open(source) as book:
                return book.render_page(0, dpi=96)

        common = "body,p {margin:0} body {color:#224466;font-family:sans-serif;font-size:20px}"
        variables = """
          .box {--Ink:#ac3316; --bg:#e9c682 !important;
                padding:var(--gap); padding-right:7px; background:var(--bg);
                border:var(--missing, 2px solid #245533); color: var(--alias)}
          .cycle {--a:var(--b, blue);--b:var(--ink,var(--a));
                  --c:var(--a,#118855);color:var(--c)}
          .case {color:var(--Ink)}
          .invalid {color:#dd1122;color:var(--missing)}
        """
        literals = """
          .box {padding:6px 12px;padding-right:7px;background:#e9c682;
                border:2px solid #245533;color:#2446a8}
          .cycle {color:#118855}
          .box .case {color:#ac3316}
          .case {color:#2446a8}
        """
        self.assertEqual(
            preview(common + variables, "--bg:#22eedd"),
            preview(common + literals, ""),
        )

        amplified = ".box{--large:" + "red " * 700 + ";--twice:var(--large) var(--large);color:var(--twice)}"
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "CSS variable value"):
            preview(common + amplified, "")

    def test_epub_svg_css_variable_values_pass_through_static_paint(self) -> None:
        def preview(styles: str) -> bytes:
            svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 48">
              <style>{styles}</style><g class="layer">
              <rect width="48" height="48"/><circle cx="74" cy="24" r="18"/></g></svg>'''
            chapter = doc_ex.EpubSvgChapterSource("Paint", svg)
            with doc_ex.open(epub_fixture()) as original:
                revised = original.edit_epub_structure([chapter, 0])
            with doc_ex.open(revised) as book:
                return book.render_page(0, dpi=96)

        self.assertEqual(
            preview("svg{--color:#43883a}.layer{--alias:var(--color)}"
                    "rect{fill:var(--alias)}circle{fill:var(--absent,#243a98)}"),
            preview("rect{fill:#43883a}circle{fill:#243a98}"),
        )
        with self.assertRaises(doc_ex.UnsupportedFormatError):
            preview("svg{--paint:url(https://example.invalid/item.svg)}rect{fill:var(--paint)}")

    def test_epub_font_stacks_follow_declared_order_and_inherit(self) -> None:
        inter = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()

        def preview(family: str, faces: str = "", child: str = "") -> bytes:
            chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><style>
              {faces}
              body {{margin:0; font-size:40px; line-height:60px; font-family:{family}}}
              p {{margin:0}} {child}
              </style></head><body><p><span>AAAA</span></p><p>BBBB</p></body></html>"""
            source = rewrite_epub(epub_fixture(chapter_one=chapter), {"OEBPS/inter.ttf": inter})
            with doc_ex.open(source) as book:
                return book.render_page(0, dpi=96)

        faces = """
          @font-face {font-family: First; src: url(inter.ttf); unicode-range: U+0041}
          @font-face {font-family: "Other, Face"; src: url(inter.ttf)}
          @font-face {font-family: "monospace"; src: url(inter.ttf)}
        """
        full_face = preview('"Other, Face"', faces)
        self.assertEqual(preview('Missing, First, "Other, Face", monospace', faces), full_face)
        self.assertEqual(preview(r'Missing, Other\,\ Face, monospace', faces), full_face)
        self.assertEqual(preview('First, "Other, Face"', faces, 'span {font-family: inherit}'), full_face)

        generic = preview("Missing, monospace")
        self.assertEqual(preview('monospace, "Other, Face"', faces), generic)
        self.assertNotEqual(generic, full_face)
        self.assertEqual(preview('"monospace", monospace', faces), full_face)
        colliding = faces.replace("font-family: First", 'font-family: "Source Sans 3"')
        self.assertEqual(preview('"Source Sans 3", "Other, Face"', colliding), full_face)
        self.assertNotEqual(preview("Missing, sans-serif"), generic)

    def test_epub_xhtml_variable_axes_cascade_and_optical_sizing(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "InterVariable.ttf").read_bytes()

        def preview(parent: str = "", child: str = "", optical: str = "none") -> bytes:
            chapter = f'''<html xmlns="http://www.w3.org/1999/xhtml"><head><style>
              @font-face {{font-family: Local Variable; src: url(variable.ttf)}}
              body {{margin: 0; font-size: 48px; line-height: 64px;
                     font-family: Missing, Local Variable, sans-serif;
                     font-optical-sizing: {optical}; {parent}}}
              p {{margin: 0}} span {{{child}}}
              </style></head><body><p><span>Variable glyphs</span></p></body></html>'''
            source = rewrite_epub(epub_fixture(chapter_one=chapter), {"OEBPS/variable.ttf": font})
            with doc_ex.open(source) as book:
                self.assertEqual(book[0].get_text(), "Variable glyphs")
                return book.render_page(0, dpi=96)

        heavy = 'font-variation-settings: "wght" 900'
        default = preview()
        bold = preview(heavy)
        self.assertNotEqual(default, bold)
        self.assertEqual(bold, preview(child=heavy))
        self.assertEqual(bold, preview('font-variation-settings: "wght" 200, "wght" 900'))
        self.assertEqual(bold, preview('font-variation-settings: "wght" 10000'))
        self.assertEqual(bold, preview(heavy + '; font-variation-settings: "bad" 100'))
        self.assertEqual(default, preview(heavy, "font-variation-settings: normal"))
        self.assertEqual(default, preview('font-variation-settings: "XXXX" 700'))

        automatic = preview(optical="auto")
        self.assertNotEqual(default, automatic)
        self.assertEqual(automatic, preview('font-variation-settings: "opsz" 32'))
        self.assertEqual(default, preview(child="font-optical-sizing: none", optical="auto"))
        self.assertEqual(default, preview('font-variation-settings: "opsz" 14', optical="auto"))

        def bundled(settings: str) -> bytes:
            chapter = f'''<html xmlns="http://www.w3.org/1999/xhtml">
              <body style="font-size: 48px; {settings}"><p>Bundled static font</p></body></html>'''
            with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
                return book.render_page(0, dpi=96)

        self.assertEqual(bundled(""), bundled("font-variation-settings: 'wght' 900"))

    def test_epub_xhtml_selects_variable_face_ranges_and_face_specific_defaults(self) -> None:
        fixtures = Path(__file__).parent / "fixtures" / "fonts"
        variable = (fixtures / "InterVariable.ttf").read_bytes()
        wide = font_with_extra_advance((fixtures / "Inter-Regular.ttf").read_bytes())

        def preview(
            definitions: str, properties: str = "", text: str = "AB", *, fixed: bool = False,
        ) -> bytes:
            xhtml = f'''<html xmlns="http://www.w3.org/1999/xhtml"><head>
              <title>Variable faces</title><meta name="viewport" content="width=360,height=90"/>
              <style>{definitions}
              body {{margin:0; font-size:48px; line-height:64px;
                     font-family: Sample, Second, sans-serif; font-optical-sizing:none; {properties}}}
              p {{margin:0}}</style></head><body><p>{text}</p></body></html>'''
            if fixed:
                source = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 360, 90)])
                resources = {"OEBPS/s0.xhtml": xhtml}
            else:
                source = epub_fixture(chapter_one=xhtml)
                resources = {}
            resources.update({"OEBPS/variable.ttf": variable, "OEBPS/wide.ttf": wide})
            with doc_ex.open(rewrite_epub(source, resources)) as book:
                return book.render_page(0, dpi=96)

        broad = "@font-face {font-family:Sample; src:url(variable.ttf); font-weight:300 800}"
        defaults = broad[:-1] + '; font-variation-settings:"wght" 632.5, "opsz" 32}'
        for fixed in (False, True):
            with self.subTest(fixed=fixed):
                sample = preview(defaults, fixed=fixed)
                self.assertEqual(sample, preview(broad, 'font-variation-settings:"wght" 632.5, "opsz" 32', fixed=fixed))
                self.assertNotEqual(sample, preview(broad, fixed=fixed))
                self.assertEqual(sample, preview(defaults, 'font-variation-settings:normal', fixed=fixed))
                self.assertEqual(preview(defaults, 'font-variation-settings:"wght" 300', fixed=fixed),
                                 preview(broad, 'font-variation-settings:"wght" 300, "opsz" 32', fixed=fixed))
                optical = defaults.replace('"opsz" 32', '"opsz" 14')
                self.assertEqual(sample, preview(optical, 'font-optical-sizing:auto', fixed=fixed))
                self.assertEqual(preview(broad, "font-weight:922.5", fixed=fixed),
                                 preview(broad, 'font-variation-settings:"wght" 800', fixed=fixed))
                self.assertEqual(preview(broad, "font-weight:553.5", fixed=fixed),
                                 preview(broad, 'font-variation-settings:"wght" 553.5', fixed=fixed))

        narrow = broad.replace("300 800", "100 350")
        heavy = "@font-face {font-family:Sample; src:url(wide.ttf); font-weight:490 650}"
        self.assertEqual(preview(narrow + heavy, "font-weight:450"), preview(heavy, "font-weight:450"))
        self.assertEqual(preview(heavy + narrow, "font-weight:320"), preview(narrow, "font-weight:320"))
        self.assertNotEqual(preview(narrow + heavy, "font-weight:450"), preview(narrow, "font-weight:450"))

        condensed = broad[:-1] + "; font-stretch:70% 92.5%}"
        expanded = heavy[:-1] + "; font-width:92.75% 127.5%}"
        for fixed in (False, True):
            with self.subTest(width=True, fixed=fixed):
                request = "font-width:92.625%; font-weight:600"
                self.assertEqual(preview(condensed + expanded, request, fixed=fixed),
                                 preview(condensed, request, fixed=fixed))
                request = "font-width:120%; font-stretch:92.75%; font-weight:600"
                self.assertEqual(preview(expanded + condensed, request, fixed=fixed),
                                 preview(expanded, request, fixed=fixed))
                self.assertNotEqual(preview(expanded + condensed, request, fixed=fixed),
                                    preview(condensed, request, fixed=fixed))

        primary = broad[:-1] + '; unicode-range:U+0041; font-variation-settings:"wght" 300}'
        second = broad.replace("Sample", "Second")[:-1] + '; unicode-range:U+0042; font-variation-settings:"wght" 800}'
        neutral = primary.replace('; font-variation-settings:"wght" 300', '') + second.replace('; font-variation-settings:"wght" 800', '')
        control = '<span style="font-variation-settings:\'wght\' 300">A</span><span style="font-variation-settings:\'wght\' 800">B</span>'
        self.assertEqual(preview(primary + second), preview(neutral, text=control))
        self.assertNotEqual(preview(primary + second), preview(neutral))

    def test_xhtml_oblique_ranges_match_before_weight_and_preserve_inline_angles(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        wide = font_with_extra_advance(font)

        def face(file: str, slope: str, extra: str = "", family: str = "Sample") -> str:
            return f"@font-face{{font-family:{family};src:url({file});font-style:{slope};{extra}}}"

        def preview(definitions: str, request: str, text: str = "Slant sample", *, fixed: bool = False) -> bytes:
            viewport = '<meta name="viewport" content="width=420,height=100"/>' if fixed else ""
            chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Oblique</title>{viewport}
            <style>{definitions}
            body {{margin:0;font-family:Sample;font-size:36px;line-height:56px;{request}}}
            p {{margin:0}}</style></head><body><p>{text}</p></body></html>"""
            if fixed:
                source = fixed_spread_fixture([("", "white", 420, 100)])
                files = {"OEBPS/s0.xhtml": chapter}
            else:
                source = epub_fixture(chapter_one=chapter)
                files = {}
            files.update({"OEBPS/book.ttf": font, "OEBPS/wide.ttf": wide})
            with doc_ex.open(rewrite_epub(source, files)) as book:
                image = book.render_page(0, dpi=96)
                if fixed:
                    with doc_ex.open(book.export_epub_fixed_pdf(dpi=96)) as pdf:
                        self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))),
                                         rgba_pixels(png_chunks(image)))
                return image

        shallow = face("book.ttf", "oblique 3deg 8deg")
        steep = face("wide.ttf", "oblique 28deg 16deg")
        for fixed in (False, True):
            with self.subTest(fixed=fixed):
                self.assertNotEqual(preview(shallow, "font-style:oblique 9deg", fixed=fixed),
                                    preview(steep, "font-style:oblique 9deg", fixed=fixed))
                for request, winner in (
                    ("font-style:oblique 9deg", shallow),
                    ("font-style:oblique 10.99deg", shallow),
                    ("font-style:oblique 11deg", steep),
                    ("--angle:oblique 12deg;font-style:var(--angle)", steep),
                    ("font-style:Oblique /* comment */ 10grad", shallow),
                    ("font-style:oblique 0.04turn", steep),
                    ("font-style:oblique 9deg;font-style:oblique 10deg 20deg", shallow),
                    ("font-style:oblique;font-style:oblique 91deg", steep),
                ):
                    self.assertEqual(preview(shallow + steep, request, fixed=fixed),
                                     preview(winner, request, fixed=fixed))

        back_shallow = face("book.ttf", "oblique -8deg -3deg")
        back_steep = face("wide.ttf", "oblique -28deg -16deg")
        for request, winner in (("oblique -9deg", back_shallow), ("oblique -12deg", back_steep)):
            property = f"font-style:{request}"
            self.assertEqual(preview(back_shallow + back_steep, property), preview(winner, property))

        italic = face("book.ttf", "italic")
        self.assertEqual(preview(steep + italic, "font-style:italic"), preview(italic, "font-style:italic"))
        auto = face("book.ttf", "oblique 16deg 20deg", "font-style:auto")
        self.assertEqual(preview(auto + steep, "font-style:normal"),
                         preview(face("book.ttf", "normal") + steep, "font-style:normal"))
        # A matching style takes precedence over a closer weight. Width still comes first.
        heavy = face("wide.ttf", "oblique 2deg 8deg", "font-weight:700")
        exact = face("book.ttf", "oblique 16deg 22deg", "font-weight:400")
        request = "font-style:oblique 18deg;font-weight:700"
        self.assertEqual(preview(heavy + exact, request), preview(exact, request))
        narrow = face("wide.ttf", "oblique 2deg 8deg", "font-width:75%;font-weight:700")
        request += ";font-width:75%"
        self.assertEqual(preview(narrow + exact, request), preview(narrow, request))

        upright = face("book.ttf", "normal")
        variable_slope = face("wide.ttf", "oblique -5deg 5deg", "font-weight:700")
        for slope in ("normal", "oblique 0deg"):
            request = f"font-style:{slope};font-weight:700"
            self.assertEqual(preview(upright + variable_slope, request),
                             preview(variable_slope, request))
            self.assertNotEqual(preview(upright + variable_slope, request),
                                preview(upright, request))

        # Adjacent spans share other typography and still choose different named faces.
        separately_named = steep.replace("font-family:Sample", "font-family:Wide")
        mixed = '<span style="font-style:oblique 8deg">A</span><span style="font-style:oblique 18deg">B</span>C'
        control = mixed.replace("font-style:oblique 18deg", "font-style:oblique 18deg;font-family:Wide")
        self.assertEqual(preview(shallow + steep, "", mixed),
                         preview(shallow + separately_named, "", control))
        self.assertNotEqual(preview(shallow + steep, "", mixed),
                            preview(shallow + steep, "font-style:oblique 8deg", "ABC"))

    def test_xhtml_synthetic_oblique_keeps_signed_fractional_angles(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()

        def preview(slope: str, text: str = "H", *, face: str = "normal", fixed: bool = False,
                    export: bool = False) -> tuple[int, bytes]:
            chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Slant</title>
            <meta name="viewport" content="width=320,height=128"/>
            <style>@font-face{{font-family:Sample;src:url(upright.ttf);font-style:{face}}}
            body {{margin:0;padding-left:64px;font-family:Sample;font-size:80px;line-height:112px;
                   letter-spacing:16px;font-style:{slope}}}
            p {{margin:0}}</style></head><body><p>{text}</p></body></html>"""
            if fixed:
                source = rewrite_epub(fixed_spread_fixture([("", "white", 320, 128)]),
                                      {"OEBPS/s0.xhtml": chapter})
            else:
                source = epub_fixture(chapter_one=chapter)
            data = rewrite_epub(source, {"OEBPS/upright.ttf": font})
            with doc_ex.open(data) as book:
                pixels = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                if export:
                    with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, searchable=True)) as pdf:
                        self.assertTrue(pixels == rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))))
                        self.assertEqual(pdf[0].get_text(), "H")
                return pixels

        def ink_x(pixels: tuple[int, bytes]) -> float:
            width, rgba = pixels
            positions = [index // 4 % width for index in range(0, len(rgba), 4) if rgba[index] < 128]
            return sum(positions) / len(positions)

        for fixed in (False, True):
            with self.subTest(fixed=fixed):
                upright = preview("oblique 0deg", fixed=fixed)
                left = preview("oblique -0.75deg", fixed=fixed)
                right = preview("oblique 0.75deg", fixed=fixed, export=fixed)
                self.assertLess(ink_x(left), ink_x(upright))
                self.assertLess(ink_x(upright), ink_x(right))
                self.assertTrue(preview("oblique 13.25deg", fixed=fixed)
                                != preview("oblique 13.75deg", fixed=fixed))

        mixed = '<span style="font-style:oblique 0.25deg">H</span><span style="font-style:oblique -0.75deg">H</span>'
        self.assertTrue(preview("normal", mixed) != preview("normal", mixed.replace("-0.75", "0.25")))
        oblique_face = "oblique -2deg 2deg"
        self.assertTrue(preview("oblique 0.75deg", face=oblique_face) == preview("oblique 0deg", face=oblique_face))
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "synthetic oblique"):
            preview("oblique 90deg")

    def test_epub_svg_variable_axes_change_rendered_text(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "InterVariable.ttf").read_bytes()

        def preview(settings: str, optical: str = "none") -> bytes:
            svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 350 100">
              <style>@font-face {{font-family: Local Variable; src: url(variable.ttf)}}
                text {{font-family: Local Variable; font-size: 40px;
                       font-optical-sizing: {optical}; font-variation-settings: {settings}}}</style>
              <text x="10" y="60">Variable glyphs</text></svg>'''
            chapter = doc_ex.EpubSvgChapterSource("Axes", svg, "OEBPS/Art/axes.svg")
            resource = doc_ex.EpubResource("font/ttf", font)
            with doc_ex.open(epub_fixture()) as original:
                data = original.edit_epub_structure(
                    [chapter, 0], resources={"OEBPS/Art/variable.ttf": resource},
                )
            with doc_ex.open(data) as book:
                return book.render_page(0, dpi=96)

        default = preview('"wght" 400')
        bold = preview('"wght" 900')
        self.assertNotEqual(default, bold)
        self.assertEqual(default, preview("initial"))
        self.assertEqual(default, preview("normal", optical="NONE"))
        self.assertNotEqual(default, preview("normal", optical="initial"))

    def test_epub_svg_variable_face_ranges_and_defaults_follow_selected_runs(self) -> None:
        fonts = Path(__file__).parent / "fixtures" / "fonts"
        variable = (fonts / "InterVariable.ttf").read_bytes()
        regular = (fonts / "Inter-Regular.ttf").read_bytes()
        wide = font_with_extra_advance(regular)
        files = {
            "OEBPS/Art/variable.ttf": doc_ex.EpubResource("font/ttf", variable),
            "OEBPS/Art/regular.ttf": doc_ex.EpubResource("font/ttf", regular),
            "OEBPS/Art/wide.ttf": doc_ex.EpubResource("font/ttf", wide),
        }

        def face(file: str, descriptors: str = "", family: str = "Author") -> str:
            return f"@font-face{{font-family:{family};src:url({file});{descriptors}}}"

        def preview(definitions: str, style: str = "", text: str = "AABBBAA", *,
                    verify_pdf: bool = False, length: str = "") -> bytes:
            style = style.replace('"', "&quot;")
            svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 420 100">
            <style>{definitions}</style><g style="font-family:Author;font-size:40px;font-optical-sizing:none;{style}">
            <text x="10" y="62" {length}>{text}</text></g></svg>"""
            chapter = doc_ex.EpubSvgChapterSource("Variable faces", svg, "OEBPS/Art/axes.svg")
            with doc_ex.open(epub_fixture()) as book:
                data = book.edit_epub_structure([chapter, 0], resources=files)
            with doc_ex.open(data) as book:
                image = book.render_page(0, dpi=96)
                if verify_pdf:
                    with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0], searchable=True)) as pdf:
                        self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))),
                                         rgba_pixels(png_chunks(image)))
                        self.assertEqual(pdf[0].get_text(), "AABBBAA")
                return image

        plain = face("variable.ttf")
        ranged = face("variable.ttf", "font-weight:300 750")
        inside = preview(ranged, "font-weight:612.5", verify_pdf=True)
        self.assertEqual(inside, preview(plain, 'font-variation-settings:"wght" 612.5'))
        self.assertNotEqual(inside, preview(plain, 'font-variation-settings:"wght" 612'))
        self.assertEqual(preview(ranged, "font-weight:910"), preview(plain, 'font-variation-settings:"wght" 750'))
        self.assertEqual(preview(ranged, "font-weight:200"), preview(plain, 'font-variation-settings:"wght" 300'))

        defaults = face("variable.ttf", 'font-weight:300 750;font-variation-settings:"wght" 530,"opsz" 24')
        self.assertEqual(preview(defaults, "font-weight:630"),
                         preview(plain, 'font-variation-settings:"wght" 530,"opsz" 24'))
        self.assertEqual(preview(defaults, 'font-weight:630;font-variation-settings:"wght" 720'),
                         preview(plain, 'font-variation-settings:"wght" 720,"opsz" 24'))
        self.assertEqual(preview(defaults, "font-optical-sizing:auto"),
                         preview(plain, 'font-optical-sizing:auto;font-variation-settings:"wght" 530'))

        first = face("variable.ttf", 'unicode-range:U+0041;font-variation-settings:"wght" 350')
        second = face("variable.ttf", 'unicode-range:U+0042;font-variation-settings:"wght" 720')
        runs = ('<tspan style="font-variation-settings:&quot;wght&quot; 350">AA</tspan>'
                '<tspan style="font-variation-settings:&quot;wght&quot; 720">BBB</tspan>'
                '<tspan style="font-variation-settings:&quot;wght&quot; 350">AA</tspan>')
        self.assertEqual(preview(first + second), preview(plain, text=runs))
        self.assertNotEqual(preview(first + second), preview(plain, 'font-variation-settings:"wght" 350'))

        explicit = 'textLength="230" lengthAdjust="spacingAndGlyphs"'
        self.assertEqual(preview(ranged, "font-weight:612.5", length=explicit),
                         preview(plain, 'font-variation-settings:"wght" 612.5', length=explicit))
        fitted = preview(first + second, length=explicit, verify_pdf=True)
        self.assertNotEqual(fitted, preview(first + second))
        local = '<tspan style="font-variation-settings:&quot;wght&quot; 720">AABBBAA</tspan>'
        self.assertEqual(preview(defaults, text=local),
                         preview(plain, 'font-variation-settings:"wght" 720,"opsz" 24'))

        condensed = face("regular.ttf", "font-width:74.5% 90.25%;font-style:oblique 14deg 25deg;font-weight:300 500")
        expanded = face("wide.ttf", "font-width:90.75% 120%;font-style:oblique 3deg 8deg;font-weight:500 750")
        style = "font-width:90.5%;font-style:oblique 18deg;font-weight:630"
        self.assertEqual(preview(condensed + expanded, style), preview(condensed, style))
        style = "font-width:90.9%;font-style:oblique 18deg;font-weight:630"
        self.assertEqual(preview(condensed + expanded, style), preview(expanded, style))
        style = "font-style:oblique 12deg;font-weight:630"
        self.assertEqual(preview(condensed.replace("font-width:74.5% 90.25%;", "") +
                                 expanded.replace("font-width:90.75% 120%;", ""), style),
                         preview(condensed.replace("font-width:74.5% 90.25%;", ""), style))

    def test_epub_svg_mixed_face_text_length_preserves_ink_search_and_link_geometry(self) -> None:
        fonts = Path(__file__).parent / "fixtures" / "fonts"
        resources = {
            "OEBPS/Art/variable.ttf": doc_ex.EpubResource(
                "font/ttf", (fonts / "InterVariable.ttf").read_bytes(),
            ),
        }

        def preview(
            length: str, *, inline: bool = False, positions: str = "144",
        ) -> tuple[list[int], tuple[float, ...], tuple[float, ...]]:
            svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 100">
              <style>
                @font-face {{font-family:Layout;src:url(Art/variable.ttf);unicode-range:U+00E9;
                             font-variation-settings:"wght" 340}}
                @font-face {{font-family:Layout;src:url(Art/variable.ttf);unicode-range:U+0042;
                             font-variation-settings:"wght" 760}}
                text {{font-family:Layout;font-size:32px;font-optical-sizing:none;
                       transform:translate(7 0);text-anchor:middle;fill:#9a2222}}
              </style>
              <a href="#label"><rect x="8" y="8" width="12" height="12" fill="#027180"/></a>
              <g transform="translate(9 3)">
                <text x="{positions}" y="70" {length}><a href="https://example.org/length"><tspan id="label">éBBé</tspan></a></text>
              </g>
              </svg>"""
            if inline:
                chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Text</title>
                <meta name="viewport" content="width=320,height=100"/>
                <style>body {{margin:0}} svg {{width:320px;height:100px}}</style>
                </head><body>{svg}</body></html>"""
                source = rewrite_epub(fixed_spread_fixture([("", "white", 320, 100)]),
                                      {"OEBPS/s0.xhtml": chapter})
                with doc_ex.open(source) as original:
                    data = original.edit_epub_structure([0], resources=resources)
                path, original_xml = "OEBPS/s0.xhtml", chapter
            else:
                raw_svg = svg.replace("url(Art/variable.ttf)", "url(variable.ttf)")
                chapter = doc_ex.EpubSvgChapterSource("Run length", raw_svg, "OEBPS/Art/text.svg")
                with doc_ex.open(epub_fixture()) as original:
                    data = original.edit_epub_structure([chapter, 0], resources=resources)
                path, original_xml = "OEBPS/Art/text.svg", raw_svg
            with doc_ex.open(data) as book:
                self.assertEqual(book.read_epub_resource(path), original_xml.encode())
                width, pixels = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                exported = book.export_epub_fixed_pdf(dpi=96, pages=[0], searchable=True)
            with doc_ex.open(exported) as pdf, pdf.edit() as editor:
                self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))), (width, pixels))
                self.assertEqual(pdf[0].get_text(), "éBBé")
                links = pdf[0].get_links()
                local, = [link for link in links if link.metadata.get("target_page") is not None]
                self.assertEqual(local.metadata["target_page"], 0)
                links = [link for link in links if link.metadata.get("target_page") is None]
                self.assertEqual({link.uri for link in links}, {"https://example.org/length"})
                bounds = (min(link.bbox[0] for link in links), min(link.bbox[1] for link in links),
                          max(link.bbox[2] for link in links), max(link.bbox[3] for link in links))
                search, = pdf[0].search_for("éBBé")
                for edge in (0, 2):
                    self.assertAlmostEqual(search[edge], bounds[edge], delta=1)
                destination, = re.findall(
                    rb"/XYZ\s+([1-9][0-9.]*)\s+([0-9.]+)\s+null", editor.to_bytes(object_streams=False),
                )
                self.assertAlmostEqual(float(destination[0]), bounds[0], delta=1)
                self.assertAlmostEqual(float(destination[1]), 75 - bounds[1], delta=1)
            ink = [index // 4 % width for index in range(0, len(pixels), 4)
                   if pixels[index] < 200 and pixels[index + 1] < 80 and pixels[index + 3] > 200]
            return ink, bounds, tuple(search)

        for inline in (False, True):
            with self.subTest(inline=inline):
                narrow, small_link, small_search = preview(
                    'textLength="90" lengthAdjust="spacingAndGlyphs"', inline=inline,
                )
                wide, large_link, large_search = preview(
                    'textLength="180px" lengthAdjust="spacingAndGlyphs"', inline=inline,
                )
                self.assertAlmostEqual(max(wide) - min(wide), 2 * (max(narrow) - min(narrow)), delta=2)
                self.assertAlmostEqual(max(wide) + min(wide), max(narrow) + min(narrow), delta=2)
                self.assertAlmostEqual((max(wide) + min(wide)) / 2, 160, delta=2)
                for small, large in ((small_link, large_link), (small_search, large_search)):
                    self.assertAlmostEqual(large[2] - large[0], 2 * (small[2] - small[0]), delta=1)
                    self.assertAlmostEqual(large[1], small[1], delta=0.75)
                    self.assertAlmostEqual(large[3], small[3], delta=0.75)

        def ink_groups(ink: list[int]) -> list[tuple[int, int]]:
            groups = []
            for x in sorted(set(ink)):
                if groups and x == groups[-1][1] + 1:
                    groups[-1] = (groups[-1][0], x)
                else:
                    groups.append((x, x))
            return groups

        for inline in (False, True):
            with self.subTest(inline=inline, adjust="spacing"):
                default = preview('textLength="84"', inline=inline)
                explicit = preview('textLength="84" lengthAdjust="spacing"', inline=inline)
                self.assertEqual(default, explicit)
                loose = preview('textLength="90px" lengthAdjust="spacing"', inline=inline)
                close_ink, close_link, close_search = default
                loose_ink, loose_link, loose_search = loose
                close_groups, loose_groups = ink_groups(close_ink), ink_groups(loose_ink)
                self.assertEqual((len(close_groups), len(loose_groups)), (4, 4))
                for before, after, dx in zip(close_groups, loose_groups, (-3, -1, 1, 3)):
                    self.assertEqual(after, (before[0] + dx, before[1] + dx))
                for before, after in ((close_link, loose_link), (close_search, loose_search)):
                    for edge, dx in enumerate((-2.25, 0, 2.25, 0)):
                        self.assertAlmostEqual(after[edge], before[edge] + dx, delta=0.75)

        _, natural_link, natural_search = preview("")
        _, compressed_link, compressed_search = preview('textLength="70"')
        for natural, compressed in ((natural_link, compressed_link), (natural_search, compressed_search)):
            self.assertLess(compressed[2] - compressed[0], natural[2] - natural[0] - 5)
            self.assertAlmostEqual(compressed[2] + compressed[0], natural[2] + natural[0], delta=0.5)

        with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "authored letter or word spacing"):
            preview('textLength="84" style="letter-spacing:1px"')
        with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "spacing and glyph geometry"):
            preview('textLength="3" lengthAdjust="spacing"')
        with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "invalid lengthAdjust"):
            preview('textLength="84" lengthAdjust="scale"')
        with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "per-glyph positioning"):
            preview('textLength="180" lengthAdjust="spacingAndGlyphs" rotate="15"')
        with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "positioned native chunk"):
            preview('textLength="180" lengthAdjust="spacingAndGlyphs"', positions="144 170")
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "geometry budget"):
            preview('textLength="1000000" lengthAdjust="spacingAndGlyphs"')

    def test_epub_svg_use_instances_keep_inherited_variable_font_requests(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "InterVariable.ttf").read_bytes()
        resources = {
            "OEBPS/Art/regular.ttf": doc_ex.EpubResource("font/ttf", font),
            "OEBPS/Art/wide.ttf": doc_ex.EpubResource("font/ttf", font_with_extra_advance(font)),
        }
        rules = """@font-face { font-family: Local; src: url(regular.ttf);
            font-weight: 200 800; font-width: 74.5% 90.25%; font-style: oblique -24deg -8deg;
            font-variation-settings: "opsz" 24; }
          @font-face { font-family: Local; src: url(regular.ttf);
            font-weight: 200 800; font-width: 90.75% 120%; font-style: oblique -24deg -8deg;
            font-variation-settings: "opsz" 29; }
          @font-face { font-family: Local; src: url(wide.ttf);
            font-weight: 200 800; font-width: 90.75% 120%; font-style: oblique 9deg 28deg;
            font-variation-settings: "opsz" 30; }
          .first { font-family: Local; font-weight: 312.5; font-width: 90.5%; font-style: oblique -17.25deg; }
          .second { font-family: Local; font-weight: 712.75; font-width: 91.25%; font-style: oblique 17.5deg;
              font-variation-settings: "opsz" 18; }
          .third { font-family: serif; font-weight: 400; }
          .face text { fill: #245397; }"""
        label = '''<text x="5" y="34" font-size="24" style="font-optical-sizing:none">
          <a href="https://example.org/badge">Badge</a><tspan id="target" x="112" font-weight="630">Tag</tspan>
          </text>'''
        reused = f'''<defs><g style="font-family:Local;font-weight:800">
          <g id="face" class="face">{label}</g></g>
          <use id="nested" href="#face" x="7" y="3"/>
          <symbol id="symbol" viewBox="0 0 200 60"><use href="#nested"/></symbol>
          </defs>
          <use class="first" xlink:href="#nested" x="10" y="8"/>
          <use class="second" href="#symbol" x="10" y="72" width="200" height="60"/>
          <use class="third" href="#nested" x="10" y="136"/>
          <a href="#target"><rect x="230" y="15" width="15" height="12"/></a>'''
        direct = f'''<defs><symbol id="expected" viewBox="0 0 200 60">
          <g class="second face" transform="translate(7 3)">{label.replace("target", "second")}</g>
          </symbol></defs>
          <g class="first face" transform="translate(17 11)">{label.replace("target", "first")}</g>
          <use href="#expected" x="10" y="72" width="200" height="60"/>
          <g class="third face" transform="translate(17 139)">{label.replace("target", "third")}</g>
          <a href="#first"><rect x="230" y="15" width="15" height="12"/></a>'''

        def preview(markup: str, sheet: str = rules, *, verify: bool = False) -> bytes:
            svg = f'''<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
              viewBox="0 0 260 200"><style>{sheet}</style>{markup}</svg>'''
            chapter = doc_ex.EpubSvgChapterSource("Reused variable fonts", svg, "OEBPS/Art/badges.svg")
            with doc_ex.open(epub_fixture()) as original:
                data = original.edit_epub_structure([chapter, 0], resources=resources)
            with doc_ex.open(data) as book:
                image = book.render_page(0, dpi=96)
                self.assertEqual(book.read_epub_resource("OEBPS/Art/badges.svg"), svg.encode())
                if verify:
                    with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0], searchable=True)) as pdf:
                        self.assertEqual(pdf[0].get_text().count("Badge"), 3)
                        links = pdf[0].get_links()
                        rows = {round((link.bbox[1] - 8) / 48) for link in links
                                if link.uri == "https://example.org/badge"}
                        self.assertEqual(rows, {0, 1, 2})
                        self.assertTrue(any(link.metadata.get("target_page") == 0 for link in links))
                        self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))),
                                         rgba_pixels(png_chunks(image)))
                return image

        image = preview(reused, verify=True)
        self.assertEqual(image, preview(direct))
        self.assertNotEqual(image, preview(reused, rules.replace("712.75", "315.25")))
        self.assertNotEqual(image, preview(reused, rules.replace("91.25%", "90.5%")))
        self.assertNotEqual(image, preview(reused, rules.replace("17.5deg", "-17.5deg")))

        variables = rules.split(".first", 1)[0] + """
          .first { --family: Local; --weight: 312.5; --width: 90.5%; --slope: -17.25deg; }
          .second { --family: Local; --weight: 712.75; --width: 91.25%; --slope: 17.5deg;
              --axes: "opsz" 18; }
          .face { font-family: var(--family, serif); font-weight: var(--weight, 400);
              font-stretch: var(--width, normal); font-style: var(--style, oblique var(--slope, 0deg));
              font-variation-settings: var(--axes, normal); }
          .third { --style: normal; }
          .face text { fill: #245397; }"""
        self.assertEqual(preview(reused, variables, verify=True), image)

    def test_svg_use_custom_properties_inherit_per_instance_and_keep_links(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" width="158" height="42" viewBox="0 0 158 42">
          <style>
            .first { --ink: #ca2020; --stroke: #205080; }
            .second { --ink: #20aa38 !important; }
            .third { --ink: var(--loop); --loop: var(--ink); --fallback: #6340ba; }
            .face rect { fill: var(--shade, #e2a020); stroke: var(--stroke, #9010aa); stroke-width: 2; }
            .definition .face rect { --shade: #111 !important; --stroke: #111 !important; stroke-width: 10; }
            .face:first-child { --override: #dea020; }
          </style>
          <defs>
            <g class="definition" style="--shade:#111;--stroke:#111">
              <rect width="1" height="1"/>
              <g id="face" class="face">
                <defs><linearGradient id="palette">
                  <stop offset="0" style="stop-color:var(--shade,#e2a020)"/>
                  <stop offset="1" style="stop-color:var(--shade,#e2a020)"/>
                </linearGradient></defs>
                <rect x="2" y="2" width="25" height="20"/>
                <rect x="2" y="26" width="25" height="10" style="--shade:var(--override,#111)"/>
                <rect x="31" y="2" width="5" height="20" style="stroke:none;fill:var(--gradient,url(#palette))"/>
              </g>
            </g>
            <use id="nested" href="#face" x="3" style="--shade:var(--ink,var(--fallback,#304080))"/>
            <symbol id="badge" viewBox="0 0 36 42"><use href="#nested"/></symbol>
          </defs>
          <a href="https://example.org/badge"><use class="first" href="#nested" x="2"/></a>
          <a href="https://example.org/badge"><use class="second" href="#badge" x="41" width="36" height="42"
            style="--ink:#b00010"/></a>
          <a href="https://example.org/badge"><use class="third" href="#nested" x="80"/></a>
          <a href="https://example.org/badge"><use href="#face" x="122"/></a>
        </svg>"""
        fixture = fixed_spread_fixture([("", "white", 158, 42)])
        with zipfile.ZipFile(io.BytesIO(fixture)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )
        xhtml = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Instance colors</title>
            <meta name="viewport" content="width=158,height=42"/></head>
            <body style="margin:0">{svg}</body></html>"""
        versions = (
            rewrite_epub(fixture, {"OEBPS/content.opf": package, "OEBPS/s0.svg": svg}),
            rewrite_epub(fixture, {"OEBPS/s0.xhtml": xhtml}),
        )
        for index, data in enumerate(versions):
            with self.subTest(inline=bool(index)), doc_ex.open(data) as book:
                self.assertEqual(book.read_epub_resource("OEBPS/s0.xhtml" if index else "OEBPS/s0.svg"),
                                 (xhtml if index else svg).encode())
                width, rgba = preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))

                def pixel(x: int, y: int) -> tuple[int, ...]:
                    offset = 4 * (y * width + x)
                    return tuple(rgba[offset:offset + 3])

                self.assertEqual([pixel(x, 10) for x in (12, 52, 92, 130)],
                                 [(202, 32, 32), (32, 170, 56), (99, 64, 186), (226, 160, 32)])
                self.assertEqual([pixel(x, 30) for x in (12, 52, 92, 130)], [(222, 160, 32)] * 4)
                self.assertEqual([pixel(x, 10) for x in (38, 76, 116, 155)],
                                 [(202, 32, 32), (32, 170, 56), (99, 64, 186), (226, 160, 32)])
                self.assertEqual(pixel(7, 10), (32, 80, 128))
                self.assertEqual(pixel(46, 10), (144, 16, 170))
                with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0])) as pdf:
                    links = pdf[0].get_links()
                    self.assertEqual({link.uri for link in links}, {"https://example.org/badge"})
                    columns = {round(link.bbox[0] / 30) for link in links}
                    self.assertEqual(columns, {0, 1, 2, 3})
                    self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))), preview)

    def test_epub_svg_use_instance_requests_share_the_font_stack_limit(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "InterVariable.ttf").read_bytes()
        instances = "".join(
            f'<use href="#label" style="font-family:Local;font-weight:{300 + i / 10:.1f}"/>'
            for i in range(513)
        )
        svg = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 40">
          <style>@font-face {{font-family:Local;src:url(variable.ttf);font-weight:200 800}}</style>
          <defs><text id="label" x="5" y="25">Label</text></defs>{instances}</svg>'''
        chapter = doc_ex.EpubSvgChapterSource("Font instance limits", svg, "OEBPS/Art/instances.svg")
        resources = {"OEBPS/Art/variable.ttf": doc_ex.EpubResource("font/ttf", font)}
        with doc_ex.open(epub_fixture()) as book:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "512 distinct stacks"):
                book.edit_epub_structure([chapter, 0], resources=resources)

    def test_epub_svg_font_ranges_and_stacks_preserve_preferred_glyphs(self) -> None:
        inter = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        wide = font_with_extra_advance(inter)
        resources = {
            "OEBPS/Art/inter.ttf": doc_ex.EpubResource("font/ttf", inter),
            "OEBPS/Art/wide.ttf": doc_ex.EpubResource("font/ttf", wide),
        }
        standard_faces = """
          @font-face {font-family: Standard; src: url(inter.ttf)}
          @font-face {font-family: "Other, Face"; src: url(wide.ttf)}
          @font-face {font-family: "monospace"; src: url(wide.ttf)}
          @font-face {font-family: Unused; src: url(not-packaged.ttf)}
        """

        def fixed(css: str, text: str) -> tuple[bytes, bytes]:
            svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 360 140">
              <style>{standard_faces} text {{font-size:34px;font-kerning:none}} {css}</style>
              {text}</svg>"""
            chapter = doc_ex.EpubSvgChapterSource("Typography", svg, "OEBPS/Art/fonts.svg")
            with doc_ex.open(epub_fixture()) as original:
                result = original.edit_epub_structure([chapter, 0], resources=resources)
            with doc_ex.open(result) as book:
                self.assertEqual(book.read_epub_resource("OEBPS/Art/fonts.svg"), svg.encode())
                png = book.render_page(0, dpi=96)
            return result, png

        broad = "@font-face {font-family: Composite; src: url(inter.ttf); unicode-range: U+004?}"
        narrow = "@font-face {font-family: Composite; src: url(wide.ttf); unicode-range: U+0042}"
        composite = '<text x="10" y="48" font-family="Composite, Standard">AAAABBBBX</text>'
        spans = """<text x="10" y="48" font-family="Standard">AAAA<tspan
          font-family="'Other, Face'">BBBB</tspan>X</text>"""
        encoded, actual = fixed(broad + narrow, composite)
        _, expected = fixed("", spans)
        _, reversed_rules = fixed(narrow + broad, composite)
        self.assertEqual(actual, expected)
        self.assertNotEqual(actual, reversed_rules)
        with doc_ex.open(encoded) as book, doc_ex.open(book.export_epub_fixed_pdf(pages=[0], dpi=96)) as pdf:
            self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))),
                             rgba_pixels(png_chunks(actual)))

        first = "@font-face {font-family: First; src: url(inter.ttf); unicode-range: U+0041}"
        stacks = """<g style="font-family:First, &quot;Other, Face&quot;">
          <text x="10" y="48" font-family="inherit">AABBBAA</text></g>
          <text x="10" y="108" font-family="First, Standard">AABBBAA</text>"""
        _, stacked = fixed(first, stacks)
        _, control = fixed("", """<text x="10" y="48" font-family="Standard">AA<tspan
          font-family="'Other, Face'">BBB</tspan>AA</text>
          <text x="10" y="108" font-family="Standard">AABBBAA</text>""")
        self.assertEqual(stacked, control)

        # A serif ligature followed by an uncovered letter must preserve both font runs.
        actual_ligature = """<text x="10" y="48" font-family="'Source Serif 4', Standard">officeƕ</text>"""
        expected_ligature = """<text x="10" y="48" font-family="'Source Serif 4'">office<tspan
          font-family="Standard">ƕ</tspan></text>"""
        _, ligature = fixed("", actual_ligature)
        _, isolated = fixed("", expected_ligature)
        self.assertEqual(ligature, isolated)

        _, quoted = fixed("", '<text x="10" y="48" font-family="&quot;monospace&quot;, monospace">WW</text>')
        _, named = fixed("", '<text x="10" y="48" font-family="&quot;Other, Face&quot;">WW</text>')
        _, generic = fixed("", '<text x="10" y="48" font-family="monospace, &quot;monospace&quot;">WW</text>')
        self.assertEqual(quoted, named)
        self.assertNotEqual(quoted, generic)

        widths = """@font-face {font-family: Widths; src:url(inter.ttf); font-stretch:condensed}
          @font-face {font-family: Widths; src:url(wide.ttf); font-width:expanded}"""
        expanded = '<text x="10" y="48" font-family="Widths" font-stretch="expanded">WW</text>'
        _, stretched = fixed(widths, expanded)
        self.assertEqual(stretched, named)
        _, condensed = fixed(widths, expanded.replace('stretch="expanded"', 'stretch="condensed"'))
        self.assertNotEqual(condensed, stretched)

    def test_epub_svg_mixed_font_inline_export_matches_explicit_runs(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        def chapter(text: str) -> bytes:
            xhtml = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head>
              <title>Inline fonts</title></head><body><p>
              <svg xmlns="http://www.w3.org/2000/svg" width="260" height="90" viewBox="0 0 260 90">
              <style>@font-face {{font-family: Standard; src: url(inter.ttf);
                unicode-range: U+0195}} text {{font-size:42px;fill:#14558a}}</style>
              {text}</svg></p></body></html>"""
            return rewrite_epub(epub_fixture(chapter_one=xhtml), {"OEBPS/inter.ttf": font})

        actual = chapter("""<text x="8" y="58" font-family="'Source Serif 4', Standard">officeƕ</text>""")
        expected = chapter("""<text x="8" y="58" font-family="'Source Serif 4'">office<tspan
          font-family="Standard">ƕ</tspan></text>""")
        with doc_ex.open(actual) as book, doc_ex.open(expected) as control:
            self.assertEqual(book.render_page(0, dpi=96), control.render_page(0, dpi=96))
            for target in ("pdf", "docx"):
                with doc_ex.open(book.convert(target)) as output, doc_ex.open(control.convert(target)) as reference:
                    png = output.render_page(0, dpi=96)
                    self.assertEqual(png, reference.render_page(0, dpi=96))
                    self.assertIn(bytes.fromhex("14558aff"), rgba_pixels(png_chunks(png))[1])

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
            replacement = book.edit_epub_structure(resources={
                "OEBPS/Typeface.ttf": doc_ex.EpubResource("font/ttf", font + b"\0" * 4),
            })
            with self.assertRaises(ValueError):
                book.edit_epub_structure(resources={
                    "OEBPS/Typeface.ttf": doc_ex.EpubResource("font/ttf", wrapped),
                })
            self.assertEqual(book.read_epub_resource("OEBPS/Typeface.ttf"), wrapped)
        with doc_ex.open(revision) as edited:
            self.assertEqual(edited.render_page(1, dpi=96), preview)
        with doc_ex.open(replacement) as edited, zipfile.ZipFile(io.BytesIO(replacement)) as archive:
            self.assertEqual(edited.read_epub_resource("OEBPS/Typeface.ttf"), wrapped + b"\0" * 4)
            self.assertEqual(edited.render_page(0, dpi=96), preview)
            self.assertEqual(archive.read("META-INF/encryption.xml"), encryption.encode())
        for disallowed in (
            encryption.replace("OEBPS/Typeface.ttf", "OEBPS/one.xhtml"),
            encryption.replace("http://www.idpf.org/2008/embedding", "http://www.w3.org/2001/04/xmlenc#aes128-cbc"),
        ):
            with self.subTest(encryption=disallowed), self.assertRaises(doc_ex.UnsupportedFormatError):
                doc_ex.open(rewrite_epub(encoded, {"META-INF/encryption.xml": disallowed}))

    def test_remove_obfuscated_fonts_updates_only_matching_declarations(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head>
        <link rel="stylesheet" href="styles.css"/>
        </head><body><p>Obfuscated font</p></body></html>"""
        source = epub_fixture(chapter_one=chapter)
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b"</manifest>",
                b'<item id="css" href="styles.css" media-type="text/css"/>'
                b'<item id="active" href="Fonts/Active%20Face.ttf" media-type="font/ttf"/>'
                b'<item id="unused" href="Fonts/Unused.ttf" media-type="font/ttf"/></manifest>',
            )
        active_path = "OEBPS/Fonts/Active Face.ttf"
        unused_path = "OEBPS/Fonts/Unused.ttf"
        active_uri = "OEBPS/Fonts/Active%20Face.ttf"
        font = (Path(__file__).parent / "fixtures" / "fonts" / "Inter-Regular.ttf").read_bytes()
        key = hashlib.sha1(b"urn:test:book").digest()
        wrapped = bytes(byte ^ key[index % len(key)] for index, byte in enumerate(font[:1040])) + font[1040:]
        declaration = """<x:EncryptedData><x:EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/>
        <x:CipherData><x:CipherReference URI="{}"/></x:CipherData></x:EncryptedData>"""
        first = declaration.format(active_uri)
        second = declaration.format(unused_path)
        encryption = f"""<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"
        xmlns:x="http://www.w3.org/2001/04/xmlenc#">{first}<!-- keep this comment -->{second}</encryption>"""
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/styles.css": "@font-face {font-family: Local; src: url('Fonts/Active%20Face.ttf')} p {font-family: Local}",
            active_path: wrapped, unused_path: wrapped, "META-INF/encryption.xml": encryption,
        })
        with doc_ex.open(source) as document:
            preview = document.render_page(0, dpi=96)
            with self.assertRaises(ValueError):
                document.edit_epub_structure(remove_resources=[active_path])
            retained = document.edit_epub_structure(remove_resources=[unused_path])
            self.assertEqual(document.read_epub_resource(unused_path), wrapped)
        with doc_ex.open(retained) as document, zipfile.ZipFile(io.BytesIO(retained)) as archive:
            self.assertEqual(archive.read("META-INF/encryption.xml"), encryption.replace(second, "").encode())
            self.assertNotIn(unused_path, archive.namelist())
            self.assertEqual(document.read_epub_resource(active_path), wrapped)
            self.assertEqual(document.render_page(0, dpi=96), preview)
            final = document.edit_epub_structure(
                resources={"OEBPS/styles.css": doc_ex.EpubResource("text/css", b"p {font-weight: bold}")},
                remove_resources=[active_path],
            )
        with doc_ex.open(final) as document, zipfile.ZipFile(io.BytesIO(final)) as archive:
            self.assertNotIn("META-INF/encryption.xml", archive.namelist())
            self.assertNotIn(active_path, archive.namelist())
            self.assertEqual(document[0].get_text(), "Obfuscated font")
            self.assertTrue(document.render_page(0, dpi=96).startswith(b"\x89PNG"))

    def test_embedded_and_linked_svg_render_in_chapter_viewport(self) -> None:
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:s="http://www.w3.org/2000/svg"><head>
        <meta name="viewport" content="width=180,height=100"/>
        <style>
        body {margin: 0; background: #ffeebb}
        .art {position: absolute; left: 10px; top: 20px; width: 60px; height: 40px;}
        img {position: absolute; left: 100px; top: 20px; width: 60px; height: 40px;}
        </style></head><body>
        <s:svg class="art" viewBox="0 0 30 20" image-rendering="optimizeQuality"
          shape-rendering="geometricPrecision" text-rendering="geometricPrecision"
          color-rendering="optimizeQuality"><s:title>Red rectangle</s:title>
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

    def test_embedded_epub_media_survives_chapter_edits_and_plays_audio(self) -> None:
        _, wav = narrated_epub()
        ogv = (Path(__file__).parent / "fixtures/epub-narration-theora-opus.ogv").read_bytes()
        caption = b"WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nA ticking sound\n"
        chapter_path = "OEBPS/Media/chapter.xhtml"
        audio_path = "OEBPS/Media/sample audio.wav"
        unsupported_path = "OEBPS/Media/teaser.ogg"
        video_path = "OEBPS/Media/teaser.ogv"
        caption_path = "OEBPS/Media/teaser.en.vtt"
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Highlights</title></head>
        <body><p>Start.</p>
        <audio id="sample" title="A ticking sound">
          <source src="teaser.ogg" type="audio/ogg; codecs=flac"/>
          <source src="sample%20audio.wav"/>
          <span>A written transcript.</span>
        </audio>
        <video id="teaser" aria-label="Preview" src="teaser.ogv" poster="../image.png"
            style="display:block; width:60px; height:40px">
          <track src="teaser.en.vtt" kind="captions" srclang="en" label="English" default="default"/>
          <source src="ignored-invalid-fallback.ogv"/>
        </video><epub:trigger action="pause" ref="teaser"/><p>End.</p></body></html>"""
        with doc_ex.open(epub_fixture()) as original:
            edited = original.edit_epub_structure(
                [0, doc_ex.EpubChapterSource("Highlights", xhtml, chapter_path), 1],
                resources={
                    "OEBPS/image.png": doc_ex.EpubResource("image/png", solid((255, 0, 0))),
                    audio_path: doc_ex.EpubResource("audio/wav", wav),
                    unsupported_path: doc_ex.EpubResource("audio/ogg", ogv),
                    video_path: doc_ex.EpubResource("video/ogg", ogv),
                    caption_path: doc_ex.EpubResource("text/vtt", caption),
                },
            )

        with doc_ex.open(edited) as book:
            self.assertEqual(book.epub_embedded_media(0), [])
            audio, video = book.epub_embedded_media(1)
            self.assertEqual(audio, doc_ex.EpubEmbeddedMedia(
                "audio", "sample", "A ticking sound",
                (
                    doc_ex.EpubMediaSource("teaser.ogg", unsupported_path, "audio/ogg", False),
                    doc_ex.EpubMediaSource("sample%20audio.wav", audio_path, "audio/wav", True),
                ),
                (), None,
            ))
            self.assertEqual(video.id, "teaser")
            self.assertEqual(video.poster_path, "OEBPS/image.png")
            self.assertEqual([source.path for source in video.sources], [video_path])
            self.assertEqual(video.tracks, (
                doc_ex.EpubMediaTrack(caption_path, "text/vtt", "captions", "English", "en", True),
            ))
            self.assertEqual(book.read_epub_resource(video.tracks[0].path), caption)
            self.assertIn("A written transcript.", book[1].get_text())
            self.assertEqual([image.alt for image in book[1].get_images()], ["Preview"])
            png = book.render_page(1, dpi=96)
            self.assertIn(bytes.fromhex("ff0000ff"), rgba_pixels(png_chunks(png))[1])

            chunks = list(book.epub_embedded_audio(1, 0, start_us=200_000, end_us=450_000, chunk_frames=701))
            self.assertEqual([clip.frame_count for clip in chunks], [701, 701, 598])
            self.assertTrue(all((clip.channels, clip.sample_rate) == (1, 8000) for clip in chunks))
            decoded = []
            for clip in chunks:
                with wave.open(io.BytesIO(clip.wave), "rb") as reader:
                    decoded.append(reader.readframes(clip.frame_count))
            with wave.open(io.BytesIO(wav), "rb") as reader:
                reader.setpos(1600)
                expected = b"".join(struct.pack("<h", (sample - 128) << 8) for sample in reader.readframes(2000))
                self.assertEqual(b"".join(decoded), expected)

            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "local source"):
                next(book.epub_embedded_audio(1, 0, source_index=0))
            preview = next(book.epub_embedded_video(1, 1, text_track=0))
            self.assertEqual((preview.width, preview.height), (64, 48))
            self.assertEqual([cue.text for cue in preview.text_cues], ["A ticking sound"])
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "semantic export"):
                book.convert("pdf")
            with self.assertRaisesRegex(ValueError, "still referenced"):
                book.edit_epub_structure(remove_resources=[caption_path])
            retained = book.edit_epub_chapters({1: xhtml.replace("Start.", "Revised.")})
        with doc_ex.open(retained) as book:
            self.assertEqual(book.epub_embedded_media(1), [audio, video])
            self.assertIn("Revised.", book[1].get_text())
            with self.assertRaisesRegex(ValueError, "still referenced"):
                book.edit_epub_structure([0, 2], remove_resources=[audio_path, video_path, caption_path, unsupported_path])
            cleared = book.edit_epub_chapters({
                1: '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Cleared</title></head>'
                   '<body><p>Retained chapter.</p></body></html>',
            })
        with doc_ex.open(cleared) as book:
            trimmed = book.edit_epub_structure([0, 2], remove_resources=[audio_path, video_path, caption_path, unsupported_path])
        with doc_ex.open(trimmed) as book:
            self.assertEqual(len(book), 2)
            self.assertEqual(book.epub_embedded_media(1), [])

    def test_embedded_epub_media_rejects_broken_local_references(self) -> None:
        def chapter(src: str) -> str:
            return (
                '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>One</title></head><body>'
                f'<audio id="sample" src="{src}">A transcript.</audio></body></html>'
            )

        for src, error, message in (
            ("missing.ogg", doc_ex.MalformedDocumentError, "absent from the manifest"),
            ("image.png", doc_ex.MalformedDocumentError, "type differs"),
            ("%2e%2e/audio.ogg", doc_ex.UnsafeArchiveError, "encoded EPUB parent"),
            ("image.png#xywh=0,0,1,1", doc_ex.UnsupportedFormatError, "temporal t dimension"),
            ("image.png#t=1", doc_ex.MalformedDocumentError, "type differs"),
            ("image.png#t=2,1", doc_ex.MalformedDocumentError, "end must follow"),
        ):
            with self.subTest(src=src), self.assertRaisesRegex(error, message):
                doc_ex.open(epub_fixture(chapter_one=chapter(src)))

        with doc_ex.open(epub_fixture(chapter_one=chapter("https://example.invalid/test.ogg"))) as book:
            source, = book.epub_embedded_media(0)[0].sources
            self.assertIsNone(source.path)
            self.assertIn("A transcript.", book[0].get_text())
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "local source"):
                next(book.epub_embedded_audio(0, 0))

    def test_embedded_text_tracks_split_video_at_cue_edges_and_source_fragment(self) -> None:
        resource = (Path(__file__).parent / "fixtures/epub-video-h264.mp4").read_bytes()
        _, wav = narrated_epub()
        caption = """WEBVTT English

NOTE Credit line
Notes outside cues are ignored.

REGION
id:lower

first
00:00.125 --> 00:00.176 line:90% region:lower
<v Narrator><b>One &amp; two</b></v>

00:00.160 --> 00:00.180
<ruby>字<rt>ji</rt></ruby><00:00.170> next
"""
        caption = ("\uFEFF" + caption.replace("\n", "\r\n")).encode()
        alternative = b"WEBVTT\n\n00:00.000 --> 00:01.000\nAlternative\n"
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Captions</title></head>
          <body><video src="clip.mp4#t=npt%3A00%3A00.100%2C0.23">
            <track src="alt.vtt" kind="subtitles" srclang="fr"/>
            <track src="english.vtt" kind="captions" srclang="en" default="default"/>
          </video>
          <audio src="sound.wav#t=00:00.500,600"/>
          <audio src="sound.wav#t=,0.125"/>
          <audio src="sound.wav#t=0.5"/>
          </body></html>"""
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Captions", xhtml, "OEBPS/captions.xhtml")],
                resources={
                    "OEBPS/clip.mp4": doc_ex.EpubResource("video/mp4", resource),
                    "OEBPS/sound.wav": doc_ex.EpubResource("audio/wav", wav),
                    "OEBPS/english.vtt": doc_ex.EpubResource("text/vtt", caption),
                    "OEBPS/alt.vtt": doc_ex.EpubResource("text/vtt", alternative),
                },
            )
        with doc_ex.open(data) as book:
            source, = book.epub_embedded_media(0)[0].sources
            self.assertEqual((source.path, source.fragment_start_us, source.fragment_end_us),
                             ("OEBPS/clip.mp4", 100_000, 230_000))
            default = book.epub_embedded_text(0, 0)
            self.assertEqual(default, [
                doc_ex.EpubTextCue(125_000, 176_000, "One & two", "first",
                                  "line:90% region:lower", "lower", "<v Narrator><b>One &amp; two</b></v>"),
                doc_ex.EpubTextCue(160_000, 180_000, "字 next", markup="<ruby>字<rt>ji</rt></ruby><00:00.170> next"),
            ])
            self.assertEqual([cue.text for cue in book.epub_embedded_text(0, 0, track_index=0)], ["Alternative"])
            self.assertEqual([(c.start_us, c.end_us) for c in
                              book.epub_embedded_text(0, 0, start_us=163_000, end_us=176_000)],
                             [(163_000, 176_000), (163_000, 176_000)])
            self.assertEqual(book.epub_embedded_text(0, 0, start_us=180_000), [])
            self.assertEqual(book.read_epub_resource("OEBPS/english.vtt"), caption)

            frames = list(book.epub_embedded_video(0, 0, text_track=1))
            self.assertEqual([(f.start_us, f.start_us + f.duration_us, tuple(c.text for c in f.text_cues)) for f in frames], [
                (100_000, 125_000, ()),
                (125_000, 160_000, ("One & two",)),
                (160_000, 166_666, ("One & two", "字 next")),
                (166_666, 176_000, ("One & two", "字 next")),
                (176_000, 180_000, ("字 next",)),
                (180_000, 230_000, ()),
            ])
            self.assertIs(frames[0].rgb, frames[2].rgb)
            self.assertIs(frames[3].rgb, frames[5].rgb)
            seek = list(book.epub_embedded_video(0, 0, start_us=164_000, end_us=178_000, text_track=1))
            self.assertEqual(seek[0].start_us, 164_000)
            self.assertEqual(tuple(c.text for c in seek[0].text_cues), ("One & two", "字 next"))
            self.assertEqual(seek[-1].start_us + seek[-1].duration_us, 178_000)
            self.assertEqual(list(book.epub_embedded_video(0, 0, start_us=230_000)), [])
            self.assertTrue(all(not frame.text_cues for frame in book.epub_embedded_video(0, 0)))
            self.assertEqual(sum(clip.frame_count for clip in book.epub_embedded_audio(0, 0)), 6240)
            self.assertEqual(sum(clip.frame_count for clip in book.epub_embedded_audio(0, 1)), 12_384)
            self.assertEqual(sum(clip.frame_count for clip in book.epub_embedded_audio(0, 2)), 1000)
            self.assertEqual(sum(clip.frame_count for clip in book.epub_embedded_audio(0, 3)), 12_384)
            with self.assertRaisesRegex(ValueError, "still referenced"):
                book.edit_epub_structure(remove_resources=["OEBPS/sound.wav"])

    def test_webm_media_fragment_ids_use_chapter_timing_for_video_audio_and_captions(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapters</title></head>
          <body><video src="chapter.webm#id=Caf%C3%A9%20%26%20%C3%A9tude&amp;track=Angle_Blue&amp;xywh=percent:25,25,50,50&amp;track=Voix%20%26%20cloche">
          <track src="caption.vtt" kind="captions" srclang="en"/></video>
          <video src="chapter.webm"><track src="caption.vtt" kind="captions" srclang="en"/></video>
          <video src="chapter.webm#id=101"/><video src="chapter.webm#id=infer-child"/>
          <video src="chapter.webm#id=last-child"/><video src="chapter.webm#id=tail"/>
          <video src="https://example.invalid/clip.webm#id=lost"/>
          <audio src="chapter-audio.webm#id=intro&amp;track=Spoken"/><audio src="chapter-audio.webm"/>
          </body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Chapters", chapter, "OEBPS/chapters.xhtml")],
                resources={
                    "OEBPS/chapter.webm": doc_ex.EpubResource(
                        "video/webm", (fixtures / "epub-video-webm-chapters.webm").read_bytes()),
                    "OEBPS/chapter-audio.webm": doc_ex.EpubResource(
                        "audio/webm", (fixtures / "epub-audio-webm-chapters.webm").read_bytes()),
                    "OEBPS/caption.vtt": doc_ex.EpubResource(
                        "text/vtt", b"WEBVTT\n\n00:00:00.175 --> 00:00:00.218\nMiddle\n"),
                },
            )

        def timing(frame: doc_ex.EpubVideoFrame) -> tuple[int, int, tuple[doc_ex.EpubTextCue, ...]]:
            return frame.start_us, frame.duration_us, frame.text_cues

        def bounds(frames: list[doc_ex.EpubVideoFrame]) -> tuple[int, int]:
            return frames[0].start_us, frames[-1].start_us + frames[-1].duration_us

        with doc_ex.open(data) as book:
            resources = book.epub_embedded_media(0)
            self.assertEqual(resources[0].sources[0].fragment_id, "Café & étude")
            self.assertIsNone(resources[1].sources[0].fragment_id)
            self.assertIsNone(resources[6].sources[0].path)
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "local"):
                next(book.epub_embedded_video(0, 6))
            cropped = list(book.epub_embedded_video(0, 0, text_track=0))
            full = list(book.epub_embedded_video(
                0, 1, start_us=151_001, end_us=325_001, text_track=0, video_track=1))
            self.assertEqual(bounds(cropped), (151_001, 325_001))
            self.assertEqual([timing(f) for f in cropped], [timing(f) for f in full])
            self.assertTrue(all((f.width, f.height) == (32, 20) and f.rgb[2] > 245 for f in cropped))
            self.assertEqual([(f.start_us, f.duration_us) for f in cropped if f.text_cues],
                             [(175_000, 43_000)])
            seek = list(book.epub_embedded_video(0, 0, start_us=200_000, end_us=230_000, text_track=0))
            self.assertEqual(bounds(seek), (200_000, 230_000))
            self.assertEqual(seek[0].text_cues[0].text, "Middle")
            self.assertEqual(list(book.epub_embedded_video(0, 0, start_us=325_001)), [])
            self.assertEqual(list(book.epub_embedded_audio(0, 0, start_us=325_001)), [])
            named = list(book.epub_embedded_audio(0, 0, start_us=160_000))
            ordinal = list(book.epub_embedded_audio(
                0, 1, start_us=160_000, end_us=325_001, audio_track=1))
            self.assertEqual(named, ordinal)
            self.assertEqual(sum(clip.frame_count for clip in named), 7_921)
            audio_only = list(book.epub_embedded_audio(0, 7))
            self.assertEqual(audio_only, list(book.epub_embedded_audio(0, 8, end_us=100_000, audio_track=0)))
            self.assertEqual(sum(clip.frame_count for clip in audio_only), 4_800)

            for index, expected in ((2, (0, 100_000)), (3, (330_000, 348_000)), (4, (348_000, 400_000))):
                with self.subTest(chapter=index):
                    frames = list(book.epub_embedded_video(0, index, video_track=1))
                    self.assertEqual(bounds(frames), expected)
            ending = list(book.epub_embedded_video(0, 5, video_track=1))
            self.assertEqual([timing(f) for f in ending], [timing(f) for f in
                book.epub_embedded_video(0, 1, start_us=475_000, video_track=1)])
            ending_audio = list(book.epub_embedded_audio(0, 5, end_us=2_000_000, audio_track=1))
            self.assertEqual(ending_audio, list(book.epub_embedded_audio(
                0, 1, start_us=475_000, audio_track=1)))

    def test_webm_media_fragment_ids_reject_conflicts_and_unsupported_chapters(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapters</title></head>
          <body><video src="chapter.webm#id=absent"/><video src="chapter.webm#id=duplicate"/>
          <video src="chapter.webm#id=77"/><video src="chapter.webm#id=linked"/>
          <video src="chapter.webm#id=marker"/><video src="chapter.webm#id=ordered"/>
          <video src="chapter.webm#id=reversed"/></body></html>'''
        path = "OEBPS/chapters.xhtml"
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Chapters", chapter, path)],
                resources={
                    "OEBPS/chapter.webm": doc_ex.EpubResource(
                        "video/webm", (fixtures / "epub-video-webm-chapters.webm").read_bytes()),
                    "OEBPS/movie.mp4": doc_ex.EpubResource(
                        "video/mp4", (fixtures / "epub-video-h264.mp4").read_bytes()),
                },
            )
        with doc_ex.open(data) as book:
            for index, error, message in (
                (0, doc_ex.MalformedDocumentError, "chapter ID .* absent"),
                (1, doc_ex.MalformedDocumentError, "chapter ID .* ambiguous"),
                (2, doc_ex.MalformedDocumentError, "chapter ID .* ambiguous"),
                (3, doc_ex.UnsupportedFormatError, "link to another segment"),
                (4, doc_ex.UnsupportedFormatError, "no playable interval"),
                (5, doc_ex.UnsupportedFormatError, "ordered edition"),
                (6, doc_ex.MalformedDocumentError, "end precedes its start"),
            ):
                with self.subTest(index=index), self.assertRaisesRegex(error, message):
                    next(book.epub_embedded_video(0, index, source_index=0, video_track=1))
            for uri, error, message in (
                ("chapter.webm#id=absent&amp;t=0,1", doc_ex.UnsupportedFormatError, "cannot combine t and id"),
                ("chapter.webm#t=0,1&amp;id=absent", doc_ex.UnsupportedFormatError, "cannot combine t and id"),
                ("chapter.webm#id=marker&amp;id=absent", doc_ex.UnsupportedFormatError, "repeated"),
                ("chapter.webm#id=", doc_ex.MalformedDocumentError, "fragment ID"),
                ("chapter.webm#id=x%00", doc_ex.MalformedDocumentError, "fragment ID"),
                ("movie.mp4#id=part", doc_ex.UnsupportedFormatError, "require a local WebM"),
            ):
                with self.subTest(uri=uri), self.assertRaisesRegex(error, message):
                    book.edit_epub_chapters({0: chapter.replace("chapter.webm#id=absent", uri)})

    def test_named_webm_fragments_keep_picture_soundtrack_and_caption_aligned(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        raw = (fixtures / "epub-video-named-tracks.webm").read_bytes()
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Selected</title></head>
          <body><video src="choice.webm#track=Voix%20%26%20cloche&amp;xywh=percent:25,25,50,50&amp;t=0.1,0.3&amp;track=Angle_Blue">
          <track src="caption.vtt" kind="captions" srclang="en"/></video>
          <video src="choice.webm"><track src="caption.vtt" kind="captions" srclang="en"/></video>
          <video src="choice.webm#track=Angle_Ruby"/><video src="choice.webm#track=Spoken"/>
          <video src="choice.webm#track=missing"/><video src="duplicate.webm#track=Angle_Blue"/>
          <video src="choice.webm#track=Angle_Ruby&amp;track=Angle_Blue"/>
          <audio src="audio.webm#track=Voix%20%26%20cloche&amp;t=0.1,0.3"/>
          <audio src="audio.webm"/>
          </body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Selected", chapter, "OEBPS/selected.xhtml")],
                resources={
                    "OEBPS/choice.webm": doc_ex.EpubResource("video/webm", raw),
                    "OEBPS/duplicate.webm": doc_ex.EpubResource("video/webm", raw.replace(b"Angle_Ruby", b"Angle_Blue")),
                    "OEBPS/audio.webm": doc_ex.EpubResource(
                        "audio/webm", (fixtures / "epub-audio-named-tracks.webm").read_bytes()),
                    "OEBPS/caption.vtt": doc_ex.EpubResource(
                        "text/vtt", b"WEBVTT\n\n00:00:00.150 --> 00:00:00.200\nPicked\n"),
                },
            )

        def timing(frame: doc_ex.EpubVideoFrame) -> tuple[int, int, tuple[doc_ex.EpubTextCue, ...]]:
            return frame.start_us, frame.duration_us, frame.text_cues

        with doc_ex.open(data) as book:
            source, = book.epub_embedded_media(0)[0].sources
            self.assertEqual(source.fragment_tracks, ("Voix & cloche", "Angle_Blue"))
            cropped = list(book.epub_embedded_video(0, 0, text_track=0))
            ordinary = list(book.epub_embedded_video(0, 1, start_us=100_000, end_us=300_000,
                                                     text_track=0, video_track=1))
            self.assertEqual([timing(f) for f in cropped], [timing(f) for f in ordinary])
            self.assertTrue(cropped)
            self.assertTrue(all((f.width, f.height) == (32, 20) and f.rgb[2] > 245 for f in cropped))
            self.assertTrue(any(f.text_cues for f in cropped))
            sound = list(book.epub_embedded_audio(0, 0))
            self.assertEqual(sound, list(book.epub_embedded_audio(0, 1, start_us=100_000,
                                                                 end_us=300_000, audio_track=1)))
            self.assertEqual(sum(clip.frame_count for clip in sound), 9_600)
            named_audio = list(book.epub_embedded_audio(0, 7))
            self.assertEqual(named_audio, list(book.epub_embedded_audio(0, 8, start_us=100_000,
                                                                       end_us=300_000, audio_track=1)))
            self.assertEqual(sum(clip.frame_count for clip in named_audio), 9_600)
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "does not select"):
                next(book.epub_embedded_audio(0, 2, source_index=0))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "does not select"):
                next(book.epub_embedded_video(0, 3, source_index=0))
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "track name .* absent"):
                next(book.epub_embedded_video(0, 4))
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "track name .* ambiguous"):
                next(book.epub_embedded_video(0, 5))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "multiple tracks of the same media type"):
                next(book.epub_embedded_video(0, 6))
            with self.assertRaisesRegex(ValueError, "video track ordinal"):
                next(book.epub_embedded_video(0, 0, video_track=0))
            with self.assertRaisesRegex(ValueError, "audio track ordinal"):
                next(book.epub_embedded_audio(0, 0, audio_track=0))

    def test_named_ogg_fragments_keep_selected_chains_on_one_presentation_clock(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Selected</title></head>
          <body><video src="choice.ogv#track=Cloche&amp;xywh=percent:25,25,50,50&amp;track=Azur&amp;t=0.45,0.62">
          <track src="caption.vtt" kind="captions" srclang="en"/></video>
          <video src="choice.ogv#track=Azur"/><video src="choice.ogv#track=Spoken"/>
          <video src="choice.ogv#track=Missing"/><video src="duplicate.ogv#track=Azur"/>
          <video src="ordinary.ogv#track=Azur"/><video src="choice.ogv#track=Ruby&amp;track=Azur"/>
          </body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Selected", chapter, "OEBPS/selected.xhtml")],
                resources={
                    "OEBPS/choice.ogv": doc_ex.EpubResource(
                        "video/ogg", (fixtures / "epub-video-named-tracks.ogv").read_bytes()),
                    "OEBPS/duplicate.ogv": doc_ex.EpubResource(
                        "video/ogg", (fixtures / "epub-video-ambiguous-tracks.ogv").read_bytes()),
                    "OEBPS/ordinary.ogv": doc_ex.EpubResource(
                        "video/ogg", (fixtures / "epub-theora-vorbis-short-audio.ogv").read_bytes()),
                    "OEBPS/caption.vtt": doc_ex.EpubResource(
                        "text/vtt", b"WEBVTT\n\n00:00:00.470 --> 00:00:00.520\nBoundary\n"),
                },
            )

        def pcm(clip: doc_ex.EpubAudioClip) -> bytes:
            self.assertEqual((clip.sample_rate, clip.channels), (48_000, 1))
            with wave.open(io.BytesIO(clip.wave), "rb") as samples:
                return samples.readframes(clip.frame_count)

        with doc_ex.open(data) as book:
            crops = list(book.epub_embedded_video(0, 0, text_track=0))
            self.assertEqual([(f.start_us, f.duration_us, bool(f.text_cues)) for f in crops], [
                (450_000, 20_000, False), (470_000, 30_000, True),
                (500_000, 20_000, True), (520_000, 100_000, False),
            ])
            self.assertEqual([(f.width, f.height) for f in crops], [(32, 20)] * 2 + [(28, 18)] * 2)
            self.assertGreater(crops[0].rgb[2], 245)
            self.assertGreater(crops[2].rgb[1], 245)
            sound = b"".join(pcm(clip) for clip in book.epub_embedded_audio(0, 0, chunk_frames=877))
            self.assertEqual(len(sound), 8_160 * 2)
            self.assertEqual(sound[:2_400 * 2], bytes(2_400 * 2))
            samples = [v for (v,) in struct.iter_unpack("<h", sound[2_400 * 2:])]
            crossings = sum(a <= 0 < b for a, b in zip(samples, samples[1:]))
            self.assertAlmostEqual(crossings, 580 * 0.12, delta=2)
            spoken = b"".join(pcm(clip) for clip in book.epub_embedded_audio(
                0, 2, start_us=250_000, end_us=450_000))
            self.assertEqual(len(spoken), 9_600 * 2)
            samples = [v for (v,) in struct.iter_unpack("<h", spoken)]
            self.assertAlmostEqual(sum(a <= 0 < b for a, b in zip(samples, samples[1:])), 380 * 0.2, delta=2)
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "does not select"):
                next(book.epub_embedded_audio(0, 1))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "does not select"):
                next(book.epub_embedded_video(0, 2))
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "track name .* absent"):
                next(book.epub_embedded_video(0, 3))
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "track name .* ambiguous"):
                next(book.epub_embedded_video(0, 4))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "Skeleton track names"):
                next(book.epub_embedded_video(0, 5))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "multiple tracks of the same media type"):
                next(book.epub_embedded_video(0, 6))
            with self.assertRaisesRegex(ValueError, "audio track ordinal"):
                next(book.epub_embedded_audio(0, 0, opus_track=0))

    def test_spatial_video_fragments_crop_picture_pixels_without_changing_the_clock(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Crops</title></head>
          <body><video src="camera.mp4"><track src="caption.vtt" kind="captions" srclang="en"/></video>
          <video><source src="camera.mp4#%78ywh=pixel%3A15%2C5%2C20%2C17&amp;t=0.1,0.23" type="video/mp4"/>
          <track src="caption.vtt" kind="captions" srclang="en"/></video>
          <video src="clip.webm"/><video src="clip.webm#t=0,0.1&amp;xywh=percent:8,13,33,41"/>
          <video src="clip.ogv"/><video src="clip.ogv#xywh=9,8,29,20"/>
          <video src="camera.mp4#xywh=125,62,18446744073709551615,20"/>
          <video src="camera.mp4#xywh=128,0,1,1"/>
          <video src="camera.mp4#xywh=0,64,1,1"/></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Crops", chapter, "OEBPS/crops.xhtml")],
                resources={
                    "OEBPS/camera.mp4": doc_ex.EpubResource(
                        "video/mp4", (fixtures / "epub-video-h264.mp4").read_bytes()),
                    "OEBPS/clip.webm": doc_ex.EpubResource(
                        "video/webm", (fixtures / "epub-video-vp9.webm").read_bytes()),
                    "OEBPS/clip.ogv": doc_ex.EpubResource(
                        "video/ogg", (fixtures / "epub-video-theora-spatial.ogv").read_bytes()),
                    "OEBPS/caption.vtt": doc_ex.EpubResource(
                        "text/vtt", b"WEBVTT\n\n00:00:00.125 --> 00:00:00.176\nCue\n"),
                },
            )

        def crop(frame: doc_ex.EpubVideoFrame, left: int, top: int, width: int, height: int) -> bytes:
            return b"".join(
                frame.rgb[3 * (row * frame.width + left):3 * (row * frame.width + left + width)]
                for row in range(top, top + height)
            )

        def timing(frame: doc_ex.EpubVideoFrame) -> tuple[int, int, bool]:
            return frame.start_us, frame.start_us + frame.duration_us, bool(frame.text_cues)

        with doc_ex.open(data) as book:
            sources = [media.sources[0] for media in book.epub_embedded_media(0)]
            self.assertIsNone(sources[0].fragment_xywh)
            self.assertEqual(sources[1].fragment_xywh, ("pixel", 15, 5, 20, 17))
            self.assertEqual((sources[1].fragment_start_us, sources[1].fragment_end_us), (100_000, 230_000))
            self.assertEqual(sources[3].fragment_xywh, ("percent", 8, 13, 33, 41))
            self.assertEqual(sources[5].fragment_xywh, ("pixel", 9, 8, 29, 20))

            full = list(book.epub_embedded_video(0, 0, start_us=110_000, end_us=230_000, text_track=0))
            spatial = list(book.epub_embedded_video(0, 1, start_us=110_000, end_us=270_000, text_track=0))
            self.assertEqual([timing(frame) for frame in spatial], [
                (110_000, 125_000, False), (125_000, 166_666, True),
                (166_666, 176_000, True), (176_000, 230_000, False),
            ])
            self.assertEqual([timing(frame) for frame in spatial], [timing(frame) for frame in full])
            self.assertEqual([frame.rgb for frame in spatial], [crop(frame, 11, 5, 16, 17) for frame in full])
            self.assertTrue(all((f.width, f.height, f.pixel_aspect_ratio) == (16, 17, (4, 3)) for f in spatial))
            self.assertIs(spatial[0].rgb, spatial[1].rgb)
            self.assertIs(spatial[2].rgb, spatial[3].rgb)
            sound = list(book.epub_embedded_audio(0, 1, start_us=110_000, end_us=270_000))
            self.assertEqual(sound, list(book.epub_embedded_audio(0, 0, start_us=110_000, end_us=230_000)))
            self.assertEqual(sum(clip.frame_count for clip in sound), 5760)

            for original, clipped, left, top, width, height in (
                (2, 3, 5, 6, 22, 21),
                (4, 5, 9, 5, 29, 14),
                (0, 6, 93, 62, 3, 2),
            ):
                with self.subTest(video=clipped):
                    full = next(book.epub_embedded_video(0, original))
                    spatial = next(book.epub_embedded_video(0, clipped))
                    self.assertEqual((spatial.width, spatial.height, spatial.pixel_aspect_ratio),
                                     (width, height, full.pixel_aspect_ratio))
                    self.assertEqual(spatial.rgb, crop(full, left, top, width, height))
            # Theora's plane rows run bottom-up and its clean aperture is smaller than the coded frame.
            self.assertEqual(next(book.epub_embedded_video(0, 4)).pixel_aspect_ratio, (2, 3))

            for index in (7, 8):
                with self.subTest(video=index), self.assertRaisesRegex(
                    doc_ex.MalformedDocumentError, "spatial fragment starts outside"
                ):
                    next(book.epub_embedded_video(0, index))

    def test_spatial_video_fragments_reject_invalid_and_unsupported_dimensions(self) -> None:
        for fragment, error, message in (
            ("xywh=0,1,0,5", doc_ex.MalformedDocumentError, "positive size"),
            ("xywh=0,-1,2,5", doc_ex.MalformedDocumentError, "unsigned integers"),
            ("xywh=0,0,18446744073709551616,5", doc_ex.MalformedDocumentError, "unsigned integers"),
            ("xywh=percent:98,0,3,10", doc_ex.MalformedDocumentError, "100 percent"),
            ("xywh=percent:0,100,3,1", doc_ex.MalformedDocumentError, "100 percent"),
            ("xywh=fraction:0,1,2,5", doc_ex.MalformedDocumentError, "fragment unit"),
            ("xywh=0,1,2,5&amp;xywh=1,2,3,4", doc_ex.UnsupportedFormatError, "repeated"),
            ("t=1&amp;t=2", doc_ex.UnsupportedFormatError, "repeated"),
            ("xywh=0,1,2,5&amp;other=2", doc_ex.UnsupportedFormatError, "fragment dimension"),
            ("xywh=0%2C0%2C1%2C1%26t%3D2", doc_ex.MalformedDocumentError, "unsigned integers"),
        ):
            chapter = ('<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Crops</title></head>'
                       f'<body><video src="clip.mp4#{fragment}"/></body></html>')
            with self.subTest(fragment=fragment), self.assertRaisesRegex(error, message):
                doc_ex.open(epub_fixture(chapter_one=chapter))

    def test_ttml_inline_sets_preserve_rational_timing_and_nested_styles(self) -> None:
        video = (Path(__file__).parent / "fixtures/epub-video-h264.mp4").read_bytes()
        ttml = b'''<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
          xmlns:tts="http://www.w3.org/ns/ttml#styling" xml:lang="fr"
          ttp:frameRate="30" ttp:frameRateMultiplier="1000 1001" ttp:tickRate="1000">
          <body begin="40ms" end="300ms"><set begin="50ms" end="170ms" tts:fontWeight="bold"/>
          <div begin="10ms" tts:color="#112233"><set begin="90ms" dur="60ms" tts:color="yellow"/>
          <p xml:id="same" begin="25ms" dur="220ms" tts:fontStyle="italic">
            <set begin="70t" dur="1f" tts:fontStyle="oblique"/>
            <set begin="95ms" end="145ms" tts:textDecoration="underline" fill="remove" repeatCount="1"/>
            <set begin="150ms" dur="10ms" tts:fontStyle="italic"/>
            <span>Same</span><span tts:color="blue"><set begin="80ms" dur="20ms" tts:color="lime"/> words</span>
          </p></div></body></tt>'''
        sequential = b'''<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling">
          <body><div begin="10ms" timeContainer="seq">
            <p dur="30ms">First<set begin="5ms" dur="100ms" tts:color="red"/></p>
            <p dur="10ms">After</p>
          </div>
          <p begin="50ms"><set begin="10ms" tts:fontWeight="bold"/>Open</p></body></tt>'''
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Timed styles</title></head>
          <body><video src="clip.mp4#t=0.12,0.26">
          <track src="sets.ttml" kind="subtitles" srclang="fr"/>
          <track src="sequence.ttml" kind="subtitles" srclang="en"/>
          </video></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Timed styles", chapter, "OEBPS/sets.xhtml")],
                resources={
                    "OEBPS/clip.mp4": doc_ex.EpubResource("video/mp4", video),
                    "OEBPS/sets.ttml": doc_ex.EpubResource("application/ttml+xml", ttml),
                    "OEBPS/sequence.ttml": doc_ex.EpubResource("application/ttml+xml", sequential),
                },
            )

        with doc_ex.open(data) as book:
            cues = book.epub_embedded_text(0, 0, track_index=0)
            boundaries = [75_000, 90_000, 140_000, 145_000, 155_000, 170_000, 175_000,
                          178_367, 200_000, 210_000, 220_000, 295_000]
            self.assertEqual([(c.start_us, c.end_us) for c in cues], list(zip(boundaries, boundaries[1:])))
            for cue in cues:
                with self.subTest(start=cue.start_us):
                    self.assertEqual(cue.text, "Same words")
                    self.assertEqual(cue, doc_ex.EpubTextCue(cue.start_us, cue.end_us, "Same words", "same"))
                    left, right = cue.runs
                    self.assertEqual((left.text, right.text), ("Same", " words"))
                    self.assertEqual(left.style, doc_ex.EpubTextStyle(
                        bold=90_000 <= cue.start_us < 210_000,
                        font_style="oblique" if 145_000 <= cue.start_us < 178_367 else "italic",
                        underline=170_000 <= cue.start_us < 220_000,
                        color=(255, 255, 0, 255) if 140_000 <= cue.start_us < 200_000 else (17, 34, 51, 255),
                        language="fr",
                    ))
                    self.assertEqual(right.style.color,
                                     (0, 255, 0, 255) if 155_000 <= cue.start_us < 175_000 else (0, 0, 255, 255))
                    self.assertEqual(right.style.font_style, left.style.font_style)

            frames = list(book.epub_embedded_video(0, 0, start_us=169_000, end_us=176_000, text_track=0))
            self.assertEqual([(f.start_us, f.duration_us) for f in frames],
                             [(169_000, 1_000), (170_000, 5_000), (175_000, 1_000)])
            self.assertTrue(all(f.rgb is frames[0].rgb for f in frames))
            self.assertEqual([f.text_cues[0].runs for f in frames], [c.runs for c in cues[4:7]])
            clipped = book.epub_embedded_text(0, 0, track_index=0, start_us=169_000, end_us=176_000)
            self.assertEqual([(c.start_us, c.end_us) for c in clipped],
                             [(169_000, 170_000), (170_000, 175_000), (175_000, 176_000)])

            cues = book.epub_embedded_text(0, 0, track_index=1)
            self.assertEqual([(c.start_us, c.end_us, c.text) for c in cues], [
                (10_000, 15_000, "First"), (15_000, 40_000, "First"), (40_000, 50_000, "After"),
                (50_000, 60_000, "Open"), (60_000, None, "Open"),
            ])
            self.assertEqual(cues[1].runs[0].style.color, (255, 0, 0, 255))
            self.assertIsNone(cues[2].runs[0].style.color)
            self.assertTrue(cues[-1].runs[0].style.bold)

    def test_ttml_inline_sets_bound_work_and_require_supported_animation(self) -> None:
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Timed styles</title></head>
          <body><video><track src="sets.ttml" kind="subtitles" srclang="fr"/></video></body></html>'''
        root = b'<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling">'
        plain = b'<body><p dur="1s">Text</p></body>'
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Timed styles", chapter, "OEBPS/sets.xhtml")],
                resources={"OEBPS/sets.ttml": doc_ex.EpubResource("application/ttml+xml", root + plain + b'</tt>')},
            )
        def inline(sets: bytes, mode: bytes = b"par") -> bytes:
            return b'<body><p dur="1s" timeContainer="' + mode + b'">' + sets + b'Text</p></body>'

        for payload, error, message in (
            (inline(b'<set begin="0s" dur="1s" tts:color="red"/><set begin="0.5s" tts:color="blue"/>'),
             doc_ex.UnsupportedFormatError, "overlapping TTML set"),
            (inline(b'<set begin="0s" fill="freeze" tts:color="red"/>'),
             doc_ex.UnsupportedFormatError, "unsupported TTML set attribute"),
            (inline(b'<set begin="0s" repeatCount="2" tts:color="red"/>'),
             doc_ex.UnsupportedFormatError, "unsupported TTML set attribute"),
            (inline(b'<set begin="0s" tts:fontSize="20px"/>'),
             doc_ex.UnsupportedFormatError, "unsupported TTML set attribute"),
            (inline(b'<set dur="0s" tts:color="red"/>' * 33),
             doc_ex.UnsafeArchiveError, "32 inline set elements"),
            (inline(b'<set dur="1s" tts:color="red"/>', b"seq"),
             doc_ex.UnsupportedFormatError, "sequential time container"),
            (b'<head><layout><region xml:id="r"><set dur="1s" tts:color="red"/></region></layout></head>' + plain,
             doc_ex.UnsupportedFormatError, "requires an inline subtitle content target"),
            (b'<head><animation><set xml:id="change" dur="1s" tts:color="red"/></animation></head>' + plain,
             doc_ex.UnsupportedFormatError, "animation references or continuous animation"),
        ):
            with self.subTest(message=message):
                changed = rewrite_epub(data, {"OEBPS/sets.ttml": root + payload + b'</tt>'})
                with doc_ex.open(changed) as book, self.assertRaisesRegex(error, message):
                    book.epub_embedded_text(0, 0)

    def test_styled_webvtt_and_ttml_runs_reach_clipped_cues_and_video_frames(self) -> None:
        video = (Path(__file__).parent / "fixtures/epub-video-h264.mp4").read_bytes()
        vtt = '''WEBVTT

voice
00:00:00.075 --> 00:00:00.300
<v Narrator &amp; Team><lang fr><c.yellow.magenta.bg_black.note>«<b>Un</b>»</c></lang> <00:00:00.140><i>mot</i><u>!</u></v>
'''.encode()
        ttml = b'''<tt xmlns="http://www.w3.org/ns/ttml"
          xmlns:tts="http://www.w3.org/ns/ttml#styling" xml:lang="en">
          <head><styling>
            <style xml:id="foundation" tts:color="#11223380" tts:fontWeight="bold"
                   tts:textDecoration="underline overline"/>
            <style xml:id="variant" style="foundation" tts:fontStyle="oblique"/>
            <style xml:id="override" tts:color="rgba(15,31,63,127)"/>
          </styling></head>
          <body style="variant"><div><p xml:id="same" begin="100ms" end="300ms" style="foundation override">
            <span end="75ms" xml:lang="fr" tts:color="#ff0000" tts:textDecoration="noUnderline">Ping</span>
            <span begin="75ms" xml:lang="ja" tts:fontStyle="normal" tts:fontWeight="normal"
                  tts:textDecoration="lineThrough noOverline">Ping</span>
          </p></div></body></tt>'''
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Styled subtitles</title></head>
          <body><video src="clip.mp4#t=0.12,0.26">
          <track src="voice.vtt" kind="captions" srclang="fr"/>
          <track src="style.ttml" kind="subtitles" srclang="en"/>
          </video></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Styled subtitles", chapter, "OEBPS/style.xhtml")],
                resources={
                    "OEBPS/clip.mp4": doc_ex.EpubResource("video/mp4", video),
                    "OEBPS/voice.vtt": doc_ex.EpubResource("text/vtt", vtt),
                    "OEBPS/style.ttml": doc_ex.EpubResource("application/ttml+xml", ttml),
                },
            )
        with doc_ex.open(data) as book:
            vtt_cue, = book.epub_embedded_text(0, 0, track_index=0, start_us=160_000, end_us=220_000)
            self.assertEqual((vtt_cue.start_us, vtt_cue.end_us, vtt_cue.text), (160_000, 220_000, "«Un» mot!"))
            self.assertIn("<i>mot</i>", vtt_cue.markup or "")
            fields = doc_ex.EpubTextStyle(
                color=(255, 0, 255, 255), background_color=(0, 0, 0, 255),
                classes=("yellow", "magenta", "bg_black", "note"), voice="Narrator & Team", language="fr")
            self.assertEqual(vtt_cue.runs, (
                doc_ex.EpubTextRun("«", fields),
                doc_ex.EpubTextRun("Un", doc_ex.EpubTextStyle(
                    bold=True, color=fields.color, background_color=fields.background_color,
                    classes=fields.classes, voice=fields.voice, language=fields.language)),
                doc_ex.EpubTextRun("»", fields),
                doc_ex.EpubTextRun(" ", doc_ex.EpubTextStyle(voice=fields.voice)),
                doc_ex.EpubTextRun("mot", doc_ex.EpubTextStyle(font_style="italic", voice=fields.voice), 140_000),
                doc_ex.EpubTextRun("!", doc_ex.EpubTextStyle(underline=True, voice=fields.voice), 140_000),
            ))
            self.assertEqual(vtt_cue, doc_ex.EpubTextCue(
                160_000, 220_000, "«Un» mot!", "voice", markup=vtt_cue.markup))
            voiced = list(book.epub_embedded_video(0, 0, start_us=160_000, end_us=220_000, text_track=0))
            self.assertTrue(voiced)
            self.assertTrue(all(frame.text_cues[0].runs == vtt_cue.runs for frame in voiced))

            first, second = book.epub_embedded_text(0, 0, track_index=1)
            self.assertEqual([(c.start_us, c.end_us, c.text) for c in (first, second)],
                             [(100_000, 175_000, "Ping"), (175_000, 300_000, "Ping")])
            self.assertEqual(first.runs, (doc_ex.EpubTextRun("Ping", doc_ex.EpubTextStyle(
                bold=True, font_style="oblique", overline=True, color=(255, 0, 0, 255), language="fr")),))
            self.assertEqual(second.runs, (doc_ex.EpubTextRun("Ping", doc_ex.EpubTextStyle(
                underline=True, line_through=True, color=(15, 31, 63, 127), language="ja")),))
            frames = list(book.epub_embedded_video(0, 0, start_us=170_000, end_us=180_000, text_track=1))
            self.assertEqual([(f.start_us, f.duration_us) for f in frames], [(170_000, 5_000), (175_000, 5_000)])
            self.assertEqual([f.text_cues[0].runs for f in frames], [first.runs, second.runs])
            self.assertIs(frames[0].rgb, frames[1].rgb)

    def test_webvtt_author_styles_cascade_and_follow_video_timing(self) -> None:
        video = (Path(__file__).parent / "fixtures/epub-video-h264.mp4").read_bytes()
        markup = ('Lead<v Narrator &amp; Team>Voice<lang fr-CA><c.highlight.bg_blue>Green'
                  '<b.highlight>Blue</b><i.accent>Normal</i></c></lang></v>'
                  '<00:00:00.140><u>End</u>')
        vtt = f'''WEBVTT

STYLE
::cue {{ color: white; background-color: rgba(0,0,0,.5); }}
::cue(:root) {{ font-weight: 600; }}
::cue(#special) {{ color: #112233; background: #223344; }}
::cue([lang|=en i]) {{ text-decoration: overline; }}
::cue(c.highlight) {{ color: orangered !important; background-color: rgba(255,0,0,.5); }}
::cue(.highlight) {{ color: red !important; }}
::cue(b.highlight) {{ color: blue !important; }}
::cue(v[voice="Narrator & Team"]), ::cue(lang:lang(fr)) {{ font-style: oblique; }}

STYLE
::cue(c.highlight) {{ color: lime !important; }}
::cue(u) {{ text-decoration: none; }}
::cue(i.accent:lang(fr)) {{ font-style: normal; color: teal; }}
::cue(.accent) {{ background-color: rgba(0,0,255,.5); text-decoration-line: underline line-through; }}

special
00:00:00.075 --> 00:00:00.170 align:start
{markup}

STYLE
::cue {{ color: black !important; }}

00:00:00.170 --> 00:00:00.260
Second
'''.encode()
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>CSS subtitles</title></head>
          <body><video src="clip.mp4#t=0.12,0.25">
          <track src="styled.vtt" kind="captions" srclang="en-US"/>
          </video></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("CSS subtitles", chapter, "OEBPS/style.xhtml")],
                resources={
                    "OEBPS/clip.mp4": doc_ex.EpubResource("video/mp4", video),
                    "OEBPS/styled.vtt": doc_ex.EpubResource("text/vtt", vtt),
                },
            )
        with doc_ex.open(data) as book:
            lead, second = book.epub_embedded_text(0, 0, start_us=167_000, end_us=200_000)
            expected = doc_ex.EpubTextCue(167_000, 170_000, "LeadVoiceGreenBlueNormalEnd",
                                          "special", "align:start", markup=markup)
            self.assertEqual(lead, expected)
            self.assertEqual(hash(lead), hash(expected))
            self.assertEqual(lead.background_color, (34, 51, 68, 255))
            self.assertEqual((second.start_us, second.end_us, second.text), (170_000, 200_000, "Second"))
            self.assertEqual(second.background_color, (0, 0, 0, 128))
            self.assertEqual(second.runs, (doc_ex.EpubTextRun(
                "Second", doc_ex.EpubTextStyle(bold=True, overline=True, color=(255, 255, 255, 255))),))
            lead_run, voice, green, blue, normal, end = lead.runs
            self.assertEqual([run.text for run in lead.runs], ["Lead", "Voice", "Green", "Blue", "Normal", "End"])
            self.assertTrue(all(run.style.bold and run.style.overline for run in lead.runs))
            self.assertEqual(lead_run.style, doc_ex.EpubTextStyle(bold=True, overline=True, color=(17, 34, 51, 255)))
            self.assertEqual((green.style.color, blue.style.color, normal.style.color),
                             ((0, 255, 0, 255), (0, 0, 255, 255), (0, 128, 128, 255)))
            self.assertEqual((voice.style.font_style, green.style.font_style,
                              blue.style.font_style, normal.style.font_style),
                             ("oblique", "oblique", "oblique", "normal"))
            self.assertEqual(green.style.background_color, (255, 0, 0, 128))
            self.assertEqual(normal.style.background_color, (85, 0, 170, 192))
            self.assertTrue(normal.style.underline and normal.style.line_through)
            self.assertEqual((normal.style.voice, normal.style.language), ("Narrator & Team", "fr-CA"))
            self.assertEqual(end.timestamp_us, 140_000)
            self.assertFalse(end.style.underline)
            self.assertEqual(end.style, lead_run.style)
            frames = list(book.epub_embedded_video(0, 0, start_us=167_000, end_us=180_000, text_track=0))
            self.assertEqual([(f.start_us, f.duration_us) for f in frames], [(167_000, 3_000), (170_000, 10_000)])
            self.assertIs(frames[0].rgb, frames[1].rgb)
            for frame, cue in zip(frames, (lead, second), strict=True):
                self.assertEqual(frame.text_cues[0].runs, cue.runs)
                self.assertEqual(frame.text_cues[0].background_color, cue.background_color)

    def test_webvtt_regions_follow_caption_selection_and_the_display_viewport(self) -> None:
        video = (Path(__file__).parent / "fixtures/epub-video-h264.mp4").read_bytes()
        vtt = '''WEBVTT

REGION\t
id:équipage width:40.5% lines:2
regionanchor:50%,100%
viewportanchor:10%,90% scroll:up

REGION
id:lower

REGION
id:replace width:70%

REGION
id:replace width:35.25%

REGION
id:fallback width:1e1% lines:-1 regionanchor:0%,oops viewportanchor:110%,0% scroll:down

REGION
id:large lines:4294967295

REGION
width:17%

STYLE
::cue(c:future) { color: red; }

lead
00:00:00.160 --> 00:00:00.205 region:équipage position:50% align:center
<c>Hello</c><00:00:00.175><c>Port</c>

floor
00:00:00.175 --> 00:00:00.195 region:lower size:100% line:auto
Lower

line
00:00:00.200 --> 00:00:00.230 region:équipage line:50%,end
Placed on a line

vertical
00:00:00.220 --> 00:00:00.235 region:équipage vertical:rl
Vertical

size
00:00:00.240 --> 00:00:00.260 region:équipage size:99.5%
Sized

ignored-line
00:00:00.260 --> 00:00:00.270 region:équipage line:5,bogus
Invalid line setting

ordered
00:00:00.270 --> 00:00:00.280 line:5 region:équipage
Region selected last

replace
00:00:00.280 --> 00:00:00.290 region:replace
Later definition

fallback
00:00:00.290 --> 00:00:00.300 region:fallback
Default values

missing
00:00:00.300 --> 00:00:00.310 region:missing
Unknown region

large
00:00:00.310 --> 00:00:00.320 region:large
Large line count

REGION
id:late width:45%

late
00:00:00.320 --> 00:00:00.330 region:late
Late region
'''
        raw = ("\ufeff" + vtt.replace("\n", "\r\n")).encode()
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Caption regions</title></head>
          <body><video src="clip.mp4#t=0.17,0.19&amp;xywh=1,1,6,4">
          <track src="styled.vtt" kind="captions" srclang="en"/>
          </video></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Caption regions", chapter, "OEBPS/regions.xhtml")],
                resources={
                    "OEBPS/clip.mp4": doc_ex.EpubResource("video/mp4", video),
                    "OEBPS/styled.vtt": doc_ex.EpubResource("text/vtt", raw),
                },
            )
        region = doc_ex.EpubTextRegion("équipage", 40.5, 2, (50, 100), (10, 90), "up")
        with doc_ex.open(data) as book:
            lead, lead_after, floor = book.epub_embedded_text(0, 0, start_us=170_000, end_us=190_000)
            self.assertEqual([(cue.start_us, cue.end_us) for cue in (lead, lead_after, floor)],
                             [(170_000, 175_000), (175_000, 190_000), (175_000, 190_000)])
            self.assertEqual(lead.region_layout, region)
            self.assertEqual(lead_after.region_layout, region)
            self.assertEqual(floor.region_layout, doc_ex.EpubTextRegion("lower"))
            self.assertEqual(lead, doc_ex.EpubTextCue(
                170_000, 175_000, "HelloPort", "lead", "region:équipage position:50% align:center",
                "équipage", "<c>Hello</c><00:00:00.175><c>Port</c>",
            ))
            self.assertEqual(lead.runs[1].timestamp_us, 175_000)
            self.assertEqual(lead.runs[1].style.color, (255, 0, 0, 255))
            self.assertIsNone(lead_after.runs[1].style.color)
            self.assertEqual(region.viewport_bounds(200, 100), (-20.5, 78, 81, 12))
            self.assertEqual(doc_ex.EpubTextRegion("lower").viewport_bounds(200, 100), (0, 82, 200, 18))
            with self.assertRaisesRegex(ValueError, "finite positive"):
                region.viewport_bounds(float("inf"), 100)

            cues = {cue.id: cue for cue in book.epub_embedded_text(0, 0)}
            for name in ("line", "vertical", "size"):
                self.assertEqual(cues[name].region, "équipage")
                self.assertIsNone(cues[name].region_layout)
            for name in ("ignored-line", "ordered"):
                self.assertEqual(cues[name].region_layout, region)
            for name in ("late", "missing"):
                self.assertEqual(cues[name].region, name)
                self.assertIsNone(cues[name].region_layout)
            self.assertEqual(cues["replace"].region_layout, doc_ex.EpubTextRegion("replace", 35.25))
            self.assertEqual(cues["fallback"].region_layout, doc_ex.EpubTextRegion("fallback"))
            self.assertEqual(cues["large"].region_layout, doc_ex.EpubTextRegion("large", lines=4_294_967_295))

            first, second = book.epub_embedded_video(0, 0, start_us=167_000, end_us=196_000, text_track=0)
            self.assertEqual([(f.start_us, f.duration_us, f.width, f.height) for f in (first, second)],
                             [(170_000, 5_000, 6, 4), (175_000, 15_000, 6, 4)])
            self.assertIs(first.rgb, second.rgb)
            self.assertEqual([cue.region_layout for cue in first.text_cues], [region])
            self.assertEqual([cue.region_layout for cue in second.text_cues],
                             [region, doc_ex.EpubTextRegion("lower")])
            revised = book.edit_epub_structure(replacements={0: chapter.replace("Caption regions", "Regions revised")})
        with doc_ex.open(revised) as reopened:
            self.assertEqual(reopened.read_epub_resource("OEBPS/styled.vtt"), raw)
            self.assertEqual(reopened.epub_embedded_text(0, 0)[0].region_layout, region)

    def test_webvtt_region_layout_limits_block_bytes_identifiers_and_lines(self) -> None:
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Caption regions</title></head>
          <body><video><track src="style.vtt" kind="captions" srclang="en"/></video></body></html>'''
        header = b"WEBVTT\n\n"
        cue = b"00:00:00.000 --> 00:00:00.200 region:local\nText"
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Caption regions", chapter, "OEBPS/style.xhtml")],
                resources={"OEBPS/style.vtt": doc_ex.EpubResource("text/vtt", header + cue)},
            )
        blocks = b"".join(
            b"REGION\nid:local note:" + b"".join(
                hashlib.sha256(f"{i}-{j}".encode()).hexdigest().encode() for j in range(60)
            ) + b"\n\n"
            for i in range(18)
        )
        for raw, message in (
            (b"REGION\nid:local\n\n" * 257, "256 REGION blocks"),
            (b"REGION\nid:local\n " + b"a" * 4096 + b"\n\n", "REGION block exceeds 4 KiB"),
            (blocks, "REGION blocks exceed 64 KiB"),
            (b"REGION\nid:" + b"a" * 1025 + b"\n\n", "REGION identifier exceeds"),
            (b"REGION\nid:local lines:4294967296\n\n", "line count exceeds 32 bits"),
            (b"REGION\nid:local lines:18446744073709551616\n\n", "line count exceeds 32 bits"),
        ):
            with self.subTest(message=message):
                changed = rewrite_epub(data, {"OEBPS/style.vtt": header + raw + cue})
                with doc_ex.open(changed) as book, self.assertRaisesRegex(doc_ex.UnsafeArchiveError, message):
                    book.epub_embedded_text(0, 0)

    def test_webvtt_timed_styles_follow_the_source_clock_during_clipped_video(self) -> None:
        video = (Path(__file__).parent / "fixtures/epub-video-h264.mp4").read_bytes()
        markup = ('<c.wrap><c.past>Before</c><00:00:00.170><c.past.future><b>Middle</b></c>'
                  '<00:00:00.174><c.future.deep>After</c><00:00:00.176><c.same>Steady</c></c>')
        vtt = f'''WEBVTT

STYLE
::cue {{ color: white; }}
::cue(c.past:past) {{ color: yellow; text-decoration: underline; font-weight: bold; }}
::cue(c.future:future) {{ color: cyan; background: #112233; font-style: italic; }}
::cue(c.deep:future) {{ color: lime !important; }}
::cue(.wrap:past), ::cue(.wrap:future) {{ color: red; }}
::cue(.same:future) {{ color: white; }}

lead
00:00:00.150 --> 00:00:00.230 align:start
{markup}

overlay
00:00:00.165 --> 00:00:00.181
<c.future>Overlay</c>
'''.encode()
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Timed subtitles</title></head>
          <body><video src="clip.mp4#t=0.169998,0.177002&amp;xywh=1,1,6,4">
          <track src="styled.vtt" kind="captions" srclang="en"/>
          </video></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Timed subtitles", chapter, "OEBPS/style.xhtml")],
                resources={
                    "OEBPS/clip.mp4": doc_ex.EpubResource("video/mp4", video),
                    "OEBPS/styled.vtt": doc_ex.EpubResource("text/vtt", vtt),
                },
            )
        intervals = [(169_998, 170_000), (170_000, 170_001), (170_001, 174_000),
                     (174_000, 174_001), (174_001, 177_002)]
        white, yellow = (255, 255, 255, 255), (255, 255, 0, 255)
        cyan, lime = (0, 255, 255, 255), (0, 255, 0, 255)
        with doc_ex.open(data) as book:
            cues = book.epub_embedded_text(0, 0, start_us=169_998, end_us=177_002)
            self.assertEqual([cue.id for cue in cues], ["lead", "overlay", "lead", "lead", "lead", "lead"])
            lead = [cue for cue in cues if cue.id == "lead"]
            overlay = cues[1]
            self.assertEqual([(cue.start_us, cue.end_us) for cue in lead], intervals)
            self.assertTrue(all(cue.text == "BeforeMiddleAfterSteady" and cue.markup == markup
                                and cue.settings == "align:start" for cue in lead))
            self.assertTrue(all([run.timestamp_us for run in cue.runs] == [None, 170_000, 174_000, 176_000]
                                for cue in lead))
            self.assertEqual([tuple(run.style.color for run in cue.runs) for cue in lead], [
                (white, cyan, lime, white),
                (white, white, lime, white),
                (yellow, white, lime, white),
                (yellow, white, white, white),
                (yellow, yellow, white, white),
            ])
            self.assertEqual(lead[0].runs[1].style.background_color, (17, 34, 51, 255))
            self.assertIsNone(lead[1].runs[1].style.background_color)
            self.assertTrue(lead[-1].runs[0].style.underline)
            self.assertTrue(lead[-1].runs[1].style.underline)
            self.assertFalse(lead[-1].runs[0].style.bold)
            self.assertTrue(lead[0].runs[1].style.bold)
            self.assertEqual(lead[0].runs[1].style.font_style, "normal")
            self.assertEqual(overlay.runs[0].style.color, white)
            sought = book.epub_embedded_text(0, 0, start_us=170_002, end_us=170_004)
            self.assertEqual([cue.runs for cue in sought], [lead[2].runs, overlay.runs])
            frames = list(book.epub_embedded_video(0, 0, start_us=160_000, end_us=190_000, text_track=0))
            self.assertEqual([(f.start_us, f.start_us + f.duration_us) for f in frames], intervals)
            control, = book.epub_embedded_video(0, 0, start_us=160_000, end_us=190_000)
            self.assertEqual((control.width, control.height, control.start_us, control.duration_us),
                             (6, 4, 169_998, 7_004))
            for frame, cue in zip(frames, lead, strict=True):
                self.assertEqual((frame.width, frame.height, frame.rgb), (6, 4, control.rgb))
                self.assertIs(frame.rgb, frames[0].rgb)
                self.assertEqual([item.id for item in frame.text_cues], ["lead", "overlay"])
                self.assertEqual([item.runs for item in frame.text_cues], [cue.runs, overlay.runs])
            clipped, = book.epub_embedded_video(0, 0, start_us=170_002, end_us=170_004, text_track=0)
            self.assertEqual([item.runs for item in clipped.text_cues], [lead[2].runs, overlay.runs])

    def test_webvtt_timed_style_projection_bounds_nodes_intervals_and_repeated_work(self) -> None:
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Timed subtitles</title></head>
          <body><video><track src="style.vtt" kind="captions" srclang="en"/></video></body></html>'''
        header = (b"WEBVTT\n\nSTYLE\n::cue(.absent:past), ::cue(.absent:future) {color:red}\n\n"
                  b"00:00:00.000 --> 00:00:04.000\n")
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Timed subtitles", chapter, "OEBPS/style.xhtml")],
                resources={"OEBPS/style.vtt": doc_ex.EpubResource("text/vtt", header + b"Text")},
            )

        def timeline(count: int) -> bytes:
            return (''.join(f'<c>t</c><00:00:{i // 1000:02d}.{i % 1000:03d}>' for i in range(1, count + 1))
                    + '<c>end</c>').encode()

        for markup, message in (
            (b"".join(f"<c.node{i}></c>".encode() for i in range(50_001)), "timed cue exceeds its node budget"),
            (timeline(2_049), "style boundary budget"),
            (timeline(650), "timed style projection exceeds its work budget"),
            (b"<c." + b"n" * 900 + b">" + timeline(150) + b"</c>",
             "timed style projection exceeds its work budget"),
        ):
            with self.subTest(message=message):
                changed = rewrite_epub(data, {"OEBPS/style.vtt": header + markup})
                with doc_ex.open(changed) as book, self.assertRaisesRegex(doc_ex.UnsafeArchiveError, message):
                    book.epub_embedded_text(0, 0)

    def test_webvtt_author_styles_bound_work_and_reject_resource_rules(self) -> None:
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>CSS subtitles</title></head>
          <body><video><track src="style.vtt" kind="captions" srclang="en"/></video></body></html>'''
        header = b"WEBVTT\n\n"
        cue = b"00:00:00.000 --> 00:00:00.200\nText"
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("CSS subtitles", chapter, "OEBPS/style.xhtml")],
                resources={"OEBPS/style.vtt": doc_ex.EpubResource("text/vtt", header + cue)},
            )
        large = b"".join(hashlib.sha256(str(i).encode()).hexdigest().encode() for i in range(1024))
        regular = b"::cue(b){color:red}\n"
        for sheet, payload, error, message in (
            (b'STYLE\n::cue { background: url(https://example.invalid/test.png); }\n\n',
             cue, doc_ex.UnsupportedFormatError, "cannot reference resources"),
            (b'STYLE\n::cue(#never) { --asset: url(../picture.png); }\n\n',
             cue, doc_ex.UnsupportedFormatError, "cannot reference resources"),
            (b'STYLE\n@import "caption.css";\n\n',
             cue, doc_ex.UnsupportedFormatError, "cannot reference resources"),
            (b'STYLE\n@media (width > 0px) { ::cue {color:red} }\n\n',
             cue, doc_ex.UnsupportedFormatError, "at-rules"),
            (b'STYLE\n/*' + large + b'*/\n\n',
             cue, doc_ex.UnsafeArchiveError, "stylesheets exceed 64 KiB"),
            (b'STYLE\n\n' * 33,
             cue, doc_ex.UnsafeArchiveError, "32 STYLE blocks"),
            (b'STYLE\n' + regular * 257 + b'\n',
             cue, doc_ex.UnsafeArchiveError, "selector count"),
            (b'STYLE\n::cue(' + b'.a' * 17 + b'){color:red}\n\n',
             cue, doc_ex.UnsafeArchiveError, "too many selector terms"),
            (b'STYLE\n::cue{' + b'color:red;' * 65 + b'}\n\n',
             cue, doc_ex.UnsafeArchiveError, "declarations exceed"),
            (b'STYLE\n' + regular * 256 + b'\n',
             cue + b'<b></b>' * 4_001, doc_ex.UnsafeArchiveError, "work exceeds|exceeds its work budget"),
        ):
            with self.subTest(message=message):
                changed = rewrite_epub(data, {"OEBPS/style.vtt": header + sheet + payload})
                with doc_ex.open(changed) as book, self.assertRaisesRegex(error, message):
                    book.epub_embedded_text(0, 0)

    def test_timed_subtitle_styles_bound_nesting_and_reject_broken_references(self) -> None:
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Styled subtitles</title></head>
          <body><video><track src="voice.vtt" kind="captions" srclang="fr"/>
          <track src="style.ttml" kind="subtitles" srclang="en"/>
          </video></body></html>'''
        vtt = b"WEBVTT\n\n00:00:00.000 --> 00:00:00.200\n"
        ttml = b'<tt xmlns="http://www.w3.org/ns/ttml"><head><styling>'
        text = b'</styling></head><body><div><p begin="0" dur="200ms" style="a">Text</p></div></body></tt>'
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Styled subtitles", chapter, "OEBPS/style.xhtml")],
                resources={
                    "OEBPS/voice.vtt": doc_ex.EpubResource("text/vtt", vtt + b"basic"),
                    "OEBPS/style.ttml": doc_ex.EpubResource(
                        "application/ttml+xml", ttml + b'<style xml:id="a"/>' + text),
                },
            )
        for path, raw, index, error, message in (
            ("voice.vtt", vtt + b"<b>" * 33 + b"text" + b"</b>" * 33, 0,
             doc_ex.UnsafeArchiveError, "span nesting"),
            ("voice.vtt", vtt + b"<c" + b".red" * 33 + b">text</c>", 0,
             doc_ex.UnsafeArchiveError, "cue classes"),
            ("voice.vtt", vtt + b"<b>x</b>y" * 5_001, 0,
             doc_ex.UnsafeArchiveError, "styled run count"),
            ("style.ttml", ttml + b'<style xml:id="a" style="b"/><style xml:id="b" style="a"/>' + text,
             1, doc_ex.MalformedDocumentError, "cyclic TTML style"),
            ("style.ttml", ttml + b'<style xml:id="a" style="b"/>' + text,
             1, doc_ex.MalformedDocumentError, "missing TTML style"),
        ):
            with self.subTest(path=path, message=message):
                changed = rewrite_epub(data, {f"OEBPS/{path}": raw})
                with doc_ex.open(changed) as book, self.assertRaisesRegex(error, message):
                    book.epub_embedded_text(0, 0, track_index=index)

    def test_embedded_ttml_nested_timing_preserves_regions_and_clip_intervals(self) -> None:
        ttml = b"""<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter"
          xml:lang="en" ttp:timeBase="media" ttp:frameRate="30" ttp:frameRateMultiplier="1000 1001"
          ttp:subFrameRate="2" ttp:tickRate="200"><head><layout><region xml:id="lower"/></layout></head>
          <body begin="100ms"><div timeContainer="seq" region="lower">
            <p xml:id="intro" dur="30f"> Begin <span begin="20t" dur="5t">timed</span><br/>World</p>
            <p begin="10t" dur="00:00:00:06.1" xml:space="preserve">Two  spaces</p>
          </div><div begin="2s" timeContainer="seq">
            <p dur="1f">One</p><p dur="1f">Two</p><p dur="1f">Three</p>
          </div><div begin="3s"><p>Open end</p></div></body></tt>"""
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Timed text</title></head>
          <body><audio><track src="timed.ttml" kind="captions" srclang="en"/></audio></body></html>"""
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Timed text", xhtml, "OEBPS/track.xhtml")],
                resources={"OEBPS/timed.ttml": doc_ex.EpubResource("application/ttml+xml", ttml)},
            )
        with doc_ex.open(data) as book:
            self.assertEqual(book.epub_embedded_text(0, 0), [
                doc_ex.EpubTextCue(100_000, 200_000, "Begin\nWorld", "intro", region="lower"),
                doc_ex.EpubTextCue(200_000, 225_000, "Begin timed\nWorld", "intro", region="lower"),
                doc_ex.EpubTextCue(225_000, 1_101_000, "Begin\nWorld", "intro", region="lower"),
                doc_ex.EpubTextCue(1_151_000, 1_367_883, "Two  spaces", region="lower"),
                doc_ex.EpubTextCue(2_100_000, 2_133_367, "One"),
                doc_ex.EpubTextCue(2_133_367, 2_166_733, "Two"),
                doc_ex.EpubTextCue(2_166_733, 2_200_100, "Three"),
                doc_ex.EpubTextCue(3_100_000, None, "Open end"),
            ])
            self.assertEqual(book.epub_embedded_text(0, 0, start_us=3_200_000, end_us=3_250_000),
                             [doc_ex.EpubTextCue(3_200_000, 3_250_000, "Open end")])
            replaced = book.edit_epub_chapters({0: xhtml.replace("Timed text", "Revised title")})
        with doc_ex.open(replaced) as book:
            self.assertEqual(book.epub_embedded_text(0, 0)[1].text, "Begin timed\nWorld")
            self.assertEqual(book.read_epub_resource("OEBPS/timed.ttml"), ttml)

    def test_embedded_text_tracks_reject_invalid_timing_and_excessive_cues(self) -> None:
        def with_track(media: str, track: bytes) -> bytes:
            xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Captions</title></head>
              <body><audio><track src="test.track" kind="captions" srclang="en"/></audio></body></html>"""
            with doc_ex.open(epub_fixture()) as book:
                return book.edit_epub_structure(
                    [doc_ex.EpubChapterSource("Captions", xhtml, "OEBPS/track.xhtml")],
                    resources={"OEBPS/test.track": doc_ex.EpubResource(media, track)},
                )

        choices = (
            ("text/vtt", b"WEBVTT\n\n00:61.000 --> 01:02.000\nInvalid", doc_ex.MalformedDocumentError, "timestamp"),
            ("text/vtt", b"WEBVTT\nX-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:0\n\n",
             doc_ex.UnsupportedFormatError, "timestamp maps"),
            ("text/vtt", ("WEBVTT\n\n" + "".join(
                f"{i:05x}\n00:00.000 --> 00:01.000\nCaption\n\n" for i in range(10_001)
            )).encode(), doc_ex.UnsafeArchiveError, "10000 cues"),
            ("application/ttml+xml",
             b'<tt xmlns="http://www.w3.org/ns/ttml"><body><p begin="invalid">No</p></body></tt>',
             doc_ex.MalformedDocumentError, "TTML time expression"),
        )
        for media, track, exception, message in choices:
            with self.subTest(message=message), doc_ex.open(with_track(media, track)) as book:
                with self.assertRaisesRegex(exception, message):
                    book.epub_embedded_text(0, 0)

    def test_embedded_mp4_video_frames_follow_presentation_order_and_clip_bounds(self) -> None:
        path = "OEBPS/clip.mp4"
        fixtures = Path(__file__).parent / "fixtures"
        resource = (fixtures / "epub-video-h264.mp4").read_bytes()
        ogg = (fixtures / "epub-narration-theora-opus.ogv").read_bytes()
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Video</title></head>
        <body><video id="clip"><source src="clip.ogv" type="video/ogg"/>
        <source src="clip.mp4" type="video/mp4"/>Transcript</video></body></html>"""
        with doc_ex.open(epub_fixture()) as source:
            data = source.edit_epub_structure(
                [doc_ex.EpubChapterSource("Video", xhtml, "OEBPS/new.xhtml")],
                resources={
                    "OEBPS/clip.ogv": doc_ex.EpubResource("video/ogg", ogg),
                    path: doc_ex.EpubResource("video/mp4", resource),
                },
            )
        with doc_ex.open(data) as book:
            media, = book.epub_embedded_media(0)
            self.assertEqual([source.path for source in media.sources], ["OEBPS/clip.ogv", path])
            self.assertTrue(media.sources[1].playable_audio)
            self.assertEqual(next(book.epub_embedded_video(0, 0)).width, 64)
            frames = list(book.epub_embedded_video(0, 0, source_index=1))
            self.assertEqual([frame.start_us for frame in frames], [i * 1_000_000 // 12 for i in range(12)])
            self.assertEqual(sum(frame.duration_us for frame in frames), 1_000_000)
            for index, frame in enumerate(frames):
                self.assertEqual((frame.width, frame.height, frame.pixel_aspect_ratio), (96, 64, (4, 3)))
                self.assertEqual(len(frame.rgb), 96 * 64 * 3)
                red, green, blue = frame.rgb[:3]
                self.assertGreater(red if index < 6 else blue, 248)
                self.assertLess(green + (blue if index < 6 else red), 8)

            clipped = list(book.epub_embedded_video(0, 0, source_index=1, start_us=480_000, end_us=650_000))
            self.assertEqual([(f.start_us, f.duration_us) for f in clipped], [
                (480_000, 20_000), (500_000, 83_333), (583_333, 66_667),
            ])
            self.assertEqual([frame.rgb for frame in clipped], [frame.rgb for frame in frames[5:8]])
            seek = list(book.epub_embedded_video(0, 0, source_index=1, start_us=750_000))
            self.assertEqual(seek, frames[9:])
            audio = list(book.epub_embedded_audio(0, 0, source_index=1, start_us=480_000, end_us=650_000))
            self.assertTrue(all((clip.sample_rate, clip.channels) == (48_000, 1) for clip in audio))
            self.assertEqual(sum(clip.frame_count for clip in audio), 8160)
            with self.assertRaisesRegex(ValueError, "outside its presentation"):
                next(book.epub_embedded_video(0, 0, source_index=1, start_us=1_000_000))
            with self.assertRaisesRegex(ValueError, "optional later end"):
                next(book.epub_embedded_video(0, 0, start_us=100, end_us=50))

        # MP4 validation runs upon playback, so a malformed source can retain
        # its chapter's authored transcript and resource metadata.
        with doc_ex.open(rewrite_epub(data, {path: resource[:-1]})) as book:
            self.assertIn("Transcript", book[0].get_text())
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(book.epub_embedded_video(0, 0, source_index=1))

        audio_only = (fixtures / "epub-narration-mono.opus").read_bytes()
        with doc_ex.open(rewrite_epub(data, {"OEBPS/clip.ogv": audio_only})) as book:
            self.assertEqual(list(book.epub_embedded_video(0, 0)), frames)
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "Theora"):
                next(book.epub_embedded_video(0, 0, source_index=0))

    def test_embedded_mp4_progressive_planar_avc_profiles(self) -> None:
        path = "OEBPS/clip.mp4"
        fixtures = Path(__file__).parent / "fixtures"
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Video</title></head>
        <body><video src="clip.mp4"/></body></html>"""
        with doc_ex.open(epub_fixture()) as source:
            data = source.edit_epub_structure(
                [doc_ex.EpubChapterSource("Video", xhtml, "OEBPS/new.xhtml")],
                resources={
                    path: doc_ex.EpubResource("video/mp4", (fixtures / "epub-video-h264.mp4").read_bytes()),
                },
            )
        profiles = ("yuv422p", "yuv444p", "yuv420p10le", "yuv422p10le", "yuv444p10le")
        for profile in profiles:
            with self.subTest(profile=profile):
                sample = (fixtures / f"epub-video-avc-{profile}.mp4").read_bytes()
                with doc_ex.open(rewrite_epub(data, {path: sample})) as book:
                    frames = list(book.epub_embedded_video(0, 0))
                    self.assertEqual([(f.start_us, f.duration_us) for f in frames], [
                        (i * 100_000, 100_000) for i in range(6)
                    ])
                    for index, frame in enumerate(frames):
                        self.assertEqual((frame.width, frame.height, frame.pixel_aspect_ratio), (64, 42, (4, 3)))
                        self.assertEqual(len(frame.rgb), 64 * 42 * 3)
                        red, green, blue = frame.rgb[-3:]
                        self.assertGreater(red if index < 3 else blue, 248)
                        self.assertLess(green + (blue if index < 3 else red), 8)
                    clipped = list(book.epub_embedded_video(0, 0, start_us=250_000, end_us=350_000))
                    self.assertEqual([(f.start_us, f.duration_us) for f in clipped], [(250_000, 50_000), (300_000, 50_000)])
                    self.assertEqual([f.rgb for f in clipped], [f.rgb for f in frames[2:4]])
                    self.assertEqual(list(book.epub_embedded_video(0, 0, start_us=300_000)), frames[3:])

        hdr = (fixtures / "epub-video-avc-hdr.mp4").read_bytes()
        with doc_ex.open(rewrite_epub(data, {path: hdr})) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "HDR transfer"):
                next(book.epub_embedded_video(0, 0))

    def test_embedded_ogg_theora_and_opus_share_the_chained_presentation_clock(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        first = (fixtures / "epub-narration-theora-opus.ogv").read_bytes()
        second = (fixtures / "epub-video-theora-tail.ogv").read_bytes()
        path = "OEBPS/clip.ogv"
        xhtml = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Video</title></head>
        <body><video><source src="clip.ogv" type="application/ogg; codecs=&quot;theora, opus&quot;"/>
        <track src="captions.vtt" kind="captions" srclang="en"/></video></body></html>"""
        captions = b"WEBVTT\n\n00:00:01.425 --> 00:00:01.575\nAcross links\n"
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Video", xhtml, "OEBPS/new.xhtml")],
                resources={
                    path: doc_ex.EpubResource("application/ogg", first + second),
                    "OEBPS/captions.vtt": doc_ex.EpubResource("text/vtt", captions),
                },
            )

        def pcm(book: doc_ex.Document, channels: int, **clip: int) -> bytes:
            samples = bytearray()
            for chunk in book.epub_embedded_audio(0, 0, chunk_frames=701, **clip):
                self.assertEqual((chunk.channels, chunk.sample_rate), (channels, 48_000))
                with wave.open(io.BytesIO(chunk.wave), "rb") as wav:
                    samples.extend(wav.readframes(chunk.frame_count))
            return bytes(samples)

        with doc_ex.open(rewrite_epub(data, {path: first})) as book:
            first_pcm = pcm(book, 2)
            self.assertEqual(len(first_pcm), 55_200 * 4)
        with doc_ex.open(rewrite_epub(data, {path: second})) as book:
            second_pcm = pcm(book, 1)
            self.assertEqual(len(second_pcm), 60_000 * 2)
        with doc_ex.open(data) as book:
            media, = book.epub_embedded_media(0)
            source, = media.sources
            self.assertTrue(source.playable_audio)
            self.assertTrue(source.playable_video)
            frames = list(book.epub_embedded_video(0, 0))
            self.assertEqual([(f.start_us, f.duration_us) for f in frames], [
                *((i * 125_000, 125_000) for i in range(12)),
                *((1_500_000 + i * 200_000, 200_000) for i in range(5)),
            ])
            tail = frames[12]
            self.assertEqual((tail.width, tail.height, tail.pixel_aspect_ratio), (66, 50, (4, 3)))
            self.assertGreater(tail.rgb[0], 245)
            self.assertLess(sum(tail.rgb[1:3]), 10)
            bottom = (tail.height - 1) * tail.width * 3
            self.assertGreater(tail.rgb[bottom + 2], 245)
            self.assertLess(sum(tail.rgb[bottom:bottom + 2]), 10)

            clip = list(book.epub_embedded_video(0, 0, start_us=1_400_000, end_us=1_800_000, text_track=0))
            self.assertEqual([(f.start_us, f.duration_us, bool(f.text_cues)) for f in clip], [
                (1_400_000, 25_000, False), (1_425_000, 75_000, True),
                (1_500_000, 75_000, True), (1_575_000, 125_000, False),
                (1_700_000, 100_000, False),
            ])
            self.assertEqual([f.rgb for f in clip], [frames[i].rgb for i in [11, 11, 12, 12, 13]])
            stereo = b"".join(second_pcm[i:i + 2] * 2 for i in range(0, len(second_pcm), 2))
            expected = first_pcm + bytes(16_800 * 4) + stereo
            self.assertEqual(pcm(book, 2), expected)
            self.assertEqual(pcm(book, 2, start_us=1_400_000, end_us=1_600_000), expected[67_200 * 4:76_800 * 4])
            self.assertEqual(list(book.epub_embedded_video(0, 0, start_us=2_600_000, end_us=2_700_000)), [])
            self.assertEqual(pcm(book, 2, start_us=2_600_000, end_us=2_700_000), expected[124_800 * 4:129_600 * 4])

        with doc_ex.open(rewrite_epub(data, {path: second + first})) as book:
            frames = list(book.epub_embedded_video(0, 0, start_us=950_000, end_us=1_300_000))
            self.assertEqual([(f.start_us, f.duration_us) for f in frames], [(950_000, 50_000), (1_250_000, 50_000)])
        with doc_ex.open(rewrite_epub(data, {path: first + second[:-1]})) as book:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(book.epub_embedded_video(0, 0))

    def test_webm_selects_audio_tracks_for_video_and_narration(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        movie = (fixtures / "epub-video-webm-selected-audio.webm").read_bytes()
        voice = "OEBPS/voice.webm"
        chapter_path = "OEBPS/tracks.xhtml"
        markup = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Tracks</title></head>
          <body><p id="voice">Select a narrator</p><video>
          <source src="fallback.ogv" type="video/ogg; codecs=&quot;theora, vorbis&quot;"/>
          <source src="tracks.webm" type="video/webm; codecs=&quot;vp8, opus, vorbis&quot;"/>
          </video><audio src="voice.webm"/></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            book_data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Tracks", markup, chapter_path)],
                resources={
                    "OEBPS/fallback.ogv": doc_ex.EpubResource("video/ogg", (
                        fixtures / "epub-theora-vorbis-short-audio.ogv").read_bytes()),
                    "OEBPS/tracks.webm": doc_ex.EpubResource("video/webm", movie),
                    voice: doc_ex.EpubResource("audio/webm", (
                        fixtures / "epub-audio-webm-selected-audio.webm").read_bytes()),
                },
                overlays={chapter_path: [doc_ex.EpubOverlayCue("voice", voice, 200_000, 400_000)]},
            )

        def pcm(chunks: list[doc_ex.EpubAudioClip], rate: int, channels: int) -> bytes:
            output = bytearray()
            for chunk in chunks:
                self.assertEqual((chunk.sample_rate, chunk.channels), (rate, channels))
                with wave.open(io.BytesIO(chunk.wave), "rb") as samples:
                    output.extend(samples.readframes(chunk.frame_count))
            return bytes(output)

        with doc_ex.open(book_data) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "multiple WebM tracks"):
                next(book.epub_embedded_audio(0, 0, source_index=1))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "multiple WebM tracks"):
                next(book.epub_narration(0))
            with self.assertRaisesRegex(ValueError, "track 2 is absent"):
                next(book.epub_embedded_audio(0, 0, audio_track=2))
            with self.assertRaisesRegex(ValueError, "requires a WebM resource"):
                next(book.epub_embedded_audio(0, 0, source_index=0, audio_track=0))
            with self.assertRaisesRegex(ValueError, "mutually exclusive"):
                next(book.epub_embedded_audio(0, 0, audio_track=0, opus_track=0))
            with self.assertRaisesRegex(ValueError, "mutually exclusive"):
                next(book.epub_narration(0, audio_tracks={voice: 1}, opus_tracks={voice: 0}))

            for track, rate, channels, frequencies in ((0, 48_000, 1, (340,)), (1, 44_100, 2, (420, 630))):
                with self.subTest(track=track):
                    selected = pcm(list(book.epub_embedded_audio(
                        0, 0, audio_track=track, start_us=200_000, end_us=400_000, chunk_frames=337,
                    )), rate, channels)
                    self.assertEqual(len(selected), rate * channels * 2 // 5)
                    signal = list(struct.iter_unpack("<" + "h" * channels, selected))
                    for index, frequency in enumerate(frequencies):
                        crossings = sum(a[index] < 0 <= b[index] for a, b in zip(signal, signal[1:]))
                        self.assertAlmostEqual(crossings, frequency / 5, delta=2)

                    narration = next(book.epub_narration(0, audio_tracks={voice: track})).audio
                    assert narration is not None
                    control = pcm(list(book.epub_embedded_audio(
                        0, 1, audio_track=track, start_us=200_000, end_us=400_000,
                    )), rate, channels)
                    self.assertEqual(pcm([narration], rate, channels), control)
                    played = []
                    book.play_epub_narration(0, audio_tracks={voice: track}, play_audio=played.append,
                                             speak_text=self.fail, chunk_frames=593)
                    self.assertEqual(pcm(played, rate, channels), control)
            pictures = list(book.epub_embedded_video(0, 0, source_index=1, start_us=200_000, end_us=400_000))
            self.assertEqual(pictures[0].start_us, 200_000)
            self.assertEqual(pictures[-1].start_us + pictures[-1].duration_us, 400_000)

        self.assertEqual(movie.count(b"A_VORBIS"), 1)
        unknown = movie.replace(b"A_VORBIS", b"A_UNKNWN")
        with doc_ex.open(rewrite_epub(book_data, {"OEBPS/tracks.webm": unknown})) as book:
            clip = next(book.epub_embedded_audio(0, 0, audio_track=0, start_us=200_000, end_us=400_000))
            self.assertEqual((clip.sample_rate, clip.channels, clip.frame_count), (48_000, 1, 9_600))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "requires an Opus or Vorbis track"):
                next(book.epub_embedded_audio(0, 0, audio_track=1))

    def test_ogg_vorbis_audio_and_smil_obey_the_final_granule(self) -> None:
        encoded = (Path(__file__).parent / "fixtures" / "epub-audio-vorbis.ogg").read_bytes()
        voice = "OEBPS/voice.ogg"
        chapter = "OEBPS/ogg.xhtml"
        markup = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Vorbis</title></head>
          <body><p id="one">Narrated text</p><audio><source src="voice.ogg" type="audio/ogg; codecs=vorbis"/>
          </audio></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Vorbis", markup, chapter)],
                resources={voice: doc_ex.EpubResource("audio/ogg", encoded)},
                overlays={chapter: [doc_ex.EpubOverlayCue("one", voice, 150_000, 650_000)]},
            )

        def pcm(clip: doc_ex.EpubAudioClip | None) -> bytes:
            assert clip is not None
            self.assertEqual((clip.sample_rate, clip.channels), (16_000, 1))
            with wave.open(io.BytesIO(clip.wave), "rb") as samples:
                return samples.readframes(clip.frame_count)

        with doc_ex.open(data) as book:
            self.assertTrue(book.epub_embedded_media(0)[0].sources[0].playable_audio)
            full = b"".join(pcm(clip) for clip in book.epub_embedded_audio(0, 0, chunk_frames=733))
            self.assertEqual(len(full), 10_400 * 2)
            narration = b"".join(pcm(cue.audio) for cue in book.epub_narration(0, chunk_frames=313))
            self.assertEqual(narration, full[2_400 * 2:])
            fragment = book.edit_epub_chapters({0: markup.replace("voice.ogg", "voice.ogg#t=0.15,0.65")})
        with doc_ex.open(fragment) as book:
            self.assertEqual(b"".join(pcm(clip) for clip in book.epub_embedded_audio(0, 0)), narration)
        with doc_ex.open(rewrite_epub(data, {voice: encoded[:-1]})) as book:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(book.epub_embedded_audio(0, 0))

    def test_surround_vorbis_preserves_speakers_in_embedded_audio_and_narration(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        chapter = "OEBPS/surround.xhtml"
        ogg = "OEBPS/surround.ogg"
        webm = "OEBPS/surround.webm"
        video = "OEBPS/clip.webm"
        layout = "OEBPS/undefined.ogg"
        markup = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Surround</title></head>
          <body><p id="ogg">Ogg surround</p><p id="webm">WebM surround</p>
          <audio src="surround.ogg#t=0.15,0.4"/><audio src="surround.webm"/>
          <video src="clip.webm"/><audio src="undefined.ogg"/></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Surround", markup, chapter)],
                resources={
                    ogg: doc_ex.EpubResource(
                        "audio/ogg", (fixtures / "epub-audio-vorbis-surround.ogg").read_bytes()),
                    webm: doc_ex.EpubResource(
                        "audio/webm", (fixtures / "epub-audio-vorbis-surround.webm").read_bytes()),
                    video: doc_ex.EpubResource(
                        "video/webm", (fixtures / "epub-video-vorbis-surround.webm").read_bytes()),
                    layout: doc_ex.EpubResource(
                        "audio/ogg", (fixtures / "epub-audio-vorbis-undefined-order.ogg").read_bytes()),
                },
                overlays={chapter: [
                    doc_ex.EpubOverlayCue("ogg", ogg, 150_000, 400_000),
                    doc_ex.EpubOverlayCue("webm", webm, 150_000, 400_000),
                ]},
            )

        def pcm(clip: doc_ex.EpubAudioClip | None) -> bytes:
            assert clip is not None
            self.assertEqual((clip.sample_rate, clip.channels), (48_000, 8))
            self.assertEqual(clip.wave[12:20], b"fmt \x28\0\0\0")
            self.assertEqual(struct.unpack_from("<HH", clip.wave, 20), (0xFFFE, 8))
            self.assertEqual(struct.unpack_from("<I", clip.wave, 40), (0x63F,))
            self.assertEqual(clip.wave[60:64], b"data")
            self.assertEqual(len(clip.wave[68:]), clip.frame_count * 16)
            return clip.wave[68:]

        with doc_ex.open(data) as book:
            narration = [pcm(cue.audio) for cue in book.epub_narration(0)]
            self.assertEqual(len(narration), 2)
            for index, expected in enumerate(narration):
                audio = book.epub_embedded_audio(0, index, start_us=150_000, end_us=400_000, chunk_frames=499)
                self.assertEqual(len(expected), 12_000 * 16)
                self.assertEqual(b"".join(pcm(clip) for clip in audio), expected)
                channels = zip(*struct.iter_unpack("<8h", expected))
                # The mask orders the speakers FL, FR, FC, LFE, BL, BR, SL, SR.
                for samples, frequency in zip(channels, (370, 430, 510, 70, 590, 650, 730, 810)):
                    self.assertGreater(max(samples), 1_000)
                    crossings = sum(a <= 0 < b for a, b in zip(samples, samples[1:]))
                    self.assertAlmostEqual(crossings, frequency * 0.25, delta=2)

            # The independently muxed WebM places its first audible packet at 3 ms.
            self.assertEqual(narration[1][144 * 16:], narration[0][:-144 * 16])
            soundtrack = book.epub_embedded_audio(
                0, 2, start_us=150_000, end_us=400_000, chunk_frames=331, audio_track=0)
            self.assertEqual(b"".join(pcm(clip) for clip in soundtrack), narration[1])
            pictures = book.epub_embedded_video(0, 2, start_us=150_000, end_us=400_000)
            self.assertEqual([(f.start_us, f.duration_us) for f in pictures],
                             [(150_000, 100_000), (250_000, 125_000), (375_000, 25_000)])
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "no defined speaker order"):
                next(book.epub_embedded_audio(0, 3, source_index=0))

    def test_chained_theora_vorbis_uses_the_longer_track_at_the_boundary(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        red = (fixtures / "epub-theora-vorbis-short-audio.ogv").read_bytes()
        blue = (fixtures / "epub-theora-vorbis-long-audio.ogv").read_bytes()
        markup = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Vorbis chain</title></head>
          <body><video><source src="chain.ogv" type="video/ogg; codecs=&quot;theora, vorbis&quot;"/>
          <track kind="captions" srclang="en" src="captions.vtt"/></video>
          <video src="red.ogv"/><video src="blue.ogv"/></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Vorbis chain", markup, "OEBPS/chain.xhtml")],
                resources={"OEBPS/chain.ogv": doc_ex.EpubResource("video/ogg", red + blue),
                           "OEBPS/red.ogv": doc_ex.EpubResource("video/ogg", red),
                           "OEBPS/blue.ogv": doc_ex.EpubResource("video/ogg", blue),
                           "OEBPS/captions.vtt": doc_ex.EpubResource(
                               "text/vtt", b"WEBVTT\n\n00:00:00.735 --> 00:00:00.770\nBoundary\n")},
            )

        def pcm(book: doc_ex.Document, index: int, **clip: int) -> bytes:
            output = bytearray()
            for item in book.epub_embedded_audio(0, index, chunk_frames=487, **clip):
                self.assertEqual((item.sample_rate, item.channels), (44_100, 2))
                with wave.open(io.BytesIO(item.wave), "rb") as samples:
                    output.extend(samples.readframes(item.frame_count))
            return bytes(output)

        with doc_ex.open(data) as book:
            source, = book.epub_embedded_media(0)[0].sources
            self.assertTrue(source.playable_audio and source.playable_video)
            full = pcm(book, 0)
            self.assertEqual(len(full), 68_355 * 4)
            self.assertEqual(full, pcm(book, 1) + bytes(15_435 * 4) + pcm(book, 2))
            self.assertEqual(pcm(book, 0, start_us=700_000, end_us=800_000),
                             full[30_870 * 4:35_280 * 4])
            pictures = list(book.epub_embedded_video(0, 0))
            self.assertEqual([frame.start_us for frame in pictures], [i * 125_000 for i in range(11)])
            self.assertGreater(pictures[5].rgb[0], 245)
            self.assertGreater(pictures[6].rgb[2], 245)
            clipped = list(book.epub_embedded_video(0, 0, start_us=700_000, end_us=800_000, text_track=0))
            self.assertEqual([(frame.start_us, frame.duration_us, bool(frame.text_cues)) for frame in clipped],
                             [(700_000, 35_000, False), (735_000, 15_000, True),
                              (750_000, 20_000, True), (770_000, 30_000, False)])

    def test_webm_selects_video_tracks_with_clipped_captions(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        markup = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Video tracks</title></head>
          <body><video><source src="fallback.ogv" type="video/ogg"/>
          <source src="tracks.webm" type="video/webm"/>
          <track kind="captions" srclang="en" src="captions.vtt"/></video></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Tracks", markup, "OEBPS/video.xhtml")],
                resources={
                    "OEBPS/fallback.ogv": doc_ex.EpubResource("video/ogg", (
                        fixtures / "epub-theora-vorbis-short-audio.ogv").read_bytes()),
                    "OEBPS/tracks.webm": doc_ex.EpubResource("video/webm", (
                        fixtures / "epub-video-webm-selected-video.webm").read_bytes()),
                    "OEBPS/captions.vtt": doc_ex.EpubResource(
                        "text/vtt", b"WEBVTT\n\n00:00:00.180 --> 00:00:00.230\nCaption\n"),
                },
            )
        with doc_ex.open(data) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "multiple WebM tracks"):
                next(book.epub_embedded_video(0, 0, source_index=1))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "WebM video track selection"):
                next(book.epub_embedded_video(0, 0, source_index=0, video_track=0))
            with self.assertRaisesRegex(ValueError, "WebM video track 2 is absent"):
                next(book.epub_embedded_video(0, 0, video_track=2))
            for track, size, channel in ((0, (48, 32), 0), (1, (64, 40), 2)):
                with self.subTest(track=track):
                    frames = list(book.epub_embedded_video(0, 0, video_track=track, start_us=150_000,
                                                          end_us=250_000, text_track=0))
                    self.assertEqual([(f.start_us, f.duration_us, bool(f.text_cues)) for f in frames],
                                     [(150_000, 30_000, False), (180_000, 20_000, True),
                                      (200_000, 30_000, True), (230_000, 20_000, False)])
                    for frame in frames:
                        self.assertEqual((frame.width, frame.height), size)
                        self.assertGreater(frame.rgb[channel], 245)
                        self.assertLess(frame.rgb[2 - channel], 10)

    def test_embedded_webm_vp8_opus_and_captions_share_the_presentation_clock(self) -> None:
        resource = (Path(__file__).parent / "fixtures" / "epub-video-vp8-opus.webm").read_bytes()
        path = "OEBPS/clip.webm"
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>WebM</title></head>
          <body><video><source src="clip.webm" type="video/webm; codecs=&quot;vp8, opus&quot;"/>
          <track kind="captions" srclang="en" src="captions.vtt"/>Transcript</video></body></html>"""
        caption = b"WEBVTT\n\n00:00:00.500 --> 00:00:00.650\nRed frame\n"
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("WebM", chapter, "OEBPS/webm.xhtml")],
                resources={path: doc_ex.EpubResource("video/webm", resource),
                           "OEBPS/captions.vtt": doc_ex.EpubResource("text/vtt", caption)},
            )

        def pcm(book: doc_ex.Document, **clip: int) -> bytes:
            chunks = bytearray()
            for audio in book.epub_embedded_audio(0, 0, chunk_frames=701, **clip):
                self.assertEqual((audio.sample_rate, audio.channels), (48_000, 1))
                with wave.open(io.BytesIO(audio.wave), "rb") as samples:
                    chunks.extend(samples.readframes(audio.frame_count))
            return bytes(chunks)

        with doc_ex.open(data) as book:
            source, = book.epub_embedded_media(0)[0].sources
            self.assertEqual(source.path, path)
            self.assertTrue(source.playable_video)
            self.assertTrue(source.playable_audio)
            frames = list(book.epub_embedded_video(0, 0))
            self.assertEqual([(f.start_us, f.duration_us) for f in frames],
                             [(i * 125_000, 125_000) for i in range(12)])
            for index, frame in enumerate(frames):
                self.assertEqual((frame.width, frame.height, frame.pixel_aspect_ratio), (96, 64, (4, 3)))
                red, green, blue = frame.rgb[:3]
                self.assertGreater(red if index < 6 else blue, 246)
                self.assertLess(green + (blue if index < 6 else red), 10)
            self.assertEqual(len(full := pcm(book)), 55_200 * 2)
            self.assertEqual(pcm(book, start_us=480_000, end_us=860_000), full[23_040 * 2:41_280 * 2])
            clipped = list(book.epub_embedded_video(0, 0, start_us=480_000, end_us=860_000, text_track=0))
            self.assertEqual([(f.start_us, f.duration_us, bool(f.text_cues)) for f in clipped], [
                (480_000, 20_000, False),
                (500_000, 125_000, True),
                (625_000, 25_000, True),
                (650_000, 100_000, False),
                (750_000, 110_000, False),
            ])
            self.assertEqual([f.rgb for f in clipped], [frames[i].rgb for i in [3, 4, 5, 5, 6]])
            self.assertEqual(clipped[1].text_cues[0].text, "Red frame")
            revised = book.edit_epub_chapters({0: chapter.replace("clip.webm", "clip.webm#t=0.48,0.86")})
        with doc_ex.open(revised) as book:
            self.assertEqual(book.read_epub_resource(path), resource)
            self.assertEqual(list(book.epub_embedded_video(0, 0, text_track=0)), clipped)
            self.assertEqual(pcm(book), full[23_040 * 2:41_280 * 2])
        with doc_ex.open(rewrite_epub(data, {path: resource[:-1]})) as book:
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "truncated WebM element"):
                next(book.epub_embedded_video(0, 0))

    def test_embedded_webm_vorbis_keeps_the_presentation_clock(self) -> None:
        path = "OEBPS/vorbis.webm"
        resource = (Path(__file__).parent / "fixtures" / "epub-video-vp8-vorbis.webm").read_bytes()
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Vorbis</title></head>
          <body><video><source src="vorbis.webm" type="video/webm; codecs=&quot;vp8, vorbis&quot;"/>
          <track kind="captions" srclang="en" src="captions.vtt"/>Transcript</video></body></html>"""
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Vorbis", chapter, "OEBPS/clip.xhtml")],
                resources={path: doc_ex.EpubResource("video/webm", resource),
                           "OEBPS/captions.vtt": doc_ex.EpubResource(
                               "text/vtt", b"WEBVTT\n\n00:00:00.500 --> 00:00:00.650\nRed frame\n")},
            )

        def pcm(book: doc_ex.Document, chunk_frames: int, **clip: int) -> bytes:
            output = bytearray()
            for audio in book.epub_embedded_audio(0, 0, chunk_frames=chunk_frames, **clip):
                self.assertEqual((audio.sample_rate, audio.channels), (44_100, 2))
                with wave.open(io.BytesIO(audio.wave), "rb") as wave_file:
                    output.extend(wave_file.readframes(audio.frame_count))
            return bytes(output)

        with doc_ex.open(data) as book:
            source, = book.epub_embedded_media(0)[0].sources
            self.assertTrue(source.playable_audio)
            self.assertTrue(source.playable_video)
            full = pcm(book, 701)
            self.assertEqual(len(full), 50_884 * 4)
            self.assertEqual(full[:132 * 4], bytes(132 * 4))
            tones = list(struct.iter_unpack("<hh", full[4_410 * 4:48_510 * 4]))
            for channel, frequency in enumerate((340, 560)):
                crossings = sum(a[channel] < 0 <= b[channel] for a, b in zip(tones, tones[1:]))
                self.assertAlmostEqual(crossings, frequency, delta=2)
            self.assertEqual(pcm(book, 4_093, start_us=480_000, end_us=860_000),
                             full[21_168 * 4:37_926 * 4])
            video = list(book.epub_embedded_video(0, 0))
            self.assertEqual([frame.start_us for frame in video], [3_000 + i * 125_000 for i in range(12)])
            clipped = list(book.epub_embedded_video(0, 0, start_us=480_000, end_us=860_000, text_track=0))
            self.assertEqual(clipped[0].start_us, 480_000)
            self.assertEqual(clipped[-1].start_us + clipped[-1].duration_us, 860_000)
            self.assertEqual({cue.text for frame in clipped for cue in frame.text_cues}, {"Red frame"})
            revised = book.edit_epub_chapters({0: chapter.replace("vorbis.webm", "vorbis.webm#t=0.48,0.86")})
        with doc_ex.open(revised) as book:
            self.assertEqual(book.read_epub_resource(path), resource)
            self.assertEqual(pcm(book, 283), full[21_168 * 4:37_926 * 4])
            self.assertEqual(list(book.epub_embedded_video(0, 0, text_track=0)), clipped)

        # The first setup codebook has a byte-aligned 24-bit entry count after its dimensions.
        huge = bytearray(resource)
        count = huge.index(b"\x05vorbis") + 7 + 1 + 3 + 2
        huge[count:count + 3] = b"\xff\xff\xff"
        with doc_ex.open(rewrite_epub(data, {path: bytes(huge)})) as book:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "Vorbis codebook entries"):
                next(book.epub_embedded_audio(0, 0))

    def test_webm_vorbis_end_stamps_gaps_and_overlaps_preserve_pcm(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Vorbis timing</title></head>
          <body><video src="modern.webm"/><audio src="end-stamps.webm"/>
          <audio src="gap-overlap.webm"/></body></html>'''
        resources = {
            "OEBPS/modern.webm": doc_ex.EpubResource(
                "video/webm", (fixtures / "epub-video-vp8-vorbis.webm").read_bytes()),
            "OEBPS/end-stamps.webm": doc_ex.EpubResource(
                "audio/webm", (fixtures / "epub-audio-vorbis-end-stamps.webm").read_bytes()),
            "OEBPS/gap-overlap.webm": doc_ex.EpubResource(
                "audio/webm", (fixtures / "epub-audio-vorbis-gap-overlap.webm").read_bytes()),
        }
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Vorbis timing", chapter, "OEBPS/timing.xhtml")],
                resources=resources,
            )

        def pcm(book: doc_ex.Document, index: int, **clip: int) -> bytes:
            output = bytearray()
            for chunk in book.epub_embedded_audio(0, index, chunk_frames=293, **clip):
                self.assertEqual((chunk.sample_rate, chunk.channels), (44_100, 2))
                with wave.open(io.BytesIO(chunk.wave), "rb") as samples:
                    output.extend(samples.readframes(chunk.frame_count))
            return bytes(output)

        with doc_ex.open(data) as book:
            modern = pcm(book, 0)
            self.assertEqual(modern[:132 * 4], bytes(132 * 4))
            raw = modern[132 * 4:]
            self.assertEqual(pcm(book, 1), raw)
            with_breaks = pcm(book, 2)
            expected = (
                modern[:10_948 * 4] + bytes(3_517 * 4)
                + raw[10_816 * 4:29_248 * 4] + raw[29_908 * 4:]
            )
            self.assertEqual(len(with_breaks), 53_741 * 4)
            self.assertEqual(with_breaks, expected)
            self.assertEqual(pcm(book, 2, start_us=200_000, end_us=950_000),
                             with_breaks[8_820 * 4:41_895 * 4])

    def test_laced_webm_vorbis_audio_and_narration_share_trimmed_pcm(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        video_path = "OEBPS/clip.webm"
        audio_path = "OEBPS/voice.webm"
        chapter_path = "OEBPS/clip.xhtml"
        chapter = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Vorbis</title></head>
          <body><p id="voice">Read this</p><video src="clip.webm"/>
          <audio><source src="voice.webm" type="audio/webm; codecs=vorbis"/></audio></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("Vorbis narration", chapter, chapter_path)],
                resources={
                    video_path: doc_ex.EpubResource(
                        "video/webm", (fixtures / "epub-video-vp8-vorbis.webm").read_bytes()),
                    audio_path: doc_ex.EpubResource(
                        "audio/webm", (fixtures / "epub-audio-laced-vorbis.webm").read_bytes()),
                },
                overlays={chapter_path: [doc_ex.EpubOverlayCue("voice", audio_path, 0, 1_143_832)]},
            )

        def pcm(clip: doc_ex.EpubAudioClip | None) -> bytes:
            assert clip is not None
            self.assertEqual((clip.sample_rate, clip.channels), (44_100, 2))
            with wave.open(io.BytesIO(clip.wave), "rb") as samples:
                return samples.readframes(clip.frame_count)

        with doc_ex.open(data) as book:
            self.assertTrue(book.epub_embedded_media(0)[1].sources[0].playable_audio)
            video = b"".join(pcm(clip) for clip in book.epub_embedded_audio(0, 0, chunk_frames=8_192))
            audio = b"".join(pcm(clip) for clip in book.epub_embedded_audio(0, 1, chunk_frames=603))
            narration = b"".join(pcm(cue.audio) for cue in book.epub_narration(0, chunk_frames=1_201))
            self.assertEqual(len(audio), 50_443 * 4)
            self.assertEqual(audio, video[:-441 * 4])
            self.assertEqual(narration, audio)

    def test_embedded_webm_av1_seeks_and_retains_sdr_depth(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        markup = '''<html xmlns="http://www.w3.org/1999/xhtml"><head><title>AV1</title></head>
          <body><video><source src="eight.webm" type="video/webm; codecs=&quot;av01.0.00M.08, opus&quot;"/>
          <track kind="captions" srclang="en" src="captions.vtt"/></video>
          <video><source src="ten.webm" type="video/webm; codecs=av01.0.00M.10"/></video></body></html>'''
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [doc_ex.EpubChapterSource("AV1", markup, "OEBPS/clip.xhtml")],
                resources={
                    "OEBPS/eight.webm": doc_ex.EpubResource(
                        "video/webm", (fixtures / "epub-video-av1-opus.webm").read_bytes()),
                    "OEBPS/ten.webm": doc_ex.EpubResource(
                        "video/webm", (fixtures / "epub-video-av1-10.webm").read_bytes()),
                    "OEBPS/captions.vtt": doc_ex.EpubResource(
                        "text/vtt", b"WEBVTT\n\n00:00:00.550 --> 00:00:00.650\nCaption\n"),
                },
            )
        with doc_ex.open(data) as book:
            for element in book.epub_embedded_media(0):
                self.assertTrue(element.sources[0].playable_video)
            pictures = list(book.epub_embedded_video(0, 0))
            self.assertEqual([frame.start_us for frame in pictures], [i * 125_000 for i in range(12)])
            for i, frame in enumerate(pictures):
                self.assertEqual((frame.width, frame.height, frame.pixel_aspect_ratio), (96, 64, (4, 3)))
                red, _, blue = frame.rgb[:3]
                self.assertGreater(red if i < 6 else blue, 245)
                self.assertLess(blue if i < 6 else red, 22)
            window = list(book.epub_embedded_video(0, 0, start_us=575_000, end_us=830_000, text_track=0))
            self.assertEqual([(frame.start_us, frame.duration_us) for frame in window],
                             [(575_000, 50_000), (625_000, 25_000), (650_000, 100_000), (750_000, 80_000)])
            self.assertEqual([frame.rgb for frame in window], [pictures[i].rgb for i in (4, 5, 5, 6)])
            self.assertEqual([bool(frame.text_cues) for frame in window], [True, True, False, False])
            audio = list(book.epub_embedded_audio(0, 0, start_us=575_000, end_us=830_000))
            self.assertEqual(sum(chunk.frame_count for chunk in audio), 12_240)
            ten = list(book.epub_embedded_video(0, 1))
            self.assertEqual([frame.start_us for frame in ten], [0, 167_000, 333_000, 500_000])
            for frame in ten:
                self.assertEqual((frame.width, frame.height), (64, 48))
                red, green, blue = frame.rgb[:3]
                self.assertLess(red, 10)
                self.assertGreater(min(green, blue), 245)

    def test_embedded_webm_vp9_retains_colour_and_depth(self) -> None:
        fixtures = Path(__file__).parent / "fixtures"
        for filename, codecs, geometry, fps, count in (
            ("epub-video-vp9.webm", "vp09.01.10.08", (66, 50), 7, 11),
            ("epub-video-vp9-10.webm", "vp09.02.10.10", (64, 48), 6, 3),
        ):
            with self.subTest(codecs=codecs):
                chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>WebM</title></head>
                  <body><video><source src="clip.webm" type="video/webm; codecs={codecs}"/>
                  </video></body></html>"""
                with doc_ex.open(epub_fixture()) as original:
                    data = original.edit_epub_structure(
                        [doc_ex.EpubChapterSource("WebM", chapter, "OEBPS/webm.xhtml")],
                        resources={"OEBPS/clip.webm": doc_ex.EpubResource("video/webm", (fixtures / filename).read_bytes())},
                    )
                with doc_ex.open(data) as book:
                    source, = book.epub_embedded_media(0)[0].sources
                    self.assertTrue(source.playable_video)
                    self.assertFalse(source.playable_audio)
                    frames = list(book.epub_embedded_video(0, 0))
                    self.assertEqual([f.start_us for f in frames],
                                     [1000 * ((1000 * i + fps // 2) // fps) for i in range(count)])
                    self.assertTrue(all((f.width, f.height) == geometry for f in frames))
                    self.assertTrue(all(len(f.rgb) == geometry[0] * geometry[1] * 3 for f in frames))
                    if fps == 7:
                        self.assertGreater(frames[0].rgb[0], 240)
                        bottom = (geometry[1] - 1) * geometry[0] * 3
                        self.assertGreater(frames[0].rgb[bottom + 2], 240)
                    else:
                        for actual, expected in zip(frames[0].rgb[:3], (39, 156, 203)):
                            self.assertAlmostEqual(actual, expected, delta=6)
                    seek = list(book.epub_embedded_video(0, 0, start_us=frames[-1].start_us))
                    self.assertEqual(seek, frames[-1:])

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

    def test_media_overlay_streaming_matches_buffered_pcm(self) -> None:
        source, wav = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        fixtures = Path(__file__).parent / "fixtures"
        for media, audio in (
            (b"audio/wav", wav),
            (b"audio/mpeg", (fixtures / "epub-narration-tone.mp3").read_bytes()),
            (b"audio/mp4", (fixtures / "epub-narration-tone.m4a").read_bytes()),
        ):
            book = rewrite_epub(source, {
                "OEBPS/content.opf": package.replace(b"audio/wav", media),
                "OEBPS/Media/voice.wav": audio,
            })
            with self.subTest(media=media), doc_ex.open(book) as document:
                for page in (0, 1):
                    [full] = document.epub_narration(page, skip_roles={"aside"})
                    assert full.audio is not None
                    with wave.open(io.BytesIO(full.audio.wave), "rb") as pcm:
                        expected = pcm.readframes(full.audio.frame_count)
                    parts = bytearray()
                    for cue in document.epub_narration(page, skip_roles={"aside"}, chunk_frames=513):
                        self.assertEqual((cue.id, cue.path, cue.fragment, cue.roles),
                                         (full.id, full.path, full.fragment, full.roles))
                        assert cue.audio is not None
                        self.assertLessEqual(cue.audio.frame_count, 513)
                        with wave.open(io.BytesIO(cue.audio.wave), "rb") as pcm:
                            parts.extend(pcm.readframes(cue.audio.frame_count))
                    self.assertEqual(parts, expected)
                    self.assertEqual([full], list(document.epub_narration(
                        page, skip_roles={"aside"}, chunk_frames=full.audio.frame_count,
                    )))

    def test_media_overlay_cue_navigation_skips_preceding_audio(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/flac")
        source = rewrite_epub(source, {"OEBPS/content.opf": package})
        with doc_ex.open(source) as document:
            events: list[tuple[str, object]] = []
            document.play_epub_narration(
                0, start_cue=1, chunk_frames=1,
                play_audio=lambda audio: events.append(("audio", audio.frame_count)),
                speak_text=lambda text: events.append(("speech", text)),
                on_cue=lambda cue: events.append(("target", cue and cue.fragment)),
            )
            self.assertEqual(events, [("target", "aside"), ("speech", "Text cue"), ("target", None)])
            self.assertEqual(list(document.epub_narration(0, start_cue=1, skip_roles={"aside"})), [])
            for index in (-1, 2):
                with self.assertRaises(IndexError):
                    list(document.epub_narration(0, start_cue=index))
            for frames in (0, 262_145):
                with self.assertRaises(ValueError):
                    list(document.epub_narration(0, chunk_frames=frames))

    def test_media_overlay_streams_long_cue_with_one_active_target(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            smil = archive.read("OEBPS/Sync/read.smil")
        smil = smil.replace(b'clipBegin="250ms" clipEnd="00:00:01.125"', b'clipBegin="0s"')
        frames = 8 * 1024 * 1024 + 1
        wav = io.BytesIO()
        with wave.open(wav, "wb") as writer:
            writer.setparams((1, 1, 8000, 0, "NONE", "not compressed"))
            writer.writeframes(bytes(range(256)) * (frames // 256) + b"\0")
        source = rewrite_epub(source, {"OEBPS/Sync/read.smil": smil, "OEBPS/Media/voice.wav": wav.getvalue()})
        with doc_ex.open(source) as document:
            with self.assertRaises(doc_ex.UnsafeArchiveError):
                next(document.epub_narration(0))
            targets: list[str | None] = []
            chunks: list[int] = []
            digest = hashlib.sha256()

            def play(audio: doc_ex.EpubAudioClip) -> None:
                self.assertEqual(targets, ["café"])
                chunks.append(audio.frame_count)
                with wave.open(io.BytesIO(audio.wave), "rb") as pcm:
                    digest.update(pcm.readframes(audio.frame_count))

            document.play_epub_narration(
                0, chunk_frames=262_144, skip_roles={"aside"}, play_audio=play,
                speak_text=lambda text: self.fail(f"unexpected narration {text}"),
                on_cue=lambda cue: targets.append(cue and cue.fragment),
            )
            self.assertEqual(targets, ["café", None])
            self.assertEqual(chunks, [262_144] * 32 + [1])
            expected = hashlib.sha256()
            ramp = struct.pack("<256h", *(index * 256 for index in range(-128, 128)))
            for _ in range(32):
                expected.update(ramp * 1024)
            expected.update(ramp[:2])
            self.assertEqual(digest.digest(), expected.digest())

    def test_media_overlay_applies_mpeg_encoder_delay_and_padding(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/mpeg")
            smil = archive.read("OEBPS/Sync/read.smil")
        # A synthesized 1 kHz tone at 44.1 kHz with Xing/Info delay and padding.
        audio = (Path(__file__).parent / "fixtures" / "epub-narration-gapless.mp3").read_bytes()
        smil = smil.replace(b'clipBegin="250ms" clipEnd="00:00:01.125"', b'clipBegin="0s"')
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package, "OEBPS/Sync/read.smil": smil, "OEBPS/Media/voice.wav": audio,
        })
        with doc_ex.open(source) as document:
            clip = next(document.epub_narration(0)).audio
            assert clip is not None
            self.assertEqual((clip.sample_rate, clip.channels, clip.frame_count), (44100, 1, 17640))
            with wave.open(io.BytesIO(clip.wave), "rb") as pcm:
                reference = pcm.readframes(17640)
            self.assertGreater(max(struct.unpack("<128h", reference[:256])), 1000)
            self.assertGreater(max(struct.unpack("<128h", reference[-256:])), 1000)
        smil = smil.replace(b'clipBegin="0s"', b'clipBegin="0.125s" clipEnd="0.4s"')
        with doc_ex.open(rewrite_epub(source, {"OEBPS/Sync/read.smil": smil})) as document:
            recorded = bytearray()
            for cue in document.epub_narration(0, chunk_frames=509, skip_roles={"aside"}):
                assert cue.audio is not None
                with wave.open(io.BytesIO(cue.audio.wave), "rb") as pcm:
                    recorded.extend(pcm.readframes(cue.audio.frame_count))
            self.assertEqual(recorded, reference[5513 * 2:])
        with doc_ex.open(rewrite_epub(source, {"OEBPS/Sync/read.smil": smil.replace(b"0.4s", b"0.401s")})) as document:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(document.epub_narration(0, chunk_frames=509))
        damaged = bytearray(audio)
        count = damaged.index(b"Info") + 8
        damaged[count + 3] += 1
        with doc_ex.open(rewrite_epub(source, {"OEBPS/Media/voice.wav": bytes(damaged)})) as document:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(document.epub_narration(0))

    def test_media_overlay_preserves_ogg_opus_stereo_presentation(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/ogg")
            smil = archive.read("OEBPS/Sync/read.smil")
        # The 1.15-second synthetic stream has 515 Hz left and 755 Hz right.
        data = (Path(__file__).parent / "fixtures" / "epub-narration-stereo.opus").read_bytes()
        source = rewrite_epub(source, {"OEBPS/content.opf": package, "OEBPS/Media/voice.wav": data})
        full_smil = smil.replace(b'clipBegin="250ms" clipEnd="00:00:01.125"', b'clipBegin="0s"')
        with doc_ex.open(rewrite_epub(source, {"OEBPS/Sync/read.smil": full_smil})) as document:
            audio = next(document.epub_narration(0)).audio
            assert audio is not None
            self.assertEqual((audio.sample_rate, audio.channels, audio.frame_count), (48000, 2, 55200))
            with wave.open(io.BytesIO(audio.wave), "rb") as pcm:
                reference = pcm.readframes(audio.frame_count)
            samples = [list(channel) for channel in zip(*struct.iter_unpack("<hh", reference))]
            for channel, frequency in zip(samples, (515, 755)):
                self.assertGreater(max(channel), 1000)
                crossings = sum(a <= 0 < b for a, b in zip(channel, channel[1:]))
                self.assertAlmostEqual(crossings, frequency * 1.15, delta=2)
        with doc_ex.open(source) as document:
            intro = bytearray()
            for cue in document.epub_narration(0, chunk_frames=773, skip_roles={"aside"}):
                assert cue.audio is not None
                with wave.open(io.BytesIO(cue.audio.wave), "rb") as pcm:
                    intro.extend(pcm.readframes(cue.audio.frame_count))
            self.assertEqual(intro, reference[12000 * 4:54000 * 4])
            tail = next(document.epub_narration(1)).audio
            assert tail is not None
            self.assertEqual(tail.frame_count, 1200)
            with wave.open(io.BytesIO(tail.wave), "rb") as pcm:
                self.assertEqual(pcm.readframes(tail.frame_count), reference[54000 * 4:])
        with doc_ex.open(rewrite_epub(source, {"OEBPS/Sync/read.smil": smil.replace(
            b"00:00:01.125", b"1.151s",
        )})) as document:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(document.epub_narration(0, chunk_frames=773))

    def test_media_overlay_chains_ogg_opus_links_and_seeks_across_them(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/ogg")
            smil = archive.read("OEBPS/Sync/read.smil")
        source = rewrite_epub(source, {"OEBPS/content.opf": package})
        full = smil.replace(b'clipBegin="250ms" clipEnd="00:00:01.125"', b'clipBegin="0s"')
        fixtures = Path(__file__).parent / "fixtures"
        stereo = (fixtures / "epub-narration-stereo.opus").read_bytes()
        mono = (fixtures / "epub-narration-mono.opus").read_bytes()

        def pcm_frames(clip: doc_ex.EpubAudioClip) -> bytes:
            with wave.open(io.BytesIO(clip.wave), "rb") as pcm:
                self.assertEqual(pcm.getnchannels(), clip.channels)
                return pcm.readframes(clip.frame_count)

        individual = []
        for payload, channels, frames in ((stereo, 2, 55200), (mono, 1, 16800)):
            with doc_ex.open(rewrite_epub(source, {
                "OEBPS/Sync/read.smil": full,
                "OEBPS/Media/voice.wav": payload,
            })) as book:
                audio = next(book.epub_narration(0)).audio
                assert audio is not None
                self.assertEqual((audio.sample_rate, audio.channels, audio.frame_count), (48000, channels, frames))
                individual.append(pcm_frames(audio))
        duplicated_mono = b"".join(sample * 2 for (sample,) in struct.iter_unpack("2s", individual[1]))
        expected = individual[0] + duplicated_mono
        chain = rewrite_epub(source, {
            "OEBPS/Sync/read.smil": full,
            "OEBPS/Media/voice.wav": stereo + mono,
        })
        with doc_ex.open(chain) as book:
            audio = next(book.epub_narration(0)).audio
            assert audio is not None
            self.assertEqual((audio.sample_rate, audio.channels, audio.frame_count), (48000, 2, 72000))
            self.assertEqual(pcm_frames(audio), expected)
            streamed = []
            for cue in book.epub_narration(0, skip_roles={"aside"}, chunk_frames=773):
                assert cue.audio is not None
                self.assertLessEqual(cue.audio.frame_count, 773)
                streamed.append(pcm_frames(cue.audio))
            self.assertEqual(b"".join(streamed), expected)

        seek = smil.replace(b'clipBegin="250ms" clipEnd="00:00:01.125"', b'clipBegin="1.14s" clipEnd="1.16s"')
        backward = (b'<par id="seek-back"><text src="../one.xhtml#aside"/>'
                    b'<audio src="../Media/voice.wav" clipBegin="1.00s" clipEnd="1.01s"/></par>')
        slot = seek.index(b"</par>") + len(b"</par>")
        seek = seek[:slot] + backward + seek[slot:]
        with doc_ex.open(rewrite_epub(chain, {"OEBPS/Sync/read.smil": seek})) as book:
            cross, back = list(book.epub_narration(0, skip_roles={"aside"}))
            assert cross.audio is not None and back.audio is not None
            self.assertEqual(pcm_frames(cross.audio), expected[54720 * 4:55680 * 4])
            self.assertEqual(pcm_frames(back.audio), expected[48000 * 4:48480 * 4])

    def test_media_overlay_preserves_ogg_opus_surround_speaker_layout(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/ogg")
            smil = archive.read("OEBPS/Sync/read.smil")
        bounds = b'clipBegin="250ms" clipEnd="00:00:01.125"'
        full = smil.replace(bounds, b'clipBegin="0s"')
        fixtures = Path(__file__).parent / "fixtures"
        surround = (fixtures / "epub-narration-surround.opus").read_bytes()
        mono = (fixtures / "epub-narration-mono.opus").read_bytes()

        def fixture(payload: bytes, timeline: bytes = full) -> bytes:
            return rewrite_epub(source, {
                "OEBPS/content.opf": package,
                "OEBPS/Sync/read.smil": timeline,
                "OEBPS/Media/voice.wav": payload,
            })

        def surround_pcm(audio: doc_ex.EpubAudioClip | None) -> bytes:
            assert audio is not None
            self.assertEqual((audio.sample_rate, audio.channels), (48000, 8))
            self.assertEqual(audio.wave[12:20], b"fmt \x28\0\0\0")
            self.assertEqual(struct.unpack_from("<HH", audio.wave, 20), (0xFFFE, 8))
            self.assertEqual(struct.unpack_from("<I", audio.wave, 40), (0x63F,))
            self.assertEqual(audio.wave[60:64], b"data")
            self.assertEqual(struct.unpack_from("<I", audio.wave, 64), (audio.frame_count * 16,))
            pcm = audio.wave[68:]
            self.assertEqual(len(pcm), audio.frame_count * 16)
            return pcm

        with doc_ex.open(fixture(surround)) as book:
            reference = surround_pcm(next(book.epub_narration(0)).audio)
        self.assertEqual(len(reference), 19200 * 16)
        channels = zip(*struct.iter_unpack("<8h", reference))
        # WAVE order is FL, FR, FC, LFE, BL, BR, SL, SR.
        for samples, frequency in zip(channels, (370, 430, 510, 70, 590, 650, 730, 810)):
            self.assertGreater(max(samples), 1000)
            crossings = sum(a <= 0 < b for a, b in zip(samples, samples[1:]))
            self.assertAlmostEqual(crossings, frequency * 0.4, delta=2)

        with doc_ex.open(fixture(mono)) as book:
            audio = next(book.epub_narration(0)).audio
            assert audio is not None
            with wave.open(io.BytesIO(audio.wave), "rb") as pcm:
                mono_pcm = pcm.readframes(audio.frame_count)
        expected = reference + b"".join(
            struct.pack("<8h", 0, 0, sample, 0, 0, 0, 0, 0)
            for (sample,) in struct.iter_unpack("<h", mono_pcm)
        )

        chain = surround + mono
        with doc_ex.open(fixture(chain)) as book:
            self.assertEqual(surround_pcm(next(book.epub_narration(0)).audio), expected)
            chunks = b"".join(
                surround_pcm(cue.audio)
                for cue in book.epub_narration(0, chunk_frames=773, skip_roles={"aside"})
            )
            self.assertEqual(chunks, expected)
        cross = smil.replace(bounds, b'clipBegin="0.39s" clipEnd="0.41s"')
        with doc_ex.open(fixture(chain, cross)) as book:
            self.assertEqual(
                surround_pcm(next(book.epub_narration(0)).audio),
                expected[18720 * 16:19680 * 16],
            )

    def test_media_overlay_reads_opus_from_grouped_ogg_and_chains(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
            smil = archive.read("OEBPS/Sync/read.smil")
        bounds = b'clipBegin="250ms" clipEnd="00:00:01.125"'
        full = smil.replace(bounds, b'clipBegin="0s"')
        fixtures = Path(__file__).parent / "fixtures"
        video = (fixtures / "epub-narration-theora-opus.ogv").read_bytes()
        stereo = (fixtures / "epub-narration-stereo.opus").read_bytes()
        mono = (fixtures / "epub-narration-mono.opus").read_bytes()

        def fixture(payload: bytes, media: bytes = b"audio/ogg", timeline: bytes = full) -> bytes:
            return rewrite_epub(source, {
                "OEBPS/content.opf": package.replace(b"audio/wav", media),
                "OEBPS/Sync/read.smil": timeline,
                "OEBPS/Media/voice.wav": payload,
            })

        def pcm(audio: doc_ex.EpubAudioClip | None) -> bytes:
            assert audio is not None
            self.assertEqual(audio.sample_rate, 48000)
            with wave.open(io.BytesIO(audio.wave), "rb") as reader:
                self.assertEqual(reader.getnframes(), audio.frame_count)
                return reader.readframes(audio.frame_count)

        with doc_ex.open(fixture(stereo)) as book:
            expected = pcm(next(book.epub_narration(0)).audio)
        self.assertEqual(len(expected), 55200 * 4)
        for media in (b"audio/ogg", b"application/ogg", b"video/ogg"):
            with self.subTest(media=media), doc_ex.open(fixture(video, media)) as book:
                self.assertEqual(pcm(next(book.epub_narration(0)).audio), expected)

        with doc_ex.open(fixture(stereo + mono)) as book:
            expected_chain = pcm(next(book.epub_narration(0)).audio)
        with doc_ex.open(fixture(video + mono, b"application/ogg")) as book:
            streamed = b"".join(
                pcm(cue.audio)
                for cue in book.epub_narration(0, chunk_frames=773, skip_roles={"aside"})
            )
            self.assertEqual(streamed, expected_chain)
        cross = smil.replace(bounds, b'clipBegin="1.14s" clipEnd="1.16s"')
        with doc_ex.open(fixture(video + mono, timeline=cross)) as book:
            self.assertEqual(pcm(next(book.epub_narration(0)).audio), expected_chain[54720 * 4:55680 * 4])

        # The auxiliary Theora stream begins first and ends after the Opus EOS.
        # This distinguishes validation of the physical segment from the audio.
        pages = []
        offset = 0
        while offset < len(video):
            lacing = video[offset + 26]
            body = offset + 27 + lacing
            end = body + sum(video[offset + 27:body])
            pages.append(video[offset:end])
            offset = end
        self.assertTrue(pages[-1][5] & 4)
        self.assertEqual(pages[0][14:18], pages[-1][14:18])
        self.assertNotEqual(pages[1][14:18], pages[-1][14:18])
        damaged_aux = video[:-1] + bytes([video[-1] ^ 1])
        missing_eos = b"".join(pages[:-1])
        late_bos = pages[1] + pages[3] + pages[0] + pages[2] + b"".join(pages[4:])
        for malformed in (damaged_aux, missing_eos, late_bos):
            with doc_ex.open(fixture(malformed)) as book:
                with self.assertRaises(doc_ex.MalformedDocumentError):
                    next(book.epub_narration(0))

        def checked_page(page: bytes | bytearray) -> bytes:
            page = bytearray(page)
            page[22:26] = bytes(4)
            checksum = 0
            for octet in page:
                checksum ^= octet << 24
                for _ in range(8):
                    checksum = ((checksum << 1) ^ (0x04C11DB7 if checksum & 0x80000000 else 0)) & 0xFFFFFFFF
            struct.pack_into("<I", page, 22, checksum)
            return bytes(page)

        # Ogg permits an auxiliary EOS page containing no packets.
        nil_eos = bytearray(pages[-1][:27])
        nil_eos[26] = 0
        with doc_ex.open(fixture(missing_eos + checked_page(nil_eos))) as book:
            self.assertEqual(pcm(next(book.epub_narration(0)).audio), expected)
        with doc_ex.open(fixture(missing_eos + nil_eos)) as book:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(book.epub_narration(0))

        # A complete auxiliary stream may occupy a single leading BOS page.
        single = bytearray(pages[0])
        single[5] |= 4
        with doc_ex.open(fixture(checked_page(single) + stereo)) as book:
            self.assertEqual(pcm(next(book.epub_narration(0)).audio), expected)

        path = "OEBPS/Media/grouped.ogg"
        with doc_ex.open(source) as book:
            edited = book.edit_epub_structure(
                [1, 0],
                resources={path: doc_ex.EpubResource("application/ogg", video)},
                overlays={
                    "OEBPS/one.xhtml": [doc_ex.EpubOverlayCue("café", path, 0, 20_000)],
                    "OEBPS/two.xhtml": None,
                },
            )
        with doc_ex.open(edited) as book:
            self.assertEqual(book.epub_media_cues(0), [])
            self.assertEqual(book.read_epub_resource(path), video)
            self.assertEqual(pcm(next(book.epub_narration(1)).audio), expected[:960 * 4])

    def test_media_overlay_selects_opus_tracks_per_resource(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"application/ogg")
            timeline = archive.read("OEBPS/Sync/read.smil")
        full = timeline.replace(b'clipBegin="250ms" clipEnd="00:00:01.125"', b'clipBegin="0s"')
        path = "OEBPS/Media/voice.wav"
        fixtures = Path(__file__).parent / "fixtures"
        video = (fixtures / "epub-narration-theora-opus.ogv").read_bytes()
        mono = (fixtures / "epub-narration-mono.opus").read_bytes()
        stereo = (fixtures / "epub-narration-stereo.opus").read_bytes()
        surround = (fixtures / "epub-narration-surround.opus").read_bytes()

        def pages(payload: bytes) -> list[bytes]:
            result = []
            offset = 0
            while offset < len(payload):
                body = offset + 27 + payload[offset + 26]
                end = body + sum(payload[offset + 27:body])
                result.append(payload[offset:end])
                offset = end
            return result

        def group(first: bytes, second: bytes) -> bytes:
            first_pages, second_pages = pages(first), pages(second)
            after_bos = next(i for i, page in enumerate(first_pages) if not page[5] & 2)
            # Both fixtures start with a single identification page per logical stream.
            return b"".join(first_pages[:after_bos] + second_pages[:2]
                            + first_pages[after_bos:] + second_pages[2:])

        def fixture(payload: bytes) -> bytes:
            return rewrite_epub(source, {
                "OEBPS/content.opf": package,
                "OEBPS/Sync/read.smil": full,
                path: payload,
            })

        def pcm(audio: doc_ex.EpubAudioClip | None) -> bytes:
            assert audio is not None
            with wave.open(io.BytesIO(audio.wave), "rb") as reader:
                return reader.readframes(audio.frame_count)

        payload = group(video, mono)
        with doc_ex.open(fixture(video)) as book:
            expected_stereo = pcm(next(book.epub_narration(0)).audio)
        with doc_ex.open(fixture(mono)) as book:
            expected_mono = pcm(next(book.epub_narration(0)).audio)
        with doc_ex.open(fixture(payload)) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "explicit selection"):
                next(book.epub_narration(0))
            self.assertEqual(pcm(next(book.epub_narration(0, opus_tracks={path: 0})).audio), expected_stereo)
            self.assertEqual(pcm(next(book.epub_narration(0, opus_tracks={path: 1})).audio), expected_mono)
            with self.assertRaisesRegex(ValueError, "absent from Ogg segment 0"):
                next(book.epub_narration(0, opus_tracks={path: 2}))
            with self.assertRaisesRegex(ValueError, "Opus track index"):
                next(book.epub_narration(0, opus_tracks={path: -1}))

        chained = payload + group(surround, stereo)
        with doc_ex.open(fixture(mono + stereo)) as book:
            expected_chain = pcm(next(book.epub_narration(0)).audio)
        with doc_ex.open(fixture(chained)) as book:
            playback = []
            targets = []
            book.play_epub_narration(
                0, opus_tracks={path: 1}, chunk_frames=751, skip_roles={"aside"},
                play_audio=lambda audio: playback.append(pcm(audio)), speak_text=self.fail, on_cue=targets.append,
            )
            self.assertEqual(b"".join(playback), expected_chain)
            self.assertEqual(len(targets), 2)
            self.assertIsNone(targets[-1])
        with doc_ex.open(fixture(payload + stereo)) as book:
            with self.assertRaisesRegex(ValueError, "absent from Ogg segment 1"):
                next(book.epub_narration(0, opus_tracks={path: 1}))

        # A corrupt page on an unselected Opus stream still invalidates the resource.
        damaged = payload[:-1] + bytes([payload[-1] ^ 1])
        with doc_ex.open(fixture(damaged)) as book:
            with self.assertRaises(doc_ex.MalformedDocumentError):
                next(book.epub_narration(0, opus_tracks={path: 0}))

        authored_payload = group(stereo, mono)
        authored = "OEBPS/Media/alternate.ogg"
        with doc_ex.open(source) as book:
            with self.assertRaisesRegex(ValueError, "requires an Ogg"):
                next(book.epub_narration(0, opus_tracks={path: 0}))
            edited = book.edit_epub_structure(
                [0, 1],
                resources={authored: doc_ex.EpubResource("audio/ogg; codecs=opus", authored_payload)},
                overlays={"OEBPS/one.xhtml": [doc_ex.EpubOverlayCue("café", authored, 0, 20_000)]},
            )
        with doc_ex.open(edited) as book:
            self.assertEqual(book.read_epub_resource(authored), authored_payload)
            audio = next(book.epub_narration(0, opus_tracks={authored: 1})).audio
            self.assertEqual(pcm(audio), expected_mono[:960 * 2])

    def test_media_overlay_rejects_damaged_ogg_transport(self) -> None:
        source, _ = narrated_epub()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"audio/wav", b"audio/ogg")
        fixtures = Path(__file__).parent / "fixtures"
        data = (fixtures / "epub-narration-stereo.opus").read_bytes()
        other = (fixtures / "epub-narration-mono.opus").read_bytes()
        damaged = bytearray(data)
        damaged[-1] ^= 1
        damaged_link = bytearray(other)
        damaged_link[-1] ^= 1
        first_page = 27 + data[26] + sum(data[27:27 + data[26]])
        for payload, error in (
            (data[:-1], doc_ex.MalformedDocumentError),
            (bytes(damaged), doc_ex.MalformedDocumentError),
            (data + data, doc_ex.MalformedDocumentError),
            (data + other[:-1], doc_ex.MalformedDocumentError),
            (data + damaged_link, doc_ex.MalformedDocumentError),
            (data[:first_page] + other + data[first_page:], doc_ex.UnsupportedFormatError),
        ):
            source_with_audio = rewrite_epub(source, {
                "OEBPS/content.opf": package, "OEBPS/Media/voice.wav": payload,
            })
            with self.subTest(error=error, size=len(payload)), doc_ex.open(source_with_audio) as document:
                with self.assertRaises(error):
                    next(document.epub_narration(0))

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

    def test_mathml_inherits_presentation_styles_without_duplicating_annotations(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
        <body><p style="font-size: 18px">Before
          <m:math mathcolor="#b52040" mathsize="24px">
            <m:semantics><m:mrow><m:mi>H</m:mi><m:mo>+</m:mo>
              <m:mstyle mathcolor="#1659b0" mathsize="150%" mathvariant="bold">
                <m:mi>N</m:mi><m:mi mathvariant="normal" mathcolor="currentColor">R</m:mi>
              </m:mstyle><m:mo>&amp;</m:mo><m:mi>Z</m:mi></m:mrow>
              <m:annotation encoding="text/plain">duplicate alternative representation</m:annotation>
            </m:semantics>
          </m:math> After</p>
        </body></html>"""
        namespaces = {
            "m": "http://schemas.openxmlformats.org/officeDocument/2006/math",
            "w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
        }
        w = "{" + namespaces["w"] + "}"
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            text = book[0].get_text()
            self.assertIn("Before", text)
            self.assertIn("After", text)
            self.assertNotIn("duplicate alternative", text)
            data = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            runs = {run.findtext("m:t", namespaces=namespaces): run for run in xml.findall(".//m:r", namespaces)}
            for token, ink, half_points in (("H", "B52040", "36"), ("N", "1659B0", "54"), ("R", "1659B0", "54")):
                with self.subTest(token=token):
                    self.assertEqual(runs[token].find("w:rPr/w:color", namespaces).get(w + "val").upper(), ink)
                    self.assertEqual(runs[token].find("w:rPr/w:sz", namespaces).get(w + "val"), half_points)
            self.assertIn("&", runs)
            self.assertIsNotNone(runs["&"].find("m:rPr/m:lit", namespaces))
            with doc_ex.open(data) as word, doc_ex.open(book.convert("pdf")) as pdf:
                for output in (book, word, pdf):
                    _, pixels = rgba_pixels(png_chunks(output.render_page(0, dpi=96)))
                    self.assertTrue(bytes((181, 32, 64, 255)) in pixels, "missing equation foreground")
                    self.assertTrue(bytes((22, 89, 176, 255)) in pixels, "missing nested run color")
                self.assertIn("Before", pdf[0].get_text())
                self.assertIn("After", word[0].get_text())

    def test_mathml_normalizes_xml_whitespace_but_keeps_nonbreaking_spaces(self) -> None:
        def one_chapter(formula: str) -> bytes:
            return epub_fixture(chapter_one=f'<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML"><body><p>Before {formula} After</p></body></html>')

        compact = "<m:math><m:mrow><m:mi>x</m:mi><m:mo>+</m:mo><m:mtext>&#160;two terms&#160;</m:mtext></m:mrow></m:math>"
        pretty = """<m:math><m:semantics>
          <m:mrow>
            <m:mi> x </m:mi>
            <m:mo> + </m:mo>
            <m:mtext>
              &#160;two	 terms&#160;
            </m:mtext>
          </m:mrow>
          <m:annotation encoding="text/plain">Other representation</m:annotation>
        </m:semantics></m:math>"""
        with doc_ex.open(one_chapter(pretty)) as book, doc_ex.open(one_chapter(compact)) as control:
            expected = "Before x+\u00a0two terms\u00a0 After"
            self.assertEqual(book[0].get_text(), expected)
            self.assertEqual(book[0].get_text(), control[0].get_text())
            equation = book[0].blocks[0].metadata["mathml"][0]
            self.assertEqual(expected[equation["start"]:equation["end"]], "x+\u00a0two terms\u00a0")
            self.assertIn("\n            <m:mi> x </m:mi>", equation["source"])
            self.assertIn("Other representation", equation["source"])
            self.assertEqual(book.render_page(0, dpi=96), control.render_page(0, dpi=96))
            with zipfile.ZipFile(io.BytesIO(book.convert("docx"))) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            math_text = [node.text for node in xml.iter("{http://schemas.openxmlformats.org/officeDocument/2006/math}t")]
            self.assertIn("\u00a0two terms\u00a0", math_text)

    def test_large_mathml_tree_uses_equation_styles_and_bounds_xhtml_boxes(self) -> None:
        def chapter(body: str) -> bytes:
            return epub_fixture(chapter_one=f"""<html xmlns="http://www.w3.org/1999/xhtml"
              xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Chapter</title>
              <style>p {{ font-size: 24px }} .formula .accent {{ color: #d63030 }}
              .formula :is(.accent) {{ color: #1644df }}</style></head>
              <body><p>{body}</p></body></html>""")

        def formula(rows: int) -> str:
            return '<m:math class="formula"><m:mrow>' + '<m:mrow/>' * rows + '<m:mi class="accent accent">X</m:mi></m:mrow></m:math>'

        with doc_ex.open(chapter(formula(8100))) as large, doc_ex.open(chapter(formula(0))) as compact:
            preview = large.render_page(0, dpi=96)
            self.assertEqual(preview, compact.render_page(0, dpi=96))
            _, pixels = rgba_pixels(png_chunks(preview))
            self.assertIn(bytes((22, 68, 223, 255)), pixels)
            with doc_ex.open(large.convert("docx")) as word:
                _, pixels = rgba_pixels(png_chunks(word.render_page(0, dpi=96)))
                self.assertIn(bytes((22, 68, 223, 255)), pixels)
        with doc_ex.open(chapter('<span/>' * 8100 + 'Text')) as html:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, '8000 box elements'):
                html.render_page(0, dpi=96)

    def test_tall_reflowable_chapter_renders_with_bounded_raster(self) -> None:
        source = epub_fixture(chapter_one="""<html xmlns="http://www.w3.org/1999/xhtml"><head>
          <style>body { margin: 0 } .space { height: 16000px }
          .end { height: 90px; background-color: #a93372 }</style></head>
          <body><div class="space"/><div class="space"/><div class="end"/></body></html>""")
        with doc_ex.open(source) as book:
            full = png_chunks(book.render_page(0, dpi=24))
            self.assertEqual(struct.unpack_from(">II", full[b"IHDR"]), (204, 8023))
            with self.assertRaises(doc_ex.UnsafeArchiveError):
                book.render_page(0, dpi=96)
            tail = png_chunks(book.render_page(0, dpi=96, clip=(3, 24_003, 12, 15)))
            self.assertEqual(struct.unpack_from(">II", tail[b"IHDR"]), (16, 20))
            self.assertEqual(rgba_pixels(tail)[1], bytes((169, 51, 114, 255)) * 320)

    def test_clipped_packaged_font_preview_uses_only_visible_outlines(self) -> None:
        font = (Path(__file__).parent / "fixtures" / "fonts" / "InterVariable.ttf").read_bytes()
        sentence = "Amber blue green: crooked letters cross the clip. "

        def chapter(repetitions: int) -> bytes:
            html = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Chapter</title><style>
              @font-face {{ font-family: Reader; src: url(inter.ttf); font-weight: 100 900 }}
              body, p {{ margin:0 }} p {{ font-family:Reader; font-size:12pt; font-weight:641.25;
              font-style:oblique -23.5deg; text-decoration:underline; color:#15324e }}
              </style></head><body><p>{sentence * repetitions}</p></body></html>"""
            source = epub_fixture(chapter_one=html)
            with doc_ex.open(source) as book:
                return book.edit_epub_structure(resources={
                    "OEBPS/inter.ttf": doc_ex.EpubResource("font/ttf", font),
                })

        with (doc_ex.open(chapter(10)) as compact, doc_ex.open(chapter(1_800)) as long):
            width, reference = rgba_pixels(png_chunks(compact.render_page(0, dpi=96)))
            clip = (12, 3, 72, 18)
            cropped = png_chunks(long.render_page(0, dpi=96, clip=clip))
            self.assertEqual(struct.unpack_from(">II", cropped[b"IHDR"]), (96, 24))
            _, pixels = rgba_pixels(cropped)
            expected = bytearray()
            for row in range(4, 28):
                start = (row * width + 16) * 4
                expected.extend(reference[start:start + 96 * 4])
            self.assertEqual(pixels, expected)
            self.assertIn(bytes((21, 50, 78, 255)), pixels)
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "outline budget"):
                long.render_page(0, dpi=24)

    def test_mathml_applies_chapter_css_in_preview_and_native_exports(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Math styles</title><link rel="stylesheet" href="math.css"/>
            <style>.compact { math-style: compact } .raised { math-style: normal }</style>
          </head><body><p>Before
            <m:math class="formula" mathsize="40px" mathcolor="#770077">
              <m:mi>H</m:mi><m:mo>+</m:mo>
              <m:mrow class="emphasis"><m:mi>N</m:mi>
                <m:mi mathcolor="#770077" style="font-size: 50%; color: currentColor; font-weight: normal; font-style: italic">R</m:mi>
                <m:mi mathsize="8pt" mathcolor="#770077" style="font-size: inherit; color: inherit; font-weight: normal">I</m:mi>
              </m:mrow>
              <m:munder class="compact" displaystyle="true"><m:mo movablelimits="true">∑</m:mo><m:mn>2</m:mn></m:munder>
              <m:munder class="raised" displaystyle="false"><m:mo movablelimits="true">∑</m:mo><m:mn>3</m:mn></m:munder>
            </m:math> After
          </p></body></html>"""
        source = epub_fixture(chapter_one=chapter)
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b"</manifest>", b'<item id="css" href="math.css" media-type="text/css"/></manifest>',
            )
        source = rewrite_epub(source, {
            "OEBPS/content.opf": package,
            "OEBPS/math.css": "body {font-size: 20px} .formula {font-size: 200%; color: #d02444} "
            ".formula .emphasis {font-size: 150%; color: #1444cc; font-weight: 700; font-style: normal}",
        })
        ns = {
            "m": "http://schemas.openxmlformats.org/officeDocument/2006/math",
            "w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
        }
        with doc_ex.open(source) as book:
            entry = book[0].blocks[0].metadata["mathml"][0]
            self.assertIn('mathcolor="#770077"', entry["source"])
            self.assertNotIn("css", entry)
            docx = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(docx)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            runs = {run.findtext("m:t", namespaces=ns): run for run in xml.findall(".//m:r", ns)}
            for token, color, half_points, style in (
                ("H", "D02444", "60", "i"), ("N", "1444CC", "90", "b"), ("R", "1444CC", "45", "i"),
                ("I", "1444CC", "90", "p"),
            ):
                with self.subTest(token=token):
                    self.assertEqual(runs[token].find("w:rPr/w:color", ns).get("{" + ns["w"] + "}val").upper(), color)
                    self.assertEqual(runs[token].find("w:rPr/w:sz", ns).get("{" + ns["w"] + "}val"), half_points)
                    self.assertEqual(runs[token].find("m:rPr/m:sty", ns).get("{" + ns["m"] + "}val"), style)
            modes = [node.get("{" + ns["m"] + "}val") for node in xml.findall(".//m:limLoc", ns)]
            self.assertEqual(modes, ["subSup", "undOvr"])
            with doc_ex.open(docx) as word, doc_ex.open(book.convert("pdf")) as pdf:
                for output in (book, word, pdf):
                    width, pixels = rgba_pixels(png_chunks(output.render_page(0, dpi=96)))
                    self.assertGreater(width, 0)
                    for color in (bytes((208, 36, 68, 255)), bytes((20, 68, 204, 255))):
                        self.assertIn(color, pixels)
                    self.assertNotIn(bytes((119, 0, 119, 255)), pixels)

    def test_fixed_layout_mathml_css_uses_chapter_viewport(self) -> None:
        def fixed_math(formula: str) -> bytes:
            source = epub_fixture(chapter_one=f"""<html xmlns="http://www.w3.org/1999/xhtml"
              xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Viewport math</title>
              <meta name="viewport" content="width=300,height=200"/></head><body><p>
              {formula}</p></body></html>""")
            with zipfile.ZipFile(io.BytesIO(source)) as archive:
                package = archive.read("OEBPS/content.opf").replace(
                    b'<itemref idref="one"/>',
                    b'<itemref idref="one" properties="rendition:layout-pre-paginated"/>',
                )
            return rewrite_epub(source, {"OEBPS/content.opf": package})

        formula = "<m:mfrac><m:mi>X</m:mi><m:mn>7</m:mn></m:mfrac>"
        source = fixed_math(f'<m:math style="font-size: 10vw; color: #1245db">{formula}</m:math>')
        expected = fixed_math(f'<m:math mathsize="22.5pt" mathcolor="#1245db">{formula}</m:math>')
        with doc_ex.open(source) as styled, doc_ex.open(expected) as control:
            preview = styled.render_page(0, dpi=96)
            self.assertEqual(preview, control.render_page(0, dpi=96))
            self.assertEqual(struct.unpack_from(">II", preview, 16), (300, 200))
            with doc_ex.open(styled.export_epub_fixed_pdf(dpi=96, pages=[0])) as visual:
                self.assertEqual(len(visual), 1)
                image = visual.render_page(0, dpi=96)
                self.assertEqual(struct.unpack_from(">II", image, 16), (300, 200))
                _, pixels = rgba_pixels(png_chunks(image))
                self.assertIn(bytes((18, 69, 219, 255)), pixels)

    def test_mathml_projects_accents_movable_limits_fences_and_tensor_pairs(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
        <body><p><m:math>
          <m:mover accent="true"><m:mi>x</m:mi><m:mo>^</m:mo></m:mover>
          <m:munder accentunder="true"><m:mi>y</m:mi><m:mo>⏟</m:mo></m:munder>
          <m:mover accent="false"><m:mi>z</m:mi><m:mo>¯</m:mo></m:mover>
          <m:munder><m:mo movablelimits="true" largeop="false">∑</m:mo><m:mn>2</m:mn></m:munder>
          <m:munder><m:mo movablelimits="false">∑</m:mo><m:mn>3</m:mn></m:munder>
          <m:munder><m:mo movablelimits="true">lim</m:mo><m:mi>x</m:mi></m:munder>
          <m:mfenced open="[" close="]" separators=",;"><m:mn>1</m:mn><m:mn>2</m:mn><m:mn>3</m:mn><m:mn>4</m:mn></m:mfenced>
          <m:mmultiscripts><m:mi>T</m:mi><m:mi>i</m:mi><m:none/><m:mi>j</m:mi><m:mi>k</m:mi>
            <m:mprescripts/><m:none/><m:mi>l</m:mi><m:mi>m</m:mi><m:none/></m:mmultiscripts>
          <m:menclose notation="radical"><m:mi>q</m:mi></m:menclose>
        </m:math></p></body></html>"""
        namespace = "{http://schemas.openxmlformats.org/officeDocument/2006/math}"
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            word = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(word)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            for tag, count in (("acc", 1), ("groupChr", 1), ("limUpp", 1), ("nary", 2), ("sSub", 1), ("sSubSup", 2), ("sPre", 2), ("rad", 1)):
                self.assertEqual(len(xml.findall(".//" + namespace + tag)), count, tag)
            limit_modes = xml.findall(".//" + namespace + "limLoc")
            self.assertEqual([mode.get(namespace + "val") for mode in limit_modes], ["subSup", "undOvr"])
            self.assertIn("1,2;3;4", "".join(xml.itertext()))
            for target, data in (("docx", word), ("pdf", book.convert("pdf"))):
                with self.subTest(target=target), doc_ex.open(data) as converted:
                    self.assertEqual(len(converted), 2)
                    preview = converted.render_page(0, dpi=96)
                    self.assertTrue(preview.startswith(b"\x89PNG"))

    def test_mathml_spanning_table_retains_geometry_and_editable_cells(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Spans</title>
          <style>body, p {margin:0;font-size:24px} .wide {color:#8343cd}</style></head>
          <body><p><m:math><m:semantics><m:mtable columnspacing="18pt" rowspacing="10pt">
          <m:mtr><m:mtd rowspan="2" rowalign="center" mathcolor="#bf222d"><m:mtext>Red</m:mtext></m:mtd>
            <m:mtd mathcolor="#198144"><m:mtext>Green</m:mtext></m:mtd>
            <m:mtd mathcolor="#1644df"><m:mtext>Blue</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd columnspan="2" columnalign="right" class="wide"><m:mtext>Right</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd columnspan="3" mathcolor="#ac661b"><m:mfrac><m:mtext>Total</m:mtext>
            <m:mn>7</m:mn></m:mfrac></m:mtd></m:mtr>
          </m:mtable><m:annotation>annotation marker</m:annotation></m:semantics></m:math></p></body></html>"""
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            self.assertIn("RedGreenBlueRightTotal7", book[0].get_text())
            self.assertNotIn("annotation marker", book[0].get_text())
            width, pixels = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))

            def bounds(color: str) -> tuple[int, int, int, int]:
                target = bytes.fromhex(color + "ff")
                points = [(i // 4 % width, i // 4 // width) for i in range(0, len(pixels), 4)
                          if pixels[i:i+4] == target]
                self.assertTrue(points, color)
                return min(x for x, _ in points), min(y for _, y in points), max(x for x, _ in points), max(y for _, y in points)

            red, green, blue, purple, orange = map(bounds, ("bf222d", "198144", "1644df", "8343cd", "ac661b"))
            self.assertLess(red[2], green[0])
            self.assertLess(green[2], blue[0])
            self.assertLess(green[1], red[1])
            self.assertLess(red[1], purple[1])
            self.assertAlmostEqual(blue[2], purple[2], delta=4)
            self.assertLess(purple[3], orange[1])
            # Ink bounds include glyph bearings, so allow six pixels at the center.
            self.assertAlmostEqual(red[0] + blue[2], orange[0] + orange[2], delta=12)

            word_bytes = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(word_bytes)) as archive:
                root = ET.fromstring(archive.read("word/document.xml"))
            ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
                  "m": "http://schemas.openxmlformats.org/officeDocument/2006/math"}
            self.assertEqual([n.attrib[f"{{{ns['w']}}}val"] for n in root.findall(".//w:gridSpan", ns)], ["2", "3"])
            self.assertEqual(len(root.findall(".//w:vMerge", ns)), 2)
            self.assertEqual(len(root.findall(".//m:f", ns)), 1)
            self.assertEqual(len(root.findall(".//m:oMath", ns)), 5)
            for data in (word_bytes, book.convert("pdf")):
                with doc_ex.open(data) as reopened:
                    first = reopened[0].get_text()
                    for value in ("Red", "Green", "Blue", "Right", "Total"):
                        self.assertEqual(first.count(value), 1, first)
                    self.assertTrue(reopened.render_page(0, dpi=96).startswith(b"\x89PNG"))

    def test_mathml_nested_spanning_tables_preserve_layout_and_export(self) -> None:
        def fixture(table: str) -> bytes:
            return epub_fixture(chapter_one=f"""<html xmlns="http://www.w3.org/1999/xhtml"
              xmlns:m="http://www.w3.org/1998/Math/MathML">
              <head><style>body, p {{margin:0; font-size:24px}}
              .inner {{color:#a32785}}</style></head>
              <body><p><m:math>{table}</m:math></p></body></html>""")

        inner = """<m:mstyle class="inner"><m:mtable rowspacing="8pt" columnspacing="12pt">
          <m:mtr><m:mtd columnspan="2"><m:mtext>InnerHead</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd mathcolor="#d52b37"><m:mtext>InnerLeft</m:mtext></m:mtd>
            <m:mtd mathcolor="#158245"><m:mfrac><m:mtext>InnerRight</m:mtext>
              <m:mn>7</m:mn></m:mfrac></m:mtd></m:mtr></m:mtable></m:mstyle>"""
        outer = """<m:mtable columnspacing="14pt">
          <m:mtr><m:mtd rowspan="2" rowalign="top">{}</m:mtd>
            <m:mtd mathcolor="#1428d7"><m:mtext>OuterTop</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd><m:mtext>OuterBottom</m:mtext></m:mtd></m:mtr></m:mtable>"""

        with doc_ex.open(fixture(outer.format(inner))) as book:
            tokens = ("InnerHead", "InnerLeft", "InnerRight", "OuterTop", "OuterBottom")
            self.assertEqual(book[0].get_text(), "InnerHeadInnerLeftInnerRight7OuterTopOuterBottom")
            width, pixels = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))

            def bounds(color: str) -> tuple[int, int, int, int]:
                target = bytes.fromhex(color + "ff")
                points = [(i // 4 % width, i // 4 // width) for i in range(0, len(pixels), 4)
                          if pixels[i:i+4] == target]
                self.assertTrue(points, color)
                return min(x for x, _ in points), min(y for _, y in points), max(x for x, _ in points), max(y for _, y in points)

            head, left, right, outside = map(bounds, ("a32785", "d52b37", "158245", "1428d7"))
            self.assertLess(head[3], left[1])
            self.assertLess(head[3], right[1])
            self.assertLess(left[2], right[0])
            self.assertLess(right[2], outside[0])

            word = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(word)) as archive:
                document = ET.fromstring(archive.read("word/document.xml"))
            ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
                  "m": "http://schemas.openxmlformats.org/officeDocument/2006/math"}
            outer_table = document.find("./w:body/w:tbl", ns)
            self.assertIsNotNone(outer_table)
            inner_table = outer_table.find("./w:tr/w:tc/w:tbl", ns)
            self.assertIsNotNone(inner_table)
            self.assertEqual(len(outer_table.findall("./w:tr/w:tc/w:tcPr/w:vMerge", ns)), 2)
            self.assertEqual(inner_table.find(".//w:gridSpan", ns).get(f"{{{ns['w']}}}val"), "2")
            self.assertEqual(len(inner_table.findall(".//m:oMath", ns)), 3)
            self.assertEqual(len(inner_table.findall(".//m:f", ns)), 1)

            for converted in (word, book.convert("pdf")):
                with doc_ex.open(converted) as reopened:
                    text = reopened[0].get_text()
                    for token in tokens:
                        self.assertEqual(text.count(token), 1, text)
                    self.assertTrue(reopened.render_page(0, dpi=96).startswith(b"\x89PNG"))

        invalid_span = inner.replace('columnspan="2"', 'rowspan="3"')
        with doc_ex.open(fixture(outer.format(invalid_span))) as book:
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "MathML table spans overlap"):
                book.render_page(0, dpi=96)
        with doc_ex.open(fixture(outer.format("<m:mrow><m:mi>x</m:mi>" + inner + "</m:mrow>"))) as book:
            converted = book.convert("docx")
        with zipfile.ZipFile(io.BytesIO(converted)) as archive:
            document = archive.read("word/document.xml")
        self.assertIn(b"<w:tbl>", document)
        self.assertIn(b"<m:m>", document)
        with doc_ex.open(converted) as reopened:
            self.assertEqual(reopened[0].get_text().count("InnerHead"), 1)
            self.assertTrue(reopened.render_page(0, dpi=96).startswith(b"\x89PNG"))

        deep = inner
        for _ in range(9):
            deep = outer.format(deep)
        with doc_ex.open(fixture(deep)) as book:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "MathML structure"):
                book.render_page(0, dpi=96)

        sparse = ('<m:mtable><m:mtr><m:mtd columnspan="32"><m:mi>x</m:mi></m:mtd></m:mtr>'
                  + '<m:mtr/>' * 127 + '</m:mtable>')
        wide = '<m:mtable>' + f'<m:mtr><m:mtd>{sparse}</m:mtd></m:mtr>' * 13 + '</m:mtable>'
        with doc_ex.open(fixture(wide)) as book:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "MathML table cells"):
                book.render_page(0, dpi=96)

    def test_mathml_spanning_table_inline_preview_and_invalid_grid(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Math spans</title></head>
          <body><p>{}</p></body></html>"""
        table = """<m:math><m:mtable><m:mtr><m:mtd columnspan="2"><m:mi>X</m:mi></m:mtd></m:mtr>
          <m:mtr><m:mtd><m:mi>A</m:mi></m:mtd><m:mtd><m:mi>B</m:mi></m:mtd></m:mtr></m:mtable></m:math>"""
        with doc_ex.open(epub_fixture(chapter_one=template.format("Before " + table + " after"))) as book:
            self.assertIn("Before XAB after", book[0].get_text())
            self.assertTrue(book.render_page(0, dpi=96).startswith(b"\x89PNG"))
            with doc_ex.open(book.convert("docx")) as word:
                text = word[0].get_text()
                for token in ("Before", "X", "A", "B", "after"):
                    self.assertIn(token, text)
                self.assertNotIn("⟡H", text)
        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 300, 200)])
        viewport = '<meta name="viewport" content="width=300,height=200"/>'
        chapter = template.format(table.replace("<m:math>", '<m:math mathcolor="#1644df">'))
        fixed = rewrite_epub(fixed, {"OEBPS/s0.xhtml": chapter.replace("</head>", viewport + "</head>")})
        with doc_ex.open(fixed) as book:
            png = book.render_page(0, dpi=96)
            self.assertEqual(struct.unpack_from(">II", png, 16), (300, 200))
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96)) as pdf:
                _, pixels = rgba_pixels(png_chunks(pdf.render_page(0, dpi=96)))
                self.assertIn(bytes.fromhex("1644dfff"), pixels)

        invalid = """<m:math><m:mtable>
          <m:mtr><m:mtd><m:mi>A</m:mi></m:mtd><m:mtd rowspan="2"><m:mi>B</m:mi></m:mtd></m:mtr>
          <m:mtr><m:mtd columnspan="2"><m:mi>X</m:mi></m:mtd></m:mtr></m:mtable></m:math>"""
        with doc_ex.open(epub_fixture(chapter_one=template.format(invalid))) as book:
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "MathML table spans overlap"):
                book.render_page(0, dpi=96)

    def test_mathml_space_and_zero_width_padding_affect_rendered_geometry(self) -> None:
        def fixture(math: str) -> bytes:
            return epub_fixture(chapter_one=f"""<html xmlns="http://www.w3.org/1999/xhtml"
              xmlns:m="http://www.w3.org/1998/Math/MathML"><body><p>
              <m:math mathsize="18pt" mathvariant="normal">{math}</m:math></p></body></html>""")

        red = '<m:mi mathcolor="#d61020">H</m:mi>'
        blue = '<m:mi mathcolor="#2344cc">M</m:mi>'
        ink = bytes((35, 68, 204, 255))

        def left(preview: bytes) -> int:
            width, pixels = rgba_pixels(png_chunks(preview))
            return min((offset // 4) % width for offset in range(0, len(pixels), 4) if pixels[offset:offset+4] == ink)

        with (
            doc_ex.open(fixture(red + blue)) as baseline,
            doc_ex.open(fixture(red + '<m:mspace width="1em"/>' + blue)) as spaced,
            doc_ex.open(fixture('<m:mpadded width="0">' + red + '</m:mpadded>' + blue)) as padded,
        ):
            start = left(baseline.render_page(0, dpi=96))
            self.assertAlmostEqual(left(spaced.render_page(0, dpi=96)) - start, 24, delta=1)
            self.assertLess(left(padded.render_page(0, dpi=96)), start - 8)
            with zipfile.ZipFile(io.BytesIO(padded.convert("docx"))) as archive:
                self.assertIn(b'<m:zeroWid m:val="1"/>', archive.read("word/document.xml"))
        for math in ('<m:mspace width="1em" linebreak="newline"/>',
                     '<m:mpadded voffset="invalid"><m:mi>x</m:mi></m:mpadded>'):
            with self.subTest(math=math), doc_ex.open(fixture(red + math)) as unsupported:
                with self.assertRaises(doc_ex.UnsupportedFormatError):
                    unsupported.render_page(0, dpi=96)

    def test_mathml_composes_spanning_tables_inside_fractions_radicals_and_scripts(self) -> None:
        def grid(head: str, left: str, right: str) -> str:
            return f"""<m:mtable rowspacing="4pt" columnspacing="8pt">
              <m:mtr><m:mtd columnspan="2" mathcolor="#{head}"><m:mtext>Top</m:mtext></m:mtd></m:mtr>
              <m:mtr><m:mtd mathcolor="#{left}"><m:mtext>Left</m:mtext></m:mtd>
                <m:mtd mathcolor="#{right}"><m:mtext>Right</m:mtext></m:mtd></m:mtr>
              </m:mtable>"""

        upper = grid("db2828", "17669a", "a15a15")
        lower = grid("7821b8", "057643", "b93181")
        exponent = grid("1000cc", "be5609", "2a7040")
        formula = f"""<m:math mathsize="21pt" display="block">
         <m:mfrac>
           <m:mfenced><m:mrow><m:mi>x</m:mi>{upper}</m:mrow></m:mfenced>
           <m:msqrt><m:mrow>{lower}<m:mi>y</m:mi></m:mrow></m:msqrt>
         </m:mfrac>
         <m:mo>+</m:mo><m:msup><m:mi mathcolor="#7b2100">B</m:mi>
         <m:mrow>{exponent}<m:mi>z</m:mi></m:mrow></m:msup></m:math>"""
        xhtml = f"""<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Composition</title><meta name="viewport" content="width=500,height=400"/></head>
          <body><p>{formula}</p></body></html>"""
        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 500, 400)])
        fixed = rewrite_epub(fixed, {"OEBPS/s0.xhtml": xhtml})

        def check_layout(png: bytes) -> None:
            width, pixels = rgba_pixels(png_chunks(png))
            def bounds(color: str) -> tuple[int, int, int, int]:
                rgba = bytes.fromhex(color + "ff")
                points = [(i // 4 % width, i // 4 // width)
                          for i in range(0, len(pixels), 4) if pixels[i:i + 4] == rgba]
                self.assertTrue(points, color)
                return min(x for x, _ in points), min(y for _, y in points), max(x for x, _ in points), max(y for _, y in points)
            for head, left, right in (("db2828", "17669a", "a15a15"),
                                      ("7821b8", "057643", "b93181"),
                                      ("1000cc", "be5609", "2a7040")):
                self.assertLess(bounds(head)[3], bounds(left)[1])
                self.assertLess(bounds(left)[2], bounds(right)[0])
            self.assertLess(bounds("17669a")[3], bounds("7821b8")[1])
            self.assertLess(bounds("1000cc")[3], bounds("7b2100")[1])
            self.assertNotIn(b"\x00\x00\x01\xff", pixels)

        with doc_ex.open(fixed) as book:
            check_layout(book.render_page(0, dpi=120))
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=120)) as pdf:
                check_layout(pdf.render_page(0, dpi=120))
        with doc_ex.open(epub_fixture(chapter_one=xhtml)) as book:
            self.assertIn("TopLeftRight", book[0].get_text())
            with doc_ex.open(book.convert("pdf")) as pdf:
                check_layout(pdf.render_page(0, dpi=120))
            word_bytes = book.convert("docx")
        with zipfile.ZipFile(io.BytesIO(word_bytes)) as archive:
            xml = archive.read("word/document.xml")
        for tag in (b"<m:f>", b"<m:rad>", b"<m:sSup>"):
            self.assertIn(tag, xml)
        self.assertEqual(xml.count(b"<m:m>"), 3)
        with doc_ex.open(word_bytes) as word:
            check_layout(word.render_page(0, dpi=120))
            for label in ("Top", "Left", "Right"):
                self.assertEqual(word[0].get_text().count(label), 3)

    def test_mathml_editable_inline_spans_keep_measured_grid_geometry(self) -> None:
        table = """<m:mtable columnspacing="9pt" rowspacing="5pt">
          <m:mtr><m:mtd rowspan="2" rowalign="center" mathcolor="#bf222d"><m:mtext>Red</m:mtext></m:mtd>
            <m:mtd columnspan="2" columnalign="right" mathcolor="#8343cd"><m:mtext>Head</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd mathcolor="#198144"><m:mtext>Green</m:mtext></m:mtd>
            <m:mtd mathcolor="#1644df"><m:mtext>Blue</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd columnspan="3" mathcolor="#ac661b"><m:mtext>Total</m:mtext></m:mtd></m:mtr></m:mtable>"""
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Editable grid</title><style>body {{margin:0}}</style></head>
          <body><p>Before <m:math mathsize="20pt">{}</m:math> after.</p></body></html>"""
        labels = ("Red", "Head", "Green", "Blue", "Total")

        def positions(doc: doc_ex.Document) -> list[tuple[int, int]]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            starts = []
            for color in ("bf222d", "8343cd", "198144", "1644df", "ac661b"):
                ink = bytes.fromhex(color + "ff")
                points = [divmod(i // 4, width) for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                self.assertTrue(points, color)
                starts.append((min(x for _, x in points), min(y for y, _ in points)))
            return starts

        with doc_ex.open(epub_fixture(chapter_one=template.format(table))) as book:
            preview = positions(book)
            converted = book.convert("docx")
        with zipfile.ZipFile(io.BytesIO(converted)) as archive:
            document = ET.fromstring(archive.read("word/document.xml"))
        ns = {"m": "http://schemas.openxmlformats.org/officeDocument/2006/math"}
        matrix, = document.findall(".//m:m", ns)
        self.assertEqual([len(row.findall("./m:e", ns)) for row in matrix.findall("./m:mr", ns)], [3, 3, 3])
        runs = [run.text for run in matrix.findall(".//m:t", ns)]
        for label in labels:
            self.assertEqual(runs.count(label), 1)
        with doc_ex.open(converted) as word:
            actual = positions(word)
            text = word[0].get_text()
            for label in ("Before", *labels, "after."):
                self.assertEqual(text.count(label), 1, text)
            self.assertNotIn("⟡H", text)
            self.assertNotIn("⟡g", text)
        for expect, got in zip(preview, actual):
            for axis in range(2):
                self.assertAlmostEqual(expect[axis] - preview[0][axis], got[axis] - actual[0][axis], delta=2)

        tiny = """<m:mtable columnspacing="0"><m:mtr><m:mtd columnspan="2"/></m:mtr>
          <m:mtr><m:mtd/><m:mtd><m:mtext>Tiny</m:mtext></m:mtd></m:mtr></m:mtable>"""
        with doc_ex.open(epub_fixture(chapter_one=template.format(tiny))) as book:
            with doc_ex.open(book.convert("docx")) as reopened:
                text = reopened[0].get_text()
                self.assertEqual(text.count("Tiny"), 1)
                self.assertEqual(text.count("■("), 1)

    def test_mathml_tight_inline_matrix_keeps_editable_cell_geometry(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Tight editable MathML matrix</title><style>body {{margin:0}}</style></head>
          <body><p>Before <m:math mathsize="18pt"><m:mrow>
            <m:mtable columnspacing="{}" rowspacing="3pt" columnwidth="6pt 65pt 48pt" columnalign="left">
              <m:mtr><m:mtd rowspan="2" rowalign="center" mathsize="6pt" mathcolor="#bf222d">
                  <m:mtext>Q</m:mtext></m:mtd>
                <m:mtd mathcolor="#1644df"><m:mtext>Upper</m:mtext></m:mtd>
                <m:mtd><m:mtext>Outer</m:mtext></m:mtd></m:mtr>
              <m:mtr><m:mtd mathcolor="#198144"><m:mtext>Lower</m:mtext></m:mtd>
                <m:mtd><m:mtext>Right</m:mtext></m:mtd></m:mtr>
              <m:mtr><m:mtd columnspan="2" mathsize="12pt" mathcolor="#ac661b">
                <m:mfrac><m:mtext>Numerator</m:mtext><m:mtext>Denominator</m:mtext></m:mfrac></m:mtd>
                <m:mtd mathcolor="#006a71"><m:mtext>Peer</m:mtext></m:mtd></m:mtr>
            </m:mtable><m:mtext mathcolor="#8343cd">Tail</m:mtext>
          </m:mrow></m:math> after.</p></body></html>"""
        colors = ("bf222d", "1644df", "198144", "ac661b", "8343cd", "006a71")

        def positions(doc: doc_ex.Document) -> list[tuple[int, int]]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=192)))
            starts = []
            for color in colors:
                ink = bytes.fromhex(color + "ff")
                points = [divmod(i // 4, width) for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                self.assertTrue(points, color)
                starts.append((min(x for _, x in points), min(y for y, _ in points)))
            x, y = starts[0]
            return [(left - x, top - y) for left, top in starts]

        rasters = []
        for gap, split in (("0", True), ("1.25pt", True), ("9pt", False)):
            with self.subTest(gap=gap), doc_ex.open(epub_fixture(chapter_one=template.format(gap))) as book:
                preview = positions(book)
                exported = book.convert("docx")
                with doc_ex.open(exported) as reopened:
                    rendered = positions(reopened)
                    if split:
                        rasters.append((preview, rendered))
                    # The vector boxes compare placement across renderers
                    # without relying on the smallest glyph's solid pixel.
                    with doc_ex.open(book.convert("pdf")) as source_pdf, doc_ex.open(reopened.convert("pdf")) as docx_pdf:
                        for label in ("Q", "Upper", "Lower", "Numerator", "Denominator", "Peer", "Tail"):
                            expected, = source_pdf[0].search_for(label)
                            actual, = docx_pdf[0].search_for(label)
                            for expected_point, actual_point in zip(expected, actual):
                                self.assertAlmostEqual(expected_point, actual_point, delta=0.2)
                    text = reopened[0].get_text()
                    labels = ("Before", "Q", "Upper", "Outer", "Lower", "Right", "Numerator", "Denominator",
                              "Peer", "Tail", "after.")
                    for label in labels:
                        self.assertEqual(text.count(label), 1, text)
                    self.assertEqual([text.index(label) for label in labels],
                                     sorted(text.index(label) for label in labels))
                    self.assertEqual(text.count("■("), 1, text)
                    self.assertNotIn("⟡H", text)
                with zipfile.ZipFile(io.BytesIO(exported)) as archive:
                    xml = ET.fromstring(archive.read("word/document.xml"))
                ns = {"m": "http://schemas.openxmlformats.org/officeDocument/2006/math"}
                tracks = xml.findall(".//m:m", ns)
                self.assertEqual(len(tracks), 3 if split else 1)
                for matrix in tracks:
                    self.assertEqual([len(row.findall("./m:e", ns)) for row in matrix.findall("./m:mr", ns)],
                                     [1, 1, 1] if split else [3, 3, 3])
                self.assertIsNotNone(tracks[0].find(".//m:f", ns))
                if split:
                    middle_text = [run.text for run in tracks[1].findall(".//m:t", ns)]
                    self.assertIn("Upper", middle_text)
                    self.assertIn("Lower", middle_text)
                    self.assertIn("Peer", [run.text for run in tracks[2].findall(".//m:t", ns)])
        for zero, spaced in zip(*rasters):
            for index, gaps in ((1, 1), (2, 1), (4, 2), (5, 2)):
                self.assertAlmostEqual(spaced[index][0] - zero[index][0], gaps * 1.25 * 192 / 72, delta=1)
            self.assertEqual(spaced[3][0], zero[3][0])
            for original, shifted in zip(zero, spaced):
                self.assertAlmostEqual(shifted[1], original[1], delta=1)

    def test_mathml_matrix_honors_authored_spacing_and_alignment(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Aligned MathML</title><style>body {{margin:0}}</style></head>
          <body><p><m:math mathsize="20pt"><m:mtext mathcolor="#5920ac">Anchor</m:mtext>
            <m:mtable mathsize="{table_size}pt" columnalign="left" rowalign="bottom baseline" align="{align}"
                rowspacing="{row_gap}pt" columnspacing="{column_gap}pt {last_gap}pt">
              <m:mtr mathsize="20pt" columnalign="right left" rowalign="top">
                <m:mtd mathcolor="#bf222d"><m:mtext>Red</m:mtext></m:mtd>
                <m:mtd mathcolor="#198144" mathsize="32pt"><m:mtext>Green</m:mtext></m:mtd>
                <m:mtd mathcolor="#8343cd"><m:mtext>Right</m:mtext></m:mtd>
              </m:mtr><m:mtr mathsize="20pt">
                <m:mtd mathcolor="#ac661b"><m:mtext>Wider</m:mtext></m:mtd>
                <m:mtd mathcolor="#1644df" columnalign="{cell_align}"><m:mtext>Blue</m:mtext></m:mtd>
                <m:mtd mathcolor="#006a71"><m:mtext>Last</m:mtext></m:mtd>
              </m:mtr>
            </m:mtable></m:math></p></body></html>"""
        colors = {
            "Anchor": "5920ac", "Red": "bf222d", "Green": "198144", "Right": "8343cd",
            "Wider": "ac661b", "Blue": "1644df", "Last": "006a71",
        }

        def source(align="baseline -1", row_gap=14, column_gap=16, last_gap=28, cell_align="right", table_size=20):
            return epub_fixture(chapter_one=template.format(
                align=align, row_gap=row_gap, column_gap=column_gap, last_gap=last_gap, cell_align=cell_align,
                table_size=table_size,
            ))

        def positions(doc: doc_ex.Document) -> dict[str, tuple[int, int]]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            starts = {}
            for label, color in colors.items():
                ink = bytes.fromhex(color + "ff")
                points = [divmod(i // 4, width) for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                self.assertTrue(points, label)
                starts[label] = min(x for _, x in points), min(y for y, _ in points)
            x, y = starts["Anchor"]
            return {label: (left - x, top - y) for label, (left, top) in starts.items()}

        with doc_ex.open(source()) as book:
            preview = positions(book)
            for target in ("pdf", "docx"):
                with self.subTest(target=target), doc_ex.open(book.convert(target)) as reopened:
                    actual = positions(reopened)
                    for label in colors:
                        self.assertAlmostEqual(preview[label][0], actual[label][0], delta=2)
                        self.assertAlmostEqual(preview[label][1], actual[label][1], delta=2)
                        self.assertEqual(reopened[0].get_text().count(label), 1)
        self.assertAlmostEqual(preview["Blue"][1], 0, delta=2)
        self.assertAlmostEqual(preview["Red"][1], preview["Green"][1], delta=2)
        self.assertGreater(preview["Red"][0], preview["Wider"][0] + 12)
        with doc_ex.open(source(align="baseline 2")) as equivalent:
            self.assertEqual(positions(equivalent), preview)
        with doc_ex.open(source(align="axis 2")) as equivalent:
            self.assertEqual(positions(equivalent), preview)
        with doc_ex.open(source(align="axis", table_size=42)) as enlarged, doc_ex.open(source(align="axis")) as original:
            self.assertEqual(positions(enlarged), positions(original))
        with doc_ex.open(source(align="baseline -99")) as out_of_range, doc_ex.open(source(align="baseline")) as whole:
            self.assertEqual(positions(out_of_range), positions(whole))
        with doc_ex.open(source(align="top 1")) as top:
            self.assertGreater(positions(top)["Red"][1], preview["Red"][1] + 35)
        with doc_ex.open(source(row_gap=34, column_gap=36, cell_align="left")) as spaced:
            shifted = positions(spaced)
            self.assertAlmostEqual(preview["Red"][1] - shifted["Red"][1], 20 * 96 / 72, delta=1)
            for label in ("Green", "Right", "Last"):
                self.assertAlmostEqual(shifted[label][0] - preview[label][0], 20 * 96 / 72, delta=1)
            self.assertLess(shifted["Blue"][0] - preview["Blue"][0], 15)
        with doc_ex.open(source(last_gap=48)) as spaced:
            shifted = positions(spaced)
            self.assertEqual(shifted["Green"], preview["Green"])
            self.assertAlmostEqual(shifted["Right"][0] - preview["Right"][0], 20 * 96 / 72, delta=1)

    def test_mathml_table_equal_and_fixed_tracks_reopen(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>MathML track sizes</title><style>body {{margin:0}}</style></head>
          <body><p><m:math mathsize="18pt">{}</m:math></p></body></html>"""
        table = """<m:mtable rowalign="top" columnalign="left" rowspacing="6pt" columnspacing="10pt" {}>
          <m:mtr><m:mtd mathcolor="#bf222d"><m:mtext>H</m:mtext></m:mtd>
            <m:mtd mathsize="36pt"><m:mtext>High</m:mtext></m:mtd>
            <m:mtd><m:mtext>Long</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd mathcolor="#ac661b"><m:mtext>H</m:mtext></m:mtd>
            <m:mtd mathcolor="#1644df"><m:mtext>H</m:mtext></m:mtd>
            <m:mtd mathcolor="#8343cd"><m:mtext>H</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd mathcolor="#006a71"><m:mtext>H</m:mtext></m:mtd>{}</m:mtr>
          </m:mtable>"""
        equal = 'equalrows="true" equalcolumns="true"'
        tail = '<m:mtext mathcolor="#5920ac">End</m:mtext>'
        colors = {
            "upper": "bf222d", "middle": "ac661b", "lower": "006a71",
            "second": "1644df", "third": "8343cd", "end": "5920ac",
        }

        def fixture(attributes: str) -> bytes:
            return epub_fixture(chapter_one=template.format(table.format(attributes, "") + tail))

        def positions(doc: doc_ex.Document) -> dict[str, tuple[int, int]]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            starts = {}
            for label, color in colors.items():
                ink = bytes.fromhex(color + "ff")
                points = [divmod(i // 4, width) for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                self.assertTrue(points, label)
                starts[label] = min(x for _, x in points), min(y for y, _ in points)
            x, y = starts["upper"]
            return {label: (left - x, top - y) for label, (left, top) in starts.items()}

        with doc_ex.open(fixture(equal)) as book:
            preview = positions(book)
            self.assertAlmostEqual(preview["lower"][1], 2 * preview["middle"][1], delta=2)
            self.assertAlmostEqual(preview["third"][0], 2 * preview["second"][0], delta=2)
            for target in ("pdf", "docx"):
                with self.subTest(target=target), doc_ex.open(book.convert(target)) as reopened:
                    placed = positions(reopened)
                    for label in colors:
                        self.assertAlmostEqual(placed[label][0], preview[label][0], delta=2)
                        self.assertAlmostEqual(placed[label][1], preview[label][1], delta=2)
                    for text in ("High", "Long", "End"):
                        self.assertEqual(reopened[0].get_text().count(text), 1)
        with doc_ex.open(fixture('equalrows="false" equalcolumns="false"')) as book:
            automatic = positions(book)
            self.assertNotAlmostEqual(automatic["lower"][1], 2 * automatic["middle"][1], delta=10)
            self.assertNotAlmostEqual(automatic["third"][0], 2 * automatic["second"][0], delta=10)

        with doc_ex.open(fixture('columnwidth="75pt auto 46pt"')) as book:
            fixed = positions(book)
            self.assertAlmostEqual(fixed["second"][0], 85 * 96 / 72, delta=1)
            with doc_ex.open(book.convert("docx")) as reopened:
                placed = positions(reopened)
                self.assertAlmostEqual(placed["second"][0], fixed["second"][0], delta=1)
                self.assertAlmostEqual(placed["end"][0], fixed["end"][0], delta=1)
        with doc_ex.open(fixture('columnwidth="75pt auto 66pt"')) as wider:
            self.assertAlmostEqual(positions(wider)["end"][0] - fixed["end"][0], 20 * 96 / 72, delta=1)

        def spanning_grid(attributes: str, label: str) -> bytes:
            spanning = table.format(attributes, f'<m:mtd columnspan="2"><m:mtext>{label}</m:mtext></m:mtd>')
            with doc_ex.open(epub_fixture(chapter_one=template.format(spanning))) as book:
                return book.convert("docx")

        word = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"

        def grid(document: ET.Element) -> list[int]:
            return [int(col.attrib[f"{{{word}}}w"]) for col in document.findall(".//w:tblGrid/w:gridCol", {"w": word})]

        converted = spanning_grid(equal, "Span")
        with zipfile.ZipFile(io.BytesIO(converted)) as archive:
            document = ET.fromstring(archive.read("word/document.xml"))
        cols = grid(document)
        rows = [int(row.attrib[f"{{{word}}}val"]) for row in document.findall(".//w:trHeight", {"w": word})]
        self.assertEqual(len(cols), 3)
        self.assertEqual(len(rows), 3)
        self.assertEqual(cols, [cols[1] - 100, cols[1], cols[1] - 100])
        self.assertEqual(rows, [rows[1] - 60, rows[1], rows[1] - 60])
        with doc_ex.open(converted) as reopened:
            self.assertEqual(reopened[0].get_text().count("Span"), 1)

        widths = 'columnwidth="75pt auto 46pt"'
        grids = []
        for label in ("Span", "Much Wider Spanning Expression"):
            converted = spanning_grid(widths, label)
            with zipfile.ZipFile(io.BytesIO(converted)) as archive:
                grids.append(grid(ET.fromstring(archive.read("word/document.xml"))))
            with doc_ex.open(converted) as reopened:
                self.assertEqual(reopened[0].get_text().count(label), 1)
        self.assertEqual([grids[0][0], grids[0][2]], [1600, 1020])
        self.assertEqual([grids[1][0], grids[1][2]], [1600, 1020])
        self.assertGreater(grids[1][1], grids[0][1])

        for attributes, message in (
            ('columnwidth="1pt auto 46pt"', "exceeds fixed column widths"),
            (equal + ' columnwidth="75pt 75pt 46pt"', "conflicting fixed widths"),
            ('columnwidth="30% fit"', "fitted or percentage column widths"),
        ):
            with self.subTest(attributes=attributes), doc_ex.open(fixture(attributes)) as book:
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, message):
                    book.convert("docx")

    def test_mathml_definite_width_allocates_percentage_and_fitted_columns(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Fitted MathML columns</title><style>body {{margin:0}}</style></head>
          <body><p><m:math mathsize="18pt">{}</m:math></p></body></html>"""
        colors = ("bf222d", "ac661b", "198144", "1644df", "8343cd", "5920ac")
        markers = [f'<m:mtext mathcolor="#{color}">H</m:mtext>' for color in colors]
        table = """<m:mtable {attributes} columnalign="left" columnspacing="10pt">
          <m:mtr>{ruler}</m:mtr>
          <m:mtr><m:mtd><m:mtext>Red</m:mtext></m:mtd><m:mtd><m:mtext>Fixed</m:mtext></m:mtd>
            <m:mtd><m:mtext>Automatic</m:mtext></m:mtd><m:mtd><m:mtext>Fitted</m:mtext></m:mtd>
            <m:mtd><m:mtext>Other</m:mtext></m:mtd></m:mtr>
          </m:mtable>"""

        def fixture(attributes: str) -> bytes:
            ruler = "".join(f"<m:mtd>{marker}</m:mtd>" for marker in markers[:-1])
            body = table.format(attributes=attributes, ruler=ruler) + markers[-1]
            return epub_fixture(chapter_one=template.format(body))

        def advances(doc: doc_ex.Document) -> list[int]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            starts = []
            for color in colors:
                ink = bytes.fromhex(color + "ff")
                points = [i // 4 % width for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                self.assertTrue(points, color)
                starts.append(min(points))
            return [start - starts[0] for start in starts]

        columns = 'columnwidth="20% 50pt auto fit fit"'
        with doc_ex.open(fixture('width="360pt" ' + columns)) as book:
            preview = advances(book)
            self.assertAlmostEqual(preview[1], 82 * 96 / 72, delta=1)
            self.assertAlmostEqual(preview[2] - preview[1], 60 * 96 / 72, delta=1)
            self.assertAlmostEqual(preview[5] - preview[4], preview[4] - preview[3] - 10 * 96 / 72, delta=2)
            self.assertAlmostEqual(preview[5], 360 * 96 / 72, delta=2)
            for target in ("pdf", "docx"):
                with self.subTest(target=target), doc_ex.open(book.convert(target)) as reopened:
                    for measured, expected in zip(advances(reopened), preview):
                        self.assertAlmostEqual(measured, expected, delta=2)
                    text = reopened[0].get_text()
                    for label in ("Red", "Fixed", "Automatic", "Fitted", "Other"):
                        self.assertEqual(text.count(label), 1, text)
        with doc_ex.open(fixture('width="400pt" ' + columns)) as book:
            wider = advances(book)
            self.assertAlmostEqual(wider[5] - preview[5], 40 * 96 / 72, delta=1)
            self.assertAlmostEqual(wider[1] - preview[1], 8 * 96 / 72, delta=1)
            self.assertAlmostEqual(wider[3] - wider[2], preview[3] - preview[2], delta=1)
        with doc_ex.open(fixture('width="360pt"')) as book:
            self.assertAlmostEqual(advances(book)[5], 360 * 96 / 72, delta=2)

        merged = """<m:mtable width="260pt" columnwidth="60pt 30% fit fit" columnspacing="12pt">
          <m:mtr><m:mtd columnspan="2"><m:mtext>Merged</m:mtext></m:mtd>
            <m:mtd><m:mi>a</m:mi></m:mtd><m:mtd><m:mi>b</m:mi></m:mtd></m:mtr>
          <m:mtr><m:mtd><m:mi>p</m:mi></m:mtd><m:mtd><m:mi>q</m:mi></m:mtd>
            <m:mtd><m:mi>r</m:mi></m:mtd><m:mtd><m:mi>s</m:mi></m:mtd></m:mtr>
          </m:mtable>"""
        word = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
        for markup, expected in (
            (merged, [1320, 1800, 1100, 980]),
            (merged.replace('width="260pt"', 'width="260pt" frame="solid" framespacing="8pt 6pt"'),
             [1488, 1800, 925, 973]),
            (merged.replace('columnwidth="60pt 30% fit fit"', 'columnwidth="fit" equalcolumns="true"'),
             [1240, 1360, 1360, 1240]),
        ):
            with doc_ex.open(epub_fixture(chapter_one=template.format(markup))) as book:
                converted = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(converted)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            grid = xml.findall(".//w:tblGrid/w:gridCol", {"w": word})
            self.assertEqual([int(col.attrib[f"{{{word}}}w"]) for col in grid], expected)
            with doc_ex.open(converted) as reopened:
                self.assertEqual(reopened[0].get_text().count("Merged"), 1)

        for attributes, diagnostic in (
            (columns, "require a definite table width"),
            ('width="50vw"', "requires a definite length"),
            ('width="4pt" ' + columns, "smaller than its gaps"),
            ('width="240pt" columnwidth="20% 50pt 135pt fit fit"', "exceed the authored width"),
            ('width="280pt" columnwidth="20% 50pt 130pt fit fit"', "exceeds fitted column widths"),
        ):
            with self.subTest(attributes=attributes), doc_ex.open(fixture(attributes)) as book:
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, diagnostic):
                    book.render_page(0)

    def test_mathml_relative_table_width_uses_the_containing_line(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Relative MathML table</title><style>
            body {{margin:0}} div {{width:320px; box-sizing:border-box; padding:10px 20px; border:5px solid black}}
          </style></head><body>{}</body></html>"""
        grid = """<m:mtable width="{}" columnwidth="50% fit" columnalign="left" columnspacing="10pt">
          <m:mtr><m:mtd><m:mtext mathcolor="#bf222d">H</m:mtext></m:mtd>
            <m:mtd><m:mtext mathcolor="#1644df">H</m:mtext></m:mtd></m:mtr>
          </m:mtable>"""

        def expression(width: str) -> str:
            return ('<m:math mathsize="18pt"><m:mrow>' + grid.format(width)
                    + '<m:mtext mathcolor="#198144">H</m:mtext></m:mrow></m:math>')

        def positions(doc: doc_ex.Document) -> tuple[int, int]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            starts = []
            for color in ("bf222d", "1644df", "198144"):
                ink = bytes.fromhex(color + "ff")
                points = [i // 4 % width for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                self.assertTrue(points, color)
                starts.append(min(points))
            return starts[1] - starts[0], starts[2] - starts[0]

        chapter = template.format("<div><p>" + expression("50%") + "</p></div>")
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            first, total = positions(book)
            # The CSS border box leaves 270 pixels of content. The matrix uses half.
            self.assertAlmostEqual(first, 270 / 4 + 10 * 96 / 72, delta=1)
            self.assertAlmostEqual(total, 135, delta=1)
            for target in ("pdf", "docx"):
                with self.subTest(target=target), doc_ex.open(book.convert(target)) as reopened:
                    first, total = positions(reopened)
                    # Reflow exports use the 468-point page text area.
                    self.assertAlmostEqual(first, (468 / 4 + 10) * 96 / 72, delta=2)
                    self.assertAlmostEqual(total, 468 / 2 * 96 / 72, delta=2)
        equivalent = template.format("<div><p>" + expression("0.5") + "</p></div>")
        with doc_ex.open(epub_fixture(chapter_one=equivalent)) as book:
            self.assertAlmostEqual(positions(book)[1], 135, delta=1)
        direct = template.format("<div>" + expression("50%") + "</div>")
        with doc_ex.open(epub_fixture(chapter_one=direct)) as book:
            self.assertAlmostEqual(positions(book)[1], 135, delta=1)

        viewport = '<meta name="viewport" content="width=400,height=260"/>'
        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 400, 260)])
        chapter = chapter.replace("</head>", viewport + "</head>")
        with doc_ex.open(rewrite_epub(fixed, {"OEBPS/s0.xhtml": chapter})) as book:
            preview = positions(book)
            self.assertAlmostEqual(preview[1], 135, delta=1)
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, searchable=True)) as reopened:
                self.assertEqual(positions(reopened), preview)

        cell = "<table><tr><td><p>" + expression("50%") + "</p></td><td>other</td></tr></table>"
        styled_cell = cell.replace("<table>", '<table style="width:300px">').replace(
            "<td>", '<td style="padding:0px 10px">'
        )
        with doc_ex.open(epub_fixture(chapter_one=template.format(styled_cell))) as book:
            first, total = positions(book)
            self.assertAlmostEqual(first, 130 / 4 + 10 * 96 / 72, delta=1)
            self.assertAlmostEqual(total, 65, delta=1)
        with doc_ex.open(epub_fixture(chapter_one=template.format(cell))) as book:
            for target in ("pdf", "docx"):
                with self.subTest(target=target), doc_ex.open(book.convert(target)) as reopened:
                    first, total = positions(reopened)
                    self.assertAlmostEqual(first, ((234 - 10.8) / 4 + 10) * 96 / 72, delta=2)
                    self.assertAlmostEqual(total, (234 - 10.8) / 2 * 96 / 72, delta=2)

        nested = '<m:math><m:mfrac>' + grid.format("50%") + '<m:mi>x</m:mi></m:mfrac></m:math>'
        with doc_ex.open(epub_fixture(chapter_one=template.format(nested))) as book:
            for operation in (lambda: book.render_page(0), lambda: book.convert("docx"), lambda: book.convert("pdf")):
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "definite containing line width"):
                    operation()
        collapsed = direct.replace("width:320px", "width:0px")
        with doc_ex.open(epub_fixture(chapter_one=collapsed)) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "definite containing text box"):
                book.render_page(0)
        beyond = direct.replace('width="50%"', 'width="125%"', 1)
        with doc_ex.open(epub_fixture(chapter_one=beyond)) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "outside the containing line"):
                book.render_page(0)

    def test_mathml_nested_relative_widths_use_definite_cell_tracks(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Nested relative MathML</title><style>body {{margin:0}}</style></head>
          <body><p><m:math mathsize="18pt">{}</m:math></p></body></html>"""
        colors = ("bf222d", "1644df", "198144", "5920ac", "b85124", "006a71")
        marker = [f'<m:mtext mathcolor="#{color}">H</m:mtext>' for color in colors]

        def inner(width: str, offset: int) -> str:
            return f"""<m:mtable width="{width}" columnwidth="50% fit" columnspacing="4pt" columnalign="left">
              <m:mtr><m:mtd>{marker[offset]}</m:mtd><m:mtd>{marker[offset + 1]}</m:mtd></m:mtr>
              </m:mtable>{marker[offset + 2]}"""

        span = inner("50%", 0)
        fitted = inner("0.5", 3)
        outer = """<m:mtable width="260pt" columnwidth="100pt fit 50pt" columnspacing="10pt" columnalign="left">
          <m:mtr><m:mtd columnspan="2"><m:mstyle><m:mrow>{}</m:mrow></m:mstyle></m:mtd>
            <m:mtd><m:mtext>Side</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd><m:mtext>Fixed</m:mtext></m:mtd><m:mtd>{}</m:mtd>
            <m:mtd><m:mtext>End</m:mtext></m:mtd></m:mtr></m:mtable>""".format(span, fitted)

        def positions(doc: doc_ex.Document) -> tuple[int, int, int, int]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            starts = []
            for color in colors:
                ink = bytes.fromhex(color + "ff")
                points = [i // 4 % width for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                self.assertTrue(points, color)
                starts.append(min(points))
            return (starts[1] - starts[0], starts[2] - starts[0],
                    starts[4] - starts[3], starts[5] - starts[3])

        def check_geometry(doc: doc_ex.Document) -> None:
            # The span contains 100 pt + 10 pt gap + 90 pt of fitted track.
            # The second nested matrix uses half of the 90-point track alone.
            expected = (54.0, 100.0, 26.5, 45.0)
            for actual, points in zip(positions(doc), expected):
                self.assertAlmostEqual(actual, points * 96 / 72, delta=2)

        chapter = template.format(outer)
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            check_geometry(book)
            for target in ("pdf", "docx"):
                with self.subTest(target=target), doc_ex.open(book.convert(target)) as reopened:
                    check_geometry(reopened)
                    for label in ("Side", "Fixed", "End"):
                        self.assertEqual(reopened[0].get_text().count(label), 1)
            word = book.convert("docx")
        with zipfile.ZipFile(io.BytesIO(word)) as archive:
            xml = archive.read("word/document.xml")
        self.assertIn(b"<w:gridSpan", xml)
        self.assertEqual(xml.count(b"<m:m>"), 2)
        automatic = outer.replace('columnwidth="100pt fit 50pt"', 'columnwidth="100pt auto 50pt"')
        with doc_ex.open(epub_fixture(chapter_one=template.format(automatic))) as book:
            check_geometry(book)
            with doc_ex.open(book.convert("docx")) as reopened:
                check_geometry(reopened)

        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 440, 320)])
        viewport = '<meta name="viewport" content="width=440,height=320"/>'
        chapter = chapter.replace("</head>", viewport + "</head>")
        shrink_wrapped = chapter.replace("<body>", '<body><div style="position:absolute;left:5px;top:5px">')
        shrink_wrapped = shrink_wrapped.replace("</body>", "</div></body>")
        for source in (chapter, shrink_wrapped):
            with doc_ex.open(rewrite_epub(fixed, {"OEBPS/s0.xhtml": source})) as book:
                check_geometry(book)
                with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, searchable=True)) as reopened:
                    self.assertEqual(positions(reopened), positions(book))

        intrinsic = outer.replace('width="260pt" columnwidth="100pt fit 50pt"',
                                  'columnwidth="100pt auto 50pt"')
        unknown_operand = outer.replace(span, "<m:mfrac><m:mrow>" + span + "</m:mrow><m:mi>x</m:mi></m:mfrac>")
        for source in (intrinsic, unknown_operand):
            with doc_ex.open(epub_fixture(chapter_one=template.format(source))) as book:
                for operation in (lambda: book.render_page(0), lambda: book.convert("pdf"), lambda: book.convert("docx")):
                    with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "definite containing line width"):
                        operation()

    def test_mathml_table_aligns_editable_numbers_at_decimal_points(self) -> None:
        source = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Aligned MathML decimals</title><meta name="viewport" content="width=440,height=340"/>
          <style>body {margin:0}</style></head><body><p>
            <m:math mathsize="16pt">
              <m:mtable groupalign="{decimalpoint} {right}" columnalign="center"
                  columnwidth="140pt 100pt" rowspacing="8pt">
                <m:mtr><m:mtd><m:mtext>Amount</m:mtext></m:mtd><m:mtd><m:mtext>Label</m:mtext></m:mtd></m:mtr>
                <m:mtr>
                  <m:mtd><m:maligngroup/><m:mn mathcolor="#bf222d">12.345</m:mn></m:mtd>
                  <m:mtd><m:maligngroup/><m:mtext>Short</m:mtext></m:mtd>
                </m:mtr>
                <m:mtr groupalign="{decimalpoint} {left}">
                  <m:mtd><m:mstyle decimalpoint=","><m:mrow>
                    <m:maligngroup/><m:mn mathcolor="#198144">7,89</m:mn>
                  </m:mrow></m:mstyle></m:mtd>
                  <m:mtd groupalign="right"><m:maligngroup/><m:mtext>Much Longer</m:mtext></m:mtd>
                </m:mtr>
                <m:mtr>
                  <m:mtd><m:maligngroup/><m:mn mathcolor="#1644df">401</m:mn></m:mtd>
                  <m:mtd groupalign="left"><m:maligngroup groupalign="right"/><m:mtext>Final</m:mtext></m:mtd>
                </m:mtr>
              </m:mtable>
            </m:math>
          </p></body></html>"""
        ns = {"m": "http://schemas.openxmlformats.org/officeDocument/2006/math"}
        labels = ("Amount", "Label", "12.345", "7,89", "401", "Short", "Much Longer", "Final")

        def boxes(doc: doc_ex.Document, terms: tuple[str, ...]) -> dict[str, tuple[float, float, float, float]]:
            page = doc[0]
            rects = {}
            for text in terms:
                rects[text], = page.search_for(text)
            return rects

        def geometry(doc: doc_ex.Document, tolerance: float) -> dict[str, tuple[float, float, float, float]]:
            rects = boxes(doc, (*labels, "12", "7"))
            for numeric in ("12", "7"):
                self.assertAlmostEqual(rects[numeric][2], rects["401"][2], delta=tolerance)
            self.assertGreater(rects["12.345"][2], rects["401"][2] + 20)
            self.assertGreater(rects["7,89"][2], rects["401"][2] + 10)
            self.assertAlmostEqual(rects["Short"][2], rects["Much Longer"][2], delta=tolerance)
            self.assertAlmostEqual(rects["Final"][2], rects["Much Longer"][2], delta=tolerance)
            self.assertGreater(rects["Label"][0], rects["Amount"][0] + 100)
            return rects

        with doc_ex.open(epub_fixture(chapter_one=source)) as book:
            with doc_ex.open(book.convert("pdf")) as reopened:
                self.assertFalse(reopened[0].get_images())
                reference = geometry(reopened, 0.05)
            exported = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(exported)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            matrix = xml.find(".//m:oMath//m:m", ns)
            self.assertIsNotNone(matrix)
            tokens = [run.text for run in matrix.findall(".//m:t", ns)]
            for text in labels:
                self.assertEqual(tokens.count(text), 1, tokens)
            with doc_ex.open(exported) as reopened:
                text = reopened[0].get_text()
                for label in labels:
                    self.assertEqual(text.count(label), 1, text)
                self.assertNotIn("⟡H", text)
                with doc_ex.open(reopened.convert("pdf")) as pdf:
                    native = geometry(pdf, 0.25)
                    for label in labels:
                        for expected, actual in zip(reference[label], native[label]):
                            self.assertAlmostEqual(actual, expected, delta=0.35)

        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 440, 340)])
        with doc_ex.open(rewrite_epub(fixed, {"OEBPS/s0.xhtml": source})) as book:
            original = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, searchable=True)) as reopened:
                self.assertEqual(rgba_pixels(png_chunks(reopened.render_page(0, dpi=96))), original)
                fixed_boxes = boxes(reopened, labels)
                for label in labels:
                    for coordinate in (0, 2):
                        self.assertAlmostEqual(
                            fixed_boxes[label][coordinate] - fixed_boxes["Amount"][0],
                            reference[label][coordinate] - reference["Amount"][0],
                            delta=0.65,
                        )

        for alignment in ("left", "center"):
            variant = source.replace("{right}", "{" + alignment + "}")
            variant = variant.replace('groupalign="right"', f'groupalign="{alignment}"')
            with self.subTest(alignment=alignment), doc_ex.open(epub_fixture(chapter_one=variant)) as book:
                with doc_ex.open(book.convert("pdf")) as pdf:
                    rects = boxes(pdf, ("Short", "Much Longer", "Final"))
                    starts = [rect[0] if alignment == "left" else (rect[0] + rect[2]) / 2
                              for rect in rects.values()]
                    self.assertLess(max(starts) - min(starts), 0.05)

        for chapter, message in (
            (source.replace('<m:mtable groupalign=', '<m:mtable alignmentscope="false" groupalign='),
             "MathML alignment groups across table scopes"),
            (source.replace('<m:maligngroup/><m:mn', '<m:maligngroup/><m:maligngroup/><m:mn', 1),
             "MathML decimal alignment requires a single numeric token"),
            (source.replace('<m:maligngroup/><m:mn', '<m:maligngroup/><m:mo>+</m:mo><m:mn', 1),
             "MathML decimal alignment requires a single numeric token"),
            (source.replace('columnwidth="140pt 100pt"', 'columnwidth="60pt 100pt"'),
             "MathML cell exceeds fixed column widths"),
        ):
            with self.subTest(message=message), doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
                for operation in (lambda: book.render_page(0), lambda: book.convert("pdf"), lambda: book.convert("docx")):
                    with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, message):
                        operation()

    def test_mathml_labeled_rows_keep_editable_labels_outside_data_columns(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Labeled MathML</title><meta name="viewport" content="width=440,height=320"/>
            <style>body {{margin:0}}</style></head>
          <body><p style="text-align:center">
            <m:math mathsize="16pt"><m:mtable {attrs} columnwidth="70pt 65pt"
                columnalign="left right" rowspacing="15pt">
              <m:mlabeledtr>
                <m:mtd mathcolor="#bf222d" columnalign="left"><m:mtext>NoOne</m:mtext></m:mtd>
                <m:mtd rowspan="2" mathcolor="#198144"><m:mtext>North</m:mtext></m:mtd>
                <m:mtd mathcolor="#1644df"><m:mtext>East</m:mtext></m:mtd>
              </m:mlabeledtr>
              <m:mtr><m:mtd mathcolor="#8343cd"><m:mtext>Center</m:mtext></m:mtd></m:mtr>
              <m:mlabeledtr>
                <m:mtd mathcolor="#ac661b" columnalign="right"><m:mtext>LongLabel</m:mtext></m:mtd>
                <m:mtd columnspan="2" mathcolor="#006a71"><m:mtext>South</m:mtext></m:mtd>
              </m:mlabeledtr>
            </m:mtable></m:math>
          </p></body></html>"""
        colors = {
            "NoOne": "bf222d", "North": "198144", "East": "1644df",
            "Center": "8343cd", "LongLabel": "ac661b", "South": "006a71",
        }
        data = ("North", "East", "Center", "South")
        ns = {"m": "http://schemas.openxmlformats.org/officeDocument/2006/math"}
        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 440, 320)])

        def ink_points(rgba: bytes, width: int, color: str) -> list[tuple[int, int]]:
            ink = bytes.fromhex(color + "ff")
            position, points = 0, []
            while (position := rgba.find(ink, position)) >= 0:
                if position % 4 == 0:
                    points.append(divmod(position // 4, width))
                position += 1
            return points

        def pixels(doc: doc_ex.Document, dpi: int = 96) -> dict[str, tuple[int, int]]:
            width, rgba = rgba_pixels(png_chunks(doc.render_page(0, dpi=dpi)))
            positions = {}
            for label, color in colors.items():
                points = ink_points(rgba, width, color)
                self.assertTrue(points, label)
                positions[label] = (min(x for _, x in points), min(y for y, _ in points))
            return positions

        def boxes(doc: doc_ex.Document) -> dict[str, tuple[float, float, float, float]]:
            result = {}
            for label in colors:
                result[label], = doc[0].search_for(label)
            return result

        preview_positions, pdf_boxes = {}, {}
        for side in ("right", "left"):
            source = template.format(attrs=f'side="{side}" minlabelspacing="18pt"')
            with doc_ex.open(epub_fixture(chapter_one=source)) as book:
                preview_positions[side] = preview = pixels(book)
                self.assertLess(book[0].get_text().index("NoOne"), book[0].get_text().index("North"))
                with doc_ex.open(book.convert("pdf")) as reopened:
                    self.assertFalse(reopened[0].get_images())
                    pdf_boxes[side] = vector = boxes(reopened)
                    self.assertAlmostEqual(vector["NoOne"][1], vector["North"][1], delta=0.1)
                    self.assertAlmostEqual(vector["LongLabel"][1], vector["South"][1], delta=0.1)
                exported = book.convert("docx")
                with zipfile.ZipFile(io.BytesIO(exported)) as archive:
                    xml = ET.fromstring(archive.read("word/document.xml"))
                self.assertIsNotNone(xml.find(".//m:oMath//m:m", ns))
                native_tokens = [node.text for node in xml.findall(".//m:t", ns)]
                with doc_ex.open(exported) as reopened:
                    text = reopened[0].get_text()
                    self.assertNotIn("⟡H", text)
                    with doc_ex.open(reopened.convert("pdf")) as reopened_pdf:
                        word_vector = boxes(reopened_pdf)
                    rendered = pixels(reopened)
                    for label in colors:
                        self.assertEqual(native_tokens.count(label), 1)
                        self.assertEqual(text.count(label), 1)
                        for axis in (0, 1):
                            self.assertAlmostEqual(
                                preview[label][axis] - preview["North"][axis],
                                rendered[label][axis] - rendered["North"][axis],
                                delta=2,
                            )
                        for epub_coordinate, word_coordinate in zip(vector[label], word_vector[label]):
                            self.assertAlmostEqual(epub_coordinate, word_coordinate, delta=0.35)
            with doc_ex.open(rewrite_epub(fixed, {"OEBPS/s0.xhtml": source})) as book:
                with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, searchable=True)) as reopened:
                    self.assertEqual(pixels(reopened), pixels(book))
                    fixed_vector = boxes(reopened)
                    for label in colors:
                        self.assertAlmostEqual(
                            fixed_vector[label][0] - fixed_vector["North"][0],
                            vector[label][0] - vector["North"][0],
                            delta=0.6,
                        )
        for label in data:
            self.assertEqual(preview_positions["left"][label], preview_positions["right"][label])
        right, left = pdf_boxes["right"], pdf_boxes["left"]
        self.assertGreater(right["NoOne"][0] - right["East"][2], 17)
        self.assertGreater(left["North"][0] - left["NoOne"][2], 17)
        self.assertAlmostEqual(left["North"][0] - left["LongLabel"][2], 18, delta=1)
        self.assertAlmostEqual(right["NoOne"][0] - right["East"][2], 18, delta=1)
        self.assertGreater(right["LongLabel"][2], right["NoOne"][2])
        self.assertGreater(right["Center"][1], right["NoOne"][1])
        self.assertLess(right["Center"][1], right["LongLabel"][1])

        for attrs, factor in (("", 1.0), ('minlabelspacing="150%"', 1.5), ('minlabelspacing="2"', 2.0)):
            with self.subTest(attrs=attrs), doc_ex.open(epub_fixture(chapter_one=template.format(attrs=attrs))) as book:
                with doc_ex.open(book.convert("pdf")) as reopened:
                    actual = boxes(reopened)
                    self.assertAlmostEqual(actual["NoOne"][0] - actual["East"][2], factor * 0.8 * 16, delta=1)

        # Use the native paragraph's math size to compare fraction geometry.
        fractional = template.format(attrs="").replace('mathsize="16pt"', 'mathsize="12pt"').replace(
            "<m:mtext>NoOne</m:mtext>",
            "<m:mfrac><m:mtext>NoOne</m:mtext><m:mn>2</m:mn></m:mfrac>",
        )
        with doc_ex.open(epub_fixture(chapter_one=fractional)) as book:
            exported = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(exported)) as archive:
                xml = ET.fromstring(archive.read("word/document.xml"))
            self.assertIsNotNone(xml.find(".//m:m//m:f/m:num//m:t", ns))
            with doc_ex.open(exported) as reopened, doc_ex.open(book.convert("pdf")) as pdf:
                with doc_ex.open(reopened.convert("pdf")) as native_pdf:
                    expected, actual = boxes(pdf)["NoOne"], boxes(native_pdf)["NoOne"]
                    for expected_coordinate, actual_coordinate in zip(expected, actual):
                        self.assertAlmostEqual(expected_coordinate, actual_coordinate, delta=0.35)

        ruled = template.format(attrs='frame="solid" mathcolor="#a01b70" rowlines="dashed"')
        with doc_ex.open(epub_fixture(chapter_one=ruled)) as book:
            with doc_ex.open(book.convert("pdf")) as reopened:
                for doc in (book, reopened):
                    width, rgba = rgba_pixels(png_chunks(doc.render_page(0, dpi=192)))
                    rule_x = [x for _, x in ink_points(rgba, width, "a01b70")]
                    self.assertTrue(rule_x)
                    positions = pixels(doc, dpi=192)
                    self.assertLess(min(rule_x), positions["North"][0])
                    self.assertGreater(max(rule_x), positions["East"][0])
                    self.assertLess(max(rule_x), positions["NoOne"][0])
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "MathML table rules inside equations"):
                book.convert("docx")

        for attrs, message in (
            ('side="rightoverlap"', "overlapping MathML table row labels"),
            ('width="250pt"', "MathML table row labels with a definite table width"),
            ('minlabelspacing="-1pt"', "MathML table label spacing"),
        ):
            with self.subTest(attrs=attrs), doc_ex.open(epub_fixture(chapter_one=template.format(attrs=attrs))) as book:
                for operation in (lambda: book.render_page(0), lambda: book.convert("pdf"), lambda: book.convert("docx")):
                    with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, message):
                        operation()

    def test_mathml_table_rules_and_frame_preserve_spans(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Ruled MathML</title><style>body {{margin:0}}</style></head>
          <body><p>{}</p></body></html>"""
        table = """<m:math mathsize="18pt"><m:mtable mathcolor="#a01b70" rowalign="top" columnalign="left"
            rowlines="solid dashed" columnlines="solid dashed" frame="solid" framespacing="9pt 7pt"
            columnwidth="65pt 52pt 64pt" columnspacing="13pt" rowspacing="14pt">
          <m:mtr><m:mtd rowspan="2" mathcolor="#252525"><m:mtext>Across</m:mtext></m:mtd>
            <m:mtd mathcolor="#bf222d"><m:mtext>Upper</m:mtext></m:mtd>
            <m:mtd mathcolor="#252525"><m:mtext>Right</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd columnspan="2" mathcolor="#198144"><m:mtext>Middle</m:mtext></m:mtd></m:mtr>
          <m:mtr><m:mtd mathcolor="#252525"><m:mtext>Below</m:mtext></m:mtd>
            <m:mtd mathcolor="#1644df"><m:mtext>Lower</m:mtext></m:mtd>
            <m:mtd mathcolor="#252525"><m:mtext>Last</m:mtext></m:mtd></m:mtr>
          </m:mtable></m:math>"""

        def fixture(body: str) -> bytes:
            return epub_fixture(chapter_one=template.format(body))

        def rules_in(doc: doc_ex.Document, padding: float = 9.0) -> tuple[int, int]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=192)))

            def colored(ink: str) -> set[tuple[int, int]]:
                color = bytes.fromhex(ink + "ff")
                positions = set()
                start = 0
                while (index := pixels.find(color, start)) != -1:
                    if index % 4 == 0:
                        positions.add(divmod(index // 4, width))
                    start = index + 1
                self.assertTrue(positions, ink)
                return positions

            rules = colored("a01b70")
            left = min(x for _, x in rules)
            right = max(x for _, x in rules)
            top = min(y for y, _ in rules)
            bottom = max(y for y, _ in rules)
            upper, middle, lower = [min(y for y, _ in colored(ink))
                                    for ink in ("bf222d", "198144", "1644df")]
            # The first horizontal rule stops at the left edge of the second column.
            first_cut = round((padding + 0.75 + 65 + 13.75 / 2) * 192 / 72)
            cut = [y for y in range(upper + 12, middle) if (y, left + first_cut + 24) in rules]
            self.assertTrue(cut)
            self.assertFalse(any((y, left + first_cut // 2) in rules for y in cut))
            dashed_row = max(
                ([(y, x) in rules for x in range(left + first_cut + 24, left + first_cut + 52)]
                 for y in range(middle + 12, lower)),
                key=sum,
            )
            self.assertIn(True, dashed_row)
            self.assertIn(False, dashed_row)
            # The second column boundary is dashed, except inside the spanning middle cell.
            dashed_x = right - round((padding + 0.75 + 64 + 13.75 / 2) * 192 / 72)

            def dashes(y: int) -> list[bool]:
                return [any((row, x) in rules for x in range(dashed_x - 3, dashed_x + 4))
                        for row in range(y, y + 16)]

            for y in (upper, lower):
                self.assertIn(True, dashes(y))
                self.assertIn(False, dashes(y))
            self.assertFalse(any(dashes(middle)))
            return right - left, bottom - top

        with doc_ex.open(fixture(table)) as book:
            preview = rules_in(book)
            with doc_ex.open(book.convert("pdf")) as pdf:
                placed = rules_in(pdf)
                for actual, expected in zip(placed, preview):
                    self.assertAlmostEqual(actual, expected, delta=2)
                self.assertEqual(pdf[0].get_images(), [])
            converted = book.convert("docx")
        with doc_ex.open(converted) as word:
            rules_in(word)
            for label in ("Across", "Upper", "Middle", "Lower", "Right", "Below", "Last"):
                self.assertEqual(word[0].get_text().count(label), 1)
        with zipfile.ZipFile(io.BytesIO(converted)) as archive:
            xml = archive.read("word/document.xml")
        for marker in (b"<w:tblBorders>", b"<w:tcBorders>", b'w:val="dashed"',
                       b'w:color="A01B70"', b"<w:gridSpan", b"<w:vMerge", b"<m:oMath>"):
            self.assertIn(marker, xml)
        with doc_ex.open(fixture(table.replace("9pt 7pt", "17pt 13pt"))) as padded:
            wider = rules_in(padded, padding=17.0)
            for actual, expected, addition in zip(wider, preview, (16, 12)):
                self.assertAlmostEqual(actual - expected, addition * 192 / 72, delta=2)

        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 400, 320)])
        viewport = '<meta name="viewport" content="width=400,height=320"/>'
        chapter = template.format(table).replace("</head>", viewport + "</head>")
        with doc_ex.open(rewrite_epub(fixed, {"OEBPS/s0.xhtml": chapter})) as book:
            fixed_preview = rules_in(book)
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=192, searchable=True)) as pdf:
                self.assertEqual(rules_in(pdf), fixed_preview)
                self.assertIn("Upper", pdf[0].get_text())

        bare = ('<m:math><m:mtable frame="dashed"><m:mtr><m:mtd><m:mi>x</m:mi></m:mtd>'
                '</m:mtr></m:mtable></m:math>')
        with doc_ex.open(fixture(bare)) as book, zipfile.ZipFile(io.BytesIO(book.convert("docx"))) as archive:
            xml = archive.read("word/document.xml")
            self.assertIn(b"<w:tblBorders>", xml)
            self.assertIn(b'w:val="dashed"', xml)

        for body, padding, diagnostic in (
            ("Before " + table + " after", 9.0, "table rules inside equations"),
            (table.replace("9pt 7pt", "0 0"), 0.0, "native DOCX border clearance"),
        ):
            with self.subTest(diagnostic=diagnostic), doc_ex.open(fixture(body)) as book:
                rules_in(book, padding)
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, diagnostic):
                    book.convert("docx")

        huge = ('<m:math><m:mtable columnwidth="15000pt" rowlines="dashed">' +
                '<m:mtr><m:mtd><m:mi>x</m:mi></m:mtd></m:mtr>' * 7 + '</m:mtable></m:math>')
        with doc_ex.open(fixture(huge)) as book:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "table rules exceed the paint budget"):
                book.render_page(0)

    def test_mathml_custom_fraction_rule_and_alignment_render_and_export(self) -> None:
        def fraction(thickness: str, alignment: str = "center") -> str:
            return f"""<m:math display="block" mathsize="24pt" mathcolor="#1360c8">
            <m:mfrac linethickness="{thickness}" numalign="{alignment}">
              <m:mtext mathcolor="#c82a31">H</m:mtext>
              <m:mtext mathcolor="#238449">HHHHH</m:mtext>
            </m:mfrac></m:math>"""

        def page(thickness: str, alignment: str = "center") -> str:
            return f"""<html xmlns="http://www.w3.org/1999/xhtml"
              xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Math rules</title>
              <meta name="viewport" content="width=440,height=320"/></head>
              <body><p>{fraction(thickness, alignment)}</p></body></html>"""

        def bounds(png: bytes) -> dict[str, tuple[int, int, int, int]]:
            width, pixels = rgba_pixels(png_chunks(png))
            inks = {bytes.fromhex(color + "ff"): [] for color in ("1360c8", "c82a31", "238449")}
            for i in range(0, len(pixels), 4):
                points = inks.get(pixels[i:i + 4])
                if points is not None:
                    points.append((i // 4 % width, i // 4 // width))
            return {
                ink[:3].hex(): (min(x for x, _ in points), min(y for _, y in points),
                                max(x for x, _ in points), max(y for _, y in points))
                for ink, points in inks.items() if points
            }

        def check_thick(png: bytes) -> None:
            ink = bounds(png)
            rule, num, den = (ink[key] for key in ("1360c8", "c82a31", "238449"))
            self.assertAlmostEqual(rule[3] - rule[1] + 1, 12, delta=1)
            self.assertAlmostEqual(num[0], den[0], delta=1)
            self.assertLess(num[3], rule[1])
            self.assertLess(rule[3], den[1])

        source = epub_fixture(chapter_one=page("6pt", "left"))
        with doc_ex.open(source) as book:
            check_thick(book.render_page(0, dpi=144))
            with doc_ex.open(book.convert("pdf")) as pdf:
                self.assertEqual(len(pdf[0].search_for("HHHHH")), 1)
                check_thick(pdf.render_page(0, dpi=144))
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "composed geometry"):
                book.convert("docx")

        fixed = fixed_spread_fixture([("rendition:page-spread-center", "#ffffff", 440, 320)])
        fixed = rewrite_epub(fixed, {"OEBPS/s0.xhtml": page("8px", "left")})
        with doc_ex.open(fixed) as book:
            check_thick(book.render_page(0, dpi=144))
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=144)) as pdf:
                check_thick(pdf.render_page(0, dpi=144))

        with doc_ex.open(epub_fixture(chapter_one=page("0", "right"))) as book:
            with doc_ex.open(book.convert("pdf")) as pdf:
                for render in (book.render_page(0, dpi=144), pdf.render_page(0, dpi=144)):
                    ink = bounds(render)
                    self.assertNotIn("1360c8", ink)
                    self.assertAlmostEqual(ink["c82a31"][2], ink["238449"][2], delta=1)

    def test_mathml_fraction_percentage_and_legacy_rules(self) -> None:
        template = """<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Relative rules</title></head>
          <body><p><m:math display="block" mathsize="32pt">
            <m:mfrac mathcolor="#b72159" linethickness="{}">
              <m:mtext mathcolor="#231a08">Top</m:mtext>
              <m:mtext mathcolor="#231a08">Bottom</m:mtext>
            </m:mfrac></m:math></p></body></html>"""

        def preview(thickness: str) -> bytes:
            with doc_ex.open(epub_fixture(chapter_one=template.format(thickness))) as book:
                return book.render_page(0, dpi=144)

        self.assertEqual(preview("thick"), preview("200%"))
        self.assertEqual(preview("thin"), preview("50%"))
        rose = bytes.fromhex("b72159ff")
        self.assertGreater(rgba_pixels(png_chunks(preview("thick")))[1].count(rose),
                           rgba_pixels(png_chunks(preview("thin")))[1].count(rose))
        for excessive in ("999999%", "30000pt"):
            with self.subTest(excessive=excessive), doc_ex.open(epub_fixture(chapter_one=template.format(excessive))) as book:
                with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "MathML fraction rule"):
                    book.render_page(0)

    def test_mathml_composed_pdf_keeps_text_and_positions_across_pages(self) -> None:
        def grid(top: str, left: str, right: str) -> str:
            return f"""<m:mtable rowspacing="4pt" columnspacing="8pt">
              <m:mtr><m:mtd columnspan="2" mathcolor="#a32785"><m:mtext>{top}</m:mtext></m:mtd></m:mtr>
              <m:mtr><m:mtd mathcolor="#d52b37"><m:mtext>{left}</m:mtext></m:mtd>
              <m:mtd mathcolor="#158245"><m:mtext>{right}</m:mtext></m:mtd></m:mtr></m:mtable>"""

        inline = grid("Canopy", "Aspen", "Birch")
        nested = grid("Orchard", "Cherry", "Damson")
        chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>PDF math</title></head><body>
          <p><a href="https://example.com">Before</a> <m:math mathsize="18pt">{inline}</m:math> <m:math mathsize="18pt"><m:mtext mathcolor="#1428d7">After</m:mtext></m:math></p>
          {"<p>Separate paragraphs allow the next equation to land on a later page.</p>" * 60}
          <table><tbody><tr><td><p>Cell <m:math mathsize="18pt">
            <m:mfrac><m:mpadded width="+2pt" lspace="2pt">{nested}</m:mpadded>
              <m:msqrt><m:mtext>Radicand</m:mtext></m:msqrt></m:mfrac>
            <m:mtext mathcolor="#000001">Ink</m:mtext></m:math> end</p></td></tr></tbody></table>
          </body></html>"""
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            result = book.convert("pdf")
        with doc_ex.open(result) as pdf:
            texts = [page.get_text() for page in pdf]
            for token in ("Before", "Canopy", "Aspen", "Birch", "After"):
                self.assertIn(token, texts[0])
            cell_page = next(i for i, text in enumerate(texts) if "Orchard" in text)
            self.assertGreater(cell_page, 0)
            for token in ("Orchard", "Cherry", "Damson", "Radicand", "Ink"):
                self.assertIn(token, texts[cell_page])
            for token in ("Canopy", "Aspen", "Birch", "Orchard", "Cherry", "Damson", "Radicand", "Ink", "After"):
                self.assertEqual(" ".join(texts).count(token), 1, token)
            self.assertEqual(pdf[0].get_links()[0].uri, "https://example.com")
            self.assertEqual(len(pdf[0].search_for("Canopy")), 1)
            self.assertEqual(len(pdf[cell_page].search_for("Orchard")), 1)

            for page_index in (0, cell_page):
                width, pixels = rgba_pixels(png_chunks(pdf.render_page(page_index, dpi=120)))

                def bounds(color: str) -> tuple[int, int, int, int]:
                    rgba = bytes.fromhex(color + "ff")
                    points = [(i // 4 % width, i // 4 // width)
                              for i in range(0, len(pixels), 4) if pixels[i:i + 4] == rgba]
                    self.assertTrue(points, color)
                    return min(x for x, _ in points), min(y for _, y in points), max(x for x, _ in points), max(y for _, y in points)

                top, left, right = map(bounds, ("a32785", "d52b37", "158245"))
                self.assertLess(top[3], min(left[1], right[1]))
                self.assertLess(left[2], right[0])
                following = bounds("1428d7" if page_index == 0 else "000001")
                self.assertLess(right[2], following[0], (page_index, right, following))

    def test_mathml_composed_pdf_rejects_equations_wider_than_the_text_area(self) -> None:
        chapter = """<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Wide PDF math</title></head>
          <body><p>Before <m:math><m:mpadded width="600pt"><m:mtext>Wide</m:mtext>
          </m:mpadded></m:math> after</p></body></html>"""
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            self.assertIn("Wide", book[0].get_text())
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "exceeds the PDF text area"):
                book.convert("pdf")

    def test_mathml_relative_padding_and_vertical_space(self) -> None:
        red = '<m:mi mathcolor="#d61020">H</m:mi>'
        blue = '<m:mi mathcolor="#2344cc">M</m:mi>'
        reference = '<m:mi mathcolor="#217629">H</m:mi>'

        def preview(source: str) -> bytes:
            chapter = f"""<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
            <head><style>body,p {{margin:0}}</style></head><body><p>
            <m:math mathsize="18pt" mathvariant="normal">{reference}{source}</m:math>
            </p></body></html>"""
            with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
                return book.render_page(0, dpi=96)

        def at(png: bytes, color: str) -> tuple[int, int]:
            width, pixels = rgba_pixels(png_chunks(png))
            rgba = bytes.fromhex(color + "ff")
            points = [(i // 4 % width, i // 4 // width)
                      for i in range(0, len(pixels), 4) if pixels[i:i + 4] == rgba]
            return min(x for x, _ in points), min(y for _, y in points)

        identity = '<m:mpadded width="100%">' + red + '</m:mpadded>'
        plain = preview(identity + blue)
        moved = preview('<m:mpadded width="+12pt" height="+6pt" lspace="6pt" voffset="3pt">'
                        + red + '</m:mpadded>' + blue)
        guide = at(moved, "217629")
        self.assertAlmostEqual(at(moved, "d61020")[0] - at(plain, "d61020")[0], 8, delta=1)
        self.assertAlmostEqual(at(moved, "2344cc")[0] - at(plain, "2344cc")[0], 16, delta=1)
        self.assertAlmostEqual(at(moved, "d61020")[1] - guide[1], -4, delta=1)
        self.assertEqual(
            preview('<m:mpadded width="+100%width" depth="100%height">' + red + "</m:mpadded>" + blue),
            preview('<m:mpadded width="200%" depth="1height">' + red + "</m:mpadded>" + blue),
        )
        lowered = preview('<m:mpadded width="100%" lspace="-3pt" voffset="-3pt">'
                          + red + '</m:mpadded>' + blue)
        self.assertAlmostEqual(at(lowered, "d61020")[0] - at(plain, "d61020")[0], -4, delta=1)
        self.assertAlmostEqual(at(lowered, "d61020")[1] - at(lowered, "217629")[1], 4, delta=1)
        spaced = preview(identity + '<m:mspace width="1em" height="20pt" depth="8pt"/>' + blue)
        self.assertAlmostEqual(at(spaced, "2344cc")[0] - at(plain, "2344cc")[0], 24, delta=2)
        self.assertGreater(at(spaced, "d61020")[1], at(plain, "d61020")[1] + 5)
        with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "native vertical extent"):
            preview('<m:mpadded height="5000pt">' + red + '</m:mpadded>')

    def test_mathml_horizontal_padding_stays_editable_in_docx(self) -> None:
        red = '<m:mtext mathcolor="#cf222e">Red</m:mtext>'
        blue = '<m:mtext mathcolor="#1644df">Blue</m:mtext>'
        green = '<m:mtext mathcolor="#198144">Green</m:mtext>'
        words = f'<m:mpadded width="100%">{red}</m:mpadded><m:mpadded width="100%">{blue}</m:mpadded>{green}'
        padded = (
            f'<m:mpadded width="+8pt" lspace="2pt"><m:mpadded width="+12pt" lspace="6pt">'
            f'{red}</m:mpadded></m:mpadded><m:mpadded width="72pt" lspace="4pt" height="100%" depth="0">'
            f'{blue}</m:mpadded>{green}'
        )
        template = """<html xmlns="http://www.w3.org/1999/xhtml"
          xmlns:m="http://www.w3.org/1998/Math/MathML"><head><title>Editable math padding</title>
          <style>body {{margin:0}}</style></head>
          <body><p>Before <m:math mathsize="18pt">{}</m:math> after.</p></body></html>"""

        def starts(png: bytes) -> dict[str, int]:
            width, pixels = rgba_pixels(png_chunks(png))
            result = {}
            for color in ("cf222e", "1644df", "198144"):
                ink = bytes.fromhex(color + "ff")
                result[color] = min(i // 4 % width for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink)
            return result

        with (
            doc_ex.open(epub_fixture(chapter_one=template.format(words))) as baseline,
            doc_ex.open(epub_fixture(chapter_one=template.format(padded))) as book,
        ):
            native = book.convert("docx")
            with zipfile.ZipFile(io.BytesIO(native)) as archive:
                xml = archive.read("word/document.xml")
            ns = {"m": "http://schemas.openxmlformats.org/officeDocument/2006/math"}
            root = ET.fromstring(xml)
            math, = root.findall(".//m:oMath", ns)
            self.assertEqual("".join(node.text or "" for node in math.findall(".//m:t", ns)).split(), ["Red", "Blue", "Green"])
            self.assertIsNotNone(math.find(".//m:phantPr/m:zeroWid", ns))
            self.assertIsNotNone(math.find(".//m:phantPr/m:zeroDesc", ns))
            self.assertNotIn(b"epub-math-box-", xml)
            with doc_ex.open(native) as word, doc_ex.open(baseline.convert("docx")) as original_word:
                expected = word[0].get_text()
                for token in ("Before", "Red", "Blue", "Green", "after."):
                    self.assertEqual(expected.count(token), 1, expected)
                for control, changed in ((baseline, book), (original_word, word)):
                    plain = starts(control.render_page(0, dpi=144))
                    moved = starts(changed.render_page(0, dpi=144))
                    self.assertAlmostEqual(moved["cf222e"] - plain["cf222e"], 16, delta=2)
                    self.assertAlmostEqual(moved["1644df"] - plain["1644df"], 48, delta=2)
                    self.assertAlmostEqual(moved["198144"] - moved["1644df"], 136, delta=2)

        grid = f"""<m:mtable><m:mtr><m:mtd columnspan="2">
          <m:mfrac><m:mpadded width="+12pt" lspace="6pt">{red}</m:mpadded>
          <m:msqrt>{blue}</m:msqrt></m:mfrac></m:mtd></m:mtr>
          <m:mtr><m:mtd>{green}</m:mtd><m:mtd><m:mn>2</m:mn></m:mtd></m:mtr></m:mtable>"""
        chapter = template.replace("Before ", "").replace(" after.", "").format(grid)
        with doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
            data = book.convert("docx")
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            xml = archive.read("word/document.xml")
            self.assertIn(b'<w:gridSpan w:val="2"/>', xml)
            self.assertIn(b"<m:f>", xml)
            self.assertIn(b"<m:rad>", xml)
        with doc_ex.open(data) as reopened:
            for token in ("Red", "Blue", "Green"):
                self.assertEqual(reopened[0].get_text().count(token), 1)
            self.assertEqual(set(starts(reopened.render_page(0, dpi=144))), {"cf222e", "1644df", "198144"})

        empty = template.format('<m:mpadded width="12pt"/>' + red)
        with doc_ex.open(epub_fixture(chapter_one=empty)) as book:
            data = book.convert("docx")
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            xml = archive.read("word/document.xml")
            self.assertNotIn(b"<m:phant", xml)
            self.assertIn(b'm:t xml:space="preserve"> </m:t>', xml)

        for attributes, message in (
            ('width="+8pt" lspace="-2pt"', "composed geometry"),
            ('width="+0.1pt"', "spacing resolution"),
            ('width="600pt"', "exceeds the DOCX text area"),
        ):
            chapter = template.format(f"<m:mpadded {attributes}>{red}</m:mpadded>{blue}")
            with self.subTest(attributes=attributes), doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, message):
                    book.convert("docx")

    def test_mathml_signed_vertical_and_overhanging_horizontal_shifts_stay_editable(self) -> None:
        red = '<m:mtext mathcolor="#cf222e">Red</m:mtext>'
        blue = '<m:mtext mathcolor="#1644df">Blue</m:mtext>'
        green = '<m:mtext mathcolor="#198144">Guard</m:mtext>'
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Editable content offsets</title><style>body {{ margin:0 }}</style></head>
          <body><p><m:math mathsize="18pt">{}</m:math></p>
          <p><m:math mathsize="18pt">{}</m:math></p></body></html>"""

        def positions(doc: doc_ex.Document) -> list[tuple[int, int]]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            result = []
            for color in ("cf222e", "1644df", "198144"):
                ink = bytes.fromhex(color + "ff")
                matches = [divmod(i // 4, width) for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                result.append((min(x for _, x in matches), min(y for y, _ in matches)))
            return result

        def sample(body: str) -> tuple[list[tuple[int, int]], list[tuple[int, int]], bytes]:
            with doc_ex.open(epub_fixture(chapter_one=template.format(body, green))) as book:
                preview = positions(book)
                data = book.convert("docx")
            with doc_ex.open(data) as word:
                reopened = positions(word)
                text = word[0].get_text()
                for label in ("Red", "Blue", "Guard"):
                    self.assertEqual(text.count(label), 1, text)
                self.assertNotIn("⟡H", text)
                self.assertNotIn("⟡g", text)
                self.assertNotIn("█(", text)
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                return preview, reopened, archive.read("word/document.xml")

        spacer = '<m:mspace width="8pt"/>'
        unshifted = f'<m:mpadded width="100%">{red}</m:mpadded>'
        baseline = sample(unshifted + spacer + blue)
        for offset in (6, -5, 3.2):
            with self.subTest(offset=offset):
                padded = f'<m:mpadded width="+3pt" lspace="9pt" voffset="{offset}pt">{red}</m:mpadded>'
                raised = sample(padded + spacer + blue)
                self.assertIn(b"<m:eqArr>", raised[2])
                for before, after in zip(baseline[:2], raised[:2]):
                    self.assertAlmostEqual(after[0][0] - before[0][0], 12, delta=1)
                    self.assertAlmostEqual(after[1][0] - before[1][0], 4, delta=1)
                    self.assertAlmostEqual(after[0][1] - before[0][1], -offset * 4 / 3, delta=1)
                    self.assertAlmostEqual(after[1][1], before[1][1], delta=1)
                    self.assertAlmostEqual(after[2][1], before[2][1], delta=1)

        fraction = '<m:mfrac>{}<m:msqrt>{}</m:msqrt></m:mfrac>'
        unshifted_fraction = fraction.format(unshifted, blue)
        shifted_fraction = fraction.format(f'<m:mpadded voffset="-25%height">{red}</m:mpadded>', blue)
        before = sample(unshifted_fraction)
        nested = sample(shifted_fraction)
        for original, shifted in zip(before[:2], nested[:2]):
            self.assertGreater(shifted[0][1], original[0][1])
            # Subpixel differences can change the first solid pixel by one.
            for (old_x, old_y), (new_x, new_y) in zip(original[1:], shifted[1:]):
                self.assertAlmostEqual(new_x, old_x, delta=1)
                self.assertAlmostEqual(new_y, old_y, delta=1)
        self.assertIn(b"<m:f>", nested[2])
        self.assertIn(b"<m:rad>", nested[2])

        def neighboring_boxes(body: str) -> list[dict[str, tuple[float, ...]]]:
            result = []
            with doc_ex.open(epub_fixture(chapter_one=template.format(body, green))) as book:
                with doc_ex.open(book.convert("docx")) as word:
                    for source in (book, word):
                        with doc_ex.open(source.convert("pdf")) as pdf:
                            boxes = {}
                            for label in ("Blue", "Guard"):
                                matches = pdf[0].search_for(label)
                                self.assertEqual(len(matches), 1, (label, matches))
                                boxes[label] = matches[0]
                            result.append(boxes)
            return result

        original_epub, original_docx = neighboring_boxes(unshifted_fraction)
        shifted_epub, shifted_docx = neighboring_boxes(shifted_fraction)
        self.assertEqual(shifted_epub, original_epub)
        # The half-point sizing of native DOCX placement guides can perturb
        # the final baselines slightly even though the operands are unchanged.
        for label, original_box in original_docx.items():
            for original, shifted in zip(original_box, shifted_docx[label]):
                self.assertAlmostEqual(shifted, original, delta=0.1)

        deep = '<m:mpadded voffset="3pt">' * 9 + red + '</m:mpadded>' * 9
        with doc_ex.open(epub_fixture(chapter_one=template.format(deep, green))) as book:
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "DOCX equation depth"):
                book.convert("docx")

    def test_mathml_vertical_padding_and_space_stay_editable_in_docx(self) -> None:
        red = '<m:mtext mathcolor="#cf222e">Red</m:mtext>'
        blue = '<m:mtext mathcolor="#1644df">Blue</m:mtext>'
        template = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Editable vertical spacing</title><style>body {{ margin:0 }}</style></head>
          <body><p><m:math mathsize="18pt">{}</m:math></p>
          <p><m:math mathsize="18pt"><m:mtext mathcolor="#198144">Guard</m:mtext></m:math></p></body></html>"""
        space = red + '<m:mspace width="6pt" height="30pt" depth="22pt"/>' + blue
        no_depth = red + '<m:mspace width="6pt" height="30pt"/>' + blue
        fixed = f'<m:mpadded width="+6pt" height="30pt" depth="22pt">{red}</m:mpadded>{blue}'
        additive = f'<m:mpadded width="+6pt" height="+22pt" depth="+22pt">{red}</m:mpadded>{blue}'
        relative = f'<m:mpadded width="+100%width" lspace="50%width" height="30pt">{red}</m:mpadded>{blue}'
        table = f"""<m:mtable><m:mtr><m:mtd columnspan="2"><m:mfrac>
          <m:mpadded height="26pt" depth="12pt">{red}</m:mpadded>{blue}</m:mfrac></m:mtd></m:mtr>
          <m:mtr><m:mtd><m:mi>x</m:mi></m:mtd><m:mtd><m:mi>y</m:mi></m:mtd></m:mtr></m:mtable>"""

        def positions(doc: doc_ex.Document) -> list[tuple[int, int]]:
            width, pixels = rgba_pixels(png_chunks(doc.render_page(0, dpi=96)))
            result = []
            for color in ("cf222e", "1644df", "198144"):
                ink = bytes.fromhex(color + "ff")
                matches = [divmod(i // 4, width) for i in range(0, len(pixels), 4) if pixels[i:i + 4] == ink]
                result.append((min(x for _, x in matches), min(y for y, _ in matches)))
            return result

        previews = {}
        reopened = {}
        for name, equation in (("plain", red + blue), ("space", space), ("no_depth", no_depth),
                               ("fixed", fixed), ("additive", additive), ("relative", relative), ("table", table)):
            with self.subTest(name=name):
                with doc_ex.open(epub_fixture(chapter_one=template.format(equation))) as book:
                    previews[name] = positions(book)
                    data = book.convert("docx")
                with zipfile.ZipFile(io.BytesIO(data)) as archive:
                    xml = archive.read("word/document.xml")
                with doc_ex.open(data) as word:
                    reopened[name] = positions(word)
                    text = word[0].get_text()
                    for label in ("Red", "Blue", "Guard"):
                        self.assertEqual(text.count(label), 1, text)
                    self.assertNotIn("⟡H", text)
                    self.assertNotIn("⟡g", text)
                if name != "plain":
                    self.assertIn(b'<m:show m:val="0"/>', xml)
                    self.assertIn(b'<m:zeroWid m:val="1"/>', xml)
                    self.assertIn(b'<m:zeroDesc m:val="1"/>', xml)
                if name == "table":
                    self.assertIn(b'<w:gridSpan w:val="2"/>', xml)
                    self.assertIn(b"<m:f>", xml)
                    self.assertLess(reopened[name][0][1], reopened[name][1][1])

        for layout in (previews, reopened):
            self.assertEqual(layout["space"], layout["fixed"])
            self.assertEqual(layout["space"][:2], layout["no_depth"][:2])
            self.assertGreaterEqual(layout["space"][2][1] - layout["no_depth"][2][1], 20)
            self.assertAlmostEqual(layout["space"][1][0] - layout["plain"][1][0], 8, delta=1)
            self.assertGreater(layout["additive"][0][1], layout["fixed"][0][1])
            before, after = layout["plain"], layout["relative"]
            advance = before[1][0] - before[0][0]
            self.assertAlmostEqual(after[0][0] - before[0][0], advance / 2, delta=2)
            self.assertAlmostEqual(after[1][0] - before[1][0], advance, delta=2)

        too_tall = f'<m:mpadded height="9999pt">{red}</m:mpadded>'
        with doc_ex.open(epub_fixture(chapter_one=template.format(too_tall))) as book:
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "DOCX spacing range"):
                book.convert("docx")

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

    def test_batch_chapter_anchor_validation_uses_staged_hyperlinks(self) -> None:
        html = '<html xmlns="http://www.w3.org/1999/xhtml"><body>{}</body></html>'
        chapter = html.format('<p xml:id="café">Target</p><p><a href="#caf%C3%A9">Same page</a></p>')
        other = html.format('<p><a href="./one.xhtml?mode=1#caf%C3%A9">Previous page</a></p>')
        source = rewrite_epub(epub_fixture(chapter_one=chapter), {"OEBPS/two.xhtml": other})
        renamed = html.format('<p id="nouveau">Renamed target</p>')
        retargeted = other.replace("caf%C3%A9", "nouveau")
        with doc_ex.open(source) as book:
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_chapters({0: renamed})
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_chapters({0: renamed, 1: other.replace("Previous page", "Still stale")})
            revised = book.edit_epub_chapters({0: renamed, -1: retargeted})
            self.assertEqual(book[1].get_links()[0].metadata["target_fragment"], "café")
            self.assertEqual(book.read_epub_resource("OEBPS/one.xhtml"), chapter.encode())
        with doc_ex.open(revised) as edited:
            self.assertEqual(edited[0].get_text(), "Renamed target")
            link, = edited[1].get_links()
            self.assertEqual(link.metadata["target_fragment"], "nouveau")

    def test_structure_replaces_chapter_and_retargets_shared_narration_atomically(self) -> None:
        source, audio = narrated_epub()
        path = "OEBPS/one.xhtml"
        voice = "OEBPS/Media/voice.wav"
        markup = '''<html xmlns="http://www.w3.org/1999/xhtml" xmlns:m="http://www.w3.org/1998/Math/MathML">
          <head><title>Revised chapter</title></head><body id="new-chapter"><p id="new">Revised narration</p>
          <p><m:math><m:mi>x</m:mi></m:math></p></body></html>'''
        narration = {path: [doc_ex.EpubOverlaySequence(
            [doc_ex.EpubOverlayCue("new"), doc_ex.EpubOverlayCue("new", voice, 0, 500_000)],
            fragment="new-chapter",
        )]}
        with doc_ex.open(source) as book:
            old_chapter = book.read_epub_resource(path)
            old_cues = book.epub_media_cues(1)
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_chapters({0: markup})
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_structure([1, 0], replacements={0: markup})
            with self.assertRaisesRegex(ValueError, "conflicting replacements"):
                book.edit_epub_structure(replacements={0: markup, -2: markup.replace("Revised", "Other")},
                                         overlays=narration)
            revised = book.edit_epub_structure([1, 0], replacements={-2: markup}, overlays=narration)
            sole_overlay = book.edit_epub_structure(overlays={"OEBPS/two.xhtml": None})
            self.assertEqual(book.read_epub_resource(path), old_chapter)
            self.assertEqual(book.epub_media_cues(1), old_cues)
        with doc_ex.open(revised) as book:
            self.assertEqual([page.metadata["chapter_path"] for page in book], ["OEBPS/two.xhtml", path])
            self.assertEqual(book.epub_media_cues(0), old_cues)
            self.assertEqual(book.epub_media_cues(1)[0]["text"]["fragment"], "new")
            cue, playable = book.epub_narration(1)
            self.assertEqual(cue.text, "Revised narration")
            assert playable.audio is not None
            self.assertEqual(playable.audio.frame_count, 4_000)
            self.assertEqual(book.read_epub_resource(voice), audio)
            item = next(item for item in book.metadata["manifest"] if item["path"] == path)
            self.assertEqual(item["properties"], "mathml")
        old_smil = "OEBPS/Sync/read.smil"
        with doc_ex.open(sole_overlay) as book:
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_structure(replacements={0: markup}, overlays=narration)
            revised = book.edit_epub_structure(replacements={0: markup}, overlays=narration,
                                               remove_resources=[old_smil])
        with doc_ex.open(revised) as book, zipfile.ZipFile(io.BytesIO(revised)) as archive:
            self.assertNotIn(old_smil, archive.namelist())
            self.assertEqual(next(book.epub_narration(0)).text, "Revised narration")

    def test_structure_reorders_staged_navigation_chapter_markup(self) -> None:
        def toc(label: str, anchor: str) -> str:
            return f'''<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
              <head><title>Contents</title></head><body><nav epub:type="toc"><ol>
                <li><a href="one.xhtml">Contents</a></li>
                <li><a href="two.xhtml#{anchor}">{label}</a></li></ol></nav></body></html>'''

        source = epub_fixture(chapter_one=toc("Original heading", "old"))
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b'<item id="one" ', b'<item id="one" properties="nav" ')
        body = '<html xmlns="http://www.w3.org/1999/xhtml"><body><h1 id="{}">A chapter</h1></body></html>'
        source = rewrite_epub(source, {"OEBPS/content.opf": package, "OEBPS/two.xhtml": body.format("old")})
        with doc_ex.open(source) as book:
            revised = book.edit_epub_structure([1, 0], replacements={
                0: toc("Revised heading", "new"), 1: body.format("new"),
            })
        with doc_ex.open(revised) as book:
            nav = book.metadata["navigation"][0]["items"]
            self.assertEqual([item["label"] for item in nav], ["Revised heading", "Contents"])
            self.assertEqual(nav[0]["fragment"], "new")
            self.assertEqual(book[0].get_text(), "A chapter")

    def test_structure_replaces_and_reorders_nonspine_navigation(self) -> None:
        source, nav_path, nav, html = self.nonspine_navigation_fixture()
        revised_nav = nav.replace("Old introduction", "New introduction").replace(
            "one.xhtml#old", "one.xhtml#fresh-%C3%A9",
        ).replace('id="old-nav"', 'id="new-nav"').replace(
            '<svg xmlns="http://www.w3.org/2000/svg"/>',
            '<math xmlns="http://www.w3.org/1998/Math/MathML"><mn>2</mn></math>',
        )
        ncx_path = "OEBPS/toc.ncx"
        with doc_ex.open(source) as book:
            authored_ncx = book.read_epub_resource(ncx_path).replace(
                b"NCX one", b"Staged NCX introduction",
            ).replace(b'one.xhtml"', b'one.xhtml#fresh-%C3%A9"')
            revised = book.edit_epub_structure(
                [1, doc_ex.EpubChapterSource(
                    "Inserted", html.format('<p id="extra">A new chapter</p>'), path="OEBPS/extra.xhtml",
                ), 0],
                replacements={
                    0: html.format('<h1 id="fresh-é">New chapter text</h1>'),
                    1: html.format('<h1 id="other">Second text</h1><a href="Nav/contents.xhtml#new-nav">Contents</a>'),
                },
                resources={
                    nav_path: doc_ex.EpubResource("application/xhtml+xml", revised_nav.encode()),
                    ncx_path: doc_ex.EpubResource("application/x-dtbncx+xml", authored_ncx),
                },
            )
            self.assertEqual(book.read_epub_resource(nav_path), nav.encode())
            self.assertEqual(len(book), 2)

        with doc_ex.open(revised) as book:
            self.assertEqual(len(book), 3)
            self.assertIn("New chapter text", book[2].get_text())
            groups = {group["kind"]: group["items"] for group in book.metadata["navigation"]}
            self.assertEqual([item["label"] for item in groups["toc"]], ["Second", "Inserted", "Part A"])
            self.assertEqual(groups["toc"][-1]["children"][0]["label"], "New introduction")
            self.assertEqual(groups["toc"][-1]["children"][0]["fragment"], "fresh-é")
            self.assertEqual(groups["page-list"][0]["fragment"], "other")
            self.assertEqual(groups["related"][0]["href"], "https://example.org/")
            manifest = {item["path"]: item for item in book.metadata["manifest"]}
            self.assertEqual(manifest[nav_path]["properties"], "nav mathml")
            self.assertIn(b'<nav id="new-nav"', book.read_epub_resource(nav_path))
            self.assertIn(b'<aside', book.read_epub_resource(nav_path))
            ncx = ET.fromstring(book.read_epub_resource("OEBPS/toc.ncx"))
            points = ncx.findall(".//{*}navPoint")
            self.assertEqual([point.attrib["playOrder"] for point in points], ["1", "2", "3"])
            self.assertEqual(
                [point.findtext("{*}navLabel/{*}text") for point in points],
                ["NCX two", "Inserted", "Staged NCX introduction"],
            )
            self.assertEqual(points[-1].find("{*}content").attrib["src"], "one.xhtml#fresh-%C3%A9")

    def test_structure_stages_supplemental_xhtml_links_navigation_and_assets(self) -> None:
        source, nav_path, nav, html = self.nonspine_navigation_fixture()
        notes, glossary = "OEBPS/notes.xhtml", "OEBPS/glossary.xhtml"
        endnotes, index = "OEBPS/endnotes.xhtml", "OEBPS/notes-index.xhtml"
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            opf = archive.read("OEBPS/content.opf").replace(b"</manifest>", b"""
                <item id="notes" href="notes.xhtml" media-type="application/xhtml+xml" properties="svg"/>
                <item id="notes-index" href="notes-index.xhtml" media-type="application/xhtml+xml"/>
                </manifest>""")
        original_notes = html.format('<p id="old-note">Original endnote</p>')
        source = rewrite_epub(source, {
            "OEBPS/content.opf": opf, notes: original_notes,
            index: html.format('<h1>Index</h1><a href="notes.xhtml#old-note">Note</a>'),
        })
        note_markup = html.format("""<p id="note-é">Edited endnote</p>
            <p><a href="one.xhtml#old">Back to chapter</a><a href="glossary.xhtml#definition">Definition</a></p>
            <p><img src="Visual/figure.png" alt="Figure"/>
            <math xmlns="http://www.w3.org/1998/Math/MathML"><mn>2</mn></math></p>""")
        glossary_markup = html.format("""<p id="definition"><a href="one.xhtml#old">Chapter</a></p>
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="12" viewBox="0 0 20 12">
            <rect width="20" height="12" fill="#3264a0"/></svg>""")
        xhtml = "application/xhtml+xml"
        resources = {
            notes: doc_ex.EpubResource(xhtml, note_markup.encode()),
            glossary: doc_ex.EpubResource(xhtml, glossary_markup.encode()),
            "OEBPS/Visual/figure.png": doc_ex.EpubResource("image/png", PNG),
        }
        with doc_ex.open(source) as book:
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_structure(resources=resources)
            index_markup = html.format('<h1>Index</h1><a href="notes.xhtml#note-%C3%A9">Note</a>')
            revised_nav = nav.replace("https://example.org/", "../endnotes.xhtml#endnote")
            note = doc_ex.EpubChapterSource(
                "Endnotes", html.format('<p id="endnote"><a href="one.xhtml#old">Back</a></p>'),
                endnotes, linear=False,
            )
            revised = book.edit_epub_structure(
                [1, 0, note], replacements={
                    0: html.format('<h1 id="old">Edited</h1><a href="endnotes.xhtml#endnote">Note</a>')
                }, resources={
                    **resources, index: doc_ex.EpubResource(xhtml, index_markup.encode()),
                    nav_path: doc_ex.EpubResource(xhtml, revised_nav.encode()),
                },
            )
            self.assertEqual(book.read_epub_resource(notes), original_notes.encode())
        with doc_ex.open(revised) as book:
            self.assertEqual(len(book), 3)
            self.assertEqual([page.metadata["linear"] for page in book], [True, True, False])
            self.assertIn("Edited", book[1].get_text())
            self.assertEqual(book[1].get_links()[0].metadata["target_path"], endnotes)
            self.assertEqual(book.read_epub_resource(notes), note_markup.encode())
            self.assertEqual(book.read_epub_resource(index), index_markup.encode())
            self.assertEqual(book.read_epub_resource(glossary), glossary_markup.encode())
            self.assertEqual(book.read_epub_resource("OEBPS/Visual/figure.png"), PNG)
            props = {item["path"]: item["properties"] for item in book.metadata["manifest"]}
            self.assertEqual((props[notes], props[glossary]), ("mathml", "svg"))
            related = next(item for item in book.metadata["navigation"] if item["kind"] == "related")
            self.assertEqual(related["items"][0]["fragment"], "endnote")

    def test_structure_rejects_invalid_supplemental_xhtml(self) -> None:
        xhtml = "application/xhtml+xml"
        html = '<html xmlns="http://www.w3.org/1999/xhtml"><body>{}</body></html>'
        cases = (
            ("<empty/>", "document body"),
            (html.format('<a href="absent.xhtml">Missing</a>'), "missing packaged resource"),
            (html.format('<a href="one.xhtml#absent">Missing</a>'), "missing fragment"),
            (html.format('<img src="missing.png" alt="Missing"/>'), "missing.png"),
        )
        source = epub_fixture()
        with doc_ex.open(source) as book:
            for markup, reason in cases:
                with self.subTest(reason=reason), self.assertRaisesRegex(doc_ex.MalformedDocumentError, reason):
                    book.edit_epub_structure(resources={
                        "OEBPS/notes.xhtml": doc_ex.EpubResource(xhtml, markup.encode())
                    })
            with self.assertRaisesRegex(ValueError, "hyperlinked EPUB content must be in the spine"):
                book.edit_epub_structure(
                    replacements={0: html.format('<a href="notes.xhtml">Note</a>')},
                    resources={"OEBPS/notes.xhtml": doc_ex.EpubResource(xhtml, html.format("<p>Note</p>").encode())},
                )
            with self.assertRaisesRegex(ValueError, "at least one linear chapter"):
                book.edit_epub_structure([doc_ex.EpubChapterSource("Notes", html.format("<p>Note</p>"), linear=False)])
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            opf = archive.read("OEBPS/content.opf").replace(
                b'<itemref idref="one"/>', b'<itemref idref="foreign"/>',
            ).replace(b"</manifest>", b"""
              <item id="foreign" href="unreadable.xml" media-type="application/xml" fallback="one"/>
              </manifest>""")
        with doc_ex.open(rewrite_epub(source, {"OEBPS/content.opf": opf})) as book:
            self.assertEqual(book[0].metadata["chapter_path"], "OEBPS/one.xhtml")
            with self.assertRaisesRegex(ValueError, "conflicts with chapter"):
                book.edit_epub_structure(resources={
                    "OEBPS/one.xhtml": doc_ex.EpubResource(xhtml, html.format("<p>Wrong entry point</p>").encode())
                })

    def test_structure_replaces_nonspine_navigation_verbatim_without_reorder(self) -> None:
        source, nav_path, nav, _ = self.nonspine_navigation_fixture()
        revised_nav = nav.replace("Old introduction", "Edited label").encode()
        with doc_ex.open(source) as book:
            updated = book.edit_epub_structure(
                resources={nav_path: doc_ex.EpubResource("application/xhtml+xml", revised_nav)},
            )
            ncx = book.read_epub_resource("OEBPS/toc.ncx")
        with doc_ex.open(updated) as book:
            self.assertEqual(book.read_epub_resource(nav_path), revised_nav)
            self.assertEqual(book.read_epub_resource("OEBPS/toc.ncx"), ncx)
            self.assertEqual(book.metadata["navigation"][0]["items"][0]["children"][0]["label"], "Edited label")

    def test_structure_rejects_invalid_nonspine_navigation_edits(self) -> None:
        source, nav_path, nav, _ = self.nonspine_navigation_fixture()
        with doc_ex.open(source) as book:
            for changed, message in (
                (nav.replace("#old", "#missing", 1), "missing fragment"),
                (nav.replace("one.xhtml#old", "absent.xhtml"), "not a packaged resource"),
                (nav.replace("../one.xhtml#old", "https://example.org/"), "local content documents"),
                (nav.replace("../one.xhtml#old", "../image.png"), "local content documents"),
                (nav.replace('epub:type="toc"', 'epub:type="other"'), "one nonempty table of contents"),
                (nav.replace('epub:type="page-list"', 'epub:type="toc"'), "one nonempty table of contents"),
            ):
                with self.subTest(message=message), self.assertRaisesRegex(doc_ex.MalformedDocumentError, message):
                    book.edit_epub_structure(
                        resources={nav_path: doc_ex.EpubResource("application/xhtml+xml", changed.encode())},
                    )
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                changed = nav.replace('id="old-nav"', 'id="removed"')
                book.edit_epub_structure(
                    resources={nav_path: doc_ex.EpubResource("application/xhtml+xml", changed.encode())},
                )
            with self.assertRaisesRegex(ValueError, "packaged XHTML manifest entry"):
                book.edit_epub_structure(resources={nav_path: doc_ex.EpubResource("text/html", nav.encode())})
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_structure(
                    [1], resources={nav_path: doc_ex.EpubResource("application/xhtml+xml", nav.encode())},
                )
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                authored = nav.replace('id="part-a"', 'id="new-part"')
                chapter = '<html xmlns="http://www.w3.org/1999/xhtml"><body><h1 id="other">Second text</h1><a href="Nav/contents.xhtml#new-part">Part</a></body></html>'
                book.edit_epub_structure(
                    [1], replacements={1: chapter},
                    resources={nav_path: doc_ex.EpubResource("application/xhtml+xml", authored.encode())},
                )
            self.assertEqual(book.read_epub_resource(nav_path), nav.encode())

        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            spine_nav = archive.read("OEBPS/content.opf").replace(
                b'<itemref idref="one"/>', b'<itemref idref="one"/><itemref idref="edit-nav"/>',
            )
        with doc_ex.open(rewrite_epub(source, {"OEBPS/content.opf": spine_nav})) as book:
            with self.assertRaisesRegex(ValueError, "spine navigation chapter through replacements"):
                book.edit_epub_structure(
                    resources={nav_path: doc_ex.EpubResource("application/xhtml+xml", nav.encode())},
                )

    def test_structure_replaces_epub2_ncx_with_spine_and_chapter_edits(self) -> None:
        html = '<html xmlns="http://www.w3.org/1999/xhtml"><body>{}</body></html>'
        source = epub_fixture(version="2.0", chapter_one=html.format('<h1 id="old">First</h1>'))
        ncx_path = "OEBPS/Nav/book.ncx"
        media = "application/x-dtbncx+xml"
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b"</manifest><spine>", b"""
              <item id="ncx" href="Nav/book.ncx" media-type="application/x-dtbncx+xml"/>
              </manifest><spine toc="ncx">""")
        ncx = """<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
          <head><meta name="dtb:uid" content="urn:test:book"/><meta name="dtb:depth" content="2"/></head>
          <docTitle><text>Original contents</text></docTitle>
          <navMap><navPoint id="first-entry" playOrder="1"><navLabel><text>First entry</text></navLabel>
              <content src="../one.xhtml#old"/>
              <navPoint id="nested" playOrder="1"><navLabel><text>Section</text></navLabel>
                <content src="../one.xhtml#old"/></navPoint></navPoint>
            <navPoint id="second-entry" playOrder="2"><navLabel><text>Second entry</text></navLabel>
              <content src="../two.xhtml"/></navPoint></navMap>
          <pageList><pageTarget id="page-one" type="normal" value="1" playOrder="1">
              <navLabel><text>One</text></navLabel><content src="../one.xhtml#old"/></pageTarget></pageList>
          <navList><navLabel><text>Illustrations</text></navLabel>
            <navTarget id="fig" playOrder="2"><navLabel><text>Caption</text></navLabel>
              <content src="../two.xhtml"/></navTarget></navList></ncx>"""
        source = rewrite_epub(source, {"OEBPS/content.opf": package, ncx_path: ncx})
        authored = ncx.replace("Original contents", "Authored contents").replace("First entry", "Changed entry")
        new_anchor = authored.replace("one.xhtml#old", "one.xhtml#new-%C3%A9")
        with doc_ex.open(source) as book:
            exact = book.edit_epub_structure(resources={ncx_path: doc_ex.EpubResource(media, authored.encode())})
            edited = book.edit_epub_structure(
                [1, doc_ex.EpubChapterSource("Inserted", html.format("<p>Inserted content</p>")), 0],
                replacements={0: html.format('<h1 id="new-é">Changed chapter</h1>')},
                resources={ncx_path: doc_ex.EpubResource(media, new_anchor.encode())},
            )
            self.assertEqual(book.read_epub_resource(ncx_path), ncx.encode())
        with doc_ex.open(exact) as book:
            self.assertEqual(book.read_epub_resource(ncx_path), authored.encode())
            self.assertEqual(book.metadata["navigation"][0]["items"][0]["label"], "Changed entry")
        with doc_ex.open(edited) as book:
            self.assertEqual(book[2].get_text(), "Changed chapter")
            toc, pages, illustrations = (group["items"] for group in book.metadata["navigation"])
            self.assertEqual([item["label"] for item in toc], ["Second entry", "Inserted", "Changed entry"])
            self.assertEqual(toc[-1]["children"][0]["fragment"], "new-é")
            self.assertEqual(pages[0]["fragment"], "new-é")
            self.assertEqual(illustrations[0]["path"], "OEBPS/two.xhtml")
            xml = ET.fromstring(book.read_epub_resource(ncx_path))
            self.assertEqual(xml.findtext("{*}docTitle/{*}text"), "Authored contents")
            self.assertEqual([node.attrib["playOrder"] for node in xml.findall(".//{*}navPoint")],
                             ["1", "2", "3", "3"])
            self.assertEqual(xml.find(".//{*}pageTarget").attrib["playOrder"], "3")
            self.assertEqual(xml.find(".//{*}navTarget").attrib["playOrder"], "1")
            self.assertEqual(book.read_epub_resource("OEBPS/image.png"), PNG)

    def test_ncx_play_order_uses_final_fragment_positions_across_lists(self) -> None:
        source, _, _, html = self.nonspine_navigation_fixture()
        original = html.format(
            '<h1 id="other">Second</h1><p id="late">Later marker</p><p id="early">Earlier marker</p>'
        )
        staged = html.format(
            '<h1 id="other">Second</h1><p id="early">Earlier marker</p><p id="late">Later marker</p>'
        )
        source = rewrite_epub(source, {"OEBPS/two.xhtml": original})
        path = "OEBPS/toc.ncx"
        ncx_lists = b"""<pageList><pageTarget id="e" type="normal" value="1" playOrder="3">
            <navLabel><text>Page one</text></navLabel><content src="two.xhtml#early"/>
            </pageTarget></pageList><navList><navLabel><text>Index</text></navLabel>
            <navTarget id="t" playOrder="3"><navLabel><text>Same place</text></navLabel>
              <content src="./two.xhtml#%65arly"/></navTarget></navList>"""
        with doc_ex.open(source) as book:
            ncx = book.read_epub_resource(path).replace(b'two.xhtml"', b'two.xhtml#late"')
            ncx = ncx.replace(b"</ncx>", ncx_lists + b"</ncx>")
            result = book.edit_epub_structure(
                [1, 0], replacements={1: staged},
                resources={path: doc_ex.EpubResource("application/x-dtbncx+xml", ncx)},
            )
        with doc_ex.open(result) as book:
            xml = ET.fromstring(book.read_epub_resource(path))
            points = xml.findall(".//{*}navPoint")
            self.assertEqual([p.find("{*}content").attrib["src"] for p in points],
                             ["two.xhtml#late", "one.xhtml"])
            self.assertEqual([p.attrib["playOrder"] for p in points], ["2", "3"])
            self.assertEqual(xml.find(".//{*}pageTarget").attrib["playOrder"], "1")
            self.assertEqual(xml.find(".//{*}navTarget").attrib["playOrder"], "1")

    def test_structure_validates_explicit_compatibility_ncx(self) -> None:
        source, nav_path, nav, html = self.nonspine_navigation_fixture()
        ncx_path = "OEBPS/toc.ncx"
        media = "application/x-dtbncx+xml"
        with doc_ex.open(source) as book:
            ncx = book.read_epub_resource(ncx_path)
            replacement = ncx.replace(b"NCX two", b"New compatibility label")
            revised = book.edit_epub_structure(resources={ncx_path: doc_ex.EpubResource(media, replacement)})
            for old, new, message in (
                (b"one.xhtml", b"one.xhtml#missing", "missing fragment"),
                (b"two.xhtml", b"absent.xhtml", "not a packaged resource"),
                (b"two.xhtml", b"image.png", "local content documents"),
                (b"one.xhtml", b"https://example.org/", "local content documents"),
                (b'<content src="one.xhtml"/>', b'<content/>', "must have one target"),
                (b"</navMap>", b"</navMap><navMap/>", "one nonempty table of contents"),
            ):
                with self.subTest(message=message), self.assertRaisesRegex(doc_ex.MalformedDocumentError, message):
                    book.edit_epub_structure(resources={ncx_path: doc_ex.EpubResource(media, ncx.replace(old, new))})
            with self.assertRaisesRegex(ValueError, "packaged NCX manifest entry"):
                book.edit_epub_structure(resources={ncx_path: doc_ex.EpubResource("application/xhtml+xml", ncx)})
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_structure(
                    replacements={0: html.format('<h1 id="old">First</h1><a href="toc.ncx#ncx-1">Index</a>')},
                    resources={ncx_path: doc_ex.EpubResource(media, ncx.replace(b"ncx-1", b"renamed"))},
                )
            self.assertEqual(book.read_epub_resource(ncx_path), ncx)
        with doc_ex.open(revised) as book:
            self.assertEqual(book.read_epub_resource(ncx_path), replacement)
            self.assertEqual(book.read_epub_resource(nav_path), nav.encode())

    @staticmethod
    def nonspine_navigation_fixture() -> tuple[bytes, str, str, str]:
        html = '<html xmlns="http://www.w3.org/1999/xhtml"><body>{}</body></html>'
        source = epub_fixture(chapter_one=html.format('<h1 id="old">Original chapter text</h1>'))
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            opf = archive.read("OEBPS/content.opf")
        opf = opf.replace(b"</manifest><spine>", b"""
            <item id="edit-nav" href="Nav/contents.xhtml" media-type="application/xhtml+xml" properties="nav svg"/>
            <item id="compat" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
            </manifest><spine toc="compat">""")
        nav_path = "OEBPS/Nav/contents.xhtml"
        nav = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
          <head><title>Contents</title></head><body><nav id="old-nav" epub:type="toc"><ol>
            <li id="part-a"><span>Part A</span><ol><li><a href="../one.xhtml#old">Old introduction</a></li></ol></li>
            <li><a href="../two.xhtml#other">Second</a></li>
          </ol></nav>
          <nav epub:type="page-list"><ol><li><a href="../two.xhtml#other">Page 2</a></li></ol></nav>
          <nav epub:type="related"><ol><li><a href="https://example.org/">More</a></li></ol></nav>
          <aside><svg xmlns="http://www.w3.org/2000/svg"/></aside></body></html>"""
        ncx = """<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
          <navPoint id="ncx-1" playOrder="1"><navLabel><text>NCX one</text></navLabel><content src="one.xhtml"/></navPoint>
          <navPoint id="ncx-2" playOrder="2"><navLabel><text>NCX two</text></navLabel><content src="two.xhtml"/></navPoint>
          </navMap></ncx>"""
        revised = rewrite_epub(source, {
            "OEBPS/content.opf": opf,
            nav_path: nav,
            "OEBPS/toc.ncx": ncx,
            "OEBPS/two.xhtml": html.format(
                '<h1 id="other">Second text</h1><a href="Nav/contents.xhtml#old-nav">Contents</a>'
                '<a href="Nav/contents.xhtml#part-a">First part</a>',
            ),
        })
        return revised, nav_path, nav, html

    def test_batch_chapter_anchor_validation_covers_non_spine_references(self) -> None:
        html = '<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body>{}</body></html>'
        chapter = html.format('<p id="spare">Spare destination</p>')
        source = epub_fixture(chapter_one=chapter)
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf")
        package = package.replace(b"</manifest><spine>", b'''
          <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
          <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
          <item id="landmarks" href="Appendix/extra.xhtml" media-type="application/xhtml+xml"/>
          <item id="figure" href="Art/jump.svg" media-type="image/svg+xml"/>
          </manifest><spine toc="ncx">''')
        nav = html.format('<nav epub:type="toc"><ol><li><a href="one.xhtml">One</a></li></ol></nav>')
        ncx = '''<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>
          <navPoint id="old"><navLabel><text>Supplement</text></navLabel>
          <content src="one.xhtml#unresolved"/></navPoint></navMap></ncx>'''
        appendix = html.format('<map id="map" name="map"><area shape="default" href="../one.xhtml#unresolved"/></map>')
        figure = '''<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">
          <a href="https://example.org/" xlink:href="../one.xhtml#spare"><text y="12">Figure</text></a></svg>'''
        members = {"OEBPS/content.opf": package, "OEBPS/nav.xhtml": nav, "OEBPS/toc.ncx": ncx,
                   "OEBPS/Appendix/extra.xhtml": appendix, "OEBPS/Art/jump.svg": figure}
        changed = html.format("<p>New chapter</p>")
        for path, replacement in (
            ("OEBPS/content.opf", package.replace(
                b"</package>", b'<guide><reference type="text" title="Start" href="one.xhtml#spare"/></guide></package>',
            )),
            ("OEBPS/toc.ncx", ncx.replace("#unresolved", "#spare")),
            ("OEBPS/Appendix/extra.xhtml", appendix.replace("#unresolved", "#spare")),
            ("OEBPS/Art/jump.svg", figure.replace('href="https://example.org/" ', "")),
        ):
            with self.subTest(path=path), doc_ex.open(rewrite_epub(source, {**members, path: replacement})) as book:
                with self.assertRaisesRegex(ValueError, "referenced anchor"):
                    book.edit_epub_chapters({0: changed})
                self.assertEqual(book[0].get_text(), "Spare destination")
        with doc_ex.open(rewrite_epub(source, members)) as book:
            revised = book.edit_epub_chapters({0: changed})
        with doc_ex.open(revised) as edited:
            self.assertEqual(edited[0].get_text(), "New chapter")
            for path, contents in members.items():
                self.assertEqual(edited.read_epub_resource(path), contents.encode() if isinstance(contents, str) else contents)

    def test_replacing_packaged_svg_checks_staged_fragment_references(self) -> None:
        def svg(body: str) -> doc_ex.EpubResource:
            return doc_ex.EpubResource("image/svg+xml", (
                f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 30">{body}</svg>'
            ).encode())

        chart = "OEBPS/chart.svg"
        old_chart = svg('<rect id="café" x="5" y="5" width="40" height="12"/>')
        new_chart = svg('<rect id="key" x="5" y="5" width="40" height="12"/>')
        guide = '<html xmlns="http://www.w3.org/1999/xhtml"><body><a href="chart.svg#{}">Chart</a></body></html>'
        old_link = guide.format("caf%C3%A9")
        new_link = guide.format("key")
        with doc_ex.open(epub_fixture()) as book:
            source = book.edit_epub_structure(resources={chart: old_chart})
        with doc_ex.open(source) as book:
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_structure([doc_ex.EpubChapterSource("Guide", old_link, "OEBPS/guide.xhtml"), 0, 1],
                                         resources={chart: new_chart})
            updated = book.edit_epub_structure([doc_ex.EpubChapterSource("Guide", new_link, "OEBPS/guide.xhtml"), 0, 1],
                                               resources={chart: new_chart})
            with_link = book.edit_epub_chapters({0: old_link})
            self.assertEqual(book.read_epub_resource(chart), old_chart.data)
        with doc_ex.open(with_link) as book:
            with self.assertRaisesRegex(ValueError, "referenced anchor"):
                book.edit_epub_structure(resources={chart: new_chart})
            self.assertEqual(book.read_epub_resource(chart), old_chart.data)
        with doc_ex.open(updated) as book:
            self.assertEqual(book.read_epub_resource(chart), new_chart.data)
            link, = book[0].get_links()
            self.assertEqual(link.metadata["target_fragment"], "key")

    def test_insert_and_replace_fixed_svg_chapter_with_staged_resources(self) -> None:
        chapter_path = "OEBPS/Art/Plate one.svg"
        css_path = "OEBPS/Styles/paint.css"
        image_path = "OEBPS/Art/tile.png"
        graphic = """<?xml-stylesheet type="text/css" href="../Styles/paint.css"?>
        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 60">
          <title>Original plate</title><rect id="plate" class="paper" width="120" height="60"/>
          <image href="tile.png" x="40" y="10" width="20" height="20"/>
          <text id="caption" x="10" y="45">Original caption</text>
        </svg>"""
        resources = {
            css_path: doc_ex.EpubResource("text/css", b".paper { fill: #336699 }"),
            image_path: doc_ex.EpubResource("image/png", solid((230, 44, 26))),
        }
        chapter = doc_ex.EpubSvgChapterSource("Plate one", graphic, chapter_path)
        with doc_ex.open(epub_fixture()) as original:
            data = original.edit_epub_structure(
                [0, chapter, 1], resources=resources,
                overlays={chapter_path: [doc_ex.EpubOverlayCue(fragment="caption")]},
            )
            self.assertEqual(original.page_count, 2)

        with doc_ex.open(data) as book:
            self.assertEqual(book.page_count, 3)
            self.assertEqual(book[1].metadata["chapter_path"], chapter_path)
            self.assertEqual(book[1].metadata["chapter_media_type"], "image/svg+xml")
            self.assertEqual(book[1].metadata["viewport"], [90, 45])
            self.assertEqual(book[1].get_images()[0].alt, "Original plate")
            manifest = book.metadata["manifest"]
            self.assertIsNone(next(item for item in manifest if item["path"] == chapter_path)["properties"])
            nav = book.metadata["navigation"][0]["items"]
            self.assertEqual([item["path"] for item in nav], ["OEBPS/one.xhtml", chapter_path, "OEBPS/two.xhtml"])
            self.assertIn("Plate%20one.svg", nav[1]["href"])
            nav_path = next(item["path"] for item in manifest if item["properties"] == "nav")
            nav_source = book.read_epub_resource(nav_path)
            link = nav[1]["href"].encode()
            nav_source = nav_source.replace(b'href="' + link + b'"', b'href="' + link + b'#plate"')
            cues = book.epub_media_cues(1)
            self.assertEqual(next(book.epub_narration(1)).text, "Original caption")
            rendered = book.render_page(1, dpi=96)
            self.assertEqual(struct.unpack_from(">II", rendered, 16), (120, 60))
            _, pixels = rgba_pixels(png_chunks(rendered))
            self.assertIn(bytes.fromhex("336699ff"), pixels)
            self.assertIn(bytes.fromhex("e62c1aff"), pixels)

        data = rewrite_epub(data, {nav_path: nav_source})
        revised_graphic = graphic.replace("Original", "Revised").replace("120 60", "100 80").replace(
            'width="120" height="60"', 'width="100" height="80"',
        )
        with doc_ex.open(data) as book:
            nav = book.metadata["navigation"][0]["items"]
            self.assertEqual(nav[1]["fragment"], "plate")
            for anchor in ("caption", "plate"):
                with self.subTest(anchor=anchor), self.assertRaisesRegex(ValueError, "referenced anchor"):
                    book.edit_epub_chapters({1: revised_graphic.replace(f'id="{anchor}"', '')})
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "numeric viewBox"):
                book.edit_epub_chapters({1: revised_graphic.replace('viewBox="0 0 100 80"', '')})
            xhtml = '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>First</title></head><body><p>Replacement text</p></body></html>'
            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "SVG root"):
                book.edit_epub_chapters({1: xhtml})
            with self.assertRaises(doc_ex.MalformedDocumentError):
                book.edit_epub_chapters({0: xhtml, 1: revised_graphic.replace("tile.png", "absent.png")})
            self.assertEqual(book[1].get_images()[0].alt, "Original plate")
            revised = book.edit_epub_chapters({0: xhtml, -2: revised_graphic})
        with (
            doc_ex.open(revised) as book,
            zipfile.ZipFile(io.BytesIO(data)) as before,
            zipfile.ZipFile(io.BytesIO(revised)) as after,
        ):
            self.assertEqual(before.namelist(), after.namelist())
            for member in set(before.namelist()) - {"OEBPS/one.xhtml", chapter_path}:
                self.assertEqual(before.read(member), after.read(member), member)
            self.assertEqual(book[0].get_text(), "Replacement text")
            self.assertEqual(book[1].get_images()[0].alt, "Revised plate")
            self.assertEqual(book.epub_media_cues(1), cues)
            self.assertEqual(next(book.epub_narration(1)).text, "Revised caption")
            self.assertEqual(book.metadata["navigation"][0]["items"], nav)
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[1])) as pdf:
                self.assertEqual(len(pdf), 1)
                png = pdf.render_page(0, dpi=96)
                self.assertEqual(struct.unpack_from(">II", png, 16), (100, 80))
                self.assertIn(bytes.fromhex("e62c1aff"), rgba_pixels(png_chunks(png))[1])
        with doc_ex.open(data) as book:
            replacement = revised_graphic.replace("tile.png", "new.png")
            with self.assertRaises(doc_ex.MalformedDocumentError):
                book.edit_epub_structure(replacements={1: replacement})
            staged = book.edit_epub_structure(replacements={1: replacement}, resources={
                "OEBPS/Art/new.png": doc_ex.EpubResource("image/png", solid((72, 23, 161))),
            })
        with doc_ex.open(staged) as book:
            self.assertEqual(book[1].metadata["viewport"], [75, 60])
            self.assertEqual(next(book.epub_narration(1)).text, "Revised caption")
            _, pixels = rgba_pixels(png_chunks(book.render_page(1, dpi=96)))
            self.assertIn(bytes((72, 23, 161, 255)), pixels)

    def test_svg_use_preserves_nested_references_and_symbol_geometry(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
              width="128" height="80" viewBox="0 0 128 80"><title>Reuse</title>
          <defs>
            <rect id="tile" width="10" height="10"/>
            <g id="pair"><use href="#tile"/><use href="#tile" x="12"/></g>
            <symbol id="écu" viewBox="0 0 12 8">
              <rect width="12" height="8" fill="#148644"/>
            </symbol>
          </defs>
          <use xlink:href="#pair" x="2" y="2" fill="#d82d44"/>
          <use xlink:href="#pair" href="#tile" x="40" y="10" fill="#193ca8"/>
          <use href="#écu" x="70" y="10" width="24" height="16"/>
          </svg>"""
        chapter = doc_ex.EpubSvgChapterSource("Reuse", svg, "OEBPS/Art/reuse.svg")
        with doc_ex.open(epub_fixture()) as original:
            archived = original.edit_epub_structure([chapter, 0])

        def check_image(png: bytes) -> None:
            self.assertEqual(struct.unpack_from(">II", png, 16), (128, 80))
            width, data = rgba_pixels(png_chunks(png))
            for x, y, color in ((3, 3, "d82d44"), (15, 3, "d82d44"), (43, 15, "193ca8"),
                                (54, 15, "ffffff"), (90, 23, "148644")):
                offset = (x + y * width) * 4
                self.assertEqual(data[offset:offset + 4], bytes.fromhex(color + "ff"))

        with doc_ex.open(archived) as book:
            self.assertEqual(book.read_epub_resource("OEBPS/Art/reuse.svg"), svg.encode())
            self.assertEqual(book[0].get_images()[0].alt, "Reuse")
            check_image(book.render_page(0, dpi=96))
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0])) as pdf:
                check_image(pdf.render_page(0, dpi=96))

        xhtml = ('<html xmlns="http://www.w3.org/1999/xhtml"><body><p>'
                 + svg + "</p></body></html>")
        with doc_ex.open(epub_fixture(chapter_one=xhtml)) as book:
            self.assertEqual(book[0].get_images()[0].alt, "Reuse")
            for converted in (book.convert("docx"), book.convert("pdf")):
                with doc_ex.open(converted) as reopened:
                    _, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=96)))
                    for color in ("d82d44", "193ca8", "148644"):
                        self.assertIn(bytes.fromhex(color + "ff"), pixels)

    def test_svg_use_rejects_unsafe_or_unbounded_references(self) -> None:
        def svg(body: str) -> str:
            return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 48">'
                    + body + "</svg>")

        with doc_ex.open(epub_fixture()) as book:
            invalid_marks = [
                ('<use href="art.svg#a"/>', doc_ex.UnsupportedFormatError, "same-document"),
                ('<use href="https://example.invalid/a.svg#a"/>', doc_ex.UnsupportedFormatError, "same-document"),
                ('<use/>', doc_ex.MalformedDocumentError, "no href"),
                ('<use href="#missing"/>', doc_ex.MalformedDocumentError, "missing ID"),
                ('<g id="a"/><g id="a"/><use href="#a"/>', doc_ex.MalformedDocumentError, "duplicate ID"),
                ('<g id="one"><use href="#two"/></g><g id="two"><use href="#one"/></g>',
                 doc_ex.MalformedDocumentError, "cyclic SVG use"),
            ]
            for body, error, message in invalid_marks:
                with self.subTest(message=message), self.assertRaisesRegex(error, message):
                    book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Broken", svg(body)), 0])

            tile = '<defs><rect id="layer0" width="2" height="2"/>'
            for level in range(1, 18):
                leaf = f'<use href="#layer{level - 1}"/>'
                tile += f'<g id="layer{level}">{leaf}{leaf}</g>'
            tile += '</defs><use href="#layer17"/>'
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "size budget"):
                book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Fanout", svg(tile)), 0])

            deep = '<defs><rect id="part0" width="2" height="2"/>'
            deep += "".join(f'<use id="part{n}" href="#part{n-1}"/>' for n in range(1, 50))
            deep += '</defs><use href="#part49"/>'
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "depth budget"):
                book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Depth", svg(deep)), 0])

            styles = "<style>#tile{stroke-dasharray:" + "1 " * 1500 + "}</style>"
            fanout = (styles + '<defs><rect id="tile" width="2" height="2"/></defs>'
                      + '<use href="#tile"/>' * 6000)
            with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "size budget"):
                book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Styles", svg(fanout)), 0])

    def test_svg_gradient_templates_and_blur_render_in_epub(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
              baseProfile="basic" width="128" height="80" viewBox="0 0 128 80">
          <title>Gradient with a halo</title>
          <style>#halo { filter: url("#soft"); }</style>
          <defs>
            <linearGradient id="stops">
              <stop offset="0" stop-color="#ff0000"/>
              <stop offset="1" stop-color="#0000ff"/>
            </linearGradient>
            <linearGradient id="linear" xlink:href="#stops"/>
            <radialGradient id="radial" href="#linear"/>
            <filter id="soft" x="-50%" y="-50%" width="200%" height="200%" color-interpolation-filters="sRGB">
              <feGaussianBlur stdDeviation="3, 3"/>
            </filter>
            <rect id="halo" x="88" y="30" width="20" height="20" fill="#000000"/>
          </defs>
          <rect width="60" height="15" x="8" y="8" fill="url(#linear)"/>
          <circle r="18" cx="36" cy="49" fill="url(#radial)"/>
          <use href="#halo"/>
        </svg>"""
        chapter = doc_ex.EpubSvgChapterSource("Gradient with a halo", svg)
        with doc_ex.open(epub_fixture()) as original:
            archived = original.edit_epub_structure([chapter, 0])

        def check_image(png: bytes) -> None:
            self.assertEqual(struct.unpack_from(">II", png, 16), (128, 80))
            width, data = rgba_pixels(png_chunks(png))

            def pixel(x: int, y: int) -> bytes:
                offset = (x + y * width) * 4
                return data[offset:offset + 4]

            self.assertGreater(pixel(11, 14)[0], 220)
            self.assertGreater(pixel(65, 14)[2], 220)
            self.assertGreater(pixel(36, 49)[0], 235)
            self.assertGreater(pixel(51, 49)[2], 200)
            self.assertLess(pixel(98, 38)[0], 20)
            self.assertTrue(165 < pixel(85, 38)[0] < 235)
            self.assertGreater(pixel(78, 38)[0], 249)

        with doc_ex.open(archived) as book:
            check_image(book.render_page(0, dpi=96))
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0])) as pdf:
                check_image(pdf.render_page(0, dpi=96))

        xhtml = ('<html xmlns="http://www.w3.org/1999/xhtml"><body><p>'
                 + svg + "</p></body></html>")
        with doc_ex.open(epub_fixture(chapter_one=xhtml)) as book:
            self.assertEqual(book[0].get_images()[0].alt, "Gradient with a halo")
            for output in ("docx", "pdf"):
                with doc_ex.open(book.convert(output)) as reopened:
                    _, pixels = rgba_pixels(png_chunks(reopened.render_page(0, dpi=96)))
                    colors = set(zip(pixels[::4], pixels[1::4], pixels[2::4]))
                    self.assertTrue(any(red > 220 and blue < 30 for red, _, blue in colors))
                    self.assertTrue(any(blue > 220 and red < 30 for red, _, blue in colors))
                    self.assertTrue(any(165 < red == green == blue < 235 for red, green, blue in colors))

    def test_svg_drop_shadow_preserves_source_alpha_pixels_and_search_geometry(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" width="152" height="112" viewBox="0 0 152 112">
          <style>.echo { color: #a020e0; flood-color: currentColor; flood-opacity: 60%; }</style>
          <defs>
            <filter id="graphic" x="0" y="0" width="152" height="112" filterUnits="userSpaceOnUse"
                    color-interpolation-filters="sRGB">
              <feDropShadow in="SourceGraphic" dx="22" dy="2" stdDeviation="1.2, 2.4"
                            flood-color="#2040f0" flood-opacity=".5"/>
            </filter>
            <filter id="alpha" x="0" y="0" width="152" height="112" filterUnits="userSpaceOnUse"
                    color-interpolation-filters="sRGB">
              <feDropShadow class="echo" in="SourceAlpha" dx="-16" dy="4" stdDeviation="0"
                            flood-color="red" flood-opacity="0%"/>
            </filter>
          </defs>
          <rect x="10" y="15" width="30" height="22" fill="#e64a18" filter="url(#graphic)"/>
          <rect x="98" y="59" width="25" height="22" fill="green" fill-opacity=".6" filter="url(#alpha)"/>
          <g id="label" filter="url(#graphic)"><text x="11" y="104" font-size="14">
            <a href="https://example.org/label">LABEL</a></text></g>
        </svg>"""
        fixture = fixed_spread_fixture([("", "white", 152, 112)] * 2)
        with zipfile.ZipFile(io.BytesIO(fixture)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )
        xhtml = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Drop shadows</title>
          <meta name="viewport" content="width=152,height=112"/>
          <style>html, body {{ margin: 0 }} svg {{ display: block }}</style>
          </head><body>{svg}</body></html>"""
        replacements = {"OEBPS/content.opf": package, "OEBPS/s0.svg": svg, "OEBPS/s1.xhtml": xhtml}
        source = rewrite_epub(fixture, replacements)
        with doc_ex.open(source) as book:
            previews = [rgba_pixels(png_chunks(book.render_page(i, dpi=96))) for i in range(2)]
            fixed = book.export_epub_fixed_pdf(dpi=96, searchable=True)
            unfiltered = book.edit_epub_chapters({0: svg.replace('id="label" filter="url(#graphic)"', 'id="label"')})
        with doc_ex.open(unfiltered) as control:
            plain = control.export_epub_fixed_pdf(dpi=96, pages=[0], searchable=True)
        with doc_ex.open(fixed) as pdf, doc_ex.open(plain) as ordinary:
            for i, preview in enumerate(previews):
                width, rgba = preview

                def pixel(x: int, y: int) -> bytes:
                    offset = 4 * (y * width + x)
                    return rgba[offset:offset + 3]

                self.assertEqual(pixel(25, 25), bytes((230, 74, 24)))
                self.assertTrue(120 < pixel(46, 27)[0] < 180 and pixel(46, 27)[2] > 230)
                alpha = pixel(115, 70)
                self.assertTrue(99 <= alpha[0] == alpha[1] == alpha[2] <= 105)
                tinted = pixel(87, 70)
                self.assertGreater(tinted[0] - tinted[1], 25)
                self.assertGreater(tinted[2] - tinted[1], 50)
                self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(i, dpi=96))), preview)
                self.assertEqual(pdf[i].search_for("LABEL"), ordinary[0].search_for("LABEL"))
                link, = pdf[i].get_links()
                self.assertEqual(link.uri, "https://example.org/label")
                self.assertEqual(link.bbox, ordinary[0].get_links()[0].bbox)

        semantic = xhtml.replace('<a href="https://example.org/label">', "").replace("</a>", "")
        with doc_ex.open(epub_fixture(chapter_one=semantic)) as reflowable:
            for output in ("docx", "pdf"):
                with doc_ex.open(reflowable.convert(output)) as reopened:
                    _, rgba = rgba_pixels(png_chunks(reopened.render_page(0, dpi=96)))
                    pixels = set(zip(rgba[::4], rgba[1::4], rgba[2::4]))
                    self.assertIn((230, 74, 24), pixels)
                    self.assertTrue(any(120 < red < 180 and blue > 230 for red, _, blue in pixels))

    def test_svg_drop_shadow_checks_source_parameters_and_scaled_work(self) -> None:
        def svg(body: str) -> str:
            return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 48">{body}</svg>'

        def effect(primitive: str, body: str) -> str:
            return f'<defs><filter id="ds">{primitive}</filter></defs>{body}'

        shape = '<rect width="12" height="12" filter="url(#ds)"/>'
        cases = (
            ('<feDropShadow in="BackgroundAlpha"/>', doc_ex.UnsupportedFormatError, "SourceGraphic or SourceAlpha"),
            ('<feDropShadow dx="NaN"/>', doc_ex.MalformedDocumentError, "invalid SVG drop shadow offset"),
            ('<feDropShadow dy="-16001"/>', doc_ex.UnsafeArchiveError, "drop shadow offset"),
            ('<feDropShadow stdDeviation="-2 3"/>', doc_ex.MalformedDocumentError, "blur deviation"),
            ('<feDropShadow flood-color="not-a-color"/>', doc_ex.MalformedDocumentError, "flood color"),
            ('<feDropShadow flood-color="url(#ds)"/>', doc_ex.UnsupportedFormatError, "active attributes"),
            ('<feDropShadow flood-opacity="1em"/>', doc_ex.MalformedDocumentError, "flood opacity"),
        )
        with doc_ex.open(epub_fixture()) as book:
            for primitive, error, message in cases:
                with self.subTest(primitive=primitive), self.assertRaisesRegex(error, message):
                    book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Invalid shadow", svg(effect(primitive, shape))), 0])

            with self.assertRaisesRegex(doc_ex.MalformedDocumentError, "misplaced SVG filter primitive"):
                orphan = svg('<feDropShadow dx="NaN"/>')
                book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Invalid shadow", orphan), 0])

            scaled = effect('<feDropShadow dx="1800" stdDeviation=".1"/>',
                            '<rect width="1" height="1" transform="scale(9)" filter="url(#ds)"/>')
            large_offset = book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Offset", svg(scaled)), 0])
            many = effect('<feDropShadow stdDeviation="0"/>',
                          '<rect width="1100" height="1100" filter="url(#ds)"/>' * 9)
            large_work = book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Work", svg(many)), 0])
        for data, reason in ((large_offset, "drop shadow offset"), (large_work, "filter work")):
            with self.subTest(reason=reason), doc_ex.open(data) as book:
                with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, reason):
                    book.render_page(0, dpi=96)

    def test_svg_color_matrix_preserves_alpha_text_and_fixed_pdf_links(self) -> None:
        matrix = "0 0 0 0 0, 1 0 0 0 0, 0 0 0 0 0, 0 0 0 1 0"
        xhtml = f"""<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Color</title>
          <meta name="viewport" content="width=120,height=60"/></head><body style="margin:0">
          <svg xmlns="http://www.w3.org/2000/svg" width="120" height="60" viewBox="0 0 120 60">
            <defs>
              <filter id="green" color-interpolation-filters="sRGB"><feColorMatrix values="{matrix}"/></filter>
              <filter id="gray" color-interpolation-filters="sRGB"><feColorMatrix type="saturate" values="0"/></filter>
              <filter id="hue" color-interpolation-filters="sRGB"><feColorMatrix type="hueRotate" values="120"/></filter>
            </defs>
            <g fill="red">
              <rect x="4" y="4" width="24" height="20" filter="url(#green)" opacity=".5"/>
              <rect x="36" y="4" width="24" height="20" filter="url(#gray)"/>
              <rect x="68" y="4" width="24" height="20" filter="url(#hue)"/>
              <a href="https://example.org/color"><text x="5" y="45" filter="url(#green)">COLOR</text></a>
            </g>
          </svg></body></html>"""
        plain = xhtml.replace('filter="url(#green)"', "")
        fixture = fixed_spread_fixture([("", "white", 120, 60)])
        with (
            doc_ex.open(rewrite_epub(fixture, {"OEBPS/s0.xhtml": xhtml})) as book,
            doc_ex.open(rewrite_epub(fixture, {"OEBPS/s0.xhtml": plain})) as reference,
        ):
            def rgba(document: doc_ex.Document) -> tuple[int, bytes]:
                return rgba_pixels(png_chunks(document.render_page(0, dpi=96)))

            width, pixels = preview = rgba(book)
            def pixel(x: int, y: int) -> bytes:
                offset = 4 * (width * y + x)
                return pixels[offset:offset + 3]

            red, green, blue = pixel(12, 12)
            self.assertTrue(125 <= red == blue <= 130 and green == 255)
            gray = pixel(44, 12)
            self.assertTrue(53 <= gray[0] == gray[1] == gray[2] <= 56)
            red, green, blue = pixel(76, 12)
            self.assertTrue(red < 10 and green > 100 and blue < 10)
            with (
                doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0], searchable=True)) as pdf,
                doc_ex.open(reference.export_epub_fixed_pdf(dpi=96, pages=[0], searchable=True)) as original,
                doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0])) as visual,
            ):
                self.assertEqual(rgba(pdf), preview)
                self.assertEqual(rgba(visual), preview)
                self.assertEqual(pdf[0].search_for("COLOR"), original[0].search_for("COLOR"))
                link, = pdf[0].get_links()
                self.assertEqual(link.uri, "https://example.org/color")
                self.assertEqual(link.bbox, original[0].get_links()[0].bbox)

        cases = (
            ('type="unknown"', doc_ex.UnsupportedFormatError, "color matrix mode"),
            ('type="saturate" values="2"', doc_ex.UnsupportedFormatError, "saturation"),
            ('type="hueRotate" values="NaN"', doc_ex.MalformedDocumentError, "color matrix values"),
            ('type="matrix" values="1,,2"', doc_ex.MalformedDocumentError, "color matrix values"),
            (f'values="2000{matrix[1:]}"', doc_ex.UnsafeArchiveError, "color matrix coefficient"),
        )
        with doc_ex.open(epub_fixture()) as book:
            for attributes, error, reason in cases:
                svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 20">
                  <defs><filter id="color"><feColorMatrix {attributes}/></filter></defs>
                  <rect width="10" height="10" filter="url(#color)"/></svg>"""
                with self.subTest(attributes=attributes), self.assertRaisesRegex(error, reason):
                    book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Color", svg), 0])

    def test_svg_filter_chain_renders_named_results_and_alpha_to_visual_outputs(self) -> None:
        tint = "0 0 0 0 0, 1 0 0 0 0, 0 0 0 0 0, 0 0 0 1 0"
        half = "1 0 0 0 0, 0 1 0 0 0, 0 0 1 0 0, 0 0 0 .5 0"
        svg = f"""<svg xmlns="http://www.w3.org/2000/svg" width="124" height="64" viewBox="0 0 124 64">
          <defs>
            <filter id="tint" filterUnits="userSpaceOnUse" x="0" y="0" width="124" height="64"
                    color-interpolation-filters="sRGB">
              <feColorMatrix in="SourceGraphic" values="{tint}" result="green"/>
              <feDropShadow in="SourceAlpha" dx="25" dy="0" stdDeviation="0" result="unused"/>
              <feColorMatrix in="green" values="{half}"/>
            </filter>
            <filter id="fade" filterUnits="userSpaceOnUse" x="0" y="0" width="124" height="64"
                    color-interpolation-filters="sRGB">
              <feColorMatrix type="luminanceToAlpha" result="luminance"/>
              <feGaussianBlur stdDeviation="1"/>
            </filter>
          </defs>
          <rect x="4" y="4" width="22" height="18" fill="red" filter="url(#tint)"/>
          <rect x="68" y="4" width="26" height="18" fill="red" filter="url(#fade)"/>
          <text x="4" y="50" id="label" fill="red" filter="url(#tint)">FILTERED</text>
        </svg>"""
        template = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Filters</title>
            <meta name="viewport" content="width=124,height=64"/></head>
            <body style="margin:0">{}</body></html>"""
        fixture = fixed_spread_fixture([("", "white", 124, 64)])
        source = rewrite_epub(fixture, {"OEBPS/s0.xhtml": template.format(svg)})

        def rgba(document: doc_ex.Document) -> tuple[int, bytes]:
            return rgba_pixels(png_chunks(document.render_page(0, dpi=96)))

        with doc_ex.open(source) as book:
            width, pixels = preview = rgba(book)

            def pixel(x: int, y: int) -> tuple[int, ...]:
                offset = 4 * (y * width + x)
                return tuple(pixels[offset:offset + 3])

            red, green, blue = pixel(14, 12)
            self.assertTrue(126 <= red == blue <= 130 and green == 255)
            self.assertEqual(pixel(36, 12), (255, 255, 255))
            red, green, blue = pixel(81, 12)
            self.assertTrue(200 <= red == green == blue <= 205)
            self.assertTrue(230 <= pixel(66, 12)[0] < 255)
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0])) as pdf:
                self.assertEqual(rgba(pdf), preview)
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0], searchable=True)) as pdf:
                self.assertTrue(pdf[0].search_for("FILTERED"))
                self.assertEqual(rgba(pdf), preview)

        linked = svg.replace(
            '<text x="4" y="50" id="label" fill="red" filter="url(#tint)">FILTERED</text>',
            '<a href="https://example.org/"><text x="4" y="50" fill="red" filter="url(#tint)">FILTERED</text></a>',
        )
        with doc_ex.open(rewrite_epub(fixture, {"OEBPS/s0.xhtml": template.format(linked)})) as book:
            self.assertEqual(rgba(book), preview)
            with doc_ex.open(book.export_epub_fixed_pdf(dpi=96, pages=[0])) as pdf:
                link, = pdf[0].get_links()
                self.assertEqual(link.uri, "https://example.org/")
                self.assertEqual(rgba(pdf), preview)

        with doc_ex.open(epub_fixture(chapter_one=template.format(svg))) as book:
            for output in ("docx", "pdf"):
                with doc_ex.open(book.convert(output)) as reopened:
                    _, rgba_data = rgba(reopened)
                    colors = set(zip(rgba_data[::4], rgba_data[1::4], rgba_data[2::4]))
                    self.assertTrue(any(126 <= red == blue <= 130 and green == 255 for red, green, blue in colors))

    def test_svg_filter_chain_keeps_source_text_and_annotations_on_safe_final_branch(self) -> None:
        tint = "0 0 0 0 0, 1 0 0 0 0, 0 0 0 0 0, 0 0 0 1 0"
        svg = f"""<svg xmlns="http://www.w3.org/2000/svg" width="112" height="62" viewBox="0 0 112 62">
          <defs>
            <filter id="fx" filterUnits="userSpaceOnUse" x="0" y="0" width="112" height="62"
                    color-interpolation-filters="sRGB">
              <feColorMatrix values="{tint}" result="tinted"/>
              <feGaussianBlur result="unused" stdDeviation="2"/>
              <feDropShadow in="tinted" stdDeviation="0" dx="14" dy="3" flood-color="magenta"/>
            </filter>
          </defs>
          <a href="https://example.org/label"><text x="8" y="30" fill="red" font-size="16"
              filter="url(#fx)">TINT</text></a>
        </svg>"""
        fixture = fixed_spread_fixture([("", "white", 112, 62)])
        with zipfile.ZipFile(io.BytesIO(fixture)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )
        members = {"OEBPS/content.opf": package, "OEBPS/s0.svg": svg}

        def filtered_pdf(data: bytes) -> bytes:
            with doc_ex.open(data) as book:
                preview = rgba_pixels(png_chunks(book.render_page(0, dpi=96)))
                result = book.export_epub_fixed_pdf(dpi=96, searchable=True)
                with doc_ex.open(result) as pdf:
                    self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))), preview)
                return result

        with (
            doc_ex.open(filtered_pdf(rewrite_epub(fixture, members))) as filtered,
            doc_ex.open(filtered_pdf(rewrite_epub(fixture, {**members, "OEBPS/s0.svg": svg.replace('filter="url(#fx)"', "")}))) as original,
        ):
            self.assertTrue(filtered[0].search_for("TINT"))
            self.assertEqual(filtered[0].search_for("TINT"), original[0].search_for("TINT"))
            link, = filtered[0].get_links()
            self.assertEqual(link.uri, "https://example.org/label")
            self.assertEqual(link.bbox, original[0].get_links()[0].bbox)

    def test_svg_blend_and_component_transfer_keep_pixels_and_source_geometry(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 180 74">
          <defs>
            <filter id="tint" filterUnits="userSpaceOnUse" x="0" y="0" width="180" height="74"
                    color-interpolation-filters="sRGB">
              <feComponentTransfer result="tint">
                <feFuncR type="linear" slope=".5"/>
                <feFuncG type="table" tableValues="1, 0"/>
                <feFuncB type="gamma" amplitude="1" exponent="2"/>
                <feFuncA/>
              </feComponentTransfer>
              <feFlood flood-color="magenta" result="unused"/>
              <feBlend in="SourceGraphic" in2="tint" mode="multiply"/>
            </filter>
            <filter id="threshold" filterUnits="userSpaceOnUse" x="0" y="0" width="180" height="74"
                    color-interpolation-filters="sRGB">
              <feComponentTransfer result="ink"><feFuncA type="discrete" tableValues="0 1"/></feComponentTransfer>
              SHIFT
            </filter>
          </defs>
          <rect x="4" y="6" width="22" height="15" fill="#80c040" filter="url(#tint)"/>
          <g filter="url(#threshold)"><text x="4" y="50" font-size="16">
            <a href="https://example.org/removed" fill-opacity=".25">Hidden</a>
            <a href="https://example.org/visible" fill-opacity=".8" x="68" id="label">Café</a>
          </text></g>
          <a href="#label"><rect x="150" y="11" width="12" height="8" fill="blue"/></a>
          <a href="https://example.org/tinted"><text x="68" y="20" fill="#80c040" font-size="13"
                filter="url(#tint)">Tinted</text></a>
        </svg>"""
        fixture = fixed_spread_fixture([("", "white", 180, 74)])
        template = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Filters</title>
            <meta name="viewport" content="width=180,height=74"/></head>
            <body style="margin:0">{}</body></html>"""
        with zipfile.ZipFile(io.BytesIO(fixture)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b'href="s0.xhtml" media-type="application/xhtml+xml"',
                b'href="s0.svg" media-type="image/svg+xml"',
            )

        def publish(markup: str, inline: bool) -> doc_ex.Document:
            replacements = ({"OEBPS/s0.xhtml": template.format(markup)} if inline else
                            {"OEBPS/content.opf": package, "OEBPS/s0.svg": markup})
            return doc_ex.open(rewrite_epub(fixture, replacements))

        def rgba(document: doc_ex.Document) -> tuple[int, bytes]:
            return rgba_pixels(png_chunks(document.render_page(0, dpi=96)))

        for inline in (False, True):
            with self.subTest(inline=inline):
                shifted = svg.replace("SHIFT", '<feOffset in="ink" dx="8"/>')
                ordinary = svg.replace("SHIFT", "").replace(' filter="url(#tint)"', "")
                with publish(shifted, inline) as book, publish(ordinary, inline) as plain:
                    preview = rgba(book)
                    width, pixels = preview
                    at = 4 * (width * 12 + 12)
                    for channel, expected in zip(pixels[at:at + 3], (32, 47, 4)):
                        self.assertAlmostEqual(channel, expected, delta=2)
                    self.assertNotEqual(preview, rgba(plain))
                    output = book.export_epub_fixed_pdf(dpi=96, searchable=True)
                    reference = plain.export_epub_fixed_pdf(dpi=96, searchable=True)
                with doc_ex.open(output) as pdf, doc_ex.open(reference) as base, pdf.edit() as editor:
                    self.assertEqual(rgba(pdf), preview)
                    self.assertNotIn("Hidden", pdf[0].get_text())
                    visible, = pdf[0].search_for("Café")
                    starting, = base[0].search_for("Café")
                    for edge, shift in enumerate((6, 0, 6, 0)):
                        self.assertAlmostEqual(visible[edge], starting[edge] + shift, delta=0.75)
                    self.assertEqual(pdf[0].search_for("Tinted"), base[0].search_for("Tinted"))
                    links = {link.uri: link.bbox for link in pdf[0].get_links() if link.uri}
                    baseline_links = {link.uri: link.bbox for link in base[0].get_links() if link.uri}
                    self.assertNotIn("https://example.org/removed", links)
                    self.assertEqual(links["https://example.org/tinted"], baseline_links["https://example.org/tinted"])
                    visible_link = links["https://example.org/visible"]
                    for edge, shift in enumerate((6, 0, 6, 0)):
                        self.assertAlmostEqual(visible_link[edge], baseline_links["https://example.org/visible"][edge] + shift,
                                               delta=0.75)
                    local, = [link for link in pdf[0].get_links() if link.metadata.get("target_page") is not None]
                    self.assertEqual(local.metadata["target_page"], 0)
                    destination, = re.findall(
                        rb"/XYZ\s+([1-9][0-9.]*)\s+([0-9.]+)\s+null", editor.to_bytes(object_streams=False),
                    )
                    self.assertAlmostEqual(float(destination[0]), visible[0], delta=1)

        additive = svg.replace("SHIFT", "").replace(
            '<feFuncA type="discrete" tableValues="0 1"/>',
            '<feFuncA type="linear" slope="-1" intercept=".5"/>',
        )
        with publish(additive, True) as book:
            book.render_page(0, dpi=96)
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "SVG filter can change the painted source"):
                book.export_epub_fixed_pdf(dpi=96, searchable=True)

    def test_svg_filter_chain_rejects_invalid_references_and_bounds_work(self) -> None:
        def svg(primitives: str, *, side: int = 80) -> str:
            return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 48">
              <defs><filter id="fx" filterUnits="userSpaceOnUse" x="0" y="0" width="{side}" height="{side}">
                {primitives}</filter></defs>
              <rect width="12" height="12" filter="url(#fx)"/></svg>"""

        cases = (
            ('<feGaussianBlur in="missing"/>', doc_ex.MalformedDocumentError, "prior named result"),
            ('<feGaussianBlur in="later"/><feColorMatrix result="later"/>',
             doc_ex.MalformedDocumentError, "prior named result"),
            ('<feDropShadow in="self" result="self"/>', doc_ex.MalformedDocumentError, "prior named result"),
            ('<feGaussianBlur result="one"/><feColorMatrix result="one"/>',
             doc_ex.MalformedDocumentError, "duplicate SVG filter result"),
            ('<feGaussianBlur result="SourceAlpha"/>', doc_ex.MalformedDocumentError, "invalid SVG filter result"),
            ('<feGaussianBlur result="two words"/>', doc_ex.MalformedDocumentError, "invalid SVG filter result"),
            ('<feGaussianBlur result="a"/><feColorMatrix in="a" width="0"/>',
             doc_ex.MalformedDocumentError, "invalid SVG filter region"),
            ('<feColorMatrix/>' * 17, doc_ex.UnsafeArchiveError, "16 primitives"),
            ('<feBlend in2="later"/><feFlood result="later"/>',
             doc_ex.MalformedDocumentError, "prior named result"),
            ('<feBlend mode="plus-lighter"/>', doc_ex.UnsupportedFormatError, "blend mode"),
            ('<feComponentTransfer><feFuncA type="identity"/><feFuncA type="linear"/></feComponentTransfer>',
             doc_ex.UnsupportedFormatError, "repeated SVG component transfer channel"),
            ('<feComponentTransfer><feFuncR type="unknown"/></feComponentTransfer>',
             doc_ex.UnsupportedFormatError, "unsupported SVG component transfer function"),
            ('<feComponentTransfer><feFuncB type="gamma" exponent="-1"/></feComponentTransfer>',
             doc_ex.UnsupportedFormatError, "nonnegative exponent"),
            ('<feComponentTransfer><feFuncG type="linear" slope="NaN"/></feComponentTransfer>',
             doc_ex.MalformedDocumentError, "invalid SVG component transfer coefficient"),
            ('<feComponentTransfer><feFuncG type="linear" slope="&#160;.5"/></feComponentTransfer>',
             doc_ex.MalformedDocumentError, "invalid SVG component transfer parameter"),
            ('<feComponentTransfer><feFuncR type="table" tableValues="0,,1"/></feComponentTransfer>',
             doc_ex.MalformedDocumentError, "invalid SVG component transfer table"),
            (f'<feComponentTransfer><feFuncA type="table" tableValues="{("0 " * 65).strip()}"/></feComponentTransfer>',
             doc_ex.UnsafeArchiveError, "table exceeds 64 entries"),
        )
        with doc_ex.open(epub_fixture()) as book:
            for primitives, error, reason in cases:
                with self.subTest(reason=reason), self.assertRaisesRegex(error, reason):
                    book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Filter", svg(primitives)), 0])
            heavy_gamma = ('<feComponentTransfer>' + ''.join(
                f'<feFunc{channel} type="gamma" exponent=".5"/>' for channel in "RGBA"
            ) + '</feComponentTransfer>')
            expensive = book.edit_epub_structure([
                doc_ex.EpubSvgChapterSource("Filter", svg(heavy_gamma, side=1200)), 0,
            ])
            large = book.edit_epub_structure([
                doc_ex.EpubSvgChapterSource("Filter", svg('<feColorMatrix/>' * 4, side=1200)), 0,
            ])
        for data in (large, expensive):
            with (
                doc_ex.open(data) as book,
                self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "SVG filter work"),
            ):
                book.render_page(0, dpi=96)

    def test_svg_flood_offset_composite_and_merge_keep_source_coverage(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" width="136" height="72" viewBox="0 0 136 72">
          <style>.shade {color:#2040d0;flood-color:currentColor;flood-opacity:50%}</style>
          <defs>
            <filter id="halo" color-interpolation-filters="sRGB" filterUnits="userSpaceOnUse"
                    x="0" y="0" width="136" height="72">
              <feFlood class="shade" result="tint"/>
              <feComposite in="tint" operator="in" in2="SourceAlpha" result="ink"/>
              <feOffset in="ink" dx="24" dy="-5" result="echo"/>
              <feMerge><feMergeNode in="echo"/><feMergeNode in="SourceGraphic"/></feMerge>
            </filter>
            <filter id="arithmetic" color-interpolation-filters="sRGB">
              <feComposite operator="arithmetic" in="SourceGraphic" in2="SourceAlpha" k2=".5"/>
            </filter>
          </defs>
          <rect x="5" y="10" width="16" height="20" fill="#d04020" filter="url(#halo)"/>
          <a href="https://example.org/gain">
            <rect x="72" y="10" width="18" height="20" fill="blue" filter="url(#arithmetic)"/>
          </a>
          <text x="5" y="60" font-size="14" filter="url(#halo)">
            <a href="https://example.org/label">Label</a>
          </text>
        </svg>"""
        template = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Composition</title>
            <meta name="viewport" content="width=136,height=72"/></head>
            <body style="margin:0">{}</body></html>"""
        fixture = fixed_spread_fixture([("", "white", 136, 72)])
        data = rewrite_epub(fixture, {"OEBPS/s0.xhtml": template.format(svg)})
        control = rewrite_epub(
            fixture, {"OEBPS/s0.xhtml": template.format(svg.replace('filter="url(#halo)"', ""))},
        )

        def rgba(document: doc_ex.Document) -> tuple[int, bytes]:
            return rgba_pixels(png_chunks(document.render_page(0, dpi=96)))

        with doc_ex.open(data) as book, doc_ex.open(control) as plain:
            width, data = preview = rgba(book)

            def pixel(x: int, y: int) -> tuple[int, ...]:
                at = 4 * (width * y + x)
                return tuple(data[at:at + 3])

            self.assertEqual(pixel(12, 20), (208, 64, 32))
            red, green, blue = pixel(37, 7)
            self.assertTrue(142 <= red <= 145 and 158 <= green <= 161 and 230 <= blue <= 233)
            red, green, blue = pixel(80, 20)
            self.assertTrue(126 <= red == green <= 130 and blue == 255)
            self.assertEqual(pixel(54, 20), (255, 255, 255))
            with (
                doc_ex.open(book.export_epub_fixed_pdf(dpi=96, searchable=True)) as pdf,
                doc_ex.open(plain.export_epub_fixed_pdf(dpi=96, searchable=True)) as original,
            ):
                self.assertEqual(rgba(pdf), preview)
                self.assertTrue(pdf[0].search_for("Label"))
                self.assertEqual(pdf[0].search_for("Label"), original[0].search_for("Label"))
                limits = {link.uri: link.bbox for link in pdf[0].get_links()}
                for actual, expected in zip(limits["https://example.org/gain"], (54.0, 7.5, 67.5, 22.5)):
                    self.assertAlmostEqual(actual, expected, delta=0.001)
                label, = [link for link in original[0].get_links() if link.uri.endswith("/label")]
                self.assertEqual(limits[label.uri], label.bbox)

    def test_svg_pointwise_alpha_samples_surviving_text_and_localizes_uncertainty(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" width="200" height="74" viewBox="0 0 200 74">
          <defs>
            <filter id="mask" color-interpolation-filters="sRGB">
              <feColorMatrix type="luminanceToAlpha"/>
            </filter>
            <filter id="move"><feOffset dx="2"/></filter>
            <filter id="uncertain"><feOffset dx="2"/><feGaussianBlur stdDeviation="1"/></filter>
          </defs>
          <rect id="decoration" x="4" y="6" width="24" height="12" fill="blue" filter="url(#move)"/>
          <a href="https://example.org/clear"><rect x="6" y="6" width="20" height="18"
            fill="black" filter="url(#mask)"/></a>
          <g id="luma" filter="url(#mask)"><text x="6" y="50" font-size="16">
            <a href="https://example.org/removed" fill="black">Missing</a>
            <a href="https://example.org/retained" fill="red" x="90">Visible</a>
          </text></g>
        </svg>"""
        template = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Coverage</title>
          <meta name="viewport" content="width=200,height=74"/></head><body style="margin:0">{}</body></html>"""
        fixture = fixed_spread_fixture([("", "white", 200, 74)])

        def book(markup: str) -> doc_ex.Document:
            return doc_ex.open(rewrite_epub(fixture, {"OEBPS/s0.xhtml": template.format(markup)}))

        with book(svg) as main, doc_ex.open(main.export_epub_fixed_pdf(dpi=96, searchable=True)) as pdf:
            self.assertEqual(pdf[0].get_text(), "Visible")
            link, = pdf[0].get_links()
            self.assertEqual(link.uri, "https://example.org/retained")
            self.assertGreaterEqual(link.bbox[0], 67.5)
            self.assertEqual(
                rgba_pixels(png_chunks(pdf.render_page(0, dpi=96))),
                rgba_pixels(png_chunks(main.render_page(0, dpi=96))),
            )

        multi = svg.replace('filter="url(#mask)"><text', 'filter="url(#mask)"><rect width="20" height="20"/><text')
        moved = svg.replace('filter="url(#mask)"><text', 'filter="url(#uncertain)"><text')
        requested = svg.replace('filter="url(#move)"', 'filter="url(#uncertain)"').replace(
            '</svg>', '<a href="#decoration"><rect x="120" y="10" width="12" height="8"/></a></svg>',
        )
        for markup in (multi, moved, requested):
            with (
                self.subTest(markup=markup),
                book(markup) as publication,
                self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "SVG filter can change the painted source"),
            ):
                publication.export_epub_fixed_pdf(dpi=96, searchable=True)

    def test_svg_offset_chains_move_searchable_text_links_and_fragments(self) -> None:
        svg = """<svg xmlns="http://www.w3.org/2000/svg" width="184" height="120" viewBox="0 0 184 120">
          <defs>
            <clipPath id="text-window"><rect x="2" y="22" width="130" height="44"/></clipPath>
            <clipPath id="marker-window"><rect x="147" y="0" width="35" height="85"/></clipPath>
            <clipPath id="source-window"><rect x="-13" y="90" width="5" height="20"/></clipPath>
            <g id="outside">
              <a href="https://example.org/recovered" clip-path="url(#source-window)">
                <rect x="-15" y="95" width="8" height="10" fill="green"/>
              </a>
            </g>
            <filter id="move" filterUnits="userSpaceOnUse" x="-75" y="-50" width="260" height="170">
              <feOffset in="SourceGraphic" dx="13.75" dy="-4.75" result="started"/>
              <feGaussianBlur in="SourceGraphic" stdDeviation="2" result="discarded"/>
              <feColorMatrix in="started" type="saturate" values="1"/>
              <feOffset dx="6.75" dy="7.75"/>
            </filter>
          </defs>
          <g clip-path="url(#text-window)">
            <g transform="translate(5 18) scale(2 1)">
              <text filter="url(#move)" x="3" y="22" font-size="16">
                <a href="https://example.org/word">Café</a>
                <tspan x="65">Hidden</tspan>
              </text>
            </g>
          </g>
          <g clip-path="url(#marker-window)">
            <a href="https://example.org/mark">
              <g transform="translate(170 35) rotate(90)">
                <g filter="url(#move)"><rect id="marker" x="4" y="5" width="16" height="12" fill="blue"/></g>
              </g>
            </a>
          </g>
          <g filter="url(#move)"><use href="#outside"/></g>
          <a href="#marker"><rect x="144" y="92" width="14" height="10" fill="orange"/></a>
        </svg>"""
        fixture = fixed_spread_fixture([("", "white", 184, 120)])
        with zipfile.ZipFile(io.BytesIO(fixture)) as archive:
            package = archive.read("OEBPS/content.opf").replace(
                b"</manifest>",
                b'<item id="toc" href="toc.xhtml" properties="nav" media-type="application/xhtml+xml"/></manifest>',
            )
        svg_package = package.replace(
            b'href="s0.xhtml" media-type="application/xhtml+xml"',
            b'href="s0.svg" media-type="image/svg+xml"',
        )
        template = """<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Shifted</title>
          <meta name="viewport" content="width=184,height=120"/></head><body style="margin:0">{}</body></html>"""

        navigation = """<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
          <head><title>Contents</title></head><body><nav epub:type="toc"><ol>
            <li><a href="{}#marker">Shifted marker</a></li>
          </ol></nav></body></html>"""

        def publication(markup: str, inline: bool) -> bytes:
            resource = "s0.xhtml" if inline else "s0.svg"
            return rewrite_epub(fixture, {
                "OEBPS/content.opf": package if inline else svg_package,
                "OEBPS/toc.xhtml": navigation.format(resource),
                "OEBPS/" + resource: template.format(markup) if inline else markup,
            })

        def offset(scale: float, first: float, second: float) -> float:
            return (int(first * scale) + int(second * scale)) / scale

        for inline in (False, True):
            for dpi in (96, 144):
                with self.subTest(inline=inline, dpi=dpi):
                    scale = dpi / 96
                    dx = offset(scale, 13.75, 6.75)
                    dy = offset(scale, -4.75, 7.75)
                    text_dx = offset(2 * scale, 13.75, 6.75)
                    control = svg.replace(
                        '<text filter="url(#move)"',
                        f'<text transform="translate({text_dx} {dy})"',
                    ).replace('filter="url(#move)"', f'transform="translate({dx} {dy})"')
                    control = control.replace(' clip-path="url(#text-window)"', '')
                    with doc_ex.open(publication(svg, inline)) as book, doc_ex.open(publication(control, inline)) as plain:
                        output = book.export_epub_fixed_pdf(dpi=dpi, searchable=True)
                        reference = plain.export_epub_fixed_pdf(dpi=dpi, searchable=True)
                        preview = rgba_pixels(png_chunks(book.render_page(0, dpi=dpi)))
                        self.assertEqual(book.read_epub_resource("OEBPS/s0.xhtml" if inline else "OEBPS/s0.svg"),
                                         (template.format(svg) if inline else svg).encode())
                    with doc_ex.open(output) as pdf, doc_ex.open(reference) as expected, pdf.edit() as editor:
                        self.assertEqual(rgba_pixels(png_chunks(pdf.render_page(0, dpi=dpi))), preview)
                        self.assertEqual(pdf[0].get_text(), "Café")
                        rect, = pdf[0].search_for("Café")
                        target, = expected[0].search_for("Café")
                        for actual, projected in zip(rect, target):
                            self.assertAlmostEqual(actual, projected, delta=0.75)
                        links = {link.uri: link.bbox for link in pdf[0].get_links() if (link.uri or "").startswith("https:")}
                        expected_links = {link.uri: link.bbox for link in expected[0].get_links() if (link.uri or "").startswith("https:")}
                        self.assertEqual(set(links), {
                            "https://example.org/word", "https://example.org/mark", "https://example.org/recovered",
                        })
                        for uri, bbox in links.items():
                            for actual, projected in zip(bbox, expected_links[uri]):
                                self.assertAlmostEqual(actual, projected, delta=0.75)
                        internal, = [link for link in pdf[0].get_links() if link.metadata.get("target_page") is not None]
                        self.assertEqual(internal.metadata["target_page"], 0)
                        self.assertEqual([(entry.title, entry.page) for entry in editor.bookmarks], [("Shifted marker", 0)])
                        destinations = re.findall(rb"/XYZ\s+([0-9.]+)\s+([0-9.]+)\s+null", editor.to_bytes(object_streams=False))
                        self.assertEqual(len(destinations), 2)
                        for x, y in destinations:
                            self.assertAlmostEqual(float(x), (153 - dy) * .75, delta=0.25)
                            self.assertAlmostEqual(float(y), (81 - dx) * .75, delta=0.25)

        stacked = svg.replace('<g transform="translate(5 18) scale(2 1)">',
                              '<g transform="translate(5 18) scale(2 1)" filter="url(#move)">')
        with (
            doc_ex.open(publication(stacked, False)) as book,
            self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "SVG filter can change the painted source"),
        ):
            book.export_epub_fixed_pdf(dpi=96, searchable=True)

    def test_svg_compositor_checks_second_inputs_children_and_scaled_budget(self) -> None:
        def svg(primitives: str, attrs: str = "", rect: str = "") -> str:
            return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 48">
              <defs><filter id="fx" {attrs}>{primitives}</filter></defs>
              <rect width="12" height="12" filter="url(#fx)" {rect}/></svg>"""

        cases = [
            ('<feComposite in2="later"/><feFlood result="later"/>',
             doc_ex.MalformedDocumentError, "prior named result"),
            ('<feMerge result="self"><feMergeNode in="self"/></feMerge>',
             doc_ex.MalformedDocumentError, "prior named result"),
            ('<feMerge><feFlood/></feMerge>', doc_ex.MalformedDocumentError, "feMergeNode children"),
            ("<feMerge/>", doc_ex.MalformedDocumentError, "merge requires an input"),
            ("<feMerge>" + '<feMergeNode in="SourceGraphic"/>' * 17 + "</feMerge>",
             doc_ex.UnsafeArchiveError, "16 inputs"),
            ('<feComposite operator="unknown"/>', doc_ex.UnsupportedFormatError, "compositing operator"),
            ('<feComposite operator="arithmetic" k4="2000"/>', doc_ex.UnsafeArchiveError, "coefficient"),
            ('<feComposite operator="arithmetic" k1="NaN"/>', doc_ex.MalformedDocumentError, "coefficient"),
            ('<feOffset dx="100000"/>', doc_ex.UnsafeArchiveError, "filter offset"),
            ('<feOffset width="5"/>', doc_ex.UnsupportedFormatError, "primitive subregions"),
        ]
        with doc_ex.open(epub_fixture()) as book:
            for source, error, message in cases:
                with self.subTest(source=source), self.assertRaisesRegex(error, message):
                    book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Effect", svg(source)), 0])
            work = svg("<feMerge>" + '<feMergeNode in="SourceGraphic"/>' * 16 + "</feMerge>",
                       'filterUnits="userSpaceOnUse" width="850" height="850"')
            shift = svg('<feOffset dx="1000"/>', rect='transform="scale(18)"')
            outputs = [
                book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Effect", source), 0])
                for source in (work, shift)
            ]
        for data, reason in zip(outputs, ("filter work", "offset exceeds")):
            with doc_ex.open(data) as book, self.assertRaisesRegex(doc_ex.UnsafeArchiveError, reason):
                book.render_page(0, dpi=96)

    def test_svg_gradient_templates_and_blur_reject_unsafe_inputs(self) -> None:
        def svg(body: str) -> str:
            return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 48">'
                    + body + "</svg>")

        blur = '<defs><filter id="blur"><feGaussianBlur stdDeviation="1"/></filter></defs>'
        layers = "".join(
            '<rect width="64" height="36" filter="url(#blur)"/>'
            for _ in range(20)
        )
        too_deep = "".join(
            f'<linearGradient id="g{i}" href="#g{i + 1}"/>' for i in range(33)
        ) + '<linearGradient id="g33"/>'
        cases = (
            ('<linearGradient href="unsafe.svg#a"/>', doc_ex.UnsupportedFormatError, "same-document"),
            ('<linearGradient href="#missing"/>', doc_ex.MalformedDocumentError, "missing ID"),
            ('<linearGradient id="x"/><radialGradient id="x"/><linearGradient href="#x"/>',
             doc_ex.MalformedDocumentError, "duplicate ID"),
            ('<linearGradient id="x" href="#y"/><radialGradient id="y" href="#x"/>',
             doc_ex.MalformedDocumentError, "cyclic"),
            (too_deep, doc_ex.UnsafeArchiveError, "depth"),
            ('<filter id="x"><feImage href="image.png"/></filter>',
             doc_ex.UnsupportedFormatError, "unsupported or active elements"),
            ('<filter><feGaussianBlur stdDeviation="-5"/></filter>',
             doc_ex.MalformedDocumentError, "deviation"),
            (blur.replace('"1"', '"100000"'), doc_ex.UnsafeArchiveError, "deviation"),
            ('<rect width="10" height="10" filter="url(#missing)"/>',
             doc_ex.MalformedDocumentError, "missing ID"),
            ('<style>rect {filter:url(https://example.invalid/#blur)}</style>'
             '<rect width="10" height="10"/>', doc_ex.UnsupportedFormatError, "same-document"),
            (blur + '<mask id="m"><rect width="10" height="10" filter="url(#blur)"/></mask>',
             doc_ex.UnsupportedFormatError, "inside a pattern, mask or clip path"),
        )
        with doc_ex.open(epub_fixture()) as book:
            for body, error, message in cases:
                with self.subTest(message=message), self.assertRaisesRegex(error, message):
                    book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Broken", svg(body)), 0])

            def archived(body: str) -> bytes:
                return book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Blur", svg(body)), 0])

            huge = archived(blur + '<rect width="8000" height="6000" filter="url(#blur)"/>')
            with doc_ex.open(huge) as large, self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "filter layer"):
                large.render_page(0, dpi=96)

            scaled = archived(
                blur.replace('"1"', '"2"')
                + '<rect width="1" height="1" transform="scale(900)" filter="url(#blur)"/>'
            )
            with (
                doc_ex.open(scaled) as large,
                self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "after scaling"),
            ):
                large.render_page(0, dpi=96)

            repeated = archived(blur + '<g transform="scale(30)">' + layers + '</g>')
            with (
                doc_ex.open(repeated) as large,
                self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "filter work"),
            ):
                large.render_page(0, dpi=96)

    def test_svg_chapter_insertion_rejects_broken_renditions(self) -> None:
        svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><rect width="20" height="10"/></svg>'
        with doc_ex.open(epub_fixture()) as book:
            inputs = (
                (svg, "wrong.xhtml", ValueError, r"end in \.svg"),
                (svg.replace(' viewBox="0 0 20 10"', ''), None, ValueError, "numeric viewBox"),
                (svg.replace("</svg>", "<script/></svg>"), None, doc_ex.UnsupportedFormatError, "active elements"),
                ('<?xml-stylesheet href="absent.css" type="text/css"?>' + svg, None,
                 doc_ex.MalformedDocumentError, r"absent\.css"),
            )
            for markup, path, error, message in inputs:
                with self.subTest(message=message), self.assertRaisesRegex(error, message):
                    book.edit_epub_structure([0, doc_ex.EpubSvgChapterSource("Plate", markup, path)])
            result = book.edit_epub_structure([doc_ex.EpubSvgChapterSource("Plate", svg), 0])
            with doc_ex.open(result) as edited:
                self.assertTrue(edited[0].metadata["chapter_path"].endswith(".svg"))
                self.assertEqual(edited[0].metadata["viewport"], [15, 7.5])
        with doc_ex.open(epub_fixture(version="2.0")) as legacy:
            with self.assertRaisesRegex(ValueError, "requires EPUB 3"):
                legacy.edit_epub_structure([doc_ex.EpubSvgChapterSource("Plate", svg), 0])

    def test_chapter_edits_update_mathml_and_svg_manifest_properties(self) -> None:
        plain = '<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Plain text</p></body></html>'
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="50" height="20"><rect width="50" height="20" fill="blue"/></svg>'
        math = '<m:math xmlns:m="http://www.w3.org/1998/Math/MathML"><m:msup><m:mi>x</m:mi><m:mn>2</m:mn></m:msup></m:math>'
        with_art = plain.replace('</body>', svg + '</body>')
        rich = with_art.replace('</p>', math + '</p>')
        source = epub_fixture()
        with zipfile.ZipFile(io.BytesIO(source)) as archive:
            package = archive.read("OEBPS/content.opf").replace(b'<item id="one" ', b"<item properties='custom:retained' id=\"one\" ")
        source = rewrite_epub(source, {"OEBPS/content.opf": package})
        with doc_ex.open(source) as book:
            replaced = book.edit_epub_chapters({0: rich, -1: with_art})
            self.assertIsNone(next(item for item in book.metadata["manifest"] if item["id"] == "two")["properties"])
        added_path = "OEBPS/new chapter.xhtml"
        with doc_ex.open(replaced) as book:
            added = book.edit_epub_structure([0, 1, doc_ex.EpubChapterSource("Art and math", rich, added_path)])
        with doc_ex.open(added) as book, zipfile.ZipFile(io.BytesIO(added)) as archive:
            props = {item["path"]: item["properties"] for item in book.metadata["manifest"]}
            self.assertEqual(props["OEBPS/one.xhtml"], "custom:retained mathml svg")
            self.assertEqual(props["OEBPS/two.xhtml"], "svg")
            self.assertEqual(props[added_path], "mathml svg")
            self.assertIn(b'<item id="image" href="image.png" media-type="image/png"/>', archive.read("OEBPS/content.opf"))
            self.assertTrue(book.render_page(2, dpi=48).startswith(b"\x89PNG"))
            removed = book.edit_epub_chapters({0: plain, 1: plain})
        with doc_ex.open(removed) as book:
            props = {item["path"]: item["properties"] for item in book.metadata["manifest"]}
            self.assertEqual(props["OEBPS/one.xhtml"], "custom:retained")
            self.assertIsNone(props["OEBPS/two.xhtml"])
            self.assertEqual(props[added_path], "mathml svg")

    def test_epub2_chapter_edit_does_not_add_epub3_manifest_properties(self) -> None:
        source = epub_fixture(version="2.0")
        svg = '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"/>'
        xhtml = '<html xmlns="http://www.w3.org/1999/xhtml"><body>' + svg + '</body></html>'
        with doc_ex.open(source) as book:
            replaced = book.edit_epub_chapters({0: xhtml})
        with zipfile.ZipFile(io.BytesIO(source)) as before, zipfile.ZipFile(io.BytesIO(replaced)) as after:
            self.assertEqual(after.read("OEBPS/content.opf"), before.read("OEBPS/content.opf"))
        with doc_ex.open(replaced) as book:
            added = book.edit_epub_structure([0, doc_ex.EpubChapterSource("Legacy chapter", xhtml)])
        with doc_ex.open(added) as book:
            xhtml_entries = [item for item in book.metadata["manifest"] if item["media_type"] == "application/xhtml+xml"]
            self.assertEqual(len(xhtml_entries), 3)
            self.assertTrue(all(item["properties"] is None for item in xhtml_entries))

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
