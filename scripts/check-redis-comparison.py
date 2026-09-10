#!/usr/bin/env python3
"""Recheck every retained reference-comparison trial without contacting running servers."""
import argparse
import importlib.util
import itertools
from pathlib import Path

from benchmark import MIXES, read, save
from workload_report import validate

spec = importlib.util.spec_from_file_location('redis_comparison', Path(__file__).with_name('redis-comparison.py'))
comparison = importlib.util.module_from_spec(spec)
spec.loader.exec_module(comparison)


def check(matrix_path, build, expected_revision):
    protocol = read(matrix_path/'protocol.json')
    matrix = read(matrix_path/'matrix.json',16*1024*1024)
    manifest = read(build/'build.json')
    if not matrix['complete'] or protocol['revision'] != expected_revision or manifest['revision'] != expected_revision:
        raise ValueError('missing completion or revision mismatch')
    if manifest['dirty'] or manifest['profile'] != 'release' or protocol['comparable_durability'] is not False:
        raise ValueError('invalid build or misleading durability label')
    if comparison.sha(build/'build.json') != comparison.sha(matrix_path/'kv9-build.json'):
        raise ValueError('matrix/build manifest mismatch')
    expected = set(itertools.product(('kv9','redis-memory'),range(protocol['repetitions']),protocol['concurrency'],MIXES))
    seen = set(); rows = []
    for entry in matrix['attempts']:
        identity = (entry['target'],entry['repeat'],entry['workers'],entry['mix'])
        if identity in seen or identity not in expected or not entry['complete']:
            raise ValueError('duplicate, unexpected, or incomplete trial')
        seen.add(identity)
        directory = matrix_path/entry['target']/entry['name']
        if read(directory/'exit.json')['exit_code'] != 0:
            raise ValueError('client exited unsuccessfully')
        config = read(directory/'requested-config.json')
        if (config['workers'],config['keys'],config['value_bytes'],config['seed'],config['measure_ms'],config['warmup_operations']) != (
                entry['workers'],protocol['keys'],protocol['value_bytes'],40+entry['repeat'],protocol['measure_ms'],32):
            raise ValueError('workload configuration differs from the common protocol')
        if config['run_id'] != entry['name']:
            raise ValueError('key prefix differs from the paired trial name')
        if entry['target'] == 'kv9':
            validate(directory/'run',build/'workload',seconds=60)
            if config['mix'] != MIXES[entry['mix']]:
                raise ValueError('KV9 operation mix differs')
            identity_record = read(directory/'client-identity.json')
            if identity_record['executable_sha256'] != read(build/'workload/build.json')['binary_sha256']:
                raise ValueError('KV9 client identity mismatch')
        else:
            if {op:config[op] for op in ('get','put','delete')} != MIXES[entry['mix']]:
                raise ValueError('Redis operation mix differs')
            report = read(directory/'report.json')
            if report['configuration'] != config or report['runtime_threads'] != 2 or report['failed'] != 0:
                raise ValueError('Redis report/configuration differs or contains failures')
            identity_record = read(directory/'redis-client-identity.json')
            if identity_record['executable_sha256'] != protocol['redis_client_sha256'] or identity_record['pid'] != report['process_id']:
                raise ValueError('Redis client identity mismatch')
            redis_config = (directory/'redis.conf').read_text()
            if 'save ""\n' not in redis_config or 'appendonly no\n' not in redis_config or 'replicaof' in redis_config:
                raise ValueError('Redis configuration is not the declared memory reference')
            fields = read(directory/'configuration.json')['stdout'].splitlines()
            if len(fields) % 2:
                raise ValueError('malformed live Redis configuration observation')
            live = dict(zip(fields[::2],fields[1::2]))
            if live.get('save') != '' or live.get('appendonly') != 'no' or live.get('io-threads') != '1':
                raise ValueError('live Redis configuration differs from the declared reference')
            observed = read(directory/'replication.json')['stdout']
            if 'role:master' not in observed or 'connected_slaves:0' not in observed:
                raise ValueError('Redis observed replication state differs')
        summary = comparison.summarize(directory,entry['target'])
        if summary != read(directory/'summary.json') or summary != entry['summary']:
            raise ValueError('fresh summary differs from retained matrix summary')
        rows.append(dict(summary,mix=entry['mix'],workers=entry['workers'],repeat=entry['repeat']))
    if seen != expected:
        raise ValueError('matrix has missing trials')
    for repeat, workers, mix in itertools.product(range(protocol['repetitions']),protocol['concurrency'],MIXES):
        paired = [e for e in matrix['attempts'] if (e['repeat'],e['workers'],e['mix']) == (repeat,workers,mix)]
        if len({e['name'] for e in paired}) != 1:
            raise ValueError('paired key sizes or prefixes differ')
    return dict(version=1,complete=True,revision=expected_revision,trials=len(rows),rows=rows,
                measured_failures=sum(row['failed'] for row in rows),
                matrix_sha256=comparison.sha(matrix_path/'matrix.json'),protocol_sha256=comparison.sha(matrix_path/'protocol.json'))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--matrix',type=Path,required=True)
    parser.add_argument('--build',type=Path,required=True)
    parser.add_argument('--expected-revision',required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    with args.output.open('x') as stream:
        # Reserve a distinct verifier attempt before processing; preserve failures.
        stream.write('{"complete": false}\n')
    result=check(args.matrix,args.build,args.expected_revision)
    save(args.output,result)
    print(f"PASS: independently rechecked {result['trials']} reference-comparison trials")


if __name__ == '__main__': main()
