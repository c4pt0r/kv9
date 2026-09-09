#!/usr/bin/env python3
"""Run isolated membership source controls with baseline/mutant/restored evidence."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CASES = [('lost-admit-wire-refusal',
  'crates/server/src/grpc.rs',
  '.map_err(|status| membership_rpc_error("AdmitNode", status))',
  '.map_err(|status| Error::Raft(format!("AdmitNode RPC: {status}")))',
  'kv9-server',
  'grpc::tests::blocking_membership_clients_preserve_real_wire_refusals',
  'membership wire refusal lost its typed leader hint'),
 ('ambiguous-membership-retry',
  'crates/server/src/grpc.rs',
  '_ => Error::Raft(format!("{rpc} RPC: {status}")),',
  '_ => { let _ = rpc; Error::NotLeader { leader: None } },',
  'kv9-server',
  'grpc::tests::membership_refusal_rejects_ambiguous_wire_metadata',
  'ambiguous membership outcome was converted to a retryable refusal'),
 ('untyped-admit-follower',
  'crates/server/src/runtime.rs',
  'return Err(Error::NotLeader {\n'
  '                leader: status.leader_id,\n'
  '            });\n'
  '        }\n'
  '        if ttl_seconds == 0 {',
  'return Err(Error::Raft("membership must be sent to the leader".into()));\n'
  '        }\n'
  '        if ttl_seconds == 0 {',
  'kv9-server',
  'runtime::tests::a_follower_refuses_an_establishing_read_with_a_typed_hint',
  'membership follower refusal must be typed before any mutation'),
 ('untyped-promote-follower',
  'crates/server/src/runtime.rs',
  'return Err(Error::NotLeader {\n'
  '                leader: status.leader_id,\n'
  '            });\n'
  '        }\n'
  '        let proposed = self.driver.promote_voter(node)?;',
  'return Err(Error::Raft("membership must be sent to the leader".into()));\n'
  '        }\n'
  '        let proposed = self.driver.promote_voter(node)?;',
  'kv9-server',
  'runtime::tests::a_follower_refuses_an_establishing_read_with_a_typed_hint',
  'membership follower refusal must be typed before any mutation')]


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str((ROOT / 'target').resolve())))
    manifest = dict(revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), controls=[])
    with tempfile.TemporaryDirectory(prefix='kv9-membership-controls.') as temp:
        tree = Path(temp)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for name, relative, before, after, package, test, failure in CASES:
            target = tree / relative
            original = (ROOT / relative).read_text()
            if original.count(before) != 1:
                raise RuntimeError(f'{name}: source anchor is not unique')
            mutant = original.replace(before, after)
            folder = output / name
            folder.mkdir()
            (folder / 'original.rs').write_text(original)
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, source=relative, source_sha256=sha(original), test=test, expected_failure=failure, mutant_sha256=sha(mutant), runs=[])
            for phase, source in [('baseline', original), ('mutant', mutant), ('restored', original)]:
                target.write_text(source)
                command = ['cargo', 'test', '--locked', '-p', package, '--lib',
                           test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one selected test')
                if phase == 'mutant':
                    if result.returncode == 0 or failure not in result.stdout or '1 failed;' not in result.stdout:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
                case['runs'].append(dict(phase=phase, exit_code=result.returncode, source_sha256=sha(source)))
            manifest['controls'].append(case)
            print(f'PASS: {name} baseline, intended failure and restored source')
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print('PASS: 4 isolated membership source controls checked')


if __name__ == '__main__':
    main()
