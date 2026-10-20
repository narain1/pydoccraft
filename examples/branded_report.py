"""Create a two-page report using a named style, stories and a semantic table."""
from pathlib import Path

import doc_ex
from doc_ex import mm


def create_report(output_dir: str | Path, *, dpi: float = 96) -> dict[str, Path]:
    """Write report.docx, report.pdf and one preview per page.

    The branding uses a text wordmark and solid page background. Headers and
    footers are separate stories; no external assets or fonts are needed.
    """
    directory = Path(output_dir)
    directory.mkdir(parents=True, exist_ok=True)
    paths = {kind: directory / f"report.{kind}" for kind in ("docx", "pdf")}
    with doc_ex.new("docx", size="A4") as report:
        section = report.sections[0]
        section.margins = mm(20)
        report.background = "#F0F4F8"
        section.header().add_paragraph("EXAMPLE STUDIO").find("EXAMPLE STUDIO")[0].format(color="navy", bold=True)
        section.footer().add_paragraph("Internal report | October 2026")
        title = report.styles.add("Brand heading", based_on="Heading 1")
        title.format(color="blue", font_size=24, space_after=12)
        report.add_paragraph("Quarterly overview", style=title)
        report.add_paragraph("Delivery remained on schedule throughout the quarter.")
        table = report.add_table([["Measure", "Result"], ["Completed projects", "12"], ["Client satisfaction", "96%"]])
        table.column_widths = [300, 180]
        table.borders = ("navy", 0.75)
        table.row(0).repeat_header = True
        for column in range(2):
            table[0, column].background = "#DDEEFF"
        report.add_page_break()
        report.add_paragraph("Next quarter", style=title)
        report.add_paragraph("Expand the training programme and review delivery capacity.")
        report.add_paragraph("Owner: Alex Morgan | Review date: 15 January 2027")
        report.save(paths["docx"])
        paths["pdf"].write_bytes(report.convert("pdf"))
        for page in range(report.page_count):
            path = directory / f"report-{page + 1}.png"
            path.write_bytes(report.render_page(page, dpi=dpi))
            paths[f"page_{page + 1}"] = path
    return paths


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--dpi", type=float, default=96)
    args = parser.parse_args()
    create_report(args.output_dir, dpi=args.dpi)


if __name__ == "__main__":
    main()
