#!/usr/bin/env python3
"""Reconstruct bytes and allocator effects independently; enforce the declared write gate."""
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

S=Path(sys.argv[1]).resolve();mode=sys.argv[2]
assert mode in ('prepare','counting','final')
load=lambda p:json.loads(p.read_text());sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
plan=load(S/'plan.json');build=load(S/'build-summary.json');declared=load(S/'declared-plan.json')
assert build['complete'] and build['plan_sha256']==sha(S/'plan.json')
assert plan['declared_plan_sha256']==sha(S/'declared-plan.json')
assert plan['gates']==dict(original_mean_percent=-10,original_p99_percent=2,original_mean_improves_both_orders=True,wide128_mean_percent=2,wide128_p99_percent=2)
for p,h in build['sources'].items():assert sha(Path(p))==h,p

def digest(rows):
    h=hashlib.sha256();h.update(struct.pack('<Q',len(rows)))
    for k,v in rows:
        h.update(struct.pack('<Q',len(k)));h.update(k);h.update(struct.pack('<Q',len(v)));h.update(v)
    return h.hexdigest()

raw=(S/'groups.bin').read_bytes();assert hashlib.sha256(raw).hexdigest()==plan['corpus_sha256']==declared['corpus']['sha256']
assert raw[:8]==b'KV9GRP01'
pos=8;previous_end=14221;groups=[]
while pos<len(raw):
    previous,last,term,length=struct.unpack_from('<QQQI',raw,pos);pos+=28;end=pos+length
    assert previous==previous_end and last>previous and term==1;previous_end=last
    count=struct.unpack_from('<I',raw,pos)[0];pos+=4;group=[]
    for _ in range(count):
        tag,cf,n=struct.unpack_from('<BBI',raw,pos);pos+=6;assert tag==cf==0 and n==27
        key=raw[pos:pos+n];pos+=n;n=struct.unpack_from('<I',raw,pos)[0];pos+=4
        value=raw[pos:pos+n];pos+=n;assert n==128 and pos<=end;group.append((key,value))
    assert pos==end;groups.append(group)
assert len(groups)==106 and sum(map(len,groups))==100096 and previous_end==15785
models={};metadata=[];transformed={}
for dataset in ('original','wide128'):
    for workload in ('overwrite','unique_insert'):
        final={};gs=[];ordinal=0
        for group in groups:
            current=[]
            for key,value in group:
                ordinal+=1
                if dataset=='wide128':key=key+b'\0'*(128-len(key))
                if workload=='unique_insert':key+=ordinal.to_bytes(8,'big')
                final[key]=value;current.append((key,value))
            gs.append(current)
        rows=sorted(final.items());models[dataset,workload]=rows;transformed[dataset,workload]=gs
        assert len(rows)==(4096 if workload=='overwrite' else 100096)
        metadata.append(dict(dataset=dataset,workload=workload,groups=106,initial_keys=4096 if workload=='overwrite' else 0,final_keys=len(rows),final_state_sha256=digest(rows)))

def probes(rows,miss):
    state=71;mask=(1<<64)-1;perm=list(range(len(rows)))
    for i in range(len(perm)-1,0,-1):
        bound=i+1;threshold=((1<<64)-bound)%bound
        while True:
            state^=(state<<13)&mask;state^=state>>7;state^=(state<<17)&mask
            if state>=threshold:break
        j=state%bound;perm[i],perm[j]=perm[j],perm[i]
    keys={k for k,v in rows};out=[]
    for i in perm[:512]:
        key=rows[i][0]
        if miss:
            key+=b'\xff'
            while key in keys:key+=b'\xff'
        out.append(key)
    assert len(set(out))==512
    return out

terminals=[]
def payload(name,profile):
    p=S/'runs'/(name+'.json');t=load(S/'runs'/(name+'-terminal.json'))
    assert t['complete'] and t['exit_code']==0 and t['profile']==profile
    assert t['result_sha256']==sha(p) and t['result_bytes']==p.stat().st_size<=plan['process_output_max_bytes']
    binary=build['arms'][t['arm']][profile]
    assert t['binary_sha256']==sha(Path(binary['path']))==binary['sha256']
    assert t['ended_ns']>=t['started_ns']
    stat=Path(f'/proc/{t["pid"]}/stat')
    if stat.exists():assert int(stat.read_text().rsplit(')',1)[1].split()[19])!=t['start_ticks']
    terminals.append(t);x=load(p)
    assert x['complete'] and x['input_plan_sha256']==sha(S/'plan.json') and x['corpus_sha256']==sha(S/'groups.bin')
    assert x['workloads']==metadata and x.get('counting_build',False)==(profile=='counting')
    return x
