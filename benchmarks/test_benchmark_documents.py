"""Harness regressions; run with the separate benchmark requirements installed."""

from datetime import datetime, timezone
import io
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import openpyxl

import benchmark_documents as benchmark


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


if __name__ == "__main__":
    unittest.main()
