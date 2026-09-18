#!/usr/bin/env python3
"""Before/after measurement for proposal batching (#20 slice), against the
PREDECLARED targets recorded in the increment workdir BEFORE any timed run:

  T1  C=32 write throughput: batched (OPS=16, DELAY=2ms) >= 1.4x unbatched.
  T2  raft-log record-sync count over the C=32 window reduced >= 2.0x.
  T3  C=1 p99 latency: batched <= unbatched p99 + 25ms.

Identical single-host fixture both sides (3 voters, local storage, same
binary, same durability): only the batching env differs. Every raw
number is retained; a missed target is published as a failed target,
never re-aimed. This script makes NO scaling or multi-host claims.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def fields(text):
    return dict(line.split('=', 1) for line in text.splitlines() if '=' in line)


class Cluster:
    def __init__(self, binary, base, base_port, env):
        self.binary, self.base, self.base_port = binary, base, base_port
        self.env = env
        self.addresses = {i: f'127.0.0.1:{base_port+i}' for i in (1, 2, 3)}
        self.processes = {}
        self.handles = []

    def command(self, label, *arguments):
        result = subprocess.run([str(self.binary), *map(str, arguments)], env=self.env,
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                timeout=300)
        (self.base / f'{label}.log').write_text(result.stdout)
        require(result.returncode == 0, f'{label}: {result.stdout[-400:]}')
        return dict(word.split('=', 1) for word in result.stdout.split() if '=' in word)

    def start(self, node):
        log = (self.base / f'node-{node}.log').open('a')
        self.handles.append(log)
        process = subprocess.Popen(
            [str(self.binary), 'start', '--node-id', str(node), '--addr', self.addresses[node],
             '--data-dir', str(self.base / f'n{node}')],
            env=self.env, stdout=log, stderr=subprocess.STDOUT)
        self.processes[node] = process

    def status(self, node):
        path = self.base / f'n{node}/status'
        if not path.exists() or node not in self.processes:
            return {}
        value = fields(path.read_text())
        return value if value.get('pid') == str(self.processes[node].pid) else {}

    def wait(self, label, predicate, seconds=90):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(0.05)
        raise RuntimeError(f'timed out: {label}')

    def boot(self):
        incarnations = []
        for node in (1, 2, 3):
            guard = socket.socket()
            guard.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            guard.bind(('127.0.0.1', self.base_port + node))
            prepared = self.command(f'prepare-{node}', 'store-prepare', '--node-id', node,
                                    '--data-dir', self.base / f'n{node}')
            incarnations.append(f"{node}={prepared['store_incarnation']}")
            guard.close()
        root_file = self.base / 'root.bin'
        self.command('root-create', 'root-create', '--output', root_file,
                     '--voters', ','.join(f'{n}@{self.addresses[n]}' for n in (1, 2, 3)),
                     '--store-incarnations', ','.join(incarnations))
        for node in (1, 2, 3):
            self.command(f'init-{node}', 'init', '--root', root_file, '--node-id', node,
                         '--data-dir', self.base / f'n{node}')
        for node in (1, 2, 3):
            self.start(node)
        leader = self.wait('leader', lambda: next(
            (n for n in self.processes if self.status(n).get('role') == 'leader'
             and self.status(n).get('endpoint_ready') == 'true'), None))
        return leader

    def sync_counts(self):
        counts = {}
        for node in (1, 2, 3):
            doc = json.loads((self.base / f'n{node}/metrics.json').read_text())
            total = 0
            for metric in doc.get('metrics', []):
                if metric.get('name') == 'raft_wal_record_sync':
                    for outcome in metric.get('latency', {}).get('outcomes', []):
                        if outcome.get('outcome') == 'success':
                            total += int(outcome.get('count', 0))
            counts[node] = total
        return counts

    def shutdown(self):
        for node in list(self.processes):
            process = self.processes.pop(node)
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
        for handle in self.handles:
            handle.close()


def run_side(name, binary, workload, build_manifest, out, base_port, batch_env, measure_ms):
    results = {}
    for offset, (trial, workers, ms) in enumerate(
            (('c32', 32, measure_ms), ('c1', 1, max(measure_ms // 2, 2000)))):
        results[trial] = run_trial(name, trial, workers, ms, binary, workload,
                                   build_manifest, out, base_port + offset * 10, batch_env)
    return results


def run_trial(name, trial, workers, ms, binary, workload, build_manifest, out,
              base_port, batch_env):
    # One FRESH cluster per trial: no cross-trial state, and the sync
    # counters cover exactly this trial's window from a clean boot.
    base = out / name / trial
    base.mkdir(parents=True)
    env = dict(os.environ, KV9_CLUSTER_TOKEN=f'bench-{name}-{trial}-cluster',
               KV9_CLIENT_TOKENS=f'admin=bench-{name}-client',
               KV9_CLIENT_TOKEN=f'bench-{name}-client',
               KV9_BOOTSTRAP_TOKEN=f'bench-{name}-bootstrap', **batch_env)
    cluster = Cluster(binary, base, base_port, env)
    try:
        leader = cluster.boot()
        receipt = cluster.command('create-keyspace', 'client', 'create-keyspace',
                                  '--addr', cluster.addresses[leader],
                                  '--name', f'bench-{name}-{trial}', '--api-type', 'raw')
        keyspace = int(receipt['keyspace_id'])
        if True:
            directory = base
            config = dict(
                version=1, rpc_transport='tonic_stream',
                client=dict(version=1,
                            peers=[dict(node_id=n, address=a)
                                   for n, a in cluster.addresses.items()],
                            keyspace_id=keyspace, epoch_conf_ver=1, epoch_version=1,
                            max_in_flight=workers, max_attempts=6, deadline_ms=1500,
                            retry_backoff_ms=5),
                mode='performance', run_id=f'{name}-{trial}', keyspace_name=f'bench-{name}',
                seed=41, workers=workers, keys=8, value_bytes=128,
                mix=dict(get=0, put=100, delete=0), warmup_operations=32,
                max_operations=1_000_000, measure_ms=ms, interval_ms=0, history_bytes=0)
            (directory / 'config.json').write_text(json.dumps(config, indent=2))
            before = cluster.sync_counts()
            log = (directory / 'workload.log').open('w')
            process = subprocess.Popen(
                [str(workload), '--config', directory / 'config.json',
                 '--build-manifest', build_manifest, '--output', directory / 'run'],
                env=env, stdout=log, stderr=subprocess.STDOUT)
            code = process.wait(timeout=ms / 1000 + 360)
            log.close()
            require(code == 0, f'{name}/{trial} workload failed')
            after_deadline = time.monotonic() + 15
            after = cluster.sync_counts()
            while time.monotonic() < after_deadline:
                nxt = cluster.sync_counts()
                if nxt == after:
                    break
                after = nxt
                time.sleep(1)
            report = json.loads((directory / 'run/report.json').read_text())
            return dict(report=report,
                        raft_sync_before=before, raft_sync_after=after,
                        raft_sync_delta=sum(after.values()) - sum(before.values()))
    finally:
        cluster.shutdown()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin', type=Path, required=True)
    parser.add_argument('--workload-dir', type=Path, required=True,
                        help='build-workload.py output (kv9-workload + build.json)')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--measure-ms', type=int, default=8000)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    binary = args.bin.resolve()
    workload = args.workload_dir.resolve() / 'kv9-workload'
    manifest = args.workload_dir.resolve() / 'build.json'
    record = dict(
        schema=1,
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        workload_sha256=hashlib.sha256(workload.read_bytes()).hexdigest(),
        revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
        measure_ms=args.measure_ms,
        predeclared_targets=dict(
            t1_throughput_ratio_min=1.4,
            t2_fsync_reduction_min=2.0,
            t3_p99_overhead_ms_max=25.0),
    )
    sides = {}
    sides['unbatched'] = run_side(
        'unbatched', binary, workload, manifest, out, 27140,
        dict(KV9_PROPOSAL_BATCH_OPS='0'), args.measure_ms)
    sides['batched'] = run_side(
        'batched', binary, workload, manifest, out, 27160,
        dict(KV9_PROPOSAL_BATCH_OPS='16', KV9_PROPOSAL_BATCH_DELAY_MS='2'),
        args.measure_ms)

    def throughput(side, trial):
        report = sides[side][trial]['report']
        rate = report.get('cohort_success_ops_per_second')
        require(rate, f'{side}/{trial}: missing success rate')
        return rate

    def p99(side, trial):
        # Measurement-phase logical latency for the put operation, success
        # outcome; the p99 bucket's upper bound in milliseconds.
        report = sides[side][trial]['report']
        metrics = report['metrics']
        put = next(i for i, op in enumerate(metrics['operations'])
                   if 'put' in str(op).lower())
        phase = metrics['logical_latency'][2][put]
        require(phase.get('valid'), f'{side}/{trial}: latency snapshot invalid')
        histogram = next(h for h in phase['outcomes'] if str(h.get('outcome', '')).lower() == 'success')
        bounds = histogram.get('p99')
        require(bounds, f'{side}/{trial}: missing p99 population')
        return bounds['upper_ns'] / 1e6

    t1_ratio = throughput('batched', 'c32') / max(throughput('unbatched', 'c32'), 1e-9)
    unbatched_sync = max(sides['unbatched']['c32']['raft_sync_delta'], 1)
    batched_sync = max(sides['batched']['c32']['raft_sync_delta'], 1)
    # Normalize per operation: sync count per completed op, then the ratio.
    per_op_unbatched = unbatched_sync / max(sides['unbatched']['c32']['report']['measured_successful'], 1)
    per_op_batched = batched_sync / max(sides['batched']['c32']['report']['measured_successful'], 1)
    t2_ratio = per_op_unbatched / max(per_op_batched, 1e-9)
    t3_overhead = p99('batched', 'c1') - p99('unbatched', 'c1')
    verdicts = dict(
        t1=dict(ratio=t1_ratio, target=1.4, passed=t1_ratio >= 1.4),
        t2=dict(per_op_unbatched=per_op_unbatched, per_op_batched=per_op_batched,
                ratio=t2_ratio, target=2.0, passed=t2_ratio >= 2.0),
        t3=dict(overhead_ms=t3_overhead, target_ms=25.0, passed=t3_overhead <= 25.0),
    )
    record.update(sides={k: v for k, v in sides.items()}, verdicts=verdicts,
                  accepted=all(v['passed'] for v in verdicts.values()))
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    for name, verdict in verdicts.items():
        print(f'{name}: {"PASS" if verdict["passed"] else "FAIL"} {verdict}')
    print(f'BENCH_{"ACCEPTED" if record["accepted"] else "FAILED"}')


if __name__ == '__main__':
    main()
