from concurrent.futures import ThreadPoolExecutor
import json
from threading import Event
import unittest
from unittest.mock import patch

import doc_ex
from doc_ex import _core, _native
from doc_ex.errors import DocumentClosedError, UnsafeArchiveError
from fixtures import docx_fixture, pptx_fixture, pdf_fixture, PNG


def multipage_office(kind):
    with doc_ex.new(kind, size=(240, 180)) as document:
        if kind == "docx":
            document.sections[0].margins = 16
            document.add_paragraph("First red page")[:].format(color="red")
            document.add_table([["A", "B"], ["1", "2"]])
            document.add_page_break()
            document.add_paragraph("Second blue page")[:].format(color="blue")
        else:
            for color in ("red", "blue"):
                slide = document.slides.add()
                slide.add_shape("rectangle", box=(10, 20, 50, 40), fill=color)
                slide.add_text(color, box=(10, 70, 190, 40))
        return document.to_bytes()


class OfficeRenderSessionTests(unittest.TestCase):
    def test_mixed_pages_and_regions_match_one_shot_without_reparse(self):
        for kind in ("docx", "pptx"):
            source = multipage_office(kind)
            with self.subTest(kind=kind), doc_ex.open(source) as document:
                self.assertEqual(document.page_count, 2)
                session = document._render_session
                self.assertEqual(session._stats(), (1, int(kind == "docx"), 0))
                requests = 0
                for dpi in (72, 144, 300):
                    for page in (1, 0, 1):
                        for clip in (None, (0.25, 0.5, 60.5, 50.25), (-3, -4, 20, 20)):
                            expected = bytes(_native(lambda: _core.render(source, kind, page, dpi, None, clip)))
                            self.assertEqual(document[page].get_pixmap(dpi, clip=clip), expected)
                            requests += 1
                self.assertEqual(session._stats(), (1, int(kind == "docx"), requests))
                with doc_ex.open(document.convert("pdf")) as converted:
                    self.assertEqual(converted.page_count, 2)

    def test_owned_extraction_values_survive_source_and_session_close(self):
        for fixture in (docx_fixture, pptx_fixture):
            source = bytearray(fixture())
            document = doc_ex.open(source, password="unused")
            source[:] = b"discarded caller bytes"
            page = document[0]
            text, tables, images = page.get_text(), page.find_tables(), page.get_images()
            blocks = list(page.blocks)
            self.assertTrue(tables)
            self.assertTrue(images)
            session = document._render_session
            self.assertTrue(page.get_pixmap(72, clip=(0, 0, 20, 20)))
            document.close()
            document.close()
            self.assertEqual(document._source, b"")
            self.assertIsNone(document._password)
            self.assertEqual(page.get_text(), text)
            self.assertEqual(page.find_tables(), tables)
            self.assertEqual(images[0].data, PNG)
            self.assertEqual(page.blocks, blocks)
            with self.assertRaises(DocumentClosedError):
                _native(session._stats)
            with self.assertRaises(DocumentClosedError):
                page.get_pixmap(72)
            with self.assertRaises(DocumentClosedError):
                document.convert("pdf")

    def test_limits_and_direct_native_close_keep_stable_errors(self):
        for kind, fixture in (("docx", docx_fixture), ("pptx", pptx_fixture)):
            source = fixture()
            with self.subTest(kind=kind), doc_ex.open(source) as document:
                session = document._render_session
                for dpi in (0, float("nan"), 1201):
                    with self.assertRaises(UnsafeArchiveError):
                        _native(lambda: session.render(0, dpi))
                with self.assertRaises(UnsafeArchiveError):
                    document.render_page(0, 1200)
                self.assertTrue(document.render_page(0, 1200, clip=(0, 0, 1, 1)))
                for clip in ((0, 0, 0, 1), (float("inf"), 0, 1, 1), (1e8, 0, 1, 1)):
                    with self.assertRaises(ValueError):
                        _native(lambda: session.render(0, 72, clip))
                with self.assertRaises(doc_ex.UnsupportedFormatError):
                    _native(lambda: session.render(document.page_count, 72))
                _native(session.close)
                with self.assertRaises(DocumentClosedError):
                    document.render_page(0, 72)
            # Session projection is identical to the existing one-shot native boundary.
            session, payload = _native(lambda: _core._open_office(source, kind))
            try:
                self.assertEqual(json.loads(payload), json.loads(_native(lambda: _core.extract(source, kind))))
            finally:
                session.close()

    def test_shared_session_and_close_have_stable_thread_lifetimes(self):
        for fixture in (docx_fixture, pptx_fixture):
            document = doc_ex.open(fixture())
            page = document[0]
            expected = page.get_pixmap(72, clip=(0, 0, 30, 30))
            session = document._render_session
            with ThreadPoolExecutor(max_workers=4) as workers:
                outputs = list(workers.map(lambda _: bytes(session.render(0, 72, (0, 0, 30, 30))), range(8)))
                self.assertEqual(outputs, [expected] * 8)
                gate = Event()
                def render_after_gate(_):
                    gate.wait()
                    try:
                        return page.get_pixmap(72, clip=(0, 0, 30, 30))
                    except DocumentClosedError:
                        return None
                futures = [workers.submit(render_after_gate, n) for n in range(8)]
                gate.set()
                document.close()
                self.assertTrue(all(future.result() in (None, expected) for future in futures))
            self.assertIsNone(document._render_session)
            with self.assertRaises(DocumentClosedError):
                _native(lambda: session.render(0, 72))

    def test_projection_failure_closes_unreturned_native_state(self):
        for fixture, name in ((docx_fixture, "_open_office"), (pptx_fixture, "_open_office"), (pdf_fixture, "_open_pdf")):
            real_open = getattr(_core, name)
            retained = []
            def capture(*args):
                session, payload = real_open(*args)
                retained.append(session)
                return session, payload
            with patch.object(_core, name, side_effect=capture), patch.object(doc_ex, "_page", side_effect=RuntimeError("projection failed")):
                with self.assertRaisesRegex(RuntimeError, "projection failed"):
                    doc_ex.open(fixture())
            self.assertEqual(len(retained), 1)
            with self.assertRaises(DocumentClosedError):
                _native(retained[0]._stats)


if __name__ == "__main__":
    unittest.main()
