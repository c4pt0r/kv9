#!/usr/bin/env python3
"""Bounded arithmetic from the accepted v3 audit and its hash-bound reports."""
import collections
import csv
import hashlib
import json
from pathlib import Path
import time

if not __debug__:
    raise RuntimeError('assertions must remain enabled')

HERE=Path(__file__).resolve().parent
AUDIT=HERE/'results-first/audit.json'


def sha(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream,'sha256').hexdigest()


def read(path):
    return json.loads(path.read_text())


def save(name,value):
    with (HERE/name).open('x') as stream:
        json.dump(value,stream,indent=2,sort_keys=True)
        stream.write('\n')


def merge_histograms(raws):
    buckets=[0]*3776
    count=total=0
    for raw in raws:
        assert (len(raw['buckets'])==len(buckets) or
                (raw['buckets']==[] and raw['count']==raw['sum_ns']==0))
        assert sum(raw['buckets'])==raw['count']
        count+=raw['count'];total+=raw['sum_ns']
        for i,n in enumerate(raw['buckets']):buckets[i]+=n
    def quantile(percent):
        if count==0:return None
        rank=(count*percent+99)//100;seen=0
        for i,n in enumerate(buckets):
            seen+=n
            if seen>=rank:
                if i<64:return dict(lower_ns=i,upper_ns=i)
                shift=(i-64)//64;lower=(64+(i-64)%64)<<shift
                return dict(lower_ns=lower,upper_ns=lower+(1<<shift)-1)
        raise ValueError('missing raw histogram rank')
    return dict(count=count,sum_ns=total,mean_ns=total/count if count else None,
                **{f'p{percent}':quantile(percent) for percent in (50,95,99)})


def add_maps(maps):
    result=collections.Counter()
    for row in maps:
        if row is not None:result.update(row)
    return dict(result)


def phase_accounting(metrics,native):
    rows=[]
    for name,op in zip(metrics['operations'],metrics['statistics']):
        outcomes={name:p['calls'] for name,p in zip(metrics['outcomes'],op['populations'])}
        calls=sum(outcomes.values())
        attempts=sum(op['attempt_reasons']) if native else op['command_attempts']
        rows.append(dict(operation=name,calls=calls,input_items=sum(p['input_items'] for p in op['populations']),
            success_calls=outcomes['success'],attempts=attempts,extra_attempts=attempts-calls,
            outcomes=outcomes,reasons=dict(zip(metrics['reasons'],op['reasons'])),
            attempt_reasons=dict(zip(metrics['reasons'],op['attempt_reasons'])) if native else None,
            terminal_rpc_codes=op['rpc_codes'] if native else None,
            connection_attempts=None if native else op['connection_attempts'],
            connection_failures=None if native else op['connection_failures']))
    return dict(operations=rows,calls=sum(r['calls'] for r in rows),input_items=sum(r['input_items'] for r in rows),
        attempts=sum(r['attempts'] for r in rows),extra_attempts=sum(r['extra_attempts'] for r in rows),
        outcomes=add_maps(r['outcomes'] for r in rows),reasons=add_maps(r['reasons'] for r in rows),
        attempt_reasons=add_maps(r['attempt_reasons'] for r in rows))


