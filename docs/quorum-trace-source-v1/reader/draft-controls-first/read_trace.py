#!/usr/bin/env python3
"""Bounded offline reader of one process-local finite quorum-trace prefix."""
import argparse
from collections import Counter,defaultdict
import hashlib,json,os,stat
from pathlib import Path

MAX_BYTES=64*1024*1024
MAX_EVENTS=65536
STAGES=('group_submit','group_admitted','group_deferred','group_refused','group_closed',
        'quorum_confirmed','completion_eligible','members_uncovered','outbound_offered',
        'outbound_accepted','outbound_rejected','outbound_dequeued','route_discarded',
        'queue_abandoned','inbound_validated','inbox_offered','inbox_admitted',
        'inbox_rejected','inbox_drained','unknown_peer','encoding_rejected','masked_outbound','driver_step')
GROUP=set(STAGES[:8])
OUTBOUND={'outbound_offered','outbound_accepted','outbound_rejected','outbound_dequeued','route_discarded'}
INBOX={'inbox_offered','inbox_admitted','inbox_rejected','inbox_drained'}
CONTINUATIONS=(OUTBOUND|INBOX|{'queue_abandoned'})-{'outbound_offered','inbox_offered'}
COUNTS=('offered','selected','recorded','contended','poisoned','full','exhausted')
LOSSES=COUNTS[3:]
IDENTITY=('node_id','process_id','exporter_created_unix_ns','process_start_ticks','boot_id')

def require(ok,message):
    if not ok:raise ValueError(message)

def fields(value,names,label):
    require(type(value)is dict and set(value)==set(names),label+' fields differ')

def uint(value,label,minimum=0,maximum=(1<<64)-1):
    require(type(value)is int and minimum<=value<=maximum,label+' integer invalid')
    return value

def histogram(values):
    """Exact integer sum/count and sparse 64-subdivision power-of-two buckets."""
    counts=Counter();total=0;low=high=None
    for value in values:
        uint(value,'duration');shift=max(0,value.bit_length()-7)
        index=value if value<64 else 64+64*shift+(value>>shift)-64
        counts[index]+=1;total+=value;low=value if low is None else min(low,value);high=value if high is None else max(high,value)
    rows=[]
    for index,count in sorted(counts.items()):
        shift=(index-64)//64 if index>=64 else 0
        lower=index if index<64 else (64+(index-64)%64)<<shift
        rows.append(dict(index=index,count=count,lower_ns=lower,upper_ns=lower+(1<<shift)-1))
    count=sum(counts.values())
    def quantile(p):
        rank=(count*p+99)//100;seen=0
        if not count:return None
        for row in rows:
            seen+=row['count']
            if seen>=rank:return {k:row[k] for k in ('lower_ns','upper_ns')}
        raise ValueError('missing histogram rank')
    return dict(count=count,sum_ns=total,min_ns=low,max_ns=high,mean_ns=total/count if count else None,
                buckets=rows,**{f'p{p}':quantile(p) for p in (50,95,99)})

def key_tuple(event):
    k=event['key'];return (bytes(k['context']).hex(),k['term'],k['from'],k['to'],k['kind'])

