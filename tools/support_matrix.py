#!/usr/bin/env python3
"""Validate the offline development evidence ledger; --write regenerates its matrix.

This checks references and drift, not document correctness or roadmap completion.
Python 3.11+, standard library only. Run from any directory with optional --root.
"""
from __future__ import annotations

import argparse
import ast
from datetime import date as calendar_date
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys
import tomllib

_roadmap_spec = importlib.util.spec_from_file_location("support_roadmap", Path(__file__).with_name("check_roadmap.py"))
check_roadmap = importlib.util.module_from_spec(_roadmap_spec)
_roadmap_spec.loader.exec_module(check_roadmap)
_reference_spec = importlib.util.spec_from_file_location("reference_inventory", Path(__file__).with_name("reference_inventory.py"))
reference_inventory = importlib.util.module_from_spec(_reference_spec)
_reference_spec.loader.exec_module(reference_inventory)

OPERATIONS = ('read', 'create', 'edit_authored', 'edit_imported', 'preserve', 'render', 'export')
STATUSES = {'verified', 'partial', 'unsupported', 'blocked_upstream', 'unassessed', 'not_applicable'}
SUPPORTED = {'verified', 'partial'}
COVERAGE = re.compile(r'^- \[([ xX])\] \*\*([A-Z]+\d+): ([^*]+)\*\*', re.M)
HEX40 = re.compile(r'[0-9a-f]{40}')
HEX64 = re.compile(r'[0-9a-f]{64}')


def safe_path(root: Path, value: object) -> Path:
    if not isinstance(value, str) or not value or Path(value).is_absolute() or '..' in Path(value).parts:
        raise ValueError('expected a repository-relative path without traversal')
    resolved = (root / value).resolve()
    if not resolved.is_relative_to(root.resolve()):
        raise ValueError('path escapes repository')
    return resolved


def anchor(text: str) -> str:
    return re.sub(r'[^\w\- ]', '', text.lower()).replace(' ', '-')


