#!/usr/bin/env python3
"""Finite receipt-screen metadata exporter. Never opens WAL objects or executables."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import stat
import tarfile

HERE = Path(__file__).resolve().parent
PREP = Path('/tmp/kv9-receipt-tail-performance-v3-preparation-20260915-first')
READY = Path('/tmp/kv9-receipt-tail-performance-v3-readiness-preparation-20260915-first')
EXEC = Path('/tmp/kv9-receipt-tail-performance-v3-execution-evidence-20260915-first')
TIMING = Path('/tmp/kv9-receipt-tail-performance-v3-timing-20260915-first')
SMOKE = Path('/tmp/kv9-receipt-tail-performance-v3-smoke-20260915-first')
SUMMARY_INPUTS = Path('/tmp/kv9-receipt-tail-performance-summary-20260915-first/input-hashes.json')
MEMBER_CAP = 8 * 1024 * 1024
TOTAL_CAP = 64 * 1024 * 1024


def main():
    contents, rows = {}, {}
    def add(path, expected=None):
        path = Path(path)
        assert path.is_absolute() and '..' not in path.parts
        assert path.suffix in ('.json', '.py', '.md', '.diff') and not path.name.startswith('issue9')
        name = str(path).lstrip('/')
        if name not in contents:
            a = path.lstat()
            assert stat.S_ISREG(a.st_mode) and a.st_size <= MEMBER_CAP, path
            data = path.read_bytes()
            b = path.lstat()
            assert (a.st_dev, a.st_ino, a.st_size, a.st_mtime_ns) == (b.st_dev, b.st_ino, b.st_size, b.st_mtime_ns)
            contents[name] = data
            rows[name] = {'member': name, 'source': str(path), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
        if expected is not None:
            assert all(rows[name][k] == expected[k] for k in ('bytes', 'sha256')), path
        assert sum(r['bytes'] for r in rows.values()) <= TOTAL_CAP
        return json.loads(contents[name]) if path.suffix == '.json' else None
    for root, pin in ((PREP, '2406a1582300d6357429e10c53f471440c5a078e170fa7492249caaa19f22552'),
                      (READY, '45c266f8821a466a219ee3ccdbc17992e5c50908da7bed4a6c5e5c562a9e81ca')):
        inventory = add(root / 'inventory.json')
        assert rows[str(root / 'inventory.json').lstrip('/')]['sha256'] == pin
        for row in inventory['files']:
            assert not Path(row['path']).is_absolute() and '..' not in Path(row['path']).parts
            add(root / row['path'], row)
    for name in ('audit-acceptance.json', 'audit-invocation.json', 'audit-terminal.json',
                 'initial-preflight-terminal.json', 'pre-timing.json', 'preflight.json',
                 'smoke-invocation.json', 'smoke-launch.json', 'smoke-readback-invocation.json',
                 'smoke-readback-terminal.json', 'smoke-terminal.json', 'timing-invocation.json',
                 'timing-launch.json', 'timing-terminal.json'):
        add(EXEC / name)
    acceptance = add(EXEC / 'audit-acceptance.json')
    assert acceptance['complete'] and acceptance['calls'] == 7296894 and acceptance['input_items'] == 64701927
    assert acceptance['audit_sha256'] == '9d87ba394620f4bdca9c8ce5a86efb7f0615b88bc3123ff5f5ffc1c3b59b7a36'
    for name, row in acceptance['artifact_pins'].items():
        assert name in ('audit.json', 'input-inventory.json', 'retention-decoder-receipts.json',
                        'logical-original-inventory.json', 'combined-physical-retention.json')
        add(PREP / 'results-first' / name, row)
    audit_inputs = add(PREP / 'results-first/input-inventory.json')
    for name in ('after.json', 'before.json', 'invocation.json', 'isolation.json',
                 'kv9-chaos-ci-p0-20260908-control-plane-isolated.json',
                 'kv9-chaos-control-plane-isolated.json', 'kv9-minio-dev-isolated.json', 'summary.json'):
        path = TIMING / name
        add(path, audit_inputs[str(path)])
    for root in (SMOKE, TIMING / 'cohorts'):
        path = root / 'matrix.json'
        add(path, audit_inputs[str(path)])
        for name in ('plan.json', 'host.json'):
            add(root / name)
    add(READY / 'role-bindings-first.json')
    add(READY / 'smoke-readback/result.json')
    roles = add(READY / 'role-inputs.json')
    for role in roles['roles'].values():
        add(role['manifest']['path'], role['manifest'])
    summary_inputs = add(SUMMARY_INPUTS)
    assert len(summary_inputs) == 60
    reports = [p for p in summary_inputs if p.endswith('/run/report.json')]
    assert len(reports) == 16 and all(str(TIMING / 'cohorts') in p for p in reports)
    for path, row in summary_inputs.items():
        add(path, row)
    manifest = {'scope': 'Exact finite metadata selection. Original payload objects, ELFs and source trees remain locally retained and hash-bound; no payload replay.',
                'original_files': len(rows), 'original_bytes': sum(r['bytes'] for r in rows.values()),
                'member_cap_bytes': MEMBER_CAP, 'total_cap_bytes': TOTAL_CAP,
                'members': [rows[k] for k in sorted(rows)]}
    blob = (json.dumps(manifest, indent=2, sort_keys=True) + '\n').encode()
    (HERE / 'members.json').write_bytes(blob)
    contents['members.json'] = blob
    with (HERE / 'evidence.tar.gz').open('xb') as raw:
        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, compresslevel=6, mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode='w', format=tarfile.USTAR_FORMAT) as tf:
                for name, data in sorted(contents.items()):
                    member = tarfile.TarInfo(name)
                    member.size, member.mode, member.mtime = len(data), 0o644, 0
                    tf.addfile(member, io.BytesIO(data))
    for name in ('audit-acceptance.json', 'initial-preflight-terminal.json'):
        (HERE / name).write_bytes(contents[str(EXEC / name).lstrip('/')])
    result = {'complete': True, 'original_files': len(rows), 'original_bytes': manifest['original_bytes'],
              'archive_members': len(contents), 'summary_inputs_included': len(summary_inputs)}
    (HERE / 'package-result.json').write_text(json.dumps(result, indent=2, sort_keys=True) + '\n')
    print(json.dumps(result))


if __name__ == '__main__':
    try:
        main()
    except BaseException as exc:
        (HERE / 'package-failure.json').write_text(json.dumps({'complete': False, 'error': repr(exc)}) + '\n')
        raise
