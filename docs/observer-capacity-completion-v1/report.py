"""Finite061..063 completed receipt reporting; no payload or acceptance replay."""
from pathlib import Path
import gzip,hashlib,io,json,os,stat,tarfile,time
P=Path(__file__).resolve().parent
ROOT=Path('/tmp/kv9-upper-bound-observer-capacity-20260915-first')
PREP=Path('/tmp/kv9-cross-voter-multicohort-migration-preparation-20260915-first')
EXEC=Path('/tmp/kv9-cross-voter-multicohort-execution-20260915-first')
MIB=1024**2; chosen={}
def data(p):
 p=Path(p);s=p.lstat();assert stat.S_ISREG(s.st_mode)and s.st_size<=4*MIB
 return p.read_bytes()
def pin(p):
 b=data(p);return dict(path=str(p),bytes=len(b),sha256=hashlib.sha256(b).hexdigest())
def check(p,r):
 actual=pin(p);assert all(actual[k]==r[k]for k in ('bytes','sha256')),str(p)
def read(p):return json.loads(data(p))
def save(n,x):
 with (P/n).open('x')as f:json.dump(x,f,indent=2,sort_keys=True);f.write('\n')
def add(p):
 p=Path(p);assert p.is_absolute()and not p.is_symlink();chosen[str(p)]=pin(p)
def metadata_tree(root):
 for p in sorted(root.rglob('*')):
  assert not p.is_symlink()
  if p.is_file():add(p)
def fresh_children(rows):
 assert all(r['complete']is True and r['exit_code']==0 and r.get('reaped',r.get('child_reaped',r.get('absent')))is True for r in rows)
 keys=[(r['boot_id'],r['pid'],r['start_ticks'])for r in rows];assert len(set(keys))==len(keys)
 return len(rows)

status=read(ROOT/'status.json');tool=read(ROOT/'tool-terminal.json');child=read(ROOT/'launch-child-terminal.json');commands=read(ROOT/'ROOT-COMMANDS.json')
assert status['state']=='COMPLETE'and status['next_ordinal']==64 and status['completed_ordinals']==[61,62,63]and status['reason']=='actual_observer_envelope_plus_margin'
assert tool['complete']and tool['session_id']==46334 and tool['terminal_receipt']=='062ac3'and tool['exit_code']==0
assert tool['result_path']==str(ROOT/'status.json')and tool['result_sha256']==pin(ROOT/'status.json')['sha256']
assert child['complete']and child['exit_code']==0 and child['argv']==commands['launch']['argv']and child['result_sha256']==tool['result_sha256']
inputs=read(ROOT/'inputs.json');assert pin(ROOT/'inputs.json')['sha256']==commands['launch']['inputs_sha256']
for name,r in inputs['own_sources'].items():check(ROOT/name,r)
for name,r in read(ROOT/'inventory.final.json')['files'].items():check(ROOT/name,r)
metadata_tree(ROOT)
results=list(ROOT.glob('result-*.json'));assert len(results)==1;final=read(results[0]);base=read(ROOT/'baseline-accounting.json')
assert final['complete']and final['historical_completed_ordinals']==list(range(61))and final['new_completed_ordinals']==[61,62,63]and final['next_ordinal']==64 and final['baseline_accounting']==base
last=final['current_accounting'];assert last['cold_cohorts']==64 and not last['incomplete_or_restored_cohorts']
assert final['reason']==status['reason']and final['actual_observer_envelope_plus_margin_met']is True
group=read(PREP/'group-plan.json');code=read(PREP/'code-pins.json')
for n in ['group-plan.json','code-pins.json','run.py','common.py','support.py','stage.py','verify.py','transition.py','readback.py','policy.json']:
 add(PREP/n)
 if n in code:check(PREP/n,code[n])
