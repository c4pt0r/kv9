#!/usr/bin/env python3
"""Bounded retained CPU profile decoding; no runtime launches or throughput acceptance."""
from pathlib import Path
from collections import Counter
import hashlib,json,os,re,subprocess,sys,time
if not __debug__: raise RuntimeError('analysis requires PYTHONOPTIMIZE=0')
sys.dont_write_bytecode=True
ROOT=Path(__file__).parent
SOURCE=Path('/tmp/kv9-point-write-measurement-v3')
BUILD=Path('/tmp/kv9-point-write-v3-release-first/native')
PERF=(Path('/usr/lib/linux-tools')/os.uname().release/'perf').resolve()
def read(p):return json.loads(Path(p).read_text())
def save(p,v):Path(p).write_text(json.dumps(v,indent=2)+'\n')
def sha(p):
 with Path(p).open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def check(v,m):
 if not v:raise ValueError(m)
protocol=read(ROOT/'protocol.json')
check(read(ROOT/'summary.json')['complete'],'profile run incomplete')
check(sorted(os.sched_getaffinity(0))==protocol['profiler_cpus'],'analysis CPU mask differs')
sys.path.insert(0,str(SOURCE/'scripts'))
import batch_benchmark_report as validator
header=re.compile(r'^(.*?)\s+(\d+)/(\d+)\s+(\d+)\.(\d+):\s+(\d+)\s+cpu-clock:\s*$')
frame=re.compile(r'^\s+([0-9a-f]+) (.*) \((.*)\)$')
patterns={
 'metadata_lookup':r'check_context|kv9_meta::|lookup_region|region_for_key',
 'allocation_copy_compare':r'malloc|(^|::)realloc|cfree|_int_free|libc_free|unlink_chunk|memmove|memcpy|memcmp|alloc::',
 'rpc_framing_serialization_buffers':r'kv9_raft::grpc|kv9_server::grpc|tonic::|h2::|hyper::|http::|http_body::|prost::|protobuf::|raft_proto::|bytes::',
 'scheduler_synchronization':r'std::sys::sync|tokio::runtime|tokio::sync|parking_lot|crossbeam|CompletionSignal|WorkSignal',
 'raft_consensus_driver':r'kv9_raft::driver|kv9_raft::catalog_driver|kv9_raft::bridge|(?:^|[ <])raft::',
 'persistent_ordered_map':r'rpds::|archery::',
 'command_conversion':r'kv9_raft::command|lower_ops|to_write_batch',
 'engine_storage':r'kv9_engine::|kv9_raft::storage|kv9_raft::disk|kv9_raft::wal',
}
def category(f):
 s=f['symbol']
 if f['dso']=='[kernel.kallsyms]':
  if re.search(r'futex|wake_up|ttwu|schedule|sched_|finish_task_switch|pick_next|enqueue_task|dequeue_task|select_task|activate_task|load_balance|resched',s):return 'kernel_scheduler_futex'
  if re.search(r'tcp|ip_|ipv|net_|netif|skb|sock|sk_|inet|nft_|nf_|checksum|sendmsg|recvmsg',s):return 'kernel_network'
  if re.search(r'raw_spin|queued_spin',s):return 'kernel_generic_spinlock'
  return 'kernel_other'
 # First match wins. Generic allocation wins over generic type names that contain a KV9 caller.
 for name in ('allocation_copy_compare','metadata_lookup','scheduler_synchronization','rpc_framing_serialization_buffers','raft_consensus_driver','persistent_ordered_map','command_conversion','engine_storage'):
  if re.search(patterns[name],s):return name
 if '[unknown]'in s or s.startswith('0x'):return 'unknown_or_unsymbolized'
 return 'other_symbols'
