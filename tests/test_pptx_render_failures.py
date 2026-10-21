from __future__ import annotations

import struct
import unittest

import doc_ex
from test_rendering import has_visible_content, png_chunks


class PptxRenderFailureTests(unittest.TestCase):
    def test_native_rendering_failure_maps_to_existing_public_error(self):
        # Boundary mapping only: actual native Busy/error recovery is exercised in Rust.
        for category in ('rendering_failure', 'conversion_failure'):
            message = 'PPTX operation failed: native renderer is already in use'

            def fail():
                raise doc_ex._core.DocExNativeError(f'{category}:{message}')

            with self.subTest(category=category), self.assertRaises(doc_ex.DocExError) as caught:
                doc_ex._native(fail)
            self.assertIs(type(caught.exception), doc_ex.DocExError)
            self.assertEqual(str(caught.exception), message)
            self.assertTrue(caught.exception.__suppress_context__)

    def test_blank_pptx_remains_successful_through_owned_imported_and_pdf_paths(self):
        with doc_ex.new('pptx', size=(240, 160)) as authored:
            authored.slides.add(layout='blank')
            before = authored.render_page(0, dpi=72)
            self.assertEqual(struct.unpack('>II', png_chunks(before)[b'IHDR'][:8]), (240, 160))
            self.assertFalse(has_visible_content(png_chunks(before)))
            source = authored.to_bytes()
            with doc_ex.open(source) as imported:
                session = imported._render_session
                self.assertIsNotNone(session)
                self.assertEqual(session._stats(), (1, 0, 0))
                for dpi in (72, 144):
                    png = imported.render_page(0, dpi=dpi)
                    chunks = png_chunks(png)
                    self.assertEqual(struct.unpack('>II', chunks[b'IHDR'][:8]), (240 * dpi // 72, 160 * dpi // 72))
                    self.assertFalse(has_visible_content(chunks))
                self.assertEqual(session._stats(), (1, 0, 2))
                self.assertEqual(imported.render_page(0, dpi=72), before)
                clipped = imported.render_page(0, dpi=144, clip=(10, 20, 30, 40))
                self.assertEqual(struct.unpack('>II', png_chunks(clipped)[b'IHDR'][:8]), (60, 80))
                with doc_ex.open(imported.convert('pdf')) as pdf:
                    self.assertEqual(len(pdf), 1)
                    self.assertEqual(pdf[0].get_text(), '')
                    self.assertFalse(has_visible_content(png_chunks(pdf.render_page(0, dpi=72))))
            with self.assertRaises(doc_ex.DocumentClosedError):
                imported.render_page(0, dpi=72)
            with doc_ex.open(authored.convert('pdf')) as pdf:
                self.assertEqual(len(pdf), 1)
                self.assertTrue(pdf.render_page(0, dpi=72).startswith(b'\x89PNG\r\n\x1a\n'))
