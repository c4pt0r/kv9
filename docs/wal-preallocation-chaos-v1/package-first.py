"""Select portable original metadata from the accepted full local Chaos archive."""
from pathlib import Path
import gzip, hashlib, io, json, tarfile

P = Path('/mnt/data/kv9-work/wal-preallocation-chaos-preparation-20260915-first')
A = Path('/mnt/data/kv9-work/kv9-chaos-e2e.Bgl9Cx')
OUT = Path(__file__).resolve().parent / 'packet'

def read(p):
    return json.loads(p.read_text())

def sha(p):
    with p.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()

def save(p, value):
    p.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')

def main():
    OUT.mkdir(exist_ok=False)
    full = read(P / 'archive-first/result.json')
    original = read(P / 'archive-first/inventory.json')['members']
    audit = read(P / 'independent/audit.json')
    cleanup = read(P / 'cleanup/summary.json')
    lifetimes = read(P / 'cleanup/all-server-lifetimes-exited.json')
    post = read(P / 'roots/runtime/post-result.json')
    assert all(v['complete'] for v in [full, audit, cleanup, lifetimes, post])
    assert audit['accepted'] and audit['evidence_accepted']
    assert len(audit['windows']) == 21 and cleanup['namespace_absent']
    assert full['all_members_read_back'] and full['all_inputs_rehashed']
    assert sha(Path(full['archive'])) == full['sha256']
    selected = {}; excluded = {}
    archive = OUT / 'metadata.tar.gz'
    with archive.open('wb') as raw, gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz, tarfile.open(fileobj=gz, mode='w|') as dest:
        def add(name, data, origin):
            row = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(), 'origin': origin}
            if name in selected:
                assert {k:selected[name][k] for k in ['bytes','sha256']} == {k:row[k] for k in ['bytes','sha256']}
                return
            info = tarfile.TarInfo(name); info.size = len(data); info.mode = 0o644
            dest.addfile(info, io.BytesIO(data)); selected[name] = row
        with tarfile.open(full['archive'], 'r:gz') as source:
            for member in source:
                if not member.isfile():
                    continue
                name = member.name; path = Path('/' + name)
                reason = None
                if path.name in ['async-write-live-observer.json', 'admission-pressure.jsonl']:
                    reason = 'large complete observer/pressure stream remains in original full archive'
                elif member.size > 256 * 1024 and path.suffix == '.stdout':
                    reason = 'large intermediate command snapshot remains in original full archive'
                elif path.is_relative_to(P) and path.relative_to(P).parts[0] in ['independent-copy-first', 'historical-lineage', 'image-context-first']:
                    reason = 'duplicate data or historical/image closure remains in original full archive'
                elif member.size > 16 * 1024**2:
                    reason = 'large original remains in full archive'
                if reason:
                    excluded[name] = {**original[name], 'reason': reason}; continue
                data = source.extractfile(member).read()
                assert len(data) == original[name]['bytes'] and hashlib.sha256(data).hexdigest() == original[name]['sha256']
                if data[:4] == b'\x7fELF':
                    excluded[name] = {**original[name], 'reason': 'original executable retained locally'}; continue
                assert 'kubeconfig' not in path.name and path.suffix not in ['.key', '.pem']
                add(name, data, 'original pre-cleanup full archive')
        # The full archive precedes deletion; retain the completed cleanup and
        # actual outer execution separately without rewriting that archive.
        for root in [P / 'roots', P / 'cleanup']:
            for path in sorted(root.rglob('*')):
                if not path.is_file() or path.is_symlink() or '__pycache__' in path.parts:
                    continue
                assert path.stat().st_size < 16 * 1024**2
                add(str(path).lstrip('/'), path.read_bytes(), 'original post/runtime record')
    checked = {}
    with tarfile.open(archive, 'r:gz') as tar:
        for member in tar:
            assert member.isfile() and '..' not in Path(member.name).parts and not member.name.startswith('/')
            data = tar.extractfile(member).read()
            checked[member.name] = {'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
    assert checked == {n:{k:r[k] for k in ['bytes','sha256']} for n,r in selected.items()}
    for label, row in audit['histories'].items():
        assert any(v['sha256'] == row['sha256'] and v['bytes'] == row['bytes'] for v in selected.values()), label
    save(OUT / 'metadata-inventory.json', selected)
    save(OUT / 'excluded-original-members.json', excluded)
    for name, path in {'audit.json': P/'independent/audit.json', 'cleanup.json': P/'cleanup/summary.json',
                       'all-lifetimes.json': P/'cleanup/all-server-lifetimes-exited.json', 'full-archive-result.json': P/'archive-first/result.json',
                       'runtime-result.json': P/'roots/runtime/result.json', 'post-result.json': P/'roots/runtime/post-result.json'}.items():
        (OUT / name).write_bytes(path.read_bytes())
    receipts = {'runtime': '47781/38e7ef/0', 'post': '55667/4cbed9/0', 'image': '9857/6737e3/0',
                'auxiliary_build': '81504/2a2c6c/0', 'auxiliary_readback': '15793d/0',
                'fresh_helper_qualification': 'a529f0/0', 'finalizer': '19e1b5/0', 'prebuilt': '2ff992/0'}
    save(OUT / 'actual-tool-receipts.json', receipts)
    result = {'complete': True, 'source_revision': audit['revision'], 'windows': len(audit['windows']),
              'histories': audit['histories'], 'operations': sum(r['invokes'] for r in audit['histories'].values()),
              'outcomes': {name: sum(r['outcomes'].get(name,0) for r in audit['histories'].values()) for name in ['ok','unknown','refused']},
              'fresh_final_drains': audit['fresh_final_drained_replicas'], 'server_lifetimes': len(lifetimes['server_lifetimes']),
              'exited_containers': len(lifetimes['container_states']), 'preserved_namespaces': len(cleanup['historical_namespaces_unchanged']),
              'metadata_archive': {'members': len(selected), 'logical_bytes': sum(r['bytes'] for r in selected.values()), 'bytes': archive.stat().st_size, 'sha256': sha(archive)},
              'excluded_file_count': len(excluded), 'full_local_archive': full, 'default_off': True, 'performance_measured': False,
              'scope': 'Actual local single-node Kind/Chaos Mesh qualification and complete independent histories; no cross-host availability, physical power-loss, C04 pre-upload, performance or industrial completion claim. Portable selection includes the three full histories; large intermediate/observer streams and executables remain in the separately verified full local archive.'}
    save(OUT / 'result.json', result)
    print(json.dumps({k:v for k,v in result.items() if k not in ['histories','full_local_archive']}, indent=2))

if __name__ == '__main__':
    main()
