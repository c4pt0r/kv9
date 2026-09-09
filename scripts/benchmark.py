#!/usr/bin/env python3
"""Run owned, repeated single-host trials; retain every attempted trial and its raw evidence."""
from root_provision import prepare_stores
import argparse
import json
import hashlib
import os
from pathlib import Path
import platform
import resource
import secrets
import socket
import subprocess
import time
import urllib.request

from workload_report import bounded, strict_json, validate

MINIO = 'quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e'
MIXES = {'read': dict(get=100, put=0, delete=0), 'write': dict(get=0, put=100, delete=0),
         'mixed': dict(get=50, put=40, delete=10)}


def save(path, value):
    text=json.dumps(value, indent=2, sort_keys=True)+'\n'
    if len(text.encode())>16*1024*1024: raise ValueError('benchmark artifact exceeds its 16 MiB bound')
    path.write_text(text)


def read(path, size=2*1024*1024):
    return strict_json(bounded(path, size))


def process_sample(pid):
    root = Path('/proc')/str(pid)
    # /proc stat comm may contain whitespace and parentheses.
    fields = (root/'stat').read_text().rsplit(')', 1)[1].split()
    status = dict(line.split(':', 1) for line in (root/'status').read_text().splitlines())
    io = dict((key, int(value)) for key, value in (line.split(':',1) for line in (root/'io').read_text().splitlines()))
    return dict(pid=pid, start_ticks=int(fields[19]), user_ticks=int(fields[11]), system_ticks=int(fields[12]),
                rss_bytes=int(status['VmRSS'].split()[0])*1024, peak_rss_bytes=int(status['VmHWM'].split()[0])*1024,
                io=io, cpu_affinity=sorted(os.sched_getaffinity(pid)), observed_unix_ns=time.time_ns(), observed_monotonic_ns=time.monotonic_ns())


def host(out, measurement):
    commands = {'cpu': ['lscpu', '--json'], 'block_devices': ['lsblk', '--json', '-o', 'NAME,TYPE,SIZE,ROTA,MODEL'],
                'filesystem': ['findmnt', '--json', '--target', str(out)], 'docker': ['docker', 'version', '--format', '{{json .Server}}']}
    result = dict(version=1, system=platform.system(), release=platform.release(), machine=platform.machine(),
                  python=platform.python_version(), cpu_affinity=sorted(os.sched_getaffinity(0)),
                  cpu_clock_ticks_per_second=os.sysconf('SC_CLK_TCK'), page_size=os.sysconf('SC_PAGE_SIZE'),
                  file_limit=list(resource.getrlimit(resource.RLIMIT_NOFILE)), network='127.0.0.1 loopback; no inter-host links')
    for name, command in commands.items():
        r=subprocess.run(command, text=True, capture_output=True, timeout=10)
        result[name]=dict(command=command, exit_code=r.returncode, stdout=r.stdout, stderr=r.stderr)
        if r.returncode: raise ValueError('required host inventory command failed: '+name)
    for name in ('meminfo', 'pressure/cpu', 'pressure/io', 'pressure/memory', 'self/cgroup'):
        result[name]=Path('/proc',name).read_text()
    allowed=result['cpu_affinity']
    if measurement and len(allowed)<4: raise ValueError('measurement requires at least four allowed CPUs')
    placement=dict(client_cpus=allowed[:2],server_cpus=allowed[2:6] if len(allowed)>=4 else allowed,
                   separated=len(allowed)>=4,exclusive_host=False)
    result['placement']=placement
    save(out/'host.json', result)
    return placement


