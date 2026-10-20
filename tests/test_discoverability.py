import unittest
from unittest.mock import patch

import doc_ex
from doc_ex._authoring import Paragraph


class DiscoverabilityTests(unittest.TestCase):
    def test_existing_child_reprs_are_bounded_and_do_not_resolve_closed_or_stale_content(self):
        with doc_ex.new('docx') as document:
            paragraph = document.add_paragraph('Private content ' * 10000)
            table = document.add_table([['Cell']])
            cell = table[0, 0]
            stale_paragraph = cell.add_paragraph('Old child')
            stale_table = cell.add_table([['Old nested child']])
            style = document.styles.add('LongStyle' * 25)
            cell.text = 'Replacement'
            values = (paragraph, table, stale_paragraph, stale_table, style)
            with patch.object(document, '_call', side_effect=AssertionError('repr read native content')):
                for value in values:
                    result = repr(value)
                    self.assertLess(len(result), 160)
                    self.assertIn('closed=False', result)
                    self.assertNotIn('Private content', result)
                    self.assertNotIn('Replacement', result)
            document.close()
            for value in values:
                self.assertIn('closed=True', repr(value))

    def test_paragraph_subclass_without_native_id_and_unexpected_errors(self):
        # Slide-cell paragraphs share Paragraph behavior but have a target path,
        # not a DOCX block ID. Repr must not resolve that path or assume _id.
        class LocatedParagraph(Paragraph):
            def __init__(self, owner):
                self._owner = owner
                self._target = [1, 2, 3]
        with doc_ex.new('docx') as document:
            value = LocatedParagraph(document)
            with patch.object(document, '_call', side_effect=AssertionError('native read')):
                self.assertEqual(repr(value), 'LocatedParagraph(closed=False)')
            document.close()
            self.assertEqual(repr(value), 'LocatedParagraph(closed=True)')
        with doc_ex.new('pptx') as deck:
            table = deck.slides.add().add_table([['Cell']], box=(10, 10, 200, 80))
            paragraph = table[0, 0].paragraphs[0]
            table[0, 0].text = 'Reset makes prior paragraph stale'
            with patch.object(deck, '_call', side_effect=AssertionError('native read')):
                self.assertEqual(repr(paragraph), '_SlideCellParagraph(closed=False)')
            deck.close()
            self.assertEqual(repr(paragraph), '_SlideCellParagraph(closed=True)')
        class BrokenOwner:
            @property
            def closed(self):
                raise RuntimeError('unexpected owner bug')
        with self.assertRaisesRegex(RuntimeError, 'unexpected owner bug'):
            repr(LocatedParagraph(BrokenOwner()))


if __name__ == '__main__':
    unittest.main()
