#!/usr/bin/env python3
"""Render paired acknowledged throughput, outcome certainty, and successful-call latency."""
import argparse
import importlib.util
from pathlib import Path

from benchmark import read, save


def module(name, filename):
    spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name(filename))
    loaded=importlib.util.module_from_spec(spec); spec.loader.exec_module(loaded)
    return loaded


comparison=module('comparison','redis-comparison.py')
analysis=module('analysis','analyze-redis-comparison.py')


def aggregate(root, mix, workers, target):
    matrix=read(root/'matrix.json',16*1024*1024)
    entries=[e for e in matrix['attempts'] if e['target']==target and e['mix']==mix and e['workers']==workers]
    successful=sum(e['summary']['successful'] for e in entries)
    elapsed=sum(e['summary']['cohort_elapsed_ns'] for e in entries)
    hist=[]
    for e in entries:
        if target=='kv9':
            report=read(root/target/e['name']/'run/report.json')
            phase=report['metrics']['phases'].index('measure')
            hist.extend(h for op in report['metrics']['logical_latency'][phase] for h in op['outcomes'] if h['outcome']=='success')
        else:
            report=read(root/target/e['name']/'report.json')
            if report['failed']:
                raise ValueError('Redis acknowledgment latency cannot include failed calls')
            hist.extend(h for w in report['workers'] for h in w['latency'])
    latency=comparison.histogram(hist)
    if latency['count'] != successful:
        raise ValueError('successful latency count differs from acknowledgment throughput population')
    return dict(success_ops_per_second=successful*1e9/elapsed,successful=successful,
                unsuccessful=sum(e['summary']['failed'] for e in entries),successful_latency=latency,
                repetitions=[dict(repeat=e['repeat'],success_ops_per_second=e['summary']['success_ops_per_second'],
                                  unsuccessful=e['summary']['failed'],cpu=e['summary']['cpu']) for e in entries])


