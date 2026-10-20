"""Assemble, reorder and split PDFs with existing page collection operations."""
from pathlib import Path

import doc_ex


def assemble_pdfs(output_dir: str | Path, *, dpi: float = 96) -> dict[str, Path]:
    """Write two source PDFs, an assembled PDF, a subset and page previews.

    A new PDF starts with one blank page. Insert real pages before deleting that
    blank; reacquire page handles after structural edits. Source files remain
    independent. This recipe does not promise imported document-level metadata.
    """
    directory = Path(output_dir)
    directory.mkdir(parents=True, exist_ok=True)
    paths = {key: directory / f"{key}.pdf" for key in ("source-a", "source-b", "assembled", "subset")}
    with doc_ex.new("pdf", size=(360, 240)) as source_a, doc_ex.new("pdf", size=(360, 240)) as source_b:
        for source, label, color in ((source_a, "Source A", "red"), (source_b, "Source B", "blue")):
            page = source.pages[0]
            page.draw_text(label, at=(30, 30), font_size=24, color=color)
            page.draw_path([("move", 30, 80), ("line", 100, 80), ("line", 100, 130),
                            ("line", 30, 130)], fill=color, stroke=None, close=True)
        source_a.pages[0].annotations.add_link(box=(30, 150, 150, 24), uri="https://example.com/source-a")
        source_a.pages[0].draw_text("Source details", at=(30, 150), color="blue")
        with doc_ex.new("pdf", size=(360, 240)) as assembled:
            assembled.pages.insert(1, source_a)
            assembled.pages.insert(2, source_b)
            del assembled.pages[0]
            assembled.pages.move(1, 0)  # Final order: Source B, Source A.
            assembled.save(paths["assembled"])
            with assembled.copy(pages=[1]) as subset:
                subset.save(paths["subset"])
            for page in range(assembled.page_count):
                path = directory / f"assembled-{page + 1}.png"
                path.write_bytes(assembled.render_page(page, dpi=dpi))
                paths[f"page_{page + 1}"] = path
        # Save the still-independent sources after the assembly operations.
        source_a.save(paths["source-a"])
        source_b.save(paths["source-b"])
    return paths


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--dpi", type=float, default=96)
    args = parser.parse_args()
    assemble_pdfs(args.output_dir, dpi=args.dpi)


if __name__ == "__main__":
    main()
