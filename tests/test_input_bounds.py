from array import array
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import doc_ex
from fixtures import pdf_fixture


class InputBoundsTests(unittest.TestCase):
    def test_binary_streams_handle_short_reads_and_remain_open(self):
        class ShortReads(io.BytesIO):
            def read(self, size=-1):
                return super().read(min(size, 7))
        stream = ShortReads(pdf_fixture())
        with doc_ex.open(stream) as document:
            self.assertEqual(document.page_count, 2)
        self.assertFalse(stream.closed)
        with self.assertRaises(TypeError):
            doc_ex.open(io.StringIO("%PDF-1.4"))

    def test_input_limit_covers_bytes_streams_paths_and_memoryview_byte_size(self):
        with patch("doc_ex._MAX_INPUT_BYTES", 8):
            for source in (b"x" * 9, bytearray(9), memoryview(array("I", range(3))), io.BytesIO(b"x" * 9)):
                with self.assertRaisesRegex(doc_ex.UnsafeArchiveError, "exceeds 8"):
                    doc_ex._read(source)
            self.assertEqual(doc_ex._read(io.BytesIO(b"12345678")), (b"12345678", ""))
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "document.pdf"
                path.write_bytes(b"x" * 9)
                with self.assertRaises(doc_ex.UnsafeArchiveError):
                    doc_ex._read(path)
