from pathlib import Path
import collections,csv,hashlib,json
root=Path('/tmp/kv9-quorum-trace-slots-root-first');prep=Path('/tmp/kv9-quorum-trace-slots-fixture-preparation-first');out=root/'reporting-first';out.mkdir()
p=prep/'results-first/analysis.json';x=json.loads(p.read_text());assert x['accepted'] and x['complete']
summary={k:v for k,v in x.items() if k!='cohorts'};summary['analysis_sha256']=hashlib.sha256(p.read_bytes()).hexdigest();summary['cohorts']=[]
span_rows=[];stage_rows=[];peer_rows=[]
for c in x['cohorts']:
 row={'arm':c['descriptor']['arm'],'descriptor':c['descriptor'],'runtime':c['runtime'],'nodes':[]};summary['cohorts'].append(row)
 if not c['quorum_trace']:continue
 for n in c['quorum_trace']['nodes']:
  a=n['analysis'];node={k:v for k,v in a.items() if k not in ('contexts','tickets')};node['node']=n['node'];node['complete_group_chains']=sum(z['complete_group_chain_timing'] for z in a['contexts']);node['complete_contexts']=sum(z['complete_context_timing'] for z in a['contexts']);row['nodes'].append(node)
  for name,v in a['stage_counts'].items():stage_rows.append({'arm':row['arm'],'node':n['node'],'stage':name,**{k:v[k] for k in ['offered','selected','recorded','contended','poisoned','full','exhausted']}})
  for name,count in a['span_accounting'].items():span_rows.append({'arm':row['arm'],'node':n['node'],'span_status':name,'count':count})
  by_peer=collections.defaultdict(list)
  for context in a['contexts']:
   for edge in context['message_edges']:
    for span in edge['spans']:
     if span['name']=='leader_dequeued_to_response_validated':
      assert span['status']=='matched' and edge['complete_local_observed_chain'] and not span['partial_observation'];by_peer[edge['to_node']].append(span['duration_ns'])
  node['leader_roundtrip_by_peer']=[]
  for peer,values in sorted(by_peer.items()):
   record={'arm':row['arm'],'leader':n['node'],'peer':peer,'count':len(values),'sum_ns':sum(values),'mean_ns':sum(values)/len(values),'min_ns':min(values),'max_ns':max(values)};peer_rows.append(record);node['leader_roundtrip_by_peer'].append(record)
summary['overhead']=[]
for before,after in [(0,1),(3,2)]:
 control=summary['cohorts'][before]['runtime']['operations'][0];trace=summary['cohorts'][after]['runtime']['operations'][0]
 summary['overhead'].append({'repeat':before//2,'qps_change_percent':100*(trace['completed_calls_per_second']/control['completed_calls_per_second']-1),'mean_change_percent':100*(trace['whole_call_latency']['mean_ns']/control['whole_call_latency']['mean_ns']-1)})
(out/'summary.json').write_text(json.dumps(summary,indent=2)+'\n')
for name,rows in [('stage-counts',stage_rows),('span-accounting',span_rows),('leader-roundtrip-by-peer',peer_rows)]:
 with (out/(name+'.csv')).open('x') as f:
  w=csv.DictWriter(f,fieldnames=list(rows[0]));w.writeheader();w.writerows(rows)
(root/'readback-first/terminal-receipt.json').write_text(json.dumps({'session_id':33748,'exit_code':0,'chunk_id':'692176'},indent=2)+'\n')
print('measured_calls',sum(c['runtime']['measured_calls'] for c in x['cohorts']));print('overhead',summary['overhead']);print('peer_roundtrips',peer_rows)
