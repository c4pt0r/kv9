#!/usr/bin/env python3
"""One actual pre-upload container kill, with one explicit leader-transfer action.

Uses only the separately frozen checkpoint-testing binary. No retention mutation
is issued by this fixture, and unconfirmed/deferred work is never settlement.
"""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import struct
import subprocess
import time
import zlib

spec = importlib.util.spec_from_file_location('automatic_owner', Path(__file__).with_name('checkpoint-owner-chaos.py'))
automatic = importlib.util.module_from_spec(spec)
spec.loader.exec_module(automatic)
base = automatic.base
require = base.require
TESTING_SHA = '16b4e0325e3bd581195255b9afa70942043ee499a8e7b9edc226e688c00fa014'
TESTING_INPUTS = Path('/mnt/data/kv9-work/checkpoint-owner-testing-inputs-20260915-first/inputs.json')
FIXTURE_INPUTS = Path('/mnt/data/kv9-work/checkpoint-owned-crash-preparation-20260915-first/fixture-inputs.json')
JOURNAL_LIMIT = 49 * base.MIB + 128
GATE_DIR = '/data/owner-gates'


class Bytes:
    def __init__(self, data):
        self.data = data
        self.at = 0

    def take(self, n):
        require(0 <= n <= len(self.data) - self.at, 'truncated payload')
        out = self.data[self.at:self.at + n]
        self.at += n
        return out

    def uint(self, n):
        return int.from_bytes(self.take(n), 'little')

    def field(self):
        n = self.uint(4)
        require(n <= 64 * base.MIB, 'SST field cap')
        return self.take(n)


def decode_sst(data, reference, needle=None):
    require(len(data) == reference['size'] and hashlib.sha256(data).hexdigest() == reference['sha256'],
            'journal object exact size/hash')
    require(len(data) >= 22 and len(data) <= 48 * base.MIB, 'checkpoint SST length')
    require(zlib.crc32(data[:-4]) == int.from_bytes(data[-4:], 'little'), 'SST CRC32')
    b = Bytes(data[:-4])
    require(b.take(5) == b'KV9S\1', 'SST magic/version')
    cf, count = b.uint(1), b.uint(4)
    require(cf in (0, 1, 2) and cf == reference['cf'] and 0 < count <= 16 * 1024**2,
            'SST family/count')
    first = last = None
    found = False
    for _ in range(count):
        key, value = b.field(), b.field()
        require(last is None or last < key, 'SST sorted unique keys')
        if first is None:
            first = key
        last = key
        found |= needle is not None and value == needle
    smallest, largest = b.field(), b.field()
    require(b.at == len(b.data) and smallest == first and largest == last, 'SST footer/end')
    require(list(first) == reference['smallest'] and list(last) == reference['largest']
            and count == reference['count'], 'SST reference range/count')
    return {'bytes': len(data), 'sha256': reference['sha256'], 'cf': cf, 'count': count,
            'trigger_value_present': found}


def decode_journal(data, root, *, version, needle=None):
    require(51 < len(data) <= JOURNAL_LIMIT, 'journal byte bound')
    require(data.startswith(b'KV9PENDING' + bytes([version])) and version in (1, 2),
            'exact journal version')
    require(hashlib.sha256(data[:-32]).digest() == data[-32:], 'journal checksum')
    b = Bytes(data[11:-32])
    generation = b.uint(8)
    require(generation < 2**64 - 1, 'journal exhausted predecessor')
    manifest = b.take(b.uint(4)) if version == 2 else b.take(len(b.data) - b.at)
    binding = automatic.derive(root, manifest, generation)
    require(0 < len(binding['resources']) <= 64, 'production owner resource cap')
    objects, checked = [], []
    total = 0
    if version == 2:
        for reference in binding['manifest']['files']:
            total += reference['size']
            require(total <= 48 * base.MIB, 'whole checkpoint payload cap')
            raw = b.take(reference['size'])
            checked.append(decode_sst(raw, reference, needle))
            objects.append(raw)
    require(b.at == len(b.data), 'journal trailing bytes')
    return {'version': version, 'binding': binding, 'manifest_bytes': manifest,
            'objects': objects, 'object_checks': checked}