def aggregate(cases,reports):
    elapsed=sum(c['cohort_elapsed_ns'] for c in cases)
    operations=[]
    for index in (0,1):
        original=[c['operations'][index] for c in cases]
        names={op['operation'] for op in original};assert len(names)==1
        raw_operations=[r['metrics']['measurement']['statistics'][index] for r in reports]
        calls=sum(op['calls'] for op in original);items=sum(op['input_items'] for op in original)
        outcomes=add_maps(op['outcomes'] for op in original)
        success_items=sum(op['populations'][0]['input_items'] for op in raw_operations)
        hist=merge_histograms(p['whole_call']['raw'] for op in raw_operations for p in op['populations'])
        assert hist['count']==calls
        operations.append(dict(operation=next(iter(names)),calls=calls,input_items=items,attempts=sum(op['attempts'] for op in original),
            outcomes=outcomes,reasons=add_maps(op['reasons'] for op in original),
            attempt_reasons=add_maps(op['attempt_reasons'] for op in original),
            completed_calls_per_second=calls*1e9/elapsed,successful_calls_per_second=outcomes['success']*1e9/elapsed,
            completed_input_items_per_second=items*1e9/elapsed,successful_input_items_per_second=success_items*1e9/elapsed,
            whole_call_latency=hist,
            successful_whole_call_latency=merge_histograms(op['populations'][0]['whole_call']['raw'] for op in raw_operations)))
    hist=merge_histograms(p['whole_call']['raw'] for r in reports for op in r['metrics']['measurement']['statistics'] for p in op['populations'])
    calls=sum(op['calls'] for op in operations);items=sum(op['input_items'] for op in operations)
    outcomes=add_maps(op['outcomes'] for op in operations)
    assert hist['count']==calls
    cpu={}
    for name in reports[0]['_accepted_cpu']:
        rows=[r['_accepted_cpu'][name] for r in reports]
        seconds=sum(row['sample_seconds'] for row in rows)
        cpu[name]=dict(cpu_cores=sum(row['cpu_cores']*row['sample_seconds'] for row in rows)/seconds,
                       sample_seconds=seconds,
                       scope='Retained resource-coverage CPU estimate, weighted by observed sample seconds across these cohorts')
    return dict(calls=calls,input_items=items,issued=sum(c['issued'] for c in cases),
        attempts=sum(c['attempts'] for c in cases),dropped_slots=sum(c['dropped_slots'] for c in cases),
        cohort_elapsed_ns=elapsed,outcomes=outcomes,reasons=add_maps(op['reasons'] for op in operations),
        attempt_reasons=add_maps(op['attempt_reasons'] for op in operations),
        completed_calls_per_second=calls*1e9/elapsed,successful_calls_per_second=outcomes['success']*1e9/elapsed,
        completed_input_items_per_second=items*1e9/elapsed,
        successful_input_items_per_second=sum(op['successful_input_items_per_second'] for op in operations),
        whole_call_latency=hist,operations=operations,cpu=cpu,
        client_cpu_cores=cpu['client']['cpu_cores'],
        server_cpu_cores=sum(row['cpu_cores'] for name,row in cpu.items() if name!='client'))


def compare(old,new):
    def delta(a,b):return (b/a-1)*100 if a else None
    return dict(completed_qps_change_percent=delta(old['completed_calls_per_second'],new['completed_calls_per_second']),
                successful_qps_change_percent=delta(old['successful_calls_per_second'],new['successful_calls_per_second']),
                mean_change_percent=delta(old['whole_call_latency']['mean_ns'],new['whole_call_latency']['mean_ns']),
                old_p95=old['whole_call_latency']['p95'],new_p95=new['whole_call_latency']['p95'],
                old_p99=old['whole_call_latency']['p99'],new_p99=new['whole_call_latency']['p99'])


def qtext(hist,percent):
    q=hist['p'+str(percent)]
    return '—' if q is None else f"[{q['lower_ns']/1000:.3f}, {q['upper_ns']/1000:.3f}]"


