from __future__ import annotations

import json
from typing import Any, Iterator

from .errors import DocumentClosedError
from .models import Block, Image, Link, Story, Table


def _table_blocks(table: Table) -> Iterator[Block]:
    for row in table.cells:
        for cell in row:
            for block in cell.blocks:
                yield block
                if block.table:
                    yield from _table_blocks(block.table)


def _source_asset(asset: Link | Image) -> tuple | None:
    path = asset.metadata.get("source_path")
    if path is None:
        return None
    source = (tuple(path), asset.metadata.get("segment"), asset.page)
    if isinstance(asset, Link):
        return source + (asset.uri, asset.text, asset.metadata.get("start"), asset.metadata.get("end"))
    return source + (asset.name, asset.alt, asset.metadata.get("source_offset"))


class Page:
    def __init__(self, number: int, blocks: list[Block] | None = None, *, links: list[Link] | None = None, images: list[Image] | None = None, tables: list[Table] | None = None, metadata: dict[str, Any] | None = None) -> None:
        self.number = number
        self.blocks = blocks or []
        self._links = links or []
        self._images = images or []
        self._tables = tables or []
        self.metadata = metadata or {}
        self._document: Document | None = None

    def get_pixmap(self, dpi: float = 144) -> bytes:
        if self._document is None:
            raise ValueError("page is not attached to a document")
        return self._document.render_page(self.number, dpi)

    def get_text(self, option: str = "text") -> str | dict[str, Any]:
        if option == "text":
            parts: list[str] = []
            for block in self.blocks:
                if block.table:
                    parts.extend("\t".join(row) for row in block.table.rows)
                elif block.text:
                    parts.append(block.text)
            return "\n".join(parts).strip()
        if option == "markdown":
            return self._markdown()
        if option == "dict":
            return self.to_dict()
        if option == "json":
            return json.dumps(self.to_dict(), ensure_ascii=False)
        raise ValueError("option must be 'text', 'markdown', 'dict', or 'json'")

    def get_links(self) -> list[Link]:
        return list(self._links)

    def get_images(self, full: bool = False) -> list[Image] | list[tuple[int, Image]]:
        images = list(self._images)
        return list(enumerate(images, start=1)) if full else images

    def find_tables(self) -> list[Table]:
        return list(self._tables)

    @staticmethod
    def _linked_text(text: str, links: list[Link]) -> tuple[str, set[tuple[str, str, int, int]]]:
        replacements: list[tuple[int, int, Link]] = []
        for link in links:
            start = link.metadata.get("start")
            end = link.metadata.get("end")
            if isinstance(start, int) and isinstance(end, int) and 0 <= start < end <= len(text) and text[start:end] == link.text:
                replacements.append((start, end, link))
        replacements.sort(key=lambda item: item[0], reverse=True)
        emitted: set[tuple[str, str, int, int]] = set()
        last_start = len(text)
        for start, end, link in replacements:
            if end > last_start:
                continue
            text = f"{text[:start]}[{text[start:end]}]({link.uri}){text[end:]}"
            emitted.add((link.uri, link.text, start, end))
            last_start = start
        return text, emitted

    def _markdown(self, table_links: set[tuple] | None = None, table_images: set[tuple] | None = None) -> str:
        parts: list[str] = []
        emitted_links: set[tuple[str, str, int, int]] = set()
        emitted_images: set[tuple[str, str]] = set()
        for block in self.blocks:
            linked_text, emitted = self._linked_text(block.text, block.links)
            emitted_links.update(emitted)
            if block.kind == "heading":
                parts.append(f"{'#' * max(1, min(block.level or 1, 6))} {linked_text}")
            elif block.table:
                parts.append(block.table.to_markdown())
                for nested in _table_blocks(block.table):
                    _, emitted = self._linked_text(nested.text, nested.links)
                    emitted_links.update(emitted)
                    emitted_images.update((image.name, image.alt) for image in nested.images)
            elif block.kind == "comment":
                parts.append(f"> **Comment:** {block.text}")
            elif block.kind == "notes":
                parts.append(f"**Notes:**\n\n{block.text}")
            elif block.text:
                parts.append(linked_text)
            for image in block.images:
                parts.append(f"![{image.alt}]({image.name})")
                emitted_images.add((image.name, image.alt))
        for image in self._images:
            reference = f"![{image.alt}]({image.name})"
            if (image.name, image.alt) not in emitted_images and _source_asset(image) not in (table_images or set()):
                parts.append(reference)
                emitted_images.add((image.name, image.alt))
        remaining = [
            link for link in self._links
            if (link.uri, link.text, link.metadata.get("start"), link.metadata.get("end")) not in emitted_links
            and _source_asset(link) not in (table_links or set())
        ]
        if remaining:
            parts.append("**Links:**\n" + "\n".join(f"- [{link.text or link.uri}]({link.uri})" for link in remaining))
        return "\n\n".join(part for part in parts if part).strip()

    def to_dict(self) -> dict[str, Any]:
        return {"number": self.number, "blocks": [block.to_dict() for block in self.blocks], "links": [link.to_dict() for link in self._links], "images": [image.to_dict() for image in self._images], "tables": [table.to_dict() for table in self._tables], "metadata": self.metadata}


