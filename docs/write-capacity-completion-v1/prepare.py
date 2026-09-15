"""Select only finite completed capacity metadata; never read original payloads."""
import hashlib,json,os,stat
from pathlib import Path
O=Path(__file__).parent
ROOTS={'initial-cache':Path('/tmp/kv9-write-next-cache-cleanup-20260915-first'),'tranche-040-055':Path('/tmp/kv9-write-release-capacity-tranche-20260915-first'),'post-dev-cache':Path('/tmp/kv9-write-post-dev-cache-cleanup-20260915-first'),'tranche-056-060':Path('/tmp/kv9-write-release-capacity-tranche-20260915-second')}
def read(p):return json.loads(p.read_text())
def pin(p):
 s=p.lstat();assert p.resolve()==p and stat.S_ISREG(s.st_mode) and s.st_size<=4*1024**2
 b=p.read_bytes();after=p.lstat();assert all(getattr(after,k)==getattr(s,k)for k in ['st_dev','st_ino','st_mode','st_uid','st_gid','st_size','st_blocks','st_mtime_ns','st_ctime_ns'])
 return dict(path=str(p),bytes=len(b),sha256=hashlib.sha256(b).hexdigest())
def save(n,x):
 with (O/n).open('x')as f:json.dump(x,f,indent=2,sort_keys=True);f.write('\n')
selected={}
def add(alias,p,root):
 name=alias+'/'+str(p.relative_to(root));assert name not in selected and not any(x in Path(name).parts for x in ['..','.git','target']);selected[name]=pin(p)
for alias in ['initial-cache','post-dev-cache','tranche-056-060']:
 root=ROOTS[alias];name='inventory.final.json'if alias.startswith('tranche')else'inventory.json';inv=read(root/name)
 for n,expected in inv['files'].items():
  p=root/n;actual=pin(p);assert {k:actual[k]for k in ['bytes','sha256']}==expected;add(alias,p,root)
 add(alias,root/name,root)
root=ROOTS['tranche-040-055'];alias='tranche-040-055'
for p in sorted(root.iterdir()):
 if p.is_file() and p.name!='campaign.lock':add(alias,p,root)
for i in range(40,56):
 for n in ['complete.json','release.json']+[f'{phase}/{n}'for phase in ['stage-verify','finish']for n in ['child.json','invocation.json','result.json','stderr','stdout','terminal.json']]:add(alias,root/f'{i:03d}'/n,root)
for n in ['inspect.py','qualify.py','named-targets.py','lineage.json','observations.json','qualification.json','named-targets.json']:add(alias,root/'post-dev-cache-plan'/n,root)
rows=[]
for alias,root in ROOTS.items():
 terminal=read(root/'tool-terminal.json');assert terminal['exit_code']==0 and terminal['complete']
 if 'cache' in alias:
  result=read(root/'result.json');assert result['complete'] and result['lock_released'] and result['failure'] is None
  before=read(root/'source-before.json');after=read(root/'source-after.json');assert before==after
  assert read(root/'protected-before.json')==read(root/'protected-after.json')
  row=dict(stage=alias,kind='generated_cache_retirement',terminal=pin(root/'tool-terminal.json'),actual_session=terminal['session_id'],actual_terminal_receipt=terminal['terminal_receipt'],result=pin(root/'result.json'),available_before_bytes=result['available_before'],available_after_bytes=result['available_after'],observed_global_available_increase_bytes=result['observed_available_increase'],deleted_files=result['deleted_files'],conservative_cache_regular_allocation_bytes=read(root/'selection.json')['regular_allocated_bytes'],source_inventory=pin(root/'source-before.json'),source_heads={name:value['revision']for name,value in before.items()}if'revision'not in before else{'/home/dongxu/kv9':before['revision']},protected_before=pin(root/'protected-before.json'),protected_after=pin(root/'protected-after.json'))
 else:
  result=read(root/'accepted-summary.json');assert result['complete'];prefix=read(Path(result['prefix_integration_receipt']['path']));assert prefix['complete']
  for i in result['new_completed_ordinals']:
   c=read(root/f'{i:03d}'/'complete.json');assert c['complete'] and c['acceptance']['state']=='COLD'
  row=dict(stage=alias,kind='lossless_retention_representation',terminal=pin(root/'tool-terminal.json'),actual_session=terminal['session_id'],actual_terminal_receipt=terminal['terminal_receipt'],result=pin(root/'accepted-summary.json'),prefix_integration=result['prefix_integration_receipt'],completed_ordinals=result['new_completed_ordinals'],next_ordinal=result['next_ordinal'],reason=result['reason'],original_objects=result['original_objects'],whole_readback_logical_bytes=result['logical_bytes_full_readback'],engine_wal_targets=result['engine_wal_targets'],successful_fresh_codec_receipts=result['successful_fresh_codec_receipts'],baseline_available_bytes=result['baseline_available_bytes'],final_accounting_boundary_available_bytes=result['final_boundary_available_bytes'],final_controller_available_bytes=read(root/'status.json')['available_bytes'],post_report_historical_available_bytes=result['fresh_available_bytes'],conservative_net_allocated_recovery_bytes=result['new_conservative_net_allocated_reduction_bytes'],new_reporting_not_in_original_accounting=True)
 rows.append(row)
