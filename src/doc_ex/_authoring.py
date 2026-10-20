"""Small Python authoring facade over owned native document models."""
from __future__ import annotations

import json
import math
import operator
import reprlib
import os
import stat
import tempfile
from collections.abc import Iterable, Iterator, Sequence
from numbers import Real
from pathlib import Path
from typing import Any, BinaryIO, Generic, Literal, Self, SupportsIndex, TYPE_CHECKING, TypeVar, TypedDict, Unpack, overload

from . import _core
from .errors import DocumentClosedError, UnsupportedFormatError

if TYPE_CHECKING:
    from ._document import Document

_Source = str | os.PathLike[str] | bytes | bytearray | memoryview | BinaryIO
_Size = str | tuple[float, float] | list[float]
_Box = tuple[float, float, float, float] | list[float]
_Color = str | tuple[int, int, int] | list[int]
_Rows = SupportsIndex | Iterable[Iterable[str]]



class _CharacterProperties(TypedDict, total=False):
    bold: bool | None
    italic: bool | None
    font_size: float | None
    font: str | None
    language: str | None
    color: str | tuple[int, int, int] | None
    underline: bool | str | None
    strike: bool | str | None
    script: str | None
    highlight: str | tuple[int, int, int] | None
    background: str | tuple[int, int, int] | None


class CharacterFormatting(_CharacterProperties, total=False):
    style: str | None


class TabStop(TypedDict, total=False):
    position: float
    alignment: str
    leader: str


class ParagraphFormatting(TypedDict, total=False):
    direction: Literal["ltr", "rtl"] | None
    borders: dict[str, BorderFormatting | None] | None
    tabs: list[TabStop] | None
    alignment: str | None
    indent_left: float | None
    indent_right: float | None
    indent_first: float | None
    space_before: float | None
    space_after: float | None
    line_spacing: float | None
    keep_with_next: bool | None
    keep_together: bool | None
    widow_control: bool | None
    page_break_before: bool | None
    background: str | tuple[int, int, int] | None


class _StyleProperties(_CharacterProperties, ParagraphFormatting, total=False):
    pass


class StyleFormatting(_StyleProperties, total=False):
    vertical_alignment: str | None
    margins: float | tuple[float, float, float, float] | None
    row_band_size: int | None


class NoteOptions(TypedDict, total=False):
    placement: str | None
    numbering: str | None
    start: int | None
    restart: str | None


class BorderFormatting(TypedDict, total=False):
    style: str
    color: str | tuple[int, int, int] | None
    width: float


class TableLook(TypedDict, total=False):
    header_row: bool
    total_row: bool
    banded_rows: bool
    first_column: bool
    last_column: bool
    banded_columns: bool


class TableFormatting(TypedDict, total=False):
    style: str | Style | None
    look: TableLook | None
    alignment: str | None
    indent: float | None
    width: float | None
    width_percent: float | None
    layout: str
    rtl: bool
    title: str | None
    description: str | None
    borders: dict[str, BorderFormatting | None] | None


class CellFormatting(TypedDict, total=False):
    background: str | tuple[int, int, int] | None
    margins: float | tuple[float, float, float, float] | None
    alignment: str | None
    vertical_alignment: str | None
    text_direction: str | None
    no_wrap: bool
    borders: dict[str, BorderFormatting | None] | None


def _table_format_patch(values, *, cell: bool = False) -> str:
    allowed = CellFormatting.__annotations__ if cell else TableFormatting.__annotations__
    if values.keys() - allowed.keys():
        raise ValueError("unknown cell/table formatting property")
    patch = {}
    for key, value in values.items():
        if key == "look" and value is not None:
            if not isinstance(value, dict) or value.keys() - TableLook.__annotations__.keys():
                raise ValueError("look must contain supported table look flags")
            if any(not isinstance(flag, bool) for flag in value.values()):
                raise TypeError("table look flags must be bool values")
            patch[key] = dict(value)
            continue
        if key == "borders" and value is not None:
            if not isinstance(value, dict) or len(value) > 6:
                raise TypeError("borders must be a mapping of at most six sides")
            sides = {"top", "right", "bottom", "left"}
            if not cell:
                sides |= {"inside_horizontal", "inside_vertical"}
            if value.keys() - sides:
                raise ValueError("unsupported border side")
            borders = {}
            for side, line in value.items():
                if line is None:
                    borders[side] = None
                    continue
                if not isinstance(line, dict) or line.keys() - BorderFormatting.__annotations__.keys():
                    raise TypeError("each border must contain only style, color and width")
                border = {}
                for prop, item in line.items():
                    if prop == "color":
                        item = None if item is None else _hex_color(item)
                    elif prop == "width":
                        item = pt(item)
                    elif not isinstance(item, str) or len(item) > 32:
                        raise ValueError("border style must be a short native style name")
                    border[prop] = item
                borders[side] = border
            patch[key] = borders
            continue
        if key == "style":
            if value is not None and (not isinstance(value, str) or len(value) > 1024):
                raise ValueError("style must be a bounded style name/ID or None")
        elif key in {"title", "description"}:
            if value is not None and (not isinstance(value, str) or len(value) > 10000):
                raise ValueError("table title/description must be strings of at most 10000 characters or None")
        elif key == "background" and value is not None:
            value = _hex_color(value)
        elif key == "margins" and value is not None:
            value = _margins(value)
        elif key in {"indent", "width", "width_percent"} and value is not None:
            value = pt(value)
        elif value is not None:
            if not isinstance(value, (str, bool)):
                raise TypeError("formatting values must match the property's scalar type")
            if isinstance(value, str) and len(value) > 128:
                raise ValueError("formatting enum exceeds its limit")
        patch[key] = value
    encoded = json.dumps(patch, ensure_ascii=False, allow_nan=False)
    if len(encoded.encode("utf-8")) > 128 * 1024:
        raise ValueError("table formatting patch exceeds 128 KiB")
    return encoded


def _format_patch(values, *, rgb_highlight: bool = False) -> str:
    # Preserve omission versus explicit None; native adapters own allowed keys.
    if len(values) > 32:
        raise ValueError("too many formatting properties")
    patch = {}
    for key, value in values.items():
        if key == "borders":
            patch.update(json.loads(_table_format_patch({key: value}, cell=True)))
            continue
        if key == "tabs" and value is not None:
            if not isinstance(value, (list, tuple)) or len(value) > 64:
                raise ValueError("tabs must contain at most 64 tab-stop mappings")
            stops = []
            for stop in value:
                if not isinstance(stop, dict) or stop.keys() - TabStop.__annotations__.keys() or "position" not in stop:
                    raise ValueError("each tab requires position and optional alignment/leader")
                item = {"position": pt(stop["position"])}
                for name in ("alignment", "leader"):
                    if name in stop:
                        if not isinstance(stop[name], str) or len(stop[name]) > 32:
                            raise ValueError("tab alignment and leader must be supported names")
                        item[name] = stop[name]
                stops.append(item)
            patch[key] = stops
            continue
        if isinstance(value, str) and len(value) > 16384:
            raise ValueError("formatting value exceeds the patch limit")
        if (key in {"color", "background"} or (rgb_highlight and key == "highlight")) and value is not None:
            value = "auto" if value == "auto" else _hex_color(value)
        if value is not None and not isinstance(value, (str, bool, int, float)):
            raise TypeError("formatting values must be scalar values or None")
        patch[key] = value
    encoded = json.dumps(patch, ensure_ascii=False, allow_nan=False)
    if len(encoded.encode("utf-8")) > 16384:
        raise ValueError("formatting patch exceeds 16 KiB")
    return encoded


def _style_format_patch(values) -> str:
    extras = {}
    scalars = dict(values)
    if "borders" in scalars:
        extras.update(json.loads(_table_format_patch({"borders": scalars.pop("borders")})))
    if "margins" in scalars:
        margins = scalars.pop("margins")
        extras["margins"] = None if margins is None else _margins(margins)
    patch = json.loads(_format_patch(scalars))
    patch.update(extras)
    encoded = json.dumps(patch, ensure_ascii=False, allow_nan=False)
    if len(encoded.encode("utf-8")) > 16384:
        raise ValueError("style formatting patch exceeds 16 KiB")
    return encoded


def _style_reference(value, owner):
    if isinstance(value, Style):
        if value._owner is not owner:
            raise ValueError("style must belong to this document")
        value = value.id
    if value is not None and not isinstance(value, str):
        raise TypeError("style must be a name, ID or Style from this document")
    return value

def pt(value: Real) -> float:
    """Return a finite length in points (1/72 inch)."""
    if isinstance(value, bool) or not isinstance(value, Real):
        raise TypeError("length must be a number in points")
    result = float(value)
    if not math.isfinite(result):
        raise ValueError("length must be finite")
    return result


def inches(value: Real) -> float:
    """Convert inches to points."""
    return pt(pt(value) * 72)


def mm(value: Real) -> float:
    """Convert millimeters to points."""
    return pt(pt(value) * 72 / 25.4)


def cm(value: Real) -> float:
    """Convert centimeters to points."""
    return pt(pt(value) * 72 / 2.54)


def _size(value) -> tuple[float, float]:
    if isinstance(value, str):
        sizes = {"a4": (mm(210), mm(297)), "letter": (612.0, 792.0), "legal": (612.0, 1008.0)}
        try:
            return sizes[value.lower()]
        except KeyError:
            raise ValueError("size must be A4, Letter, Legal or a (width, height) pair in points") from None
    if not isinstance(value, (tuple, list)) or len(value) != 2:
        raise TypeError("size must be a (width, height) pair in points")
    width, height = map(pt, value)
    if width <= 0 or height <= 0:
        raise ValueError("page dimensions must be positive")
    return width, height


def _color(value) -> tuple[int, int, int]:
    if isinstance(value, str):
        colors = {"black": "000000", "white": "FFFFFF", "red": "FF0000", "green": "008000", "blue": "0000FF", "gray": "808080", "grey": "808080", "yellow": "FFFF00", "navy": "000080"}
        value = colors.get(value.lower(), value.removeprefix("#"))
        if len(value) != 6 or any(c not in "0123456789abcdefABCDEF" for c in value):
            raise ValueError("color must be #RRGGBB, a supported color name or an RGB tuple")
        return tuple(int(value[index:index + 2], 16) for index in (0, 2, 4))
    if not isinstance(value, (tuple, list)) or len(value) != 3:
        raise TypeError("color must be an RGB tuple; alpha is not supported by this operation")
    if any(isinstance(c, bool) or not isinstance(c, int) or not 0 <= c <= 255 for c in value):
        raise ValueError("RGB channels must be integers between 0 and 255")
    return tuple(value)


def _hex_color(value) -> str:
    return "#" + "".join(f"{channel:02X}" for channel in _color(value))


def _index(index, length: int) -> int:
    index = operator.index(index)
    if index < 0:
        index += length
    if not 0 <= index < length:
        raise IndexError("index out of range")
    return index


def _picture_crop(value):
    if value is None:
        value = (0, 0, 0, 0)
    if not isinstance(value, (tuple, list)) or len(value) != 4:
        raise TypeError("crop must be (left, top, right, bottom) fractions or None")
    values = tuple(map(pt, value))
    if any(fraction < 0 or fraction > 0.45 for fraction in values):
        raise ValueError("crop fractions must be between 0 and 0.45")
    return values


def _rows(rows, columns: int | None = None) -> list[list[str]]:
    if columns is not None:
        if isinstance(rows, bool) or isinstance(columns, bool):
            raise TypeError("row and column counts must be integers")
        count, columns = operator.index(rows), operator.index(columns)
        if not 1 <= count <= 10000 or not 1 <= columns <= 1000 or count * columns > 100000:
            raise ValueError("table counts require 1..10000 rows, 1..1000 columns and at most 100000 cells")
        return [[""] * columns for _ in range(count)]
    if isinstance(rows, (str, bytes)):
        raise TypeError("table rows must be a sequence of rows of strings")
    result = []
    total = 0
    for row in rows:
        if len(result) >= 10000:
            raise ValueError("table must not exceed 10000 rows")
        if isinstance(row, (str, bytes)):
            raise TypeError("each table row must be a sequence of strings")
        cells = []
        for cell in row:
            if len(cells) >= 1000:
                raise ValueError("table must not exceed 1000 columns")
            if not isinstance(cell, str):
                raise TypeError("table cells must be strings; format values before insertion")
            cells.append(cell)
            total += 1
            if total > 100000:
                raise ValueError("table must not exceed 100000 cells")
        result.append(cells)
    if not result or not result[0] or any(len(row) != len(result[0]) for row in result):
        raise ValueError("table must have nonempty rectangular rows")
    if len(result) * len(result[0]) > 100000:
        raise ValueError("table must not exceed 100000 cells")
    return result


