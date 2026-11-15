#!/usr/bin/env python3
"""Emit a static, source-based migration inventory as JSON; no extension import needed."""
import ast
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKAGE = ROOT / "src" / "doc_ex"
PROTOCOL = {"__init__", "__enter__", "__exit__", "__len__", "__iter__", "__next__", "__getitem__", "__setitem__", "__delitem__", "__contains__", "__bool__", "__str__", "__repr__", "__eq__", "__hash__", "__bytes__", "__fspath__"}


def visible(name):
    return not name.startswith("_") or name in PROTOCOL


def text(node):
    return ast.unparse(node) if node is not None else None


def references(node):
    # Syntactic references only; includes nested functions and does not resolve aliases.
    calls = sorted({text(n.func) for n in ast.walk(node) if isinstance(n, ast.Call)})
    return {"calls": calls, "native_refs": sorted({text(n) for n in ast.walk(node)
            if isinstance(n, ast.Attribute) and isinstance(n.value, ast.Name) and n.value.id == "_core"})}


def function(node):
    return {"name": node.name, "line": node.lineno,
            "signature": f"({text(node.args)})" + (f" -> {text(node.returns)}" if node.returns else ""),
            "decorators": [text(x) for x in node.decorator_list], **references(node)}


def fields(node):
    result = []
    for item in node.body:
        if isinstance(item, ast.AnnAssign) and isinstance(item.target, ast.Name) and visible(item.target.id):
            result.append({"name": item.target.id, "annotation": text(item.annotation), "default": text(item.value), "line": item.lineno})
    return result


def inventory():
    modules = {}
    for path in sorted(PACKAGE.glob("*.py")):
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        module = {"path": str(path.relative_to(ROOT)), "exports": None, "imports": [], "functions": [], "classes": []}
        for node in tree.body:
            if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "__all__" for t in node.targets):
                module["exports"] = ast.literal_eval(node.value)
            elif isinstance(node, (ast.Import, ast.ImportFrom)):
                module["imports"].append(text(node))
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and visible(node.name):
                module["functions"].append(function(node))
            elif isinstance(node, ast.ClassDef) and visible(node.name):
                methods = [function(n) for n in node.body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef)) and visible(n.name)]
                # Instance attributes include unannotated assignments, not just dataclass fields.
                attrs = sorted({n.attr for n in ast.walk(node) if isinstance(n, ast.Attribute)
                                and isinstance(n.value, ast.Name) and n.value.id == "self"
                                and isinstance(n.ctx, ast.Store) and visible(n.attr)})
                module["classes"].append({"name": node.name, "line": node.lineno,
                    "bases": [text(x) for x in node.bases], "decorators": [text(x) for x in node.decorator_list],
                    "fields": fields(node), "instance_attributes": attrs, "methods": methods})
        modules[path.stem] = module
    return {"schema": 1, "scope": "static public source declarations; see docs/python-api-migration-inventory.md", "modules": modules}


if __name__ == "__main__":
    print(json.dumps(inventory(), indent=2, ensure_ascii=False))