class Document:
    def __init__(self, pages: list[Page], metadata: dict[str, Any], source: bytes, filetype: str, password: str | None, *, stories: list[Story] | None = None) -> None:
        self._pages = pages
        self.metadata = metadata
        self.stories = list(stories or [])
        self._source = source
        self._filetype = filetype
        self._password = password
        self._closed = False
        for page in self._pages:
            page._document = self

    @property
    def page_count(self) -> int:
        self._ensure_open()
        return len(self._pages)

    def load_page(self, number: int) -> Page:
        self._ensure_open()
        if number < 0:
            number += len(self._pages)
        if number < 0 or number >= len(self._pages):
            raise IndexError("page index out of range")
        return self._pages[number]

    def __len__(self) -> int:
        return self.page_count

    def __getitem__(self, number: int) -> Page:
        return self.load_page(number)

    def __iter__(self) -> Iterator[Page]:
        self._ensure_open()
        return iter(self._pages)

    def to_markdown(self) -> str:
        self._ensure_open()
        table_links: set[tuple] = set()
        table_images: set[tuple] = set()
        for page in self._pages:
            for block in page.blocks:
                if block.table:
                    for nested in _table_blocks(block.table):
                        _, emitted = Page._linked_text(nested.text, nested.links)
                        table_links.update(key for link in nested.links if (link.uri, link.text, link.metadata.get("start"), link.metadata.get("end")) in emitted and (key := _source_asset(link)) is not None)
                        table_images.update(key for image in nested.images if (key := _source_asset(image)) is not None)
        return "\n\n".join(markdown for page in self._pages if (markdown := page._markdown(table_links, table_images))).strip()

    def render_page(self, page: int, dpi: float = 144) -> bytes:
        self._ensure_open()
        if page < 0:
            page += len(self._pages)
        if page < 0 or page >= len(self._pages):
            raise IndexError("page index out of range")
        from . import _core, _native
        return bytes(_native(lambda: _core.render(self._source, self._filetype, page, dpi, self._password)))

    def convert(self, output_format: str) -> bytes:
        self._ensure_open()
        from . import convert
        return convert(self._source, output_format, filetype=self._filetype, password=self._password)

    def edit(self):
        """Create an independent editable native PDF; unsupported preservation fails."""
        self._ensure_open()
        from . import _core, _native
        from ._authoring import PdfDocument
        from .errors import UnsupportedFormatError
        if self._filetype != "pdf":
            raise UnsupportedFormatError("editing existing Office files requires preservation diagnostics; create a new document with new()")
        return PdfDocument(_native(lambda: _core.EditablePdf.from_bytes(self._source, self._password)))

    def close(self) -> None:
        self._closed = True

    def _ensure_open(self) -> None:
        if self._closed:
            raise DocumentClosedError("document is closed")

    def __enter__(self) -> Document:
        self._ensure_open()
        return self

    def __exit__(self, exc_type: object, exc: object, traceback: object) -> None:
        self.close()
