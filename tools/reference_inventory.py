#!/usr/bin/env python3
"""Check the pinned comparison inventory offline; --write regenerates its guide.

Optional --source-root verifies previously downloaded sources. Never downloads,
imports comparison backends, or treats reference members as local support proof.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


def require(condition, message):
    if not condition:
        raise ValueError(message)


def keys(value, expected, context):
    require(isinstance(value, dict) and set(value) == set(expected.split()), context + ': unexpected or missing fields')


def strings(value, context, *, nonempty=False):
    require(isinstance(value, list) and all(isinstance(v, str) and v.strip() for v in value), context + ': expected string list')
    require(len(value) == len(set(value)) and (bool(value) or not nonempty), context + ': duplicate or empty entries')
    return value


def validate(root: Path, data: dict, source_root: Path | None = None, *, profile_ids: set[str] | None = None) -> None:
    keys(data, 'schema_version purpose snapshots sources families audited_commit', 'inventory')
    require(data['schema_version'] == 1, 'unknown inventory version')
    require(isinstance(data['purpose'], str) and data['purpose'].strip(), 'missing inventory scope')
    require(isinstance(data['audited_commit'], str) and re.fullmatch(r'[a-f0-9]{40}', data['audited_commit']), 'invalid audit commit')
    roadmap = json.loads((root / 'docs/roadmap-workstreams.json').read_text())
    criteria = {c for w in roadmap['workstreams'] for c in w['coverage']}
    upstream = set(re.findall(r'\bU\d{2}\b', (root / 'docs/upstream-roadmap.md').read_text()))
    if profile_ids is None:
        ledger = json.loads((root / 'docs/support-ledger.json').read_text())
        require(isinstance(ledger, dict) and isinstance(ledger.get('profiles'), list), 'support ledger has no profile inventory')
        require(all(isinstance(p, dict) and isinstance(p.get('id'), str) for p in ledger['profiles']), 'invalid support profile identity')
        profile_ids = {p['id'] for p in ledger['profiles']}
    require(isinstance(data['snapshots'], list) and data['snapshots'], 'snapshots must be nonempty')
    snapshots = {}
    for snapshot in data['snapshots']:
        keys(snapshot, 'id project version commit', 'snapshot')
        require(all(isinstance(v, str) and v for v in snapshot.values()), 'snapshot values must be strings')
        require(re.fullmatch(r'[a-z][a-z0-9-]*', snapshot['id']) and snapshot['id'] not in snapshots, 'invalid or duplicate snapshot ID')
        require(re.fullmatch(r'[\w.-]+/[\w.-]+', snapshot['project']), 'invalid project path')
        require(re.fullmatch(r'[a-f0-9]{40}', snapshot['commit']), 'snapshot must pin an immutable commit')
        snapshots[snapshot['id']] = snapshot
    sources, source_text = {}, {}
    require(isinstance(data['sources'], list) and data['sources'], 'sources must be nonempty')
    for source in data['sources']:
        keys(source, 'key url sha256 bytes lines', 'source')
        key = source['key']
        require(isinstance(key, str) and key not in sources, 'invalid or duplicate source key')
        prefix, separator, path = key.partition(':')
        require(separator and prefix in snapshots and path and not Path(path).is_absolute() and '..' not in Path(path).parts, 'invalid source path')
        snapshot = snapshots[prefix]
        require(source['url'] == f"https://github.com/{snapshot['project']}/blob/{snapshot['commit']}/{path}", 'source URL differs from pinned snapshot')
        require(isinstance(source['sha256'], str) and re.fullmatch(r'[a-f0-9]{64}', source['sha256']), 'invalid source digest')
        require(all(type(source[f]) is int and source[f] > 0 for f in ('bytes', 'lines')), 'invalid source extent')
        if source_root is not None:
            local = (source_root / prefix / path).resolve()
            require(local.is_relative_to(source_root.resolve()), 'cached source escapes source root')
            raw = local.read_bytes()
            require(len(raw) == source['bytes'] and hashlib.sha256(raw).hexdigest() == source['sha256'], 'cached source bytes differ: ' + key)
            lines = raw.decode('utf-8').splitlines()
            require(len(lines) == source['lines'], 'cached source line count differs: ' + key)
            source_text[key] = lines
        sources[key] = source
    require(isinstance(data['families'], list) and data['families'], 'families must be nonempty')
    seen, used = set(), set()
    dispositions = {'implemented_subset', 'local_candidate', 'recipe_existing', 'reference_comparison', 'upstream_blocked', 'upstream_or_unassessed'}
    for family in data['families']:
        keys(family, 'id title reference_members coverage local_surface remaining upstream_candidates profiles disposition', 'family')
        fid = family['id']
        require(isinstance(fid, str) and re.fullmatch(r'REF-[A-Z]+-[A-Z-]+', fid) and fid not in seen, 'invalid or duplicate family ID')
        seen.add(fid)
        for field in ('title', 'local_surface', 'remaining'):
            require(isinstance(family[field], str) and family[field].strip(), fid + ': missing ' + field)
        require(isinstance(family['disposition'], str) and family['disposition'] in dispositions, fid + ': invalid disposition')
        for field, allowed in (('coverage', criteria), ('profiles', profile_ids), ('upstream_candidates', upstream)):
            values = strings(family[field], fid + '.' + field, nonempty=field == 'coverage')
            require(set(values) <= allowed, fid + ': unknown ' + field + ' reference')
        require(isinstance(family['reference_members'], list) and family['reference_members'], fid + ': members must be nonempty')
        members = set()
        for member in family['reference_members']:
            keys(member, 'name source line', fid + ': member')
            require(isinstance(member['name'], str) and re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', member['name']), fid + ': invalid member name')
            source = member['source']
            require(isinstance(source, str) and source in sources, fid + ': unknown member source')
            require(type(member['line']) is int and 1 <= member['line'] <= sources[source]['lines'], fid + ': member line outside source')
            identity = (source, member['name'])
            require(identity not in members, fid + ': duplicate member')
            members.add(identity)
            used.add(source)
            if source_root is not None:
                require(re.search(r'\b' + re.escape(member['name']) + r'\b', source_text[source][member['line'] - 1]), fid + ': member absent at audited line')
    require(used == set(sources), 'source inventory must contain exactly cited snapshots')


def generate(root: Path, data: dict) -> str:
    sources = {s['key']: s for s in data['sources']}
    roadmap = json.loads((root / 'docs/roadmap-workstreams.json').read_text())
    owners = {c: w['id'] for w in roadmap['workstreams'] for c in w['coverage']}
    lines = ['# Pinned reference API families', '', '<!-- Generated by tools/reference_inventory.py --write. -->', '',
             'This comparison groups related reference members under existing Python handles and formatting properties. It is not an exhaustive member inventory, implementation test, or format-conformance claim. Open XML schema/model APIs do not establish pagination or rendering. Reference products are not runtime dependencies.', '',
             f"Audited local surface: `{data['audited_commit']}`. The inventory contains {len(data['families'])} families and {sum(len(f['reference_members']) for f in data['families'])} representative members from {len(data['sources'])} pinned source files. Later implementation changes require review.", '']
    for snapshot in data['snapshots']:
        lines.append(f"- [{snapshot['project']} {snapshot['version']}](https://github.com/{snapshot['project']}/tree/{snapshot['commit']}) — `{snapshot['commit']}`")
    lines += ['', 'Local dispositions describe a bounded existing subset, candidate work, recipe or remaining assessment. A family marked upstream-blocked names the specific limitation in its remaining scope; it does not imply the native library lacks all related primitives. See the separate [operation evidence matrix](support-matrix.md) for tested scopes and artifacts.', '',
              'PyMuPDF core and its separate Office product already document Markdown/structured extraction and Office image rendering respectively. These are reference capabilities, not uniqueness claims for this package.', '',
              'Maintain `reference-api-inventory.json`; run `python tools/reference_inventory.py` to check or add `--write` to regenerate. The optional `--source-root` checks previously downloaded source hashes and cited member lines. Offline structural checks do not independently retrieve or certify external sources.', '']
    for family in data['families']:
        references = ', '.join(f"[{m['name']}]({sources[m['source']]['url']}#L{m['line']})" for m in family['reference_members'])
        lines += ['## ' + family['id'], '', family['title'] + '.', '', '**Reference members:** ' + references, '',
                  '**Criteria / owners:** ' + ', '.join(c + ' (' + owners[c] + ')' for c in family['coverage']), '',
                  '**Audited local surface:** ' + family['local_surface'] + '.', '',
                  '**Disposition:** `' + family['disposition'] + '`. **Remaining:** ' + family['remaining'], '',
                  '**Upstream candidates:** ' + (', '.join(f'[{u}](upstream-roadmap.md)' for u in family['upstream_candidates']) or 'None identified in this family audit.')]
        if family['profiles']:
            lines += ['', '**Evidence profiles:** ' + ', '.join(f'[{p}](support-matrix.md#{p.replace(".", "")})' for p in family['profiles'])]
        lines.append('')
    lines += ['## Criterion joins', '', 'All IDs below are derived from existing ownership. Empty joins mean this bounded reference inventory has no family mapping, not that the criterion is unimplemented. Checkboxes and behavior statuses are not duplicated here.', '', '| Criterion | Owner | Reference families |', '|---|---|---|']
    for criterion, owner in owners.items():
        matches = [f'[{f["id"]}](#{f["id"].lower()})' for f in data['families'] if criterion in f['coverage']]
        lines.append(f'| {criterion} | {owner} | ' + (', '.join(matches) or 'Unassessed / no direct reference family') + ' |')
    return '\n'.join(lines) + '\n'


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--source-root', type=Path)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    try:
        data = json.loads((args.root / 'docs/reference-api-inventory.json').read_text())
        validate(args.root, data, args.source_root)
        output = args.root / 'docs/reference-api-inventory.md'
        expected = generate(args.root, data)
        if args.write:
            output.write_text(expected, encoding='utf-8')
        else:
            require(output.exists() and output.read_text() == expected, 'generated reference guide differs; run with --write')
    except (OSError, UnicodeError, ValueError) as exc:
        print('ERROR: ' + str(exc), file=sys.stderr)
        return 1
    print(f"Reference inventory valid: {len(data['families'])} families. No local capability certification.")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
