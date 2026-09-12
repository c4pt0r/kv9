#!/usr/bin/env python3
"""Bounded reporting extraction from the already accepted four-cohort analysis.

No fixture, validator, trace correlation, codec or performance run is executed.
"""
import csv
from collections import Counter
import hashlib
import io
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
PREP = Path('/tmp/kv9-quorum-trace-fixture-preparation-first')
RUN = Path('/tmp/kv9-quorum-trace-isolation-first/cohorts')
ROOT = Path('/tmp/kv9-quorum-trace-fixture-root-first')
MAX_BYTES = 64 * 1024**2
ANALYSIS_SHA = '99f9624670dd154f1dbfda0ebaff7f1a40d5c55138a3943025334e3759029bf3'
INVENTORY_SHA = 'd730a62bd1f34c6488328db4ac16e6abda74c867f4a5326dc8364c0b628da266'
INPUTS = {}


def require(ok, message):
    if not ok:
        raise ValueError(message)


def pin(path, expected=None):
    path = Path(path)
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= MAX_BYTES,
            'bounded reporting input differs: ' + str(path))
    data = path.read_bytes()
    entry = dict(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
    require(expected is None or entry == expected, 'accepted input bytes differ: ' + str(path))
    require(str(path) not in INPUTS or INPUTS[str(path)] == entry, 'report input changed')
    INPUTS[str(path)] = entry
    return data


def load(path, expected=None):
    return json.loads(pin(path, expected))


def write_json(path, value):
    data = (json.dumps(value, indent=2, sort_keys=True) + '\n').encode()
    require(len(data) <= MAX_BYTES, 'reporting output exceeds bound')
    with path.open('xb') as stream:
        stream.write(data)


def table(headers, rows):
    return '| ' + ' | '.join(headers) + ' |\n|' + '|'.join(['---'] * len(headers)) + '|\n' + ''.join(
        '| ' + ' | '.join(map(str, row)) + ' |\n' for row in rows)


def interval(value):
    return 'unavailable' if value is None else f"{value['lower_ns']/1000:.3f}–{value['upper_ns']/1000:.3f}"


def percent(new, old):
    return 100 * (new / old - 1)


def main():
    output = HERE / 'results-first'
    output.mkdir(exist_ok=False)
    a = load(PREP / 'results-first/analysis.json', dict(bytes=19490312, sha256=ANALYSIS_SHA))
    inventory = load(PREP / 'results-first/input-inventory.json', dict(bytes=70534, sha256=INVENTORY_SHA))
    require(a['accepted'] is True and a['complete'] is True and a['parent_confirmed_session'] == 15017 and
            a['parent_confirmed_exit_code'] == 0 and len(a['cohorts']) == 4, 'accepted four-cohort authority differs')
    def accepted(path):
        return load(path, inventory[str(path)])
    protocol = accepted(RUN / 'protocol.json')
    require([c['descriptor']['server_role'] for c in a['cohorts']] == ['control', 'trace', 'trace', 'control'],
            'original observer-overhead order differs')
    clients, traces = [], []
    stage_rows, span_rows = [], []
    for cohort in a['cohorts']:
        d = cohort['descriptor']
        folder = RUN / f"{cohort['index']:03d}-{d['arm']}"
        report = accepted(folder / 'run/report.json')
        runtime = cohort['runtime']
        operations = []
        for phase, metrics in report['metrics'].items():
            for name, op in zip(metrics['operations'], metrics['statistics'], strict=True):
                operations.append(dict(phase=phase, operation=name,
                    outcomes={outcome:p['calls'] for outcome,p in zip(metrics['outcomes'],op['populations'],strict=True)},
                    calls=sum(p['calls'] for p in op['populations']), input_items=sum(p['input_items'] for p in op['populations']),
                    attempts=sum(h['raw']['count'] for h in op['attempts'])))
        measured = next(op for op in runtime['operations'] if op['operation'] == 'get')
        histogram = report['metrics']['measurement']['statistics'][0]['populations'][0]['whole_call']
        require(histogram['raw']['count'] == measured['successful_whole_call_latency']['count'] and
                histogram['raw']['sum_ns'] == measured['successful_whole_call_latency']['sum_ns'] and
                all(histogram[k] == measured['successful_whole_call_latency'][k] for k in ['mean_ns','p50','p95','p99']),
                'original report and accepted client latency differ')
        client = dict(index=cohort['index'], arm=d['arm'], role=d['server_role'], repeat=d['repeat'],
            process_id=report['process_id'], elapsed_ns=report['cohort_elapsed_ns'], issued=report['measured_issued'],
            completed=report['measured_completed'], dropped_slots=report['dropped_slots'],
            measured_outcomes=runtime['outcomes'], measured_attempts=runtime['attempts'],
            healthy_single_attempt=runtime['healthy_single_attempt'],
            completed_after_cutoff=report['measured_completed']-sum(p['completed_before_cutoff']
                for op in report['metrics']['measurement']['statistics'] for p in op['populations']),
            successful_gets_per_second=measured['successful_calls_per_second'],
            successful_whole_call_latency=measured['successful_whole_call_latency'], phase_operations=operations,
            all_phase_calls=sum(row['calls'] for row in operations), all_phase_attempts=sum(row['attempts'] for row in operations),
            dataset=runtime['dataset'], memory=runtime['memory'], storage=runtime['storage'],
            read_stages=cohort['read_stages'], original_report=str(folder/'run/report.json'))
        clients.append(client)
        if cohort['quorum_trace']:
            states = accepted(folder/'after-status.json')
            for node in cohort['quorum_trace']['nodes']:
                trace = node['analysis']; node_id = str(node['node'])
                status = states[node_id]
                require(int(status['pid']) == trace['identity']['process_id'], 'accepted trace/status PID differs')
                entry = {k:trace[k] for k in ['identity','captured_unix_ns','captured_at_ns','total_events',
                    'observations_lost','observation_loss','stage_counts','group_event_count','message_event_count',
                    'ticket_event_count','unique_local_tickets','unique_contexts','span_accounting','distributions',
                    'storage_order_reversals','timestamp_tie_events','complete_context_attribution_allowed','scope']}
                entry.update(arm=d['arm'], repeat=d['repeat'], node=int(node_id), observed_role=status['role'],
                    observed_term=status['term'], observed_leader=status['leader_id'],
                    complete_context_count=sum(c['complete_context_timing'] for c in trace['contexts']),
                    complete_group_chain_count=sum(c['complete_group_chain_timing'] for c in trace['contexts']),
                    group_term_status_counts=dict(Counter(c['group_term_status'] for c in trace['contexts'])),
                    ticket_identity_status_counts=dict(Counter(t['status'] for t in trace['tickets'])),
                    matched_ticket_span_count=sum(s['status']=='matched' for t in trace['tickets'] for s in t['spans']),
                    partial_matched_ticket_span_count=sum(s['status']=='matched' and s.get('partial_observation') is True
                        for t in trace['tickets'] for s in t['spans']))
                traces.append(entry)
                for name, counts in trace['stage_counts'].items():
                    stage_rows.append(dict(arm=d['arm'],node=int(node_id),stage=name,
                        **{k:counts[k] for k in ['offered','selected','recorded','contended','poisoned','full','exhausted']},
                        **{'recorded_'+kind:v for kind,v in counts['recorded_by_kind'].items()}))
                for name, count in trace['span_accounting'].items():
                    span, status_name = name.split(':',1)
                    span_rows.append(dict(arm=d['arm'],node=int(node_id),span=span,status=status_name,count=count))
    comparisons=[]
    for repeat in [0,1]:
        control=next(c for c in clients if c['role']=='control' and c['repeat']==repeat)
        trace=next(c for c in clients if c['role']=='trace' and c['repeat']==repeat)
        comparisons.append(dict(repeat=repeat,order='control→trace' if repeat==0 else 'trace→control',
            control_arm=control['arm'],trace_arm=trace['arm'],
            get_rate_change_percent=percent(trace['successful_gets_per_second'],control['successful_gets_per_second']),
            whole_call_mean_change_percent=percent(trace['successful_whole_call_latency']['mean_ns'],
                                                  control['successful_whole_call_latency']['mean_ns'])))
    pooled={}
    for role in ['control','trace']:
        selected=[c for c in clients if c['role']==role]
        count=sum(c['completed'] for c in selected);elapsed=sum(c['elapsed_ns'] for c in selected)
        total=sum(c['successful_whole_call_latency']['sum_ns'] for c in selected)
        pooled[role]=dict(count=count,elapsed_ns=elapsed,sum_ns=total,gets_per_second=count*1e9/elapsed,mean_ns=total/count)
    receipt=load(ROOT/'runtime-first/terminal-receipt.json')
    runtime_result=load(ROOT/'runtime-first/result.json')
    readback_result=load(ROOT/'readback-first/result.json')
    require(receipt['session_id']==15017 and receipt['exit_code']==0 and receipt['chunk_id']=='c5772f' and
            runtime_result['complete'] and runtime_result['exit_code']==0 and
            readback_result['complete'] and readback_result['exit_code']==0, 'original terminal reporting differs')
    terminals=dict(runtime=receipt,readback=dict(session_id=55391,exit_code=0,tool_receipt='468553',
        provenance='Actual yielded tool terminal explicitly supplied by root; no separate root terminal file existed at handoff.',
        original_result_path=str(ROOT/'readback-first/result.json'),original_result_sha256=INPUTS[str(ROOT/'readback-first/result.json')]['sha256']))
    for stage in ['runtime-first','readback-first']:
        for name in ['invocation.json','output.log']:
            pin(ROOT/stage/name)
    totals=dict(measured_calls=sum(c['completed'] for c in clients), measured_attempts=sum(c['measured_attempts'] for c in clients),
        dropped_slots=sum(c['dropped_slots'] for c in clients), all_phase_calls=sum(c['all_phase_calls'] for c in clients),
        all_phase_attempts=sum(c['all_phase_attempts'] for c in clients),
        recorded_trace_events=sum(t['total_events'] for t in traces),lost_selected_observations=sum(t['observations_lost'] for t in traces),
        complete_context_count=sum(t['complete_context_count'] for t in traces),
        valid_partial_ticket_spans=sum(t['partial_matched_ticket_span_count'] for t in traces))
    require(all(t['complete_context_attribution_allowed'] is False and t['complete_context_count']==0 for t in traces),
            'this reporting narrative requires the actual globally lossy prefixes')
    summary=dict(scope='Reporting extraction from accepted original analysis; no audit rerun or performance promotion.',
        protocol_id=protocol['protocol_id'], clients=clients, comparisons=comparisons, pooled=pooled,processes=traces,totals=totals,
        acceptance={k:a[k] for k in ['accepted','complete','owned_lifetimes_exited','metric_documents','fresh_drain_documents',
                                   'voter_listener_bindings','retained_files','retained_bytes','outer_restoration']},terminals=terminals)
    write_json(output/'summary.json',summary)
    write_json(output/'receipt-summary.json',terminals)
    for name, rows in [('stage-counts.csv',stage_rows),('span-accounting.csv',span_rows)]:
        with (output/name).open('x',newline='') as stream:
            writer=csv.DictWriter(stream,fieldnames=list(rows[0]));writer.writeheader();writer.writerows(rows)

    report = '# C1 quorum observer: accepted capture, incomplete context timing\n\n'
    report += f"All four original cohorts passed runtime and independent readback: {totals['measured_calls']:,} measured GETs succeeded with one attempt, no dropped slots, and {totals['all_phase_calls']:,} successful all-phase calls. All six trace prefixes lost selected observations to observer contention ({totals['lost_selected_observations']:,} in total). **No complete-context RTT attribution is available.** This is an instrumentation diagnostic, not a performance promotion.\n\n"
    report += 'The fixed order was control → trace → trace → control, one c1 pure-GET cohort for each role in each order. Every cohort used 5 seconds, 4,096 keys plus sentinel, 128-byte values, 128 warmup calls, seed 71, the original closed-loop v3 streaming client, three ordinary voters and tmpfs WAL. Client/server/helper CPU placement and storage/lifecycle guards were unchanged. No Redis trial, loaded c64 trial, fault or new optimization comparison is included.\n\n'
    report += '## All four client results\n\nWhole-call latency is the measured client call, including the original post-cutoff completion. Each cohort has one such completion; none is dropped or removed. Quantiles below are raw histogram bucket intervals, not averaged percentiles.\n\n'
    report += table(['Cohort','GET successes / attempts','GET/s','Mean µs','p50 µs','p95 µs','p99 µs','All-phase calls'],[
        [c['arm'],f"{c['completed']:,} / {c['measured_attempts']:,}",f"{c['successful_gets_per_second']:.3f}",
         f"{c['successful_whole_call_latency']['mean_ns']/1000:.6f}",*[interval(c['successful_whole_call_latency'][q]) for q in ['p50','p95','p99']],f"{c['all_phase_calls']:,}"] for c in clients])
    report += '\nPer cohort, initialization contains 4,097 BatchGet and 4,097 BatchPut calls, warmup 128 point GETs, and verification 4,097 BatchGets. Measured point PUT populations are zero. These setup/verification batches retain their original labels. All outcome, attempt and API/item populations remain in `summary.json` and the four original reports. Final scans retained 4,097 deterministic values with nonce zero and unchanged sentinel; no complete history or linearizability proof is inferred.\n\n'
    report += table(['Order','Trace GET-rate change','Trace whole-call mean change'],[[c['order'],f"{c['get_rate_change_percent']:+.4f}%",f"{c['whole_call_mean_change_percent']:+.4f}%"] for c in comparisons])
    report += f"\nPooling by successful calls / elapsed time gives {pooled['control']['gets_per_second']:.3f} control and {pooled['trace']['gets_per_second']:.3f} trace GET/s ({percent(pooled['trace']['gets_per_second'],pooled['control']['gets_per_second']):+.4f}%). Count-weighted whole-call means are {pooled['control']['mean_ns']/1000:.6f} and {pooled['trace']['mean_ns']/1000:.6f} µs. No pooled percentile is computed. Both trace p99 intervals are 45.056–45.567 µs versus 44.544–45.055 µs for control.\n\n"
    report += 'This measures the combined diagnostic build, including both quorum-message observation and the six read-stage histograms. It does not isolate the cost of the raw trace collector, establish statistical significance, or remove shared-host/order/cache effects. Source and feature identities are listed below.\n\n'
    report += '## Six independently bound process prefixes\n\nNode 2 was the observed leader, term 1, in both trace cohorts. Identities below are independently joined to launch/cleanup, listener, status and exporter evidence. Counts belong to each local prefix, including setup, verification and post-client activity; they are not measurement-window event counts.\n\n'
    report += table(['Cohort / node','PID','Role','Recorded events','Contended losses','Other losses','Local tickets','Valid partial spans','Complete contexts'],[
        [f"{t['arm']} / {t['node']}",t['identity']['process_id'],t['observed_role'],t['total_events'],t['observation_loss']['contended'],
         sum(v for k,v in t['observation_loss'].items() if k!='contended'),t['unique_local_tickets'],t['partial_matched_ticket_span_count'],t['complete_context_count']]for t in traces])
    report += '\nThe observer recorded 40,113 events and lost 1,083 selected observations; every loss was `contended`, with no full/poisoned/exhausted loss. These are lost diagnostic observations, not evidence of dropped Raft messages. Deterministic context-suffix sampling, missing observations and the finite prefix prevent treating the remaining sample as an unbiased latency population. `stage-counts.csv` preserves all 23 rows for each of six processes, including zero counts and recorded message kinds.\n\n'
    report += '## Ticket-local spans that remain available\n\nEvery matched duration below is marked partial observation. Ticket/key/route identity and local timestamp ordering are retained. Spans overlap and have differing populations; neither means nor follower durations may be added or subtracted to produce request latency. Accepted→dequeued can be reversed because admission is recorded after publication; reversed durations remain unavailable, never clamped to zero.\n\n'
    valid=[]
    for t in traces:
        for name,h in t['distributions'].items():
            if h['count']:
                valid.append([f"r{t['repeat']}/n{t['node']}",name,h['count'],h['sum_ns'],f"{h['mean_ns']/1000:.6f}",interval(h['p50']),interval(h['p95']),interval(h['p99']),
                    t['span_accounting'].get(name+':unmatched',0),t['span_accounting'].get(name+':reversed',0)])
    report += table(['Process','Span','Count','Sum ns','Mean µs','p50 µs','p95 µs','p99 µs','Unmatched','Reversed'],valid)
    report += '\n## Unavailable context timing\n\nGlobal selected-observation loss disables every context/message timing join in each prefix, even where endpoint candidates appear locally plausible. The frozen reader remains unchanged. No cross-process clock subtraction, first-response matching or synthetic zero latency is used. Group terms remain unobserved under this loss rule. CompletionEligible is a selected registry member observation, not successful client delivery or proof of which follower closed quorum.\n\n'
    names=['leader_dequeued_to_response_validated','leader_group_submit_to_heartbeat_outbound_offered','leader_group_submit_to_heartbeat_outbound_accepted','leader_response_validated_to_driver_step','leader_response_driver_step_to_quorum_confirmed']
    report += table(['Leader prefix','Context candidate span','Unavailable: loss','Unavailable: repeated context/term','Matched'],[
        [t['arm'],name,t['span_accounting'].get(name+':observation_loss',0),t['span_accounting'].get(name+':ambiguous_repeated_context_or_term',0),t['span_accounting'].get(name+':matched',0)]
        for t in traces if t['observed_role']=='leader' for name in names])
    report += '\nFor the dequeue→response candidate alone, all 2,308 observed leader-edge candidates are unavailable: 2,306 classified as observation loss and two as repeated context/term ambiguity. These are local candidate-edge counts, not distinct requests. Every other unavailable group/follower/message span and reason is retained in `span-accounting.csv`; empty distributions retain null latency fields in `summary.json`. The recorded losses block the requested complete-context conclusion; they do not invalidate the successfully accounted client cohorts or the explicitly partial ticket observations.\n\n'
    report += '## Source, acceptance and limitations\n\n'
    bindings=load(PREP/'root-build-bindings.json')
    report += table(['Role','Revision','Server SHA-256'],[[b['role'],b['revision'],b['binary_sha256']]for b in bindings])
    report += '\nControl uses empty production features. Trace uses root/server `quorum-trace`, Raft `quorum-trace` plus `read-stage-timing`, and an unchanged empty engine feature set. Both retain ThinLTO release codegen; fixed client revision is `0be806d9671e2c50701a64aa7889c8859b7648ba`, binary `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`. Runtime `15017 / 0 / c5772f` and independent readback `55391 / 0 / 468553` are distinct terminal executions. The audit accepted 16 exited lifetimes, 12 fresh drains, 24 endpoint metric documents, 12 voter/listener bindings and exact outer restoration. It independently verified 144 retained fixture files totaling 32,567,058 bytes.\n\n'
    report += 'The original helper preparation and its correction history remain intact. The initial 13-test campaign run passed 12 and failed one negative test because it expected ValueError while the preserved validator raised AssertionError (`057aaa/1`). Only that test exception expectation changed; the one repaired control passed `8e328d/0`, without rerunning the other 12. Earlier reader revisions and the complete-chain/identity corrections, all original controls and the read-only discovery failure are inventory-bound. They are preparation history, not repeated production measurements.\n\n'
    report += 'The bounded inventory references original source/report/trace bytes and the accepted full input inventory. It is not a WAL, executable or payload backup and does not claim standalone replay of the entire fixture audit. The separately archived [original evidence](original-evidence.tar.gz) and [archive inventory](archive-inventory.json) retain root\'s full selected acceptance inputs: archive SHA-256 `7eb6413a12a4bebdb62762506ca021773f0abf7a4fffc8130a0255662b04fb22`, 18,936,215 stored bytes / 98,045,608 raw bytes, independently read back by root (`83c66c/0`). This report did not reprocess that archive. The next collector implementation, any further capture and any acceptance of complete RTT require fresh qualification; none is inferred here.\n'
    (output/'REPORT.md').write_text(report)

    # Select exact, bounded reporting inputs only; never hash WALs or executables.
    for path_s, expected in inventory.items():
        path=Path(path_s)
        if path.is_relative_to(RUN):
            rel=path.relative_to(RUN)
            if 'fixture' in rel.parts:
                keep=path.name in {'tmpfs-retention.json','tmpfs-voters.json','tmpfs-created.json'}
            else:
                keep=path.suffix in {'.json','.txt'}
            if keep:pin(path,expected)
    for path_s, expected_sha in protocol['helper_hashes'].items():
        path=Path(path_s);data=pin(path)
        require(hashlib.sha256(data).hexdigest()==expected_sha,'frozen helper differs')
    for root in [PREP,Path('/tmp/kv9-quorum-trace-reader-preparation-first'),Path('/tmp/kv9-quorum-trace-export-hook-preparation-first')]:
        for path in sorted(root.rglob('*')):
            relative=path.relative_to(root)
            if any(part in {'results-first','synthetic-fixtures-first'} for part in relative.parts):continue
            if path.is_file() and not path.is_symlink() and path.suffix in {'.py','.rs','.json','.md','.log','.diff'}:
                pin(path)
    for binding in protocol['role_bindings'].values():
        for path_s, expected_sha in binding['evidence'].items():
            data=pin(Path(path_s));require(hashlib.sha256(data).hexdigest()==expected_sha,'original build reporting differs')
    write_json(output/'source-and-raw-input-inventory.json',dict(scope='Bounded original reporting/source/raw-trace selection; no raw WAL/ELF backup.',
        files=INPUTS,file_count=len(INPUTS),total_bytes=sum(v['bytes'] for v in INPUTS.values()),
        accepted_full_runtime_inventory=dict(path=str(PREP/'results-first/input-inventory.json'),sha256=INVENTORY_SHA),
        omissions=['Raw WAL/data payloads and executables','Unrelated GitHub API bodies or credentials','Synthetic fixture payload trees (their frozen inventories remain referenced)']))
    print(json.dumps(dict(complete=True,totals=totals,comparisons=comparisons,report=str(output/'REPORT.md'),
        input_files=len(INPUTS),input_bytes=sum(v['bytes'] for v in INPUTS.values()),stage_rows=len(stage_rows),span_accounting_rows=len(span_rows)),indent=2))


if __name__=='__main__':main()
