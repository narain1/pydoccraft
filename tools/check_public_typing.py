"""Static consumer check, not a runtime test.

Run mypy==1.18.2 against an installed package with:
  mypy --strict --follow-imports=silent --python-executable /path/to/python \
      tools/check_public_typing.py
Only imported package diagnostics are suppressed; this consumer is checked strictly.
The private class imports supply expected identities, not extra public exports.
"""
from fractions import Fraction
from io import BytesIO
from pathlib import Path
from numbers import Real
from typing import Any, Literal, assert_type

import doc_ex
from doc_ex._authoring import (
    Annotation, Bookmark, BorderFormatting, Comment, Comments, EditableCell, EditableTable, Paragraph, ParagraphFormatting, PdfDocument, PdfField,
    PdfPage, Picture, Presentation, Shape, SlidePicture, SlideTable, TabStop, TextRange,
    WordDocument, _WordShape, _SlideCellParagraph,
)


def verify(document_bytes: bytes, png: bytes, mode: str) -> None:
    with doc_ex.new('docx') as word:
        assert_type(word, WordDocument)
        shape = word.add_shape("rectangle", width=40, height=20, fill="red", stroke="blue", stroke_width=2)
        assert_type(shape, _WordShape)
        assert_type(shape.kind, Literal["rectangle", "ellipse"])
        assert_type(shape.size, tuple[float, float])
        assert_type(shape.fill, str | None)
        assert_type(shape.stroke, tuple[str, float] | None)
        assert_type(word.shapes[:], list[_WordShape])
        assert_type(next(iter(word.shapes)), _WordShape)
        shape.size = (80, 40)
        shape.fill = (255, 0, 0)
        shape.stroke = ("blue", 2)
        assert_type(shape.format(wrap="behind", anchor="page", position=(80, 100)), None)
        assert_type(shape.formatting["position"], tuple[float, float])
        del word.shapes[0]

        assert_type(word.copy(), WordDocument)
        word.update_metadata(language="en-GB")
        assert_type(word.metadata["language"], str)
        assert_type(word.mirror_margins, bool)
        assert_type(word.sections[0].vertical_alignment, Literal["top", "center", "bottom", "justify"])
        word.sections[0].vertical_alignment = "center"
        word.mirror_margins = True
        word.mirror_margins = False
        assert_type(word.notes.convert(kind="endnote"), int)
        paragraph = word.add_paragraph('Example')
        assert_type(paragraph, Paragraph)
        word.bookmarks['example'] = paragraph[:]
        assert_type(word.bookmarks['example'], TextRange)
        assert_type(next(iter(word.bookmarks)), str)
        del word.bookmarks['example']
        assert_type(paragraph.add_run(' extra', bold=True, language='en-GB'), TextRange)
        assert_type(paragraph.format(direction='rtl'), None)
        assert_type(paragraph[:].format(language=None), None)
        assert_type(paragraph.add_run(" spaced", letter_spacing=0.3), TextRange)
        assert_type(paragraph[:].format(letter_spacing=None), None)
        assert_type(word.styles.format_defaults(letter_spacing=0), None)
        assert_type(word.styles.format_defaults(font_size=None), None)
        comment_paragraph = word.add_paragraph("Comment target")
        assert_type(word.comments, Comments)
        comment = word.comments.add(comment_paragraph[:], "Review", author="Ada")
        assert_type(comment, Comment)
        assert_type(word.comments.add(comment, "Reply"), Comment)
        assert_type(comment.range, TextRange)
        assert_type(comment.parent, Comment | None)
        assert_type(comment.text, str)
        assert_type(comment.author, str)
        assert_type(comment.date, str)
        assert_type(comment.resolved, bool)
        assert_type(word.comments[:], list[Comment])
        assert_type(next(iter(word.comments)), Comment)
        note = word.notes.add(paragraph[:], 'Detail')
        note.add_paragraph('Remove')
        del note.paragraphs[1]
        assert_type(note.clear(), None)
        assert_type(word.sections[0].header().clear(), None)
        assert_type(word.sections[0].footer('even').clear(), None)
        assert_type(word.add_page_break(), Paragraph)
        assert_type(word.add_image(png, width=40), Picture)
        picture = word.add_image(png, width=40)
        assert_type(picture.format(wrap="behind", anchor="page", position=(20, 30), distance=2), None)
        assert_type(picture.formatting["wrap"], Literal["inline", "square", "top_bottom", "behind"])
        assert_type(picture.formatting["anchor"], Literal["page", "margin", "paragraph"])
        assert_type(picture.formatting["position"], tuple[float, float])
        assert_type(picture.formatting["distance"], tuple[float, float, float, float])
        table = word.add_table([['Heading'], ['Value']])
        assert_type(table, EditableTable)
        assert_type(table[0, 0], EditableCell)
        assert_type(table[:, 0], list[EditableCell])
        assert_type(table[0, :], list[EditableCell])
        assert_type(table[:, :], list[list[EditableCell]])
        assert_type(table.merge((0, 0), (1, 0)), EditableCell)
        assert_type(table[0, 0].add_table(1, columns=1), EditableTable)
        assert_type(table[0, 0].add_image(png, width=20), Picture)
        disposable = word.add_table([['Delete']])
        assert_type(disposable, EditableTable)
        del word.tables[-1]
        assert_type(word.tables.move(0, -1), None)
        assert_type(word.sections[0].header().tables.move(0, 0), None)
        assert_type(word.sections[0].footer().tables.move(0, 0), None)
        assert_type(table[0, 0].tables.move(0, -1), None)
        header = word.sections[0].header()
        header.add_table([['Delete']])
        del header.tables[-1]
        cell = word.add_table([['Parent']])[0, 0]
        cell.add_table([['Delete']])
        del cell.tables[-1]
        assert_type(word.paragraphs[:], list[Paragraph])
        assert_type(word.paragraphs.move(0, -1), None)
        assert_type(table[0, 0].paragraphs.move(0, -1), None)
        assert_type(word.sections[0].header().paragraphs.move(0, 0), None)
        assert_type(note.paragraphs.move(0, 0), None)
        assert_type(next(iter(word.images)), Picture)

    with doc_ex.new('pptx') as deck:
        cell = deck.slides.add().add_table([['Remove']], box=(20, 20, 200, 80))[0, 0]
        assert_type(cell.paragraphs[0], Paragraph)
        assert_type(cell.paragraphs.move(0, -1), None)
        del cell.paragraphs[0]

    with doc_ex.new('pdf') as pdf:
        assert_type(pdf, PdfDocument)
        assert_type(pdf.to_bytes(object_streams=False), bytes)
        assert_type(pdf.save('output.pdf', object_streams=True), None)
        assert_type(pdf.copy(pages=[0]), PdfDocument)
        page = pdf.pages.add(size=(200, 300))
        assert_type(page, PdfPage)
        assert_type(pdf.pages.insert(0, document_bytes, pages=[0]), list[PdfPage])
        assert_type(pdf.pages.move(0, 1), None)
        bookmark = pdf.bookmarks.add('Title', 0)
        assert_type(bookmark, Bookmark)
        assert_type(pdf.bookmarks.move(0, 0), Bookmark)
        annotation = page.annotations.add('note', box=(10, 10, 20, 20))
        assert_type(annotation, Annotation)
        assert_type(annotation.opacity, float)
        annotation.opacity = .5
        annotation.update(opacity=.25, color="blue")
        assert_type(annotation.locked, bool)
        annotation.locked = True
        assert_type(annotation.locked, bool)
        field = page.fields.add('customer', (10, 60, 160, 30))
        assert_type(field, PdfField)
        assert_type(field.tooltip, str)
        assert_type(field.read_only, bool)
        field.read_only = True
        assert_type(field.read_only, bool)
        assert_type(field.boxes, tuple[tuple[float, float, float, float], ...])
        radio = page.fields.add('delivery', kind='radio', options=('A', 'B'),
                                boxes=((10, 110, 24, 24), (50, 110, 24, 24)))
        assert_type(radio, PdfField)
        assert_type(radio.boxes, tuple[tuple[float, float, float, float], ...])
        radio.value = 'A'
        field = page.fields['customer']
        field.tooltip = 'Customer identifier'
        assert_type(field.tooltip, str)
        assert_type(page.annotations.add_link(box=[10, 10, 20, 20], uri='https://example.com'), Annotation)
        assert_type(page.search_for('Example'), list[tuple[float, float, float, float]])
        assert_type(page.rotate(90), None)
        assert_type(pdf.update_metadata(title='Draft', author=None), None)
        assert_type(pdf.update_metadata(**pdf.metadata), None)
        assert_type(page.draw_text('Underlay', at=(10, 10), behind=True), None)
        assert_type(page.draw_text('Box underlay', box=[10, 30, 100, 30], color=[20, 30, 40], behind=True), None)
        assert_type(page.add_image(png, box=(10, 70, 40, 40), crop=[0, 0, 0, 0], behind=True), None)
        assert_type(page.get_text(), str)
        assert_type(page.get_text('dict'), dict[str, Any])
        assert_type(page.get_text(mode), str | dict[str, Any])

    with doc_ex.new('pptx') as presentation:
        assert_type(presentation, Presentation)
        assert_type(presentation.copy(), Presentation)
        slide = presentation.slides.add()
        assert_type(slide.background, str | None)
        slide.background = None
        slide.background = "white"
        assert_type(slide.background, str)  # Normal narrowing after a string assignment.
        assert_type(slide.add_text('Example', box=(10, 10, 100, 20)), Shape)
        assert_type(slide.add_shape('rectangle', box=(10, 40, 100, 20)), Shape)
        assert_type(slide.add_table(2, columns=2, box=(10, 70, 100, 60)), SlideTable)
        assert_type(slide.add_image(png, box=(10, 150, 40, 40)), SlidePicture)
        cell_paragraph = slide.add_table([['Cell']], box=(10, 210, 200, 80))[0, 0].paragraphs[0]
        assert_type(cell_paragraph, Paragraph)
        assert_type(cell_paragraph.add_run(' rich', bold=True), TextRange)
        assert_type(cell_paragraph.add_break(), TextRange)
        assert_type(cell_paragraph.add_run(' spaced', letter_spacing=0.3), TextRange)
        assert_type(cell_paragraph[:].format(letter_spacing=None), None)
        assert_type(cell_paragraph.format(indent_left=12, indent_right=None, indent_first=-6,
                                         space_before=4, space_after=8, line_spacing=1.5), None)
        assert_type(cell_paragraph.format(tabs=[{"position": 72, "alignment": "decimal"}]), None)
        assert_type(cell_paragraph.format(tabs=[]), None)
        assert_type(cell_paragraph.format(tabs=None), None)
        assert_type(cell_paragraph.formatting["line_spacing"], float | None)
        cell_snapshot = cell_paragraph.formatting
        assert_type(cell_paragraph.format(**cell_snapshot), None)
        cell_patch: ParagraphFormatting = {
            "alignment": cell_snapshot["alignment"],
            "indent_left": cell_snapshot["indent_left"],
            "indent_right": cell_snapshot["indent_right"],
            "indent_first": cell_snapshot["indent_first"],
            "space_before": cell_snapshot["space_before"],
            "space_after": cell_snapshot["space_after"],
            "line_spacing": cell_snapshot["line_spacing"],
        }
        assert_type(cell_paragraph.format(**cell_patch), None)
        assert_type(slide.add_text('Spaced', box=(10, 300, 200, 40))[:].format(letter_spacing=0), None)
        assert_type(slide.get_text('markdown'), str)
        assert_type(slide.get_text('dict'), dict[str, Any])

    with doc_ex.open(document_bytes, filetype='pdf') as extracted:
        assert_type(extracted, doc_ex.Document)
        assert_type(extracted.edit(), PdfDocument)
        assert_type(extracted[0].search_for('Example'), list[tuple[float, float, float, float]])
        assert_type(extracted[0].get_text(), str)
        assert_type(extracted[0].get_text('text'), str)
        assert_type(extracted[0].get_text('json'), str)
        assert_type(extracted[0].get_text('dict'), dict[str, Any])
        assert_type(extracted[0].get_text(mode), str | dict[str, Any])


