"""Finite raw-status boundary capture around the unchanged native trial.

The first fresh status is retained before checking its trace quality. No polling
decision examines trace validity, loss, rows or joins. No periodic sampler runs.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import time

POLICY = dict(phases=['before', 'post-client', 'post-drain', 'post-readback'],
              voters=3, max_status_bytes=2*1024**2, max_raw_bytes_per_cohort=24*1024**2,
              fresh_timeout_seconds=20, freshness_export_advances=2,
              first_fresh_only=True, periodic_sampling=False)


def need(value, message):
    if not value:
        raise ValueError(message)


def save(path, value):
    with path.open('x') as out:
        json.dump(value, out, sort_keys=True, indent=2)
        out.write('\n')


def reader(path):
    spec = importlib.util.spec_from_file_location('bounded_stage_reader', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_live(path):
    # The server atomically replaces status. One open FD pins one complete old
    # or new inode; a concurrent rename is harmless, an in-place write is not.
    with path.open('rb') as stream:
        a = os.fstat(stream.fileno())
        need(stat.S_ISREG(a.st_mode) and 0 < a.st_size <= POLICY['max_status_bytes'],
             'raw status must be a bounded regular file')
        raw = stream.read(POLICY['max_status_bytes']+1)
        b = os.fstat(stream.fileno())
    # Replacing the pathname can unlink this still-open inode and change ctime.
    # The opened regular file's data identity and mtime must remain unchanged.
    need((a.st_dev,a.st_ino,a.st_size,a.st_mtime_ns) ==
         (b.st_dev,b.st_ino,b.st_size,b.st_mtime_ns) and len(raw) == a.st_size,
         'raw status inode changed during read')
    return raw


def fields(raw):
    need(type(raw) is bytes and 0 < len(raw) <= POLICY['max_status_bytes'], 'raw byte bound')
    result = {}
    for line in raw.decode('utf-8').splitlines():
        key, sep, value = line.partition('=')
        need(sep and key not in result, 'malformed or duplicate raw status field')
        result[key] = value
    return result


def exports(status):
    value = status.get('metrics_export_successes', '')
    need(value.isascii() and value.isdecimal() and 0 < len(value) <= 20,
         'missing export freshness counter')
    value = int(value)
    need(value <= (1<<64)-1, 'export freshness counter overflow')
    return value


def bound_identity(status, node, identity):
    need(status.get('pid') == str(identity['pid']) and status.get('node_id') == str(node) and
         status.get('process_start_ticks') == str(identity['start_ticks']) and
         status.get('process_boot_id') == identity['boot_id'], 'owned status identity changed')


def fresh(initial, candidate):
    # Two advances exclude the one snapshot that may already be rendering when
    # this boundary starts. It is not a fixed 20 ms timing guarantee.
    return exports(candidate) >= exports(initial) + POLICY['freshness_export_advances']


class Hooks:
    def __init__(self, driver, benchmark, directory, instrumented, guard, reader_path):
        self.driver, self.benchmark, self.directory = driver, benchmark, directory
        self.instrumented, self.guard = instrumented, guard
        self.reader = reader(reader_path)
        self.fixture = None
        self.phases = []
        self.raw_bytes = 0

    def capture(self, fixture, phase):
        need(phase == POLICY['phases'][len(self.phases)], 'capture boundary order')
        self.fixture = fixture
        self.guard()
        root = self.directory/'raw-status'/phase
        root.mkdir(parents=True, exist_ok=False)
        started = time.time_ns()
        baseline = {n: fields(read_live(fixture.out/'data'/f'n{n}'/'status')) for n in fixture.nodes}
        need(len(baseline) == POLICY['voters'], 'three bound voters required')
        for n, status in baseline.items():
            bound_identity(status,n,fixture.identities[fixture.nodes[n].pid])
        result = dict(complete=False, phase=phase, started_unix_ns=started,
                      baseline_exports={str(n):exports(s) for n,s in baseline.items()}, voters={})
        remaining = set(baseline)
        end = time.monotonic()+POLICY['fresh_timeout_seconds']
        try:
            while remaining:
                self.guard()
                need(all(p.poll() is None for p in fixture.nodes.values()), 'owned voter exited during capture')
                need(time.monotonic() < end, 'fresh raw status deadline')
                for n in sorted(remaining):
                    raw = read_live(fixture.out/'data'/f'n{n}'/'status')
                    status = fields(raw)
                    bound_identity(status,n,fixture.identities[fixture.nodes[n].pid])
                    need(exports(status) >= exports(baseline[n]), 'export counter reset')
                    if not fresh(baseline[n], status):
                        continue
                    # Preserve first fresh bytes even when subsequent trace or
                    # feature validation refuses this observation.
                    path = root/f'node-{n}.status'
                    with path.open('xb') as out:
                        out.write(raw)
                    self.raw_bytes += len(raw)
                    need(self.raw_bytes <= POLICY['max_raw_bytes_per_cohort'], 'raw capture allocation bound')
                    result['voters'][str(n)] = dict(path=str(path),bytes=len(raw),
                        sha256=hashlib.sha256(raw).hexdigest(), observed_unix_ns=time.time_ns(),
                        metrics_export_successes=exports(status),
                        role=status.get('role'),leader_id=status.get('leader_id'))
                    remaining.remove(n)
                if remaining:
                    time.sleep(.02)
            # All first fresh voters are already retained. Quality failure is
            # terminal for this attempted cohort, never a reason to poll again.
            for row in result['voters'].values():
                raw = Path(row['path']).read_bytes()
                status = fields(raw)
                need(('write_stage_trace' in status) == self.instrumented and
                     ('write_path_diagnostics' in status) == self.instrumented, 'runtime feature graph differs')
                if self.instrumented:
                    identity, trace = self.reader.parse_status(raw)
                    row['identity'] = identity
                    row['capture_started_ns'] = trace['capture_started_ns']
                    row['capture_finished_ns'] = trace['capture_finished_ns']
            self.phases.append(phase)
            result['complete'] = True
        except BaseException as error:
            result['failure_type'] = type(error).__name__
            result['failure'] = str(error) if isinstance(error, ValueError) else 'capture interrupted or I/O failed'
            raise
        finally:
            result['ended_unix_ns'] = time.time_ns()
            save(root/'capture.json',result)

    def __enter__(self):
        self.old_snapshot = self.benchmark.Fixture.snapshot
        self.old_observe = self.driver.observe_client
        self.old_drain = self.driver.fresh_drain
        def snapshot(fixture, directory, phase):
            result = self.old_snapshot(fixture,directory,phase)
            if phase == 'before':
                self.capture(fixture,'before')
            return result
        def observe(*args, **kwargs):
            report = self.old_observe(*args,**kwargs)
            need(self.fixture is not None,'before boundary missing')
            self.capture(self.fixture,'post-client')
            return report
        def drain(fixture,directory,label):
            result = self.old_drain(fixture,directory,label)
            if label in ('post-client','post-readback'):
                self.capture(fixture,'post-drain' if label == 'post-client' else 'post-readback')
            return result
        self.benchmark.Fixture.snapshot = snapshot
        self.driver.observe_client = observe
        self.driver.fresh_drain = drain
        return self

    def __exit__(self, *_error):
        self.benchmark.Fixture.snapshot = self.old_snapshot
        self.driver.observe_client = self.old_observe
        self.driver.fresh_drain = self.old_drain


def compare_retained(directory, instrumented, reader_path):
    check = reader(reader_path)
    result = dict(complete=False, instrumented=instrumented, nodes={},
                  complete_history=False, periodic_sampling=False, no_resampling=True)
    try:
        records = [json.loads((directory/'raw-status'/phase/'capture.json').read_text()) for phase in POLICY['phases']]
        need(all(r['complete'] for r in records), 'incomplete raw boundary capture')
        need(all(set(r['voters']) == {'1','2','3'} for r in records), 'raw voter inventory')
        for n in ['1','2','3']:
            raw = []
            for receipt in records:
                row = receipt['voters'][n]
                data = check.read_file(Path(row['path']))
                need(len(data)==row['bytes'] and hashlib.sha256(data).hexdigest()==row['sha256'], 'retained raw bytes changed')
                raw.append(data)
            if not instrumented:
                need(all('write_stage_trace' not in fields(v) and 'write_path_diagnostics' not in fields(v) for v in raw),
                     'default unexpectedly exports observation features')
                result['nodes'][n] = dict(default_trace_absent=True)
                continue
            node = {}
            for a,b in [(0,1),(1,2),(2,3)]:
                analysis = check.analyze_status(raw[a],raw[b])
                path = directory/'raw-status'/f'node-{n}-{POLICY["phases"][a]}-to-{POLICY["phases"][b]}.json'
                save(path,analysis)
                _, ta = check.parse_status(raw[a]); _, tb = check.parse_status(raw[b])
                node[f'{POLICY["phases"][a]}_to_{POLICY["phases"][b]}'] = dict(
                    result_path=str(path),result_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                    counter_delta=analysis['counter_delta'],coverage=analysis['coverage'],join_counts=analysis['join_counts'],
                    group_ring_unchanged=ta['groups']==tb['groups'],inspection_ring_unchanged=ta['inspections']==tb['inspections'])
            result['nodes'][n] = node
        result['complete'] = True
    except BaseException as error:
        result['failure_type'] = type(error).__name__
        result['failure'] = str(error) if isinstance(error,ValueError) else 'readback interrupted or I/O failed'
        raise
    finally:
        save(directory/'raw-status'/'comparison.json',result)