def validate(root: Path, ledger: object) -> list[str]:
    """Return validation errors without changing any file or running document code."""
    errors = []
    def require(condition, message):
        if not condition:
            errors.append(message)
        return condition
    def mapping(value, context):
        if not isinstance(value, dict):
            errors.append(context + ' must be an object')
            return {}
        return value
    def strings(value, context, nonempty=False):
        if not isinstance(value, list) or any(not isinstance(v, str) or not v.strip() for v in value):
            errors.append(context + ' must be a list of nonempty strings')
            return []
        require(len(value) == len(set(value)), context + ' contains duplicates')
        if nonempty:
            require(bool(value), context + ' must not be empty')
        return value
    def read(path):
        return safe_path(root, path).read_text(encoding='utf-8')
    def reference(value, context):
        if not isinstance(value, str):
            errors.append(context + ' must be a file reference')
            return ''
        path, _, fragment = value.partition('#')
        try:
            text = read(path)
            if fragment:
                headings = {anchor(m.group(1)) for m in re.finditer(r'^#+ (.+)$', text, re.M)}
                require(fragment in headings, context + ': missing heading ' + fragment)
            return text
        except (OSError, UnicodeError, ValueError) as exc:
            errors.append(context + ': ' + str(exc))
            return ''
    ledger = mapping(ledger, 'ledger')
    require(set(ledger) == {'schema_version', 'audited_commit', 'references', 'test_sources', 'profiles', 'coverage'}, 'unexpected or missing top-level keys')
    require(ledger.get('schema_version') == 1, 'schema_version must be 1')
    require(isinstance(ledger.get('audited_commit'), str) and bool(HEX40.fullmatch(ledger['audited_commit'])), 'audited_commit must be an immutable commit')
    try:
        todo = read('TODO.md')
        ownership = json.loads(read('docs/roadmap-workstreams.json'))
        upstream = read('docs/upstream-roadmap.md')
        cargo = tomllib.loads(read('Cargo.toml'))
    except (OSError, ValueError, UnicodeError) as exc:
        return errors + ['cannot read repository inputs: ' + str(exc)]
    errors.extend(check_roadmap.validate(todo, ownership, upstream))
    ids = {item[1] for item in COVERAGE.findall(todo)}
    upstream_ids = set(check_roadmap.UPSTREAM.findall(upstream))
    refs = {key: mapping(value, 'reference ' + key) for key, value in mapping(ledger.get('references'), 'references').items()}
    engine_pins = {}
    def visit(value):
        if isinstance(value, dict):
            if 'git' in value and 'rev' in value:
                engine_pins.setdefault(value['git'].removesuffix('.git'), set()).add(value['rev'])
            for item in value.values():
                visit(item)
        elif isinstance(value, list):
            for item in value:
                visit(item)
    visit(cargo)
    for name, raw in refs.items():
        ref = mapping(raw, 'reference ' + name)
        kind = ref.get('kind')
        if kind == 'engine':
            revision, repository = ref.get('revision'), ref.get('repository')
            require(isinstance(revision, str) and bool(HEX40.fullmatch(revision)), name + ': invalid immutable revision')
            require(isinstance(repository, str) and isinstance(revision, str) and engine_pins.get(repository) == {revision}, name + ': native pin differs from Cargo.toml')
        elif kind == 'external':
            text = reference(ref.get('source'), name)
            revision = ref.get('revision')
            require(isinstance(revision, str) and bool(HEX40.fullmatch(revision)) and revision in text, name + ': reference snapshot not recorded in source')
        elif kind == 'artifact':
            try:
                calendar_date.fromisoformat(ref.get('date', ''))
            except (TypeError, ValueError):
                errors.append(name + ': artifact needs an ISO calendar date')
            commit = ref.get('source_commit')
            require(isinstance(commit, str) and bool(HEX40.fullmatch(commit)), name + ': artifact needs source commit')
            text = reference(ref.get('validation'), name)
            digest = ref.get('wheel_sha256')
            # Restrict the hash check to the named section, not any historical artifact.
            if isinstance(ref.get('validation'), str) and '#' in ref['validation']:
                target = ref['validation'].split('#', 1)[1]
                parts = re.split(r'(?m)^## (.+)\n', text)
                text = next((parts[i + 1] for i in range(1, len(parts) - 1, 2) if anchor(parts[i]) == target), '')
            require(isinstance(digest, str) and bool(HEX64.fullmatch(digest)) and digest in text, name + ': wheel hash missing from specified validation section')
        else:
            errors.append(name + ': unknown reference kind')
    require({ref.get('repository') for ref in refs.values() if ref.get('kind') == 'engine' and isinstance(ref.get('repository'), str)} == set(engine_pins), 'engine reference inventory differs from Cargo Git repositories')
    test_sources = mapping(ledger.get('test_sources'), 'test_sources')
    parsed = {}
    for path, digest in test_sources.items():
        try:
            data = safe_path(root, path).read_bytes()
            require(isinstance(digest, str) and bool(HEX64.fullmatch(digest)) and hashlib.sha256(data).hexdigest() == digest, path + ': evidence source changed; review assertions and fixture provenance before refreshing its digest')
            parsed[path] = ast.parse(data)
        except (OSError, ValueError, SyntaxError) as exc:
            errors.append(path + ': ' + str(exc))
    raw_profiles = ledger.get('profiles')
    if not isinstance(raw_profiles, list):
        errors.append('profiles must be a list')
        raw_profiles = []
    profiles = {}
    used_tests = set()
    for index, raw in enumerate(raw_profiles):
        profile = mapping(raw, f'profiles[{index}]')
        pid = profile.get('id')
        if not require(isinstance(pid, str) and bool(re.fullmatch(r'[a-z][a-z0-9.-]+', pid)), f'profiles[{index}]: invalid id'):
            continue
        require(pid not in profiles, 'duplicate profile ' + pid)
        profiles[pid] = profile
        require(set(profile) == {'id', 'format', 'scope', 'public_members', 'adapter_sources', 'native_sources', 'fixture_provenance', 'operations'}, pid + ': unexpected or missing profile keys')
        require(profile.get('format') in ('pdf', 'docx', 'pptx'), pid + ': invalid format')
        for field in ('scope', 'fixture_provenance'):
            require(isinstance(profile.get(field), str) and bool(profile[field].strip()), pid + ': missing ' + field)
        strings(profile.get('public_members'), pid + '.public_members')
        adapters = mapping(profile.get('adapter_sources'), pid + '.adapter_sources')
        for path, symbols in adapters.items():
            text = reference(path, pid + ': adapter')
            for symbol in strings(symbols, pid + ': adapter symbols', nonempty=True):
                require(bool(re.search(r'\b' + re.escape(symbol) + r'\b', text)), pid + ': missing adapter symbol ' + symbol)
        native = mapping(profile.get('native_sources'), pid + '.native_sources')
        for key, source in native.items():
            source = mapping(source, pid + ': native ' + key)
            require(isinstance(source.get('reference'), str) and refs.get(source['reference'], {}).get('kind') == 'engine', pid + ': invalid native reference')
            try:
                safe_path(root, source.get('path'))
            except ValueError as exc:
                errors.append(pid + ': native path: ' + str(exc))
            strings(source.get('symbols'), pid + ': native symbols', nonempty=True)
        cells = mapping(profile.get('operations'), pid + '.operations')
        require(set(cells) == set(OPERATIONS), pid + ': all seven operation cells are required')
        for operation, raw in cells.items():
            cell = mapping(raw, pid + '.' + operation)
            context = pid + '.' + operation
            status = cell.get('status')
            require(set(cell) <= {'status', 'scope', 'artifact', 'native', 'tests', 'blocker'}, context + ': unknown operation keys')
            require(isinstance(status, str) and status in STATUSES, context + ': invalid status')
            require(isinstance(cell.get('scope'), str) and bool(cell['scope'].strip()), context + ': missing scope/reason')
            supported = isinstance(status, str) and status in SUPPORTED
            if supported or 'artifact' in cell:
                require(isinstance(cell.get('artifact'), str) and refs.get(cell['artifact'], {}).get('kind') == 'artifact', context + ': supported cell needs artifact')
            if supported or 'native' in cell:
                for key in strings(cell.get('native'), context + ': native', nonempty=supported):
                    require(key in native, context + ': unknown native source ' + key)
            if supported or 'tests' in cell:
                for selector in strings(cell.get('tests'), context + ': tests', nonempty=supported):
                    path, sep, target = selector.partition('::')
                    used_tests.add(path)
                    tree = parsed.get(path)
                    names = target.split('.')
                    found = bool(sep and tree and len(names) == 2 and any(isinstance(n, ast.ClassDef) and n.name == names[0] and any(isinstance(m, ast.FunctionDef) and m.name == names[1] for m in n.body) for n in tree.body))
                    require(found, context + ': missing fingerprinted test selector ' + selector)
            if status == 'blocked_upstream' or 'blocker' in cell:
                blocker = mapping(cell.get('blocker'), context + ': blocker')
                require(isinstance(blocker.get('id'), str) and blocker['id'] in upstream_ids, context + ': unknown upstream blocker')
                register = reference(blocker.get('register'), context + ': blocker register')
                require(isinstance(blocker.get('id'), str) and blocker['id'] in set(check_roadmap.UPSTREAM.findall(register)), context + ': blocker ID absent from cited register')
                for key in strings(blocker.get('native'), context + ': blocker native', nonempty=True):
                    require(key in native, context + ': unknown blocker native source ' + key)
    require(used_tests == set(test_sources), 'test_sources must exactly match referenced operation test files')
    coverage = mapping(ledger.get('coverage'), 'coverage')
    require(set(coverage) == ids, 'coverage IDs differ from TODO.md: missing=' + ','.join(sorted(ids - set(coverage))) + '; extra=' + ','.join(sorted(set(coverage) - ids)))
    used_profiles = set()
    for identifier, raw in coverage.items():
        item = mapping(raw, 'coverage ' + identifier)
        require(set(item) == {'profiles', 'remaining'}, identifier + ': coverage only stores profile joins and unassessed remainder, not ownership/status')
        for pid in strings(item.get('profiles'), identifier + '.profiles'):
            used_profiles.add(pid)
            require(pid in profiles, identifier + ': unknown profile ' + pid)
        require(isinstance(item.get('remaining'), str) and item['remaining'].startswith('Unassessed:'), identifier + ': remainder must explicitly remain unassessed')
    require(used_profiles == set(profiles), 'profiles must all be referenced by coverage')
    return errors


