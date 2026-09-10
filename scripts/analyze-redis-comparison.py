#!/usr/bin/env python3
"""Report outcome certainty and residual server jobs separately from timed client completion."""
import argparse
from pathlib import Path

from benchmark import read, save


def analyze(root):
    matrix = read(root/'matrix.json',16*1024*1024)
    rows = []
    for entry in matrix['attempts']:
        if entry['target'] != 'kv9' or not entry['complete']:
            continue
        directory = root/'kv9'/entry['name']
        report = read(directory/'run/report.json')
        phase = report['metrics']['phases'].index('measure')
        families = dict(acknowledged=0,unknown_write=0,pre_execution_admission_refusal=0,
                        not_leader_refusal=0,read_failure=0,client_rejection=0)
        per_operation = {}
        for op, latency, populations, rpc_codes in zip(('get','put','delete'),
                report['metrics']['logical_latency'][phase],report['metrics']['logical_counts'][phase],
                report['metrics']['logical_rpc_codes'][phase]):
            counts = {h['outcome']:h['count'] for h in latency['outcomes']}
            reasons = dict(zip(report['metrics']['populations'],populations))
            refused = sum(reasons[k] for k in ('admission_count','admission_bytes','admission_oversize'))
            if counts['aborted'] or counts['replaced'] or counts['rejected'] != refused + reasons['not_leader']:
                raise ValueError('logical outcome certainty cannot be classified exactly')
            mapped = dict(acknowledged=counts['success'],unknown_write=counts['unconfirmed'],
                          pre_execution_admission_refusal=refused,not_leader_refusal=reasons['not_leader'],
                          read_failure=counts['error'],client_rejection=counts['released'])
            for name,count in mapped.items(): families[name] += count
            latency_by_outcome = {h['outcome']: {k:h[k] for k in ('count','sum_ns','min_ns','max_ns','p50','p95','p99')}
                                  for h in latency['outcomes'] if h['count']}
            per_operation[op] = dict(families=mapped,reasons=reasons,rpc_status_codes=rpc_codes,
                                     latency_by_outcome=latency_by_outcome)
        after = read(directory/'after-status.json')
        before = read(directory/'before-status.json')
        backend = {}
        for node,status in after.items():
            backend[node] = dict(in_flight=int(status['public_rpc_in_flight']),
                                 running=int(status['public_rpc_running']),queued=int(status['public_rpc_queued']),
                                 encoded_bytes=int(status['public_rpc_encoded_bytes']),
                                 leader_id=status['leader_id'],term=status['term'],role=status['role'],
                                 raw_read=status['public_rpc_raw_read'],raw_write=status['public_rpc_raw_write'])
        if sum(families.values()) != report['measured_completed']:
            raise ValueError('classified outcomes do not cover measured completion')
        rows.append(dict(name=entry['name'],mix=entry['mix'],workers=entry['workers'],repeat=entry['repeat'],
                         success_ops_per_second=entry['summary']['success_ops_per_second'],
                         families=families,per_operation=per_operation,after_verification_backend=backend,
                         residual_backend_jobs=sum(s['in_flight'] for s in backend.values()),
                         before_leaders={n:s['leader_id'] for n,s in before.items()},
                         after_leaders={n:s['leader_id'] for n,s in after.items()}))
    return dict(version=1,matrix_complete=matrix['complete'],rows=rows,
                interpretation='UnknownWrite means the client did not establish the outcome; it is not proof that the write failed. '
                               'Configured client concurrency does not bound outstanding backend jobs after caller cancellation. '
                               'After snapshots are captured after client drain and final verification; they are not atomic across nodes. '
                               'Trials use the same persistent KV9 fixture, so retained background work can affect subsequent trials. '
                               'A fast-refusal population can dominate aggregate terminal latency; acknowledged latency must be reported separately.')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--matrix',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    with args.output.open('x') as stream: stream.write('{"complete": false}\n')
    result=analyze(args.matrix); save(args.output,result)
    print(f"Retained outcome certainty and backend occupancy for {len(result['rows'])} KV9 trials")


if __name__ == '__main__': main()
