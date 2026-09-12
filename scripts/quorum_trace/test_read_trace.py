"""Finite synthetic schema/correlation controls; no server, codec or real trace reads."""
import copy,json,unittest
import read_trace as r

def event(stage,at,*,kind='heartbeat',source=1,target=2,term=7,ticket=None,route=None,members=None,index=None,nonce=256):
    return dict(at_ns=at,stage=stage,key=dict(context=list(bytes(16)+nonce.to_bytes(8,'big')),term=term,**{'from':source,'to':target},kind=kind),
                local_ticket=ticket,local_route=route,members=members,index=index)

def group(stage,at,members=4,index=None):return event(stage,at,kind='group',source=1,target=1,term=None,members=members,index=index)

def fixture(events=None,node=1):
    if events is None:
        events=[group('group_submit',1),group('group_admitted',2),
                event('outbound_offered',3,ticket=1,route=100),event('outbound_accepted',4,ticket=1,route=100),
                event('outbound_dequeued',6,ticket=1,route=100),
                event('inbound_validated',20,kind='heartbeat_response',source=2,target=1),
                event('inbox_offered',21,kind='heartbeat_response',source=2,target=1,ticket=2),
                event('inbox_admitted',22,kind='heartbeat_response',source=2,target=1,ticket=2),
                event('inbox_drained',24,kind='heartbeat_response',source=2,target=1,ticket=2),
                event('driver_step',25,kind='heartbeat_response',source=2,target=1),
                group('quorum_confirmed',26,index=9),group('completion_eligible',30,index=9)]
    counts=[dict.fromkeys(r.COUNTS,0) for _ in r.STAGES]
    for e in events:
        c=counts[r.STAGES.index(e['stage'])]
        for k in ('offered','selected','recorded'):c[k]+=1
    identity=dict(node_id=node,process_id=123,exporter_created_unix_ns='1000000000',process_start_ticks=50,boot_id='synthetic-boot')
    trace=dict(schema_version=1,node=node,region=0,process_id=123,sample_every=256,maximum_events=65536,captured_at_ns=100,
               counters_valid=True,clock='process_local_monotonic_instant',scope=r.SOURCE_SCOPE,
               stage_names=list(r.STAGES),stage_counts=counts,events=events)
    return dict(identity,captured_unix_ns='1100000000',trace=trace),identity

def only_context(result):
    assert len(result['contexts'])==1
    return result['contexts'][0]

