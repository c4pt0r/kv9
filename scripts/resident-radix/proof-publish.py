#!/usr/bin/env python3
"""Audit and retain the scoped point-proof packet; never close the index gate."""
import ast
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import tarfile

REPO = Path(__file__).resolve().parents[2]
BASE = Path('/mnt/data/kv9-work')
PROOF = BASE / 'radix-point-proof-20260916-first'
DEVELOPMENT = BASE / 'radix-proof-development-20260916-first'
DEST = REPO / 'docs/radix-point-proof-v1'
DEST.mkdir(exist_ok=False)
read = lambda p: json.loads(p.read_text())
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
result = read(PROOF / 'result.json')
contract = read(REPO / 'proofs/lean/radix/source-contract.json')
assert read(PROOF / 'source-contract.json') == contract
assert result['complete'] and result['accepted'] and result['complete_algorithm_gate'] is False
assert result['sources'] == contract['sources']
for name, digest in contract['sources'].items():
    assert sha(REPO / name) == digest, name
for module in contract['modules']:
    assert (PROOF / 'positive' / module).read_bytes() == (REPO / 'proofs/lean/radix' / module).read_bytes()
    assert 'set_option autoImplicit false' in (REPO / 'proofs/lean/radix' / module).read_text()
commands = read(PROOF / 'positive/commands.json')
assert len(commands) == 11 and all(row['exit_code'] == 0 for row in commands)
axioms = read(PROOF / 'positive/axioms.json')
assert axioms == result['theorems'] and len(axioms) == 155
assert set(axioms) == {'Kv9.Radix.' + name for name in contract['theorems']}
assert all(set(deps) <= {'propext', 'Classical.choice', 'Quot.sound'} for deps in axioms.values())
assert len(result['controls']) == 17 and all(row['rejected'] for row in result['controls'])
semantic_controls = []
for row in result['controls'][:12]:
    work = PROOF / ('control-' + row['name'])
    log = '\n'.join(path.read_text() for path in work.glob('*.log'))
    assert any(message in log for message in [
        'unsolved goals', 'Type mismatch', 'not definitionally equal to target',
        'Tactic `rewrite` failed', 'Tactic `apply` failed', 'Tactic `simp` failed',
    ]), row['name']
    assert read(work / 'commands.json')[-1]['exit_code'] != 0
    semantic_controls.append({'name': row['name'], 'substantive_proof_failure': True})
assert 'sorry' in (PROOF / 'control-proof-hole/Tree.lean.log').read_text()
assert 'injectedFalse' in (PROOF / 'control-custom-axiom/Audit.lean.log').read_text()
for name, expected in {
    'scripts/checkpoint-publication-chaos.py': 'ebef796c6e19d4f3d391aa69b973e22df501f2e9af13f64dfcc7f34fd564d343',
    'scripts/checkpoint-owned-crash.py': '3e9d4babe9f263b2360f676cdcdd92985f4722710394580659890b7c6abe617a',
}.items():
    assert sha(REPO / name) == expected
assert not subprocess.check_output(['git', 'diff', '--name-only', '--', 'crates', 'scripts/resident-radix/src'], cwd=REPO)
for path in (REPO / 'scripts/resident-radix').glob('*.py'):
    ast.parse(path.read_text())
audit = {'complete': True, 'theorems': 155, 'rejecting_controls': 17,
         'all_positive_modules_match_reviewed_source': True, 'positive_compiler_commands': len(commands),
         'semantic_controls': semantic_controls, 'allowed_axioms': ['propext', 'Classical.choice', 'Quot.sound'],
         'protected_drafts_unchanged': True, 'rust_index_and_production_crates_unchanged': True,
         'source_mapping': 'Reviewed and hash-bound; not verified Rust extraction.',
         'complete_algorithm_gate': False, 'remaining': contract['remaining']}
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
for root in [REPO / 'proofs/lean/radix']:
    for path in sorted(root.rglob('*')):
        if path.is_file():
            inputs.append(('repo/' + str(path.relative_to(REPO)), path))
for name in ['scripts/resident-radix/src/lib.rs', 'scripts/resident-radix/src/tests.rs',
             'scripts/resident-radix/prove.py', 'scripts/resident-radix/proof-publish.py']:
    inputs.append(('repo/' + name, REPO / name))
archive = DEST / 'evidence.tar.gz'
members = []
with archive.open('xb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', mtime=0) as zipped, tarfile.open(fileobj=zipped, mode='w') as tar:
    for name, path in sorted(inputs):
        data = path.read_bytes()
        info = tarfile.TarInfo(name)
        info.size = len(data)
        info.mode = 0o644
        tar.addfile(info, io.BytesIO(data))
        members.append({'path': name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
with tarfile.open(archive) as tar:
    assert tar.getnames() == [row['path'] for row in members]
    for row in members:
        data = tar.extractfile(row['path']).read()
        assert len(data) == row['bytes'] and hashlib.sha256(data).hexdigest() == row['sha256']
(DEST / 'members.json').write_text(json.dumps(members, indent=2) + '\n')
manifest = {'archive': 'evidence.tar.gz', 'sha256': sha(archive), 'bytes': archive.stat().st_size,
            'members': len(members), 'expanded_bytes': sum(row['bytes'] for row in members),
            'all_members_read_back': True, 'excluded': ['Lean compiler outputs', 'Python bytecode'],
            'complete_algorithm_gate': False,
            'prior_rust_qualification': {'manifest': 'docs/persistent-radix-prototype-v1/manifest.json',
                'archive_sha256': '03839a5c0a4bc7278c823e8d6599221f581c8d8a1c5cfe8f9883fcc000762a7a'}}
assert read(REPO / manifest['prior_rust_qualification']['manifest'])['sha256'] == manifest['prior_rust_qualification']['archive_sha256']
(DEST / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({'complete': True, 'theorems': 155, 'controls': 17, 'packet': manifest}))
