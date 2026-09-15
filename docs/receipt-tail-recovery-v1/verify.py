#!/usr/bin/env python3
"""Read every retained byte and recompute operation counts without extraction."""
from collections import Counter
import gzip
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile

HERE = Path(__file__).resolve().parent


def require(ok, message):
    if not ok:
        raise ValueError(message)


def main():
    manifest = json.loads((HERE/'manifest.json').read_text())
    for name, pin in manifest['files'].items():
        require(Path(name).name == name, 'unsafe package filename')
        path = HERE/name
        require(path.is_file() and not path.is_symlink(), 'nonordinary package file')
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        require(path.stat().st_size == pin['bytes'] and digest == pin['sha256'], 'package bytes differ: '+name)
    inventory = json.loads((HERE/'input-inventory.json').read_text())
    expected = {row['member']: row for row in inventory}
    require(len(expected) == len(inventory) == manifest['members'], 'duplicate or missing inventory member')
    seen, decoded, data = set(), 0, {}
    with gzip.open(HERE/'original-evidence.tar.gz', 'rb') as stream:
        with tarfile.open(fileobj=stream, mode='r|') as archive:
            for member in archive:
                path = PurePosixPath(member.name)
                require(member.isfile() and not path.is_absolute() and '..' not in path.parts,
                        'unsafe archive member')
                require(member.name in expected and member.name not in seen, 'unexpected or duplicate member')
                pin = expected[member.name]
                require(member.size == pin['bytes'], 'member size differs')
                keep = member.name in ('audit/audit.json', 'release-root/readback-first/result.json',
                                      'release-root/terminal.json', 'recovery-root/terminal.json') or member.name.endswith('/history.jsonl')
                digest, size, chunks = hashlib.sha256(), 0, []
                with archive.extractfile(member) as contents:
                    while block := contents.read(1024*1024):
                        digest.update(block)
                        size += len(block)
                        if keep:
                            chunks.append(block)
                require(size == pin['bytes'] and digest.hexdigest() == pin['sha256'], 'member bytes differ: '+member.name)
                if keep:
                    data[member.name] = b''.join(chunks)
                seen.add(member.name)
                decoded += size
        while stream.read(1024*1024):
            pass
    require(seen == set(expected) and decoded == manifest['decoded_bytes'], 'incomplete archive readback')
    audit = json.loads(data['audit/audit.json'])
    release = json.loads(data['release-root/readback-first/result.json'])
    require(audit['complete'] and audit['accepted'] and release['complete'], 'original qualification is incomplete')
    require(audit['revision'] == release['revision'] == manifest['revision'], 'revision differs')
    require(audit['server_sha256'] == release['server_sha256'] and audit['client_sha256'] == release['workload_sha256'], 'binary binding differs')
    for phase in ('release', 'recovery'):
        terminal = json.loads(data[phase+'-root/terminal.json'])
        require(terminal['complete'] and terminal['exit_code'] == 0 and terminal['session_id'] > 0, 'unsuccessful original terminal')
        require(terminal['result_sha256'] == expected[phase+'-root/source-result.json']['sha256'], 'terminal result binding differs')
    totals = Counter()
    require({case['transport'] for case in audit['cases']} == {'tonic_stream', 'tonic_unary'}, 'transport coverage differs')
    for case in audit['cases']:
        name = 'recovery/batch-'+case['transport'].replace('_','-')+'/history.jsonl'
        raw = data[name]
        require(hashlib.sha256(raw).hexdigest() == case['history_sha256'], 'history binding differs')
        rows = [json.loads(line) for line in raw.splitlines()]
        calls = [row for row in rows if row['type'] == 'invoke']
        returns = [row for row in rows if row['type'] == 'return']
        require(len(calls) == len(returns) == case['operations'], 'incomplete history population')
        require(len({row['id'] for row in calls}) == len(calls) and
                len({row['id'] for row in returns}) == len(returns) and
                {row['id'] for row in calls} == {row['id'] for row in returns}, 'call identity mismatch')
        outcomes = Counter(row['outcome'] for row in returns)
        require(dict(outcomes) == case['outcomes'] and case['fresh_drained_voters'] == 3, 'outcome or drain population differs')
        totals.update(outcomes)
    print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=decoded,
                         operations=sum(totals.values()), outcomes=dict(totals),
                         scope='Byte verification and original history population recomputation; not a new runtime or linearizability test'), sort_keys=True))


if __name__ == '__main__':
    main()
