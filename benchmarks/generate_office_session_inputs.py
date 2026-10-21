"""Generate bounded synthetic fixtures with installed native document writers.

Defaults retain the original twenty-page workload. Larger text workloads and
unique high-entropy images exercise different parse/layout/decode costs; these
are generated workloads, not third-party document fidelity evidence.
"""
from pathlib import Path
import argparse
import hashlib
import json
import random
import struct
import zlib

import doc_ex


def chunk(tag, data):
    return (struct.pack(">I", len(data)) + tag + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff))


def png(size, seed=None):
    if seed is None:
        pixels = b"\xff\0\0\xff" * size * size
    else:
        pixels = bytearray(random.Random(seed).randbytes(size * size * 4))
        pixels[3::4] = b"\xff" * (size * size)
    rows = b"".join(b"\0" + pixels[y * size * 4:(y + 1) * size * 4]
                    for y in range(size))
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new output directory")
    parser.add_argument("--pages", type=int, default=20)
    parser.add_argument("--image-pixels", type=int, default=64)
    parser.add_argument("--unique-images", action="store_true")
    parser.add_argument("--pdf", action="store_true", help="also export and reopen both native PDF outputs")
    parser.add_argument("--image-points", type=int,
                        help="displayed square size; defaults to DOCX24/PPTX60")
    args = parser.parse_args()
    if not 1 <= args.pages <= 200:
        parser.error("pages must be between 1 and 200")
    if not 1 <= args.image_pixels <= 1024:
        parser.error("image-pixels must be between 1 and 1024")
    if args.image_points is not None and not 1 <= args.image_points <= 144:
        parser.error("image-points must be between 1 and 144")
    image_count = args.pages if args.unique_images else 1
    if image_count * args.image_pixels ** 2 * 4 > 48 * 1024 * 1024:
        parser.error("generated decoded media must fit the 48 MiB fixture budget")
    args.output.mkdir(parents=True, exist_ok=False)
    shared = None if args.unique_images else png(args.image_pixels)

    def image(number):
        return png(args.image_pixels, number) if shared is None else shared

    with doc_ex.new("docx", size=(612, 792)) as document:
        document.sections[0].margins = 36
        for page in range(args.pages):
            if page:
                document.add_page_break()
            document.add_paragraph(f"Synthetic report page {page + 1}", style="Heading 1")
            for row in range(12):
                document.add_paragraph(
                    f"Row {row + 1}: bounded native layout, shared rendering and retained document state.")
            document.add_table([["Item", "Count", "Unit", "Total"]]
                               + [[f"Item {n}", "2", "4.50", "9.00"] for n in range(4)])
            size = args.image_points or 24
            document.add_image(image(page), width=size, height=size)
        document.save(args.output / "report.docx")
    with doc_ex.new("pptx", size=(720, 540)) as document:
        for number in range(args.pages):
            slide = document.slides.add()
            slide.add_text(f"Synthetic slide {number + 1}", box=(30, 20, 640, 50), font_size=24)
            for row in range(8):
                slide.add_text(f"Row {row + 1}: shared native resources and model reuse",
                               box=(30, 80 + row * 30, 640, 28), font_size=14)
            slide.add_table([["Item", "Count"], ["A", "20"], ["B", "30"]],
                            box=(30, 350, 500, 120), font_size=14)
            size = args.image_points or 60
            slide.add_image(image(number), box=(570, 360, size, size))
        document.save(args.output / "slides.pptx")
    manifest = {"pages_requested": args.pages, "image_pixels": args.image_pixels,
                "unique_images": args.unique_images, "image_points": args.image_points,
                "package_version": doc_ex.__version__, "pdf_exports": args.pdf, "inputs": {}}
    names = ["report.docx", "slides.pptx"]
    if args.pdf:
        for name in tuple(names):
            path = args.output / name
            with doc_ex.open(path) as document:
                target = path.with_suffix(".pdf")
                target.write_bytes(document.convert("pdf"))
            names.append(target.name)
    for name in names:
        path = args.output / name
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        with doc_ex.open(path) as document:
            manifest["inputs"][name] = {"bytes": path.stat().st_size,
                                       "pages": len(document), "sha256": digest}
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
