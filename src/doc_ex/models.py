from __future__ import annotations

from dataclasses import dataclass, field
from html import escape
from typing import Any, Callable, Iterable, Literal
from urllib.parse import urlsplit

BBox = tuple[float, float, float, float]


@dataclass(frozen=True, slots=True)
class EpubChapterSource:
    """XHTML and navigation title for a new spine entry."""

    title: str
    xhtml: str
    path: str | None = None
    fixed_layout: bool = False
    linear: bool = True


@dataclass(frozen=True, slots=True)
class EpubSvgChapterSource:
    """Static SVG and navigation title for a new fixed-layout spine entry."""

    title: str
    svg: str
    path: str | None = None
    linear: bool = True


@dataclass(frozen=True, slots=True)
class EpubSpineProperties:
    """Edit reading order participation and rendition of an existing spine entry."""

    linear: bool | None = None
    layout: Literal["reflowable", "pre-paginated", "inherit"] | None = None
    page_spread: Literal["left", "right", "center", "inherit"] | None = None
    spread: Literal["none", "landscape", "portrait", "both", "auto", "inherit"] | None = None


@dataclass(frozen=True, slots=True)
class EpubResource:
    """Media type and bytes for a local publication resource."""

    media_type: str
    data: bytes


@dataclass(frozen=True, slots=True)
class EpubOverlayCue:
    """A chapter fragment and its optional local narration interval in microseconds."""

    fragment: str | None = None
    audio_path: str | None = None
    clip_begin_us: int = 0
    clip_end_us: int | None = None
    epub_type: str | None = None


@dataclass(frozen=True, slots=True)
class EpubOverlaySequence:
    """An ordered group of chapter cues with an optional fragment and inherited roles."""

    children: Iterable[EpubOverlayCue | EpubOverlaySequence]
    fragment: str | None = None
    epub_type: str | None = None


@dataclass(frozen=True, slots=True)
class EpubAudioClip:
    """Owned signed 16-bit PCM WAVE and its sample geometry."""

    wave: bytes
    sample_rate: int
    channels: int
    frame_count: int


@dataclass(frozen=True, slots=True)
class EpubTextCue:
    """Plain text and an interval on the embedded media presentation timeline."""

    start_us: int
    end_us: int | None
    text: str
    id: str | None = None
    settings: str | None = None
    region: str | None = None
    markup: str | None = None


@dataclass(frozen=True, slots=True)
class EpubVideoFrame:
    """Owned packed RGB8 pixels and a frame interval on the presentation timeline."""

    rgb: bytes
    width: int
    height: int
    pixel_aspect_ratio: tuple[int, int]
    start_us: int
    duration_us: int
    text_cues: tuple[EpubTextCue, ...] = ()


@dataclass(frozen=True, slots=True)
class EpubMediaSource:
    """An authored URL and its optional local publication resource."""

    uri: str
    path: str | None
    media_type: str | None
    playable_audio: bool
    fragment_start_us: int = 0
    fragment_end_us: int | None = None
    playable_video: bool = False
    fragment_xywh: tuple[Literal["pixel", "percent"], int, int, int, int] | None = None
    fragment_tracks: tuple[str, ...] = ()
    fragment_id: str | None = None


@dataclass(frozen=True, slots=True)
class EpubMediaTrack:
    """A local caption, subtitle, description or metadata track."""

    path: str
    media_type: str
    kind: str
    label: str | None
    language: str | None
    default: bool


@dataclass(frozen=True, slots=True)
class EpubEmbeddedMedia:
    """An XHTML audio or video element and its ordered resource references."""

    kind: Literal["audio", "video"]
    id: str | None
    label: str | None
    sources: tuple[EpubMediaSource, ...]
    tracks: tuple[EpubMediaTrack, ...]
    poster_path: str | None


@dataclass(frozen=True, slots=True)
class EpubNarrationCue:
    """Playable narration for one SMIL target."""

    id: str | None
    path: str
    fragment: str | None
    roles: tuple[str, ...]
    audio: EpubAudioClip | None = None
    text: str | None = None


@dataclass(slots=True)
class Link:
    uri: str
    text: str = ""
    bbox: BBox | None = None
    page: int | None = None
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"uri": self.uri, "text": self.text, "bbox": self.bbox, "page": self.page, "metadata": self.metadata}


@dataclass(slots=True)
class Image:
    name: str
    data: bytes
    media_type: str = "application/octet-stream"
    alt: str = ""
    bbox: BBox | None = None
    page: int | None = None
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"name": self.name, "media_type": self.media_type, "alt": self.alt, "bbox": self.bbox, "page": self.page, "size": len(self.data), "metadata": self.metadata}


