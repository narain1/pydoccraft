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


if __name__ == "__main__":
    directory = Path("letters")
    directory.mkdir(exist_ok=True)
    with make_template() as template:
        for index, values in enumerate([
            {"customer": "Acme", "company": "Example Ltd"},
            {"customer": "Mira", "company": "Example Ltd"},
        ], start=1):
            # Failures are caller-owned; no partial document is returned by fill_letter.
            with fill_letter(template, values) as letter:
                letter.save(directory / f"letter-{index}.docx")
                (directory / f"letter-{index}.pdf").write_bytes(letter.convert("pdf"))
