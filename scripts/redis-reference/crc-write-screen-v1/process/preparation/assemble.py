#!/usr/bin/env python3
"""Bind separately built same-source correctness client to the timed server."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import time

assert __debug__
SOURCE = Path('/tmp/kv9-wal-crc32-table')
SERVER = Path('/tmp/kv9-wal-crc32-release-first')
WORKLOAD = Path('/tmp/kv9-crc-correctness-workload-release-first')
OUT = Path('/tmp/kv9-crc-recovery-release-first')
REVISION = 'ca0002c7f8e9ee6f595efcc9f4151085ccce87cb'


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(path.read_text())


assert not OUT.exists()
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=SOURCE, text=True).strip() == REVISION
assert not subprocess.check_output(['git', 'status', '--porcelain=v1'], cwd=SOURCE)
assert sha(SERVER / 'build.json') == '8c2ea115afc1ec8b6c82224dc6449d1d2439f5f27b5758e45090589752d186e8'
assert sha(SERVER / 'kv9') == 'b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13'
server = read(SERVER / 'build.json')
workload = read(WORKLOAD / 'build.json')
inventory = read(WORKLOAD / 'sources.json')
assert server['revision'] == workload['revision'] == inventory['revision'] == REVISION
assert not server['dirty'] and not workload['dirty'] and not inventory['dirty']
assert server['sources'] == inventory['sources'] and len(server['sources']) == 590
assert server['source_tree_sha256'] == workload['source_tree_sha256'] == inventory['source_tree_sha256']
assert server['rustc'] == workload['rustc'] and server['profile'] == workload['profile'] == 'release'
assert inventory['build_environment'] == {}
assert sha(WORKLOAD / 'kv9-batch-workload') == workload['binary_sha256'] == inventory['binary_sha256']
for directory, log, binary in [(SERVER, 'kv9-cargo.jsonl', 'kv9'), (WORKLOAD, 'cargo.jsonl', 'kv9-batch-workload')]:
    records = [json.loads(line) for line in (directory / log).read_text().splitlines()]
    assert records[-1] == {'reason': 'build-finished', 'success': True}
    matches = [r for r in records if r.get('reason') == 'compiler-artifact' and r.get('target', {}).get('name') == binary and r.get('executable')]
    assert len(matches) == 1 and matches[0]['features'] == []
for name, expected in server['sources'].items():
    assert sha(SOURCE / name) == expected, name

OUT.mkdir()
copies = {}
for name in ['kv9', 'kv9-cargo.jsonl', 'kv9-build.log']:
    shutil.copy2(SERVER / name, OUT / name)
    assert sha(SERVER / name) == sha(OUT / name)
    copies[name] = dict(source=str(SERVER / name), sha256=sha(OUT / name))
shutil.copy2(SERVER / 'build.json', OUT / 'server-original-build.json')
copies['server-original-build.json'] = dict(source=str(SERVER / 'build.json'), sha256=sha(OUT / 'server-original-build.json'))
shutil.copytree(WORKLOAD, OUT / 'workload')
for path in sorted(WORKLOAD.rglob('*')):
    if path.is_file():
        destination = OUT / 'workload' / path.relative_to(WORKLOAD)
        assert sha(destination) == sha(path)
        copies[str(destination.relative_to(OUT))] = dict(source=str(path), sha256=sha(destination))
assert 'workload_build_sha256' not in server
aggregate = dict(server, workload_build_sha256=sha(OUT / 'workload/build.json'))
(OUT / 'build.json').write_text(json.dumps(aggregate, sort_keys=True, indent=2) + '\n')
assembly = dict(complete=True, assembled_unix_ns=time.time_ns(), revision=REVISION,
    source_files=590, server_original_manifest_sha256=sha(SERVER / 'build.json'),
    workload_original_manifest_sha256=sha(WORKLOAD / 'build.json'),
    aggregate_manifest_sha256=sha(OUT / 'build.json'), copied_files=copies,
    scope='New aggregate binding of separately built identical-source artifacts. The server binary and original Cargo evidence are exact copies; the timed server manifest is preserved and unchanged. No combined server build is claimed.')
(OUT / 'assembly.json').write_text(json.dumps(assembly, sort_keys=True, indent=2) + '\n')
print(json.dumps({key: value for key, value in assembly.items() if key != 'copied_files'}))
