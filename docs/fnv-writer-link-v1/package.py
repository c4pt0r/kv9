#!/usr/bin/env python3
"""Bounded Link11 publication only; no original runtime/source/payload re-audit."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import sys
import tarfile
import time

HERE = Path(__file__).resolve().parent
ART = Path('/tmp/kv9-native-link-acceptance-fnv12f-20260914-first')
ROOT = Path('/tmp/kv9-fnv-writer-link11-root-20260914-first')
PREP = Path('/tmp/kv9-fnv-writer-link11-preparation-20260914-first')
AUDIT = PREP/'acceptance-first/audit.json'
REV = '12f44d35590ede5f89337fe731dd950162865154'
M = 1024**2
G = 1024**3
MEMBER_CAP, TOTAL_CAP, PART_CAP = 64*M, 512*M, 2*M
MAX_FILES, MAX_PARTS = 10000, 256
SUFFIXES = {'.json', '.jsonl', '.ndjson', '.md', '.py', '.sh', '.txt',
            '.log', '.stdout', '.stderr', '.yaml', '.yml', '.toml', '.diff', '.patch'}
DENIED = {'.git', 'target', 'data', 'retention-objects', '__pycache__',
          'source', 'sources', 'source-tree', 'source-worktree', 'objects'}
SECRET = re.compile(rb'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|'
                    rb'AKIA[0-9A-Z]{16}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)')


def safe(name):
    p = PurePosixPath(name)
    return bool(name) and not p.is_absolute() and str(p) == name and '\\' not in name and all(x not in ('.', '..') for x in p.parts)


def canonical(path):
    path = Path(path)
    assert path.is_absolute() and '..' not in path.parts
    assert not any(p.is_symlink() for p in (path, *path.parents)), str(path)
    return path


def save(path, value):
    data = (json.dumps(value, sort_keys=True, indent=2) + '\n').encode()
    assert len(data) <= MEMBER_CAP
    with path.open('xb') as stream:
        stream.write(data)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def available(path):
    s = os.statvfs(path)
    return s.f_bavail*s.f_frsize


def identity(s):
    return (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns, s.st_mode)


def metadata(path):
    path = canonical(path)
    assert path.is_file() and path.stat().st_size <= MEMBER_CAP
    return json.loads(path.read_text())


def terminal(path, expected, session, chunk, phase):
    value = metadata(path)
    assert sha(path) == expected and value['exit_code'] == 0
    assert value['session_id'] == session and value['chunk_id'] == chunk
    result = metadata(value['result_path'])
    assert sha(value['result_path']) == value['result_sha256']
    assert result['complete'] is True and result['phase'] == phase
    assert all(x['exit_code'] == 0 and x['pid_absent'] for x in result['commands'])
    return value


class Parts:
    def __init__(self, out, guard):
        self.out, self.guard = out, guard
        self.stream = None
        self.parts = []
        self.total = 0
        self.whole = hashlib.sha256()

    def write(self, data):
        length = len(data)
        assert self.total + length <= TOTAL_CAP
        self.whole.update(data)
        while data:
            if self.stream is None:
                assert len(self.parts) < MAX_PARTS
                self.stream = (self.out/f'evidence.tar.gz.{len(self.parts)+1:03d}').open('xb')
                self.size, self.digest = 0, hashlib.sha256()
            n = min(PART_CAP-self.size, len(data))
            piece, data = data[:n], data[n:]
            self.stream.write(piece)
            self.digest.update(piece)
            self.size += n
            self.total += n
            if self.size == PART_CAP:
                self.finish()
        self.guard()
        return length

    def flush(self):
        if self.stream:
            self.stream.flush()

    def finish(self):
        if self.stream:
            self.stream.flush()
            os.fsync(self.stream.fileno())
            self.parts.append(dict(name=Path(self.stream.name).name, bytes=self.size, sha256=self.digest.hexdigest()))
            self.stream.close()
            self.stream = None


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--receipts', type=Path, required=True)
    ap.add_argument('--preparation-inventory-sha256', required=True)
    args = ap.parse_args()
    assert __debug__ and sys.dont_write_bytecode
    assert set(os.sched_getaffinity(0)) == set(range(6,16)) | set(range(22,32))
    assert sha(HERE/'inventory.preparation.json') == args.preparation_inventory_sha256
    for row in metadata(HERE/'inventory.preparation.json')['files']:
        assert sha(HERE/row['path']) == row['sha256']
    out, receipts = canonical(args.output), canonical(args.receipts)
    assert out != receipts and not out.is_relative_to(receipts) and not receipts.is_relative_to(out)
    assert not out.exists() or not any(out.iterdir()), 'publication output must be empty'
    assert not receipts.exists(), 'fresh receipt directory required'
    for original in (ART, ROOT, PREP):
        assert not out.is_relative_to(original) and not receipts.is_relative_to(original)
    runtime = terminal(ROOT/'runtime-terminal.json', '5cac3d1ff546fefd7dcca88b7f5663bb793ef8eddf982f2d55a4e89e59e17b1f',69956,'d310bf','runtime')
    post = terminal(ROOT/'post-terminal.json', '42b4666e48e5a0abb1ba2d1ab1308f00e05c6d0129a6cb090e87176a26b71ef3',38527,'a8923f','post')
    audit, summary = metadata(AUDIT), metadata(ART/'summary.json')
    assert audit['accepted'] and audit['revision'] == REV and audit['windows'] == 11
    assert summary['accepted'] and summary['source_revision'] == REV and summary['windows'] == 11
    assert summary['cleanup']['complete'] and summary['cleanup']['namespace_absent']
    assert metadata(PREP/'acceptance-first/leaf.json')['netem_leaf_drop_delta'] == 203
    assert metadata(PREP/'acceptance-first/source-final.json')['accepted']
    assert metadata(HERE/'protected-uids-readback.json')['complete']
    known, selected, omitted = {}, {}, []

    def remember(path, binding):
        path = str(Path(path))
        assert isinstance(binding['bytes'], int) and binding['bytes'] >= 0
        assert re.fullmatch('[0-9a-f]{64}', binding['sha256'])
        b = {k: binding[k] for k in ('bytes', 'sha256')}
        assert path not in known or known[path] == b, 'conflicting authority: '+path
        known[path] = b

    def add(path):
        path = canonical(path)
        if str(path) in selected:
            return
        assert path.is_file() and (path.suffix in SUFFIXES or path.name in ('status','workload.exit','workload.phase','workload.stop')) and not DENIED.intersection(path.parts), str(path)
        s = path.stat()
        assert s.st_size <= MEMBER_CAP and stat.S_ISREG(s.st_mode), str(path)
        name = 'originals/'+str(path).lstrip('/')
        assert safe(name)
        selected[str(path)] = dict(source=str(path), member=name, bytes=s.st_size, identity=identity(s))
        assert len(selected) <= MAX_FILES and sum(v['bytes'] for v in selected.values()) <= TOTAL_CAP

    def walk(root):
        canonical(root)
        assert root.is_dir()
        for base, dirs, files in os.walk(root, followlinks=False):
            dirs[:] = sorted(d for d in dirs if d not in DENIED | {'retained'} and not (Path(base)/d).is_symlink())
            for name in sorted(files):
                path = Path(base)/name
                if path.suffix in SUFFIXES and not path.is_symlink():
                    add(path)


    # Finite owned roots; no historical/capacity store traversal or source copies.
    # Retained tree is excluded by walk and selected from its exact inventory below.
    for root in (ART, ROOT, PREP):
        walk(root)
    for row in metadata(HERE/'inventory.preparation.json')['files']:
        add(HERE/row['path'])
    add(HERE/'inventory.preparation.json')
    data_index = metadata(ART/'retained-inventory.json')
    for name, binding in data_index.items():
        source = ART/'retained'/name
        # Complete native history/report/config plus per-voter status/metrics.
        if name.startswith('data-native/') and (source.suffix in SUFFIXES or source.name in ('workload.exit','workload.phase','workload.stop')) or name.split('/')[-1] in ('status','metrics.json'):
            add(source);remember(source,binding)
        else:
            omitted.append(dict(kind='original_retained_data',source=str(source),**binding,inspected_payload=False))
    source_binding = metadata(PREP/'source-build-binding.json')
    for path,digest in source_binding['files'].items():
        q=Path(path)
        if q.suffix in SUFFIXES:
            add(q);remember(q,dict(bytes=q.stat().st_size,sha256=digest))
        else:
            omitted.append(dict(kind='original_executable',source=path,bytes=q.stat().st_size,sha256=digest,inspected_payload=False))
    for path,digest in metadata(PREP/'image-binding.json')['evidence_files'].items():
        q=Path(path)
        if q.suffix in SUFFIXES:
            add(q);remember(q,dict(bytes=q.stat().st_size,sha256=digest))
    for name,digest in metadata(PREP/'frozen-inputs.json').items():
        q=PREP/name
        if str(q) in selected:remember(q,dict(bytes=q.stat().st_size,sha256=digest))
        else:omitted.append(dict(kind='original_observer_tool_archive',source=str(q),bytes=q.stat().st_size,sha256=digest,inspected_payload=False))
    # Bind the original archive without opening/extracting its members again.
    original_tar=ART/'owned-data.tar';before=original_tar.stat();original_tar_sha=sha(original_tar)
    assert identity(original_tar.stat()) == identity(before)
    omitted.append(dict(kind='complete_original_data_tar',source=str(original_tar),bytes=before.st_size,sha256=original_tar_sha,
                        original_regular_members=len(data_index),original_regular_member_bytes=sum(v['bytes']for v in data_index.values()),
                        archive_payload_reaudited=False,original_runner_full_member_readback=True))
    for path,row in selected.items():
        if path in known:
            assert row['bytes']==known[path]['bytes'];row['expected_sha256']=known[path]['sha256']
    out.mkdir(parents=True, exist_ok=True)
    receipts.mkdir()
    started = time.monotonic()
    initial = available(out)
    assert initial-G >= 96*G, '1 GiB publication reservation above 96 GiB floor'
    observations, last_sample = [], [0.0]

    def guard(force=False):
        now = time.monotonic()
        assert now-started < 1200, 'publication deadline'
        if force or now-last_sample[0] >= 5:
            free = available(out)
            assert free >= max(96*G, initial-G), 'publication capacity'
            observations.append(dict(monotonic_ns=time.monotonic_ns(), available_bytes=free))
            last_sample[0] = now

    result = dict(complete=False, argv=sys.argv, pid=os.getpid(), scope='Portable Link11 metadata/history/control bytes; original full data archive and executables remain local. Independent archive byte verification is not a new runtime or whole-system acceptance.')
    save(receipts/'invocation.json', result)
    sink, inventory = Parts(out, guard), {}
    try:
        with gzip.GzipFile(filename='', fileobj=sink, mode='wb', compresslevel=6, mtime=0) as gz, tarfile.open(fileobj=gz, mode='w|', format=tarfile.PAX_FORMAT) as archive:
            for row in sorted(selected.values(), key=lambda x:x['member']):
                path = canonical(row['source'])
                fd = os.open(path, os.O_RDONLY|os.O_NOFOLLOW)
                with os.fdopen(fd, 'rb') as source:
                    before = os.fstat(source.fileno())
                    assert identity(before) == tuple(row['identity']), str(path)
                    digest, count, tail = hashlib.sha256(), [0], [b'']

                    class Reader:
                        def read(self, amount):
                            data = source.read(min(amount, M))
                            if count[0] == 0:
                                assert not data.startswith(b'\x7fELF'), str(path)
                            assert not SECRET.search(tail[0]+data), str(path)
                            tail[0] = data[-256:]
                            count[0] += len(data)
                            digest.update(data)
                            guard()
                            return data

                    info = tarfile.TarInfo(row['member'])
                    info.size, info.mode, info.mtime = row['bytes'], 0o644, 0
                    archive.addfile(info, Reader())
                    assert count[0] == row['bytes'] and not source.read(1)
                    assert identity(os.fstat(source.fileno())) == identity(before) == identity(path.stat()), str(path)
                    actual = digest.hexdigest()
                    assert 'expected_sha256' not in row or row['expected_sha256'] == actual, str(path)
                    inventory[row['member']] = dict(bytes=row['bytes'], sha256=actual, source=str(path))
        sink.finish()
        top = {}
        for path in (HERE/'verify.py', HERE/'package.py', HERE/'ACCEPTANCE.json', HERE/'README.md'):
            binding = inventory[selected[str(path)]['member']]
            data = path.read_bytes()
            assert len(data) == binding['bytes'] and hashlib.sha256(data).hexdigest() == binding['sha256']
            name = path.name
            with (out/name).open('xb') as stream:
                stream.write(data)
            top[name] = {k:binding[k] for k in ('bytes', 'sha256')}
        save(out/'omitted-payload-references.json', omitted)
        top['omitted-payload-references.json'] = dict(bytes=(out/'omitted-payload-references.json').stat().st_size, sha256=sha(out/'omitted-payload-references.json'))
        index = dict(version=1, files=inventory, top_level=top, parts=sink.parts, compressed_bytes=sink.total,
                     compressed_sha256=sink.whole.hexdigest(), decoded_member_count=len(inventory),
                     decoded_member_bytes=sum(x['bytes'] for x in inventory.values()), audit_sha256=sha(AUDIT),
                     runtime_terminal_sha256=sha(ROOT/'runtime-terminal.json'), post_terminal_sha256=sha(ROOT/'post-terminal.json'), candidate_revision=REV,
                     limits=dict(member_bytes=MEMBER_CAP, decoded_member_bytes=TOTAL_CAP, compressed_bytes=TOTAL_CAP, parts=MAX_PARTS), scope=result['scope'])
        save(out/'inventory.json', index)
        guard(True)
        result.update(complete=True, files=len(inventory), decoded_bytes=index['decoded_member_bytes'], compressed_bytes=sink.total,
                      parts=len(sink.parts), inventory_sha256=sha(out/'inventory.json'))
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        if sink.stream:
            sink.stream.close()
        result['elapsed_seconds'] = time.monotonic()-started
        save(receipts/'result.json', result)
        save(receipts/'resource-samples.json', observations)
    print(json.dumps(result))


if __name__ == '__main__':
    main()
