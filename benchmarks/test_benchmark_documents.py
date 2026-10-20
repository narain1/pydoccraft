"""Harness regressions; run with the separate benchmark requirements installed."""

from datetime import datetime, timezone
import io
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import openpyxl

import benchmark_documents as benchmark
import benchmark_test_files as local_benchmark


class BenchmarkTests(unittest.TestCase):
    def test_xlsx_fixture_ignores_writer_save_timestamp(self):
        clock = Mock()
        fake_datetime = SimpleNamespace(datetime=clock, timezone=timezone)
        with patch("openpyxl.writer.excel.datetime", fake_datetime):
            clock.now.return_value = datetime(2024, 1, 2, tzinfo=timezone.utc)
            first = benchmark.generate_xlsx(3, 2)
            clock.now.return_value = datetime(2026, 10, 20, tzinfo=timezone.utc)
            second = benchmark.generate_xlsx(3, 2)
        self.assertEqual(first, second)
        workbook = openpyxl.load_workbook(io.BytesIO(first), read_only=True)
        try:
            self.assertEqual(list(workbook["Benchmark"].values), [
                ("R1C1", "R1C2"), ("R2C1", "R2C2"), ("R3C1", "R3C2"),
            ])
            self.assertEqual(workbook.properties.modified, datetime(2020, 1, 1))
        finally:
            workbook.close()

    def test_markdown_output_path_cannot_replace_json(self):
        for suffix in (".md", ".MD"):
            with self.subTest(suffix=suffix), patch("sys.argv", ["benchmark", "--output", "result" + suffix]):
                with patch("sys.stderr", io.StringIO()), self.assertRaises(SystemExit) as error:
                    benchmark.parse_arguments()
                self.assertEqual(error.exception.code, 2)
        with patch("sys.argv", ["benchmark", "--output", "result.json"]):
            self.assertEqual(str(benchmark.parse_arguments().output), "result.json")


class LocalBenchmarkTests(unittest.TestCase):
    def test_cli_rejects_invalid_counts_and_colliding_output_paths(self):
        for option, value in (("--warmups", "-1"), ("--repeats", "0"),
                              ("--dpi", "0"), ("--output", "result.md"),
                              ("--output", "result.MD")):
            with self.subTest(option=option, value=value), patch("sys.argv", ["benchmark", ".", option, value]):
                with patch("sys.stderr", io.StringIO()), self.assertRaises(SystemExit) as error:
                    local_benchmark.parse_arguments()
                self.assertEqual(error.exception.code, 2)

    def test_conversion_guard_ignores_serialization_length_variation(self):
        data = benchmark.generate_pdf(1, 1)
        with benchmark.pymupdf.open(stream=data, filetype="pdf") as document:
            first = document.tobytes()
            document.set_metadata({"title": "A different metadata length"})
            second = document.tobytes()
        self.assertNotEqual(len(first), len(second))
        self.assertEqual(local_benchmark.summarize_conversion((first, "pdf")),
                         local_benchmark.summarize_conversion((second, "pdf")))

    def test_cli_errors_omit_document_paths_and_extracted_details(self):
        secret = "/private/CustomerName/secret-document.pdf: confidential text"
        with patch("sys.argv", ["benchmark", "."]), patch.object(local_benchmark, "run", side_effect=RuntimeError(secret)):
            with self.assertRaises(SystemExit) as error:
                local_benchmark.main()
        self.assertEqual(str(error.exception), "local benchmark failed (RuntimeError); document details omitted")
        self.assertTrue(error.exception.__suppress_context__)


if __name__ == "__main__":
    unittest.main()
