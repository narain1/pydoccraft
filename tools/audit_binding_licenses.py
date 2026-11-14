"""Check a separately locked binding graph against the root attribution inventory.

Pass metadata from the root and the binding's `cargo metadata --locked` runs.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

from audit_licenses import inventory


def missing_attributions(root_metadata, binding_metadata):
    root_rows = set(inventory(root_metadata))
    root_package = next(
        package for package in root_metadata["packages"]
        if package["id"] == root_metadata["resolve"]["root"]
    )
    root_rows.add((root_package["name"], root_package["version"], root_package["license"]))
    return [row for row in inventory(binding_metadata) if row not in root_rows]


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: audit_binding_licenses.py ROOT_METADATA BINDING_METADATA")
    root, binding = (json.loads(Path(path).read_text(encoding="utf-8")) for path in sys.argv[1:])
    missing = missing_attributions(root, binding)
    if missing:
        raise SystemExit(f"binding dependencies missing from root attribution: {missing}")
    print("Binding dependency licenses and root attribution verified")


if __name__ == "__main__":
    main()
