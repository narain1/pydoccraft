from io import BytesIO
import unittest
from xml.etree import ElementTree as ET
from xml.sax.saxutils import escape, quoteattr
from zipfile import ZipFile

import doc_ex
from doc_ex.errors import UnsafeArchiveError
from test_docx_stories import paragraph, stories_fixture

CP = 'http://schemas.openxmlformats.org/officeDocument/2006/custom-properties'
VT = 'http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes'
W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'


def replace_part(data, name, value):
    output = BytesIO()
    with ZipFile(BytesIO(data)) as source, ZipFile(output, 'w') as target:
        for entry in source.infolist():
            target.writestr(entry, value if entry.filename == name else source.read(entry.filename))
    return output.getvalue()


def property_source(entries=(), *, raw=None):
    with doc_ex.new('docx') as authored:
        authored.add_paragraph('Visible content')
        authored.update_metadata(custom={'Seed': 'Replaced'})
        source = authored.to_bytes()
    if raw is None:
        values = ''.join(f'<property name={quoteattr(name)} pid="{index + 2}"><vt:{kind}>{escape(value)}</vt:{kind}></property>'
                         for index, (name, kind, value) in enumerate(entries))
        raw = f'<Properties xmlns="{CP}" xmlns:vt="{VT}">{values}</Properties>'
    return replace_part(source, 'docProps/custom.xml', raw)


