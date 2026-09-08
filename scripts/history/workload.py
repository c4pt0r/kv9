#!/usr/bin/env python3
"""Record concurrent public-CLI calls with endpoint failover and unknown outcomes."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import random
import re
import subprocess
import threading
import time

from checker import History, Malformed, hex_bytes, require


def parse_success(kind, stdout):
    lines = stdout.strip().splitlines()
    if kind == 'scan':
        require(bool(lines) and re.fullmatch(r'count=[0-9]+', lines[-1]), 'scan lacks count')
        rows = []
        for line in lines[:-1]:
            match = re.fullmatch(r'key_hex=([0-9a-f]*) value_hex=([0-9a-f]*)', line)
            require(match is not None, 'malformed scan row')
            rows.append([hex_bytes(match[1]), hex_bytes(match[2])])
        require(int(lines[-1].split('=')[1]) == len(rows), 'scan count mismatch')
        return {'rows': rows}, {}
    if kind == 'get':
        if stdout.strip() == 'found=false':
            return {'value': None}, {}
        require(len(lines) == 1 and lines[0].startswith('value_hex='), 'malformed get result')
        return {'value': hex_bytes(lines[0].split('=', 1)[1])}, {}
    fields = {}
    for line in lines:
        parts = line.split('=', 1)
        require(len(parts) == 2 and parts[0] not in fields and re.fullmatch('[0-9]+', parts[1]), 'malformed receipt')
        fields[parts[0]] = int(parts[1])
    if kind == 'create_keyspace':
        require(set(fields) == {'keyspace_id', 'proposed_term', 'proposed_index'} and all(fields.values()), 'missing creation receipt')
        return {'id': fields['keyspace_id']}, fields
    if kind == 'delete_range':
        require(set(fields) == {'committed_chunks', 'last_applied_term', 'last_applied_index'}, 'missing range receipt')
        require((fields['committed_chunks'] == 0 and fields['last_applied_term'] == fields['last_applied_index'] == 0)
                or (fields['committed_chunks'] > 0 and fields['last_applied_term'] > 0 and fields['last_applied_index'] > 0), 'inconsistent range receipt')
        return {'committed_chunks': fields['committed_chunks']}, fields
    require(set(fields) == {'applied_term', 'applied_index'} and all(fields.values()), 'missing write receipt')
    return {}, fields


class Recorder:
    def __init__(self, args):
        self.args = args
        self.lock = threading.Lock()
        self.sequence = 0
        self.next_id = 0
        self.preferred = 0
        self.failed = threading.Event()
        self.phase_counts = {}
        self.token = os.environ['KV9_CLIENT_TOKEN']
        self.addresses = args.addresses.split(',')
        require(len(set(self.addresses)) >= 3, 'at least three distinct endpoints are required')
        self.file = args.history.open('x', buffering=1)
        self.header = {'type': 'header', 'version': 1, 'range_chunk_size': 1024,
                       'seed': args.seed, 'endpoints': self.addresses, 'workers': args.workers,
                       'initial': json.loads(args.initial_catalog.read_text()) if args.initial_catalog else {}}
        self.file.write(json.dumps(self.header, sort_keys=True) + '\n')

    def event(self, event):
        # A single coordinator sequences both invocation and response events.
        # Lock ownership also prevents concurrent JSONL writes from interleaving.
        event.update(seq=self.sequence, monotonic_ns=time.monotonic_ns())
        self.sequence += 1
        self.file.write(json.dumps(event, sort_keys=True) + '\n')

    def call(self, worker, kind, arguments):
        with self.lock:
            oid = self.next_id; self.next_id += 1
            endpoint_index = self.preferred
            endpoint = self.addresses[endpoint_index]
            phase = self.args.phase.read_text().strip() if self.args.phase and self.args.phase.exists() else 'baseline'
            self.event({'type': 'invoke', 'id': oid, 'client': str(worker), 'op': kind,
                        'args': arguments, 'endpoint': endpoint, 'phase': phase})
        command = 'create-keyspace' if kind == 'create_keyspace' else 'raw-' + kind.replace('_', '-')
        argv = ['timeout', '5', self.args.binary, 'client', command, '--addr', endpoint]
        if kind == 'create_keyspace':
            argv += ['--name', arguments['name'], '--api-type', 'raw']
        else:
            argv += ['--keyspace', str(arguments['keyspace'])]
            for field, option in [('key', '--key-hex'), ('value', '--value-hex'), ('start', '--start-hex'), ('end', '--end-hex'), ('limit', '--limit')]:
                if field in arguments:
                    argv += [option, str(arguments[field])]
        if self.args.client_pod:
            argv = [self.args.kubectl, '--kubeconfig', str(self.args.kubeconfig), 'exec', '-n', self.args.namespace,
                    self.args.client_pod, '--', 'env', 'KV9_CLIENT_TOKEN=' + self.token, *argv]
        stdout, stderr, rc, reason = '', '', None, 'rpc_error'
        malformed = None
        try:
            process = subprocess.run(argv, text=True, capture_output=True, timeout=7)
            stdout, stderr, rc = process.stdout, process.stderr, process.returncode
        except subprocess.TimeoutExpired as error:
            stdout = (error.stdout or b'').decode() if isinstance(error.stdout, bytes) else error.stdout or ''
            stderr = (error.stderr or b'').decode() if isinstance(error.stderr, bytes) else error.stderr or ''
            reason = 'transport_timeout'
        except OSError as error:
            stderr = str(error)
            malformed = error
        outcome, result, observation = 'unknown', {}, {}
        if rc == 0:
            try:
                result, observation = parse_success(kind, stdout)
                History.validate_result(kind, 'ok', result)
                outcome, reason = 'ok', 'confirmed'
            except Malformed as error:
                malformed = error
        elif kind == 'delete_range' and 'partial_write=true' in stderr.splitlines():
            fields = dict(line.split('=', 1) for line in stderr.splitlines() if '=' in line)
            value = fields.get('committed_chunks', '')
            if re.fullmatch('[0-9]+', value):
                result = {'committed_chunks': int(value)}
                reason = 'partial_range_unknown_tail'
            else:
                malformed = Malformed('partial receipt lacks chunk count')
        elif stderr.startswith('not_leader=true'):
            reason = 'not_leader_unknown_effect'
        elif stderr.startswith('read_unconfirmed=true'):
            reason = 'read_unconfirmed'
        with self.lock:
            self.event({'type': 'return', 'id': oid, 'outcome': outcome, 'result': result,
                        'observation': {'endpoint': endpoint, 'exit_code': rc, 'reason': reason,
                                        'stdout': stdout, 'stderr': stderr, 'receipt': observation,
                                        'malformed': str(malformed) if malformed else None}})
            if outcome == 'ok':
                self.preferred = endpoint_index
                counts = self.phase_counts.setdefault(phase, {})
                counts[kind] = counts.get(kind, 0) + 1
            elif self.preferred == endpoint_index:
                self.preferred = (endpoint_index + 1) % len(self.addresses)
            if self.args.progress:
                temporary = self.args.progress.with_suffix('.tmp')
                temporary.write_text(json.dumps(self.phase_counts, sort_keys=True) + '\n')
                temporary.replace(self.args.progress)
        if malformed:
            self.failed.set()
            raise malformed
        return result if outcome == 'ok' else None

    def eventually(self, kind, arguments, timeout=35):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            result = self.call('setup-or-heal', kind, arguments)
            if result is not None:
                return result
            time.sleep(0.05)
        raise RuntimeError(f'no successful {kind} through the endpoint set')

    def worker(self, number, keyspace):
        rng = random.Random(self.args.seed + number)
        iteration = 0
        started = time.monotonic()
        while not self.failed.is_set():
            if self.args.stop and self.args.stop.exists():
                break
            if self.args.duration and time.monotonic() - started >= self.args.duration:
                break
            kind = ['put', 'get', 'delete', 'scan', 'delete_range'][iteration % 5]
            key = bytes([rng.randrange(8)]).hex()
            arguments = {'keyspace': keyspace}
            if kind in {'put', 'get', 'delete'}:
                arguments['key'] = key
            if kind == 'put':
                arguments['value'] = f'{number}:{iteration}'.encode().hex()
            if kind in {'scan', 'delete_range'}:
                start, end = sorted([rng.randrange(9), rng.randrange(9)])
                arguments.update(start=bytes([start]).hex(), end=bytes([end]).hex())
            if kind == 'scan':
                arguments['limit'] = rng.randrange(1, 9)
            self.call(number, kind, arguments)
            if iteration % 20 == 0:
                # The shared names deliberately race across workers; a name may
                # have only one confirmed successful creation and IDs never alias.
                self.call(number, 'create_keyspace', {'name': f'{self.args.name}-catalog-{iteration // 20}'})
            iteration += 1
            time.sleep(self.args.interval)

    def run(self):
        keyspace = self.eventually('create_keyspace', {'name': self.args.name})['id']
        # Keep a reserved witness key outside all randomized mutation ranges.
        self.eventually('put', {'keyspace': keyspace, 'key': 'ff', 'value': '61636b'})
        require(self.eventually('get', {'keyspace': keyspace, 'key': 'ff'}) == {'value': '61636b'}, 'baseline acknowledged value missing')
        if self.args.ready:
            self.args.ready.write_text(str(keyspace) + '\n')
        with ThreadPoolExecutor(max_workers=self.args.workers) as executor:
            futures = [executor.submit(self.worker, i, keyspace) for i in range(self.args.workers)]
            try:
                for future in futures:
                    future.result()
            except BaseException:
                self.failed.set()
                raise
        require(self.eventually('get', {'keyspace': keyspace, 'key': 'ff'}) == {'value': '61636b'}, 'acknowledged sentinel lost after faults')
        self.eventually('scan', {'keyspace': keyspace, 'start': '', 'end': '', 'limit': 100})
        self.file.flush(); os.fsync(self.file.fileno()); self.file.close()
        print(f'PASS: recorded {self.next_id} operations through {len(self.addresses)} endpoints', flush=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--history', type=Path, required=True)
    p.add_argument('--binary', default='./target/debug/kv9')
    p.add_argument('--addresses', required=True)
    p.add_argument('--name', required=True)
    p.add_argument('--initial-catalog', type=Path)
    p.add_argument('--seed', type=int, default=120912)
    p.add_argument('--workers', type=int, default=3)
    p.add_argument('--interval', type=float, default=0.15)
    p.add_argument('--duration', type=float, default=0)
    p.add_argument('--stop', type=Path)
    p.add_argument('--ready', type=Path)
    p.add_argument('--phase', type=Path)
    p.add_argument('--progress', type=Path)
    p.add_argument('--client-pod')
    p.add_argument('--namespace')
    p.add_argument('--kubeconfig', type=Path)
    p.add_argument('--kubectl', default='kubectl')
    args = p.parse_args()
    require(args.workers >= 2 and (args.duration > 0 or args.stop is not None), 'bounded concurrent workload required')
    if args.client_pod:
        require(args.namespace and args.kubeconfig and args.kubeconfig.is_absolute(), 'explicit isolated cluster required')
    try:
        Recorder(args).run()
    except Exception as error:
        raise SystemExit(f'FAIL: history recorder: {error}') from error


if __name__ == '__main__':
    main()
