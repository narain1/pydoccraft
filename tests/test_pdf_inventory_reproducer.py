"""Fixture correctness, not acceptance criteria for the upstream resource gap."""
import importlib.util
from pathlib import Path
import unittest

import doc_ex


SCRIPT = Path(__file__).resolve().parents[1] / "tools" / "reproduce_pdf_annotation_inventory_limits.py"
spec = importlib.util.spec_from_file_location("pdf_inventory_reproducer", SCRIPT)
reproducer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reproducer)


class InventoryReproducerTests(unittest.TestCase):
    def test_deterministic_fixture_offsets_and_native_identity(self):
        data = reproducer.fixture(repeats=3)
        self.assertEqual(data, reproducer.fixture(repeats=3))
        xref = int(data.rsplit(b"startxref\n", 1)[1].splitlines()[0])
        self.assertTrue(data[xref:].startswith(b"xref\n0 5\n"))
        rows = data[xref:].splitlines()[3:7]
        for number, row in enumerate(rows, 1):
            offset = int(row.split()[0])
            self.assertTrue(data[offset:].startswith(f"{number} 0 obj\n".encode()))
        with doc_ex.open(data) as source, source.edit() as edited:
            self.assertEqual(len(source), 1)
            annotations = list(edited.pages[0].annotations)
            self.assertEqual(len(annotations), 3)
            self.assertEqual([annotation.text for annotation in annotations], ["probe"] * 3)

    def test_fixture_parameter_limits(self):
        for values in ({"colors":100_001}, {"quads":8194}, {"repeats":4}, {"colors":-1}):
            with self.subTest(values=values), self.assertRaises(ValueError):
                reproducer.fixture(**values)
        for value in (True, 1.0, "1"):
            with self.subTest(value=value), self.assertRaises(TypeError):
                reproducer.fixture(colors=value)


if __name__ == "__main__":
    unittest.main()
