#!/usr/bin/env python3
"""Owned native link/quorum fixture. A separately reviewed release file is mandatory."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tarfile
import threading
import time
import traceback

HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE/'vendor'))
sys.path.insert(0,str(HERE/'vendor/scripts'))
from native_capture import Commands
from native_protocol import prefix, state_writer, process_identity
from batch_workload_report import validate, config_check as workload_config_check
from contract import require, discovery, config_check, fault_manifest, active_fault, history_window, same_socket_generation, fresh_drained
from verify_inputs import verify
import observer_tools
import batch_observation


def load(path): return json.loads(Path(path).read_text())
def sha(path):
    with Path(path).open('rb') as f: return hashlib.file_digest(f,'sha256').hexdigest()
def save(path,value): Path(path).write_text(json.dumps(value,indent=2,sort_keys=True)+'\n')
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path); result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result);return result
PROBE=module('link_probe',HERE/'vendor/process-probe.py')
FIELDS=module('link_fields',HERE/'vendor/observer-fields.py')


class Fixture(Commands):
    def __init__(self,plan,out):
        super().__init__(out/'commands',plan,plan['run_max_seconds'])
        self.artifact=out;self.lock=threading.Lock();self.ns=plan['namespace'];self.node=plan['node'];self.node_root=plan['node_root']
        self.observer_root=plan['observer_tools']['node_root'];self.observer_created=False;self.observer_identity=None
        self.active=[];self.client=None;self.created=False;self.node_created=False;self.node_identity=None;self.native=[];self.servers=[];self.effects=[];self.windows=[];self.node_lifetimes={}

    def execute(self,argv,check=True,input=None,timeout=30):
        remaining=self.deadline-time.monotonic();require(remaining>0,'fixture global deadline expired')
        with self.lock:
            n=len(self.rows);row={'command':list(map(str,argv)),'required_success':check,'started_unix_ns':time.time_ns(),'stdout':f'command-{n:04}.stdout','stderr':f'command-{n:04}.stderr','exit_code':None};self.rows.append(row)
        try:
            r=subprocess.run(row['command'],input=input,capture_output=True,timeout=min(timeout,remaining),env=dict(os.environ,KUBECONFIG=self.plan['kubeconfig'],PYTHONDONTWRITEBYTECODE='1'))
            row['exit_code']=r.returncode;stdout,stderr=r.stdout,r.stderr
        except subprocess.TimeoutExpired as error:
            stdout,stderr=error.stdout or b'',error.stderr or b'';row['timeout']=True
        except OSError as error:
            stdout,stderr=b'',str(error).encode();row['os_error']=repr(error)
        finally: row['ended_unix_ns']=time.time_ns()
        (self.out/row['stdout']).write_bytes(stdout);(self.out/row['stderr']).write_bytes(stderr)
        with self.lock:save(self.out/'commands.json',self.rows)
        require(not check or row['exit_code']==0,'fixture command failed: '+str(row['command']))
        return row,stdout.decode()

    def run(self,*args): return self.execute(['kubectl','--kubeconfig',self.plan['kubeconfig'],'--request-timeout=15s',*args])
    def get(self,kind,name=None): return load_text(self.run('get',kind,*([name] if name else []),'-n',self.ns,'-o','json')[1])
    def pod(self,name,*args,check=True): return self.execute(['kubectl','--kubeconfig',self.plan['kubeconfig'],'--request-timeout=15s','exec','-n',self.ns,name,'--',*args],check=check)
    def host(self,*args,**kwargs):
        interactive=['-i'] if kwargs.get('input') is not None else []
        return self.execute(['docker','exec',*interactive,self.node,*args],**kwargs)
    def apply(self,obj): return self.execute(['kubectl','--kubeconfig',self.plan['kubeconfig'],'create','-f','-','-o','json'],input=json.dumps(obj).encode())
    def wait(self,fn,seconds):
        end=min(self.deadline,time.monotonic()+seconds)
        while time.monotonic()<end:
            value=fn()
            if value:return value
            time.sleep(.2)
        raise ValueError('bounded fixture predicate timed out')
    def pause(self,seconds):
        require(time.monotonic()+seconds<self.deadline,'pause would exceed fixture deadline')
        time.sleep(seconds)
    def write_node(self,name,content):
        data=content.encode()
        self.host('sh','-c','cat > "$1"','owned-write',self.node_root+'/'+name,input=data)
        command,_=self.host('cat',self.node_root+'/'+name)
        require((self.out/command['stdout']).read_bytes()==data,'owned node write readback differs: '+name)
    def phase(self,name): self.write_node('data-native/workload.phase',name+'\n')

    def topology(self):
        _,services=self.run('get','services','-n',self.ns,'-o','json')
        _,slices=self.run('get','endpointslices.discovery.k8s.io','-n',self.ns,'-o','json')
        _,pods=self.run('get','pods','-n',self.ns,'-o','json')
        services,slices,pods=map(load_text,(services,slices,pods))
        peers=[{'node_id':n,'address':next(s['spec']['clusterIP'] for s in services['items'] if s['metadata']['name']==f'voter-{n}')+':20160'} for n in (1,2,3)]
        result=discovery(self.ns,services['items'],slices['items'],pods['items'],peers)
        if hasattr(self,'bindings'):require(result==self.bindings,'current Service/EndpointSlice/Pod binding changed')
        return result,peers

    def net(self,pod,*args):
        before=self.get('pod',pod);cid=before['status']['containerStatuses'][0]['containerID'].split('://',1)[1]
        _,text=self.host('crictl','inspect',cid);pid=int(load_text(text)['info']['pid'])
        _,stat=self.host('cat',f'/proc/{pid}/stat');start=int(stat.rsplit(')',1)[1].split()[19])
        self.node_lifetimes[cid]={'container_id':cid,'pod':pod,'pod_uid':before['metadata']['uid'],'node_pid':pid,'node_start_ticks':start}
        save(self.artifact/'owned-node-lifetimes.json',self.node_lifetimes)
        if args and args[0]=='ipset':args=observer_tools.command(self.observer_root,args)
        row,text=self.host('nsenter','-t',str(pid),'-n',*args)
        after=self.get('pod',pod);require(process_identity(before)==process_identity(after),'netns container changed')
        return row,text

    def statuses(self):
        sample={'started_ns':time.time_ns(),'rows':{}}
        for node in (1,2,3):
            name=f'voter-{node}';before=self.get('pod',name)
            require(before['status']['phase']=='Running' and all(c['restartCount']==0 and c['imageID'] in self.plan['accepted_cri_image_ids'] for c in before['status']['containerStatuses']),'server image/lifetime differs')
            command,text=self.pod(name,'/bin/bash','-c',PROBE.PROBE,'link-status','/data/status','/usr/local/bin/kv9')
            row=PROBE.parse(text,self.plan['production_binary']['sha256'],'/usr/local/bin/kv9');FIELDS.validate(row,before,self.plan['source_default_limits'])
            require(row['cpu_allowed']==self.plan['pod_cpus'],'observed server CPU mask differs')
            for status in (row['state'],row['state_after']):
                integer=lambda key:int(status[key])
                require(0<=integer('public_rpc_queued')+integer('public_rpc_running')<=integer('public_rpc_in_flight')<=integer('public_rpc_peak_requests')<=integer('public_rpc_limit_requests'),'public request occupancy bound violated')
                require(0<=integer('public_rpc_encoded_bytes')<=integer('public_rpc_peak_encoded_bytes')<=integer('public_rpc_limit_encoded_bytes'),'public byte occupancy bound violated')
                require(0<=integer('raft_async_read_queued')+integer('raft_async_read_active')<=integer('raft_async_read_in_flight')<=integer('raft_async_read_peak')<=integer('raft_async_read_limit'),'async read occupancy bound violated')
                require(0<=integer('raft_async_read_active_groups')<=integer('raft_async_read_active'),'async read group bound violated')
            after=self.get('pod',name);require(process_identity(before)==process_identity(after),'server Pod changed across status')
            row.update(pod_uid=before['metadata']['uid'],pod=name,command=self.rows.index(command));sample['rows'][str(node)]=row
        sample['ended_ns']=time.time_ns();self.servers.append(sample);save(self.artifact/'server-observations.json',self.servers)
        if len(self.servers)>1:
            old=self.servers[0]['rows']
            previous=self.servers[-2]['rows']
            for n,row in sample['rows'].items():
                require(state_writer(old[n])==state_writer(row),'server runtime restarted unexpectedly')
                for key in ('metrics_export_successes','public_rpc_peak_requests','public_rpc_peak_encoded_bytes','raft_async_read_peak','raft_async_apply_peak'):
                    require(int(previous[n]['state_after'][key])<=int(row['state'][key])<=int(row['state_after'][key]),'same-lifetime exported counter regressed: '+key)
        batch_observation.check(self.servers[-2:])
        return sample

    def client_capture(self,phase):
        row,parsed=self.capture(self.ns,self.config,phase)
        require(row['cpu_allowed']==self.plan['pod_cpus'],'observed native client CPU mask differs')
        row.update(socket_inodes=re.findall(r'socket:\[(\d+)\]',self.pod('kv9-native-batch-client','ls','-l',f"/proc/{row['process_pid']}/fd")[1]))
        self.native.append(row);save(self.artifact/'native-observations.json',self.native)
        if len(self.native)>1:require(state_writer(row)==state_writer(self.native[0]),'native runtime changed')
        return row,parsed

    def effect(self,name):
        record={'name':name,'started_ns':time.time_ns(),'pods':{}}
        for pod in ('kv9-native-batch-client','control-client','voter-1','voter-2','voter-3'):
            d={}
            for key,args in [('qdisc',('tc','-s','-d','qdisc','show','dev','eth0')),('filter',('tc','-s','-d','filter','show','dev','eth0')),('iptables',('iptables-legacy-save','-c')),('ipset',('ipset','save')),('sockets',('ss','-4','-ntpie')),('snmp',('cat','/proc/net/snmp'))]:
                command,text=self.net(pod,*args);d[key]={'command':self.rows.index(command),'text':text}
            record['pods'][pod]=d
        record['ended_ns']=time.time_ns();self.effects.append(record);save(self.artifact/'packet-effects.json',self.effects);return record

    def probes(self):
        targets=[(n,x[k],k) for n,x in self.bindings.items() for k in ('vip','pod_ip')]
        script='for ip in "$@"; do for i in 1 2 3; do t=${EPOCHREALTIME/./}; if timeout 0.75 bash -c \'exec 3<>/dev/tcp/"$1"/20160\' probe "$ip" 2>/dev/null; then result=ok; else result=failed; fi; now=${EPOCHREALTIME/./}; printf "%s %s %s\\n" "$ip" "$result" "$((now-t))"; done; done'
        def one(pod):
            row,text=self.pod(pod,'/bin/bash','-c',script,'owned-probes',*[ip for _,ip,_ in targets])
            values=[{'target':a,'outcome':b,'latency_us':int(c)} for a,b,c in (line.split() for line in text.splitlines())]
            require(len(values)==18,'probe population incomplete');return pod,{'command':self.rows.index(row),'rows':values}
        with ThreadPoolExecutor(max_workers=5) as pool:return dict(pool.map(one,('kv9-native-batch-client','control-client','voter-1','voter-2','voter-3')))

    def control(self,label):
        before=self.statuses();leaders={r['state_after']['leader_id'] for r in before['rows'].values()}
        require(len(leaders)==1 and next(iter(leaders)) in ('1','2','3'),'control lacks agreed leader')
        leader=next(iter(leaders));addr=self.bindings[leader]['vip']+':20160';key=('control-'+label).encode().hex();value=label.encode().hex()
        args=['--addr',addr,'--keyspace',str(self.keyspace),'--key-hex',key]
        write=self.pod('control-client','/usr/local/bin/kv9','client','raw-put',*args,'--value-hex',value)
        read=self.pod('control-client','/usr/local/bin/kv9','client','raw-get',*args)
        require('value_hex='+value in read[1],'control read did not observe its write')
        old={n:int(r['state_after']['applied_index']) for n,r in before['rows'].items()}
        self.wait(lambda:all(int(r['state_after']['applied_index'])>old[n] for n,r in self.statuses()['rows'].items()),10)
        return {'write_command':self.rows.index(write[0]),'read_command':self.rows.index(read[0]),'old_applied':old}

    def inject(self,action,quorum=False):
        expected=[fault_manifest(self.plan,self.uid,self.bindings,action,n if quorum else None) for n in ((1,2,3) if quorum else (None,))]
        for obj in expected:
            self.apply(obj);self.active.append(obj)
        self.wait(lambda:all(any(c['type']=='AllInjected' and c['status']=='True' for c in self.get('networkchaos',o['metadata']['name']).get('status',{}).get('conditions',[])) for o in expected),20)
        return self.faults()

    def faults(self):
        records=[]
        for expected in self.active:
            row,text=self.run('get','networkchaos',expected['metadata']['name'],'-n',self.ns,'-o','json');observed=load_text(text)
            uid=active_fault(observed,expected);records.append({'expected':expected,'uid':uid,'command':self.rows.index(row),'started_ns':row['started_unix_ns'],'ended_ns':row['ended_unix_ns']})
        return records

    def recover(self):
        for obj in list(self.active):
            self.run('delete','networkchaos',obj['metadata']['name'],'-n',self.ns,'--wait=true','--timeout=20s');self.active.remove(obj)
        require(not self.get('networkchaos')['items'],'owned fault remains after recovery')

    def reset(self):
        before,_=self.client_capture('client-stream-drop');leader=self.statuses()['rows']['1']['state_after']['leader_id']
        require(leader in self.bindings,'reset lacks leader endpoint');dest=self.bindings[leader]['vip']
        source=self.get('pod','kv9-native-batch-client')['status']['podIP']
        _,text=self.net('kv9-native-batch-client','ss','-4','-ntpie','state','established','dst',dest,'dport','=',':20160')
        old=socket_tuple(text,source,dest,before['socket_inodes'])
        command,result=self.net('kv9-native-batch-client','ss','-4','-t','-K','state','established','src',source,'sport','=',':'+str(old['source_port']),'dst',dest,'dport','=',':20160')
        require(source+':'+str(old['source_port']) in result and dest+':20160' in result,'reset did not identify exact tuple')
        require(not (self.out/command['stderr']).read_text(),'reset emitted an error')
        def reconnected():
            after,_=self.client_capture('client-stream-drop')
            _,sockets=self.net('kv9-native-batch-client','ss','-4','-ntpie','state','established','dst',dest,'dport','=',':20160')
            try:new=socket_tuple(sockets,source,dest,after['socket_inodes'])
            except ValueError:return None
            if old['inode'] in after['socket_inodes']:return None
            same_socket_generation(before,after,old,new);return {'kind':'non-Chaos Linux SOCK_DESTROY','before':before,'after':after,'old':old,'new':new,'command':self.rows.index(command)}
        return self.wait(reconnected,15)


def load_text(text):return json.loads(text)


def socket_tuple(text,source,dest,owned):
    found=[]
    for line in text.splitlines():
        match=re.search(re.escape(source)+r':(\d+)\s+'+re.escape(dest)+r':20160',line);ino=re.search(r'ino:(\d+)',line)
        if match and ino and ino[1] in owned:found.append({'source_ip':source,'source_port':int(match[1]),'destination_ip':dest,'destination_port':20160,'inode':ino[1]})
    require(len(found)==1,'exact single owned socket tuple unavailable');return found[0]


def reachable_drops(text):
    rules={};policies={}
    for line in text.splitlines():
        if line.startswith(':'):
            name,policy,*_=line[1:].split();policies[name]=policy
        match=re.match(r'\[(\d+):\d+\] -A (\S+) (.*)',line)
        if match:rules.setdefault(match[2],[]).append((int(match[1]),match[3]))
    pending=['INPUT','OUTPUT','FORWARD'];seen=set();drops=[]
    while pending:
        chain=pending.pop()
        if chain in seen:continue
        seen.add(chain)
        require(policies.get(chain)!='DROP','unexpected base DROP policy')
        for count,rule in rules.get(chain,[]):
            target=re.search(r'(?:^|\s)-(?:j|g) (\S+)',rule)
            if target:
                if target[1]=='DROP':drops.append((chain,count,rule))
                elif target[1] in policies:pending.append(target[1])
    return drops


def effects_check(kind,bindings,before,after,probes):
    native='kv9-native-batch-client';vips={x['vip'] for x in bindings.values()}
    for pod,sample in probes.items():
        for row in sample['rows']:
            blocked=(kind=='vip-partition' and pod==native and row['target'] in vips)
            if kind=='quorum-loss' and pod.startswith('voter-'):
                own=bindings[pod[-1]];blocked=row['target'] not in (own['vip'],own['pod_ip'])
            delayed=kind=='vip-delay' and pod==native and row['target'] in vips
            lossy=kind=='vip-partial-loss' and pod==native and row['target'] in vips
            if blocked:require(row['outcome']=='failed','blocked path still connects')
            elif delayed:require(row['outcome']=='ok' and row['latency_us']>=180000,'configured delay not observed')
            elif not lossy:require(row['outcome']=='ok' and row['latency_us']<150000,'unaffected control/direct path unhealthy')
    selected=[native] if kind.startswith('vip-') else ['voter-1','voter-2','voter-3'] if kind=='quorum-loss' else []
    for pod in selected:
        a,b=before['pods'][pod],after['pods'][pod]
        if kind in ('vip-partition','quorum-loss'):
            x,y=reachable_drops(a['iptables']['text']),reachable_drops(b['iptables']['text'])
            require(x and y and sum(v[1] for v in y)>sum(v[1] for v in x),'selected actual DROP counters did not advance')
            targets=vips if pod==native else {ip for n,v in bindings.items() if n!=pod[-1] for ip in (v['vip'],v['pod_ip'])}
            require(all(ip in b['ipset']['text'] for ip in targets),'kernel IP set lacks actual target addresses')
        elif kind=='vip-delay':require('netem' in b['qdisc']['text'] and re.search(r'delay 250(?:\.0+)?ms',b['qdisc']['text']),'actual qdisc delay differs')
        elif kind=='vip-partial-loss':
            require('netem' in b['qdisc']['text'] and re.search(r'loss 30(?:\.0+)?%',b['qdisc']['text']),'actual qdisc is not genuine 30% partial loss')
            dropped=lambda text:sum(map(int,re.findall(r'dropped (\d+)',text)))
            require(dropped(b['qdisc']['text'])>dropped(a['qdisc']['text']),'partial loss has no actual packet drops')
            def tcp(text):
                lines=[line.split()[1:] for line in text.splitlines() if line.startswith('Tcp:')]
                require(len(lines)==2 and len(lines[0])==len(lines[1]),'TCP counter record incomplete')
                return dict(zip(lines[0],map(int,lines[1])))
            require(tcp(b['snmp']['text'])['RetransSegs']>tcp(a['snmp']['text'])['RetransSegs'],'partial loss has no TCP retransmission evidence')
    for pod in before['pods']:
        if pod in selected:continue
        require('netem' not in after['pods'][pod]['qdisc']['text'] and not reachable_drops(after['pods'][pod]['iptables']['text']),'unselected Pod has netem/reachable DROP')


def setup(f):
    p=f.plan
    _,text=f.execute([p['kind'],'get','clusters']);require(p['kind_cluster'] in text.splitlines(),'owned Kind cluster missing')
    _,text=f.run('config','current-context');require(text.strip()=='kind-'+p['kind_cluster'],'explicit kubeconfig context differs')
    _,text=f.execute(['docker','inspect',f.node]);node=load_text(text)[0]
    require(node['Config']['Labels'].get('io.x-k8s.kind.cluster')==p['kind_cluster'],'Kind node container ownership differs')
    f.run('get','crd','networkchaos.chaos-mesh.org','podnetworkchaos.chaos-mesh.org','-o','json')
    _,text=f.run('get','pods','-n','chaos-mesh','-o','json');chaos=load_text(text)['items']
    require(chaos and all(x['status']['phase']=='Running' and all(c.get('ready') is True for c in x['status'].get('containerStatuses',[])) for x in chaos),'Chaos Mesh Pods are not healthy')
    f.host('bash','-c','for tool in "$@"; do command -v "$tool" || exit 1; done','required-tools','bash','sh','timeout','tc','iptables-legacy','iptables-legacy-save','nsenter','ss','crictl','tar','sha256sum','taskset','cat','stat','mkdir','cp','rm','findmnt')
    f.host('uname','-r');f.host('iptables-legacy','--version');f.host('tc','-V');f.host('ss','--version')
    _,text=f.run('get','namespaces','-o','json');f.preserved={x['metadata']['name']:x['metadata']['uid'] for x in load_text(text)['items']}
    require(f.ns not in f.preserved and all(f.preserved.get(n)==u for n,u in p['preserve_namespaces'].items()),'namespace already exists or historical UID differs')
    save(f.artifact/'namespace-uids-before.json',f.preserved)
    require(f.host('test','-e',f.node_root,check=False)[0]['exit_code']==1,'owned node directory already exists')
    f.host('mkdir',f.node_root);f.node_created=True
    f.node_identity=f.host('stat','-c','%d:%i',f.node_root)[1].strip()
    save(f.artifact/'owned-node-root.json',{'path':f.node_root,'device_inode':f.node_identity})
    f.host('mkdir',*[f.node_root+'/data-'+n for n in ('1','2','3','native','control')])
    _,mount=f.host('findmnt','-T','/opt','-J','-o','OPTIONS')
    require('noexec' not in json.loads(mount)['filesystems'][0]['options'].split(','),'observer filesystem is noexec')
    manifest=observer_tools.stage(f,HERE)
    save(f.artifact/'observer-tools-ready.json',{'before_namespace_creation':True,'manifest':manifest,'node_root_identity':f.node_identity,'observer_root':f.observer_root,'observer_root_identity':f.observer_identity})
    ns={'apiVersion':'v1','kind':'Namespace','metadata':{'name':f.ns,'labels':{'native-link-owner':p['namespace_label']},'annotations':{'chaos-mesh.org/inject':'enabled'}}}
    _,created=f.apply(ns);f.created=True;f.uid=load_text(created)['metadata']['uid']
    _,text=f.run('get','namespace',f.ns,'-o','json');require(load_text(text)['metadata']['uid']==f.uid,'new namespace UID changed');save(f.artifact/'owned-namespace.json',load_text(text))
    for n in (1,2,3):f.apply({'apiVersion':'v1','kind':'Service','metadata':{'name':f'voter-{n}','namespace':f.ns},'spec':{'selector':{'app':'kv9','kv9-node':str(n)},'ports':[{'port':20160,'targetPort':20160,'protocol':'TCP'}]}})
    env=[{'name':k,'value':v} for k,v in {'KV9_BOOTSTRAP_TOKEN':'owned-native-link-bootstrap','KV9_CLUSTER_TOKEN':'owned-native-link-cluster','KV9_CLIENT_TOKENS':'admin=owned-native-link-client','KV9_CLIENT_TOKEN':'owned-native-link-client'}.items()]
    for name,directory,n in [(f'voter-{n}',str(n),n) for n in (1,2,3)]+[('kv9-native-batch-client','native',None),('control-client','control',None)]:
        labels={'app':'kv9','kv9-node':str(n)} if n else {'app':name}
        script=f'while [ ! -f /data/start ]; do sleep .1; done; exec taskset -c 6-15,22-31 /usr/local/bin/kv9 start --node-id {n} --addr 0.0.0.0:20160 --data-dir /data' if n else 'exec taskset -c 6-15,22-31 sleep 1800'
        mounts=[{'name':'data','mountPath':'/data'}]
        if n is None:mounts.append({'name':'data','mountPath':'/tmp'})
        obj={'apiVersion':'v1','kind':'Pod','metadata':{'name':name,'namespace':f.ns,'labels':labels},'spec':{'restartPolicy':'Never','terminationGracePeriodSeconds':3,'containers':[{'name':'fixture','image':p['image'],'imagePullPolicy':'Never','command':['/bin/bash','-c',script],'env':env,'volumeMounts':mounts}],'volumes':[{'name':'data','hostPath':{'path':f.node_root+'/data-'+directory,'type':'Directory'}}]}}
        f.apply(obj)
    f.run('wait','--for=condition=Ready','pod','--all','-n',f.ns,'--timeout=60s')
    f.bindings,peers=f.topology();save(f.artifact/'initial-topology.json',f.bindings)
    incarnations=[]
    for n in (1,2,3):
        _,text=f.pod(f'voter-{n}','/usr/local/bin/kv9','store-prepare','--node-id',str(n),'--data-dir','/data');fields=dict(x.split('=',1) for x in text.split());incarnations.append(f'{n}='+fields['store_incarnation'])
    voter_text=','.join(str(v['node_id'])+'@'+v['address'] for v in peers)
    # The exact image CLI runs in the owned control Pod; no rebuilt host executable.
    f.pod('control-client','/usr/local/bin/kv9','root-create','--output','/tmp/root.bin','--voters',voter_text,'--store-incarnations',','.join(incarnations))
    for n in (1,2,3):
        f.host('cp',f.node_root+'/data-control/root.bin',f.node_root+f'/data-{n}/root.bin')
        f.pod(f'voter-{n}','/usr/local/bin/kv9','init','--root','/data/root.bin','--node-id',str(n),'--data-dir','/data');f.pod(f'voter-{n}','touch','/data/start')
    def serving():
        leaders=set()
        for n in (1,2,3):
            row,text=f.pod(f'voter-{n}','cat','/data/status',check=False)
            if row['exit_code']!=0 or 'bootstrap_state=Serving' not in text:return None
            state=dict(line.split('=',1) for line in text.splitlines() if '=' in line);leaders.add(state.get('leader_id'))
        return len(leaders)==1 and next(iter(leaders)) in ('1','2','3')
    f.wait(serving,60);states=f.statuses();leader=states['rows']['1']['state_after']['leader_id'];require(leader in f.bindings,'startup leader unavailable')
    run_id=f'native-link-{int(time.time())}-{os.getpid()}'
    _,text=f.pod('control-client','/usr/local/bin/kv9','client','create-keyspace','--addr',f.bindings[leader]['vip']+':20160','--name',run_id,'--api-type','raw')
    f.keyspace=int(dict(x.split('=',1) for x in text.splitlines())['keyspace_id'])
    f.config=json.loads(json.dumps(p['config_template']));f.config.update(run_id=run_id,keyspace_name=run_id);f.config['client'].update(peers=peers,keyspace_id=f.keyspace)
    config_check(f.config,p['config_template'],f.bindings);workload_config_check(f.config);save(f.artifact/'native-config.json',f.config);f.write_node('data-native/input.json',json.dumps(f.config));f.phase('baseline')
    _,mounted=f.pod('kv9-native-batch-client','cat','/tmp/input.json');require(mounted.encode()==json.dumps(f.config).encode(),'mounted client input differs from prepared configuration')
    script='set +e; taskset -c 6-15,22-31 /usr/local/bin/kv9-batch-workload --config /tmp/input.json --build-manifest /opt/kv9-batch-workload/build.json --output /tmp/workload --stop-file /tmp/workload.stop --phase-file /tmp/workload.phase; rc=$?; printf "%s\\n" "$rc" > /tmp/workload.exit; exit "$rc"'
    command=['kubectl','--kubeconfig',p['kubeconfig'],'exec','-n',f.ns,'kv9-native-batch-client','--','/bin/bash','-c',script]
    f.client_log=(f.artifact/'native-client.log').open('xb');f.client=subprocess.Popen(command,stdout=f.client_log,stderr=subprocess.STDOUT,env=dict(os.environ,KUBECONFIG=p['kubeconfig']))
    save(f.artifact/'native-launch.json',{'command':command,'host_pid':f.client.pid,'started_ns':time.time_ns()})
    def ready():
        require(f.client.poll() is None,'native client exited before ready; inspect native-client.log')
        return f.pod('kv9-native-batch-client','test','-e','/tmp/workload/ready.json',check=False)[0]['exit_code']==0
    f.wait(ready,30)


def traffic(f):
    for selected in f.plan['windows']:
        phase=selected['phase'];name=selected['name'];record=dict(selected);f.phase(phase);f.topology()
        if name in ('vip-delay','vip-partial-loss','vip-partition','quorum-loss'):
            f.inject('partition' if name=='quorum-loss' else selected['action'],name=='quorum-loss')
        elif name=='exact-tcp-reset':record['reset']=f.reset()
        before=f.effect(name+'-before')
        f.client_capture(phase)
        if selected['expect']=='no-new-success':f.pause(f.plan['quiesce_seconds'])
        record['fault_before']=f.faults();record['start_ns']=time.time_ns()
        # Independent simultaneous control probes run inside the same fault interval.
        record['probes']=f.probes()
        if name!='quorum-loss':record['control']=f.control(name)
        seconds=f.plan['negative_seconds'] if selected['expect']=='no-new-success' else f.plan['partial_loss_seconds'] if name=='vip-partial-loss' else f.plan['fault_seconds'] if f.active else f.plan['progress_seconds']
        elapsed=(time.time_ns()-record['start_ns'])/1e9
        if elapsed<seconds:f.pause(seconds-elapsed)
        f.client_capture(phase);after=f.effect(name+'-after');record['end_ns']=time.time_ns();record['fault_after']=f.faults()
        require(record['end_ns']-record['start_ns']<=60_000_000_000,'window exceeded its declared 60-second observation bound')
        require([(x['uid'],x['expected']) for x in record['fault_before']]==[(x['uid'],x['expected']) for x in record['fault_after']],'fault identity changed across window')
        effects_check(name,f.bindings,before,after,record['probes'])
        f.topology();f.statuses();f.windows.append(record);save(f.artifact/'windows.json',f.windows)
        f.recover()
        print('RETAINED: '+name,flush=True)


def stop_client(f):
    if f.client is None:return
    f.pod('kv9-native-batch-client','touch','/tmp/workload.stop',check=False)
    code=f.client.wait(timeout=30);f.client_exit_ns=time.time_ns();f.client_log.close()
    row,text=f.pod('kv9-native-batch-client','cat','/tmp/workload.exit');require(code==0 and text.strip()=='0','native collector failed')
    pid=f.native[0]['process_pid'];row,_=f.pod('kv9-native-batch-client','test','-e',f'/proc/{pid}',check=False);require(row['exit_code']==1,'native workload PID still exists')
    save(f.artifact/'client-exit.json',{'exit_code':code,'observed_ns':f.client_exit_ns,'process_pid':pid,'pod_process_absent':True})


def retain_data(f):
    archive=f.artifact/'owned-data.tar';command=['docker','exec',f.node,'tar','-C',f.node_root,'-cf','-','data-1','data-2','data-3','data-native','data-control']
    with archive.open('xb') as target:r=subprocess.run(command,stdout=target,stderr=subprocess.PIPE,timeout=40)
    (f.artifact/'archive.stderr').write_bytes(r.stderr);require(r.returncode==0,'archive failed')
    with tarfile.open(archive) as tar:tar.extractall(f.artifact/'retained',filter='data')
    inventory={}
    for path in sorted((f.artifact/'retained').rglob('*')):
        if path.is_file():inventory[str(path.relative_to(f.artifact/'retained'))]={'bytes':path.stat().st_size,'sha256':sha(path)}
    with tarfile.open(archive) as tar:
        for member in tar:
            if member.isfile():
                require(hashlib.file_digest(tar.extractfile(member),'sha256').hexdigest()==inventory[member.name]['sha256'],'archive readback differs')
    save(f.artifact/'retained-inventory.json',inventory)
    return f.artifact/'retained/data-native/workload'


def remove_node_root(f):
    require(f.node_created and f.node_identity and f.host('stat','-c','%d:%i',f.node_root)[1].strip()==f.node_identity,'owned node directory identity changed')
    observer_tools.cleanup(f)
    f.host('rm','-rf','--',f.node_root)
    require(f.host('test','-e',f.node_root,check=False)[0]['exit_code']==1,'owned node data remains')


def cleanup(f):
    report={'complete':False}
    try:
        f.deadline=max(f.deadline,time.monotonic()+180)
        if not f.created:
            if f.node_created:
                _,text=f.run('get','namespaces','-o','json');actual={x['metadata']['name']:x['metadata']['uid'] for x in load_text(text)['items']}
                require(f.ns not in actual and all(actual.get(n)==u for n,u in f.preserved.items()),'pre-namespace cleanup identity uncertainty')
                remove_node_root(f)
            report.update(complete=True,no_namespace_created=True,owned_node_removed=f.node_created)
            return
        _,text=f.run('get','namespace',f.ns,'-o','json');require(load_text(text)['metadata']['uid']==f.uid,'refuse cleanup of replaced namespace')
        f.recover()
        if f.client and f.client.poll() is None:
            f.pod('kv9-native-batch-client','touch','/tmp/workload.stop',check=False);f.client.wait(timeout=30);f.client_log.close()
        pods=f.get('pods');save(f.artifact/'pods-before-cleanup.json',pods)
        f.run('delete','pod','--all','-n',f.ns,'--wait=true','--timeout=30s');require(not f.get('pods')['items'],'owned Pods remain')
        for record in f.node_lifetimes.values():
            row,stat=f.host('cat',f"/proc/{record['node_pid']}/stat",check=False)
            require(row['exit_code']!=0 or int(stat.rsplit(')',1)[1].split()[19])!=record['node_start_ticks'],'owned node process still exists after Pod cleanup')
        if not (f.artifact/'owned-data.tar').exists():retain_data(f)
        # Compare original stable node bytes before deleting only this owned path.
        inventory=load(f.artifact/'retained-inventory.json')
        for name,record in inventory.items():require(f.host('sha256sum',f.node_root+'/'+name)[1].split()[0]==record['sha256'],'node/archive file differs')
        f.run('delete','namespace',f.ns,'--wait=true','--timeout=30s')
        _,text=f.run('get','namespaces','-o','json');actual={x['metadata']['name']:x['metadata']['uid'] for x in load_text(text)['items']}
        require(f.ns not in actual and all(actual.get(n)==u for n,u in f.preserved.items()),'historical namespace UID changed')
        remove_node_root(f)
        report.update(complete=True,namespace_absent=True,preserved=f.preserved,namespace_uid=f.uid)
    except BaseException as error:report.update(failure=str(error),traceback=traceback.format_exc())
    finally:save(f.artifact/'cleanup.json',report)


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--plan',type=Path,default=HERE/'plan.json');parser.add_argument('--release',type=Path,required=True);parser.add_argument('--artifact',type=Path,required=True);args=parser.parse_args()
    p=load(args.plan);release=load(args.release)
    require(sys.flags.optimize==0,'Python assertions must remain enabled for frozen observers')
    require(release.get('authorized') is True and release.get('plan_sha256')==sha(args.plan) and release.get('namespace')==p['namespace'],'separate exact-plan launch release required')
    require(release.get('other_matrix_terminal') is True and release.get('no_timing_overlap') is True,'quiet-window coordination release required')
    require(release.get('artifact')==str(args.artifact),'release must name this exact new artifact directory')
    require(p['node']==p['kind_cluster']+'-control-plane' and p['namespace'].startswith('kv9-native-link-acceptance-') and p['node_root']=='/tmp/'+p['namespace'],'owned node/namespace/path boundary differs')
    require(p['observer_tools']['node_root']=='/opt/'+p['namespace']+'-observer','owned observer directory boundary differs')
    require(p['host_cpus']==p['pod_cpus']=='6-15,22-31' and set(os.sched_getaffinity(0))==set(range(6,16))|set(range(22,32)),'host and planned Pod affinity must be exactly 6-15,22-31')
    require(not args.artifact.exists() and args.artifact.is_absolute() and str(args.artifact).startswith('/tmp/kv9-native-link-acceptance-'),'new owned artifact path required')
    require(sha(p['source_plan'])==p['source_plan_sha256'],'retained source plan changed')
    base=load(p['source_plan']);tree=Path(p['worktree'])
    require(subprocess.check_output(['git','-C',str(tree),'rev-parse','HEAD'],text=True).strip()==p['revision'],'source revision differs')
    require(not subprocess.check_output(['git','-C',str(tree),'status','--porcelain']),'source worktree dirty')
    for name,expected in base['source'].items():require(sha(tree/name)==expected,'source bytes differ: '+name)
    require(sha(p['production_binary']['path'])==p['production_binary']['sha256'],'server bytes differ')
    require(load(Path(p['native_build_directory'])/'build.json')==p['native_client_build'],'native build differs')
    require(sha(Path(p['native_build_directory'])/'kv9-batch-workload')==p['native_client_build']['binary_sha256'],'native client bytes differ')
    for name,record in load(HERE/'vendor-provenance.json').items():require(sha(HERE/name)==record['sha256'],'frozen vendor helper differs')
    frozen=load(HERE/'frozen-inputs.json')
    for name,expected in frozen.items():require(sha(HERE/name)==expected,'prepared helper differs: '+name)
    binding=verify(p);observer_tools.verify_bundle(p,HERE)
    used=HERE/('release-used-'+sha(args.release)+'.json')
    with used.open('x') as handle:json.dump({'release':str(args.release),'artifact':str(args.artifact),'started_ns':time.time_ns()},handle)
    args.artifact.mkdir();save(args.artifact/'executed-plan.json',p);save(args.artifact/'launch-release.json',release);save(args.artifact/'input-verification.json',binding)
    f=Fixture(p,args.artifact);summary={'accepted':False}
    try:
        setup(f);traffic(f);stop_client(f)
        samples=[f.statuses()];deadline=time.monotonic()+p['drain_seconds']
        while time.monotonic()<deadline:
            samples.append(f.statuses())
            try:fresh_drained(samples,f.client_exit_ns);break
            except ValueError:f.pause(.2)
        fresh_drained(samples,f.client_exit_ns);save(args.artifact/'fresh-drain.json',{'client_exit_ns':f.client_exit_ns,'samples':samples})
        save(args.artifact/'batch-counter-observations.json',batch_observation.check(f.servers,require_activity=True))
        # Stop replica writers before stable store retention, after the fresh drain evidence.
        f.run('delete','pod','voter-1','voter-2','voter-3','-n',f.ns,'--wait=true','--timeout=30s')
        workload=retain_data(f);verdict=validate(workload,Path(p['native_build_directory']),p['revision'],20)
        report=load(workload/'report.json');require(report['stop']['reason']=='stop_file','collector hit its finite cap/duration')
        full=prefix((workload/'history.jsonl').read_text(),f.config)
        results={w['name']:history_window(full,report,w) for w in f.windows}
        require(len(results)==len(p['windows']),'window population incomplete')
        save(args.artifact/'native-history-check.json',verdict);save(args.artifact/'window-outcomes.json',results)
        summary.update(accepted=True,windows=len(results),source_revision=p['revision'])
    except BaseException as error:summary.update(failure=str(error),traceback=traceback.format_exc());print(summary['traceback'],flush=True)
    finally:
        cleanup(f);summary['cleanup']=load(args.artifact/'cleanup.json');summary['accepted']=summary['accepted'] and summary['cleanup']['complete'];save(args.artifact/'summary.json',summary)
    require(summary['accepted'],'owned acceptance failed; all partial evidence retained')


if __name__=='__main__':main()
