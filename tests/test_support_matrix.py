"""Offline consistency and evidence-drift checks; no engine import/build required."""
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('support_matrix', ROOT / 'tools/support_matrix.py')
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class SupportMatrixTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.original = json.loads((ROOT / 'docs/support-ledger.json').read_text(encoding='utf-8'))

    def setUp(self):
        self.ledger = deepcopy(self.original)

    def errors(self, *expected, root=ROOT):
        errors = checker.validate(root, self.ledger)
        for text in expected:
            self.assertTrue(any(text in error for error in errors), (text, errors))
        return errors

    def test_baseline_and_generated_bytes_are_consistent_without_mutation(self):
        self.assertEqual(checker.validate(ROOT, self.ledger), [])
        self.assertEqual(self.ledger, self.original)
        self.assertEqual(len(self.ledger['coverage']), 139)
        self.assertEqual(len(self.ledger['profiles']), 21)
        self.assertEqual(checker.generate(ROOT, self.ledger), (ROOT / 'docs/support-matrix.md').read_text(encoding='utf-8'))

    def test_missing_unknown_coverage_and_unknown_profile_are_rejected(self):
        del self.ledger['coverage']['API1']
        self.ledger['coverage']['BOGUS1'] = {'profiles': ['not.real'], 'remaining': 'Unassessed: missing.'}
        self.errors('missing=API1', 'extra=BOGUS1', 'unknown profile not.real')

    def test_no_second_ownership_or_completion_database(self):
        self.ledger['coverage']['API1'].update(owner='W01', status='complete')
        self.ledger['coverage']['API7']['remaining'] = 'Complete.'
        self.errors('coverage only stores profile joins', 'remainder must explicitly remain unassessed')

    def test_duplicate_profiles_and_missing_operation_cells(self):
        self.ledger['profiles'].append(deepcopy(self.ledger['profiles'][0]))
        del self.ledger['profiles'][0]['operations']['read']
        self.errors('duplicate profile pdf.region', 'all seven operation cells are required')

    def test_supported_cell_needs_tests_native_scope_and_artifact(self):
        cell = self.ledger['profiles'][0]['operations']['render']
        cell.update(tests=[], native=['invented'], artifact='pdfcraft', scope='')
        self.errors('missing scope/reason', 'supported cell needs artifact', 'unknown native source invented', 'tests must not be empty')

    def test_selector_and_source_digest_drift_are_not_silently_refreshed(self):
        cell = self.ledger['profiles'][0]['operations']['render']
        cell['tests'][0] = 'tests/test_render_clipping.py::RenderClippingTests.test_missing'
        self.ledger['test_sources']['tests/test_render_clipping.py'] = '0' * 64
        self.errors('evidence source changed', 'missing fingerprinted test selector')

    def test_native_pin_sdk_snapshot_and_artifact_section_are_verified(self):
        self.ledger['references']['pdfcraft']['revision'] = '0' * 40
        self.ledger['references']['openxml-sdk']['revision'] = '1' * 40
        artifact = self.ledger['references']['cookbook-2026-10-20']
        artifact['validation'] = '.kiro/specs/rust-document-engine/validation.md#does-not-exist'
        self.errors('native pin differs', 'reference snapshot not recorded', 'missing heading', 'wheel hash missing from specified validation section')

    def test_artifact_hash_from_other_section_is_not_accepted(self):
        artifact = self.ledger['references']['cookbook-2026-10-20']
        artifact['validation'] = '.kiro/specs/rust-document-engine/validation.md#2026-10-20-private-office-render-reuse'
        self.errors('wheel hash missing from specified validation section')

    def test_optional_evidence_and_dates_cannot_pass_then_break_generation(self):
        artifact = self.ledger['references']['cookbook-2026-10-20']
        for value in (None, [], '2026-02-30', 'not-a-date'):
            artifact['date'] = value
            self.errors('artifact needs an ISO calendar date')
        artifact['date'] = '2026-10-20'
        cell = self.ledger['profiles'][0]['operations']['read']
        cell.update(artifact='unknown', native=['missing'], tests=[None], blocker=[])
        self.errors('cell needs artifact', 'unknown native source',
                    'tests must be a list', 'blocker must be an object')

    def test_upstream_blocker_needs_real_register_and_native_citation(self):
        profile = next(p for p in self.ledger['profiles'] if p['id'] == 'pdf.vertical-redaction')
        cell = profile['operations']['edit_authored']
        cell['blocker'].update(id='U999', native=['invented'])
        self.errors('unknown upstream blocker', 'unknown blocker native source')

    def test_malformed_structures_are_errors_not_tracebacks(self):
        cases = [None, [], {'schema_version': 1}]
        for malformed in cases:
            with self.subTest(value=malformed):
                self.assertTrue(checker.validate(ROOT, malformed))
        self.ledger['profiles'][0]['operations']['render']['status'] = []
        self.ledger['references']['pdfcraft'] = []
        self.errors('invalid status', 'reference pdfcraft must be an object')

    def test_missing_adapter_symbol_and_unsafe_paths_are_rejected(self):
        self.ledger['profiles'][0]['adapter_sources'] = {'rust/src/pdf.rs': ['made_up_symbol'], '../escape.rs': ['bad']}
        self.ledger['profiles'][0]['native_sources']['raster']['path'] = '/absolute.rs'
        self.errors('missing adapter symbol', 'without traversal', 'native path')

    def test_generated_join_tracks_roadmap_without_storing_ownership(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(ROOT / 'docs', root / 'docs')
            shutil.copy(ROOT / 'TODO.md', root / 'TODO.md')
            before = checker.generate(root, self.ledger)
            todo = (root / 'TODO.md').read_text(encoding='utf-8').replace('- [ ] **API1:', '- [x] **API1:')
            (root / 'TODO.md').write_text(todo, encoding='utf-8')
            after = checker.generate(root, self.ledger)
            self.assertNotEqual(before, after)
            self.assertIn('| W01 / implementation | checked |', after)
            self.assertEqual(self.ledger, self.original)

    def test_cli_checks_exact_generated_output_and_write_is_explicit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ('docs', 'tests', 'tools', 'rust', 'src', '.kiro'):
                shutil.copytree(ROOT / name, root / name, ignore=shutil.ignore_patterns('__pycache__'))
            for name in ('TODO.md', 'Cargo.toml'):
                shutil.copy(ROOT / name, root / name)
            command = [sys.executable, str(root / 'tools/support_matrix.py'), '--root', str(root)]
            initial = subprocess.run(command, capture_output=True, text=True, encoding='utf-8')
            self.assertEqual(initial.returncode, 0, initial.stderr)
            matrix = root / 'docs/support-matrix.md'
            matrix.write_text('stale\n', encoding='utf-8')
            stale = subprocess.run(command, capture_output=True, text=True, encoding='utf-8')
            self.assertEqual(stale.returncode, 1)
            self.assertIn('generated support-matrix.md differs', stale.stderr)
            self.assertEqual(matrix.read_text(encoding='utf-8'), 'stale\n')
            rebuilt = subprocess.run(command + ['--write'], capture_output=True, text=True, encoding='utf-8')
            self.assertEqual(rebuilt.returncode, 0, rebuilt.stderr)
            self.assertEqual(matrix.read_text(encoding='utf-8'), checker.generate(root, self.ledger))
            malformed = root / 'docs/support-ledger.json'
            malformed.write_text('{', encoding='utf-8')
            failed = subprocess.run(command + ['--write'], capture_output=True, text=True, encoding='utf-8')
            self.assertEqual(failed.returncode, 1)
            self.assertNotIn('Traceback', failed.stderr)
            self.assertEqual(matrix.read_text(encoding='utf-8'), checker.generate(root, self.ledger))


if __name__ == '__main__':
    unittest.main()
