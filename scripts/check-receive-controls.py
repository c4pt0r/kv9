#!/usr/bin/env python3
"""Reject isolated receive-authority source faults in real runtime tests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
RUNTIME = 'crates/server/src/runtime.rs'
GRPC = 'crates/raft/src/grpc.rs'


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def replace_once(text, before, after):
    if text.count(before) != 1 or before == after:
        raise RuntimeError('source control anchor is not unique')
    return text.replace(before, after)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    sources = {p: (ROOT / p).read_text() for p in (RUNTIME, GRPC)}
    runtime, grpc = sources[RUNTIME], sources[GRPC]
    stale = 'fresh_joiner_rejects_stale_heartbeat_before_raft_owner_starts'
    routes = 'registration_refusals_preserve_routes_and_renewed_tickets_cannot_rebind'
    recover = 'a_real_joiner_follows_the_wire_hint_to_a_non_seed_leader_end_to_end'
    local_check = runtime[runtime.index('fn local_member_is_active('):runtime.index('/// A running real-process metadata member.')]
    bypass_local = replace_once(local_check, 'if !matches!', 'if false && !matches!')
    binding = runtime[runtime.index('        // A renewed ticket cannot authorize'):runtime.index('        match existing {')]
    bypass_binding = replace_once(binding, 'if !matches!',
        'if existing.as_ref().is_none_or(|admission| admission.state != kv9_meta::admission::AdmissionState::Pending) && !matches!')
    endpoint = '        self.transport.register_peer(node, canonical_addr);\n'
    early_endpoint = replace_once(runtime, endpoint, '')
    early_endpoint = replace_once(early_endpoint, '        let canonical = canonical_addr.to_string();\n',
                                  '        let canonical = canonical_addr.to_string();\n' + endpoint)
    cases = [
        ('allow-unregistered-receive', GRPC, replace_once(grpc,
         'if !self.discovery.raft_receive_allowed() {', 'if false && !self.discovery.raft_receive_allowed() {'),
         stale, 'stale heartbeat entered an unauthorized replica'),
        ('start-unregistered-owner', RUNTIME, replace_once(runtime,
         'let authorized = !joining || recovered_member;', 'let authorized = !joining || recovered_member || joining;'),
         stale, 'unregistered owner started'),
        ('accept-wrong-local-store', RUNTIME, replace_once(runtime, local_check, bypass_local),
         recover, 'a different store accepted the durable local membership binding'),
        ('route-before-validation', RUNTIME, early_endpoint,
         routes, 'invalid ticket installed a transport route'),
        ('rebind-with-renewed-ticket', RUNTIME, replace_once(runtime, binding, bypass_binding),
         routes, 'a renewed ticket rebound an existing replica to an empty store'),
    ]
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    manifest = dict(sources={p: sha(s) for p, s in sources.items()}, controls=[])
    original = output / 'original'
    for p, s in sources.items():
        (original / p).parent.mkdir(parents=True, exist_ok=True)
        (original / p).write_text(s)
    with tempfile.TemporaryDirectory(prefix='kv9-receive-controls.') as temp:
        tree = Path(temp)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for name, path, mutant, test, failure in cases:
            folder = output / name
            folder.mkdir()
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, path=path, test=test, expected_failure=failure, mutant_sha256=sha(mutant), runs=[])
            for phase, text in [('baseline', sources[path]), ('mutant', mutant), ('restored', sources[path])]:
                for p, s in sources.items():
                    (tree / p).write_text(text if p == path else s)
                command = ['cargo', 'test', '--locked', '-p', 'kv9-server', '--lib',
                           'runtime::tests::' + test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one compiled test')
                if phase == 'mutant':
                    if result.returncode != 101 or failure not in result.stdout or '1 failed;' not in result.stdout:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
                case['runs'].append(dict(phase=phase, command=command, exit_code=result.returncode,
                    sources={p: sha((tree / p).read_text()) for p in sources}))
            manifest['controls'].append(case)
            (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
            print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
    print('PASS: 5 isolated receive-authority source controls checked', flush=True)


if __name__ == '__main__':
    main()
