from __future__ import annotations

from dataclasses import dataclass, field
from html import escape
from typing import Any

BBox = tuple[float, float, float, float]


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
        # Pipe tables cannot represent nested tables, merged cells or rich content.
        # HTML preserves that hierarchy without emitting the flat rows a second time.
        if any(
            cell.col_span != 1 or cell.row_span != 1 or cell.covered
            or cell.metadata.get("grid_column", column) != column
            or any(block.table or block.links or block.images for block in cell.blocks)
            for row in self.cells for column, cell in enumerate(row)
        ):
            return self._to_html()
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

    def _to_html(self) -> str:
        rows = self.cells or [[Cell([Block("paragraph", text)]) for text in row] for row in self.rows]
        output = ["<table>"]
        occupied: dict[int, int] = {}
        for row in rows:
            output.append("<tr>")
            column = 0
            for cell in row:
                grid_column = cell.metadata.get("grid_column", column)
                if (
                    not isinstance(grid_column, int) or not 0 <= grid_column < 1000
                    or not isinstance(cell.col_span, int) or not 1 <= cell.col_span <= 1000
                    or grid_column + cell.col_span > 1000
                    or not isinstance(cell.row_span, int) or not 1 <= cell.row_span <= 32767
                ):
                    raise ValueError("structured table spans must fit a 1000-column, 32767-row grid")
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
                output.append(f'<td colspan="{cell.col_span}" rowspan="{cell.row_span}">')
                for block in cell.blocks:
                    if block.table:
                        output.append(block.table._to_html())
                    elif block.text:
                        output.append(f"<p>{_html_text(block.text, block.links)}</p>")
                    for image in block.images:
                        output.append(f'<img src="{escape(image.name, quote=True)}" alt="{escape(image.alt, quote=True)}">')
                output.append("</td>")
            output.append("</tr>")
            occupied = {column: remaining - 1 for column, remaining in occupied.items() if remaining > 1}
        output.append("</table>")
        return "".join(output)


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
        output.append(f'<a href="{escape(link.uri, quote=True)}">{escape(text[start:end])}</a>')
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