def validate(envelope,identity):
    fields(envelope,(*IDENTITY,'captured_unix_ns','trace'),'export envelope')
    fields(identity,IDENTITY,'expected identity')
    for name in IDENTITY:
        require(envelope[name]==identity[name] and type(envelope[name])is type(identity[name]),'export identity mismatch: '+name)
    for name in ('node_id','process_id','process_start_ticks'):uint(envelope[name],name,1)
    require(type(envelope['boot_id'])is str and 0<len(envelope['boot_id'])<=128,'boot identity missing')
    for name in ('exporter_created_unix_ns','captured_unix_ns'):
        require(type(envelope[name])is str and envelope[name].isascii() and envelope[name].isdigit() and
                0<len(envelope[name])<=39 and int(envelope[name])>0,'Unix-ns decimal string invalid: '+name)
    s=envelope['trace']
    fields(s,('schema_version','node','region','process_id','sample_every','maximum_events','captured_at_ns',
              'counters_valid','clock','scope','stage_names','stage_counts','events'),'trace')
    require(type(s['schema_version'])is int and s['schema_version']==1,'trace version differs')
    require(type(s['sample_every'])is int and s['sample_every']==256,'context sample selector differs')
    require(type(s['maximum_events'])is int and s['maximum_events']==MAX_EVENTS,'event bound differs')
    uint(s['node'],'trace node',1);uint(s['region'],'region');uint(s['process_id'],'trace PID',1,(1<<32)-1)
    require(s['node']==envelope['node_id'] and s['process_id']==envelope['process_id'],'inner exporter identity differs')
    require(s['counters_valid'] is True,'overflow/invalid counters cannot be accepted')
    require(s['clock']=='process_local_monotonic_instant','clock domain differs')
    require(type(s['scope'])is str and 0<len(s['scope'])<=4096,'missing capture scope')
    captured=uint(s['captured_at_ns'],'local capture time')
    require(s['stage_names']==list(STAGES),'exact stage-name inventory/order differs')
    require(type(s['events'])is list and len(s['events'])<=MAX_EVENTS,'event list bound')
    require(type(s['stage_counts'])is list and len(s['stage_counts'])==len(STAGES),'counter inventory differs')
    observed=Counter();kind_counts={stage:Counter() for stage in STAGES};events=[]
    for ordinal,event in enumerate(s['events']):
        fields(event,('at_ns','stage','key','local_ticket','local_route','members','index'),'event')
        uint(event['at_ns'],'event time',maximum=captured)
        stage=event['stage'];require(stage in STAGES,'unknown stage')
        k=event['key'];fields(k,('context','term','from','to','kind'),'event key')
        require(type(k['context'])is list and len(k['context'])==24,'context width differs')
        for b in k['context']:uint(b,'context byte',maximum=255)
        require(int.from_bytes(bytes(k['context'][16:]),'big')%256==0,'unselected context was recorded')
        uint(k['from'],'from');uint(k['to'],'to')
        require(k['kind'] in ('group','heartbeat','heartbeat_response'),'unknown message kind')
        if k['kind']=='group':
            require(stage in GROUP and k['term'] is None and k['from']==k['to']==s['node'],'group identity/term differs')
            uint(event['members'],'group member observation')
            require(event['local_ticket'] is None and event['local_route'] is None,'group has queue identity')
            if stage in {'quorum_confirmed','completion_eligible'}:require(event['index'] is not None,'missing read index')
            if stage in {'group_submit','group_admitted','group_deferred','group_refused'}:require(event['index'] is None,'unobserved index invented')
        else:
            require(stage not in GROUP and event['members'] is None and event['index'] is None,'message fields differ')
            uint(k['term'],'observed message term')
            if stage in OUTBOUND|{'unknown_peer','encoding_rejected','masked_outbound'}:require(k['from']==s['node'],'outbound local identity differs')
            if stage in INBOX|{'inbound_validated','driver_step'}:require(k['to']==s['node'],'inbound local identity differs')
            if stage in OUTBOUND|INBOX|{'queue_abandoned'}:require(event['local_ticket'] is not None,'missing local queue ticket')
            else:require(event['local_ticket'] is None and event['local_route'] is None,'unticketed stage has queue identity')
            if stage in OUTBOUND:require(event['local_route'] is not None,'outbound route identity missing')
            if stage in INBOX:require(event['local_route'] is None,'inbox route identity invented')
        for name in ('local_ticket','local_route'):
            if event[name] is not None:uint(event[name],name,1)
        if event['index'] is not None:uint(event['index'],'read index')
        observed[stage]+=1;kind_counts[stage][k['kind']]+=1
        events.append(dict(event,ordinal=ordinal))
    counters={};loss=Counter()
    for stage,row in zip(STAGES,s['stage_counts']):
        fields(row,COUNTS,'stage counter')
        for name,n in row.items():uint(n,'counter '+name)
        require(row['recorded']==observed[stage],'recorded/event count differs: '+stage)
        require(row['selected']==row['recorded']+sum(row[x] for x in LOSSES),'selected/lost accounting differs: '+stage)
        require(row['offered']>=row['selected'],'offered/selected accounting differs: '+stage)
        if stage in CONTINUATIONS:require(row['offered']==row['selected'],'ticket continuation sampling differs')
        counters[stage]=dict(row,recorded_by_kind={k:kind_counts[stage][k] for k in ('group','heartbeat','heartbeat_response')})
        loss.update({name:row[name] for name in LOSSES})
    require(sum(x['recorded'] for x in counters.values())==len(events),'global event accounting differs')
    return s,events,counters,dict(loss)