@dataclass(slots=True)
class Cell:
    """Owned cell content in source order, including nested table blocks."""

    blocks: list[Block] = field(default_factory=list)
    col_span: int = 1
    row_span: int = 1
    covered: bool = False
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"blocks": [block.to_dict() for block in self.blocks], "col_span": self.col_span, "row_span": self.row_span, "covered": self.covered, "metadata": self.metadata}


@dataclass(slots=True)
class Table:
    rows: list[list[str]]
    bbox: BBox | None = None
    page: int | None = None
    metadata: dict[str, Any] = field(default_factory=dict)
    cells: list[list[Cell]] = field(default_factory=list)

    def to_markdown(self) -> str:
        return self._markdown()

    def _markdown(self, image_reference: Callable[[Image, tuple], str] | None = None, path: tuple = ()) -> str:
        # Pipe tables cannot represent nested tables, merged cells or rich content.
        # HTML preserves that hierarchy without emitting the flat rows a second time.
        declared_headers = any("header" in cell.metadata for row in self.cells for cell in row)
        simple_headers = bool(self.cells and self.cells[0]) and all(
            cell.metadata.get("header") is True for cell in self.cells[0]
        ) and all(cell.metadata.get("header") is not True for row in self.cells[1:] for cell in row)
        source_attributes = ("html_id", "html_scope", "html_headers", "html_abbr")
        if (
            "html_row_groups" in self.metadata
            or (declared_headers and not simple_headers)
            or any(any(key in cell.metadata for key in source_attributes) for row in self.cells for cell in row)
            or any(
                cell.col_span != 1 or cell.row_span != 1 or cell.covered
                or cell.metadata.get("grid_column", column) != column
                or any(block.table or block.links or block.images for block in cell.blocks)
                for row in self.cells for column, cell in enumerate(row)
            )
        ):
            return self._to_html(image_reference, path)
        if not self.rows:
            return ""
        width = max(len(row) for row in self.rows)
        normalized = [row + [""] * (width - len(row)) for row in self.rows]
        escaped = [[cell.replace("|", "\\|").replace("\n", "<br>") for cell in row] for row in normalized]
        header = escaped[0]
        body = escaped[1:]
        lines = ["| " + " | ".join(header) + " |", "| " + " | ".join(["---"] * width) + " |"]
        lines.extend("| " + " | ".join(row) + " |" for row in body)
        return "\n".join(lines)

    def to_dict(self) -> dict[str, Any]:
        value = {"rows": self.rows, "bbox": self.bbox, "page": self.page, "metadata": self.metadata}
        if self.cells:
            value["cells"] = [[cell.to_dict() for cell in row] for row in self.cells]
        return value

    def _to_html(self, image_reference: Callable[[Image, tuple], str] | None = None, path: tuple = ()) -> str:
        rows = self.cells or [[Cell([Block("paragraph", text)]) for text in row] for row in self.rows]
        groups = self._html_groups(len(rows))
        groups_by_start: dict[int, list[tuple[str, int, str | None]]] = {}
        ends: dict[int, str] = {}
        row_group_end: list[int | None] = [None] * len(rows)
        for kind, start, count, group_id in groups:
            groups_by_start.setdefault(start, []).append((kind, count, group_id))
            if count:
                ends[start + count] = kind
                for index in range(start, start + count):
                    row_group_end[index] = start + count
        output = ["<table>"]
        occupied: dict[int, int] = {}
        for row_number, row in enumerate(rows):
            for kind, count, group_id in groups_by_start.get(row_number, []):
                attribute = f' id="{escape(group_id, quote=True)}"' if group_id is not None else ""
                output.append(f"<{kind}{attribute}>")
                if not count:
                    output.append(f"</{kind}>")
            output.append("<tr>")
            column = 0
            for cell_number, cell in enumerate(row):
                grid_column = cell.metadata.get("grid_column", column)
                if (
                    not isinstance(grid_column, int) or not 0 <= grid_column < 1000
                    or not isinstance(cell.col_span, int) or not 1 <= cell.col_span <= 1000
                    or grid_column + cell.col_span > 1000
                    or not isinstance(cell.row_span, int) or not 1 <= cell.row_span <= 32767
                ):
                    raise ValueError("structured table spans must fit a 1000-column, 32767-row grid")
                group_end = row_group_end[row_number]
                if group_end is not None and cell.row_span > group_end - row_number:
                    raise ValueError("structured table row span exceeds its row group")
                while column < grid_column:
                    if column not in occupied:
                        output.append("<td></td>")
                    column += 1
                column = grid_column + cell.col_span
                # Keep unexpected content in a covered source cell visible rather
                # than dropping it. Ordinary empty merge continuations are omitted.
                if cell.covered and not any(block.text or block.table or block.images for block in cell.blocks):
                    continue
                if cell.row_span > 1:
                    occupied.update((index, cell.row_span) for index in range(grid_column, column))
                tag = "th" if cell.metadata.get("header") is True else "td"
                attributes = f' colspan="{cell.col_span}" rowspan="{cell.row_span}"'
                for key, name in (("html_id", "id"), ("html_scope", "scope"), ("html_headers", "headers"), ("html_abbr", "abbr")):
                    if key in cell.metadata:
                        value = cell.metadata[key]
                        if not isinstance(value, str):
                            raise ValueError("structured table HTML attributes must be strings")
                        attributes += f' {name}="{escape(value, quote=True)}"'
                output.append(f"<{tag}{attributes}>")
                for block_number, block in enumerate(cell.blocks):
                    block_path = path + ("cells", row_number, cell_number, "blocks", block_number)
                    if block.table:
                        output.append(block.table._to_html(image_reference, block_path + ("table",)))
                    elif block.text:
                        output.append(f"<p>{_html_text(block.text, block.links)}</p>")
                    for image_number, image in enumerate(block.images):
                        reference = image_reference(image, block_path + ("images", image_number)) if image_reference else image.name
                        output.append(f'<img src="{escape(reference, quote=True)}" alt="{escape(image.alt, quote=True)}">')
                output.append(f"</{tag}>")
            output.append("</tr>")
            if row_number + 1 in ends:
                output.append(f'</{ends[row_number + 1]}>')
            occupied = {column: remaining - 1 for column, remaining in occupied.items() if remaining > 1}
        for kind, count, group_id in groups_by_start.get(len(rows), []):
            attribute = f' id="{escape(group_id, quote=True)}"' if group_id is not None else ""
            output.append(f"<{kind}{attribute}></{kind}>")
        output.append("</table>")
        return "".join(output)

    def _html_groups(self, row_count: int) -> list[tuple[str, int, int, str | None]]:
        raw = self.metadata.get("html_row_groups", [])
        if not isinstance(raw, list):
            raise ValueError("structured table row groups must be a list")
        result: list[tuple[str, int, int, str | None]] = []
        previous_end = 0
        for group in raw:
            if not isinstance(group, dict):
                raise ValueError("invalid structured table row group")
            kind, start, count = group.get("kind"), group.get("start_row"), group.get("row_count")
            group_id = group.get("id")
            if (
                kind not in ("thead", "tbody", "tfoot")
                or type(start) is not int or type(count) is not int
                or start < previous_end or count < 0 or start + count > row_count
                or (group_id is not None and not isinstance(group_id, str))
            ):
                raise ValueError("invalid structured table row group")
            result.append((kind, start, count, group_id))
            previous_end = start + count
        return result


