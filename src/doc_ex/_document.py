from __future__ import annotations

import hashlib
import json
import math
from numbers import Real
from typing import Any, Callable, Iterator

from .errors import DocumentClosedError, MalformedDocumentError, UnsafeArchiveError
from .models import Block, Image, Link, Story, Table


_MAX_EXPORT_BYTES = 128 * 1024 * 1024
_MAX_EXPORT_NODES = 100_000
_MAX_EXPORT_DEPTH = 32
_MEDIA_EXTENSIONS = {
    "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif",
    "image/webp": "webp", "image/tiff": "tiff", "image/bmp": "bmp",
}


def _render_clip(value) -> tuple[float, float, float, float] | None:
    if value is None:
        return None
    if not isinstance(value, (tuple, list)) or len(value) != 4:
        raise TypeError("clip must be (x, y, width, height) in points")
    if any(isinstance(item, bool) or not isinstance(item, Real) for item in value):
        raise TypeError("clip coordinates must be real numbers")
    result = tuple(float(item) for item in value)
    if any(not math.isfinite(item) for item in result) or result[2] <= 0 or result[3] <= 0:
        raise ValueError("clip must have finite coordinates and positive dimensions")
    return result


def _markdown_alt(text: str) -> str:
    return text.replace("\\", "\\\\").replace("[", "\\[").replace("]", "\\]").replace("\r", " ").replace("\n", " ")


def _image_placement(image: Image) -> tuple:
    return _source_asset(image) or (image.name, image.alt, image.page, image.bbox)