results=[]
for label in protocol['profiles']:
 api={'point_put':'point_put','batch_put64':'batch_put'}[label]
 d=ROOT/api;r=read(d/'run/report.json');c=read(d/'requested-config.json');record=read(d/'profile-record.json')
 check(record['complete']and record['perf_exit_code']==0 and record['profiler_lifetimes_exited'],'recording incomplete')
 check(sha(d/'perf.data')==record['raw_perf_sha256'] and 0<(d/'perf.data').stat().st_size<protocol['max_perf_bytes'],'raw recording identity/cap differs')
 original=validator.validate(d/'run',BUILD,d/'requested-config.json',protocol['client_revision'],require_timing=False)
 check(original['accepted']and r['complete']and r['stop_reason']=='duration'and c['write_api']==api and c['read_percent']==0 and c['batch_size']==(1 if api=='point_put' else 64) and c['measure_ms']==5000,'strict report identity differs')
 commands={}
 for label,args in [('perf-script',['script','-f','--ns','-F','comm,pid,tid,time,period,event,ip,sym,symoff,dso']),('perf-top',['report','-f','--stdio','--no-children','--sort','symbol','--percent-limit','0'])]:
  cmd=['sudo','-n','env','DEBUGINFOD_URLS=',str(PERF),*args,'-i',str(d/'perf.data')]
  with (d/(label+'.txt')).open('x')as out,(d/(label+'.stderr')).open('x')as err:
   code=subprocess.run(cmd,stdout=out,stderr=err,timeout=90).returncode
  commands[label]=dict(command=cmd,exit_code=code)
  check(code==0,'perf decoder failed')
 save(d/'decode-commands.json',commands)
 check('# Total Lost Samples: 0'in(d/'perf-top.txt').read_text(),'lost sample evidence differs')
 check(not re.search(r'LOST|lost \d+|lost event', (d/'perf-script.stderr').read_text(),re.I),'decoder reports sample loss')
 samples=[]
 for block in (d/'perf-script.txt').read_text().strip().split('\n\n'):
  lines=block.splitlines();m=header.fullmatch(lines[0]);check(m is not None,'unrecognized sample '+lines[0]);frames=[]
  for line in lines[1:]:
   fm=frame.fullmatch(line);check(fm is not None,'unrecognized frame '+line)
   symbol=fm[2];s=re.fullmatch(r'(.*)\+0x([0-9a-f]+)',symbol)
   frames.append(dict(ip=fm[1],symbol=s[1]if s else symbol,offset=int(s[2],16)if s else None,dso=fm[3]))
  check(frames,'sample lacks frames')
  samples.append(dict(comm=m[1].strip(),pid=int(m[2]),tid=int(m[3]),monotonic_ns=int(m[4])*10**9+int(m[5].ljust(9,'0')),period_ns=int(m[6]),frames=frames))
 check(samples and set(s['pid']for s in samples)<=set(record['recorded_pid_scope']),'recorded PID scope differs')
 offsets=[a['realtime_ns']-(a['monotonic_before_ns']+a['monotonic_after_ns'])//2 for a in [record['anchor_before'],record['anchor_after']]]
 spread=max(offsets)-min(offsets);check(spread<1_000_000,'clock anchors differ by over edge exclusion')
 offset=sum(offsets)//2;wall_start=r['measurement_start_unix_ns'];wall_end=wall_start+5_000_000_000
 start=wall_start-offset;end=wall_end-offset;inside_start=start+1_000_000;inside_end=end-1_000_000
 check(min(s['monotonic_ns']for s in samples)<start and max(s['monotonic_ns']for s in samples)>end,'recording fails to contain measurement')
 selected=[s for s in samples if inside_start<=s['monotonic_ns']<=inside_end]
 check(len(selected)>1000,'too few bounded CPU samples');check(len({s['period_ns']for s in selected})==1,'sampling periods differ')
 bins=Counter(min(31,(s['monotonic_ns']-inside_start)*32//(inside_end-inside_start))for s in selected)
 check(set(bins)==set(range(32)),'aggregate measurement interval lacks sample coverage')
 check(min(s['monotonic_ns']for s in selected)-inside_start<50_000_000 and inside_end-max(s['monotonic_ns']for s in selected)<50_000_000,'sampled measurement edges absent')
 save(d/'measured-samples.json',selected)
 counts=Counter(category(s['frames'][0])for s in selected);leaves=Counter(s['frames'][0]['symbol']for s in selected)
 inclusive={name:sum(any(re.search(pattern,f['symbol'])for f in s['frames'])for s in selected)for name,pattern in patterns.items()}
 inclusive['kernel']=sum(any(f['dso']=='[kernel.kallsyms]'for f in s['frames'])for s in selected)
 stacks=Counter(';'.join(f['symbol']for f in s['frames'])for s in selected)
 outcomes={name:sum(op['populations'][i]['calls']for op in r['metrics']['measurement']['statistics'])for i,name in enumerate(r['metrics']['measurement']['outcomes'])}
 cleanup=read(d/'cleanup.json');check(not cleanup.get('errors')and all(x['exit_code']is not None and x['absent']and not Path('/proc',str(x['identity']['pid'])).exists()for x in cleanup['children']),'owned fixture lifetime remains')
 retention=read(d/'fixture/tmpfs-retention.json');check(retention['complete']and retention['scratch_removed']and not Path(retention['original_scratch']).exists(),'tmpfs cleanup incomplete')
 for name,item in retention['files'].items():
  p=Path(retention['retained_data'])/name;check(p.stat().st_size==item['bytes']and sha(p)==item['sha256'],'retained data differs')
 check(sha(d/'perf.data')==record['raw_perf_sha256'],'decoder changed raw recording')
 count=len(selected)
 result=dict(api=api,complete=True,instrumented=True,throughput_acceptance=False,server_revision=protocol['server_revision'],server_sha256=protocol['server_sha256'],client_revision=protocol['client_revision'],client_sha256=protocol['client_sha256'],raw_perf_sha256=record['raw_perf_sha256'],raw_perf_bytes=record['raw_perf_bytes'],recorded_samples=len(samples),selected_samples=count,lost_samples=0,recording_contains_nominal_measurement=True,measurement_unix_ns=[wall_start,wall_end],measurement_monotonic_ns=[start,end],selected_monotonic_ns=[inside_start,inside_end],excluded_edge_ns=1_000_000,wall_to_monotonic_offsets_ns=offsets,offset_spread_ns=spread,aggregate_32_bins=dict(sorted(bins.items())),period_ns=selected[0]['period_ns'],sampled_cpu_ns=sum(s['period_ns']for s in selected),categories={k:dict(samples=v,percent=100*v/count)for k,v in counts.most_common()},top_leaf_symbols=[dict(symbol=k,samples=v,percent=100*v/count)for k,v in leaves.most_common(60)],inclusive_partial_stack_counts={k:dict(samples=v,percent=100*v/count)for k,v in inclusive.items()},per_voter={node:dict(pid=v['pid'],samples=sum(s['pid']==v['pid']for s in selected),equal_bins=sorted({min(31,(s['monotonic_ns']-inside_start)*32//(inside_end-inside_start))for s in selected if s['pid']==v['pid']}))for node,v in record['voters'].items()},stack_depth_counts=dict(Counter(len(s['frames'])for s in selected)),top_multiframe_stacks=[dict(stack=k,samples=v,percent=100*v/count)for k,v in stacks.most_common(50)],calls=r['measured_completed'],outcomes=outcomes,whole_call_metrics=r['metrics']['measurement'],cohort_elapsed_ns=r['cohort_elapsed_ns'],owned_fixture_lifetimes_exited=len(cleanup['children']),profiler_lifetimes_exited=len(record['profiler_lifetimes']),retained_files=len(retention['files']),retained_bytes=sum(v['bytes']for v in retention['files'].values()),resource_coverage=read(d/'resource-coverage.json'),category_rules=patterns,exclusive_priority=['kernel','allocation_copy_compare','metadata_lookup','scheduler_synchronization','rpc_framing_serialization_buffers','raft_consensus_driver','persistent_ordered_map','command_conversion','engine_storage','unknown_or_unsymbolized','other_symbols'],limits=['One instrumented 5-second sample per API; shared host, no repeated A/B throughput or latency acceptance.','Volatile tmpfs WAL retains ordinary Raft quorum/sync calls; no disk or power-loss claim.','Only on-CPU samples: percentages are not end-to-end latency fractions.','Inclusive recovered stacks overlap and are not additive. Optimized code/async boundaries limit unwinding.','Unmeasured idle followers need not cover every interval; aggregate 32-bin coverage is checked.','No prior executable instruction offsets are reused.'])
 save(d/'profile-summary.json',result);results.append({k:result[k]for k in ('api','complete','selected_samples','recorded_samples','calls','outcomes','categories','inclusive_partial_stack_counts','top_leaf_symbols')})
 print('PASS: '+api+' '+str(count)+' selected CPU samples',flush=True)
save(ROOT/'analysis-summary.json',dict(complete=True,analyzer_sha256=sha(__file__),profiles=results))