def verify_properties(word: WordDocument, pdf: PdfDocument, presentation: Presentation, png: bytes) -> None:
    """Ordinary properties retain their native nullable and collection types."""
    picture = word.copy().images[0]
    assert_type(picture.size, tuple[float, float])
    assert_type(picture.alt, str)
    assert_type(picture.data, bytes)
    assert_type(picture.crop, tuple[float, float, float, float])
    assert_type(picture.flip_h, bool)
    picture.size = [40, 30]
    picture.alt = "Description"
    picture.data = png
    picture.crop = None
    assert_type(word.tables[0][0, 0].background, str | None)
    assert_type(word.tables[0][0, 0].vertical_alignment, str | None)

    page = pdf.copy().pages[0]
    assert_type(pdf.page_count, int)
    assert_type(page.number, int)
    assert_type(page.size, tuple[float, float])
    assert_type(page.background, str | None)
    assert_type(page.fields['customer'].page.size, tuple[float, float])
    assert_type(next(iter(page.fields)).value, str | bool | None)
    assert_type(page.fields['customer'].options, tuple[str, ...])
    page.fields['customer'].box = [10, 10, 100, 20]
    page.fields['customer'].value = "Example"
    bookmark = pdf.bookmarks[0]
    assert_type(bookmark.children[0].title, str)
    assert_type(bookmark.page, int | None)
    assert_type(bookmark.expanded, bool)
    annotation = page.annotations[0]
    assert_type(annotation.kind, str)
    assert_type(annotation.box, tuple[float, float, float, float])
    assert_type(annotation.text, str)
    assert_type(annotation.author, str)
    assert_type(annotation.color, str | None)
    assert_type(annotation.uri, str | None)
    assert_type(annotation.target_page, int | None)
    assert_type(annotation.opacity, float)
    assert_type(annotation.update(color="blue", opacity=.5), None)

    assert_type(presentation.page_count, int)
    slide = presentation.copy().slides[0]
    assert_type(slide.number, int)
    assert_type(slide.size, tuple[float, float])
    assert_type(slide.name, str)
    assert_type(slide.hidden, bool)
    assert_type(slide.notes, str)
    assert_type(slide.background_image, bytes | None)
    shape = slide.shapes[0]
    assert_type(shape.rotation, float)
    assert_type(shape.fill, str | None)
    assert_type(shape.stroke, tuple[str, float] | None)
    assert_type(shape.box, tuple[float, float, float, float])
    assert_type(shape.text, str)
    assert_type(shape.color, str | None)
    assert_type(shape.font_size, float | None)
    assert_type(shape.bold, bool | None)
    assert_type(shape.italic, bool | None)
    assert_type(shape.font, str | None)
    shape.box = [10, 20, 100, 40]
    shape.fill = None
    shape.rotation = 90
    slide_picture = slide.add_image(png, box=(10, 100, 40, 30))
    assert_type(slide_picture.box, tuple[float, float, float, float])
    assert_type(slide_picture.rotation, float)
    assert_type(slide_picture.data, bytes)
    assert_type(slide_picture.alt, str)
    slide_picture.data = png
    slide_picture.crop = [0, .1, 0, .1]