@overload
def new(filetype: Literal["docx", "word"], *, size: _Size | None = None) -> WordDocument: ...

@overload
def new(filetype: Literal["pdf"], *, size: _Size | None = None) -> PdfDocument: ...

@overload
def new(filetype: Literal["pptx", "powerpoint"], *, size: _Size | None = None) -> Presentation: ...

@overload
def new(filetype: str, *, size: _Size | None = None) -> EditableDocument: ...


def new(filetype: str, *, size: _Size | None = None) -> EditableDocument:
    """Create a native DOCX, PDF or PPTX document; dimensions use points."""
    from . import _native
    kind = filetype.lower().lstrip(".")
    kind = {"word": "docx", "powerpoint": "pptx"}.get(kind, kind)
    if kind == "docx":
        native = _native(_core.EditableDocx)
        if size is not None:
            _native(lambda: native.set_page_size(*_size(size)))
        return WordDocument(native)
    if kind == "pdf":
        return PdfDocument(_native(lambda: _core.EditablePdf(*_size("Letter" if size is None else size))))
    if kind == "pptx":
        return Presentation(_native(lambda: _core.EditablePptx(*_size((720, 540) if size is None else size))))
    raise UnsupportedFormatError(f"Cannot create file type: {filetype}")


class EditableDocument:
    """Native document owner with explicit serialization, copying and lifetime."""
    _format: str

    def __init__(self, native):
        self._native = native
        self._closed = False
        self._cached = None
        self._page_generation = 0
        self._annotation_generations = {}
        self._bookmark_generation = 0
        self._field_generation = 0

    def _ensure_open(self):
        if self._closed:
            raise DocumentClosedError("document is closed")

    def _call(self, name, *args, _change=False, **kwargs):
        from . import _native
        self._ensure_open()
        result = _native(lambda: getattr(self._native, name)(*args, **kwargs))
        if _change:
            self._invalidate()
        return result

    def _invalidate(self):
        if self._cached is not None:
            self._cached.close()
            self._cached = None

    def _atomic(self, operation):
        from . import _native
        staged = self._call("copy")
        try:
            result = _native(lambda: operation(staged))
        except BaseException:
            staged.close()
            raise
        self._native.close()
        self._native = staged
        self._invalidate()
        return result

    def _snapshot(self):
        from . import open
        self._ensure_open()
        if self._cached is None:
            self._cached = open(self.to_bytes(), filetype=self._format)
        return self._cached

    @property
    def filetype(self) -> str:
        return self._format

    @property
    def closed(self) -> bool:
        return self._closed

    @property
    def page_count(self) -> int:
        return self._snapshot().page_count

    def to_bytes(self) -> bytes:
        """Serialize the native format through its upstream writer."""
        return bytes(self._call("to_bytes"))

    def save(self, destination: str | Path | BinaryIO) -> None:
        """Save the native format; path replacement is atomic, stream writes are not."""
        self._ensure_open()
        if hasattr(destination, "write"):
            data = self.to_bytes()
            if destination.write(data) != len(data):
                raise OSError("destination did not accept the complete document")
            return
        path = Path(destination).expanduser()
        if path.suffix and path.suffix.lower() != "." + self._format:
            raise ValueError(f"save writes {self._format}; use convert() to change formats")
        data = self.to_bytes()
        previous_mode = stat.S_IMODE(path.stat().st_mode) if path.exists() else None
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(mode="wb", dir=path.parent, prefix=f".{path.name}.", suffix=".tmp", delete=False) as output:
                temporary = Path(output.name)
                output.write(data)
            if previous_mode is not None:
                os.chmod(temporary, previous_mode)
            os.replace(temporary, path)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)

    def copy(self) -> Self:
        """Return an independent native model; child handles retain their own owner."""
        return type(self)(self._call("copy"))

    def render_page(self, index: int, dpi: float = 144, *, clip: tuple[float, float, float, float] | None = None) -> bytes:
        return self._snapshot().render_page(index, dpi=dpi, clip=clip)

    def convert(self, output_format: str) -> bytes:
        from . import convert
        return convert(self.to_bytes(), output_format, filetype=self._format)

    def to_markdown(self) -> str:
        return self._snapshot().to_markdown()

    def close(self) -> None:
        if not self._closed:
            self._native.close()
            self._invalidate()
            self._closed = True

    def __enter__(self) -> Self:
        self._ensure_open()
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def __repr__(self):
        return f"{type(self).__name__}(filetype={self._format!r}, closed={self.closed})"


_Item = TypeVar("_Item")


class _Collection(Sequence[_Item], Generic[_Item]):
    def __init__(self, owner, ids, factory):
        self._owner, self._ids, self._factory = owner, ids, factory

    def __len__(self) -> int:
        self._owner._ensure_open()
        return len(self._ids())

    def __iter__(self) -> Iterator[_Item]:
        self._owner._ensure_open()
        items = [self._factory(self._owner, identifier) for identifier in self._ids()]
        for item in items:
            self._owner._ensure_open()
            yield item

    @overload
    def __getitem__(self, index: int) -> _Item: ...

    @overload
    def __getitem__(self, index: slice) -> list[_Item]: ...

    def __getitem__(self, index):
        self._owner._ensure_open()
        ids = self._ids()
        if isinstance(index, slice):
            return [self._factory(self._owner, identifier) for identifier in ids[index]]
        return self._factory(self._owner, ids[_index(index, len(ids))])


class WordDocument(EditableDocument):
    _format = "docx"

    @property
    def metadata(self) -> dict:
        """Owned core properties and a nested mapping of typed custom properties."""
        return json.loads(self._call("metadata"))

    def update_metadata(self, **values) -> None:
        """Patch supported properties; None clears a field or custom entry."""
        allowed = {"title", "author", "subject", "keywords", "description", "category",
                   "last_modified_by", "created", "modified", "revision", "custom"}
        if values.keys() - allowed:
            raise ValueError("unsupported DOCX metadata key")

        def bounded_string(value, limit):
            if len(value) > limit or len(value.encode("utf-8")) > limit:
                raise ValueError(f"metadata string exceeds {limit} UTF-8 bytes")

        for key, value in values.items():
            if value is None:
                continue
            if key == "revision":
                if not isinstance(value, int) or isinstance(value, bool) or not 0 <= value <= 0xFFFFFFFF:
                    raise ValueError("revision requires an unsigned 32-bit integer or None")
            elif key == "custom":
                if not isinstance(value, dict):
                    raise TypeError("custom properties require a dictionary or None")
                if len(value) > 1024:
                    raise ValueError("custom properties exceed 1024 entries")
                for name, item in value.items():
                    if not isinstance(name, str):
                        raise TypeError("custom property names must be strings")
                    bounded_string(name, 255)
                    if isinstance(item, str):
                        bounded_string(item, 65536)
                    elif item is None or isinstance(item, bool):
                        pass
                    elif isinstance(item, int):
                        if not -(1 << 63) <= item < (1 << 63):
                            raise ValueError("custom integers require signed 64-bit values")
                    elif isinstance(item, float):
                        if not math.isfinite(item):
                            raise ValueError("custom floating-point values must be finite")
                    else:
                        raise TypeError("custom values require str, bool, int, float or None")
            else:
                if not isinstance(value, str):
                    raise TypeError("core metadata values require strings or None")
                bounded_string(value, 65536)
        # Each scalar is bounded before encoding; stop before building an oversized patch.
        chunks, used = [], 0
        encoder = json.JSONEncoder(ensure_ascii=False, allow_nan=False, separators=(",", ":"))
        for chunk in encoder.iterencode(values):
            used += len(chunk.encode("utf-8"))
            if used > 256 * 1024:
                raise ValueError("metadata update exceeds 256 KiB")
            chunks.append(chunk)
        self._call("update_metadata", "".join(chunks), _change=True)

    @property
    def notes(self) -> Notes:
        return Notes(self)

    @property
    def styles(self) -> Styles:
        return Styles(self)

    @property
    def paragraphs(self) -> _Collection[Paragraph]:
        return _Collection(self, lambda: self._call("paragraph_ids"), Paragraph)

    @property
    def tables(self) -> _Collection[EditableTable]:
        return _Collection(self, lambda: self._call("table_ids"), EditableTable)

    @property
    def sections(self) -> Sections:
        return Sections(self)

    @property
    def images(self) -> Pictures:
        return Pictures(self, lambda: self._call("image_ids"))

    @property
    def background(self) -> str | None:
        return self._call("background")

    @background.setter
    def background(self, color):
        self._call("set_background", None if color is None else _hex_color(color), _change=True)

    @property
    def background_image(self) -> bytes | None:
        """Owned PNG bytes stretched over every page, above the background color."""
        value = self._call("background_image")
        return None if value is None else bytes(value)

    @background_image.setter
    def background_image(self, source) -> None:
        """Set a stretched PNG, or clear only the image with None."""
        self._ensure_open()
        from . import _read
        data = None if source is None else _read(source, limit=16 * 1024 * 1024)[0]
        # Reading caller-owned streams may close or edit the owner. _call checks
        # lifetime again and applies this document-wide property to current state.
        self._call("set_background_image", data, _change=True)

    def add_paragraph(self, text: str = "", *, style: str | Style | None = None) -> Paragraph:
        """Append a paragraph with an optional named style and return its editable handle."""
        style = _style_reference(style, self)
        if style is None:
            identifier = self._call("add_paragraph", text, _change=True)
        else:
            def append(native):
                identifier = native.add_paragraph(text)
                native.set_paragraph_style(identifier, style)
                return identifier
            identifier = self._atomic(append)
        return Paragraph(self, identifier)

    def add_page_break(self) -> Paragraph:
        """Start a new layout page with an explicit native paragraph break."""
        return Paragraph(self, self._call("add_page_break", _change=True))

    def add_table(self, rows: _Rows, *, columns: SupportsIndex | None = None) -> EditableTable:
        """Append a table from string rows, or row count plus columns; return its handle."""
        return EditableTable(self, self._call("add_table", _rows(rows, columns), _change=True))

    def add_image(self, source: _Source, *, width: float | None = None, height: float | None = None, alt: str = "") -> Picture:
        """Append an inline PNG and return its handle; size is in points, aspect preserved if one dimension is omitted."""
        from . import _read
        data, _ = _read(source)
        identifier = self._call("add_image", data, None if width is None else pt(width), None if height is None else pt(height), alt, _change=True)
        return Picture(self, identifier)


class Styles(_Collection["Style"]):
    """Ordered named paragraph, character and table styles; look up by ID or name."""
    def __init__(self, owner: WordDocument):
        super().__init__(owner, lambda: owner._call("style_ids"), Style)

    @overload
    def __getitem__(self, key: str | int) -> Style: ...

    @overload
    def __getitem__(self, key: slice) -> list[Style]: ...

    def __getitem__(self, key):
        if isinstance(key, str):
            return Style(self._owner, json.loads(self._owner._call("style_info", key))["id"])
        return super().__getitem__(key)

    def add(self, name: str, *, kind: str = "paragraph", based_on: str | Style | None = None) -> Style:
        identifier = self._owner._call("add_style", name, kind, _style_reference(based_on, self._owner), _change=True)
        return Style(self._owner, identifier)

    @property
    def defaults(self) -> dict:
        """Direct native document character and paragraph defaults."""
        return json.loads(self._owner._call("default_formatting"))

    def format_defaults(self, **formatting: Unpack[_StyleProperties]) -> None:
        """Update defaults; None clears properties except unsupported font-size reset."""
        self._owner._call("format_defaults", _style_format_patch(formatting), _change=True)


