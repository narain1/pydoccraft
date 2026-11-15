import runpy
import unittest
from pathlib import Path


class PythonApiInventoryTests(unittest.TestCase):
    def test_inventory_tracks_public_api_without_importing_extension(self):
        root = Path(__file__).resolve().parents[1]
        inventory = runpy.run_path(str(root / "tools" / "python_api_inventory.py"))["inventory"]()
        modules = inventory["modules"]
        self.assertIn("open", modules["__init__"]["exports"])
        self.assertIn("Document", {item["name"] for item in modules["_document"]["classes"]})
        self.assertIn("EditableDocument", {item["name"] for item in modules["_authoring"]["classes"]})
        self.assertTrue(any(item["native_refs"] for item in modules["__init__"]["functions"]))


if __name__ == "__main__":
    unittest.main()
