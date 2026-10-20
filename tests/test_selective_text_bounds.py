import unittest
from unittest.mock import patch

import doc_ex
from doc_ex import _core, _native
from doc_ex.errors import UnsafeArchiveError
from fixtures import pdf_fixture, docx_fixture, pptx_fixture


def counted_pages(count):
    for _ in range(10_002):
        count[0] += 1
        if count[0] > 10_001:
            raise AssertionError('selection consumed beyond its bounded lookahead')
        yield 0


class SelectiveTextBoundsTests(unittest.TestCase):
    def test_public_selection_stops_before_document_processing_for_every_format(self):
        for filetype in ('pdf', 'docx', 'pptx'):
            count = [0]
            with self.subTest(filetype=filetype), patch.object(doc_ex, '_read') as read:
                with self.assertRaises(UnsafeArchiveError):
                    doc_ex.extract_text(b'', filetype=filetype, pages=counted_pages(count))
                read.assert_not_called()
                self.assertEqual(count[0], 10_001)

    def test_native_selection_is_bounded_before_vector_conversion(self):
        count = [0]
        with self.assertRaises(UnsafeArchiveError):
            _native(lambda: _core.extract_text(pdf_fixture(), counted_pages(count)))
        self.assertEqual(count[0], 10_001)

    def test_unrepresentable_indexes_are_stable_index_errors(self):
        for fixture in (pdf_fixture, docx_fixture, pptx_fixture):
            for index in (1 << 100, -(1 << 100)):
                with self.subTest(fixture=fixture.__name__, index=index), self.assertRaises(IndexError):
                    doc_ex.extract_text(fixture(), pages=[index])
        for index in (1 << 100, -(1 << 100)):
            with self.assertRaises(IndexError):
                _native(lambda: _core.extract_text(pdf_fixture(), [index]))

    def test_office_duplicates_obey_utf8_output_budget_and_all_page_limit(self):
        for filetype in ('docx', 'pptx'):
            with self.subTest(filetype=filetype), doc_ex.new(filetype) as document:
                if filetype == 'docx':
                    document.add_paragraph('é')
                else:
                    document.slides.add().add_text('é', box=(10, 10, 100, 30))
                source = document.to_bytes()
            with patch.object(doc_ex, '_MAX_TEXT_OUTPUT_BYTES', 5):
                self.assertEqual(doc_ex.extract_text(source, pages=[0, 0]), 'é\fé')
                with self.assertRaises(UnsafeArchiveError):
                    doc_ex.extract_text(source, pages=[0, 0, 0])
            with patch.object(doc_ex, '_MAX_SELECTED_PAGES', 0):
                with self.assertRaises(UnsafeArchiveError):
                    doc_ex.extract_text(source)

    def test_utf8_chunk_boundaries_and_empty_page_separators_count_toward_output(self):
        with patch.object(doc_ex, '_MAX_TEXT_OUTPUT_BYTES', 8194):
            self.assertEqual(doc_ex._bounded_text_join(['é' * 4096, 'x']), 'é' * 4096 + '\fx')
        with patch.object(doc_ex, '_MAX_TEXT_OUTPUT_BYTES', 8193), self.assertRaises(UnsafeArchiveError):
            doc_ex._bounded_text_join(['é' * 4096, 'x'])
        with patch.object(doc_ex, '_MAX_TEXT_OUTPUT_BYTES', 1):
            self.assertEqual(doc_ex._bounded_text_join(['', '']), '\f')
            with self.assertRaises(UnsafeArchiveError):
                doc_ex._bounded_text_join(['', '', ''])


if __name__ == '__main__':
    unittest.main()
