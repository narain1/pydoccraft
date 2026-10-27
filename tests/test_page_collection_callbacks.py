"""Index callbacks cannot make page/slide operations target a different item."""
import unittest
from unittest.mock import patch

import doc_ex
from doc_ex.errors import DocumentClosedError


class Index:
    def __init__(self, callback=lambda: None, value=0):
        self.callback, self.value = callback, value

    def __index__(self):
        self.callback()
        return self.value


def document(kind):
    result = doc_ex.new(kind, size=(200, 200))
    if kind == 'pdf':
        result.pages.add(size=(250, 250))
        result.pages.add(size=(300, 300))
        for page, name in zip(result.pages, ('Alpha', 'Beta', 'Gamma')):
            page.draw_text(name, at=(20, 30))
    else:
        for name in ('Alpha', 'Beta', 'Gamma'):
            result.slides.add().name = name
    return result


def collection(owner):
    return owner.pages if owner.filetype == 'pdf' else owner.slides


def names(owner):
    if owner.filetype == 'pdf':
        return [page.get_text().strip() for page in owner.pages]
    return [slide.name for slide in owner.slides]


class PageCollectionCallbackTests(unittest.TestCase):
    def test_structural_callbacks_cannot_retarget_delete_or_either_move_index(self):
        for kind in ('pdf', 'pptx'):
            for operation in ('delete', 'move_source', 'move_destination'):
                for change in ('append', 'delete', 'move', 'close') + (('rotate',) if kind == 'pdf' else ()):
                    with self.subTest(kind=kind, operation=operation, change=change), document(kind) as owner:
                        selected = collection(owner)
                        after = []
                        native_calls = []
                        operations = {'delete_page', 'move_page', 'remove_slide', 'move_slide'}
                        with patch.object(owner, '_call', wraps=owner._call) as calls:
                            def mutate():
                                if change == 'append':
                                    selected.add()
                                elif change == 'delete':
                                    del selected[1]
                                elif change == 'move':
                                    selected.move(0, 1)
                                elif change == 'rotate':
                                    selected[0].rotate(90)
                                else:
                                    owner.close()
                                if not owner.closed:
                                    after.append(owner.to_bytes())
                                native_calls.extend(call for call in calls.call_args_list if call.args[0] in operations)
                            index = Index(mutate, -1 if operation == 'move_destination' else 0)
                            error = DocumentClosedError if change == 'close' else ValueError
                            with self.assertRaisesRegex(error, 'closed' if change == 'close' else 'stale'):
                                if operation == 'delete':
                                    del selected[index]
                                elif operation == 'move_source':
                                    selected.move(index, -1)
                                else:
                                    selected.move(0, index)
                            self.assertEqual([call for call in calls.call_args_list if call.args[0] in operations], native_calls)
                        if not owner.closed:
                            self.assertEqual(owner.to_bytes(), after[0])

    def test_pdf_lookup_checks_callbacks_before_constructing_current_handles(self):
        for lookup in ('index', 'slice_start', 'slice_stop', 'slice_step'):
            for change in ('append', 'delete', 'move', 'close', 'rotate'):
                with self.subTest(lookup=lookup, change=change), document('pdf') as owner:
                    after = []
                    def mutate():
                        if change == 'append':
                            owner.pages.add()
                        elif change == 'delete':
                            del owner.pages[1]
                        elif change == 'move':
                            owner.pages.move(0, 1)
                        elif change == 'rotate':
                            owner.pages[0].rotate(90)
                        else:
                            owner.close()
                        if not owner.closed:
                            after.append(owner.to_bytes())
                    index = Index(mutate, 1)
                    key = {'index': index, 'slice_start': slice(index, None),
                           'slice_stop': slice(None, index), 'slice_step': slice(None, None, index)}[lookup]
                    error = DocumentClosedError if change == 'close' else ValueError
                    with self.assertRaisesRegex(error, 'closed' if change == 'close' else 'stale'):
                        owner.pages[key]
                    if not owner.closed:
                        self.assertEqual(owner.to_bytes(), after[0])
        # Slide lookups already capture stable native IDs; reordering retains identity.
        with document('pptx') as owner:
            first = owner.slides[Index(lambda: owner.slides.move(0, 1))]
            self.assertEqual(first.name, 'Alpha')
            self.assertEqual(first.number, 1)

    def test_nonstructural_callbacks_negative_indexes_and_saved_output_remain_valid(self):
        for kind in ('pdf', 'pptx'):
            with self.subTest(kind=kind), document(kind) as owner:
                selected = collection(owner)
                def edit():
                    if kind == 'pdf':
                        owner.update_metadata(title='Callback metadata')
                    else:
                        owner.slides[1].hidden = True
                selected.move(Index(edit, -1), Index(edit, 0))
                self.assertEqual(names(owner), ['Gamma', 'Alpha', 'Beta'])
                del selected[Index(edit, -1)]
                self.assertEqual(names(owner), ['Gamma', 'Alpha'])
                if kind == 'pdf':
                    self.assertEqual(selected[Index(edit, -1)].get_text().strip(), 'Alpha')
                    self.assertEqual([p.get_text().strip() for p in selected[slice(None, None, Index(edit, -1))]], ['Alpha', 'Gamma'])
                    self.assertEqual(owner.metadata['title'], 'Callback metadata')
                with doc_ex.open(owner.to_bytes()) as reopened:
                    self.assertEqual(len(reopened), 2)
                    if kind == 'pdf':
                        self.assertEqual([p.get_text().strip() for p in reopened], ['Gamma', 'Alpha'])
                    self.assertTrue(reopened.render_page(0, dpi=72).startswith(b'\x89PNG'))

    def test_closed_bounds_and_callback_errors_do_not_dispatch_outer_mutations(self):
        for kind in ('pdf', 'pptx'):
            with self.subTest(kind=kind), document(kind) as owner:
                selected = collection(owner)
                before = owner.to_bytes()
                marker = RuntimeError('index callback failed')
                def fail():
                    raise marker
                with self.assertRaises(RuntimeError) as caught:
                    del selected[Index(fail)]
                self.assertIs(caught.exception, marker)
                touched = []
                with self.assertRaises(IndexError):
                    selected.move(99, Index(lambda: touched.append(True)))
                self.assertEqual(touched, [])
                for bad in (slice(None), 0.5):
                    with self.assertRaises(TypeError):
                        del selected[bad]
                self.assertEqual(owner.to_bytes(), before)
                owner.close()
                for action in (lambda: selected.__delitem__(Index(lambda: touched.append(True))),
                               lambda: selected.move(Index(lambda: touched.append(True)), 0)):
                    with self.assertRaises(DocumentClosedError):
                        action()
                if kind == 'pdf':
                    with self.assertRaises(DocumentClosedError):
                        selected[slice(None, None, Index(lambda: touched.append(True), 1))]
                self.assertEqual(touched, [])


if __name__ == '__main__':
    unittest.main()
