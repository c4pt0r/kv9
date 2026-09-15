#!/usr/bin/env python3
"""Check two retained local status snapshots; report only bounded trace fields.

This is a retained-window analysis, not a complete history or a causal proof.
No database, process inspection, workload, codec or network operation is run.
"""
import argparse
import hashlib
import json
import re
import stat
from pathlib import Path

U64 = (1 << 64) - 1
CAPACITY = 512
STRIDE = 16
MAX_STATUS_BYTES = 2 * 1024 * 1024
MAX_TRACE_BYTES = 384 * 1024
IDENTITY = ('pid', 'node_id', 'root_digest', 'store_incarnation',
            'process_boot_id', 'process_start_ticks')
TRACE_KEYS = {
    'schema_version', 'trace_instance', 'clock', 'scope', 'snapshot_consistency',
    'capture_started_ns', 'capture_finished_ns', 'sample_stride', 'capacity_per_ring',
    'rows_available', 'valid', 'dropped_recording_calls', 'groups_seen',
    'group_commands_seen', 'terminal_inspections_seen', 'groups', 'inspections',
}
GROUP_KEYS = {
    'sequence', 'group_sequence', 'term', 'index', 'commands', 'encoded_bytes',
    'prepare_started_ns', 'locks_acquired_ns', 'apply_started_ns',
    'apply_finished_ns', 'receipts_inserted_ns',
}
INSPECTION_KEYS = {
    'sequence', 'term', 'index', 'inspect_started_ns', 'inspect_finished_ns',
    'registration_age_ns', 'outcome',
}
OUTCOMES = {'applied', 'manifest', 'fence_rejected', 'replaced', 'failed', 'unconfirmed'}
RECEIPT_OUTCOMES = {'applied', 'manifest', 'fence_rejected'}


class Refusal(ValueError):
    """A fixed diagnostic string; never includes unapproved status values."""


def need(condition, reason):
    if not condition:
        raise Refusal(reason)


def uint(value, reason, positive=False):
    need(type(value) is int and (1 if positive else 0) <= value <= U64, reason)
    return value


def keys(value, expected, reason):
    need(type(value) is dict and set(value) == expected, reason)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        need(key not in result, 'duplicate JSON key')
        result[key] = value
    return result


def forbidden_constant(_value):
    raise Refusal('nonfinite JSON constant')


def parse_status(raw):
    need(type(raw) is bytes and 0 < len(raw) <= MAX_STATUS_BYTES, 'status byte bound')
    try:
        text = raw.decode('utf-8')
    except UnicodeDecodeError:
        raise Refusal('status UTF-8') from None
    selected = {}
    for line in text.splitlines():
        key, separator, value = line.partition('=')
        if key not in IDENTITY and key != 'write_stage_trace':
            continue
        need(separator and key not in selected, 'duplicate or malformed required status field')
        selected[key] = value
    need(set(selected) == set(IDENTITY) | {'write_stage_trace'}, 'missing required status field')
    identity = {key: selected[key] for key in IDENTITY}
    for key in ('pid', 'node_id', 'process_start_ticks'):
        need(re.fullmatch(r'[1-9][0-9]{0,19}', identity[key]) is not None, 'unsigned process identity')
        uint(int(identity[key]), 'process identity range', positive=True)
    need(re.fullmatch(r'[0-9a-f]{64}', identity['root_digest']) is not None, 'root digest format')
    need(re.fullmatch(r'[0-9a-f]{32}', identity['store_incarnation']) is not None, 'store incarnation format')
    need(re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}', identity['process_boot_id']) is not None,
         'boot identity format')
    encoded = selected['write_stage_trace']
    need(len(encoded.encode('utf-8')) <= MAX_TRACE_BYTES, 'trace byte bound')
    try:
        trace = json.loads(encoded, object_pairs_hook=unique_object, parse_constant=forbidden_constant)
    except Refusal:
        raise
    except (ValueError, RecursionError):
        raise Refusal('trace JSON syntax, numeric conversion or nesting') from None
    validate_trace(trace)
    identity['trace_instance'] = trace['trace_instance']
    return identity, trace


