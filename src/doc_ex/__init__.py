from __future__ import annotations

import base64
import json
import operator
import sys
from collections.abc import Iterable
from pathlib import Path
from typing import BinaryIO

from . import _core
from ._document import Document, Page
from .errors import (
    DocExError,
    DocumentClosedError,
    MalformedDocumentError,
    ToolExecutionError,
    ToolNotFoundError,
    UnsafeArchiveError,
    UnsupportedFormatError,
)
from .models import Block, Cell, Image, Link, Story, Table
from ._authoring import EditableDocument, new, pt, mm, cm, inches

__all__ = [
    "open", "convert", "extract_text", "new", "pt", "mm", "cm", "inches", "EditableDocument", "Document", "Page", "Block", "Cell", "Table", "Link", "Image", "Story",
    "DocExError", "DocumentClosedError", "MalformedDocumentError", "UnsafeArchiveError",
    "UnsupportedFormatError", "ToolExecutionError", "ToolNotFoundError",
]
__version__ = "0.2.1"


_MAX_INPUT_BYTES = 128 * 1024 * 1024
_MAX_SELECTED_PAGES = 10_000
_MAX_TEXT_OUTPUT_BYTES = 64 * 1024 * 1024


def _read(source, *, limit: int | None = None) -> tuple[bytes, str]:
    limit = _MAX_INPUT_BYTES if limit is None else min(limit, _MAX_INPUT_BYTES)
    if isinstance(source, (bytes, bytearray, memoryview)):
        if (source.nbytes if isinstance(source, memoryview) else len(source)) > limit:
            raise UnsafeArchiveError(f"input exceeds {limit} bytes")
        return bytes(source), ""

    def read_stream(stream):
        data = bytearray()
        while True:
            chunk = stream.read(min(1024 * 1024, limit + 1 - len(data)))
            if not isinstance(chunk, (bytes, bytearray, memoryview)):
                raise TypeError("document streams must return bytes")
            if not chunk:
                return bytes(data)
            if len(data) + (chunk.nbytes if isinstance(chunk, memoryview) else len(chunk)) > limit:
                raise UnsafeArchiveError(f"input exceeds {limit} bytes")
            data.extend(bytes(chunk))

    if hasattr(source, "read"):
        return read_stream(source), ""
    path = Path(source).expanduser()
    if not path.is_file():
        raise FileNotFoundError(path)
    with path.open("rb") as stream:
        return read_stream(stream), path.suffix


def _kind(data: bytes, suffix: str, filetype: str | None) -> str:
    if filetype:
        aliases = {"pdf": "pdf", "docx": "docx", "word": "docx", "pptx": "pptx", "powerpoint": "pptx"}
        normalized = filetype.lower().lstrip(".")
        if normalized not in aliases:
            raise UnsupportedFormatError(f"Unsupported file type: {filetype}")
        kind = aliases[normalized]
        detected = _native(lambda: _core.detect(data))
        if detected != kind:
            raise UnsupportedFormatError(f"Input is {detected}, not {kind}")
        return kind
    if suffix.lower().lstrip(".") in {"pdf", "docx", "pptx"}:
        expected = suffix.lower().lstrip(".")
        detected = _native(lambda: _core.detect(data))
        if detected != expected:
            raise UnsupportedFormatError(f"Input is {detected}, not {expected}")
        return expected
    return _native(lambda: _core.detect(data))


def open(path_or_bytes: str | Path | bytes | bytearray | memoryview | BinaryIO, filetype: str | None = None, password: str | None = None) -> Document:
    """Open PDF, DOCX or PPTX for extraction and rendering; returned models own their data."""
    data, suffix = _read(path_or_bytes)
    kind = _kind(data, suffix, filetype)
    if kind == "pdf":
        render_session, encoded = _native(lambda: _core._open_pdf(data, password))
    else:
        render_session, encoded = _native(lambda: _core._open_office(data, kind, password))
    try:
        payload = json.loads(encoded)
        pages = [_page(value) for value in payload["pages"]]
        metadata = dict(payload.get("metadata", {}))
        metadata.setdefault("format", payload["format"])
        stories = [Story(value["kind"], [_block(block) for block in value.get("blocks", [])], value.get("metadata", {})) for value in payload.get("stories", [])]
        return Document(pages, metadata, data, kind, password, stories=stories, _render_session=render_session)
    except BaseException:
        # A retained traceback must not keep an unreturned native session alive.
        render_session.close()
        raise


