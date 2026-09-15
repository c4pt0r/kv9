#!/usr/bin/env python3
"""Synthetic behavioral controls for the independent write-stage reader."""
import argparse
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

READER = Path(__file__).with_name('check-write-stage-trace.py')
spec = importlib.util.spec_from_file_location('write_stage_reader', READER)
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)


def group(sequence, index=None, term=1, group_sequence=None):
    start = sequence * 10
    return dict(sequence=sequence, group_sequence=group_sequence or sequence,
        term=term, index=index or sequence*16, commands=1, encoded_bytes=32,
        prepare_started_ns=start, locks_acquired_ns=start+1,
        apply_started_ns=start+2, apply_finished_ns=start+3, receipts_inserted_ns=start+4)


def inspection(sequence, index=None, term=1, outcome='applied'):
    start = sequence*10+5
    return dict(sequence=sequence,term=term,index=index or sequence*16,
        inspect_started_ns=start,inspect_finished_ns=start+1,
        registration_age_ns=2,outcome=outcome)


def snapshot(total=0):
    first = max(1,total-511)
    return dict(schema_version=1,trace_instance=1,
        clock='nanoseconds_since_this_driver_trace_creation',
        scope='sampled_actual_group_members_and_terminal_async_receipt_inspections',
        snapshot_consistency='coherent_rows_independent_loss_and_clock_observations',
        capture_started_ns=total*10+7,capture_finished_ns=total*10+8,
        sample_stride=16,capacity_per_ring=512,rows_available=True,valid=True,
        dropped_recording_calls=0,groups_seen=total,group_commands_seen=total,
        terminal_inspections_seen=total,
        groups=dict(total_recorded=total,overwritten=max(0,total-512),rows=[group(n) for n in range(first,total+1)]),
        inspections=dict(total_recorded=total,overwritten=max(0,total-512),rows=[inspection(n) for n in range(first,total+1)]))


def status(trace, changes=None):
    identity=dict(pid='123',node_id='2',root_digest='a'*64,store_incarnation='b'*32,
        process_boot_id='12345678-1234-1234-1234-123456789abc',process_start_ticks='456')
    identity.update(changes or {})
    return ('\n'.join(f'{k}={v}' for k,v in identity.items())+
        '\nprivate_unapproved_field=DO_NOT_EXPORT_THIS_VALUE\nwrite_stage_trace='+
        json.dumps(trace,separators=(',',':'))+'\n').encode()


def analyze(before, after):
    return reader.analyze_status(status(before),status(after))


