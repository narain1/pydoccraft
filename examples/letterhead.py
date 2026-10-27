"""Compose an owned letterhead with section-first inline PNG logos.

The caller owns the returned document and chooses save/preview destinations.
No images, fonts or other assets are downloaded. Supply two 2:1 PNG logos for
this example's 36-by-18-point frame.
"""
import doc_ex


def build_letterhead(logo_png: bytes, appendix_logo_png: bytes):
    """Return a document whose headers follow sections as body content reflows."""
    document = doc_ex.new("docx", size=(300, 360))
    try:
        document.styles.format_defaults(font="Source Sans 3", font_size=12, space_after=0, line_spacing=1)
        first = document.sections[0]
        first.page_number_start = 2
        first.margins = (64, 24, 28, 24)
        first.header_distance = 12
        first.footer_distance = 10
        for variant, label, color in (("first", "first", "#008000"),
                                      ("even", "even", "#800080"),
                                      ("default", "odd", "#008080")):
            header = first.header(variant)
            if variant == "first":
                header.add_image(logo_png, width=36, height=18, alt="Main section logo")
            header.add_paragraph("Main " + label)[:].format(color=color)
            footer = first.footer(variant).add_paragraph("Main " + label + " footer ")
            footer.add_field("page")
        document.add_paragraph("\n".join(f"Main item {i:03}" for i in range(24)))
        document.add_paragraph("Project guide").find("Project guide")[0].link = "https://example.com/letterhead"
        document.add_table([["Item label", "Value label"], ["Table alpha", "Table beta"]])
        document.add_paragraph("End of main section")

        # New sections copy all variants into independent native parts.
        second = document.sections.add()
        second.page_number_start = 20
        for variant, label, color in (("first", "first", "#008000"),
                                      ("even", "even", "#800080"),
                                      ("default", "odd", "#008080")):
            second.header(variant).paragraphs[0].text = "Appendix " + label
            second.header(variant).paragraphs[0][:].format(color=color)
            footer = second.footer(variant).paragraphs[0]
            footer.text = "Appendix " + label + " footer "
            footer.add_field("page")
        picture = second.header("first").images[0]
        picture.data = appendix_logo_png
        picture.alt = "Appendix section logo"
        document.add_paragraph("\n".join(f"Appendix item {i:03}" for i in range(54)))
        return document
    except Exception:
        document.close()
        raise
