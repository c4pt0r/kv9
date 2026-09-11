from pathlib import Path
import hashlib,json,shutil,time,sys
sys.path.insert(0,'/tmp/kv9-crc-broad-statistics-preparation')
from core import phase_accounting
root=Path('/tmp/kv9-crc-broad-root-preparation')
freeze=json.loads((root/'launch-freeze-first.json').read_text())
for name,digest in freeze['inputs'].items():
 assert hashlib.sha256(Path(name).read_bytes()).hexdigest()==digest,name
terminal=json.loads((root/'smoke-terminal-first.json').read_text())
assert terminal['exit_code']==0 and terminal['reaped']
p=Path('/tmp/kv9-crc-broad-workloads-smoke-first/matrix.json');matrix=json.loads(p.read_text())
assert matrix['complete'] and matrix['smoke'] and len(matrix['attempts'])==36
rows=[];total_calls=0;retained=0;largest=0
for arm in matrix['attempts']:
 assert arm['complete'] and arm['cleanup_complete']
 d=Path(arm['directory']);r=d/'run/report.json';report=json.loads(r.read_text())
 assert hashlib.sha256(r.read_bytes()).hexdigest()==arm['report_sha256']
 phases={name:phase_accounting(metrics,arm['descriptor']['target']=='kv9') for name,metrics in report['metrics'].items()}
 m=phases['measurement']
 assert m['calls']==m['attempts'] and m['calls']==m['outcomes']['success']
 total_calls+=m['calls']
 wal=d/'fixture/tmpfs-retention.json'
 if wal.exists():
  w=json.loads(wal.read_text());assert w['complete'] and w['children_exited'] and w['scratch_removed']
  size=sum(v['bytes'] for v in w['files'].values());retained+=size;largest=max(largest,size)
 rows.append({'index':arm['index'],'directory':str(d),'phases':phases,'report_sha256':arm['report_sha256']})
free={'tmpfs':shutil.disk_usage('/dev/shm').free,'retention':shutil.disk_usage('/tmp').free}
assert free['tmpfs']>32*1024**3 and free['retention']>96*1024**3
record={'complete':True,'checked_ns':time.time_ns(),'smoke_session':94821,'measured_calls':total_calls,'all_measured_success_single_attempt':True,'matrix_sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'cases':rows,'smoke_retained_data_bytes':retained,'largest_smoke_retained_data_bytes':largest,'free_bytes':free,'space_interpretation':'Observed smoke growth and current free space only; timing uses a different CPU placement, so no linear extrapolation or worst-case capacity proof is asserted.'}
with (root/'smoke-gate-first.json').open('x') as f:json.dump(record,f,indent=2);f.write('\n')
print(json.dumps({k:v for k,v in record.items() if k!='cases'}))