class Style:
    """A native named style with an immutable ID, name and kind."""
    def __init__(self, owner: WordDocument, identifier: str):
        self._owner, self._id = owner, identifier

    def _info(self):
        return json.loads(self._owner._call("style_info", self._id))

    @property
    def id(self) -> str:
        return self._info()["id"]

    @property
    def name(self) -> str:
        return self._info()["name"]

    @property
    def kind(self) -> str:
        return self._info()["kind"]

    @property
    def builtin(self) -> bool:
        return self._info()["builtin"]

    @property
    def based_on(self) -> Style | None:
        identifier = self._info()["based_on"]
        return None if identifier is None else Style(self._owner, identifier)

    @based_on.setter
    def based_on(self, value: str | Style | None):
        self._owner._call("set_style_base", self._id, _style_reference(value, self._owner), _change=True)

    def _set_relation(self, key: str, value: str | Style | None) -> None:
        target = _style_reference(value, self._owner)
        if target is not None and len(target) > 1024:
            raise ValueError("style relation must be a bounded style name/ID or None")
        self._owner._call("style_relations_patch", self._id,
                          json.dumps({key: target}, ensure_ascii=False), _change=True)

    @property
    def next(self) -> Style | None:
        """Stored next-paragraph style; inserting text does not apply it automatically."""
        identifier = self._info()["next"]
        return None if identifier is None else Style(self._owner, identifier)

    @next.setter
    def next(self, value: str | Style | None):
        self._set_relation("next", value)

    @property
    def linked(self) -> Style | None:
        """Reciprocal paragraph/character style partner, if present."""
        identifier = self._info()["linked"]
        if identifier is None:
            return None
        if identifier not in self._owner._call("style_ids"):
            raise UnsupportedFormatError(f"native linked style {identifier!r} is undefined; clear or replace the link")
        return Style(self._owner, identifier)

    @linked.setter
    def linked(self, value: str | Style | None):
        self._set_relation("linked", value)

    @property
    def formatting(self) -> dict:
        """Direct properties; None denotes inheritance."""
        return json.loads(self._owner._call("style_formatting", self._id))

    @property
    def resolved_formatting(self) -> dict:
        """Formatting resolved through the native style inheritance chain."""
        return json.loads(self._owner._call("style_formatting", self._id, True))

    def format(self, *, region: str | None = None, **formatting: Unpack[StyleFormatting]) -> None:
        """Set direct properties; an optional region selects table-style formatting."""
        if region is not None and (not isinstance(region, str) or len(region) > 32):
            raise ValueError("region must be a supported table-style region name or None")
        self._owner._call("format_style", self._id, _style_format_patch(formatting), region, _change=True)

    def __repr__(self) -> str:
        return f"{type(self).__name__}(id={reprlib.repr(self._id)}, closed={self._owner.closed})"


class Paragraph:
    def __init__(self, owner: WordDocument, identifier: int):
        self._owner, self._id = owner, identifier

    @property
    def text(self) -> str:
        return self._owner._call("paragraph_text", self._id)

    @text.setter
    def text(self, value: str):
        self._owner._call("set_paragraph_text", self._id, value, _change=True)

    @property
    def style(self) -> str:
        return self._owner._call("paragraph_style", self._id)

    @style.setter
    def style(self, value: str | Style):
        self._owner._call("set_paragraph_style", self._id, _style_reference(value, self._owner), _change=True)

    @property
    def alignment(self) -> str:
        return self._owner._call("paragraph_alignment", self._id)

    @alignment.setter
    def alignment(self, value: str):
        self._owner._call("set_paragraph_alignment", self._id, value, _change=True)

    @property
    def list_style(self):
        """The direct list kind/level, or None for an ordinary paragraph."""
        return self._owner._call("paragraph_list", self._id)

    def set_list(self, kind: str | None = None, *, level: int = 0, restart: bool = False,
                 start: int | None = None):
        """Set list membership; start creates a new top-level numbered sequence."""
        level = operator.index(level)
        if start is not None:
            if isinstance(start, bool):
                raise TypeError("list start must be an integer, not a boolean")
            start = operator.index(start)
            if not 1 <= start <= 32767:
                raise ValueError("list start must be between 1 and 32767")
        self._owner._call("set_paragraph_list", self._id, kind, level, restart, start, _change=True)

    @property
    def page_break_before(self):
        return self._owner._call("page_break_before", self._id)

    @page_break_before.setter
    def page_break_before(self, value: bool):
        self._owner._call("set_page_break_before", self._id, value, _change=True)

    @property
    def spacing(self):
        """Spacing before/after in points and line-spacing multiplier."""
        return self._owner._call("paragraph_spacing", self._id)

    @spacing.setter
    def spacing(self, value):
        if not isinstance(value, (tuple, list)) or len(value) != 3:
            raise TypeError("spacing must be (before, after, line_spacing)")
        self._owner._call("set_paragraph_spacing", self._id, *map(pt, value), _change=True)

    @property
    def _range_args(self):
        return (self._id,)

    @property
    def formatting(self) -> dict:
        """Direct paragraph properties; None denotes inheritance."""
        return json.loads(self._owner._call("paragraph_formatting", self._id))

    @property
    def resolved_formatting(self) -> dict:
        """Paragraph properties resolved through native defaults and styles."""
        return json.loads(self._owner._call("paragraph_formatting", self._id, True))

    def format(self, **formatting: Unpack[ParagraphFormatting]) -> None:
        """Set paragraph properties; None restores inheritance.

        DOCX direction is ltr/rtl; native alignment, indents and tabs are
        measured from the paragraph start edge.
        """
        self._owner._call("format_paragraph", self._id, _format_patch(formatting), _change=True)

    def add_run(self, text: str, **formatting: Unpack[CharacterFormatting]) -> TextRange:
        """Append formatted characters and return their revision-bound range; omitted properties inherit."""
        start, end, revision = self._owner._call("add_run_patch", self._id, text, _format_patch(formatting), _change=True)
        return TextRange(self, start, end, revision)

    def add_field(self, kind: str) -> TextRange:
        """Append a decimal page, page_count or section_page_count field.

        The returned range contains one object marker. Native rendering evaluates
        the number; source extraction retains the empty stored field result.
        """
        start, end, revision = self._owner._call("add_paragraph_field", self._id, kind, _change=True)
        return TextRange(self, start, end, revision)

    def add_break(self, kind: str = "line") -> TextRange:
        """Append a native line, page or column break and return its text range."""
        start, end, revision = self._owner._call("add_paragraph_break", self._id, kind, _change=True)
        return TextRange(self, start, end, revision)

    _native_text_method = "paragraph_text"

    def __getitem__(self, key: slice) -> TextRange:
        """Select Unicode characters using a contiguous slice; empty slices insert text."""
        return _slice_range(self, key, TextRange)

    def find(self, text: str) -> list[TextRange]:
        """Find non-overlapping literal matches; later edits invalidate ranges."""
        return _find_ranges(self, text, TextRange)

    def replace_text(self, old: str, new: str) -> int:
        """Atomically replace literal matches, preserving surrounding formatting."""
        return _replace_text(self, old, new)

    def __repr__(self) -> str:
        # Identity only: native reads can fail on stale/closed handles or copy large text.
        identifier = self.__dict__.get("_id")
        identity = "" if identifier is None else f"id={identifier!r}, "
        return f"{type(self).__name__}({identity}closed={self._owner.closed})"


class TextRange:
    """A Unicode character range that fails explicitly after incompatible edits."""
    def __init__(self, paragraph: Paragraph | Shape, start: int, end: int, revision: int):
        self._paragraph = paragraph
        self._start, self._end, self._revision = start, end, revision

    @property
    def text(self) -> str:
        paragraph = self._paragraph
        return paragraph._owner._call("range_text", *paragraph._range_args, self._start, self._end, self._revision, **getattr(paragraph, "_range_kwargs", {}))

    @text.setter
    def text(self, value: str):
        paragraph = self._paragraph
        self._start, self._end, self._revision = paragraph._owner._call("replace_range", *paragraph._range_args, self._start, self._end, self._revision, value, _change=True, **getattr(paragraph, "_range_kwargs", {}))

    @property
    def link(self):
        paragraph = self._paragraph
        return paragraph._owner._call("range_link", *paragraph._range_args, self._start, self._end, self._revision, **getattr(paragraph, "_range_kwargs", {}))

    @link.setter
    def link(self, url: str | None):
        paragraph = self._paragraph
        self._revision = paragraph._owner._call("set_range_link", *paragraph._range_args, self._start, self._end, self._revision, url, _change=True, **getattr(paragraph, "_range_kwargs", {}))

    def format(self, **formatting: Unpack[CharacterFormatting]) -> None:
        """Set character properties; omitted values stay and None inherits.

        DOCX language is a literal ASCII alphanumeric/hyphen tag of at most
        63 bytes, not a complete BCP47 validator or font-selection guarantee.
        """
        paragraph = self._paragraph
        self._revision = paragraph._owner._call("format_range_patch", *paragraph._range_args, self._start, self._end, self._revision, _format_patch(formatting, rgb_highlight=paragraph._owner._format == "pptx"), _change=True, **getattr(paragraph, "_range_kwargs", {}))


def _slice_range(content, key: slice, factory):
    if not isinstance(key, slice):
        raise TypeError("text selection requires a slice")
    revision = content._owner._call("revision")
    text = content.text
    start, stop, step = key.indices(len(text))
    if step != 1:
        raise ValueError("text selection requires a slice step of 1")
    # A forward slice with a stop before its start is empty, at the start.
    stop = max(start, stop)
    selected, current_revision = content._owner._call("text_range", *content._range_args, start, stop, **getattr(content, "_range_kwargs", {}))
    if current_revision != revision or selected != text[start:stop]:
        raise ValueError("text changed during selection")
    return factory(content, start, stop, current_revision)


def _find_ranges(content, text: str, factory):
    if not isinstance(text, str):
        raise ValueError("search text must be a nonempty string")
    # Literal matching uses the stored string, not subclass conversion/length
    # hooks which could edit the owner while ranges or a transaction are built.
    text = str.__str__(text)
    if not text:
        raise ValueError("search text must be a nonempty string")
    value = content.text
    args = content._range_args
    position = 0
    result = []
    while (start := value.find(text, position)) >= 0:
        end = start + len(text)
        actual, revision = content._owner._call("text_range", *args, start, end, **getattr(content, "_range_kwargs", {}))
        if actual != text:
            raise ValueError("text changed during search")
        result.append(factory(content, start, end, revision))
        position = end
    return result


def _replace_text(content, old: str, new: str) -> int:
    if not isinstance(old, str):
        raise ValueError("search text must be a nonempty string")
    old = str.__str__(old)
    if not old:
        raise ValueError("search text must be a nonempty string")
    if not isinstance(new, str):
        raise TypeError("replacement text must be a string")
    new = str.__str__(new)
    args = content._range_args
    def replace(native):
        value = getattr(native, content._native_text_method)(*args, **getattr(content, "_range_kwargs", {}))
        positions = []
        offset = 0
        while (start := value.find(old, offset)) >= 0:
            positions.append(start)
            offset = start + len(old)
        for start in reversed(positions):
            _, revision = native.text_range(*args, start, start + len(old), **getattr(content, "_range_kwargs", {}))
            native.replace_range(*args, start, start + len(old), revision, new, **getattr(content, "_range_kwargs", {}))
        return len(positions)
    return content._owner._atomic(replace)


def _slide_link_target(owner, target):
    if isinstance(target, Slide):
        if target._owner is not owner:
            raise ValueError("link destination must belong to this presentation")
        target.number  # Reject a deleted target before mutation.
        return None, target._id
    if target is not None and not isinstance(target, str):
        raise TypeError("link destination must be a URL, Slide or None")
    return target, None


def _slide_link_value(owner, value):
    if value is None:
        return None
    url, slide_id, _ = value
    return url if slide_id is None else Slide(owner, slide_id)


class _SlideTextRange(TextRange):
    def _link_info(self, *, allow_mixed: bool = False):
        content = self._paragraph
        return content._owner._call("range_link", *content._range_args, self._start, self._end, self._revision, allow_mixed, **getattr(content, "_range_kwargs", {}))

    @property
    def link(self) -> str | Slide | None:
        return _slide_link_value(self._paragraph._owner, self._link_info())

    @link.setter
    def link(self, target: str | Slide | None):
        content = self._paragraph
        url, slide_id = _slide_link_target(content._owner, target)
        previous = self._link_info(allow_mixed=True)
        tooltip = previous[2] if previous is not None and target is not None else ""
        self._revision = content._owner._call("set_range_link", *content._range_args, self._start, self._end, self._revision, url, slide_id, tooltip, _change=True, **getattr(content, "_range_kwargs", {}))

    @property
    def link_tooltip(self) -> str:
        value = self._link_info()
        return "" if value is None else value[2]

    @link_tooltip.setter
    def link_tooltip(self, value: str):
        info = self._link_info()
        if info is None:
            raise ValueError("set a link before setting its tooltip")
        content = self._paragraph
        self._revision = content._owner._call("set_range_link", *content._range_args, self._start, self._end, self._revision, info[0], info[1], value, _change=True, **getattr(content, "_range_kwargs", {}))


