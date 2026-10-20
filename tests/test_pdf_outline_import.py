import unittest

import doc_ex
from doc_ex.errors import DocExError


class PdfOutlineImportTests(unittest.TestCase):
    def test_native_import_preserves_nested_outlines_and_destinations(self):
        with doc_ex.new('pdf') as source, doc_ex.new('pdf') as target:
            source.pages.add()
            source.pages.add()
            for index, page in enumerate(source.pages):
                page.draw_text(f'Source {index}', at=(30, 30))
            chapter = source.bookmarks.add('Chapter zero', 0)
            chapter.children.add('Detail two', 2)
            source.bookmarks.add('Omitted one', 1)
            source.pages[0].annotations.add_link(box=(20, 70, 100, 20), page=2)
            source.pages[0].annotations.add_link(box=(20, 100, 100, 20), uri='https://example.com/source')
            old = target.bookmarks.add('Existing', 0)
            imported = target.pages.insert(1, source, pages=[2, 0, 2], outline_title='Imported report')
            self.assertEqual([page.number for page in imported], [1, 2, 3])
            with self.assertRaises(ValueError):
                _ = old.title
            self.assertEqual([item.title for item in target.bookmarks], ['Existing', 'Imported report'])
            children = target.bookmarks[1].children
            self.assertEqual([(item.title, item.page) for item in children], [('Chapter zero', 2), ('Omitted one', None)])
            self.assertEqual(children[0].children[0].page, 1)
            links = target.pages[2].annotations
            self.assertEqual([item.target_page for item in links if item.uri is None], [1])
            self.assertIn('https://example.com/source', [item.uri for item in links])
            with doc_ex.open(target.to_bytes()) as reopened, reopened.edit() as edited:
                self.assertEqual(edited.bookmarks[1].children[0].children[0].page, 1)
                edited.pages.move(1, 3)
                self.assertEqual(edited.bookmarks[1].children[0].children[0].page, 3)
                edited.pages[3].rotate(90)
                self.assertTrue(edited.render_page(3, dpi=72).startswith(b'\x89PNG'))

    def test_subset_reorder_delete_preserve_links_and_rendered_content(self):
        with doc_ex.new('pdf') as source, doc_ex.new('pdf') as merged:
            source.pages.add()
            source.pages[0].draw_text('Linked content', at=(30, 30))
            source.pages[1].draw_text('Destination', at=(30, 30))
            source.pages[0].annotations.add_link(box=(20, 70, 100, 20), page=1)
            source.pages[0].annotations.add_link(box=(20, 100, 100, 20), uri='https://example.com/retained')
            merged.pages.insert(1, source)
            with merged.copy(pages=[1, 2]) as subset:
                subset.pages.move(0, 1)
                with doc_ex.open(subset.to_bytes()) as reopened, reopened.edit() as edited:
                    self.assertEqual(edited.page_count, 2)
                    links = list(edited.pages[1].annotations)
                    self.assertEqual([link.target_page for link in links if link.uri is None], [0])
                    self.assertEqual([link.uri for link in links if link.uri], ['https://example.com/retained'])
                    self.assertEqual(edited.render_page(1, dpi=72), source.render_page(0, dpi=72))
                    del edited.pages[0]
                    self.assertEqual(edited.page_count, 1)
                    links = list(edited.pages[0].annotations)
                    self.assertTrue(all(link.target_page is None for link in links))
                    self.assertIn('https://example.com/retained', [link.uri for link in links])
                    self.assertEqual(edited.render_page(0, dpi=72), source.render_page(0, dpi=72))
                    with doc_ex.open(edited.to_bytes()) as final:
                        self.assertIn('Linked content', final.to_markdown())

    def test_optional_outline_policy_and_failed_import_are_atomic(self):
        with doc_ex.new('pdf') as source, doc_ex.new('pdf') as target:
            source.bookmarks.add('Source title', 0)
            target.pages[0].fields.add('owner', (20, 20, 160, 30)).value = 'Preserved'
            target.pages.insert(1, source)
            self.assertEqual(len(target.bookmarks), 0)
            self.assertEqual(target.fields['owner'].value, 'Preserved')
            old = target.pages[0]
            before = target.to_bytes()
            with self.assertRaises((ValueError, DocExError)):
                target.pages.insert(0, source, outline_title='')
            self.assertEqual(target.to_bytes(), before)
            self.assertEqual(old.number, 0)
            with doc_ex.new('pdf') as blank:
                target.pages.insert(0, blank, outline_title='No outline container')
            self.assertEqual(len(target.bookmarks), 0)


if __name__ == '__main__':
    unittest.main()