controller=Path('/tmp/kv9-cross-voter-multicohort-continuation-preparation-20260915-first/campaign.py');add(controller);check(controller,inputs['files'][str(controller)])
repair=Path('/tmp/kv9-retention-013-reconciliation-preparation-20260915-first')
for n in ['inventory.json','authority.py','readback_repaired.py','transition_repaired.py','reconcile.py','ROOT-COMMANDS.json']:add(repair/n)
cohorts=[];all_children=[];omitted=[]
for ordinal in (61,62,63):
 selected=group['rows'][ordinal];sf=Path(selected['selection']['path']);s=read(sf);check(sf,selected['selection']);add(sf);tx=Path(s['root']);accept=read(ROOT/f'{ordinal:03d}/complete.json');a=accept['acceptance']
 assert a['state']=='COLD'and a['complete']and accept['complete']and a['selection_sha256']==pin(sf)['sha256']
 check(ROOT/f'{ordinal:03d}/release.json',accept['release_pin']);release=read(ROOT/f'{ordinal:03d}/release.json');assert release['verified_sha256']==a['verified_sha256']
 bound=next(r for r in final['completed']if r['ordinal']==ordinal);check(ROOT/f'{ordinal:03d}/complete.json',bound['complete_record']);assert bound['acceptance']==a
 phase_files={'stage':'staged.json','verify':'verification-first/result.json','retire-1':'retire-1/result.json','restore-1':'restore-1/result.json','readback-1-repaired':'readback-1-repaired-first/result.json','retire-2':'retire-2/result.json'}
 phases={}
 for phase,n in phase_files.items():
  f=tx/n;add(f);r=read(f);assert r['complete'];phases[phase]=r
  receipt=read(EXEC/f'{ordinal:03d}/{phase}-complete.json');assert receipt['complete']and receipt['exit_code']==0 and receipt['result_path']==str(f)and receipt['result_sha256']==pin(f)['sha256']
 metadata_tree(EXEC/f'{ordinal:03d}')
 for directory in ['retire-1','retire-2','restore-1','work']:
  for f in sorted((tx/directory).rglob('*.json')):add(f)
 add(tx/'toolchain.json')
 stage,verified,restore,rb,cold=[phases[k]for k in ['stage','verify','restore-1','readback-1-repaired','retire-2']]
 assert pin(tx/'verification-first/result.json')['sha256']==a['verified_sha256']==accept['verified_sha256']
 assert verified['staged_sha256']==pin(tx/'staged.json')['sha256']
 assert pin(tx/'restore-1/result.json')['sha256']==a['restore_sha256']==rb['restore_result_sha256']
 assert pin(tx/'readback-1-repaired-first/result.json')['sha256']==a['readback_sha256']==cold['reconciliation']['corrected_readback']['sha256']
 assert pin(tx/'retire-2/result.json')['sha256']==a['cold_sha256']and cold['state']=='COLD'and cold['cycle']==2
 assert len(cold['rows'])==len(restore['rows'])==s['target_count']and all(x['absent']for x in cold['rows'])and all(x['original_bytes_exact']for x in restore['rows'])
 assert {x['id']for x in cold['rows']}=={x['target']for x in s['pairs']}
 members={m['path']:m for m in s['members']};decoders=rb['decoder_receipts'];assert len(decoders)==len(members)==s['original_object_count']and {d['object']for d in decoders}==set(members)
 for d in decoders:assert d['bytes']==members[d['object']]['original']['bytes']and d['sha256']==members[d['object']]['original']['sha256']and d['absent']is True
 for key in ['original_reader','original_selected_reader','readback_reader','reader_manifest']:
  binding=a['reader_binding'][key];f=Path(binding['path']);add(f);assert pin(f)['sha256']==binding['sha256']and rb[key]==binding
 children=[c for r in stage['rows']for c in r['children']]+verified['children']+restore['children'];assert len(children)==9*s['target_count']
 assert stage['all_children_reaped']and verified['all_children_reaped']and restore['all_children_reaped']
 count=fresh_children(children+decoders);all_children+=children+decoders
 retained=[]
 for m in s['metadata']:
  f=Path(m['path'])
  if f.name in ['retention-catalog.json','retention-original.json','tmpfs-retention.json','summary.json','cleanup.json','requested-config.json','client-exit.json','retention-cleanup-intent.json']:
   check(f,m);add(f);retained.append(pin(f))
  else:omitted.append(dict(m,reason='Historical benchmark input retained locally and hash-bound by exact original selection; not re-audited in this capacity completion.'))
 cohorts.append(dict(ordinal=ordinal,screen=s['screen'],cohort=s['cohort'],selection=pin(sf),acceptance=a,original_objects=s['original_object_count'],original_logical_bytes=s['logical_cohort_bytes'],targets=s['target_count'],fresh_migration_codec_receipts=len(children),fresh_whole_reader_decoder_receipts=len(decoders),successful_fresh_codec_receipts=count,original_target_allocation_bytes=s['target_allocated_bytes'],charged_decoded_bytes=rb['charged_decoded_bytes'],decode_cap_bytes=s['limits']['cumulative_decoded_bytes'],phase_pins={k:pin(tx/n)for k,n in phase_files.items()},retained_original_metadata=retained))
