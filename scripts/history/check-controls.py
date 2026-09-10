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
     '(op.outcome == "unknown" or op.response is None or op.response >= event["seq"])',
     '(op.response is None or op.response >= event["seq"])',
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
    ('confirmed-range-snapshot-deferred', 'checker.py',
     'prepare_confirmed_range = (guided_unknown and range_preference and op.outcome == "ok"',
     'prepare_confirmed_range = (False and op.outcome == "ok"',
     'test_confirmed_range_snapshot_precedes_overlapping_new_key'),
    ('confirmed-range-observation-contradicted', 'checker.py',
     'range_preference = observed_value == pending.args["value"]',
     'range_preference = True',
     'test_confirmed_range_can_also_capture_the_overlapping_insert'),
    ('confirmed-range-read-hint-ignored', 'checker.py',
     'if hint:', 'if False:',
     'test_range_snapshot_can_fall_between_two_overlapping_insertions'),
    ('scan-absence-hint-ignored', 'checker.py',
     'if covers_key:', 'if False:',
     'test_range_hint_observes_absence_inside_a_scan'),
    ('scan-prefix-limit-ignored', 'checker.py',
     'len(rows) < args["limit"] or (rows and key < rows[-1][0])', 'True',
     'test_range_hint_respects_scan_bounds_and_limit'),
    ('scan-interval-bounds-ignored', 'checker.py',
     'in_range(key, args) and args["limit"] > 0', 'args["limit"] > 0',
     'test_range_hint_respects_scan_bounds_and_limit'),
    ('unknown-write-cannot-explain-overlapping-read', 'checker.py',
     'for target in targets', 'for target in [wanted]',
     'test_unknown_write_can_explain_an_overlapping_read_before_mutation'),
    ('following-read-write-order-ignored', 'checker.py',
     'matches_read = competing_write and observed', 'matches_read = False and observed',
     'test_following_read_orders_either_concurrent_write_first'),
    ('reversed-write-responses-expand-unknowns-first', 'checker.py',
     'competing_write = (guided_unknown and op.outcome == "ok"',
     'competing_write = (False and op.outcome == "ok"',
     'test_reversed_write_responses_do_not_expand_old_unknown_deletions_first'),
    ('early-range-selection-omitted', 'checker.py',
     "allow_preparation = (prepare_ranges and", "allow_preparation = (False and",
     'test_guided_search_prepares_unknown_range_before_new_key'),
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

# The batch suite keeps raw-record counterexamples and a separate permutation
# oracle. Each mutation below changes one semantic rule in an isolated copy.
BATCH_CONTROLS = [(label, 'checker.py', old, new, test) for label, old, new, test in [
    ('batch-read-observation-ignored',
     'if [kv.get((kid, key)) for key in args["keys"]] != op.result["values"]:',
     'if False:', 'test_atomic_batch_read_cannot_observe_a_fractured_batch_write'),
    ('batch-write-prefix-only', 'for key, value in args["pairs"]:',
     'for key, value in args["pairs"][:1]:', 'test_confirmed_batch_write_has_no_partial_effect'),
    ('batch-write-order-reversed', 'for key, value in args["pairs"]:',
     'for key, value in reversed(args["pairs"]):',
     'test_duplicate_batch_put_pairs_apply_in_order_with_last_pair_winning'),
    ('batch-read-order-sorted',
     'if [kv.get((kid, key)) for key in args["keys"]] != op.result["values"]:',
     'if [kv.get((kid, key)) for key in sorted(args["keys"])] != op.result["values"]:',
     'test_batch_read_retains_input_order_and_duplicate_positions'),
    ('batch-missing-becomes-empty',
     'if [kv.get((kid, key)) for key in args["keys"]] != op.result["values"]:',
     'if [kv.get((kid, key), "") for key in args["keys"]] != op.result["values"]:',
     'test_missing_keys_and_present_empty_values_are_distinct'),
    ('batch-read-cardinality-ignored',
     'require(len(event["result"]["values"]) == len(calls[oid]["args"]["keys"]),',
     'require(True,', 'test_batch_result_shapes_cardinality_and_values_are_strict'),
    ('batch-timeout-becomes-effect-upper-bound',
     '(op.outcome == "unknown" or op.response is None or op.response >= event["seq"])',
     '(op.response is None or op.response >= event["seq"])',
     'test_unknown_batch_write_may_settle_after_its_unknown_return'),
]]


def demand(condition, message):
    if not condition:
        raise RuntimeError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--batch', action='store_true', help='run atomic batch semantic mutations')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    source = Path(__file__).resolve().parent
    suite = 'test_batch_checker' if args.batch else 'test_checker'
    test_class = 'BatchCheckerControls' if args.batch else 'CheckerControls'
    test_count = 21 if args.batch else 44
    controls = BATCH_CONTROLS if args.batch else CONTROLS
    names = ['checker.py', suite + '.py'] + ([] if args.batch else ['workload.py'])
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
            selected = suite + '.' + test_class + '.' + test if test else suite
            process = subprocess.run([sys.executable, '-B', '-m', 'unittest', selected, '-v'], cwd=work,
                                     env={**os.environ, 'PYTHONDONTWRITEBYTECODE': '1'}, capture_output=True, text=True, timeout=30)
            output = process.stdout+process.stderr
            (args.output/(label+'.log')).write_text(output)
            count = re.findall(r'^Ran (\d+) tests? in ', output, re.M)
            demand(count == [str(1 if test else test_count)], f'{label}: wrong selected test count: {count}')
            if red:
                attributable_failures = (re.search(r'FAILED \(failures=[1-9][0-9]*\)', output)
                                         if args.batch else 'FAILED (failures=1)' in output)
                demand(process.returncode == 1 and attributable_failures and 'AssertionError:' in output
                       and f'FAIL: {test} ' in output and 'ERROR:' not in output, f'{label}: no attributable assertion failure')
            else:
                demand(process.returncode == 0 and output.rstrip().endswith('\nOK'), f'{label}: baseline/restored tests failed')
        run('baseline-suite')
        unchanged(originals)
        for label, filename, old, new, test in controls:
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
            print(f'PASS: {label}: baseline 1 test green, mutant rejected by assertions, restored 1 test green', flush=True)
        run('restored-suite')
        unchanged(originals)
    demand(all((source/name).read_bytes() == data for name, data in originals.items()), 'source changed during control run')
    print(f'PASS: {test_count} history tests and {len(controls)} isolated source mutations checked', flush=True)


if __name__ == '__main__':
    main()