def analyze(envelope,identity):
    snapshot,events,counters,loss=validate(envelope,identity)
    clean=not any(loss.values());node=snapshot['node'];spans=[];span_counts=Counter();tickets=[];contexts=[]
    by_ticket=defaultdict(list);by_context=defaultdict(list)
    for e in events:
        by_context[bytes(e['key']['context']).hex()].append(e)
        if e['local_ticket'] is not None:by_ticket[e['local_ticket']].append(e)
    def span(name,a,b,*,ticket=None,context=None,local_partial=False):
        if len(a)!=1 or len(b)!=1:
            status='ambiguous' if len(a)>1 or len(b)>1 else 'unmatched'
            span_counts[name+':'+status]+=1;return dict(name=name,status=status,start_count=len(a),end_count=len(b))
        start,end=a[0],b[0]
        if end['at_ns']<start['at_ns']:
            span_counts[name+':reversed']+=1;return dict(name=name,status='reversed',event_ordinals=[start['ordinal'],end['ordinal']])
        if not clean and not local_partial:
            span_counts[name+':observation_loss']+=1;return dict(name=name,status='observation_loss')
        item=dict(name=name,status='matched',start_ns=start['at_ns'],end_ns=end['at_ns'],duration_ns=end['at_ns']-start['at_ns'],
                  event_ordinals=[start['ordinal'],end['ordinal']],timestamp_tie=end['at_ns']==start['at_ns'],
                  context=context,local_ticket=ticket,partial_observation=not clean)
        spans.append(item);span_counts[name+':matched']+=1;return item
    def unavailable(name,status):
        span_counts[name+':'+status]+=1
        return dict(name=name,status=status)
    for ticket,rows in sorted(by_ticket.items()):
        stages=defaultdict(list)
        for e in rows:stages[e['stage']].append(e)
        origin=stages.get('outbound_offered',[])+stages.get('inbox_offered',[])
        family='outbound' if stages.get('outbound_offered') else 'inbox'
        allowed=(OUTBOUND if family=='outbound' else INBOX)|{'queue_abandoned'}
        terminals={'outbound_rejected','outbound_dequeued','route_discarded','queue_abandoned'} if family=='outbound' else {'inbox_rejected','inbox_drained','queue_abandoned'}
        consistent=len(origin)==1 and len({key_tuple(e) for e in rows})==1 and len({e['local_route'] for e in rows})==1 and set(stages)<=allowed and sum(len(stages.get(s,[])) for s in terminals)<=1
        rec=dict(local_ticket=ticket,event_ordinals=[e['ordinal'] for e in rows],stage_counts=dict(Counter(e['stage'] for e in rows)),
                 status='matched_identity' if consistent else 'unmatched_or_conflicting_identity',spans=[])
        if consistent:
            rec.update(local_route=origin[0]['local_route'],key=origin[0]['key'])
            endpoints=[('outbound_offered','outbound_accepted'),('outbound_offered','outbound_dequeued'),('outbound_accepted','outbound_dequeued')] if family=='outbound' else [
                       ('inbox_offered','inbox_admitted'),('inbox_offered','inbox_drained'),('inbox_admitted','inbox_drained')]
            terminals={'outbound_rejected','route_discarded','queue_abandoned'} if family=='outbound' else {'inbox_rejected','queue_abandoned'}
            endpoints += [(origin[0]['stage'],stage) for stage in sorted(terminals) if stages.get(stage)]
            for a,b in endpoints:rec['spans'].append(span(a+'_to_'+b,stages[a],stages[b],ticket=ticket,context=key_tuple(origin[0])[0],local_partial=True))
        tickets.append(rec)
    for context,rows in sorted(by_context.items()):
        group=defaultdict(list);messages=defaultdict(list)
        for e in rows:
            if e['key']['kind']=='group':group[e['stage']].append(e)
            else:messages[key_tuple(e)].append(e)
        has_group=bool(group)
        terms=sorted({e['key']['term'] for e in rows if e['key']['kind']!='group'})
        inferred=terms[0] if len(terms)==1 and clean else None
        rec=dict(context=context,event_ordinals=[e['ordinal'] for e in rows],event_counts=dict(Counter(e['stage'] for e in rows)),
            group_event_counts={stage:len(group[stage]) for stage in STAGES if stage in GROUP},
            member_observation_sums={stage:sum(e['members'] for e in group[stage]) for stage in STAGES if stage in GROUP},
            group_term=inferred,group_term_status='uniquely_joined_observed_message_term' if inferred is not None else ('observation_loss' if not clean else 'unobserved' if not terms else 'ambiguous'),
            observed_terms=terms,group_spans=[],message_edges=[],complete_group_chain_timing=False,complete_context_timing=False)
        if has_group:
            index_consistent=not(group['quorum_confirmed'] and group['completion_eligible']) or {e['index'] for e in group['quorum_confirmed']+group['completion_eligible']}.__len__()==1
            duplicate_group=any(len(group[s])>1 for s in ('group_submit','group_admitted','quorum_confirmed','completion_eligible'))
            for a,b in [('group_submit','group_admitted'),('group_admitted','quorum_confirmed'),('quorum_confirmed','completion_eligible')]:
                if duplicate_group or not index_consistent:
                    rec['group_spans'].append(unavailable(a+'_to_'+b,'ambiguous_group_context_or_index'))
                else:rec['group_spans'].append(span(a+'_to_'+b,group[a],group[b],context=context))
            rec['completion_eligible_member_observations']=sum(e['members'] for e in group['completion_eligible'])
            rec['complete_group_chain_timing']=clean and not duplicate_group and index_consistent and all(x['status']=='matched' for x in rec['group_spans'])
        edges={key if key[4]=='heartbeat' else (key[0],key[1],key[3],key[2],'heartbeat') for key in messages}
        for key in sorted(edges):
            _,term,source,target,kind=key
            rows_for_key=messages.get(key,[])
            reverse=(context,term,target,source,'heartbeat_response')
            response=messages.get(reverse,[])
            stages=defaultdict(list);reply=defaultdict(list)
            for e in rows_for_key:stages[e['stage']].append(e)
            for e in response:reply[e['stage']].append(e)
            repeated=any(len(v)>1 for v in list(stages.values())+list(reply.values()))
            edge=dict(term=term,from_node=source,to_node=target,heartbeat_stage_counts={s:len(v) for s,v in stages.items()},
                      response_stage_counts={s:len(v) for s,v in reply.items()},repeated_context=repeated,spans=[])
            if source==node:
                pairs=[('leader_dequeued_to_response_validated',stages['outbound_dequeued'],reply['inbound_validated'])]
                required=len(stages['outbound_offered'])==len(stages['outbound_dequeued'])==len(reply['inbound_validated'])==1
            elif target==node:
                pairs=[('follower_validated_to_driver_step',stages['inbound_validated'],stages['driver_step']),
                       ('follower_driver_step_to_response_offered',stages['driver_step'],reply['outbound_offered'])]
                required=len(stages['inbound_validated'])==len(stages['driver_step'])==len(reply['outbound_offered'])==1
            else:
                pairs=[];required=False
            for name,a,b in pairs:
                if repeated or len(terms)>1:out=unavailable(name,'ambiguous_repeated_context_or_term')
                elif not clean:out=unavailable(name,'observation_loss')
                elif not required:out=unavailable(name,'unmatched')
                else:out=span(name,a,b,context=context)
                edge['spans'].append(out)
            rec['message_edges'].append(edge)
        # A group eligibility span remains local evidence, but a repeated message
        # context cannot be advertised as complete message-cycle attribution.
        rec['complete_context_timing']=rec['complete_group_chain_timing'] and inferred is not None and bool(rec['message_edges']) and all(
            not e['repeated_context'] and e['spans'] and all(s['status']=='matched' for s in e['spans']) for e in rec['message_edges'])
        contexts.append(rec)
    distributions={name:histogram(s['duration_ns'] for s in spans if s['name']==name) for name in sorted({key.split(':')[0] for key in span_counts})}
    return dict(schema_version=1,identity={n:envelope[n] for n in IDENTITY},captured_unix_ns=envelope['captured_unix_ns'],
        captured_at_ns=snapshot['captured_at_ns'],capture_is_process_shutdown=False,identity_bound=True,
        schema_and_counter_accounting_accepted=True,observation_loss=loss,observations_lost=sum(loss.values()),
        complete_context_attribution_allowed=clean,total_events=len(events),stage_counts=counters,
        group_event_count=sum(e['key']['kind']=='group' for e in events),message_event_count=sum(e['key']['kind']!='group' for e in events),
        ticket_event_count=sum(e['local_ticket'] is not None for e in events),unique_local_tickets=len(tickets),unique_contexts=len(contexts),
        storage_order_reversals=sum(b['at_ns']<a['at_ns'] for a,b in zip(events,events[1:])),
        timestamp_tie_events=sum(n-1 for n in Counter(e['at_ns'] for e in events).values()),
        tickets=tickets,contexts=contexts,span_accounting=dict(span_counts),distributions=distributions,
        scope='One process-local finite observation prefix. Offers count eligible observer invocations, not every protocol packet. Context sampling is suffix modulo256, not offered-count downsampling. Local ticket/retained route IDs never cross processes and are not wire identity. Leader dequeue-to-response intervals are unique observed candidates, not network-only RTT proof. No cross-process clock subtraction or inferred group term without a unique observed join. CompletionEligible counts selected registry members, not successful client responses. Repeated member observations must not be added across stages. Missing/ambiguous endpoints have no duration sample. Loss permits only matched ticket-local partial spans.')

