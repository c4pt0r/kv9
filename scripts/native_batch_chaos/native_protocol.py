"""Native live-prefix and exact fault predicates; no point progress is synthesized."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import json

def require(condition,message):
    if not condition:raise ValueError(message)

def fault_check(phase,faults,victim_pod=None):
    if phase.startswith("pod-failure"):
        kind, name, action = "PodChaos", "fail-leader", "pod-failure"
    elif phase.startswith("io-voter"):
        kind, name, action = "IOChaos", "raft-io-fault", "fault"
    elif phase.startswith("store-loss-voter"):
        kind, name, action = "PodChaos", "store-loss-kill", "pod-kill"
    elif phase.startswith("endpoint-migration"):
        kind, name, action = "PodChaos", "endpoint-migration-kill", "pod-kill"
    else:
        kind = "NetworkChaos"
        name, action = {"registration-seed-blackhole": ("registration-seed-blackhole", "partition"),
                        "partition": ("isolate-leader", "partition"),
                        "public-admission-overload": ("isolate-leader", "partition"),
                        "delay": ("delay-follower", "delay")}[phase]
    matches = [item for item in faults["items"] if item["kind"] == kind and item["metadata"]["name"] == name]
    require(len(matches) == 1, "fault window lacks its exact Chaos resource")
    fault = matches[0]
    require(fault["spec"]["action"] == action and not fault["metadata"].get("deletionTimestamp"), "fault was removed or has the wrong action")
    require(any(c["type"] == "AllInjected" and c["status"] == "True" for c in fault["status"]["conditions"]),
            "fault window does not contain an injected Chaos resource")
    require(isinstance(fault['metadata'].get('uid'),str) and fault['metadata']['uid'], 'fault UID missing')
    if action != 'pod-kill':
        require(not any(c['type']=='AllRecovered' and c['status']=='True' for c in fault['status']['conditions']), 'continuous fault already recovered')
    selector = fault["spec"]["selector"]
    require(selector["namespaces"] == [fault["metadata"]["namespace"]],
            "fault selector is not restricted to the owned database namespace")
    if phase.startswith(("io-voter", "store-loss-voter", "endpoint-migration")):
        require(victim_pod is not None and victim_pod["metadata"]["namespace"] == fault["metadata"]["namespace"] and
                selector["pods"] == {fault["metadata"]["namespace"]: [victim_pod["metadata"]["name"]]},
                "fault does not select the retained victim Pod")
        labels = victim_pod["metadata"]["labels"]
        if phase.startswith("io-voter"):
            require(fault["spec"]["volumePath"] == "/data" and fault["spec"]["path"] == "/data/raft/raft.log" and
                    fault["spec"]["methods"] == ["WRITE"] and fault["spec"]["percent"] == 100,
                    "I/O fault does not target the actual Raft log writes")
        else:
            expected_victim = '4' if phase.startswith('endpoint-migration') else phase.split('-')[3]
            require(labels["kv9-node"] == expected_victim and
                    any(r['id'] == fault['metadata']['namespace'] + '/' + victim_pod['metadata']['name'] and
                        r['phase'] == 'Injected' and r['injectedCount'] > 0 and
                        any(e['operation'] == 'Apply' and e['type'] == 'Succeeded' for e in r['events'])
                        for r in fault['status']['experiment']['containerRecords']),
                    'owner-death fault did not kill the original member')
    else:
        labels = selector["labelSelectors"]
    require(labels["app"] == "kv9", "fault does not select a database Pod")
    victim = labels["kv9-node"]
    require(victim in (("4",) if phase.startswith("endpoint-migration") else ("1", "2", "3")),
            "fault did not select the intended database member")
    if phase.startswith("pod-failure"):
        require(victim == phase[-1], "Pod fault selected the wrong voter")
    if phase.startswith("io-voter"):
        require(victim == phase.split("-")[2] and fault["spec"]["errno"] == int(phase.split("-")[-1]),
                "I/O fault selected the wrong voter/errno")
    return dict(phase=phase,kind=kind,resource=name,victim=victim,namespace=fault['metadata']['namespace'],uid=fault['metadata']['uid'],spec=fault['spec'])

def prefix(text,config):
    require(len(text.encode())<=config['history_bytes'],'native live history exceeds configured bound')
    end=text.rfind('\n')+1
    require(end>0,'native history has no complete header')
    complete=text[:end]
    rows=[json.loads(line) for line in complete.splitlines()]
    header=rows[0]
    require(header==dict(type='header',version=2,range_chunk_size=1024,generator='kv9-native-batch-workload',configuration=config,initial={'keyspaces':[{'name':config['keyspace_name'],'id':config['client']['keyspace_id']}],'kv':[]}),'native history header/configuration differs')
    calls={};returns={};last_time=0
    for seq,row in enumerate(rows[1:]):
        require(type(row.get('seq')) is int and row['seq']==seq,'native prefix sequence gap')
        require(type(row.get('monotonic_ns')) is int and row['monotonic_ns']>=last_time,'native prefix time regressed')
        last_time=row['monotonic_ns']
        if row['type']=='invoke':
            require(row['id'] not in calls,'native duplicate invocation');calls[row['id']]=row
        else:
            require(row['type']=='return' and row['id'] in calls and row['id'] not in returns,'native unmatched/duplicate completion');returns[row['id']]=row
    return dict(complete_text=complete,last_seq=len(rows)-2,calls=calls,returns=returns,partial_tail_bytes=len(text[end:].encode()))

def completed_batches(parsed,phase,after_seq):
    found={'batch_get':[],'batch_put':[]}
    for identity,returned in parsed['returns'].items():
        call=parsed['calls'][identity]
        if call['phase']==phase and call['seq']>after_seq and call['op'] in found and returned['outcome']=='ok':
            found[call['op']].append(dict(invocation=call,completion=returned))
    return found

def process_identity(pod):
    return pod['metadata']['uid'],sorted((r['name'],r.get('containerID'),r.get('imageID'),r.get('restartCount')) for r in pod['status']['containerStatuses'])

def state_writer(row):
    return row['pod_uid'],row['process_pid'],row['process_start_ticks'],row['process_boot_id']

def config_contract(config):
    """Exact frozen matrix choices; only generated name/keyspace/service addresses vary."""
    import ipaddress,re
    expected=dict(version=1,rpc_transport='tonic_stream',seed=40,workers=4,keys=4,batch_size=8,value_bytes=128,mix=[10,10,10,35,35],max_calls=6000,measure_ms=1800000,interval_ms=500,history_bytes=268435456)
    require(all(type(config.get(k))is type(v) and config[k]==v for k,v in expected.items()),'native matrix configuration differs from frozen contract')
    require(config['run_id']==config['keyspace_name'] and re.fullmatch(r'native-batch-[0-9]+-[0-9]+',config['run_id']) and len(config['run_id'])<=64,'native matrix keyspace identity invalid')
    client=config['client'];expected_client=dict(version=1,epoch_conf_ver=1,epoch_version=1,max_in_flight=4,max_attempts=6,deadline_ms=1500,retry_backoff_ms=10)
    require(all(type(client.get(k))is type(v) and client[k]==v for k,v in expected_client.items()),'native matrix client limits/retries differ')
    require([x['node_id']for x in client['peers']]==[1,2,3] and len({x['address']for x in client['peers']})==3,'native matrix peer set differs')
    for peer in client['peers']:
        host,port=peer['address'].rsplit(':',1);require(port=='20160' and not ipaddress.ip_address(host).is_loopback,'native client does not use ordinary Service endpoints')
