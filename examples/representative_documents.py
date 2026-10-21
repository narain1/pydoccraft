"""Build owned resume, review and intake documents with ordinary public APIs.

Each function returns a document. The caller closes it and chooses whether to
save, render, convert or copy it. Importing this module writes no files.
"""
import doc_ex


def _word_document(title):
    document = doc_ex.new("docx", size=(360, 500))
    document.sections[0].margins = 30
    document.background = "#F0F4F8"
    document.styles["Normal"].format(font="Source Sans 3", font_size=11,
                                      space_after=6)
    heading = document.styles.add("Document heading", based_on="Normal")
    heading.format(font_size=18, bold=True, color="navy", space_after=10)
    document.sections[0].header().add_paragraph("EXAMPLE STUDIO")
    document.sections[0].footer().add_paragraph("Prepared for discussion")
    document.add_paragraph(title, style=heading)
    return document


def build_resume():
    """Return an English/French/German resume using the Latin glyph profile."""
    document = _word_document("Camille Müller")
    document.add_paragraph("Document specialist | Lyon, France")
    for text, language in (
        ("Résumé professionnel", "fr-FR"),
        ("Création de rapports et coordination des équipes.", "fr-FR"),
        ("Berufserfahrung", "de-DE"),
        ("Präzise Dokumentation für internationale Teams.", "de-DE"),
    ):
        document.add_paragraph(text)[:].format(language=language)
    table = document.add_table([["Skill", "Experience"],
                                ["Python automation", "Five years"],
                                ["Document quality", "Layout and review"]])
    table.column_widths = [140, 160]
    table.borders = ("navy", 0.5)
    for column in range(2):
        table[0, column].background = "#DDEEFF"
    document.add_paragraph("Selected work").find("Selected work")[0].link = (
        "https://example.com/portfolio")
    return document


def build_review():
    """Return a proposal with whole-paragraph comments and one direct reply.

    Native PDF export retains the proposal but omits comment prose/markup.
    This recipe does not create tracked revisions or arbitrary range anchors.
    """
    document = _word_document("Delivery proposal")
    target = document.add_paragraph("Publish the reviewed package on Friday.")
    document.add_paragraph("The owner will confirm the validation results.")
    table = document.add_table([["Decision", "Owner"],
                                ["Release readiness", "Mira Chen"]])
    table.column_widths = [180, 120]
    table.borders = ("navy", 0.5)
    for column in range(2):
        table[0, column].background = "#DDEEFF"
    comment = document.comments.add(target[:], "Confirm the delivery date.",
                                    author="Ada", date="2026-10-21T09:00:00Z")
    reply = document.comments.add(comment, "The validation owner will confirm.",
                                  author="Ben", date="2026-10-21T09:30:00Z")
    reply.resolved = True
    return document


def build_intake_form():
    """Return an owned PDF with filled text, checkbox and choice fields.

    Flatten a copy for a noninteractive output. Imported-field editing and
    signatures are outside this recipe; help text is stored, not printed.
    """
    document = doc_ex.new("pdf", size=(360, 420))
    page = document.pages[0]
    page.background = "#F0F4F8"
    page.draw_text("Project intake", at=(30, 25), font_size=22, color="navy")
    page.draw_text("Review these details before confirming.", at=(30, 58), font_size=11)
    for label, y in (("Contact", 95), ("Service", 175), ("Approved for review", 273)):
        page.draw_text(label, at=(30 if y != 273 else 65, y), font_size=12)
    contact = page.fields.add("contact", (30, 115, 280, 30))
    contact.value = "Mira Chen"
    contact.tooltip = "Person responsible for this request"
    service = page.fields.add("service", (30, 195, 280, 30), kind="choice",
                              options=["Training", "Document review"])
    service.value = "Document review"
    service.tooltip = "Requested service"
    approved = page.fields.add("approved", (30, 267, 24, 24), kind="checkbox")
    approved.value = True
    approved.tooltip = "Ready for the review team"
    page.draw_text("Service guide", at=(30, 340), font_size=12, color="blue")
    page.annotations.add_link(box=(30, 336, 100, 22), uri="https://example.com/services")
    return document
