#!/usr/bin/env python3
"""Check isolated wakeup faults against actual Rust tests, retaining every phase."""
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
    ('missing-completion-publication', 'crates/raft/src/driver.rs',
     'if let Err(cause) = self.completion.publish() {',
     'if let Err(cause) = Ok::<(), Error>(()) {',
     'driver::tests::completion_notifications_require_the_exact_applied_receipt',
     'exact applied receipt did not wake a parked waiter'),
    ('completion-wakes-only-one', 'crates/raft/src/work.rs',
     '*generation = generation.and_then(|value| value.checked_add(1));\n        self.changed.notify_all();',
     '*generation = generation.and_then(|value| value.checked_add(1));\n        self.changed.notify_one();',
     'work::tests::one_completion_wakes_all_registered_waiters',
     'one or more registered completion waiters were left parked'),
    ('completion-forgets-observed-generation', 'crates/raft/src/work.rs',
     '|current| *current == Some(observed)', '|current| current.is_some()',
     'work::tests::completion_between_lookup_and_wait_is_retained_and_exhaustion_cannot_wrap',
     'publication before park was lost'),
    ('completion-generation-wraps', 'crates/raft/src/work.rs',
     'generation.and_then(|value| value.checked_add(1))',
     'generation.map(|value| value.wrapping_add(1))',
     'work::tests::completion_between_lookup_and_wait_is_retained_and_exhaustion_cannot_wrap',
     'exhausted generation wrapped'),
    ('traffic-accelerates-ticks', 'crates/raft/src/work.rs',
     'if now < self.next {', 'if now < self.next && false {',
     'work::tests::traffic_does_not_accelerate_or_postpone_tick_deadlines',
     'traffic accelerated a tick before its deadline'),
    ('traffic-postpones-ticks', 'crates/raft/src/work.rs',
     'if now < self.next {', 'self.next = now + self.period;\n        if now < self.next {',
     'work::tests::traffic_does_not_accelerate_or_postpone_tick_deadlines',
     'traffic postponed a due tick'),
    ('retained-inbox-loses-notification', 'crates/raft/src/work.rs',
     'if state.messages.is_empty() {\n                None\n            } else {\n                state.signal.clone()\n            }',
     'if state.messages.is_empty() {\n                None\n            } else {\n                None::<Arc<WorkSignal>>\n            }',
     'work::tests::inbox_bounds_and_retained_prefix_force_another_turn',
     'retained inbox work did not request another turn'),
]


def sha(source):
    return hashlib.sha256(source.encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get(
        'CARGO_TARGET_DIR', str((ROOT / 'target').resolve())))
    manifest = dict(version=1, revision=subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        runner_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        accepted=False, controls=[])

    def save():
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    save()
    with tempfile.TemporaryDirectory(prefix='kv9-wakeup-controls.') as temp:
        tree = Path(temp)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for name, relative, before, after, test, failure in CASES:
            target = tree / relative
            original = (ROOT / relative).read_text()
            if original.count(before) != 1:
                raise RuntimeError(f'{name}: source anchor is not unique')
            mutant = original.replace(before, after)
            folder = output / name
            folder.mkdir()
            (folder / 'original.rs').write_text(original)
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, source=relative, test=test,
                        source_sha256=sha(original), mutant_sha256=sha(mutant),
                        expected_failure=failure, runs=[])
            manifest['controls'].append(case)
            save()
            for phase, source in [('baseline', original), ('mutant', mutant), ('restored', original)]:
                target.write_text(source)
                command = ['cargo', 'test', '--locked', '-p', 'kv9-raft', '--lib',
                           test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                        timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                case['runs'].append(dict(phase=phase, exit_code=result.returncode,
                                         source_sha256=sha(source), command=command))
                save()
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one selected test')
                if phase == 'mutant':
                    if (result.returncode == 0 or failure not in result.stdout
                            or '1 failed;' not in result.stdout):
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
            print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
    manifest['accepted'] = True
    save()
    print(f'PASS: {len(CASES)} isolated Raft wakeup source controls checked')


if __name__ == '__main__':
    main()