class TraceControls(unittest.TestCase):
    def test_unique_leader_group_and_local_tickets(self):
        e,i=fixture();out=r.analyze(e,i);ctx=only_context(out)
        self.assertTrue(ctx['complete_context_timing']);self.assertEqual(ctx['group_term'],7)
        self.assertEqual(ctx['group_event_counts']['group_admitted'],1)
        self.assertEqual(ctx['member_observation_sums']['group_admitted'],4)
        self.assertEqual(ctx['completion_eligible_member_observations'],4)
        self.assertNotIn('client_successes',out)
        self.assertEqual(ctx['message_edges'][0]['spans'][0]['duration_ns'],14)
        spans={s['name']:s for s in ctx['message_edges'][0]['spans']}
        self.assertEqual(spans['leader_group_submit_to_heartbeat_outbound_offered']['duration_ns'],2)
        self.assertEqual(spans['leader_group_submit_to_heartbeat_outbound_accepted']['duration_ns'],3)
        self.assertEqual(spans['leader_response_validated_to_driver_step']['duration_ns'],5)
        self.assertEqual(spans['leader_response_driver_step_to_quorum_confirmed']['duration_ns'],1)
        self.assertFalse(spans['leader_response_driver_step_to_quorum_confirmed']['this_response_proved_quorum_closer'])
        self.assertEqual(out['total_events'],out['group_event_count']+out['message_event_count'])
        self.assertEqual(out['ticket_event_count'],6)
        self.assertEqual(sum(len(c['event_ordinals']) for c in out['contexts']),out['total_events'])

    def test_repeated_heartbeat_cannot_be_first_matched(self):
        e,i=fixture();events=e['trace']['events']+[event('outbound_offered',9,ticket=3,route=101),event('outbound_dequeued',10,ticket=3,route=101)]
        out=r.analyze(*fixture(events));ctx=only_context(out)
        self.assertFalse(ctx['complete_context_timing']);self.assertTrue(ctx['message_edges'][0]['repeated_context'])
        self.assertEqual(ctx['message_edges'][0]['spans'][0]['status'],'ambiguous_repeated_context_or_term')
        self.assertEqual(out['distributions']['leader_dequeued_to_response_validated']['count'],0)

    def test_repeated_response_is_ambiguous(self):
        e,_=fixture();e['trace']['events'].append(event('inbound_validated',23,kind='heartbeat_response',source=2,target=1))
        ctx=only_context(r.analyze(*fixture(e['trace']['events'])))
        self.assertEqual(ctx['message_edges'][0]['response_stage_counts']['inbound_validated'],2)
        self.assertEqual(ctx['message_edges'][0]['spans'][0]['status'],'ambiguous_repeated_context_or_term')

    def test_distinct_followers_are_separate_edges(self):
        e,_=fixture();events=e['trace']['events']+[event('outbound_offered',5,target=3,ticket=3,route=200),
          event('outbound_accepted',6,target=3,ticket=3,route=200),event('outbound_dequeued',8,target=3,ticket=3,route=200),
          event('inbound_validated',20,kind='heartbeat_response',source=3,target=1),
          event('inbox_offered',21,kind='heartbeat_response',source=3,target=1,ticket=4),
          event('inbox_admitted',22,kind='heartbeat_response',source=3,target=1,ticket=4),
          event('inbox_drained',23,kind='heartbeat_response',source=3,target=1,ticket=4),
          event('driver_step',24,kind='heartbeat_response',source=3,target=1)]
        ctx=only_context(r.analyze(*fixture(events)))
        self.assertEqual(len(ctx['message_edges']),2);self.assertTrue(ctx['complete_context_timing'])
        self.assertFalse(any(x['repeated_context'] for x in ctx['message_edges']))

    def test_unobserved_or_multiple_terms_never_invented(self):
        e,_=fixture();groups=[x for x in e['trace']['events'] if x['key']['kind']=='group']
        ctx=only_context(r.analyze(*fixture(groups)));self.assertIsNone(ctx['group_term'])
        self.assertTrue(ctx['complete_group_chain_timing']);self.assertFalse(ctx['complete_context_timing'])
        events=e['trace']['events']+[event('unknown_peer',7,term=8)]
        ctx=only_context(r.analyze(*fixture(events)));self.assertIsNone(ctx['group_term'])
        self.assertEqual(ctx['group_term_status'],'ambiguous');self.assertFalse(ctx['complete_context_timing'])

    def test_any_loss_disables_context_timing_but_ticket_spans_are_partial(self):
        for loss in r.LOSSES:
            e,i=fixture();c=e['trace']['stage_counts'][r.STAGES.index('unknown_peer')]
            c.update(offered=1,selected=1);c[loss]=1
            out=r.analyze(e,i);ctx=only_context(out)
            self.assertFalse(out['complete_context_attribution_allowed']);self.assertIsNone(ctx['group_term'])
            self.assertFalse(ctx['complete_group_chain_timing']);self.assertFalse(ctx['complete_context_timing'])
            spans=out['tickets'][0]['spans'];self.assertTrue(all(s['partial_observation'] for s in spans if s['status']=='matched'))
            self.assertEqual(ctx['message_edges'][0]['spans'][0]['status'],'observation_loss')

    def test_ticket_key_route_origin_and_terminal_conflicts_remain_unknown(self):
        for change in ('route','key','origin','terminal'):
            e,_=fixture();events=e['trace']['events']
            if change=='route':events[4]['local_route']=101
            elif change=='key':events[4]['key']['term']=8
            elif change=='origin':events=[x for x in events if x['stage']!='outbound_offered']
            else:events.append(event('outbound_rejected',7,ticket=1,route=100))
            out=r.analyze(*fixture(events));ticket=next(t for t in out['tickets'] if t['local_ticket']==1)
            self.assertEqual(ticket['status'],'unmatched_or_conflicting_identity');self.assertEqual(ticket['spans'],[])
            ctx=only_context(out);self.assertFalse(ctx['complete_context_timing'])
            self.assertTrue(all(s['status']!='matched' for edge in ctx['message_edges'] for s in edge['spans']))

    def test_reversed_accepted_dequeued_span_is_not_clamped(self):
        e,_=fixture();e['trace']['events'][3]['at_ns']=8
        out=r.analyze(*fixture(e['trace']['events']));spans=out['tickets'][0]['spans']
        self.assertEqual(spans[2]['status'],'reversed');self.assertNotIn('duration_ns',spans[2])
        self.assertEqual(spans[1]['duration_ns'],3)

    def test_follower_local_processing_is_not_leader_rtt(self):
        events=[event('inbound_validated',3),event('inbox_offered',4,ticket=1),event('inbox_admitted',5,ticket=1),
          event('inbox_drained',8,ticket=1),event('driver_step',9),
          event('outbound_offered',12,kind='heartbeat_response',source=2,target=1,ticket=2,route=100),
          event('outbound_accepted',13,kind='heartbeat_response',source=2,target=1,ticket=2,route=100),
          event('outbound_dequeued',14,kind='heartbeat_response',source=2,target=1,ticket=2,route=100)]
        out=r.analyze(*fixture(events,node=2));ctx=only_context(out)
        self.assertEqual([s['duration_ns'] for s in ctx['message_edges'][0]['spans']],[6,3])
        self.assertNotIn('leader_dequeued_to_response_validated',out['distributions'])
        self.assertEqual(ctx['group_spans'],[])

    def test_prefix_suffix_and_measured_timestamp_ties(self):
        e,i=fixture([event('inbound_validated',3,kind='heartbeat_response',source=2,target=1)])
        ctx=only_context(r.analyze(e,i));self.assertEqual(ctx['message_edges'][0]['spans'][0]['status'],'unmatched')
        e,_=fixture();e['trace']['events'][3]['at_ns']=e['trace']['events'][2]['at_ns']
        out=r.analyze(*fixture(e['trace']['events']));span=out['tickets'][0]['spans'][0]
        self.assertEqual(span['duration_ns'],0);self.assertTrue(span['timestamp_tie'])
        self.assertFalse(out['capture_is_process_shutdown'])
        empty=r.histogram([]);self.assertEqual(empty['count'],0);self.assertIsNone(empty['mean_ns']);self.assertIsNone(empty['p99'])

    def test_exact_counter_selector_clock_and_schema_failures(self):
        for change in ('names','counter','offered','overflow','selection','clock','time','bound','boolean','continuation'):
            e,i=fixture();s=e['trace']
            if change=='names':s['stage_names']=list(reversed(s['stage_names']))
            elif change=='counter':s['stage_counts'][0]['recorded']+=1
            elif change=='offered':s['stage_counts'][0]['offered']=0
            elif change=='overflow':s['counters_valid']=False
            elif change=='selection':s['events'][0]['key']['context'][-1]=1
            elif change=='clock':s['clock']='unix'
            elif change=='time':s['events'][0]['at_ns']=101
            elif change=='bound':s['maximum_events']=65537
            elif change=='boolean':s['events'][0]['at_ns']=True
            else:s['stage_counts'][r.STAGES.index('outbound_accepted')]['offered']+=1
            with self.subTest(change=change),self.assertRaises(ValueError):r.analyze(e,i)
        with self.assertRaises(ValueError):json.loads('{"a":1,"a":2}',object_pairs_hook=r.no_duplicates)

    def test_expected_process_identity_is_required(self):
        for name in r.IDENTITY:
            e,i=fixture();i[name]=None
            with self.subTest(name=name),self.assertRaises(ValueError):r.analyze(e,i)
        e,i=fixture();e['process_start_ticks']=i['process_start_ticks']=None
        with self.assertRaises(ValueError):r.analyze(e,i)
        e,i=fixture();e['trace']['process_id']=124
        with self.assertRaises(ValueError):r.analyze(e,i)

    def test_histogram_uses_raw_integer_counts_not_averaged_percentiles(self):
        h=r.histogram([1]*99+[1024])
        self.assertEqual(h['sum_ns'],1123);self.assertEqual(h['mean_ns'],11.23)
        self.assertEqual(h['p99'],{'lower_ns':1,'upper_ns':1})
        self.assertEqual(sum(x['count'] for x in h['buckets']),100)

    def test_late_response_is_not_a_quorum_closer_or_zero_duration(self):
        e,_=fixture()
        for event in e['trace']['events']:
            if event['key']['kind']=='heartbeat_response':event['at_ns']+=20
        ctx=only_context(r.analyze(*fixture(e['trace']['events'])))
        spans={s['name']:s for s in ctx['message_edges'][0]['spans']}
        late=spans['leader_response_driver_step_to_quorum_confirmed']
        self.assertEqual(late['status'],'reversed');self.assertNotIn('duration_ns',late)
        self.assertFalse(late['this_response_proved_quorum_closer'])
        self.assertTrue(ctx['complete_group_chain_timing']);self.assertFalse(ctx['complete_context_timing'])

    def test_repeated_group_admission_prevents_group_message_join(self):
        e,_=fixture();e['trace']['events'].append(group('group_admitted',3))
        out=r.analyze(*fixture(e['trace']['events']));ctx=only_context(out)
        self.assertEqual(ctx['group_event_counts']['group_admitted'],2)
        self.assertFalse(ctx['complete_group_chain_timing']);self.assertFalse(ctx['complete_context_timing'])
        self.assertTrue(all(s['status']=='ambiguous_group_context_or_index' for s in ctx['message_edges'][0]['spans']))

    def test_only_exact_declared_v1_source_scope_is_accepted(self):
        for scope in ('',r.SOURCE_SCOPE+' ',r.SOURCE_SCOPE.replace('not process shutdown','process shutdown')):
            e,i=fixture();e['trace']['scope']=scope
            with self.assertRaisesRegex(ValueError,'source capture scope'):r.analyze(e,i)

    def test_missing_queue_intermediates_never_make_a_complete_context(self):
        for removed in ({'inbox_offered','inbox_admitted','inbox_drained'}, {'inbox_admitted'}, {'outbound_accepted'}):
            e,_=fixture();events=[x for x in e['trace']['events'] if x['stage'] not in removed]
            ctx=only_context(r.analyze(*fixture(events)));edge=ctx['message_edges'][0]
            self.assertTrue(ctx['complete_group_chain_timing']);self.assertFalse(ctx['complete_context_timing'])
            self.assertFalse(edge['complete_local_observed_chain'])
            self.assertEqual(edge['local_chain_status'],'missing_or_repeated_stages')
            candidate=edge['spans'][0]
            self.assertEqual(candidate['status'],'matched');self.assertEqual(candidate['duration_ns'],14)
            self.assertFalse(candidate['context_complete'])

if __name__=='__main__':unittest.main(verbosity=2)
