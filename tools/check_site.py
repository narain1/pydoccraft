#!/usr/bin/env python3
"""Check every local generated link, asset and HTML fragment without networking."""
from __future__ import annotations
import argparse
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urlsplit


class Page(HTMLParser):
    def __init__(self, text: str):
        super().__init__()
        self.ids: set[str] = set()
        self.urls: list[str] = []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        for name, value in attrs:
            if name == "id":
                if value in self.ids:
                    raise ValueError(f"Duplicate HTML id: {value}")
                self.ids.add(value)
            if name in ("href", "src") and value:
                self.urls.append(value)


def check(root: Path) -> None:
    pages = {p.resolve(): Page(p.read_text()) for p in root.rglob("*.html")}
    if (root / "index.html").resolve() not in pages:
        raise ValueError("Missing index.html")
    count = 0
    errors = []
    for source, page in pages.items():
        for value in page.urls:
            url = urlsplit(value)
            if url.scheme or url.netloc:
                continue
            target = (source.parent / unquote(url.path)).resolve() if url.path else source
            if not target.is_relative_to(root) or not target.is_file():
                errors.append(f"{source.relative_to(root)}: missing local target {value}")
            elif url.fragment and target in pages and unquote(url.fragment) not in pages[target].ids:
                errors.append(f"{source.relative_to(root)}: missing fragment {value}")
            count += 1
    if errors:
        raise ValueError("\n".join(errors))
    print(f"Checked {len(pages)} pages and {count} local links/assets/fragments")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("site", type=Path)
    check(parser.parse_args().site.resolve())
