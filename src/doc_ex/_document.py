from __future__ import annotations

import base64
import hashlib
import json
import math
import operator
from collections.abc import Iterable, Mapping
from datetime import datetime, timezone
from numbers import Real
from html import escape
from itertools import chain
from types import SimpleNamespace
from typing import Any, Callable, Iterator, Literal, TYPE_CHECKING, overload

from .errors import DocumentClosedError, MalformedDocumentError, UnsafeArchiveError
from .models import (
    Block, EpubAudioClip, EpubChapterSource, EpubEmbeddedMedia, EpubMediaSource, EpubMediaTrack, EpubNarrationCue, EpubOverlayCue, EpubOverlaySequence, EpubResource, EpubSpineProperties, EpubSvgChapterSource, EpubTextCue, EpubTextRegion, EpubTextRun, EpubTextStyle, EpubTtmlRegion, EpubVideoFrame, Image, Link, Story, Table,
    _html_text,
)

if TYPE_CHECKING:
    from ._authoring import PdfDocument


_MAX_EXPORT_BYTES = 128 * 1024 * 1024
_MAX_EXPORT_NODES = 100_000
_MAX_EXPORT_DEPTH = 32
_MAX_EPUB_CHAPTER_BYTES = 8 * 1024 * 1024
_MAX_EPUB_EDIT_BYTES = 128 * 1024 * 1024
_OverlayEdit = tuple[int, bool, str | None, str | None, int, int | None, str | None]
_MEDIA_EXTENSIONS = {
    "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif",
    "image/webp": "webp", "image/tiff": "tiff", "image/bmp": "bmp",
}


def _epub_chunk_size(frames: int) -> int:
    frames = operator.index(frames)
    if not 1 <= frames <= 262_144:
        raise ValueError("EPUB audio chunk_frames must be between 1 and 262144")
    return frames


def _epub_track_index(index: int | None, name: str) -> int | None:
    if index is None:
        return None
    index = operator.index(index)
    if not 0 <= index <= 0xFFFFFFFF:
        raise ValueError(f"{name} track index must be an unsigned 32-bit integer")
    return index


def _epub_playback_interval(start_us: int, end_us: int | None) -> tuple[int, int | None]:
    start = operator.index(start_us)
    end = None if end_us is None else operator.index(end_us)
    if not 0 <= start <= 0xFFFFFFFFFFFFFFFF or (
        end is not None and not start < end <= 0xFFFFFFFFFFFFFFFF
    ):
        raise ValueError("EPUB playback requires a nonnegative start and an optional later end")
    return start, end


def _epub_text_style(native_style: tuple[Any, ...]) -> EpubTextStyle:
    *properties, classes, voice, language = native_style
    return EpubTextStyle(*properties, tuple(classes), voice, language)


class _EpubTextTimeline:
    def __init__(self, cues: list[EpubTextCue]) -> None:
        self._cues = cues
        self._events = [(cue.start_us, True, index) for index, cue in enumerate(cues)]
        self._events.extend(
            (cue.end_us, False, index)
            for index, cue in enumerate(cues)
            if cue.end_us is not None
        )
        self._events.sort()
        self._index = 0
        self._active: set[int] = set()
        self._visible: tuple[EpubTextCue, ...] = ()

    def _advance(self, time: int) -> None:
        previous = self._index
        while self._index < len(self._events) and self._events[self._index][0] <= time:
            _, starts, index = self._events[self._index]
            self._index += 1
            if starts:
                self._active.add(index)
            else:
                self._active.discard(index)
        if self._index != previous:
            if len(self._active) > 128:
                raise UnsafeArchiveError("EPUB video exceeds 128 simultaneous text cues")
            self._visible = tuple(self._cues[index] for index in sorted(self._active))

    def segments(self, start: int, end: int) -> Iterator[tuple[int, int, tuple[EpubTextCue, ...]]]:
        self._advance(start)
        while self._index < len(self._events) and self._events[self._index][0] < end:
            boundary = self._events[self._index][0]
            yield start, boundary, self._visible
            self._advance(boundary)
            start = boundary
        yield start, end, self._visible


def _epub_edit_text(value: str, limit: int, label: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"{label} must be a string")
    value = str.__str__(value)
    if len(value) > limit or len(value.encode("utf-8")) > limit:
        raise UnsafeArchiveError(f"EPUB {label} exceeds its byte budget")
    return value


