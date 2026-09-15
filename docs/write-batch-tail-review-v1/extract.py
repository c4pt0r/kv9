#!/usr/bin/env python3
"""Finite offline review of 12 already accepted loaded BatchPut64 cohorts."""
import hashlib
import json
import subprocess
from pathlib import Path

OUT = Path(__file__).parent
REPO = Path('/home/dongxu/kv9')
TIMING = Path('/mnt/data/kv9-work/upper-bound-requalified-report-preparation-20260915-first/results-first')
INPUTS = {}

def read(path, expected=None):
    path = Path(path)
    raw = path.read_bytes()
    assert len(raw) <= 8 * 1024**2
    pin = dict(bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())
    if expected is not None:
        assert pin == {k: expected[k] for k in pin}, str(path)
    INPUTS[str(path)] = pin
    return json.loads(raw)

def quantile(buckets, p, native=False):
    n = sum(buckets)
    if not n:
        return None
    rank = (n * p + 99) // 100
    seen = 0
    for i, v in enumerate(buckets):
        seen += v
        if seen >= rank:
            if native:
                if i < 64:
                    return dict(lower_ns=i, upper_ns=i)
                exp, part = divmod(i - 64, 64)
                lo = (64 + part) << exp
                return dict(lower_ns=lo, upper_ns=lo + (1 << exp) - 1)
            return dict(lower_ns=0 if i == 0 else 1 << (i - 1), upper_ns=(1 << i) - 1)
    raise ValueError('incomplete population')

def native(row, authority):
    p = Path(row['directory']) / 'run/report.json'
    d = read(p, authority[str(p)])
    op = d['metrics']['measurement']['statistics'][1]
    pop = op['populations'][0]
    raw = pop['whole_call']['raw']
    assert raw['valid'] is True and len(raw['buckets']) == 3776
    assert raw['count'] == sum(raw['buckets']) == pop['calls'] == d['measured_completed'] == d['measured_issued']
    assert pop['input_items'] == 64 * pop['calls'] and d['dropped_slots'] == 0
    assert sum(p['calls'] for p in op['populations'][1:]) == 0
    assert sum(a['raw']['count'] for a in op['attempts']) == pop['calls']
    return dict(report=str(p), calls=pop['calls'], items=pop['input_items'], elapsed_ns=d['cohort_elapsed_ns'],
                sum_ns=raw['sum_ns'], buckets=raw['buckets'])

def pool(raws):
    calls = sum(x['calls'] for x in raws)
    items = sum(x['items'] for x in raws)
    elapsed = sum(x['elapsed_ns'] for x in raws)
    ns = sum(x['sum_ns'] for x in raws)
    buckets = [sum(x['buckets'][i] for x in raws) for i in range(3776)]
    assert sum(buckets) == calls and items == calls * 64
    return dict(calls=calls, items=items, elapsed_ns=elapsed, sum_ns=ns,
                calls_per_second=calls * 1e9 / elapsed, items_per_second=items * 1e9 / elapsed,
                mean_ns=ns / calls, **{f'p{p}': quantile(buckets, p, True) for p in (50,95,99)})