audit=read(AUDIT);inventory=read(HERE/'results-first/input-inventory.json')
assert audit['complete'] and audit['matched_diagnostic_accepted'] and audit['parent_confirmed_session']==87861
assert len(audit['cases'])==72 and read(HERE/'audit-exit-first.json')['exit_code']==0
reports={};bound={}
for case in audit['cases']:
    path=Path(case['directory'])/'run/report.json';digest=sha(path)
    assert inventory[str(path)]['sha256']==case['report_sha256']==digest
    reports[case['ordinal']]=read(path);bound[str(path)]=dict(bytes=path.stat().st_size,sha256=digest)
    coverage_path=Path(case['directory'])/'resource-coverage.json'
    coverage_digest=sha(coverage_path)
    assert inventory[str(coverage_path)]['sha256']==coverage_digest
    coverage=read(coverage_path)
    reports[case['ordinal']]['_accepted_cpu']=coverage['cpu']
    bound[str(coverage_path)]=dict(bytes=coverage_path.stat().st_size,sha256=coverage_digest)
    r=reports[case['ordinal']]
    for i,op in enumerate(r['metrics']['measurement']['statistics']):
        assert merge_histograms(p['whole_call']['raw'] for p in op['populations'])==case['operations'][i]['whole_call_latency']
    assert merge_histograms(p['whole_call']['raw'] for op in r['metrics']['measurement']['statistics'] for p in op['populations'])==case['whole_call_latency']

cases=[]
for case in audit['cases']:
    r=reports[case['ordinal']]
    row={k:case[k] for k in ['ordinal','repeat','concurrency','arm','workload','read_api','write_api','batch_size','read_percent',
                            'directory','report_sha256','all_success_single_attempt','dataset','storage']}
    row['role']=case['arm'].split('-',1)[0]
    row.update(aggregate([case],[r]))
    row['phases']={phase:phase_accounting(metrics,row['role']!='redis') for phase,metrics in r['metrics'].items()}
    cases.append(row)

pooled=[]
for concurrency in (1,64):
    for workload in ('point-r000','point-r050','point-r100','batch64-r000','batch64-r050','batch64-r100'):
        for role in ('old','new','redis'):
            selected=[c for c in cases if c['concurrency']==concurrency and c['workload']==workload and c['role']==role]
            assert len(selected)==2 and {c['repeat'] for c in selected}=={0,1}
            original=[audit['cases'][c['ordinal']] for c in selected]
            row={k:selected[0][k] for k in ['concurrency','workload','role','read_api','write_api','batch_size','read_percent']}
            row.update(aggregate(original,[reports[c['ordinal']] for c in selected]))
            row['source_ordinals']=[c['ordinal'] for c in selected];row['repeats']=[c['repeat'] for c in selected]
            pooled.append(row)

pairs=[];pooled_pairs=[]
for repeat in (0,1):
    for concurrency in (1,64):
        for workload in ('point-r000','point-r050','point-r100','batch64-r000','batch64-r050','batch64-r100'):
            selected={c['role']:c for c in cases if c['repeat']==repeat and c['concurrency']==concurrency and c['workload']==workload}
            assert set(selected)=={'old','new','redis'}
            pairs.append(dict(repeat=repeat,concurrency=concurrency,workload=workload,
                role_ordinals={role:c['ordinal'] for role,c in selected.items()},
                combined=compare(selected['old'],selected['new']),
                operations=[dict(operation=selected['old']['operations'][i]['operation'],
                    **compare(selected['old']['operations'][i],selected['new']['operations'][i])) for i in (0,1)]))
for concurrency in (1,64):
    for workload in ('point-r000','point-r050','point-r100','batch64-r000','batch64-r050','batch64-r100'):
        selected={c['role']:c for c in pooled if c['concurrency']==concurrency and c['workload']==workload}
        pooled_pairs.append(dict(concurrency=concurrency,workload=workload,combined=compare(selected['old'],selected['new']),
            operations=[dict(operation=selected['old']['operations'][i]['operation'],
                **compare(selected['old']['operations'][i],selected['new']['operations'][i])) for i in (0,1)]))

phase_totals={}
for phase in ['initialization','warmup','measurement','verification']:
    selected=[case['phases'][phase] for case in cases]
    phase_totals[phase]={key:sum(c[key] for c in selected) for key in ['calls','input_items','attempts','extra_attempts']}
    for key in ['outcomes','reasons','attempt_reasons']:phase_totals[phase][key]=add_maps(c[key] for c in selected)