def verify_snapshots(word: WordDocument, pdf: PdfDocument, picture: Picture,
                     table: EditableTable, cell: EditableCell) -> None:
    assert_type(picture.format(**picture.formatting), None)
    assert_type(table.format(**table.formatting), None)
    assert_type(cell.format(**cell.formatting), None)
    # Metadata keys are absent when unset; custom is always a non-null mapping.
    assert_type(word.metadata["title"], str)
    assert_type(word.metadata.get("title"), str | None)
    assert_type(word.metadata.get("revision"), int | None)
    assert_type(word.metadata["custom"], dict[str, str | bool | int | float])
    assert_type(word.metadata["custom"]["value"], str | bool | int | float)
    assert_type(pdf.metadata["author"], str)
    assert_type(pdf.metadata.get("subject"), str | None)
    assert_type(picture.formatting["position"], tuple[float, float])
    assert_type(picture.formatting["distance"], tuple[float, float, float, float])
    assert_type(table.formatting["alignment"], str | None)
    assert_type(table.formatting["indent"], float | None)
    assert_type(table.formatting["width"], float | None)
    assert_type(table.formatting["width_percent"], float | None)
    assert_type(table.formatting["layout"], Literal["fixed", "autofit"])
    assert_type(table.formatting["rtl"], bool)
    assert_type(table.formatting["title"], str | None)
    assert_type(table.formatting["description"], str | None)
    assert_type(table.formatting["style"], str | None)
    assert_type(table.formatting["look"]["header_row"], bool)
    borders = table.formatting["borders"]
    if borders is not None:
        line = borders["top"]
        if line is not None:
            assert_type(line["style"], str)
            assert_type(line["color"], str | None)
            assert_type(line["width"], float)
    assert_type(cell.formatting["background"], str | None)
    assert_type(cell.formatting["margins"], list[float] | None)
    assert_type(cell.formatting["alignment"], str | None)
    assert_type(cell.formatting["vertical_alignment"], str | None)
    assert_type(cell.formatting["text_direction"], str | None)
    assert_type(cell.formatting["no_wrap"], bool)
    cell_borders = cell.formatting["borders"]
    if cell_borders is not None:
        bottom = cell_borders["bottom"]
        if bottom is not None:
            assert_type(bottom["color"], str | None)
            assert_type(bottom["width"], float)


