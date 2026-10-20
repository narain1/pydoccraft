"""Run the corpus against an installed wheel and record reproducible evidence.

Usage: python tools/validate_installed.py --wheel-dir dist --output dist/validation.json
Install the wheel into a clean environment first. This script never installs packages.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import json
import platform
import sys
import time
import tomllib
import unittest
import zipfile
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--wheel-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    wheels = list(args.wheel_dir.glob("pydoccraft-*.whl"))
    if len(wheels) != 1:
        parser.error("wheel directory must contain exactly one pydoccraft wheel")
    wheel = wheels[0]

    import doc_ex
    from doc_ex import _core

    distribution = importlib.metadata.distribution("pydoccraft")
    installed = Path(distribution.locate_file("doc_ex")).resolve()
    for module in (doc_ex, _core):
        location = Path(module.__file__).resolve()
        if not location.is_relative_to(installed) or location.is_relative_to(root / "src"):
            parser.error(f"expected installed wheel module, found {location}")
    if distribution.version != doc_ex.__version__:
        parser.error("installed distribution and Python facade versions disagree")

    # Compare installed package bytes with the specified artifact, including native code.
    # This prevents a stale installation from being reported as a newly built wheel.
    with zipfile.ZipFile(wheel) as archive:
        package_files = [item for item in archive.infolist()
                         if item.filename.startswith("doc_ex/") and not item.is_dir()]
        if not package_files:
            parser.error("wheel contains no doc_ex package")
        member_paths = {Path(distribution.locate_file(item.filename)).resolve()
                        for item in package_files}
        for module in (doc_ex, _core):
            if Path(module.__file__).resolve() not in member_paths:
                parser.error(f"loaded module is not part of the wheel: {module.__file__}")
        for item in package_files:
            actual = Path(distribution.locate_file(item.filename))
            if not actual.is_file() or actual.read_bytes() != archive.read(item):
                parser.error(f"installed file does not match wheel: {item.filename}")
        extension_size = sum(item.file_size for item in package_files
                             if item.filename.endswith((".so", ".pyd", ".dylib")))

    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    revisions = {name: spec["rev"] for name, spec in manifest["dependencies"].items()
                 if isinstance(spec, dict) and "rev" in spec}
    revisions.update({name: spec["rev"] for name, spec in
                      manifest.get("patch", {}).get("crates-io", {}).items() if "rev" in spec})
    started = time.monotonic()
    suite = unittest.defaultTestLoader.discover(str(root / "tests"))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    evidence = {
        "passed": result.wasSuccessful(),
        "tests": result.testsRun,
        "failures": len(result.failures),
        "errors": len(result.errors),
        "skipped": len(result.skipped),
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "python": sys.version,
        "platform": platform.platform(),
        "distribution_version": distribution.version,
        "installed_package": str(installed),
        "wheel": wheel.name,
        "wheel_bytes": wheel.stat().st_size,
        "wheel_sha256": hashlib.sha256(wheel.read_bytes()).hexdigest(),
        "native_extension_bytes": extension_size,
        "cargo_lock_sha256": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
        "upstream_revisions": revisions,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n")
    print(f"Validation evidence: {args.output}")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
