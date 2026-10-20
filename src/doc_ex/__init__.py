from __future__ import annotations

import base64
import json
from pathlib import Path

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
from .models import Block, Image, Link, Table

__all__ = [
    "open", "convert", "Document", "Page", "Block", "Table", "Link", "Image",
    "DocExError", "DocumentClosedError", "MalformedDocumentError", "UnsafeArchiveError",
    "UnsupportedFormatError", "ToolExecutionError", "ToolNotFoundError",
]
__version__ = "0.2.0"


def _read(source: str | Path | bytes | bytearray | memoryview) -> tuple[bytes, str]:
    if isinstance(source, (bytes, bytearray, memoryview)):
        return bytes(source), ""
    path = Path(source).expanduser()
    if not path.is_file():
        raise FileNotFoundError(path)
    return path.read_bytes(), path.suffix


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


def open(path_or_bytes: str | Path | bytes | bytearray | memoryview, filetype: str | None = None, password: str | None = None) -> Document:
    data, suffix = _read(path_or_bytes)
    kind = _kind(data, suffix, filetype)
    payload = json.loads(_native(lambda: _core.extract(data, kind, password)))
    pages = [_page(value) for value in payload["pages"]]
    metadata = dict(payload.get("metadata", {}))
    metadata.setdefault("format", payload["format"])
    return Document(pages, metadata, data, kind, password)


def convert(source: str | Path | bytes | bytearray | memoryview, output_format: str, *, filetype: str | None = None, password: str | None = None) -> bytes:
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
            "encrypted": MalformedDocumentError,
            "conversion_failure": DocExError,
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
    return Table(value.get("rows", []), _bbox(value.get("bbox")), value.get("page"), value.get("metadata", {}))


def _block(value: dict) -> Block:
    return Block(
        value["kind"], value.get("text", ""), _bbox(value.get("bbox")), value.get("level"), value.get("style"),
        [_link(item) for item in value.get("links", [])], [_image(item) for item in value.get("images", [])],
        _table(value["table"]) if value.get("table") else None, value.get("metadata", {}),
    )


def _bbox(value):
    return tuple(value) if value is not None else None
