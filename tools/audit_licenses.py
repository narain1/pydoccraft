"""Audit Cargo's resolved normal/build graph and regenerate its license inventory.

Pass JSON produced by `cargo metadata --locked --format-version 1`.
The allowlist reflects reviewed licenses; new license identifiers require review.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

ALLOWED = {"0BSD", "Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause", "BSL-1.0",
           "OFL-1.1", "Ubuntu-font-1.0", "Unicode-3.0", "Unlicense", "Zlib"}
EXCEPTIONS = {("Apache-2.0", "LLVM-exception")}


def allowed(expression: str | None) -> bool:
    if not expression:
        return False
    tokens = re.findall(r"\(|\)|[^\s()]+", expression.replace("/", " OR "))
    position = 0

    def atom():
        nonlocal position
        if position >= len(tokens):
            raise ValueError("incomplete license expression")
        term = tokens[position]
        position += 1
        if term == "(":
            result = either()
            if position >= len(tokens) or tokens[position] != ")":
                raise ValueError("unbalanced license expression")
            position += 1
            return result
        if term in {"AND", "OR", "WITH", ")"}:
            raise ValueError("unexpected license operator")
        if position < len(tokens) and tokens[position] == "WITH":
            position += 1
            if position >= len(tokens):
                raise ValueError("missing license exception")
            exception = tokens[position]
            position += 1
            return (term, exception) in EXCEPTIONS
        return term in ALLOWED

    def both():
        nonlocal position
        result = atom()
        while position < len(tokens) and tokens[position] == "AND":
            position += 1
            value = atom()
            result = result and value
        return result

    def either():
        nonlocal position
        result = both()
        while position < len(tokens) and tokens[position] == "OR":
            position += 1
            value = both()
            result = result or value
        return result

    try:
        result = either()
        return position == len(tokens) and result
    except ValueError:
        return False


def inventory(metadata):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    pending, reached = [root], set()
    while pending:
        identifier = pending.pop()
        if identifier in reached:
            continue
        reached.add(identifier)
        for dependency in nodes[identifier]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    reached.remove(root)
    rows = sorted((packages[key]["name"], packages[key]["version"], packages[key]["license"])
                  for key in reached)
    rejected = [(name, version, license) for name, version, license in rows if not allowed(license)]
    if rejected:
        raise ValueError(f"unreviewed or prohibited licenses: {rejected}")
    prohibited = [name for name, _, _ in rows if re.search(
        r"(?:^|[-_])(engine|ui|cli|mcp|automation|updater|ocr|javascript|winprint|print|printing|signing)(?:[-_]|$)", name)]
    if prohibited:
        raise ValueError(f"prohibited application/runtime packages: {prohibited}")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("metadata", type=Path)
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    rows = inventory(json.loads(args.metadata.read_text(encoding="utf-8")))
    root = Path(__file__).resolve().parents[1]
    inventory_path = root / "THIRD_PARTY_LICENSES.md"
    existing = inventory_path.read_text(encoding="utf-8")
    prefix, marker, _ = existing.partition("| Crate | Version | License |\n|---|---:|---|\n")
    if not marker:
        parser.error("license inventory table header is missing")
    rendered = prefix + marker + "".join(f"| `{name}` | `{version}` | {license} |\n" for name, version, license in rows)
    if args.write:
        inventory_path.write_text(rendered, encoding="utf-8")
        (root / "LICENSE-THIRD-PARTY").write_text(rendered, encoding="utf-8")
    elif rendered != existing or (root / "LICENSE-THIRD-PARTY").read_text(encoding="utf-8") != rendered:
        parser.error("license inventory is stale; review the graph and rerun with --write")
    print(f"Reviewed license expressions: {len(rows)} normal/build packages")


if __name__ == "__main__":
    main()