class Sections(_Collection["Section"]):
    """Append-only document sections with stable identities."""
    def __init__(self, owner: WordDocument):
        super().__init__(owner, lambda: owner._call("section_ids"), Section)

    def add(self, *, start: str = "next_page") -> Section:
        """Append a section, copying its predecessor's setup and independent stories."""
        return Section(self._owner, self._owner._call("add_section", start, _change=True))


class Section:
    def __init__(self, owner: WordDocument, identifier: int):
        self._owner, self._id = owner, identifier

    def _props(self):
        return json.loads(self._owner._call("section_props", self._id))

    def _set(self, **values):
        self._owner._call("set_section_props", self._id, json.dumps(values, allow_nan=False), _change=True)

    @property
    def start(self) -> str:
        return self._props()["start"]

    @start.setter
    def start(self, value: str):
        self._set(start=value)

    @property
    def line_numbers(self) -> tuple[int, int, float, str] | None:
        """(count_by, start, distance_pt, restart), or None; single columns only.

        Restart is each_page, each_section or continuous. Distance zero uses
        the native 18pt default; (1, 1, 0, "each_page") numbers every body line.
        """
        value = self._props()["line_numbers"]
        return None if value is None else tuple(value)

    @line_numbers.setter
    def line_numbers(self, value: tuple[int, int, float, str] | None) -> None:
        if value is None:
            self._set(line_numbers=None)
            return
        if not isinstance(value, (tuple, list)) or len(value) != 4:
            raise TypeError("line_numbers must be (count_by, start, distance, restart), or None")
        count_by, start, distance, restart = value
        if isinstance(count_by, bool) or isinstance(start, bool):
            raise TypeError("line-number count_by and start must be integers, not booleans")
        count_by, start = operator.index(count_by), operator.index(start)
        if not 1 <= count_by <= 100 or not 1 <= start <= 32768:
            raise ValueError("line-number count_by must be 1..100 and start must be 1..32768")
        distance = pt(distance)
        if not 0 <= distance <= 1584:
            raise ValueError("line-number distance must be between 0 and 1584 points")
        if restart not in ("each_page", "each_section", "continuous"):
            raise ValueError("line-number restart must be each_page, each_section or continuous")
        self._set(line_numbers=[count_by, start, distance, restart])

    @property
    def page_number_start(self) -> int | None:
        """First displayed page number, or None to continue the prior section."""
        return self._props()["page_number_start"]

    @page_number_start.setter
    def page_number_start(self, value: int | None) -> None:
        if value is not None:
            if isinstance(value, bool):
                raise TypeError("page_number_start must be an integer, not a boolean")
            value = operator.index(value)
            if not 1 <= value <= 32767:
                raise ValueError("page_number_start must be between 1 and 32767")
        self._set(page_number_start=value)

    @property
    def page_size(self) -> tuple[float, float]:
        return tuple(self._props()["page_size"])

    @page_size.setter
    def page_size(self, value):
        self._set(page_size=_size(value))

    @property
    def margins(self) -> tuple[float, float, float, float]:
        """Top, right, bottom and left margins in points."""
        return tuple(self._props()["margins"])

    @margins.setter
    def margins(self, value):
        self._set(margins=_margins(value))

    @property
    def gutter(self) -> float:
        """Additional left binding space in points."""
        return self._props()["gutter"]

    @gutter.setter
    def gutter(self, value):
        self._set(gutter=pt(value))

    @property
    def header_distance(self) -> float:
        return self._props()["distances"][0]

    @header_distance.setter
    def header_distance(self, value):
        self._set(distances=[pt(value), self.footer_distance])

    @property
    def footer_distance(self) -> float:
        return self._props()["distances"][1]

    @footer_distance.setter
    def footer_distance(self, value):
        self._set(distances=[self.header_distance, pt(value)])

    @property
    def columns(self) -> tuple[int, float, bool]:
        """Equal column count, gap in points, and separator-line flag."""
        return tuple(self._props()["columns"])

    @columns.setter
    def columns(self, value):
        if not isinstance(value, (tuple, list)) or len(value) != 3:
            raise TypeError("columns must be (count, gap, separator)")
        count, gap, separator = value
        if isinstance(count, bool) or not isinstance(separator, bool):
            raise TypeError("column count must be an integer and separator a bool")
        self._set(columns=[operator.index(count), pt(gap), separator])

    @property
    def column_widths(self) -> tuple[float, ...] | None:
        """Explicit widths in points, or None for equal-width native columns."""
        widths = self._props()["column_widths"]
        return None if widths is None else tuple(widths)

    @column_widths.setter
    def column_widths(self, value):
        if value is None:
            self._set(column_widths=None)
            return
        if not isinstance(value, (tuple, list)):
            raise TypeError("column_widths must be a sequence of point widths or None")
        if not 1 <= len(value) <= 45:
            raise ValueError("column_widths requires 1..45 widths")
        self._set(column_widths=[pt(width) for width in value])

    def note_options(self, *, kind: str = "footnote") -> dict:
        """Return resolved native numbering and placement for this section."""
        return json.loads(self._owner._call("section_note_options", self._id, kind))

    def configure_notes(self, *, kind: str = "footnote", **options: Unpack[NoteOptions]) -> None:
        """Update placement/numbering/start/restart; None inherits native defaults."""
        if options.keys() - {"placement", "numbering", "start", "restart"}:
            raise ValueError("note options must be placement, numbering, start or restart")
        if any(value is not None and not isinstance(value, (str, int)) for value in options.values()):
            raise TypeError("note options must be strings, integer start or None")
        self._owner._call("set_section_note_options", self._id, kind,
                          _format_patch(options), _change=True)

    def header(self, variant: str = "default") -> EditableStory:
        """Access this section's default, first-page or even-page header."""
        if variant not in {"default", "first", "even"}:
            raise ValueError("header variant must be default, first or even")
        return EditableStory(self._owner, "header", variant, self._id)

    def footer(self, variant: str = "default") -> EditableStory:
        """Access this section's default, first-page or even-page footer."""
        if variant not in {"default", "first", "even"}:
            raise ValueError("footer variant must be default, first or even")
        return EditableStory(self._owner, "footer", variant, self._id)


def _margins(value):
    if isinstance(value, Real):
        value = (value,) * 4
    if not isinstance(value, (tuple, list)) or len(value) != 4:
        raise TypeError("margins must be a number or (top, right, bottom, left) in points")
    return tuple(map(pt, value))


class EditableStory:
    """A header or footer that shares the normal paragraph API."""
    def __init__(self, owner: WordDocument, kind: str, variant: str, section: int = 0):
        self._owner, self._kind, self._variant, self._section = owner, kind, variant, section

    def _identifier(self):
        return self._owner._call("story_id", self._kind, self._variant, self._section)

    def _ensure_identifier(self, native):
        return native.ensure_story(self._kind, self._variant, self._section)

    @property
    def paragraphs(self) -> _Collection[Paragraph]:
        def identifiers():
            identifier = self._identifier()
            return [] if identifier is None else self._owner._call("story_paragraph_ids", identifier)
        return _Collection(self._owner, identifiers, Paragraph)

    def _check_rich_content(self):
        self._owner._ensure_open()
        if isinstance(self, Note):
            raise UnsupportedFormatError("rich note content requires native note-container geometry support")

    @property
    def tables(self) -> _Collection[EditableTable]:
        """Direct header/footer tables, sharing the normal table and cell API."""
        self._check_rich_content()
        def identifiers():
            self._check_rich_content()
            identifier = self._identifier()
            return [] if identifier is None else self._owner._call("story_table_ids", identifier)
        return _Collection(self._owner, identifiers, EditableTable)

    @property
    def images(self) -> Pictures:
        """Direct inline pictures; nested cell pictures belong to cell.images."""
        self._check_rich_content()
        def identifiers():
            self._check_rich_content()
            identifier = self._identifier()
            return [] if identifier is None else self._owner._call("story_image_ids", identifier)
        return Pictures(self._owner, identifiers)

    def add_table(self, rows: _Rows, *, columns: SupportsIndex | None = None) -> EditableTable:
        """Append a header/footer table from string rows or row/column counts."""
        self._check_rich_content()
        values = _rows(rows, columns)
        self._check_rich_content()
        def append(native):
            return native.story_add_table(self._ensure_identifier(native), values)
        return EditableTable(self._owner, self._owner._atomic(append))

    def add_image(self, source: _Source, *, width: float | None = None, height: float | None = None, alt: str = "") -> Picture:
        """Append an inline PNG in points, preserving aspect when one dimension is omitted."""
        from . import _read
        self._check_rich_content()
        width, height = None if width is None else pt(width), None if height is None else pt(height)
        data, _ = _read(source, limit=16 * 1024 * 1024)
        self._check_rich_content()
        def append(native):
            return native.story_add_image(self._ensure_identifier(native), data, width, height, alt)
        return Picture(self._owner, self._owner._atomic(append))

    def add_paragraph(self, text: str = "", *, style: str | Style | None = None) -> Paragraph:
        """Append a paragraph with an optional named style and return its editable handle."""
        style = _style_reference(style, self._owner)
        def append(native):
            story = self._ensure_identifier(native)
            identifier = native.story_add_paragraph(story, text)
            if style is not None:
                native.set_paragraph_style(identifier, style)
            return identifier
        return Paragraph(self._owner, self._owner._atomic(append))


class Notes(_Collection["Note"]):
    """Native footnote/endnote stories in creation order."""
    def __init__(self, owner: WordDocument):
        super().__init__(owner, lambda: owner._call("note_ids"), Note)

    def add(self, reference: TextRange, text: str = "", *, kind: str = "footnote") -> Note:
        """Attach a note at a same-document body range's end, retaining its text."""
        if not isinstance(reference, TextRange) or not isinstance(reference._paragraph, Paragraph):
            raise TypeError("reference must be a DOCX paragraph TextRange")
        paragraph = reference._paragraph
        if paragraph._owner is not self._owner:
            raise ValueError("reference must belong to this document")
        identifier = self._owner._call("add_note", paragraph._id, reference._start,
                                      reference._end, reference._revision, text, kind, _change=True)
        _, reference._revision = self._owner._call("text_range", paragraph._id,
                                                  reference._start, reference._end)
        return Note(self._owner, identifier)


class Note(EditableStory):
    """An editable native note story sharing the paragraph and range APIs."""
    def __init__(self, owner: WordDocument, identifier: int):
        self._owner, self._id = owner, identifier

    def _identifier(self):
        return json.loads(self._owner._call("note_info", self._id))["story"]

    def _ensure_identifier(self, native):
        return json.loads(native.note_info(self._id))["story"]

    @property
    def kind(self) -> str:
        return json.loads(self._owner._call("note_info", self._id))["kind"]


class Pictures(_Collection["Picture"]):
    """Ordered inline pictures; deletion removes the selected placement."""
    def __init__(self, owner, ids):
        super().__init__(owner, ids, Picture)

    def __delitem__(self, index: int) -> None:
        picture = self[operator.index(index)]
        self._owner._call("remove_image", picture._id, _change=True)


class Picture:
    def __init__(self, owner: WordDocument, identifier: int):
        self._owner, self._id = owner, identifier

    @property
    def data(self) -> bytes:
        """Owned PNG bytes; assigning a source preserves size and alternative text."""
        return bytes(self._owner._call("image_data", self._id))

    @data.setter
    def data(self, source) -> None:
        from . import _read
        data, _ = _read(source)
        self._owner._call("set_image_data", self._id, data, _change=True)

    @property
    def size(self):
        return self._owner._call("image_size", self._id)

    @size.setter
    def size(self, value):
        self._owner._call("set_image_size", self._id, *_size(value), _change=True)

    @property
    def alt(self):
        return self._owner._call("image_alt", self._id)

    @alt.setter
    def alt(self, value: str):
        self._owner._call("set_image_alt", self._id, value, _change=True)

    @property
    def crop(self) -> tuple[float, float, float, float]:
        return self._owner._call("image_crop", self._id)

    @crop.setter
    def crop(self, value) -> None:
        values = _picture_crop(value)
        self._owner._call("set_image_crop", self._id, values, _change=True)

    @property
    def flip_h(self) -> bool:
        return self._owner._call("image_flips", self._id)[0]

    @flip_h.setter
    def flip_h(self, value: bool) -> None:
        if not isinstance(value, bool):
            raise TypeError("flip_h must be a boolean")
        self._owner._call("set_image_flips", self._id, flip_h=value, _change=True)

    @property
    def flip_v(self) -> bool:
        return self._owner._call("image_flips", self._id)[1]

    @flip_v.setter
    def flip_v(self, value: bool) -> None:
        if not isinstance(value, bool):
            raise TypeError("flip_v must be a boolean")
        self._owner._call("set_image_flips", self._id, flip_v=value, _change=True)