def _epub_overlay_nodes(
    nodes: Iterable[EpubOverlayCue | EpubOverlaySequence], depth: int = 0,
) -> Iterator[_OverlayEdit]:
    if depth > 32:
        raise UnsafeArchiveError("EPUB media overlay nesting exceeds 32")
    for node in nodes:
        if not isinstance(node, (EpubOverlayCue, EpubOverlaySequence)):
            raise TypeError("overlay entries must be EpubOverlayCue or EpubOverlaySequence values")
        fragment = None if node.fragment is None else _epub_edit_text(node.fragment, 4_096, "cue fragment")
        roles = None if node.epub_type is None else _epub_edit_text(node.epub_type, 1_024, "cue roles")
        if isinstance(node, EpubOverlaySequence):
            yield (depth, True, fragment, None, 0, None, roles)
            yield from _epub_overlay_nodes(node.children, depth + 1)
            continue
        audio = None if node.audio_path is None else _epub_edit_text(node.audio_path, 4_096, "cue audio path")
        begin = operator.index(node.clip_begin_us)
        end = None if node.clip_end_us is None else operator.index(node.clip_end_us)
        if not 0 <= begin < 2**53 or end is not None and not begin < end < 2**53:
            raise ValueError("invalid EPUB media overlay interval")
        yield (depth, False, fragment, audio, begin, end, roles)


def _format_join(parts: list[str], separator: str, skip_empty: bool = False) -> str:
    # Preserve Python's handling of string subclasses and lone surrogates.
    if all(type(part) is str for part in parts):
        from . import _core
        try:
            result = _core._format_join(parts, separator, skip_empty)
            if result is not None:
                return result.strip()
        except UnicodeEncodeError:
            pass
    if skip_empty:
        return separator.join(part for part in parts if part).strip()
    return separator.join(parts).strip()


def _search_query(text: str) -> str:
    if not isinstance(text, str):
        raise TypeError("search text must be a string")
    # Use base str operations: a subclass must not run callbacks during ownership
    # validation or conceal its actual size with overridden encode/len methods.
    if str.__len__(text) > 4096:
        raise UnsafeArchiveError("PDF search query exceeds 4096 UTF-8 bytes")
    text = str.__str__(text)
    try:
        size = len(text.encode("utf-8"))
    except UnicodeEncodeError as error:
        raise ValueError("search text must contain valid Unicode") from error
    if size > 4096:
        raise UnsafeArchiveError("PDF search query exceeds 4096 UTF-8 bytes")
    return text


