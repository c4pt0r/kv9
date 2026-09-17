#!/usr/bin/env python3
"""Single-host multi-group throughput sweep: a development diagnostic.

Runs one three-voter cluster per cell, binds G data groups to G fresh Raw
keyspaces, and drives G concurrent single-keyspace kv9-workload processes.
Aggregate rates sum per-run success counts over per-run measured windows and
report the overlap; per-worker-thread CPU is sampled around measurement.

This is NOT 3/6/9-host scaling evidence, NOT online expansion and NOT a
durability panel; same-CPU processes are not added nodes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def fields(text):
    return dict(line.split('=', 1) for line in text.splitlines() if '=' in line)


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class Cell:
    def __init__(self, args, name, groups, mix, workers_per_group, rep):
        self.args, self.name, self.groups, self.mix, self.wpg, self.rep = \
            args, name, groups, mix, workers_per_group, rep
        self.output = args.output / name
        self.output.mkdir(parents=True)
        self.stores = (args.store_root / f'{args.output.name}-{name}') if args.store_root else self.output
        self.stores.mkdir(parents=True, exist_ok=True)
        self.addresses = {}
        self.processes = {}
        self.handles = []
        self.record = dict(name=name, groups=groups, mix=mix, workers_per_group=workers_per_group,
                           rep=rep, data_workers=args.data_workers, verdict='running')

    def command(self, label, *arguments, token='sweep-client'):
        env = dict(os.environ, KV9_CLIENT_TOKEN=token)
        result = subprocess.run([str(self.args.server), *map(str, arguments)], env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        (self.output / f'{label}.log').write_text(result.stdout)
        require(result.returncode == 0, f'{label}: exit {result.returncode}; see {self.output}')
        return dict(word.split('=', 1) for word in result.stdout.split() if '=' in word)

    def status(self, node):
        path = self.stores / f'n{node}/status'
        if not path.exists():
            return {}
        value = fields(path.read_text())
        return value if value.get('pid') == str(self.processes[node].pid) else {}

    def wait(self, label, predicate, seconds=60):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            value = predicate()
            if value:
                return value
            time.sleep(.05)
        raise RuntimeError(f'timed out: {label}; see {self.output}')

    def leader(self):
        return next((n for n in self.processes
                     if self.status(n).get('role') == 'leader'
                     and self.status(n).get('endpoint_ready') == 'true'), None)

    def data_groups(self, node):
        return {g['region']: g for g in json.loads(self.status(node).get('data_groups', '[]'))}

    def start_cluster(self):
        env = dict(os.environ, KV9_CLUSTER_TOKEN='sweep-cluster',
                   KV9_CLIENT_TOKENS='sweep=sweep-client', KV9_CLIENT_TOKEN='sweep-client',
                   KV9_BOOTSTRAP_TOKEN='sweep-bootstrap',
                   KV9_PUBLIC_MAX_REQUESTS=str(self.args.max_requests))
        guards, incarnations = [], []
        for node in (1, 2, 3):
            guard = socket.socket()
            guard.bind(('127.0.0.1', 0))
            self.addresses[node] = f'127.0.0.1:{guard.getsockname()[1]}'
            guards.append(guard)
        for node in (1, 2, 3):
            prepared = self.command(f'prepare-{node}', 'store-prepare', '--node-id', node,
                                    '--data-dir', self.stores / f'n{node}')
            incarnations.append(f"{node}={prepared['store_incarnation']}")
        root_file = self.output / 'root.bin'
        env_boot = dict(env)
        subprocess.run([str(self.args.server), 'root-create', '--output', str(root_file),
                        '--voters', ','.join(f'{n}@{a}' for n, a in self.addresses.items()),
                        '--store-incarnations', ','.join(incarnations)],
                       env=env_boot, check=True, stdout=subprocess.DEVNULL)
        for node in (1, 2, 3):
            subprocess.run([str(self.args.server), 'init', '--root', str(root_file), '--node-id',
                            str(node), '--data-dir', str(self.stores / f'n{node}')],
                           env=env_boot, check=True, stdout=subprocess.DEVNULL)
        for guard in guards:
            guard.close()
        for node in (1, 2, 3):
            log = (self.output / f'n{node}.log').open('w')
            self.handles.append(log)
            self.processes[node] = subprocess.Popen(
                [str(self.args.server), 'start', '--node-id', str(node),
                 '--addr', self.addresses[node], '--data-dir', str(self.stores / f'n{node}'),
                 '--data-workers', str(self.args.data_workers)],
                env=env, stdout=log, stderr=subprocess.STDOUT)
        self.wait('metadata leader', self.leader)
        self.wait('all endpoints ready',
                  lambda: all(self.status(n).get('endpoint_ready') == 'true' for n in self.processes))

    def bind_groups(self):
        owner = self.leader()
        root = self.status(owner)['root_digest']
        keyspaces = []
        regions = []
        for index in range(self.groups):
            created = self.command(f'group-{index}', 'client', 'create-data-group',
                                   '--addr', self.addresses[owner], '--root-digest', root,
                                   '--operation-id', f'{index + 1:032x}', '--voters', '1,2,3')
            bound = self.command(f'keyspace-{index}', 'client', 'create-data-keyspace',
                                 '--addr', self.addresses[owner], '--root-digest', root,
                                 '--creation-task', created['task_id'], '--name', f'sweep-{index}')
            keyspaces.append(int(bound['keyspace_id']))
            regions.append(int(bound['region_id']))

        def all_active():
            snapshots = [self.data_groups(n) for n in self.processes]
            return all(region in s and s[region]['state'] == 'active'
                       and s[region]['driver_applied'] is not None
                       for region in regions for s in snapshots) and all(
                sum(s.get(region, {}).get('role') == 'Leader' for s in snapshots) == 1
                for region in regions)
        self.wait('all groups active with one leader each', all_active, 120)

        # A group can be active before its public range ownership applies, and
        # workload clients probe peers in order: every node must either serve
        # the read (leader) or refuse with an explicit leader hint (follower),
        # never with a pre-ownership unavailability.
        def routable(keyspace, region):
            for node in self.processes:
                probe = subprocess.run(
                    [str(self.args.server), 'client', 'raw-get', '--addr', self.addresses[node],
                     '--keyspace', str(keyspace), '--key-hex', '70726f6265'],
                    env=dict(os.environ, KV9_CLIENT_TOKEN='sweep-client'),
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)
                if probe.returncode != 0 and 'not_leader=true' not in probe.stdout:
                    return False
            return True
        for keyspace, region in zip(keyspaces, regions):
            self.wait(f'public route for group {region}',
                      lambda k=keyspace, r=region: routable(k, r), 180)
        # Large fleets can still churn leaders right after activation: require
        # a stable leader set across consecutive polls plus a full re-probe.
        def stable():
            first = {r: [n for n in self.processes
                         if self.data_groups(n).get(r, {}).get('role') == 'Leader']
                     for r in regions}
            if any(len(v) != 1 for v in first.values()):
                return False
            time.sleep(1)
            second = {r: [n for n in self.processes
                          if self.data_groups(n).get(r, {}).get('role') == 'Leader']
                      for r in regions}
            return first == second and all(
                routable(k, r) for k, r in zip(keyspaces, regions))
        self.wait('stable leaders across consecutive polls', stable, 180)
        self.record.update(keyspaces=keyspaces, regions=regions)
        return keyspaces

    def thread_cpu(self):
        ticks = {}
        for node, process in self.processes.items():
            for task in Path(f'/proc/{process.pid}/task').iterdir():
                try:
                    comm = (task / 'comm').read_text().strip()
                    stat = (task / 'stat').read_text().split()
                    ticks[f'n{node}/{comm}/{task.name}'] = int(stat[13]) + int(stat[14])
                except OSError:
                    continue
        return ticks

    def run_workloads(self, keyspaces):
        mixes = {'put': dict(get=0, put=100, delete=0), 'get': dict(get=100, put=0, delete=0),
                 'mixed': dict(get=50, put=50, delete=0)}
        launches = []
        for index, keyspace in enumerate(keyspaces):
            config = dict(
                version=1,
                client=dict(version=1,
                            peers=[dict(node_id=n, address=a) for n, a in self.addresses.items()],
                            keyspace_id=keyspace, epoch_conf_ver=1, epoch_version=1,
                            max_in_flight=max(self.wpg, 1), max_attempts=8,
                            deadline_ms=10_000, retry_backoff_ms=10),
                mode='performance', run_id=f'{self.name}-g{index}'.replace('.', '-'),
                keyspace_name=f'sweep-{index}', seed=1000 * self.rep + index,
                workers=max(self.wpg, 1), keys=self.args.keys, value_bytes=128,
                mix=mixes[self.mix], warmup_operations=self.args.warmup,
                max_operations=1_000_000, measure_ms=self.args.measure_ms,
                interval_ms=0, history_bytes=0)
            path = self.output / f'workload-{index}.json'
            path.write_text(json.dumps(config) + '\n')
            log = (self.output / f'workload-{index}.log').open('w')
            self.handles.append(log)
            run_dir = self.output / f'run-{index}'
            command = [str(self.args.workload), '--config', str(path), '--output', str(run_dir),
                       '--build-manifest', str(self.args.build_manifest)]
            launches.append((index, run_dir, subprocess.Popen(
                command, env=dict(os.environ, KV9_CLIENT_TOKEN='sweep-client'),
                stdout=log, stderr=subprocess.STDOUT)))
            if self.args.launch_stagger_ms:
                time.sleep(self.args.launch_stagger_ms / 1000)
        before = self.thread_cpu()
        started = time.monotonic()
        self.record['groups_at_launch'] = {n: self.data_groups(n) for n in self.processes}
        reports = {}
        for index, run_dir, process in launches:
            code = process.wait(timeout=self.args.measure_ms / 1000 + 600)
            if code != 0:
                self.record['groups_at_failure'] = {n: self.data_groups(n) for n in self.processes}
                self.record['failed_run'] = index
                for node in self.processes:
                    probe = subprocess.run(
                        [str(self.args.server), 'client', 'raw-get', '--addr',
                         self.addresses[node], '--keyspace', str(keyspaces[index]),
                         '--key-hex', '70726f6265'],
                        env=dict(os.environ, KV9_CLIENT_TOKEN='sweep-client'),
                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=30)
                    (self.output / f'failure-probe-n{node}.log').write_text(
                        f'exit={probe.returncode}\n{probe.stdout}')
            require(code == 0, f'workload {index} exited {code}; see {self.output}')
            report = json.loads((run_dir / 'report.json').read_bytes())
            require(report['complete'] is True, f'workload {index} incomplete: {report["failure"]}')
            reports[index] = report
        elapsed = time.monotonic() - started
        after = self.thread_cpu()
        cpu = {name: after.get(name, 0) - ticks for name, ticks in before.items()
               if after.get(name, 0) != ticks}
        hz = os.sysconf('SC_CLK_TCK')
        self.record.update(
            per_run=[dict(index=index,
                          measured_successful=r['measured_successful'],
                          measured_completed=r['measured_completed'],
                          measured_issued=r['measured_issued'],
                          cohort_elapsed_ns=r['cohort_elapsed_ns'],
                          success_ops_per_second=r['cohort_success_ops_per_second'],
                          wall_anchor_unix_ns=r['wall_anchor_unix_ns'],
                          stages=r['stages'], report_sha256=sha(self.output / f'run-{index}/report.json'))
                     for index, r in sorted(reports.items())],
            aggregate_success_ops_per_second=sum(
                r['cohort_success_ops_per_second'] or 0 for r in reports.values()),
            harness_elapsed_seconds=elapsed,
            server_thread_cpu_seconds={k: v / hz for k, v in sorted(cpu.items())})
        # Measurement-window overlap: rates are per-run; report skew honestly.
        starts, ends = [], []
        for r in reports.values():
            anchor = r['wall_anchor_unix_ns'] - r['wall_anchor_monotonic_ns']
            m = r['stages']['measurement']
            starts.append(anchor + m['start_ns'])
            ends.append(anchor + m['end_ns'])
        self.record.update(measurement_overlap_seconds=(min(ends) - max(starts)) / 1e9,
                           measurement_skew_seconds=(max(starts) - min(starts)) / 1e9)

    def close(self, verdict, error=None):
        for process in self.processes.values():
            if process.poll() is None:
                process.kill()
        for process in self.processes.values():
            process.wait(timeout=10)
        for handle in self.handles:
            handle.close()
        self.record['verdict'] = verdict
        if error:
            self.record['error'] = error
        (self.output / 'cell.json').write_text(json.dumps(self.record, indent=2) + '\n')
        if verdict == 'accepted' and not self.args.keep_stores:
            for node in (1, 2, 3):
                shutil.rmtree(self.stores / f'n{node}', ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server', type=Path, required=True)
    parser.add_argument('--workload', type=Path, required=True)
    parser.add_argument('--build-manifest', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--groups', default='1,2,4,8')
    parser.add_argument('--mixes', default='put,get,mixed')
    parser.add_argument('--reps', type=int, default=2)
    parser.add_argument('--data-workers', type=int, default=2)
    parser.add_argument('--aggregate-workers', type=int, default=64)
    parser.add_argument('--scale-load', action='store_true',
                        help='panel B: per-group workers stay fixed at aggregate/first-group value')
    parser.add_argument('--measure-ms', type=int, default=30_000)
    parser.add_argument('--warmup', type=int, default=2_000)
    parser.add_argument('--keys', type=int, default=2_048)
    parser.add_argument('--keep-stores', action='store_true')
    parser.add_argument('--launch-stagger-ms', type=int, default=0,
                        help='delay between workload launches; setup-only, windows still overlap-checked')
    parser.add_argument('--max-requests', type=int, default=64,
                        help='KV9_PUBLIC_MAX_REQUESTS per node (default matches production)')
    parser.add_argument('--store-root', type=Path, default=None,
                        help='place node stores here (e.g. a tmpfs panel); artifacts stay in --output')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = dict(
        verdict='running',
        server_sha256=sha(args.server), workload_sha256=sha(args.workload),
        build_manifest_sha256=sha(args.build_manifest),
        revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
        dirty=bool(subprocess.check_output(['git', 'status', '--porcelain'])),
        arguments={k: str(v) for k, v in vars(args).items()},
        host=dict(cores=os.cpu_count(), clock_tick=os.sysconf('SC_CLK_TCK')),
        single_host_development_diagnostic=True, scaling_benchmark=False, chaos_mesh=False,
        store_root=str(args.store_root) if args.store_root else None,
        store_rotational=None)
    cells = []
    try:
        for rep in range(1, args.reps + 1):
            for mix in args.mixes.split(','):
                for groups in map(int, args.groups.split(',')):
                    wpg = (args.aggregate_workers if args.scale_load
                           else max(args.aggregate_workers // groups, 1))
                    name = f'g{groups}-{mix}-w{wpg}-r{rep}'
                    cell = Cell(args, name, groups, mix, wpg, rep)
                    try:
                        cell.start_cluster()
                        keyspaces = cell.bind_groups()
                        cell.run_workloads(keyspaces)
                        cell.close('accepted')
                    except Exception as error:
                        # Record and continue: one failed cell must not discard
                        # the rest of the matrix. Failed stores are retained.
                        cell.close('failed', str(error))
                        cells.append(cell.record)
                        print(f'CELL {name}: FAILED {error}', flush=True)
                        continue
                    cells.append(cell.record)
                    print(f"CELL {name}: {cell.record['aggregate_success_ops_per_second']:.1f} ops/s "
                          f"overlap {cell.record['measurement_overlap_seconds']:.1f}s", flush=True)
        manifest['verdict'] = 'accepted'
    except Exception as error:
        manifest.update(verdict='failed', error=str(error))
        raise
    finally:
        manifest['cells'] = cells
        (args.output / 'sweep.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
