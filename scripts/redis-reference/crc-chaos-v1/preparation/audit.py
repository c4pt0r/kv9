#!/usr/bin/env python3
"""Read-only replay of the new fixture's retained native observations and windows."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import time

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE/'vendor'));sys.path.insert(0,str(HERE/'vendor/scripts'))
from native_capture import parse_capture,PROBE as CLIENT_PROBE
from native_protocol import prefix,process_identity,state_writer
from batch_workload_report import validate
from contract import require,history_window,fresh_drained,active_fault,same_socket_generation,fault_manifest
from checker import History,verify_witness


def obj(p):return json.loads(Path(p).read_text())
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def module(name,path):
    s=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
runtime=module('owned_link_runner',HERE/'run.py')


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--artifact',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);a=parser.parse_args()
    require(sys.flags.optimize==0,'Python assertions must remain enabled for frozen observers')
    require(not a.output.exists(),'audit output must be new');p=obj(a.artifact/'executed-plan.json');summary=obj(a.artifact/'summary.json')
    require(summary['accepted'] is True and summary['cleanup']['complete'] is True,'fixture did not complete')
    rows=obj(a.artifact/'commands/commands.json');folder=a.artifact/'commands'
    def text(i):
        r=rows[i];require(type(i)is int and 0<=i<len(rows) and r['started_unix_ns']<=r['ended_unix_ns'],'invalid raw command interval')
        require((folder/r['stdout']).is_file() and (folder/r['stderr']).is_file(),'command bytes missing');return (folder/r['stdout']).read_text()
    for i,r in enumerate(rows):
        text(i)
        if r['command'][0]=='kubectl':require(r['command'][1:3]==['--kubeconfig',p['kubeconfig']],'ambient Kubernetes context used')
        if r.get('required_success',True):require(r['exit_code']==0,'required command did not succeed')
    tool_manifest=runtime.observer_tools.verify_bundle(p,HERE)
    tool_ready=obj(a.artifact/'observer-tools-ready.json')
    require(tool_ready['before_namespace_creation'] is True and tool_ready['manifest']==tool_manifest,'observer tool source binding differs')
    namespace=obj(a.artifact/'owned-namespace.json')
    creates=[i for i,row in enumerate(rows) if row['command']==['kubectl','--kubeconfig',p['kubeconfig'],'create','-f','-','-o','json'] and row['exit_code']==0 and json.loads(text(i)).get('kind')=='Namespace']
    require(len(creates)==1,'namespace creation command missing/ambiguous')
    creation=creates[0]
    for action in ('--version','save'):
        expected=['docker','exec',p['node'],*runtime.observer_tools.command(p['observer_tools']['node_root'],('ipset',action))]
        require(any(r['command']==expected and r['exit_code']==0 for r in rows[:creation]),'observer executable/protocol preflight did not precede namespace creation')
    hashes=dict(tool_manifest['ambient_dependencies'])
    hashes.update({p['observer_tools']['node_root']+'/observer-tools/'+name:record['sha256'] for name,record in tool_manifest['files'].items() if name!='__archive__'})
    for path,expected in hashes.items():
        found=[i for i,r in enumerate(rows) if r['command']==['docker','exec',p['node'],'sha256sum',path]]
        require(any(i<creation for i in found) and any(i>creation for i in found),'observer source lacked execution-bracketing readback')
        require(all(text(i).split()[0]==expected for i in found),'observer binary/library execution hash differs')
    directory=a.artifact/'retained/data-native/workload';verdict=validate(directory,Path(p['native_build_directory']),p['revision'],20)
    stored=obj(a.artifact/'native-history-check.json')
    require(stored['accepted'] is True and all(stored[k]==verdict[k] for k in ('revision','dirty','report_sha256','full_history_independently_checked','traffic_successes')),'history validator identity/accounting differs')
    report=obj(directory/'report.json');config=obj(directory/'config.json');full=prefix((directory/'history.jsonl').read_text(),config)
    require(verify_witness(History.parse([json.loads(line) for line in full['complete_text'].splitlines()]),stored['history']['witness']),'original history witness is invalid')
    captures=obj(a.artifact/'native-observations.json');identity=None
    for capture in captures:
        first,body,last=[capture[k]for k in ('before_command','capture_command','after_command')]
        command=rows[body]['command'];expected=['kubectl','--kubeconfig',p['kubeconfig'],'--request-timeout=15s','exec','-n',p['namespace'],'kv9-native-batch-client','--','/bin/bash','-c',CLIENT_PROBE,'native-probe',str(config['history_bytes']+1)]
        require(command==expected,'native probe source/command differs')
        parsed,partial=parse_capture(text(body),json.loads(text(first)),json.loads(text(last)),p,config,capture['phase'])
        require(all(parsed[k]==capture[k]for k in parsed),'derived native capture differs')
        require(parsed['cpu_allowed']==p['pod_cpus'],'native CPU mask differs')
        require(full['complete_text'].startswith(partial['complete_text']),'retained native prefix not in final history')
        identity=state_writer(parsed) if identity is None else identity;require(identity==state_writer(parsed),'native lifetime changed')
        require(parsed['process_pid']==report['process_id'],'report PID not bound to executing client')
    servers=obj(a.artifact/'server-observations.json')
    for sample in servers:
        for node,record in sample['rows'].items():
            parsed=runtime.PROBE.parse(text(record['command']),p['production_binary']['sha256'],'/usr/local/bin/kv9')
            require(all(parsed[k]==record[k]for k in parsed),'derived server identity/status differs')
            before,after=json.loads(text(record['command']-1)),json.loads(text(record['command']+1))
            require(process_identity(before)==process_identity(after) and before['metadata']['uid']==record['pod_uid'],'server Pod/container identity differs')
            require(before['metadata']['name']==record['pod'] and before['metadata']['labels']['kv9-node']==node and before['metadata']['labels']['app']=='kv9','server role differs')
            require(all(c['imageID'] in p['accepted_cri_image_ids'] and c['restartCount']==0 for c in before['status']['containerStatuses']),'server image/restart identity differs')
            runtime.FIELDS.validate(parsed,before,p['source_default_limits'])
            require(parsed['cpu_allowed']==p['pod_cpus'],'server CPU mask differs')
    batch_counters=runtime.batch_observation.check(servers,require_activity=True)
    require(batch_counters==obj(a.artifact/'batch-counter-observations.json'),'derived batch counter envelope differs')
    windows=obj(a.artifact/'windows.json');require([w['name']for w in windows]==[w['name']for w in p['windows']],'window inventory differs')
    effects={r['name']:r for r in obj(a.artifact/'packet-effects.json')};bindings=obj(a.artifact/'initial-topology.json');outcomes={}
    for i,row in enumerate(rows):
        if row['command'][4:6]==['get','services']:
            require(rows[i+1]['command'][4:6]==['get','endpointslices.discovery.k8s.io'] and rows[i+2]['command'][4:6]==['get','pods'],'topology snapshot command sequence differs')
            observed=runtime.discovery(p['namespace'],json.loads(text(i))['items'],json.loads(text(i+1))['items'],json.loads(text(i+2))['items'],config['client']['peers'])
            require(observed==bindings,'current Service/EndpointSlice bindings differ from frozen client endpoints')
    for window in windows:
        uid=obj(a.artifact/'owned-namespace.json')['metadata']['uid']
        if window['name']=='quorum-loss':expected_faults=[fault_manifest(p,uid,bindings,'partition',n)for n in (1,2,3)]
        elif window['name'].startswith('vip-'):expected_faults=[fault_manifest(p,uid,bindings,window['action'])]
        else:expected_faults=[]
        require([f['expected']for f in window['fault_before']]==expected_faults and [f['expected']for f in window['fault_after']]==expected_faults,'fault count/specification differs from planned phase')
        for before,after in zip(window['fault_before'],window['fault_after']):
            require(before['expected']==after['expected'] and before['uid']==after['uid'],'fault identity changed')
            for f in (before,after):require(active_fault(json.loads(text(f['command'])),f['expected'])==f['uid'],'raw fault identity differs')
            require(rows[before['command']]['ended_unix_ns']<=window['start_ns']<window['end_ns']<=rows[after['command']]['started_unix_ns'],'fault wall envelope differs')
        for pod,sample in window['probes'].items():
            actual=[{'target':x,'outcome':y,'latency_us':int(z)}for x,y,z in (line.split()for line in text(sample['command']).splitlines())]
            require(actual==sample['rows'],'packet probe rows differ')
        for which in ('before','after'):
            effect=effects[window['name']+'-'+which]
            for pod,snapshot in effect['pods'].items():
                for item in snapshot.values():require(text(item['command'])==item['text'],'packet effect bytes differ')
        runtime.effects_check(window['name'],bindings,effects[window['name']+'-before'],effects[window['name']+'-after'],window['probes'])
        outcomes[window['name']]=history_window(full,report,window)
        if 'reset'in window:
            r=window['reset'];same_socket_generation(r['before'],r['after'],r['old'],r['new'])
            c=rows[r['command']]['command'];expected=['ss','-4','-t','-K','state','established','src',r['old']['source_ip'],'sport','=',':'+str(r['old']['source_port']),'dst',r['old']['destination_ip'],'dport','=',':20160']
            require(c[-len(expected):]==expected,'reset command is not the exact owned IPv4 TCP tuple')
    require(outcomes==obj(a.artifact/'window-outcomes.json'),'derived window outcomes differ')
    drain=obj(a.artifact/'fresh-drain.json');require(drain['client_exit_ns']==obj(a.artifact/'client-exit.json')['observed_ns'],'drain exit boundary differs');fresh_drained(drain['samples'],drain['client_exit_ns'])
    retained=obj(a.artifact/'retained-inventory.json')
    for name,r in retained.items():
        path=a.artifact/'retained'/name;require(path.stat().st_size==r['bytes'] and sha(path)==r['sha256'],'retained data changed')
    result={'accepted':True,'scope':'Retained execution, packet effects, complete atomic native history and bounded windows; no new runtime execution','completed_ns':time.time_ns(),'revision':p['revision'],'native_writer':identity,'windows':len(windows),'history':verdict,'outcomes':outcomes,'retained_files':len(retained),'batch_counter_envelope':batch_counters}
    a.output.write_text(json.dumps(result,indent=2,sort_keys=True)+'\n');print('PASS: read-only native link/quorum evidence audit')


if __name__=='__main__':main()