class EditableTable:
    def __init__(self, owner: EditableDocument, identifier: int, slide: int | None = None):
        self._owner, self._id = owner, identifier
        self._slide = slide

    @property
    def _args(self):
        return (self._id,) if self._slide is None else (self._owner._call("slide_index", self._slide), self._id)

    @property
    def rows(self) -> list[list[str]]:
        """Return a text snapshot; use table[row, column].text to edit a cell."""
        return self._owner._call("table_rows", *self._args)

    @overload
    def __getitem__(self, key: tuple[int, int]) -> EditableCell: ...

    @overload
    def __getitem__(self, key: tuple[int, slice] | tuple[slice, int]) -> list[EditableCell]: ...

    @overload
    def __getitem__(self, key: tuple[slice, slice]) -> list[list[EditableCell]]: ...

    def __getitem__(self, key):
        """Address cells or ordered row/column slices; slices return handle lists."""
        if not isinstance(key, tuple) or len(key) != 2:
            raise TypeError("address cells with table[row, column] or row/column slices")
        rows = self.rows
        row_key, column_key = key
        if isinstance(row_key, slice):
            row_indices = range(len(rows))[row_key]
            if isinstance(column_key, slice):
                columns = range(len(rows[0]))[column_key]
                return [[EditableCell(self, row, column) for column in columns] for row in row_indices]
            column = _index(column_key, len(rows[0]))
            return [EditableCell(self, row, column) for row in row_indices]
        row = _index(row_key, len(rows))
        if isinstance(column_key, slice):
            return [EditableCell(self, row, column) for column in range(len(rows[row]))[column_key]]
        return EditableCell(self, row, _index(column_key, len(rows[row])))

    def __repr__(self) -> str:
        return f"{type(self).__name__}(id={self._id!r}, closed={self._owner.closed})"

    def _word_id(self):
        if self._slide is not None:
            raise UnsupportedFormatError("this table layout operation is currently DOCX-only")
        return self._id

    @property
    def formatting(self) -> dict:
        """A snapshot of direct native table formatting."""
        return json.loads(self._owner._call("table_formatting", self._word_id()))

    def format(self, **formatting: Unpack[TableFormatting]) -> None:
        if "style" in formatting:
            formatting["style"] = _style_reference(formatting["style"], self._owner)
        self._owner._call("table_format_patch", self._word_id(),
                          _table_format_patch(formatting), _change=True)

    def fit(self) -> list[float]:
        """Measure native cell content and apply checked column widths."""
        return self._owner._call("fit_table", self._word_id(), _change=True)

    @property
    def column_widths(self):
        return self._owner._call("column_widths", *self._args)

    @column_widths.setter
    def column_widths(self, widths):
        values = []
        for width in widths:
            if len(values) >= 1000:
                raise ValueError("table column widths exceed 1000 columns")
            values.append(pt(width))
        self._owner._call("set_column_widths", *self._args, values, _change=True)

    @property
    def borders(self):
        return self._owner._call("table_borders", *self._args)

    @borders.setter
    def borders(self, value):
        if value is None:
            self._owner._call("clear_table_borders", *self._args, _change=True)
        elif isinstance(value, (tuple, list)) and len(value) == 2:
            self._owner._call("set_table_borders", *self._args, _hex_color(value[0]), pt(value[1]), _change=True)
        else:
            raise TypeError("borders must be (color, width) or None")

    def row(self, index: int) -> EditableRow:
        """Access native row layout properties without copying its cells."""
        return EditableRow(self, _index(index, len(self.rows)))

    def merge(self, first: tuple[int, int], last: tuple[int, int]) -> EditableCell:
        """Merge an inclusive rectangular range and return its anchor cell."""
        start = self[first]
        end = self[last]
        self._owner._call("merge_cells", *self._args, start._row, start._column, end._row, end._column, _change=True)
        return self[first]


    def _structure_generation(self):
        return self._owner._call("table_generation", *self._args)

    def insert_row(self, index: int) -> EditableRow:
        index = _insertion_index(index, len(self.rows))
        self._owner._call("insert_row", *self._args, index, _change=True)
        return EditableRow(self, index)

    def delete_row(self, index: int) -> None:
        index = _index(index, len(self.rows))
        self._owner._call("delete_row", *self._args, index, _change=True)

    def insert_column(self, index: int) -> list[EditableCell]:
        rows = self.rows
        index = _insertion_index(index, len(rows[0]))
        self._owner._call("insert_column", *self._args, index, _change=True)
        return [EditableCell(self, row, index) for row in range(len(rows))]

    def delete_column(self, index: int) -> None:
        index = _index(index, len(self.rows[0]))
        self._owner._call("delete_column", *self._args, index, _change=True)


def _insertion_index(index: int, length: int) -> int:
    index = operator.index(index)
    if index < 0:
        index += length
    if not 0 <= index <= length:
        raise IndexError("insertion index out of range")
    return index


class EditableRow:
    def __init__(self, table: EditableTable, index: int):
        self._table, self._index = table, index
        self._generation = table._structure_generation()

    def _check(self):
        self._table._owner._ensure_open()
        if self._generation != self._table._structure_generation():
            raise ValueError("row handle is stale after a table structure change")

    @property
    def _args(self):
        self._check()
        return (*self._table._args, self._index)


    @property
    def height(self):
        """(height in points or None, rule): auto, at_least or exact."""
        return self._table._owner._call("row_height", *self._args)

    @height.setter
    def height(self, value):
        if value is None:
            height, rule = None, "auto"
        elif isinstance(value, (tuple, list)) and len(value) == 2:
            height, rule = None if value[0] is None else pt(value[0]), value[1]
        else:
            raise TypeError("height must be (points, rule) or None for automatic height")
        self._table._owner._call("set_row_height", *self._args, height, rule, _change=True)

    @property
    def repeat_header(self):
        self._check()
        return self._table._owner._call("row_repeat_header", self._table._word_id(), self._index)

    @repeat_header.setter
    def repeat_header(self, value: bool):
        self._check()
        self._table._owner._call("set_row_repeat_header", self._table._word_id(), self._index, value, _change=True)

    @property
    def allow_split(self):
        self._check()
        return self._table._owner._call("row_allow_split", self._table._word_id(), self._index)

    @allow_split.setter
    def allow_split(self, value: bool):
        self._check()
        self._table._owner._call("set_row_allow_split", self._table._word_id(), self._index, value, _change=True)


class EditableCell:
    def __init__(self, table: EditableTable, row: int, column: int):
        self._table, self._row, self._column = table, row, column
        self._generation = table._structure_generation()

    def _check(self):
        self._table._owner._ensure_open()
        if self._generation != self._table._structure_generation():
            raise ValueError("cell handle is stale after a table structure change")

    @property
    def _args(self):
        self._check()
        return (*self._table._args, self._row, self._column)


    @property
    def text(self) -> str:
        table = self._table
        return table._owner._call("cell_text", *self._args)

    @text.setter
    def text(self, value: str):
        table = self._table
        table._owner._call("set_cell_text", *self._args, value, _change=True)

    def _content_args(self):
        self._check()
        table = self._table
        if table._slide is not None:
            raise UnsupportedFormatError("PPTX cells support text paragraphs, not nested tables or images")
        return table._id, self._row, self._column

    def _children(self, kind, factory):
        owner = self._table._owner
        return _Collection(owner, lambda: owner._call(f"cell_{kind}_ids", *self._content_args()), factory)

    @property
    def paragraphs(self) -> _Collection[Paragraph]:
        if self._table._slide is not None:
            owner = self._table._owner
            def targets():
                row, column, grid, body, count = owner._call("cell_paragraph_info", *self._args)
                return [(row, column, index, grid, body) for index in range(count)]
            return _Collection(owner, targets, lambda owner, target: _SlideCellParagraph(self, target))
        return self._children("paragraph", Paragraph)

    @property
    def tables(self) -> _Collection[EditableTable]:
        return self._children("table", EditableTable)

    @property
    def images(self) -> Pictures:
        owner = self._table._owner
        return Pictures(owner, lambda: owner._call("cell_image_ids", *self._content_args()))

    def add_paragraph(self, text: str = "", *, style: str | Style | None = None) -> Paragraph:
        """Append a cell paragraph; named styles are available only in DOCX."""
        if self._table._slide is not None:
            self._check()
            if style is not None:
                raise UnsupportedFormatError("named paragraph styles are unavailable in PPTX cells")
            target = self._table._owner._call("cell_add_paragraph", *self._args, text, _change=True)
            return _SlideCellParagraph(self, target)
        style = _style_reference(style, self._table._owner)
        args = self._content_args()
        def append(native):
            identifier = native.cell_add_paragraph(*args, text)
            if style is not None:
                native.set_paragraph_style(identifier, style)
            return identifier
        return Paragraph(self._table._owner, self._table._owner._atomic(append))

    def add_table(self, rows: _Rows, *, columns: SupportsIndex | None = None) -> EditableTable:
        """Append a nested DOCX table from string rows or row/column counts."""
        owner = self._table._owner
        return EditableTable(owner, owner._call("cell_add_table", *self._content_args(), _rows(rows, columns), _change=True))

    def add_image(self, source: _Source, *, width: float | None = None, height: float | None = None, alt: str = "") -> Picture:
        """Append an inline PNG and return its handle; size is in points, aspect preserved if one dimension is omitted."""
        from . import _read
        data, _ = _read(source)
        owner = self._table._owner
        identifier = owner._call("cell_add_image", *self._content_args(), data, None if width is None else pt(width), None if height is None else pt(height), alt, _change=True)
        return Picture(owner, identifier)

    def split(self, *, columns: int | None = None) -> None:
        """Split DOCX horizontally or restore a PPTX cell's existing merged grid."""
        table = self._table
        self._check()
        if table._slide is None and columns is None:
            columns = 2
        if columns is not None:
            columns = operator.index(columns)
        table._owner._call("split_cell", *self._args, columns, _change=True)

    @property
    def formatting(self) -> dict:
        """A snapshot of direct native cell formatting."""
        self._check()
        return json.loads(self._table._owner._call("cell_formatting", self._table._word_id(), self._row, self._column))

    def format(self, **formatting: Unpack[CellFormatting]) -> None:
        self._check()
        self._table._owner._call("cell_format_patch", self._table._word_id(), self._row, self._column,
                                _table_format_patch(formatting, cell=True), _change=True)

    @property
    def margins(self) -> tuple[float, float, float, float]:
        return self._table._owner._call("cell_margins", *self._args)

    @margins.setter
    def margins(self, value):
        self._table._owner._call("set_cell_margins", *self._args, None if value is None else _margins(value), _change=True)

    @property
    def alignment(self) -> str | None:
        return self._table._owner._call("cell_alignment", *self._args)

    @alignment.setter
    def alignment(self, value: str | None):
        self._table._owner._call("set_cell_alignment", *self._args, value, _change=True)

    @property
    def background(self):
        table = self._table
        value = table._owner._call("cell_background", *self._args)
        return None if value is None else _hex_color(value)

    @background.setter
    def background(self, value):
        table = self._table
        table._owner._call("set_cell_background", *self._args, _hex_color(value) if table._slide is None else _color(value), _change=True)

    @property
    def vertical_alignment(self):
        table = self._table
        return table._owner._call("cell_vertical_alignment", *self._args)

    @vertical_alignment.setter
    def vertical_alignment(self, value: str):
        table = self._table
        table._owner._call("set_cell_vertical_alignment", *self._args, value, _change=True)


