#!/usr/bin/env python3
"""Finite two-client routed data-group gate on an existing single-node Kind cluster.

Build/load is deliberately external. Failure preserves the owned namespace and every
attempt. Only a successful independent pre-cleanup audit permits exact-UID cleanup.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import queue
import re
import secrets
import selectors
import signal
import subprocess
import tarfile
import threading
import time

GIB = 1024**3
CPUS = set(range(6, 16)) | set(range(22, 32))
MAX_OUTPUT = 512 * 1024**2
MAX_STORE = 128 * 1024**2


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def sha(path):
    with Path(path).open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def save(path, value):
    with Path(path).open('x') as f:
        json.dump(value, f, indent=2, sort_keys=True); f.write('\n'); f.flush(); os.fsync(f.fileno())


def fields(text):
    return dict(line.split('=', 1) for line in text.splitlines() if '=' in line)


def cli_fields(text):
    result = {}
    for word in text.split():
        if '=' in word:
            key, value = word.split('=', 1)
            require(key not in result, 'duplicate CLI receipt key')
            result[key] = value
    return result


def identity(text, boot):
    tail = text.rsplit(')', 1)[1].split()
    return {'pid': int(text.split(' ', 1)[0]), 'start_ticks': int(tail[19]), 'boot_id': boot.strip()}


class Client:
    def __init__(self, gate, row):
        self.g, self.row = gate, row; self.directory = gate.out/row['name']; self.directory.mkdir()
        self.events = queue.Queue(); self.number = 0; self.commands = []; self.results = []
        self.requests = (self.directory/'commands.jsonl').open('x'); self.stdout = (self.directory/'stdout.jsonl').open('x')
        self.stderr = (self.directory/'stderr').open('xb')
        argv = gate.kargs(['exec', '-i', '-n', gate.ns, row['name'], '--', '/usr/local/bin/kv9-routed-workload', '--config', '/tmp/config.json'])
        self.child = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr, start_new_session=True)
        self.closed = False; gate.clients.append(self)
        save(self.directory/'invocation.json', {'argv': argv, 'pid': self.child.pid, 'started_ns': time.time_ns()})
        def consume():
            try:
                total = 0
                while True:
                    b = self.child.stdout.readline(1024**2 + 1)
                    if not b: break
                    total += len(b)
                    require(len(b) <= 1024**2 and total <= 32 * 1024**2, 'client output cap')
                    line = b.decode(); self.stdout.write(line); self.stdout.flush()
                    self.events.put(json.loads(line))
            except BaseException as e:
                self.events.put(e)
            finally:
                self.events.put(EOFError('client stdout ended'))
        self.thread = threading.Thread(target=consume, daemon=True); self.thread.start()
        header = self.event(15)
        require(header['kind'] == 'start' and header['config'] == row['config'], 'workload header/config mismatch')
        row['process'] = gate.remote_identity(row['name'], header['pid'])
        require(row['process']['pid'] == header['pid'], 'workload PID differs')
        self.header = header; self.closed = False

    def event(self, timeout):
        try: value = self.events.get(timeout=timeout)
        except queue.Empty: raise RuntimeError('client response deadline')
        if isinstance(value, BaseException): raise value
        return value

    def call(self, phase, operation):
        self.g.guard(); self.number += 1; require(self.number <= 256, 'client operation cap')
        command = {'id': self.number, 'operation': operation}
        record = dict(command, phase=phase, sent_unix_ns=time.time_ns()); self.commands.append(record)
        self.requests.write(json.dumps(record)+'\n'); self.requests.flush()
        self.child.stdin.write((json.dumps(command)+'\n').encode()); self.child.stdin.flush()
        row = self.event(8)
        require(row['kind'] == 'operation' and row['id'] == self.number and row['operation'] == operation, 'command/return identity')
        self.g.checker.report_check(row, self.row['region']); self.results.append(row)
        return row

    def progress(self, phase, timeout=35):
        deadline = time.monotonic()+timeout; written = None; first = self.number
        while time.monotonic() < deadline:
            key = list(f'call-{self.number+1:04d}'.encode())
            value = list(f'{self.row["name"]}:{phase}:{self.number+1}'.encode())
            result = self.call(phase, {'kind': 'put', 'key': key, 'value': value})
            if result['report']['outcome']['kind'] == 'success':
                written = (key, value)
                result = self.call(phase, {'kind': 'get', 'key': key})
                if result['report']['outcome']['kind'] == 'success':
                    require(result['report']['outcome']['value']['value'] == value, 'acknowledged value lost/misrouted')
                    return {'first_id': first+1, 'last_id': self.number, 'key': key, 'value': value}
        raise RuntimeError(f'no acknowledged put/read progress: {self.row["name"]} {phase}; prior={written is not None}')

    def close(self):
        if self.closed: return
        self.child.stdin.close(); rc = self.child.wait(timeout=15); self.thread.join(timeout=5)
        require(not self.thread.is_alive(), 'stdout reader did not terminate')
        self.stdout.close(); self.requests.close(); self.stderr.close(); self.closed = True
        self.g.wait('client remote lifetime exit', lambda: not self.g.remote_same(self.row['name'], self.row['process']), 10)
        same = self.g.remote_same(self.row['name'], self.row['process'])
        save(self.directory/'exit.json', {'exit_code': rc, 'remote_absent': not same,
             'remote_identity': self.row['process'], 'host_pid': self.child.pid, 'ended_ns': time.time_ns()})
        require(rc == 0 and not same, 'client lifetime remains/failed')


class Gate:
    def __init__(self, args):
        self.a = args; self.out = args.output.resolve(); self.out.mkdir(parents=True, exist_ok=False)
        os.chmod(self.out, 0o700); self.started = time.monotonic(); self.commands = 0; self.clients = []
        self.ns = 'kv9-routed-'+str(int(time.time()))+'-'+secrets.token_hex(3); self.uid = None; self.pods = {}; self.services = {}; self.cids = set()
        self.stores = {}; self.samples = []; self.snapshots = []; self.faults = []; self.boot = None
        self.host_processes = {}
        self.stale_status = []
        self.baselines = self.space(); self.result = {'runtime_complete': False, 'cleanup_complete': False, 'single_host_only': True, 'namespace': self.ns}
        spec = importlib.util.spec_from_file_location('routed_checker', Path(__file__).with_name('check-routed-client-chaos.py'))
        self.checker = importlib.util.module_from_spec(spec); spec.loader.exec_module(self.checker)
        self.guard(launch=True)

    def space(self):
        return {str(p): {'device': p.stat().st_dev, 'available_bytes': os.statvfs(p).f_bavail*os.statvfs(p).f_frsize}
                for p in [Path('/'), self.out]}

    def guard(self, launch=False):
        now = self.space(); seen = set(); decrease = 0
        for name, row in now.items():
            require(row['device'] == self.baselines[name]['device'], 'filesystem replaced')
            require(row['available_bytes'] >= (20*GIB+8*1024**2 if launch else 8*GIB), 'space floor')
            if row['device'] not in seen:
                decrease += max(0, self.baselines[name]['available_bytes']-row['available_bytes']); seen.add(row['device'])
        require(decrease <= 12*GIB and time.monotonic()-self.started <= 1200, 'allocation/time envelope')
        allocated = sum(p.lstat().st_blocks*512 for p in self.out.rglob('*') if p.is_file())
        require(allocated <= MAX_OUTPUT, 'retained output cap')
        self.samples.append({'observed_ns': time.time_ns(), 'filesystems': now, 'allocated_bytes': allocated})

    def kargs(self, args):
        return [str(self.a.kubectl), '--kubeconfig', str(self.a.kubeconfig), '--request-timeout=15s', *map(str, args)]

    def command(self, argv, *, data=None, allow=(), secret=False, timeout=25, env=None):
        self.guard(); self.commands += 1; prefix = self.out/f'command-{self.commands:05d}'; start = time.time_ns()
        # Exact public argv retained. Secret material is stdin and is never copied to artifacts.
        p = subprocess.run(list(map(str, argv)), input=data, capture_output=True, timeout=timeout, env=env)
        require(len(p.stdout) <= 8*1024**2 and len(p.stderr) <= 1024**2, 'command output cap')
        prefix.with_suffix('.stdout').write_bytes(p.stdout); prefix.with_suffix('.stderr').write_bytes(p.stderr)
        save(prefix.with_suffix('.json'), {'argv': list(map(str, argv)), 'exit_code': p.returncode,
             'started_ns': start, 'ended_ns': time.time_ns(), 'stdin_secret_omitted': secret,
             'stdin_sha256': None if secret or data is None else hashlib.sha256(data).hexdigest()})
        require(p.returncode == 0 or p.returncode in allow, f'command {self.commands} failed: exit {p.returncode}')
        return p

    def k(self, args, **kw): return self.command(self.kargs(args), **kw)
    def get(self, resource, name=None, ns=True):
        return json.loads(self.k(['get', resource, *([name] if name else []), *(['-n', self.ns] if ns else []), '-o', 'json']).stdout)
    def apply(self, obj, secret=False):
        return self.k(['create', '-f', '-'], data=json.dumps(obj).encode(), secret=secret)
    def exec(self, pod, args, **kw): return self.k(['exec', *(['-i'] if kw.get('data') is not None else []), '-n', self.ns, pod, '--', *args], **kw)
    def wait(self, label, f, timeout=45):
        end = time.monotonic()+timeout
        while time.monotonic() < end:
            self.guard(); value = f()
            if value: return value
            time.sleep(.3)
        raise RuntimeError('timed out: '+label)
    def remote_identity(self, pod, pid):
        r = self.exec(pod, ['/bin/bash', '-c', 'cat /proc/$1/stat; cat /proc/sys/kernel/random/boot_id', 'identity', str(pid)])
        a, b = r.stdout.decode().splitlines(); return identity(a, b)
    def remote_same(self, pod, old):
        r = self.exec(pod, ['/bin/bash', '-c', 'test ! -r /proc/$1/stat || cat /proc/$1/stat', 'identity', str(old['pid'])])
        return bool(r.stdout) and identity(r.stdout.decode(), old['boot_id']) == old
    def inspect_container(self, cid):
        # Inspect includes resolved Secret env values: never retain its raw output.
        require(re.fullmatch('containerd://[0-9a-f]{64}', cid), 'invalid container identity')
        q = subprocess.run(['docker', 'exec', self.kind_node, 'crictl', 'inspect', cid.removeprefix('containerd://')], capture_output=True, timeout=20)
        require(len(q.stdout) <= 8*1024**2 and len(q.stderr) <= 1024**2, 'CRI inspect cap')
        if q.returncode:
            require(b'NotFound' in q.stderr, 'CRI inspection failed without absence evidence')
            return None
        return json.loads(q.stdout)
    def host_process(self, pid):
        require(type(pid) is int and 0 < pid < 2**31, 'invalid Kind process ID')
        r = self.command(['docker', 'exec', self.kind_node, '/bin/bash', '-c',
             'if [ -r /proc/$1/stat ]; then cat /proc/$1/stat; cat /proc/sys/kernel/random/boot_id; fi', 'identity', str(pid)])
        if not r.stdout: return None
        lines = r.stdout.decode().splitlines(); require(len(lines) == 2, 'Kind process identity framing')
        return identity(lines[0], lines[1])
    def pod_for(self, node):
        rows = self.get('pods')['items']; rows = [r for r in rows if r['metadata'].get('labels', {}).get('kv9-node') == str(node)]
        return rows[0] if len(rows) == 1 and rows[0]['status'].get('phase') == 'Running' else None
    def status(self, node):
        pod = self.pod_for(node)
        if not pod: return None
        name = pod['metadata']['name']; r = self.exec(name, ['cat', '/data/status'], allow=(1, 126, 137))
        if r.returncode: return None
        f = fields(r.stdout.decode())
        if not f.get('pid') or f.get('endpoint_ready') != 'true': return None
        proc = self.exec(name, ['/bin/bash', '-c', 'test ! -r /proc/$1/stat || { cat /proc/$1/stat; cat /proc/sys/kernel/random/boot_id; }', 'identity', f['pid']], allow=(1, 126, 137))
        if proc.returncode or not proc.stdout: return None
        parts = proc.stdout.decode().splitlines()
        if len(parts) != 2: return None
        live = identity(parts[0], parts[1])
        if str(live['start_ticks']) != f['process_start_ticks'] or live['boot_id'] != f['process_boot_id']:
            # The PVC intentionally preserves the previous process's status
            # until the new owner exports. Never accept it as a fresh sample.
            self.stale_status.append({'node': node, 'pod': name, 'observed_ns': time.time_ns(),
                 'live': live, 'recorded_start_ticks': f['process_start_ticks'],
                 'recorded_boot_id': f['process_boot_id']})
            return None
        c = next(x for x in pod['status']['containerStatuses'] if x['name'] == 'kv9')
        if c['containerID'] not in self.host_processes:
            inspection = self.inspect_container(c['containerID'])
            if inspection is None: return None  # A sampled container can exit during restart.
            process = self.host_process(inspection['info']['pid'])
            if process is None: return None
            self.host_processes[c['containerID']] = process
        self.cids.add(c['containerID'])
        previous = c.get('lastState', {}).get('terminated', {})
        row = {'node': node, 'pod': name, 'pod_uid': pod['metadata']['uid'], 'pvc_uid': self.stores[node],
               'container_id': c['containerID'], 'process': live, 'kind_process': self.host_processes[c['containerID']], 'store_incarnation': f['store_incarnation'],
               'root_digest': f['root_digest'], 'status': f, 'groups': json.loads(f['data_groups']),
               'last_exit_code': previous.get('exitCode'), 'old_container_id': previous.get('containerID'), 'observed_ns': time.time_ns()}
        self.pods[node] = name; self.snapshots.append(row); return row
    def group_ready(self, nodes=(1, 2, 3)):
        observations = [self.status(n) for n in nodes]
        if not all(observations): return None
        for region in self.regions:
            rows = [next((g for g in x['groups'] if g['region'] == region), {}) for x in observations]
            if not all(g.get('state') == 'active' and g.get('error') is None and g.get('driver_applied') and
                       g['driver_applied']['index'] == g['committed'] > 0 and g['driver_applied']['term'] == g['term'] for g in rows): return None
            if len({(g['term'], g['leader']) for g in rows}) != 1: return None
            if sum(g['role'] == 'Leader' for g in rows) != 1: return None
        return observations
    def group_leader(self, region):
        rows = self.wait('group agreement', self.group_ready)
        return next(x['node'] for x in rows if next(g for g in x['groups'] if g['region'] == region)['role'] == 'Leader')
    def cli(self, node, *argv):
        return cli_fields(self.exec(self.pods[node], ['/usr/local/bin/kv9', 'client', *map(str, argv)]).stdout.decode())
    def meta_leader(self):
        rows = [self.status(n) for n in (1, 2, 3)]
        return next((r['node'] for r in rows if r and r['status']['role'] == 'leader'), None)

    def prepare(self):
        save(self.out/'execution-environment.json', {'uid': os.geteuid(), 'cpu_affinity': sorted(os.sched_getaffinity(0)), 'single_host_only': True})
        require(not any(k.startswith(('KV9_TESTING_', 'KV9_OBJECT_STORE_')) for k in os.environ), 'inherited testing/object-store environment')
        require(sha(self.a.server) == self.a.server_sha256, 'retained server pin')
        qualification = self.checker.read(self.a.qualification)
        require(qualification['complete'] is True and qualification['default_features'] == [] and qualification['profile'] in ('debug', 'release') and
                qualification['server_sha256'] == self.a.server_sha256 and qualification['workload_sha256'] == self.a.workload_sha256 and
                re.fullmatch('[0-9a-f]{40}', qualification['revision']), 'current default-feature correctness qualification')
        for evidence in qualification.get('evidence', [])+[qualification['dirty_source_manifest']]:
            ep = Path(evidence['path']); require(ep.stat().st_size == evidence['bytes'] and sha(ep) == evidence['sha256'], 'qualification evidence changed')
        save(self.out/'qualification.json', qualification)
        image = json.loads(self.command(['docker', 'image', 'inspect', self.a.image]).stdout)[0]
        require(image['Id'] == self.a.image_id, 'image ID mismatch')
        self.kind_node = self.command([str(self.a.kind), 'get', 'nodes', '--name', self.a.kind_cluster]).stdout.decode().strip()
        nodes = self.get('nodes', ns=False)['items']; require(len(nodes) == 1 and nodes[0]['metadata']['name'] == self.kind_node, 'Kind/kubeconfig mismatch')
        require(any(c['type'] == 'Ready' and c['status'] == 'True' for c in nodes[0]['status']['conditions']), 'Kind node not Ready')
        old_ns = self.get('namespaces', ns=False)['items']; self.protected = {n['metadata']['name']: n['metadata']['uid'] for n in old_ns}
        require(len(self.protected) == 8, 'unexpected historical namespace set; review required')
        cri = json.loads(self.command(['docker', 'exec', self.kind_node, 'crictl', 'images', '--output', 'json']).stdout)['images']
        selected = [x for x in cri if any(tag.removeprefix('docker.io/library/') == self.a.image for tag in x.get('repoTags', []))]
        require(len(selected) == 1 and selected[0]['id'] == self.a.image_id, 'preloaded Kind image binding')
        self.image_ids = {self.a.image_id} | {x.split('@')[-1] for x in selected[0].get('repoDigests', [])}
        save(self.out/'image-binding.json', {'docker_image_id': self.a.image_id, 'cri': selected[0], 'accepted_content_ids': sorted(self.image_ids)})
        self.protected_faults = self.k(['get', 'podchaos,networkchaos,iochaos', '-A', '-o', 'json']).stdout
        save(self.out/'protected.json', {'namespaces': self.protected, 'faults': self.fault_ids(json.loads(self.protected_faults))})
        save(self.out/'inputs.json', {'server': self.checker.pin(self.a.server), 'qualification': self.checker.pin(self.a.qualification),
             'runner': self.checker.pin(__file__), 'checker': self.checker.pin(Path(__file__).with_name('check-routed-client-chaos.py')),
             'image': self.a.image, 'image_id': self.a.image_id, 'workload_sha256': self.a.workload_sha256})
        self.apply({'apiVersion': 'v1', 'kind': 'Namespace', 'metadata': {'name': self.ns, 'annotations': {'chaos-mesh.org/inject': 'enabled'}}})
        self.uid = self.get('namespace', self.ns, ns=False)['metadata']['uid']; self.result['namespace_uid'] = self.uid
        save(self.out/'namespace-created.json', {'namespace': self.ns, 'namespace_uid': self.uid, 'created_ns': time.time_ns()})
        token = secrets.token_hex(24); cluster = secrets.token_hex(24); bootstrap = secrets.token_hex(24)
        self.apply({'apiVersion': 'v1', 'kind': 'Secret', 'metadata': {'name': 'auth', 'namespace': self.ns},
             'stringData': {'client': token, 'clients': 'admin='+token, 'cluster': cluster, 'bootstrap': bootstrap}}, secret=True)
        for n in (1, 2, 3):
            self.apply({'apiVersion': 'v1', 'kind': 'Service', 'metadata': {'name': f'n{n}', 'namespace': self.ns},
                 'spec': {'selector': {'app': 'kv9-routed-voter', 'kv9-node': str(n)}, 'ports': [{'port': 20160, 'targetPort': 20160}]}})
            self.services[n] = self.get('service', f'n{n}')['spec']['clusterIP']+':20160'
            self.apply({'apiVersion': 'v1', 'kind': 'PersistentVolumeClaim', 'metadata': {'name': f'data-{n}', 'namespace': self.ns},
                 'spec': {'accessModes': ['ReadWriteOnce'], 'resources': {'requests': {'storage': '1Gi'}}}})
            self.stores[n] = self.get('pvc', f'data-{n}')['metadata']['uid']
            self.apply(self.pod(f'prepare-{n}', ['sleep', '1200'], n))
            self.k(['wait', '-n', self.ns, '--for=condition=Ready', f'pod/prepare-{n}', '--timeout=60s'], timeout=65)
        incarnations = []
        for n in (1, 2, 3):
            result = cli_fields(self.exec(f'prepare-{n}', ['/usr/local/bin/kv9', 'store-prepare', '--node-id', str(n), '--data-dir', '/data']).stdout.decode())
            require(re.fullmatch('[0-9a-f]{32}', result['store_incarnation']), 'store incarnation')
            incarnations.append(f'{n}='+result['store_incarnation'])
        root = self.out/'root.bin'
        self.command([str(self.a.server), 'root-create', '--output', str(root), '--voters', ','.join(f'{n}@{a}' for n, a in self.services.items()), '--store-incarnations', ','.join(incarnations)], env=dict(os.environ, KV9_BOOTSTRAP_TOKEN=bootstrap))
        self.apply({'apiVersion': 'v1', 'kind': 'ConfigMap', 'metadata': {'name': 'root', 'namespace': self.ns}, 'binaryData': {'root.bin': base64.b64encode(root.read_bytes()).decode()}})
        for n in (1, 2, 3):
            self.k(['delete', 'pod', f'prepare-{n}', '-n', self.ns, '--wait=true', '--timeout=30s'], timeout=35)
            labels = {'app': 'kv9-routed-voter', 'kv9-node': str(n)}
            script = f'set -eu; if [ ! -f /data/kv9-store-identity ]; then /usr/local/bin/kv9 init --root /root/root.bin --node-id {n} --data-dir /data; fi; exec /usr/local/bin/kv9 start --node-id {n} --addr 0.0.0.0:20160 --data-dir /data'
            pod = self.pod(f'n{n}', ['/bin/bash', '-c', script], n, labels)
            pod['spec']['restartPolicy'] = 'Always'; pod['spec']['volumes'].append({'name': 'root', 'configMap': {'name': 'root'}})
            pod['spec']['containers'][0]['volumeMounts'].append({'name': 'root', 'mountPath': '/root', 'readOnly': True})
            self.apply({'apiVersion': 'apps/v1', 'kind': 'Deployment', 'metadata': {'name': f'n{n}', 'namespace': self.ns},
                'spec': {'replicas': 1, 'strategy': {'type': 'Recreate'}, 'selector': {'matchLabels': labels}, 'template': {'metadata': {'labels': labels}, 'spec': pod['spec']}}})
        owner = self.wait('metadata leader', self.meta_leader, 60)
        rows = self.wait('all endpoints', lambda: [self.status(n) for n in (1, 2, 3)] if all(self.status(n) for n in (1, 2, 3)) else None)
        digest = rows[0]['root_digest']; require(all(x['root_digest'] == digest for x in rows), 'root mismatch')
        self.regions = []; self.setup = {'namespace': self.ns, 'namespace_uid': self.uid, 'root_digest': digest, 'clients': []}
        for i in range(2):
            owner = self.wait('metadata leader before create', self.meta_leader)
            group = self.cli(owner, 'create-data-group', '--addr', self.services[owner], '--root-digest', digest, '--operation-id', f'{i+1:032x}', '--voters', '1,2,3')
            binding = self.cli(owner, 'create-data-keyspace', '--addr', self.services[owner], '--root-digest', digest, '--creation-task', group['task_id'], '--name', f'group-{i}', '--tenant-id', '0')
            region = int(group['region_id']); require(int(binding['region_id']) == region, 'binding group mismatch'); self.regions.append(region)
            config = {'version': 1, 'root_digest': list(bytes.fromhex(digest)), 'tenant_id': 0, 'keyspace_id': int(binding['keyspace_id']),
                 'seeds': [{'node_id': n, 'address': a} for n, a in self.services.items()], 'max_in_flight': 4, 'max_attempts': 16,
                 'deadline_ms': 5000, 'probe_timeout_ms': 300, 'retry_backoff_ms': 150, 'cache_capacity': 8}
            name = f'client-{i}'; self.apply(self.pod(name, ['sleep', '1200'], labels={'app': 'kv9-routed-client', 'client-id': str(i)}))
            self.k(['wait', '-n', self.ns, '--for=condition=Ready', 'pod/'+name, '--timeout=60s'], timeout=65)
            self.setup['clients'].append({'name': name, 'region': region, 'group_receipt': group, 'binding_receipt': binding, 'config': config})
            save(self.out/f'setup-bound-{i}.json', self.setup)
        self.wait('both groups ready', self.group_ready)
        for name in [self.pods[n] for n in (1, 2, 3)]+['client-0', 'client-1']:
            hashes = self.exec(name, ['sha256sum', '/usr/local/bin/kv9', '/usr/local/bin/kv9-routed-workload']).stdout.decode().splitlines()
            require([x.split()[0] for x in hashes] == [self.a.server_sha256, self.a.workload_sha256], 'running image executable mismatch')
            pod = self.get('pod', name); cs = pod['status']['containerStatuses'][0]
            require(cs['imageID'].removeprefix('containerd://').split('@')[-1] in self.image_ids, 'Pod running another image digest')
            save(self.out/f'image-{name}.json', {'pod': name, 'pod_uid': pod['metadata']['uid'], 'image_id': cs['imageID'],
                 'server_sha256': hashes[0].split()[0], 'workload_sha256': hashes[1].split()[0]})
        self.token = None

    def pod(self, name, command, node=None, labels=None):
        env = [{'name': k, 'valueFrom': {'secretKeyRef': {'name': 'auth', 'key': v}}} for k, v in
               [('KV9_CLUSTER_TOKEN', 'cluster'), ('KV9_CLIENT_TOKENS', 'clients'), ('KV9_CLIENT_TOKEN', 'client'), ('KV9_BOOTSTRAP_TOKEN', 'bootstrap')]]
        container = {'name': 'kv9' if node else 'client', 'image': self.a.image, 'imagePullPolicy': 'Never', 'command': command,
             'env': env, 'resources': {'limits': {'cpu': '2', 'memory': '512Mi'}}}
        spec = {'restartPolicy': 'Never', 'terminationGracePeriodSeconds': 2, 'containers': [container]}
        if node:
            spec['volumes'] = [{'name': 'data', 'persistentVolumeClaim': {'claimName': f'data-{node}'}}]
            container['volumeMounts'] = [{'name': 'data', 'mountPath': '/data'}]
        return {'apiVersion': 'v1', 'kind': 'Pod', 'metadata': {'name': name, 'namespace': self.ns, 'labels': labels or {'app': 'kv9-routed-maintenance'}}, 'spec': spec}

    def start_client(self, i):
        row = self.setup['clients'][i]
        self.exec(row['name'], ['/bin/bash', '-c', 'umask 077; cat > /tmp/config.json'], data=json.dumps(row['config']).encode())
        client = Client(self, row); save(self.out/f'setup-client-{i}.json', self.setup); return client

    @staticmethod
    def fault_ids(doc):
        return sorted((x['kind'], x['metadata']['namespace'], x['metadata']['name'], x['metadata']['uid']) for x in doc['items'])

    def inject(self, phase, kind, spec):
        name = phase; self.apply({'apiVersion': 'chaos-mesh.org/v1alpha1', 'kind': kind, 'metadata': {'name': name, 'namespace': self.ns}, 'spec': spec})
        self.k(['wait', '-n', self.ns, '--for=condition=AllInjected', kind.lower()+'/'+name, '--timeout=30s'], timeout=35)
        fault = self.get(kind.lower(), name)
        require(any(x['type'] == 'AllInjected' and x['status'] == 'True' for x in fault['status']['conditions']), 'fault injection absent')
        row = {'phase': phase, 'namespace_uid': self.uid, 'fault': fault, 'injected_observed_ns': time.time_ns()}; self.faults.append(row); return row
    def selector(self, pods): return {'namespaces': [self.ns], 'pods': {self.ns: pods}}
    def partition(self, phase, source, targets):
        return self.inject(phase, 'NetworkChaos', {'action': 'partition', 'mode': 'all', 'selector': self.selector(source),
             'direction': 'both', 'target': {'mode': 'all', 'selector': self.selector(targets)}})
    def probe(self, source, node):
        r = self.exec(source, ['timeout', '2', '/bin/bash', '-c', f'exec 3<>/dev/tcp/{self.services[node].split(":")[0]}/20160'], allow=(1, 124))
        return {'from': source, 'to': node, 'exit_code': r.returncode, 'observed_ns': time.time_ns()}
    def voter_edges(self): return [self.probe(self.pods[a], b) for a in (1, 2, 3) for b in (1, 2, 3) if a != b]
    def finish_fault(self, row):
        current = self.get(row['fault']['kind'].lower(), row['phase'])
        require(current['metadata']['uid'] == row['fault']['metadata']['uid'] and
                any(x['type'] == 'AllInjected' and x['status'] == 'True' for x in current['status']['conditions']), 'fault healed/changed before phase end')
        row['fault_after_progress'] = current; row['progress_finished_ns'] = time.time_ns()
        save(self.out/f'fault-{row["phase"]}.json', row)
        self.k(['delete', row['fault']['kind'].lower(), row['phase'], '-n', self.ns, '--wait=true', '--timeout=30s'], timeout=35)

    def exercise(self):
        c0 = self.start_client(0)
        c0.progress('baseline')
        # Positive discovery failover retains reachable metadata/data leaders.
        # Client-only loss of a leader does not itself elect its replacement.
        data_leader = self.group_leader(self.regions[1])
        metadata_leader = self.wait('metadata leader before seed isolation', self.meta_leader)
        seed = next(n for n in (1, 2, 3) if n not in (data_leader, metadata_leader))
        row1 = self.setup['clients'][1]; row1['config']['seeds'].sort(key=lambda x: (x['node_id'] != seed, x['node_id']))
        fault = self.partition('discovery-seed-partition', ['client-1'], [self.pods[seed]])
        fault['effect'] = {'blocked': [self.probe('client-1', seed)], 'connected': self.voter_edges()}
        fault['seed'] = seed; fault['data_leader_before'] = data_leader
        fault['metadata_leader_before'] = metadata_leader
        c1 = self.start_client(1); c1.progress('discovery-seed-partition'); c0.progress('discovery-seed-partition')
        self.finish_fault(fault)
        for c in self.clients: c.progress('baseline')
        victim = self.group_leader(self.regions[0]); before = self.status(victim)
        fault = self.inject('container-kill', 'PodChaos', {'action': 'container-kill', 'mode': 'all', 'containerNames': ['kv9'], 'selector': self.selector([before['pod']])})
        def replaced():
            row = self.status(victim)
            return row if row and row['container_id'] != before['container_id'] else None
        after = self.wait('actual killed container replaced', replaced, 60)
        fault.update(before=before, after=after)
        self.wait('groups after container kill', self.group_ready)
        for c in self.clients: c.progress('container-kill')
        self.finish_fault(fault)
        victim = self.group_leader(self.regions[0]); before = self.status(victim); others = [n for n in (1, 2, 3) if n != victim]
        fault = self.partition('leader-partition', [self.pods[victim]], [self.pods[n] for n in others]); fault['before'] = before
        fault['effect'] = {'blocked': [self.probe(self.pods[victim], others[0]), self.probe(self.pods[others[0]], victim)],
                           'connected': [self.probe(self.pods[others[0]], others[1]), self.probe(self.pods[others[1]], others[0])]}
        fault['majority'] = self.wait('majority groups elect', lambda: self.group_ready(tuple(others)))
        def minority_nonleader():
            row = self.status(victim)
            if row:
                group = next(g for g in row['groups'] if g['region'] == self.regions[0])
                if group['role'] in ('Follower', 'Candidate'): return row
            return None
        minority = self.wait('isolated old leader steps down', minority_nonleader, 30)
        key = b'minority-control'
        argv = ['/usr/local/bin/kv9', 'client', 'raw-put', '--addr', self.services[victim], '--keyspace',
                str(self.setup['clients'][0]['config']['keyspace_id']), '--key-hex', key.hex(), '--value-hex', b'must-not-commit'.hex()]
        started = time.time_ns(); response = self.exec('client-0', argv, allow=(1,), timeout=10)
        stderr = response.stderr.decode()
        require(response.returncode == 1 and not response.stdout and re.fullmatch(
            r'not_leader=true leader_node_id=(?:[1-9][0-9]*|unknown)\n(?:command terminated with exit code 1\n)?', stderr), 'minority probe did not return exclusive NotLeader')
        fault['minority_control'] = {'node': victim, 'region': self.regions[0], 'keyspace': self.setup['clients'][0]['config']['keyspace_id'],
            'key': list(key), 'argv': argv, 'exit_code': response.returncode, 'stdout': response.stdout.decode(), 'stderr': stderr,
            'started_ns': started, 'finished_ns': time.time_ns(), 'before': minority, 'retried': False}
        for c in self.clients: c.progress('leader-partition')
        self.finish_fault(fault); self.wait('healed group voters', self.group_ready)
        check = c0.call('healed', {'kind': 'get', 'key': list(b'minority-control')})
        require(check['report']['outcome']['kind'] == 'success' and check['report']['outcome']['value']['value'] is None, 'minority refused mutation became visible')
        # Derive the endpoint from an actual successful client attempt, never direct clients using status.
        c0.progress('healed'); endpoint = c0.results[-1]['report']['attempts'][-1]['node_id']
        fault = self.partition('client-endpoint-partition', ['client-0'], [self.pods[endpoint]])
        fault['endpoint'] = endpoint; fault['effect'] = {'blocked': [self.probe('client-0', endpoint)], 'connected': self.voter_edges()}
        fault['groups_before'] = self.wait('voter quorum remains intact', self.group_ready)
        # Failure/unavailability is legitimate for the isolated client. Never replay these writes.
        for _ in range(3):
            key = list(f'call-{c0.number+1:04d}'.encode()); value = list(f'client-0:isolated:{c0.number+1}'.encode())
            c0.call('client-endpoint-partition', {'kind': 'put', 'key': key, 'value': value})
        c1.progress('client-endpoint-partition')
        fault['groups_after'] = self.wait('voter quorum still intact', self.group_ready)
        self.finish_fault(fault)
        for c in self.clients:
            c.progress('healed')
            # Observe EVERY submitted mutation, including Unknown; this is a read, never a write replay.
            submitted = [(r['operation']['key'], r['operation']['value'], r['report']['outcome']['kind'])
                         for r in c.results if r['operation']['kind'] == 'put']
            for key, value, outcome in submitted:
                r = c.call('healed', {'kind': 'get', 'key': key})
                require(r['report']['outcome']['kind'] == 'success', 'final mutation-key read failed')
                actual = r['report']['outcome']['value']['value']
                allowed = [value] if outcome == 'success' else [None, value] if outcome == 'unknown_write' else [None]
                require(actual in allowed, 'final mutation-key value violates its retained outcome')
            c.close()
        save(self.out/'setup.json', self.setup)
        first = self.wait('first group drain', self.group_ready)
        def fresh():
            second = self.group_ready()
            return second if second and all(int(b['status']['metrics_export_successes']) > int(a['status']['metrics_export_successes']) and a['process'] == b['process'] for a, b in zip(first, second)) else None
        second = self.wait('fresh independently exported group drain', fresh)
        save(self.out/'drains.json', {'first': first, 'second': second})

    def archive(self):
        for n in (1, 2, 3): self.k(['scale', '-n', self.ns, 'deployment/n'+str(n), '--replicas=0'])
        self.wait('owned voters stopped', lambda: not any(p['metadata'].get('labels', {}).get('app') == 'kv9-routed-voter' for p in self.get('pods')['items']), 45)
        self.wait('observed voter process identities ended',
                  lambda: all(self.host_process(p['pid']) != p for p in self.host_processes.values()), 15)
        archives = []
        for n in (1, 2, 3):
            name = f'archive-{n}'; self.apply(self.pod(name, ['sleep', '300'], n))
            self.k(['wait', '-n', self.ns, '--for=condition=Ready', 'pod/'+name, '--timeout=45s'], timeout=50)
            output = self.out/f'store-{n}.tar'; argv = self.kargs(['exec', '-n', self.ns, name, '--', 'tar', '-C', '/data', '-cf', '-', '.'])
            err = (self.out/f'archive-{n}.stderr').open('xb')
            child = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=err, start_new_session=True)
            count = 0; started = time.monotonic(); selector = selectors.DefaultSelector()
            selector.register(child.stdout, selectors.EVENT_READ)
            try:
                with output.open('xb') as f:
                    while selector.get_map():
                        self.guard(); require(time.monotonic()-started <= 120, 'store archive deadline')
                        for key, _ in selector.select(.5):
                            block = os.read(key.fileobj.fileno(), 1024**2)
                            if not block: selector.unregister(key.fileobj); continue
                            count += len(block); require(count <= MAX_STORE, 'store tar cap'); f.write(block)
                    f.flush(); os.fsync(f.fileno())
                require(child.wait(timeout=10) == 0, 'store tar child failed')
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGTERM)
                    try: child.wait(timeout=10)
                    except subprocess.TimeoutExpired: os.killpg(child.pid, signal.SIGKILL); child.wait(timeout=10)
                save(self.out/f'archive-{n}-child.json', {'argv': argv, 'pid': child.pid, 'exit_code': child.returncode, 'bytes': count})
                selector.close(); child.stdout.close(); err.close()
            members = {}
            with tarfile.open(output, 'r:') as tar:
                for m in tar:
                    key = m.name.removeprefix('./'); require(key not in members and (m.isdir() or m.isfile()) and not key.startswith('/') and '..' not in Path(key).parts, 'unsafe archive member')
                    members[key] = {'directory': True} if m.isdir() else {'bytes': m.size, 'sha256': hashlib.file_digest(tar.extractfile(m), 'sha256').hexdigest()}
            entry = {'file': output.name, 'node': n, 'pvc_uid': self.stores[n], 'bytes': count, 'sha256': sha(output), 'members': members, 'child_exit_code': child.returncode}
            self.checker.verify_archive(output, entry); archives.append(entry)
        save(self.out/'archives.json', archives)

    def cleanup(self):
        require(self.get('namespace', self.ns, ns=False)['metadata']['uid'] == self.uid and self.ns not in self.protected, 'cleanup ownership changed')
        self.k(['delete', 'namespace', self.ns, '--wait=true', '--timeout=60s'], timeout=65)
        namespaces = {x['metadata']['name']: x['metadata']['uid'] for x in self.get('namespaces', ns=False)['items']}
        require(namespaces == self.protected, 'historical namespace set changed')
        faults = self.fault_ids(json.loads(self.k(['get', 'podchaos,networkchaos,iochaos', '-A', '-o', 'json']).stdout))
        require(faults == self.fault_ids(json.loads(self.protected_faults)), 'historical fault set changed')
        # Kubelet may garbage-collect stopped container records immediately.
        # Retain that fact separately from a directly observed exit code.
        cri = json.loads(self.command(['docker', 'exec', self.kind_node, 'crictl', 'ps', '-a', '-o', 'json']).stdout)
        remaining = {c['id'] for c in cri['containers']}
        tasks = set(self.command(['docker', 'exec', self.kind_node, 'ctr', '-n', 'k8s.io', 'tasks', 'list', '-q']).stdout.decode().split())
        states = []
        for cid in sorted(self.cids):
            old = self.host_processes[cid]
            require(self.host_process(old['pid']) != old, 'recorded server process still live')
            inspection = self.inspect_container(cid)
            if inspection is None:
                bare = cid.removeprefix('containerd://')
                require(bare not in remaining and bare not in tasks, 'unresolved container disappearance')
                state = {'id': bare, 'state': 'GARBAGE_COLLECTED', 'exitCode': None,
                         'cri_absent': True, 'task_absent': True}
            else:
                status = inspection['status']
                require(status['state'] == 'CONTAINER_EXITED', 'recorded server container still live')
                state = {k: status.get(k) for k in ['id', 'state', 'exitCode', 'startedAt', 'finishedAt']}
            states.append(dict(state, process=old, process_absent=True))
        save(self.out/'cleanup.json', {'complete': True, 'namespace': self.ns, 'namespace_uid': self.uid, 'namespace_absent': True,
             'protected_namespaces': namespaces, 'protected_faults': faults, 'recorded_server_containers': states})

    def run(self):
        try:
            self.prepare(); self.exercise(); self.archive(); self.result['runtime_complete'] = True
            save(self.out/'result.json', self.result)
            result = self.checker.audit(self.out); save(self.out/'independent-before-cleanup.json', result)
            self.cleanup(); self.result['cleanup_complete'] = True
            save(self.out/'accepted.json', self.result)
        except BaseException as e:
            self.result['failure'] = repr(e)
            # No deletion on failure. Close exact local exec transports and retain actual remaining resources.
            for c in self.clients:
                if not c.closed:
                    try: c.close()
                    except BaseException as error:
                        self.result.setdefault('client_close_errors', []).append(repr(error))
                        if c.child.poll() is None:
                            os.killpg(c.child.pid, signal.SIGTERM)
                            try: c.child.wait(timeout=10)
                            except subprocess.TimeoutExpired: os.killpg(c.child.pid, signal.SIGKILL); c.child.wait(timeout=10)
                        save(c.directory/'failure-lifetime.json', {'host_exit_code': c.child.returncode, 'remote_identity': c.row.get('process'),
                             'remote_exit_confirmed': False, 'namespace_retained': self.ns})
            save(self.out/'failure.json', self.result)
            raise
        finally:
            save(self.out/'resource-samples.json', self.samples)
            save(self.out/'status-observations.json', self.snapshots)
            save(self.out/'stale-status-observations.json', self.stale_status)
            save(self.out/'owned-resources.json', {'namespace': self.ns, 'namespace_uid': self.uid,
                 'server_container_ids': sorted(self.cids), 'cleanup_complete': self.result['cleanup_complete'],
                 'kind_processes': self.host_processes,
                 'client_host_pids': [c.child.pid for c in self.clients], 'client_host_exits': [c.child.poll() for c in self.clients], 'failure_preserves_resources': True})


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['server', 'qualification', 'kubeconfig', 'kind', 'output']:
        p.add_argument('--'+name, type=Path, required=True)
    for name in ['server-sha256', 'workload-sha256', 'image', 'image-id', 'kind-cluster']:
        p.add_argument('--'+name, required=True)
    p.add_argument('--kubectl', type=Path, default=Path('/usr/local/bin/kubectl'))
    args = p.parse_args(); Gate(args).run()


if __name__ == '__main__':
    main()