class Controls(unittest.TestCase):
    def test_valid_join_has_exact_stage_intervals_and_no_secret_output(self):
        result=analyze(snapshot(),snapshot(1))
        self.assertTrue(result['accepted_retained_window_analysis'])
        self.assertFalse(result['complete_history'])
        self.assertFalse(result['causality_proven'])
        self.assertEqual(result['joins'][0]['status'],'unique_compatible_pair')
        self.assertEqual(set(result['joins'][0]['intervals_ns'].values()),{1})
        self.assertNotIn('DO_NOT_EXPORT_THIS_VALUE',json.dumps(result))
        self.assertNotIn('private_unapproved_field',json.dumps(result))

    def test_actual_gaps_and_terms_do_not_fabricate_group_members(self):
        after=snapshot(2)
        first=group(1);first['commands']=3
        second=copy.deepcopy(first);second.update(sequence=2,term=2,index=48)
        after['groups'].update(rows=[first,second])
        after.update(groups_seen=1,group_commands_seen=3,terminal_inspections_seen=1)
        after['inspections'].update(total_recorded=1,rows=[inspection(1,index=48,term=2)])
        result=analyze(snapshot(),after)
        self.assertEqual([(r['term'],r['index']) for r in result['retained_groups']],[(1,16),(2,48)])
        self.assertEqual(result['join_counts'],{'unmatched_group':1,'unique_compatible_pair':1})

    def test_replaced_term_is_not_joined_by_index_alone(self):
        after=snapshot(1)
        after['groups']['rows'][0]['term']=2
        after['inspections']['rows'][0]['outcome']='replaced'
        result=analyze(snapshot(),after)
        self.assertEqual(result['join_counts'],{'unmatched_group':1,'unmatched_inspection':1})

    def test_missing_group_and_missing_inspection_are_explicit(self):
        for kind in ('groups','inspections'):
            with self.subTest(kind=kind):
                after=snapshot(1)
                after[kind]=dict(total_recorded=0,overwritten=0,rows=[])
                result=analyze(snapshot(),after)
                self.assertEqual(len(result['joins']),1)
                self.assertTrue(result['joins'][0]['status'].startswith('unmatched_'))

    def test_duplicate_exact_group_or_inspection_is_ambiguous(self):
        for kind in ('groups','inspections'):
            with self.subTest(kind=kind):
                after=snapshot(2)
                after[kind]['rows'][1]['index']=16
                result=analyze(snapshot(),after)
                at16=next(j for j in result['joins'] if j['index']==16)
                self.assertEqual(at16['status'],'ambiguous_exact_identity')
                self.assertNotIn('intervals_ns',at16)

    def test_failed_apply_and_non_success_receipts_have_no_success_intervals(self):
        for outcome in ('failed','replaced','unconfirmed'):
            with self.subTest(outcome=outcome):
                after=snapshot(1)
                after['groups']['rows'][0]['receipts_inserted_ns']=None
                after['inspections']['rows'][0]['outcome']=outcome
                result=analyze(snapshot(),after)
                self.assertEqual(result['joins'][0]['status'],'unique_non_successful_receipt_pair')
                self.assertNotIn('intervals_ns',result['joins'][0])

    def test_wrap_reports_retained_window_and_missing_new_rows(self):
        result=analyze(snapshot(10),snapshot(530))
        for ring in ('groups','inspections'):
            coverage=result['coverage'][ring]
            self.assertEqual(coverage['new_recorded_rows'],520)
            self.assertEqual(coverage['retained_new_rows'],512)
            self.assertEqual(coverage['missing_new_rows'],8)
            self.assertEqual(coverage['retained_union_rows'],522)
        self.assertFalse(result['complete_history'])

    def test_overlap_rows_cannot_change(self):
        before,after=snapshot(10),snapshot(12)
        after['groups']['rows'][0]['encoded_bytes']+=1
        with self.assertRaisesRegex(reader.Refusal,'overlap row changed'):
            analyze(before,after)

    def test_counter_and_sequence_and_overwrite_refusals(self):
        bad=[]
        a=snapshot(1);a['groups']['rows'][0]['sequence']=2;bad.append(a)
        a=snapshot(1);a['groups']['overwritten']=1;bad.append(a)
        a=snapshot(1);a['group_commands_seen']=0;bad.append(a)
        a=snapshot(1);a['inspections']['rows']=[];bad.append(a)
        for after in bad:
            with self.subTest(after=bad.index(after)),self.assertRaises(reader.Refusal):
                analyze(snapshot(),after)
        after=snapshot();after.update(capture_started_ns=100,capture_finished_ns=101)
        with self.assertRaisesRegex(reader.Refusal,'counter reset'):
            analyze(snapshot(1),after)

    def test_strict_schema_numeric_boolean_and_finite_types(self):
        for key,value in [('schema_version',True),('trace_instance',0),('sample_stride',True),
                          ('groups_seen',1.0),('valid',1),('rows_available',1),('terminal_inspections_seen',1<<64)]:
            after=snapshot(1);after[key]=value
            with self.subTest(key=key),self.assertRaises(reader.Refusal):analyze(snapshot(),after)
        for key,value in [('index',True),('index',17),('term',0),('commands',129),('apply_started_ns',-1),('receipts_inserted_ns',False)]:
            after=snapshot(1);after['groups']['rows'][0][key]=value
            with self.subTest(key=key,value=value),self.assertRaises(reader.Refusal):analyze(snapshot(),after)
        after=snapshot(1);after['unexpected_field']=0
        with self.assertRaises(reader.Refusal):analyze(snapshot(),after)
        after=snapshot(1);after['inspections']['rows'][0]['outcome']='unknown'
        with self.assertRaises(reader.Refusal):analyze(snapshot(),after)

    def test_timestamp_order_capture_bound_and_registration_age(self):
        for key,value in [('locks_acquired_ns',9),('apply_started_ns',10),('apply_finished_ns',100),('receipts_inserted_ns',12)]:
            after=snapshot(1);after['groups']['rows'][0][key]=value
            with self.subTest(key=key),self.assertRaises(reader.Refusal):analyze(snapshot(),after)
        after=snapshot(1);after['inspections']['rows'][0]['registration_age_ns']=100
        with self.assertRaises(reader.Refusal):analyze(snapshot(),after)
        with self.assertRaisesRegex(reader.Refusal,'temporally ordered'):analyze(snapshot(2),snapshot(1))

    def test_cross_join_time_incompatibility_is_not_fabricated_duration(self):
        after=snapshot(1)
        after['inspections']['rows'][0].update(inspect_started_ns=10,inspect_finished_ns=11)
        result=analyze(snapshot(),after)
        self.assertEqual(result['joins'][0]['status'],'unique_time_incompatible_pair')
        self.assertNotIn('intervals_ns',result['joins'][0])

    def test_loss_busy_and_invalid_snapshots_refuse_analysis(self):
        for key,value in [('dropped_recording_calls',1),('valid',False),('rows_available',False)]:
            for when in ('before','after'):
                before,after=snapshot(),snapshot(1)
                (before if when=='before' else after)[key]=value
                with self.subTest(key=key,when=when),self.assertRaises(reader.Refusal):analyze(before,after)

    def test_process_store_and_trace_replacement_refuse(self):
        changes=dict(pid='124',node_id='3',root_digest='c'*64,store_incarnation='d'*32,
            process_boot_id='87654321-1234-1234-1234-123456789abc',process_start_ticks='457')
        for key,value in changes.items():
            with self.subTest(key=key),self.assertRaisesRegex(reader.Refusal,'replaced'):
                reader.analyze_status(status(snapshot()),status(snapshot(1),{key:value}))
        after=snapshot(1);after['trace_instance']=2
        with self.assertRaisesRegex(reader.Refusal,'replaced'):analyze(snapshot(),after)

    def test_missing_duplicate_and_bounded_raw_fields(self):
        raw=status(snapshot(1))
        for bad in [raw.replace(b'node_id=2\n',b''),raw+b'node_id=2\n',
                    raw.replace(b'"schema_version":1',b'"schema_version":1,"schema_version":1'),
                    raw.replace(b'"trace_instance":1',b'"trace_instance":NaN'),
                    b'x'*(reader.MAX_STATUS_BYTES+1)]:
            with self.subTest(length=len(bad)),self.assertRaises(reader.Refusal):
                reader.analyze_status(status(snapshot()),bad)

    def test_huge_json_integer_is_a_preserved_refusal(self):
        malformed=status(snapshot(1)).replace(b'"trace_instance":1',b'"trace_instance":'+b'9'*5000)
        with self.assertRaises(reader.Refusal):
            reader.analyze_status(status(snapshot()),malformed)
        with tempfile.TemporaryDirectory(prefix='trace-reader-integer-',dir=OUTPUT_PARENT) as temp:
            root=Path(temp);before=root/'before.txt';after=root/'after.txt';output=root/'refused.json'
            before.write_bytes(status(snapshot()));after.write_bytes(malformed)
            result=subprocess.run([sys.executable,'-B',str(READER),'--before',str(before),
                '--after',str(after),'--output',str(output)],capture_output=True,timeout=10)
            self.assertEqual(result.returncode,1)
            saved=json.loads(output.read_text())
            self.assertFalse(saved['complete'])
            self.assertFalse(saved['accepted_retained_window_analysis'])
            self.assertNotIn('DO_NOT_EXPORT_THIS_VALUE',output.read_text())

    def test_cli_preserves_first_success_and_reports_refusal_without_secrets(self):
        with tempfile.TemporaryDirectory(prefix='trace-reader-control-',dir=OUTPUT_PARENT) as temp:
            root=Path(temp);before=root/'before.txt';after=root/'after.txt';output=root/'result.json'
            before.write_bytes(status(snapshot()));after.write_bytes(status(snapshot(1)))
            argv=[sys.executable,'-B',str(READER),'--before',str(before),'--after',str(after),'--output',str(output)]
            first=subprocess.run(argv,capture_output=True,timeout=10)
            self.assertEqual(first.returncode,0,first.stderr.decode())
            saved=output.read_bytes()
            self.assertNotIn(b'DO_NOT_EXPORT_THIS_VALUE',saved)
            second=subprocess.run(argv,capture_output=True,timeout=10)
            self.assertNotEqual(second.returncode,0)
            self.assertEqual(saved,output.read_bytes())
            after.write_bytes(status(snapshot(1),{'pid':'124'}))
            refused=root/'refused.json';argv[-1]=str(refused)
            third=subprocess.run(argv,capture_output=True,timeout=10)
            self.assertEqual(third.returncode,1)
            result=json.loads(refused.read_text())
            self.assertFalse(result['complete'])
            self.assertFalse(result['accepted_retained_window_analysis'])
            self.assertNotIn('DO_NOT_EXPORT_THIS_VALUE',refused.read_text())


