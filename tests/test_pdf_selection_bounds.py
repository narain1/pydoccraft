from io import BytesIO
import sys
import unittest
from unittest.mock import patch

import doc_ex
from doc_ex.errors import DocumentClosedError, UnsafeArchiveError


def labeled_document():
    document = doc_ex.new('pdf', size=(200, 200))
    document.pages[0].draw_text('Alpha', at=(20, 30))
    document.pages.add(size=(200, 200)).draw_text('Beta', at=(20, 30))
    return document


class PdfSelectionBoundsTests(unittest.TestCase):
    def test_unbounded_iterables_stop_at_one_lookahead_before_native_work(self):
        class Selection:
            yielded = 0
            converted = 0
            def __iter__(self):
                return self
            def __next__(self):
                self.yielded += 1
                if self.yielded > 10001:
                    raise AssertionError('selection consumed beyond bounded lookahead')
                selection = self
                class Index:
                    def __index__(self):
                        if selection.yielded == 10001:
                            raise AssertionError('excess item must not be normalized')
                        selection.converted += 1
                        return 0
                return Index()
        with labeled_document() as document:
            source = document.to_bytes()
            before, png = document.to_bytes(), document.render_page(0, dpi=72)
            for operation in ('copy', 'insert'):
                selection = Selection()
                with self.subTest(operation=operation), patch.object(document, '_call', wraps=document._call) as calls:
                    with self.assertRaises(UnsafeArchiveError):
                        if operation == 'copy':
                            document.copy(pages=selection)
                        else:
                            document.pages.insert(1, source, pages=selection)
                    self.assertFalse(any(call.args[0] in ('subset', 'import_pages') for call in calls.call_args_list))
                self.assertEqual((selection.yielded, selection.converted), (10001, 10000))
                self.assertEqual(document.to_bytes(), before)
                self.assertEqual(document.render_page(0, dpi=72), png)

    def test_exact_limit_reaches_native_boundary_and_copy_keeps_index_error_precedence(self):
        class NativeBoundaryReached(Exception):
            pass
        with labeled_document() as document:
            source = document.to_bytes()
            original_call = document._call
            for operation in ('copy', 'insert'):
                converted = []
                class Index:
                    def __init__(self, value):
                        self.value = value
                    def __index__(self):
                        converted.append(self.value)
                        return self.value
                selection = (Index(0 if index % 2 == 0 else -1) for index in range(10000))
                def boundary(name, *args, **kwargs):
                    if name in ('subset', 'import_pages'):
                        values = args[0] if name == 'subset' else args[2]
                        expected_pair = [0, 1] if name == 'subset' else [0, -1]
                        self.assertEqual(len(values), 10000)
                        self.assertEqual(values, expected_pair * 5000)
                        # Stop at the real facade/native boundary: allocating and
                        # serializing 10000 duplicated pages adds no cap evidence.
                        raise NativeBoundaryReached()
                    return original_call(name, *args, **kwargs)
                with self.subTest(operation=operation), patch.object(document, '_call', side_effect=boundary):
                    with self.assertRaises(NativeBoundaryReached):
                        if operation == 'copy':
                            document.copy(pages=selection)
                        else:
                            document.pages.insert(1, source, pages=selection)
                self.assertEqual(converted, [0, -1] * 5000)
            document.pages[0].fields.add('active', (20, 70, 120, 30))
            before = document.to_bytes()
            # Invalid copy indexes must keep their Python IndexError, even though
            # native subsets otherwise reject documents containing active fields.
            for index in (99, -99):
                with self.assertRaises(IndexError):
                    document.copy(pages=[index])
            with self.assertRaises(doc_ex.UnsupportedFormatError):
                document.copy(pages=[0])
            self.assertEqual(document.to_bytes(), before)

    def test_bad_items_and_huge_indexes_do_not_consume_tail_or_mutate(self):
        with labeled_document() as document:
            source = document.to_bytes()
            original = document.to_bytes()
            page = document.pages[0]
            for operation in ('copy', 'insert'):
                for bad, error in ((1.5, TypeError), ('1', TypeError), (sys.maxsize + 1, IndexError),
                                   (-sys.maxsize - 2, IndexError), (99, IndexError)):
                    consumed = []
                    def selection():
                        consumed.append(0)
                        yield 0
                        consumed.append(bad)
                        yield bad
                        if bad != 99:
                            raise AssertionError('invalid item must stop consumption')
                    with self.subTest(operation=operation, bad=bad), self.assertRaises(error):
                        if operation == 'copy':
                            document.copy(pages=selection())
                        else:
                            document.pages.insert(1, source, pages=selection())
                    self.assertEqual(consumed, [0, bad])
                    self.assertEqual(document.to_bytes(), original)
                    self.assertIn('Alpha', page.get_text())

    def test_order_duplicates_negative_indexes_none_and_source_snapshot(self):
        class Last:
            def __index__(self):
                return -1
        with labeled_document() as source:
            with source.copy(pages=iter((Last(), 0, Last()))) as copied:
                self.assertEqual([page.get_text().strip() for page in copied.pages], ['Beta', 'Alpha', 'Beta'])
            with source.copy() as copied:
                self.assertEqual(copied.page_count, 2)
            with doc_ex.new('pdf', size=(200, 200)) as destination:
                added = destination.pages.insert(1, source, pages=(value for value in (-1, 0, -1)))
                self.assertEqual([page.get_text().strip() for page in added], ['Beta', 'Alpha', 'Beta'])
                self.assertEqual(destination.page_count, 4)
            with doc_ex.new('pdf', size=(200, 200)) as destination:
                added = destination.pages.insert(1, source)
                self.assertEqual([page.get_text().strip() for page in added], ['Alpha', 'Beta'])
            snapshot = source.to_bytes()
            with doc_ex.new('pdf', size=(200, 200)) as destination:
                def close_source():
                    source.close()
                    yield -1
                added = destination.pages.insert(1, source, pages=close_source())
                self.assertIn('Beta', added[0].get_text())
            with doc_ex.open(snapshot) as opened, doc_ex.new('pdf', size=(200, 200)) as destination:
                def close_opened_source():
                    opened.close()
                    yield 0
                added = destination.pages.insert(1, opened, pages=close_opened_source())
                self.assertIn('Alpha', added[0].get_text())

    def test_callbacks_cannot_retarget_selection_but_nonstructural_edits_are_allowed(self):
        for operation in ('copy', 'insert'):
            for change in ('append', 'move', 'close'):
                with self.subTest(operation=operation, change=change), labeled_document() as document:
                    source = document.to_bytes()
                    changed = []
                    class Index:
                        def __index__(self):
                            if change == 'append':
                                document.pages.add(size=(200, 200))
                            elif change == 'move':
                                document.pages.move(0, 1)
                            else:
                                document.close()
                            if not document.closed:
                                changed.append(document.to_bytes())
                            return -1
                    with patch.object(document, '_call', wraps=document._call) as calls:
                        error = DocumentClosedError if change == 'close' else ValueError
                        with self.assertRaisesRegex(error, 'closed' if change == 'close' else 'stale'):
                            if operation == 'copy':
                                document.copy(pages=[Index()])
                            else:
                                document.pages.insert(1, source, pages=[Index()])
                        self.assertFalse(any(call.args[0] in ('subset', 'import_pages') for call in calls.call_args_list))
                    if not document.closed:
                        self.assertEqual(document.to_bytes(), changed[0])
        for operation in ('copy', 'insert'):
            with self.subTest(nonstructural=operation), labeled_document() as document:
                source = document.to_bytes()
                class Index:
                    def __index__(self):
                        document.update_metadata(title='Changed by callback')
                        return 0
                if operation == 'copy':
                    with document.copy(pages=[Index()]) as copied:
                        self.assertIn('Alpha', copied.pages[0].get_text())
                else:
                    self.assertIn('Alpha', document.pages.insert(1, source, pages=[Index()])[0].get_text())
                self.assertEqual(document.metadata['title'], 'Changed by callback')

    def test_insert_index_and_source_read_callbacks_recheck_target(self):
        for stage in ('index', 'read'):
            for change in ('append', 'close'):
                with self.subTest(stage=stage, change=change), labeled_document() as document:
                    data = document.to_bytes()
                    changed = []
                    def mutate():
                        if change == 'close':
                            document.close()
                            changed.append(None)
                        else:
                            document.pages.add(size=(200, 200))
                            changed.append(document.to_bytes())
                    class Index:
                        def __index__(self):
                            mutate()
                            return 1
                    class Source(BytesIO):
                        def read(self, size=-1):
                            if not changed:
                                mutate()
                            return super().read(size)
                    with patch.object(document, '_call', wraps=document._call) as calls:
                        error = DocumentClosedError if change == 'close' else ValueError
                        with self.assertRaisesRegex(error, 'closed' if change == 'close' else 'stale'):
                            document.pages.insert(Index() if stage == 'index' else 1,
                                                  Source(data) if stage == 'read' else data, pages=[0])
                        self.assertFalse(any(call.args[0] == 'import_pages' for call in calls.call_args_list))
                    if not document.closed:
                        self.assertEqual(document.to_bytes(), changed[0])


if __name__ == '__main__':
    unittest.main()
