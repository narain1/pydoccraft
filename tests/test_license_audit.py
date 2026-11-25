"""License expressions and resolved graph checks do not require Cargo or network."""
from copy import deepcopy
import importlib.util
from pathlib import Path
import sys
import unittest

SPEC = importlib.util.spec_from_file_location("audit_licenses", Path(__file__).resolve().parents[1] / "tools/audit_licenses.py")
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)
sys.modules["audit_licenses"] = audit
BINDING_SPEC = importlib.util.spec_from_file_location("audit_binding_licenses", Path(__file__).resolve().parents[1] / "tools/audit_binding_licenses.py")
binding_audit = importlib.util.module_from_spec(BINDING_SPEC)
BINDING_SPEC.loader.exec_module(binding_audit)


class LicenseAuditTests(unittest.TestCase):
    def test_regeneration_preserves_attribution_after_table(self):
        existing = "Attribution\n\n| Crate | Version | License |\n|---|---:|---|\n| `old` | `1` | MIT |\n\nImage attribution.\n"
        expected = existing.replace("| `old` | `1` | MIT |", "| `new` | `2` | Apache-2.0 |")
        self.assertEqual(audit.render_inventory(existing, [("new", "2", "Apache-2.0")]), expected)

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
        reviewed = deepcopy(metadata)
        reviewed["packages"][2].update(
            name="parley_engine", version="0.13.0",
            source="registry+https://github.com/rust-lang/crates.io-index",
        )
        self.assertIn(("parley_engine", "0.13.0", "MIT"), audit.inventory(reviewed))
        reviewed["packages"][2]["source"] = "git+https://example.com/replacement"
        with self.assertRaisesRegex(ValueError, "prohibited application"):
            audit.inventory(reviewed)
        metadata["packages"][2]["license"] = "AGPL-3.0-only"
        with self.assertRaisesRegex(ValueError, "prohibited licenses"):
            audit.inventory(metadata)

    def test_binding_graph_requires_root_attribution(self):
        def graph(root, dependency):
            return {
                "packages": [{"id": name, "name": name, "version": "1", "license": "MIT"} for name in (root, dependency)],
                "resolve": {"root": root, "nodes": [
                    {"id": root, "deps": [{"pkg": dependency, "dep_kinds": [{"kind": None}]}]},
                    {"id": dependency, "deps": []},
                ]},
            }
        root = graph("core", "shared")
        self.assertEqual(binding_audit.missing_attributions(root, graph("binding", "shared")), [])
        self.assertEqual(binding_audit.missing_attributions(root, graph("binding", "missing")), [("missing", "1", "MIT")])
