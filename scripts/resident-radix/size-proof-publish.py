#!/usr/bin/env python3
"""Audit and package the cardinality extension without closing the index gate."""
import ast
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile

REPO = Path(__file__).resolve().parents[2]
PROOF, DEVELOPMENT, DEST = [Path(arg).resolve() for arg in sys.argv[1:]]
assert PROOF.is_relative_to('/mnt/data/kv9-work')
assert DEVELOPMENT.is_relative_to('/mnt/data/kv9-work')
assert DEST.is_relative_to(REPO / 'docs')
DEST.mkdir(exist_ok=False)
read = lambda path: json.loads(path.read_text())
sha = lambda data: hashlib.sha256(data).hexdigest()
contract = read(REPO / 'proofs/lean/radix/size-contract.json')
result = read(PROOF / 'result.json')
assert read(PROOF / 'source-contract.json') == contract
assert result['accepted'] and result['complete'] and result['complete_algorithm_gate'] is False
assert result['sources'] == contract['sources']
for name, digest in contract['sources'].items():
    assert sha((REPO / name).read_bytes()) == digest, name
for module in contract['modules']:
    assert (PROOF / 'positive' / module).read_bytes() == (REPO / 'proofs/lean/radix' / module).read_bytes()
commands = read(PROOF / 'positive/commands.json')
assert len(commands) == len(contract['modules']) + 1 and all(row['exit_code'] == 0 for row in commands)
axioms = read(PROOF / 'positive/axioms.json')
assert axioms == result['theorems']
assert set(axioms) == {'Kv9.Radix.' + name for name in contract['theorems']}
assert all(set(deps) <= {'propext', 'Classical.choice', 'Quot.sound'} for deps in axioms.values())
expected_controls = contract['controls'] + contract['source_controls']
assert [row['name'] for row in result['controls']] == [row['name'] for row in expected_controls]
assert all(row['rejected'] for row in result['controls'])
control_audit = []
for row in contract['controls']:
    work = PROOF / ('control-' + row['name'])
    log = '\n'.join(path.read_text() for path in work.glob('*.log'))
    steps = read(work / 'commands.json')
    if row['kind'] == 'semantic':
        assert steps[-1]['exit_code'] != 0
        assert not any(message in log for message in ['unexpected token', 'Unknown identifier', 'unknown tactic'])
        assert any(message in log for message in [
            'unsolved goals', 'Type mismatch', 'not definitionally equal to target',
            'Tactic `', 'omega could not prove',
        ]), row['name']
    elif row['kind'] == 'hole':
        assert steps[-1]['exit_code'] != 0 and 'sorry' in log
    elif row['kind'] == 'axiom':
        assert all(step['exit_code'] == 0 for step in steps)
        assert 'injectedCardinalityFalse' in (work / 'Audit.lean.log').read_text()
    else:
        raise ValueError(row['kind'])
    control_audit.append({'name': row['name'], 'kind': row['kind'], 'checked': True})
for row in contract['source_controls']:
    expected = (REPO / 'scripts/resident-radix/src/lib.rs').read_text().replace(row['old'], row['new'])
    assert (PROOF / (row['name'] + '.rs')).read_text() == expected
    assert sha(expected.encode()) != contract['sources']['scripts/resident-radix/src/lib.rs']
protected = {
    'scripts/checkpoint-publication-chaos.py': 'ebef796c6e19d4f3d391aa69b973e22df501f2e9af13f64dfcc7f34fd564d343',
    'scripts/checkpoint-owned-crash.py': '3e9d4babe9f263b2360f676cdcdd92985f4722710394580659890b7c6abe617a',
}
for name, digest in protected.items():
    assert sha((REPO / name).read_bytes()) == digest, name
assert not subprocess.check_output(['git', 'diff', '--name-only', '--', 'crates', 'scripts/resident-radix/src'], cwd=REPO)
for path in (REPO / 'scripts/resident-radix').glob('*.py'):
    ast.parse(path.read_text())
prior = read(REPO / 'docs/radix-point-proof-v1/manifest.json')
assert prior['sha256'] == '54877ebc02ac08e1b63a98397dbec6ab0f5564867fc5b440381707dbac39918d'
audit = {'complete': True, 'theorems_checked_together': len(axioms),
         'additional_theorems': len(contract['new_theorems']), 'new_rejecting_controls': len(result['controls']),
         'controls': control_audit, 'positive_compiler_commands': len(commands),
         'sources_match': True, 'protected_drafts_unchanged': True,
         'candidate_rust_and_production_unchanged': True,
         'machine_counter_scope': 'Conditional on resident-result representability; heap correspondence remains open.',
         'local_bounds_scope': 'Loop/path/search/length premises remain to be established for the actual Rust loops.',
         'complete_algorithm_gate': False, 'remaining': contract['remaining'],
         'prior_point_packet_sha256': prior['sha256']}
(DEST / 'audit.json').write_text(json.dumps(audit, indent=2) + '\n')
(DEST / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
inputs = []
for prefix, root in [('qualification', PROOF), ('development', DEVELOPMENT)]:
    for path in sorted(root.rglob('*')):
        if not path.is_file() or '__pycache__' in path.parts:
            continue
        if '.olean' in path.name or path.suffix in ['.ilean', '.c', '.o']:
            continue
        inputs.append((prefix + '/' + str(path.relative_to(root)), path))
repo_files = set(contract['sources']) | {
    'proofs/lean/radix/size-contract.json', 'proofs/lean/radix/SIZE-README.md',
    'scripts/resident-radix/size-proof-publish.py', 'docs/RADIX-CARDINALITY-PROOF.md',
}
for name in repo_files:
    inputs.append(('repo/' + name, REPO / name))
archive = DEST / 'evidence.tar.gz'
members = []
with archive.open('xb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', mtime=0) as zipped, tarfile.open(fileobj=zipped, mode='w') as tar:
    for name, path in sorted(inputs):
        data = path.read_bytes()
        info = tarfile.TarInfo(name)
        info.size, info.mode = len(data), 0o644
        tar.addfile(info, io.BytesIO(data))
        members.append({'path': name, 'bytes': len(data), 'sha256': sha(data)})
with tarfile.open(archive) as tar:
    assert tar.getnames() == [row['path'] for row in members]
    for row in members:
        data = tar.extractfile(row['path']).read()
        assert len(data) == row['bytes'] and sha(data) == row['sha256']
(DEST / 'members.json').write_text(json.dumps(members, indent=2) + '\n')
manifest = {'archive': 'evidence.tar.gz', 'sha256': sha(archive.read_bytes()), 'bytes': archive.stat().st_size,
            'members': len(members), 'expanded_bytes': sum(row['bytes'] for row in members),
            'all_members_read_back': True, 'excluded': ['Lean compiler outputs', 'Python bytecode'],
            'complete_algorithm_gate': False, 'prior_point_packet_sha256': prior['sha256']}
(DEST / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({'complete': True, 'theorems': len(axioms), 'new_theorems': len(contract['new_theorems']),
                  'controls': len(result['controls']), 'packet': manifest}))