def no_duplicates(pairs):
    result={}
    for key,value in pairs:
        require(key not in result,'duplicate JSON object key');result[key]=value
    return result

def read_bounded(path):
    fd=os.open(path,os.O_RDONLY|getattr(os,'O_NOFOLLOW',0))
    with os.fdopen(fd,'rb') as stream:
        before=os.fstat(stream.fileno());require(stat.S_ISREG(before.st_mode) and before.st_size<=MAX_BYTES,'input type/size bound')
        data=stream.read(MAX_BYTES+1);after=os.fstat(stream.fileno())
    require(len(data)<=MAX_BYTES and (before.st_size,before.st_mtime_ns,before.st_ino)==(after.st_size,after.st_mtime_ns,after.st_ino),'input changed or oversized')
    return json.loads(data,object_pairs_hook=no_duplicates),dict(path=str(Path(path).resolve()),bytes=len(data),sha256=hashlib.sha256(data).hexdigest())

def main():
    p=argparse.ArgumentParser(allow_abbrev=False)
    for n in ('input','identity','output'):p.add_argument('--'+n,type=Path,required=True)
    args=p.parse_args();require(not args.output.exists(),'fresh output required')
    envelope,input_pin=read_bounded(args.input);identity,identity_pin=read_bounded(args.identity)
    result=analyze(envelope,identity);result['inputs']=[input_pin,identity_pin]
    result['reader_sha256']=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    data=(json.dumps(result,sort_keys=True,separators=(',',':'))+'\n').encode()
    require(len(data)<=MAX_BYTES,'output bound')
    with args.output.open('xb') as stream:stream.write(data)
    print(json.dumps(dict(schema_and_counter_accounting_accepted=True,events=result['total_events'],lost=result['observations_lost'],output=str(args.output))))

if __name__=='__main__':main()