def _check_export_structure(document: Document) -> None:
    """Bound formatter work before recursive formatting or large string joins."""
    estimate = 0
    nodes = 0
    active: set[int] = set()

    def charge(*strings: str) -> None:
        nonlocal estimate, nodes
        nodes += 1
        if nodes > _MAX_EXPORT_NODES:
            raise UnsafeArchiveError("Markdown export exceeds 100000 content nodes")
        estimate += 128
        for text in strings:
            for offset in range(0, len(text), 4096):
                # HTML escaping expands a UTF-8 byte by at most six characters.
                estimate += 6 * len(text[offset:offset + 4096].encode("utf-8"))
                if estimate > _MAX_EXPORT_BYTES:
                    raise UnsafeArchiveError("Markdown export content exceeds the bounded formatting budget")
        if estimate > _MAX_EXPORT_BYTES:
            raise UnsafeArchiveError("Markdown export content exceeds the bounded formatting budget")

    def images(values: list[Image]) -> None:
        for image in values:
            charge(image.name, image.alt, image.media_type)

    def blocks(values: list[Block], depth: int = 0) -> None:
        nonlocal estimate, nodes
        if depth > _MAX_EXPORT_DEPTH:
            raise UnsafeArchiveError("Markdown export table nesting exceeds 32")
        for block in values:
            charge(block.text)
            for link in block.links:
                charge(link.uri, link.text)
            images(block.images)
            table = block.table
            if table is None:
                continue
            if id(table) in active:
                raise MalformedDocumentError("Markdown export contains a cyclic table")
            active.add(id(table))
            # Pipe tables pad ragged rows; HTML may emit omitted grid cells.
            # Bound that expanded grid before either formatter allocates it.
            flat_grid = len(table.rows) * max((len(row) for row in table.rows), default=0)
            rich_width = 0
            for row in table.cells:
                column = 0
                for cell in row:
                    grid_column = cell.metadata.get("grid_column", column)
                    if not isinstance(grid_column, int) or not isinstance(cell.col_span, int):
                        raise MalformedDocumentError("Markdown export has invalid table geometry")
                    column = grid_column + cell.col_span
                    rich_width = max(rich_width, column)
            grid = max(flat_grid, len(table.cells) * rich_width)
            nodes += grid
            estimate += 128 * grid
            if nodes > _MAX_EXPORT_NODES or estimate > _MAX_EXPORT_BYTES:
                raise UnsafeArchiveError("Markdown export expanded table grid exceeds the formatting budget")
            for row in table.rows:
                charge()
                for value in row:
                    charge(value)
            for row in table.cells:
                for cell in row:
                    charge()
                    blocks(cell.blocks, depth + 1)
            active.remove(id(table))

    for page in document._pages:
        charge()
        blocks(page.blocks)
        images(page._images)
        for link in page._links:
            charge(link.uri, link.text)
    for story in document.stories:
        charge(story.kind)
        blocks(story.blocks)


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

    def get_pixmap(self, dpi: float = 144, *, clip: tuple[float, float, float, float] | None = None) -> bytes:
        if self._document is None:
            raise ValueError("page is not attached to a document")
        return self._document.render_page(self.number, dpi, clip=clip)

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

    def _markdown(self, table_links: set[tuple] | None = None, table_images: set[tuple] | None = None, *, image_reference: Callable[[Image, tuple], str] | None = None, path: tuple = ()) -> str:
        parts: list[str] = []
        emitted_links: set[tuple[str, str, int, int]] = set()
        emitted_images: set[tuple[str, str]] = set()
        placed_images: dict[tuple, int] = {}

        def placed(image: Image) -> None:
            if image_reference is None:
                return
            key = _image_placement(image)
            placed_images[key] = placed_images.get(key, 0) + 1

        for block_number, block in enumerate(self.blocks):
            block_path = path + ("blocks", block_number)
            linked_text, emitted = self._linked_text(block.text, block.links)
            emitted_links.update(emitted)
            if block.kind == "heading":
                parts.append(f"{'#' * max(1, min(block.level or 1, 6))} {linked_text}")
            elif block.table:
                parts.append(block.table._markdown(image_reference, block_path + ("table",)))
                for nested in _table_blocks(block.table):
                    _, emitted = self._linked_text(nested.text, nested.links)
                    emitted_links.update(emitted)
                    emitted_images.update((image.name, image.alt) for image in nested.images)
                    for image in nested.images:
                        placed(image)
            elif block.kind == "comment":
                parts.append(f"> **Comment:** {block.text}")
            elif block.kind == "notes":
                parts.append(f"**Notes:**\n\n{block.text}")
            elif block.text:
                parts.append(linked_text)
            for image_number, image in enumerate(block.images):
                reference = image_reference(image, block_path + ("images", image_number)) if image_reference else image.name
                alt = _markdown_alt(image.alt) if image_reference else image.alt
                parts.append(f"![{alt}]({reference})")
                emitted_images.add((image.name, image.alt))
                placed(image)
        for image_number, image in enumerate(self._images):
            if image_reference:
                key = _image_placement(image)
                if placed_images.get(key, 0):
                    placed_images[key] -= 1
                    continue
            is_new = image_reference is not None or (image.name, image.alt) not in emitted_images
            if is_new and _source_asset(image) not in (table_images or set()):
                reference = image_reference(image, path + ("images", image_number)) if image_reference else image.name
                alt = _markdown_alt(image.alt) if image_reference else image.alt
                parts.append(f"![{alt}]({reference})")
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
    def __init__(self, pages: list[Page], metadata: dict[str, Any], source: bytes, filetype: str, password: str | None, *, stories: list[Story] | None = None, _render_session=None) -> None:
        self._pages = pages
        self.metadata = metadata
        self.stories = list(stories or [])
        self._source = source
        self._filetype = filetype
        self._password = password
        self._render_session = _render_session
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
        return self._markdown()

    def _markdown(self, image_reference: Callable[[Image, tuple], str] | None = None) -> str:
        table_links: set[tuple] = set()
        table_images: set[tuple] = set()
        for page in self._pages:
            for block in page.blocks:
                if block.table:
                    for nested in _table_blocks(block.table):
                        _, emitted = Page._linked_text(nested.text, nested.links)
                        table_links.update(key for link in nested.links if (link.uri, link.text, link.metadata.get("start"), link.metadata.get("end")) in emitted and (key := _source_asset(link)) is not None)
                        table_images.update(key for image in nested.images if (key := _source_asset(image)) is not None)
        return "\n\n".join(markdown for index, page in enumerate(self._pages) if (markdown := page._markdown(table_links, table_images, image_reference=image_reference, path=("pages", index)))).strip()

    def export_markdown(self) -> dict[str, bytes]:
        """Return portable Markdown, media and a JSON manifest as owned file bytes."""
        self._ensure_open()
        _check_export_structure(self)
        files: dict[str, bytes] = {}
        assets: dict[str, dict[str, Any]] = {}
        digests: dict[int, str] = {}
        placements: list[dict[str, Any]] = []
        stories: list[dict[str, Any]] = []
        total = 0
        current_file = "document.md"

        def add_file(name: str, value: bytes) -> None:
            nonlocal total
            if len(value) > _MAX_EXPORT_BYTES - total:
                raise UnsafeArchiveError("Markdown export package exceeds 128 MiB")
            total += len(value)
            files[name] = value

        def reference(image: Image, path: tuple) -> str:
            if not image.data:
                raise MalformedDocumentError("Markdown export image has no data")
            if len(image.data) > _MAX_EXPORT_BYTES:
                raise UnsafeArchiveError("Markdown export image exceeds 128 MiB")
            digest = digests.get(id(image.data)) if isinstance(image.data, bytes) else None
            if digest is None:
                digest = hashlib.sha256(image.data).hexdigest()
                if isinstance(image.data, bytes):
                    digests[id(image.data)] = digest
            asset = assets.get(digest)
            if asset is None:
                extension = _MEDIA_EXTENSIONS.get(image.media_type.lower(), "bin")
                name = f"media/{digest}.{extension}"
                if len(image.data) > _MAX_EXPORT_BYTES - total:
                    raise UnsafeArchiveError("Markdown export package exceeds 128 MiB")
                add_file(name, bytes(image.data))
                asset = {"path": name, "sha256": digest, "size": len(image.data), "media_type": image.media_type}
                assets[digest] = asset
            if len(placements) >= _MAX_EXPORT_NODES:
                raise UnsafeArchiveError("Markdown export exceeds 100000 image placements")
            placements.append({"file": current_file, "path": list(path), "asset": asset["path"], "alt": image.alt, "source_name": image.name, "source_page": image.page})
            return ("../" if current_file.startswith("stories/") else "") + asset["path"]

        add_file(current_file, self._markdown(reference).encode("utf-8"))
        for index, story in enumerate(self.stories):
            # Story kinds and source filenames never become archive paths.
            current_file = f"stories/{index + 1:04d}.md"
            stories.append({"file": current_file, "kind": story.kind, "index": index})
            markdown = Page(0, story.blocks)._markdown(image_reference=reference, path=("stories", index))
            add_file(current_file, markdown.encode("utf-8"))
        manifest = {
            "version": 1,
            "document": "document.md",
            "assets": list(assets.values()),
            "placements": placements,
            "stories": stories,
            "limitations": [
                "Nested tables, merged cells and rich table cells use HTML; the reader must support inline HTML.",
                "Auxiliary stories are separate files, not repeated at page positions; legacy comment blocks may also appear in the body.",
                "Media bytes are preserved, not transcoded; unknown media types use .bin and may not display in Markdown readers.",
                "This is extracted content, not page-layout reproduction; source hyperlinks are retained and are not fetched or sanitized.",
            ],
        }
        encoded = bytearray()
        encoder = json.JSONEncoder(ensure_ascii=False, indent=2)
        for chunk in encoder.iterencode(manifest):
            # iterencode may yield an entire string, already bounded by preflight.
            data = chunk.encode("utf-8")
            if len(data) > _MAX_EXPORT_BYTES - total - len(encoded):
                raise UnsafeArchiveError("Markdown export package exceeds 128 MiB")
            encoded.extend(data)
        add_file("manifest.json", bytes(encoded))
        return files

    def render_page(self, page: int, dpi: float = 144, *, clip: tuple[float, float, float, float] | None = None) -> bytes:
        self._ensure_open()
        if page < 0:
            page += len(self._pages)
        if page < 0 or page >= len(self._pages):
            raise IndexError("page index out of range")
        from . import _core, _native
        region = _render_clip(clip)
        session = self._render_session
        if session is not None:
            return bytes(_native(lambda: session.render(page, dpi, region)))
        self._ensure_open()
        return bytes(_native(lambda: _core.render(self._source, self._filetype, page, dpi, self._password, region)))

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
        session = self._render_session
        self._render_session = None
        self._source = b""
        self._password = None
        if session is not None:
            from . import _native
            _native(session.close)

    def _ensure_open(self) -> None:
        if self._closed:
            raise DocumentClosedError("document is closed")

    def __enter__(self) -> Document:
        self._ensure_open()
        return self

    def __exit__(self, exc_type: object, exc: object, traceback: object) -> None:
        self.close()