def summary(stage,n):
    x=load(S/(stage+'-summary.json'));assert x['complete'] and len(x['processes'])==n
    assert x['plan_sha256']==sha(S/'plan.json')
    for p,h in x['tools'].items():assert sha(Path(p))==h
    for t in x['processes']:assert t==load(S/'runs'/(t['name']+'-terminal.json'))
    return x

prepared={};canonical=[];query_identity=[]
for profile in ('timing','counting'):
    for arm in ('baseline','candidate'):
        name=f'prepare-{profile}-{arm}';x=payload(name,profile)
        assert x['mode']=='prepare'
        assert x['checks']==[dict(dataset=d,workload=w,pinned=p,live_prefixes=106,old_views=106 if p else 0,position_checks=106,refusal_checks=212,other_cfs_checked=True)
                             for d,w in models for p in (False,True)]
        assert len(x['read_inputs'])==4
        identity=[]
        for entry,(d,w) in zip(x['read_inputs'],models):
            rows=models[d,w];assert entry['dataset']==d and entry['workload']==w
            assert entry['keys_hex']==[k.hex() for k,v in rows]
            assert [q['operation'] for q in entry['queries']]==['get_hit','get_miss']
            for q in entry['queries']:
                expected=probes(rows,q['operation']=='get_miss')
                assert q['queries_hex']==[k.hex() for k in expected] and q['query_sha256']==digest([(k,b'') for k in expected])
                identity.append(dict(dataset=d,workload=w,operation=q['operation'],sha256=q['query_sha256']))
        if query_identity:assert query_identity==identity
        query_identity=identity;prepared[name]=sha(S/'runs'/(name+'.json'))
        x.pop('counting_build',None);canonical.append(x)
assert all(x==canonical[0] for x in canonical)
accepted=dict(complete=True,models=metadata,prepared_sha256=prepared,live_prefixes=3392,old_views=1696,refusal_checks=6784,independent_reconstruction=True,query_identities=query_identity)
summary('prepare',4)
if mode=='prepare':
    with (S/'inputs-accepted.json').open('x') as f:json.dump(accepted,f,indent=2)
    print(json.dumps({k:v for k,v in accepted.items() if k not in ('models','prepared_sha256','query_identities')}));sys.exit(0)
assert load(S/'inputs-accepted.json')==accepted

def metric(samples):
    assert samples and all(type(x) is int and x>0 for x in samples)
    ordered=sorted(samples);n=len(samples)
    return dict(windows=n,sum_ns=sum(samples),mean_ns=sum(samples)/n,**{f'p{q}_ns':ordered[math.ceil(n*q/100)-1] for q in (50,95,99)})
def row_check(row,spec,counting):
    rows=models[spec['dataset'],spec['workload']]
    assert row['phase']=='apply_group' and row['passes']==12 and row['mutations']==1201152
    assert row['applied_index']==row['data_revision']==106 and row['other_cfs_checked']
    assert row['final_keys']==len(rows) and row['final_state_sha256']==digest(rows)
    assert (row['operation_window'] if counting else row['unit'])=='ns_per_group'
    m=row['metrics'];raw=m['window_records' if counting else 'samples_ns']
    assert len(raw)==m['windows']==1272
    if not counting:
        assert {k:v for k,v in m.items() if k!='samples_ns'}==metric(raw);return raw
    assert row['unit']=='allocator_counts_per_window' and m['elapsed_time_recorded'] is False
    assert set(m)=={'windows','allocation_counts','maximum_window_extra_live_bytes','sum_window_live_delta_bytes','window_records','elapsed_time_recorded'}
    for counts,delta,peak in raw:
        assert len(counts)==6 and all(type(x) is int and x>=0 for x in counts)
        assert counts[2:4]==[0,0]
        assert delta==counts[1]-counts[5] and max(0,delta)<=peak<=counts[1]
    assert m['allocation_counts']==[sum(r[0][i] for r in raw) for i in range(6)]
    assert m['maximum_window_extra_live_bytes']==max(r[2] for r in raw)
    assert m['sum_window_live_delta_bytes']==sum(r[1] for r in raw)
    footprint=row['index_footprint']
    assert footprint['default_keys']==len(rows) and footprint['sentinel_keys']==2
    assert footprint['requested_bytes']>0 and footprint['fully_reclaimed'] and not footprint['includes_stack_bytes'] and not footprint['elapsed_time_recorded']
    return raw