totals={key:sum(c[key] for c in cases) for key in ['calls','input_items','issued','attempts','dropped_slots']}
for key in ['outcomes','reasons','attempt_reasons']:totals[key]=add_maps(c[key] for c in cases)
deviation=HERE/'root-freeze-summary-first-failure.json'
freeze_note=read(deviation)
assert freeze_note['failed_exit_code']==1 and freeze_note['timing_started_despite_failed_ancillary_check'] is True
assert freeze_note['changed_driver_protocol_auditor_or_acceptance_predicates'] is False
checks={k:audit[k] for k in ['successful_arms','owned_lifetimes_exited','role_source_file_checks','resource_samples','qualifying_drains',
                           'voter_writer_listener_bindings','retained_files','retained_bytes','outer_restoration']}
checks['endpoint_snapshot_files_bound']=sum(str(Path(c['directory'])/(phase+'-'+kind+'.json')) in inventory for c in cases if c['role']!='redis'
    for phase in ['before','after'] for kind in ['metrics','status','resources'])
assert checks['endpoint_snapshot_files_bound']==288
checks['minimum_available_bytes']={name:min(c['storage']['minimum_available_bytes'][name] for c in cases) for name in ('tmpfs','retention')}
stat=dict(accepted_audit_sha256=sha(AUDIT),accepted_inventory_sha256=sha(HERE/'results-first/input-inventory.json'),
          protocol_id=audit['protocol_id'],scope=audit['scope'],cases=cases,pooled=pooled,pairs=pairs,pooled_pairs=pooled_pairs,
          totals=totals,phase_totals=phase_totals,checks=checks,
          method='Completed/successful calls and input keys per second use the complete cohort elapsed denominator. Per-operation rates use that same denominator. Means use total whole-call nanoseconds/count. Combined and pooled p50/p95/p99 merge original validated raw histogram buckets by call count; no percentile averaging or division of latency by key count. Inactive operation latency is null.',
          orchestration_deviation=dict(path=str(deviation),sha256=sha(deviation),details=freeze_note),
          performance_promotion=False,endpoint_stage_semantics='Separate root-owned validation; six existing envelopes per native cohort are hash-bound only.',
          derivation_attempts={'first':{'script':'derive-statistics-first.py','script_sha256':sha(HERE/'derive-statistics-first.py'),
              'log':'statistics-first.log','log_sha256':sha(HERE/'statistics-first.log'),'exit_code':1,
              'cause':'New derivation required 3776 buckets even for an inactive zero-population histogram serialized as buckets=[]; failed before any statistics/readout output.'},
              'second':{'script':Path(__file__).name,'scope':'Accept only count=sum=0 empty-vector histograms as permitted by the unchanged source validator; no audit/runtime rerun.'}})
save('statistics-first.json',stat)

fields=['repeat','concurrency','workload','role','view','operation','calls','input_items','attempts','completed_calls_per_second',
        'successful_calls_per_second','completed_input_items_per_second','successful_input_items_per_second','mean_us',
        'p50_lower_us','p50_upper_us','p95_lower_us','p95_upper_us','p99_lower_us','p99_upper_us','outcomes_json','reasons_json',
        'client_cpu_cores','server_cpu_cores']
for filename,source_rows in [('cohorts-first.csv',cases),('pooled-first.csv',pooled)]:
    with (HERE/filename).open('x',newline='') as stream:
        writer=csv.DictWriter(stream,fieldnames=fields);writer.writeheader()
        for case in source_rows:
            for view,op in [('combined',case),*[(r['operation'],r) for r in case['operations']]]:
                h=op['whole_call_latency']
                row={key:op[key] for key in ['calls','input_items','attempts','completed_calls_per_second','successful_calls_per_second',
                                           'completed_input_items_per_second','successful_input_items_per_second']}
                row.update(repeat=case.get('repeat','pooled 0+1'),concurrency=case['concurrency'],workload=case['workload'],role=case['role'],
                           view=view,operation=op.get('operation','read+write'),mean_us=h['mean_ns']/1000 if h['mean_ns'] is not None else None,
                           outcomes_json=json.dumps(op['outcomes'],sort_keys=True),reasons_json=json.dumps(op['reasons'],sort_keys=True),
                           client_cpu_cores=case['client_cpu_cores'],server_cpu_cores=case['server_cpu_cores'])
                for q in ('p50','p95','p99'):
                    for edge in ('lower','upper'):row[q+'_'+edge+'_us']=h[q][edge+'_ns']/1000 if h[q] is not None else None
                writer.writerow(row)