def verify_formatting_snapshots(word: WordDocument, paragraph: Paragraph) -> None:
    defaults = word.styles.defaults
    assert_type(defaults["font_size"], float | None)
    assert_type(defaults["language"], str | None)
    assert_type(defaults["strike"], bool | str | None)
    assert_type(defaults["direction"], str | None)
    direct = paragraph.formatting
    assert_type(direct["alignment"], str | None)
    assert_type(direct.get("direction"), Literal["ltr", "rtl"] | None)
    paragraph.format(**direct)
    # A Paragraph can be a PPTX cell paragraph with alignment alone.
    assert_type(direct.get("indent_left"), float | None)
    tabs = direct.get("tabs")
    if tabs is not None:
        assert_type(tabs[0]["position"], float)
        assert_type(tabs[0]["alignment"], str)
        assert_type(tabs[0]["leader"], str)
    resolved = paragraph.resolved_formatting  # Successful results are DOCX only.
    assert_type(resolved["alignment"], str)
    assert_type(resolved["direction"], Literal["ltr", "rtl"])
    paragraph.format(**resolved)
    # Existing patch dictionaries keep contextual typing alongside full replay.
    border: BorderFormatting = {"width": 1, "color": (10, 20, 30)}
    input_borders: dict[str, BorderFormatting | None] = {"top": border, "bottom": None}
    stop: TabStop = {"position": 24}
    paragraph.format(borders={"top": border}, tabs=[stop])
    paragraph.format(borders=input_borders)
    paragraph.format(borders={"top": {"width": 1, "color": (10, 20, 30)}})
    paragraph.format(tabs=[{"position": 24}, {"position": 48, "leader": "dot"}])
    paragraph.format(direction=None, borders=None, tabs=None)
    assert_type(resolved["indent_left"], float)
    assert_type(resolved["keep_with_next"], bool)
    assert_type(resolved["tabs"][0]["position"], float)
    assert_type(resolved["background"], str | None)
    for kind in ("character", "paragraph", "table"):
        style = word.styles.add("Typed " + kind, kind=kind)
        for snapshot in (style.formatting, style.resolved_formatting):
            assert_type(snapshot["font"], str | None)
            assert_type(snapshot["language"], str | None)
            assert_type(snapshot["strike"], bool | str | None)
            assert_type(snapshot.get("alignment"), str | None)
            margins = snapshot.get("margins")
            if margins is not None:
                assert_type(margins, list[float])
            regions = snapshot.get("regions")
            if regions is not None:
                region = regions["first_row"]
                assert_type(region["vertical_alignment"], str | None)
                assert_type(region["bold"], bool | None)
                borders = region["borders"]
                if borders is not None:
                    edge = borders["top"]
                    if edge is not None:
                        assert_type(edge["width"], float)
                        assert_type(edge["color"], str | None)


