"""Benchmark guard tests do not install or execute the comparison backend."""
import importlib.util
from pathlib import Path
import struct
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


path = Path(__file__).resolve().parents[1] / "benchmarks" / "benchmark_pdf_vs_pymupdf.py"
spec = importlib.util.spec_from_file_location("_pdf_benchmark_under_test", path)
benchmark = importlib.util.module_from_spec(spec)
with patch.dict(sys.modules, {spec.name: benchmark, "pymupdf": SimpleNamespace()}):
    spec.loader.exec_module(benchmark)


class PdfBenchmarkTests(unittest.TestCase):
    def test_output_preflight_preserves_source_files_including_aliases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.PDF"
            source.write_bytes(b"original input")
            for name, link in (("symbolic.json", "symlink_to"), ("hard.json", "hardlink_to")):
                alias = root / name
                getattr(alias, link)(source)
                with self.subTest(alias=name), self.assertRaises(ValueError):
                    benchmark.input_paths(root, alias)
            with self.assertRaises(ValueError):
                benchmark.run(root, 0, 1, 72, source)
            self.assertEqual(source.read_bytes(), b"original input")
            self.assertEqual(benchmark.input_paths(root, root / "result.json"), [source])

    def test_blank_text_retains_expected_pages_and_missing_pages_fail(self):
        self.assertEqual(benchmark.text_summary("", 1), (1, 0))
        self.assertEqual(benchmark.text_summary("\f\f", 3), (3, 2))
        with self.assertRaises(RuntimeError):
            benchmark.text_summary("one page", 2)

    def test_render_dimensions_must_be_nonzero_and_match_the_comparison(self):
        header = b"\x89PNG\r\n\x1a\n" + b"\0" * 8
        png = header + struct.pack(">II", 200, 100)
        self.assertEqual(benchmark.png_summary(png, (200, 100)), (200, 100))
        for data, expected in ((png, (100, 200)), (header + struct.pack(">II", 0, 100), None)):
            with self.subTest(expected=expected), self.assertRaises(RuntimeError):
                benchmark.png_summary(data, expected)

    def test_cli_failures_do_not_publish_input_paths_or_document_text(self):
        with patch("sys.argv", ["benchmark", "."]), patch.object(benchmark, "run", side_effect=RuntimeError("/private/secret.pdf: hidden text")):
            with self.assertRaises(SystemExit) as error:
                benchmark.main()
        self.assertEqual(str(error.exception), "PDF benchmark failed (RuntimeError); document details omitted")
        self.assertTrue(error.exception.__suppress_context__)


if __name__ == "__main__":
    unittest.main()