lines=['# V3 point and batch64 workload diagnostic readout','',
       f"The first frozen independent audit accepted all 72 cohorts from timing session 87861 (exit 0). Measurement retained {totals['calls']:,} calls / {totals['input_items']:,} input keys, all successful with one attempt each; all refusals, unknown writes, read failures, client rejections and dropped slots were zero. This is accounting and environment acceptance, not a promotion decision.",'',
       'Control is 5ee897a; candidate is 57ff685. Both share the frozen native/Redis v3 clients at 0be806d. Two repeats run the entire 36-case order forward then reverse. Each timed cell is 10 seconds, c1 or c64, point size 1 or batch size 64, read percentage 0/50/100, 4,096 keys plus sentinel, 128-byte values, seed 71, 128 warmup calls, 10M cap and 1,500 ms deadline. Configured native maximum attempts remains six; actual phase counts below preserve any routing attempts.','',
       'All original per-repeat combined and per-operation call/key rates, means, p50/p95/p99 intervals and outcome populations are in `cohorts-first.csv` (216 rows) and `statistics-first.json`. `pooled-first.csv` contains 108 corresponding pooled rows. Pooled rates divide summed calls/keys by summed cohort elapsed time; means divide summed whole-call latency by summed count. Pooled quantiles merge original raw buckets, never average percentiles. Mixed combined latency weights every read/write call by its actual population; latency is never divided by batch size.','',
       '| Workers | Workload | Role | Pooled calls/s | Pooled keys/s | Mean µs | p95 µs | p99 µs | Client CPU cores | Server CPU cores |',
       '|---:|---|---|---:|---:|---:|---|---|---:|---:|']
for row in pooled:
    h=row['whole_call_latency'];lines.append(f"| {row['concurrency']} | {row['workload']} | {row['role']} | {row['completed_calls_per_second']:.3f} | {row['completed_input_items_per_second']:.3f} | {h['mean_ns']/1000:.3f} | {qtext(h,95)} | {qtext(h,99)} | {row['client_cpu_cores']:.3f} | {row['server_cpu_cores']:.3f} |")
lines+=['','Both original candidate/control repeats remain visible below; complete per-operation comparisons are in the JSON and CSV artifacts.','',
        '| Workers | Workload | Repeat 0 calls/s change | Repeat 1 calls/s change | Pooled calls/s change | Pooled mean change |',
        '|---:|---|---:|---:|---:|---:|']
for row in pooled_pairs:
    original=[next(r for r in pairs if r['repeat']==repeat and r['concurrency']==row['concurrency'] and r['workload']==row['workload']) for repeat in (0,1)]
    lines.append(f"| {row['concurrency']} | {row['workload']} | {original[0]['combined']['completed_qps_change_percent']:+.3f}% | {original[1]['combined']['completed_qps_change_percent']:+.3f}% | {row['combined']['completed_qps_change_percent']:+.3f}% | {row['combined']['mean_change_percent']:+.3f}% |")
lines+=['','All-phase accounting (includes initialization and final verification, outside measured comparisons):','',
        '| Phase | Calls | Attempts | Extra attempts | Non-success terminal outcomes |','|---|---:|---:|---:|---:|']