def verify_layout_values(word: WordDocument, deck: Presentation) -> None:
    """Existing native values and accepted tuple/list/iterable assignment forms."""
    paragraph = word.add_paragraph("Layout")
    assert_type(paragraph.page_break_before, bool)
    assert_type(paragraph.spacing, tuple[float, float, float])
    paragraph.text = "Updated"
    paragraph.style = word.styles["Normal"]
    paragraph.alignment = "left"
    paragraph.page_break_before = False
    paragraph.spacing = (1, 2, 1.2)
    paragraph.spacing = [1, 2, 1.2]
    table = word.add_table([["A", "B"]])
    assert_type(table.column_widths, list[float])
    assert_type(table.borders, tuple[str, float] | None)
    table.column_widths = (width for width in (100, 120))
    table.column_widths = [100, 120]
    table.borders = ("navy", 1)
    table.borders = [(10, 20, 30), 1]
    table.borders = None
    row = table.row(0)
    assert_type(row.height, tuple[float | None, str])
    assert_type(row.repeat_header, bool)
    assert_type(row.allow_split, bool)
    row.height = (20, "at_least")
    row.height = [20, "at_least"]
    row.height = (None, "auto")
    row.height = None
    row.repeat_header = True
    row.allow_split = False
    slide_table = deck.slides.add().add_table([["Slide"]], box=(10, 10, 100, 40))
    assert_type(slide_table.column_widths, list[float])
    assert_type(slide_table.borders, tuple[str, float] | None)
    assert_type(slide_table.row(0).height, tuple[float | None, str])
    slide_table.row(0).height = [30, "at_least"]


