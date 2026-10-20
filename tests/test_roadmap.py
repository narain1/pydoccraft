"""Development-ledger checks; no document engine or network required."""

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_roadmap", ROOT / "tools/check_roadmap.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class RoadmapTests(unittest.TestCase):
    def setUp(self):
        self.todo = "- [ ] **A1: First.**\n- [x] **B1: Second.**\n"
        self.upstream = "| U01 | Example owner |\n"
        self.ledger = {
            "schema_version": 1,
            "source": "TODO.md",
            "workstreams": [
                {"id": "W01", "priority": "P0", "kind": "implementation",
                 "depends_on": [], "coverage": ["A1"],
                 "feature_roles": {"A1": "implementation"}, "upstream_candidates": ["U01"]},
                {"id": "W02", "priority": "P1", "kind": "implementation",
                 "depends_on": ["W01"], "coverage": ["B1"],
                 "feature_roles": {"B1": "existing"}, "upstream_candidates": []},
            ],
        }

    def check_errors(self, *expected):
        errors = checker.validate(self.todo, self.ledger, self.upstream)
        for substring in expected:
            self.assertTrue(any(substring in error for error in errors), (substring, errors))

    def test_repository_ledger_is_consistent(self):
        ledger = json.loads((ROOT / "docs/roadmap-workstreams.json").read_text())
        self.assertEqual([], checker.validate(
            (ROOT / "TODO.md").read_text(), ledger,
            (ROOT / "docs/upstream-roadmap.md").read_text(),
        ))

    def test_valid_dependency_and_existing_feature(self):
        original = deepcopy(self.ledger)
        self.assertEqual([], checker.validate(self.todo, self.ledger, self.upstream))
        self.assertEqual(original, self.ledger)

    def test_missing_duplicate_and_unknown_coverage(self):
        second = self.ledger["workstreams"][1]
        second["coverage"] = ["A1", "C1"]
        second["feature_roles"] = {"A1": "existing", "C1": "implementation"}
        self.check_errors("unowned coverage B1", "A1 has multiple owners", "unknown coverage C1")

    def test_repeated_definition_is_not_hidden_by_sets(self):
        self.todo += "- [ ] **A1: Duplicate definition.**\n"
        self.ledger["workstreams"][0]["coverage"].append("A1")
        self.check_errors("TODO.md repeats A1", "W01.coverage repeats A1", "A1 has multiple owners")

    def test_dependencies_reject_cycles_and_missing_nodes(self):
        self.ledger["workstreams"][0]["depends_on"] = ["W02", "W99"]
        self.check_errors("unknown dependency W99", "dependency cycle: W01 -> W02 -> W01")

    def test_roles_and_upstream_references_are_checked(self):
        first = self.ledger["workstreams"][0]
        first["feature_roles"] = {"B1": "fiction"}
        first["upstream_candidates"] = ["U99"]
        self.check_errors("no role for A1", "unowned feature B1", "invalid role", "unknown upstream candidate U99")

    def test_recipe_work_cannot_hide_implementation(self):
        self.ledger["workstreams"][0]["kind"] = "recipe"
        self.check_errors("recipe workstream cannot own implementation")

    def test_malformed_collections_are_reported(self):
        first = self.ledger["workstreams"][0]
        first["depends_on"] = "W02"
        first["coverage"] = {"A1": True}
        first["feature_roles"] = []
        self.check_errors("depends_on must be a list", "coverage must be a list", "feature_roles must be an object")


if __name__ == "__main__":
    unittest.main()