class Fixture:
    def __init__(self, out, target, build, placement):
        self.out, self.target, self.build = out, target, build
        self.placement=placement
        self.identities={}
        self.children=[]
        out.mkdir()
        self.env={k:v for k,v in os.environ.items() if not k.startswith('KV9_')}
        self.env.update(KV9_CLIENT_TOKEN=secrets.token_hex(24), KV9_BOOTSTRAP_TOKEN=secrets.token_hex(24),
                        KV9_CLUSTER_TOKEN=secrets.token_hex(24), KV9_PUBLIC_MAX_REQUESTS='64',
                        KV9_PUBLIC_MAX_ENCODED_BYTES='16777216', KV9_FLUSH_INTERVAL_MS='100')
        self.env['KV9_CLIENT_TOKENS']='benchmark='+self.env['KV9_CLIENT_TOKEN']
        self.nodes, self.workloads, self.logs, self.sockets = {}, [], [], []
        self.container='kv9-benchmark-'+secrets.token_hex(6)
        self.container_created=False
        self.secret_file=out/'minio.env'
        self.command_index=0
        self.record=dict(version=1, target=target, addresses={}, trials=[], complete=False,
                         placement=placement, server_settings=dict(public_max_requests=64, public_max_encoded_bytes=16777216, flush_interval_ms=100),
                         topology='single-process loopback calibration' if target=='loopback' else 'three voters on one host')

    def command(self, command, timeout=30):
        command=list(map(str,command))
        r=subprocess.run(command, env=self.env, text=True, capture_output=True, timeout=timeout)
        # Commands never contain credentials; Docker receives its one secret via environment.
        save(self.out/f'command-{self.command_index:03}.json',dict(command=command,exit_code=r.returncode,stdout=r.stdout,stderr=r.stderr))
        self.command_index+=1
        if r.returncode: raise ValueError('fixture command failed; see retained command evidence')
        return r.stdout.strip()

    def wait(self, label, condition, seconds=45):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            if any(p.poll() is not None for p in self.nodes.values()): raise ValueError('fixture process exited: '+label)
            value=condition()
            if value: return value
            time.sleep(0.02)
        raise ValueError('timed out: '+label)

    def state(self, node):
        try:
            fields=dict(line.split('=',1) for line in (self.out/'data'/f'n{node}'/'status').read_text().splitlines())
            return fields if int(fields['pid'])==self.nodes[node].pid else {}
        except (OSError, KeyError, ValueError): return {}

    def leader(self):
        states={n:self.state(n) for n in self.nodes}
        if any(s.get('bootstrap_state')!='Serving' or s.get('fatal') for s in states.values()): return None
        leaders={s.get('leader_id') for s in states.values()}
        if len(leaders)!=1 or leaders & {None,'0'}: return None
        n=int(leaders.pop())
        return n if n in states and states[n].get('role')=='leader' else None

    def launch(self, command, logfile, client=False):
        log=logfile.open('w');self.logs.append(log)
        # This single-threaded harness sets affinity before spawn, then restores
        # its own mask. The child inherits the mask before executing any code.
        original=os.sched_getaffinity(0)
        try:
            os.sched_setaffinity(0,self.placement['client_cpus' if client else 'server_cpus'])
            process=subprocess.Popen(list(map(str,command)),env=self.env,stdout=log,stderr=log)
            self.children.append(process)
        finally:
            os.sched_setaffinity(0,original)
        identity=process_sample(process.pid)
        with Path('/proc',str(process.pid),'exe').open('rb') as binary:
            identity['executable_sha256']=hashlib.file_digest(binary,'sha256').hexdigest()
        self.identities[process.pid]=identity
        return process

    def start(self):
        if self.target=='loopback':
            p=self.launch([self.build/'workload-loopback',self.out/'ready.json',self.out/'stop',self.out/'loopback.json'],self.out/'loopback.log')
            self.nodes[1]=p
            self.wait('calibration listener',lambda:(self.out/'ready.json').exists())
            ready=read(self.out/'ready.json')
            if ready['pid']!=p.pid or ready['durability'] is not False: raise ValueError('invalid calibration identity')
            self.addresses={1:ready['address']}
        else:
            if self.target=='minio': self.start_minio()
            for _ in range(3):
                s=socket.socket();s.bind(('127.0.0.1',0));self.sockets.append(s)
            self.addresses={i:f'127.0.0.1:{s.getsockname()[1]}' for i,s in enumerate(self.sockets,1)}
            prepared = prepare_stores(self.command, self.build/'kv9', {n: self.out/'data'/f'n{n}' for n in self.addresses})
            self.command([self.build/'kv9','root-create','--output',self.out/'root.bin','--voters',','.join(f'{n}@{a}' for n,a in self.addresses.items()),'--store-incarnations',prepared])
            for s in self.sockets: s.close()
            for n,a in self.addresses.items():
                directory=self.out/'data'/f'n{n}'
                self.command([self.build/'kv9','init','--root',self.out/'root.bin','--node-id',n,'--data-dir',directory])
                self.nodes[n]=self.launch([self.build/'kv9','start','--node-id',n,'--addr',a,'--data-dir',directory],self.out/f'n{n}.log')
            self.wait('agreed serving leader',self.leader)
        self.record['addresses']={str(n):a for n,a in self.addresses.items()}
        self.record['process_ids']={str(n):p.pid for n,p in self.nodes.items()}
        self.record['process_identities']={str(n):self.identities[p.pid] for n,p in self.nodes.items()}
        save(self.out/'fixture.json',self.record)

    def start_minio(self):
        access='kv9'+secrets.token_hex(8);secret=secrets.token_hex(24)
        self.env.update(KV9_STORAGE='minio', KV9_OBJECT_STORE_BUCKET='kv9-benchmark',
                        KV9_OBJECT_STORE_ACCESS_KEY=access, KV9_OBJECT_STORE_SECRET_KEY=secret)
        with open(self.secret_file,'x',opener=lambda p,f:os.open(p,f,0o600)) as stream:
            stream.write('MINIO_ROOT_USER='+access+'\nMINIO_ROOT_PASSWORD='+secret+'\n')
        self.command(['docker','run','-d','--name',self.container,'--cpuset-cpus',','.join(map(str,self.placement['server_cpus'])),'-p','127.0.0.1::9000','--env-file',self.secret_file,MINIO,'server','/data'])
        self.container_created=True
        endpoint='http://'+self.command(['docker','port',self.container,'9000/tcp'])
        self.env['KV9_OBJECT_STORE_ENDPOINT']=endpoint
        def healthy():
            try:
                with urllib.request.urlopen(endpoint+'/minio/health/live',timeout=1) as r: return r.status==200
            except OSError: return False
        self.wait('owned MinIO health',healthy)
        self.env['MC_HOST_benchmark']=f'http://{access}:{secret}@127.0.0.1:9000'
        self.command(['docker','exec','-e','MC_HOST_benchmark',self.container,'mc','mb','benchmark/kv9-benchmark'])
        del self.env['MC_HOST_benchmark'];self.secret_file.unlink()
        self.record['object_store']=dict(image=MINIO, endpoint=endpoint,
            image_id=self.command(['docker','inspect','--format','{{.Image}}',self.container]),
            version=self.command(['docker','exec',self.container,'minio','--version']),
            cpu_affinity=self.command(['docker','inspect','--format','{{.HostConfig.CpusetCpus}}',self.container]))

    def snapshot(self, directory, phase):
        started=time.time_ns()
        if self.target!='loopback':
            def fresh():
                docs={}
                for n in self.nodes:
                    try: doc=read(self.out/'data'/f'n{n}'/'metrics.json',512*1024)
                    except (OSError,ValueError): return None
                    if int(doc['captured_unix_ns'])<started or doc['process_id']!=self.nodes[n].pid: return None
                    docs[str(n)]=doc
                return docs
            metrics=self.wait('fresh server metrics',fresh,10)
            save(directory/(phase+'-metrics.json'),metrics)
            save(directory/(phase+'-status.json'),{str(n):self.state(n) for n in self.nodes})
        save(directory/(phase+'-resources.json'),dict(requested_unix_ns=started,
             processes={str(n):process_sample(p.pid) for n,p in self.nodes.items()}))

    def trial(self, name, mix, concurrency, repeat, protocol, guard=False):
        directory=self.out/name;directory.mkdir()
        entry=dict(name=name,mix=mix,workers=concurrency,repeat=repeat,guard=guard,complete=False)
        self.record['trials'].append(entry);save(self.out/'fixture.json',self.record)
        if self.target=='loopback': keyspace=1
        else:
            serving=self.wait('serving before trial setup',self.leader)
            receipt=self.command([self.build/'kv9','client','create-keyspace','--addr',self.addresses[serving],
                                 '--name',name,'--api-type','raw'])
            (directory/'create-keyspace.out').write_text(receipt+'\n')
            keyspace=int(dict(line.split('=',1) for line in receipt.splitlines())['keyspace_id'])
        c=dict(version=1,client=dict(version=1,peers=[dict(node_id=n,address=a) for n,a in self.addresses.items()],
               keyspace_id=keyspace,epoch_conf_ver=1,epoch_version=1,max_in_flight=concurrency,max_attempts=6,deadline_ms=1500,retry_backoff_ms=5),
               mode='correctness' if guard else 'performance',run_id=name,keyspace_name=name,seed=40+repeat,
               workers=concurrency,keys=8,value_bytes=128,mix=MIXES[mix],warmup_operations=32,
               max_operations=180 if guard else 1_000_000,measure_ms=2000 if guard else protocol['measure_ms'],
               interval_ms=2 if guard else 0,history_bytes=64*1024*1024 if guard else 0)
        save(directory/'requested-config.json',c)
        self.snapshot(directory,'before')
        p=self.launch([self.build/'workload/kv9-workload','--config',directory/'requested-config.json',
                       '--build-manifest',self.build/'workload/build.json','--output',directory/'run'],directory/'workload.log',client=True)
        self.workloads.append(p)
        entry['client_identity']=self.identities[p.pid]
        save(self.out/'fixture.json',self.record)
        code=p.wait(timeout=protocol['measure_ms']/1000+360)
        entry['exit_code']=code;save(self.out/'fixture.json',self.record)
        self.snapshot(directory,'after')
        if code: raise ValueError('workload failed: '+name)
        result=validate(directory/'run',self.build/'workload',seconds=60)
        save(directory/'checked.json',result)
        report=read(directory/'run/report.json')
        if not guard and report['stop']['reason']!='duration': raise ValueError('performance trial did not finish its duration: '+name)
        entry.update(complete=True,report_sha256=result['report_sha256'])
        save(self.out/'fixture.json',self.record)
        print('PASS: benchmark trial '+self.target+'/'+name,flush=True)

    def finish(self):
        if self.target=='loopback':
            (self.out/'stop').touch()
            if self.nodes[1].wait(timeout=10): raise ValueError('calibration endpoint failed to drain')
        elif self.target=='minio':
            self.wait('remote checkpoint on each voter',lambda:all((self.out/'data'/f'n{n}'/'catalog.checkpoint').exists() for n in self.nodes))
            self.record['remote_checkpoints']={}
            for n in self.nodes:
                data=bounded(self.out/'data'/f'n{n}'/'catalog.checkpoint',48*1024*1024)
                (self.out/f'n{n}-checkpoint.bin').write_bytes(data)
                self.record['remote_checkpoints'][str(n)]=dict(bytes=len(data),sha256=hashlib.sha256(data).hexdigest())
        self.record['complete']=True;save(self.out/'fixture.json',self.record)

    def close(self):
        for p in self.children:
            if p.poll() is None: p.kill()
            p.wait(timeout=10)
        for log in self.logs: log.close()
        for s in self.sockets: s.close()
        self.secret_file.unlink(missing_ok=True)
        if self.container_created:
            subprocess.run(['docker','rm','-f',self.container],stdout=subprocess.DEVNULL,timeout=30,check=True)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--build',type=Path,required=True)
    parser.add_argument('--smoke',action='store_true',help='short functional matrix; never accepted as a measured baseline')
    parser.add_argument('--expected-revision')
    args=parser.parse_args();out=args.output.resolve();build=args.build.resolve()
    out.mkdir(parents=True,exist_ok=False)
    protocol=dict(version=1,kind='smoke' if args.smoke else 'measurement',repetitions=2 if args.smoke else 3,
                  measure_ms=500 if args.smoke else 5000,targets=['wal','minio','loopback'],mixes=MIXES,
                  concurrency=[1,4],keys=8,value_bytes=128,warmup_operations=32,seed_base=40)
    save(out/'protocol.json',protocol)
    from benchmark_report import check_build, validate_matrix
    check_build(build,args.expected_revision,release=not args.smoke)
    placement=host(out,not args.smoke)
    index=dict(version=1,complete=False,fixtures=[],protocol_sha256=hashlib.sha256((out/'protocol.json').read_bytes()).hexdigest())
    save(out/'index.json',index)
    try:
        for repeat in range(protocol['repetitions']):
            order=protocol['targets'] if repeat%2==0 else list(reversed(protocol['targets']))
            for target in order:
                name=f'r{repeat}-{target}';fixture=Fixture(out/name,target,build,placement)
                index['fixtures'].append(name);save(out/'index.json',index)
                try:
                    fixture.start()
                    fixture.trial(name+'-before','mixed',4,repeat,protocol,guard=True)
                    cells=[(mix,n) for mix in MIXES for n in protocol['concurrency']]
                    if repeat%2: cells.reverse()
                    for mix,n in cells: fixture.trial(f'{name}-{mix}-c{n}',mix,n,repeat,protocol)
                    fixture.trial(name+'-after','mixed',4,repeat,protocol,guard=True)
                    fixture.finish()
                finally: fixture.close()
        index['complete']=True;save(out/'index.json',index)
        result=validate_matrix(out,build,args.expected_revision)
        save(out/'report.json',result)
        print('PASS: complete benchmark matrix and calibration evidence independently checked',flush=True)
    except Exception as error:
        index.update(complete=False,failure=str(error));save(out/'index.json',index)
        raise


if __name__=='__main__': main()
