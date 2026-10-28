import base64
import unittest

from doc_ex._document import Page
from doc_ex.models import Block, Cell, Image, Link, Table


class PageHtmlTests(unittest.TestCase):
    def test_semantic_content_and_images(self):
        image = Image('x.png', b'png', 'image/png', 'a"b')
        link = Link('https://example.com/?a=1&b=2', 'link', metadata={'start': 2, 'end': 6})
        table = Table([['unused']], cells=[[Cell([Block('paragraph', '<cell>')], col_span=2)]])
        page = Page(0, [Block('heading', 'A link &', level=2, links=[link]), Block('table', table=table), Block('paragraph', images=[image])], images=[image])
        html = page.get_text('html')
        self.assertIn('<h2>A <a href="https://example.com/?a=1&amp;b=2">link</a> &amp;</h2>', html)
        self.assertIn('colspan="2"', html)
        self.assertIn('&lt;cell&gt;', html)
        self.assertNotIn('unused', html)
        self.assertEqual(html.count('<img '), 1)
        self.assertIn(base64.b64encode(b'png').decode(), html)
        self.assertIn('alt="a&quot;b"', html)

    def test_unsafe_link_is_text(self):
        link = Link('javascript:alert(1)', 'click', metadata={'start': 0, 'end': 5})
        self.assertEqual(Page(0, [Block('paragraph', 'click', links=[link])]).get_text('html'), '<p>click</p>')


if __name__ == '__main__':
    unittest.main()