def _selected_pages(pages):
    if pages is None:
        return None
    selected = []
    for value in pages:
        if len(selected) == _MAX_SELECTED_PAGES:
            raise UnsafeArchiveError("selected page count exceeds 10000")
        index = operator.index(value)
        if not -sys.maxsize - 1 <= index <= sys.maxsize:
            raise IndexError("page index is out of range")
        selected.append(index)
    return selected


def _bounded_text_join(texts):
    parts = []
    used = 0
    for text in texts:
        used += bool(parts)
        # Bound the temporary UTF-8 encoding too; keep the original owned strings.
        for start in range(0, len(text), 4096):
            used += len(text[start:start + 4096].encode("utf-8"))
            if used > _MAX_TEXT_OUTPUT_BYTES:
                raise UnsafeArchiveError("selected text exceeds the 64 MiB output limit")
        if used > _MAX_TEXT_OUTPUT_BYTES:
            raise UnsafeArchiveError("selected text exceeds the 64 MiB output limit")
        parts.append(text)
    return "\f".join(parts)


def extract_text(
    source: str | Path | bytes | bytearray | memoryview | BinaryIO,
    *,
    filetype: str | None = None,
    password: str | None = None,
    pages: Iterable[int] | None = None,
) -> str:
    """Extract selected text; PDF skips public models, while Office uses eager extraction."""
    indices = _selected_pages(pages)
    data, suffix = _read(source)
    kind = _kind(data, suffix, filetype)
    if kind == "pdf":
        return _native(lambda: _core.extract_text(data, indices, password))
    with open(data, filetype=kind, password=password) as document:
        selected = range(document.page_count) if indices is None else indices
        if len(selected) > _MAX_SELECTED_PAGES:
            raise UnsafeArchiveError("selected page count exceeds 10000")
        return _bounded_text_join(document[index].get_text("text") for index in selected)


def convert(source: str | Path | bytes | bytearray | memoryview | BinaryIO, output_format: str, *, filetype: str | None = None, password: str | None = None) -> bytes:
    """Return converted file bytes for a supported format pair; unsupported conversions raise explicitly."""
    data, suffix = _read(source)
    kind = _kind(data, suffix, filetype)
    target = output_format.lower().lstrip(".")
    if target not in {"pdf", "docx", "pptx"}:
        raise UnsupportedFormatError(f"Unsupported output format: {output_format}")
    return bytes(_native(lambda: _core.convert(data, kind, target, password)))


def _native(operation):
    try:
        return operation()
    except _core.DocExNativeError as error:
        category, _, message = str(error).partition(":")
        exception = {
            "unsupported": UnsupportedFormatError,
            "malformed": MalformedDocumentError,
            "unsafe_archive": UnsafeArchiveError,
            "resource_limit": UnsafeArchiveError,
            "index": IndexError,
            "encrypted": MalformedDocumentError,
            "conversion_failure": DocExError,
            "closed": DocumentClosedError,
        }.get(category, DocExError)
        raise exception(message or str(error)) from None


def _page(value: dict) -> Page:
    links = [_link(item) for item in value.get("links", [])]
    images = [_image(item) for item in value.get("images", [])]
    tables = [_table(item) for item in value.get("tables", [])]
    blocks = [_block(item) for item in value.get("blocks", [])]
    return Page(value["number"], blocks, links=links, images=images, tables=tables, metadata=value.get("metadata", {}))


def _link(value: dict) -> Link:
    return Link(value["uri"], value.get("text", ""), _bbox(value.get("bbox")), value.get("page"), value.get("metadata", {}))


def _image(value: dict) -> Image:
    return Image(value["name"], base64.b64decode(value["data"]), value.get("media_type", "application/octet-stream"), value.get("alt", ""), _bbox(value.get("bbox")), value.get("page"), value.get("metadata", {}))


def _table(value: dict) -> Table:
    cells = [
        [Cell([_block(block) for block in cell.get("blocks", [])], cell.get("col_span", 1), cell.get("row_span", 1), cell.get("covered", False), cell.get("metadata", {})) for cell in row]
        for row in value.get("cells", [])
    ]
    return Table(value.get("rows", []), _bbox(value.get("bbox")), value.get("page"), value.get("metadata", {}), cells)


def _block(value: dict) -> Block:
    return Block(
        value["kind"], value.get("text", ""), _bbox(value.get("bbox")), value.get("level"), value.get("style"),
        [_link(item) for item in value.get("links", [])], [_image(item) for item in value.get("images", [])],
        _table(value["table"]) if value.get("table") else None, value.get("metadata", {}),
    )


def _bbox(value):
    return tuple(value) if value is not None else None
