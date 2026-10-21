#!/usr/bin/env python3
"""Build the documentation without importing doc_ex or requiring Rust.

Source Markdown remains canonical. Only site presentation lives in website/.
Python-Markdown is a pinned build-only dependency. Run check_site.py afterward.
"""
from __future__ import annotations

import argparse
import html
from html.parser import HTMLParser
import os
from pathlib import Path
import re
import shutil
from urllib.parse import quote, unquote, urlsplit, urlunsplit

import markdown

ROOT = Path(__file__).resolve().parents[1]
REPO = "https://github.com/narain1/pydoccraft"
PAGES = {Path("website/index.md"): Path("index.html"),
         Path("README.md"): Path("overview.html"),
         Path("TODO.md"): Path("roadmap.html"),
         Path("CONTRIBUTING.md"): Path("contributing.html"),
         Path("SECURITY.md"): Path("security.html"),
         Path("SUPPORT.md"): Path("support.html")}
PAGES.update({p.relative_to(ROOT): p.relative_to(ROOT).with_suffix(".html")
              for p in sorted((ROOT / "docs").glob("*.md"))})
NAV = [
    ("Start here", [("Home", "index.html"), ("Python API", "docs/api.html"),
                    ("Formats & rendering", "docs/formats.html")]),
    ("Create & compose", [("Authoring · development", "docs/authoring.html"),
                          ("Cookbook", "docs/cookbook.html"),
                          ("Markdown & assets", "docs/markdown-assets.html")]),
    ("Understand support", [("Operation matrix", "docs/support-matrix.html"),
                            ("Editing readiness", "docs/format-authoring-readiness.html"),
                            ("Safety & limits", "docs/safety.html"),
                            ("Licenses & dependencies", "docs/dependencies-and-licenses.html")]),
    ("Build with us", [("Feature roadmap", "roadmap.html"),
                       ("Workstreams", "docs/roadmap-workstreams.html"),
                       ("Upstream work", "docs/upstream-roadmap.html"),
                       ("Development", "docs/development.html")]),
]


def relative(target: Path, current: Path) -> str:
    return Path(os.path.relpath(target, current.parent)).as_posix()


def slugify(value: str, separator: str) -> str:
    # Match existing GitHub-style Markdown fragment links (including underscores).
    value = re.sub(r"[^\w\- ]", "", value.lower())
    return value.replace(" ", separator)


class Links(HTMLParser):
    """Rewrite links after Markdown so fenced code examples are untouched."""
    def __init__(self, source: Path, destination: Path):
        super().__init__(convert_charrefs=False)
        self.source, self.destination = source, destination
        self.parts: list[str] = []

    def url(self, value: str) -> str:
        url = urlsplit(value)
        if url.scheme or url.netloc or not url.path:
            return value
        if url.path.startswith("/"):
            raise ValueError(f"Root-relative URL in {self.source}: {value}")
        target = (ROOT / self.source.parent / unquote(url.path)).resolve()
        if not target.is_relative_to(ROOT) or not target.exists():
            raise ValueError(f"Missing repository target in {self.source}: {value}")
        key = target.relative_to(ROOT)
        if key in PAGES:
            path = relative(PAGES[key], self.destination)
            return urlunsplit(("", "", path, url.query, url.fragment))
        kind = "tree" if target.is_dir() else "blob"
        return f"{REPO}/{kind}/main/{quote(key.as_posix())}" + (
            f"#{url.fragment}" if url.fragment else "")

    def handle_starttag(self, tag, attrs):
        attrs = [(key, self.url(value) if key in ("href", "src") and value else value)
                 for key, value in attrs]
        rendered = "".join(f' {key}="{html.escape(value, quote=True)}"'
                           if value is not None else f" {key}" for key, value in attrs)
        self.parts.append(f"<{tag}{rendered}>")

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)

    def handle_endtag(self, tag):
        self.parts.append(f"</{tag}>")

    def handle_data(self, data):
        self.parts.append(data)

    def handle_entityref(self, name):
        self.parts.append(f"&{name};")

    def handle_charref(self, name):
        self.parts.append(f"&#{name};")

    def handle_comment(self, data):
        self.parts.append(f"<!--{data}-->")


def build(output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    for source, destination in PAGES.items():
        text = (ROOT / source).read_text()
        title = text.splitlines()[0].removeprefix("# ")
        renderer = markdown.Markdown(extensions=["tables", "fenced_code", "toc", "sane_lists"],
                                    extension_configs={"toc": {"slugify": slugify,
                                                               "toc_depth": "2-3"}})
        content = Links(source, destination)
        content.feed(renderer.convert(text))
        nav = ""
        for label, items in NAV:
            nav += f'<div class="nav-group"><p>{label}</p>'
            for name, href in items:
                active = ' aria-current="page"' if str(destination) == href else ""
                nav += f'<a href="{relative(Path(href), destination)}"{active}>{name}</a>'
            nav += "</div>"
        other = [(p.stem.replace("-", " ").capitalize(), dest) for p, dest in PAGES.items()
                 if str(dest) not in {item[1] for _, group in NAV for item in group}]
        nav += '<details class="more-guides"><summary>More guides</summary>'
        nav += "".join(f'<a href="{relative(dest, destination)}">{name}</a>' for name, dest in other)
        nav += '</details>'
        home = relative(Path("index.html"), destination)
        css = relative(Path("assets/site.css"), destination)
        page = f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="description" content="pydoccraft documentation: native Python document extraction, rendering, conversion and authoring.">
<title>{html.escape(title)} · pydoccraft</title><link rel="stylesheet" href="{css}"></head>
<body><a class="skip" href="#content">Skip to content</a>
<header><a class="brand" href="{home}"><span class="mark" aria-hidden="true">p</span>pydoccraft<span class="docs-label">/ docs</span></a>
<a class="github" href="{REPO}">GitHub ↗</a></header>
<div class="layout"><nav aria-label="Documentation">{nav}</nav>
<main id="content"><div class="version-note">Development documentation · Features marked unreleased require a build from source. <a href="{REPO}/releases">Check releases →</a></div>
<article>{''.join(content.parts)}</article>
<footer><a href="{REPO}/blob/main/{source}">View this page on GitHub</a><span>Apache-2.0 · Offline document processing</span></footer></main>
<aside aria-label="On this page"><p>On this page</p>{renderer.toc}</aside></div></body></html>'''
        path = output / destination
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(page)
    shutil.copytree(ROOT / "website/assets", output / "assets", dirs_exist_ok=True)
    (output / ".nojekyll").touch()
    print(f"Built {len(PAGES)} pages in {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "build/site")
    build(parser.parse_args().output.resolve())
