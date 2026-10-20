"""License expressions and resolved graph checks do not require Cargo or network."""
from copy import deepcopy
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("audit_licenses", Path(__file__).resolve().parents[1] / "tools/audit_licenses.py")
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


class LicenseAuditTests(unittest.TestCase):
    def test_permissive_alternatives_and_required_terms(self):
        for expression in ("MIT", "MIT/Apache-2.0", "GPL-3.0-only OR MIT", "(MIT OR Apache-2.0) AND BSD-3-Clause", "Apache-2.0 WITH LLVM-exception"):
            self.assertTrue(audit.allowed(expression), expression)
        for expression in (None, "", "LGPL-2.1-only", "MIT AND GPL-3.0-only", "MIT WITH unknown", "MIT OR", "MIT AND (Apache-2.0", "MIT OR Apache-2.0)", "OR MIT", "LicenseRef-Custom"):
            self.assertFalse(audit.allowed(expression), expression)

    def test_graph_follows_transitive_normal_and_build_but_not_dev(self):
        names = ("root", "native", "helper", "dev-only")
        metadata = {
            "packages": [{"id": name, "name": name, "version": "1.0", "license": "GPL-3.0-only" if name == "dev-only" else "MIT"} for name in names],
            "resolve": {"root": "root", "nodes": [
                {"id": "root", "deps": [
                    {"pkg": "native", "dep_kinds": [{"kind": None}]},
                    {"pkg": "dev-only", "dep_kinds": [{"kind": "dev"}]},
                ]},
                {"id": "native", "deps": [{"pkg": "helper", "dep_kinds": [{"kind": "build"}]}]},
                {"id": "helper", "deps": []}, {"id": "dev-only", "deps": []},
            ]},
        }
        self.assertEqual([row[0] for row in audit.inventory(metadata)], ["helper", "native"])
        prohibited = deepcopy(metadata)
        prohibited["packages"][2]["name"] = "wordcraft-engine"
        with self.assertRaisesRegex(ValueError, "prohibited application"):
            audit.inventory(prohibited)
        metadata["packages"][2]["license"] = "AGPL-3.0-only"
        with self.assertRaisesRegex(ValueError, "prohibited licenses"):
            audit.inventory(metadata)