class PdfDocument(EditableDocument):
    _format = "pdf"

    @property
    def fields(self) -> PdfFields:
        return PdfFields(self)

    @property
    def metadata(self):
        """A snapshot of supported document-info fields."""
        return json.loads(self._call("metadata"))

    def update_metadata(self, **values):
        """Update title/author/subject/keywords; None or empty strings clear fields."""
        if values.keys() - {"title", "author", "subject", "keywords"}:
            raise ValueError("metadata keys must be title, author, subject or keywords")
        for value in values.values():
            if value is not None and not isinstance(value, str):
                raise TypeError("metadata values must be strings or None")
            if value is not None and (len(value) > 65536 or len(value.encode("utf-8")) > 65536):
                raise ValueError("metadata values must not exceed 65536 bytes")
        self._call("update_metadata", json.dumps(values, ensure_ascii=False, allow_nan=False), _change=True)

    @property
    def bookmarks(self) -> Bookmarks:
        return Bookmarks(self)

    @property
    def page_count(self):
        return self._call("page_count")

    @property
    def pages(self) -> PdfPages:
        return PdfPages(self)

    def copy(self, *, pages: Iterable[SupportsIndex] | None = None) -> Self:
        """Copy independently, optionally selecting ordered pages with duplicates and negative indexes."""
        if pages is None:
            return super().copy()
        from . import _selected_pages
        self._ensure_open()
        generation, count = self._page_generation, self.page_count
        indices = _selected_pages(pages)
        self._ensure_open()
        if (generation, count) != (self._page_generation, self.page_count):
            raise ValueError("page selection is stale after page organization")
        indices = [_index(index, count) for index in indices]
        return type(self)(self._call("subset", indices))


class PdfFields:
    """Name-indexed fields; iteration returns checked field handles."""
    def __init__(self, owner: PdfDocument, page: PdfPage | None = None):
        self._owner, self._page = owner, page

    def _page_number(self):
        self._owner._ensure_open()
        return None if self._page is None else self._page.number

    def _items(self):
        return json.loads(self._owner._call("fields", self._page_number()))

    def __len__(self) -> int:
        return len(self._items())

    def __iter__(self) -> Iterator[PdfField]:
        generation = (self._owner._field_generation, self._owner._page_generation)
        for item in self._items():
            self._page_number()
            if generation != (self._owner._field_generation, self._owner._page_generation):
                raise ValueError("field iteration is stale after structural edits")
            yield PdfField(self._owner, item["name"])

    def __getitem__(self, name: str) -> PdfField:
        if not isinstance(name, str):
            raise TypeError("fields are indexed by their immutable names")
        self._page_number()
        if not any(item["name"] == name for item in self._items()):
            raise KeyError(name)
        return PdfField(self._owner, name)

    def __delitem__(self, name: str) -> None:
        field = self[name]
        page = field.page.number
        self._owner._call("delete_field", field.name, _change=True)
        self._owner._field_generation += 1
        self._owner._annotation_generations[page] = self._owner._annotation_generations.get(page, 0) + 1

    def add(self, name: str, box, *, kind: str = "text", options=()) -> PdfField:
        """Create one text, checkbox or choice widget on this page."""
        if self._page is None:
            raise ValueError("add fields through page.fields")
        page = self._page_number()
        if not isinstance(box, (tuple, list)) or len(box) != 4:
            raise TypeError("box must be (x, y, width, height) in top-left points")
        if isinstance(options, (str, bytes)):
            raise TypeError("options must be a sequence of choice strings")
        values = []
        for value in options:
            if len(values) >= 256:
                raise ValueError("choice fields must not exceed 256 options")
            if not isinstance(value, str):
                raise TypeError("choice options must be strings")
            if len(value) > 4096 or len(value.encode("utf-8")) > 4096:
                raise ValueError("choice options must not exceed 4096 bytes")
            values.append(value)
        identifier = self._owner._call("add_field", page, name, kind, tuple(map(pt, box)), values, _change=True)
        self._owner._field_generation += 1
        return PdfField(self._owner, identifier)

    def flatten(self) -> int:
        """Replace all owned form widgets with their visible page content."""
        if self._page is not None:
            raise ValueError("flatten all fields through document.fields")
        count = self._owner._call("flatten_fields", _change=True)
        if count:
            self._owner._field_generation += 1
            self._owner._annotation_generations.clear()
            self._owner._page_generation += 1
        return count


class PdfField:
    """One owned field with an immutable name and checked typed value."""
    def __init__(self, owner: PdfDocument, name: str):
        self._owner, self._name = owner, name
        self._generation = (owner._field_generation, owner._page_generation)

    def _check(self):
        self._owner._ensure_open()
        if self._generation != (self._owner._field_generation, self._owner._page_generation):
            raise ValueError("field handle is stale after structural edits; retrieve it again")

    def _info(self):
        self._check()
        return json.loads(self._owner._call("field_info", self._name))

    @property
    def name(self) -> str:
        self._check()
        return self._name

    @property
    def kind(self) -> str:
        return self._info()["kind"]

    @property
    def options(self) -> tuple[str, ...]:
        return tuple(self._info()["options"])

    @property
    def page(self) -> PdfPage:
        return PdfPage(self._owner, self._info()["widgets"][0]["page"])

    @property
    def box(self) -> tuple[float, float, float, float]:
        """Widget (x, y, width, height) in current page view points."""
        return tuple(self._info()["widgets"][0]["box"])

    @box.setter
    def box(self, value) -> None:
        if not isinstance(value, (tuple, list)) or len(value) != 4:
            raise TypeError("box must be (x, y, width, height) in top-left points")
        coordinates = tuple(map(pt, value))
        # Coordinate conversion can execute caller code, including structural edits.
        self._check()
        self._owner._call("set_field_box", self._name, coordinates, _change=True)

    @property
    def value(self) -> str | bool | None:
        return self._info()["value"]

    @value.setter
    def value(self, value: str | bool | None):
        self._check()
        if value is not None and not isinstance(value, (str, bool)):
            raise TypeError("field values must be text, a checkbox bool or None for a choice")
        if isinstance(value, str) and (len(value) > 4096 or len(value.encode("utf-8")) > 4096):
            raise ValueError("field values must not exceed 4096 bytes")
        self._owner._call("set_field_value", self._name,
                          json.dumps(value, ensure_ascii=False, allow_nan=False), _change=True)


class Bookmarks(_Collection["Bookmark"]):
    def __init__(self, owner: PdfDocument, path=(), parent=None):
        self._path, self._parent = tuple(path), parent
        def identifiers():
            self._check()
            return [tuple(value["path"]) for value in json.loads(owner._call("bookmarks"))
                    if len(value["path"]) == len(self._path) + 1 and tuple(value["path"][:-1]) == self._path]
        super().__init__(owner, identifiers, Bookmark)

    def _check(self):
        self._owner._ensure_open()
        if self._parent is not None:
            self._parent._check()

    def add(self, title: str, page: int, *, index: int | None = None) -> Bookmark:
        """Insert an outline entry targeting a zero-based page and return its handle."""
        self._check()
        page = _index(page, self._owner.page_count)
        if index is not None:
            index = operator.index(index)
            if not 0 <= index <= len(self):
                raise IndexError("bookmark insertion index out of range")
        path = self._owner._call("add_bookmark", title, page, list(self._path), index, _change=True)
        self._owner._bookmark_generation += 1
        if self._parent is not None:
            self._parent._generation = self._owner._bookmark_generation
        return Bookmark(self._owner, path)

    def __delitem__(self, index):
        bookmark = self[index]
        self._owner._call("delete_bookmark", list(bookmark._path), _change=True)
        self._owner._bookmark_generation += 1
        if self._parent is not None:
            self._parent._generation = self._owner._bookmark_generation

    def move(self, index: int, destination: int, *, to: Bookmarks | None = None) -> Bookmark:
        """Move an entry within this document and return a fresh handle."""
        bookmark = self[index]
        target = self if to is None else to
        if not isinstance(target, Bookmarks) or target._owner is not self._owner:
            raise ValueError("bookmark destination must belong to the same document")
        target._check()
        destination = operator.index(destination)
        maximum = len(target) - int(target._path == self._path)
        if not 0 <= destination <= maximum:
            raise IndexError("bookmark destination index out of range")
        path = self._owner._call("move_bookmark", list(bookmark._path), list(target._path), destination, _change=True)
        self._owner._bookmark_generation += 1
        if target._path == self._path:
            for collection in (self, target):
                if collection._parent is not None:
                    collection._parent._generation = self._owner._bookmark_generation
        return Bookmark(self._owner, path)


class Bookmark:
    """A native outline entry; structural outline edits invalidate old handles."""
    def __init__(self, owner: PdfDocument, path):
        self._owner, self._path = owner, tuple(path)
        self._generation = owner._bookmark_generation
        self._page_generation = owner._page_generation

    def _check(self):
        self._owner._ensure_open()
        if self._generation != self._owner._bookmark_generation or self._page_generation != self._owner._page_generation:
            raise ValueError("bookmark handle is stale after organization; retrieve it again")

    def _value(self):
        self._check()
        for value in json.loads(self._owner._call("bookmarks")):
            if tuple(value["path"]) == self._path:
                return value
        raise ValueError("bookmark no longer exists")

    @property
    def children(self):
        self._check()
        return Bookmarks(self._owner, self._path, self)

    @property
    def title(self):
        return self._value()["title"]

    @title.setter
    def title(self, value: str):
        self._check()
        self._owner._call("update_bookmark", list(self._path), title=value, _change=True)

    @property
    def page(self):
        return self._value()["page"]

    @page.setter
    def page(self, value: int):
        self._check()
        self._owner._call("update_bookmark", list(self._path), page=_index(value, self._owner.page_count), _change=True)

    @property
    def expanded(self):
        return self._value()["open"]

    @expanded.setter
    def expanded(self, value: bool):
        self._check()
        self._owner._call("update_bookmark", list(self._path), open=value, _change=True)


class PdfPages(_Collection["PdfPage"]):
    def __init__(self, owner):
        super().__init__(owner, lambda: range(owner.page_count), PdfPage)

    def add(self, *, size: _Size = "Letter") -> PdfPage:
        """Append a blank page; dimensions use points or a named paper size."""
        identifier = self._owner._call("add_page", *_size(size), _change=True)
        return PdfPage(self._owner, identifier)

    def __delitem__(self, index):
        self._owner._call("delete_page", _index(index, len(self)), _change=True)
        self._owner._page_generation += 1

    def move(self, index: int, destination: int) -> None:
        self._owner._call("move_page", _index(index, len(self)), _index(destination, len(self)), _change=True)
        self._owner._page_generation += 1

    def insert(self, index: int, source: _Source | EditableDocument | Document, *, pages: Iterable[SupportsIndex] | None = None, password: str | None = None, outline_title: str | None = None) -> list[PdfPage]:
        """Import ordered PDF pages and return fresh handles; organization invalidates prior page handles."""
        from . import _read, _selected_pages
        self._owner._ensure_open()
        generation, page_count = self._owner._page_generation, self._owner.page_count
        index = operator.index(index)
        self._owner._ensure_open()
        if (generation, page_count) != (self._owner._page_generation, self._owner.page_count):
            raise ValueError("page selection is stale after page organization")
        if not 0 <= index <= page_count:
            raise IndexError("insertion index must be between zero and page count")
        if isinstance(source, EditableDocument):
            if source.filetype != "pdf":
                raise UnsupportedFormatError("page insertion requires a PDF source")
            data = source.to_bytes()
        elif hasattr(source, "_source"):
            source._ensure_open()
            if source._filetype != "pdf":
                raise UnsupportedFormatError("page insertion requires a PDF source")
            data = source._source
            password = source._password if password is None else password
        else:
            data, _ = _read(source)
        indices = _selected_pages(pages)
        self._owner._ensure_open()
        if (generation, page_count) != (self._owner._page_generation, self._owner.page_count):
            raise ValueError("page selection is stale after page organization")
        count = self._owner._call("import_pages", data, password, indices, index, outline_title, _change=True)
        self._owner._page_generation += 1
        return [PdfPage(self._owner, position) for position in range(index, index + count)]