summary('counting',16);cells=[]
for case,spec in enumerate(plan['cases']):
    result={};raws={}
    for order,arm in enumerate(('baseline','candidate')):
        x=payload(f'case-{case:02d}-counting-{order}-{arm}','counting')
        assert (x['mode'],x['case'],x['order'],x['backend'],x['spec'])==('measure',case,order,arm,spec)
        assert len(x['rows'])==1;result[arm]=x['rows'][0];raws[arm]=row_check(result[arm],spec,True)
    dataset,workload=spec['dataset'],spec['workload'];key_len=len(models[dataset,workload][0][0])
    assert key_len in (27,35,128,136)
    cost_delta=-24  # One smaller Entry for every constructed key/value pair.
    for i,(b,c) in enumerate(zip(raws['baseline'],raws['candidate'])):
        group=transformed[dataset,workload][i%106];n=len(group)
        freed=0 if workload=='unique_insert' else n-(len({k for k,v in group}) if spec['pinned'] else 0)
        bc,cc=b[0],c[0]
        expected=[bc[0]-n,bc[1]+n*cost_delta,0,0,bc[4]-freed,bc[5]+freed*cost_delta]
        assert cc==expected,(case,i,cc,expected)
        assert c[1]-b[1]==(n-freed)*cost_delta
    footprints={arm:result[arm]['index_footprint']['requested_bytes'] for arm in result}
    assert footprints['candidate']-footprints['baseline']==(len(models[dataset,workload])+2)*cost_delta
    cells.append(dict(case=case,spec=spec,window_effects_verified=1272,index_footprint_requested_bytes=footprints,
                      arms={arm:{k:v for k,v in row['metrics'].items() if k!='window_records'} for arm,row in result.items()}))
allocation=dict(complete=True,accepted=True,rows=16,cells=cells,windows_compared=8*1272,elapsed_time_recorded=False,input_acceptance=accepted,
                limits='Rust requested bytes, not physical jemalloc/RSS. Net window bytes include input destruction; final-index footprint is observed separately with full reclamation.')
if mode=='counting':
    with (S/'allocation-analysis.json').open('x') as f:json.dump(allocation,f,indent=2)
    print(json.dumps({'accepted':True,'rows':16,'windows_compared':8*1272}));sys.exit(0)
assert load(S/'allocation-analysis.json')==allocation
summary('timing',32);comparisons=[];failures=[]
for case,spec in enumerate(plan['cases']):
    orders={}
    for order,arm in enumerate(plan['orders']):
        x=payload(f'case-{case:02d}-timing-{order}-{arm}','timing')
        assert (x['mode'],x['case'],x['order'],x['backend'],x['spec'])==('measure',case,order,arm,spec) and len(x['rows'])==1
        orders[order]=row_check(x['rows'][0],spec,False)
    baseline=metric(orders[0]+orders[3]);candidate=metric(orders[1]+orders[2])
    delta=lambda b,c:{k:(c[k]/b[k]-1)*100 for k in ('mean_ns','p99_ns')}
    pooled=delta(baseline,candidate);by_order=[delta(metric(orders[b]),metric(orders[c])) for b,c in ((0,1),(3,2))]
    failed=[]
    if pooled['mean_ns']>(-10 if spec['dataset']=='original' else 2):failed.append('pooled_mean')
    if pooled['p99_ns']>2:failed.append('pooled_p99')
    if spec['dataset']=='original' and not all(o['mean_ns']<0 for o in by_order):failed.append('both_order_means')
    row=dict(case=case,spec=spec,unit='ns_per_group',baseline=baseline,candidate=candidate,percent_change=pooled,by_order=by_order,failed_metrics=failed)
    comparisons.append(row)
    if failed:failures.append(dict(case=case,metrics=failed))
assert len(terminals)==52
result=dict(complete=True,processes=52,timing_rows=32,counting_rows=16,comparisons=comparisons,allocations_accepted=True,
            stage_one_gate_passed=not failures,failures=failures,stage_two_status='not run; eligible after write pass' if not failures else 'not run; stopped by declared write gate',
            production_promoted=False,limits='Shared-host engine component, two order directions; not WAL/Raft/RPC/database QPS or Redis comparison.')
with (S/'analysis.json').open('x') as f:json.dump(result,f,indent=2)
print(json.dumps({k:v for k,v in result.items() if k!='comparisons'}))
