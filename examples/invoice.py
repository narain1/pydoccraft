"""Build a small invoice with ordinary tables and Python Decimal arithmetic."""
from decimal import Decimal
from pathlib import Path

import doc_ex
from doc_ex import mm


def create_invoice(output_dir: str | Path, *, dpi: float = 96) -> dict[str, Path]:
    """Write invoice.docx, invoice.pdf and invoice.png; return their paths.

    Table rows are snapshots. Use table[row, column] to edit a cell. This example
    formats currency in Python; the document API is not a calculation engine.
    """
    directory = Path(output_dir)
    directory.mkdir(parents=True, exist_ok=True)
    paths = {kind: directory / f"invoice.{kind}" for kind in ("docx", "pdf", "png")}
    items = [("Support", 2, Decimal("125.00")), ("Training", 1, Decimal("300.00"))]
    rows = [["Service", "Quantity", "Amount"], *[
        [name, str(quantity), f"${quantity * price:.2f}"] for name, quantity, price in items
    ]]
    total = sum((quantity * price for _, quantity, price in items), Decimal("0"))
    with doc_ex.new("docx", size="A4") as invoice:
        invoice.sections[0].margins = mm(20)
        invoice.add_paragraph("INVOICE 2026-001", style="Heading 1")
        invoice.add_paragraph("Prepared for Example Client Company")
        invoice.add_paragraph("Issued 20 October 2026 | Due 30 October 2026")
        table = invoice.add_table(rows)
        table.column_widths = [270, 85, 125]
        table.borders = ("navy", 0.75)
        table.row(0).repeat_header = True
        for column in range(3):
            table[0, column].background = "#DDEEFF"
        for row in range(1, len(rows)):
            table[row, 1].alignment = "center"
            table[row, 2].alignment = "right"
        invoice.add_paragraph(f"Grand total: ${total:.2f}").format(space_before=12)
        terms = invoice.add_paragraph().add_run("Payment terms", color="blue")
        terms.link = "https://example.com/payment-terms"
        invoice.save(paths["docx"])
        paths["pdf"].write_bytes(invoice.convert("pdf"))
        paths["png"].write_bytes(invoice.render_page(0, dpi=dpi))
    return paths


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--dpi", type=float, default=96)
    args = parser.parse_args()
    create_invoice(args.output_dir, dpi=args.dpi)


if __name__ == "__main__":
    main()
