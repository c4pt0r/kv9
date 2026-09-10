"""Bounded raw native-client capture with exact Pod, process, configuration and image identity."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import json,re,subprocess,time
from pathlib import Path
from native_protocol import require,process_identity,prefix
PROBE=r'''set -euo pipefail
test ! -e /tmp/workload.exit
ready=$(cat /tmp/workload/ready.json)
pid=$(printf '%s\n' "$ready" | sed -n 's/.*"process_id": *\([0-9]*\).*/\1/p')
[[ "$pid" =~ ^[1-9][0-9]*$ ]]
printf '@@READY@@\n%s\n' "$ready"
printf '@@BOOT_BEFORE@@\n'; cat /proc/sys/kernel/random/boot_id
printf '@@STAT_BEFORE@@\n'; cat "/proc/$pid/stat"
printf '@@HASH_BEFORE@@\n'; sha256sum /usr/local/bin/kv9-batch-workload "/proc/$pid/exe"
printf '@@CPU@@\n'; sed -n '/Cpus_allowed_list/p' "/proc/$pid/status"
printf '@@CONFIG@@\n'; cat /tmp/workload/config.json
printf '\n@@BUILD@@\n'; cat /tmp/workload/build.json
printf '\n@@PHASE@@\n'; cat /tmp/workload.phase
printf '@@HISTORY@@\n'; head -c "$1" /tmp/workload/history.jsonl
printf '\n@@HASH_AFTER@@\n'; sha256sum "/proc/$pid/exe"
printf '@@STAT_AFTER@@\n'; cat "/proc/$pid/stat"
printf '@@BOOT_AFTER@@\n'; cat /proc/sys/kernel/random/boot_id
printf '@@END@@\n'
test ! -e /tmp/workload.exit
'''
class Commands:
 def __init__(self,out,plan,seconds):
  self.out=Path(out);self.out.mkdir(exist_ok=False);self.plan=plan;self.deadline=time.monotonic()+seconds;self.rows=[]
 def run(self,*argv):
  remaining=self.deadline-time.monotonic();require(remaining>0,'native observation deadline expired')
  cmd=['kubectl','--kubeconfig',self.plan['kubeconfig'],'--request-timeout=8s',*argv];n=len(self.rows);row=dict(command=cmd,started_unix_ns=time.time_ns(),stdout=f'command-{n:04}.stdout',stderr=f'command-{n:04}.stderr',exit_code=None);self.rows.append(row)
  try:
   r=subprocess.run(cmd,capture_output=True,timeout=min(10,remaining));row['exit_code']=r.returncode;stdout=r.stdout;stderr=r.stderr
  except subprocess.TimeoutExpired as e:
   stdout=e.stdout or b'';stderr=e.stderr or b'';row['timeout']=True
  except OSError as e:
   stdout=b'';stderr=str(e).encode();row['os_error']=repr(e)
  finally:row['ended_unix_ns']=time.time_ns()
  (self.out/row['stdout']).write_bytes(stdout);(self.out/row['stderr']).write_bytes(stderr)
  (self.out/'commands.json').write_text(json.dumps(self.rows,indent=2)+'\n')
  require(row['exit_code']==0,'native evidence command failed: '+str(cmd));return row,stdout.decode()
 def capture(self,namespace,config,phase):
  before,pod1=self.run('get','pod','-n',namespace,'kv9-native-batch-client','-o','json')
  capture,text=self.run('exec','-n',namespace,'kv9-native-batch-client','--','/bin/bash','-c',PROBE,'native-probe',str(config['history_bytes']+1))
  after,pod2=self.run('get','pod','-n',namespace,'kv9-native-batch-client','-o','json')
  result,parsed=parse_capture(text,json.loads(pod1),json.loads(pod2),self.plan,config,phase)
  result.update(before_command=self.rows.index(before),capture_command=self.rows.index(capture),after_command=self.rows.index(after),started_unix_ns=before['started_unix_ns'],ended_unix_ns=after['ended_unix_ns'])
  return result,parsed

def parse_capture(text,pod,other,plan,config,phase):
 require(process_identity(pod)==process_identity(other),'native Pod/container lifetime changed')
 for p in (pod,other):
  require(p['metadata']['name']=='kv9-native-batch-client' and p['metadata']['labels']=={'app':'kv9-native-batch-client'},'native client selector identity changed')
  require(p['status']['phase']=='Running' and len(p['status']['containerStatuses'])==1 and all('running' in x['state'] and x['restartCount']==0 for x in p['status']['containerStatuses']),'native client not continuously running')
  require(len(p['spec']['containers'])==1 and p['spec']['containers'][0]['image']==plan['image'],'native client image tag changed')
  require(p['status']['containerStatuses'][0]['imageID'] in plan['accepted_cri_image_ids'],'native client image digest differs')
 parts=re.split(r'^@@([A-Z_]+)@@\n',text,flags=re.M);sections=dict(zip(parts[1::2],parts[2::2]));require(len(sections)==13,'native capture sections malformed')
 ready=json.loads(sections['READY']);pid=ready['process_id'];require(type(pid)is int and pid>0 and ready['version']==1,'native ready identity invalid')
 def stat(key):
  s=sections[key].strip();require(s.split(' ',1)[0]==str(pid),'native stat PID differs');f=s.rsplit(')',1)[1].split();require(f[0]not in ('Z','X','x'),'native process exited');return int(f[19])
 start=stat('STAT_BEFORE');require(start==stat('STAT_AFTER'),'native process start changed')
 boot=sections['BOOT_BEFORE'].strip();require(re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}',boot) and boot==sections['BOOT_AFTER'].strip(),'native process boot changed')
 expected=plan['native_client_build']['binary_sha256']
 def hashes(key):return {s.split(None,1)[1].strip():s.split(None,1)[0] for s in sections[key].splitlines() if s.strip()}
 require(hashes('HASH_BEFORE')=={'/usr/local/bin/kv9-batch-workload':expected,f'/proc/{pid}/exe':expected} and hashes('HASH_AFTER')=={f'/proc/{pid}/exe':expected},'native executing/static binary differs')
 require(json.loads(sections['CONFIG'])==config and json.loads(sections['BUILD'])==plan['native_client_build'],'native configuration/build differs')
 require(sections['PHASE'].strip()==phase,'native phase mirror differs');cpu=sections['CPU'].strip().split(':',1)[1].strip();require(cpu,'native CPU mask missing')
 # The shell adds precisely one delimiter newline. Remove only that delimiter;
 # a genuinely incomplete final JSONL record remains incomplete in prefix().
 history=sections['HISTORY'];require(history.endswith('\n'),'native capture delimiter missing');history=history[:-1];parsed=prefix(history,config)
 return dict(pod_uid=pod['metadata']['uid'],process_pid=pid,process_start_ticks=start,process_boot_id=boot,cpu_allowed=cpu,executable_sha256=expected,phase=phase,last_seq=parsed['last_seq'],partial_tail_bytes=parsed['partial_tail_bytes']),parsed
