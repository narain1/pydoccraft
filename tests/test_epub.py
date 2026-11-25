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
        with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "mixed variable fonts"):
            preview(first + second, length=explicit)
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
          <source src="teaser.ogg" type="audio/ogg; codecs=vorbis"/>
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
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "composed geometry"):
                book.convert("docx")

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
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "mixed with other paragraph content"):
                book.convert("docx")
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
            with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, "composed geometry"):
                book.convert("docx")

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
            ('width="+8pt" voffset="3pt"', "composed geometry"),
            ('width="+8pt" lspace="-2pt"', "composed geometry"),
            ('width="+8pt" lspace="9pt"', "composed geometry"),
            ('width="+0.1pt"', "spacing resolution"),
            ('width="600pt"', "exceeds the DOCX text area"),
        ):
            chapter = template.format(f"<m:mpadded {attributes}>{red}</m:mpadded>{blue}")
            with self.subTest(attributes=attributes), doc_ex.open(epub_fixture(chapter_one=chapter)) as book:
                with self.assertRaisesRegex(doc_ex.UnsupportedFormatError, message):
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
