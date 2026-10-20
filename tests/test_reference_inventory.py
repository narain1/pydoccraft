"""Reference mapping checks are development evidence, not runtime parity tests."""
from copy import deepcopy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('reference_inventory', ROOT / 'tools/reference_inventory.py')
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class ReferenceInventoryTests(unittest.TestCase):
    def setUp(self):
        self.data = json.loads((ROOT / 'docs/reference-api-inventory.json').read_text())

    def test_pinned_inventory_and_generated_joins_are_consistent(self):
        before = deepcopy(self.data)
        checker.validate(ROOT, self.data)
        self.assertEqual(checker.generate(ROOT, self.data), (ROOT / 'docs/reference-api-inventory.md').read_text())
        self.assertEqual(self.data, before)
        roadmap = json.loads((ROOT / 'docs/roadmap-workstreams.json').read_text())
        guide = checker.generate(ROOT, self.data)
        for stream in roadmap['workstreams']:
            for criterion in stream['coverage']:
                self.assertIn(f'| {criterion} | {stream["id"]} |', guide)

    def test_snapshot_urls_lines_and_unique_members_reject_drift(self):
        mutations = [
            lambda d: d['snapshots'][0].update(commit='main'),
            lambda d: d['sources'][0].update(url='https://example.com/unpinned'),
            lambda d: d['families'][0]['reference_members'][0].update(line=10**9),
            lambda d: d['families'][0]['reference_members'].append(d['families'][0]['reference_members'][0]),
        ]
        for mutate in mutations:
            data = deepcopy(self.data)
            mutate(data)
            with self.subTest(mutation=mutate), self.assertRaises(ValueError):
                checker.validate(ROOT, data)

    def test_unknown_joins_and_duplicated_ownership_cannot_enter_inventory(self):
        for field, value in [('coverage', ['MADEUP1']), ('profiles', ['missing.profile']),
                             ('upstream_candidates', ['U999']), ('workstreams', ['W01'])]:
            data = deepcopy(self.data)
            data['families'][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                checker.validate(ROOT, data)
        for malformed in (None, [], {}, {'schema_version': 1}):
            with self.assertRaises(ValueError):
                checker.validate(ROOT, malformed)

    def test_cached_source_hash_and_member_line_checks_are_optional_and_offline(self):
        data = deepcopy(self.data)
        family = data['families'][0]
        source_key = family['reference_members'][0]['source']
        source = next(s for s in data['sources'] if s['key'] == source_key)
        data['families'] = [family]
        data['sources'] = [source]
        raw = '\n'.join(m['name'] for m in family['reference_members']).encode() + b'\n'
        source.update(sha256=hashlib.sha256(raw).hexdigest(), bytes=len(raw), lines=len(family['reference_members']))
        for index, member in enumerate(family['reference_members'], 1):
            member['line'] = index
        prefix, path = source_key.split(':', 1)
        with tempfile.TemporaryDirectory() as directory:
            cache = Path(directory)
            file = cache / prefix / path
            file.parent.mkdir(parents=True)
            file.write_bytes(raw)
            checker.validate(ROOT, data, cache)
            family['reference_members'][0]['line'] = 2
            with self.assertRaisesRegex(ValueError, 'member absent'):
                checker.validate(ROOT, data, cache)
            family['reference_members'][0]['line'] = 1
            file.write_bytes(raw + b'changed')
            with self.assertRaisesRegex(ValueError, 'cached source bytes differ'):
                checker.validate(ROOT, data, cache)
            # Ordinary checks do not access this missing external cache.
            checker.validate(ROOT, data)


if __name__ == '__main__':
    unittest.main()
