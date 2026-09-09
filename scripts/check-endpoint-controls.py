#!/usr/bin/env python3
"""Reject isolated endpoint CAS and route-writer faults in catalog/runtime paths."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
ENDPOINT = 'crates/meta/src/endpoint.rs'
SCHEMA = 'crates/meta/src/schema.rs'
TESTS = 'crates/meta/tests/endpoint.rs'
RUNTIME = 'crates/server/src/runtime.rs'
NODE = 'crates/server/src/node.rs'


def digest(text):
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
    sources = {p: (ROOT / p).read_text() for p in (ENDPOINT, SCHEMA, TESTS, RUNTIME, NODE)}
    endpoint = sources[ENDPOINT]
    runtime = sources[RUNTIME]
    cases = [
        ('ignore-cas-generation', ENDPOINT, replace_once(endpoint,
         'current.generation == request.expected_generation', 'true'),
         'endpoint_cas_rejects_aba_and_late_duplicate_with_same_target',
         'old address matched after ABA but its generation was obsolete'),
        ('ignore-confirmed-generation', ENDPOINT, replace_once(endpoint,
         'current.generation == next_generation', 'true'),
         'endpoint_cas_rejects_aba_and_late_duplicate_with_same_target',
         'matching target address acknowledged a superseded transition'),
        ('ignore-confirmed-old-address', ENDPOINT, replace_once(endpoint,
         '&& current.previous_address == Some(request.expected_address)', '&& true'),
         'endpoint_retry_confirms_one_step_without_reapplying',
         "confirmation ignored the original transition's old address"),
        ('ignore-store-binding', ENDPOINT, replace_once(endpoint,
         'if current.incarnation != request.incarnation {', 'if false && current.incarnation != request.incarnation {'),
         'endpoint_binding_is_required_for_initial_update_and_confirmation',
         'wrong incarnation changed an existing endpoint'),
        ('wrap-exhausted-generation', ENDPOINT, replace_once(endpoint,
         'request.expected_generation.checked_add(1)', 'request.expected_generation.checked_add(1).or(Some(0))'),
         'endpoint_generation_exhaustion_never_wraps',
         'endpoint generation exhausted but a new transition was staged'),
        ('ignore-active-membership', ENDPOINT, replace_once(endpoint,
         'if !current.active {', 'if false && !current.active {'),
         'endpoint_update_requires_active_membership',
         'inactive member received endpoint authorization'),
        ('retain-obsolete-admission', ENDPOINT, replace_once(endpoint,
         'if revoke {', 'if false && revoke {'),
         'endpoint_change_revokes_prior_admission_in_the_same_batch',
         'endpoint change retained obsolete registration authority'),
        ('confirmation-revokes-later-admission', ENDPOINT, replace_once(endpoint,
         'return Ok(Confirmed(current));',
         'crate::admission::revoke_admission(txn, request.node)?;\n        return Ok(Confirmed(current));'),
         'endpoint_confirmation_does_not_revoke_a_later_admission',
         'confirmation or refusal staged catalog mutations'),
        ('registration-reuses-generation', ENDPOINT, replace_once(endpoint,
         '.generation\n        .checked_add(1)', '.generation\n        .checked_add(0)'),
         'registration_address_changes_participate_in_endpoint_aba_fencing',
         'registration did not advance the endpoint generation'),
        ('registration-skips-versioned-writer', RUNTIME, replace_once(runtime,
         '''                    kv9_meta::endpoint::refresh_registration_endpoint(
                        &mut txn,
                        node,
                        store_incarnation,
                        canonical_addr,
                    )
                    .map_err(RegistrationError::Failed)?;''',
         '                    // Skip the versioned endpoint planner.'),
         'runtime::tests::registration_refusals_preserve_routes_and_renewed_tickets_cannot_rebind',
         'renewed registration bypassed endpoint versioning'),
        ('unlocked-catalog-route-snapshot', RUNTIME, replace_once(runtime,
         '''        let _guard = self.node.meta_raft.lock_catalog_txn();
        let txn = self.node.meta_raft.store.begin()?;
        let rows = txn.scan(&NODES_DESC, MAX_PHASE1_NODES + 1)?;''',
         '''        let txn = self.node.meta_raft.store.begin()?;
        let rows = txn.scan(&NODES_DESC, MAX_PHASE1_NODES + 1)?;'''),
         'runtime::tests::delayed_catalog_snapshot_cannot_restore_an_older_live_route',
         'stale catalog snapshot restored the old live endpoint'),
        ('registration-response-overrides-catalog', RUNTIME, replace_once(runtime,
         'applied_endpoint.map_or(via.1, |endpoint| endpoint.address)',
         '{ let _ = applied_endpoint; via.1 }'),
         'runtime::tests::registration_response_fallback_defers_to_the_applied_directory',
         'registration response overwrote an applied endpoint with its dial address'),
        ('consumed-retry-ignores-current-endpoint', RUNTIME, replace_once(runtime,
         'if endpoint.address != canonical_addr {',
         'if false && endpoint.address != canonical_addr {'),
         'runtime::tests::committed_endpoint_change_revokes_registration_and_rejects_its_retry',
         'obsolete consumed admission bypassed current endpoint validation'),
    ]
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    manifest = dict(sources={p: digest(s) for p, s in sources.items()}, controls=[])
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    for p, s in sources.items():
        target = output / 'original' / p
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(s)
    with tempfile.TemporaryDirectory(prefix='kv9-endpoint-controls.') as directory:
        tree = Path(directory)
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
            case = dict(name=name, path=path, test=test, expected_failure=failure,
                        mutant_sha256=digest(mutant), runs=[])
            for phase, text in [('baseline', sources[path]), ('mutant', mutant), ('restored', sources[path])]:
                for p, s in sources.items():
                    (tree / p).write_text(text if p == path else s)
                expected_sources = {p: digest(text if p == path else s) for p, s in sources.items()}
                target = ['-p', 'kv9-server', '--lib'] if test.startswith('runtime::') else ['-p', 'kv9-meta', '--test', 'endpoint']
                command = ['cargo', 'test', '--locked', *target, test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one compiled test')
                if phase == 'mutant':
                    if result.returncode != 101 or failure not in result.stdout or '0 passed; 1 failed;' not in result.stdout:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
                actual_sources = {p: digest((tree / p).read_text()) for p in sources}
                if actual_sources != expected_sources:
                    raise RuntimeError('isolated source changed during a control')
                case['runs'].append(dict(phase=phase, command=command, exit_code=result.returncode, sources=actual_sources))
            manifest['controls'].append(case)
            (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
            print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
    if any((ROOT / p).read_text() != text for p, text in sources.items()):
        raise RuntimeError('source changed during controls')
    print('PASS: 13 isolated endpoint CAS and writer source controls checked', flush=True)


if __name__ == '__main__':
    main()
