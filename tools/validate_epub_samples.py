"""Validate local EPUB 3 samples from the W3C 20230704 release."""
from __future__ import annotations

import argparse
import wave
from hashlib import sha256
from io import BytesIO
from pathlib import Path
from struct import unpack_from

import doc_ex


SAMPLES = {
    "childrens-literature.epub": ("191d14d07bcaefde647e494256f73d789f265c8edc46c404320641b23c86b76a", 3, 119),
    "internallinks.epub": ("deaf03a1843f52cc5e4cedb226ca8659610efe1be5e810fccf615bcb549238aa", 7, 41),
    "cole-voyage-of-life.epub": ("e38ed606ff604e638f737efaf66e0fd0b2997c3eab7432bcba9a00a81a925cf3", 10, 0),
    "cc-shared-culture.epub": ("7bec3acf5ca153fd8a3b8bba50549c5cde3cb617ddd74c1d3b26fbc5eb9eebdf", 8, None),
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def png_size(data: bytes) -> tuple[int, int]:
    require(len(data) >= 24 and data.startswith(b"\x89PNG\r\n\x1a\n") and data[12:16] == b"IHDR",
            "Invalid preview PNG")
    width, height = unpack_from(">II", data, 16)
    require(width > 0 and height > 0, "Empty preview PNG")
    return width, height


def verify_reverse_spine(book: doc_ex.Document, link_count: int) -> None:
    chapters = [page.metadata["chapter_path"] for page in book]
    text = [page.get_text() for page in book]
    navigation = {
        item["path"] for item in book.metadata["manifest"]
        if "nav" in (item.get("properties") or "").split()
    }
    with doc_ex.open(book.edit_epub_structure(reversed(range(len(book))))) as edited:
        require([page.metadata["chapter_path"] for page in edited] == chapters[::-1], "Wrong spine order")
        require([page.get_text() for page in edited] == text[::-1], "Reordering changed chapter text")
        require(sum(len(page.get_links()) for page in edited) == link_count, "Reordering lost links")
        for path in set(chapters) - navigation:
            require(edited.read_epub_resource(path) == book.read_epub_resource(path), f"Changed member {path}")


def verify_fixed_pdf(book: doc_ex.Document, sizes: list[tuple[int, int]]) -> int:
    selected = [page.number for page in book if page.metadata.get("fixed_layout")]
    if selected:
        with doc_ex.open(book.export_epub_fixed_pdf(dpi=24, pages=selected, searchable=True)) as pdf:
            require(len(pdf) == len(selected), "Wrong number of exported PDF pages")
            for index, source in enumerate(selected):
                viewport = [pdf[index].metadata[key] for key in ("width", "height")]
                require(viewport == book[source].metadata["viewport"], "Changed PDF viewport")
                require(png_size(pdf.render_page(index, dpi=24)) == sizes[source], "Changed PDF raster size")
    return len(selected)


def verify_shared_culture(book: doc_ex.Document) -> None:
    alternatives = 0
    clip = {"start_us": 500_000, "end_us": 700_000}
    for page in book:
        for index, media in enumerate(book.epub_embedded_media(page.number)):
            for source, entry in enumerate(media.sources):
                if media.kind != "video" or (entry.media_type or "").partition(";")[0] != "video/webm":
                    continue
                frames = 0
                for chunk in book.epub_embedded_audio(page.number, index, source_index=source, **clip):
                    with wave.open(BytesIO(chunk.wave), "rb") as wav:
                        require((wav.getframerate(), wav.getnchannels(), wav.getsampwidth()) == (48_000, 2, 2),
                                "Wrong Vorbis soundtrack format")
                        require(wav.getnframes() == chunk.frame_count, "Wrong Vorbis PCM length")
                    frames += chunk.frame_count
                require(frames == 9_600, "Wrong Vorbis sample count for the clip")
                pictures = list(book.epub_embedded_video(page.number, index, source_index=source, **clip))
                require(len(pictures) == 6, "Wrong VP8 picture count for the clip")
                for picture in pictures:
                    require((picture.width, picture.height) == (720, 480), "Wrong VP8 picture size")
                    require(500_000 <= picture.start_us < picture.start_us + picture.duration_us <= 700_000,
                            "Wrong VP8 picture interval")
                alternatives += 1
    require(alternatives == 5, "Wrong number of WebM alternatives")


def verify_sample(directory: Path, name: str, checksum: str, chapters: int, links: int | None) -> None:
    with (directory / name).open("rb") as source:
        data = source.read(64 * 1024 * 1024 + 1)
    require(len(data) <= 64 * 1024 * 1024, "Sample exceeds 64 MiB")
    require(sha256(data).hexdigest() == checksum, "Unexpected sample contents")
    with doc_ex.open(data) as book:
        require(len(book) == chapters, "Wrong chapter count")
        sizes = []
        for page in book:
            clip = (0, 0, 500, 500) if name == "childrens-literature.epub" and page.number == 2 else None
            sizes.append(png_size(book.render_page(page.number, dpi=24, clip=clip)))
        if name == "childrens-literature.epub":
            require(sizes[2] == (167, 167), "Changed clipped preview size")
            png_size(book.render_page(2, dpi=48, clip=(30, 14_000, 300, 120)))
        if links is None:
            verify_shared_culture(book)
        else:
            require(sum(len(page.get_links()) for page in book) == links, "Wrong source link count")
            verify_reverse_spine(book, links)
        fixed = verify_fixed_pdf(book, sizes)
        require(fixed == (4 if name == "cole-voyage-of-life.epub" else 0), "Wrong fixed-layout page count")
    print(f"{name} passed ({chapters} chapter previews, {fixed} fixed-layout PDF pages)")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="Directory containing the four original EPUB files")
    args = parser.parse_args()
    for name, settings in SAMPLES.items():
        try:
            verify_sample(args.directory, name, *settings)
        except (OSError, ValueError, doc_ex.DocExError) as error:
            parser.exit(1, f"{name} failed. {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