def missing_object(raw, code):
    """A typed S3 key absence; generic mc/read/auth errors never qualify."""
    rows = [json.loads(line) for line in raw.decode().splitlines() if line.strip()]
    require(len(rows) == 1 and isinstance(rows[0], dict), 'one typed mc stat result')
    row = rows[0]
    if code == 0:
        require(row.get('status') == 'success', 'mc stat success status')
        return False
    require(code == 1 and row.get('status') == 'error', 'mc stat typed error status')
    codes = []
    def walk(value):
        if isinstance(value, dict):
            for key, child in value.items():
                if key.lower() == 'code' and isinstance(child, str):
                    codes.append(child)
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)
    walk(row.get('error'))
    require(codes and set(codes) == {'NoSuchKey'}, 'only typed NoSuchKey counts as absence')
    return True


def varint(value):
    require(type(value) is int and 0 <= value < 2**64, 'protobuf u64')
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def integer(field, value):
    return varint(field << 3) + varint(value)


def blob(field, value):
    return varint((field << 3) | 2) + varint(len(value)) + value


def transfer_request(root_digest, source, destination):
    require(len(root_digest) == 32 and source in (1, 2, 3) and destination in (1, 2, 3)
            and source != destination, 'exact distinct owned voter transfer')
    # eraftpb.Message: MsgTransferLeader=13, to=current leader, from=transferee;
    # term deliberately omitted (=0), matching the local transfer seam.
    message = integer(1, 13) + integer(2, destination) + integer(3, source)
    envelope = integer(1, 1) + integer(2, source) + integer(3, destination) + blob(4, message)
    batch = blob(1, envelope) + blob(3, root_digest)
    return b'\0' + struct.pack('>I', len(batch)) + batch


def grpc_done(headers, body):
    lines = headers.decode('ascii').splitlines()
    status = [x for x in lines if x.startswith('HTTP/')]
    trailers = [x.split(':', 1)[1].strip() for x in lines if x.lower().startswith('grpc-status:')]
    require(status == ['HTTP/2 200'] and trailers == ['0'] and body == bytes(5),
            'exact HTTP2 gRPC successful Done; transport exit alone is insufficient')


def transferred(rows, target, previous_term):
    return (rows and set(rows) == {1, 2, 3} and rows[target]['role'] == 'leader'
            and int(rows[target]['term']) > previous_term
            and all(row['leader_id'] == str(target) and row['term'] == rows[target]['term']
                    and row['bootstrap_state'] == 'Serving' and not row['fatal']
                    for row in rows.values()))


def settlement(logs, node, generation, cut):
    """Associate a typed worker settlement with its own exact preceding attempt."""
    selected = False
    found = []
    for line in logs.decode().splitlines():
        start = re.fullmatch(r'node (\d+) checkpoint pending generation=(\d+) through=(\d+) resuming', line)
        if start and int(start[1]) == node:
            selected = (int(start[2]), int(start[3])) == (generation, cut)
        done = re.fullmatch(r'node (\d+) checkpoint pending settled (.+)', line)
        if done and int(done[1]) == node:
            if selected:
                found.append(done[2])
            selected = False
    require(len(set(found)) <= 1, 'contradictory typed attempt settlements')
    if not found:
        return None
    value = found[0]
    success = re.fullmatch(r'(MyChangeApplied|AlreadyApplied) \{ generation: (\d+) \}', value)
    if success:
        require(int(success[2]) == generation + 1, 'positive settlement generation')
        return {'class': 'published', 'variant': success[1], 'raw': value}
    if value == 'EffectSettled':
        return {'class': 'published', 'variant': value, 'raw': value}
    stale = re.fullmatch(r'ReceiptRefused \{ current_generation: (\d+) \}', value)
    window = re.fullmatch(r'WindowRefused \{ winner_change_id: \[([0-9, ]+)\] \}', value)
    if stale:
        require(int(stale[1]) > generation, 'strictly spent predecessor')
        return {'class': 'competing-predecessor-pinned', 'variant': 'ReceiptRefused', 'raw': value}
    if window:
        winner = bytes(int(x.strip()) for x in window[1].split(','))
        require(len(winner) == 32, 'winner identity length')
        return {'class': 'competing-predecessor-pinned', 'variant': 'WindowRefused',
                'winner_change_id': winner.hex(), 'raw': value}
    raise RuntimeError('unaccepted typed attempt outcome: ' + value)