class PdfPage:
    def __init__(self, owner: PdfDocument, identifier: int):
        self._owner, self._id = owner, identifier
        self._generation = owner._page_generation

    def _check(self):
        self._owner._ensure_open()
        if self._generation != self._owner._page_generation:
            raise ValueError("page handle is stale after page organization; retrieve it again")

    @property
    def number(self):
        self._check()
        return self._id

    @property
    def size(self):
        self._check()
        return self._owner._call("page_size", self._id)

    @property
    def boxes(self) -> dict[str, tuple[float, float, float, float]]:
        """Effective PDF boxes as (x, y, width, height) in current view points."""
        self._check()
        return {name: tuple(box) for name, box in json.loads(self._owner._call("page_boxes", self._id)).items()}

    def set_box(self, kind: str, box=None) -> None:
        """Set media/crop/bleed/trim/art geometry; None resets a non-media box."""
        self._check()
        if box is not None:
            if not isinstance(box, (tuple, list)) or len(box) != 4:
                raise TypeError("box must be (x, y, width, height) in current view points")
            box = tuple(map(pt, box))
        self._owner._call("set_page_box", self._id, kind, box, _change=True)
        self._owner._page_generation += 1
        self._generation = self._owner._page_generation

    @property
    def background(self):
        self._check()
        value = self._owner._call("background", self._id)
        return None if value is None else _hex_color(value)

    @background.setter
    def background(self, value):
        self._check()
        self._owner._call("set_background", self._id, _color(value), _change=True)

    def rotate(self, degrees: int):
        self._check()
        self._owner._call("rotate_page", self._id, degrees, _change=True)
        self._owner._page_generation += 1
        self._generation = self._owner._page_generation

    @property
    def fields(self) -> PdfFields:
        self._check()
        return PdfFields(self._owner, self)

    @property
    def annotations(self) -> Annotations:
        self._check()
        return Annotations(self)

    def draw_text(self, text: str, *, at: tuple[float, float] | None = None,
                  box: tuple[float, float, float, float] | None = None,
                  font_size: float = 12, font=None, color="black", align: str = "left"):
        """Draw searchable text at a point or inside a bounded top-left box.

        Built-in fonts wrap within boxes; supplied fonts require explicit lines.
        Overflow fails atomically. Alignment applies only to boxes.
        """
        self._check()
        if (at is None) == (box is None):
            raise ValueError("provide exactly one of at or box")
        if not isinstance(align, str) or align not in {"left", "center", "right", "justify"}:
            raise ValueError("align must be left, center, right or justify")
        if box is not None:
            if not isinstance(box, (tuple, list)) or len(box) != 4:
                raise TypeError("box must be (x, y, width, height) in top-left points")
            rect = tuple(map(pt, box))
            data = None
            if font is not None:
                from . import _read
                data, _ = _read(font, limit=16 * 1024 * 1024)
            self._owner._call("add_text_box", self._id, text, rect, pt(font_size),
                              _color(color), align, data, _change=True)
            return
        if align != "left":
            raise ValueError("alignment requires a text box")
        if not isinstance(at, (tuple, list)) or len(at) != 2:
            raise TypeError("at must be an (x, y) pair in top-left points")
        if font is None:
            self._owner._call("add_text", self._id, text, pt(at[0]), pt(at[1]), pt(font_size), _color(color), _change=True)
        else:
            from . import _read
            data, _ = _read(font, limit=16 * 1024 * 1024)
            self._owner._call("add_text_with_font", self._id, text, pt(at[0]), pt(at[1]), pt(font_size), data, _color(color), _change=True)

    def draw_path(self, commands, *, stroke="black", fill=None, width: float = 1, close: bool = False):
        """Draw move/line/curve commands in top-left points through the native writer."""
        self._check()
        normalized = []
        for command in commands:
            if len(normalized) >= 10000:
                raise ValueError("path must not exceed 10000 commands")
            if not isinstance(command, (tuple, list)) or not command:
                raise TypeError("path commands must be tuples beginning with move, line or curve")
            kind, *coordinates = command
            if kind not in {"move", "line", "curve"} or len(coordinates) != (6 if kind == "curve" else 2):
                raise ValueError("move/line require two coordinates; curve requires six")
            normalized.append([kind, *map(pt, coordinates)])
        payload = json.dumps(normalized, separators=(",", ":"), allow_nan=False)
        if len(payload) > 262144:
            raise ValueError("path commands exceed 256 KiB")
        self._owner._call("draw_path", self._id, payload, None if stroke is None else _color(stroke), None if fill is None else _color(fill), pt(width), close, _change=True)

    def add_image(self, source, *, box, crop=None, rotation: int = 0,
                  flip_h: bool = False, flip_v: bool = False):
        """Place an image with optional visual crop, quarter-turn and flips.

        Crop is (left, top, right, bottom), each a fraction from 0 to 0.45.
        Rotation is counterclockwise. Original embedded pixels are retained.
        """
        self._check()
        from . import _read
        if not isinstance(box, (tuple, list)) or len(box) != 4:
            raise TypeError("box must be (x, y, width, height) in points")
        if crop is not None:
            if not isinstance(crop, (tuple, list)) or len(crop) != 4:
                raise TypeError("crop must be (left, top, right, bottom) fractions")
            crop = tuple(map(pt, crop))
            if any(value < 0 or value > 0.45 for value in crop):
                raise ValueError("crop fractions must be between 0 and 0.45")
        if isinstance(rotation, bool):
            raise TypeError("rotation must be an integer quarter-turn")
        rotation = operator.index(rotation)
        if rotation not in (0, 90, 180, 270):
            raise ValueError("rotation must be 0, 90, 180 or 270 degrees")
        if not isinstance(flip_h, bool) or not isinstance(flip_v, bool):
            raise TypeError("flip_h and flip_v must be booleans")
        data, _ = _read(source)
        self._owner._call("add_image", self._id, data, *map(pt, box), crop,
                          rotation, flip_h, flip_v, _change=True)

    def get_pixmap(self, dpi: float = 144, *, clip: tuple[float, float, float, float] | None = None) -> bytes:
        self._check()
        return self._owner.render_page(self._id, dpi, clip=clip)

    @overload
    def get_text(self, option: Literal["text", "markdown", "json"] = "text") -> str: ...

    @overload
    def get_text(self, option: Literal["dict"]) -> dict[str, Any]: ...

    @overload
    def get_text(self, option: str) -> str | dict[str, Any]: ...

    def get_text(self, option: str = "text") -> str | dict[str, Any]:
        """Extract text, Markdown, JSON, or an owned dict from the current native snapshot."""
        self._check()
        return self._owner._snapshot()[self._id].get_text(option)


def _annotation_rect(box):
    if not isinstance(box, (tuple, list)) or len(box) != 4:
        raise TypeError("box must be (x, y, width, height) in points")
    x, y, width, height = map(pt, box)
    if width <= 0 or height <= 0:
        raise ValueError("annotation dimensions must be positive")
    return x, y, x + width, y + height


class Annotations(_Collection["Annotation"]):
    def __init__(self, page: PdfPage):
        self._page = page
        def identifiers():
            page._check()
            return [value["index"] for value in json.loads(page._owner._call("annotations", page.number))]
        super().__init__(page._owner, identifiers, lambda owner, identifier: Annotation(page, identifier))

    def add(self, kind: str, *, box: _Box, text: str = "", author: str = "", color: _Color = "#FFD200") -> Annotation:
        """Add a supported annotation in a top-left (x, y, width, height) point box."""
        self._page._check()
        rect, rgb = _annotation_rect(box), _color(color)
        self._page._check()
        identifier = self._owner._call("add_annotation", self._page.number, kind, rect, text, author, rgb, _change=True)
        return Annotation(self._page, identifier)

    def add_link(self, *, box: _Box, uri: str | None = None, page: int | None = None) -> Annotation:
        """Add a URL or zero-based page link in a top-left point box."""
        self._page._check()
        rect = _annotation_rect(box)
        target = None if page is None else _index(page, self._owner.page_count)
        self._page._check()
        identifier = self._owner._call("add_link", self._page.number, rect, uri, target, _change=True)
        return Annotation(self._page, identifier)

    def __delitem__(self, index):
        annotation = self[index]
        page = self._page.number
        self._owner._call("delete_annotation", page, annotation._id, _change=True)
        self._owner._annotation_generations[page] = self._owner._annotation_generations.get(page, 0) + 1


class Annotation:
    """A checked PDF annotation handle; deletion invalidates page annotation handles."""
    def __init__(self, page: PdfPage, identifier: int):
        self._page, self._id = page, identifier
        self._generation = page._owner._annotation_generations.get(page.number, 0)
        self._page_generation = page._owner._page_generation

    def _check(self):
        page = self._page
        page._check()
        if page._owner._page_generation != self._page_generation:
            raise ValueError("annotation handle is stale after page organization; retrieve it again")
        if page._owner._annotation_generations.get(page.number, 0) != self._generation:
            raise ValueError("annotation handle is stale after deletion; retrieve it again")

    def _value(self):
        self._check()
        values = json.loads(self._page._owner._call("annotations", self._page.number))
        for value in values:
            if value["index"] == self._id:
                return value
        raise ValueError("annotation no longer exists")

    def update(self, *, text: str | None = None, box=None, color=None, uri: str | None = None, page: int | None = None, author: str | None = None):
        self._check()
        rect = None if box is None else _annotation_rect(box)
        rgb = None if color is None else _color(color)
        target = None if page is None else _index(page, self._page._owner.page_count)
        self._check()
        self._page._owner._call("update_annotation", self._page.number, self._id,
            text=text, rect=rect, color=rgb, uri=uri, target_page=target,
            author=author, _change=True)

    @property
    def kind(self):
        return self._value()["kind"]

    @property
    def box(self):
        x0, y0, x1, y1 = self._value()["rect"]
        return x0, y0, x1 - x0, y1 - y0

    @box.setter
    def box(self, value):
        self.update(box=value)

    @property
    def text(self):
        return self._value()["text"]

    @text.setter
    def text(self, value: str):
        self.update(text=value)

    @property
    def author(self):
        return self._value()["author"]

    @author.setter
    def author(self, value: str):
        self.update(author=value)

    @property
    def color(self):
        value = self._value()["color"]
        return None if value is None else _hex_color(value)

    @color.setter
    def color(self, value):
        self.update(color=value)

    @property
    def uri(self):
        return self._value()["uri"]

    @uri.setter
    def uri(self, value: str):
        self.update(uri=value)

    @property
    def target_page(self):
        return self._value()["target_page"]

    @target_page.setter
    def target_page(self, value: int):
        self.update(page=value)


class Presentation(EditableDocument):
    _format = "pptx"

    @property
    def page_count(self):
        return self._call("slide_count")

    @property
    def slides(self) -> Slides:
        return Slides(self)


class Slides(_Collection["Slide"]):
    def __init__(self, owner):
        super().__init__(owner, lambda: [owner._call("slide_id", index) for index in range(owner.page_count)], Slide)

    def add(self, *, layout: str = "blank") -> Slide:
        """Add a slide using one supported native authored layout."""
        index = self._owner._call("add_slide", layout, _change=True)
        return Slide(self._owner, self._owner._call("slide_id", index))

    def __delitem__(self, index):
        self._owner._call("remove_slide", _index(index, len(self)), _change=True)

    def move(self, index: int, destination: int) -> None:
        self._owner._call("move_slide", _index(index, len(self)), _index(destination, len(self)), _change=True)


