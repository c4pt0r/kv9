"""Read-only same-artifact acceptance; run validators on a fresh copy, never relabel a harness exit."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

HELPERS = Path(__file__).resolve().parent
TREE = HELPERS.parents[1]


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def inventory(root):
    result = {}
    for path in sorted(root.rglob('*')):
        name = str(path.relative_to(root))
        if path.is_symlink():
            result[name] = dict(symlink=os.readlink(path), resolved=str(path.resolve()))
        elif path.is_file():
            result[name] = dict(bytes=path.stat().st_size, sha256=sha(path))
    return result


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--plan', type=Path, required=True)
    ap.add_argument('--artifact', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--cleanup', type=Path, help='Retained post-run cleanup directory, if already cleaned')
    a = ap.parse_args()
    raw, out = a.artifact.resolve(), a.output.resolve()
    assert not out.is_relative_to(raw) and not out.is_relative_to(TREE), 'readback must be outside source/original evidence'
    out.mkdir(parents=True, exist_ok=False)
    p = read(a.plan)
    copied = out/'artifact'
    commands = []
    result = dict(accepted=False, original_artifact=str(raw), original_harness_exit_code=p.get('fixture_exit_code'),
                  original_harness_complete=p.get('fixture_exit_code') == 0, matrix_rerun=False)
    original = inventory(raw)
    (out/'original-inventory.json').write_text(json.dumps(original, indent=2)+'\n')

    def run(name, command):
        row = dict(name=name, command=command, cwd=str(TREE), started_unix_ns=time.time_ns(), exit_code=None)
        commands.append(row)
        try:
            with (out/(name+'.stdout')).open('x') as stdout, (out/(name+'.stderr')).open('x') as stderr:
                done = subprocess.run(command, cwd=TREE, env=dict(os.environ, PYTHONDONTWRITEBYTECODE='1', PYTHONOPTIMIZE='0'), stdout=stdout, stderr=stderr, timeout=180)
            row['exit_code'] = done.returncode
        except BaseException as error:
            row['failure'] = repr(error)
            raise
        finally:
            row['ended_unix_ns'] = time.time_ns()
            (out/'commands.json').write_text(json.dumps(commands, indent=2)+'\n')
        assert row['exit_code'] == 0, row
        return (out/(name+'.stdout')).read_text()

    try:
        assert p['fixture_exit_code'] in (0, 1) and p['observer_exit_code'] == 0
        log = (a.plan.parent/'matrix.log').read_text().splitlines()
        assert log[-1] == 'COMMAND_EXIT='+str(p['fixture_exit_code'])
        if p['fixture_exit_code'] == 1:
            assert read(raw/'native-window-audit.json')['failure'] == 'physical observation timestamp malformed'
            assert 'ValueError: physical observation timestamp malformed' in log
        assert sha(p['ready_plan_path']) == p['ready_plan_sha256']
        for rel, expected in p['source'].items():
            assert sha(Path(p['worktree'])/rel) == expected, rel
            # New maintained validators must still use exactly the old runtime,
            # workload, history checker and original point/effect predicates.
            if rel.startswith(('crates/', 'scripts/history/')) or rel in ('Cargo.toml', 'Cargo.lock') or (rel.startswith('scripts/') and rel.endswith('.py') and 'native_batch_chaos/' not in rel):
                assert sha(TREE/rel) == expected, ('source composition differs', rel)
        for path, expected in p['overlay_inputs'].items():
            assert sha(path) == expected, ('executed helper changed', path)
        sequence = read(a.plan.parent/'build-sequence.json')
        assert all(row['exit_code'] == 0 for row in sequence)
        server = next(x for x in sequence if x['name']=='server-cargo')
        pressure = next(x for x in sequence if x['name']=='pressure-cargo')
        point = next(x for x in sequence if x['name']=='workload-build')
        assert server['command']==['cargo','build','--locked','--bin','kv9','--message-format=json-render-diagnostics']
        assert server['ended_unix_ns'] < pressure['started_unix_ns'] < pressure['ended_unix_ns'] < point['started_unix_ns']
        for file, target in [(a.plan.parent/'server-cargo.stdout', 'kv9'), (raw/'persistent-build/cargo.jsonl', 'kv9-workload'), (raw/'native-build/cargo.jsonl', 'kv9-batch-workload')]:
            rows = [json.loads(line) for line in file.read_text().splitlines()]
            assert rows[-1] == dict(reason='build-finished', success=True)
            for name in (target, 'kv9_engine', 'kv9_raft', 'kv9_server'):
                found = [x for x in rows if x.get('reason')=='compiler-artifact' and x['target']['name']==name]
                assert len(found)==1 and found[0]['features']==[] and found[0]['profile']['test'] is False, (file,name)
        hashes = p['prepared_image']['hashes']
        for file, image_path in [(Path(p['production_binary']['path']), '/usr/local/bin/kv9'), (Path(p['pressure_binary']['path']), '/usr/local/bin/admission-pressure'), (raw/'persistent-build/kv9-workload', '/usr/local/bin/kv9-workload'), (raw/'native-build/kv9-batch-workload', '/usr/local/bin/kv9-batch-workload')]:
            assert sha(file) == hashes[image_path], image_path
        assert read(a.plan.parent/'image-inspect.stdout')[0]['Id'] == p['prepared_image']['docker_image_id']
        for rel, expected in p['context_files'].items():
            assert sha(Path(p['context'])/rel)==expected, rel
        point_identity_path = raw/'point-live-client-identity.json'
        if not point_identity_path.exists():
            point_identity_path = a.plan.parent/'point-live-client-identity.json'
        client = read(point_identity_path)
        assert client['complete'] and client['executing_sha256']==hashes['/usr/local/bin/kv9-workload']
        assert client['configuration']==read(raw/'persistent-config.json')
        assert client['configuration']['rpc_transport']=='tonic_stream'
        assert client['process_pid']==read(raw/'persistent-run/report.json')['process_id']
        assert client['before']['metadata']['uid']==client['after']['metadata']['uid']==client['pod_uid']
        assert client['before']['status']['containerStatuses']==client['after']['status']['containerStatuses']
        assert all(x['imageID'] in p['accepted_cri_image_ids'] for x in client['before']['status']['containerStatuses'])
        capture=next(x for x in reversed(client['commands']) if x['exit_code']==0 and '@@STAT_BEFORE@@' in x['stdout'])
        def section(name):
            return capture['stdout'].split('@@'+name+'@@\n',1)[1].split('@@',1)[0].strip()
        for name in ('STAT_BEFORE','STAT_AFTER'):
            stat=section(name); assert int(stat.split(' ',1)[0])==client['process_pid']
            fields=stat.rsplit(')',1)[1].split()
            assert fields[0] not in ('Z','X','x') and int(fields[19])==client['process_start_ticks']
        assert section('BOOT_BEFORE')==section('BOOT_AFTER')==client['boot_id']
        assert json.loads(section('CONFIG'))==client['configuration']
        assert json.loads(section('BUILD'))==p['correctness_client_build']
        assert client['executing_sha256']+f'  /proc/{client["process_pid"]}/exe' in section('HASH')
        assert client['executing_sha256']+'  /usr/local/bin/kv9-workload' in section('HASH')
        inodes={x[8:-1] for x in section('SOCKET_FDS').splitlines() if x.startswith('socket:[')}
        connected=[x.split() for x in section('TCP').splitlines()[1:] if x.split()[9] in inodes and x.split()[3]=='01']
        assert connected==client['connections'] and all(int(x[2].split(':')[1],16)==20160 for x in connected)
        live = read(raw/'async-write-live-observer.json')
        assert live['complete'] and live['expected_server_sha256']==hashes['/usr/local/bin/kv9']
        matched = [r for batch in live['observations'] for r in batch['rows'] if r.get('identity_matched')]
        assert matched and {'1','2','3','4'} <= {x['node'] for x in matched}
        for row in matched:
            assert row['status_contract_valid'] is True
            assert all(x['imageID'] in p['accepted_cri_image_ids'] for x in row['container_statuses'])
        # All original effect markers are required; only the historical parser
        # failure may explain the missing final whole-fixture PASS marker.
        block = (Path(p['worktree'])/'.github/workflows/correctness.yml').read_text().split('      - name: Run and verify the Chaos Mesh acceptance matrix\n',1)[1].split('      - name: Collect cluster diagnostics',1)[0]
        markers = re.findall(r"grep -Fxc '([^']+)'", block)
        final_marker='PASS: Chaos Mesh root boundary, Pod kill/failure, partition, delay, container recovery, and Raft I/O faults'
        for marker in markers:
            assert log.count(marker) == (int(p['fixture_exit_code']==0) if marker==final_marker else 1), marker
        excluded = {'persistent-evidence-controls', 'persistent-history-checker.json', 'native-window-audit.json'}
        shutil.copytree(raw, copied, symlinks=True, ignore=lambda directory,names: excluded if Path(directory)==raw else [])
        run('persistent', ['python3','scripts/check-persistent-chaos.py',str(copied),'--expected-revision',p['revision']])
        phases = [x['phase'] for x in read(copied/'persistent-history-checker.json')['windows']]
        assert len(phases)==len(set(phases))==21
        for phase in phases:
            for client in ('concurrent','persistent'):
                assert log.count(f'PASS: {client} put and read completed during {phase}')==1
        run('cli', ['python3','scripts/history/checker.py',str(copied/'history.jsonl'),'--output',str(copied/'independent-history-checker.json'),'--acceptance','--require','put','get','delete','scan','delete_range','create_keyspace','--seconds','60','--require-phase',*phases])
        assert read(copied/'independent-history-checker.json')['verdict']=='valid'
        shutil.copy2(copied/'independent-history-checker.json', copied/'history-checker.json')
        for name, suffix, extra in [('formation','-chaos',[]),('store-loss','-chaos',[]),('store-replacement','-chaos',[]),('endpoint-migration','-chaos',[]),('latency-metrics','',[]),('admission-pressure','',['--self-test'])]:
            run(name,['python3',f'scripts/check-{name}{suffix}.py',str(copied),*extra])
        for name in ('inline-audit','apply-audit','delay-audit'):
            run(name,['python3',str(HELPERS/(name+'.py')),'--plan',str(a.plan),'--artifact',str(copied)])
        run('native', ['python3',str(HELPERS/'check-native-windows.py'),'--plan',str(a.plan),'--artifact',str(copied)])
        native = read(copied/'native-window-audit.json')
        assert native['accepted'] and len(native['windows'])==22 and len(native['final_drain']['writers'])==4
        histories = {}
        for name, rel in [('cli','history.jsonl'),('persistent','persistent-run/history.jsonl'),('native','native-run/history.jsonl')]:
            assert sha(raw/rel)==sha(copied/rel)
            events=[json.loads(line) for line in (raw/rel).read_text().splitlines()]
            inv=[x for x in events if x.get('type')=='invoke']; ret=[x for x in events if x.get('type')=='return']
            assert len(inv)==len(ret)
            histories[name]=dict(calls=len(inv),outcomes=dict(Counter(x['outcome'] for x in ret)),sha256=sha(raw/rel))
        cleanup = None
        if a.cleanup:
            cleanup=read(a.cleanup/'summary.json')
            exited=read(a.cleanup/'all-server-lifetimes-exited.json')
            # Cleanup attestation is retained evidence, not a new live cluster query.
            assert cleanup.get('complete') is True and exited.get('complete') is True
            assert cleanup['namespace']==live['namespace'] and cleanup['namespace_absent'] is True
            assert cleanup['historical_namespaces_unchanged']==p['preserve_namespaces'] and cleanup['host_fixture_observer_exited'] is True
            observed={(x['node'],x['pod_uid'],x['process_pid'],x['process_start_ticks'],x['process_boot_id']) for x in matched}
            attested={(x['node'],x['pod_uid'],x['namespace_pid'],x['start_ticks'],x['boot_id']) for x in exited['server_lifetimes']}
            assert observed==attested
            assert all(x['all_containers_exited'] is True and x['container_ids'] for x in exited['server_lifetimes'])
            assert all(state in ('removed','CONTAINER_EXITED') for state in exited['container_states'].values())
            for row in exited['server_lifetimes']:
                assert all(cid in exited['container_states'] for cid in row['container_ids'])

        assert inventory(raw)==original, 'original artifact was mutated during readback'
        result.update(accepted=True, revision=p['revision'], histories=histories, native=native,
                      observer_batches=len(live['observations']), exact_runtime_samples=len(matched),
                      cleanup=cleanup, source_files=len(p['source']), current_helpers={x.name:sha(x) for x in HELPERS.iterdir() if x.is_file()},
                      scope='Maintained validator compatibility on the original 21-fault artifact. No new runtime, fault execution, client-link, quorum-loss, performance or physical crash claim.')
    except BaseException as error:
        result.update(failure=repr(error), traceback=traceback.format_exc())
        raise
    finally:
        result['commands']=commands
        (out/'summary.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ('accepted','original_harness_exit_code','revision','histories','observer_batches','exact_runtime_samples')}))


if __name__=='__main__':
    main()
