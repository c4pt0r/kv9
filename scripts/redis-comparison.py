#!/usr/bin/env python3
"""Owned Redis memory reference and durable KV9 hot-set comparison; no equivalent-durability claim."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time

from benchmark import Fixture, MIXES, host, process_sample, read, save
from workload_report import validate


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def observe(process, servers, directory, timeout):
    samples = []
    end = time.monotonic() + timeout
    while process.poll() is None:
        if time.monotonic() >= end:
            process.kill()
            process.wait(timeout=10)
            save(directory/'resource-samples.json', samples)
            raise TimeoutError('bounded client deadline exceeded; samples retained')
        observed = {'unix_ns': time.time_ns(), 'processes': {}}
        for name, pid in {'client': process.pid, **servers}.items():
            try:
                item = process_sample(pid)
                item['threads'] = {}
                for entry in Path('/proc', str(pid), 'task').iterdir():
                    try:
                        fields = (entry/'stat').read_text().rsplit(')', 1)[1].split()
                        item['threads'][entry.name] = {'user_ticks': int(fields[11]), 'system_ticks': int(fields[12])}
                    except OSError:
                        pass
                observed['processes'][name] = item
            except (OSError, KeyError):
                pass  # Process may have exited after poll; never fabricate a sample.
        samples.append(observed)
        time.sleep(.05)
    save(directory/'resource-samples.json', samples)
    return process.wait()


def cpu_summary(samples, start_ns, end_ns):
    inside = [s for s in samples if start_ns <= s['unix_ns'] <= end_ns]
    if len(inside) < 2:
        raise ValueError('insufficient samples within the measurement cohort')
    result = {}
    for name in inside[0]['processes'].keys() & inside[-1]['processes'].keys():
        a, b = inside[0]['processes'][name], inside[-1]['processes'][name]
        if a['pid'] != b['pid'] or a['start_ticks'] != b['start_ticks']:
            raise ValueError('resource sample process identity changed')
        seconds = (b['observed_monotonic_ns'] - a['observed_monotonic_ns']) / 1e9
        hz = os.sysconf('SC_CLK_TCK')
        ticks = b['user_ticks'] + b['system_ticks'] - a['user_ticks'] - a['system_ticks']
        threads = {tid: (b['threads'][tid]['user_ticks'] + b['threads'][tid]['system_ticks'] -
                         a['threads'][tid]['user_ticks'] - a['threads'][tid]['system_ticks']) / hz / seconds
                   for tid in a['threads'].keys() & b['threads'].keys()}
        result[name] = {'cpu_cores': ticks / hz / seconds, 'sample_seconds': seconds,
                        'max_thread_cpu_cores': max(threads.values(), default=0),
                        'thread_cpu_cores': threads, 'peak_rss_bytes': max(a['peak_rss_bytes'], b['peak_rss_bytes'])}
    return result


def histogram(histograms):
    buckets = [0] * 65
    count = total = 0
    for h in histograms:
        if len(h['buckets']) != 65 or sum(h['buckets']) != h['count']:
            raise ValueError('invalid latency population')
        count += h['count']; total += h['sum_ns']
        buckets = [a + b for a, b in zip(buckets, h['buckets'])]
    def percentile(p):
        if not count:
            return None
        threshold = (count * p + 99) // 100
        cumulative = 0
        for index, n in enumerate(buckets):
            cumulative += n
            if cumulative >= threshold:
                return [0, 0] if index == 0 else [1 << (index - 1), (1 << index) - 1]
        raise ValueError('missing histogram quantile')
    return dict(count=count, sum_ns=total, mean_ns=total / count if count else None,
                buckets=buckets, p50_ns=percentile(50), p95_ns=percentile(95), p99_ns=percentile(99))


def summarize(directory, target):
    r = read(directory/('report.json' if target == 'redis-memory' else 'run/report.json'), 16*1024*1024)
    if not r['complete']:
        raise ValueError('incomplete client report')
    if target == 'redis-memory':
        start = r['measurement_start_unix_ns']; elapsed = r['cohort_elapsed_ns']
        issued, success, completed = r['issued'], r['succeeded'], r['issued']
        if sum(w['issued'] for w in r['workers']) != issued or sum(w['succeeded'] for w in r['workers']) != success:
            raise ValueError('Redis worker totals do not match')
        if r['failed'] != issued - success or sum(w['failed'] for w in r['workers']) != r['failed']:
            raise ValueError('Redis failed counts do not match')
        hist = [histogram(w['latency'][op] for w in r['workers']) for op in range(3)]
        reason = r['stop_reason']; attempts = issued
        missing = sum(w['missing_gets'] for w in r['workers'])
        outcomes = {'success': success, 'failed': r['failed']}
    else:
        start = r['wall_anchor_unix_ns'] + r['stages']['measurement']['start_ns'] - r['wall_anchor_monotonic_ns']
        elapsed = r['cohort_elapsed_ns']
        issued, success, completed = r['measured_issued'], r['measured_successful'], r['measured_completed']
        phase = r['metrics']['phases'].index('measure')
        hist = [histogram(op['outcomes']) for op in r['metrics']['logical_latency'][phase]]
        reason = r['stop']['reason']
        attempts = sum(sum(row) for row in r['metrics']['attempt_counts'][phase])
        outcomes = {population: sum(row[i] for row in r['metrics']['logical_counts'][phase])
                    for i, population in enumerate(r['metrics']['populations'])}
        missing = None  # Existing performance report does not export hit/miss counts.
    if reason != 'duration' or completed != issued or sum(h['count'] for h in hist) != completed:
        raise ValueError('trial was truncated or histogram does not cover every completed operation')
    return dict(target=target, issued=issued, completed=completed, successful=success,
                failed=completed-success, attempts=attempts, outcomes=outcomes, missing_gets=missing,
                cohort_elapsed_ns=elapsed, success_ops_per_second=success*1e9/elapsed,
                latency=dict(zip(('get','put','delete'),hist)),
                cpu=cpu_summary(read(directory/'resource-samples.json',16*1024*1024),start,start+elapsed),
                report_sha256=sha(directory/('report.json' if target == 'redis-memory' else 'run/report.json')))


def kv9_trial(f, directory, mix, concurrency, repeat, protocol):
    serving = f.wait('serving leader before workload', f.leader)
    name = directory.name
    receipt = f.command([f.build/'kv9','client','create-keyspace','--addr',f.addresses[serving],
                         '--name',name,'--api-type','raw'])
    (directory/'create-keyspace.out').write_text(receipt+'\n')
    keyspace = int(dict(line.split('=',1) for line in receipt.splitlines())['keyspace_id'])
    c = dict(version=1, client=dict(version=1, peers=[dict(node_id=n,address=a) for n,a in f.addresses.items()],
             keyspace_id=keyspace,epoch_conf_ver=1,epoch_version=1,max_in_flight=concurrency,max_attempts=6,
             deadline_ms=1500,retry_backoff_ms=5),mode='performance',run_id=name,keyspace_name=name,seed=40+repeat,
             workers=concurrency,keys=protocol['keys'],value_bytes=protocol['value_bytes'],mix=MIXES[mix],
             warmup_operations=32,max_operations=1_000_000,measure_ms=protocol['measure_ms'],interval_ms=0,history_bytes=0)
    save(directory/'requested-config.json', c)
    f.snapshot(directory,'before')
    p = f.launch([f.build/'workload/kv9-workload','--config',directory/'requested-config.json',
                  '--build-manifest',f.build/'workload/build.json','--output',directory/'run'],directory/'workload.log',client=True)
    save(directory/'client-identity.json',f.identities[p.pid])
    code = observe(p, {f'server-{n}':q.pid for n,q in f.nodes.items()}, directory, protocol['measure_ms']/1000+180)
    save(directory/'exit.json',dict(exit_code=code))
    f.snapshot(directory,'after')
    if code:
        raise ValueError('KV9 client failed; raw attempt retained')
    save(directory/'checked.json', validate(directory/'run',f.build/'workload',seconds=60))


def redis_trial(directory, client, mix, concurrency, repeat, protocol, placement):
    reservation = socket.socket(); reservation.bind(('127.0.0.1',0)); port = reservation.getsockname()[1]
    config = directory/'redis.conf'
    config.write_text(f'bind 127.0.0.1\nport {port}\nprotected-mode yes\nsave ""\nappendonly no\n'
                      f'dir {directory}\ndaemonize no\nlogfile ""\nio-threads 1\nmaxclients 256\n')
    env = dict(os.environ); children = []; logs = []
    def launch(command, label, cpus):
        log = (directory/(label+'.log')).open('w'); logs.append(log)
        original = os.sched_getaffinity(0)
        try:
            os.sched_setaffinity(0,cpus)
            p = subprocess.Popen(list(map(str,command)),env=env,stdout=log,stderr=log); children.append(p)
        finally:
            os.sched_setaffinity(0,original)
        save(directory/(label+'-identity.json'),dict(**process_sample(p.pid),executable_sha256=sha(command[0])))
        return p
    try:
        reservation.close()
        server = launch([shutil.which('redis-server'),config],'redis-server',placement['server_cpus'])
        for _ in range(100):
            ready = subprocess.run(['redis-cli','-h','127.0.0.1','-p',str(port),'PING'],capture_output=True,text=True)
            if ready.returncode == 0 and ready.stdout.strip() == 'PONG':
                break
            if server.poll() is not None:
                raise ValueError('owned Redis server exited before readiness')
            time.sleep(.05)
        else:
            raise TimeoutError('Redis readiness timeout')
        for label, command in [('configuration',['CONFIG','GET','save','appendonly','appendfsync','io-threads']),('replication',['INFO','replication'])]:
            command = ['redis-cli','-h','127.0.0.1','-p',str(port),*command]
            result = subprocess.run(command,capture_output=True,text=True,check=True)
            save(directory/(label+'.json'),dict(command=command,stdout=result.stdout))
        c = dict(address=f'127.0.0.1:{port}',run_id=directory.name,seed=40+repeat,workers=concurrency,
                 keys=protocol['keys'],value_bytes=protocol['value_bytes'],**MIXES[mix],warmup_operations=32,
                 measure_ms=protocol['measure_ms'],max_operations=5_000_000)
        save(directory/'requested-config.json',c)
        p = launch([client,directory/'requested-config.json',directory/'report.json'],'redis-client',placement['client_cpus'])
        code = observe(p,{'server-1':server.pid},directory,protocol['measure_ms']/1000+30)
        save(directory/'exit.json',dict(exit_code=code))
        if code:
            raise ValueError('Redis client failed; raw attempt retained')
    finally:
        for p in reversed(children):
            if p.poll() is None:
                p.terminate()
                try: p.wait(timeout=5)
                except subprocess.TimeoutExpired: p.kill(); p.wait(timeout=5)
        for log in logs: log.close()
        reservation.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--build',type=Path,required=True)
    parser.add_argument('--redis-client',type=Path,required=True)
    parser.add_argument('--expected-revision',required=True)
    parser.add_argument('--concurrency',default='1,4,16,32,64')
    parser.add_argument('--repetitions',type=int,default=2)
    parser.add_argument('--measure-ms',type=int,default=3000)
    parser.add_argument('--keys',type=int,default=64)
    args = parser.parse_args()
    args.output = args.output.resolve(); args.build=args.build.resolve(); args.redis_client=args.redis_client.resolve()
    args.output.mkdir(parents=True,exist_ok=False)
    build = read(args.build/'build.json')
    if build['revision'] != args.expected_revision or build['dirty'] or build['profile'] != 'release':
        raise ValueError('exact clean release KV9 build required')
    for name, record in build['binaries'].items():
        if sha(args.build/name) != record['sha256']:
            raise ValueError('retained binary hash mismatch')
    concurrency = list(map(int,args.concurrency.split(',')))
    if not concurrency or any(c < 1 or c > 64 for c in concurrency) or len(set(concurrency)) != len(concurrency):
        raise ValueError('unique concurrency levels must be inside KV9 public admission limit 1..64')
    if not 1 <= args.repetitions <= 5 or not 500 <= args.measure_ms <= 10000 or not 1 <= args.keys <= 256:
        raise ValueError('comparison exceeds bounded local protocol')
    protocol = dict(version=1,reference_only=True,revision=args.expected_revision,keys=args.keys,value_bytes=128,
                    concurrency=concurrency,repetitions=args.repetitions,measure_ms=args.measure_ms,mixes=MIXES,
                    warmup_operations=32,pipeline_depth=1,network='IPv4 TCP loopback',
                    kv9_durability='Three WAL-backed voters; normal quorum acknowledgment and read barriers',
                    redis_durability='Standalone memory reference; save disabled; appendonly no; no replicas',
                    comparable_durability=False,redis_missing_get='A successful nil response; expected in mixed traffic',
                    kv9_missing_get='A successful None response; existing performance report does not export hit/miss counts',
                    retries='Redis none; KV9 only explicit NotLeader refusals, never an uncertain write',
                    operation_caps='KV9 1,000,000 total; Redis 5,000,000 measured; reaching either cap invalidates a timed trial',
                    key_format='Fixed-width trial name plus colon and 16 lowercase hexadecimal digits',
                    redis_client_sha256=sha(args.redis_client),redis_server_sha256=sha(shutil.which('redis-server')),
                    redis_version=subprocess.check_output(['redis-server','--version'],text=True).strip(),
                    limitations=['Single shared host; no inter-host RTT or independent storage failure domains',
                                 'Two different protocol clients; CPU and per-thread samples expose client saturation',
                                 'Closed-loop traffic; latency is not an open-loop overload/SLO claim',
                                 'Read hit/miss counts are only available in the Redis reference report'])
    save(args.output/'protocol.json',protocol)
    shutil.copy2(args.build/'build.json',args.output/'kv9-build.json')
    source_root=Path(__file__).resolve().parent
    save(args.output/'driver-sources.json',{str(p.relative_to(source_root)):sha(p) for p in
         [Path(__file__).resolve(),source_root/'redis-reference/Cargo.toml',source_root/'redis-reference/Cargo.lock',source_root/'redis-reference/src/main.rs']})
    placement=host(args.output,True)
    manifest=dict(version=1,complete=False,attempts=[],placement=placement)
    save(args.output/'matrix.json',manifest)
    f=Fixture(args.output/'kv9','wal',args.build,placement)
    try:
        f.start()
        (args.output/'redis-memory').mkdir()
        print('READY: '+str(args.output)+' KV9 addresses='+str(f.addresses),flush=True)
        for repeat in range(args.repetitions):
            # Reverse order in repetition two to expose simple thermal/order drift.
            for c in (concurrency if repeat % 2 == 0 else list(reversed(concurrency))):
                for mi,mix in enumerate(MIXES):
                    name=f'b{repeat:02}{c:02}{mi}'
                    for target in (['kv9','redis-memory'] if repeat % 2 == 0 else ['redis-memory','kv9']):
                        directory=args.output/target/name; directory.mkdir()
                        entry=dict(target=target,name=name,mix=mix,workers=c,repeat=repeat,complete=False)
                        manifest['attempts'].append(entry); save(args.output/'matrix.json',manifest)
                        try:
                            if target == 'kv9': kv9_trial(f,directory,mix,c,repeat,protocol)
                            else: redis_trial(directory,args.redis_client,mix,c,repeat,protocol,placement)
                            result=summarize(directory,target); save(directory/'summary.json',result)
                            entry.update(complete=True,summary=result)
                            print(f"PASS {target} {mix} c={c} r={repeat}: {result['success_ops_per_second']:.1f} ops/s, failures={result['failed']}",flush=True)
                        except Exception as error:
                            entry['error']=repr(error); raise
                        finally: save(args.output/'matrix.json',manifest)
        f.finish(); manifest['complete']=True
    finally:
        f.close(); save(args.output/'matrix.json',manifest)
    print('PASS: bounded Redis reference comparison; all raw attempts retained',flush=True)


if __name__ == '__main__':
    main()
