#!/usr/bin/env python3
"""Copy only this completed stage's finite reporting metadata; never open payloads."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import stat
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = Path('/tmp/kv9-cross-voter-multicohort-continuation-root-20260915-first')
REPORT = Path('/tmp/kv9-retention-continuation-terminal-publication-20260915-first')
MEMBER_CAP = 1024 * 1024
TOTAL_CAP = 4 * 1024 * 1024


def main():
    sources = [(f'terminal/{name}', REPORT / name) for name in (
        'child-terminal.json', 'completion.json', 'controller-result.json',
        'reporting-correction.json', 'status.json', 'tool-terminal.json')]
    sources.append(('controller/binding.json', ROOT / 'binding.json'))
    for n in range(14, 40):
        names = ['complete.json', 'release.json']
        names += [f'{phase}/{name}.json' for phase in ('stage-verify', 'finish')
                  for name in ('child', 'invocation', 'result', 'terminal')]
        sources += [(f'controller/{n:03d}/{name}', ROOT / f'{n:03d}' / name)
                    for name in names]
    contents, rows = {}, []
    for name, source in sources:
        before = source.lstat()
        assert stat.S_ISREG(before.st_mode) and before.st_size <= MEMBER_CAP, source
        data = source.read_bytes()
        after = source.lstat()
        assert (before.st_ino, before.st_size, before.st_mtime_ns) == (
            after.st_ino, after.st_size, after.st_mtime_ns), source
        json.loads(data)
        contents[name] = data
        rows.append({'member': name, 'source': str(source), 'bytes': len(data),
                     'sha256': hashlib.sha256(data).hexdigest()})
    assert sum(map(len, contents.values())) <= TOTAL_CAP
    completion = json.loads(contents['terminal/completion.json'])
    status = json.loads(contents['terminal/status.json'])
    result = json.loads(contents['terminal/controller-result.json'])
    child = json.loads(contents['terminal/child-terminal.json'])
    terminal = json.loads(contents['terminal/tool-terminal.json'])
    binding = json.loads(contents['controller/binding.json'])
    assert terminal['session_id'] == 85630 and terminal['terminal']['chunk_id'] == 'b96c6f'
    assert terminal['terminal']['exit_code'] == child['exit_code'] == 0 and child['complete']
    assert child['result_sha256'] == hashlib.sha256(contents['terminal/status.json']).hexdigest()
    assert completion['result_sha256'] == hashlib.sha256(contents['terminal/controller-result.json']).hexdigest()
    assert status == completion['status'] and status['state'] == 'COMPLETE'
    assert result['complete'] and result['completed_ordinals'] == status['completed_ordinals'] == 39
    assert status['next_ordinal'] == completion['completed_plan_cohorts'] == 40
    assert binding['cohort_zero']['ordinal'] == 0 and binding['cohort_zero']['state'] == 'COLD'
    assert [r['ordinal'] for r in binding['historical_prefix']] == list(range(1, 14))
    account = result['accounting']
    assert [r['ordinal'] for r in account['rows']] == list(range(40))
    assert all(r['currently_cold'] and r['current_target_allocated_bytes'] == 0 for r in account['rows'])
    assert not account['incomplete_or_restored_cohorts'] and account['cold_cohorts'] == 40
    assert account['actual_available_bytes'] == 85304766464
    assert account['conservative_net_allocated_change_bytes'] == 59995123712
    for n in range(14, 40):
        base = f'controller/{n:03d}/'
        complete = json.loads(contents[base + 'complete.json'])
        release = contents[base + 'release.json']
        assert complete['complete'] and complete['ordinal'] == n
        assert complete['acceptance']['state'] == 'COLD'
        assert complete['release_pin'] == {'bytes': len(release), 'sha256': hashlib.sha256(release).hexdigest()}
        assert complete['verified_sha256'] == json.loads(release)['verified_sha256']
    manifest = {'scope': 'Terminal reporting metadata only; original phase results and payloads remain local authorities.',
                'members': rows, 'original_files': len(rows),
                'original_bytes': sum(r['bytes'] for r in rows),
                'member_cap_bytes': MEMBER_CAP, 'original_total_cap_bytes': TOTAL_CAP}
    encoded = (json.dumps(manifest, indent=2, sort_keys=True) + '\n').encode()
    (HERE / 'members.json').write_bytes(encoded)
    contents['members.json'] = encoded
    with (HERE / 'terminal-metadata.tar.gz').open('xb') as raw:
        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode='w', format=tarfile.USTAR_FORMAT) as archive:
                for name, data in sorted(contents.items()):
                    info = tarfile.TarInfo(name)
                    info.size, info.mode, info.mtime = len(data), 0o644, 0
                    archive.addfile(info, io.BytesIO(data))
    for name in ('completion.json', 'reporting-correction.json', 'tool-terminal.json'):
        (HERE / name).write_bytes(contents['terminal/' + name])
    print(json.dumps({'complete': True, 'original_files': len(rows),
                      'original_bytes': manifest['original_bytes'], 'archive_members': len(contents)}))


if __name__ == '__main__':
    main()