assert fresh_children(all_children)==sum(c['successful_fresh_codec_receipts']for c in cohorts)
assert not Path(group['rows'][64]['root']).exists()and not (EXEC/'064').exists()
failure=read(ROOT/'root-prelaunch-first-failure.json');assert failure['terminal_receipt']=='bc47f5'and failure['exit_code']==1 and failure['runtime_started']is False
v=os.statvfs(P);free=v.f_bavail*v.f_frsize
overview=dict(complete=True,scope='Completed observer-capacity061..063 only; existing exact accepted metadata, no payload replay.',actual_tool_terminal=pin(ROOT/'tool-terminal.json'),actual_child_terminal=pin(ROOT/'launch-child-terminal.json'),controller_status=pin(ROOT/'status.json'),final_prefix=pin(results[0]),historical_prefix=list(range(61)),new_completed_ordinals=[61,62,63],next_ordinal=64,no_064_started=True,cohorts=cohorts,original_objects=sum(c['original_objects']for c in cohorts),original_logical_bytes=sum(c['original_logical_bytes']for c in cohorts),targets=sum(c['targets']for c in cohorts),successful_fresh_codec_receipts=len(all_children),baseline_available_bytes=base['actual_available_bytes'],historical_final_accounting_available_bytes=last['actual_available_bytes'],historical_final_status_available_bytes=status['available_bytes'],historical_global_available_change_bytes=status['available_bytes']-base['actual_available_bytes'],conservative_net_allocated_recovery_bytes=last['conservative_net_allocated_change_bytes']-base['conservative_net_allocated_change_bytes'],observer_launch_bytes=25778192384,observer_margin_target_bytes=26315063296,observer_target_achieved_at_completed_boundary=True,current_observation=dict(observed_ns=time.time_ns(),available_bytes=free,after_separate_observer=True),historical_capacity_is_not_current_or_reserved=True,full_performance_launch_bytes=79455850496,full_performance_current_gap_bytes=max(0,79455850496-free),full_performance_capacity_funded=False,original_prelaunch_failure=pin(ROOT/'root-prelaunch-first-failure.json'),no_payload_codec_control_workload_or_old_acceptance_replay=True,new_reporting_allocation_outside_historical_controller_accounting=True)
save('overview.json',overview);save('retained-locally.json',dict(scope='Exact old workload inputs excluded from this small capacity packet; original selections retain every hash.',files=omitted,payloads='All WAL, ordinary compressed objects, patch objects, scratch and toolchain ELF bytes stay at their original local paths; identities and hashes are in the included original selections/stage/restore/catalog results.'))
(P/'README.md').write_text(f'''# Completed observer-capacity tranche 061–063

Actual root session **46334 / 062ac3 / exit 0** completed all three original cohorts in COLD state and stopped at **064 without starting it**. The exact completed000–060 prefix remains an input authority. The prelaunch metadata lookup failure **bc47f5 / exit 1** is retained separately; no child ran in that failed attempt.

The three cohorts bind **{overview['original_objects']:,} original objects**, **{overview['original_logical_bytes']:,} logical bytes**, **{overview['targets']} targets** and **{overview['successful_fresh_codec_receipts']} successful fresh codec receipts**. These counts come from complete original stage/verification/restoration and corrected whole-reader records. Historical producer counters remain historical. This report performs no payload decode, reconstruction, tests or workload replay.

Conservative net allocated recovery was **{overview['conservative_net_allocated_recovery_bytes']:,} bytes**, measured between the original baseline and final accounting. Global available space changed from **{base['actual_available_bytes']:,}** to final status **{status['available_bytes']:,} bytes**. These are different accounting measures; logical decoded bytes are not recovered disk allocation. The completed boundary exceeded the observer launch **25,778,192,384 bytes** and its **26,315,063,296-byte** margin target.

That availability is historical. A fresh report-time observation after the separate observer capture saw **{free:,} bytes**. The **79,455,850,496-byte** full performance screen remains unfunded; this approximately1GB tranche does not qualify that screen. No later space is reserved by this report.

`overview.json` binds all actual acceptance and phase result hashes. The packet includes exact tranche source/input/command records, failures, all three original selections/catalogs, original phase outcomes and codec-intent/start/final metadata, corrected-reader and migration helper sources. `retained-locally.json` explains the excluded old benchmark input records, whose original hashes remain in the selections. All raw/encoded/patch payloads and retained executables stay local and are excluded. The original raw byte/encoded byte proofs are carried by the completed records; no payload proof was rerun.

`packet-manifest.json` maps each archive member to its unchanged absolute original path, size and SHA256. The independent reader streams the entire gzip through EOF and checks every tar member byte/hash once without extracting. New reporting allocation is measured separately and capped at4MiB; it is not retroactively folded into historical controller accounting. Original source, payload and evidence roots are unchanged.
''')
for n in ['overview.json','README.md','retained-locally.json','report.py','verify.py']:add(P/n)
assert len(chosen)<=5000 and sum(r['bytes']for r in chosen.values())<=32*MIB
rows=[dict(member=('report/'+Path(path).name if Path(path).parent==P else 'originals/'+path.lstrip('/')),**r)for path,r in sorted(chosen.items())]
assert len(set(r['member']for r in rows))==len(rows)
save('packet-manifest.json',dict(schema=1,rows=rows,members=len(rows),file_bytes=sum(r['bytes']for r in rows),decoded_cap_bytes=32*MIB,member_cap_bytes=4*MIB,compressed_cap_bytes=3*MIB))
with (P/'metadata.tar.gz').open('xb')as raw:
 with gzip.GzipFile(fileobj=raw,mode='wb',mtime=0,compresslevel=6)as gz:
  with tarfile.open(fileobj=gz,mode='w|',format=tarfile.USTAR_FORMAT)as tar:
   for r in rows:
    b=data(r['path']);assert len(b)==r['bytes']and hashlib.sha256(b).hexdigest()==r['sha256'];info=tarfile.TarInfo(r['member']);info.size=len(b);info.mode=0o644;tar.addfile(info,io.BytesIO(b))
  assert raw.tell()<=3*MIB
 raw.flush();os.fsync(raw.fileno())
assert (P/'metadata.tar.gz').stat().st_size<=3*MIB
allocated=sum(f.lstat().st_blocks*512 for f in [P,*P.rglob('*')]);assert allocated+256*1024<=4*MIB
save('packet-result.json',dict(complete=True,archive=pin(P/'metadata.tar.gz'),manifest=pin(P/'packet-manifest.json'),overview=pin(P/'overview.json'),members=len(rows),file_bytes=sum(r['bytes']for r in rows),report_allocated_bytes_before_result=allocated,report_cap_bytes=4*MIB))
print(json.dumps(dict(overview=pin(P/'overview.json'),archive=pin(P/'metadata.tar.gz'),members=len(rows),file_bytes=sum(r['bytes']for r in rows),net_allocated_recovery_bytes=overview['conservative_net_allocated_recovery_bytes'],logical_bytes=overview['original_logical_bytes'],fresh_codec_receipts=len(all_children),report_allocated_bytes=allocated),sort_keys=True))
