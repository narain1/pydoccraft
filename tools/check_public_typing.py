"""Static consumer check, not a runtime test.

Run mypy==1.18.2 against an installed package with:
  mypy --strict --follow-imports=silent --python-executable /path/to/python \
      tools/check_public_typing.py
Only imported package diagnostics are suppressed; this consumer is checked strictly.
The private class imports supply expected identities, not extra public exports.
"""
from typing import Any, Literal, assert_type

import doc_ex
from doc_ex._authoring import (
    Annotation, Bookmark, EditableCell, EditableTable, Paragraph, PdfDocument, PdfField,
    PdfPage, Picture, Presentation, Shape, SlidePicture, SlideTable, TextRange,
    WordDocument,
)


def verify(document_bytes: bytes, png: bytes, mode: str) -> None:
    with doc_ex.new('docx') as word:
        assert_type(word, WordDocument)
        assert_type(word.copy(), WordDocument)
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
        assert_type(word.paragraphs[:], list[Paragraph])
        assert_type(word.paragraphs.move(0, -1), None)
        assert_type(table[0, 0].paragraphs.move(0, -1), None)
        assert_type(word.sections[0].header().paragraphs.move(0, 0), None)
        assert_type(note.paragraphs.move(0, 0), None)
        assert_type(next(iter(word.images)), Picture)

    with doc_ex.new('pptx') as deck:
        cell = deck.slides.add().add_table([['Remove']], box=(20, 20, 200, 80))[0, 0]
        assert_type(cell.paragraphs[0], Paragraph)
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
        assert_type(cell_paragraph.add_run(' spaced', letter_spacing=0.3), TextRange)
        assert_type(cell_paragraph[:].format(letter_spacing=None), None)
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