def verify_unit_converters(real: Real) -> None:
    for converter in (doc_ex.pt, doc_ex.mm, doc_ex.cm, doc_ex.inches):
        assert_type(converter(1), float)
        assert_type(converter(1.5), float)
        assert_type(converter(Fraction(1, 2)), float)
        assert_type(converter(real), float)


class _PropertyIndex:
    def __index__(self) -> int:
        return 2

def verify_property_setters(word: WordDocument, deck: Presentation, png: bytes) -> None:
    word.background = (255, 0, 0)
    word.background = [0, 0, 255]
    word.background = None
    word.background_image = Path('background.png')
    word.background_image = BytesIO(png)
    word.background_image = None
    section = word.sections[0]
    section.page_size = 'A4'
    section.page_size = [400, 600]
    section.margins = Fraction(12, 1)
    section.margins = (10, 20, 30, 40)
    section.margins = [10, 20, 30, 40]
    section.gutter = Fraction(5, 1)
    section.header_distance = 12
    section.footer_distance = 12.5
    section.columns = (_PropertyIndex(), 12, True)
    section.columns = [2, 12, True]
    section.column_widths = (100, 200)
    section.column_widths = [100, 200]
    section.column_widths = None
    cell = word.add_table([['value']])[0, 0]
    cell.margins = Fraction(3, 1)
    cell.margins = [1, 2, 3, 4]
    cell.margins = None
    cell.background = 'navy'
    cell.background = [0, 0, 255]
    slide = deck.slides[0]
    slide.background_image = png
    shape = slide.add_shape('rectangle', box=(10, 10, 50, 50))
    shape.stroke = ('blue', 2)
    shape.stroke = [(0, 0, 255), 2]
    shape.stroke = None


def verify_slide_cell_alignment_reset(paragraph: _SlideCellParagraph) -> None:
    paragraph.alignment = None
    paragraph.alignment = "center"
