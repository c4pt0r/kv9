#!/usr/bin/env python3
import hashlib,json,os,pathlib,subprocess,sys,time
if not __debug__:raise RuntimeError('assertions required')
ROOT=pathlib.Path('/tmp/kv9-crc-profile-active-prefix-preparation')
OUT=pathlib.Path(__file__).parent
stage=sys.argv[1]
assert stage in ('profile','analyze')
def sha(p):
 with pathlib.Path(p).open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def save(n,v):
 with(OUT/n).open('x')as f:json.dump(v,f,indent=2,sort_keys=True);f.write('\n')
preflight=json.loads((OUT/'preflight-first.json').read_text())
before=preflight['source_bindings']
assert all(sha(p)==h for p,h in before.items())
assert sorted(os.sched_getaffinity(0))==list(range(6,16))+list(range(22,32))
if stage=='profile':assert not(ROOT/'summary.json').exists()
else:
 assert json.loads((OUT/'profile-terminal-first.json').read_text())['exit_code']==0
 assert not(ROOT/'analysis-summary.json').exists()
 for api in ('point_put','batch_put'):assert not(ROOT/api/'perf-script.txt').exists()
argv=['env','PYTHONOPTIMIZE=0','PYTHONDONTWRITEBYTECODE=1','taskset','-c','6-15,22-31','/usr/bin/python3',str(ROOT/(stage+'.py'))]
save(stage+'-source-before-first.json',before)
with(OUT/(stage+'-first.log')).open('x')as log:
 p=subprocess.Popen(argv,stdout=log,stderr=subprocess.STDOUT)
 identity=None
 try:
  q=pathlib.Path('/proc',str(p.pid));fields=(q/'stat').read_text().rsplit(')',1)[1].split()
  identity={'pid':p.pid,'start_ticks':int(fields[19]),'boot_id':pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(),'cpu_affinity':sorted(os.sched_getaffinity(p.pid)),'executable':os.readlink(q/'exe')}
 finally:
  save(stage+'-invocation-first.json',{'argv':argv,'pid':p.pid,'identity':identity,'supervisor_pid':os.getpid(),'started_unix_ns':time.time_ns(),'preflight_sha256':sha(OUT/'preflight-first.json'),'supervisor_sha256':sha(__file__),'source_before_sha256':sha(OUT/(stage+'-source-before-first.json'))})
  print(json.dumps({'stage':stage,'pid':p.pid,'supervisor_pid':os.getpid(),'log':str(OUT/(stage+'-first.log'))}),flush=True)
 code=p.wait()
after={p:sha(p)for p in before}
save(stage+'-source-after-first.json',after)
terminal={'exit_code':code,'pid':p.pid,'reaped':p.poll()is not None,'absent':not pathlib.Path('/proc',str(p.pid)).exists(),'source_unchanged':before==after,'ended_unix_ns':time.time_ns(),'log_sha256':sha(OUT/(stage+'-first.log')),'invocation_sha256':sha(OUT/(stage+'-invocation-first.json'))}
save(stage+'-terminal-first.json',terminal)
print(json.dumps({'stage':stage,'terminal':terminal}),flush=True)
assert before==after
sys.exit(code)
