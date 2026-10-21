"""Static consumer check, not a runtime test.

Run mypy==1.18.2 against an installed package with:
  mypy --strict --follow-imports=silent --python-executable /path/to/python \
      tools/check_public_typing.py
Only imported package diagnostics are suppressed; this consumer is checked strictly.
The private class imports supply expected identities, not extra public exports.
"""
from typing import Any, assert_type

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
        assert_type(next(iter(word.images)), Picture)

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
