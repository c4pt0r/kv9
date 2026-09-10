"""Build/stage a frozen normal runtime in private paths; never launches faults."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse
import importlib.util
import re
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

HELPERS = Path(__file__).resolve().parent
TREE = HELPERS.parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--kubeconfig', type=Path, required=True)
parser.add_argument('--kind', required=True)
parser.add_argument('--kind-cluster', required=True)
parser.add_argument('--image', required=True, help='Fresh, unique local image tag')
parser.add_argument('--target', type=Path, help='Fresh private Cargo target (default OUTPUT/target)')
args = parser.parse_args()
ROOT = args.output.resolve()
assert not ROOT.is_relative_to(TREE), 'evidence output must be outside the repository'
assert re.fullmatch(r'[a-z0-9][a-z0-9._-]*:[a-zA-Z0-9_][a-zA-Z0-9_.-]*', args.image), 'use a local repository:tag'
assert not subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=all'], cwd=TREE), 'freeze a clean source first'
os.umask(0o077)
ROOT.mkdir(parents=True, exist_ok=False)
PLAN = ROOT / 'plan.json'
spec = importlib.util.spec_from_file_location('workload_build_contract', TREE/'scripts/build-workload.py')
source_module = importlib.util.module_from_spec(spec); spec.loader.exec_module(source_module)
snapshot = source_module.snapshot()
assert not snapshot['dirty']
cpus = set(range(6, 32))
assert cpus <= os.sched_getaffinity(0), 'launch with access to CPUs 6-31'
os.sched_setaffinity(0, cpus)
target_arg = (args.target or ROOT/'target').resolve()
assert not target_arg.exists() and not target_arg.is_relative_to(TREE), 'private target must be fresh and outside source'
# Explicit source-derived scope: a changed production limit needs a reviewed gate.
limits = dict(public_rpc_limit_requests=64, public_rpc_limit_encoded_bytes=64*1024*1024,
              raft_async_read_limit=128, raft_async_apply_limit=128, sources=[])
for relative, markers in [
    ('crates/server/src/admission.rs', ['max_requests: 64,', 'max_encoded_bytes: 64 * 1024 * 1024,']),
    ('crates/raft/src/async_read.rs', ['const MAX_REQUESTS: usize = 128;']),
    ('crates/raft/src/async_apply.rs', ['const MAX_REQUESTS: usize = 128;'])]:
    path = TREE/relative
    assert all(marker in path.read_text() for marker in markers), 'review changed production limits'
    limits['sources'].append(dict(path=str(path), sha256=source_module.sha(path.read_bytes())))
p = dict(version=2, revision=snapshot['revision'], worktree=str(TREE), source=snapshot['sources'],
         source_tree_sha256=source_module.sha(source_module.canonical(snapshot['sources'])),
         source_default_limits=limits, private_cargo_target=str(target_arg), context=str(ROOT/'image-context'),
         evidence_parent=str(ROOT/'runs'), kubeconfig=str(args.kubeconfig.resolve()), kind=str(Path(args.kind).resolve()),
         kind_cluster=args.kind_cluster, image=args.image, host_cpus=sorted(cpus),
         pod_cpu_limitations='Shared kind host. Pod CPU masks are recorded, not host-core isolated; correctness only.',
         build_started=False, build_complete=False, fixture_started=False, fixture_launch_authorized=False,
         overlay_inputs={str(path): source_module.sha(path.read_bytes()) for path in sorted(HELPERS.iterdir()) if path.is_file()},
         scope='Original 21 actual faults with additive normal-port streaming native atomic batch histories; separate client-link and quorum-loss acceptance required.')
Path(p['evidence_parent']).mkdir()
tree = Path(p['worktree'])
target = Path(p['private_cargo_target'])
assert not target.exists()
context = Path(p['context'])
context.mkdir(exist_ok=False)
env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS='8', GITHUB_SHA=p['revision'], KUBECONFIG=p['kubeconfig'], PYTHONDONTWRITEBYTECODE='1', PYTHONOPTIMIZE='0')
records = []


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save():
    PLAN.write_text(json.dumps(p, indent=2) + '\n')
    (ROOT / 'build-sequence.json').write_text(json.dumps(records, indent=2) + '\n')


def command(name, argv, cwd=tree):
    stdout, stderr = ROOT / (name + '.stdout'), ROOT / (name + '.stderr')
    start = time.time_ns()
    row = dict(name=name, command=argv, cwd=str(cwd), started_unix_ns=start, exit_code=None, stdout=str(stdout), stderr=str(stderr))
    records.append(row)
    try:
        with stdout.open('x') as out, stderr.open('x') as err:
            done = subprocess.run(argv, cwd=cwd, env=env, stdout=out, stderr=err, timeout=1800)
        row['exit_code'] = done.returncode
    except BaseException as error:
        row['failure'] = repr(error)
        raise
    finally:
        row['ended_unix_ns'] = time.time_ns()
        save()
    assert row['exit_code'] == 0, row
    print('PASS: ' + name, flush=True)
    return stdout


def features(path, executable):
    rows = [json.loads(line) for line in path.read_text().splitlines()]
    assert rows[-1]['reason'] == 'build-finished' and rows[-1]['success'] is True
    selected = {}
    for name in (executable, 'kv9_engine', 'kv9_raft', 'kv9_server'):
        found = [r for r in rows if r.get('reason') == 'compiler-artifact' and r.get('target', {}).get('name') == name]
        assert len(found) == 1 and found[0]['features'] == [] and found[0]['profile']['test'] is False, (name, found)
        selected[name] = found[0]
    assert selected[executable].get('executable')
    return selected


p.update(build_started=True, build_started_unix_ns=time.time_ns(), build_environment={
    key: env[key] for key in ('CARGO_TARGET_DIR', 'CARGO_BUILD_JOBS', 'GITHUB_SHA')})
save()
try:
    before_ns = json.loads(command('namespace-before-build', ['kubectl', '--kubeconfig', p['kubeconfig'], '--request-timeout=10s', 'get', 'namespaces', '-o', 'json']).read_text())
    p['preserve_namespaces'] = {x['metadata']['name']: x['metadata']['uid'] for x in before_ns['items']}
    server_json = command('server-cargo', ['cargo', 'build', '--locked', '--bin', 'kv9', '--message-format=json-render-diagnostics'])
    server_artifacts = features(server_json, 'kv9')
    executable = Path(server_artifacts['kv9']['executable'])
    shutil.copy2(executable, ROOT / 'kv9')
    server_hash = sha(ROOT / 'kv9')
    p['production_binary'] = dict(path=str(ROOT / 'kv9'), sha256=server_hash)
    p['default_dependency_features'] = {name: row['features'] for name, row in server_artifacts.items()}
    p['server_artifacts'] = server_artifacts
    save()
    pressure_json = command('pressure-cargo', ['cargo', 'build', '--locked', '-p', 'kv9-server', '--example',
                                             'admission-pressure', '--message-format=json-render-diagnostics'])
    assert sha(executable) == server_hash
    pressure_rows = [json.loads(line) for line in pressure_json.read_text().splitlines()]
    pressure = [r for r in pressure_rows if r.get('reason') == 'compiler-artifact' and
                r.get('target', {}).get('name') == 'admission-pressure' and r.get('executable')]
    assert len(pressure) == 1
    shutil.copy2(pressure[0]['executable'], ROOT / 'admission-pressure')
    p['pressure_binary'] = dict(path=str(ROOT / 'admission-pressure'), sha256=sha(ROOT / 'admission-pressure'))
    command('workload-build', ['python3', 'scripts/build-workload.py', '--output', str(ROOT / 'persistent-build')])
    assert sha(executable) == server_hash
    client = json.loads((ROOT / 'persistent-build/build.json').read_text())
    inventory = json.loads((ROOT / 'persistent-build/sources.json').read_text())
    assert client['revision'] == p['revision'] and client['dirty'] is False
    assert inventory['sources'] == p['source'] and inventory['source_tree_sha256'] == p['source_tree_sha256']
    p['workload_artifacts'] = features(ROOT / 'persistent-build/cargo.jsonl', 'kv9-workload')
    p['correctness_client_build'] = client
    command('native-build', ['python3', 'scripts/build-workload.py', '--binary', 'kv9-batch-workload', '--output', str(ROOT / 'native-build')])
    native = json.loads((ROOT / 'native-build/build.json').read_text())
    native_sources = json.loads((ROOT / 'native-build/sources.json').read_text())
    assert native_sources['sources'] == p['source'] and native['revision'] == p['revision'] and native['dirty'] is False
    assert sha(executable) == server_hash
    p['native_workload_artifacts'] = features(ROOT / 'native-build/cargo.jsonl', 'kv9-batch-workload')
    p['native_client_build'] = native
    for source, dest in (
        (ROOT / 'kv9', context / 'target/debug/kv9'),
        (ROOT / 'admission-pressure', context / 'target/debug/examples/admission-pressure'),
        (ROOT / 'persistent-build/kv9-workload', context / 'target/chaos-persistent/kv9-workload'),
        (ROOT / 'persistent-build/build.json', context / 'target/chaos-persistent/build.json'),
        (tree/'chaos/native-batch.Dockerfile', context / 'chaos/Dockerfile'),
        (ROOT / 'native-build/kv9-batch-workload', context / 'target/chaos-native/kv9-batch-workload'),
        (ROOT / 'native-build/build.json', context / 'target/chaos-native/build.json'),
        (tree / 'chaos/no-statx.c', context / 'chaos/no-statx.c'),
    ):
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)
    p['context_files'] = {str(path.relative_to(context)): sha(path) for path in context.rglob('*') if path.is_file()}
    command('image-build', ['docker', 'build', '-q', '-f', 'chaos/Dockerfile', '-t', p['image'], '.'], context)
    inspect = json.loads(command('image-inspect', ['docker', 'image', 'inspect', p['image']]).read_text())[0]
    hashes_output = command('image-payload-hashes', ['docker', 'run', '--rm', '--entrypoint', 'sha256sum', p['image'],
        '/usr/local/bin/kv9', '/usr/local/bin/kv9-workload', '/usr/local/bin/admission-pressure', '/opt/kv9-workload/build.json', '/usr/local/bin/kv9-batch-workload', '/opt/kv9-batch-workload/build.json'])
    hashes = {line.split()[1]: line.split()[0] for line in hashes_output.read_text().splitlines()}
    assert hashes['/usr/local/bin/kv9'] == server_hash
    assert hashes['/usr/local/bin/kv9-workload'] == client['binary_sha256']
    assert hashes['/usr/local/bin/admission-pressure'] == p['pressure_binary']['sha256']
    assert hashes['/opt/kv9-workload/build.json'] == sha(ROOT / 'persistent-build/build.json')
    assert hashes['/usr/local/bin/kv9-batch-workload'] == native['binary_sha256']
    assert hashes['/opt/kv9-batch-workload/build.json'] == sha(ROOT / 'native-build/build.json')
    p['prepared_image'] = dict(tag=p['image'], docker_image_id=inspect['Id'], hashes=hashes, complete=True)
    command('kind-load', [p['kind'], 'load', 'docker-image', '--name', p['kind_cluster'], p['image']])
    for name, expected in p['source'].items():
        assert sha(tree / name) == expected, name
    assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=tree)
    namespaces = json.loads(command('namespace-after-build', ['kubectl', '--kubeconfig', p['kubeconfig'],
        '--request-timeout=10s', 'get', 'namespaces', '-o', 'json']).read_text())
    actual = {row['metadata']['name']: row['metadata']['uid'] for row in namespaces['items']}
    assert all(actual.get(name) == uid for name, uid in p['preserve_namespaces'].items())
    nodes = command('kind-nodes', [p['kind'], 'get', 'nodes', '--name', p['kind_cluster']]).read_text().splitlines()
    digests = set()
    p['cri_mappings'] = {}
    for node in nodes:
        mapping = json.loads(command('cri-' + node, ['docker', 'exec', node, 'crictl', 'inspecti', inspect['Id']]).read_text())
        assert mapping['status']['id'] == inspect['Id'] and 'docker.io/library/' + p['image'] in mapping['status']['repoTags']
        digests.update(mapping['status']['repoDigests'])
        p['cri_mappings'][node] = mapping
    assert digests and nodes
    p['accepted_cri_image_ids'] = sorted(digests)
    p.update(build_complete=True, build_exit_code=0, client_build_exit_code=0, source_unchanged=True,
             namespace_recheck_after_image=actual, build_ended_unix_ns=time.time_ns())
except Exception as error:
    p.update(build_complete=False, build_failure=repr(error), build_ended_unix_ns=time.time_ns())
    raise
finally:
    save()
subprocess.run(['python3', str(HELPERS/'freeze.py'), '--plan', str(PLAN)], check=True, env=env)
print(json.dumps(dict(complete=True, server=p['production_binary'], client_sha256=client['binary_sha256'],
                     image=p['prepared_image']['docker_image_id'], fault_matrix_launched=False), indent=2))
