#!/usr/bin/env python3
"""Read original release bindings only; never builds or runs a fixture."""
import hashlib,importlib.util,json,os,pathlib,time
if not __debug__:raise RuntimeError('assertions required')
OUT=pathlib.Path(__file__).parent
SOURCE=pathlib.Path('/tmp/kv9-wal-crc32-slicing8')
BUILD=pathlib.Path('/tmp/kv9-slicing8-release-first')
RECEIPTS=pathlib.Path('/tmp/kv9-slicing8-root-first')
REV='e5662bbce1443e6907e3d832a7b300881796340c'
def read(q):return json.loads(q.read_text())
def sha(q):
 with q.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(n,v):
 with(OUT/n).open('x')as f:json.dump(v,f,indent=2,sort_keys=True);f.write('\n')
assert sorted(os.sched_getaffinity(0))==list(range(6,16))+list(range(22,32))
source_pin=read(OUT/'SOURCE-PIN.json')
for name,row in source_pin['helpers'].items():assert sha(SOURCE/name)==row['sha256']
assert sha(SOURCE/'crates/engine/src/wal.rs')==source_pin['wal_sha256']
assert sha(OUT/'process-runner.py')==source_pin['process_runner_sha256']
inputs=list(BUILD.rglob('*'))+[RECEIPTS/n for n in ['release-invocation-first.json','release-child-first.json','release-terminal-first.json','release-first.log']]
inputs=[q for q in inputs if q.is_file()]
assert all(not q.is_symlink()for q in inputs)
before={str(q):{'bytes':q.stat().st_size,'sha256':sha(q)}for q in inputs}
manifest=read(BUILD/'build.json');workload=read(BUILD/'workload/build.json');inventory=read(BUILD/'workload/sources.json')
assert manifest['revision']==workload['revision']==inventory['revision']==REV
assert manifest['dirty']is workload['dirty']is inventory['dirty']is False
assert manifest['profile']==workload['profile']=='release'
assert manifest['rustc']==workload['rustc']
assert manifest['sources']==inventory['sources']
tree=hashlib.sha256(json.dumps(manifest['sources'],sort_keys=True,separators=(',',':')).encode()).hexdigest()
assert tree==manifest['source_tree_sha256']==workload['source_tree_sha256']==inventory['source_tree_sha256']
spec=importlib.util.spec_from_file_location('retained_original_source_builder',SOURCE/'scripts/build-workload.py')
builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)
snapshot=builder.snapshot()
assert snapshot=={'revision':REV,'dirty':False,'sources':manifest['sources']}
assert len(snapshot['sources'])==596
assert sha(BUILD/'kv9')==manifest['binaries']['kv9']['sha256']
assert sha(BUILD/'workload/kv9-batch-workload')==workload['binary_sha256']==inventory['binary_sha256']
assert sha(BUILD/'workload/build.json')==manifest['workload_build_sha256']
groups={}
for group,rel,names,command in (
 ('server','kv9-cargo.jsonl',('kv9','kv9_engine','kv9_raft','kv9_server'),manifest['binaries']['kv9']['command']),
 ('workload','workload/cargo.jsonl',('kv9-batch-workload','kv9_engine','kv9_raft','kv9_server'),inventory['command'])):
 assert command[:2]==['cargo','build']and '--locked'in command and '--release'in command
 assert '--bin'in command and command[command.index('--bin')+1]==names[0]
 assert not any(x.startswith(('--features','--all-features','--no-default-features'))for x in command)
 records=[json.loads(line)for line in(BUILD/rel).read_text().splitlines()]
 assert records[-1]=={'reason':'build-finished','success':True}
 selected={}
 for name in names:
  rows=[x for x in records if x.get('reason')=='compiler-artifact'and x.get('target',{}).get('name')==name]
  assert len(rows)==1,name
  row=rows[0];assert row['features']==[] and row['profile']['test']is False and row['profile']['opt_level']=='3'
  assert pathlib.Path(row['manifest_path']).is_relative_to(SOURCE)
  assert row['target']['kind']==(['bin']if name==names[0]else ['lib'])
  selected[name]=row
 assert selected[names[0]]['executable']==str(pathlib.Path('/home/dongxu/kv9/target/release')/names[0])
 groups[group]={'command':command,'cargo_sha256':sha(BUILD/rel),'artifacts':selected}
terminal=read(RECEIPTS/'release-terminal-first.json');child=read(RECEIPTS/'release-child-first.json');inv=read(RECEIPTS/'release-invocation-first.json')
expected=read(OUT/'commands-source-pinned.json')['original-release']
assert inv['argv']==child['argv']==terminal['argv']==expected['argv']
assert inv['cwd']==child['cwd']==terminal['cwd']==str(SOURCE)
assert terminal['exit_code']==0 and terminal['child_reaped']is True
assert terminal['cargo_target']=='/home/dongxu/kv9/target' and terminal['source_revision']==REV
assert terminal['child_pid']==child['child_pid']==1794666 and terminal['supervisor_pid']==inv['supervisor_pid']==1794663
assert all(not pathlib.Path('/proc',str(pid)).exists()for pid in [1794663,1794666])
assert builder.snapshot()==snapshot
assert all(q.stat().st_size==before[str(q)]['bytes']and sha(q)==before[str(q)]['sha256']for q in inputs)
pins=read(OUT/'pending-build-pins-source-pinned.json')
pins.update(build_manifest_sha256=sha(BUILD/'build.json'),server_sha256=sha(BUILD/'kv9'),workload_sha256=sha(BUILD/'workload/kv9-batch-workload'),workload_manifest_sha256=sha(BUILD/'workload/build.json'),source_count=len(snapshot['sources']),source_tree_sha256=tree,cargo_artifact_identities=groups,original_build_session=44220,original_build_terminal={'exit_code':0,'tool_chunk':'fe4df2','path':str(RECEIPTS/'release-terminal-first.json'),'sha256':sha(RECEIPTS/'release-terminal-first.json')})
save('release-pins-final.json',pins)
result={'accepted':True,'scope':'Readback of the original default-feature release only; no process fixture, history audit, Chaos or performance acceptance.','revision':REV,'clean_source_unchanged':True,'source_files':len(snapshot['sources']),'profile':'release','opt_level':'3','server_features':[],'workload_features':[],'custom_build_artifacts_excluded_from_library_bin_profile_checks':True,'source_tree_sha256':tree,'server_sha256':pins['server_sha256'],'workload_sha256':pins['workload_sha256'],'build_manifest_sha256':pins['build_manifest_sha256'],'workload_manifest_sha256':pins['workload_manifest_sha256'],'original_build_session':44220,'original_build_exit_code':0,'root_terminal_tool_chunk':'fe4df2','builder_and_supervisor_absent':True,'source_helpers_match':9,'process_runner_sha256':sha(OUT/'process-runner.py'),'input_bindings':before,'original_input_bytes_unchanged':True,'provisional_records_unchanged':True,'release_pins_sha256':sha(OUT/'release-pins-final.json'),'executed_readback_script_sha256':sha(pathlib.Path(__file__)),'completed_unix_ns':time.time_ns()}
save('release-readback-first.json',result)
print(json.dumps({k:v for k,v in result.items()if k!='input_bindings'},indent=2));print('READBACK_SHA256',sha(OUT/'release-readback-first.json'))