def write_stages(root):
    entries=read(root/'matrix.json',16*1024*1024)['attempts']
    trial=next(e for e in entries if e['target']=='kv9' and e['mix']=='write' and e['workers']==min(x['workers'] for x in entries) and e['repeat']==0)
    directory=root/'kv9'/trial['name']
    before=read(directory/'before-metrics.json'); after=read(directory/'after-metrics.json')
    config=read(directory/'requested-config.json'); report=read(directory/'run/report.json')
    writes=config['keys']+1+config['warmup_operations']+report['measured_successful']
    nodes={}
    for node, document in after.items():
        earlier={m['name']:m for m in before[node]['metrics']}; deltas={}
        for metric in document['metrics']:
            if metric['name'] not in ('raft_wal_record_sync','engine_wal_record_sync','public_raw_write_backend'):
                continue
            old=earlier[metric['name']]['latency']['outcomes']; new=metric['latency']['outcomes']
            count=sum(h['count'] for h in new)-sum(h['count'] for h in old)
            total=sum(h['sum_ns'] for h in new)-sum(h['sum_ns'] for h in old)
            deltas[metric['name']]=dict(count=count,sum_ns=total,mean_ns=total/count if count else None)
        nodes[node]=deltas
    return dict(trial=trial['name'],workers=trial['workers'],acknowledged_writes_including_setup_and_warmup=writes,
                unsuccessful_measured=trial['summary']['failed'],nodes=nodes,
                scope='Before/after exporter snapshots encompass initialization, warmup, measurement, drain, and final reads; '
                      'stage means are not isolated measured-only averages and counters are independent between metrics.')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('baseline','candidate','baseline-check','candidate-check','output'):
        parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args(); args.output.mkdir(parents=True,exist_ok=False)
    paths={'baseline':args.baseline,'candidate':args.candidate}
    checks={'baseline':args.baseline_check,'candidate':args.candidate_check}
    protocols={name:read(path/'protocol.json') for name,path in paths.items()}
    comparable=[]; checked={}
    for name,path in paths.items():
        check=read(checks[name],16*1024*1024)
        checked[name]=check
        if not check['complete'] or check['matrix_sha256']!=comparison.sha(path/'matrix.json') or check['revision']!=protocols[name]['revision']:
            raise ValueError('paired report requires a fresh successful independent matrix recheck')
        common=dict(protocols[name]); del common['revision']; comparable.append(common)
    if comparable[0]!=comparable[1] or read(args.baseline/'driver-sources.json')!=read(args.candidate/'driver-sources.json'):
        raise ValueError('paired protocols or exact driver source hashes differ')
    placement=read(args.baseline/'matrix.json',16*1024*1024)['placement']
    if placement!=read(args.candidate/'matrix.json',16*1024*1024)['placement']:
        raise ValueError('paired CPU placement differs')
    outcomes={name:analysis.analyze(path) for name,path in paths.items()}
    rows=[]
    for mix in ('read','write','mixed'):
        for workers in protocols['baseline']['concurrency']:
            row=dict(mix=mix,workers=workers)
            for name,path in paths.items():
                row[name]=aggregate(path,mix,workers,'kv9')
                row[name]['outcomes']=[e for e in outcomes[name]['rows'] if e['mix']==mix and e['workers']==workers]
                row[name+'_redis']=aggregate(path,mix,workers,'redis-memory')
            row['acknowledgment_rate_ratio']=row['candidate']['success_ops_per_second']/row['baseline']['success_ops_per_second']
            rows.append(row)
    stages=write_stages(args.candidate)
    result=dict(version=1,complete=True,protocols=protocols,rows=rows,durable_write_stages=stages,
                matrices={name:dict(path=str(path),sha256=comparison.sha(path/'matrix.json'),check_sha256=comparison.sha(checks[name])) for name,path in paths.items()},
                interpretation=outcomes['baseline']['interpretation'])
    save(args.output/'paired.json',result)
    protocol=protocols['baseline']; highest=max(protocol['concurrency'])
    lines=['# Redis reference and KV9 scheduling comparison','',
           f"Baseline: `{protocols['baseline']['revision']}`. Candidate: `{protocols['candidate']['revision']}`.",'',
           f"The complete baseline ({checked['baseline']['trials']} trials) and candidate ({checked['candidate']['trials']} trials) matrices passed independent report rechecking. Measurement completion does not mean that every logical call was acknowledged.", '',
           'This is a standalone Redis memory reference (`save ""`, `appendonly no`, zero replicas) alongside three WAL-backed KV9 voters with unchanged quorum semantics. It is not an equal-durability comparison.', '',
           f"The paired protocol uses {protocol['keys']} hot keys, 23-byte keys, {protocol['value_bytes']}-byte values, {protocol['repetitions']} repetitions, {protocol['measure_ms']}-ms measurement windows, two client runtime threads, and logical concurrency {protocol['concurrency']}. Client CPUs: {placement['client_cpus']}; server CPUs: {placement['server_cpus']}. Host and filesystem inventories are retained in each matrix. Logical CPU affinity is not exclusive physical-core or host isolation. Every trial retains its full latency histograms, CPU samples, outcomes, and server snapshots.", '',
           'Rates below pool acknowledged operations over the sum of both measured cohort durations, including drain. The ratio is descriptive; overloaded rows must be read with their non-acknowledgment counts.', '',
           '| Mix | Concurrency | Baseline KV9 ops/s | Candidate KV9 ops/s | Ratio | Baseline non-acknowledged | Candidate non-acknowledged | Redis reference ops/s (baseline / candidate run) |',
           '|---|---:|---:|---:|---:|---:|---:|---:|']
    for r in rows:
        a,b=r['baseline'],r['candidate']
        lines.append(f"| {r['mix']} | {r['workers']} | {a['success_ops_per_second']:.1f} | {b['success_ops_per_second']:.1f} | {r['acknowledgment_rate_ratio']:.2f}x | {a['unsuccessful']} | {b['unsuccessful']} | {r['baseline_redis']['success_ops_per_second']:.0f} / {r['candidate_redis']['success_ops_per_second']:.0f} |")
    lines += ['', '## Outcome certainty and residual work', '',
              '| Revision | Trial | Acknowledged | UnknownWrite | Admission refused | NotLeader refused | Read failure | Client rejected | Backend jobs after verification |',
              '|---|---|---:|---:|---:|---:|---:|---:|---:|']
    for name,data in outcomes.items():
        for r in data['rows']:
            f=r['families']
            if sum(v for k,v in f.items() if k!='acknowledged') or r['residual_backend_jobs']:
                lines.append(f"| {name} | {r['name']} | {f['acknowledged']} | {f['unknown_write']} | {f['pre_execution_admission_refusal']} | {f['not_leader_refusal']} | {f['read_failure']} | {f['client_rejection']} | {r['residual_backend_jobs']} |")
    lines += ['',outcomes['baseline']['interpretation'],'',
              'Workers can issue new logical operations after fast explicit refusals. They never retry a prior uncertain write. Where refusals dominate, aggregate terminal percentiles largely describe refusals; the following percentiles include acknowledged calls only. Values are histogram intervals, not exact percentile estimates.', '',
              f'## Acknowledged latency at concurrency {highest}','',
              '| Mix | Baseline mean ms | Candidate mean ms | Baseline p99 interval ms | Candidate p99 interval ms |',
              '|---|---:|---:|---|---|']
    def interval(h): return '–'.join(f'{v/1e6:.3f}' for v in h['p99_ns'])
    for r in rows:
        if r['workers']==highest:
            a,b=r['baseline']['successful_latency'],r['candidate']['successful_latency']
            lines.append(f"| {r['mix']} | {a['mean_ns']/1e6:.3f} | {b['mean_ns']/1e6:.3f} | {interval(a)} | {interval(b)} |")
    lines += ['', '## Durable-write stage sample', '',
              f"Candidate pure-write trial `{stages['trial']}` has {stages['acknowledged_writes_including_setup_and_warmup']} acknowledged writes including initialization and warmup, and {stages['unsuccessful_measured']} unsuccessful measured calls. {stages['scope']}", '',
              '| Voter | Raft WAL sync calls | Mean Raft sync ms | Engine WAL sync calls | Mean engine sync ms |',
              '|---|---:|---:|---:|---:|']
    for node,data in stages['nodes'].items():
        raft,engine=data['raft_wal_record_sync'],data['engine_wal_record_sync']
        lines.append(f"| {node} | {raft['count']} | {raft['mean_ns']/1e6:.3f} | {engine['count']} | {engine['mean_ns']/1e6:.3f} |")
    lines += ['', 'In this sample, the recorded sync counts expose the durable I/O cost per acknowledged write. They are observations of this snapshot interval, not a universal fixed-count claim across elections, migrations, or future group-commit implementations.','']
    lines += ['', '## Retained evidence', '']
    for name,path in paths.items():
        build=read(path/'kv9-build.json')
        lines += [f"- {name}: `{path}`; matrix SHA-256 `{comparison.sha(path/'matrix.json')}`.",
                  f"- {name} independent recheck: `{checks[name]}`; SHA-256 `{comparison.sha(checks[name])}`.",
                  f"- {name} release database SHA-256: `{build['binaries']['kv9']['sha256']}`; source tree SHA-256 `{build['source_tree_sha256']}`."]
    lines += [f"- Redis: {protocol['redis_version']}; server SHA-256 `{protocol['redis_server_sha256']}`.",
              f"- Redis reference client SHA-256: `{protocol['redis_client_sha256']}`.", '',
              f"The full paired artifact is retained at `{args.output/'paired.json'}` (SHA-256 `{comparison.sha(args.output/'paired.json')}`). It includes per-repetition CPU and throughput, pooled successful latency histograms, exact certainty families, and per-node backend occupancy. The local fixture, native Redis client, independent rechecker, outcome analyzer, and paired renderer are versioned together. Hosted CI was not invoked.",'']
    (args.output/'report.md').write_text('\n'.join(lines))
    print('PASS: rendered paired acknowledged throughput and outcome-aware latency')


if __name__=='__main__': main()