first=rows[1];delta=first['final_accounting_boundary_available_bytes']-first['baseline_available_bytes']
summary=dict(complete=True,report_kind='Completed capacity evidence only; no current reservation or new runtime acceptance.',stages=rows,cache_observed_global_available_increase_sum_bytes=rows[0]['observed_global_available_increase_bytes']+rows[2]['observed_global_available_increase_bytes'],retention_conservative_net_allocated_recovery_sum_bytes=rows[1]['conservative_net_allocated_recovery_bytes']+rows[3]['conservative_net_allocated_recovery_bytes'],retention_whole_readback_logical_sum_bytes=rows[1]['whole_readback_logical_bytes']+rows[3]['whole_readback_logical_bytes'],concurrent_dev_interval=dict(stage='tranche-040-055',observed_global_available_delta_bytes=delta,conservative_retention_net_bytes=first['conservative_net_allocated_recovery_bytes'],difference_bytes=first['conservative_net_allocated_recovery_bytes']-delta,interpretation='Parent development Cargo/proof work overlapped this interval and rebuilt caches. This difference also includes other host/report allocation and timing boundaries; it is not an independently measured build-only byte count.'),failures=[pin(ROOTS['tranche-040-055']/'freeze-first-failure.json'),pin(ROOTS['tranche-056-060']/'initial-metadata-read-failure.json')],limitations=['No new payload decode, codec lifetime check, source test, benchmark or cleanup. Original completed readbacks and per-cohort complete/release/phase terminals are reused.','Original WAL/data, retained compressed objects, patch objects, ELF binaries, generated build payloads, full original retention transaction directories and large resource samples stay local. Their exact source/selection/result hash bindings are retained.','Final available space was a historical observation, never a reservation. Root has since resumed release builds. Refresh actual capacity under the next gate.','This reporting root and archive add new separately measured metadata allocation; no historical accounting is rewritten.'])
save('summary.json',summary)
assert len(selected)<=1500 and sum(x['bytes']for x in selected.values())<32*1024**2
save('source-selection.json',dict(schema=1,original_members=selected,excluded='All payload data and builds; no traversal outside the four exact roots or their frozen file lists.',member_cap_bytes=4*1024**2,total_decoded_file_cap_bytes=32*1024**2,archive_decoded_cap_bytes=40*1024**2,compressed_cap_bytes=8*1024**2,free_floor_bytes=8*1024**3,maximum_new_report_allocation_bytes=64*1024**2))
print(json.dumps(dict(files=len(selected),bytes=sum(x['bytes']for x in selected.values()),summary=pin(O/'summary.json'),selection=pin(O/'source-selection.json'),concurrent_interval=summary['concurrent_dev_interval'])))
