"""Literal replacement in an authored template; no external template parser."""
from pathlib import Path

import doc_ex


def make_template():
    template = doc_ex.new("docx")
    template.sections[0].header().add_paragraph("Prepared by {{company}}")
    paragraph = template.add_paragraph("Dear ")
    paragraph.add_run("{{cus", bold=True)
    paragraph.add_run("tomer}}", italic=True)
    paragraph.add_run(",")
    terms = template.add_paragraph("Thank you for your business. ").add_run("Terms", italic=True)
    terms.link = "https://example.com/terms"
    return template


def fill_letter(template, values):
    """This recipe intentionally knows exactly two fields and two story scopes."""
    required = {"customer", "company"}
    missing, unused = required - values.keys(), values.keys() - required
    if missing or unused:
        raise ValueError(f"missing values: {sorted(missing)}; unused values: {sorted(unused)}")
    if any(not isinstance(value, str) for value in values.values()):
        raise TypeError("format all values as strings before replacement")
    letter = template.copy()
    try:
        # Explicit scopes: the body and section zero's default header.
        paragraphs = [*letter.paragraphs, *letter.sections[0].header().paragraphs]
        counts = {key: sum(p.text.count("{{" + key + "}}") for p in paragraphs) for key in required}
        if any(count == 0 for count in counts.values()):
            raise ValueError("the template is missing a required literal placeholder")
        for key, value in values.items():
            replaced = sum(p.replace_text("{{" + key + "}}", value) for p in paragraphs)
            if replaced != counts[key]:
                raise ValueError("literal replacement count changed unexpectedly")
        return letter
    except BaseException:
        letter.close()
        raise


def write_letters(output_dir: str | Path, *, dpi: float = 96) -> list[dict[str, Path]]:
    """Write two independent example letters as DOCX/PDF/PNG; return path maps.

    This template is authored in memory. The recipe does not open arbitrary
    existing DOCX files for editing or bind content controls.
    """
    directory = Path(output_dir)
    directory.mkdir(parents=True, exist_ok=True)
    outputs = []
    with make_template() as template:
        for index, values in enumerate([
            {"customer": "Acme", "company": "Example Ltd"},
            {"customer": "Mira", "company": "Example Ltd"},
        ], start=1):
            paths = {kind: directory / f"letter-{index}.{kind}" for kind in ("docx", "pdf", "png")}
            # Failures are caller-owned; no partial document is returned by fill_letter.
            with fill_letter(template, values) as letter:
                letter.save(paths["docx"])
                paths["pdf"].write_bytes(letter.convert("pdf"))
                paths["png"].write_bytes(letter.render_page(0, dpi=dpi))
            outputs.append(paths)
    return outputs


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output_dir", type=Path)
    parser.add_argument("--dpi", type=float, default=96)
    args = parser.parse_args()
    write_letters(args.output_dir, dpi=args.dpi)


if __name__ == "__main__":
    main()