def validate_ring(ring, kind, trace):
    keys(ring, {'total_recorded', 'overwritten', 'rows'}, 'ring inventory')
    total = uint(ring['total_recorded'], 'ring total')
    overwritten = uint(ring['overwritten'], 'ring overwritten')
    rows = ring['rows']
    need(type(rows) is list and len(rows) == min(total, CAPACITY), 'retained ring row count')
    need(overwritten == max(0, total - CAPACITY), 'overwrite accounting')
    previous_group = 0
    group_shapes = {}
    group_samples = {}
    for expected_sequence, row in enumerate(rows, overwritten + 1):
        keys(row, GROUP_KEYS if kind == 'groups' else INSPECTION_KEYS, 'row inventory')
        for key, value in row.items():
            if key == 'outcome' or (key == 'receipts_inserted_ns' and value is None):
                continue
            uint(value, 'row unsigned integer', positive=key in {'sequence', 'group_sequence', 'term', 'index', 'commands', 'encoded_bytes'})
        need(row['sequence'] == expected_sequence, 'row sequence accounting')
        need(row['index'] % STRIDE == 0, 'actual-index sample rule')
        if kind == 'groups':
            need(1 <= row['commands'] <= 128, 'group command bound')
            group = row['group_sequence']
            need(previous_group <= group <= trace['groups_seen'], 'group sequence order')
            previous_group = group
            times = [row[k] for k in ('prepare_started_ns', 'locks_acquired_ns', 'apply_started_ns', 'apply_finished_ns')]
            if row['receipts_inserted_ns'] is not None:
                times.append(row['receipts_inserted_ns'])
            shape = {k: row[k] for k in GROUP_KEYS - {'sequence', 'term', 'index'}}
            need(group not in group_shapes or group_shapes[group] == shape, 'same-group metadata differs')
            group_shapes[group] = shape
            group_samples[group] = group_samples.get(group, 0) + 1
            need(group_samples[group] <= row['commands'], 'sampled members exceed group commands')
        else:
            need(type(row['outcome']) is str and row['outcome'] in OUTCOMES, 'inspection outcome')
            times = [row['inspect_started_ns'], row['inspect_finished_ns']]
            need(row['registration_age_ns'] <= row['inspect_started_ns'], 'registration age exceeds trace lifetime')
        need(times == sorted(times) and times[-1] <= trace['capture_finished_ns'], 'row timestamp order or capture bound')


def validate_trace(trace):
    keys(trace, TRACE_KEYS, 'trace inventory')
    for key in TRACE_KEYS - {'clock', 'scope', 'snapshot_consistency', 'rows_available', 'valid', 'groups', 'inspections'}:
        uint(trace[key], 'trace unsigned integer', positive=key == 'trace_instance')
    need(trace['schema_version'] == 1 and trace['sample_stride'] == STRIDE and trace['capacity_per_ring'] == CAPACITY,
         'schema, sample or capacity mismatch')
    need(trace['clock'] == 'nanoseconds_since_this_driver_trace_creation', 'trace clock')
    need(trace['scope'] == 'sampled_actual_group_members_and_terminal_async_receipt_inspections', 'trace scope')
    need(trace['snapshot_consistency'] == 'coherent_rows_independent_loss_and_clock_observations', 'snapshot consistency')
    need(type(trace['valid']) is bool and type(trace['rows_available']) is bool, 'trace boolean types')
    need(trace['valid'] and trace['rows_available'], 'invalid or busy snapshot')
    need(trace['dropped_recording_calls'] == 0, 'lost recording calls')
    need(trace['capture_started_ns'] <= trace['capture_finished_ns'], 'capture clock order')
    validate_ring(trace['groups'], 'groups', trace)
    validate_ring(trace['inspections'], 'inspections', trace)
    need(trace['groups_seen'] <= trace['group_commands_seen'] <= 128 * trace['groups_seen'], 'group counter conservation')
    need(trace['groups']['total_recorded'] <= trace['group_commands_seen'], 'sampled group member counter')
    need(trace['inspections']['total_recorded'] <= trace['terminal_inspections_seen'], 'sampled inspection counter')