def _search_empty(text: str) -> bool:
    # Only the common ASCII no-render case is handled locally. Native matching
    # owns all other Unicode whitespace/control semantics and receives the original.
    return not text.strip(" \t\n\r\v\f")


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

    def search_for(self, text: str) -> list[tuple[float, float, float, float]]:
        """Find PDF text as view-point (x0, y0, x1, y1) rectangles.

        Native matching ignores case and normalizes whitespace. Multiline matches
        return one box per line; ligature matches can share a box. Invisible and
        clipped text is included. Boxes describe glyph em extents, not tight ink.
        Native text order can cross columns and differ from normalized table text.
        """
        if self._document is None:
            raise ValueError("page is not attached to a document")
        owner = self._document
        owner._ensure_open()
        text = _search_query(text)
        from .errors import UnsupportedFormatError
        if owner._filetype != "pdf":
            raise UnsupportedFormatError("text search geometry is available only for PDF")
        number = operator.index(self.number)
        owner._ensure_open()
        if self._document is not owner:
            raise ValueError("page ownership changed during search validation")
        if number < 0 or number >= len(owner._pages):
            raise IndexError("page index out of range")
        if _search_empty(text):
            return []
        from . import _core, _native
        session = owner._render_session
        temporary = session is None
        if temporary:
            session, _ = _native(lambda: _core._open_pdf(owner._source, owner._password))
        try:
            return list(_native(lambda: session.search_for(number, text)))
        finally:
            if temporary:
                _native(session.close)

    @overload
    def get_text(self, option: Literal["text", "markdown", "html", "json"] = "text") -> str: ...

    @overload
    def get_text(self, option: Literal["dict"]) -> dict[str, Any]: ...

    @overload
    def get_text(self, option: str) -> str | dict[str, Any]: ...

    def get_text(self, option: str = "text") -> str | dict[str, Any]:
        """Return text, Markdown, HTML or JSON; dict returns normalized content data."""
        if option == "text":
            parts: list[str] = []
            for block in self.blocks:
                if block.table:
                    parts.extend("\t".join(row) for row in block.table.rows)
                elif block.text:
                    parts.append(block.text)
            return _format_join(parts, "\n")
        if option == "markdown":
            return self._markdown()
        if option == "html":
            return self._html()
        if option == "dict":
            return self.to_dict()
        if option == "json":
            return json.dumps(self.to_dict(), ensure_ascii=False)
        raise ValueError("option must be 'text', 'markdown', 'html', 'dict', or 'json'")

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
        return _format_join(parts, "\n\n", True)

    def _html(self) -> str:
        # Apply the same structure and expansion limits as Markdown export.
        _check_export_structure(SimpleNamespace(_pages=[self], stories=[]))
        parts: list[str] = []
        placements: dict[tuple, int] = {}
        image_bytes = 0

        def image_reference(image: Image, path: tuple) -> str:
            nonlocal image_bytes
            if image.media_type not in _MEDIA_EXTENSIONS or not image.data:
                raise MalformedDocumentError("HTML export image has missing data or unsupported media type")
            image_bytes += ((len(image.data) + 2) // 3) * 4
            if image_bytes > _MAX_EXPORT_BYTES:
                raise UnsafeArchiveError("HTML export images exceed the formatting budget")
            key = _image_placement(image)
            placements[key] = placements.get(key, 0) + 1
            return "data:" + image.media_type + ";base64," + base64.b64encode(image.data).decode("ascii")

        def image_html(image: Image, path: tuple) -> str:
            return f'<img src="{escape(image_reference(image, path), quote=True)}" alt="{escape(image.alt, quote=True)}">'

        for number, block in enumerate(self.blocks):
            if block.table:
                parts.append(block.table._to_html(image_reference, ("blocks", number, "table")))
            elif block.text:
                tag = f"h{max(1, min(block.level or 1, 6))}" if block.kind == "heading" else "p"
                parts.append(f"<{tag}>{_html_text(block.text, block.links)}</{tag}>")
            for index, image in enumerate(block.images):
                parts.append(image_html(image, ("blocks", number, "images", index)))
        for index, image in enumerate(self._images):
            key = _image_placement(image)
            if placements.get(key, 0):
                placements[key] -= 1
            else:
                parts.append(image_html(image, ("images", index)))
        result = "\n".join(parts)
        if len(result.encode("utf-8")) > _MAX_EXPORT_BYTES:
            raise UnsafeArchiveError("HTML export exceeds the formatting budget")
        return result

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

    def _page_index(self, number: int) -> int:
        self._ensure_open()
        number = operator.index(number)
        self._ensure_open()
        count = len(self._pages)
        if not -count <= number < count:
            raise IndexError("page index out of range")
        return number % count

    def load_page(self, number: int) -> Page:
        return self._pages[self._page_index(number)]

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
        return _format_join([page._markdown(table_links, table_images, image_reference=image_reference, path=("pages", index)) for index, page in enumerate(self._pages)], "\n\n", True)

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
        page = self._page_index(page)
        from . import _core, _native
        region = _render_clip(clip)
        session = self._render_session
        if session is not None:
            return bytes(_native(lambda: session.render(page, dpi, region)))
        self._ensure_open()
        return bytes(_native(lambda: _core.render(self._source, self._filetype, page, dpi, self._password, region)))

    def render_epub_spread(
        self,
        page: int,
        dpi: float = 144,
        *,
        orientation: Literal["landscape", "portrait"] = "landscape",
        clip: tuple[float, float, float, float] | None = None,
    ) -> bytes:
        """Render the EPUB spread containing a spine page as PNG."""
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("spread rendering requires EPUB input")
        if orientation not in ("landscape", "portrait"):
            raise ValueError("EPUB spread orientation must be landscape or portrait")
        from . import _core, _native

        page = self._page_index(page)
        region = _render_clip(clip)
        self._ensure_open()
        return bytes(_native(lambda: _core._render_epub_spread(
            self._source, page, dpi, orientation == "landscape", region,
        )))

    def export_epub_fixed_pdf(
        self, *, dpi: float = 144, pages: Iterable[int] | None = None, searchable: bool = False,
    ) -> bytes:
        """Export fixed-layout spine pages with bookmarks, links and optional searchable text."""
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("fixed-layout visual PDF export requires EPUB input")
        from . import _core, _native

        if type(searchable) is not bool:
            raise TypeError("searchable must be a bool")
        selected = None
        if pages is not None:
            selected = []
            for page in pages:
                self._ensure_open()
                if len(selected) >= 10_000:
                    raise UnsafeArchiveError("too many EPUB PDF export pages")
                selected.append(self._page_index(page))
        self._ensure_open()
        return bytes(_native(lambda: _core._export_epub_fixed_pdf(self._source, dpi, selected, searchable)))

    def convert(self, output_format: str) -> bytes:
        self._ensure_open()
        from . import convert
        return convert(self._source, output_format, filetype=self._filetype, password=self._password)

    def read_epub_resource(self, path: str, *, offset: int = 0, length: int | None = None) -> bytes:
        """Read up to 16 MiB from a local EPUB member, optionally by byte range."""
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("EPUB resource access requires EPUB input")
        from . import _core, _native

        offset = operator.index(offset)
        length = None if length is None else operator.index(length)
        if offset < 0 or (length is not None and length < 0):
            raise ValueError("resource offset and length must be nonnegative")
        if offset > 128 * 1024 * 1024:
            raise ValueError("resource offset is out of range")
        if length is not None and length > 16 * 1024 * 1024:
            raise UnsafeArchiveError("resource slice exceeds 16 MiB")
        self._ensure_open()
        return bytes(_native(lambda: _core._epub_resource(self._source, path, offset, length)))

    def epub_embedded_media(self, page: int) -> list[EpubEmbeddedMedia]:
        """List the chapter's audio and video elements in XHTML source order."""
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("embedded media requires EPUB input")
        def media_source(source: dict[str, Any]) -> EpubMediaSource:
            fields = dict(source)
            for key in ("fragment_xywh", "fragment_tracks"):
                if key in fields:
                    fields[key] = tuple(fields[key])
            return EpubMediaSource(**fields)

        chapter = self.load_page(operator.index(page))
        return [
            EpubEmbeddedMedia(
                item["kind"], item["id"], item["label"],
                tuple(media_source(source) for source in item["sources"]),
                tuple(EpubMediaTrack(**track) for track in item["tracks"]),
                item["poster_path"],
            )
            for item in chapter.metadata.get("embedded_media", ())
        ]

    def epub_embedded_text(
        self,
        page: int,
        media_index: int,
        *,
        track_index: int | None = None,
        start_us: int = 0,
        end_us: int | None = None,
    ) -> list[EpubTextCue]:
        """Return clipped WebVTT or TTML cues on the media presentation timeline."""
        embedded = self.epub_embedded_media(page)[operator.index(media_index)]
        if track_index is None:
            track = next((track for track in embedded.tracks if track.default), None)
            if track is None and embedded.tracks:
                track = embedded.tracks[0]
        else:
            track = embedded.tracks[operator.index(track_index)]
        start, end = _epub_playback_interval(start_us, end_us)
        if track is None:
            return []
        cues = self._read_epub_text(track, start, end, video_context=embedded.kind == "video")
        cues.sort(key=operator.attrgetter("start_us"))
        return cues

    def _read_epub_text(
        self, track: EpubMediaTrack, start: int, end: int | None, *, video_context: bool = True
    ) -> list[EpubTextCue]:
        from . import _core, _native

        self._ensure_open()
        result = _native(lambda: _core._epub_text_track(
            self._source, track.path, track.media_type, start, end, track.language, video_context
        ))
        cues = []
        for *legacy_fields, native_runs, background_color, native_region, native_ttml in result:
            runs = []
            for text, native_style, timestamp in native_runs:
                runs.append(EpubTextRun(text, _epub_text_style(native_style), timestamp))
            region_layout = None
            if native_region is not None:
                *geometry, native_style, opacity, visibility = native_region
                region_layout = EpubTextRegion(
                    *geometry, style=_epub_text_style(native_style), opacity=opacity, visibility=visibility
                )
            ttml_region_layout = None if native_ttml is None else EpubTtmlRegion(*native_ttml)
            cues.append(EpubTextCue(
                *legacy_fields, runs=tuple(runs), background_color=background_color,
                region_layout=region_layout, ttml_region_layout=ttml_region_layout,
            ))
        return cues

    def _epub_source_interval(
        self, source: EpubMediaSource, start_us: int, end_us: int | None
    ) -> tuple[int, int | None]:
        start, end = _epub_playback_interval(start_us, end_us)
        authored_start, authored_end = source.fragment_start_us, source.fragment_end_us
        if source.fragment_id is not None:
            from . import _core, _native

            chapter = (
                _core._epub_webm_chapter if source.media_type in ("audio/webm", "video/webm")
                else _core._epub_mp4_chapter
            )
            authored_start, authored_end = _native(lambda: chapter(self._source, source.path, source.fragment_id))
        start = max(start, authored_start)
        if authored_end is not None:
            end = authored_end if end is None else min(end, authored_end)
        return start, end

    def epub_embedded_audio(
        self,
        page: int,
        media_index: int,
        *,
        source_index: int | None = None,
        start_us: int = 0,
        end_us: int | None = None,
        chunk_frames: int = 65_536,
        opus_track: int | None = None,
        audio_track: int | None = None,
    ) -> Iterator[EpubAudioClip]:
        """Stream PCM from an XHTML media element with optional WebM audio track selection."""
        from . import _core, _native
        from .errors import UnsupportedFormatError

        embedded = self.epub_embedded_media(page)[operator.index(media_index)]
        sources = embedded.sources if source_index is None else (embedded.sources[operator.index(source_index)],)
        frames = _epub_chunk_size(chunk_frames)
        opus_selection = _epub_track_index(opus_track, "Opus")
        audio_selection = _epub_track_index(audio_track, "WebM audio")
        if opus_selection is not None and audio_selection is not None:
            raise ValueError("Opus and WebM audio track selections are mutually exclusive")
        unsupported_source: UnsupportedFormatError | None = None
        for source in sources:
            if source.path is None or source.media_type is None or not source.playable_audio:
                continue
            if (
                source_index is None and audio_selection is not None
                and source.media_type not in ("audio/webm", "video/webm")
            ):
                continue
            self._ensure_open()
            try:
                start, end = self._epub_source_interval(source, start_us, end_us)
                if end is not None and start >= end:
                    return
                decoder = _native(lambda: _core._open_epub_audio(
                    self._source, source.path, source.media_type, opus_selection, embedded.kind == "video",
                    audio_selection, source.fragment_tracks,
                ))
            except UnsupportedFormatError as error:
                if source_index is not None:
                    raise
                unsupported_source = error
            else:
                yield from self._epub_pcm_chunks(
                    decoder, start, end, frames,
                    clamp_to_duration=source.fragment_end_us is not None or source.fragment_id is not None,
                )
                return
        if unsupported_source is not None:
            raise unsupported_source
        if audio_selection is not None:
            raise UnsupportedFormatError("WebM audio track selection requires a supported local WebM source")
        raise UnsupportedFormatError("embedded audio requires a supported local source")

    def epub_embedded_video(
        self,
        page: int,
        media_index: int,
        *,
        source_index: int | None = None,
        start_us: int = 0,
        end_us: int | None = None,
        text_track: int | None = None,
        video_track: int | None = None,
    ) -> Iterator[EpubVideoFrame]:
        """Read local video frames with authored clips and optional synchronized text cues."""
        from . import _core, _native
        from .errors import UnsupportedFormatError

        embedded = self.epub_embedded_media(page)[operator.index(media_index)]
        if embedded.kind != "video":
            raise UnsupportedFormatError("video playback requires an XHTML video element")
        track = None if text_track is None else embedded.tracks[operator.index(text_track)]
        selection = _epub_track_index(video_track, "video")
        sources = embedded.sources if source_index is None else (embedded.sources[operator.index(source_index)],)
        ordinal_types = ("video/webm", "video/ogg", "application/ogg")
        if source_index is None and selection is not None:
            # An ordinal previously selected WebM automatically. Keep that
            # priority when an earlier candidate is now also eligible.
            sources = tuple(sorted(sources, key=lambda source: source.media_type != "video/webm"))
        unsupported_source: UnsupportedFormatError | None = None
        for source in sources:
            if source.path is None or source.media_type is None or not source.playable_video:
                continue
            if source_index is None and selection is not None and source.media_type not in ordinal_types:
                continue
            self._ensure_open()
            try:
                start, end = self._epub_source_interval(source, start_us, end_us)
                if end is not None and start >= end:
                    return
                decoder = _native(lambda: _core._open_epub_video(
                    self._source, source.path, source.media_type, start, end, selection, source.fragment_tracks,
                ))
                if source.fragment_xywh is not None:
                    unit, *rectangle = source.fragment_xywh
                    _native(lambda: decoder.clip_spatial(unit == "percent", rectangle))
            except UnsupportedFormatError as error:
                if source_index is not None:
                    raise
                unsupported_source = error
            else:
                break
        else:
            if unsupported_source is not None:
                raise unsupported_source
            if selection is not None:
                raise UnsupportedFormatError("video track selection requires a supported local WebM or Ogg source")
            raise UnsupportedFormatError("embedded video requires a supported local MP4, Ogg or WebM source")

        timeline = None if track is None else _EpubTextTimeline(self._read_epub_text(track, start, end))
        while True:
            self._ensure_open()
            result = _native(decoder.next_frame)
            if result is None:
                return
            rgb, width, height, aspect, timestamp, duration = result
            pixels = bytes(rgb)
            if timeline is None:
                yield EpubVideoFrame(pixels, width, height, aspect, timestamp, duration)
            else:
                for begin, finish, visible in timeline.segments(timestamp, timestamp + duration):
                    self._ensure_open()
                    yield EpubVideoFrame(pixels, width, height, aspect, begin, finish - begin, visible)

    def epub_media_cues(self, page: int) -> list[dict[str, Any]]:
        """Return SMIL cues for a spine page, in playback order."""
        return [cue for cue, _ in self._epub_cues(page)]

    def _epub_cues(self, page: int) -> Iterator[tuple[dict[str, Any], tuple[str, ...]]]:
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("media cues require EPUB input")
        index = operator.index(page)
        chapter = self.load_page(index).metadata
        overlay_id = chapter.get("media_overlay")
        if overlay_id is None:
            return
        overlay = self.metadata["media_overlays"][overlay_id]
        stack: list[tuple[dict[str, Any], tuple[str, ...]]] = [(overlay["body"], ())]
        while stack:
            node, parent_roles = stack.pop()
            roles = (*parent_roles, *(node.get("epub_type") or "").split())
            if node["kind"] == "par":
                if node["text"]["path"] == chapter["chapter_path"]:
                    yield {
                        "id": node["id"],
                        "text": dict(node["text"]),
                        "audio": None if node["audio"] is None else dict(node["audio"]),
                        "epub_type": node["epub_type"],
                    }, roles
            else:
                stack.extend((child, roles) for child in reversed(node["children"]))

    def _epub_narration_groups(
        self,
        page: int,
        skip_roles: Iterable[str],
        start_cue: int,
        chunk_frames: int | None,
        opus_tracks: Mapping[str, int] | None,
        audio_tracks: Mapping[str, int] | None,
    ) -> Iterator[Iterator[EpubNarrationCue]]:
        from . import _core, _native

        start_cue = operator.index(start_cue)
        if start_cue < 0:
            raise IndexError("EPUB narration cue index out of range")
        if chunk_frames is not None:
            chunk_frames = _epub_chunk_size(chunk_frames)
        skipped = frozenset(skip_roles)
        opus_selections = dict(opus_tracks) if opus_tracks is not None else {}
        audio_selections = dict(audio_tracks) if audio_tracks is not None else {}
        current_track = None
        decoder = None
        found_start = start_cue == 0
        for index, (cue, roles) in enumerate(self._epub_cues(page)):
            self._ensure_open()
            if index < start_cue:
                continue
            found_start = True
            if skipped.intersection(roles):
                continue
            target = cue["text"]
            args = (cue["id"], target["path"], target["fragment"], roles)
            audio = cue["audio"]
            if audio is None:
                spoken = _native(lambda: _core._epub_narration_text(self._source, target["path"], target["fragment"]))
                yield iter((EpubNarrationCue(*args, text=spoken),))
                continue
            track = (audio["path"], audio["media_type"])
            if track != current_track:
                opus_selection = _epub_track_index(opus_selections.get(track[0]), "Opus")
                audio_selection = _epub_track_index(audio_selections.get(track[0]), "WebM audio")
                decoder = None
                decoder = _native(lambda: _core._open_epub_audio(
                    self._source, *track, opus_selection, audio_track=audio_selection,
                ))
                current_track = track
            yield self._epub_audio_group(
                args, decoder, audio["clip_begin_us"], audio["clip_end_us"], chunk_frames,
            )
        if not found_start:
            raise IndexError("EPUB narration cue index out of range")

    def _epub_audio_group(
        self,
        args: tuple[str | None, str, str | None, tuple[str, ...]],
        decoder: Any,
        begin: int,
        end: int | None,
        chunk_frames: int | None,
    ) -> Iterator[EpubNarrationCue]:
        for clip in self._epub_pcm_chunks(decoder, begin, end, chunk_frames, clamp_to_duration=True):
            yield EpubNarrationCue(*args, audio=clip)

    def _epub_pcm_chunks(
        self,
        decoder: Any,
        begin: int,
        end: int | None,
        chunk_frames: int | None,
        *,
        clamp_to_duration: bool = False,
    ) -> Iterator[EpubAudioClip]:
        from . import _native

        self._ensure_open()
        if chunk_frames is None:
            wave, rate, channels, frames = _native(lambda: decoder.clip(begin, end, clamp_to_duration))
            yield EpubAudioClip(bytes(wave), rate, channels, frames)
            return
        _native(lambda: decoder.start_stream(begin, end, clamp_to_duration))
        while True:
            self._ensure_open()
            chunk = _native(lambda: decoder.next_chunk(chunk_frames))
            if chunk is None:
                return
            wave, rate, channels, frames = chunk
            yield EpubAudioClip(bytes(wave), rate, channels, frames)

    def epub_narration(
        self,
        page: int,
        *,
        skip_roles: Iterable[str] = (),
        start_cue: int = 0,
        chunk_frames: int | None = None,
        opus_tracks: Mapping[str, int] | None = None,
        audio_tracks: Mapping[str, int] | None = None,
    ) -> Iterator[EpubNarrationCue]:
        """Read cues with optional chunking and zero-based Ogg Opus or WebM audio selections."""
        for group in self._epub_narration_groups(
            page, skip_roles, start_cue, chunk_frames, opus_tracks, audio_tracks,
        ):
            yield from group

    def play_epub_narration(
        self,
        page: int,
        *,
        play_audio: Callable[[EpubAudioClip], None],
        speak_text: Callable[[str], None],
        on_cue: Callable[[EpubNarrationCue | None], None] | None = None,
        skip_roles: Iterable[str] = (),
        start_cue: int = 0,
        chunk_frames: int | None = None,
        opus_tracks: Mapping[str, int] | None = None,
        audio_tracks: Mapping[str, int] | None = None,
    ) -> None:
        """Play cues through blocking sinks with optional Ogg Opus or WebM audio selections."""
        for group in self._epub_narration_groups(
            page, skip_roles, start_cue, chunk_frames, opus_tracks, audio_tracks,
        ):
            first = next(group)
            if on_cue is not None:
                on_cue(first)
            try:
                for cue in chain((first,), group):
                    self._ensure_open()
                    if cue.audio is not None:
                        play_audio(cue.audio)
                    elif cue.text:
                        speak_text(cue.text)
            finally:
                if on_cue is not None:
                    on_cue(None)

    def edit_epub_metadata(self, *, title: str | None = None, description: str | None = None) -> bytes:
        """Return an EPUB copy with updated package metadata and unchanged publication resources."""
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("EPUB metadata editing requires EPUB input")
        from . import _core, _native

        return bytes(_native(lambda: _core._edit_epub_metadata(self._source, title, description)))

    def edit_epub_chapters(self, replacements: Mapping[int, str]) -> bytes:
        """Replace XHTML or static SVG chapters by spine index and return a new EPUB."""
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("chapter editing requires EPUB input")
        from . import _core, _native

        changes, _ = self._epub_chapter_replacements(replacements)
        self._ensure_open()
        return bytes(_native(lambda: _core._edit_epub_chapters(self._source, changes)))

    def _epub_chapter_replacements(self, replacements: Mapping[int, str]) -> tuple[list[tuple[int, str]], int]:
        changes: list[tuple[int, str]] = []
        total_bytes = 0
        page_count = len(self._pages)
        for page, markup in replacements.items():
            if len(changes) >= 10_000:
                raise UnsafeArchiveError("too many EPUB chapter replacements")
            index = operator.index(page)
            self._ensure_open()
            if not -page_count <= index < page_count:
                raise IndexError("page index out of range")
            if not isinstance(markup, str):
                raise TypeError("replacement chapter markup must be a string")
            markup = str.__str__(markup)
            if len(markup) > _MAX_EPUB_CHAPTER_BYTES:
                raise UnsafeArchiveError("replacement chapter exceeds its XML byte budget")
            byte_count = len(markup.encode("utf-8"))
            if byte_count > _MAX_EPUB_CHAPTER_BYTES:
                raise UnsafeArchiveError("replacement chapter exceeds its XML byte budget")
            total_bytes += byte_count
            if total_bytes > _MAX_EPUB_EDIT_BYTES:
                raise UnsafeArchiveError("chapter replacements exceed their byte budget")
            changes.append((index, markup))
        return changes, total_bytes

    def edit_epub_structure(
        self,
        order: Iterable[int | EpubChapterSource | EpubSvgChapterSource] | None = None,
        *,
        replacements: Mapping[int, str] | None = None,
        spine_properties: Mapping[int, EpubSpineProperties] | None = None,
        resources: Mapping[str, EpubResource] | None = None,
        remove_resources: Iterable[str] = (),
        overlays: Mapping[str, Iterable[EpubOverlayCue | EpubOverlaySequence] | None] | None = None,
    ) -> bytes:
        """Stage chapter replacements by original index alongside spine, resource and narration edits.

        To replace a packaged EPUB 3 navigation document outside the spine, pass
        its archive path and an XHTML resource in `resources`. A spine-linked
        NCX uses its archive path and an NCX resource in the same mapping.
        Use `spine_properties` with original indices to edit retained entries.
        """
        self._ensure_open()
        if self._filetype != "epub":
            from .errors import UnsupportedFormatError

            raise UnsupportedFormatError("spine editing requires EPUB input")
        from . import _core, _native

        chapters, total_bytes = ([], 0) if replacements is None else self._epub_chapter_replacements(replacements)
        selected: list[tuple[int | None, tuple[str, str, str | None, bool, bool, bool] | None]] | None = None
        if order is not None:
            selected = []
            count = len(self._pages)
            for entry in order:
                self._ensure_open()
                if len(selected) >= 10_000:
                    raise UnsafeArchiveError("too many EPUB spine entries")
                if isinstance(entry, (EpubChapterSource, EpubSvgChapterSource)):
                    title = _epub_edit_text(entry.title, 1_024, "chapter title")
                    if isinstance(entry, EpubSvgChapterSource):
                        markup = _epub_edit_text(entry.svg, _MAX_EPUB_CHAPTER_BYTES, "chapter SVG")
                        fixed, svg = True, True
                    else:
                        markup = _epub_edit_text(entry.xhtml, _MAX_EPUB_CHAPTER_BYTES, "chapter XHTML")
                        if type(entry.fixed_layout) is not bool:
                            raise TypeError("fixed_layout must be a bool")
                        fixed, svg = entry.fixed_layout, False
                    path = None if entry.path is None else _epub_edit_text(entry.path, 4_096, "chapter path")
                    total_bytes += len(title.encode()) + len(markup.encode())
                    if path is not None:
                        total_bytes += len(path.encode())
                    if type(entry.linear) is not bool:
                        raise TypeError("linear must be a bool")
                    selected.append((None, (title, markup, path, fixed, svg, entry.linear)))
                else:
                    index = operator.index(entry)
                    self._ensure_open()
                    if not -count <= index < count:
                        raise IndexError("page index out of range")
                    selected.append((index, None))
                if total_bytes > _MAX_EPUB_EDIT_BYTES:
                    raise UnsafeArchiveError("EPUB edits exceed their byte budget")

        property_edits: list[tuple[int, bool | None, str | None, str | None, str | None]] = []
        if spine_properties is not None:
            for index, properties in spine_properties.items():
                self._ensure_open()
                if len(property_edits) >= 10_000:
                    raise UnsafeArchiveError("too many EPUB spine property edits")
                index = operator.index(index)
                if not -len(self._pages) <= index < len(self._pages):
                    raise IndexError("page index out of range")
                if not isinstance(properties, EpubSpineProperties):
                    raise TypeError("spine property edits require EpubSpineProperties values")
                linear = properties.linear
                if linear is not None and type(linear) is not bool:
                    raise TypeError("linear must be a bool or None")
                layout = properties.layout
                page_spread = properties.page_spread
                spread = properties.spread
                property_edits.append((
                    index, linear,
                    None if layout is None else _epub_edit_text(layout, 32, "spine layout"),
                    None if page_spread is None else _epub_edit_text(page_spread, 32, "spine page spread"),
                    None if spread is None else _epub_edit_text(spread, 32, "spine synthetic spread"),
                ))

        changes: list[tuple[str, str, bytes]] = []
        if resources is not None:
            for path, resource in resources.items():
                self._ensure_open()
                if len(changes) >= 10_000:
                    raise UnsafeArchiveError("too many EPUB resource edits")
                if not isinstance(resource, EpubResource) or not isinstance(resource.data, bytes):
                    raise TypeError("resource edits require EpubResource values with bytes")
                name = _epub_edit_text(path, 4_096, "resource path")
                media = _epub_edit_text(resource.media_type, 255, "resource media type")
                total_bytes += len(name.encode()) + len(media.encode()) + bytes.__len__(resource.data)
                if total_bytes > _MAX_EPUB_EDIT_BYTES:
                    raise UnsafeArchiveError("EPUB edits exceed their byte budget")
                changes.append((name, media, resource.data))
        removed: list[str] = []
        for path in remove_resources:
            self._ensure_open()
            if len(removed) >= 10_000:
                raise UnsafeArchiveError("too many EPUB resource removals")
            path = _epub_edit_text(path, 4_096, "removed resource path")
            total_bytes += len(path.encode())
            if total_bytes > _MAX_EPUB_EDIT_BYTES:
                raise UnsafeArchiveError("EPUB edits exceed their byte budget")
            removed.append(path)

        narration: list[tuple[str, list[_OverlayEdit] | None]] = []
        node_count = 0
        if overlays is not None:
            for path, cues in overlays.items():
                self._ensure_open()
                if len(narration) >= 10_000:
                    raise UnsafeArchiveError("too many EPUB media overlay edits")
                chapter = _epub_edit_text(path, 4_096, "media overlay chapter path")
                total_bytes += len(chapter.encode())
                encoded = None
                if cues is not None:
                    encoded = []
                    for entry in _epub_overlay_nodes(cues):
                        self._ensure_open()
                        if node_count >= 10_000:
                            raise UnsafeArchiveError("too many EPUB media overlay entries")
                        _, _, fragment, audio, _, _, roles = entry
                        total_bytes += sum(len(value.encode()) for value in (fragment, audio, roles) if value is not None)
                        if total_bytes > _MAX_EPUB_EDIT_BYTES:
                            raise UnsafeArchiveError("EPUB edits exceed their byte budget")
                        encoded.append(entry)
                        node_count += 1
                if total_bytes > _MAX_EPUB_EDIT_BYTES:
                    raise UnsafeArchiveError("EPUB edits exceed their byte budget")
                narration.append((chapter, encoded))

        self._ensure_open()
        modified = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
        return bytes(_native(lambda: _core._edit_epub_structure(
            self._source, selected, changes, modified, removed, narration, chapters, property_edits,
        )))

    def edit(self) -> PdfDocument:
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
