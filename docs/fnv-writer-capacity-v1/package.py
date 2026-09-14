import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import tarfile
import time

ROOT = Path(__file__).parent
PREP = ROOT
OUT = ROOT/'package-first'
G = 1024**3
read = lambda p: json.loads(Path(p).read_text())
sha = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()
def available():
    s = os.statvfs('/tmp')
    return s.f_bavail * s.f_frsize
def safe(name):
    p = PurePosixPath(name)
    return bool(name) and not p.is_absolute() and str(p) == name and all(x not in ('.', '..') for x in p.parts) and '\\' not in name
def identity(s):
    return (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns, s.st_mode)

assert __debug__
parser=argparse.ArgumentParser(description='Package completed capacity metadata; no payload or benchmark audit.')
parser.add_argument('--approve-inventory-sha256',required=True)
parser.add_argument('--coordination-terminal',required=True)
parser.add_argument('--coordination-terminal-sha256',required=True)
args=parser.parse_args()
assert sha(ROOT/'inventory.actual-terminal.json')==args.approve_inventory_sha256
coordination=Path(args.coordination_terminal)
assert sha(coordination)==args.coordination_terminal_sha256
terminal=read(coordination)
assert args.coordination_terminal_sha256=='5cac3d1ff546fefd7dcca88b7f5663bb793ef8eddf982f2d55a4e89e59e17b1f'
assert terminal['session_id']==69956 and terminal['exit_code']==0 and terminal['chunk_id']=='d310bf'
assert sha(Path(terminal['result_path']))==terminal['result_sha256']
coordination_result=read(Path(terminal['result_path']))
assert coordination_result['complete'] and len(coordination_result['commands'])==1
assert coordination_result['commands'][0]['exit_code']==0 and coordination_result['commands'][0]['pid_absent']
assert coordination_result['commands'][0]['ended_ns']<=coordination_result['ended_ns']
selection=read(ROOT/'selection.json')
assert selection['complete']
selected=list(selection['members'])
assert len(selected)==selection['selected_files'] and sum(x['bytes'] for x in selected)==selection['selected_bytes']
for name,binding in read(ROOT/'inventory.actual-terminal.json')['files'].items():
    assert safe(name) and Path(name).name==name and sha(ROOT/name)==binding['sha256'] and (ROOT/name).stat().st_size==binding['bytes']
    selected.append(dict(original_path=str(ROOT/name),archive_name='publication-preparation/'+name,**binding))
selected.append(dict(original_path=str(ROOT/'inventory.actual-terminal.json'),archive_name='publication-preparation/inventory.actual-terminal.json',bytes=(ROOT/'inventory.actual-terminal.json').stat().st_size,sha256=sha(ROOT/'inventory.actual-terminal.json')))
assert sum(x['bytes'] for x in selected)<=256*1024**2 and len(selected)<=16000
assert len({x['archive_name'] for x in selected})==len(selected)==len({x['original_path'] for x in selected})
cleanup=Path('/tmp/kv9-fnv-writer-cache-cleanup-root-20260914-first/terminal.json')
ct=read(cleanup)
assert ct['complete'] and ct['exit_code']==0 and ct['session_id']==12594 and sha(ct['result_path'])==ct['result_sha256']
qualification=read(ROOT/'summary.json')
assert qualification['complete'] and qualification['protected_executables_match']==6
assert qualification['cleanup']['benchmark_capacity_released'] is False
started = time.time_ns()
initial = available()
assert initial - G >= 96*G
OUT.mkdir(parents=True, exist_ok=False)
parts, files, samples = [], {}, []
whole = hashlib.sha256()
stored = 0
current = None
current_size = 0
current_hash = None
def guard():
    free = available()
    samples.append(dict(observed_ns=time.time_ns(), available_bytes=free, completed_members=len(files)))
    assert free >= max(96*G, initial-G)
    assert time.time_ns()-started < 1200*10**9
class Parts:
    def write(self, data):
        global current, current_size, current_hash, stored
        total = len(data)
        whole.update(data)
        while data:
            if current is None:
                assert len(parts) < 128
                current = (OUT/f'evidence.tar.gz.{len(parts)+1:03d}').open('xb')
                current_size = 0
                current_hash = hashlib.sha256()
            size = min(len(data), 2*1024**2-current_size)
            piece, data = data[:size], data[size:]
            current.write(piece)
            current_hash.update(piece)
            current_size += size
            stored += size
            assert stored <= 256*1024**2
            if current_size == 2*1024**2:
                self.finish()
        return total
    def flush(self):
        if current:
            current.flush()
    def finish(self):
        global current
        if current:
            current.flush()
            os.fsync(current.fileno())
            parts.append(dict(name=Path(current.name).name, bytes=current_size, sha256=current_hash.hexdigest()))
            current.close()
            current = None
            guard()
