from io import BytesIO
from pathlib import Path
from tempfile import TemporaryDirectory
import struct
import unittest
from zipfile import ZipFile
import zlib

import doc_ex
from doc_ex.errors import DocumentClosedError
from test_authoring import image_fixture
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def media_parts(document):
    with ZipFile(BytesIO(document.to_bytes())) as archive:
        return [archive.read(name) for name in archive.namelist() if name.startswith("word/media/")]


def red_picture(width, height):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    raw = (b"\x00" + b"\xff\x00\x00\xff" * width) * height
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def count_color(png, color):
    _, pixels = rgba_pixels(png_chunks(png))
    return sum(pixels[index:index + 4] == color for index in range(0, len(pixels), 4))


class PictureMediaTests(unittest.TestCase):
    def test_payload_replacement_accepts_paths_streams_and_preserves_placement(self):
        original = image_fixture(32, 16)
        replacement = red_picture(16, 32)
        with TemporaryDirectory() as directory, doc_ex.new("docx") as document:
            source = Path(directory) / "source.png"
            source.write_bytes(original)
            before = document.add_paragraph("Before picture")
            picture = document.add_image(source, width=64, alt="Original alt")
            after = document.add_paragraph("After picture")
            self.assertEqual(picture.size, (64, 32))
            original_render = document.render_page(0, dpi=72)
            self.assertGreater(count_color(original_render, b"\x20\x60\xc0\xff"), 100)
            self.assertEqual(count_color(original_render, b"\xff\x00\x00\xff"), 0)
            with document.copy() as copied:
                picture.data = BytesIO(replacement)
                replaced_render = document.render_page(0, dpi=72)
                self.assertNotEqual(replaced_render, original_render)
                self.assertGreater(count_color(replaced_render, b"\xff\x00\x00\xff"), 100)
                self.assertEqual(count_color(replaced_render, b"\x20\x60\xc0\xff"), 0)
                self.assertEqual(copied.render_page(0, dpi=72), original_render)
                self.assertEqual(picture.data, replacement)
                self.assertEqual(copied.images[0].data, original)
                self.assertEqual((picture.size, picture.alt), ((64, 32), "Original alt"))
                self.assertEqual((before.text, after.text), ("Before picture", "After picture"))
                self.assertEqual(media_parts(document), [replacement])
                self.assertEqual(media_parts(copied), [original])
                for data in (document.to_bytes(), document.convert("pdf")):
                    with doc_ex.open(data) as reopened:
                        self.assertGreater(count_color(reopened.render_page(0, dpi=72),
                                                       b"\xff\x00\x00\xff"), 100)
                        text = reopened.to_markdown()
                        self.assertLess(text.index("Before picture"), text.index("After picture"))
                picture.data = source
                self.assertEqual(picture.data, original)
                raw = document.to_bytes()
                for invalid in (b"invalid PNG", replacement[:-8]):
                    with self.assertRaises((ValueError, doc_ex.DocExError)):
                        picture.data = invalid
                    self.assertEqual(document.to_bytes(), raw)
                document.close()
                with self.assertRaises(DocumentClosedError):
                    _ = picture.data
                self.assertEqual(copied.images[0].data, original)

    def test_shared_media_and_cell_grid_deletion_reclaim_only_unused_payloads(self):
        first = image_fixture(8, 4)
        second = image_fixture(4, 8)
        third = image_fixture(6, 6)
        with doc_ex.new("docx") as document:
            body = document.add_image(first, width=30, alt="Body")
            table = document.add_table([["Keep"], ["Drop"]])
            cell = table[0, 0].add_image(first, width=30, alt="Cell")
            self.assertEqual(media_parts(document), [first])
            body.data = second
            self.assertCountEqual(media_parts(document), [first, second])
            self.assertEqual(cell.data, first)
            cell.data = second
            self.assertEqual(media_parts(document), [second])
            snapshot = body.data
            del document.images[0]
            with self.assertRaises((ValueError, IndexError)):
                _ = body.data
            self.assertEqual(snapshot, second)
            self.assertEqual(cell.data, second)
            table[0, 0].text = "Replaced cell"
            self.assertEqual(media_parts(document), [])
            with self.assertRaises((ValueError, IndexError)):
                _ = cell.data
            survivor = document.add_image(second, width=30)
            dropped = table[1, 0].add_image(first, width=30)
            table.delete_row(1)
            with self.assertRaises((ValueError, IndexError)):
                _ = dropped.data
            fresh = document.add_image(third, width=30)
            self.assertEqual(survivor.data, second)
            self.assertEqual(fresh.data, third)
            self.assertCountEqual(media_parts(document), [second, third])

    def test_cell_picture_deletion_preserves_anchor_and_other_handles(self):
        with doc_ex.new("docx") as document:
            outside = document.add_paragraph("Outside")
            table = document.add_table([[""]])
            picture = table[0, 0].add_image(image_fixture(), width=32, alt="Delete me")
            following = table[0, 0].add_paragraph("Following")
            del table[0, 0].images[-1]
            self.assertEqual(len(table[0, 0].images), 0)
            self.assertEqual(outside.text, "Outside")
            self.assertEqual(following.text, "Following")
            self.assertEqual(table[0, 0].paragraphs[0].text, "")
            table[0, 0].paragraphs[0].text = "Preserved anchor"
            with self.assertRaises((ValueError, IndexError)):
                picture.alt = "Stale"
            self.assertEqual(media_parts(document), [])
            with doc_ex.open(document.convert("pdf")) as pdf:
                text = pdf.to_markdown()
                for label in ("Outside", "Preserved anchor", "Following"):
                    self.assertEqual(text.count(label), 1)


if __name__ == "__main__":
    unittest.main()
