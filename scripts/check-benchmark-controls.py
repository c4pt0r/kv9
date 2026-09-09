#!/usr/bin/env python3
"""Reject isolated false claims in retained benchmark evidence; revalidate the original."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from benchmark_report import read, validate_matrix


def save(path,value): path.write_text(json.dumps(value,indent=2)+'\n')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--evidence',type=Path,required=True)
    p.add_argument('--build',type=Path,required=True)
    p.add_argument('--output',type=Path,required=True)
    a=p.parse_args()
    if a.output.resolve().is_relative_to(a.evidence.resolve()): raise ValueError('controls must be outside original evidence')
    a.output.mkdir(parents=True,exist_ok=False)
    baseline=validate_matrix(a.evidence,a.build)
    manifest=dict(version=1,source_sha256={name:hashlib.sha256((Path(__file__).parent/name).read_bytes()).hexdigest()
        for name in ('benchmark_report.py','check-benchmark-controls.py','workload_report.py')},controls=[])
    cases=[('missing-trial','trial matrix omits or duplicates a cell'),
           ('missing-calibration','benchmark matrix omits or duplicates a fixture'),
           ('premature-stop','performance trial stopped before its required duration'),
           ('false-throughput','reported throughput differs from its cohort'),
           ('stale-server-metrics','server metric snapshot is stale'),
           ('false-calibration-count','calibration endpoint accounting differs from actual client attempts'),
           ('durable-calibration','loopback calibration is mislabeled as durable storage'),
           ('false-rollup','stored benchmark summary differs from independent recomputation'),
           ('cpu-ticks-regressed','invalid bounded integer'),
           ('client-cpu-placement','client process or CPU placement differs'),
           ('excluded-operation','trial issued an operation excluded by its workload mix')]
    for name,expected in cases:
        folder=a.output/name
        # WAL/database directories are outside the verifier's evidence inputs.
        # Keep complete workload histories, manifests, snapshots and raw reports.
        shutil.copytree(a.evidence,folder,ignore=shutil.ignore_patterns('data','*.log'))
        fixture=folder/'r0-wal';f=read(fixture/'fixture.json');entry=f['trials'][1]
        if name=='missing-trial':
            f['trials'].pop(1);save(fixture/'fixture.json',f)
        elif name=='missing-calibration':
            index=read(folder/'index.json');index['fixtures'].remove('r0-loopback');save(folder/'index.json',index)
        elif name in ('premature-stop','false-throughput','excluded-operation'):
            report_path=fixture/entry['name']/'run/report.json';report=read(report_path)
            if name=='premature-stop': report['stop']['reason']='operation_limit'
            elif name=='false-throughput': report['cohort_success_ops_per_second']*=2
            else:
                # Preserve internally consistent counts/histograms while falsely
                # claiming that the read-only trial performed writes.
                for field in ('logical_counts','attempt_counts','logical_rpc_codes','attempt_rpc_codes','logical_latency','attempt_latency'):
                    row=report['metrics'][field][2];row[0],row[1]=row[1],row[0]
                row=report['history']['phase_successes'][2];row['get'],row['put']=row['put'],row['get']
            save(report_path,report)
            # Preserve the transport digest so the semantic check must reject
            # the changed duration/rate claim, not merely a stale wrapper hash.
            entry['report_sha256']=hashlib.sha256(report_path.read_bytes()).hexdigest()
            save(fixture/'fixture.json',f)
        elif name=='stale-server-metrics':
            directory=fixture/entry['name'];metrics=read(directory/'before-metrics.json');resources=read(directory/'before-resources.json')
            metrics['1']['captured_unix_ns']=str(resources['requested_unix_ns']-1)
            save(directory/'before-metrics.json',metrics)
        elif name in ('false-calibration-count','durable-calibration'):
            path=folder/'r0-loopback/loopback.json';doc=read(path)
            if name=='false-calibration-count': doc['requests'][0]-=1
            else: doc['durability']=True
            save(path,doc)
        elif name=='cpu-ticks-regressed':
            directory=fixture/entry['name'];before=read(directory/'before-resources.json');after=read(directory/'after-resources.json')
            after['processes']['1']['user_ticks']=before['processes']['1']['user_ticks']-1
            save(directory/'after-resources.json',after)
        elif name=='client-cpu-placement':
            entry['client_identity']['cpu_affinity']=[];save(fixture/'fixture.json',f)
        else:
            path=folder/'report.json';report=read(path,16*1024*1024);report['groups'][0]['median']*=2;save(path,report)
        try: validate_matrix(folder,a.build)
        except ValueError as error:
            if str(error)!=expected: raise ValueError(f'{name}: wrong rejection: {error}') from error
        else: raise ValueError(name+': corrupt benchmark evidence accepted')
        if validate_matrix(a.evidence,a.build)!=baseline: raise ValueError('original benchmark evidence changed')
        manifest['controls'].append(dict(name=name,rejected=True,reason=expected,original_revalidated=True))
        save(a.output/'manifest.json',manifest)
        print('PASS: rejected benchmark evidence control '+name,flush=True)
    print('PASS: 11 corrupt benchmark evidence controls rejected; original matrix revalidated after each',flush=True)


if __name__=='__main__': main()