secret = re.compile(rb'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|AKIA[0-9A-Z]{16}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)')
class CheckedReader:
    def __init__(self, handle):
        self.handle = handle
        self.digest = hashlib.sha256()
        self.count = 0
        self.tail = b''
    def read(self, size=-1):
        data = self.handle.read(size)
        if self.count == 0:
            assert not data.startswith((b'\x7fELF', b'\x1f\x8b', b'\x28\xb5\x2f\xfd'))
        assert not secret.search(self.tail+data), 'Credential pattern in selected member'
        self.tail = (self.tail+data)[-512:]
        self.digest.update(data)
        self.count += len(data)
        return data
result = dict(complete=False, pid=os.getpid(), argv=list(os.sys.argv), coordination_terminal_path=str(coordination), coordination_terminal_sha256=args.coordination_terminal_sha256, started_ns=started, initial_available_bytes=initial, maximum_added_bytes=G, original_benchmark_guards_unchanged=True)
try:
    sink = Parts()
    with gzip.GzipFile(filename='', fileobj=sink, mode='wb', compresslevel=6, mtime=0) as gz, tarfile.open(fileobj=gz, mode='w|') as archive:
        for row in selected:
            path = Path(row['original_path'])
            name = row['archive_name']
            assert safe(name) and name not in files and 0 <= row['bytes'] <= 64*1024**2
            assert not any(x in path.parts for x in ('target', '.git', '__pycache__', 'retention-objects'))
            assert not path.name.endswith(('.zst', '.tar.gz', '.kubeconfig')) and path.suffix not in ('.pem', '.key', '.wal')
            before = path.lstat()
            assert stat.S_ISREG(before.st_mode) and before.st_size == row['bytes']
            with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), 'rb') as handle:
                assert identity(os.fstat(handle.fileno())) == identity(before)
                reader = CheckedReader(handle)
                entry = tarfile.TarInfo(name)
                entry.size = row['bytes']
                entry.mode = 0o644
                entry.mtime = 0
                archive.addfile(entry, reader)
                assert reader.count == row['bytes'] and not handle.read(1)
                assert reader.digest.hexdigest() == row['sha256'], str(path)
                assert identity(os.fstat(handle.fileno())) == identity(before)
            assert identity(path.lstat()) == identity(before)
            files[name] = dict(bytes=row['bytes'], sha256=row['sha256'], source=str(path))
            if len(files) % 25 == 0:
                guard()
    sink.finish()
    shutil.copy2(ROOT/'verify.py', OUT/'verify.py')
    shutil.copy2(ROOT/'summary.json', OUT/'summary.json')
    shutil.copy2(Path(__file__), OUT/'package.py')
    shutil.copy2(ROOT/'PACKAGE-README.md', OUT/'README.md')
    top = {name: dict(bytes=(OUT/name).stat().st_size, sha256=sha(OUT/name)) for name in ('verify.py','summary.json','package.py','README.md')}
    index = dict(version=1, scope='Completed storage-pilot and cache-cleanup reporting bytes only. WAL, compressed objects, executable payloads and unrelated histories are excluded; hashes in original receipts remain authoritative. No symlinks are created.', stage='fnv-writer-capacity-pilot-and-cache-cleanup-v1', selection_inventory_sha256=sha(PREP/'inventory.actual-terminal.json'), top_level=top, parts=parts, compressed_bytes=stored, compressed_sha256=whole.hexdigest(), decoded_member_count=len(files), decoded_member_bytes=sum(v['bytes'] for v in files.values()), files=files, limits=dict(member_bytes=64*1024**2, decoded_bytes=256*1024**2, compressed_bytes=256*1024**2, part_bytes=2*1024**2, added_bytes=G))
    (OUT/'inventory.json').write_text(json.dumps(index,indent=2)+'\n')
    guard()
    result.update(complete=True, members=len(files), decoded_bytes=index['decoded_member_bytes'], compressed_bytes=stored, compressed_sha256=whole.hexdigest(), parts=len(parts), inventory_sha256=sha(OUT/'inventory.json'), credentials_pattern_scan_passed=True)
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result.update(ended_ns=time.time_ns(), final_available_bytes=available())
    with (ROOT/'result.json').open('x') as f: f.write(json.dumps(result,indent=2)+'\n')
    with (ROOT/'resource-samples.json').open('x') as f: f.write(json.dumps(samples,indent=2)+'\n')
print(json.dumps(result))