def metrics(directory, authority):
    paths = [Path(directory) / (when + '-metrics.json') for when in ('before','after')]
    before, after = [read(p, authority[str(p)]) for p in paths]
    status_paths = [Path(directory) / (when + '-status.json') for when in ('before','after')]
    statuses = [read(p, authority[str(p)]) for p in status_paths]
    nodes = {}
    for node in ('1','2','3'):
        a,b = before[node], after[node]
        for k in ('schema_version','process_id','node_id','exporter_created_unix_ns','bucket_count','bucket_rule','clock','duration_unit'):
            assert a[k] == b[k], (node,k)
        assert a['bucket_count'] == 65 and a['reset'] == b['reset'] == 'node_component_construction'
        assert a['export_failures_before_capture'] == b['export_failures_before_capture'] == 0
        assert a['export_failures_saturated'] is False and b['export_failures_saturated'] is False
        assert int(b['captured_unix_ns']) > int(a['captured_unix_ns'])
        sa,sb = [s[node] for s in statuses]
        identity_keys = [k for k in sa if any(x in k for x in ('boot_id','start_ticks','process_id'))]
        assert identity_keys
        for k in identity_keys:
            assert sa[k] == sb[k], k
        changes = {}
        assert [m['name'] for m in a['metrics']] == [m['name'] for m in b['metrics']]
        for ma,mb in zip(a['metrics'], b['metrics']):
            outcomes = {}
            for oa,ob in zip(ma['latency']['outcomes'],mb['latency']['outcomes']):
                assert oa['outcome'] == ob['outcome']
                for endpoint in (oa,ob):
                    assert not any(endpoint[k] for k in ('count_saturated','sum_saturated','duration_clamped'))
                    assert len(endpoint['buckets']) == 65 and sum(endpoint['buckets']) == endpoint['count']
                dc, ds = ob['count']-oa['count'], ob['sum_ns']-oa['sum_ns']
                db = [y-x for x,y in zip(oa['buckets'],ob['buckets'])]
                assert dc >= 0 and ds >= 0 and min(db) >= 0 and sum(db) == dc
                assert sum(n*(0 if i == 0 else 1 << (i-1)) for i,n in enumerate(db)) <= ds <= sum(n*((1 << i)-1) for i,n in enumerate(db))
                if dc:
                    outcomes[oa['outcome']] = dict(count=dc,sum_ns=ds,mean_ns=ds/dc,
                        **{f'p{p}':quantile(db,p) for p in (50,95,99)})
            changes[ma['name']] = outcomes
        nodes[node] = dict(identity={k:sa[k] for k in identity_keys},
            role_before=sa.get('role'),role_after=sb.get('role'),leader_before=sa.get('leader_id'),leader_after=sb.get('leader_id'),
            capture_before_unix_ns=a['captured_unix_ns'],capture_after_unix_ns=b['captured_unix_ns'],
            endpoint_elapsed_ns=int(b['captured_unix_ns'])-int(a['captured_unix_ns']),metrics=changes)
    return nodes

def compare(a,b):
    return dict(throughput_percent=(b['items_per_second']/a['items_per_second']-1)*100,
                mean_percent=(b['mean_ns']/a['mean_ns']-1)*100,
                p99_change_percent_bounds=[(b['p99']['lower_ns']/a['p99']['upper_ns']-1)*100,
                                           (b['p99']['upper_ns']/a['p99']['lower_ns']-1)*100])

