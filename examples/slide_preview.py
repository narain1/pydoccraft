"""Create a two-slide deck and render saved slides at a chosen DPI, in process."""
from pathlib import Path

import doc_ex


def create_slide_preview(output_dir: str | Path, *, dpi: float = 96) -> dict[str, Path]:
    """Write slides.pptx, slides.pdf, slide PNGs and a small region preview.

    New PPTX documents start with zero slides. All boxes are (x, y, width,
    height) in points. The same render_page call also accepts PDF and DOCX.
    """
    directory = Path(output_dir)
    directory.mkdir(parents=True, exist_ok=True)
    paths = {kind: directory / f"slides.{kind}" for kind in ("pptx", "pdf")}
    with doc_ex.new("pptx", size=(480, 270)) as deck:
        first = deck.slides.add()
        first.background = "#F0F4F8"
        first.add_shape("rectangle", box=(20, 20, 60, 40), fill="red", stroke=None)
        first.add_text("Project overview", box=(100, 20, 350, 50), font_size=26)
        first.add_table([["Milestone", "Status"], ["Design", "Complete"]],
                        box=(30, 100, 420, 100), font_size=18)
        second = deck.slides.add()
        second.background = "#FFF4D6"
        second.add_shape("rectangle", box=(20, 20, 60, 40), fill="blue", stroke=None)
        second.add_text("Next steps", box=(100, 20, 350, 50), font_size=26)
        second.add_text("Review the prototype with the project team.",
                        box=(30, 100, 420, 100), font_size=20)
        deck.save(paths["pptx"])
        paths["pdf"].write_bytes(deck.convert("pdf"))
    # Reopen the saved file: previews exercise the same path used for input files.
    with doc_ex.open(paths["pptx"]) as document:
        for page in range(document.page_count):
            path = directory / f"slide-{page + 1}.png"
            path.write_bytes(document.render_page(page, dpi=dpi))
            paths[f"page_{page + 1}"] = path
        paths["clip"] = directory / "slide-1-detail.png"
        paths["clip"].write_bytes(document.render_page(0, dpi=dpi, clip=(20, 20, 60, 40)))
    return paths


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--dpi", type=float, default=96)
    args = parser.parse_args()
    create_slide_preview(args.output_dir, dpi=args.dpi)


if __name__ == "__main__":
    main()
