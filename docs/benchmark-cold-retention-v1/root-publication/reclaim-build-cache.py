import hashlib,importlib.util,json,os,subprocess,time
from pathlib import Path
root=Path('/tmp/kv9-release-thin-lto-root-first');out=root/'release-cache-reclaim-first';out.mkdir()
source=Path('/tmp/kv9-release-thin-lto')
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip()=='02d0c01024b65a84b220c6948ff2224bfa7900bc'
assert not subprocess.check_output(['git','status','--porcelain','--untracked-files=all'],cwd=source,text=True).strip()
pins={
'/tmp/kv9-release-thin-lto-release-first/kv9':'dd028cb2f61633dda133b05d814a6d173a79a83145d8ced2f81dc0cf8e0f33bc',
'/tmp/kv9-release-thin-lto-release-first/workload/kv9-batch-workload':'bd18336b5da137adbe008bc69db15dbf7769b73a74a876b9461c493665086d83',
'/tmp/kv9-wal-crc32-release-first/kv9':'b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13',
'/tmp/kv9-point-write-v3-release-first/native/kv9-batch-benchmark':'8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957'}
def verify():
 for name,expected in pins.items():
  with Path(name).open('rb') as f:assert hashlib.file_digest(f,'sha256').hexdigest()==expected,name
verify()
def free():
 v=os.statvfs('/tmp');return v.f_bavail*v.f_frsize
record={'complete':False,'started_ns':time.time_ns(),'before_available_bytes':free(),'retained_artifact_pins':pins,'scope':'Invalidate reproducible first-party release target cache only under existing BuildCache lock. No build, test, original binary/WAL/history deletion or candidate requalification.'}
try:
 spec=importlib.util.spec_from_file_location('build_cache',source/'scripts/build_cache.py');mod=importlib.util.module_from_spec(spec);spec.loader.exec_module(mod)
 with mod.BuildCache(source,out,True,{'revision':'02d0c01024b65a84b220c6948ff2224bfa7900bc','operation':'cleanup-only'}) as cache:
  cache.record['scope']=record['scope'];cache.save()
 verify();record['after_available_bytes']=free();record['complete']=True
finally:
 record['ended_ns']=time.time_ns();(out/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(record,indent=2));print((out/'cache-clean.stderr').read_text())