def ring_union(before, after):
    old = {r['sequence']: r for r in before['rows']}
    new = {r['sequence']: r for r in after['rows']}
    for sequence in old.keys() & new.keys():
        need(old[sequence] == new[sequence], 'overlap row changed')
    rows = {**old, **new}
    return [rows[k] for k in sorted(rows)]


def analyze_status(before_raw, after_raw):
    identity, before = parse_status(before_raw)
    after_identity, after = parse_status(after_raw)
    need(identity == after_identity, 'process, store or trace instance replaced')
    need(before['capture_finished_ns'] <= after['capture_started_ns'], 'snapshots not temporally ordered')
    counters = ('groups_seen', 'group_commands_seen', 'terminal_inspections_seen', 'dropped_recording_calls')
    delta = {}
    for key in counters:
        need(after[key] >= before[key], 'counter reset')
        delta[key] = after[key] - before[key]
    need(delta['groups_seen'] <= delta['group_commands_seen'] <= 128 * delta['groups_seen'], 'delta group conservation')
    merged, coverage = {}, {}
    for kind in ('groups', 'inspections'):
        a,b = before[kind], after[kind]
        need(b['total_recorded'] >= a['total_recorded'] and b['overwritten'] >= a['overwritten'], 'ring counter reset')
        merged[kind] = ring_union(a,b)
        new_count = b['total_recorded'] - a['total_recorded']
        retained_new = sum(r['sequence'] > a['total_recorded'] for r in b['rows'])
        coverage[kind] = dict(before_total=a['total_recorded'], after_total=b['total_recorded'],
            before_overwritten=a['overwritten'], after_overwritten=b['overwritten'],
            new_recorded_rows=new_count, retained_new_rows=retained_new,
            missing_new_rows=new_count-retained_new, retained_union_rows=len(merged[kind]),
            historical_rows_unavailable_before_first_snapshot=a['overwritten'])
    need(coverage['groups']['new_recorded_rows'] <= delta['group_commands_seen'], 'delta sampled members')
    need(coverage['inspections']['new_recorded_rows'] <= delta['terminal_inspections_seen'], 'delta sampled inspections')
    # A group may straddle the first retained member in a wrapped ring.
    shapes = {}
    for row in merged['groups']:
        shape = {k:row[k] for k in GROUP_KEYS-{'sequence','term','index'}}
        gs = row['group_sequence']
        need(gs not in shapes or shapes[gs] == shape, 'cross-snapshot group metadata differs')
        shapes[gs] = shape
    grouped, inspected = {}, {}
    for row in merged['groups']:
        grouped.setdefault((row['term'], row['index']), []).append(row)
    for row in merged['inspections']:
        inspected.setdefault((row['term'], row['index']), []).append(row)
    joins = []
    for term,index in sorted(grouped.keys() | inspected.keys()):
        gs, ins = grouped.get((term,index),[]), inspected.get((term,index),[])
        row = dict(term=term,index=index,group_sequences=[g['sequence'] for g in gs],
            inspection_sequences=[i['sequence'] for i in ins])
        if len(gs)>1 or len(ins)>1:
            row['status']='ambiguous_exact_identity'
        elif not gs:
            row['status']='unmatched_inspection'
        elif not ins:
            row['status']='unmatched_group'
        elif gs[0]['receipts_inserted_ns'] is None or ins[0]['outcome'] not in RECEIPT_OUTCOMES:
            row['status']='unique_non_successful_receipt_pair'
        elif gs[0]['receipts_inserted_ns'] > ins[0]['inspect_started_ns']:
            row['status']='unique_time_incompatible_pair'
        else:
            g,i=gs[0],ins[0]
            row.update(status='unique_compatible_pair', group_sequence=g['group_sequence'],
                inspection_recorded_after_before=i['sequence']>before['inspections']['total_recorded'],
                intervals_ns=dict(prepare_to_locks=g['locks_acquired_ns']-g['prepare_started_ns'],
                    locks_to_apply=g['apply_started_ns']-g['locks_acquired_ns'],
                    apply=g['apply_finished_ns']-g['apply_started_ns'],
                    apply_return_to_receipt_insertion=g['receipts_inserted_ns']-g['apply_finished_ns'],
                    receipt_insertion_to_inspection=i['inspect_started_ns']-g['receipts_inserted_ns'],
                    inspection=i['inspect_finished_ns']-i['inspect_started_ns']),
                registration_age_ns=i['registration_age_ns'],outcome=i['outcome'])
        joins.append(row)
    return dict(complete=True,accepted_retained_window_analysis=True,complete_history=False,causality_proven=False,
        schema_version=1,identity=identity,capture_before=dict(started_ns=before['capture_started_ns'],finished_ns=before['capture_finished_ns']),
        capture_after=dict(started_ns=after['capture_started_ns'],finished_ns=after['capture_finished_ns']),
        counter_delta=delta,coverage=coverage,retained_groups=merged['groups'],retained_inspections=merged['inspections'],
        joins=joins,join_counts={status:sum(r['status']==status for r in joins) for status in sorted({r['status'] for r in joins})},
        limitations=[
            'Only the union of two retained sampled windows is represented; overwritten or unmatched rows are not reconstructed.',
            'Index modulo 16 sampling is deterministic, not a random sample or an unbiased group distribution.',
            'Several sampled members can share group_sequence; deduplicate groups for group-level statistics.',
            'Receipt insertion occurs under locks, before notification and possible visibility to a waiter.',
            'Only terminal async receipt-inspection callbacks are traced; timeout/cancel/stop and synchronous wait terminals are outside this hook.',
            'Registration age is sampled before the inspection timestamp; it is not an exact registration timestamp.',
            'Only unique compatible pairs have intervals; exact identity and temporal order alone are not proof of per-call causality.',
            'No proposal/Ready, full network/RPC/client interval or client p99 membership is established.'])