def main():
    global OUTPUT_PARENT
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--only',action='append',choices=unittest.defaultTestLoader.getTestCaseNames(Controls),
                        help='Run only a newly changed control, preserving earlier control results.')
    args=parser.parse_args()
    if args.output.exists():raise SystemExit('control output already exists')
    OUTPUT_PARENT=args.output.parent
    log=io.StringIO();started=time.time_ns()
    suite=(unittest.TestSuite(Controls(name) for name in args.only) if args.only else
           unittest.defaultTestLoader.loadTestsFromTestCase(Controls))
    result=unittest.TextTestRunner(stream=log,verbosity=2).run(suite)
    pins={str(p):dict(bytes=p.stat().st_size,sha256=hashlib.sha256(p.read_bytes()).hexdigest()) for p in (READER,Path(__file__))}
    report=dict(complete=result.wasSuccessful(),scope='Synthetic offline reader controls only; no live capture acceptance',
        tests=result.testsRun,failures=len(result.failures),errors=len(result.errors),skipped=len(result.skipped),
        started_unix_ns=started,finished_unix_ns=time.time_ns(),argv=sys.argv,source_pins=pins,log=log.getvalue())
    with args.output.open('x') as handle:handle.write(json.dumps(report,indent=2)+'\n')
    print(json.dumps({k:report[k] for k in ('complete','tests','failures','errors','skipped')}))
    return 0 if result.wasSuccessful() else 1


if __name__=='__main__':raise SystemExit(main())
