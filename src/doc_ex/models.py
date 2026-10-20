from __future__ import annotations

from dataclasses import dataclass, field
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
class Table:
    rows: list[list[str]]
    bbox: BBox | None = None
    page: int | None = None
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_markdown(self) -> str:
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
        return {"rows": self.rows, "bbox": self.bbox, "page": self.page, "metadata": self.metadata}


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