class Slide:
    def __init__(self, owner: Presentation, identifier: int):
        self._owner, self._id = owner, identifier

    @property
    def number(self):
        return self._owner._call("slide_index", self._id)

    @property
    def size(self):
        self.number
        return self._owner._call("slide_size")

    @property
    def name(self):
        return self._owner._call("slide_name", self.number)

    @name.setter
    def name(self, value):
        self._owner._call("set_slide_name", self.number, value, _change=True)

    @property
    def hidden(self):
        return self._owner._call("slide_hidden", self.number)

    @hidden.setter
    def hidden(self, value):
        self._owner._call("set_slide_hidden", self.number, value, _change=True)

    @property
    def notes(self):
        return self._owner._call("slide_notes", self.number)

    @notes.setter
    def notes(self, value):
        self._owner._call("set_slide_notes", self.number, value, _change=True)

    @property
    def background(self):
        value = self._owner._call("slide_background", self.number)
        return None if value is None else _hex_color(value)

    @background.setter
    def background(self, color):
        self._owner._call("background", self.number, _color(color), _change=True)

    @property
    def shapes(self) -> Shapes:
        return Shapes(self)

    @property
    def background_image(self):
        value = self._owner._call("background_image", self.number)
        return None if value is None else bytes(value)

    @background_image.setter
    def background_image(self, source):
        from . import _read
        data, _ = _read(source)
        self._owner._call("set_background_image", self.number, data, _change=True)

    def add_shape(self, kind: str, *, box: _Box, fill: _Color | None = None, stroke: _Color | None = None, stroke_width: float = 1) -> Shape:
        """Add a supported shape and return its handle; box is top-left x/y/width/height in points."""
        if not isinstance(box, (tuple, list)) or len(box) != 4:
            raise TypeError("box must be (x, y, width, height) in points")
        coordinates = tuple(map(pt, box))
        fill = None if fill is None else _color(fill)
        stroke = None if stroke is None else _color(stroke)
        stroke_width = pt(stroke_width)
        identifier = self._owner._call("add_shape", self.number, kind, *coordinates, fill, stroke, stroke_width, _change=True)
        return Shape(self._owner, self._id, identifier)

    def add_text(self, text: str, *, box: _Box, font_size: float = 18) -> Shape:
        """Add a text shape in a top-left point box and return its editable handle."""
        if not isinstance(box, (tuple, list)) or len(box) != 4:
            raise TypeError("box must be (x, y, width, height) in points")
        coordinates = tuple(map(pt, box))
        font_size = pt(font_size)
        identifier = self._owner._call("add_text", self.number, text, *coordinates, font_size, _change=True)
        return Shape(self._owner, self._id, identifier)

    def add_table(self, rows: _Rows, *, columns: SupportsIndex | None = None, box: _Box, font_size: float = 18) -> SlideTable:
        """Add string rows or a counted grid in a top-left point box; return its table handle."""
        if not isinstance(box, (tuple, list)) or len(box) != 4:
            raise TypeError("box must be (x, y, width, height) in points")
        rows = _rows(rows, columns)
        coordinates = tuple(map(pt, box))
        font_size = pt(font_size)
        identifier = self._owner._call("add_table", self.number, rows, *coordinates, font_size, _change=True)
        return SlideTable(self._owner, identifier, self._id)

    def add_image(self, source: _Source, *, box: _Box, alt: str = "") -> SlidePicture:
        """Add a PNG in a top-left point box and return its picture handle."""
        from . import _read
        if not isinstance(box, (tuple, list)) or len(box) != 4:
            raise TypeError("box must be (x, y, width, height) in points")
        coordinates = tuple(map(pt, box))
        data, _ = _read(source, limit=16 * 1024 * 1024)
        identifier = self._owner._call("add_image", self.number, data, *coordinates, alt, _change=True)
        return SlidePicture(self._owner, self._id, identifier)

    def get_pixmap(self, dpi: float = 144, *, clip: tuple[float, float, float, float] | None = None) -> bytes:
        return self._owner.render_page(self.number, dpi, clip=clip)

    @overload
    def get_text(self, option: Literal["text", "markdown", "json"] = "text") -> str: ...

    @overload
    def get_text(self, option: Literal["dict"]) -> dict[str, Any]: ...

    @overload
    def get_text(self, option: str) -> str | dict[str, Any]: ...

    def get_text(self, option: str = "text") -> str | dict[str, Any]:
        """Extract text, Markdown, JSON, or an owned dict from the current native snapshot."""
        return self._owner._snapshot()[self.number].get_text(option)


class Shapes(_Collection["Shape | SlideTable"]):
    def __init__(self, slide: Slide):
        self._slide_owner = slide
        def factory(owner, pair):
            identifier, kind = pair
            if kind == "table":
                return SlideTable(owner, identifier, slide._id)
            if kind == "image":
                return SlidePicture(owner, slide._id, identifier)
            return Shape(owner, slide._id, identifier)
        super().__init__(slide._owner, lambda: slide._owner._call("shape_ids", slide.number), factory)

    def __delitem__(self, index):
        shape = self[index]
        self._owner._call("delete_shape", self._slide_owner.number, shape._id, _change=True)

    def move(self, index: int, destination: int) -> None:
        shape = self[index]
        destination = _index(destination, len(self))
        self._owner._call("move_shape", self._slide_owner.number, shape._id, destination, _change=True)


class Shape:
    def __init__(self, owner: Presentation, slide: int, identifier: int):
        self._owner, self._slide, self._id = owner, slide, identifier

    @property
    def placeholder(self) -> str | None:
        """Native inherited placeholder role, or None for an ordinary shape."""
        return self._owner._call("shape_placeholder", self._slide_number, self._id)

    @property
    def _slide_number(self):
        return self._owner._call("slide_index", self._slide)

    @property
    def _range_args(self):
        return self._slide_number, self._id

    _native_text_method = "shape_text"

    def __getitem__(self, key: slice) -> _SlideTextRange:
        """Select contiguous Unicode text across runs and LF-separated paragraphs."""
        return _slice_range(self, key, _SlideTextRange)

    def add_run(self, text: str, **formatting: Unpack[CharacterFormatting]) -> _SlideTextRange:
        start, end, revision = self._owner._call("add_run_patch", *self._range_args, text, _format_patch(formatting, rgb_highlight=True), _change=True)
        return _SlideTextRange(self, start, end, revision)

    def find(self, text: str) -> list[_SlideTextRange]:
        """Find literal ranges across runs and LF-separated paragraphs."""
        return _find_ranges(self, text, _SlideTextRange)

    def replace_text(self, old: str, new: str) -> int:
        """Atomically replace literal matches while retaining other formatting."""
        return _replace_text(self, old, new)

    def _link_info(self):
        return self._owner._call("shape_link", self._slide_number, self._id)

    @property
    def link(self) -> str | Slide | None:
        return _slide_link_value(self._owner, self._link_info())

    @link.setter
    def link(self, target: str | Slide | None):
        url, slide_id = _slide_link_target(self._owner, target)
        previous = self._link_info()
        tooltip = previous[2] if previous is not None and target is not None else ""
        self._owner._call("set_shape_link", self._slide_number, self._id, url, slide_id, tooltip, _change=True)

    @property
    def link_tooltip(self) -> str:
        info = self._link_info()
        return "" if info is None else info[2]

    @link_tooltip.setter
    def link_tooltip(self, value: str):
        info = self._link_info()
        if info is None:
            raise ValueError("set a link before setting its tooltip")
        self._owner._call("set_shape_link", self._slide_number, self._id, info[0], info[1], value, _change=True)

    @property
    def rotation(self):
        return self._owner._call("shape_rotation", self._slide_number, self._id)

    @rotation.setter
    def rotation(self, degrees):
        self._owner._call("set_shape_rotation", self._slide_number, self._id, pt(degrees), _change=True)

    @property
    def fill(self):
        value = self._owner._call("shape_fill", self._slide_number, self._id)
        return None if value is None else _hex_color(value)

    @fill.setter
    def fill(self, color):
        self._owner._call("set_shape_fill", self._slide_number, self._id, None if color is None else _color(color), _change=True)

    @property
    def stroke(self):
        value = self._owner._call("shape_stroke", self._slide_number, self._id)
        return None if value is None else (_hex_color(value[0]), value[1])

    @stroke.setter
    def stroke(self, value):
        if value is None:
            color, width = None, 1
        elif isinstance(value, (tuple, list)) and len(value) == 2:
            color, width = _color(value[0]), pt(value[1])
        else:
            raise TypeError("stroke must be (color, width) or None")
        self._owner._call("set_shape_stroke", self._slide_number, self._id, color, width, _change=True)

    @property
    def box(self):
        return self._owner._call("shape_box", self._slide_number, self._id)

    @box.setter
    def box(self, value):
        if not isinstance(value, (tuple, list)) or len(value) != 4:
            raise TypeError("box must be (x, y, width, height) in points")
        self._owner._call("set_shape_box", self._slide_number, self._id, *map(pt, value), _change=True)

    @property
    def text(self):
        return self._owner._call("shape_text", self._slide_number, self._id)

    @text.setter
    def text(self, value):
        self._owner._call("set_shape_text", self._slide_number, self._id, value, _change=True)

    @property
    def color(self):
        value = self._owner._call("text_format", self._slide_number, self._id)[1]
        return None if value is None else _hex_color(value)

    @color.setter
    def color(self, value):
        self._owner._call("set_text_format", self._slide_number, self._id, color=_color(value), _change=True)

    @property
    def font_size(self):
        return self._owner._call("text_format", self._slide_number, self._id)[0]

    @font_size.setter
    def font_size(self, value):
        self._owner._call("set_text_format", self._slide_number, self._id, size=pt(value), _change=True)

    @property
    def bold(self):
        return self._owner._call("text_format", self._slide_number, self._id)[2]

    @bold.setter
    def bold(self, value: bool):
        self._owner._call("set_text_format", self._slide_number, self._id, bold=value, _change=True)

    @property
    def italic(self):
        return self._owner._call("text_format", self._slide_number, self._id)[3]

    @italic.setter
    def italic(self, value: bool):
        self._owner._call("set_text_format", self._slide_number, self._id, italic=value, _change=True)

    @property
    def font(self):
        return self._owner._call("text_format", self._slide_number, self._id)[4]

    @font.setter
    def font(self, value: str):
        self._owner._call("set_text_format", self._slide_number, self._id, font=value, _change=True)


class SlidePicture(Shape):
    """A native picture placement retaining its source and editable transforms."""

    @property
    def data(self) -> bytes:
        return bytes(self._owner._call("picture_data", self._slide_number, self._id))

    @data.setter
    def data(self, source) -> None:
        from . import _read
        self._owner._ensure_open()
        data, _ = _read(source, limit=16 * 1024 * 1024)
        self._owner._call("set_picture_data", self._slide_number, self._id, data, _change=True)

    @property
    def alt(self) -> str:
        return self._owner._call("picture_alt", self._slide_number, self._id)

    @alt.setter
    def alt(self, value: str) -> None:
        self._owner._call("set_picture_alt", self._slide_number, self._id, value, _change=True)

    @property
    def crop(self) -> tuple[float, float, float, float]:
        return self._owner._call("picture_crop", self._slide_number, self._id)

    @crop.setter
    def crop(self, value) -> None:
        values = _picture_crop(value)
        self._owner._call("set_picture_crop", self._slide_number, self._id, values, _change=True)

    @property
    def flip_h(self) -> bool:
        return self._owner._call("picture_flips", self._slide_number, self._id)[0]

    @flip_h.setter
    def flip_h(self, value: bool) -> None:
        if not isinstance(value, bool):
            raise TypeError("flip_h must be a boolean")
        self._owner._call("set_picture_flips", self._slide_number, self._id, flip_h=value, _change=True)

    @property
    def flip_v(self) -> bool:
        return self._owner._call("picture_flips", self._slide_number, self._id)[1]

    @flip_v.setter
    def flip_v(self, value: bool) -> None:
        if not isinstance(value, bool):
            raise TypeError("flip_v must be a boolean")
        self._owner._call("set_picture_flips", self._slide_number, self._id, flip_v=value, _change=True)


class SlideTable(EditableTable, Shape):
    """A placed semantic table with the same cell access as a DOCX table."""


class _SlideCellParagraph(Paragraph):
    """A paragraph ordinal within a generation-checked, canonical PPTX cell."""
    def __init__(self, cell, target):
        self._cell, self._target = cell, target
        self._owner = cell._table._owner

    @property
    def _range_args(self):
        self._cell._check()
        return self._cell._table._args

    @property
    def _range_kwargs(self):
        return {"cell": self._target}

    _native_text_method = "shape_text"

    @property
    def text(self):
        return self._owner._call("shape_text", *self._range_args, **self._range_kwargs)

    @text.setter
    def text(self, value):
        self[:].text = value

    @property
    def alignment(self):
        return self._owner._call("cell_paragraph_alignment", *self._range_args, self._target)

    @alignment.setter
    def alignment(self, value):
        self._owner._call("set_cell_paragraph_alignment", *self._range_args, self._target, value, _change=True)

    @property
    def formatting(self):
        return {"alignment": self.alignment}

    def format(self, **formatting):
        if formatting.keys() - {"alignment"}:
            self._unsupported()
        if "alignment" in formatting:
            self.alignment = formatting["alignment"]
        else:
            _ = self.text  # Empty patches still validate the owner and handle.

    def add_run(self, text, **formatting):
        patch = _format_patch(formatting, rgb_highlight=True)
        start, end, revision = self._owner._call("add_run_patch", *self._range_args, text, patch,
                                               _change=True, **self._range_kwargs)
        return _SlideTextRange(self, start, end, revision)

    def __getitem__(self, key):
        return _slice_range(self, key, _SlideTextRange)

    def find(self, text):
        return _find_ranges(self, text, _SlideTextRange)

    def _unsupported(self, *args, **kwargs):
        self._owner._ensure_open()
        raise UnsupportedFormatError("this Word paragraph operation is unavailable in PPTX cells")

    style = property(_unsupported, _unsupported)
    list_style = property(_unsupported)
    page_break_before = property(_unsupported, _unsupported)
    spacing = property(_unsupported, _unsupported)
    resolved_formatting = property(_unsupported)
    set_list = _unsupported
    add_field = _unsupported
    add_break = _unsupported
