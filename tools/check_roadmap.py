#!/usr/bin/env python3
"""Check roadmap ownership and dependency references without runtime dependencies.

Run from any directory: python /path/to/repo/tools/check_roadmap.py.
This checks planning consistency, not feature implementation or completion.
"""

from __future__ import annotations

import argparse
from collections import Counter
import json
from pathlib import Path
import re
import sys


COVERAGE = re.compile(r"^- \[[ xX]\] \*\*([A-Z]+[0-9]+):", re.MULTILINE)
UPSTREAM = re.compile(r"^\| (U[0-9]+) \|", re.MULTILINE)
ROLES = {"implementation", "milestone", "recipe", "acceptance", "existing"}
KINDS = {"implementation", "recipe", "validation"}


def validate(todo: str, ledger: dict, upstream: str) -> list[str]:
    """Return all discoverable consistency errors in a decoded ledger."""
    errors: list[str] = []
    coverage_counts = Counter(COVERAGE.findall(todo))
    if not coverage_counts:
        errors.append("TODO.md has no coverage checkboxes")
    for identifier, count in sorted(coverage_counts.items()):
        if count != 1:
            errors.append(f"TODO.md repeats {identifier} ({count} definitions)")
    upstream_counts = Counter(UPSTREAM.findall(upstream))
    for identifier, count in sorted(upstream_counts.items()):
        if count != 1:
            errors.append(f"upstream register repeats {identifier}")
    if not isinstance(ledger, dict):
        return errors + ["ledger must be an object"]
    if ledger.get("schema_version") != 1:
        errors.append("schema_version must be 1")
    if ledger.get("source") != "TODO.md":
        errors.append("source must be TODO.md")
    workstreams = ledger.get("workstreams")
    if not isinstance(workstreams, list) or not workstreams:
        return errors + ["workstreams must be a nonempty list"]

    owners: dict[str, list[str]] = {}
    graph: dict[str, list[str]] = {}

    def strings(value: object, context: str) -> list[str]:
        if not isinstance(value, list) or any(not isinstance(x, str) for x in value):
            errors.append(f"{context} must be a list of strings")
            return []
        for entry, count in sorted(Counter(value).items()):
            if count > 1:
                errors.append(f"{context} repeats {entry}")
        return value

    for index, workstream in enumerate(workstreams):
        if not isinstance(workstream, dict):
            errors.append(f"workstreams[{index}] must be an object")
            continue
        identifier = workstream.get("id")
        if not isinstance(identifier, str) or not re.fullmatch(r"W[0-9]{2,}", identifier):
            errors.append(f"workstreams[{index}] has invalid id")
            continue
        if identifier in graph:
            errors.append(f"duplicate workstream {identifier}")
        dependencies = strings(workstream.get("depends_on"), f"{identifier}.depends_on")
        graph[identifier] = dependencies
        coverage = strings(workstream.get("coverage"), f"{identifier}.coverage")
        if not coverage:
            errors.append(f"{identifier} has no coverage")
        for feature in coverage:
            owners.setdefault(feature, []).append(identifier)
        kind = workstream.get("kind")
        if not isinstance(kind, str) or kind not in KINDS:
            errors.append(f"{identifier} has invalid kind")
        if workstream.get("priority") not in ("P0", "P1", "P2"):
            errors.append(f"{identifier} has invalid priority")
        roles = workstream.get("feature_roles")
        if not isinstance(roles, dict):
            errors.append(f"{identifier}.feature_roles must be an object")
            roles = {}
        for feature in sorted(set(coverage) - roles.keys()):
            errors.append(f"{identifier} has no role for {feature}")
        for feature in sorted(roles.keys() - set(coverage)):
            errors.append(f"{identifier} assigns role to unowned feature {feature}")
        for feature, role in roles.items():
            if not isinstance(role, str) or role not in ROLES:
                errors.append(f"{identifier}.{feature} has invalid role {role!r}")
            elif kind == "recipe" and role not in {"recipe", "milestone"}:
                errors.append(f"{identifier}.{feature}: recipe workstream cannot own {role}")
            elif kind == "validation" and role != "acceptance":
                errors.append(f"{identifier}.{feature}: validation workstream must own acceptance")
            elif kind == "implementation" and role in {"recipe", "acceptance"}:
                errors.append(f"{identifier}.{feature}: {role} needs its own workstream kind")
        candidates = strings(workstream.get("upstream_candidates"), f"{identifier}.upstream_candidates")
        for candidate in candidates:
            if candidate not in upstream_counts:
                errors.append(f"{identifier} references unknown upstream candidate {candidate}")

    for feature in sorted(coverage_counts.keys() - owners.keys()):
        errors.append(f"unowned coverage {feature}")
    for feature, assigned in sorted(owners.items()):
        if feature not in coverage_counts:
            errors.append(f"unknown coverage {feature}")
        if len(assigned) != 1:
            errors.append(f"{feature} has multiple owners: {', '.join(assigned)}")
    for identifier, dependencies in graph.items():
        for dependency in dependencies:
            if dependency not in graph:
                errors.append(f"{identifier} references unknown dependency {dependency}")

    visited: set[str] = set()
    active: list[str] = []

    def visit(identifier: str) -> None:
        if identifier in active:
            cycle = active[active.index(identifier):] + [identifier]
            errors.append("dependency cycle: " + " -> ".join(cycle))
            return
        if identifier in visited or identifier not in graph:
            return
        active.append(identifier)
        for dependency in graph[identifier]:
            visit(dependency)
        active.pop()
        visited.add(identifier)

    for identifier in graph:
        visit(identifier)
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        todo = (args.root / "TODO.md").read_text(encoding="utf-8")
        ledger = json.loads((args.root / "docs/roadmap-workstreams.json").read_text(encoding="utf-8"))
        upstream = (args.root / "docs/upstream-roadmap.md").read_text(encoding="utf-8")
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        print(f"Cannot read roadmap: {error}", file=sys.stderr)
        return 1
    errors = validate(todo, ledger, upstream)
    if errors:
        print("\n".join(f"ERROR: {error}" for error in errors), file=sys.stderr)
        return 1
    print(f"Roadmap valid: {len(COVERAGE.findall(todo))} coverage IDs, "
          f"{len(ledger['workstreams'])} workstreams, no ownership or dependency conflicts.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
