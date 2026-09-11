"""Unchanged arithmetic helpers from the accepted v3 statistics reader."""
import collections

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