for phase,row in phase_totals.items():lines.append(f"| {phase} | {row['calls']:,} | {row['attempts']:,} | {row['extra_attempts']:,} | {sum(n for name,n in row['outcomes'].items() if name!='success'):,} |")
lines+=['',
       'Preserved orchestration deviation: root’s ancillary freeze-summary script failed at an unsupported all-smoke-phases single-attempt assertion (tool 23f5ec, exit 1), but root then started this first timing run in the same orchestration sequence. The 24 native smoke initialization reads each had one extra routing attempt; smoke warmup/measurement/verification were single-attempt. The driver, protocol and auditor were already frozen and independently reviewed, and no acceptance predicate or source changed. The final root inventory was written after launch and is not presented as a prelaunch inventory. The original failure record and launch chronology remain unchanged; this audit does not erase that process deviation.','',
       f"The audit checked {checks['owned_lifetimes_exited']} exited owned lifetimes, {checks['qualifying_drains']} fresh drain stages, {checks['voter_writer_listener_bindings']} writer/listener bindings, {checks['resource_samples']:,} resource samples, {checks['role_source_file_checks']:,} role source-file checks and {checks['retained_files']:,} retained files / {checks['retained_bytes']:,} bytes. Exact original/effective CPU restoration and historical namespace identities passed. All 288 already-recorded endpoint envelopes are bound in the accepted inventory; root’s stage analysis remains separate and its stage means must not be treated as an additive latency partition.",'',
       f"Observed free-space minima were {checks['minimum_available_bytes']['tmpfs']:,} bytes for tmpfs and {checks['minimum_available_bytes']['retention']:,} bytes for retention storage. Preflight and runtime storage guards passed. They are operational observations, not a proof that the call cap bounds future disk growth.",'',
       'Scope: shared host; native three-voter quorum WAL on tmpfs versus standalone memory Redis, with unequal durability. Final deterministic values, sentinel preservation, configured nonce budgets and write-key membership passed. The aggregate reports do not record the exact issued nonce set or full concurrent histories. No whole-history, linearizability, Chaos, sustained-capacity, statistical-significance or no-regression claim is made. Both repeats and all original outcomes remain retained.','',
       'CPU core equivalents are taken from the already accepted resource-coverage summaries. Per-process estimates are weighted by their observed sample seconds; server CPU is the sum of those estimates (three native voters or one Redis process). They are not an additive partition of request latency.','',
       'The first statistics-only reader failed on the valid empty-bucket representation for inactive operations, before producing statistics or readout files. Its original script/log are retained; this separately named second reader supports only the validator-permitted zero-count/zero-sum empty vector. The frozen audit and runtime were not changed or rerun.','',
       f"Accepted audit SHA-256: `{sha(AUDIT)}`.",f"Statistics SHA-256: `{sha(HERE/'statistics-first.json')}`.",
       f"Original orchestration-deviation SHA-256: `{sha(deviation)}`."]
with (HERE/'READOUT.md').open('x') as stream:stream.write('\n'.join(lines)+'\n')
save('statistics-inputs-first.json',dict(accepted_audit=dict(path=str(AUDIT),sha256=sha(AUDIT)),
    accepted_inventory=dict(path=str(HERE/'results-first/input-inventory.json'),sha256=sha(HERE/'results-first/input-inventory.json')),
    reports=bound,root_orchestration_deviation=dict(path=str(deviation),sha256=sha(deviation)),
    derivation_sha256=sha(Path(__file__)),completed_ns=time.time_ns()))
print(json.dumps({'measurement_totals':totals,'phase_totals':phase_totals,'checks':checks,
                  'pooled_changes':[dict(concurrency=r['concurrency'],workload=r['workload'],**r['combined']) for r in pooled_pairs],
                  'hashes':{name:sha(HERE/name) for name in ['results-first/audit.json','results-first/input-inventory.json','statistics-first.json',
                       'READOUT.md','cohorts-first.csv','pooled-first.csv','statistics-inputs-first.json']}},indent=2))
