from concurrent.futures import ThreadPoolExecutor
from threading import Event
import unittest

import doc_ex
from doc_ex import _core, _native
from doc_ex.errors import DocumentClosedError, UnsafeArchiveError
from fixtures import pdf_fixture
from test_render_geometry import geometry_pdf


class PdfRenderSessionTests(unittest.TestCase):
    def test_reused_full_and_clipped_rasters_match_one_shot_at_fractional_dpi(self):
        for rotation in (0, 90, 180, 270):
            source = geometry_pdf(rotation=rotation)
            with self.subTest(rotation=rotation), doc_ex.open(source) as document:
                session = document._render_session
                initial = session._stats()
                for dpi in (72, 150.5):
                    for clip in (None, (0.25, 0.5, 40.25, 30.5), (-4, -5, 12, 15)):
                        expected = bytes(_native(lambda: _core.render(source, 'pdf', 0, dpi, None, clip)))
                        self.assertEqual(document[-1].get_pixmap(dpi, clip=clip), expected)
                self.assertEqual(session._stats()[0], 1)
                self.assertEqual(session._stats()[2] - initial[2], 6)

    def test_private_session_retains_native_limits(self):
        with doc_ex.open(pdf_fixture()) as document:
            session = document._render_session
            before = session._stats()
            for dpi in (0, float('nan'), 1201):
                with self.assertRaises(UnsafeArchiveError):
                    _native(lambda: session.render(0, dpi))
            for clip in ((0, 0, 0, 1), (float('inf'), 0, 1, 1), (1e8, 0, 1, 1)):
                with self.assertRaises(ValueError):
                    _native(lambda: session.render(0, 72, clip))
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                _native(lambda: session.render(document.page_count, 72))
            self.assertEqual(session._stats(), before)

    def test_shared_session_and_close_have_stable_thread_lifetimes(self):
        document = doc_ex.open(pdf_fixture())
        page = document[0]
        expected = document.render_page(0, 72, clip=(0, 0, 30, 30))
        session = document._render_session
        with ThreadPoolExecutor(max_workers=4) as workers:
            outputs = list(workers.map(lambda _: bytes(session.render(0, 72, (0, 0, 30, 30))), range(8)))
            self.assertEqual(outputs, [expected] * 8)
            self.assertEqual(session._stats()[0], 1)
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
        document.close()
        self.assertIsNone(document._render_session)
        with self.assertRaises(DocumentClosedError):
            _native(session._stats)
        with self.assertRaises(DocumentClosedError):
            _native(lambda: session.render(0, 72, (0, 0, 30, 30)))
        self.assertIn('Page one', page.get_text())
        with self.assertRaises(DocumentClosedError):
            page.get_pixmap(72, clip=(0, 0, 30, 30))


if __name__ == '__main__':
    unittest.main()
