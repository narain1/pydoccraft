"""Native PDF text geometry: bounded search, owned results, and live-owner checks."""
import math
import unittest

import doc_ex
from doc_ex._document import Document, Page
from doc_ex.errors import DocumentClosedError, UnsupportedFormatError, UnsafeArchiveError
from test_pdf_text_boxes import FONT
from test_render_geometry import rgba_pixels
from test_rendering import png_chunks


def search_pdf(content=b"BT /F1 20 Tf 1 0 0 1 60 260 Tm (oWce) Tj ET", *, rotation=0, ligature=True):
    """Original fixture: one painted W optionally maps to ffi; no third-party asset."""
    mapping = b"1 beginbfchar <57> <006600660069> endbfchar" if ligature else b""
    cmap = (b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap "
            b"/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def "
            b"/CMapName /SearchFixture def /CMapType 2 def 1 begincodespacerange "
            b"<00> <FF> endcodespacerange " + mapping +
            b" endcmap CMapName currentdict /CMap defineresource pop end end")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        (f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /CropBox [20 30 220 330] "
         f"/Rotate {rotation} /UserUnit 1.25 /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>").encode(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>",
        f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream",
        f"<< /Length {len(cmap)} >>\nstream\n".encode() + cmap + b"\nendstream",
    ]
    result = bytearray(b"%PDF-1.7\n")
    offsets = []
    for number, obj in enumerate(objects, 1):
        offsets.append(len(result))
        result.extend(f"{number} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(result)
    result.extend(f"xref\n0 {len(objects)+1}\n0000000000 65535 f \n".encode())
    for offset in offsets:
        result.extend(f"{offset:010d} 00000 n \n".encode())
    result.extend(f"trailer\n<< /Size {len(objects)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
    return bytes(result)


class PdfSearchTests(unittest.TestCase):
    def test_ligature_rectangles_rotation_crop_userunit_and_visible_overlap(self):
        expected = ((63.9, 67.5, 87.5, 92.5), (282.5, 63.9, 307.5, 87.5),
                    (162.5, 282.5, 186.1, 307.5), (67.5, 162.5, 92.5, 186.1))
        for rotation, want in zip((0, 90, 180, 270), expected):
            with self.subTest(rotation=rotation), doc_ex.open(search_pdf(rotation=rotation)) as document:
                page = document[0]
                before = page.get_text("json")
                boxes = page.search_for("FFI")
                self.assertEqual(len(boxes), 1)
                self.assertIsInstance(boxes[0], tuple)
                for actual, target in zip(boxes[0], want):
                    self.assertAlmostEqual(actual, target, places=3)
                self.assertEqual(page.search_for("f"), boxes * 2)
                self.assertEqual(page.get_text("json"), before)
                png = page.get_pixmap(dpi=144)
                with doc_ex.open(search_pdf(rotation=rotation, ligature=False)) as control:
                    self.assertEqual(control[0].get_pixmap(dpi=144), png)
                width, rgba = rgba_pixels(png_chunks(png))
                x0, y0, x1, y1 = boxes[0]
                ink = sum(rgba[(y*width+x)*4] < 100
                          for y in range(math.floor(y0*2), math.ceil(y1*2))
                          for x in range(math.floor(x0*2), math.ceil(x1*2)))
                self.assertGreater(ink, 20)
            self.assertEqual(len(boxes), 1)  # owned tuples remain available after close

    def test_multiline_normalized_whitespace_and_literal_matching(self):
        content = b"BT /F1 16 Tf 1 0 0 1 60 260 Tm (Quick  brown) Tj 0 -24 Td (fox) Tj ET"
        for rotation in (0, 90, 180, 270):
            with doc_ex.open(search_pdf(content, rotation=rotation)) as document:
                page = document[0]
                original = page.get_text()
                self.assertEqual(len(page.search_for(" QUICK\t brown\nfox ")), 2)
                self.assertEqual(page.search_for("brown.*fox"), [])
                self.assertEqual(page.get_text(), original)
        with doc_ex.new("pdf") as document:
            page = document.pages[0]
            page.draw_text("Café Ω Σσς İıI Straße", at=(20, 30), font=FONT)
            self.assertEqual(len(page.search_for("CAFÉ")), 1)
            self.assertEqual(len(page.search_for("σσσ iii straße")), 1)
            self.assertEqual(page.search_for("STRASSE"), [])

    def test_actualtext_replacement_and_native_column_order_are_preserved(self):
        marked = (b"BT /F1 20 Tf 1 0 0 1 60 260 Tm "
                  b"/Span <</ActualText <FEFF00630061006600E9>>> BDC (xy) Tj EMC ET")
        control = b"BT /F1 20 Tf 1 0 0 1 60 260 Tm (xy) Tj ET"
        with doc_ex.open(search_pdf(marked, ligature=False)) as document, \
                doc_ex.open(search_pdf(control, ligature=False)) as painted:
            page = document[0]
            self.assertEqual(page.get_text(), "café")
            self.assertEqual(page.search_for("café"), page.search_for("é"))
            self.assertEqual(page.search_for("café"), painted[0].search_for("xy"))
            self.assertEqual(page.get_pixmap(), painted[0].get_pixmap())
        columns = (b"BT /F1 12 Tf 1 0 0 1 160 240 Tm (R2) Tj "
                   b"1 0 0 1 60 260 Tm (L1) Tj 1 0 0 1 160 260 Tm (R1) Tj "
                   b"1 0 0 1 60 240 Tm (L2) Tj ET")
        with doc_ex.open(search_pdf(columns, ligature=False)) as document:
            page = document[0]
            before = page.get_text("json")
            self.assertEqual(page.get_text(), "L1\tR1\nL2\tR2")
            self.assertEqual(page.search_for("L1 L2"), page.search_for("L1") + page.search_for("L2"))
            # Native text order crosses from the end of one column to the next.
            # The normalized table's row-major text is a different representation.
            self.assertEqual(page.search_for("L2 R1"), page.search_for("L2") + page.search_for("R1"))
            self.assertEqual(page.get_text("json"), before)

    def test_negative_text_scale_keeps_ordered_native_rectangles(self):
        for rotation in (0, 90, 180, 270):
            for matrix in ("-1 0 0 1 160 260", "1 0 0 -1 60 260"):
                content = f"BT /F1 20 Tf {matrix} Tm (Mirror) Tj ET".encode()
                with doc_ex.open(search_pdf(content, rotation=rotation, ligature=False)) as document:
                    boxes = document[0].search_for("mirror")
                    self.assertEqual(len(boxes), 1)
                    x0, y0, x1, y1 = boxes[0]
                    self.assertLess(x0, x1)
                    self.assertLess(y0, y1)
                    self.assertTrue(all(math.isfinite(value) for value in boxes[0]))

    def test_clipped_and_invisible_text_is_searchable_without_visible_ink(self):
        cases = (
            b"q 0 0 1 1 re W n BT /F1 20 Tf 1 0 0 1 60 260 Tm (Hidden) Tj ET Q",
            b"BT /F1 20 Tf 3 Tr 1 0 0 1 60 260 Tm (Hidden) Tj ET",
        )
        with doc_ex.open(search_pdf(b"")) as blank:
            blank_png = blank[0].get_pixmap()
        for content in cases:
            with doc_ex.open(search_pdf(content)) as document:
                self.assertEqual(len(document[0].search_for("hidden")), 1)
                self.assertEqual(document[0].get_pixmap(), blank_png)

    def test_query_limits_empty_and_session_statistics(self):
        with doc_ex.open(search_pdf()) as document:
            page = document[0]
            native = document._render_session
            before = native._stats()
            for empty in ("", " \t\r\n\v\f", "\u2003\u00a0"):
                self.assertEqual(page.search_for(empty), [])
            self.assertEqual(native._stats(), before)
            for text in ("a"*4097, "α"*2049):
                with self.assertRaises(UnsafeArchiveError):
                    page.search_for(text)
            with self.assertRaises(UnsafeArchiveError):
                doc_ex._native(lambda: native.search_for(0, "a"*4097))
            for value in (None, 123, b"office", True):
                with self.assertRaises(TypeError):
                    page.search_for(value)
            with self.assertRaises(ValueError):
                page.search_for("\ud800")
            self.assertEqual(native._stats(), before)
            for text in ("a"*4096, "α"*2048, "office", "OFFICE"):
                page.search_for(text)
            after = native._stats()
            self.assertEqual(after, (before[0], before[1]+4, before[2]))
            with self.assertRaises(IndexError):
                doc_ex._native(lambda: native.search_for(1, ""))
        with self.assertRaises(DocumentClosedError):
            doc_ex._native(lambda: native.search_for(0, ""))

    def test_snapshots_cache_mutation_copy_stale_and_failed_queries(self):
        with doc_ex.new("pdf") as document:
            page = document.pages[0]
            page.draw_text("First", at=(20, 30))
            self.assertIsNone(document._cached)
            self.assertEqual(page.search_for(" \t\n"), [])
            self.assertIsNone(document._cached)
            with self.assertRaises(UnsafeArchiveError):
                page.search_for("x"*4097)
            self.assertIsNone(document._cached)
            boxes = page.search_for("first")
            cached = document._cached
            before = document.to_bytes()
            self.assertEqual(page.search_for("FIRST"), boxes)
            self.assertIs(document._cached, cached)
            self.assertEqual(document.to_bytes(), before)
            with document.copy() as copied:
                self.assertEqual(copied.pages[0].search_for("First"), boxes)
            page.draw_text("Second", at=(20, 70))
            with self.assertRaises(DocumentClosedError):
                cached.render_page(0)
            self.assertEqual(len(page.search_for("second")), 1)
            document.pages.add()
            document.pages.move(0, 1)
            with self.assertRaisesRegex(ValueError, "stale"):
                page.search_for(123)
            self.assertEqual(document.pages[1].search_for("first"), boxes)
        self.assertTrue(all(isinstance(box, tuple) for box in boxes))
        with self.assertRaises(DocumentClosedError):
            page.search_for(None)

    def test_string_subclasses_cannot_run_callbacks_or_conceal_size(self):
        calls = []
        class Query(str):
            def __str__(self):
                calls.append("str"); raise AssertionError("callback")
            def __len__(self):
                calls.append("len"); return 0
            def encode(self, *args, **kwargs):
                calls.append("encode"); raise AssertionError("callback")
            def strip(self, *args):
                calls.append("strip"); raise AssertionError("callback")
        with doc_ex.open(search_pdf()) as document:
            self.assertEqual(len(document[0].search_for(Query("office"))), 1)
            with self.assertRaises(UnsafeArchiveError):
                document[0].search_for(Query("a"*4097))
        with self.assertRaises(DocumentClosedError):
            document[0].search_for(Query("office"))
        self.assertEqual(calls, [])
        with doc_ex.new("pdf") as document:
            page = document.pages[0]
            page.draw_text("office", at=(20, 30))
            self.assertEqual(len(page.search_for(Query("office"))), 1)
            self.assertEqual(calls, [])

    def test_page_index_callbacks_recheck_closed_and_ownership(self):
        document = doc_ex.open(search_pdf())
        page = document[0]
        class CloseIndex:
            def __index__(self):
                document.close(); return 0
        page.number = CloseIndex()
        with self.assertRaises(DocumentClosedError):
            page.search_for("office")
        with doc_ex.open(search_pdf()) as first, doc_ex.open(search_pdf()) as second:
            page = first[0]
            class ReattachIndex:
                def __index__(self):
                    page._document = second; return 0
            page.number = ReattachIndex()
            with self.assertRaisesRegex(ValueError, "ownership changed"):
                page.search_for("office")
        with self.assertRaisesRegex(ValueError, "not attached"):
            Page(0).search_for(123)

    def test_office_unsupported_even_empty_and_manually_attached_pdf(self):
        for kind in ("docx", "pptx"):
            with doc_ex.new(kind) as authored:
                if kind == "pptx":
                    authored.slides.add()
                with doc_ex.open(authored.to_bytes()) as read:
                    for value in ("", "hello"):
                        with self.assertRaises(UnsupportedFormatError):
                            read[0].search_for(value)
                    page = read[0]
                with self.assertRaises(DocumentClosedError):
                    page.search_for(123)
        # Document's public constructor has no retained native session; its fallback
        # closes the temporary search session and leaves document state unchanged.
        source = search_pdf()
        with Document([Page(0)], {}, source, "pdf", None) as document:
            self.assertEqual(len(document[0].search_for("office")), 1)
            self.assertIsNone(document._render_session)


if __name__ == "__main__":
    unittest.main()
