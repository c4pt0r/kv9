#!/usr/bin/env python3
"""Run isolated latency source controls with baseline/mutant/restored evidence."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CASES = [
    ('queue-in-backend-duration', 'crates/server/src/admission.rs',
     'ended.saturating_duration_since(started)', 'ended.saturating_duration_since(self.reserved_at)',
     'kv9-server', 'admission::tests::admission_timing_separates_queue_execution_and_unsubmitted_release',
     'backend timing included preparation or queue time'),
    ('replaced-reported-successful', 'crates/raft/src/driver.rs',
     'Ok(ApplyWaitOutcome::Replaced) => Outcome::Replaced,',
     'Ok(ApplyWaitOutcome::Replaced) => Outcome::Success,',
     'kv9-raft', 'driver::tests::a_position_taken_by_another_leaders_command_is_replaced_not_applied',
     'real protocol outcome missing from observation'),
    ('missing-real-fsync-observation', 'crates/raft/src/storage.rs',
     'metrics.sync.measure(|| file.sync_data())', 'file.sync_data()',
     'kv9-raft', 'storage::persistence_model::wal_observation_preserves_write_sync_short_circuit_and_writer_poison',
     'sync observation must match actual fsync attempts'),
    ('unfinished-pump-reported-complete', 'crates/raft/src/driver.rs',
     '|| self.step_inner(),',
     '|| { self.metrics.pump_service.record(Duration::ZERO, Outcome::Success); self.step_inner() },',
     'kv9-raft', 'driver::tests::pump_service_finishes_only_after_the_actual_transport_drain_returns',
     'unfinished pump was reported as completed'),
    ('failed-pump-reported-successful', 'crates/raft/src/driver.rs',
     '|result| {\n                if result.is_ok() {',
     '|result| {\n                if result.is_ok() || result.is_err() {',
     'kv9-raft', 'driver::tests::failed_background_pump_records_error_without_another_idle_wait',
     'failed pump was reported as successful'),
    ('idle-observation-after-sleep', 'crates/raft/src/driver.rs',
     'let idle = driver.metrics.pump_idle_wait.start();\n                std::thread::sleep(tick_every);',
     'std::thread::sleep(tick_every);\n                let idle = driver.metrics.pump_idle_wait.start();',
     'kv9-raft', 'driver::tests::background_pump_records_returned_sleeps_and_no_first_spacing_sample',
     'idle observation did not include the actual configured sleep'),
]


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
    with tempfile.TemporaryDirectory(prefix='kv9-latency-controls.') as temp:
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
    print('PASS: 6 isolated latency source controls checked')


if __name__ == '__main__':
    main()