def main():
    assert __debug__ and not (OUT/'summary.json').exists()
    summary = read(TIMING/'summary.json')
    assert summary['complete'] and summary['healthy_comparison_eligible'] and not summary['performance_promotion']
    authority = read('/mnt/data/kv9-work/upper-bound-requalified-preparation-20260915-first/results-first/input-inventory.json')
    assert INPUTS['/mnt/data/kv9-work/upper-bound-requalified-preparation-20260915-first/results-first/input-inventory.json']['sha256'] == summary['audit_input_inventory_sha256']
    audit = read(summary['audit_path'])
    assert INPUTS[summary['audit_path']]['sha256'] == summary['audit_sha256']
    rows, raw = [], {}
    for r in summary['repetitions']:
        if (r['workload'],r['concurrency']) != ('batch64',64):
            continue
        row = {k:r[k] for k in ('ordinal','repeat','target','order','server_cpu_cores','client_cpu_cores','server_memory')}
        row['directory'] = str(Path(r['source_report']).parent.parent)
        n = native(row, authority); raw[r['ordinal']] = n
        row['measurement'] = pool([n])
        assert row['measurement']['mean_ns'] == r['whole_call_latency']['mean_ns']
        assert row['measurement']['p99'] == r['whole_call_latency']['p99']
        row['whole_capture_nodes'] = metrics(row['directory'], authority)
        rows.append(row)
    assert len(rows) == 4
    pooled = {mode:pool([raw[r['ordinal']] for r in rows if r['target']==mode]) for mode in ('old','new')}
    comparisons = []
    for repeat in (0,1):
        rr = {r['target']:r for r in rows if r['repeat']==repeat}
        comparisons.append(dict(repeat=repeat,**compare(rr['old']['measurement'],rr['new']['measurement'])))
    result = dict(complete=True,scope='Loaded BatchPut64 only; separate accepted populations; no new runtime or causality.',
        timing=dict(source_roles=summary['source_roles'],rows=rows,pooled=pooled,comparisons=comparisons,pooled_change=compare(pooled['old'],pooled['new'])),observers=[])
    for tag, analysis_path in [('schema1','/tmp/kv9-write-diagnostics-analysis-20260915-first/results-first/result.json'),('schema2','/tmp/kv9-upper-bound-observer-preparation-20260915-first/analysis-first/result.json')]:
        a=read(analysis_path)
        expected={'schema1':'11311dee4d8a31ff20dbc18f5d52038296ae2a4677591f2a00d699af81e928de','schema2':'3cc9e8f56593ca4f58eff29da7640b8b68dfca29b90169ecdb847a73f4a776a9'}[tag]
        assert INPUTS[analysis_path]['sha256']==expected and a['complete']
        au=read(Path(analysis_path).parent/'input-hashes.json')
        rr=[]; nr={}
        for r in a['rows']:
            if (r['api'],r['concurrency']) != ('BatchPut64',64): continue
            n=native(r,au);nr[r['ordinal']]=n
            row={k:r[k] for k in ('ordinal','mode','repeat','directory','nodes','whole_capture_scope') if k in r}
            row['measurement']=pool([n]);row['whole_capture_metrics']=metrics(r['directory'],au)
            if r['mode']=='diagnostic':
                # Retain compact accepted distributions and joint marginals for all three nodes.
                for node,v in row['nodes'].items():
                    delta=v['delta']
                    for dist in delta['driver']['distributions'] + delta['ready']:
                        dist['nonempty_mean']=dist['sum']/(dist['count']-dist['buckets'][0]) if dist['count']>dist['buckets'][0] else None
                        dist['singleton_fraction']=dist['buckets'][1]/dist['count'] if dist['count'] else None
                        dist.pop('buckets')
            rr.append(row)
        assert len(rr)==4
        pools={mode:pool([nr[r['ordinal']] for r in rr if r['mode']==mode]) for mode in ('default','diagnostic')}
        cmp=[]
        for repeat in (0,1):
            pair={r['mode']:r for r in rr if r['repeat']==repeat}
            cmp.append(dict(repeat=repeat,**compare(pair['default']['measurement'],pair['diagnostic']['measurement'])))
        result['observers'].append(dict(schema=tag,source_revision=a['source_revision'],client_binary_sha256='8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957',rows=rr,pooled=pools,comparisons=cmp,pooled_change=compare(pools['default'],pools['diagnostic'])))
    rels=['crates/raft/src/driver.rs','crates/raft/src/async_apply.rs','crates/raft/src/rawnode.rs','crates/raft/src/state_machine/raw_group.rs','crates/raft/src/write_diagnostics.rs','crates/engine/src/wal.rs','crates/engine/src/wal_segment.rs','crates/engine/src/persist.rs']
    source=[]
    for rev in ['bd42e60f84657e22e36e34924a5c80a08eac623a','9317e63a4b866aad4acf05d569f15e39ded58368','e2e23cca5e70a9ea0cc241877b3b35b5b6433d27']:
        for rel in rels:
            if rev.startswith('bd42') and rel.endswith('write_diagnostics.rs'):continue
            data=subprocess.run(['git','show',f'{rev}:{rel}'],cwd=REPO,check=True,capture_output=True).stdout
            assert len(data)<1024**2
            source.append(dict(repository=str(REPO),revision=rev,path=rel,bytes=len(data),sha256=hashlib.sha256(data).hexdigest()))
    result['source_blobs']=source
    (OUT/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
    INPUTS[str(Path(__file__).resolve())]=dict(bytes=Path(__file__).stat().st_size,sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest())
    (OUT/'input-hashes.json').write_text(json.dumps(INPUTS,indent=2)+'\n')
    print(json.dumps(dict(complete=True,timed_rows=len(rows),observer_rows=sum(len(x['rows']) for x in result['observers']),inputs=len(INPUTS),source_blobs=len(source),summary_bytes=(OUT/'summary.json').stat().st_size,summary_sha256=hashlib.sha256((OUT/'summary.json').read_bytes()).hexdigest())))

if __name__=='__main__':main()