class ImportedPropertyTests(unittest.TestCase):
    def test_authored_scalar_metadata_roundtrips_and_survives_close(self):
        custom = {'Text': '  Ω <&>  ', 'Empty': '', 'Approved': False,
                  'Count': -42, 'Minimum': -(2**63), 'Maximum': 2**63 - 1, 'Ratio': 1.25}
        with doc_ex.new('docx') as authored:
            authored.add_paragraph('Visible content')
            baseline = authored.render_page(0, dpi=72)
            authored.update_metadata(custom=custom)
            with doc_ex.open(authored.to_bytes()) as imported:
                snapshot = imported.metadata['custom']
                self.assertEqual(snapshot, authored.metadata['custom'])
                self.assertEqual(imported.render_page(0, dpi=72), baseline)
                self.assertEqual(imported[0].get_text().strip(), 'Visible content')
                self.assertFalse(imported.metadata['diagnostics'])
            self.assertEqual(snapshot, custom)
        with doc_ex.new('docx') as authored:
            authored.add_paragraph('No custom values')
            with doc_ex.open(authored.to_bytes()) as imported:
                self.assertEqual(imported.metadata['custom'], {})

    def test_unsupported_and_malformed_variants_are_not_coerced(self):
        entries = [('Kept', 'i8', '7'), ('Alias', 'i4', '42'), ('StringAlias', 'lpstr', 'text'),
                   ('One', 'bool', '1'), ('Uppercase', 'bool', 'TRUE'), ('Padded', 'i8', ' 4'),
                   ('Overflow', 'i8', str(2**63)), ('NotFinite', 'r8', 'NaN'),
                   ('Date', 'filetime', '2026-10-20T00:00:00Z')]
        source = property_source(entries)
        with doc_ex.open(source) as imported:
            self.assertEqual(imported.metadata['custom'], {'Kept': 7})
            notices = {notice['code']: notice for notice in imported.metadata['diagnostics']}
            self.assertEqual(notices['unsupported_custom_property']['count'], 3)
            self.assertEqual(notices['malformed_custom_property']['count'], 5)
            self.assertEqual(notices['unsupported_custom_property']['sources'][0], {'property_index': 1, 'kind': 'i4'})
        raw = f'<Properties xmlns="{CP}" xmlns:vt="{VT}">' + ''.join(
            f'<property name="Raw{index}"><vt:vector size="1" baseType="lpwstr"><vt:lpwstr>opaque</vt:lpwstr></vt:vector></property>'
            for index in range(150)) + '</Properties>'
        with doc_ex.open(property_source(raw=raw)) as imported:
            self.assertEqual(imported.metadata['custom'], {})
            notice, = imported.metadata['diagnostics']
            self.assertEqual(notice['code'], 'unsupported_custom_property')
            self.assertEqual(notice['count'], 150)
            self.assertEqual(len(notice['sources']), 100)
            self.assertTrue(all(set(item) == {'property_index', 'kind'} and item['kind'] == 'raw' for item in notice['sources']))
            self.assertNotIn('opaque', str(notice))

    def test_ascii_case_collisions_omit_every_ambiguous_record(self):
        source = property_source([('Name', 'lpwstr', 'first'), ('NAME', 'lpwstr', 'second'),
                                  ('Same', 'bool', 'true'), ('Same', 'i8', '1'),
                                  ('Ä', 'lpwstr', 'upper'), ('ä', 'lpwstr', 'lower')])
        with doc_ex.open(source) as imported:
            self.assertEqual(imported.metadata['custom'], {'Ä': 'upper', 'ä': 'lower'})
            notice, = imported.metadata['diagnostics']
            self.assertEqual(notice['code'], 'ambiguous_custom_property')
            self.assertEqual(notice['count'], 4)
            self.assertEqual([entry['property_index'] for entry in notice['sources']], [0, 1, 2, 3])

    def test_custom_property_resource_limits_reject_before_projection(self):
        with doc_ex.open(property_source([(str(i), 'i8', str(i)) for i in range(1024)])) as imported:
            self.assertEqual(len(imported.metadata['custom']), 1024)
        with doc_ex.open(property_source([('Text', 'lpwstr', 'x' * 65536)])) as imported:
            self.assertEqual(len(imported.metadata['custom']['Text']), 65536)
        for entries in ([(str(i), 'i8', '0') for i in range(1025)],
                        [('Text', 'lpwstr', 'x' * 65537)],
                        [(str(i), 'lpwstr', 'x' * 65530) for i in range(4)]):
            with self.subTest(properties=len(entries)), self.assertRaisesRegex(UnsafeArchiveError, 'custom properties exceed'):
                doc_ex.open(property_source(entries))

    def test_native_malformed_part_and_source_entry_drops_remain_explicit_limitations(self):
        # The pinned native reader discards invalid parts before the adapter sees them.
        # It also takes 10,000 source property elements before skipping nameless records.
        # These fixtures document unavailable completeness, not fabricated diagnostics.
        too_many = f'<Properties xmlns="{CP}" xmlns:vt="{VT}">' + '<property/>' * 10000 + '<property name="BeyondNativeLimit"><vt:i8>7</vt:i8></property></Properties>'
        for raw in ('<Properties><broken', too_many):
            with self.subTest(case='malformed' if raw.startswith('<Properties><broken') else 'native cap'), doc_ex.open(property_source(raw=raw)) as imported:
                self.assertEqual(imported.metadata['custom'], {})
                self.assertFalse(imported.metadata['diagnostics'])
                self.assertIn('Visible content', imported.to_markdown())

    def test_table_titles_descriptions_are_owned_nonvisible_metadata_at_each_depth(self):
        with doc_ex.new('docx') as authored:
            table = authored.add_table([['Outer cell']])
            table.format(title='Outer title', description='Outer description')
            inner = table[0, 0].add_table([['Nested cell']])
            inner.format(title='', description='Nested description')
            baseline = authored.render_page(0, dpi=72)
            with doc_ex.open(authored.to_bytes()) as imported:
                outer = imported[0].find_tables()[0]
                self.assertEqual(outer.metadata['title'], 'Outer title')
                self.assertEqual(outer.metadata['description'], 'Outer description')
                nested = next(block.table for block in outer.cells[0][0].blocks if block.table)
                self.assertEqual(nested.metadata['title'], '')
                self.assertEqual(nested.metadata['description'], 'Nested description')
                self.assertNotIn('description', imported.metadata['custom'])
                self.assertEqual(imported.render_page(0, dpi=72), baseline)
                text = imported.to_markdown()
                for label in ('Outer title', 'Outer description', 'Nested description'):
                    self.assertNotIn(label, text)
                self.assertEqual(text.count('Nested cell'), 1)
            self.assertEqual(nested.metadata['description'], 'Nested description')
            table.format(title=None, description=None)
            with doc_ex.open(authored.to_bytes()) as imported:
                outer = imported[0].find_tables()[0]
                self.assertNotIn('title', outer.metadata)
                self.assertNotIn('description', outer.metadata)
        header = '<w:tbl><w:tblPr><w:tblCaption w:val="Header title"/><w:tblDescription w:val=""/></w:tblPr><w:tr><w:tc>' + paragraph('Header cell') + '</w:tc></w:tr></w:tbl>'
        with doc_ex.open(stories_fixture(header)) as imported:
            story = next(story for story in imported.stories if story.kind == 'header')
            table = next(block.table for block in story.blocks if block.table)
            self.assertEqual(table.metadata['title'], 'Header title')
            self.assertEqual(table.metadata['description'], '')
            self.assertNotIn('Header title', story.get_text('markdown'))

    def test_native_table_alt_limit_exposes_only_retained_prefix(self):
        with doc_ex.new('docx') as authored:
            authored.add_table([['Content']]).format(title='Replace')
            source = authored.to_bytes()
        with ZipFile(BytesIO(source)) as archive:
            root = ET.fromstring(archive.read('word/document.xml'))
        root.find(f'.//{{{W}}}tblCaption').set(f'{{{W}}}val', 'Ω' * 10001)
        source = replace_part(source, 'word/document.xml', ET.tostring(root))
        with doc_ex.open(source) as imported:
            self.assertEqual(imported[0].find_tables()[0].metadata['title'], 'Ω' * 10000)
            self.assertIn('Content', imported.to_markdown())
            self.assertNotIn('Ω', imported.to_markdown())


if __name__ == '__main__':
    unittest.main()
