"""Compose a one-page letter. Run: python examples/letter.py OUTPUT_DIR --dpi 96."""
from pathlib import Path

import doc_ex
from doc_ex import mm


def create_letter(output_dir: str | Path, *, dpi: float = 96) -> dict[str, Path]:
    """Write a DOCX, its PDF export and a PNG preview; return their paths.

    Lengths are points (mm converts millimetres). Saving is explicit: leaving the
    context closes the document without saving it automatically.
    """
    directory = Path(output_dir)
    directory.mkdir(parents=True, exist_ok=True)
    paths = {kind: directory / f"letter.{kind}" for kind in ("docx", "pdf", "png")}
    with doc_ex.new("docx", size="A4") as letter:
        section = letter.sections[0]
        section.margins = mm(20)
        section.header().add_paragraph("EXAMPLE STUDIO").add_run(" | Client services", color="navy")
        section.footer().add_paragraph("Example Studio | example.com")
        letter.add_paragraph("20 October 2026")
        letter.add_paragraph("Mira Chen\nExample Client Company")
        letter.add_paragraph("Dear Mira,")
        paragraph = letter.add_paragraph("Your project is ready for ")
        paragraph.add_run("review", bold=True, color="navy")
        paragraph.add_run(".")
        letter.add_paragraph("Please send your comments by 30 October 2026.")
        link = letter.add_paragraph("Further details: ").add_run("Project guide")
        link.link = "https://example.com/project-guide"
        letter.add_paragraph("Kind regards,\nAlex Morgan")
        letter.save(paths["docx"])
        paths["pdf"].write_bytes(letter.convert("pdf"))
        paths["png"].write_bytes(letter.render_page(0, dpi=dpi))
    return paths


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--dpi", type=float, default=96)
    args = parser.parse_args()
    create_letter(args.output_dir, dpi=args.dpi)


if __name__ == "__main__":
    main()
