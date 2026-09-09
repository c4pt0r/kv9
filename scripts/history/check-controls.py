#!/usr/bin/env python3
"""Run isolated, single-defect history-checker mutations with exact test counts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


# Each tuple is one semantic defect, one exact source replacement, and the
# specific independently known history/guard that must fail an assertion.
CONTROLS = [
    ('timeout-is-a-write-upper-bound',
     '(op.outcome != "unknown" and op.response is not None and op.response < event["seq"])',
     '(op.response is not None and op.response < event["seq"])',
     'test_unknown_write_may_commit_after_timeout'),
    ('ignore-observed-get-value',
     'if kv.get((kid, args["key"])) != op.result["value"]:',
     'if False:', 'test_lost_acknowledged_write'),
    ('permit-effect-before-invocation',
     'require(op.invocation < history.events[boundary]["seq"], "witness effect before invocation")',
     'require(True, "witness effect before invocation")',
     'test_witness_replay_refuses_fabricated_or_missing_steps'),
    ('reuse-allocated-keyspace-id',
     'if kid not in catalog.values():\n                yield freeze',
     'if True:\n                yield freeze', 'test_duplicate_name_and_id_are_rejected'),
    ('delete-keys-outside-captured-chunk',
     'deleted = keys[count * chunk:(count + 1) * chunk]',
     'deleted = tuple(key for space, key in sorted(kv) if space == kid and in_range(key, args))',
     'test_range_uses_one_snapshot_across_chunks'),
    ('treat-restricted-failure-as-invalid',
     'if result["verdict"] == "valid" or (limit is None and not guided):',
     'if result["verdict"] in {"valid", "invalid"} or (limit is None and not guided):',
     'test_unknown_write_may_commit_after_timeout'),
    ('skip-fault-phase-coverage',
     'for phase in args.require_phase:', 'for phase in []:',
     'test_cli_requires_successful_put_and_read_in_each_fault_phase'),
]

CONTROLS = [(label, 'checker.py', old, new, test) for label, old, new, test in CONTROLS] + [
    ('matching-overlapping-read-deferred', 'checker.py',
     'prefer_observed = (guided_unknown and op.outcome == "ok"', 'prefer_observed = (False and op.outcome == "ok"',
     'test_guided_search_orders_matching_overlapping_read_before_write'),
    ('oldest-unknown-first-frontier-explosion', 'checker.py',
     '-i if op.outcome == "unknown" else i', 'i',
     'test_recent_unknown_write_avoids_old_range_frontier_explosion'),
    ('refused-read-treated-as-observed', 'checker.py',
     'if op.outcome != "ok":', 'if False:',
     'test_guided_search_handles_refused_reads_without_values'),
    ('timeout-masquerades-as-admission-refusal', 'workload.py',
     'if rc != 1 or stdout:', 'if False:',
     'test_admission_refusal_requires_exclusive_cli_evidence'),
]


def demand(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parent
    names = ['checker.py', 'workload.py', 'test_checker.py']
    originals = {name: (source/name).read_bytes() for name in names}
    manifest = {'source_sha256': {name: hashlib.sha256(data).hexdigest() for name, data in originals.items()}, 'controls': []}
    (args.output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    with tempfile.TemporaryDirectory(prefix='kv9-history-controls-') as directory:
        work = Path(directory)
        for name, data in originals.items(): (work/name).write_bytes(data)
        def unchanged(expected):
            actual = {p.name: p.read_bytes() for p in work.iterdir() if p.is_file()}
            demand(actual == expected and all(p.is_file() for p in work.iterdir()), 'undeclared files changed in isolated control')
        def run(label, test=None, red=False):
            selected = 'test_checker.CheckerControls.'+test if test else 'test_checker'
            process = subprocess.run([sys.executable, '-B', '-m', 'unittest', selected, '-v'], cwd=work,
                                     env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'}, capture_output=True, text=True, timeout=30)
            output = process.stdout+process.stderr
            (args.output/(label+'.log')).write_text(output)
            count = re.findall(r'^Ran (\d+) tests? in ', output, re.M)
            demand(count == [str(1 if test else 34)], f'{label}: wrong selected test count: {count}')
            if red:
                demand(process.returncode == 1 and 'FAILED (failures=1)' in output and 'AssertionError:' in output
                       and f'FAIL: {test} ' in output and 'ERROR:' not in output, f'{label}: no attributable assertion failure')
            else:
                demand(process.returncode == 0 and output.rstrip().endswith('\nOK'), f'{label}: baseline/restored tests failed')
        run('baseline-suite')
        unchanged(originals)
        for label, filename, old, new, test in CONTROLS:
            target = work/filename
            run(label+'-baseline', test)
            text = originals[filename].decode()
            demand(text.count(old) == 1, f'{label}: target literal is not unique')
            mutated = text.replace(old, new).encode()
            demand(mutated != originals[filename] and old not in mutated.decode(), f'{label}: mutation did not land')
            try:
                target.write_bytes(mutated)
                unchanged({**originals, filename: mutated})
                run(label+'-mutant', test, red=True)
                unchanged({**originals, filename: mutated})
            finally:
                target.write_bytes(originals[filename])
            run(label+'-restored', test)
            unchanged(originals)
            manifest['controls'].append({'name': label, 'source': filename, 'test': test, 'selected': 1, 'verdict': 'rejected',
                                         'mutant_sha256': hashlib.sha256(mutated).hexdigest()})
            (args.output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
            print(f'PASS: {label}: baseline 1 green, mutant 1 assertion failure, restored 1 green', flush=True)
        run('restored-suite')
        unchanged(originals)
    demand(all((source/name).read_bytes() == data for name, data in originals.items()), 'source changed during control run')
    print(f'PASS: 34 history tests and {len(CONTROLS)} isolated source mutations checked', flush=True)


if __name__ == '__main__':
    main()