def generate(root: Path, ledger: dict) -> str:
    """Render validated evidence; ownership/title/checkbox state is always joined."""
    def esc(value):
        return str(value).replace('|', '\\|').replace('\n', ' ')
    ownership = json.loads((root / 'docs/roadmap-workstreams.json').read_text())
    inventory = json.loads((root / 'docs/reference-api-inventory.json').read_text())
    reference_inventory.validate(root, inventory, profile_ids={p['id'] for p in ledger['profiles']})
    reference_families = inventory['families']
    owners = {c: (w['id'], w['feature_roles'][c]) for w in ownership['workstreams'] for c in w['coverage']}
    lines = ['# Operation support evidence', '', '<!-- Generated by tools/support_matrix.py --write; edit support-ledger.json. -->', '',
             f'This initial inventory records {len(ledger["profiles"])} bounded profiles, not complete format parity. '
             'All roadmap IDs remain visible below. Statuses cite historical installed-wheel evidence; '
             'validation of this file is not a new test run or a completion decision.', '',
             f'Ledger source-audit baseline: `{ledger["audited_commit"]}`. Historical tests do not certify this source revision. Native adapter/facade changes need their own behavioral review and release validation; matching fixture hashes are only a drift check.', '',
             'Operations: **read** normalized content; **create** native-format serialization; '
             '**edit authored/imported** distinguish model provenance; **preserve** only named unchanged content; '
             '**render** PNG; **export** conversion to a named different format. Saving is not conversion.', '',
             'Statuses: **verified** for the stated scope; **partial** with a stated gap; **unsupported** in the current facade; '
             '**blocked upstream** for a cited native constraint; **unassessed** without adequate evidence; **n/a** where the operation has no meaning.', '',
             'The pinned SDK feature-family crosswalk remains in [API design](authoring-api-design.md). '
             'The [pinned reference-family inventory](reference-api-inventory.md) groups representative SDK/PyMuPDF members; complete member dispositions remain unfinished work. '
             'No public capability-query API is introduced.', '',
             'Maintain `docs/support-ledger.json`; run `python tools/support_matrix.py` to check and '
             '`python tools/support_matrix.py --write` to regenerate. A changed test-file digest requires '
             'review of its assertions/fixture provenance and relevant artifact evidence, not a blind hash refresh. '
             'Native citations were source-audited at their immutable pins; the offline checker validates '
             'their references, not native code execution or remote symbol existence.', '',
             '| Profile | Read | Create | Edit authored | Edit imported | Preserve | Render | Export |',
             '|---|---|---|---|---|---|---|---|']
    labels = {'not_applicable': 'n/a', 'blocked_upstream': 'blocked upstream', 'edit_authored': 'Edit authored', 'edit_imported': 'Edit imported'}
    for p in ledger['profiles']:
        lines.append('| [' + p['id'] + '](#' + anchor(p['id']) + ') | ' + ' | '.join(labels.get(p['operations'][o]['status'], p['operations'][o]['status']) for o in OPERATIONS) + ' |')
    for p in ledger['profiles']:
        lines += ['', '## ' + p['id'], '', p['scope'], '', '**Public paths:** ' + (', '.join('`' + s + '`' for s in p['public_members']) or 'None; this is an explicit gap.') , '', '**Fixture provenance:** ' + p['fixture_provenance'], '', '| Operation | Evidence and limits |', '|---|---|']
        for operation in OPERATIONS:
            cell = p['operations'][operation]
            evidence = []
            for selector in cell.get('tests', []):
                path, _, target = selector.partition('::')
                evidence.append('[' + target + '](../' + path + ')')
            if 'artifact' in cell:
                evidence.append('[historical artifact: ' + ledger['references'][cell['artifact']]['date'] + '](../' + ledger['references'][cell['artifact']]['validation'] + ')')
            native_keys = cell.get('native', []) + cell.get('blocker', {}).get('native', [])
            for key in dict.fromkeys(native_keys):
                n = p['native_sources'][key]
                ref = ledger['references'][n['reference']]
                evidence.append('[' + ', '.join(n['symbols']) + '](' + ref['repository'] + '/blob/' + ref['revision'] + '/' + n['path'] + ')')
            if 'blocker' in cell:
                evidence.append('[' + cell['blocker']['id'] + '](../' + cell['blocker']['register'] + ')')
            lines.append('| ' + labels.get(operation, operation.title()) + ' | **' + labels.get(cell['status'], cell['status']) + '** — ' + esc(cell['scope']) + (' ' + '; '.join(evidence) if evidence else '') + ' |')
        families = [f'[{f["id"]}](reference-api-inventory.md#{f["id"].lower()})' for f in reference_families if p['id'] in f['profiles']]
        if families:
            lines += ['', '**Comparison families:** ' + ', '.join(families)]
    lines += ['', '## Roadmap coverage joins', '', 'Owners, roles and checkboxes below come from existing roadmap files. A checked criterion does not verify every operation, and an unseeded criterion is not presumed unimplemented. The remainder is deliberately unassessed, including recipes and acceptance gates.', '', '| ID / current criterion | Owner / role | Roadmap | Profiles | Remaining audit |', '|---|---|---|---|---|']
    for mark, identifier, title in COVERAGE.findall((root / 'TODO.md').read_text()):
        item = ledger['coverage'][identifier]
        profiles = ', '.join('[' + p + '](#' + anchor(p) + ')' for p in item['profiles']) or '—'
        lines.append(f'| {identifier}: {esc(title)} | {owners[identifier][0]} / {owners[identifier][1]} | {"checked" if mark.lower() == "x" else "open"} | {profiles} | {esc(item["remaining"])} |')
    return '\n'.join(lines) + '\n'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--write', action='store_true', help='write the matrix only after successful validation')
    args = parser.parse_args()
    try:
        ledger = json.loads((args.root / 'docs/support-ledger.json').read_text())
        errors = validate(args.root, ledger)
        if errors:
            print('\n'.join('ERROR: ' + e for e in errors), file=sys.stderr)
            return 1
        rendered = generate(args.root, ledger)
        output = args.root / 'docs/support-matrix.md'
        if args.write:
            output.write_text(rendered, encoding='utf-8')
        elif not output.exists() or output.read_text(encoding='utf-8') != rendered:
            print('ERROR: generated support-matrix.md differs; run tools/support_matrix.py --write', file=sys.stderr)
            return 1
    except (OSError, UnicodeError, ValueError) as exc:
        print('ERROR: cannot validate support ledger: ' + str(exc), file=sys.stderr)
        return 1
    print(f'Support evidence valid: {len(ledger["profiles"])} profiles, {len(ledger["coverage"])} coverage joins. No behavioral tests run.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