class OwnedCrashCell(base.Cell):
    def __init__(self, args):
        super().__init__(args)
        self.owner_history = []
        self.binding = None
        self.target = None
        self.journal_before = None
        self.schedule = None
        self.port_forward = None
        self.port_forward_began = None

    def guard(self):
        super().guard()
        headers = self.out / 'leadership-transfer.headers'
        require(not headers.exists() or headers.stat().st_size <= base.MIB, 'bounded response headers')
        folder = self.out / 'leadership-port-forward'
        if folder.exists():
            require(sum(p.stat().st_size for p in folder.iterdir() if p.name in ('stdout', 'stderr'))
                    <= base.MIB, 'bounded port-forward output')
        if self.port_forward is not None:
            require(self.port_forward.poll() is None and time.monotonic() - self.port_forward_began < 45,
                    'owned port-forward lifetime bound')

    def run(self):
        require(self.args.binary_sha256 == TESTING_SHA and self.args.inputs.resolve() == TESTING_INPUTS,
                'only exact separately frozen checkpoint-testing inputs')
        inputs = json.loads(TESTING_INPUTS.read_text())
        require(inputs['binary_sha256'] == TESTING_SHA and inputs['binary_bytes'] == 46985360,
                'testing binary identity')
        require(all(n in inputs['source_pins'] for n in automatic.REQUIRED_SOURCES),
                'worker/journal source pins')
        authority = json.loads(FIXTURE_INPUTS.read_text())
        for row in authority['files']:
            require(base.pin(row['path']) == row, 'fixture tool/protocol input changed')
        (self.out / 'checkpoint-owned-crash.py').write_bytes(Path(__file__).read_bytes())
        (self.out / 'checkpoint-owner-chaos.py').write_bytes(Path(automatic.__file__).read_bytes())
        self.save('owned-source-binding.json', {'helper': base.pin(__file__),
            'owner_derivation': base.pin(automatic.__file__), 'base_helper': base.pin(base.__file__),
            'testing_inputs': base.pin(TESTING_INPUTS), 'testing_build_terminal': base.pin(Path(inputs['build_terminal'])),
            'fixture_inputs': base.pin(FIXTURE_INPUTS),
            'scope': 'One pre-upload PodChaos plus at most one explicit authenticated Raft leadership-transfer request.'})
        super().run()

    def apply(self, obj, *, sensitive=False):
        if obj.get('kind') == 'Deployment' and obj['metadata']['name'].startswith('kv9-n'):
            obj = copy.deepcopy(obj)
            containers = obj['spec']['template']['spec']['containers']
            require(len(containers) == 1 and containers[0]['name'] == 'kv9', 'owned worker Deployment')
            env = containers[0]['env']
            require(not any(x['name'] == 'KV9_TESTING_FLUSH_PAUSE_DIR' for x in env), 'testing gate env collision')
            env.append({'name': 'KV9_TESTING_FLUSH_PAUSE_DIR', 'value': GATE_DIR})
        super().apply(obj, sensitive=sensitive)

    def read_owner(self, name):
        return automatic.OwnerCell.read_owner(self, name)

    def exact_owner(self, name, phase):
        got = self.read_owner(name)
        expected = {'found': 'false'} if phase is None else {
            'found': 'true', 'owner_hex': automatic.owner_bytes(self.binding, name, phase).hex()}
        require(got == expected, 'exact cut owner state: ' + name)

    def gate(self, name):
        return f'{GATE_DIR}/{self.target}-{name}'

    def arrived(self, name):
        raw, code = self.exec(self.pods[self.target], '/bin/sh', '-c',
            'test ! -L "$1" && test ! -L "$2" || exit 2; test -f "$1" && test -f "$2" || exit 3; cat "$2"',
            'gate', self.gate(name), self.gate(name) + '.arrived', allowed=(0, 3))
        if code:
            return False
        require(raw == b'paused', 'exact pause marker')
        return True

    def pending(self, label=None, *, version=None):
        raw, code = self.exec(self.pods[self.target], '/bin/sh', '-c',
            'test ! -L /data/catalog.pending || exit 2; if test -f /data/catalog.pending; then cat /data/catalog.pending; '
            'elif test ! -e /data/catalog.pending; then exit 3; else exit 2; fi',
            allowed=(0, 3), cap=JOURNAL_LIMIT, payload=True)
        if code:
            return None
        require(version in (1, 2), 'caller selects exact pending journal version')
        parsed = decode_journal(raw, (self.out / 'root.bin').read_bytes(), version=version,
                                needle=getattr(self, 'trigger_value', None))
        if label:
            folder = self.out / 'owned-payloads'
            folder.mkdir(exist_ok=True)
            path = folder / (label + '.pending')
            os.link(self.out / 'commands' / f'{self.count:05d}' / 'stdout', path)
            self.save(label + '-journal.json', {'journal': base.pin(path), 'command_number': self.count,
                'version': version, 'binding': parsed['binding'], 'objects': parsed['object_checks']})
        return raw, parsed

    def drop_gate(self, name):
        self.exec(self.pods[self.target], '/bin/sh', '-c',
                  'test -f "$1" && test ! -L "$1" && test -f "$2" && test ! -L "$2" || exit 2; '
                  'test "$(cat "$2")" = paused || exit 2; rm -- "$1"',
                  'gate', self.gate(name), self.gate(name) + '.arrived')
        self.save('gate-' + name + '-removed.json', {'gate': self.gate(name), 'command_number': self.count,
                  'scope': 'Exact owned test gate only; arrived marker and journal retained.'})

    def before_fault(self):
        self.phase = 'owned-cut-arm'
        self.target = self.leader_id
        self.arm_pod = self.pod(self.target)
        def idle():
            rows = self.statuses()
            require(rows and set(rows) == {1, 2, 3} and self.agreement(rows) and self.leader_id == self.target,
                    'complete unchanged leader while arming')
            _, code = self.exec(self.pods[self.target], '/bin/sh', '-c',
                               'test ! -L /data/catalog.pending && test ! -e /data/catalog.pending', allowed=(0, 1))
            return code == 0
        self.wait('old pending slot clear before gate', idle)
        self.prior_checkpoint = self.layout(self.target, 'owned-prior-base').checkpoint
        self.exec(self.pods[self.target], '/bin/sh', '-c',
            'set -eu; test ! -e "$1" && test ! -L "$1"; mkdir -m 700 "$1"; '
            '(set -C; : > "$2"; : > "$3"); sync -f "$1"',
            'gate', GATE_DIR, self.gate('owned'), self.gate('prepared'))
        self.trigger_value = ('owned-pre-upload-' + self.namespace).encode()
        self.trigger = self.put(b'owned-pre-upload-trigger', self.trigger_value)
        self.wait('actual pre-upload owned gate arrival', lambda: self.arrived('owned'))
        require(self.agreement() and self.leader_id == self.target, 'same leader at actual owned cut')
        captured = self.pending('owned-before', version=2)
        require(captured is not None, 'durable v2 journal at owned gate')
        self.journal_before, parsed = captured
        self.binding = parsed['binding']
        require(self.binding['manifest']['index'] >= int(self.trigger['applied_index']) and
                any(x['trigger_value_present'] for x in parsed['object_checks']), 'new acknowledged value in actual planned SST')
        self.exact_owner('pending', 2)
        self.exact_owner('version', None)
        self.save('owned-binding.json', {'binding': self.binding, 'trigger_receipt': self.trigger,
                  'journal': base.pin(self.out / 'owned-payloads/owned-before.pending'),
                  'selected_prior_base': self.prior_checkpoint})
        files = []
        for n, (reference, data) in enumerate(zip(self.binding['manifest']['files'], parsed['objects'])):
            path = self.out / 'owned-payloads' / f'planned-{n:03d}.sst'
            path.write_bytes(data)
            files.append({'reference': reference, 'retained': base.pin(path)})
        self.save('owned-object-inventory.json', {'complete': True, 'files': files})
        absences = []
        for n, reference in enumerate(self.binding['manifest']['files']):
            raw, code = self.exec('minio', '/bin/sh', '-c',
                'export MC_HOST_local="http://${MINIO_ROOT_USER}:${MINIO_ROOT_PASSWORD}@127.0.0.1:9000"; '
                'mc stat --json "local/$1/$2"', 'stat', self.bucket, reference['key'], allowed=(0, 1))
            absent = missing_object(raw, code)
            absences.append({'file': n, 'key': reference['key'], 'typed_absent': absent,
                             'command_number': self.count, 'exit_code': code})
        require(any(x['typed_absent'] for x in absences), 'at least one fresh object absent before worker PUT')
        require(self.arrived('owned') and self.pending(version=2)[0] == self.journal_before,
                'same paused v2 plan after absence observation')
        self.save('owned-pre-upload.json', {'complete': True, 'minio_stats': absences,
                  'pending_published': True, 'version_absent': True, 'actual_owned_gate': self.gate('owned'),
                  'scope': 'Production worker paused before recover/PUT; fixture stat reads are explicitly separate.'})
        for n in (1, 2, 3):
            (self.out / f'owned-before-n{n}.log').write_bytes(self.k('logs', '-n', self.namespace, self.pods[n], '-c', 'kv9')[0])

    def choose_victim(self):
        require(self.leader_id == self.target, 'kill only the agreed gated leader')
        current = self.pod(self.target)
        require(current['metadata']['uid'] == self.arm_pod['metadata']['uid'] and
                current['status']['containerStatuses'][0]['containerID'] ==
                self.arm_pod['status']['containerStatuses'][0]['containerID'], 'armed old process remains exact victim')
        return self.target

    def complete_status(self):
        rows = self.statuses()
        return rows if rows and set(rows) == {1, 2, 3} and self.agreement(rows) else None

    def transfer_once(self, rows):
        leader = self.leader_id
        previous_term = int(rows[leader]['term'])
        if leader == self.target:
            self.schedule = {'request_count': 0, 'outcome': 'already-target-leader', 'target': self.target,
                             'observed_status': rows, 'is_fault': False}
            self.save('leadership-scheduling.json', self.schedule)
            return
        request = transfer_request(bytes.fromhex(self.binding['root_digest']), self.target, leader)
        request_path = self.out / 'leadership-transfer.grpc'
        request_path.write_bytes(request)
        folder = self.out / 'leadership-port-forward'
        folder.mkdir()
        argv = ['kubectl', '--kubeconfig', base.KUBECONFIG, '--request-timeout=30s',
                'port-forward', '--address=127.0.0.1', '-n', self.namespace, 'pod/' + self.pods[leader], ':20160']
        self.save('leadership-port-forward/invocation.json', {'argv': argv, 'started_ns': time.time_ns(),
                  'maximum_seconds': 45, 'maximum_output_bytes': base.MIB})
        child = None
        birth = None
        sent = False
        error = None
        parent = os.getpid()
        signals = []
        start = time.monotonic()
        try:
            with (folder / 'stdout').open('xb') as stdout, (folder / 'stderr').open('xb') as stderr:
                child = subprocess.Popen(argv, env=self.env, stdout=stdout, stderr=stderr,
                                         start_new_session=True, preexec_fn=lambda: base.parent_death(parent))
                self.port_forward = child
                self.port_forward_began = start
                raw = Path('/proc', str(child.pid), 'stat').read_text()
                birth = {'pid': child.pid, 'start_ticks': int(raw[raw.rfind(')') + 2:].split()[19]),
                         'boot_id': Path('/proc/sys/kernel/random/boot_id').read_text().strip()}
                self.save('leadership-port-forward/child-start.json', birth)
                def forwarding():
                    self.guard()
                    require(child.poll() is None and time.monotonic() - start < 15, 'owned port-forward startup')
                    require(sum((folder / n).stat().st_size for n in ('stdout', 'stderr')) <= base.MIB,
                            'port-forward output bound')
                    text = (folder / 'stdout').read_text()
                    ports = re.findall(r'Forwarding from 127\.0\.0\.1:(\d+) -> 20160', text)
                    require(len(set(ports)) <= 1, 'one actual loopback forwarded port')
                    return int(ports[0]) if ports else None
                port = self.wait('owned loopback port-forward', forwarding, seconds=15)
                fresh = self.complete_status()
                require(fresh and self.leader_id == leader and int(fresh[leader]['term']) == previous_term
                        and all(int(row['driver_applied_index']) >= int(fresh[leader]['raft_committed'])
                                for row in fresh.values()), 'fresh caught-up committed log at exact transfer source term')
                require(self.pending(version=2)[0] == self.journal_before,
                        'exact armed v2 journal immediately before transfer')
                self.exec(self.pods[self.target], 'test', '-f', self.gate('owned'))
                self.save('leadership-before-request.json', {'statuses': fresh, 'previous_term': previous_term,
                    'target': self.target, 'destination': leader, 'request': base.pin(request_path)})
                headers = self.out / 'leadership-transfer.headers'
                # This private stdin is handled by base.command's mode-0600,
                # redacted-and-unlinked sensitive transport. No secret in argv.
                config = ('header = "kv9-cluster-token: ' + self.values['cluster'] + '"\n'
                          'header = "kv9-node-id: ' + str(self.target) + '"\n').encode()
                sent = True
                response, _ = self.command(['/usr/bin/curl', '--http2-prior-knowledge', '--silent', '--show-error',
                    '--max-time', '15', '--request', 'POST', '--header', 'content-type: application/grpc',
                    '--header', 'te: trailers', '--config', '-', '--dump-header', str(headers),
                    '--data-binary', '@' + str(request_path),
                    f'http://127.0.0.1:{port}/kv9.raft.v2.Kv9Raft/BatchRaft'], stdin=config, sensitive=True, seconds=20)
                header_bytes = self.redact(headers.read_bytes())
                headers.write_bytes(header_bytes)
                grpc_done(header_bytes, response)
                self.schedule = {'request_count': 1, 'request': base.pin(request_path), 'command_number': self.count,
                    'from_node': self.target, 'to_node': leader, 'root_digest': self.binding['root_digest'],
                    'previous_term': previous_term,
                    'message_type': 'MsgTransferLeader', 'is_fault': False, 'transport_confirmed': True,
                    'scope': 'One authenticated fixture scheduling request on behalf of the actual recovered voter; no product API added.'}
        except BaseException as exc:
            error = repr(exc)
            raise
        finally:
            if child is not None and child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
                signals.append('SIGTERM')
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(child.pid, signal.SIGKILL)
                    signals.append('SIGKILL')
                    child.wait(timeout=5)
            self.port_forward = None
            self.save('leadership-port-forward/terminal.json', {'complete': error is None,
                'error': error, 'identity': birth, 'exit_code': None if child is None else child.returncode,
                'reaped': child is not None and child.poll() is not None, 'signals': signals,
                'request_started': sent, 'elapsed_seconds': time.monotonic() - start})
        self.wait('one transfer observed on all voters in a newer term',
                  lambda: transferred(self.complete_status(), self.target, previous_term),
                  seconds=15)
        self.schedule.update(outcome='observed-target-leader', observed_status=self.statuses())
        self.save('leadership-scheduling.json', self.schedule)

    def after_container_restart(self, victim, before, after):
        self.phase = 'owned-reopened-v2-and-leadership-scheduling'
        require(victim == self.target and before['metadata']['uid'] == after['metadata']['uid']
                and before['spec']['volumes'] == after['spec']['volumes'], 'same restarted owned Pod/volumes')
        old = before['status']['containerStatuses'][0]
        new = after['status']['containerStatuses'][0]
        require(new['containerID'] != old['containerID'] and
                new['lastState']['terminated']['containerID'] == old['containerID'] and
                new['lastState']['terminated']['exitCode'] == 137, 'actual selected old death before unblocking')
        pvc = self.obj('get', 'pvc', f'data-n{victim}', '-n', self.namespace)
        original_pvc = json.loads((self.out / 'pvc-before.json').read_text())
        require(pvc['metadata']['uid'] == original_pvc['metadata']['uid'] and
                pvc['spec']['volumeName'] == original_pvc['spec']['volumeName'], 'same PVC/PV before transfer')
        require(self.exec(self.pods[victim], 'cat', '/data/kv9-store-lifecycle')[0]
                == (self.out / 'lifecycle-before.bin').read_bytes(), 'same store before gate removal')
        self.save('owned-reopened-pvc.json', pvc)
        rows = self.wait('Serving after actual startup journal validation', self.complete_status)
        barrier = max(int(row['raft_committed']) for row in rows.values())
        def caught_up():
            now = self.complete_status()
            return now if now and all(int(row['driver_applied_index']) >= barrier for row in now.values()) else None
        rows = self.wait('recovered owner caught up before transfer', caught_up)
        reopened = self.pending('owned-reopened', version=2)
        require(reopened is not None and reopened[0] == self.journal_before, 'exact v2 journal survives old process death')
        self.exec(self.pods[self.target], 'test', '-f', self.gate('owned'))
        self.exact_owner('pending', 2)
        self.exact_owner('version', None)
        self.save('owned-reopened-status.json', {'complete': True, 'statuses': rows, 'barrier': barrier,
            'old_container': old['containerID'], 'new_container': new['containerID'],
            'journal_authority': 'Exact bytes plus Serving: prepare_remote validates and syncs the journal before starting driver/listener threads. The old arrived marker is not a new-process arrival receipt.'})
        self.transfer_once(rows)
        require(self.complete_status() and self.leader_id == self.target, 'target is leader before owned gate removal')
        require(self.pending(version=2)[0] == self.journal_before, 'journal unchanged through transfer')
        self.drop_gate('owned')
        self.phase = 'owned-post-crash-upload-verification'
        self.wait('actual prepared gate after post-crash upload', lambda: self.arrived('prepared'), seconds=60)
        prepared = self.pending('owned-prepared', version=1)
        require(prepared is not None and prepared[1]['binding'] == self.binding, 'same identity v2-to-v1 prepared transition')
        remote = []
        for n, reference in enumerate(self.binding['manifest']['files']):
            raw = self.exec('minio', '/bin/sh', '-c',
                'export MC_HOST_local="http://${MINIO_ROOT_USER}:${MINIO_ROOT_PASSWORD}@127.0.0.1:9000"; '
                'mc cat "local/$1/$2"', 'get', self.bucket, reference['key'], cap=48 * base.MIB, payload=True)[0]
            require(raw == (self.out / 'owned-payloads' / f'planned-{n:03d}.sst').read_bytes(),
                    'exact original planned object now returned by MinIO')
            path = self.out / 'owned-payloads' / f'uploaded-{n:03d}.sst'
            os.link(self.out / 'commands' / f'{self.count:05d}' / 'stdout', path)
            remote.append({'reference': reference, 'retained': base.pin(path), 'command_number': self.count})
        self.save('owned-upload-readback.json', {'complete': True, 'files': remote,
                  'authority': 'Actual prepared worker gate and exact complete external GET bytes; not a fabricated PUT receipt.'})
        self.drop_gate('prepared')

    def after_recovery(self):
        self.phase = 'owned-attempt-settlement'
        def observe():
            logs = self.k('logs', '-n', self.namespace, self.pods[self.target], '-c', 'kv9')[0]
            outcome = settlement(logs, self.target, self.binding['predecessor_generation'], self.binding['manifest']['index'])
            if outcome:
                (self.out / 'owned-settlement.log').write_bytes(logs)
            return outcome
        outcome = self.wait('typed exact attempt settlement, not follower deferral', observe, seconds=60)
        if outcome['class'] == 'published':
            self.exact_owner('pending', 4)
            self.exact_owner('version', 2)
        else:
            if 'winner_change_id' in outcome:
                require(outcome['winner_change_id'] != self.binding['operation'], 'negative winner differs from own identity')
            self.exact_owner('pending', 2)
            self.exact_owner('version', None)
        # Any newer journal remains preserved; never clear a file from the fixture.
        raw, code = self.exec(self.pods[self.target], '/bin/sh', '-c',
            'test ! -L /data/catalog.pending || exit 2; if test -f /data/catalog.pending; then cat /data/catalog.pending; '
            'elif test ! -e /data/catalog.pending; then exit 3; else exit 2; fi',
            allowed=(0, 3), cap=JOURNAL_LIMIT, payload=True)
        if code == 0:
            version = raw[10] if len(raw) > 10 else 0
            current = decode_journal(raw, (self.out / 'root.bin').read_bytes(), version=version)
            require(current['binding']['operation'] != self.binding['operation'], 'settled original slot must be cleared')
        self.check(b'owned-pre-upload-trigger', self.trigger_value)
        rows = self.wait('complete final owned cut status', self.complete_status)
        self.save('owned-history.json', self.owner_history)
        self.save('owned-outcome.json', {'complete': True, 'attempt_settled': True, 'outcome': outcome,
            'flush_publication_success': outcome['class'] == 'published',
            'pending_released': outcome['class'] == 'published', 'negative_pending_retained': outcome['class'] != 'published',
            'binding': self.binding, 'leadership_scheduling': self.schedule, 'final_statuses': rows,
            'retention_mutations_by_fixture': 0, 'actual_podchaos_count': 1,
            'scope': 'Actual durable v2 pre-upload kill and same-store recovery, one explicit scheduling transfer if needed, exact typed settlement. A competing predecessor is a pinned negative result, never a successful publication.'})

    def k(self, *args, **kwargs):
        if args == ('scale', 'deployment', '--all', '-n', self.namespace, '--replicas=0'):
            for n in (1, 2, 3):
                (self.out / f'owned-final-n{n}.log').write_bytes(super().k('logs', '-n', self.namespace, self.pods[n], '-c', 'kv9')[0])
        return super().k(*args, **kwargs)

    def public_manifest(self):
        if self.owner_history and not (self.out / 'owned-history.json').exists():
            self.save('failed-owned-history.json', self.owner_history)
        # The base selector's exact exclusions plus this cell's explicit new
        # journal/object payload directory. Originals remain named and pinned.
        excluded = {'credentials.json', 'public-evidence-manifest.json', 'image-context/kv9'}
        for folder in (self.out / 'commands').iterdir():
            inv = folder / 'invocation.json'
            if inv.exists() and json.loads(inv.read_text()).get('stdout_is_payload'):
                excluded.add(str((folder / 'stdout').relative_to(self.out)))
        files, omitted, payloads = [], [], []
        for path in sorted(self.out.rglob('*')):
            if not path.is_file():
                continue
            name = str(path.relative_to(self.out))
            is_payload = name.startswith('owned-payloads/') or path.suffix == '.tar' or (path.name.startswith('selected-sst-') and path.suffix == '.bin')
            if is_payload:
                payloads.append(base.pin(path))
            if name in excluded or name == 'runtime-image-context/ca-certificates.crt' or is_payload:
                omitted.append({'path': str(path), 'reason': 'Private/executable/CA, duplicate payload stdout, or separately retained protocol payload.'})
            else:
                require(not path.is_symlink(), 'public evidence symlink')
                files.append(dict(relative_path=name, **base.pin(path)))
        self.save('owned-protocol-payloads.json', {'runtime_complete': (self.out / 'result.json').exists(),
            'files': payloads, 'scope': 'Exact journals/SSTs and all available stopped-store archives, including partial failure evidence.'})
        files.append(dict(relative_path='owned-protocol-payloads.json', **base.pin(self.out / 'owned-protocol-payloads.json')))
        self.save('public-evidence-manifest.json', {'complete': True, 'files': files, 'excluded': omitted,
                  'scope': 'Exact nonsecret metadata selector plus separately named protocol payload inventory; no credentials or CA bytes.'})


if __name__ == '__main__':
    base.Cell = OwnedCrashCell
    base.main()