def read_file(path):
    before = path.lstat()
    need(stat.S_ISREG(before.st_mode) and 0 < before.st_size <= MAX_STATUS_BYTES, 'regular bounded input required')
    raw = path.read_bytes()
    after = path.lstat()
    need((before.st_dev,before.st_ino,before.st_size,before.st_mtime_ns,before.st_ctime_ns) ==
         (after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns), 'input changed while reading')
    need(len(raw) == before.st_size, 'input length changed')
    return raw


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before',type=Path,required=True)
    parser.add_argument('--after',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    need(not args.output.exists(), 'output already exists')
    inputs={}
    try:
        raw=[]
        for name,path in (('before',args.before),('after',args.after)):
            data=read_file(path)
            inputs[name]=dict(path=str(path.absolute()),bytes=len(data),sha256=hashlib.sha256(data).hexdigest())
            raw.append(data)
        result=analyze_status(*raw)
    except (Refusal,OSError) as error:
        result=dict(complete=False,accepted_retained_window_analysis=False,
            error=str(error) if isinstance(error,Refusal) else type(error).__name__)
    result['inputs']=inputs
    source=Path(__file__).read_bytes()
    result['reader']=dict(bytes=len(source),sha256=hashlib.sha256(source).hexdigest())
    with args.output.open('x') as handle:
        handle.write(json.dumps(result,indent=2,sort_keys=True)+'\n')
    print(json.dumps(dict(complete=result['complete'],output=str(args.output),
                         join_counts=result.get('join_counts'),error=result.get('error'))))
    return 0 if result['complete'] else 1


if __name__=='__main__':
    raise SystemExit(main())
