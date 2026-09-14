#!/usr/bin/env python3
"""Root-only local source qualification under the existing shared BuildCache."""
from pathlib import Path
import argparse,importlib.util,json,os,re,signal,time

root=Path('/tmp/kv9-raft-fnv-interleave-writer-20260914-first')
out=Path('/tmp/kv9-fnv-writer-source-20260914-first')
TARGET='/home/dongxu/kv9/target'

def main():
 assert __debug__
 ap=argparse.ArgumentParser();ap.add_argument('--expected-revision',required=True);args=ap.parse_args()
 assert re.fullmatch('[0-9a-f]{40}',args.expected_revision),'actual frozen source HEAD required'
 out.mkdir(exist_ok=False)
 os.environ.update(CARGO_TARGET_DIR=TARGET,CARGO_BUILD_JOBS='4',CARGO_NET_OFFLINE='true',CARGO_TERM_VERBOSE='true')
 spec=importlib.util.spec_from_file_location('fnv_builder',root/'scripts/build-workload.py')
 builder=importlib.util.module_from_spec(spec);spec.loader.exec_module(builder)
 def save(name,data):(out/name).write_text(json.dumps(data,indent=2)+'\n')
 before=None;result=dict(complete=False,commands=[],source_root=str(root),expected_revision=args.expected_revision)
 commands=[
  ('raft-compile',['cargo','test','--locked','-p','kv9-raft','--no-run','--message-format=json-render-diagnostics'],True),
  ('raft-storage-tests',['cargo','test','--locked','-p','kv9-raft','storage::','--','--test-threads=4'],False),
  ('compile-tests',['cargo','test','--locked','--workspace','--no-run','--message-format=json-render-diagnostics'],True),
  ('workspace-tests',['cargo','test','--locked','--workspace','--','--test-threads=4'],False),
  ('format',['cargo','fmt','--all','--','--check'],False),
  ('experimental-check',['cargo','check','--locked','-p','kv9-server','--all-targets','--features','experimental-leader-lease'],False),
  ('clippy',['cargo','clippy','--locked','--workspace','--all-targets','--','-D','warnings'],False),
 ]
 save('commands.json',commands)
 def terminated(signum,frame):raise SystemExit(128+signum)
 signal.signal(signal.SIGTERM,terminated)
 try:
  before=builder.snapshot();save('source-before.json',before)
  assert before['revision']==args.expected_revision,'source HEAD differs'
  result['source_dirty_at_start']=before['dirty']
  with builder.cache.BuildCache(root,out,False,before) as cache:
   assert str(cache.target)==TARGET,'shared target differs'
   for name,argv,check in commands:
    row=dict(name=name,argv=argv,started_ns=time.time_ns());result['commands'].append(row)
    try:
     with (out/(name+'.stdout')).open('x') as stdout,(out/(name+'.stderr')).open('x') as stderr:
      row['exit_code']=cache.run(argv,stdout=stdout,stderr=stderr,timeout=1200).returncode
     if check:cache.check_artifacts(out/(name+'.stdout'))
     if name in ('raft-storage-tests','workspace-tests'):
      text=(out/(name+'.stdout')).read_text()
      matched=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text)
      assert matched and all(entry[1]=='0' for entry in matched),matched
      row.update(test_results=matched,passed=sum(int(entry[0]) for entry in matched),ignored=sum(int(entry[2]) for entry in matched))
      assert row['passed']>0,'focused test filter matched no tests'
      if name=='workspace-tests':assert row['passed']>=700 and row['ignored']==23,row
     row['source_unchanged']=builder.snapshot()==before
     assert row['source_unchanged'],'source changed during checks'
     row['complete']=True;print(name,'passed',flush=True)
    except BaseException as error:
     row['failure']=repr(error)
     if 'exit_code' not in row and cache.record['commands']:
      last=cache.record['commands'][-1]
      if last['argv']==argv:row['exit_code']=last.get('exit_code')
     raise
    finally:
     row['ended_ns']=time.time_ns();save('result.json',result)
  result['complete']=True
 except BaseException as error:
  result['failure']=repr(error);raise
 finally:
  try:
   after=builder.snapshot();save('source-after.json',after)
   result['source_unchanged']=before is not None and after==before
   if not result['source_unchanged']:
    result['complete']=False;raise RuntimeError('final source snapshot differs')
  except BaseException as error:
   result['complete']=False;result['final_snapshot_failure']=repr(error);raise
  finally:
   result['ended_ns']=time.time_ns();save('result.json',result)

if __name__=='__main__':main()