def _html_text(text: str, links: list[Link]) -> str:
    output: list[str] = []
    position = 0
    valid = []
    for link in links:
        start, end = link.metadata.get("start"), link.metadata.get("end")
        if isinstance(start, int) and isinstance(end, int) and 0 <= start < end <= len(text) and text[start:end] == link.text:
            valid.append((start, end, link))
    selected = []
    last_start = len(text)
    for start, end, link in sorted(valid, key=lambda value: value[0], reverse=True):
        if end > last_start:
            continue
        selected.append((start, end, link))
        last_start = start
    for start, end, link in reversed(selected):
        output.append(escape(text[position:start]))
        uri = link.uri
        try:
            safe_scheme = urlsplit(uri).scheme.lower() in ("", "http", "https", "mailto")
        except ValueError:
            safe_scheme = False
        if any(ord(char) < 32 or ord(char) == 127 for char in uri) or not safe_scheme:
            output.append(escape(text[start:end]))
        else:
            output.append(f'<a href="{escape(uri, quote=True)}">{escape(text[start:end])}</a>')
        position = end
    output.append(escape(text[position:]))
    return "".join(output).replace("\n", "<br>")


@dataclass(slots=True)
class Block:
    kind: str
    text: str = ""
    bbox: BBox | None = None
    level: int | None = None
    style: str | None = None
    links: list[Link] = field(default_factory=list)
    images: list[Image] = field(default_factory=list)
    table: Table | None = None
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "text": self.text,
            "bbox": self.bbox,
            "level": self.level,
            "style": self.style,
            "links": [link.to_dict() for link in self.links],
            "images": [image.to_dict() for image in self.images],
            "table": self.table.to_dict() if self.table else None,
            "metadata": self.metadata,
        }


@dataclass(slots=True)
class Story:
    """Owned auxiliary content, independent of computed page placement."""

    kind: str
    blocks: list[Block] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {"kind": self.kind, "blocks": [block.to_dict() for block in self.blocks], "metadata": self.metadata}

    def get_text(self, option: str = "text") -> str | dict[str, Any]:
        if option == "dict":
            return self.to_dict()
        if option == "json":
            import json
            return json.dumps(self.to_dict(), ensure_ascii=False)
        from ._document import Page
        return Page(0, self.blocks).get_text(option)
