import unittest

import doc_ex
from test_authoring import image_fixture
from test_picture_media import media_parts, red_picture, count_color


def field(paragraph, label, kind="page"):
    paragraph.add_run(label + '=')
    paragraph.add_field(kind)


class FieldImageCompositionTests(unittest.TestCase):
    def test_fields_and_images_work_in_both_insertion_orders_and_native_outputs(self):
        payload = image_fixture()
        for image_first in (True, False):
            with self.subTest(image_first=image_first), doc_ex.new('docx') as document:
                if image_first:
                    picture = document.add_image(payload, width=64)
                field(document.add_paragraph(), 'Body page')
                if not image_first:
                    picture = document.add_image(payload, width=64)
                self.assertEqual(picture.data, payload)
                self.assertEqual(media_parts(document), [payload])
                for kind, data in (('docx', document.to_bytes()), ('pdf', document.convert('pdf'))):
                    with doc_ex.open(data) as reopened:
                        self.assertGreater(count_color(reopened.render_page(0, dpi=144), b'\x20\x60\xc0\xff'), 100)
                        if kind == 'pdf':
                            self.assertIn('Body page=1', reopened[0].get_text())

    def test_supported_story_composition_preserves_shared_images_and_reclamation(self):
        blue = image_fixture()
        red = red_picture(16, 32)
        with doc_ex.new('docx') as document:
            body = document.add_image(blue, width=64, alt='Body')
            table = document.add_table([['Cell']])
            nested = table[0, 0].add_table([['Nested']])[0, 0]
            picture = nested.add_image(blue, width=48, alt='Nested')
            field(nested.add_paragraph(), 'Cell pages', 'page_count')
            field(document.sections[0].header().add_paragraph(), 'Header page')
            field(document.sections[0].footer().add_paragraph(), 'Footer page')
            anchor = document.add_paragraph('Note anchor')
            note = document.notes.add(anchor.find('anchor')[0])
            field(note.paragraphs[0], 'Note page')
            self.assertEqual(media_parts(document), [blue])
            before = document.render_page(0, dpi=144)
            with document.copy() as copied:
                picture.data = red
                self.assertCountEqual(media_parts(document), [blue, red])
                self.assertEqual(body.data, blue)
                self.assertEqual(copied.render_page(0, dpi=144), before)
                self.assertEqual(media_parts(copied), [blue])
                self.assertGreater(count_color(document.render_page(0, dpi=144), b'\xff\0\0\xff'), 100)
                del document.images[0]
                self.assertEqual(media_parts(document), [red])
                with doc_ex.open(document.convert('pdf')) as pdf:
                    text = pdf[0].get_text()
                    for label in ('Cell pages=1', 'Header page=1', 'Footer page=1', 'Note page=1'):
                        self.assertIn(label, text)
                    self.assertGreater(count_color(pdf.render_page(0, dpi=144), b'\xff\0\0\xff'), 100)
                del nested.images[0]
                self.assertEqual(media_parts(document), [])
                self.assertEqual(media_parts(copied), [blue])
                # Resource pruning leaves every field live, and a new image may reuse
                # the released budget without altering any story's field markers.
                with doc_ex.open(document.convert('pdf')) as pdf:
                    self.assertIn('Note page=1', pdf[0].get_text())
                fresh = document.add_image(red, width=32)
                self.assertEqual(fresh.data, red)
                self.assertEqual(media_parts(document), [red])


if __name__ == '__main__':
    unittest.main()
